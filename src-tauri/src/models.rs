use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCfg {
    pub id: String,
    pub name: String,
    pub base_url: String,
    /// 该厂商下配置的模型列表（同一厂商可配多个模型）
    #[serde(default)]
    pub models: Vec<String>,
    /// 旧版单模型字段，仅用于读取旧配置并迁移到 models
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub api_key: String,
}

fn default_access_mode() -> String {
    "confirm".into()
}

/// 规范化访问模式取值：仅接受 confirm / full_access，其余（含空串、历史 NULL）一律按 confirm。
/// 访问模式为会话级、无“跟随全局”状态，因此运行时与落库始终是具体值。
pub fn normalize_access_mode(mode: &str) -> String {
    if mode == "full_access" {
        "full_access".into()
    } else {
        "confirm".into()
    }
}
fn default_max_steps() -> u32 {
    30
}
fn default_task_subtask_max_steps() -> u32 {
    30
}
fn default_cmd_timeout() -> u64 {
    120
}
fn default_ctx_tokens() -> usize {
    64000
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ToolInfo {
    pub name: String,
    pub description: String,
    pub risk: String,
    pub is_temp: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SettingsData {
    #[serde(default)]
    pub providers: Vec<ProviderCfg>,
    #[serde(default)]
    pub active_provider_id: Option<String>,
    /// 全局激活的模型（属于 active_provider_id 指向的厂商），缺省时取该厂商第一个模型
    #[serde(default)]
    pub active_model_id: Option<String>,
    #[serde(default)]
    pub active_model: Option<String>,
    #[serde(default = "default_access_mode")]
    pub global_access_mode: String,
    #[serde(default = "default_max_steps")]
    pub max_steps: u32,
    /// 长任务单子任务轮次预算硬上限（默认 30 轮）
    #[serde(default = "default_task_subtask_max_steps")]
    pub task_subtask_max_steps: u32,
    #[serde(default = "default_cmd_timeout")]
    pub command_timeout_secs: u64,
    #[serde(default = "default_ctx_tokens")]
    pub context_token_limit: usize,
    #[serde(default)]
    pub model_context_limits: std::collections::HashMap<String, usize>,
    /// 各模型的能力映射：key 为 "provider_id:model_name" 或 "model_name"，value 为能力列表如 ["chat", "image_gen", "vision"]
    #[serde(default)]
    pub model_capabilities: std::collections::HashMap<String, Vec<String>>,
    /// 全局默认生图模型配置
    #[serde(default)]
    pub active_image_provider_id: Option<String>,
    #[serde(default)]
    pub active_image_model_id: Option<String>,
    /// 全局默认视觉感知模型配置
    #[serde(default)]
    pub active_vision_provider_id: Option<String>,
    #[serde(default)]
    pub active_vision_model_id: Option<String>,
    #[serde(default)]
    pub last_workspace_path: Option<String>,
    #[serde(default)]
    pub disabled_tools: Vec<String>,
    #[serde(default)]
    pub disabled_sops: Vec<String>,
    /// 全局默认思考程度（"low" | "medium" | "high" | None / "default" 不传）
    #[serde(default)]
    pub reasoning_effort: Option<String>,
}

impl Default for SettingsData {
    fn default() -> Self {
        Self {
            providers: vec![],
            active_provider_id: None,
            active_model_id: None,
            active_model: None,
            global_access_mode: default_access_mode(),
            max_steps: default_max_steps(),
            task_subtask_max_steps: default_task_subtask_max_steps(),
            command_timeout_secs: default_cmd_timeout(),
            context_token_limit: default_ctx_tokens(),
            model_context_limits: std::collections::HashMap::new(),
            model_capabilities: std::collections::HashMap::new(),
            active_image_provider_id: None,
            active_image_model_id: None,
            active_vision_provider_id: None,
            active_vision_model_id: None,
            last_workspace_path: None,
            disabled_tools: vec![],
            disabled_sops: vec![],
            reasoning_effort: None,
        }
    }
}

/// 根据模型名称启发式推断预设上下文上限
pub fn infer_model_context_limit(model: &str) -> usize {
    let lower = model.to_lowercase();
    // 优先检查显式上下文规格标识（避免被厂商名提前拦截，例如 deepseek-64k 或 deepseek-1m）
    if lower.contains("2m") || lower.contains("2000k") || lower.contains("200w") {
        1_600_000
    } else if lower.contains("1m") || lower.contains("1000k") || lower.contains("100w") {
        800_000
    } else if lower.contains("500k") {
        400_000
    } else if lower.contains("200k") {
        180_000
    } else if lower.contains("128k") {
        110_000
    } else if lower.contains("64k") {
        56_000
    } else if lower.contains("32k") {
        28_000
    } else if lower.contains("16k") {
        14_000
    } else if lower.contains("8k") {
        7_000
    // 海外阵营旗舰 1M 窗口
    } else if lower.contains("gpt-5") || lower.contains("gpt5") {
        800_000
    } else if lower.contains("opus-5") || lower.contains("claude-5") || lower.contains("claude-opus-5") {
        800_000
    } else if lower.contains("gemini") {
        // Gemini 系列（如 Gemini 1.5 / 2.0 / 3.8 Flash 等）支持 1M - 2M 窗口
        800_000
    } else if lower.contains("deepseek") {
        // DeepSeek 原生支持 1M 窗口（如 DeepSeek-v4-flash、deepseek-chat 等）
        800_000
    } else if lower.contains("qwen-3") || lower.contains("qwen3") {
        // Qwen3 / Qwen3.8 系列（如 Qwen3.8-flash）原生 1M 窗口
        800_000
    } else if lower.contains("glm-5") || lower.contains("glm5") {
        // GLM-5 / GLM-5.3 系列（如 GLM-5.3-flash）原生 1M 窗口
        800_000
    } else if lower.contains("claude") {
        180_000
    } else if lower.contains("o1") || lower.contains("o3") {
        180_000
    } else if lower.contains("gpt-4o") || lower.contains("gpt-4.5") {
        110_000
    } else if lower.contains("qwen") || lower.contains("千问") {
        if lower.contains("long") {
            800_000
        } else {
            110_000
        }
    } else if lower.contains("glm") {
        if lower.contains("long") {
            800_000
        } else {
            110_000
        }
    } else if lower.contains("kimi") || lower.contains("moonshot") {
        if lower.contains("long") {
            800_000
        } else {
            180_000
        }
    } else if lower.contains("llama-3.1") || lower.contains("llama-3.2") || lower.contains("llama-3.3") || lower.contains("mistral-large") || lower.contains("codestral") {
        110_000
    } else {
        64_000
    }
}

impl SettingsData {
    /// 旧版配置迁移：models 为空且 model 非空时，把单模型迁移进 models
    pub fn migrate_legacy_model(&mut self) {
        for p in self.providers.iter_mut() {
            if p.models.is_empty() && !p.model.is_empty() {
                p.models = vec![p.model.clone()];
            }
        }
    }

    /// 统一 active_model_id 与 active_model 两个字段，双向兼容
    pub fn normalize(&mut self) {
        self.migrate_legacy_model();
        let m = self.active_model_id.clone().or_else(|| self.active_model.clone());
        self.active_model_id = m.clone();
        self.active_model = m;
    }

    /// 解析指定模型的生效上下文上限（三级回落）：
    /// 1. provider_id:model
    /// 2. model
    /// 3. infer_model_context_limit
    /// 4. self.context_token_limit
    pub fn resolve_context_limit(&self, provider_id: Option<&str>, model: &str) -> usize {
        if let Some(pid) = provider_id {
            let key = format!("{}:{}", pid, model);
            if let Some(&lim) = self.model_context_limits.get(&key) {
                return lim;
            }
        }
        if let Some(&lim) = self.model_context_limits.get(model) {
            return lim;
        }
        let inferred = infer_model_context_limit(model);
        if inferred != 64_000 {
            return inferred;
        }
        self.context_token_limit
    }

    /// 获取指定模型的能力列表（优先用户显式配置，缺省时启发式推断）
    pub fn resolve_model_capabilities(&self, provider_id: Option<&str>, model: &str) -> Vec<String> {
        if let Some(pid) = provider_id {
            let key = format!("{}:{}", pid, model);
            if let Some(caps) = self.model_capabilities.get(&key) {
                return caps.clone();
            }
        }
        if let Some(caps) = self.model_capabilities.get(model) {
            return caps.clone();
        }
        infer_default_capabilities(model)
    }

    /// 检查指定模型是否具备某项能力（如 "chat", "image_gen", "vision"）
    pub fn has_capability(&self, provider_id: Option<&str>, model: &str, cap: &str) -> bool {
        let caps = self.resolve_model_capabilities(provider_id, model);
        caps.iter().any(|c| c == cap)
    }
}

/// 根据模型名称推断默认能力集合（未显式配置时的平滑初始推断）
pub fn infer_default_capabilities(model: &str) -> Vec<String> {
    let lower = model.to_lowercase();
    if lower.contains("embedding") || lower.contains("rerank") {
        return vec![];
    }
    let is_img = lower.contains("seedream")
        || lower.contains("seedance")
        || lower.contains("seed-edit")
        || lower.contains("cogview")
        || lower.contains("dall-e")
        || lower.contains("dalle")
        || lower.contains("flux")
        || lower.contains("sdxl")
        || lower.contains("stable-diffusion")
        || lower.contains("wanx")
        || lower.contains("kolors")
        || lower.contains("t2i")
        || lower.contains("imagen")
        || lower.contains("image-gen")
        || lower.contains("image_gen");
    if is_img {
        return vec!["image_gen".to_string()];
    }
    let is_vis = lower.contains("vl")
        || lower.contains("vision")
        || lower.contains("4v")
        || lower.contains("omni")
        || lower.contains("gpt-4o")
        || lower.contains("claude-3")
        || lower.contains("gemini");
    if is_vis {
        return vec!["chat".to_string(), "vision".to_string()];
    }
    vec!["chat".to_string()]
}

/// 预设角色的默认调度触发规则（主进程 System Prompt 注入使用）
pub fn default_dispatch_rule_for_role(role: &str) -> &'static str {
    match role {
        "image_gen" => "当用户提出画图、生成图片、插图、海报、Logo、图标、配图制作等视觉生成需求时，必须优先委派本协作者。",
        "frontend" => "当涉及 UI 界面设计、页面实现、组件重构、Vue/React 模板与 CSS 交互开发时，必须优先委派本协作者。",
        "backend" => "当涉及服务端业务逻辑、API 接口、数据库 CRUD、后台架构开发时，必须优先委派本协作者。",
        "pm" => "当涉及需求分析梳理、PRD 方案编写、功能边界与业务流程设计时，必须优先委派本协作者。",
        "pmo" => "当涉及任务拆解(WBS)、里程碑节点排期、进度与风险追踪时，必须优先委派本协作者。",
        "vision" => "当用户发送图片、截图、设计稿并要求视觉识别分析时，必须优先委派本协作者。",
        "testing" => "当需要编写自动化测试用例、单元测试、执行回归测试与缺陷验证时，必须优先委派本协作者。",
        "review" => "当需要对代码实现进行质量审查、重构优化与架构防劣化时，必须优先委派本协作者。",
        "fullstack" => "当涉及端到端打通前后端完整功能链路开发时，必须优先委派本协作者。",
        _ => "当用户任务属于本协作者专业领域范围时，必须优先委派本协作者处理。",
    }
}

/// 解析全局生效的 (生图厂商, 生图模型)：优先全局激活项，若无则在已配置厂商中寻找具备 image_gen 能力的模型
pub fn resolve_active_image_model(settings: &SettingsData) -> Option<(&ProviderCfg, &str)> {
    if let (Some(pid), Some(mid)) = (&settings.active_image_provider_id, &settings.active_image_model_id) {
        if let Some(p) = settings.providers.iter().find(|p| &p.id == pid && p.models.iter().any(|m| m == mid)) {
            return Some((p, mid.as_str()));
        }
    }
    // 回落：查找任意配置了 image_gen 能力的模型
    for p in &settings.providers {
        for m in &p.models {
            if settings.has_capability(Some(&p.id), m, "image_gen") {
                return Some((p, m.as_str()));
            }
        }
    }
    None
}

/// 解析全局生效的 (视觉厂商, 视觉模型)：优先全局激活项，若无则在已配置厂商中寻找具备 vision 能力的模型
pub fn resolve_active_vision_model(settings: &SettingsData) -> Option<(&ProviderCfg, &str)> {
    if let (Some(pid), Some(mid)) = (&settings.active_vision_provider_id, &settings.active_vision_model_id) {
        if let Some(p) = settings.providers.iter().find(|p| &p.id == pid && p.models.iter().any(|m| m == mid)) {
            return Some((p, mid.as_str()));
        }
    }
    // 回落：查找任意配置了 vision 能力的模型
    for p in &settings.providers {
        for m in &p.models {
            if settings.has_capability(Some(&p.id), m, "vision") {
                return Some((p, m.as_str()));
            }
        }
    }
    None
}


/// 解析当前生效的 (厂商, 模型)：优先全局激活项，激活项缺失/失效时回落到第一个有模型的厂商
pub fn resolve_active_model(settings: &SettingsData) -> Option<(&ProviderCfg, &str)> {
    let provider = settings
        .providers
        .iter()
        .find(|p| Some(p.id.as_str()) == settings.active_provider_id.as_deref() && !p.models.is_empty())
        .or_else(|| settings.providers.iter().find(|p| !p.models.is_empty()))?;
    let active = settings
        .active_model_id
        .as_deref()
        .or(settings.active_model.as_deref());
    let model = active
        .filter(|m| provider.models.iter().any(|x| x == m))
        .or_else(|| provider.models.first().map(|x| x.as_str()))?;
    Some((provider, model))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(id: &str, name: &str, models: &[&str]) -> ProviderCfg {
        ProviderCfg {
            id: id.into(),
            name: name.into(),
            base_url: "https://example.com".into(),
            models: models.iter().map(|m| m.to_string()).collect(),
            model: String::new(),
            api_key: String::new(),
        }
    }

    #[test]
    fn legacy_single_model_migrates_into_models() {
        let mut s = SettingsData::default();
        s.providers = vec![ProviderCfg {
            id: "p1".into(),
            name: "智谱".into(),
            base_url: "https://example.com".into(),
            models: vec![],
            model: "glm-4.6".into(),
            api_key: String::new(),
        }];
        s.migrate_legacy_model();
        assert_eq!(s.providers[0].models, vec!["glm-4.6".to_string()]);
        let (p, m) = resolve_active_model(&s).unwrap();
        assert_eq!(p.id, "p1");
        assert_eq!(m, "glm-4.6");
    }

    #[test]
    fn resolves_active_provider_and_model() {
        let mut s = SettingsData::default();
        s.providers = vec![
            provider("a", "厂商A", &["model-a1", "model-a2"]),
            provider("b", "厂商B", &["model-b1"]),
        ];
        s.active_provider_id = Some("b".into());
        s.active_model = Some("model-b1".into());
        let (p, m) = resolve_active_model(&s).unwrap();
        assert_eq!(p.id, "b");
        assert_eq!(m, "model-b1");

        // active_model 不属于当前厂商时回落到第一个模型
        s.active_model = Some("model-a1".into());
        let (p, m) = resolve_active_model(&s).unwrap();
        assert_eq!(p.id, "b");
        assert_eq!(m, "model-b1");

        // 激活厂商没有模型时回落到第一个有模型的厂商
        s.providers = vec![
            provider("c", "空厂商", &[]),
            provider("b", "厂商B", &["model-b1"]),
        ];
        s.active_provider_id = Some("c".into());
        s.active_model = None;
        let (p, m) = resolve_active_model(&s).unwrap();
        assert_eq!(p.id, "b");
        assert_eq!(m, "model-b1");
    }

    #[test]
    fn no_models_yields_none() {
        let mut s = SettingsData::default();
        s.providers = vec![provider("c", "空厂商", &[])];
        assert!(resolve_active_model(&s).is_none());
        s.providers = vec![];
        assert!(resolve_active_model(&s).is_none());
    }

    #[test]
    fn resolves_active_model_id_and_normalizes() {
        let json = r#"{
            "providers": [
                {"id": "p1", "name": "厂商1", "baseUrl": "https://example.com", "models": ["m1", "m2"], "model": "", "apiKey": ""}
            ],
            "activeProviderId": "p1",
            "activeModelId": "m2"
        }"#;
        let mut s: SettingsData = serde_json::from_str(json).unwrap();
        assert_eq!(s.active_model_id.as_deref(), Some("m2"));
        let (_p, m) = resolve_active_model(&s).unwrap();
        assert_eq!(m, "m2");

        s.normalize();
        assert_eq!(s.active_model.as_deref(), Some("m2"));
        assert_eq!(s.active_model_id.as_deref(), Some("m2"));

        let serialized = serde_json::to_string(&s).unwrap();
        assert!(serialized.contains(r#""activeModelId":"m2""#));
        assert!(serialized.contains(r#""activeModel":"m2""#));
    }

    #[test]
    fn resolves_model_context_limit_with_fallbacks() {
        let mut s = SettingsData::default();
        s.context_token_limit = 64_000;

        // 1. 智能推断测试 - 旗舰 1M 阵营
        assert_eq!(s.resolve_context_limit(None, "gpt-5.5"), 800_000);
        assert_eq!(s.resolve_context_limit(None, "claude-opus-5"), 800_000);
        assert_eq!(s.resolve_context_limit(None, "gemini-3.8-flash"), 800_000);
        assert_eq!(s.resolve_context_limit(None, "deepseek-v4-flash"), 800_000);
        assert_eq!(s.resolve_context_limit(None, "qwen3.8-flash"), 800_000);
        assert_eq!(s.resolve_context_limit(None, "glm-5.3-flash"), 800_000);

        // 智能推断测试 - 经典/显式规格与其他模型
        assert_eq!(s.resolve_context_limit(None, "deepseek-chat"), 800_000);
        assert_eq!(s.resolve_context_limit(None, "deepseek-64k"), 56_000);
        assert_eq!(s.resolve_context_limit(None, "deepseek-128k"), 110_000);
        assert_eq!(s.resolve_context_limit(None, "deepseek-v3-1m"), 800_000);
        assert_eq!(s.resolve_context_limit(None, "gemini-1.5-pro"), 800_000);
        assert_eq!(s.resolve_context_limit(None, "claude-3-5-sonnet"), 180_000);
        assert_eq!(s.resolve_context_limit(None, "gpt-4o"), 110_000);
        assert_eq!(s.resolve_context_limit(None, "o1-preview"), 180_000);
        assert_eq!(s.resolve_context_limit(None, "qwen-max"), 110_000);
        assert_eq!(s.resolve_context_limit(None, "qwen-long"), 800_000);
        assert_eq!(s.resolve_context_limit(None, "llama-3-8k"), 7_000);
        assert_eq!(s.resolve_context_limit(None, "unknown-model"), 64_000);

        // 2. 自定义覆盖测试（model 级别）
        s.model_context_limits.insert("unknown-model".into(), 45_000);
        assert_eq!(s.resolve_context_limit(None, "unknown-model"), 45_000);

        // 3. 自定义覆盖测试（providerId:model 优先于 model）
        s.model_context_limits.insert("deepseek-chat".into(), 60_000);
        assert_eq!(s.resolve_context_limit(None, "deepseek-chat"), 60_000);
        s.model_context_limits.insert("p1:deepseek-chat".into(), 62_000);
        assert_eq!(s.resolve_context_limit(Some("p1"), "deepseek-chat"), 62_000);
        assert_eq!(s.resolve_context_limit(Some("p2"), "deepseek-chat"), 60_000);
    }
}

/// 审批规则：仅对所属对话生效（会话级；不跨对话共享，也不存在全局规则）
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRule {
    pub id: String,
    /// 所属对话 id：规则仅在该对话内参与判定，随对话删除一并清理
    pub session_id: String,
    pub kind: String, // "command_prefix" | "path_write" | "tool"
    pub pattern: String,
    pub created_at: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    pub name: String,
    /// 项目绑定的目录（工作区即项目）；早期手动创建的项目可能为空
    #[serde(default)]
    pub path: Option<String>,
    pub pinned: bool,
    pub created_at: String,
    #[serde(default)]
    pub last_activity_at: Option<String>,
    /// 项目约束（Markdown：规范 / 注意事项等）；空 = 未设置。
    /// 该项目下每个新对话都会把此内容注入 system prompt
    #[serde(default)]
    pub constraints: String,
    /// 交付前自检 SOP 命令（如 cargo check, npm test 等）
    #[serde(default)]
    pub sop_verify_cmd: Option<String>,
    /// 是否开启交付前自检
    #[serde(default = "default_true")]
    pub sop_enabled: bool,
    /// 计划与执行模式："standard" | "always_plan" | "always_proceed"
    #[serde(default = "default_plan_mode")]
    pub plan_mode: String,
}

fn default_true() -> bool {
    true
}

fn default_plan_mode() -> String {
    "standard".to_string()
}

/// 关联项目：当前项目对另一目录的引用 + 说明（Markdown）。
/// 只存目录不存对方 project_id：对方项目被移除后目录关系仍成立，
/// 注入时按路径只读解析对方项目的自身约束
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ProjectLink {
    pub id: String,
    pub project_id: String,
    pub path: String,
    pub description: String,
    pub created_at: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub id: String,
    pub title: String,
    pub workspace_path: String,
    #[serde(default)]
    pub access_mode: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
    pub status: String, // active | archived
    #[serde(default)]
    pub last_message_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    /// 临时空间对话：工作区为项目/关联项目的临时副本，合并前原目录不受影响
    #[serde(default)]
    pub is_temp: bool,
    #[serde(default)]
    pub temp_code: Option<String>,
    #[serde(default)]
    pub temp_root: Option<String>,
    /// 临时空间来源：主项目原始目录
    #[serde(default)]
    pub source_workspace: Option<String>,
    /// 最近一次合并时的消息序号边界：该序号（含）之前的消息永久不可编辑重发
    #[serde(default)]
    pub merged_seq: Option<i64>,
    /// 已合并且临时空间尚未清空：期间禁止继续发送消息（清空后解除）
    #[serde(default)]
    pub merged_pending: bool,
    /// 统计：该会话消耗的累积总 Token
    #[serde(default)]
    pub total_tokens: Option<u64>,
    #[serde(default)]
    pub prompt_tokens: Option<u64>,
    #[serde(default)]
    pub completion_tokens: Option<u64>,
    #[serde(default)]
    pub cached_tokens: Option<u64>,
    #[serde(default)]
    pub cache_hit_rate: Option<f64>,
    /// 父会话 ID（子 Agent 会话非空）
    #[serde(default)]
    pub parent_session_id: Option<String>,
    /// 会话类型：main | subagent
    #[serde(default = "default_session_type")]
    pub session_type: String,
    /// 子 Agent 角色标签
    #[serde(default)]
    pub subagent_role: Option<String>,
    /// 子 Agent 初始任务要求
    #[serde(default)]
    pub subagent_task: Option<String>,
    /// 会话专属上下文 Token 上限（覆盖全局及模型推荐值）
    #[serde(default)]
    pub context_token_limit: Option<usize>,
    /// 增量汇报水位线游标：上次已汇报的消息 ID（仅协作者使用）
    #[serde(default)]
    pub last_reported_msg_id: Option<String>,
    /// 是否在任务执行完成后自动汇报主进程（默认 true）
    #[serde(default = "default_auto_report")]
    pub auto_report: Option<bool>,
    /// 触发派生该子进程/子 Agent 的工具事件 ID
    #[serde(default)]
    pub trigger_tool_event_id: Option<String>,
    /// 会话专属模型厂商 ID（为空则跟随全局）
    #[serde(default)]
    pub provider_id: Option<String>,
    /// 会话专属模型标识（为空则跟随全局）
    #[serde(default)]
    pub model_id: Option<String>,
    /// 注入主进程的调度触发规则（由用户在创建/编辑协作者时配置）
    #[serde(default)]
    pub dispatch_rule: Option<String>,
    /// 会话专属生图模型厂商 ID（为空则跟随全局）
    #[serde(default)]
    pub image_provider_id: Option<String>,
    /// 会话专属生图模型标识（为空则跟随全局）
    #[serde(default)]
    pub image_model_id: Option<String>,
    /// 会话专属视觉感知模型厂商 ID（为空则跟随全局）
    #[serde(default)]
    pub vision_provider_id: Option<String>,
    /// 会话专属视觉感知模型标识（为空则跟随全局）
    #[serde(default)]
    pub vision_model_id: Option<String>,
    /// 分支来源会话 ID（若从某个会话节点分叉出来）
    #[serde(default)]
    pub forked_from_session_id: Option<String>,
    /// 分支来源消息 ID
    #[serde(default)]
    pub forked_from_message_id: Option<String>,
    /// 会话专属思考程度（low / medium / high / None 即默认不传）
    #[serde(default)]
    pub reasoning_effort: Option<String>,
    /// 最近一次 Run 的执行终态（'done' | 'failed' | 'interrupted' | 'cancelled' | 'running'）
    #[serde(default)]
    pub last_run_status: Option<String>,
}

fn default_auto_report() -> Option<bool> {
    Some(true)
}

fn default_session_type() -> String {
    "main".into()
}

impl Default for Session {
    fn default() -> Self {
        Self {
            id: String::new(),
            title: String::new(),
            workspace_path: String::new(),
            access_mode: Some("confirm".into()),
            project_id: None,
            status: "active".into(),
            last_message_at: None,
            created_at: String::new(),
            updated_at: String::new(),
            is_temp: false,
            temp_code: None,
            temp_root: None,
            source_workspace: None,
            merged_seq: None,
            merged_pending: false,
            total_tokens: Some(0),
            prompt_tokens: Some(0),
            completion_tokens: Some(0),
            cached_tokens: Some(0),
            cache_hit_rate: Some(0.0),
            parent_session_id: None,
            session_type: "main".into(),
            subagent_role: None,
            subagent_task: None,
            context_token_limit: None,
            last_reported_msg_id: None,
            auto_report: Some(true),
            trigger_tool_event_id: None,
            provider_id: None,
            model_id: None,
            dispatch_rule: None,
            image_provider_id: None,
            image_model_id: None,
            vision_provider_id: None,
            vision_model_id: None,
            forked_from_session_id: None,
            forked_from_message_id: None,
            reasoning_effort: None,
            last_run_status: None,
        }
    }
}

/// 临时空间中被拷贝的单个项目条目（主项目 key="main"，关联项目 key="link:<id>"）
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct TempProjectEntry {
    pub key: String,
    pub name: String,
    pub source: String,
    pub temp: String,
    #[serde(default)]
    pub description: String,
    /// 拷贝后基线提交的 commit sha（变更检测与合并的对照基准）
    #[serde(default)]
    pub baseline: Option<String>,
}

/// 临时空间清单：会话与 `数据目录\temp-project\<code>` 的映射快照（存 session_kv）
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct TempManifest {
    pub code: String,
    pub root: String,
    pub projects: Vec<TempProjectEntry>,
}

/// alloc_temp_code 返回给草稿的临时空间计划（首发落库时原样回传）
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct TempAlloc {
    pub code: String,
    pub root: String,
    pub main_temp: String,
    pub source_workspace: String,
    pub projects: Vec<TempProjectEntry>,
}

/// get_temp_info：驱动临时对话按钮禁用态与消息编辑边界的运行时状态
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct TempInfo {
    pub is_temp: bool,
    /// 临时空间目录当前是否存在
    pub exists: bool,
    pub has_changes: bool,
    pub changed_count: usize,
    pub merged: bool,
    pub merged_pending: bool,
    pub merged_seq: Option<i64>,
    pub temp_root: Option<String>,
    pub source_workspace: Option<String>,
}

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct MergeProjectSummary {
    pub name: String,
    pub source: String,
    /// 直接复制写回的文件数
    pub applied: usize,
    /// 经 AI 智能合并写回的文件数
    pub ai_merged: usize,
    /// 未能自动处理（需人工介入）的条目："路径: 原因"
    pub skipped: Vec<String>,
}

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct MergeSummary {
    pub projects: Vec<MergeProjectSummary>,
    pub total_applied: usize,
    pub total_ai_merged: usize,
    pub total_skipped: usize,
}

// ---------- 变更列表 / 文件 diff（临时空间变更弹窗） ----------

/// 变更文件条目（左侧列表）
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct TempChangeFile {
    /// 相对项目根的路径（正斜杠，来自 git 输出）
    pub path: String,
    pub name: String,
    /// added | modified | deleted
    pub change: String,
    pub added: usize,
    pub removed: usize,
    pub binary: bool,
    pub too_large: bool,
}

/// 按项目分组的变更（左侧列表的分组头）
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct TempChangeProject {
    pub key: String,
    pub name: String,
    pub source: String,
    pub temp: String,
    pub files: Vec<TempChangeFile>,
}

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct TempChanges {
    pub total_files: usize,
    pub projects: Vec<TempChangeProject>,
}

/// diff 行：tag = same | del | add；old_no / new_no 为 1 起始行号（该侧无对应行时为 None）
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DiffLine {
    pub tag: String,
    pub old_no: Option<usize>,
    pub new_no: Option<usize>,
    pub text: String,
}

/// diff 分块（±3 行上下文），同一份数据可渲染统一视图与并排对比
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DiffHunk {
    pub old_start: usize,
    pub old_lines: usize,
    pub new_start: usize,
    pub new_lines: usize,
    pub lines: Vec<DiffLine>,
}

/// 单文件 diff（弹窗右侧）
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct TempFileDiff {
    pub project_key: String,
    pub project_name: String,
    pub path: String,
    pub name: String,
    pub change: String,
    pub binary: bool,
    pub too_large: bool,
    pub added: usize,
    pub removed: usize,
    /// diff 行数超限时截断
    pub truncated: bool,
    pub hunks: Vec<DiffHunk>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ToolEvent {
    pub id: String,
    pub message_id: String,
    pub tool_name: String,
    #[serde(default)]
    pub tool_call_id: Option<String>,
    #[serde(default)]
    pub params: serde_json::Value,
    #[serde(default)]
    pub result_text: Option<String>,
    pub status: String, // pending_approval | running | success | failed | denied | timeout
    #[serde(default)]
    pub approval_scope: Option<String>, // mode | session | once | none
    pub created_at: String,
    /// 该工具调用派生创建的子进程/子 Agent 真实会话 ID
    #[serde(default)]
    pub subprocess_id: Option<String>,
    /// 工具快照软撤回时间（若该工具事件产生的快照已被撤回则非空）
    #[serde(default)]
    pub reverted_at: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    pub id: String,
    pub name: String,
    pub mime_type: String,
    pub size: u64,
    pub path: String,
    #[serde(default)]
    pub is_image: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub id: String,
    pub session_id: String,
    #[serde(default)]
    pub run_id: Option<String>,
    pub seq: i64,
    pub role: String, // user | assistant | tool | system
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub reasoning: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<serde_json::Value>,
    #[serde(default)]
    pub tool_call_id: Option<String>,
    #[serde(default)]
    pub queued: bool,
    #[serde(default)]
    pub usage: Option<serde_json::Value>,
    pub created_at: String,
    #[serde(default)]
    pub tool_events: Vec<ToolEvent>,
    /// 本步单次推理耗时（毫秒）
    #[serde(default)]
    pub duration_ms: Option<u64>,
    /// 本次对话完整耗时（从发送消息到该计划执行完毕，毫秒）
    #[serde(default)]
    pub turn_duration_ms: Option<u64>,
    #[serde(default)]
    pub prompt_tokens: Option<u64>,
    #[serde(default)]
    pub completion_tokens: Option<u64>,
    #[serde(default)]
    pub total_tokens: Option<u64>,
    #[serde(default)]
    pub cached_tokens: Option<u64>,
    #[serde(default)]
    pub is_estimated: Option<bool>,
    /// 消息携带的附件列表（图片或文件）
    #[serde(default)]
    pub attachments: Option<Vec<Attachment>>,
    /// 软撤回时间戳（若非空表示本消息已被软撤回，排除在上下文组装之外）
    #[serde(default)]
    pub reverted_at: Option<String>,
}

/// 代码文件影子快照条目（与 tool_event_id / message_id 关联）
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ToolFileSnapshot {
    pub id: String,
    pub session_id: String,
    pub message_id: String,
    pub tool_event_id: String,
    pub file_path: String,
    pub before_hash: Option<String>,
    pub after_hash: String,
    #[serde(default)]
    pub is_new_file: bool,
    #[serde(default)]
    pub reverted_at: Option<String>,
    pub created_at: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RevertResult {
    pub success: bool,
    pub reverted_files: Vec<String>,
    pub has_conflict: bool,
    pub conflicted_files: Vec<String>,
    pub message: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ReapplyResult {
    pub success: bool,
    pub reapplied_files: Vec<String>,
    pub has_conflict: bool,
    pub conflicted_files: Vec<String>,
    pub message: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotFileDiff {
    pub file_path: String,
    pub is_new_file: bool,
    pub added: usize,
    pub removed: usize,
    pub diff_text: String,
    pub before_content: Option<String>,
    pub after_content: String,
}


#[derive(Serialize, Deserialize, Default, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RunTokenMetrics {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    #[serde(default)]
    pub cached_tokens: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRequest {
    pub event_id: String,
    pub session_id: String,
    pub tool_name: String,
    pub params: serde_json::Value,
    pub risk: String,    // write | execute | path
    pub preview: String, // 命令文本或 diff 预览
    pub force_once: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SendResult {
    pub session_id: String,
    pub message_id: String,
    pub queued: bool,
    /// 会话实体（草稿首发落库时为新会话）：前端在切换会话前先入列，
    /// 避免「currentId 已切换、会话事件未到达」期间界面按草稿渲染造成闪现
    pub session: Option<Session>,
}

/// 粗略 token 估算：CJK 按字计、其余按 4 字符 1 token
pub fn estimate_tokens(s: &str) -> usize {
    let mut cjk = 0usize;
    let mut other = 0usize;
    for ch in s.chars() {
        if ch.is_ascii() {
            other += 1;
        } else {
            cjk += 1;
        }
    }
    (cjk as f64 * 0.7) as usize + other / 4
}

/// 估算 JSON 结构体（例如 tool_calls、参数或 schema）的 token 开销
pub fn estimate_value_tokens(v: &serde_json::Value) -> usize {
    match v {
        serde_json::Value::Null => 1,
        serde_json::Value::Bool(_) => 1,
        serde_json::Value::Number(_) => 1,
        serde_json::Value::String(s) => estimate_tokens(s),
        serde_json::Value::Array(arr) => {
            arr.iter().map(estimate_value_tokens).sum::<usize>() + 2
        }
        serde_json::Value::Object(obj) => {
            obj.iter()
                .map(|(k, val)| estimate_tokens(k) + estimate_value_tokens(val) + 2)
                .sum::<usize>() + 2
        }
    }
}

/// 归一化解析大模型实际返回的 Token 使用量与缓存命中详情
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct ParsedTokenUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    pub cached_tokens: u64,
    pub reasoning_tokens: Option<u64>,
    pub is_estimated: bool,
}

pub fn parse_llm_usage(
    raw: Option<&serde_json::Value>,
    est_in: usize,
    est_out: usize,
) -> ParsedTokenUsage {
    let Some(u) = raw else {
        return ParsedTokenUsage {
            prompt_tokens: est_in as u64,
            completion_tokens: est_out as u64,
            total_tokens: (est_in + est_out) as u64,
            cached_tokens: 0,
            reasoning_tokens: None,
            is_estimated: true,
        };
    };

    // 1. 输入 Prompt 兼容提取 (OpenAI: prompt_tokens, Anthropic: input_tokens, Gemini: promptTokenCount)
    let p = u
        .get("prompt_tokens")
        .or_else(|| u.get("promptTokens"))
        .or_else(|| u.get("input_tokens"))
        .or_else(|| u.get("promptTokenCount"))
        .and_then(|v| v.as_u64());

    // 2. 缓存 Token 提取 (兼容 DeepSeek, OpenAI, Claude 原生与各种中转/代理)
    let cached = u
        .get("prompt_tokens_details")
        .and_then(|d| d.get("cached_tokens").or_else(|| d.get("cache_read_input_tokens")))
        .or_else(|| u.get("prompt_cache_hit_tokens"))
        .or_else(|| u.get("cache_read_input_tokens"))
        .or_else(|| u.get("cachedContentTokenCount"))
        .or_else(|| u.get("cached_tokens"))
        .or_else(|| u.get("cachedTokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    // 3. 输出 Completion 提取
    let c = u
        .get("completion_tokens")
        .or_else(|| u.get("completionTokens"))
        .or_else(|| u.get("output_tokens"))
        .or_else(|| u.get("candidatesTokenCount"))
        .and_then(|v| v.as_u64());

    // 4. 推理 Token 提取 (DeepSeek R1 / OpenAI o1 / o3)
    let reasoning = u
        .get("completion_tokens_details")
        .and_then(|d| d.get("reasoning_tokens"))
        .or_else(|| u.get("reasoning_tokens"))
        .and_then(|v| v.as_u64());

    let is_estimated = p.is_none() && c.is_none();
    let raw_prompt = p.unwrap_or(est_in as u64);
    let prompt_tokens = if raw_prompt < cached {
        // Anthropic 原生格式中 input_tokens 可能不包含 cache_read_input_tokens
        raw_prompt + cached
    } else {
        raw_prompt
    };

    let completion_tokens = c.unwrap_or(est_out as u64);
    let total_tokens = u
        .get("total_tokens")
        .or_else(|| u.get("totalTokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(prompt_tokens + completion_tokens);

    ParsedTokenUsage {
        prompt_tokens,
        completion_tokens,
        total_tokens,
        cached_tokens: cached.min(prompt_tokens),
        reasoning_tokens: reasoning,
        is_estimated,
    }
}

fn default_compaction_timeout() -> u64 {
    30
}

/// 上下文自动压缩挂起请求（推送给前端展示可视化卡片并等待用户确认/补充）
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CompactionRequest {
    pub event_id: String,
    pub session_id: String,
    pub start_seq: i64,
    pub end_seq: i64,
    pub start_preview: String,
    pub end_preview: String,
    pub message_count: usize,
    pub tokens_before: usize,
    pub summary: String,
    #[serde(default = "default_compaction_timeout")]
    pub timeout_seconds: u64,
}

/// 用户对压缩请求的确认决定（包含用户可能补充或编辑后的 Markdown 内容）
#[derive(Clone, Debug)]
pub struct CompactionDecision {
    pub approved: bool,
    pub final_summary: String,
}

/// 会话上下文压缩记录（持久化实体）
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SessionCompaction {
    pub id: String,
    pub session_id: String,
    pub start_seq: i64,
    pub end_seq: i64,
    pub summary_markdown: String,
    pub tokens_before: usize,
    pub created_at: String,
}

/// 上下文硬截断提醒（推送给前端展示提醒卡片，待用户手动关闭）
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct TruncationNotice {
    pub id: String,
    pub session_id: String,
    pub dropped_turns: usize,
    pub dropped_messages: usize,
    pub dropped_tokens: usize,
    pub token_limit: usize,
    pub est_tokens_before: usize,
    pub est_tokens_after: usize,
    pub first_preview: String,
    pub created_at: String,
}

/// 工具结果上限
pub const TOOL_RESULT_LIMIT: usize = 32 * 1024;

pub fn truncate_result(s: &str) -> String {
    if s.len() <= TOOL_RESULT_LIMIT {
        return s.to_string();
    }
    let mut end = TOOL_RESULT_LIMIT;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n\n[结果过长，已截断至 32KB]", &s[..end])
}

/// 运行中的会话（全局查询结果项）：界面刷新后恢复运行状态用
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RunningSession {
    pub session_id: String,
    pub run_id: String,
}

/// Agent 成长条目（经验反思与沉淀）
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct GrowthItem {
    pub id: String,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub session_title: Option<String>,
    #[serde(default)]
    pub message_id: Option<String>,
    #[serde(default)]
    pub run_id: Option<String>,
    pub trigger_type: String, // user_rejection | self_healed | user_taught | manual
    pub trigger_context: String,
    pub reflection_thought: String,
    pub category: String, // command_rule | code_style | build_test | pitfall | workflow
    pub title: String,
    pub rule_content: String,
    pub status: String, // proposed | accepted | rejected | disabled
    #[serde(default)]
    pub applied_count: i64,
    pub created_at: String,
    pub updated_at: String,
}

/// 工作区技能条目（.harness/skills/<name>/）
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SkillItem {
    pub name: String,
    pub description: String,
    pub script_type: String, // bat | ps1 | sh | py | js
    pub path: String,
    pub content: String,
    pub updated_at: String,
}

/// 项目 SOP 交付自检配置
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSopInfo {
    pub project_id: String,
    pub project_name: String,
    pub sop_verify_cmd: String,
    pub sop_enabled: bool,
    pub detected_stack: String,
    pub detected_default_cmd: String,
}

// ---------- Token 统计数据结构 ----------

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct TokenStatsSummary {
    pub total_prompt_tokens: u64,
    pub total_completion_tokens: u64,
    pub total_tokens: u64,
    #[serde(default)]
    pub total_cached_tokens: u64,
    #[serde(default)]
    pub overall_cache_hit_rate: f64,
    pub today_prompt_tokens: u64,
    pub today_completion_tokens: u64,
    pub today_tokens: u64,
    #[serde(default)]
    pub today_cached_tokens: u64,
    #[serde(default)]
    pub today_cache_hit_rate: f64,
    pub total_sessions: u64,
    pub total_messages: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProjectTokenStats {
    pub project_id: Option<String>,
    pub project_name: String,
    pub project_path: Option<String>,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    #[serde(default)]
    pub cached_tokens: u64,
    #[serde(default)]
    pub cache_hit_rate: f64,
    pub session_count: u64,
    pub message_count: u64,
    pub last_used_at: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct DailyTokenStats {
    pub date: String, // YYYY-MM-DD
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    #[serde(default)]
    pub cached_tokens: u64,
    #[serde(default)]
    pub cache_hit_rate: f64,
    pub message_count: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct SessionTokenStats {
    pub session_id: String,
    pub title: String,
    pub project_id: Option<String>,
    pub project_name: Option<String>,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    #[serde(default)]
    pub cached_tokens: u64,
    #[serde(default)]
    pub cache_hit_rate: f64,
    pub message_count: u64,
    pub last_message_at: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct TokenStatsReport {
    pub summary: TokenStatsSummary,
    pub by_project: Vec<ProjectTokenStats>,
    pub by_time: Vec<DailyTokenStats>,
    pub by_session: Vec<SessionTokenStats>,
}

// ==================== 长任务（Long-Running Task）模型 ====================

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum LongTaskStatus {
    Planning,
    Running,
    Paused,
    WaitingApproval,
    Completed,
    Failed,
    Cancelled,
}

impl LongTaskStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            LongTaskStatus::Planning => "planning",
            LongTaskStatus::Running => "running",
            LongTaskStatus::Paused => "paused",
            LongTaskStatus::WaitingApproval => "waiting_approval",
            LongTaskStatus::Completed => "completed",
            LongTaskStatus::Failed => "failed",
            LongTaskStatus::Cancelled => "cancelled",
        }
    }

    #[allow(dead_code)]
    pub fn from_str(s: &str) -> Self {
        match s {
            "planning" => LongTaskStatus::Planning,
            "running" => LongTaskStatus::Running,
            "paused" => LongTaskStatus::Paused,
            "waiting_approval" => LongTaskStatus::WaitingApproval,
            "completed" => LongTaskStatus::Completed,
            "failed" => LongTaskStatus::Failed,
            "cancelled" => LongTaskStatus::Cancelled,
            _ => LongTaskStatus::Running,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct TaskSubItem {
    pub id: String,
    pub index: usize,
    pub title: String,
    pub description: Option<String>,
    pub status: String, // "pending" | "in_progress" | "verifying" | "completed" | "failed" | "skipped"
    pub summary: Option<String>,
    pub error: Option<String>,
    pub verify_command: Option<String>,
    pub verify_output: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct LongTask {
    pub id: String,
    pub session_id: String,
    pub workspace_path: String,
    pub goal: String,
    pub status: String, // "planning" | "running" | "paused" | "waiting_approval" | "completed" | "failed" | "cancelled"
    pub current_subtask_index: usize,
    pub subtasks: Vec<TaskSubItem>,
    pub max_budget_tokens: Option<u64>,
    pub total_tokens_used: u64,
    pub current_step: usize,
    pub max_steps: usize,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct TaskCheckpoint {
    pub id: String,
    pub task_id: String,
    pub step_number: usize,
    pub subtask_id: Option<String>,
    pub status: String,
    pub summary: String,
    pub working_memory: String,
    pub git_commit_hash: Option<String>,
    pub created_at: String,
}



