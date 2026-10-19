use super::types::{JsonRpcNotification, JsonRpcRequest, JsonRpcResponse};
use serde_json::Value;
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin};
use tokio::sync::{oneshot, Mutex};

pub struct StdioTransport {
    stdin: Arc<Mutex<ChildStdin>>,
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<JsonRpcResponse>>>>,
    child: Arc<Mutex<Child>>,
    next_id: AtomicU64,
}

impl StdioTransport {
    pub async fn spawn(
        command: &str,
        args: &[String],
        env: &HashMap<String, String>,
    ) -> Result<Self, String> {
        let mut cmd = Self::build_command(command, args);

        for (k, v) in env {
            cmd.env(k, v);
        }

        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        #[cfg(windows)]
        {
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let mut child = cmd
            .spawn()
            .map_err(|e| format!("启动 MCP 进程失败 ({command}): {e}"))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "无法获取 MCP 进程 stdin".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "无法获取 MCP 进程 stdout".to_string())?;
        let stderr = child.stderr.take();

        let pending: Arc<Mutex<HashMap<String, oneshot::Sender<JsonRpcResponse>>>> =
            Arc::new(Mutex::new(HashMap::new()));

        // 后台任务：读取 stdout
        let pending_clone = pending.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                if let Ok(resp) = serde_json::from_str::<JsonRpcResponse>(trimmed) {
                    let id_str = match &resp.id {
                        Value::String(s) => s.clone(),
                        Value::Number(n) => n.to_string(),
                        _ => resp.id.to_string(),
                    };
                    let mut map = pending_clone.lock().await;
                    if let Some(tx) = map.remove(&id_str) {
                        let _ = tx.send(resp);
                    }
                }
            }
        });

        // 后台任务：读取 stderr 记录日志，防止子进程缓冲区填满阻塞
        if let Some(stderr) = stderr {
            tokio::spawn(async move {
                let mut reader = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = reader.next_line().await {
                    #[cfg(debug_assertions)]
                    eprintln!("[MCP stderr] {}", line);
                }
            });
        }

        Ok(Self {
            stdin: Arc::new(Mutex::new(stdin)),
            pending,
            child: Arc::new(Mutex::new(child)),
            next_id: AtomicU64::new(1),
        })
    }

    #[cfg(windows)]
    fn build_command(command: &str, args: &[String]) -> tokio::process::Command {
        let trimmed = command.trim();
        // Windows 下若为 npx, npm 等批处理，通过 cmd.exe /c 封装以保证兼容性
        if trimmed == "npx" || trimmed == "npm" || trimmed.ends_with(".cmd") || trimmed.ends_with(".bat") {
            let mut cmd = tokio::process::Command::new("cmd.exe");
            cmd.arg("/c").arg(trimmed);
            for arg in args {
                cmd.arg(arg);
            }
            cmd
        } else {
            let mut cmd = tokio::process::Command::new(trimmed);
            for arg in args {
                cmd.arg(arg);
            }
            cmd
        }
    }

    #[cfg(not(windows))]
    fn build_command(command: &str, args: &[String]) -> tokio::process::Command {
        let mut cmd = tokio::process::Command::new(command);
        for arg in args {
            cmd.arg(arg);
        }
        cmd
    }

    pub async fn send_request(
        &self,
        method: &str,
        params: Option<Value>,
        timeout_dur: std::time::Duration,
    ) -> Result<Value, String> {
        let id_num = self.next_id.fetch_add(1, Ordering::SeqCst);
        let id_str = id_num.to_string();

        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Value::Number(id_num.into()),
            method: method.to_string(),
            params,
        };

        let json_line = serde_json::to_string(&req)
            .map_err(|e| format!("序列化请求失败: {e}"))? + "\n";

        let (tx, rx) = oneshot::channel();
        {
            let mut map = self.pending.lock().await;
            map.insert(id_str.clone(), tx);
        }

        {
            let mut stdin = self.stdin.lock().await;
            stdin
                .write_all(json_line.as_bytes())
                .await
                .map_err(|e| format!("写入 MCP 进程失败: {e}"))?;
            stdin
                .flush()
                .await
                .map_err(|e| format!("刷新 MCP 进程失败: {e}"))?;
        }

        match tokio::time::timeout(timeout_dur, rx).await {
            Ok(Ok(resp)) => {
                if let Some(err) = resp.error {
                    Err(format!("MCP 错误 [{}]: {}", err.code, err.message))
                } else if let Some(res) = resp.result {
                    Ok(res)
                } else {
                    Ok(Value::Null)
                }
            }
            Ok(Err(_)) => {
                let mut map = self.pending.lock().await;
                map.remove(&id_str);
                Err("MCP 响应管道中断（子进程可能已退出）".to_string())
            }
            Err(_) => {
                let mut map = self.pending.lock().await;
                map.remove(&id_str);
                Err(format!("MCP 请求超时 ({}s): {method}", timeout_dur.as_secs()))
            }
        }
    }

    pub async fn send_notification(
        &self,
        method: &str,
        params: Option<Value>,
    ) -> Result<(), String> {
        let notif = JsonRpcNotification {
            jsonrpc: "2.0".to_string(),
            method: method.to_string(),
            params,
        };
        let json_line = serde_json::to_string(&notif)
            .map_err(|e| format!("序列化通知失败: {e}"))? + "\n";

        let mut stdin = self.stdin.lock().await;
        stdin
            .write_all(json_line.as_bytes())
            .await
            .map_err(|e| format!("写入 MCP 通知失败: {e}"))?;
        stdin
            .flush()
            .await
            .map_err(|e| format!("刷新 MCP 通知失败: {e}"))?;
        Ok(())
    }

    pub async fn terminate(&self) {
        let mut child = self.child.lock().await;
        let _ = child.kill().await;
    }
}
