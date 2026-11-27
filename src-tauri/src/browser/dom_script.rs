pub const EXTRACT_TREE_SCRIPT: &str = r#"
(() => {
    const vWidth = window.innerWidth || document.documentElement.clientWidth || 1440;
    const vHeight = window.innerHeight || document.documentElement.clientHeight || 960;
    const scrollX = window.scrollX || window.pageXOffset || document.documentElement.scrollLeft || 0;
    const scrollY = window.scrollY || window.pageYOffset || document.documentElement.scrollTop || 0;
    const totalWidth = Math.max(document.documentElement.scrollWidth, document.body ? document.body.scrollWidth : 0, vWidth);
    const totalHeight = Math.max(document.documentElement.scrollHeight, document.body ? document.body.scrollHeight : 0, vHeight);

    // 1. 可交互元素提取 (广谱候选 + 可点击性判定)
    //
    // 历史缺陷（2026-11-24 实测暴露）：旧选择器〔仅 semiconductor 元素〕不含 li/div/卡片/下拉项，
    // 导致机构选择页 4 个机构卡片、Element Plus 下拉选项、弹窗全部不可见，
    // Agent 拿不到可用 ref 只能退化为截图 + 识别。
    // 现改为：广谱查询 → JS 侧可点击性启发式过滤 → 叶子优先去重 → 可点击元素优先配额。
    const selectors = [
        // 表单与基础控件
        'input', 'textarea', 'select', 'button', 'a[href]',
        // ARIA 语义角色
        '[role="button"]', '[role="checkbox"]', '[role="menuitem"]', '[role="option"]',
        '[role="tab"]', '[role="link"]', '[role="switch"]',
        // 显式可交互
        '[onclick]', '[tabindex]:not([tabindex="-1"])',
        // 常见 UI 库的可点击容器：卡片 / 列表项 / 菜单 / 下拉选项 / 表格行
        'li', '.el-card', '.el-select-dropdown__item', '.ant-select-item-option',
        '.el-menu-item', '.el-submenu__title', '.el-tabs__item',
        '.ant-menu-item', '.ant-card', '.el-table__row', '.vxe-body--row',
        '.el-tree-node__content', '.ant-tree-node-content-wrapper',
        '.el-dialog', '.el-drawer', '[role="dialog"]'
    ];
    const rawElements = document.querySelectorAll(selectors.join(','));
    const items = [];
    let hiddenCount = 0;

    // ---- 可点击性判定：命中任一条即视为"可交互候选" ----
    const looksInteractive = (el) => {
        const tag = el.tagName.toLowerCase();
        // 表单控件与原生可交互标签直接放行
        if (['input', 'textarea', 'select', 'button', 'a'].includes(tag)) return true;
        const role = (el.getAttribute('role') || '').toLowerCase();
        if (['button', 'option', 'menuitem', 'tab', 'checkbox', 'switch', 'link'].includes(role)) return true;
        if (el.hasAttribute('onclick') || el.hasAttribute('tabindex')) return true;
        // 鼠标指针样式是最可靠的"可点击"信号
        try {
            if (window.getComputedStyle(el).cursor === 'pointer') return true;
        } catch (_) {}
        // class 语义兜底（覆盖各类 UI 库命名习惯）
        const cls = typeof el.className === 'string' ? el.className : '';
        if (/btn|button|card|item|tab|menu|option|link|cell|row|node|choice|select/i.test(cls)) return true;
        return false;
    };

    // ---- 弹窗归属判定：元素是否位于「可见」弹窗容器内 ----
    //
    // 实测痛点（2026-11-24）：弹窗里 3 个 `placeholder="请输入"` 的 input 与页面上同名 input
    // 指纹一致，列表里只以 `_2`/`_3` 后缀区分，Agent 无法判断该填哪一个。
    // 必须显式标注归属弹窗，并在 markdown 中分组展示。
    const DIALOG_SEL = '[class*="dialog"],[class*="modal"],[class*="drawer"],[class*="popup"],[role="dialog"]';
    const dialogContainerOf = (el) => {
        let node = el;
        let depth = 0;
        while (node && node.nodeType === 1 && depth < 12) {
            try {
                if (node.matches && node.matches(DIALOG_SEL)) return node;
            } catch (_) {}
            node = node.parentElement;
            depth++;
        }
        return null;
    };
    // 缓存已判定过的弹窗容器可见性，避免对同一容器重复取样式
    const dialogVisibleCache = new Map();
    const isInDialog = (el) => {
        const box = dialogContainerOf(el);
        if (!box) return false;
        if (dialogVisibleCache.has(box)) return dialogVisibleCache.get(box);
        let vis = false;
        try {
            const r = box.getBoundingClientRect();
            const st = window.getComputedStyle(box);
            vis = r.width > 0 && r.height > 0
                && st.display !== 'none' && st.visibility !== 'hidden'
                && st.opacity !== '0' && parseFloat(st.opacity) !== 0;
        } catch (_) { vis = false; }
        dialogVisibleCache.set(box, vis);
        return vis;
    };

    // ---- 叶子优先：父容器与其唯一子元素文本相同则丢弃父容器，避免父子重复计入 ----
    const isDuplicateParent = (el, normalizedText) => {
        if (!normalizedText) return false;
        for (const c of el.children) {
            const ct = (c.innerText || c.textContent || '').trim().replace(/\s+/g, ' ');
            if (ct && ct === normalizedText) return true;
        }
        return false;
    };

    const elements = [];
    for (const el of rawElements) {
        try {
            if (!looksInteractive(el)) continue;
            const txt = (el.innerText || el.textContent || '').trim().replace(/\s+/g, ' ');
            const tag = el.tagName.toLowerCase();
            const isFormControl = ['input', 'textarea', 'select', 'button'].includes(tag);
            // 无文本且非表单控件 → 无操作价值，丢弃
            if (!txt && !isFormControl) continue;
            if (isDuplicateParent(el, txt)) continue;
            elements.push(el);
        } catch (_) {}
    }

    // 配额优先序：表单控件（输入/按钮）最常用，必须优先占位；
    // 其次才是卡片/菜单/下拉项等容器类可点击元素。
    // 否则卡片较多时会把 100 个名额占满，导致关键输入框丢失。
    elements.sort((a, b) => {
        const rank = (el) => {
            const t = el.tagName.toLowerCase();
            if (['input', 'textarea', 'select'].includes(t)) return 0;
            if (t === 'button' || t === 'a') return 1;
            return 2;
        };
        return rank(a) - rank(b);
    });

    // ============ 稳定 ref 指纹机制 ============
    // 历史缺陷：旧实现按遍历序号生成 @e1..@eN，任一前置元素隐藏/显示即导致其后全部重编号，
    // Agent 缓存的 @e5 会漂移到完全不同的元素上。改为「元素特征指纹 → 哈希」后 ref 抗 DOM 波动。
    const refMap = {};
    const usedRefs = {};

    // FNV-1a 变体双通道哈希，输出 8 位十六进制
    const hashText = (str) => {
        let h1 = 0x811c9dc5, h2 = 0x01000193;
        for (let i = 0; i < str.length; i++) {
            const c = str.charCodeAt(i);
            h1 = Math.imul(h1 ^ c, 0x01000193) >>> 0;
            h2 = Math.imul((h2 + c) >>> 0, 0x85ebca6b) >>> 0;
        }
        return ((h1 ^ h2) >>> 0).toString(16).padStart(8, '0');
    };

    // 动态 ID 特征识别：超过 4 位连续数字 / 长十六进制 / 框架前缀
    const isDynamicId = (id) => {
        if (!id) return true;
        if (/\d{4,}/.test(id)) return true;
        if (/^[0-9a-f]{8,}$/i.test(id)) return true;
        if (/^(react|ember|vue-|radix-|headlessui-|:r)/i.test(id)) return true;
        return false;
    };

    // 结构路径兜底（仅在元素缺少一切可辨识特征时使用）
    const buildPath = (el) => {
        const parts = [];
        let node = el;
        let depth = 0;
        while (node && node.nodeType === 1 && depth < 6) {
            const parent = node.parentElement;
            const t = node.tagName.toLowerCase();
            if (!parent) { parts.unshift(t); break; }
            const idx = Array.prototype.indexOf.call(parent.children, node) + 1;
            parts.unshift(t + ':nth-child(' + idx + ')');
            node = parent;
            depth++;
        }
        return parts.join('>');
    };

    // 指纹生成：按稳定性优先级依次取证
    const buildFingerprint = (el, tag) => {
        for (const attr of ['data-testid', 'data-test', 'data-cy', 'data-qa']) {
            const v = el.getAttribute(attr);
            if (v) return 'testid:' + v;
        }
        const id = el.getAttribute('id');
        if (id && !isDynamicId(id)) return 'id:' + id;
        const name = el.getAttribute('name');
        if (name) return 'name:' + tag + ':' + name;
        const aria = el.getAttribute('aria-label') || el.getAttribute('aria-labelledby');
        if (aria) return 'aria:' + tag + ':' + aria;
        const type = el.getAttribute('type') || '';
        const ph = el.getAttribute('placeholder') || '';
        const href = el.getAttribute('href') || '';
        let text = (el.innerText || el.textContent || '').trim().replace(/\s+/g, ' ');
        if (text.length > 40) text = text.slice(0, 40);
        if (type || ph || text || href) {
            return 'feat:' + tag + ':' + type + ':' + ph + ':' + text + ':' + href.slice(0, 60);
        }
        return 'path:' + buildPath(el);
    };

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

            const tagName = el.tagName.toLowerCase();

            // 生成稳定 ref 并处理同指纹碰撞
            const fp = buildFingerprint(el, tagName);
            let refId = '@e' + hashText(fp);
            if (usedRefs[refId]) {
                usedRefs[refId] += 1;
                refId = refId + '_' + usedRefs[refId];
            } else {
                usedRefs[refId] = 1;
            }
            el.setAttribute('data-harness-ref', refId);
            refMap[refId] = {
                tag: tagName,
                name: el.getAttribute('name') || '',
                placeholder: el.getAttribute('placeholder') || '',
                text: (el.innerText || el.textContent || '').trim().replace(/\s+/g, ' ').slice(0, 40),
                path: buildPath(el)
            };

            // 检查元素是否在当前视口内可见 (In-Viewport)
            const inViewport = (
                rect.bottom > 0 &&
                rect.right > 0 &&
                rect.top < vHeight &&
                rect.left < vWidth
            );

            if (inViewport) {
                if (items.length < 100) {
                    const type = el.getAttribute('type') || '';
                    const name = el.getAttribute('name') || '';
                    const placeholder = el.getAttribute('placeholder') || '';
                    let text = (el.innerText || el.textContent || '').trim().replace(/\s+/g, ' ');
                    if (text.length > 60) text = text.slice(0, 60) + '...';
                    const value = el.value !== undefined && el.value !== null ? String(el.value) : '';
                    const href = el.getAttribute('href') || '';

                    items.push({
                        ref: refId,
                        tag: tagName,
                        type: type || undefined,
                        name: name || undefined,
                        placeholder: placeholder || undefined,
                        text: text || undefined,
                        value: value || undefined,
                        href: href ? href.slice(0, 60) : undefined,
                        inDialog: isInDialog(el) || undefined,
                    });
                } else {
                    hiddenCount++;
                }
            } else {
                hiddenCount++;
            }
        } catch (_) {}
    }

    // 暴露反查映射：DOM 重渲染后 data-harness-ref 属性会丢失，靠此表二次定位
    window.__harness_ref_map = refMap;

    // 2. 页面通知、警告、空状态以及关键错误遮罩 (Webpack / Vite / UI Error Overlays)
    const notices = [];
    const dom_overlays = [];
    try {
        // Webpack Dev Server 编译错误遮罩 (iframe 容器或 Shadow DOM)
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
                const item = `[Webpack 编译错误]: ${txt.slice(0, 500)}`;
                dom_overlays.push(item);
                notices.push(item);
            }
        }

        // Vite 构建错误遮罩
        const viteOverlay = document.querySelector('vite-error-overlay');
        if (viteOverlay) {
            let txt = viteOverlay.shadowRoot ? viteOverlay.shadowRoot.textContent : (viteOverlay.innerText || viteOverlay.textContent);
            txt = (txt || '').trim();
            if (txt) {
                const item = `[Vite 构建错误]: ${txt.slice(0, 500)}`;
                dom_overlays.push(item);
                notices.push(item);
            }
        }

        const noticeSelectors = [
            '.el-message--error', '.yh-message--error', '.ant-message-error',
            '.el-alert__title', '.el-alert__description', '.el-message__content',
            '.yh-alert__title', '.yh-alert__description', '.yh-message__content',
            '.ant-alert-message', '.ant-alert-description', '.ant-message-custom-content',
            // 空状态：独立空态组件
            '.el-empty__description', '.ant-empty-description',
            // 空状态：表格空数据（Element Plus / vxe-table / 通用）
            // 缺这几项会导致「暂无数据」完全不可见，Agent 只能靠截图判断，
            // 这是 2026-11-24 实测中被迫截图的首要原因之一。
            '.el-table__empty-text', '.el-table__empty-block', '.ant-table-placeholder',
            '.vxe-table--empty-text', '.vxe-table--empty-placeholder',
            '[role="alert"]', '.empty-text', '.empty-data',
            // 弹窗 / 抽屉 / 遮罩（Element Plus 2.x 已移除 __wrapper，改用 el-overlay）
            '.el-overlay', '.el-dialog__title', '.el-drawer__title',
            // 加载态：与「空数据」区分开，避免把"还在加载"误判成"没有数据"
            '.el-loading-mask', '.el-loading-spinner'
        ];
        const noticeNodes = document.querySelectorAll(noticeSelectors.join(','));
        for (let nel of noticeNodes) {
            let t = (nel.innerText || nel.textContent || '').trim().replace(/\s+/g, ' ');
            if (t && t.length > 0 && !notices.includes(t)) {
                if (nel.classList.contains('el-message--error') || nel.classList.contains('yh-message--error') || nel.classList.contains('ant-message-error')) {
                    dom_overlays.push(`[全局错误弹窗]: ${t.slice(0, 200)}`);
                }
                // 加载态单独标注：明确区分「正在加载」与「确实为空」
                if (nel.classList.contains('el-loading-mask') || nel.classList.contains('el-loading-spinner')) {
                    notices.push(`[加载中] ${t.slice(0, 80)}`);
                } else if (nel.classList.contains('el-table__empty-text') || nel.classList.contains('el-table__empty-block')
                    || nel.classList.contains('ant-table-placeholder') || nel.classList.contains('vxe-table--empty-text')
                    || nel.classList.contains('el-empty__description') || nel.classList.contains('ant-empty-description')) {
                    notices.push(`[空数据] ${t.slice(0, 80)}`);
                } else {
                    notices.push(t.slice(0, 120));
                }
                if (notices.length >= 12) break;
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

    const totalPages = Math.max(1, Math.ceil(totalHeight / vHeight));
    const currentPage = Math.min(totalPages, Math.floor(scrollY / vHeight) + 1);

    return {
        title: document.title,
        url: window.location.href,
        viewport: {
            scroll_x: Math.round(scrollX),
            scroll_y: Math.round(scrollY),
            viewport_width: Math.round(vWidth),
            viewport_height: Math.round(vHeight),
            total_width: Math.round(totalWidth),
            total_height: Math.round(totalHeight),
            current_page: currentPage,
            total_pages: totalPages,
            has_more_down: (scrollY + vHeight + 10) < totalHeight,
            has_more_up: scrollY > 10
        },
        elements: items,
        hidden_elements_count: hiddenCount,
        notices: notices,
        dom_overlays: dom_overlays,
        tables: tables
    };
})()
"#;

/// 共享的元素定位 JS（嵌入到各交互脚本中）
///
/// 支持四种定位策略，按可靠性降序：
/// 1. 稳定 ref 属性直查 `[data-harness-ref="@e..."]`
/// 2. ref 属性丢失（DOM 重渲染）→ 走 `window.__harness_ref_map` 反查表二次定位
/// 3. 结构路径兜底（nth-child 链）
/// 4. 语义选择器 `:has-text('...')` / `:contains('...')`
/// 5. 普通 CSS 选择器
pub const FIND_TARGET_JS: &str = r#"
        const buildPathSafe = (el) => {
            const parts = [];
            let node = el;
            let depth = 0;
            while (node && node.nodeType === 1 && depth < 6) {
                const parent = node.parentElement;
                const t = node.tagName.toLowerCase();
                if (!parent) { parts.unshift(t); break; }
                const idx = Array.prototype.indexOf.call(parent.children, node) + 1;
                parts.unshift(t + ':nth-child(' + idx + ')');
                node = parent;
                depth++;
            }
            return parts.join('>');
        };

        const findTarget = (sel) => {
            if (!sel) return null;

            // 1 & 2. 稳定 ref：属性直查 → refMap 反查
            const isRefQuery = sel.startsWith('[data-harness-ref=');
            if (isRefQuery) {
                const byAttr = document.querySelector(sel);
                if (byAttr) return byAttr;

                const mRef = sel.match(/\[data-harness-ref=["']([^"']+)["']\]/);
                const refKey = mRef ? mRef[1] : null;
                const meta = refKey && window.__harness_ref_map ? window.__harness_ref_map[refKey] : null;
                if (meta) {
                    const tag = meta.tag || '*';
                    const candidates = document.querySelectorAll(tag);
                    // 2a. 按 name 精确区分
                    if (meta.name) {
                        for (let el of candidates) {
                            if ((el.getAttribute('name') || '') === meta.name) return el;
                        }
                    }
                    // 2b. 按 placeholder
                    if (meta.placeholder) {
                        for (let el of candidates) {
                            if ((el.getAttribute('placeholder') || '') === meta.placeholder) return el;
                        }
                    }
                    // 2c. 按归一化文本
                    if (meta.text) {
                        for (let el of candidates) {
                            const tx = (el.innerText || el.textContent || '').trim().replace(/\s+/g, ' ').slice(0, 40);
                            if (tx === meta.text) return el;
                        }
                    }
                    // 3. 结构路径兜底
                    if (meta.path) {
                        const all = document.querySelectorAll('*');
                        for (let el of all) {
                            if (buildPathSafe(el) === meta.path) return el;
                        }
                    }
                }
                // ref 已彻底失联，落到下面的通用策略再试一次
            }

            // 4. 语义选择器
            if (sel.includes(':has-text(') || sel.includes(':contains(')) {
                const m = sel.match(/^(.*?):(has-text|contains)\(['"](.*)['"]\)$/);
                if (m) {
                    const baseTag = m[1].trim() || '*';
                    const textToFind = m[3].trim();
                    const candidates = document.querySelectorAll(baseTag);
                    let firstHit = null;
                    for (let el of candidates) {
                        if ((el.innerText || el.textContent || '').includes(textToFind)) {
                            if (!firstHit) firstHit = el;
                            // 多实例时同样优先可见者，避免命中隐藏的历史残留节点
                            try { if (isVisibleEl(el)) return el; } catch (_) {}
                        }
                    }
                    if (firstHit) return firstHit;
                }
            }

            // 5. 普通 CSS
            try {
                const all = Array.prototype.slice.call(document.querySelectorAll(sel));
                if (all.length <= 1) return all.length ? all[0] : null;
                // 多实例：优先返回第一个可见实例
                for (const el of all) {
                    try { if (isVisibleEl(el)) return el; } catch (_) {}
                }
                return all[0];
            } catch (_) {
                return null;
            }
        };

        // ---- 可见性判定：尺寸非零且未被 display/visibility/opacity 隐藏 ----
        const isVisibleEl = (el) => {
            if (!el || typeof el.getBoundingClientRect !== 'function') return false;
            let r;
            try { r = el.getBoundingClientRect(); } catch (_) { return false; }
            if (!(r.width > 0 && r.height > 0)) return false;
            let st = null;
            try { st = window.getComputedStyle(el); } catch (_) { return false; }
            if (!st) return false;
            if (st.display === 'none' || st.visibility === 'hidden') return false;
            if (st.opacity === '0' || parseFloat(st.opacity) === 0) return false;
            return true;
        };

        // 收集选择器命中的全部候选实例（ref / 语义选择器 / 普通 CSS 三路）
        const collectTargets = (sel) => {
            if (!sel) return [];
            try {
                if (sel.startsWith('[data-harness-ref=')) {
                    const one = findTarget(sel);
                    return one ? [one] : [];
                }
                if (sel.includes(':has-text(') || sel.includes(':contains(')) {
                    const m = sel.match(/^(.*?):(has-text|contains)\(['"](.*)['"]\)$/);
                    if (m) {
                        const baseTag = m[1].trim() || '*';
                        const textToFind = m[3].trim();
                        const out = [];
                        for (const el of document.querySelectorAll(baseTag)) {
                            if ((el.innerText || el.textContent || '').includes(textToFind)) out.push(el);
                        }
                        return out;
                    }
                }
                return Array.prototype.slice.call(document.querySelectorAll(sel));
            } catch (_) {
                return [];
            }
        };

        // ---- 多实例择优 ----
        // 历史缺陷（2026-11-24 实测）：querySelector 只取第一个匹配，命中历史残留的隐藏实例后
        // 直接报「元素存在但不可见」，而页面上其实有可见的同名弹窗实例，导致 Agent 误判。
        // 现改为遍历全部实例，取「可见 + 面积最大」者，并回传实例总数/可见数供诊断分级。
        const findBestVisible = (sel) => {
            const list = collectTargets(sel);
            let best = null;
            let bestArea = -1;
            let visibleCount = 0;
            for (const n of list) {
                if (!isVisibleEl(n)) continue;
                visibleCount++;
                let area = 0;
                try {
                    const r = n.getBoundingClientRect();
                    area = r.width * r.height;
                } catch (_) { area = 0; }
                if (area > bestArea) { bestArea = area; best = n; }
            }
            return { el: best, total: list.length, visibleCount: visibleCount };
        };
    "#;

/// 探测下拉/列表的选项数量（用于区分「未展开」「空数据」「有数据」）
///
/// 背景（2026-11-24 实测）：Agent 用 `wait_for_selector(".el-select-dropdown__item")`
/// 超时后只能得到"未找到目标元素"，无法判断究竟是下拉没展开、还是接口返回空数据。
/// 结果只能靠截图肉眼确认——这正是视觉调用泛滥的直接原因。
///
/// 本脚本返回结构化的选项统计，让"空"成为可读的事实而非未知。
pub fn build_probe_options_script(container_selector: &str) -> String {
    let sel = container_selector.replace('"', "\\\"");
    format!(
        r#"(() => {{
            {find}
            // 选项候选集：覆盖 Element Plus / Ant Design / 原生 select / 通用列表
            const OPTION_SEL = [
                '.el-select-dropdown__item', '.el-select-dropdown__item span',
                '.ant-select-item-option', '.ant-select-item',
                'option', '[role="option"]',
                '.el-scrollbar__view li', 'li'
            ].join(',');

            const collect = (root) => {{
                const out = [];
                if (!root) return out;
                const nodes = root.querySelectorAll(OPTION_SEL);
                for (const n of nodes) {{
                    try {{
                        const r = n.getBoundingClientRect();
                        if (r.width <= 0 || r.height <= 0) continue;
                        const st = window.getComputedStyle(n);
                        if (st.display === 'none' || st.visibility === 'hidden') continue;
                        const t = (n.innerText || n.textContent || '').trim().replace(/\s+/g, ' ');
                        if (!t) continue;
                        out.push(t.slice(0, 40));
                    }} catch (_) {{}}
                }}
                // 去重（父容器与子 span 可能同文本）
                return Array.from(new Set(out));
            }};

            // 1. 优先在全局查找可见的下拉浮层（Element Plus 的下拉挂在 body 下）
            const popupSelectors = ['.el-select-dropdown', '.ant-select-dropdown', '.el-popper', '.el-autocomplete-suggestion'];
            let visiblePopup = null;
            for (const ps of popupSelectors) {{
                for (const p of document.querySelectorAll(ps)) {{
                    try {{
                        const r = p.getBoundingClientRect();
                        const st = window.getComputedStyle(p);
                        if (r.width > 0 && r.height > 0 && st.display !== 'none' && st.visibility !== 'hidden'
                            && st.opacity !== '0') {{
                            visiblePopup = p;
                            break;
                        }}
                    }} catch (_) {{}}
                }}
                if (visiblePopup) break;
            }}

            // 2. 统计选项
            let opts = collect(visiblePopup);
            let source = visiblePopup ? 'popup' : 'none';

            // 3. 若浮层未出现，退而在目标容器自身范围内找（适配内联列表）
            if (opts.length === 0) {{
                const el = findTarget("{sel}");
                if (el) {{
                    const inEl = collect(el);
                    if (inEl.length > 0) {{
                        opts = inEl;
                        source = 'container';
                    }}
                }}
            }}

            // 4. 判断是否存在"空数据"提示文案
            let emptyHint = '';
            const emptySelectors = ['.el-select-dropdown__empty', '.ant-select-item-empty', '.el-empty__description', '.el-table__empty-text'];
            for (const es of emptySelectors) {{
                for (const n of document.querySelectorAll(es)) {{
                    try {{
                        const r = n.getBoundingClientRect();
                        if (r.width > 0 && r.height > 0) {{
                            const t = (n.innerText || n.textContent || '').trim();
                            if (t) {{ emptyHint = t.slice(0, 40); break; }}
                        }}
                    }} catch (_) {{}}
                }}
                if (emptyHint) break;
            }}

            // 5. 是否处于加载中（避免把"还在加载"误判成"空数据"）
            let loading = false;
            for (const ls of ['.el-loading-mask', '.el-loading-spinner', '.ant-spin-spinning']) {{
                for (const n of document.querySelectorAll(ls)) {{
                    try {{
                        const r = n.getBoundingClientRect();
                        const st = window.getComputedStyle(n);
                        if (r.width > 0 && r.height > 0 && st.display !== 'none' && st.visibility !== 'hidden') {{
                            loading = true; break;
                        }}
                    }} catch (_) {{}}
                }}
                if (loading) break;
            }}

            return {{
                success: true,
                popup_open: !!visiblePopup,
                source: source,
                option_count: opts.length,
                samples: opts.slice(0, 10),
                empty_hint: emptyHint || null,
                loading: loading
            }};
        }})()"#,
        find = FIND_TARGET_JS,
        sel = sel
    )
}

/// 探测当前界面结构（弹窗 / 表单项 / 按钮可点击性 / 提示消息）
///
/// 背景（2026-11-24 实测）：Agent 反复点不到保存按钮、也不知道点击后发生了什么，
/// 只能靠截图。本探针让它一次调用看清「当前界面上有什么、哪个按钮能点、被谁挡住」。
///
/// 设计要点：
/// - 全部用**通用 class 匹配**（[class*="dialog"] 等），不绑定 el-/ant-/yh- 任一 UI 库
/// - 按钮用 elementFromPoint 检测是否被遮挡，直接告诉 Agent 能不能点
/// - label 从 form-item / label / aria-label 多路推断
pub fn build_probe_page_script() -> String {
    format!(
        r#"(() => {{
            {find}
            const desc = (n) => n ? (n.tagName.toLowerCase() + (typeof n.className === 'string' && n.className
                ? '.' + n.className.trim().split(/\s+/)[0] : '')) : null;
            const txt = (n, len) => ((n && (n.innerText || n.textContent)) || '').trim().replace(/\s+/g, ' ').slice(0, len || 40);
            const isVisible = (n) => {{
                try {{
                    const r = n.getBoundingClientRect();
                    if (!(r.width > 0 && r.height > 0)) return false;
                    const st = window.getComputedStyle(n);
                    return st.display !== 'none' && st.visibility !== 'hidden' && st.opacity !== '0';
                }} catch (_) {{ return false; }}
            }};

            // ============ 1. 可见弹窗 / 遮罩（通用 class 匹配） ============
            const overlaySel = '[class*="dialog"],[class*="modal"],[class*="drawer"],[class*="overlay"],[role="dialog"]';
            const overlays = [];
            for (const n of document.querySelectorAll(overlaySel)) {{
                if (!isVisible(n)) continue;
                const r = n.getBoundingClientRect();
                // 只登记有实际尺寸的容器，避免把一堆无高度包装层都列出来
                if (r.width < 80 || r.height < 40) continue;
                const cls = typeof n.className === 'string' ? n.className.trim().split(/\s+/)[0] : '';
                const item = {{
                    selector: 'div.' + cls,
                    size: Math.round(r.width) + 'x' + Math.round(r.height),
                    title: txt(n.querySelector('[class*="title"],[class*="header"]'), 30) || null
                }};
                if (!overlays.some(o => o.selector === item.selector)) overlays.push(item);
                if (overlays.length >= 6) break;
            }}

            // ============ 2. 表单项（含 label 推断） ============
            const fieldSel = 'input:not([type=hidden]),textarea,select,[contenteditable="true"]';
            const forms = [];
            for (const n of document.querySelectorAll(fieldSel)) {{
                if (!isVisible(n)) continue;
                // label 多路推断
                let label = n.getAttribute('aria-label') || n.getAttribute('placeholder') || '';
                if (!label) {{
                    const box = n.closest('[class*="form-item"],[class*="formItem"],.form-group,label,tr');
                    if (box) {{
                        const lb = box.querySelector('[class*="label"],[class*="title"],label');
                        if (lb) label = txt(lb, 30);
                    }}
                }}
                const r = n.getBoundingClientRect();
                const hit = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
                forms.push({{
                    tag: n.tagName.toLowerCase(),
                    type: n.getAttribute('type') || null,
                    placeholder: n.getAttribute('placeholder') || null,
                    label: label || null,
                    value: (n.value !== undefined && n.value !== null) ? String(n.value).slice(0, 40) : null,
                    disabled: !!(n.disabled || n.readOnly),
                    clickable: !!(hit && (hit === n || n.contains(hit))),
                    blocked_by: (hit && !(hit === n || n.contains(hit))) ? desc(hit) : null
                }});
                if (forms.length >= 40) break;
            }}

            // ============ 3. 按钮可点击性 ============
            const btnSel = 'button,[role="button"],[class*="btn"],[class*="button"]';
            const buttons = [];
            for (const n of document.querySelectorAll(btnSel)) {{
                if (!isVisible(n)) continue;
                const t = txt(n, 20);
                if (!t) continue;
                const r = n.getBoundingClientRect();
                // 取按钮中心点，看最顶层命中的是不是它自己（或其后代）
                const hit = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
                const selfHit = !!(hit && (hit === n || n.contains(hit)));
                const item = {{
                    text: t,
                    disabled: !!(n.disabled || n.getAttribute('aria-disabled') === 'true'),
                    clickable: selfHit,
                    blocked_by: selfHit ? null : desc(hit)
                }};
                // 去重（同一文字只留一个）
                if (!buttons.some(b => b.text === item.text)) buttons.push(item);
                if (buttons.length >= 30) break;
            }}

            // ============ 4. 提示消息 / 空态 ============
            const notices = [];
            const noticeSel = '[class*="message"],[class*="alert"],[class*="notify"],[class*="toast"],'
                + '[class*="empty"],[role="alert"]';
            for (const n of document.querySelectorAll(noticeSel)) {{
                if (!isVisible(n)) continue;
                const t = txt(n, 80);
                if (!t || t.length < 2) continue;
                const cls = typeof n.className === 'string' ? n.className : '';
                const tag = /empty/i.test(cls) ? '[空数据] ' : (/loading|spin/i.test(cls) ? '[加载中] ' : '[提示] ');
                const line = tag + t;
                if (!notices.includes(line)) notices.push(line);
                if (notices.length >= 10) break;
            }}

            return {{
                success: true,
                url: location.href,
                overlays: overlays,
                forms: forms,
                buttons: buttons,
                notices: notices
            }};
        }})()"#,
        find = FIND_TARGET_JS
    )
}

pub fn build_interact_script(action: &str, target: &str, value: Option<&str>) -> String {
    let clean_target = target.trim();
    let selector = if clean_target.starts_with('@') {
        format!("[data-harness-ref=\"{clean_target}\"]")
    } else {
        clean_target.to_string()
    };

    let escaped_selector = selector.replace('"', "\\\"");
    let escaped_value = value.unwrap_or("").replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n");

    // 共享定位器（含稳定 ref + refMap 反查 + 语义选择器）
    let find_target_fn = FIND_TARGET_JS;

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
        // 安全填值：绕过 React _valueTracker，确保受控组件 onChange 被触发
        "fill" | "type" => format!(
            r#"(() => {{
                {find_target_fn}
                const el = findTarget("{escaped_selector}");
                if (!el) return {{ success: false, error: "未找到目标元素: {clean_target}" }};
                if (el.disabled) return {{ success: false, error: "目标元素处于 disabled 状态: {clean_target}" }};
                if (el.readOnly) return {{ success: false, error: "目标元素处于 readonly 状态: {clean_target}" }};
                el.scrollIntoView({{ behavior: 'instant', block: 'center' }});
                el.focus();
                // 关键：改写原生 value setter，绕过 React 对 el.value 的 tracker 劫持。
                // 否则直接赋值会同步更新 _valueTracker，随后派发的 input 事件被判定"值未变"而丢弃，
                // 导致 onChange 不触发、提交空值 —— 这是登录表单填不进去的头号原因。
                let proto = el;
                let desc = null;
                while (proto && !desc) {{
                    proto = Object.getPrototypeOf(proto);
                    if (!proto) break;
                    desc = Object.getOwnPropertyDescriptor(proto, 'value');
                }}
                const newVal = "{escaped_value}";
                if (desc && desc.set) {{
                    desc.set.call(el, newVal);
                }} else {{
                    el.value = newVal;
                }}
                el.dispatchEvent(new Event('input', {{ bubbles: true, composed: true }}));
                el.dispatchEvent(new Event('change', {{ bubbles: true, composed: true }}));
                el.dispatchEvent(new Event('blur', {{ bubbles: true, composed: true }}));
                return {{ success: true, value_len: String(el.value || '').length }};
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
        // 单次判定（真轮询由 Rust 侧 driver 循环调用本脚本实现，避免长驻 JS 阻塞）
        "wait_for_selector" => format!(
            r#"(() => {{
                {find_target_fn}
                const best = findBestVisible("{escaped_selector}");
                // 完全不存在的语义：连一个实例都没命中
                if (best.total === 0) {{
                    return {{ success: false, found: false, visible: false, total: 0, visible_count: 0,
                               error: "未找到目标元素: {clean_target}" }};
                }}
                if (best.el) {{
                    return {{ success: true, found: true, visible: true,
                               total: best.total, visible_count: best.visibleCount, error: null }};
                }}
                // 命中了实例但全部不可见：区分「唯一隐藏」与「多个实例全部隐藏」
                const reason = best.total === 1
                    ? "元素存在但不可见: {clean_target}"
                    : ("所有 " + best.total + " 个匹配实例均不可见: {clean_target}");
                return {{ success: false, found: true, visible: false,
                           total: best.total, visible_count: 0, error: reason }};
            }})()"#
        ),
        // 选第一个 / 第 N 个 / 按文本匹配：解决"选第一个机构"这类位置语义诉求
        "pick_first" | "pick_by_index" | "pick_by_text" => {
            let pick_mode = action;
            let idx_json = value.and_then(|v| v.parse::<i64>().ok()).unwrap_or(1);
            let text_json = value.unwrap_or("").replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n");
            format!(
                r#"(() => {{
                    {find_target_fn}
                    const container = findTarget("{escaped_selector}") || document.body;
                    const mode = "{pick_mode}";
                    const wantIdx = {idx_json};
                    const wantText = "{text_json}";

                    // 候选集：容器内所有可见、文字非空的叶子级可点击项
                    const all = container.querySelectorAll('li, tr, [role="option"], [role="menuitem"], .org-item, .el-select-dropdown__item, .ant-select-item, a, button, div, span');
                    const picks = [];
                    for (let el of all) {{
                        const r = el.getBoundingClientRect();
                        if (r.width <= 0 || r.height <= 0) continue;
                        const st = window.getComputedStyle(el);
                        if (st.display === 'none' || st.visibility === 'hidden') continue;
                        const tx = (el.innerText || el.textContent || '').trim().replace(/\s+/g, ' ');
                        if (!tx) continue;
                        // 只保留"最内层"节点，避免父容器重复计入
                        let hasChildWithSameText = false;
                        for (let c of el.children) {{
                            const ctv = (c.innerText || c.textContent || '').trim().replace(/\s+/g, ' ');
                            if (ctv && ctv === tx) {{ hasChildWithSameText = true; break; }}
                        }}
                        if (hasChildWithSameText) continue;
                        picks.push({{ el: el, text: tx }});
                    }}

                    let chosen = null;
                    let chosenText = '';
                    if (mode === 'pick_first') {{
                        if (picks.length > 0) {{ chosen = picks[0].el; chosenText = picks[0].text; }}
                    }} else if (mode === 'pick_by_index') {{
                        const i = wantIdx - 1;
                        if (i >= 0 && i < picks.length) {{ chosen = picks[i].el; chosenText = picks[i].text; }}
                    }} else {{
                        // pick_by_text：取文字包含目标且长度最短（最精确）的候选
                        let best = null;
                        for (let p of picks) {{
                            if (p.text.includes(wantText)) {{
                                if (!best || p.text.length < best.text.length) best = p;
                            }}
                        }}
                        if (best) {{ chosen = best.el; chosenText = best.text; }}
                    }}

                    if (!chosen) {{
                        return {{ success: false, error: "未匹配到候选项 (mode=" + mode + ", 共 " + picks.length + " 个候选)" }};
                    }}
                    chosen.scrollIntoView({{ behavior: 'instant', block: 'center' }});
                    chosen.click();
                    return {{ success: true, matched_text: chosenText.slice(0, 60), candidates: picks.length }};
                }})()"#
            )
        }
        "assert_text" => format!(
            r#"(() => {{
                {find_target_fn}
                const el = findTarget("{escaped_selector}");
                if (!el) return {{ success: false, error: "断言失败：未找到目标元素: {clean_target}" }};
                const actual = (el.innerText || el.textContent || '').trim().replace(/\s+/g, ' ');
                const expect = "{escaped_value}";
                const ok = actual.includes(expect);
                return {{ success: ok, actual: actual.slice(0, 120),
                           error: ok ? null : ("断言失败：期望包含【" + expect + "】，实际为【" + actual.slice(0, 120) + "】") }};
            }})()"#
        ),
        "assert_url" => format!(
            r#"(() => {{
                const pattern = "{escaped_value}";
                const url = window.location.href || '';
                let ok = false;
                try {{ ok = new RegExp(pattern).test(url); }} catch (e) {{ ok = url.indexOf(pattern) >= 0; }}
                return {{ success: ok, url: url, error: ok ? null : ("URL 断言失败：期望匹配 /" + pattern + "/，实际 " + url) }};
            }})()"#
        ),
        "scroll" => {
            let dist: Option<f64> = value.and_then(|v| v.parse::<f64>().ok());
            build_scroll_script("down", dist, if clean_target.is_empty() { None } else { Some(clean_target) })
        }
        _ => format!(
            r#"(() => ({{ success: false, error: "不支持的操作类型: {action}" }}))()"#
        ),
    }
}

/// URL 匹配判定（供 Rust 侧 wait_for_url 轮询调用）
pub fn build_url_match_script(pattern: &str) -> String {
    let escaped = pattern.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n");
    format!(
        r#"(() => {{
            const pattern = "{escaped}";
            const url = window.location.href || '';
            let ok = false;
            try {{ ok = new RegExp(pattern).test(url); }} catch (e) {{ ok = url.indexOf(pattern) >= 0; }}
            return {{ success: ok, url: url }};
        }})()"#
    )
}

/// 任意 JS 表达式真值判定（供 Rust 侧 wait_for_function 轮询调用）
pub fn build_function_truthy_script(expr: &str) -> String {
    format!(
        r#"(() => {{
            try {{ return {{ success: !!({expr}) }}; }}
            catch (e) {{ return {{ success: false, error: String(e) }}; }}
        }})()"#
    )
}

pub fn build_scroll_script(direction: &str, distance: Option<f64>, target: Option<&str>) -> String {
    let clean_target = target.unwrap_or("").trim();
    let selector = if clean_target.starts_with('@') {
        format!("[data-harness-ref=\"{clean_target}\"]")
    } else {
        clean_target.to_string()
    };
    let escaped_selector = selector.replace('"', "\\\"");

    // 共享定位器（含稳定 ref + refMap 反查 + 语义选择器）
    let find_target_fn = FIND_TARGET_JS;

    if !clean_target.is_empty() {
        format!(
            r#"(() => {{
                {find_target_fn}
                const el = findTarget("{escaped_selector}");
                if (!el) return {{ success: false, error: "未找到目标元素: {clean_target}" }};
                el.scrollIntoView({{ behavior: 'instant', block: 'center', inline: 'center' }});
                return {{ success: true }};
            }})()"#
        )
    } else {
        let dist_val = distance.unwrap_or(0.0);
        match direction {
            "top" => r#"(() => {
                window.scrollTo({ top: 0, left: 0, behavior: 'instant' });
                return { success: true };
            })()"#.to_string(),
            "bottom" => r#"(() => {
                const maxScroll = Math.max(document.documentElement.scrollHeight, document.body ? document.body.scrollHeight : 0);
                window.scrollTo({ top: maxScroll, left: 0, behavior: 'instant' });
                return { success: true };
            })()"#.to_string(),
            "up" => format!(
                r#"(() => {{
                    const d = {dist_val} > 0 ? {dist_val} : (window.innerHeight * 0.8);
                    window.scrollBy({{ top: -d, left: 0, behavior: 'instant' }});
                    return {{ success: true }};
                }})()"#
            ),
            "left" => format!(
                r#"(() => {{
                    const d = {dist_val} > 0 ? {dist_val} : (window.innerWidth * 0.8);
                    window.scrollBy({{ top: 0, left: -d, behavior: 'instant' }});
                    return {{ success: true }};
                }})()"#
            ),
            "right" => format!(
                r#"(() => {{
                    const d = {dist_val} > 0 ? {dist_val} : (window.innerWidth * 0.8);
                    window.scrollBy({{ top: 0, left: d, behavior: 'instant' }});
                    return {{ success: true }};
                }})()"#
            ),
            _ => format!(
                r#"(() => {{
                    const d = {dist_val} > 0 ? {dist_val} : (window.innerHeight * 0.8);
                    window.scrollBy({{ top: d, left: 0, behavior: 'instant' }});
                    return {{ success: true }};
                }})()"#
            ),
        }
    }
}
