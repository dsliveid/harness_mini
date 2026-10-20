use super::auth::AuthContract;
use super::evaluator::{evaluate_page_state, PageHealthState};
use super::session::BrowserSession;
use serde_json::json;
use std::time::Duration;

pub const AUTO_FILL_LOGIN_FORM_SCRIPT: &str = r#"
((username, password) => {
    try {
        const setValue = (el, val) => {
            if (!el) return;
            el.focus();
            const proto = Object.getPrototypeOf(el);
            const desc = Object.getOwnPropertyDescriptor(proto, 'value') ||
                         Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value');
            if (desc && desc.set) {
                desc.set.call(el, val);
            } else {
                el.value = val;
            }
            el.dispatchEvent(new Event('input', { bubbles: true, composed: true }));
            el.dispatchEvent(new Event('change', { bubbles: true, composed: true }));
            el.blur();
        };

        // 1. 定位用户名输入框
        const userSelectors = [
            'input[name="username"]',
            'input[name="account"]',
            'input[name="user"]',
            'input[placeholder*="账号"]',
            'input[placeholder*="用户名"]',
            'input[placeholder*="账户"]',
            'input[type="text"]:not([readonly])'
        ];
        let userInput = null;
        for (const sel of userSelectors) {
            const el = document.querySelector(sel);
            if (el && el.getBoundingClientRect().height > 0) {
                userInput = el;
                break;
            }
        }

        // 2. 定位密码输入框
        const pwdInput = document.querySelector('input[type="password"]');

        if (!userInput || !pwdInput) {
            return { success: false, error: "未能在页面中同时定位到用户名与密码输入框" };
        }

        setValue(userInput, username);
        setValue(pwdInput, password);

        // 3. 定位并点击提交按钮
        const submitSelectors = [
            'button[type="submit"]',
            'button.el-button--primary',
            'button.ant-btn-primary',
            'button.yh-button--primary',
            'button',
            'input[type="submit"]'
        ];
        let submitBtn = null;
        for (const sel of submitSelectors) {
            const btns = document.querySelectorAll(sel);
            for (const b of btns) {
                const txt = (b.innerText || b.textContent || '').trim().replace(/\s+/g, '');
                if (txt.includes('登录') || txt.toLowerCase().includes('login') || txt.includes('进入')) {
                    submitBtn = b;
                    break;
                }
            }
            if (submitBtn) break;
        }

        if (submitBtn) {
            submitBtn.click();
            return { success: true, clicked_button: true };
        } else {
            // 回退到按回车提交
            pwdInput.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', code: 'Enter', keyCode: 13, which: 13, bubbles: true }));
            return { success: true, pressed_enter: true };
        }
    } catch (e) {
        return { success: false, error: String(e) };
    }
})
"#;

pub const GENERIC_SELECT_MODAL_ITEM_SCRIPT: &str = r#"
((matchText, confirmBtn) => {
    try {
        if (!matchText) return { success: false, error: "未指定匹配文本" };
        const cleanName = matchText.trim();
        const candidates = document.querySelectorAll(
            '.el-dialog li, .el-dialog tr, .el-dialog .org-item, .el-dialog td, .el-dialog span, .el-dialog div, ' +
            '.ant-modal li, .ant-modal tr, .ant-modal td, .ant-modal span, .ant-modal div, ' +
            '.vxe-modal li, .vxe-modal tr, .vxe-modal td, .vxe-modal div, ' +
            '[role="dialog"] li, [role="dialog"] tr, [role="dialog"] [role="option"], [role="dialog"] div'
        );
        let clicked = false;
        let matchedText = '';
        for (const el of candidates) {
            const txt = (el.innerText || el.textContent || '').trim();
            if (txt && txt.length <= 60 && txt.includes(cleanName)) {
                el.click();
                clicked = true;
                matchedText = txt.slice(0, 50);
                break;
            }
        }
        if (!clicked) return { success: false, error: `未在弹窗中匹配到包含【${cleanName}】的目标选项` };

        if (confirmBtn) {
            const btns = document.querySelectorAll(
                '.el-dialog button, .ant-modal button, .vxe-modal button, [role="dialog"] button, button.el-button--primary'
            );
            for (const b of btns) {
                const bTxt = (b.innerText || b.textContent || '').trim();
                if (bTxt.includes(confirmBtn)) {
                    b.click();
                    break;
                }
            }
        }
        return { success: true, matched_text: matchedText };
    } catch (e) {
        return { success: false, error: String(e) };
    }
})
"#;

pub const DISMISS_NOTICE_SCRIPT: &str = r#"
(() => {
    try {
        const keywords = ['我知道了', '关闭', '确定', '同意', '知道了'];
        const btns = document.querySelectorAll('.el-message-box button, .el-dialog button, .ant-modal button, [role="dialog"] button');
        for (const b of btns) {
            const txt = (b.innerText || b.textContent || '').trim();
            if (keywords.some(kw => txt.includes(kw))) {
                b.click();
                return { success: true, dismissed: txt };
            }
        }
        return { success: false };
    } catch (e) {
        return { success: false, error: String(e) };
    }
})()
"#;

pub async fn execute_auth_pipeline(
    session: &BrowserSession,
    contract: &AuthContract,
) -> Result<(), String> {
    let sid = Some(session.target_session_id());

    // 1. 确定性导航至契约指定的登录入口
    session.navigate(&contract.login_url).await?;
    tokio::time::sleep(Duration::from_millis(1000)).await;

    // 2. 注入账号密码并点击登录
    let fill_expr = format!(
        "({AUTO_FILL_LOGIN_FORM_SCRIPT})({}, {})",
        serde_json::to_string(&contract.credentials.username).unwrap(),
        serde_json::to_string(&contract.credentials.password_cipher).unwrap()
    );

    let eval_res = session
        .cdp()
        .send(
            "Runtime.evaluate",
            Some(json!({
                "expression": fill_expr,
                "returnByValue": true
            })),
            sid,
            Duration::from_secs(10),
        )
        .await?;

    let fill_val = eval_res
        .get("result")
        .and_then(|r| r.get("value"))
        .cloned()
        .unwrap_or(json!({}));

    if fill_val.get("success").and_then(|s| s.as_bool()) != Some(true) {
        let err_msg = fill_val
            .get("error")
            .and_then(|e| e.as_str())
            .unwrap_or("自动填充表单失败");
        return Err(format!("自愈流水线失败: {err_msg}"));
    }

    // 3. 等待网络请求与状态流转
    tokio::time::sleep(Duration::from_millis(1200)).await;

    // 4. 后置交互处理：通用动作链 (actions) 或向下兼容 org_name
    if let Some(ref post) = contract.post_login {
        if let Some(ref actions) = post.actions {
            for action in actions {
                match action.action_type.as_str() {
                    "select_modal_item" => {
                        let match_text = action.match_text.as_deref().unwrap_or("");
                        let confirm_btn = action.confirm_btn.as_deref().unwrap_or("确定");
                        let start = std::time::Instant::now();
                        let timeout = Duration::from_secs(5);
                        while start.elapsed() < timeout {
                            let state = evaluate_page_state(session).await;
                            if matches!(state, PageHealthState::ActionBlocked { .. }) || state == PageHealthState::Ready {
                                let expr = format!(
                                    "({GENERIC_SELECT_MODAL_ITEM_SCRIPT})({}, {})",
                                    serde_json::to_string(match_text).unwrap(),
                                    serde_json::to_string(confirm_btn).unwrap()
                                );
                                let _ = session
                                    .cdp()
                                    .send(
                                        "Runtime.evaluate",
                                        Some(json!({ "expression": expr, "returnByValue": true })),
                                        sid,
                                        Duration::from_secs(5),
                                    )
                                    .await;
                                tokio::time::sleep(Duration::from_millis(600)).await;
                                break;
                            }
                            tokio::time::sleep(Duration::from_millis(300)).await;
                        }
                    }
                    "dismiss_notice" => {
                        let _ = session
                            .cdp()
                            .send(
                                "Runtime.evaluate",
                                Some(json!({ "expression": DISMISS_NOTICE_SCRIPT, "returnByValue": true })),
                                sid,
                                Duration::from_secs(5),
                            )
                            .await;
                        tokio::time::sleep(Duration::from_millis(400)).await;
                    }
                    "click" => {
                        if let Some(ref sel) = action.selector {
                            let _ = session.interact("click", sel, None).await;
                            tokio::time::sleep(Duration::from_millis(400)).await;
                        }
                    }
                    "wait" => {
                        tokio::time::sleep(Duration::from_millis(800)).await;
                    }
                    _ => {}
                }
            }
        } else if let Some(ref org_name) = post.org_name {
            // 向下兼容旧 v2.0 单一机构选择
            let start = std::time::Instant::now();
            let timeout = Duration::from_secs(4);
            while start.elapsed() < timeout {
                let state = evaluate_page_state(session).await;
                if matches!(state, PageHealthState::ActionBlocked { .. }) || state == PageHealthState::Ready {
                    let select_expr = format!(
                        "({GENERIC_SELECT_MODAL_ITEM_SCRIPT})({}, \"确定\")",
                        serde_json::to_string(org_name).unwrap()
                    );
                    let _ = session
                        .cdp()
                        .send(
                            "Runtime.evaluate",
                            Some(json!({ "expression": select_expr, "returnByValue": true })),
                            sid,
                            Duration::from_secs(5),
                        )
                        .await;
                    tokio::time::sleep(Duration::from_millis(800)).await;
                    break;
                }
                tokio::time::sleep(Duration::from_millis(400)).await;
            }
        }
    }

    // 5. 断言成功跳出登录态
    let start_assert = std::time::Instant::now();
    let timeout_assert = Duration::from_secs(6);
    while start_assert.elapsed() < timeout_assert {
        let state = evaluate_page_state(session).await;
        if state == PageHealthState::Ready {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    // 最终兜底校验：检查当前 URL 是否已经跳离登录页
    let curr_state = session.get_debug_state().await?;
    let target_assert = contract
        .post_login
        .as_ref()
        .and_then(|p| p.success_assert_url.as_ref());

    if let Some(assert_rule) = target_assert {
        let parts: Vec<&str> = assert_rule.split('|').map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
        if parts.iter().any(|p| curr_state.url.contains(p)) {
            return Ok(());
        }
    }

    if !curr_state.url.contains("/login") && !curr_state.url.contains("#/login") && !curr_state.url.contains("#/401") {
        return Ok(());
    }

    Err("自愈流水线执行完成，但页面仍停留在登录或拦截态，可能需要人工协同验证".to_string())
}
