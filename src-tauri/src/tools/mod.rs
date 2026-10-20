pub mod command;
pub mod file;
pub mod helpers;
pub mod registry;
pub mod search;
pub mod skills_memory_plan;
pub mod specs;
pub mod subagent;
pub mod temp;
#[cfg(test)]
pub mod tests;
pub mod types;
pub mod web;

// Re-export public types and functions
pub use command::{assign_pid_to_job, kill_process_tree};
pub use file::record_file_snapshot_raw;
pub use helpers::{inside_workspace, is_high_danger};
pub use registry::ToolRegistry;
pub use specs::tool_specs;
pub use types::{HostCtx, PartialCb, Risk, Tool, ToolCtx, ToolSpec, SUBAGENT_TOOL_NAMES, TEMP_TOOL_NAMES};

use serde_json::Value;
use std::sync::OnceLock;

static GLOBAL_REGISTRY: OnceLock<ToolRegistry> = OnceLock::new();

pub fn global_registry() -> &'static ToolRegistry {
    GLOBAL_REGISTRY.get_or_init(ToolRegistry::new)
}

pub async fn execute(
    name: &str,
    args: &Value,
    ctx: &ToolCtx,
    on_partial: PartialCb<'_>,
) -> Result<String, String> {
    global_registry().execute(name, args, ctx, on_partial).await
}

pub(crate) async fn dispatch_static(
    name: &str,
    args: &Value,
    ctx: &ToolCtx,
    on_partial: PartialCb<'_>,
) -> Result<String, String> {
    match name {
        "read_file" => file::read_file(args, ctx).await,
        "list_dir" => file::list_dir(args, ctx).await,
        "file_outline" => file::file_outline(args, ctx).await,
        "glob" => search::glob(args, ctx).await,
        "grep" => search::grep(args, ctx).await,
        "write_file" => file::write_file(args, ctx).await,
        "edit_file" => file::edit_file(args, ctx).await,
        "run_command" => command::run_command(args, ctx, on_partial).await,
        "fetch_web_page" => web::fetch_web_page(args, ctx).await,
        "browser_navigate" => crate::browser::browser_navigate(args, ctx).await,
        "browser_interact" => crate::browser::browser_interact(args, ctx).await,
        "browser_get_debug_state" => crate::browser::browser_get_debug_state(ctx).await,
        "browser_screenshot" => crate::browser::browser_screenshot(args, ctx).await,
        "browser_scroll" => crate::browser::browser_scroll(args, ctx).await,
        "browser_close" => crate::browser::browser_close(ctx).await,
        "todo" => Ok("ok".to_string()),
        "list_skills" => skills_memory_plan::list_skills_tool(ctx).await,
        "save_skill" => skills_memory_plan::save_skill_tool(args, ctx).await,
        "run_skill" => skills_memory_plan::run_skill_tool(args, ctx, on_partial).await,
        "record_memory" => skills_memory_plan::record_memory_tool(args, ctx).await,
        "read_memory" => skills_memory_plan::read_memory_tool(args, ctx).await,
        "create_plan" => skills_memory_plan::create_plan_tool(args, ctx).await,
        "update_plan" => skills_memory_plan::update_plan_tool(args, ctx).await,
        "switch_plan" => skills_memory_plan::switch_plan_tool(args, ctx).await,
        "read_plan" => skills_memory_plan::read_plan_tool(args, ctx).await,
        "list_plans" => skills_memory_plan::list_plans_tool(args, ctx).await,
        "spawn_subagent" | "spawn_subprocess" => subagent::spawn_subagent_tool(args, ctx).await,
        "get_subagent_status" => subagent::get_subagent_status_tool(args, ctx).await,
        "wait_subagents" | "wait_subprocesses" => subagent::wait_subagents_tool(args, ctx).await,
        "stop_subagent" | "stop_subprocess" => subagent::stop_subagent_tool(args, ctx).await,
        "resume_subagent" | "resume_subprocess" => subagent::resume_subagent_tool(args, ctx).await,
        "dispatch_collaborator" => subagent::dispatch_collaborator_tool(args, ctx).await,
        "wait_collaborators" => subagent::wait_collaborators_tool(args, ctx).await,
        "get_collaborators" => subagent::get_collaborators_tool(ctx).await,
        "generate_image" => skills_memory_plan::generate_image_tool(args, ctx).await,
        "recognize_image" => skills_memory_plan::recognize_image_tool(args, ctx).await,
        "judge_decision" => skills_memory_plan::judge_decision_tool(args, ctx).await,
        "choice_decision" => skills_memory_plan::choice_decision_tool(args, ctx).await,
        "score_decision" => skills_memory_plan::score_decision_tool(args, ctx).await,
        "temp_status" => temp::temp_status(ctx).await,
        "temp_changes" => temp::temp_changes(args, ctx).await,
        "temp_diff" => temp::temp_diff(args, ctx).await,
        "temp_snapshot" => temp::temp_snapshot(args, ctx).await,
        "temp_restore" => temp::temp_restore(args, ctx).await,
        "temp_merge" => temp::temp_merge(ctx).await,
        _ => Err(format!("未知工具: {name}")),
    }
}
