pub mod ingress_normalizer;
pub mod message_stream;
pub mod static_system;
pub mod tail_anchor;

pub use ingress_normalizer::normalize_tool_output_for_ingress;
pub use message_stream::{assemble_execution_context, ExecutionContext};
pub use static_system::{
    build_static_collaborator_section, build_static_project_section, build_static_system_prompt,
};
pub use tail_anchor::build_runtime_anchor;

use crate::models::TruncationNotice;
use rusqlite::Connection;
use serde_json::Value;

/// 上下文工程统一构建门面
pub fn build_execution_context(
    conn: &Connection,
    session_id: &str,
    system_prompt: &str,
    token_limit: usize,
) -> (Result<Vec<Value>, String>, usize, Option<TruncationNotice>) {
    assemble_execution_context(conn, session_id, system_prompt, token_limit)
}
