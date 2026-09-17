mod agent;
mod approval;
mod commands;
mod diffutil;
mod growth;
mod llm;
mod models;
mod paths;
mod secrets;
mod skills;
mod sop;
mod store;
mod temp;
mod tools;

use agent::SessionHandle;
use approval::Decision;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Mutex, OnceLock};
use tauri::Manager;
use tokio::sync::oneshot;

/// 等待用户审批的挂起请求
pub struct PendingApproval {
    pub session_id: String,
    pub tx: oneshot::Sender<Decision>,
}

/// 等待用户确认的上下文自动压缩挂起请求
pub struct PendingCompaction {
    pub session_id: String,
    pub tx: oneshot::Sender<crate::models::CompactionDecision>,
}

pub struct RunningCommand {
    pub session_id: String,
    pub pid: Option<u32>,
    pub tx: tokio::sync::oneshot::Sender<()>,
}

pub struct AppState {
    pub db: Mutex<rusqlite::Connection>,
    /// 生效的数据目录；None = 程序目录不可写，等待用户选择（DB 为内存哨兵库）。
    /// 切换数据目录时原地替换连接（热切换，进程不重启）。
    pub data_dir: Mutex<Option<PathBuf>>,
    pub data_pending: AtomicBool,
    /// 默认数据目录（程序同级 data\；用于设置页展示）
    pub default_data_dir: PathBuf,
    /// data_pending 时不可写目录的提示信息
    pub unwritable_path: Option<String>,
    /// 是否使用自定义数据目录（指针文件生效）
    pub is_custom_dir: AtomicBool,
    /// API Key 加密主密钥（data_pending 时为临时密钥，数据仅存内存、退出即失效；
    /// 切换数据目录时随库一起更换，锁序统一为 db → master）
    pub master_key: Mutex<[u8; 32]>,
    pub handles: Mutex<HashMap<String, SessionHandle>>,
    pub approvals: Mutex<HashMap<String, PendingApproval>>,
    pub compactions: Mutex<HashMap<String, PendingCompaction>>,
    pub running_commands: Mutex<HashMap<String, RunningCommand>>,
    pub app: OnceLock<tauri::AppHandle>,
}

impl AppState {
    pub fn emit(&self, event: &str, payload: &impl serde::Serialize) {
        if let Some(app) = self.app.get() {
            let _ = tauri::Emitter::emit(app, event, payload);
        }
    }
}

/// 显示并聚焦主窗口
fn show_main_window(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

fn mem_db() -> Result<rusqlite::Connection, String> {
    let conn = rusqlite::Connection::open_in_memory().map_err(|e| e.to_string())?;
    store::init_schema(&conn)?;
    Ok(conn)
}

/// 在确定可写的数据目录中完成旧库迁移、主密钥加载与数据库打开
fn init_real_db(dir: &Path, legacy_dir: Option<&Path>) -> Result<([u8; 32], rusqlite::Connection), String> {
    paths::migrate_legacy_db(dir, legacy_dir)?;
    let master = secrets::load_or_create_master_key(dir)?;
    let db = store::open_db(&dir.join("harness_mini.db"))?;
    // 旧版本 keyring 凭据一次性迁入加密 secrets 表
    let _ = store::migrate_secrets_from_keyring(&db, &master);
    // 启动时自适应校准临时空间路径至当前数据目录（支持程序/数据迁移后自动重定位）
    let _ = temp::rebase_temp_storage(&db, dir);
    Ok((master, db))
}

/// 解析数据目录并打开数据库：自定义目录（指针文件）优先，其次默认目录（程序同级 data）。
/// 目录不可写或初始化失败时进入待选择状态：DB 用内存哨兵库顶替，等待用户在界面中选择数据目录。
fn resolve_and_open(
    legacy_dir: Option<&Path>,
    custom_dir: Option<&Path>,
    default_dir: &Path,
) -> Result<(Option<PathBuf>, [u8; 32], rusqlite::Connection, bool, Option<String>), String> {
    let chosen = custom_dir
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| default_dir.to_path_buf());
    if paths::ensure_writable(&chosen) {
        match init_real_db(&chosen, legacy_dir) {
            Ok((master, db)) => return Ok((Some(chosen), master, db, false, None)),
            Err(e) => {
                return Ok((
                    None,
                    secrets::ephemeral_master_key(),
                    mem_db()?,
                    true,
                    Some(format!("{}（初始化失败：{e}）", chosen.display())),
                ))
            }
        }
    }
    Ok((
        None,
        secrets::ephemeral_master_key(),
        mem_db()?,
        true,
        Some(chosen.display().to_string()),
    ))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            use tauri::{
                menu::{Menu, MenuItem},
                tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
            };
            let handle = app.handle();
            let legacy_dir = app.path().app_data_dir().ok();
            let custom_dir = paths::read_pointer(handle);
            let default_dir = paths::default_data_dir()
                .unwrap_or_else(|| std::env::temp_dir().join("harness_mini_data"));
            let (data_dir, master_key, db, pending, unwritable_path) =
                resolve_and_open(legacy_dir.as_deref(), custom_dir.as_deref(), &default_dir)?;
            let state = AppState {
                db: Mutex::new(db),
                handles: Mutex::new(HashMap::new()),
                approvals: Mutex::new(HashMap::new()),
                compactions: Mutex::new(HashMap::new()),
                running_commands: Mutex::new(HashMap::new()),
                app: OnceLock::new(),
                data_dir: Mutex::new(data_dir),
                data_pending: AtomicBool::new(pending),
                default_data_dir: default_dir,
                unwritable_path,
                is_custom_dir: AtomicBool::new(custom_dir.is_some()),
                master_key: Mutex::new(master_key),
            };
            app.manage(state);
            let handle = app.handle().clone();
            let _ = app.state::<AppState>().app.set(handle);

            // 系统托盘：左键点击显示主窗口，右键弹出菜单（显示主窗口 / 退出）
            let show_item = MenuItem::with_id(app, "show", "显示主窗口", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
            let tray_menu = Menu::with_items(app, &[&show_item, &quit_item])?;
            TrayIconBuilder::with_id("main-tray")
                .icon(app.default_window_icon().expect("missing app icon").clone())
                .tooltip("harness_mini")
                .menu(&tray_menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => show_main_window(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        show_main_window(tray.app_handle());
                    }
                })
                .build(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            // 点击窗口关闭：默认隐藏到托盘继续运行，从托盘菜单“退出”才真正退出
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_settings,
            commands::set_settings,
            commands::list_tools,
            commands::test_provider,
            commands::list_projects,
            commands::create_project,
            commands::remove_project,
            commands::set_project_pinned,
            commands::set_project_constraints,
            commands::list_project_links,
            commands::add_project_link,
            commands::update_project_link,
            commands::delete_project_link,
            commands::list_sessions,
            commands::list_archived,
            commands::rename_session,
            commands::delete_session,
            commands::archive_session,
            commands::unarchive_session,
            commands::set_session_mode,
            commands::set_session_workspace,
            commands::set_session_project,
            commands::get_messages,
            commands::send_message,
            commands::list_queued,
            commands::guide_message,
            commands::delete_queued_message,
            commands::stop_run,
            commands::kill_command,
            commands::list_running_sessions,
            commands::edit_and_resend,
            commands::respond_approval,
            commands::respond_compaction,
            commands::list_session_compactions,
            commands::list_session_rules,
            commands::delete_session_rule,
            commands::get_session_todos,
            commands::alloc_temp_code,
            commands::get_temp_info,
            commands::list_temp_changes,
            commands::get_temp_change_diff,
            commands::merge_temp_space,
            commands::clear_temp_space,
            commands::open_dir,
            commands::get_data_status,
            commands::set_data_dir,
            commands::reset_data_dir,
            commands::exit_app,
            commands::list_growths,
            commands::update_growth_status,
            commands::update_growth_rule,
            commands::delete_growth,
            commands::trigger_growth_reflection,
            commands::list_project_skills,
            commands::save_project_skill,
            commands::delete_project_skill,
            commands::get_project_sop,
            commands::set_project_sop,
            commands::run_workspace_sop,
            commands::get_token_stats,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
