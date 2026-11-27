//! 登录 SOP 协议
//!
//! 设计取向：登录流程的知识应以**人类可读文档**形式存在（可 diff、可 Git 版本化、
//! 换系统只改 md 不改代码），而不是硬编码在 Rust 里。
//!
//! 首次检测到未登录时自动落一份模板到 `<工作区>/.harness/login_sop.md`，
//! Agent 收到 `AUTH_REQUIRED` 后读取该文件，用 `browser_batch` 一次性完成登录。

use std::path::{Path, PathBuf};

pub fn sop_path(workspace: &Path) -> PathBuf {
    workspace.join(".harness").join("login_sop.md")
}

/// SOP 模板正文
pub fn template(login_url: &str) -> String {
    format!(
        r#"# 登录流程 SOP

> 本文件由 harness_mini 自动生成。请按实际系统补全后保存，Agent 将依此流程自动登录。
> ⚠️ 密码请使用 `${{secret:KEY}}` 占位符，**严禁写入明文**——明文会进入对话上下文与数据库。

## 目标系统
- 登录入口：{login_url}
- 凭据：
  - 用户名：admin
  - 密码：`${{secret:login_pwd}}`

## 流程（两步即可完成登录）
1. 输入账号密码，点击「登录」
2. 弹出机构选择弹窗，选择**第一个**机构并确认

## 关键注意事项
- **中间必须等待**：点完登录到机构弹窗出现之间有网络请求，必须用 `wait_for_selector` 等待，不要直接进下一步。
- **机构弹窗可能不出现**：若账号只有一个机构，弹窗不会弹出，此时该步会超时。请给该步加 `"optional": true`，超时会自动跳过而不中断。
- **结尾必须断言**：用 `wait_for_url` 或 `assert_text` 明确确认已进入主界面，不要靠猜。

## 参考写法（可直接交给 browser_batch）
```json
{{
  "steps": [
    {{"op": "wait_for_selector", "target": "input[type=password]", "desc": "等登录页就绪"}},
    {{"op": "fill", "target": "input[name=username]", "value": "admin"}},
    {{"op": "fill", "target": "input[type=password]", "value": "${{secret:login_pwd}}"}},
    {{"op": "click", "target": "button:has-text('登录')", "desc": "点登录"}},
    {{"op": "wait_for_selector", "target": "[role=dialog]", "optional": true, "timeout_ms": 8000, "desc": "机构弹窗（可能不出现）"}},
    {{"op": "pick_first", "target": "[role=dialog]", "optional": true, "desc": "选第一个机构"}},
    {{"op": "wait_for_url", "pattern": "/dashboard|/home|/index", "timeout_ms": 15000, "desc": "断言进入主界面"}}
  ],
  "detail": "summary"
}}
```

## 验证码场景（如有）
- 图形验证码：在对应位置插入 `{{"op":"pause","value":"captcha_required"}}`，由人工输入或先用 browser_screenshot 截图识别。
- 短信/邮箱验证码：**必须人工**，Agent 拿不到验证码，插入 pause 并提示用户。

## 凭据录入方式
在对话中告知 Agent「更新 login_pwd 为 xxxx」，或由前端凭据面板写入。
凭据加密存放于 `.harness/browser_secrets.json`（AES-256-GCM，已自动加入 .gitignore）。
"#,
        login_url = if login_url.is_empty() { "http://localhost:8080/#/login" } else { login_url }
    )
}

/// 若 SOP 不存在则落模板
///
/// 返回 `Result`：写盘失败时明确返回错误原因，由调用方决定如何兜底。
/// 历史缺陷：原实现静默返回 `created: false`，但错误消息仍指引 Agent 去读一个
/// 不存在的文件（实测中 Agent 因此卡住），必须把失败原因暴露出来。
pub fn ensure_sop(workspace: &Path, login_url: &str) -> Result<(PathBuf, bool), String> {
    let path = sop_path(workspace);
    if path.exists() {
        return Ok((path, false));
    }
    if workspace.as_os_str().is_empty() {
        return Err("工作区路径为空，无法创建 login_sop.md".to_string());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("创建 .harness 目录失败: {e}"))?;
    }
    std::fs::write(&path, template(login_url))
        .map_err(|e| format!("写入 login_sop.md 失败（{}）: {e}", path.display()))?;
    Ok((path, true))
}

/// 构造结构化的 AUTH_REQUIRED 错误文本
///
/// 设计原则：**无论 SOP 文件是否创建成功，都必须把登录流程知识送达 Agent。**
/// 写盘失败时把模板内联到消息中，保证信息不丢失（实测中这正是 Agent 卡住的点）。
pub fn auth_required_message(workspace: &Path, current_url: &str, reason: &str) -> String {
    let sop_result = ensure_sop(workspace, current_url);

    let mut msg = format!(
        "{{\"code\":\"AUTH_REQUIRED\",\"reason\":\"{reason}\",\"current_url\":\"{current_url}\""
    );

    match &sop_result {
        Ok((path, created)) => {
            let rel = path
                .strip_prefix(workspace)
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_else(|_| path.to_string_lossy().replace('\\', "/"));
            msg.push_str(&format!(",\"sop_path\":\"{rel}\",\"sop_created\":{created}}}"));
            msg.push_str(&format!(
                "\n\n> ⚠️ **需要登录**：当前页面处于未登录态（{reason}）。\n\
                 > 处理步骤：\n\
                 > 1. 读取 `{rel}` 获取本系统的登录流程；\n\
                 > 2. 用 `browser_batch` 一次性完成登录（填账号 → 填密码 → 点登录 → 选机构 → 断言进入主页）；\n\
                 > 3. 密码用 `${{secret:KEY}}` 占位符，**不要询问用户明文密码**。"
            ));
            if *created {
                msg.push_str(&format!(
                    "\n> 📄 已为你生成登录 SOP 模板（`{rel}`），请先补全其中的登录入口与凭据 key。"
                ));
            }
        }
        Err(e) => {
            // 关键兜底：写盘失败也要把流程知识交给 Agent，而不是给一个读不到的路径
            msg.push_str(",\"sop_path\":null,\"sop_created\":false}");
            msg.push_str(&format!(
                "\n\n> ⚠️ **需要登录**：当前页面处于未登录态（{reason}）。\n\
                 > ⚠️ 登录 SOP 文件创建失败（{e}），无法读取。\n\
                 > **以下为内置登录流程参考，请直接据此用 `browser_batch` 完成登录**："
            ));
            msg.push_str("\n```markdown\n");
            msg.push_str(&template(current_url));
            msg.push_str("\n```");
        }
    }

    msg
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_ws(tag: &str) -> PathBuf {
        let base = std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("target")
            .join("sop_test_tmp");
        let dir = base.join(format!("{}_{}", tag, uuid::Uuid::new_v4()));
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    #[test]
    fn test_template_contains_key_hints() {
        let t = template("http://x/#/login");
        assert!(t.contains("${secret:login_pwd}"));
        assert!(t.contains("optional"));
        assert!(t.contains("pick_first"));
        assert!(t.contains("wait_for_url"));
        assert!(t.contains("http://x/#/login"));
    }

    #[test]
    fn test_ensure_sop_creates_once() {
        let ws = tmp_ws("create");
        let (p1, created1) = ensure_sop(&ws, "http://a/#/login").expect("首次创建应成功");
        assert!(created1);
        assert!(p1.exists());

        // 二次调用不应重复创建
        let (_, created2) = ensure_sop(&ws, "http://b/#/login").expect("二次调用应成功");
        assert!(!created2);

        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn test_ensure_sop_fails_on_empty_workspace() {
        let empty = PathBuf::new();
        assert!(ensure_sop(&empty, "http://a").is_err(), "空工作区应返回错误而非静默失败");
    }

    #[test]
    fn test_auth_required_message_inlines_template_on_failure() {
        // 空工作区必然写盘失败，此时必须把模板内联进消息，保证 Agent 仍能拿到登录流程
        let empty = PathBuf::new();
        let msg = auth_required_message(&empty, "http://a/#/login", "url_redirect");
        assert!(msg.contains("AUTH_REQUIRED"));
        assert!(msg.contains("\"sop_created\":false"));
        assert!(msg.contains("sop_path\":null"));
        // 关键：模板内容被内联，不至于让 Agent 去读一个不存在的文件
        assert!(msg.contains("browser_batch"), "应内联登录流程模板");
        assert!(msg.contains("pick_first"), "内联模板应含选机构示例");
    }

    #[test]
    fn test_auth_required_message_shape() {
        let ws = tmp_ws("authmsg");
        let msg = auth_required_message(&ws, "http://a/#/login", "login_form_detected");
        assert!(msg.contains("AUTH_REQUIRED"));
        assert!(msg.contains("login_sop.md"));
        assert!(msg.contains("browser_batch"));
        let _ = std::fs::remove_dir_all(&ws);
    }
}
