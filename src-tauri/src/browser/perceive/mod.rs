//! 感知层（Perception Layer）
//!
//! 职责边界：
//! - 负责「把浏览器内部状态转成 LLM 可理解的结构化事实」；
//! - **只读**：本层允许向页面注入只读采集脚本，但**不得派发任何输入事件**
//!   （点击/填写/按键属于 `act` 层）；
//! - 不含动作决策，不知道 Agent 想做什么。
//!
//! 子模块规划：
//! - `model`     统一元素模型（ElementDescriptor / ElementState / Rect / Lod）— 已完成
//! - `dom`       DOM 通道采集器（穿透 shadow / frame，全量采集 + 分页游标）
//! - `a11y_name` accessible name 级联计算
//! - `health`    页面健康态判定（原 evaluator.rs）
//! - `lod`       LOD 分级渲染 + 视觉预算
//! - `ax`        Accessibility 第二通道（阶段 6）

pub mod model;

pub use model::{ElementDescriptor, ElementState, Lod, Obstruction, Rect};
