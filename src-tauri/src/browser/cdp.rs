use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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
    pub requests_map: Arc<Mutex<HashMap<String, (String, String)>>>,
    pub screencast_tx: broadcast::Sender<String>,
    pub auth_expired: Arc<AtomicBool>,
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
        let requests_map: Arc<Mutex<HashMap<String, (String, String)>>> = Arc::new(Mutex::new(HashMap::new()));
        let (screencast_tx, _) = broadcast::channel(64);
        let auth_expired = Arc::new(AtomicBool::new(false));

        let pending_clone = pending.clone();
        let logs_clone = console_logs.clone();
        let net_clone = network_errors.clone();
        let reqs_clone = requests_map.clone();
        let cast_clone = screencast_tx.clone();
        let write_tx_clone = write_tx.clone();
        let auth_expired_clone = auth_expired.clone();

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
                                        let text = p
                                            .get("exceptionDetails")
                                            .and_then(|d| d.get("text"))
                                            .and_then(|t| t.as_str())
                                            .unwrap_or("未捕获的 JavaScript 异常");
                                        let mut l = logs_clone.lock().await;
                                        if l.len() > 100 {
                                            l.remove(0);
                                        }
                                        l.push(format!("[Uncaught Exception] {text}"));
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
                                            if status == 401 || status == 403 {
                                                auth_expired_clone.store(true, Ordering::SeqCst);
                                            }
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
                                            let mut n = net_clone.lock().await;
                                            if n.len() > 50 {
                                                n.remove(0);
                                            }
                                            if let Some((url, method)) = req_info {
                                                n.push(format!("[Network Failed] {method} {url} -> {err}"));
                                            } else {
                                                n.push(format!("[Network Failed] {err}"));
                                            }
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
            requests_map,
            screencast_tx,
            auth_expired,
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
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
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
