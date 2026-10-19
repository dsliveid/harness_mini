pub const EXTRACT_TREE_SCRIPT: &str = r#"
(() => {
    // 1. 可交互元素提取
    const selectors = [
        'input', 'textarea', 'select', 'button', 'a[href]',
        '[role="button"]', '[role="checkbox"]', '[role="menuitem"]',
        '[onclick]', '[tabindex]:not([tabindex="-1"])'
    ];
    const elements = document.querySelectorAll(selectors.join(','));
    const items = [];
    let count = 0;

    for (let el of elements) {
        try {
            const style = window.getComputedStyle(el);
            if (style.display === 'none' || style.visibility === 'hidden' || style.opacity === '0') {
                continue;
            }
            const rect = el.getBoundingClientRect();
            if (rect.width === 0 && rect.height === 0) {
                continue;
            }

            count++;
            const refId = `@e${count}`;
            el.setAttribute('data-harness-ref', refId);

            const tag = el.tagName.toLowerCase();
            const type = el.getAttribute('type') || '';
            const name = el.getAttribute('name') || '';
            const placeholder = el.getAttribute('placeholder') || '';
            let text = (el.innerText || el.textContent || '').trim().replace(/\s+/g, ' ');
            if (text.length > 60) text = text.slice(0, 60) + '...';
            const value = el.value !== undefined && el.value !== null ? String(el.value) : '';
            const href = el.getAttribute('href') || '';

            items.push({
                ref: refId,
                tag,
                type: type || undefined,
                name: name || undefined,
                placeholder: placeholder || undefined,
                text: text || undefined,
                value: value || undefined,
                href: href ? href.slice(0, 60) : undefined,
            });

            if (count >= 80) break; // 防止超长 DOM 消耗过多 Token
        } catch (_) {}
    }

    // 2. 页面通知、警告与空状态提示
    const notices = [];
    try {
        const noticeSelectors = [
            '.el-alert__title', '.el-alert__description', '.el-message__content',
            '.ant-alert-message', '.ant-alert-description', '.ant-message-custom-content',
            '.el-empty__description', '.ant-empty-description',
            '.vxe-table--empty-text', '.vxe-table--empty-placeholder',
            '[role="alert"]', '.empty-text', '.empty-data'
        ];
        const noticeNodes = document.querySelectorAll(noticeSelectors.join(','));
        for (let nel of noticeNodes) {
            let t = (nel.innerText || nel.textContent || '').trim().replace(/\s+/g, ' ');
            if (t && t.length > 0 && !notices.includes(t)) {
                notices.push(t.slice(0, 120));
                if (notices.length >= 6) break;
            }
        }
    } catch (_) {}

    // 3. 业务数据表格概览 (支持标准 table, el-table, vxe-table, ant-table)
    const tables = [];
    try {
        const tableNodes = document.querySelectorAll('table, .el-table, .vxe-table, .ant-table');
        for (let tbl of tableNodes) {
            // 提取表头
            const thNodes = tbl.querySelectorAll('th, .vxe-header--column, .el-table__header th');
            const headers = [];
            for (let th of thNodes) {
                let ht = (th.innerText || th.textContent || '').trim().replace(/\s+/g, ' ');
                if (ht && !headers.includes(ht)) {
                    headers.push(ht.slice(0, 25));
                }
            }

            // 提取行数据 (最多前 10 行)
            const rows = [];
            const trNodes = tbl.querySelectorAll('tbody tr, .vxe-body--row, .el-table__body-wrapper tbody tr');
            let rCount = 0;
            for (let tr of trNodes) {
                const cellNodes = tr.querySelectorAll('td, .vxe-body--column, .el-table__cell');
                const rowData = [];
                for (let c of cellNodes) {
                    let ct = (c.innerText || c.textContent || '').trim().replace(/\s+/g, ' ');
                    rowData.push(ct.slice(0, 35));
                }
                if (rowData.some(x => x.length > 0)) {
                    rows.push(rowData);
                    rCount++;
                    if (rCount >= 10) break;
                }
            }

            if (headers.length > 0 || rows.length > 0) {
                let mdTable = '';
                if (headers.length > 0) {
                    mdTable += '| ' + headers.join(' | ') + ' |\n';
                    mdTable += '| ' + headers.map(() => '---').join(' | ') + ' |\n';
                }
                for (let r of rows) {
                    const padded = headers.length > 0
                        ? headers.map((_, i) => r[i] || '')
                        : r;
                    mdTable += '| ' + padded.join(' | ') + ' |\n';
                }
                if (mdTable.trim().length > 0) {
                    tables.push(mdTable.trim());
                    if (tables.length >= 3) break;
                }
            }
        }
    } catch (_) {}

    return {
        title: document.title,
        url: window.location.href,
        elements: items,
        notices: notices,
        tables: tables
    };
})()
"#;

pub const INJECT_RECORDER_SCRIPT: &str = r#"
(() => {
    if (window.__harness_recorder_installed) return;
    window.__harness_recorder_installed = true;
    window.__harness_recorded_steps = window.__harness_recorded_steps || [];

    function getBestSelector(el) {
        if (!el || el === document.body || el === document.documentElement) return '';
        if (el.getAttribute('data-testid')) return `[data-testid="${el.getAttribute('data-testid')}"]`;
        if (el.getAttribute('data-test')) return `[data-test="${el.getAttribute('data-test')}"]`;
        if (el.id && !el.id.match(/\d{5,}/)) return `#${el.id}`;

        const tag = el.tagName.toLowerCase();
        if (tag === 'input' || tag === 'textarea' || tag === 'select') {
            const type = el.getAttribute('type') || '';
            const name = el.getAttribute('name');
            const placeholder = el.getAttribute('placeholder');
            if (type === 'password') return 'input[type="password"]';
            if (name) return `${tag}[name="${name}"]`;
            if (placeholder) return `${tag}[placeholder="${placeholder}"]`;
            if (type) return `${tag}[type="${type}"]`;
        }

        const text = (el.innerText || el.textContent || '').trim().replace(/\s+/g, ' ');
        if (text && text.length > 0 && text.length <= 30) {
            if (tag === 'button' || el.getAttribute('role') === 'button' || el.classList.contains('el-button') || el.classList.contains('ant-btn')) {
                return `button:has-text('${text}')`;
            }
            if (el.closest('.el-dialog, .el-select-dropdown, .vxe-modal, .ant-modal, .dropdown, ul, table')) {
                const cls = el.className && typeof el.className === 'string' ? '.' + el.className.trim().split(/\s+/)[0] : tag;
                return `${cls}:has-text('${text}')`;
            }
        }

        if (el.className && typeof el.className === 'string') {
            const classes = el.className.trim().split(/\s+/).filter(c => !c.match(/\d{5,}/) && !c.includes('active') && !c.includes('focus'));
            if (classes.length > 0) {
                return `${tag}.${classes.slice(0, 2).join('.')}`;
            }
        }
        return tag;
    }

    document.addEventListener('change', (e) => {
        const el = e.target;
        if (!el || !['input', 'textarea', 'select'].includes(el.tagName.toLowerCase())) return;
        const sel = getBestSelector(el);
        const val = el.value || '';
        const isPwd = el.type === 'password' || sel.includes('password');
        const desc = isPwd ? '输入密码' : (el.placeholder ? `输入 "${el.placeholder}"` : `输入: ${val}`);

        const steps = window.__harness_recorded_steps;
        const last = steps[steps.length - 1];
        if (last && last.action === 'type' && last.selector === sel) {
            last.value = val;
            last.desc = desc;
        } else {
            steps.push({
                order: steps.length + 1,
                action: 'type',
                selector: sel,
                value: val,
                is_encrypted: isPwd,
                desc: desc
            });
        }
    }, true);

    document.addEventListener('click', (e) => {
        let el = e.target;
        if (!el) return;
        const clickable = el.closest('button, a, [role="button"], input[type="submit"], input[type="button"], .el-button, .ant-btn, .el-dropdown-item, .org-item, tr, li');
        if (clickable) el = clickable;

        const sel = getBestSelector(el);
        const text = (el.innerText || el.textContent || '').trim().replace(/\s+/g, ' ');
        const desc = text ? `点击 "${text.slice(0, 20)}"` : `点击 ${sel}`;

        const steps = window.__harness_recorded_steps;
        steps.push({
            order: steps.length + 1,
            action: 'click',
            selector: sel,
            desc: desc
        });
    }, true);
})();
"#;

pub const EXTRACT_RECORDED_STEPS_SCRIPT: &str = r#"
(() => {
    return window.__harness_recorded_steps || [];
})()
"#;

pub fn build_interact_script(action: &str, target: &str, value: Option<&str>) -> String {
    let clean_target = target.trim();
    let selector = if clean_target.starts_with('@') {
        format!("[data-harness-ref=\"{clean_target}\"]")
    } else {
        clean_target.to_string()
    };

    let escaped_selector = selector.replace('"', "\\\"");
    let escaped_value = value.unwrap_or("").replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n");

    let find_target_fn = r#"
        const findTarget = (sel) => {
            if (sel.includes(':has-text(') || sel.includes(':contains(')) {
                const m = sel.match(/^(.*?):(has-text|contains)\(['"](.*)['"]\)$/);
                if (m) {
                    const baseTag = m[1].trim() || '*';
                    const textToFind = m[3].trim();
                    const candidates = document.querySelectorAll(baseTag);
                    for (let el of candidates) {
                        if ((el.innerText || el.textContent || '').includes(textToFind)) {
                            return el;
                        }
                    }
                }
            }
            return document.querySelector(sel);
        };
    "#;

    match action {
        "click" => format!(
            r#"(() => {{
                {find_target_fn}
                const el = findTarget("{escaped_selector}");
                if (!el) return {{ success: false, error: "未找到目标元素: {clean_target}" }};
                el.scrollIntoView({{ behavior: 'instant', block: 'center' }});
                el.focus();
                el.click();
                return {{ success: true }};
            }})()"#
        ),
        "type" => format!(
            r#"(() => {{
                {find_target_fn}
                const el = findTarget("{escaped_selector}");
                if (!el) return {{ success: false, error: "未找到目标元素: {clean_target}" }};
                el.scrollIntoView({{ behavior: 'instant', block: 'center' }});
                el.focus();
                el.value = "{escaped_value}";
                el.dispatchEvent(new Event('input', {{ bubbles: true }}));
                el.dispatchEvent(new Event('change', {{ bubbles: true }}));
                return {{ success: true }};
            }})()"#
        ),
        "press_enter" => format!(
            r#"(() => {{
                {find_target_fn}
                const el = findTarget("{escaped_selector}");
                if (el) {{
                    el.scrollIntoView({{ behavior: 'instant', block: 'center' }});
                    el.focus();
                }}
                const target = el || document.activeElement || document.body;
                target.dispatchEvent(new KeyboardEvent('keydown', {{ key: 'Enter', code: 'Enter', keyCode: 13, which: 13, bubbles: true }}));
                target.dispatchEvent(new KeyboardEvent('keypress', {{ key: 'Enter', code: 'Enter', keyCode: 13, which: 13, bubbles: true }}));
                target.dispatchEvent(new KeyboardEvent('keyup', {{ key: 'Enter', code: 'Enter', keyCode: 13, which: 13, bubbles: true }}));
                return {{ success: true }};
            }})()"#
        ),
        "wait_for_selector" => format!(
            r#"(() => {{
                {find_target_fn}
                const el = findTarget("{escaped_selector}");
                return {{ success: !!el, found: !!el }};
            }})()"#
        ),
        _ => format!(
            r#"(() => ({{ success: false, error: "不支持的操作类型: {action}" }}))()"#
        ),
    }
}
