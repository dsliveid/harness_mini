use tauri::Manager;
use super::helpers::*;
use super::types::*;
use crate::models::truncate_result;
use serde_json::Value;
use std::path::Path;
use tokio::io::AsyncBufReadExt;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
pub(crate) const CREATE_NO_WINDOW: u32 = 0x08000000;

/// 强制杀死指定进程及其所有子进程树（跨平台安全强杀）
pub fn kill_process_tree(pid: u32) {
    #[cfg(windows)]
    {
        let taskkill_bin = std::env::var("SystemRoot")
            .map(|r| format!("{r}\\System32\\taskkill.exe"))
            .unwrap_or_else(|_| "taskkill".into());
        let mut kill_cmd = std::process::Command::new(taskkill_bin);
        kill_cmd.args(["/F", "/T", "/PID", &pid.to_string()]);
        kill_cmd.creation_flags(CREATE_NO_WINDOW);
        let _ = kill_cmd.output();
    }
    #[cfg(not(windows))]
    {
        unsafe {
            libc::kill(-(pid as i32), libc::SIGKILL);
            libc::kill(pid as i32, libc::SIGKILL);
        }
    }
}

/// 将子进程挂载到系统级 Job Object，确保主进程意外退出或崩溃时内核级联强杀孤儿进程
#[cfg(windows)]
pub fn assign_pid_to_job(pid: u32) {
    use std::sync::OnceLock;
    type HANDLE = *mut std::ffi::c_void;
    type BOOL = i32;
    type DWORD = u32;

    #[repr(C)]
    struct IO_COUNTERS {
        read_operation_count: u64,
        write_operation_count: u64,
        other_operation_count: u64,
        read_transfer_count: u64,
        write_transfer_count: u64,
        other_transfer_count: u64,
    }

    #[repr(C)]
    struct JOBOBJECT_BASIC_LIMIT_INFORMATION {
        per_process_user_time_limit: i64,
        per_job_user_time_limit: i64,
        limit_flags: DWORD,
        minimum_working_set_size: usize,
        maximum_working_set_size: usize,
        active_process_limit: DWORD,
        affinity: usize,
        priority_class: DWORD,
        scheduling_class: DWORD,
    }

    #[repr(C)]
    struct JOBOBJECT_EXTENDED_LIMIT_INFORMATION {
        basic_limit_information: JOBOBJECT_BASIC_LIMIT_INFORMATION,
        io_info: IO_COUNTERS,
        process_memory_limit: usize,
        job_memory_limit: usize,
        peak_process_memory_limit: usize,
        peak_job_memory_limit: usize,
    }

    extern "system" {
        fn CreateJobObjectW(lpJobAttributes: *mut std::ffi::c_void, lpName: *const u16) -> HANDLE;
        fn SetInformationJobObject(
            hJob: HANDLE,
            JobObjectInformationClass: i32,
            lpJobObjectInformation: *const std::ffi::c_void,
            cbJobObjectInformationLength: DWORD,
        ) -> BOOL;
        fn OpenProcess(dwDesiredAccess: DWORD, bInheritHandle: BOOL, dwProcessId: DWORD) -> HANDLE;
        fn AssignProcessToJobObject(hJob: HANDLE, hProcess: HANDLE) -> BOOL;
        fn CloseHandle(hObject: HANDLE) -> BOOL;
    }

    const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: DWORD = 0x2000;
    const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION_CLASS: i32 = 9;
    const PROCESS_SET_QUOTA: DWORD = 0x0100;
    const PROCESS_TERMINATE: DWORD = 0x0001;

    static GLOBAL_JOB: OnceLock<usize> = OnceLock::new();
    let job_handle = *GLOBAL_JOB.get_or_init(|| unsafe {
        let job = CreateJobObjectW(std::ptr::null_mut(), std::ptr::null());
        if !job.is_null() {
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.basic_limit_information.limit_flags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let _ = SetInformationJobObject(
                job,
                JOB_OBJECT_EXTENDED_LIMIT_INFORMATION_CLASS,
                &info as *const _ as *const std::ffi::c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as DWORD,
            );
        }
        job as usize
    });

    if job_handle != 0 {
        unsafe {
            let h_proc = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, pid);
            if !h_proc.is_null() {
                let _ = AssignProcessToJobObject(job_handle as HANDLE, h_proc);
                let _ = CloseHandle(h_proc);
            }
        }
    }
}

/// 进程树生命周期守卫：协程被 abort / drop 时兜底强杀进程树
struct ProcessTreeGuard {
    pid: Option<u32>,
    active: bool,
}

impl ProcessTreeGuard {
    fn new(pid: Option<u32>) -> Self {
        Self { pid, active: true }
    }
    fn defuse(&mut self) {
        self.active = false;
    }
}

impl Drop for ProcessTreeGuard {
    fn drop(&mut self) {
        if self.active {
            if let Some(pid) = self.pid {
                kill_process_tree(pid);
            }
        }
    }
}

pub(crate) async fn run_command(
    args: &Value,
    ctx: &ToolCtx,
    on_partial: PartialCb<'_>,
) -> Result<String, String> {
    let command = args.get("command").and_then(|v| v.as_str()).ok_or("缺少 command")?;
    let cwd = args
        .get("cwd")
        .and_then(|v| v.as_str())
        .map(|c| resolve(ctx, c))
        .unwrap_or_else(|| ctx.workspace.clone());

    #[cfg(windows)]
    let mut cmd = {
        let mut c = tokio::process::Command::new("powershell");
        c.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command"]);
        c.arg(format!(
            "$OutputEncoding = [Console]::OutputEncoding = [System.Text.Encoding]::UTF8;\n\
             $PSDefaultParameterValues['Out-File:Encoding'] = 'utf8';\n\
             $PSDefaultParameterValues['*:Encoding'] = 'utf8';\n\
             {command}\n\
             if ($LASTEXITCODE -ne $null) {{ exit $LASTEXITCODE }}"
        ));
        c.creation_flags(CREATE_NO_WINDOW);
        c
    };
    #[cfg(not(windows))]
    let mut cmd = {
        let mut c = tokio::process::Command::new("sh");
        c.arg("-c").arg(command);
        c
    };
    if let Some(ref proxy) = ctx.proxy_url {
        let norm = crate::llm::normalize_proxy_url(proxy);
        if !norm.is_empty() {
            cmd.env("HTTP_PROXY", &norm);
            cmd.env("HTTPS_PROXY", &norm);
            cmd.env("ALL_PROXY", &norm);
            cmd.env("http_proxy", &norm);
            cmd.env("https_proxy", &norm);
            cmd.env("all_proxy", &norm);
            cmd.env("NO_PROXY", "localhost,127.0.0.1,::1");
            cmd.env("no_proxy", "localhost,127.0.0.1,::1");
        }
    }
    cmd.current_dir(&cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);

    let mut child = cmd.spawn().map_err(|e| format!("启动命令失败: {e}"))?;
    let pid = child.id();
    #[cfg(windows)]
    if let Some(p) = pid {
        assign_pid_to_job(p);
    }
    let mut tree_guard = ProcessTreeGuard::new(pid);
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();

    let (kill_tx, mut kill_rx) = tokio::sync::oneshot::channel::<()>();
    let event_id_opt = ctx.event_id.clone();
    if let (Some(host), Some(eid)) = (&ctx.host, &event_id_opt) {
        let state = host.app.state::<crate::AppState>();
        state.running_commands.lock().unwrap().insert(
            eid.clone(),
            crate::RunningCommand {
                session_id: host.session_id.clone(),
                pid,
                tx: kill_tx,
            },
        );
    }

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let tx_err = tx.clone();
    tokio::spawn(async move {
        pipe_lines(stdout, tx).await;
    });
    tokio::spawn(async move {
        pipe_lines(stderr, tx_err).await;
    });

    let mut output = String::new();
    let deadline = tokio::time::Instant::now() + ctx.command_timeout;
    let mut timed_out = false;
    let mut killed_by_user = false;
    loop {
        tokio::select! {
            _ = &mut kill_rx => {
                killed_by_user = true;
                if let Some(p) = pid {
                    kill_process_tree(p);
                }
                let _ = child.start_kill();
                break;
            }
            line = rx.recv() => {
                match line {
                    Some(l) => {
                        use std::fmt::Write;
                        let _ = writeln!(output, "{l}");
                        if output.len() < 64 * 1024 {
                            on_partial(&l);
                        }
                        if output.len() > 512 * 1024 {
                            output.push_str("\n[输出过长，已停止收集]");
                            let _ = child.start_kill();
                            break;
                        }
                    }
                    None => break,
                }
            }
            _ = tokio::time::sleep_until(deadline) => {
                timed_out = true;
                let _ = child.start_kill();
                break;
            }
        }
    }

    if let (Some(host), Some(eid)) = (&ctx.host, &event_id_opt) {
        let state = host.app.state::<crate::AppState>();
        state.running_commands.lock().unwrap().remove(eid);
    }

    let wait_res = tokio::time::timeout(std::time::Duration::from_secs(3), child.wait()).await;
    let status = match wait_res {
        Ok(s) => s,
        Err(_) => {
            let _ = child.start_kill();
            Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "等待子进程退出超时"))
        }
    };
    tree_guard.defuse();
    if killed_by_user {
        return Err(format!("{output}\n[控制台进程已由用户手动关闭]"));
    }
    if timed_out {
        return Ok(truncate_result(&format!(
            "{output}\n[命令超时（{}s），已强制终止]",
            ctx.command_timeout.as_secs()
        )));
    }
    let code = match status {
        Ok(s) => s.code().unwrap_or(-1),
        Err(_) => -1,
    };
    Ok(truncate_result(&format!("{output}\n[exit code: {code}]")))
}
