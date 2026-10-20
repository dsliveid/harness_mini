use super::auth::{
    AuthAction, AuthContract, AuthContractCredentials, AuthContractPostLogin, AuthStep,
};
use crate::llm::LlmCfg;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const DEFAULT_AUTH_DISTILL_SYSTEM_PROMPT: &str = r##"你是一个 Web 自动化与浏览器免登契约提炼专家。
用户在网页端录制了一段人工登录的原始交互操作序列（可能包含账号输入、密码输入、点击提交按钮、弹窗中选择机构/科室/角色、关闭公告、到达业务首页等）。
你的任务是：将这段原始操作序列进行语义提炼，输出一份确定性、结构化的声明式鉴权契约 JSON (AuthContract v3.0)。

【提炼与清洗规则】
1. login_url: 必须是用户真正输入账号密码的登录入口 URL。优先采用【录制起始页面 URL】（例如包含 "#/login" 或 "/login" 的路径）。绝对不能把登录完成后的终态业务主页（如包含 "#/index"、"#/dashboard"）当成 login_url！
2. username: 提取录制中输入用户名/账号输入框的准确取值。
3. raw_password: 提取录制中输入密码框的准确取值（若无则为空字符串）。
4. post_login_actions: 识别在点击“登录”或“提交”之后发生的后置动作列表（若登录后直接进主页，则为 []）：
   - 若用户在弹窗中点选了科室、机构、账套、角色等：生成 action_type="select_modal_item", match_text="用户点击的目标名称（如呼吸内科/扬州市江都区卫生健康委员会）", confirm_btn="确定"
   - 若用户关闭了系统提示或更新公告：生成 action_type="dismiss_notice"
   - 若用户点击了特定勾选项或按钮：生成 action_type="click", match_text="文本", selector="可选选择器"
5. success_assert_url: 识别登录完全就绪后，页面所处的业务路由特征（参考【录制结束页面 URL】，例如包含 "#/index"、"#/dashboard"、"#/medicalAffairManage" 等，必须排除 "#/login"）。

严格只输出纯 JSON 字符串，不要输出任何额外的问候或 markdown 说明。
输出 JSON 格式示例：
{
  "system_name": "医疗业务系统",
  "login_url": "http://127.0.0.1:8080/#/login",
  "username": "zyt",
  "raw_password": "123",
  "post_login_actions": [
    {
      "step": 1,
      "desc": "在弹窗中选择呼吸内科",
      "action_type": "select_modal_item",
      "dialog_title": "选择科室",
      "match_text": "呼吸内科",
      "confirm_btn": "确定"
    }
  ],
  "success_assert_url": "#/index|#/medicalAffairManage"
}"##;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DistillTrace {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_name: Option<String>,
    pub system_prompt: String,
    pub user_prompt: String,
    pub raw_response: String,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DistillAuthContractResult {
    pub contract: AuthContract,
    pub trace: DistillTrace,
}

pub async fn distill_auth_contract(
    raw_steps: &[AuthStep],
    start_url: Option<&str>,
    end_url: Option<&str>,
    custom_system_prompt: Option<&str>,
    refine_prompt: Option<&str>,
    provider_name: Option<&str>,
    model_name: Option<&str>,
    master_key: &[u8; 32],
    cfg: &LlmCfg,
) -> Result<DistillAuthContractResult, String> {
    if raw_steps.is_empty() {
        return Err("录制步骤为空，请先在视窗中执行登录交互".to_string());
    }

    let start_instant = std::time::Instant::now();

    let mut steps_text = String::new();
    for s in raw_steps {
        let val_str = s.value.as_deref().unwrap_or("");
        let sel_str = s.selector.as_deref().unwrap_or("");
        let desc_str = s.desc.as_deref().unwrap_or("");
        steps_text.push_str(&format!(
            "- 步骤 {}: 动作={}, 描述=\"{}\", 目标=\"{}\", 录入值=\"{}\"\n",
            s.order, s.action, desc_str, sel_str, val_str
        ));
    }

    let start_u = start_url.unwrap_or("");
    let end_u = end_url.unwrap_or("");

    let system_prompt = custom_system_prompt
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_AUTH_DISTILL_SYSTEM_PROMPT);

    let mut user_prompt = format!(
        "【录制环境 URL 上下文】\n- 录制起始页面 URL (登录页面入口): {}\n- 录制结束页面 URL (终态业务主页): {}\n\n【原始录制交互轨迹如下】：\n{}\n请深度提炼并输出纯 JSON 格式的 AuthContract：",
        if !start_u.is_empty() { start_u } else { "(未检测到)" },
        if !end_u.is_empty() { end_u } else { "(未检测到)" },
        steps_text
    );

    if let Some(refine) = refine_prompt.map(|s| s.trim()).filter(|s| !s.is_empty()) {
        user_prompt.push_str(&format!(
            "\n\n【用户针对性修正与补充提示词要求】（最高优先级，请严格按此要求进行参数修正与提炼）：\n{}\n",
            refine
        ));
    }

    let messages = vec![
        json!({ "role": "system", "content": system_prompt }),
        json!({ "role": "user", "content": user_prompt }),
    ];

    let result = crate::llm::chat_stream(&cfg, &messages, &[], |_| {}, |_| {})
        .await
        .map_err(|e| format!("AI 提炼免登契约失败: {e}"))?;

    let text = result.content.trim();
    let json_text = if let Some(start) = text.find('{') {
        if let Some(end) = text.rfind('}') {
            &text[start..=end]
        } else {
            text
        }
    } else {
        text
    };

    let parsed = serde_json::from_str::<Value>(json_text)
        .map_err(|e| format!("解析 AI 返回的契约 JSON 失败: {e}\n原文: {text}"))?;

    let elapsed_ms = start_instant.elapsed().as_millis() as u64;
    let contract = parse_distilled_json(&parsed, raw_steps, start_url, end_url, master_key)?;

    Ok(DistillAuthContractResult {
        contract,
        trace: DistillTrace {
            provider_name: provider_name.map(|s| s.to_string()),
            model_name: model_name.map(|s| s.to_string()),
            system_prompt: system_prompt.to_string(),
            user_prompt,
            raw_response: text.to_string(),
            elapsed_ms,
        },
    })
}

/// 解析大模型返回的 JSON 对象并组装为经过加密的 AuthContract v3.0
pub fn parse_distilled_json(
    parsed: &Value,
    raw_steps: &[AuthStep],
    start_url: Option<&str>,
    end_url: Option<&str>,
    master_key: &[u8; 32],
) -> Result<AuthContract, String> {
    let start_u = start_url.unwrap_or("");
    let end_u = end_url.unwrap_or("");

    let system_name = parsed.get("system_name").and_then(|v| v.as_str()).map(|s| s.to_string());
    
    let mut login_url = parsed
        .get("login_url")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("")
        .to_string();

    // 智能校正：若提炼结果错误地把终态首页（例如包含 #/index 或与 end_u 一致）当成 login_url，且检测到了合法的 start_u，则强制校正为 start_u
    if !start_u.is_empty() && (login_url.is_empty() || login_url == end_u || login_url.contains("#/index") || login_url.ends_with("/index")) {
        if start_u.contains("login") || !start_u.contains("index") {
            login_url = start_u.to_string();
        }
    }
    if login_url.is_empty() {
        login_url = if !start_u.is_empty() {
            start_u.to_string()
        } else if !end_u.is_empty() {
            end_u.to_string()
        } else {
            "http://127.0.0.1:8080/#/login".to_string()
        };
    }

    let username = parsed
        .get("username")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let raw_pwd = parsed
        .get("raw_password")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let password_cipher = if !raw_pwd.is_empty() {
        crate::secrets::encrypt(master_key, raw_pwd)?
    } else {
        // 如果录制步骤中有密码框的原始输入
        let fallback_pwd = raw_steps
            .iter()
            .find(|s| s.is_encrypted == Some(true) && s.value.as_ref().map(|v| !v.is_empty()).unwrap_or(false))
            .and_then(|s| s.value.as_deref())
            .unwrap_or("");
        if !fallback_pwd.is_empty() {
            crate::secrets::encrypt(master_key, fallback_pwd)?
        } else {
            String::new()
        }
    };

    let mut actions = Vec::new();
    if let Some(arr) = parsed.get("post_login_actions").and_then(|v| v.as_array()) {
        for (idx, item) in arr.iter().enumerate() {
            let step = item.get("step").and_then(|v| v.as_u64()).map(|v| v as usize).unwrap_or(idx + 1);
            let desc = item.get("desc").and_then(|v| v.as_str()).map(|s| s.to_string());
            let action_type = item.get("action_type").and_then(|v| v.as_str()).unwrap_or("select_modal_item").to_string();
            let dialog_title = item.get("dialog_title").and_then(|v| v.as_str()).map(|s| s.to_string());
            let match_text = item.get("match_text").and_then(|v| v.as_str()).map(|s| s.to_string());
            let selector = item.get("selector").and_then(|v| v.as_str()).map(|s| s.to_string());
            let confirm_btn = item.get("confirm_btn").and_then(|v| v.as_str()).map(|s| s.to_string());

            actions.push(AuthAction {
                step,
                desc,
                action_type,
                dialog_title,
                match_text,
                selector,
                confirm_btn,
            });
        }
    }

    let success_assert_url = parsed
        .get("success_assert_url")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let post_login = Some(AuthContractPostLogin {
        org_name: actions.first().and_then(|a| a.match_text.clone()),
        actions: if actions.is_empty() { None } else { Some(actions) },
        success_assert_url,
    });

    Ok(AuthContract {
        version: "3.0".to_string(),
        system_name,
        login_url,
        credentials: AuthContractCredentials {
            username,
            password_cipher,
        },
        post_login,
        updated_at: chrono::Utc::now().to_rfc3339(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_distilled_json() {
        let master_key = [7u8; 32];
        let json_val = serde_json::json!({
            "system_name": "智慧医务管理平台",
            "login_url": "http://127.0.0.1:8080/#/login",
            "username": "doctor_test",
            "raw_password": "my_secret_password_123",
            "post_login_actions": [
                {
                    "step": 1,
                    "desc": "在科室弹窗中选中心内科",
                    "action_type": "select_modal_item",
                    "dialog_title": "选择登录科室",
                    "match_text": "心内科"
                },
                {
                    "step": 2,
                    "desc": "关闭欢迎须知提示",
                    "action_type": "dismiss_notice",
                    "dialog_title": "系统须知",
                    "confirm_btn": "我知道了"
                }
            ],
            "success_assert_url": "#/index|#/medicalManage"
        });

        let contract = parse_distilled_json(
            &json_val,
            &[],
            Some("http://127.0.0.1:8080/#/login"),
            Some("http://127.0.0.1:8080/#/index"),
            &master_key,
        ).unwrap();

        assert_eq!(contract.version, "3.0");
        assert_eq!(contract.system_name.as_deref(), Some("智慧医务管理平台"));
        assert_eq!(contract.login_url, "http://127.0.0.1:8080/#/login");
        assert_eq!(contract.credentials.username, "doctor_test");

        // Decrypt password
        let decrypted = crate::secrets::decrypt(&master_key, &contract.credentials.password_cipher).unwrap();
        assert_eq!(decrypted, "my_secret_password_123");

        let post = contract.post_login.unwrap();
        // Backward compatibility: org_name matches first action
        assert_eq!(post.org_name.as_deref(), Some("心内科"));
        assert_eq!(post.success_assert_url.as_deref(), Some("#/index|#/medicalManage"));

        let acts = post.actions.unwrap();
        assert_eq!(acts.len(), 2);
        assert_eq!(acts[0].action_type, "select_modal_item");
        assert_eq!(acts[0].match_text.as_deref(), Some("心内科"));
        assert_eq!(acts[1].action_type, "dismiss_notice");
        assert_eq!(acts[1].confirm_btn.as_deref(), Some("我知道了"));

        // 测试终态首页误当登录地址时的智能校正
        let bad_json = serde_json::json!({
            "login_url": "http://127.0.0.1:8080/#/index",
            "username": "doctor_test",
            "raw_password": "123"
        });
        let fixed_contract = parse_distilled_json(
            &bad_json,
            &[],
            Some("http://127.0.0.1:8080/#/login"),
            Some("http://127.0.0.1:8080/#/index"),
            &master_key,
        ).unwrap();
        assert_eq!(fixed_contract.login_url, "http://127.0.0.1:8080/#/login");
    }

    #[test]
    fn test_default_prompt_and_trace_struct() {
        assert!(DEFAULT_AUTH_DISTILL_SYSTEM_PROMPT.contains("AuthContract v3.0"));
        assert!(DEFAULT_AUTH_DISTILL_SYSTEM_PROMPT.contains("login_url"));

        let trace = DistillTrace {
            provider_name: Some("DeepSeek".to_string()),
            model_name: Some("deepseek-chat".to_string()),
            system_prompt: DEFAULT_AUTH_DISTILL_SYSTEM_PROMPT.to_string(),
            user_prompt: "test user prompt".to_string(),
            raw_response: "{}".to_string(),
            elapsed_ms: 120,
        };

        let serialized = serde_json::to_string(&trace).unwrap();
        assert!(serialized.contains("DeepSeek"));
        assert!(serialized.contains("deepseek-chat"));
    }
}
