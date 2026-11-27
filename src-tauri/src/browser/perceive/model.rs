//! 感知层统一数据模型
//!
//! 本文件是**全项目唯一的「页面元素」表示**。所有上层（snapshot diff、LOD 渲染、
//! 动作策略、断言校验、证据包）一律消费本模型，不得再自行定义元素结构。
//!
//! 设计原则：
//! 1. **全量状态**：一次采集把浏览器能告诉我们的状态全部取回（disabled/checked/
//!    selected/expanded/focused/readonly/required），下游按需裁剪，而非采集端裁剪。
//!    历史教训：旧 `InteractiveElement` 只有 9 个字段，Agent 看到 8 个
//!    `placeholder="请输入"` 的输入框无法区分，只能截图。
//! 2. **序列化即省 token**：`ElementState` 用位标记 + 自定义序列化，只输出置位项，
//!    因此新增状态不会线性推高 token。
//! 3. **可解释的失败**：`rect` / `obscured_by` 直接支撑「点击被遮挡」的归因，
//!    避免 Agent 靠截图自查。

use serde::{Deserialize, Serialize};

/// 元素在视口中的矩形（CSS 像素，已扣除滚动偏移）
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        Self { x, y, w, h }
    }

    /// 尺寸是否有效（宽高均大于 0）
    pub fn has_area(&self) -> bool {
        self.w > 0.0 && self.h > 0.0
    }

    /// 中心点
    pub fn center(&self) -> (f64, f64) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }
}

/// 元素状态位标记
///
/// 用 `u16` 位域而非一堆 `Option<bool>`，序列化时**只输出置位的状态名**，
/// 未置位的一律省略，从而做到「状态越加越多，token 不涨」。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ElementState(u16);

impl std::ops::BitOr for ElementState {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

impl ElementState {
    pub const NONE: Self = Self(0);
    pub const DISABLED: Self = Self(1 << 0);
    pub const CHECKED: Self = Self(1 << 1);
    pub const SELECTED: Self = Self(1 << 2);
    pub const EXPANDED: Self = Self(1 << 3);
    pub const FOCUSED: Self = Self(1 << 4);
    pub const READONLY: Self = Self(1 << 5);
    pub const REQUIRED: Self = Self(1 << 6);
    pub const INVALID: Self = Self(1 << 7);
    /// 元素被其他元素遮挡（命中检测未通过）
    pub const OBSCURED: Self = Self(1 << 8);
    /// 元素自身尺寸为零或 CSS 不可见
    pub const INVISIBLE: Self = Self(1 << 9);
    /// 位于当前视口之外（但仍在文档内，可滚动到达）
    pub const OFFSCREEN: Self = Self(1 << 10);

    pub fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    pub fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }

    pub fn remove(&mut self, other: Self) {
        self.0 &= !other.0;
    }

    pub fn is_empty(&self) -> bool {
        self.0 == 0
    }

    /// 状态名清单（用于序列化与 LOD 渲染）
    pub fn names(self) -> Vec<&'static str> {
        const TABLE: &[(u16, &str)] = &[
            (1 << 0, "disabled"),
            (1 << 1, "checked"),
            (1 << 2, "selected"),
            (1 << 3, "expanded"),
            (1 << 4, "focused"),
            (1 << 5, "readonly"),
            (1 << 6, "required"),
            (1 << 7, "invalid"),
            (1 << 8, "obscured"),
            (1 << 9, "invisible"),
            (1 << 10, "offscreen"),
        ];
        TABLE
            .iter()
            .filter(|(bit, _)| self.0 & bit != 0)
            .map(|(_, name)| *name)
            .collect()
    }
}

impl Serialize for ElementState {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        // 置位为空时输出空数组而非 null，避免下游 null 判断
        s.collect_seq(self.names())
    }
}

impl<'de> Deserialize<'de> for ElementState {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let names: Vec<String> = Vec::deserialize(d)?;
        let mut st = Self::NONE;
        for n in names {
            let bit = match n.as_str() {
                "disabled" => Self::DISABLED,
                "checked" => Self::CHECKED,
                "selected" => Self::SELECTED,
                "expanded" => Self::EXPANDED,
                "focused" => Self::FOCUSED,
                "readonly" => Self::READONLY,
                "required" => Self::REQUIRED,
                "invalid" => Self::INVALID,
                "obscured" => Self::OBSCURED,
                "invisible" => Self::INVISIBLE,
                "offscreen" => Self::OFFSCREEN,
                _ => continue,
            };
            st.insert(bit);
        }
        Ok(st)
    }
}

/// 遮挡信息：谁挡住了目标元素
///
/// 直接回答「为什么点不到」，替代旧实现里笼统的一句「元素被遮挡」。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Obstruction {
    /// 遮挡者的简短描述，如 `div.el-overlay` / `header.sticky-top`
    pub node: String,
    /// 遮挡者的 CSS position 计算值（sticky / fixed 最常见）
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub position: String,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub z_index: i32,
}

fn is_zero(v: &i32) -> bool {
    *v == 0
}

/// 页面元素统一描述
///
/// 这是感知层对外的**唯一**元素模型。字段取舍原则：
/// - 进：任何影响「Agent 能否正确决策」的事实（状态、可访问名、位置、归属）
/// - 出：纯实现细节（内部 CSS 路径、原始 class 串）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ElementDescriptor {
    /// 稳定引用 ID（形如 `@e1a2b3c4`），由特征指纹哈希生成，抗 DOM 波动
    pub ref_id: String,
    /// 语义角色：优先取 AX 树 role，DOM 通道下按 tag/aria 推断
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub role: String,
    /// 原始标签名（小写）
    pub tag: String,
    /// `<input>` 的 type（text/password/checkbox/radio/...），非 input 为空
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub input_type: String,
    /// ★ 可访问名称：label[for] → aria-labelledby → aria-label → placeholder → title → 文本
    ///
    /// 这是「8 个 placeholder=请输入 无法区分」问题的正面解法。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub accessible_name: String,
    /// 元素可见文本（截断）
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
    /// 当前值（input/textarea/select）
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub value: String,
    /// placeholder 原文（保留原始信息，供选择器匹配）
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub placeholder: String,
    /// name 属性
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// href（截断）
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub href: String,
    /// 状态位集合
    #[serde(default, skip_serializing_if = "ElementState::is_empty")]
    pub state: ElementState,
    /// 视口矩形
    pub rect: Rect,
    /// 被谁遮挡（仅当 OBSCURED 置位时有值）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub obscured_by: Option<Obstruction>,
    /// 是否位于可见弹窗/抽屉容器内
    #[serde(default, skip_serializing_if = "is_false")]
    pub in_dialog: bool,
    /// 所属 frame 标识（主文档为 None）；跨 iframe 采集时用于回派动作
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in_frame: Option<String>,
    /// 若元素位于表格内，给出 (行, 列)，从 1 开始
    ///
    /// 解决「N 行表格有 N 个相同『编辑』按钮，Agent 无法区分点哪一行」。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub table_pos: Option<(usize, usize)>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl ElementDescriptor {
    /// 是否可交互（用于 LOD 过滤）
    pub fn is_actionable(&self) -> bool {
        !self
            .state
            .contains(ElementState::DISABLED | ElementState::INVISIBLE)
    }

    /// 是否在当前视口内（供 LOD 分层：L0/L1 只报视口内）
    pub fn in_viewport(&self) -> bool {
        !self.state.contains(ElementState::OFFSCREEN)
    }
}

/// 感知输出详细度分级
///
/// 核心取舍：**采集端永远全量，输出端按 LOD 裁剪**。
/// 历史教训：旧实现把「视口内 + 100 条配额」写在采集端，导致准确率下降
/// 却仍需 Agent 反复取，token 一点没省。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lod {
    /// 摘要（默认）：变化 Top-N + 断言结果 + 告警，约 100~300 token
    #[default]
    L0,
    /// 结构：可交互元素清单 + 状态标记 + 可访问名，约 500~1500 token
    L1,
    /// 全量：完整清单 + 表格 + 层级，3000+ token
    L2,
}

impl Lod {
    /// 从字符串解析，未知值降级为 L0（保守，省 token）
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "l0" | "summary" | "brief" => Self::L0,
            "l1" | "structure" | "normal" => Self::L1,
            "l2" | "full" | "all" => Self::L2,
            _ => Self::L0,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::L0 => "l0",
            Self::L1 => "l1",
            Self::L2 => "l2",
        }
    }
}
