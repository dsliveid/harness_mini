use super::session::BrowserSession;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "payload")]
pub enum PageHealthState {
    /// 页面处于正常业务态，目标元素或数据表格可访问
    Ready,
    /// 页面处于未鉴权态（例如重定向到登录页、或页面弹出“登录超时”遮罩）
    Unauthenticated {
        current_url: String,
        reason: String, // "url_redirect" | "login_form_detected" | "auth_modal_detected"
    },
    /// 页面存在阻断性全局弹窗（如“请选择所属机构”、“系统维护公告”）
    ActionBlocked {
        title: String,
        description: String,
    },
    /// 页面发生不可逆错误（HTTP 500、404、网络断开）
    Error {
        code: Option<u16>,
        message: String,
    },
}

impl Default for PageHealthState {
    fn default() -> Self {
        Self::Ready
    }
}

pub const EVALUATE_PAGE_STATE_SCRIPT: &str = r#"
(() => {
    try {
        const url = window.location.href || '';
        
        // 1. 检查路由与 URL 语义 (包含 /login, #/login, #/401 等)
        if (url.includes('/login') || url.includes('#/login') || url.includes('#/401') || url.includes('/401')) {
            return {
                type: 'Unauthenticated',
                payload: { current_url: url, reason: 'url_redirect' }
            };
        }
        
        // 2. 检查视口内是否存在密码输入框（强特征：未登录表单）
        const pwdInput = document.querySelector('input[type="password"]');
        if (pwdInput) {
            const rect = pwdInput.getBoundingClientRect();
            const style = window.getComputedStyle(pwdInput);
            if (rect.width > 0 && rect.height > 0 && style.visibility !== 'hidden' && style.display !== 'none') {
                return {
                    type: 'Unauthenticated',
                    payload: { current_url: url, reason: 'login_form_detected' }
                };
            }
        }
        
        // 3. 检查全局业务错误弹窗或登录过期提示
        const authAlertKeywords = ['登录已过期', '请重新登录', 'Token已失效', 'Token已过期', '登录超时', '未授权', '登录凭证失效'];
        const modals = document.querySelectorAll('.el-message-box, .el-dialog, .ant-modal, .vxe-modal, .yh-dialog, [role="dialog"], .ant-message, .el-message');
        for (const m of modals) {
            const text = (m.innerText || m.textContent || '').trim();
            if (authAlertKeywords.some(kw => text.includes(kw))) {
                return {
                    type: 'Unauthenticated',
                    payload: { current_url: url, reason: 'auth_modal_detected' }
                };
            }
        }
        
        // 4. 检查是否出现多机构/组织选择弹窗（ActionBlocked 态）
        for (const m of modals) {
            const title = m.querySelector('.el-dialog__title, .ant-modal-title, .vxe-modal--title, .yh-dialog__title');
            const titleText = (title ? title.textContent || '' : m.textContent || '').trim();
            if (titleText.includes('选择机构') || titleText.includes('切换科室') || titleText.includes('组织选择') || titleText.includes('请选择组织')) {
                return {
                    type: 'ActionBlocked',
                    payload: {
                        title: titleText.slice(0, 50),
                        description: '检测到机构/组织选择弹窗，需要确认所属机构后方可进入业务页面'
                    }
                };
            }
        }

        // 5. 检查 HTTP 500 / 404 / 崩溃静态错误页
        const bodyText = (document.body ? document.body.innerText || '' : '').trim();
        if (document.title.includes('500') || document.title.includes('404') || bodyText.startsWith('Whitelabel Error Page')) {
            return {
                type: 'Error',
                payload: {
                    code: document.title.includes('500') ? 500 : 404,
                    message: document.title || '服务器异常或页面不存在'
                }
            };
        }

        return { type: 'Ready' };
    } catch (e) {
        return { type: 'Ready' };
    }
})()
"#;

pub async fn evaluate_page_state(session: &BrowserSession) -> PageHealthState {
    let sid = session.target_session_id();
    let res = session
        .cdp()
        .send(
            "Runtime.evaluate",
            Some(json!({
                "expression": EVALUATE_PAGE_STATE_SCRIPT,
                "returnByValue": true
            })),
            Some(sid),
            Duration::from_secs(5),
        )
        .await;

    if let Ok(eval_val) = res {
        if let Some(val) = eval_val.get("result").and_then(|r| r.get("value")) {
            if let Ok(state) = serde_json::from_value::<PageHealthState>(val.clone()) {
                return state;
            }
        }
    }

    PageHealthState::Ready
}
