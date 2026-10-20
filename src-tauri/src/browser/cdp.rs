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

impl CdpClient {
    pub async fn connect(ws_url: &str) -> Result<Self, String> {
        let (ws_stream, _) = tokio_tungstenite::connect_async(ws_url)
            .await
            .map_err(|e| format!("连接 CDP WebSocket 失败 ({ws_url}): {e}"))?;

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
                                "Network.loadingFailed" => {
                                    if let Some(p) = params {
                                        let err = p.get("errorText").and_then(|e| e.as_str()).unwrap_or("请求失败");
                                        // 过滤掉 SPA 路由切换与静态资源预加载的主动取消 (net::ERR_ABORTED)
                                        if err != "net::ERR_ABORTED" {
                                            let req_id = p.get("requestId").and_then(|r| r.as_str()).unwrap_or("");
                                            let req_info = {
                                                let map = reqs_clone.lock().await;
                                                map.get(req_id).cloned()
                                            };
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
        {
            let mut map = self.pending.lock().await;
            map.insert(id, tx);
        }

        self.write_tx
            .send(Message::Text(req.to_string()))
            .map_err(|e| format!("发送 CDP 命令失败: {e}"))?;

        match tokio::time::timeout(timeout_dur, rx).await {
            Ok(Ok(res)) => res,
            Ok(Err(_)) => {
                let mut map = self.pending.lock().await;
                map.remove(&id);
                Err("CDP 连接已断开".to_string())
            }
            Err(_) => {
                let mut map = self.pending.lock().await;
                map.remove(&id);
                Err(format!("CDP 操作超时: {method}"))
            }
        }
    }
}
