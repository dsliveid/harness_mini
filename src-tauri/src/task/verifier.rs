use std::path::Path;
use std::time::Duration;
use tokio::sync::Mutex;

/// 全局构建/门禁命令排他互斥锁
/// 当多个子任务并行执行时，防止它们在同一毫秒敲下 `cargo check` 或 `npm test` 造成本地构建目录 (.lock / target) 竞态死锁
static VERIFY_COMMAND_MUTEX: Mutex<()> = Mutex::const_new(());

/// 零 Git 污染改造：彻底废除在用户主工作区执行 git commit 脏提交。
/// 代码快照与时光机回滚全量移交 CAS 影子快照引擎接管。
pub fn create_git_snapshot(_workspace: &Path, _task_id: &str, _step_num: usize) -> Option<String> {
    None
}

/// 执行自动化质量门禁命令（检测 Exit Code 与捕获输出，带超时防挂死保护与跨平台兼容，自动走互斥队列）
pub async fn execute_verify_command(workspace: &Path, command_str: &str) -> (bool, String) {
    let _lock = VERIFY_COMMAND_MUTEX.lock().await;
    let timeout = Duration::from_secs(45);
    match crate::sop::run_verify_cmd(workspace, command_str, timeout).await {
        Ok((success, output)) => (success, output),
        Err(err) => (false, format!("自动化质量门禁执行失败或超时: {}", err)),
    }
}
