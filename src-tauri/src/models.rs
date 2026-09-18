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
    50
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
    #[serde(default = "default_cmd_timeout")]
    pub command_timeout_secs: u64,
    #[serde(default = "default_ctx_tokens")]
    pub context_token_limit: usize,
    #[serde(default)]
    pub model_context_limits: std::collections::HashMap<String, usize>,
    #[serde(default)]
    pub last_workspace_path: Option<String>,
    #[serde(default)]
    pub disabled_tools: Vec<String>,
    #[serde(default)]
    pub disabled_sops: Vec<String>,
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
            command_timeout_secs: default_cmd_timeout(),
            context_token_limit: default_ctx_tokens(),
            model_context_limits: std::collections::HashMap::new(),
            last_workspace_path: None,
            disabled_tools: vec![],
            disabled_sops: vec![],
        }
    }
}

/// 根据模型名称启发式推断预设上下文上限
pub fn infer_model_context_limit(model: &str) -> usize {
    let lower = model.to_lowercase();
    if lower.contains("deepseek") {
        56_000
    } else if lower.contains("claude") {
        180_000
    } else if lower.contains("gpt-4o") || lower.contains("gpt-4.5") || lower.contains("o1") || lower.contains("o3") {
        110_000
    } else if lower.contains("qwen") || lower.contains("千问") {
        110_000
    } else if lower.contains("glm") {
        110_000
    } else if lower.contains("kimi") || lower.contains("moonshot") {
        180_000
    } else if lower.contains("gemini") {
        200_000
    } else if lower.contains("8k") {
        7_000
    } else if lower.contains("16k") {
        14_000
    } else if lower.contains("32k") {
        28_000
    } else if lower.contains("64k") {
        56_000
    } else if lower.contains("128k") {
        110_000
    } else if lower.contains("200k") {
        180_000
    } else if lower.contains("1m") {
        200_000
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

        // 1. 智能推断测试
        assert_eq!(s.resolve_context_limit(None, "deepseek-chat"), 56_000);
        assert_eq!(s.resolve_context_limit(None, "claude-3-5-sonnet"), 180_000);
        assert_eq!(s.resolve_context_limit(None, "gpt-4o"), 110_000);
        assert_eq!(s.resolve_context_limit(None, "qwen-max"), 110_000);
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
}

fn default_true() -> bool {
    true
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
}

fn default_auto_report() -> Option<bool> {
    Some(true)
}

fn default_session_type() -> String {
    "main".into()
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
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DiffLine {
    pub tag: String,
    pub old_no: Option<usize>,
    pub new_no: Option<usize>,
    pub text: String,
}

/// diff 分块（±3 行上下文），同一份数据可渲染统一视图与并排对比
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DiffHunk {
    pub old_start: usize,
    pub old_lines: usize,
    pub new_start: usize,
    pub new_lines: usize,
    pub lines: Vec<DiffLine>,
}

/// 单文件 diff（弹窗右侧）
#[derive(Serialize, Clone, Debug)]
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
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RunTokenMetrics {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
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
    pub today_prompt_tokens: u64,
    pub today_completion_tokens: u64,
    pub today_tokens: u64,
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


