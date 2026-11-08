use crate::agent;
use crate::models::LongTask;
use crate::store;
use serde_json::json;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::watch;

/// 会话自动推进守护器 (Autonomous Session Auto-Driver)
/// 架构第一性原理：
/// 1. 主会话是唯一的大脑与决策中心，享有 100% 完整上下文，不再受外部粗暴预拆解与 200 字符截断困扰；
/// 2. 守护进程只负责检测主会话空闲并提供自动推进脉冲，彻底消除 300ms DB 轮询与复杂的静态波次矩阵；
/// 3. 实施工件由主会话在当轮直接调用 spawn_subagent 派发给 Worker 子沙箱，主会话只接收精炼摘要以保护 Token。
pub async fn run_task_loop(app: AppHandle, mut task: LongTask, rx: watch::Receiver<crate::task::DaemonControlSignal>) {
    let state = app.state::<crate::AppState>();
    let task_id = task.id.clone();
    let session_id = task.session_id.clone();

    // 初始状态广播
    task.status = "running".to_string();
    task.updated_at = store::now();
    {
        // 步骤11：长任务数据路由到会话专属库
        let _ = state.with_session_db(&session_id, |conn| store::update_long_task(conn, &task));
    }
    let _ = app.emit("task:update", &task);

    let mut idle_consecutive_ticks = 0usize;

    loop {
        // 1. 响应控制信道指令（强类型信号分流与终态保护）
        let signal = rx.borrow().clone();
        match signal {
            crate::task::DaemonControlSignal::Pause { .. } => {
                let db_status = state
                    .with_session_db(&session_id, |conn| store::get_long_task(conn, &task_id))
                    .ok()
                    .flatten()
                    .map(|t| t.status);
                // 终态保护：若已在数据库中宣告完成或取消，严禁反向写回 paused！
                if db_status.as_deref() != Some("completed") && db_status.as_deref() != Some("cancelled") {
                    task.status = "paused".to_string();
                    task.updated_at = store::now();
                    let _ = state.with_session_db(&session_id, |conn| store::update_long_task(conn, &task));
                    let _ = app.emit("task:update", &task);
                }
                crate::task::remove_control(&task_id);
                return;
            }
            crate::task::DaemonControlSignal::Cancel { .. } => {
                crate::task::remove_control(&task_id);
                return;
            }
            crate::task::DaemonControlSignal::Complete { .. } => {
                crate::task::remove_control(&task_id);
                return;
            }
            crate::task::DaemonControlSignal::Running => {}
        }

        // 2. 检查任务是否已在数据库中达成终态（如已被 complete_long_task 结案）
        // 长任务状态走会话库；sessions.total_tokens 是 core.db 元数据，走 catalog
        let db_task_status = state
            .with_session_db(&session_id, |conn| store::get_long_task(conn, &task_id))
            .ok()
            .flatten()
            .map(|t| t.status)
            .unwrap_or_else(|| "unknown".into());
        let total_tokens: u64 = {
            let db = state.db.lock();
            db.query_row(
                "SELECT COALESCE(total_tokens, 0) FROM sessions WHERE id = ?1",
                rusqlite::params![&session_id],
                |r| r.get::<_, i64>(0),
            )
            .map(|v| v as u64)
            .unwrap_or(0)
        };

        if db_task_status == "completed" || db_task_status == "cancelled" || db_task_status == "paused" {
            crate::task::remove_control(&task_id);
            return;
        }

        // 3. 预算硬熔断防护
        if let Some(budget) = task.max_budget_tokens {
            if total_tokens >= budget {
                task.status = "paused".to_string();
                task.updated_at = store::now();
                let _ = state.with_session_db(&session_id, |conn| {
                    let _ = store::update_long_task(conn, &task);
                    let alert = format!(
                        "⚠️ **长任务已触发 Token 预算硬熔断保护！**\n已消耗 {} tokens，超过预设上限 {} tokens。任务已自动挂起，等待人工确认。",
                        total_tokens, budget
                    );
                    let _ = store::new_message(conn, &session_id, "assistant", Some(alert), false);
                    Ok::<(), String>(())
                });
                let _ = app.emit("task:update", &task);
                let _ = app.emit("task:budget_exceeded", json!({ "taskId": task_id, "tokensUsed": total_tokens, "budget": budget }));
                return;
            }
        }

        // 4. 同步 Token 使用量至长任务状态（供前端 Task 面板实时感知）
        if total_tokens != task.total_tokens_used {
            task.total_tokens_used = total_tokens;
            task.updated_at = store::now();
            let _ = app.emit("task:update", &task);
        }

        // 5. 检查会话运行态
        let is_running = agent::is_run_active(&state, &session_id);

        if is_running {
            idle_consecutive_ticks = 0;
            // 主会话或其子沙箱正在运行，休眠 800ms 继续守候
            tokio::time::sleep(Duration::from_millis(800)).await;
            continue;
        }

        // 6. 主会话处于空闲状态（当前轮次已执行完）：
        // 防抖缓冲：确保会话确实稳定处于空闲，防止在轮次切换间隙误判
        idle_consecutive_ticks += 1;
        if idle_consecutive_ticks < 2 {
            tokio::time::sleep(Duration::from_millis(600)).await;
            continue;
        }

        // 7. 检查最后一轮助手消息内容，判断是否达成终态目标
        let last_message_info = state
            .with_session_db(&session_id, |conn| store::all_messages(conn, &session_id))
            .map(|msgs| {
                msgs.last()
                    .map(|m| (m.role.clone(), m.content.clone().unwrap_or_default()))
            })
            .unwrap_or(None);

        if let Some((role, content)) = last_message_info {
            // 若最后一轮消息是 user，说明上一轮指令仍在排队或等待模型初次响应，绝不可重复叠加自治推进脉冲！
            if role == "user" {
                tokio::time::sleep(Duration::from_millis(800)).await;
                continue;
            }

            // 终态结案识别
            if role == "assistant" && (
                content.contains("🎯 **长任务全部目标达成**")
                || content.contains("🎯 长任务全部目标达成")
                || content.contains("【长任务结案报告】")
                || content.contains("长任务已全部达成")
            ) {
                let _ = crate::task::complete_long_task(&app, &task_id, Some("长任务已由主会话推进完成并通过自检验收"), None);
                return;
            }

            // 识别是否停下等待人工确认（不确定性二元决策门禁，自动安全挂起防止后台空转催促）
            if role == "assistant" && is_waiting_for_user_confirmation(&content) {
                task.status = "paused".to_string();
                task.updated_at = store::now();
                let _ = state.with_session_db(&session_id, |conn| store::update_long_task(conn, &task));
                let _ = app.emit("task:update", &task);
                return;
            }
        }

        // 8. 尚未完结且会话已空闲，发送自动推进脉冲！
        task.current_step += 1;
        task.updated_at = store::now();
        {
            let _ = state.with_session_db(&session_id, |conn| store::update_long_task(conn, &task));
        }
        let _ = app.emit("task:update", &task);

        // 驱动主会话进入下一轮自治推进
        idle_consecutive_ticks = 0;
        let _ = agent::continue_turn_autonomous(&app, &session_id);

        tokio::time::sleep(Duration::from_millis(1000)).await;
    }
}

/// 智能判定助手是否停下请求用户裁决或审阅确认
pub fn is_waiting_for_user_confirmation(content: &str) -> bool {
    let lower = content.to_lowercase();
    content.contains("请确认")
        || content.contains("等待确认")
        || content.contains("等待您的确认")
        || content.contains("请审阅确认")
        || content.contains("请您确认")
        || content.contains("需要您的裁决")
        || content.contains("请指示")
        || content.contains("请问是否")
        || content.contains("等待人工")
        || lower.contains("<user_confirmation_needed>")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_waiting_for_user_confirmation() {
        assert!(is_waiting_for_user_confirmation("关于这 14 个未提交的测试文件，请确认是否需要一并提交？"));
        assert!(is_waiting_for_user_confirmation("遇到核心阻断问题，等待人工裁决后再行继续。"));
        assert!(is_waiting_for_user_confirmation("请审阅确认以上计划后，我们将开始执行。"));
        assert!(!is_waiting_for_user_confirmation("已完成第一阶段代码重构，接下来开始编译自检。"));
    }
}
