//! Jev (TypeSafe AI) 决策网关客户端与双系统 (System 1) 接入层。
//!
//! 提供基于 HTTP POST /v1/systemone 的极速结构化决策能力（Noul / Choice / Score），
//! 并实现“三层开关 + Abstain 全失败降级契约”。

use crate::models::JevCfg;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;

/// Jev 问题的三种基础原语
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum JevQuestion {
    /// 二元命题判定，返回概率 0.0 ~ 1.0
    Noul { instructions: String },
    /// 离散选项分类
    Choice { instructions: String, options: Vec<String> },
    /// 标准分级评估
    Score { instructions: String, rubric: Vec<String> },
}

/// 发送给 Jev /v1/systemone 的请求体
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SystemOneRequest {
    pub model: String,
    pub state: String,
    pub questions: HashMap<String, JevQuestion>,
}

/// 单个问题的评估结果
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct JevAnswer {
    #[serde(default)]
    pub noul: Option<f32>,
    #[serde(default)]
    pub choice: Option<String>,
    #[serde(default)]
    pub score: Option<f32>,
    #[serde(default)]
    pub confidence: Option<f32>,
    #[serde(default)]
    pub probabilities: Option<HashMap<String, f32>>,
}

/// 决策门控状态：允许 / 拒绝 / 弃权降级
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", content = "reason", rename_all = "lowercase")]
pub enum Gate {
    Allow,
    Deny(String),
    Abstain,
}

impl Gate {
    pub fn is_allowed(&self) -> bool {
        matches!(self, Gate::Allow)
    }

    pub fn is_denied(&self) -> bool {
        matches!(self, Gate::Deny(_))
    }

    pub fn is_abstain(&self) -> bool {
        matches!(self, Gate::Abstain)
    }
}

/// 任务复杂度分类结果
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskComplexity {
    Trivial,
    Small,
    Large,
    DeepResearch,
}

impl TaskComplexity {
    pub fn from_choice(choice: &str) -> Self {
        match choice.trim().to_lowercase().as_str() {
            "trivial" => TaskComplexity::Trivial,
            "small" => TaskComplexity::Small,
            "large" => TaskComplexity::Large,
            "deep_research" | "deepresearch" | "research" => TaskComplexity::DeepResearch,
            _ => TaskComplexity::Small,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            TaskComplexity::Trivial => "trivial",
            TaskComplexity::Small => "small",
            TaskComplexity::Large => "large",
            TaskComplexity::DeepResearch => "deep_research",
        }
    }
}

/// 方案可行性体检指标报告
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PlanReviewReport {
    pub completeness_prob: Option<f32>,
    pub consistency_prob: Option<f32>,
    pub verification_gap_prob: Option<f32>,
    pub context_fit_score: Option<f32>,
    pub risk_level: Option<String>,
    pub overall_verdict: String,
    pub confidence: f32,
    pub passed: bool,
    pub details: Vec<String>,
}

/// Jev 客户端实例
#[derive(Clone, Debug)]
pub struct JevClient {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub timeout_ms: u64,
    pub proxy_url: Option<String>,
}

/// 智能解析并规范化 Jev /v1/systemone 目标端点
/// 兼容用户直接填写：
/// - "https://api.typesafe.ai" -> "https://api.typesafe.ai/v1/systemone"
/// - "https://api.typesafe.ai/v1" -> "https://api.typesafe.ai/v1/systemone"
/// - "https://api.typesafe.ai/v1/systemone" -> "https://api.typesafe.ai/v1/systemone"
/// - "https://opencode.ai/zen" -> "https://opencode.ai/zen/v1/systemone"
/// - "https://opencode.ai/zen/v1" -> "https://opencode.ai/zen/v1/systemone"
/// - "https://opencode.ai/zen/v1/systemone" -> "https://opencode.ai/zen/v1/systemone"
pub fn resolve_systemone_endpoint(base_url: &str) -> String {
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.ends_with("/systemone") {
        trimmed.to_string()
    } else if trimmed.ends_with("/v1") {
        format!("{trimmed}/systemone")
    } else {
        format!("{trimmed}/v1/systemone")
    }
}

impl JevClient {
    pub fn new(
        base_url: String,
        api_key: String,
        model: String,
        timeout_ms: u64,
        proxy_url: Option<String>,
    ) -> Self {
        Self {
            base_url,
            api_key,
            model,
            timeout_ms: if timeout_ms == 0 { 800 } else { timeout_ms },
            proxy_url,
        }
    }

    /// 根据 SettingsData 中的 Jev 配置构建客户端；未启用或 Key 为空则返回 None
    pub fn from_cfg(cfg: &JevCfg, proxy_url: Option<String>) -> Option<Self> {
        if !cfg.enabled || cfg.api_key.trim().is_empty() {
            return None;
        }
        Some(Self::new(
            cfg.base_url.clone(),
            cfg.api_key.trim().to_string(),
            cfg.model.clone(),
            cfg.timeout_ms,
            proxy_url,
        ))
    }

    /// 构建 reqwest 客户端（包含超时与代理）
    fn build_http_client(&self) -> Result<reqwest::Client, String> {
        let mut builder = reqwest::Client::builder()
            .user_agent("harness-mini/0.1.0")
            .timeout(Duration::from_millis(self.timeout_ms));

        if let Some(ref p) = self.proxy_url {
            if !p.trim().is_empty() {
                if let Ok(proxy) = reqwest::Proxy::all(p) {
                    builder = builder.proxy(proxy);
                }
            }
        }

        builder.build().map_err(|e| format!("构建 HTTP 客户端失败: {e}"))
    }

    /// 智能解析并规范化 /v1/systemone 目标端点（支持自动去重，兼容官方地址与中转站格式）
    pub fn resolve_endpoint(&self) -> String {
        resolve_systemone_endpoint(&self.base_url)
    }

    /// 发起原生 POST /v1/systemone 请求
    pub async fn systemone(
        &self,
        state: &str,
        questions: HashMap<String, JevQuestion>,
    ) -> Result<HashMap<String, JevAnswer>, String> {
        if questions.is_empty() {
            return Ok(HashMap::new());
        }

        let http = self.build_http_client()?;
        let endpoint = resolve_systemone_endpoint(&self.base_url);

        let body = SystemOneRequest {
            model: self.model.clone(),
            state: state.to_string(),
            questions,
        };

        let sess_id = format!("jev-{}", uuid::Uuid::new_v4().simple());
        let resp = http
            .post(&endpoint)
            .header("Authorization", format!("Bearer {}", self.api_key.trim()))
            .header("Content-Type", "application/json")
            .header("x-opencode-session", &sess_id)
            .header("x-session-id", &sess_id)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Jev 网络请求失败（目标端点: {endpoint}）: {e}"))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let err_txt = resp.text().await.unwrap_or_default();
            let hint = match status.as_u16() {
                401 => "【401 未授权】：请检查输入的 Jev API Key 是否有效。若为中转站，需确保该 Key 拥有访问权限",
                403 => "【403 访问被拒绝】：权限不足或当前 IP/代理受到接口限制",
                404 => "【404 路径不存在】：请核对 API 端点 (Base URL) 是否正确，该服务是否支持 /v1/systemone 接口",
                _ => "",
            };
            let hint_str = if hint.is_empty() { String::new() } else { format!("\n排查建议: {hint}") };
            return Err(format!("Jev 服务返回错误 HTTP {status}（目标端点: {endpoint}）{hint_str}\n原始详情: {err_txt}"));
        }

        let raw_val: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("Jev 响应 JSON 解析失败: {e}"))?;

        parse_systemone_response(raw_val)
    }

    /// 单个 Noul 问题判定
    pub async fn decide_noul(
        &self,
        state: &str,
        instruction: &str,
        min_conf: f32,
    ) -> (Option<f32>, Gate) {
        let mut questions = HashMap::new();
        questions.insert(
            "q".into(),
            JevQuestion::Noul {
                instructions: instruction.to_string(),
            },
        );

        match self.systemone(state, questions).await {
            Ok(mut map) => {
                if let Some(ans) = map.remove("q") {
                    let conf = ans.confidence.unwrap_or(1.0);
                    if conf < min_conf {
                        return (ans.noul, Gate::Abstain);
                    }
                    if let Some(p) = ans.noul {
                        if p >= 0.5 {
                            return (Some(p), Gate::Allow);
                        } else {
                            return (Some(p), Gate::Deny(format!("Noul 概率过低: {p:.2}")));
                        }
                    }
                }
                (None, Gate::Abstain)
            }
            Err(_) => (None, Gate::Abstain),
        }
    }

    /// 单个 Choice 问题分类
    pub async fn decide_choice(
        &self,
        state: &str,
        instruction: &str,
        options: Vec<String>,
        min_conf: f32,
    ) -> Option<(String, f32)> {
        let mut questions = HashMap::new();
        questions.insert(
            "choice_q".into(),
            JevQuestion::Choice {
                instructions: instruction.to_string(),
                options,
            },
        );

        let map = self.systemone(state, questions).await.ok()?;
        let ans = map.get("choice_q")?;
        let conf = ans.confidence.unwrap_or(1.0);
        if conf < min_conf {
            return None;
        }
        ans.choice.as_ref().map(|c| (c.clone(), conf))
    }

    /// 痛点 1 决策：判定记忆内容真实性与长期必要性
    pub async fn judge_memory_quality(
        &self,
        content: &str,
        workspace_summary: &str,
        min_conf: f32,
    ) -> Gate {
        let mut questions = HashMap::new();
        questions.insert(
            "is_verified".into(),
            JevQuestion::Noul {
                instructions: "以下知识或经验记录，是否源自对工程代码/文件的实地读取与验证，而非模型的凭空脑补或推测？仅当存在客观依据时为真。".into(),
            },
        );
        questions.insert(
            "is_reusable".into(),
            JevQuestion::Noul {
                instructions: "该内容是否属于本项目或后续开发会重复用到的关键架构/规范/踩坑事实，而非单次排查的临时冗余信息？".into(),
            },
        );
        questions.insert(
            "value_score".into(),
            JevQuestion::Score {
                instructions: "评估该知识内容作为长期记忆的价值档位：".into(),
                rubric: vec![
                    "1: 局部临时信息，无长期复用价值".into(),
                    "2: 具备一定参考价值的通用知识".into(),
                    "3: 关键架构约定或高价值避坑经验".into(),
                ],
            },
        );

        let state = format!(
            "【工作区摘要】: {}\n【待入库记忆内容】:\n{}",
            if workspace_summary.is_empty() { "(未指定工作区)" } else { workspace_summary },
            content
        );

        let res = match self.systemone(&state, questions).await {
            Ok(m) => m,
            Err(_) => return Gate::Abstain, // 降级走原逻辑
        };

        let is_verified_ans = res.get("is_verified");
        let is_reusable_ans = res.get("is_reusable");
        let score_ans = res.get("value_score");

        let verified_prob = is_verified_ans.and_then(|a| a.noul).unwrap_or(0.0);
        let reusable_prob = is_reusable_ans.and_then(|a| a.noul).unwrap_or(0.0);
        let score_val = score_ans.and_then(|a| a.score).unwrap_or(1.0);

        let conf = is_verified_ans
            .and_then(|a| a.confidence)
            .unwrap_or(1.0)
            .min(is_reusable_ans.and_then(|a| a.confidence).unwrap_or(1.0));

        if conf < min_conf {
            return Gate::Abstain;
        }

        if verified_prob < 0.35 {
            return Gate::Deny(format!("内容疑似推测脑补（客观证据概率仅 {:.0}%）", verified_prob * 100.0));
        }

        if reusable_prob < 0.40 && score_val < 1.8 {
            return Gate::Deny(format!("内容长期复用价值过低（长期复用概率 {:.0}%，价值分 {:.1}）", reusable_prob * 100.0, score_val));
        }

        Gate::Allow
    }

    /// 痛点 2 & 3 决策：任务复杂度分类与外部调研判定
    pub async fn classify_task_complexity(
        &self,
        prompt: &str,
        min_conf: f32,
    ) -> Option<(TaskComplexity, bool, f32)> {
        let mut questions = HashMap::new();
        questions.insert(
            "complexity".into(),
            JevQuestion::Choice {
                instructions: "评估当前用户请求的任务规模与复杂度：".into(),
                options: vec![
                    "trivial".into(),       // 单行修改、简单查询、纯对话
                    "small".into(),         // 单文件小修复、小改动
                    "large".into(),         // 跨文件改动、新功能模块、重构
                    "deep_research".into(), // 根因不明、复杂疑难Bug、超出常识需深度探索
                ],
            },
        );
        questions.insert(
            "needs_research".into(),
            JevQuestion::Noul {
                instructions: "该请求是否明显超出已有局部认知，必须查阅外部网络资料或新库文档？".into(),
            },
        );

        let map = self.systemone(prompt, questions).await.ok()?;
        let comp_ans = map.get("complexity")?;
        let research_ans = map.get("needs_research");

        let conf = comp_ans.confidence.unwrap_or(1.0);
        if conf < min_conf {
            return None;
        }

        let choice_str = comp_ans.choice.as_deref().unwrap_or("small");
        let complexity = TaskComplexity::from_choice(choice_str);
        let needs_research = research_ans.and_then(|a| a.noul).map(|p| p > 0.65).unwrap_or(false);

        Some((complexity, needs_research, conf))
    }

    /// 场景 4 决策：命令高危破坏性语义双保险
    pub async fn check_command_safety(
        &self,
        command: &str,
        workspace_ctx: &str,
        min_conf: f32,
    ) -> Gate {
        let mut questions = HashMap::new();
        questions.insert(
            "is_destructive".into(),
            JevQuestion::Noul {
                instructions: "该终端命令是否具有不可逆删除工程文件、清空磁盘、泄露外发本地敏感数据或越权破坏系统环境的高危属性？".into(),
            },
        );
        questions.insert(
            "risk_score".into(),
            JevQuestion::Score {
                instructions: "评估执行该命令的潜在破坏与安全风险：".into(),
                rubric: vec![
                    "1: 安全无害的只读或受控编译测试命令".into(),
                    "2: 普通的文件改动或常规工具安装".into(),
                    "3: 破坏性或系统级强变更命令".into(),
                ],
            },
        );

        let state = format!(
            "【执行目录】: {}\n【拟执行命令】:\n{}",
            if workspace_ctx.is_empty() { "(全局上下文)" } else { workspace_ctx },
            command
        );

        let res = match self.systemone(&state, questions).await {
            Ok(m) => m,
            Err(_) => return Gate::Abstain,
        };

        let is_destructive_ans = res.get("is_destructive");
        let risk_score_ans = res.get("risk_score");

        let destructive_prob = is_destructive_ans.and_then(|a| a.noul).unwrap_or(0.0);
        let risk_score = risk_score_ans.and_then(|a| a.score).unwrap_or(1.0);
        let conf = is_destructive_ans.and_then(|a| a.confidence).unwrap_or(1.0);

        if conf < min_conf {
            return Gate::Abstain;
        }

        if destructive_prob > 0.60 || risk_score >= 2.6 {
            return Gate::Deny(format!(
                "Jev 语义风控识别为高危破坏性命令 (风险概率: {:.0}%, 风险评级: {:.1})",
                destructive_prob * 100.0,
                risk_score
            ));
        }

        Gate::Allow
    }

    /// 场景 5 决策：方案可行性“体检”评估
    pub async fn evaluate_plan_feasibility(
        &self,
        plan_content: &str,
        tech_stack: &str,
        min_conf: f32,
    ) -> Result<PlanReviewReport, String> {
        let mut questions = HashMap::new();
        questions.insert(
            "completeness".into(),
            JevQuestion::Noul {
                instructions: "方案中规划的变更文件与分步步骤，是否完整覆盖了需求目标（Goals）所列的所有要求？".into(),
            },
        );
        questions.insert(
            "consistency".into(),
            JevQuestion::Noul {
                instructions: "方案中的执行顺序与逻辑是否存在前后矛盾、步骤颠倒或前置依赖缺失？".into(),
            },
        );
        questions.insert(
            "verification_gap".into(),
            JevQuestion::Noul {
                instructions: "方案中是否明显缺失自动化测试、编译检查或失败回滚等验证手段？".into(),
            },
        );
        questions.insert(
            "context_fit".into(),
            JevQuestion::Score {
                instructions: "评估该方案架构与当前项目技术栈的切合匹配程度：".into(),
                rubric: vec![
                    "1: 严重脱离实际技术栈或引入不兼容体系".into(),
                    "2: 基本符合，局部可能需要补充调整".into(),
                    "3: 深度契合现有工程架构与最佳实践".into(),
                ],
            },
        );
        questions.insert(
            "risk".into(),
            JevQuestion::Choice {
                instructions: "该方案实施落地时的综合风险等级：".into(),
                options: vec!["low".into(), "medium".into(), "high".into()],
            },
        );

        let state = format!(
            "【项目技术栈】: {}\n【待体检技术方案】:\n{}",
            if tech_stack.is_empty() { "通用工程" } else { tech_stack },
            plan_content
        );

        let map = self.systemone(&state, questions).await?;

        let comp_ans = map.get("completeness");
        let cons_ans = map.get("consistency");
        let verif_ans = map.get("verification_gap");
        let fit_ans = map.get("context_fit");
        let risk_ans = map.get("risk");

        let completeness = comp_ans.and_then(|a| a.noul);
        let consistency = cons_ans.and_then(|a| a.noul);
        let verification_gap = verif_ans.and_then(|a| a.noul);
        let context_fit = fit_ans.and_then(|a| a.score);
        let risk_choice = risk_ans.and_then(|a| a.choice.clone());

        let avg_conf = comp_ans
            .and_then(|a| a.confidence)
            .unwrap_or(0.9)
            .min(risk_ans.and_then(|a| a.confidence).unwrap_or(0.9));

        let mut details = Vec::new();
        let mut passed = true;

        if let Some(c) = completeness {
            if c < 0.6 {
                passed = false;
                details.push(format!("⚠️ 目标覆盖度不足（覆盖概率仅 {:.0}%）", c * 100.0));
            } else {
                details.push(format!("✅ 目标覆盖完整（覆盖度 {:.0}%）", c * 100.0));
            }
        }

        if let Some(c) = consistency {
            if c < 0.5 {
                passed = false;
                details.push(format!("⚠️ 存在步骤逻辑不一致或依赖倒置风险（自洽度 {:.0}%）", c * 100.0));
            } else {
                details.push("✅ 步骤自洽无明显矛盾".into());
            }
        }

        if let Some(vg) = verification_gap {
            if vg > 0.6 {
                details.push(format!("⚠️ 验证/兜底机制薄弱（缺失概率 {:.0}%）", vg * 100.0));
            } else {
                details.push("✅ 具备基本的验证防护".into());
            }
        }

        if let Some(score) = context_fit {
            details.push(format!("📊 技术栈契合评分: {:.1}/3.0", score));
        }

        if let Some(ref r) = risk_choice {
            details.push(format!("⚡ 综合风险评级: {}", r.to_uppercase()));
            if r == "high" {
                passed = false;
            }
        }

        let overall_verdict = if avg_conf < min_conf {
            "置信度较低，评估结论仅供参考".into()
        } else if passed {
            "方案整体完备，建议采纳推进".into()
        } else {
            "方案存在潜在缺陷，建议用户复核后再批准执行".into()
        };

        Ok(PlanReviewReport {
            completeness_prob: completeness,
            consistency_prob: consistency,
            verification_gap_prob: verification_gap,
            context_fit_score: context_fit,
            risk_level: risk_choice,
            overall_verdict,
            confidence: avg_conf,
            passed,
            details,
        })
    }
}

/// 解析各种兼容形态的 SystemOne 返回 JSON
fn parse_systemone_response(val: serde_json::Value) -> Result<HashMap<String, JevAnswer>, String> {
    // 形式 1: { "answers": { "q1": { ... } } }
    if let Some(answers_obj) = val.get("answers").and_then(|v| v.as_object()) {
        let mut map = HashMap::new();
        for (k, v) in answers_obj {
            let item: JevAnswer = serde_json::from_value(v.clone())
                .map_err(|e| format!("解析 answer[{k}] 失败: {e}"))?;
            map.insert(k.clone(), item);
        }
        return Ok(map);
    }

    // 形式 2: 直接为平铺字典 { "q1": { "noul": 0.98 }, "q2": { "choice": ... } }
    if let Some(obj) = val.as_object() {
        let mut map = HashMap::new();
        for (k, v) in obj {
            // 跳过 usage / model 等顶级元数据
            if k == "usage" || k == "model" || k == "id" || k == "created" {
                continue;
            }
            if let Ok(item) = serde_json::from_value::<JevAnswer>(v.clone()) {
                map.insert(k.clone(), item);
            }
        }
        if !map.is_empty() {
            return Ok(map);
        }
    }

    Err(format!("无法识别的 Jev 响应格式: {val}"))
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DecisionExecutionResult {
    #[serde(default)]
    pub verdict: Option<bool>,
    #[serde(default)]
    pub choice: Option<String>,
    #[serde(default)]
    pub score: Option<f32>,
    pub confidence: f32,
    pub reason: String,
    pub latency_ms: u64,
    pub model: String,
}

/// 统一决策模型执行器：若 Base URL 为 TypeSafe / SystemOne 则走极速原生协议，否则走标准 OpenAI 兼容结构化决策 Prompt
/// 原生 TypeSafe / SystemOne 决策执行
async fn execute_decision_with_systemone(
    provider: &crate::models::ProviderCfg,
    model: &str,
    kind: &str,
    state: &str,
    instruction: &str,
    options: Option<Vec<String>>,
    rubric: Option<Vec<String>>,
    proxy_url: Option<String>,
) -> Result<DecisionExecutionResult, String> {
    let client = JevClient::new(
        provider.base_url.clone(),
        provider.api_key.clone(),
        model.to_string(),
        15000,
        proxy_url,
    );
    let t0 = std::time::Instant::now();
    match kind {
        "judge" => {
            let (prob, gate) = client.decide_noul(state, instruction, 0.0).await;
            if prob.is_none() && matches!(gate, Gate::Abstain) {
                return Err("SystemOne 未返回有效判定概率".into());
            }
            let lat = t0.elapsed().as_millis() as u64;
            let v = prob.map(|p| p >= 0.5);
            let reason = match gate {
                Gate::Allow => "判断为真（符合准则）".to_string(),
                Gate::Deny(r) => r,
                Gate::Abstain => "模型置信度偏低或弃权".to_string(),
            };
            Ok(DecisionExecutionResult {
                verdict: v,
                choice: None,
                score: prob,
                confidence: prob.unwrap_or(0.8),
                reason,
                latency_ms: lat,
                model: model.to_string(),
            })
        }
        "choice" => {
            let opts = options.unwrap_or_default();
            let res = client.decide_choice(state, instruction, opts.clone(), 0.0).await;
            let (c, conf) = res.ok_or_else(|| "SystemOne 多选决策未返回有效结果".to_string())?;
            let lat = t0.elapsed().as_millis() as u64;
            Ok(DecisionExecutionResult {
                verdict: None,
                choice: Some(c.clone()),
                score: None,
                confidence: conf,
                reason: format!("在备选项中选定「{c}」"),
                latency_ms: lat,
                model: model.to_string(),
            })
        }
        "score" | _ => {
            let rub = rubric.unwrap_or_else(|| vec![
                "1: 严重缺陷或完全不符合".into(),
                "2: 基本可用或部分符合".into(),
                "3: 深度契合或极佳".into(),
            ]);
            let mut questions = HashMap::new();
            questions.insert("q_score".into(), JevQuestion::Score { instructions: instruction.to_string(), rubric: rub });
            let map = client.systemone(state, questions).await.map_err(|e| format!("Jev 评分失败: {e}"))?;
            let lat = t0.elapsed().as_millis() as u64;
            let ans = map.get("q_score");
            let score_val = ans.and_then(|a| a.score).ok_or_else(|| "SystemOne 评分未返回有效数值".to_string())?;
            let conf = ans.and_then(|a| a.confidence).unwrap_or(0.8);
            Ok(DecisionExecutionResult {
                verdict: None,
                choice: None,
                score: Some(score_val),
                confidence: conf,
                reason: format!("综合评估打分: {:.1}%", score_val * 100.0),
                latency_ms: lat,
                model: model.to_string(),
            })
        }
    }
}

/// 通用大模型结构化提示词决策执行 (兼容 OpenAI Chat、Claude Messages、OpenAI Responses 等任意模型)
pub async fn execute_decision_with_llm(
    provider: &crate::models::ProviderCfg,
    model: &str,
    kind: &str,
    state: &str,
    instruction: &str,
    options: Option<Vec<String>>,
    rubric: Option<Vec<String>>,
    proxy_url: Option<String>,
) -> Result<DecisionExecutionResult, String> {
    let t0 = std::time::Instant::now();
    let protocol = match provider.get_model_protocol(model) {
        crate::models::RequestProtocol::SystemOne => {
            crate::models::RequestProtocol::infer(&provider.base_url, model)
        }
        p => p,
    };

    let cfg = crate::llm::LlmCfg {
        base_url: provider.base_url.clone(),
        api_key: provider.api_key.clone(),
        model: model.to_string(),
        protocol,
        reasoning_effort: None,
        proxy_url,
        session_id: Some(format!("decision-{}", uuid::Uuid::new_v4().simple())),
    };

    let system_prompt = match kind {
        "judge" => "你是一个高精度二元判断决策专家。请根据提供的上下文(state)与判断准则(instruction)，做出二元判定。\n必须且仅返回如下格式的纯 JSON 对象，不要添加任何其他前缀或 Markdown 标记：\n{\"verdict\": true, \"confidence\": 0.95, \"reason\": \"判定依据简述\"}",
        "choice" => "你是一个高精度多选决策专家。请根据提供的上下文(state)、选择准则(instruction)以及给定可选项列表(options)，从选项中严格挑选最匹配的一个。\n必须且仅返回如下格式的纯 JSON 对象，不要添加任何其他前缀或 Markdown 标记：\n{\"choice\": \"选中的选项文本\", \"confidence\": 0.95, \"reason\": \"选择理由简述\"}",
        "score" | _ => "你是一个高精度量化评分决策专家。请根据提供的上下文(state)、评分目标(instruction)以及评分量表(rubric)，给出 0 到 100 的评分（以 0.0 到 1.0 的浮点数表示，例如 0.92 代表 92%）。\n必须且仅返回如下格式的纯 JSON 对象，不要添加任何其他前缀或 Markdown 标记：\n{\"score\": 0.92, \"confidence\": 0.95, \"reason\": \"详细评分依据\"}",
    };

    let user_prompt = match kind {
        "judge" => format!("【上下文背景】:\n{}\n\n【判断准则】:\n{}", state, instruction),
        "choice" => {
            let opts = options.unwrap_or_default();
            format!("【上下文背景】:\n{}\n\n【选择目标】:\n{}\n\n【备选列表】:\n{}", state, instruction, opts.join("\n- "))
        }
        "score" | _ => {
            let rub = rubric.unwrap_or_default();
            format!("【待评估内容】:\n{}\n\n【评估要求】:\n{}\n\n【评分量表】:\n{}", state, instruction, rub.join("\n- "))
        }
    };

    let messages = vec![
        serde_json::json!({"role": "system", "content": system_prompt}),
        serde_json::json!({"role": "user", "content": user_prompt}),
    ];

    let mut content = String::new();
    let res = crate::llm::chat_stream(
        &cfg,
        &messages,
        &[],
        |delta| content.push_str(delta),
        |_reasoning| {},
    ).await.map_err(|e| format!("决策模型请求失败: {e}"))?;

    if content.trim().is_empty() {
        content = res.content;
    }

    let lat = t0.elapsed().as_millis() as u64;

    // 提取 JSON
    let json_str = if let Some(start) = content.find('{') {
        if let Some(end) = content.rfind('}') {
            &content[start..=end]
        } else {
            &content
        }
    } else {
        &content
    };

    let parsed: serde_json::Value = serde_json::from_str(json_str).unwrap_or_else(|_| serde_json::json!({
        "reason": content
    }));

    let verdict = parsed.get("verdict").and_then(|v| v.as_bool());
    let choice = parsed.get("choice").and_then(|v| v.as_str()).map(|s| s.to_string());
    let raw_score = parsed.get("score").and_then(|v| v.as_f64()).map(|s| s as f32);
    let score = raw_score.map(|s| if s > 1.0 { (s / 100.0).clamp(0.0, 1.0) } else { s.clamp(0.0, 1.0) });
    let confidence = parsed.get("confidence").and_then(|v| v.as_f64()).map(|s| s as f32).unwrap_or(0.9);
    let reason = parsed.get("reason").and_then(|v| v.as_str()).unwrap_or(&content).to_string();

    Ok(DecisionExecutionResult {
        verdict,
        choice,
        score,
        confidence,
        reason,
        latency_ms: lat,
        model: model.to_string(),
    })
}

/// 统一决策模型执行入口：优先原生 SystemOne 极速协议，若失败或端点不支持则自动无缝回退至 LLM 结构化提示词模式
pub async fn execute_decision(
    provider: &crate::models::ProviderCfg,
    model: &str,
    kind: &str, // "judge" | "choice" | "score"
    state: &str,
    instruction: &str,
    options: Option<Vec<String>>,
    rubric: Option<Vec<String>>,
    proxy_url: Option<String>,
) -> Result<DecisionExecutionResult, String> {
    let protocol = provider.get_model_protocol(model);
    let is_typesafe = protocol == crate::models::RequestProtocol::SystemOne
        || provider.base_url.contains("typesafe")
        || provider.base_url.contains("systemone");

    if is_typesafe {
        match execute_decision_with_systemone(
            provider,
            model,
            kind,
            state,
            instruction,
            options.clone(),
            rubric.clone(),
            proxy_url.clone(),
        ).await {
            Ok(res) => Ok(res),
            Err(e) => {
                eprintln!(
                    "目标端点 ({}) 原生 SystemOne 协议调用失败 ({e})，正在自动无缝降级为通用 LLM 结构化提示词决策模式...",
                    provider.base_url
                );
                execute_decision_with_llm(
                    provider,
                    model,
                    kind,
                    state,
                    instruction,
                    options,
                    rubric,
                    proxy_url,
                ).await
            }
        }
    } else {
        execute_decision_with_llm(
            provider,
            model,
            kind,
            state,
            instruction,
            options,
            rubric,
            proxy_url,
        ).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_systemone_nested_answers() {
        let json_str = r#"{
            "model": "jev-latest",
            "answers": {
                "is_urgent": { "noul": 0.95, "confidence": 0.98 },
                "category": { "choice": "bug", "confidence": 0.92 }
            },
            "usage": { "input_tokens": 12, "output_tokens": 0 }
        }"#;

        let val: serde_json::Value = serde_json::from_str(json_str).unwrap();
        let map = parse_systemone_response(val).unwrap();

        assert_eq!(map.len(), 2);
        assert_eq!(map["is_urgent"].noul, Some(0.95));
        assert_eq!(map["category"].choice.as_deref(), Some("bug"));
    }

    #[test]
    fn test_parse_systemone_flat_answers() {
        let json_str = r#"{
            "is_dangerous": { "noul": 0.05, "confidence": 0.99 },
            "risk_score": { "score": 1.2, "confidence": 0.94 }
        }"#;

        let val: serde_json::Value = serde_json::from_str(json_str).unwrap();
        let map = parse_systemone_response(val).unwrap();

        assert_eq!(map.len(), 2);
        assert_eq!(map["is_dangerous"].noul, Some(0.05));
        assert_eq!(map["risk_score"].score, Some(1.2));
    }

    #[test]
    fn test_gate_behavior() {
        assert!(Gate::Allow.is_allowed());
        assert!(!Gate::Allow.is_denied());
        assert!(!Gate::Allow.is_abstain());

        let deny = Gate::Deny("疑似脑补".into());
        assert!(deny.is_denied());
        assert!(!deny.is_allowed());

        assert!(Gate::Abstain.is_abstain());
    }

    #[test]
    fn test_task_complexity_conversion() {
        assert_eq!(TaskComplexity::from_choice("trivial"), TaskComplexity::Trivial);
        assert_eq!(TaskComplexity::from_choice("small"), TaskComplexity::Small);
        assert_eq!(TaskComplexity::from_choice("large"), TaskComplexity::Large);
        assert_eq!(TaskComplexity::from_choice("deep_research"), TaskComplexity::DeepResearch);
        assert_eq!(TaskComplexity::from_choice("unknown"), TaskComplexity::Small);
    }

    #[test]
    fn test_from_cfg_resilience() {
        let mut cfg = JevCfg::default();
        // 默认未启用，返回 None
        assert!(JevClient::from_cfg(&cfg, None).is_none());

        // 启用了但未配置 Key，仍返回 None
        cfg.enabled = true;
        cfg.api_key = "   ".into();
        assert!(JevClient::from_cfg(&cfg, None).is_none());

        // 启用且 Key 有效
        cfg.api_key = "ts-test-key".into();
        let client = JevClient::from_cfg(&cfg, Some("http://127.0.0.1:7890".into())).unwrap();
        assert_eq!(client.api_key, "ts-test-key");
        assert_eq!(client.model, "jev-latest");
        assert_eq!(client.proxy_url.as_deref(), Some("http://127.0.0.1:7890"));
    }

    #[test]
    fn test_systemone_request_serialization() {
        let mut questions = HashMap::new();
        questions.insert(
            "q_noul".into(),
            JevQuestion::Noul { instructions: "Is it verified?".into() },
        );
        questions.insert(
            "q_choice".into(),
            JevQuestion::Choice {
                instructions: "Pick one".into(),
                options: vec!["a".into(), "b".into()],
            },
        );
        questions.insert(
            "q_score".into(),
            JevQuestion::Score {
                instructions: "Rate quality".into(),
                rubric: vec!["1: bad".into(), "2: good".into()],
            },
        );

        let req = SystemOneRequest {
            model: "jev-latest".into(),
            state: "test state context".into(),
            questions,
        };

        let json_val = serde_json::to_value(&req).unwrap();
        assert_eq!(json_val["model"], "jev-latest");
        assert_eq!(json_val["state"], "test state context");
        assert_eq!(json_val["questions"]["q_noul"]["type"], "noul");
        assert_eq!(json_val["questions"]["q_choice"]["type"], "choice");
        assert_eq!(json_val["questions"]["q_score"]["type"], "score");
    }

    #[test]
    fn test_plan_review_report_serialization() {
        let report = PlanReviewReport {
            completeness_prob: Some(0.95),
            consistency_prob: Some(0.90),
            verification_gap_prob: Some(0.10),
            context_fit_score: Some(2.8),
            risk_level: Some("low".into()),
            overall_verdict: "方案结构完备".into(),
            confidence: 0.94,
            passed: true,
            details: vec!["✅ 目标覆盖完整".into()],
        };

        let json_str = serde_json::to_string(&report).unwrap();
        assert!(json_str.contains("\"completenessProb\":0.95"));
        assert!(json_str.contains("\"overallVerdict\":\"方案结构完备\""));
        assert!(json_str.contains("\"passed\":true"));
    }

    #[test]
    fn test_resolve_systemone_endpoint() {
        assert_eq!(
            resolve_systemone_endpoint("https://api.typesafe.ai"),
            "https://api.typesafe.ai/v1/systemone"
        );
        assert_eq!(
            resolve_systemone_endpoint("https://api.typesafe.ai/"),
            "https://api.typesafe.ai/v1/systemone"
        );
        assert_eq!(
            resolve_systemone_endpoint("https://api.typesafe.ai/v1"),
            "https://api.typesafe.ai/v1/systemone"
        );
        assert_eq!(
            resolve_systemone_endpoint("https://api.typesafe.ai/v1/"),
            "https://api.typesafe.ai/v1/systemone"
        );
        assert_eq!(
            resolve_systemone_endpoint("https://api.typesafe.ai/v1/systemone"),
            "https://api.typesafe.ai/v1/systemone"
        );
        assert_eq!(
            resolve_systemone_endpoint("https://opencode.ai/zen/v1/systemone"),
            "https://opencode.ai/zen/v1/systemone"
        );
        assert_eq!(
            resolve_systemone_endpoint("https://opencode.ai/zen/v1"),
            "https://opencode.ai/zen/v1/systemone"
        );
        assert_eq!(
            resolve_systemone_endpoint("https://opencode.ai/zen"),
            "https://opencode.ai/zen/v1/systemone"
        );
    }
}
