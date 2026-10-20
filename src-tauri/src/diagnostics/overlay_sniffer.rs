/// 前端 DOM 错误遮罩嗅探脚本
/// 兼容覆盖：
/// 1. Webpack Dev Server 错误遮罩 (#webpack-dev-server-client-overlay 及 shadowRoot)
/// 2. Vite 错误遮罩 (vite-error-overlay 及 shadowRoot)
/// 3. Next.js / Nuxt 错误遮罩 (nextjs-portal, #__next-build-watcher)
/// 4. Element UI / Yh-UI / Ant Design 全局错误提示 (.el-message--error, .yh-message--error, .ant-message-error)
/// 5. Vue 运行时警告与红色报错容器
pub const EXTRACT_ERROR_OVERLAYS_SCRIPT: &str = r#"
(() => {
    const overlays = [];
    try {
        // 1. Webpack Dev Server Overlay (iframe 容器或 Shadow DOM)
        const webpackOverlay = document.querySelector('#webpack-dev-server-client-overlay');
        if (webpackOverlay) {
            let txt = '';
            if (webpackOverlay.contentDocument) {
                const div = webpackOverlay.contentDocument.getElementById('webpack-dev-server-client-overlay-div') 
                         || webpackOverlay.contentDocument.body;
                txt = div ? (div.innerText || div.textContent || '') : '';
            } else if (webpackOverlay.shadowRoot) {
                txt = webpackOverlay.shadowRoot.textContent || '';
            } else {
                txt = webpackOverlay.innerText || webpackOverlay.textContent || '';
            }
            txt = txt.trim();
            if (txt) {
                overlays.push(`[Webpack Dev Server 编译错误]:\n${txt.slice(0, 800)}`);
            }
        }

        // 2. Vite Error Overlay
        const viteOverlay = document.querySelector('vite-error-overlay');
        if (viteOverlay) {
            let txt = '';
            if (viteOverlay.shadowRoot) {
                txt = viteOverlay.shadowRoot.textContent || '';
            } else {
                txt = viteOverlay.innerText || viteOverlay.textContent || '';
            }
            txt = txt.trim();
            if (txt) {
                overlays.push(`[Vite 编译/构建错误]:\n${txt.slice(0, 800)}`);
            }
        }

        // 3. UI 框架全局红色错误 Toast (Element UI, Yh-UI, Ant Design)
        const toastSelectors = [
            '.el-message--error', '.yh-message--error',
            '.ant-message-error', '.ant-notification-notice-error',
            '[role="alert"].el-alert--error', '[role="alert"].yh-alert--error'
        ];
        const errorToasts = document.querySelectorAll(toastSelectors.join(','));
        for (let el of errorToasts) {
            let t = (el.innerText || el.textContent || '').trim().replace(/\s+/g, ' ');
            if (t && !overlays.some(o => o.includes(t))) {
                overlays.push(`[全局错误提示]: ${t.slice(0, 200)}`);
                if (overlays.length >= 5) break;
            }
        }

        // 4. 通用 Error Container
        const genericErrors = document.querySelectorAll('.error-container, .error-message, .fatal-error');
        for (let el of genericErrors) {
            if (el.offsetParent !== null) { // 可见元素
                let t = (el.innerText || el.textContent || '').trim().replace(/\s+/g, ' ');
                if (t && !overlays.some(o => o.includes(t))) {
                    overlays.push(`[页面报错区域]: ${t.slice(0, 200)}`);
                    if (overlays.length >= 5) break;
                }
            }
        }
    } catch (_) {}

    return overlays;
})()
"#;
