use crate::diagnostics::types::{JsRuntimeError, NetworkErrorItem, SourceLocation};
use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, oneshot, Mutex};
use tokio_tungstenite::tungstenite::Message;

pub struct CdpClient {
    write_tx: tokio::sync::mpsc::UnboundedSender<Message>,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>>,
    next_id: AtomicU64,
    pub console_logs: Arc<Mutex<Vec<String>>>,
    pub network_errors: Arc<Mutex<Vec<String>>>,
    pub runtime_exceptions: Arc<Mutex<Vec<JsRuntimeError>>>,
    pub structured_network_errors: Arc<Mutex<Vec<NetworkErrorItem>>>,
    pub requests_map: Arc<Mutex<HashMap<String, (String, String)>>>,
    pub screencast_tx: broadcast::Sender<String>,
}

/// 在无代理干扰的连接上下文中执行 WebSocket 连接
///
/// 背景（2026-11-24 定位）：开发机常开 Clash / V2Ray 等代理，环境变量里存在
/// `ALL_PROXY=http://127.0.0.1:7890`。`tokio-tungstenite` 的 `connect_async(url)`
/// 会读取进程级代理环境变量且**不认 `NO_PROXY`**，导致对
/// `ws://127.0.0.1:<devtools端口>` 的连接被送去代理中转：
/// - 握手要等到代理侧超时才回落 → 首次连接耗时高达 6~7 秒；
/// - 更严重的是代理中转后无法可靠双向透传 WebSocket 帧 →
///   浏览器级命令（Target.*）能通，但所有 target 级命令（带 sessionId 的
///   Page.enable / Runtime.evaluate 等）全部超时，表现为「浏览器一直显示启动中」。
///
/// 修复：CDP 端点是本机回环地址，按设计绝不应走代理。这里在连接期间临时移除
/// 代理类环境变量（连接完成后立即恢复），从根源保证直连。
async fn connect_without_proxy(
    ws_url: &str,
) -> Result<
    (
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        tokio_tungstenite::tungstenite::handshake::client::Response,
    ),
    tokio_tungstenite::tungstenite::Error,
> {
    const PROXY_VARS: [&str; 6] = [
        "ALL_PROXY",
        "all_proxy",
        "HTTP_PROXY",
        "http_proxy",
        "HTTPS_PROXY",
        "https_proxy",
    ];

    // 保存并在连接期间移除，连接后恢复，避免影响其它网络行为
    let saved: Vec<(String, String)> = PROXY_VARS
        .iter()
        .filter_map(|k| std::env::var(k).ok().map(|v| ((*k).to_string(), v)))
        .collect();
    for (k, _) in &saved {
        std::env::remove_var(k);
    }

    let result = tokio_tungstenite::connect_async(ws_url).await;

    for (k, v) in saved {
        std::env::set_var(k, v);
    }

    result
}

impl CdpClient {
    pub async fn connect(ws_url: &str) -> Result<Self, String> {
        // 连接超时保护：
        //
        // 历史缺陷 —— 此处直接 await 无超时。当 Chromium 已开出 DevToolsActivePort
        // 但进程半死（如 Crashpad 写 LOCALAPPDATA 被拒、profile 锁冲突导致主进程卡在
        // 初始化），WebSocket 握手会永不完成。因为这是整条启动链路上唯一没有超时的
        // await，前端 `navigating` 状态就会永久停在「启动中...」，既无报错也无恢复。
        let (ws_stream, _) = match tokio::time::timeout(
            Duration::from_secs(8),
            connect_without_proxy(ws_url),
        )
        .await
        {
            Ok(Ok(pair)) => pair,
            Ok(Err(e)) => {
                return Err(format!("连接 CDP WebSocket 失败 ({ws_url}): {e}"));
            }
            Err(_) => {
                return Err(format!(
                    "连接 CDP WebSocket 超时 (8s)：{ws_url}\n\
                     浏览器可能处于半启动状态（进程已写出调试端口但握手未完成）。\n\
                     常见原因：profile 目录被其它实例锁定，或低权限进程无法写入 LOCALAPPDATA。\n\
                     建议：关闭其它 harness-mini / 浏览器实例后点击「重新检测活跃实例」重试。"
                ));
            }
        };

        let (mut write_half, mut read_half) = ws_stream.split();
        let (write_tx, mut write_rx) = tokio::sync::mpsc::unbounded_channel::<Message>();

        // 后台写入循环
        tokio::spawn(async move {
            while let Some(msg) = write_rx.recv().await {
                if write_half.send(msg).await.is_err() {
                    break;
                }
            }
        });

        let pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let console_logs = Arc::new(Mutex::new(Vec::new()));
        let network_errors = Arc::new(Mutex::new(Vec::new()));
        let runtime_exceptions = Arc::new(Mutex::new(Vec::new()));
        let structured_network_errors = Arc::new(Mutex::new(Vec::new()));
        let requests_map: Arc<Mutex<HashMap<String, (String, String)>>> = Arc::new(Mutex::new(HashMap::new()));
        let (screencast_tx, _) = broadcast::channel(64);

        let pending_clone = pending.clone();
        let logs_clone = console_logs.clone();
        let net_clone = network_errors.clone();
        let exceptions_clone = runtime_exceptions.clone();
        let struct_net_clone = structured_network_errors.clone();
        let reqs_clone = requests_map.clone();
        let cast_clone = screencast_tx.clone();
        let write_tx_clone = write_tx.clone();

        // 后台读取与事件调度循环
        tokio::spawn(async move {
            while let Some(Ok(msg)) = read_half.next().await {
                if let Message::Text(text) = msg {
                    if let Ok(v) = serde_json::from_str::<Value>(&text) {
                        // 1. 请求响应 (有 id)
                        if let Some(id) = v.get("id").and_then(|i| i.as_u64()) {
                            let mut map = pending_clone.lock().await;
                            if let Some(tx) = map.remove(&id) {
                                if let Some(err) = v.get("error") {
                                    let msg = err
                                        .get("message")
                                        .and_then(|m| m.as_str())
                                        .unwrap_or("CDP 错误");
                                    let _ = tx.send(Err(msg.to_string()));
                                } else {
                                    let res = v.get("result").cloned().unwrap_or(Value::Null);
                                    let _ = tx.send(Ok(res));
                                }
                            }
                            continue;
                        }

                        // 2. DevTools 事件通知 (有 method)
                        if let Some(method) = v.get("method").and_then(|m| m.as_str()) {
                            let params = v.get("params");
                            match method {
                                "Runtime.consoleAPICalled" => {
                                    if let Some(p) = params {
                                        let log_type = p.get("type").and_then(|t| t.as_str()).unwrap_or("log");
                                        let args: Vec<String> = p
                                            .get("args")
                                            .and_then(|a| a.as_array())
                                            .map(|arr| {
                                                arr.iter()
                                                    .map(|item| {
                                                        item.get("value")
                                                            .map(|v| v.to_string())
                                                            .unwrap_or_else(|| item.to_string())
                                                    })
                                                    .collect()
                                            })
                                            .unwrap_or_default();
                                        let log_line = format!("[Console.{log_type}] {}", args.join(" "));
                                        let mut l = logs_clone.lock().await;
                                        if l.len() > 100 {
                                            l.remove(0);
                                        }
                                        l.push(log_line);
                                    }
                                }
                                "Runtime.exceptionThrown" => {
                                    if let Some(p) = params {
                                        let details = p.get("exceptionDetails");
                                        let text = details
                                            .and_then(|d| d.get("text"))
                                            .and_then(|t| t.as_str())
                                            .unwrap_or("Uncaught");
                                        let desc = details
                                            .and_then(|d| d.get("exception"))
                                            .and_then(|e| e.get("description"))
                                            .and_then(|d| d.as_str())
                                            .unwrap_or(text);

                                        let mut frames = Vec::new();
                                        if let Some(cf_arr) = details
                                            .and_then(|d| d.get("stackTrace"))
                                            .and_then(|st| st.get("callFrames"))
                                            .and_then(|cf| cf.as_array())
                                        {
                                            for f in cf_arr {
                                                let mut url = f.get("url").and_then(|u| u.as_str()).unwrap_or("").to_string();
                                                if let Some(idx) = url.find("webpack-internal:///./") {
                                                    url = url[idx + "webpack-internal:///./".len()..].to_string();
                                                } else if let Some(idx) = url.find("webpack:///") {
                                                    url = url[idx + "webpack:///".len()..].to_string();
                                                }
                                                frames.push(SourceLocation {
                                                    file: url,
                                                    line: f.get("lineNumber").and_then(|l| l.as_u64()).unwrap_or(0) as u32,
                                                    column: f.get("columnNumber").and_then(|c| c.as_u64()).unwrap_or(0) as u32,
                                                    function_name: f.get("functionName").and_then(|n| n.as_str()).map(ToString::to_string),
                                                });
                                            }
                                        }

                                        let first_line = desc.lines().next().unwrap_or(desc);
                                        let frame_summary = if !frames.is_empty() {
                                            let top = &frames[0];
                                            let fn_part = top.function_name.as_deref().unwrap_or("<anonymous>");
                                            format!(" at {} ({}:{})", fn_part, top.file, top.line)
                                        } else {
                                            String::new()
                                        };

                                        let log_line = format!("[Uncaught Exception] {first_line}{frame_summary}");
                                        let mut l = logs_clone.lock().await;
                                        if l.len() > 100 {
                                            l.remove(0);
                                        }
                                        l.push(log_line);

                                        let err_item = JsRuntimeError {
                                            name: text.to_string(),
                                            message: first_line.to_string(),
                                            stack_trace: frames,
                                            raw_description: desc.to_string(),
                                            timestamp: std::time::SystemTime::now()
                                                .duration_since(std::time::UNIX_EPOCH)
                                                .map(|d| d.as_millis() as u64)
                                                .unwrap_or(0),
                                        };
                                        let mut ex = exceptions_clone.lock().await;
                                        if ex.len() > 50 {
                                            ex.remove(0);
                                        }
                                        ex.push(err_item);
                                    }
                                }
                                "Log.entryAdded" => {
                                    if let Some(entry) = params.and_then(|p| p.get("entry")) {
                                        let level = entry.get("level").and_then(|l| l.as_str()).unwrap_or("info");
                                        let text = entry.get("text").and_then(|t| t.as_str()).unwrap_or("");
                                        let url = entry.get("url").and_then(|u| u.as_str()).unwrap_or("");
                                        let line = entry.get("lineNumber").and_then(|l| l.as_i64()).unwrap_or(0);
                                        if level == "error" {
                                            let mut l = logs_clone.lock().await;
                                            if l.len() > 100 {
                                                l.remove(0);
                                            }
                                            let loc = if !url.is_empty() { format!(" ({url}:{line})") } else { String::new() };
                                            l.push(format!("[Log.error] {text}{loc}"));
                                        }
                                    }
                                }
                                "Network.requestWillBeSent" => {
                                    if let Some(p) = params {
                                        let req_id = p.get("requestId").and_then(|r| r.as_str()).unwrap_or("");
                                        if let Some(req) = p.get("request") {
                                            let url = req.get("url").and_then(|u| u.as_str()).unwrap_or("");
                                            let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("GET");
                                            if !req_id.is_empty() && !url.is_empty() {
                                                let mut map = reqs_clone.lock().await;
                                                if map.len() > 300 {
                                                    map.clear();
                                                }
                                                map.insert(req_id.to_string(), (url.to_string(), method.to_string()));
                                            }
                                        }
                                    }
                                }
                                "Network.responseReceived" => {
                                    if let Some(resp) = params.and_then(|p| p.get("response")) {
                                        let status = resp.get("status").and_then(|s| s.as_u64()).unwrap_or(200);
                                        let url = resp.get("url").and_then(|u| u.as_str()).unwrap_or("");
                                        if status >= 400 {
                                            let req_id = params.and_then(|p| p.get("requestId")).and_then(|r| r.as_str()).unwrap_or("");
                                            let method = {
                                                let map = reqs_clone.lock().await;
                                                map.get(req_id).map(|(_, m)| m.clone()).unwrap_or_else(|| "REQ".to_string())
                                            };
                                            let mut n = net_clone.lock().await;
                                            if n.len() > 50 {
                                                n.remove(0);
                                            }
                                            n.push(format!("[HTTP {status}] {method} {url}"));

                                            let mut sn = struct_net_clone.lock().await;
                                            if sn.len() > 50 {
                                                sn.remove(0);
                                            }
                                            sn.push(NetworkErrorItem {
                                                url: url.to_string(),
                                                method,
                                                status: Some(status as u16),
                                                failure_reason: format!("HTTP {status}"),
                                                response_body_snippet: None,
                                            });
                                        }
                                    }
                                }
                                "Network.loadingFinished" => {
                                    // 历史缺陷（2026-11-24 实测）：requests_map 只插入不删除，
                                    // 导致「在途请求数」永久只增不减，wait_for_network_idle
                                    // 无法基于它判定静默。补上完成即移除。
                                    if let Some(p) = params {
                                        let req_id = p.get("requestId").and_then(|r| r.as_str()).unwrap_or("");
                                        if !req_id.is_empty() {
                                            let mut map = reqs_clone.lock().await;
                                            map.remove(req_id);
                                        }
                                    }
                                }
                                "Network.loadingFailed" => {
                                    if let Some(p) = params {
                                        let err = p.get("errorText").and_then(|e| e.as_str()).unwrap_or("请求失败");
                                        // 无论成功失败，该请求都已终结，必须从在途集合中移除
                                        let req_id = p.get("requestId").and_then(|r| r.as_str()).unwrap_or("");
                                        let req_info = {
                                            let mut map = reqs_clone.lock().await;
                                            if req_id.is_empty() {
                                                None
                                            } else {
                                                map.remove(req_id)
                                            }
                                        };

                                        // 过滤掉 SPA 路由切换与静态资源预加载的主动取消 (net::ERR_ABORTED)：
                                        // 仅影响「错误上报」，不影响上面的在途移除。
                                        if err != "net::ERR_ABORTED" {
                                            let (url, method) = match req_info {
                                                Some((u, m)) => (u, m),
                                                None => ("unknown".to_string(), "REQ".to_string()),
                                            };

                                            let mut n = net_clone.lock().await;
                                            if n.len() > 50 {
                                                n.remove(0);
                                            }
                                            n.push(format!("[Network Failed] {method} {url} -> {err}"));

                                            let mut sn = struct_net_clone.lock().await;
                                            if sn.len() > 50 {
                                                sn.remove(0);
                                            }
                                            sn.push(NetworkErrorItem {
                                                url,
                                                method,
                                                status: None,
                                                failure_reason: err.to_string(),
                                                response_body_snippet: None,
                                            });
                                        }
                                    }
                                }
                                "Page.screencastFrame" => {
                                    let target_sid = v.get("sessionId").and_then(|s| s.as_str());
                                    if let Some(p) = params {
                                        if let Some(data) = p.get("data").and_then(|d| d.as_str()) {
                                            let _ = cast_clone.send(data.to_string());
                                        }
                                        if let Some(sid) = p.get("sessionId").and_then(|s| s.as_i64()) {
                                            let mut ack = json!({
                                                "id": 0,
                                                "method": "Page.screencastFrameAck",
                                                "params": { "sessionId": sid }
                                            });
                                            if let Some(tsid) = target_sid {
                                                ack["sessionId"] = json!(tsid);
                                            }
                                            let _ = write_tx_clone.send(Message::Text(ack.to_string()));
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
        });

        Ok(Self {
            write_tx,
            pending,
            next_id: AtomicU64::new(1),
            console_logs,
            network_errors,
            runtime_exceptions,
            structured_network_errors,
            requests_map,
            screencast_tx,
        })
    }

    pub fn is_alive(&self) -> bool {
        !self.write_tx.is_closed()
    }

    /// 当前在途请求快照：(总数, 前若干条 "METHOD URL")
    ///
    /// 供 `wait_for_network_idle` 判定「网络真正静默」。
    /// 依赖 `Network.requestWillBeSent` 插入、`loadingFinished` / `loadingFailed` 移除。
    pub async fn inflight_requests(&self, sample: usize) -> (usize, Vec<String>) {
        let map = self.requests_map.lock().await;
        let total = map.len();
        let samples = map
            .values()
            .take(sample)
            .map(|(url, method)| {
                let short = if url.len() > 80 { &url[..80] } else { url.as_str() };
                format!("{method} {short}")
            })
            .collect();
        (total, samples)
    }

    pub async fn send(
        &self,
        method: &str,
        params: Option<Value>,
        session_id: Option<&str>,
        timeout_dur: Duration,
    ) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let mut req = json!({
            "id": id,
            "method": method
        });

        if let Some(p) = params {
            req["params"] = p;
        }
        if let Some(sid) = session_id {
            req["sessionId"] = json!(sid);
        }

        let (tx, rx) = oneshot::channel();
        // 历史缺陷（2026-11-24 定案）：此处 `pending.lock().await` 原先在
        // `tokio::time::timeout` 之外。一旦读循环因事件洪水/锁竞争没能及时释放
        // pending，send 就会在这一步**永久阻塞**，而 timeout 根本还没生效 ——
        // 表现为「连超时都不触发」，前端永久停在「启动中...」。
        // 修复：把「取锁 + 插入」也纳入超时保护。
        match tokio::time::timeout(
            Duration::from_secs(5),
            async {
                let mut map = self.pending.lock().await;
                map.insert(id, tx);
            },
        )
        .await
        {
            Ok(()) => {}
            Err(_) => {
                return Err(format!("CDP 请求队列繁忙，注册回调超时: {method}"));
            }
        }

        self.write_tx
            .send(Message::Text(req.to_string()))
            .map_err(|e| format!("发送 CDP 命令失败: {e}"))?;

        match tokio::time::timeout(timeout_dur, rx).await {
            Ok(Ok(res)) => res,
            Ok(Err(_)) => {
                // 清理注册的回调；同样加超时保护，避免清理阶段再次永久阻塞
                let _ = tokio::time::timeout(Duration::from_secs(2), async {
                    let mut map = self.pending.lock().await;
                    map.remove(&id);
                })
                .await;
                Err("CDP 连接已断开".to_string())
            }
            Err(_) => {
                let _ = tokio::time::timeout(Duration::from_secs(2), async {
                    let mut map = self.pending.lock().await;
                    map.remove(&id);
                })
                .await;
                Err(format!("CDP 操作超时: {method}"))
            }
        }
    }
}
