use crate::models::{ApprovalRequest, CompactionRequest, ToolEvent};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StepPhase {
    Idle,
    Connecting,
    Thinking {
        reasoning: String,
    },
    ToolPreparing {
        call_id: String,
        tool_name: String,
        partial_args: String,
        bytes_received: usize,
    },
    ToolExecuting {
        call_id: String,
        tool_name: String,
        event_id: String,
    },
    Delivering {
        content: String,
    },
}

impl Default for StepPhase {
    fn default() -> Self {
        Self::Idle
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ActiveStepState {
    pub session_id: String,
    pub run_id: String,
    pub step_index: usize,
    pub message_id: String,
    pub phase: StepPhase,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SessionActiveState {
    pub session_id: String,
    pub is_running: bool,
    pub active_run_id: Option<String>,
    pub current_message_id: Option<String>,
    pub streaming_content: String,
    pub streaming_reasoning: String,
    pub active_tool_events: Vec<ToolEvent>,
    pub pending_approval: Option<ApprovalRequest>,
    pub pending_compaction: Option<CompactionRequest>,
    pub active_step: Option<ActiveStepState>,
}

pub struct SnapshotStore {
    inner: Mutex<HashMap<String, SessionActiveState>>,
}

impl Default for SnapshotStore {
    fn default() -> Self {
        Self::new()
    }
}

impl SnapshotStore {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }

    pub fn get(&self, session_id: &str) -> SessionActiveState {
        let mut map = self.inner.lock().unwrap();
        map.entry(session_id.to_string())
            .or_insert_with(|| SessionActiveState {
                session_id: session_id.to_string(),
                ..Default::default()
            })
            .clone()
    }

    pub fn start_run(&self, session_id: &str, run_id: &str) {
        let mut map = self.inner.lock().unwrap();
        let st = map.entry(session_id.to_string()).or_default();
        st.session_id = session_id.to_string();
        st.is_running = true;
        st.active_run_id = Some(run_id.to_string());
        st.current_message_id = None;
        st.streaming_content.clear();
        st.streaming_reasoning.clear();
        st.active_tool_events.clear();
        st.pending_approval = None;
        st.pending_compaction = None;
        st.active_step = None;
    }

    /// 切换并开启新步骤：原子清空流式思考与正文缓冲区，彻底杜绝上一步思考跨步骤残留
    pub fn transition_to_step(&self, session_id: &str, run_id: &str, step_index: usize, message_id: &str) {
        let mut map = self.inner.lock().unwrap();
        let st = map.entry(session_id.to_string()).or_default();
        st.session_id = session_id.to_string();
        st.is_running = true;
        st.active_run_id = Some(run_id.to_string());
        st.current_message_id = Some(message_id.to_string());
        st.streaming_content.clear();
        st.streaming_reasoning.clear();
        st.active_step = Some(ActiveStepState {
            session_id: session_id.to_string(),
            run_id: run_id.to_string(),
            step_index,
            message_id: message_id.to_string(),
            phase: StepPhase::Connecting,
            updated_at_ms: chrono::Utc::now().timestamp_millis() as u64,
        });
    }

    pub fn finish_run(&self, session_id: &str) {
        let mut map = self.inner.lock().unwrap();
        if let Some(st) = map.get_mut(session_id) {
            st.is_running = false;
            st.active_run_id = None;
            st.current_message_id = None;
            st.streaming_content.clear();
            st.streaming_reasoning.clear();
            st.active_tool_events.clear();
            st.pending_approval = None;
            st.pending_compaction = None;
            st.active_step = None;
        }
    }

    pub fn finish_step(&self, session_id: &str) {
        let mut map = self.inner.lock().unwrap();
        if let Some(st) = map.get_mut(session_id) {
            st.streaming_content.clear();
            st.streaming_reasoning.clear();
            st.active_step = None;
        }
    }

    pub fn append_content_delta(&self, session_id: &str, delta: &str, message_id: Option<&str>) {
        let mut map = self.inner.lock().unwrap();
        let st = map.entry(session_id.to_string()).or_default();
        if let Some(mid) = message_id {
            if st.current_message_id.as_deref() != Some(mid) {
                st.current_message_id = Some(mid.to_string());
                st.streaming_content.clear();
                st.streaming_reasoning.clear();
            }
        }
        st.streaming_content.push_str(delta);
        if let Some(ref mut step) = st.active_step {
            step.phase = StepPhase::Delivering {
                content: st.streaming_content.clone(),
            };
            step.updated_at_ms = chrono::Utc::now().timestamp_millis() as u64;
        }
    }

    pub fn append_reasoning_delta(&self, session_id: &str, delta: &str, message_id: Option<&str>) {
        let mut map = self.inner.lock().unwrap();
        let st = map.entry(session_id.to_string()).or_default();
        if let Some(mid) = message_id {
            if st.current_message_id.as_deref() != Some(mid) {
                st.current_message_id = Some(mid.to_string());
                st.streaming_content.clear();
                st.streaming_reasoning.clear();
            }
        }
        st.streaming_reasoning.push_str(delta);
        if let Some(ref mut step) = st.active_step {
            step.phase = StepPhase::Thinking {
                reasoning: st.streaming_reasoning.clone(),
            };
            step.updated_at_ms = chrono::Utc::now().timestamp_millis() as u64;
        }
    }

    pub fn start_tool_preparing(&self, session_id: &str, call_id: &str, tool_name: &str) {
        let mut map = self.inner.lock().unwrap();
        let st = map.entry(session_id.to_string()).or_default();
        if let Some(ref mut step) = st.active_step {
            step.phase = StepPhase::ToolPreparing {
                call_id: call_id.to_string(),
                tool_name: tool_name.to_string(),
                partial_args: String::new(),
                bytes_received: 0,
            };
            step.updated_at_ms = chrono::Utc::now().timestamp_millis() as u64;
        }
    }

    pub fn append_tool_args_delta(&self, session_id: &str, _call_id: &str, delta: &str) {
        let mut map = self.inner.lock().unwrap();
        let st = map.entry(session_id.to_string()).or_default();
        if let Some(ref mut step) = st.active_step {
            if let StepPhase::ToolPreparing {
                ref mut partial_args,
                ref mut bytes_received,
                ..
            } = step.phase
            {
                partial_args.push_str(delta);
                *bytes_received += delta.len();
                step.updated_at_ms = chrono::Utc::now().timestamp_millis() as u64;
            }
        }
    }

    pub fn start_tool_executing(&self, session_id: &str, call_id: &str, tool_name: &str, event_id: &str) {
        let mut map = self.inner.lock().unwrap();
        let st = map.entry(session_id.to_string()).or_default();
        if let Some(ref mut step) = st.active_step {
            step.phase = StepPhase::ToolExecuting {
                call_id: call_id.to_string(),
                tool_name: tool_name.to_string(),
                event_id: event_id.to_string(),
            };
            step.updated_at_ms = chrono::Utc::now().timestamp_millis() as u64;
        }
    }

    pub fn upsert_tool_event(&self, session_id: &str, ev: ToolEvent) {
        let mut map = self.inner.lock().unwrap();
        let st = map.entry(session_id.to_string()).or_default();
        if let Some(idx) = st.active_tool_events.iter().position(|e| e.id == ev.id) {
            st.active_tool_events[idx] = ev;
        } else {
            st.active_tool_events.push(ev);
        }
    }

    pub fn remove_finished_tool_event(&self, session_id: &str, event_id: &str) {
        let mut map = self.inner.lock().unwrap();
        if let Some(st) = map.get_mut(session_id) {
            st.active_tool_events.retain(|e| e.id != event_id);
        }
    }

    pub fn set_pending_approval(&self, session_id: &str, req: Option<ApprovalRequest>) {
        let mut map = self.inner.lock().unwrap();
        let st = map.entry(session_id.to_string()).or_default();
        st.pending_approval = req;
    }

    pub fn set_pending_compaction(&self, session_id: &str, req: Option<CompactionRequest>) {
        let mut map = self.inner.lock().unwrap();
        let st = map.entry(session_id.to_string()).or_default();
        st.pending_compaction = req;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_snapshot_lifecycle_and_deltas() {
        let store = SnapshotStore::new();
        let sid = "session-test-1";

        // 初始为空
        let init = store.get(sid);
        assert!(!init.is_running);
        assert_eq!(init.streaming_content, "");

        // 启动 run
        store.start_run(sid, "run-100");
        let after_start = store.get(sid);
        assert!(after_start.is_running);
        assert_eq!(after_start.active_run_id.as_deref(), Some("run-100"));

        // 步骤切换：Step 0
        store.transition_to_step(sid, "run-100", 0, "msg-1");
        let step0 = store.get(sid);
        assert_eq!(step0.current_message_id.as_deref(), Some("msg-1"));
        assert!(matches!(step0.active_step.unwrap().phase, StepPhase::Connecting));

        // 追加流式思考与正文
        store.append_reasoning_delta(sid, "思考中...", Some("msg-1"));
        store.append_content_delta(sid, "正在生成代码...", Some("msg-1"));
        let with_delta = store.get(sid);
        assert_eq!(with_delta.current_message_id.as_deref(), Some("msg-1"));
        assert_eq!(with_delta.streaming_reasoning, "思考中...");
        assert_eq!(with_delta.streaming_content, "正在生成代码...");

        // 模拟工具准备中（写长文件）
        store.start_tool_preparing(sid, "call-1", "write_file");
        store.append_tool_args_delta(sid, "call-1", r#"{"path": "test.rs"}"#);
        let preparing = store.get(sid);
        if let Some(step) = preparing.active_step {
            if let StepPhase::ToolPreparing { tool_name, bytes_received, .. } = step.phase {
                assert_eq!(tool_name, "write_file");
                assert_eq!(bytes_received, 19);
            } else {
                panic!("Expected ToolPreparing phase");
            }
        }

        // 步骤切换：Step 1（原子清空前一步残留）
        store.transition_to_step(sid, "run-100", 1, "msg-2");
        let step1 = store.get(sid);
        assert_eq!(step1.current_message_id.as_deref(), Some("msg-2"));
        assert_eq!(step1.streaming_reasoning, ""); // 必须彻底清空！
        assert_eq!(step1.streaming_content, "");

        // 活跃工具事件
        let ev = ToolEvent {
            id: "tool-ev-1".into(),
            message_id: "msg-1".into(),
            tool_name: "run_command".into(),
            tool_call_id: Some("call-1".into()),
            params: serde_json::json!({"command": "cargo check"}),
            result_text: None,
            status: "running".into(),
            approval_scope: None,
            created_at: "2026-09-17T00:00:00Z".into(),
            subprocess_id: None,
            reverted_at: None,
        };
        store.upsert_tool_event(sid, ev);
        assert_eq!(store.get(sid).active_tool_events.len(), 1);

        // 工具完成移除
        store.remove_finished_tool_event(sid, "tool-ev-1");
        assert_eq!(store.get(sid).active_tool_events.len(), 0);

        // 完成 run
        store.finish_run(sid);
        let finished = store.get(sid);
        assert!(!finished.is_running);
        assert_eq!(finished.streaming_content, "");
        assert_eq!(finished.active_run_id, None);
        assert!(finished.active_step.is_none());
    }
}
