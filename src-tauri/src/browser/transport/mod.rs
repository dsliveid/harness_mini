//! 传输层（Transport Layer）
//!
//! 职责边界（严格单向依赖的最底层）：
//! - 只负责「与 Chromium 建立并维持通道」以及「收发 CDP 报文」；
//! - **不感知任何业务语义**：不知道什么是"登录""表单""下拉"，
//!   也不知道哪个工具在调用它；
//! - 不含任何 DOM 采集/注入脚本逻辑（那属于 `perceive` 层）；
//! - 不含任何动作策略（那属于 `act` 层）。
//!
//! 上层（`perceive` / `act` / `verify` / `orchestrator`）依赖本层，
//! 本层**不得反向依赖**任何上层。

pub mod cdp;
pub mod daemon;

pub use cdp::CdpClient;
pub use daemon::{global_browser_daemon, BrowserDaemon};
