//! 浏览器批量执行器（browser_batch）
//!
//! 解决的核心痛点：原设计里「一次工具调用 = 一次原子操作 + 必定全量返回整页状态」，
//! 登录要输账号、输密码、点登录、选机构共 4~5 次 LLM 往返，每次都吃一份完整页面快照。
//! 本执行器让整条流程压缩为 **一次工具调用**，并按 `detail` 分级返回，token 下降 80%+。
//!
//! 设计要点：
//! - 优先走 CDP Input 域真实事件（isTrusted=true），拿不到坐标才回退 JS 合成；
//! - 等待类 op 用 Rust 侧真轮询，不用固定 sleep；
//! - `${secret:KEY}` 在执行前内存替换，返回值与日志一律回显 `***`；
//! - 连续失败达阈值熔断，避免 LLM 无限重试导致账号被锁。

use super::session::BrowserSession;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;

/// 连续失败熔断阈值
pub const CIRCUIT_BREAKER_THRESHOLD: u32 = 3;
/// 单步默认超时
const DEFAULT_TIMEOUT_MS: u64 = 10_000;
/// 轮询间隔
const POLL_INTERVAL_MS: u64 = 100;

/// 单步执行结果
#[derive(Debug, Clone, Serialize)]
pub struct StepResult {
    pub i: usize,
    pub op: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub desc: Option<String>,
    pub ok: bool,
    pub ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub echo: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matched_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidates: Option<String>,
    pub skipped: bool,
}

impl StepResult {
    fn ok(i: usize, op: &str) -> Self {
        Self {
            i,
            op: op.to_string(),
            target: None,
            desc: None,
            ok: true,
            ms: 0,
            echo: None,
            error: None,
            fallback: None,
            matched_text: None,
            candidates: None,
            skipped: false,
        }
    }
}

/// 批次执行结果
#[derive(Debug, Clone, Serialize)]
pub struct BatchOutcome {
    pub ok: bool,
    pub total: usize,
    pub executed: usize,
    pub skipped: usize,
    pub action_ms: u128,
    pub steps: Vec<StepResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failed_at: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    /// 页面状态 markdown（按 detail 分级）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<String>,
}

/// 判断批次自身是否包含登录动作（用于避免 AUTH 门禁自锁）
///
/// 判定依据：存在 fill/type 目标含 password，或值里用了 `${secret:...}` 占位符。
pub fn batch_looks_like_login(args: &Value) -> bool {
    let Some(steps) = args.get("steps").and_then(|v| v.as_array()) else {
        return false;
    };
    steps.iter().any(|s| {
        let op = s.get("op").and_then(|v| v.as_str()).unwrap_or("");
        if op != "fill" && op != "type" {
            return false;
        }
        let target = s
            .get("target")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let value = s.get("value").and_then(|v| v.as_str()).unwrap_or("");
        target.contains("password") || value.contains("${secret:")
    })
}

/// 解析 `${secret:KEY}` 占位符
///
/// 返回 (替换后的值, 是否为敏感字段)。敏感字段的回显必须脱敏。
pub fn resolve_secret_placeholder(raw: &str, lookup: &dyn Fn(&str) -> Option<String>) -> (String, bool) {
    let mut out = String::new();
    let mut sensitive = false;
    let mut rest = raw;

    while let Some(start) = rest.find("${secret:") {
        out.push_str(&rest[..start]);
        let after = &rest[start + "${secret:".len()..];
        match after.find('}') {
            Some(end) => {
                let key = after[..end].trim();
                match lookup(key) {
                    Some(v) => {
                        out.push_str(&v);
                        sensitive = true;
                    }
                    None => {
                        // 未命中时保留原占位符，由调用方判定 SECRET_MISSING
                        out.push_str(&rest[start..start + "${secret:".len() + end + 1]);
                    }
                }
                rest = &after[end + 1..];
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    (out, sensitive)
}

/// 检查字符串中是否仍残留未解析的 secret 占位符
pub fn has_unresolved_secret(s: &str) -> Option<String> {
    if let Some(start) = s.find("${secret:") {
        let after = &s[start + "${secret:".len()..];
        if let Some(end) = after.find('}') {
            return Some(after[..end].trim().to_string());
        }
    }
    None
}

#[derive(Debug, Clone, Deserialize)]
struct RawStep {
    op: String,
    #[serde(default)]
    target: Option<String>,
    #[serde(default)]
    value: Option<String>,
    #[serde(default)]
    pattern: Option<String>,
    #[serde(default)]
    index: Option<i64>,
    #[serde(default)]
    timeout_ms: Option<u64>,
    #[serde(default)]
    optional: Option<bool>,
    /// 关键步骤标记：即使同时标了 optional，失败也会使整批返回 ok=false
    #[serde(default)]
    critical: Option<bool>,
    #[serde(default)]
    desc: Option<String>,
    #[serde(default)]
    detail: Option<String>,
}

/// 执行一个批次
///
/// `secret_lookup`：按 key 查询明文的回调（由调用方从加密凭据库解密后提供）
pub async fn execute_batch<F>(
    session: &BrowserSession,
    args: &Value,
    secret_lookup: &F,
) -> Result<BatchOutcome, String>
where
    F: Fn(&str) -> Option<String> + Send + Sync,
{
    let steps_val = args
        .get("steps")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "缺少必填参数: steps（动作数组）".to_string())?;

    if steps_val.is_empty() {
        return Err("steps 不能为空数组".to_string());
    }

    let stop_on_error = args
        .get("stop_on_error")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let final_detail = args
        .get("detail")
        .and_then(|v| v.as_str())
        .unwrap_or("diff")
        .to_string();

    let steps: Vec<RawStep> = serde_json::from_value(Value::Array(steps_val.clone()))
        .map_err(|e| format!("steps 参数格式错误: {e}"))?;

    let started = std::time::Instant::now();
    let mut results: Vec<StepResult> = Vec::with_capacity(steps.len());
    let mut executed = 0usize;
    let mut skipped = 0usize;
    let mut failed_at: Option<usize> = None;
    let mut fail_code: Option<String> = None;
    let mut fail_msg: Option<String> = None;
    let mut fail_hint: Option<String> = None;

    for (idx, step) in steps.iter().enumerate() {
        let i = idx + 1;
        let step_start = std::time::Instant::now();
        let mut sr = StepResult::ok(i, &step.op);
        sr.target = step.target.clone();
        sr.desc = step.desc.clone();

        // 解析 secret 占位符
        let raw_value = step.value.clone().unwrap_or_default();
        let (resolved_value, is_sensitive) = resolve_secret_placeholder(&raw_value, secret_lookup);
        if let Some(missing_key) = has_unresolved_secret(&resolved_value) {
            sr.ok = false;
            sr.ms = step_start.elapsed().as_millis() as u64;
            sr.error = Some(format!("凭据未找到: ${{secret:{missing_key}}}"));
            fail_code = Some("SECRET_MISSING".to_string());
            fail_msg = Some(format!("凭据未找到: {missing_key}"));
            fail_hint = Some(format!("请先录入凭据 {missing_key}，例如在对话中告知「更新 {missing_key} 为 xxx」。"));
            results.push(sr);
            failed_at = Some(i);
            break;
        }
        if is_sensitive {
            sr.echo = Some("***".to_string());
        } else if !resolved_value.is_empty() {
            let echo = if resolved_value.chars().count() > 60 {
                let truncated: String = resolved_value.chars().take(60).collect();
                format!("{truncated}...")
            } else {
                resolved_value.clone()
            };
            sr.echo = Some(echo);
        }

        let timeout_ms = step.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS);
        let optional = step.optional.unwrap_or(false);

        let exec_res = exec_step(
            session,
            &step.op,
            step.target.as_deref(),
            &resolved_value,
            step.pattern.as_deref(),
            step.index,
            timeout_ms,
        )
        .await;

        sr.ms = step_start.elapsed().as_millis() as u64;

        match exec_res {
            Ok(StepOk {
                matched_text,
                candidates,
                fallback,
            }) => {
                sr.matched_text = matched_text;
                sr.candidates = candidates;
                sr.fallback = fallback;
                executed += 1;
            }
            Err(StepErr { code, message, hint }) => {
                let is_critical = step.critical.unwrap_or(false);
                if optional && !is_critical {
                    // 可选步骤：失败不中断，标记跳过
                    sr.ok = false;
                    sr.skipped = true;
                    sr.error = Some(format!("[可选步骤已跳过] {message}"));
                    skipped += 1;
                } else {
                    // 必失败步骤，或标记为 critical 的关键可选步骤（如登录后的 URL 断言）
                    sr.ok = false;
                    if optional {
                        sr.skipped = true;
                        skipped += 1;
                        sr.error = Some(format!("[关键步骤失败] {message}"));
                    } else {
                        sr.error = Some(message.clone());
                    }
                    failed_at = Some(i);
                    fail_code = Some(code);
                    fail_msg = Some(message);
                    fail_hint = hint;
                    results.push(sr);
                    if stop_on_error {
                        break;
                    }
                    continue;
                }
            }
        }

        results.push(sr);
    }

    let total = steps.len();
    let ok = failed_at.is_none();

    // 登录成功后自动固化登录态：
    // 历史缺陷 —— 原设计需人工点「保存登录态」按钮，Agent 登完就丢；
    // 这里在批次成功且含登录语义（填过密码框 / 有 assert_url、wait_for_url）时自动落盘。
    if ok {
        let looks_like_login = batch_looks_like_login(&json!({
            "steps": steps.iter().map(|s| json!({
                "op": s.op,
                "target": s.target,
                "value": s.value,
            })).collect::<Vec<_>>()
        }));
        if looks_like_login {
            let ws = session.get_workspace().await;
            if let Some(ws) = ws.filter(|w| w.exists()) {
                let curr_url = session.get_debug_state().await.ok().map(|d| d.url);
                let _ = super::storage::save_storage_state(
                    session.cdp(),
                    session.target_session_id(),
                    &ws,
                    curr_url.as_deref(),
                )
                .await;
            }
        }
    }

    // 批次结束取一次页面状态（供 Agent 判断下一步）
    let page = match session.get_debug_state().await {
        Ok(st) => Some(render_detail(&st, &final_detail, session).await),
        Err(_) => None,
    };

    Ok(BatchOutcome {
        ok,
        total,
        executed,
        skipped,
        action_ms: started.elapsed().as_millis(),
        steps: results,
        failed_at,
        code: fail_code,
        message: fail_msg,
        hint: fail_hint,
        page,
    })
}

/// 按 detail 级别渲染页面状态
async fn render_detail(
    state: &super::session::BrowserDebugState,
    detail: &str,
    session: &BrowserSession,
) -> String {
    match detail {
        "full" => {
            // 记录快照供后续 diff
            let snap = super::snapshot::PageSnapshot::from_state(state);
            let cursor_arc = session.snapshot_cursor();
            let mut holder = cursor_arc.lock().await;
            *holder = Some(snap);
            state.to_markdown()
        }
        "summary" => {
            let snap = super::snapshot::PageSnapshot::from_state(state);
            let cursor_arc = session.snapshot_cursor();
            let mut holder = cursor_arc.lock().await;
            let summary = match holder.as_ref() {
                Some(prev) => {
                    let mut d = super::snapshot::compute_diff(prev, &snap);
                    fill_new_logs(session, &mut d).await;
                    super::snapshot::render_summary(&snap, &d)
                }
                None => {
                    let mut d = super::snapshot::SnapshotDiff::default();
                    fill_new_logs(session, &mut d).await;
                    super::snapshot::render_summary(&snap, &d)
                }
            };
            *holder = Some(snap);
            summary
        }
        _ => {
            // diff：只回变化项
            let snap = super::snapshot::PageSnapshot::from_state(state);
            let cursor_arc = session.snapshot_cursor();
            let mut holder = cursor_arc.lock().await;
            let body = match holder.as_ref() {
                Some(prev) => {
                    let mut d = super::snapshot::compute_diff(prev, &snap);
                    fill_new_logs(session, &mut d).await;
                    super::snapshot::render_diff(&snap, &d)
                }
                None => {
                    let mut d = super::snapshot::SnapshotDiff::default();
                    fill_new_logs(session, &mut d).await;
                    super::snapshot::render_diff(&snap, &d)
                }
            };
            *holder = Some(snap);
            body
        }
    }
}

/// 用游标切片填充「自上次以来新增」的 console / network 错误
///
/// 这是修掉「反复回吐陈旧错误」的关键：cdp.rs 的缓冲只截断不清空，若不做游标切片，
/// 每次返回都会固定带上末 5 条 console + 末 8 条 network，纯浪费 token 且误导归因。
async fn fill_new_logs(session: &BrowserSession, diff: &mut super::snapshot::SnapshotDiff) {
    let console_logs = {
        let logs = session.cdp().console_logs.lock().await;
        logs.iter()
            .filter(|l| {
                l.contains("[Console.error]")
                    || l.contains("[Uncaught Exception]")
                    || l.contains("[Log.error]")
            })
            .cloned()
            .collect::<Vec<String>>()
    };
    let network_logs = {
        let net = session.cdp().network_errors.lock().await;
        net.clone()
    };

    // 读出并推进游标
    let (console_new, network_new) = {
        let cursor_arc = session.log_cursor();
        let mut cursor = cursor_arc.lock().await;
        let (c_new, c_next, _) = super::snapshot::slice_since(&console_logs, cursor.0);
        let (n_new, n_next, _) = super::snapshot::slice_since(&network_logs, cursor.1);
        cursor.0 = c_next;
        cursor.1 = n_next;
        (c_new, n_new)
    };

    diff.console_new = console_new;
    diff.network_new = network_new;
}

struct StepOk {
    matched_text: Option<String>,
    candidates: Option<String>,
    fallback: Option<String>,
}

struct StepErr {
    code: String,
    message: String,
    hint: Option<String>,
}

fn err(code: &str, message: impl Into<String>, hint: Option<String>) -> StepErr {
    StepErr {
        code: code.to_string(),
        message: message.into(),
        hint,
    }
}

/// 执行单个步骤
async fn exec_step(
    session: &BrowserSession,
    op: &str,
    target: Option<&str>,
    value: &str,
    pattern: Option<&str>,
    index: Option<i64>,
    timeout_ms: u64,
) -> Result<StepOk, StepErr> {
    let empty_ok = || StepOk {
        matched_text: None,
        candidates: None,
        fallback: None,
    };

    match op {
        // 真实事件点击；被遮挡时明确失败，不再静默降级
        "click" => {
            let t = target.ok_or_else(|| err("ELEMENT_NOT_FOUND", "click 缺少 target", None))?;
            match session.real_click_by_selector(t).await {
                Ok(()) => Ok(empty_ok()),
                Err(e) => {
                    // 关键分流（2026-11-24 定案）：
                    // 元素被遮挡 / 不可见 / disabled 时，合成点击同样无效，
                    // 必须直接报错而不是静默降级 —— 否则 Agent 会以为点成功了。
                    if e.starts_with("CLICK_BLOCKED")
                        || e.contains("disabled")
                        || e.contains("不可见")
                    {
                        return Err(err(
                            "CLICK_BLOCKED",
                            e.trim_start_matches("CLICK_BLOCKED:").trim().to_string(),
                            Some(
                                "元素被遮挡或不可用。请：① 先关闭可能存在的遮罩/上层弹窗；\
                                 ② 用 probe_page 查看当前可见弹窗与按钮可点击性；\
                                 ③ 确认该按钮是否处于 disabled 状态"
                                    .to_string(),
                            ),
                        ));
                    }

                    // 其余情况（如 CDP 通道短抖）允许一次合成点击兜底，
                    // 但必须在结果中显式警告，不能让 Agent 误判成功。
                    match session.interact("click", t, None).await {
                        Ok(_) => Ok(StepOk {
                            matched_text: Some(
                                "⚠️ 真实点击失败，已降级为合成点击（isTrusted=false）。\
                                 若组件校验严格，本次点击可能未生效，请用 assert_text/probe_page 确认结果，勿直接判定成功"
                                    .to_string(),
                            ),
                            candidates: None,
                            fallback: Some("js_synthetic_warned".to_string()),
                        }),
                        Err(_) => Err(err(
                            "ELEMENT_NOT_FOUND",
                            e,
                            Some(
                                "可先 wait_for_selector 确认元素存在，或改用 :has-text('文字') 语义选择器"
                                    .to_string(),
                            ),
                        )),
                    }
                }
            }
        }
        // 真实键盘输入；失败回退安全填值脚本
        "fill" | "type" => {
            let t = target.ok_or_else(|| err("ELEMENT_NOT_FOUND", "fill 缺少 target", None))?;
            match session.real_fill_by_selector(t, value).await {
                Ok(()) => Ok(empty_ok()),
                Err(e) => match session.interact("fill", t, Some(value)).await {
                    Ok(_) => Ok(StepOk {
                        matched_text: None,
                        candidates: None,
                        fallback: Some("js_synthetic".to_string()),
                    }),
                    Err(_) => Err(err("ELEMENT_NOT_VISIBLE", e, None)),
                },
            }
        }
        "press_enter" => {
            let t = target.unwrap_or("");
            session
                .interact("press_enter", t, None)
                .await
                .map(|_| empty_ok())
                .map_err(|e| err("ELEMENT_NOT_FOUND", e, None))
        }
        "press_key" => {
            // 简化为 Enter/Tab/Escape 三类常用键
            let key = if value.is_empty() { "Enter" } else { value };
            let t = target.unwrap_or("");
            let action = if key.eq_ignore_ascii_case("enter") {
                "press_enter"
            } else {
                // 其他键暂用 press_enter 相同的合成路径，避免引入未验证路径
                "press_enter"
            };
            session
                .interact(action, t, None)
                .await
                .map(|_| empty_ok())
                .map_err(|e| err("ELEMENT_NOT_FOUND", e, None))
        }
        "scroll" => {
            let dist: Option<f64> = if value.is_empty() {
                None
            } else {
                value.parse::<f64>().ok()
            };
            session
                .scroll(Some("down"), dist, target)
                .await
                .map(|_| empty_ok())
                .map_err(|e| err("ASSERT_FAILED", e, None))
        }
        // 真轮询等待
        "wait_for_selector" => {
            let t = target.ok_or_else(|| err("ELEMENT_NOT_FOUND", "wait_for_selector 缺少 target", None))?;
            let script = super::dom_script::build_interact_script("wait_for_selector", t, None);
            let (ok, _, msg) = session
                .poll_until(&script, timeout_ms, POLL_INTERVAL_MS)
                .await
                .map_err(|e| err("CHANNEL_CLOSED", e, None))?;
            if ok {
                Ok(empty_ok())
            } else {
                // 超时后做一次诊断分级，避免 Agent 面对"未找到"三字无从判断
                let diag = diagnose_selector_timeout(session, t).await;
                Err(err("TIMEOUT", format!("{msg}｜{diag}"), Some(format!(
                    "确认选择器是否正确，或加长 timeout_ms（当前 {timeout_ms}）"
                ))))
            }
        }
        "wait_for_url" => {
            let p = pattern
                .or(if value.is_empty() { None } else { Some(value) })
                .ok_or_else(|| err("ASSERT_FAILED", "wait_for_url 缺少 pattern", None))?;
            let script = super::dom_script::build_url_match_script(p);
            let (ok, url, msg) = session
                .poll_until(&script, timeout_ms.max(15_000), POLL_INTERVAL_MS)
                .await
                .map_err(|e| err("CHANNEL_CLOSED", e, None))?;
            if ok {
                Ok(StepOk {
                    matched_text: Some(url),
                    candidates: None,
                    fallback: None,
                })
            } else {
                // 超时时附上当前实际 URL：
                // 实测中 Agent 因为只看到"超时"二字，误以为整个登录流程成功，
                // 实际上还停在机构选择页。给出实际 URL 可直接消除这种误判。
                let current = session
                    .get_debug_state()
                    .await
                    .ok()
                    .map(|d| d.url)
                    .unwrap_or_default();
                Err(err(
                    "TIMEOUT",
                    format!("{msg}｜当前实际 URL: {current}（未匹配 /{p}/）"),
                    Some(format!("期望 URL 匹配 /{p}/，但页面停在 {current}，请检查上一步是否真正生效")),
                ))
            }
        }
        "wait_for_function" => {
            // 2026-11-24 实测缺陷：Agent 把 JS 表达式写在 `target` 里，
            // 而这里只读 `value`，导致生成 `!!()` 语法错误后每次空转 3~10 秒。
            // 修复：value 为空时回退取 target，两者都接受。
            let expr = if !value.trim().is_empty() {
                value
            } else {
                target.unwrap_or("").trim()
            };
            if expr.is_empty() {
                return Err(err(
                    "STEP_UNSUPPORTED",
                    "wait_for_function 缺少 JS 表达式（value 或 target 至少要有一个）",
                    Some("示例：{\"op\":\"wait_for_function\",\"value\":\"location.hash !== '#/login'\"}".to_string()),
                ));
            }

            // 语法预检：非法表达式立即报错，绝不进入轮询空转
            let syntax_check = format!(
                r#"(() => {{
                    try {{ new Function({expr_json}); return {{ ok: true }}; }}
                    catch (e) {{ return {{ ok: false, err: String(e) }}; }}
                }})()"#,
                expr_json = serde_json::to_string(expr).unwrap_or_else(|_| "\"\"".to_string())
            );
            let chk = eval_once(session, &syntax_check).await?;
            if chk.get("ok").and_then(|v| v.as_bool()) != Some(true) {
                return Err(err(
                    "STEP_UNSUPPORTED",
                    format!(
                        "wait_for_function 表达式语法非法：{}",
                        chk.get("err").and_then(|e| e.as_str()).unwrap_or("未知语法错误")
                    ),
                    Some("请检查括号/引号是否配对；表达式应是能返回真值的 JS 片段".to_string()),
                ));
            }

            let script = super::dom_script::build_function_truthy_script(expr);
            let (ok, _, msg) = session
                .poll_until(&script, timeout_ms, POLL_INTERVAL_MS)
                .await
                .map_err(|e| err("CHANNEL_CLOSED", e, None))?;
            if ok {
                Ok(empty_ok())
            } else {
                Err(err("TIMEOUT", format!("{msg}｜表达式: {expr}"), None))
            }
        }
        "wait_for_network_idle" => {
            // 2026-11-24 实测缺陷：旧实现只判 `document.readyState === 'complete'`，
            // 而 SPA 路由切换 / 表单提交期间 readyState 本来就是 complete，
            // 于是 0ms 立即返回 ✅，给出虚假的「已加载完」信号，紧随其后的断言必然失败。
            //
            // 现改为真正的网络静默判定：
            //   1. 基于 CDP Network 域的在途请求计数（requestWillBeSent 插入，
            //      loadingFinished / loadingFailed 移除）；
            //   2. 在途数连续 300ms 保持为 0 才判定静默（避免请求间隙误判）。
            const QUIET_WINDOW_MS: u64 = 300;
            let deadline = std::time::Instant::now() + Duration::from_millis(timeout_ms);
            let mut quiet_since: Option<std::time::Instant> = None;

            loop {
                let (total, samples) = session.inflight_requests(5).await;

                {
                    let now = std::time::Instant::now();
                    if advance_quiet_window(&mut quiet_since, total, now, QUIET_WINDOW_MS) {
                        return Ok(empty_ok());
                    }
                }

                if std::time::Instant::now() >= deadline {
                    let sample_txt = if samples.is_empty() {
                        String::new()
                    } else {
                        format!("；在途样本：{}", samples.join(" | "))
                    };
                    return Err(err(
                        "TIMEOUT",
                        format!(
                            "等待网络静默超时（{timeout_ms}ms），仍有 {total} 个请求在途{sample_txt}"
                        ),
                        Some(
                            "若接口本身较慢，可加长 timeout_ms；若为长轮询/SSE 常驻连接，\
                             该页面永远无法静默，请改用 wait_for_selector 等待目标元素出现"
                                .to_string(),
                        ),
                    ));
                }

                tokio::time::sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
            }
        }
        // 位置/文本语义选择
        "pick_first" => {
            let t = target.unwrap_or("body");
            let script = super::dom_script::build_interact_script("pick_first", t, index.map(|i| i.to_string()).as_deref());
            run_pick(session, &script).await
        }
        "pick_by_index" => {
            let t = target.unwrap_or("body");
            let idx = index.unwrap_or(
                value.parse::<i64>().unwrap_or(1),
            );
            let script = super::dom_script::build_interact_script("pick_by_index", t, Some(&idx.to_string()));
            run_pick(session, &script).await
        }
        "pick_by_text" => {
            let t = target.unwrap_or("body");
            let script = super::dom_script::build_interact_script("pick_by_text", t, Some(value));
            run_pick(session, &script).await
        }
        // 探测下拉/列表选项数量：区分「未展开 / 空数据 / 有数据」
        //
        // 这是消除「空态盲区」的核心原语。Agent 打开弹窗后，
        // 一次调用即可知道「科室下拉 0 项」，无需截图。
        "probe_options" => {
            let t = target.ok_or_else(|| {
                err("ELEMENT_NOT_FOUND", "probe_options 缺少 target（下拉输入框或容器选择器）", None)
            })?;
            let script = super::dom_script::build_probe_options_script(t);

            // 第一探：当前状态下有多少选项
            let res = eval_once(session, &script).await?;
            let first_count = res.get("option_count").and_then(|v| v.as_u64()).unwrap_or(0);
            let popup_open = res.get("popup_open").and_then(|v| v.as_bool()).unwrap_or(false);

            // 若尚未展开且无选项，尝试点击展开后再探一次
            if first_count == 0 && !popup_open {
                let _ = session.real_click_by_selector(t).await;
                tokio::time::sleep(Duration::from_millis(600)).await;
                let res2 = eval_once(session, &script).await?;
                return Ok(probe_result_to_step_ok(&res2, true));
            }

            Ok(probe_result_to_step_ok(&res, false))
        }
        // 探测当前界面结构：弹窗 / 表单项 / 按钮可点击性 / 提示消息
        //
        // 这是「看清界面有什么」的核心能力，替代反复截图。
        "probe_page" => {
            let script = super::dom_script::build_probe_page_script();
            let res = eval_once(session, &script).await?;
            Ok(StepOk {
                matched_text: Some(render_probe_page(&res)),
                candidates: res
                    .get("forms")
                    .and_then(|v| v.as_array())
                    .map(|a| a.len().to_string()),
                fallback: None,
            })
        }
        "assert_text" => {
            let t = target.ok_or_else(|| err("ASSERT_FAILED", "assert_text 缺少 target", None))?;
            // 对整页 body/html 断言必然失败（body 含全部导航文本），
            // 自动收窄到提示容器，避免 Agent 白跑一轮。
            let effective_target = if t.eq_ignore_ascii_case("body") || t.eq_ignore_ascii_case("html") {
                let script = format!(
                    r#"(() => {{
                        const cands = ['[class*="message"]','[class*="alert"]','[class*="notify"]',
                                       '[class*="toast"]','[role="alert"]'];
                        for (const c of cands) {{
                            for (const n of document.querySelectorAll(c)) {{
                                try {{
                                    const r = n.getBoundingClientRect();
                                    const st = window.getComputedStyle(n);
                                    if (r.width > 0 && r.height > 0 && st.display !== 'none'
                                        && st.visibility !== 'hidden' && (n.innerText || '').trim()) {{
                                        return {{ found: true }};
                                    }}
                                }} catch (_) {{}}
                            }}
                        }}
                        return {{ found: false }};
                    }})()"#
                );
                let probe = eval_once(session, &script).await?;
                let found = probe.get("found").and_then(|v| v.as_bool()).unwrap_or(false);
                if !found {
                    return Err(err(
                        "ASSERT_FAILED",
                        format!(
                            "对 body 断言时未找到任何提示容器（期望包含「{value}」）。\
                             当前页面没有可见的 message/alert/toast 提示"
                        ),
                        Some("提示可能已自动消失；建议改用 probe_page 查看当前界面状态".to_string()),
                    ));
                }
                // 收窄到提示容器后，用多选择器联合匹配
                "[class*=\"message\"],[class*=\"alert\"],[class*=\"notify\"],[role=\"alert\"]"
            } else {
                t
            };
            let script = super::dom_script::build_interact_script("assert_text", effective_target, Some(value));
            let res = eval_once(session, &script).await?;
            if res.get("success").and_then(|v| v.as_bool()) == Some(true) {
                Ok(empty_ok())
            } else {
                Err(err(
                    "ASSERT_FAILED",
                    res.get("error").and_then(|e| e.as_str()).unwrap_or("断言失败").to_string(),
                    None,
                ))
            }
        }
        "assert_text_raw" => {
            let t = target.ok_or_else(|| err("ASSERT_FAILED", "assert_text 缺少 target", None))?;
            let script = super::dom_script::build_interact_script("assert_text", t, Some(value));
            let res = eval_once(session, &script).await?;
            if res.get("success").and_then(|v| v.as_bool()) == Some(true) {
                Ok(empty_ok())
            } else {
                Err(err(
                    "ASSERT_FAILED",
                    res.get("error").and_then(|e| e.as_str()).unwrap_or("断言失败").to_string(),
                    None,
                ))
            }
        }
        "assert_url" => {
            let script = super::dom_script::build_interact_script("assert_url", "", Some(value));
            let res = eval_once(session, &script).await?;
            if res.get("success").and_then(|v| v.as_bool()) == Some(true) {
                Ok(empty_ok())
            } else {
                Err(err(
                    "ASSERT_FAILED",
                    res.get("error").and_then(|e| e.as_str()).unwrap_or("URL 断言失败").to_string(),
                    None,
                ))
            }
        }
        // 显式人工暂停点（验证码等无法自动化环节）
        "pause" => Err(StepErr {
            code: "INTERVENTION_REQUIRED".to_string(),
            message: if value.is_empty() {
                "需要人工介入".to_string()
            } else {
                value.to_string()
            },
            hint: Some("请在界面上完成该步骤，然后继续后续操作".to_string()),
        }),
        other => Err(err(
            "STEP_UNSUPPORTED",
            format!("不支持的 op: {other}"),
            Some("可用 op 见 browser_batch 工具说明".to_string()),
        )),
    }
}

/// 选择器等待超时后的诊断分级
///
/// 2026-11-24 实测：Agent 猜 `.el-dialog__wrapper`（Element Plus 2.x 已移除）
/// 白等 10 秒后只得到"未找到目标元素"，不得不截图确认弹窗其实已经渲染。
/// 本函数给出可操作的分级结论，让 Agent 当场自我纠正。
async fn diagnose_selector_timeout(session: &BrowserSession, target: &str) -> String {
    // 诊断脚本：区分「存在但不可见」与「完全不存在」，并列出页面实际存在的近似容器
    //
    // 2026-11-24 二次修订：旧版只 querySelector 取第一个实例，遇到「历史残留的隐藏弹窗排在前面、
    // 真正可见的弹窗排在后面」时，给出「可能被折叠/隐藏，需先触发展开或滚动」的错误指引
    // （弹窗根本没有折叠）。现统计全部实例的可见数，分别给出可操作建议。
    let escaped = target.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n");
    let script = format!(
        r#"(() => {{
            const raw = "{escaped}";
            const sel = raw.startsWith('@') ? ('[data-harness-ref="' + raw + '"]') : raw;

            const isVisible = (el) => {{
                try {{
                    const r = el.getBoundingClientRect();
                    if (!(r.width > 0 && r.height > 0)) return false;
                    const st = window.getComputedStyle(el);
                    return st.display !== 'none' && st.visibility !== 'hidden'
                        && st.opacity !== '0' && parseFloat(st.opacity) !== 0;
                }} catch (_) {{ return false; }}
            }};

            // 1. 选择器本身是否合法 + 统计全部实例的可见情况
            let exists = false, visible = false, total = 0, visibleCount = 0, err = '';
            try {{
                const all = document.querySelectorAll(sel);
                total = all.length;
                exists = total > 0;
                for (const n of all) {{
                    if (isVisible(n)) visibleCount++;
                }}
                visible = visibleCount > 0;
            }} catch (e) {{ err = String(e); }}

            // 2. 列出页面上真实存在的"疑似目标容器"，帮助 Agent 找到正确选择器
            const nearPatterns = [
                '.el-overlay', '.el-dialog', '.el-drawer', '[role="dialog"]',
                '.el-select-dropdown', '.el-popper', '.el-message-box',
                '.ant-modal', '.ant-drawer', '.vxe-modal'
            ];
            const near = [];
            for (const p of nearPatterns) {{
                for (const n of document.querySelectorAll(p)) {{
                    try {{
                        const r = n.getBoundingClientRect();
                        const st = window.getComputedStyle(n);
                        if (r.width > 0 && r.height > 0 && st.display !== 'none' && st.visibility !== 'hidden') {{
                            const cls = typeof n.className === 'string' ? n.className.split(/\s+/).filter(Boolean).slice(0, 2).join('.') : '';
                            if (cls && !near.includes('div.' + cls)) near.push('div.' + cls);
                        }}
                    }} catch (_) {{}}
                }}
                if (near.length >= 6) break;
            }}

            return {{ exists: exists, visible: visible, total: total,
                      visible_count: visibleCount, error: err, similar: near }};
        }})()"#
    );

    match eval_once(session, &script).await {
        Ok(res) => {
            let exists = res.get("exists").and_then(|v| v.as_bool()).unwrap_or(false);
            let visible = res.get("visible").and_then(|v| v.as_bool()).unwrap_or(false);
            let total = res.get("total").and_then(|v| v.as_u64()).unwrap_or(0);
            let visible_count = res.get("visible_count").and_then(|v| v.as_u64()).unwrap_or(0);
            let parse_err = res.get("error").and_then(|v| v.as_str()).unwrap_or("");
            let similar: Vec<String> = res
                .get("similar")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|s| s.as_str().map(|x| x.to_string()))
                        .collect()
                })
                .unwrap_or_default();

            // 按「实例总数 / 可见数」三分类：
            //   · 全部不可见      → 才是真的需要滚动/展开
            //   · 存在可见实例    → 命中了误区，引导用 probe_page 或可见实例选择器
            //   · 零匹配          → 选择器本身不对，给出近似容器候选
            let visibility_summary = if total > 0 {
                format!("（匹配 {total} 个实例，其中可见 {visible_count} 个）")
            } else {
                String::new()
            };

            let near_hint = if similar.is_empty() {
                String::new()
            } else {
                format!("；页面当前可用的疑似容器：{}", similar.join(", "))
            };

            if !parse_err.is_empty() {
                format!("【诊断：选择器语法错误】{parse_err}")
            } else if exists && !visible {
                // 全部实例都不可见 —— 这才是「被折叠/隐藏」的真实场景
                let label = if total > 1 {
                    format!("【诊断：container_exists_but_hidden】匹配到 {total} 个实例但全部不可见")
                } else {
                    "【诊断：container_exists_but_hidden】元素存在但不可见".to_string()
                };
                format!(
                    "{label}{visibility_summary}｜建议：确需操作时先触发展开，或滚动到视口后再试；\
                     若判断本应已显示，请用 probe_page 查看当前可见容器"
                )
            } else if exists && visible_count > 0 {
                // 存在可见实例却仍判定超时 —— 说明判定与诊断口径不一致（尺寸/opacity 边界）
                // 也可能是 wait_for_selector 的轮询脚本命中了另一个隐藏实例。
                format!(
                    "【诊断：multiple_instances_some_visible】{visibility_summary}｜\
                     选择器本身能匹配到可见元素，原先的「不可见」判定命中的是隐藏实例。\
                     请改用 probe_page 查看当前可见弹窗与表单，或直接用可见实例的 @ref 定位"
                )
            } else {
                format!("【诊断：selector_no_match】{near_hint}")
            }
        }
        Err(_) => "【诊断：无法执行诊断脚本（CDP 通道异常）】".to_string(),
    }
}

/// 网络静默窗口推进
///
/// 语义：在途请求数为 0 时开始计时；一旦出现新在途请求立即重置窗口；
/// 静默持续 ≥ quiet_ms 则返回 true（可判定「网络已空闲」）。
///
/// 抽成纯函数是为了不依赖真实浏览器即可验证「0ms 虚假成功」不再复现。
fn advance_quiet_window(
    quiet_since: &mut Option<std::time::Instant>,
    inflight: usize,
    now: std::time::Instant,
    quiet_ms: u64,
) -> bool {
    if inflight > 0 {
        *quiet_since = None;
        return false;
    }
    match *quiet_since {
        None => {
            *quiet_since = Some(now);
            false
        }
        Some(start) => now.duration_since(start) >= Duration::from_millis(quiet_ms),
    }
}

/// 把 probe_page 的返回值渲染为结构化摘要
fn render_probe_page(res: &Value) -> String {
    let mut out = String::new();

    // 弹窗
    let overlays = res.get("overlays").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    if overlays.is_empty() {
        out.push_str("**可见弹窗**: 无\n");
    } else {
        out.push_str("**可见弹窗**:\n");
        for o in &overlays {
            let sel = o.get("selector").and_then(|v| v.as_str()).unwrap_or("");
            let size = o.get("size").and_then(|v| v.as_str()).unwrap_or("");
            let title = o.get("title").and_then(|v| v.as_str()).unwrap_or("");
            out.push_str(&format!(
                "- `{sel}` ({size}){} \n",
                if title.is_empty() { String::new() } else { format!(" 标题「{title}」") }
            ));
        }
    }

    // 表单项
    let forms = res.get("forms").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    if forms.is_empty() {
        out.push_str("**表单项**: 无\n");
    } else {
        out.push_str(&format!("**表单项** ({} 个):\n", forms.len()));
        for f in forms.iter().take(20) {
            let tag = f.get("tag").and_then(|v| v.as_str()).unwrap_or("");
            let label = f.get("label").and_then(|v| v.as_str()).unwrap_or("");
            let ph = f.get("placeholder").and_then(|v| v.as_str()).unwrap_or("");
            let val = f.get("value").and_then(|v| v.as_str()).unwrap_or("");
            let disabled = f.get("disabled").and_then(|v| v.as_bool()).unwrap_or(false);
            let clickable = f.get("clickable").and_then(|v| v.as_bool()).unwrap_or(true);
            let blocked = f.get("blocked_by").and_then(|v| v.as_str()).unwrap_or("");
            let mut line = format!("- `<{tag}>`");
            if !label.is_empty() {
                line.push_str(&format!(" 标签「{label}」"));
            }
            if !ph.is_empty() {
                line.push_str(&format!(" placeholder=\"{ph}\""));
            }
            if !val.is_empty() {
                line.push_str(&format!(" value=\"{val}\""));
            }
            if disabled {
                line.push_str(" [只读/禁用]");
            }
            if !clickable && !blocked.is_empty() {
                line.push_str(&format!(" ⚠️被 {blocked} 遮挡"));
            }
            out.push_str(&line);
            out.push('\n');
        }
    }

    // 按钮
    let buttons = res.get("buttons").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    if !buttons.is_empty() {
        out.push_str(&format!("**按钮** ({} 个):\n", buttons.len()));
        for b in buttons.iter().take(20) {
            let text = b.get("text").and_then(|v| v.as_str()).unwrap_or("");
            let disabled = b.get("disabled").and_then(|v| v.as_bool()).unwrap_or(false);
            let clickable = b.get("clickable").and_then(|v| v.as_bool()).unwrap_or(true);
            let blocked = b.get("blocked_by").and_then(|v| v.as_str()).unwrap_or("");
            let mut line = format!("- 「{text}」");
            if disabled {
                line.push_str(" [disabled，点不动]");
            } else if !clickable {
                line.push_str(&format!(" ⚠️被 {blocked} 遮挡，真实点击会失败"));
            } else {
                line.push_str(" [可点击]");
            }
            out.push_str(&line);
            out.push('\n');
        }
    }

    // 提示
    let notices = res.get("notices").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    if !notices.is_empty() {
        out.push_str("**提示消息**:\n");
        for n in notices.iter().take(10) {
            if let Some(s) = n.as_str() {
                out.push_str(&format!("- {s}\n"));
            }
        }
    }

    out.trim_end().to_string()
}

/// 把 probe_options 的 JS 返回值转换成 StepOk，并生成人类可读的 matched_text
///
/// 关键：无论有数据还是空，都返回 Ok（不是失败）。
/// 「空」是业务事实，不是错误——把它表达成明确事实，Agent 才不会去截图求证。
fn probe_result_to_step_ok(res: &Value, expanded_attempted: bool) -> StepOk {
    let count = res.get("option_count").and_then(|v| v.as_u64()).unwrap_or(0);
    let popup_open = res.get("popup_open").and_then(|v| v.as_bool()).unwrap_or(false);
    let loading = res.get("loading").and_then(|v| v.as_bool()).unwrap_or(false);
    let empty_hint = res.get("empty_hint").and_then(|v| v.as_str()).unwrap_or("");
    let source = res.get("source").and_then(|v| v.as_str()).unwrap_or("none");
    let samples: Vec<String> = res
        .get("samples")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|s| s.as_str().map(|x| x.to_string()))
                .collect()
        })
        .unwrap_or_default();

    let summary = if loading {
        format!("⏳ 下拉加载中（暂未拿到选项，请稍后重试）；弹出层={popup_open}")
    } else if count > 0 {
        format!(
            "选项 {} 项（来源={source}，弹出层={}）：{}",
            count,
            popup_open,
            samples.join(" / ")
        )
    } else if !popup_open {
        format!(
            "尚未展开或无可选项（已{}，来源={source}）",
            if expanded_attempted { "尝试点击展开" } else { "检测" }
        )
    } else if !empty_hint.is_empty() {
        format!("下拉已展开但无选项，页面提示「{empty_hint}」→ 属业务空数据，不是选择器问题")
    } else {
        "下拉已展开但选项数为 0 → 属业务空数据，不是选择器问题".to_string()
    };

    StepOk {
        matched_text: Some(summary),
        candidates: Some(count.to_string()),
        fallback: None,
    }
}

/// 执行一次 eval 并返回 value
async fn eval_once(session: &BrowserSession, script: &str) -> Result<Value, StepErr> {
    let res = session
        .cdp()
        .send(
            "Runtime.evaluate",
            Some(json!({
                "expression": script,
                "returnByValue": true,
                "awaitPromise": true
            })),
            Some(session.target_session_id()),
            Duration::from_secs(10),
        )
        .await
        .map_err(|e| err("CHANNEL_CLOSED", e, None))?;
    Ok(res
        .get("result")
        .and_then(|r| r.get("value"))
        .cloned()
        .unwrap_or(json!({})))
}

/// pick_* 系列：执行并解析 matched_text / candidates
async fn run_pick(session: &BrowserSession, script: &str) -> Result<StepOk, StepErr> {
    let res = eval_once(session, script).await?;
    if res.get("success").and_then(|v| v.as_bool()) == Some(true) {
        Ok(StepOk {
            matched_text: res.get("matched_text").and_then(|v| v.as_str()).map(|s| s.to_string()),
            candidates: res
                .get("candidates")
                .and_then(|v| v.as_u64())
                .map(|n| n.to_string()),
            fallback: None,
        })
    } else {
        Err(err(
            "ELEMENT_NOT_FOUND",
            res.get("error")
                .and_then(|e| e.as_str())
                .unwrap_or("未匹配到候选项")
                .to_string(),
            Some("弹窗可能未出现；若该账号只有单一机构，可给此步加 optional:true 跳过".to_string()),
        ))
    }
}

/// 把批次结果渲染为 Markdown
pub fn render_outcome(outcome: &BatchOutcome) -> String {
    let mut md = String::new();

    if outcome.ok && outcome.skipped > 0 {
        // 部分完成：所有未跳过的步骤成功，但有步骤被跳过。
        // 实测教训：Agent 看到「✅ 批量操作完成」就认为全流程成功，
        // 而实际结尾的 wait_for_url 断言已被跳过（页面还停在机构选择页）。
        // 这里必须显式降级提示，并列出被跳过的关键步骤。
        md.push_str(&format!(
            "### ⚠️ 批量操作部分完成（{} 步被跳过，结论不可全信）\n\
             - 总步骤: {} | 成功执行: {} | 跳过: {} | 耗时: {}ms\n\n",
            outcome.skipped, outcome.total, outcome.executed, outcome.skipped, outcome.action_ms
        ));
        md.push_str("> ⚠️ **注意**：以下步骤被跳过，其对应的验证目标**未得到确认**，请勿据此判定流程成功：\n");
        for s in outcome.steps.iter().filter(|s| s.skipped) {
            let mut line = format!("> - [{}] `{}`", s.i, s.op);
            if let Some(t) = &s.target {
                line.push_str(&format!(" target=`{t}`"));
            }
            if let Some(d) = &s.desc {
                line.push_str(&format!(" （{d}）"));
            }
            if let Some(e) = &s.error {
                line.push_str(&format!(" — {e}"));
            }
            md.push_str(&line);
            md.push('\n');
        }
        md.push_str("> 建议：用 `browser_get_debug_state` 或 `assert_url` 单独确认当前真实状态后再继续。\n\n");
    } else if outcome.ok {
        md.push_str(&format!(
            "### ✅ 批量操作完成\n- 总步骤: {} | 成功执行: {} | 跳过: {} | 耗时: {}ms\n\n",
            outcome.total, outcome.executed, outcome.skipped, outcome.action_ms
        ));
    } else {
        md.push_str(&format!(
            "### ❌ 批量操作中断（第 {} 步失败）\n- 错误码: `{}`\n- 原因: {}\n",
            outcome.failed_at.unwrap_or(0),
            outcome.code.as_deref().unwrap_or("UNKNOWN"),
            outcome.message.as_deref().unwrap_or("")
        ));
        if let Some(hint) = &outcome.hint {
            md.push_str(&format!("- 建议: {hint}\n"));
        }
        md.push_str(&format!(
            "- 总步骤: {} | 已执行: {} | 跳过: {} | 耗时: {}ms\n\n",
            outcome.total, outcome.executed, outcome.skipped, outcome.action_ms
        ));
    }

    md.push_str("#### 步骤明细\n");
    for s in &outcome.steps {
        let mark = if s.skipped {
            "⏭️"
        } else if s.ok {
            "✅"
        } else {
            "❌"
        };
        let mut line = format!("- {mark} [{}] `{}`", s.i, s.op);
        if let Some(t) = &s.target {
            line.push_str(&format!(" target=`{t}`"));
        }
        if let Some(d) = &s.desc {
            line.push_str(&format!(" （{d}）"));
        }
        if let Some(e) = &s.echo {
            line.push_str(&format!(" value=`{e}`"));
        }
        if let Some(m) = &s.matched_text {
            line.push_str(&format!(" → `{m}`"));
        }
        if let Some(f) = &s.fallback {
            if f == "js_synthetic_warned" {
                line.push_str(" ⚠️[合成点击-可能未生效]");
            } else {
                line.push_str(&format!(" [回退:{f}]"));
            }
        }
        line.push_str(&format!(" {}ms", s.ms));
        if let Some(err) = &s.error {
            line.push_str(&format!(" — {err}"));
        }
        md.push_str(&line);
        md.push('\n');
    }
    md.push('\n');

    if let Some(page) = &outcome.page {
        md.push_str(page);
        md.push('\n');
    }

    md
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_secret_success() {
        let lookup = |k: &str| -> Option<String> {
            if k == "login_pwd" {
                Some("S3cret!".to_string())
            } else {
                None
            }
        };
        let (val, sensitive) = resolve_secret_placeholder("pass=${secret:login_pwd}", &lookup);
        assert_eq!(val, "pass=S3cret!");
        assert!(sensitive);
    }

    #[test]
    fn test_resolve_secret_no_placeholder() {
        let lookup = |_: &str| -> Option<String> { None };
        let (val, sensitive) = resolve_secret_placeholder("admin", &lookup);
        assert_eq!(val, "admin");
        assert!(!sensitive);
    }

    #[test]
    fn test_resolve_secret_missing_keeps_placeholder() {
        let lookup = |_: &str| -> Option<String> { None };
        let (val, sensitive) = resolve_secret_placeholder("${secret:nope}", &lookup);
        assert_eq!(val, "${secret:nope}");
        assert!(!sensitive);
        assert_eq!(has_unresolved_secret(&val), Some("nope".to_string()));
    }

    #[test]
    fn test_resolve_multiple_placeholders() {
        let lookup = |k: &str| -> Option<String> {
            match k {
                "u" => Some("admin".to_string()),
                "p" => Some("pw".to_string()),
                _ => None,
            }
        };
        let (val, sensitive) = resolve_secret_placeholder("${secret:u}:${secret:p}", &lookup);
        assert_eq!(val, "admin:pw");
        assert!(sensitive);
    }

    #[test]
    fn test_has_unresolved_secret_none() {
        assert_eq!(has_unresolved_secret("plain text"), None);
        assert_eq!(has_unresolved_secret("${secret:abc}"), Some("abc".to_string()));
    }

    #[test]
    fn test_batch_looks_like_login_detects_password_fill() {
        let args = json!({"steps": [
            {"op":"fill","target":"input[type=password]","value":"x"}
        ]});
        assert!(batch_looks_like_login(&args));
    }

    #[test]
    fn test_batch_looks_like_login_detects_secret_placeholder() {
        let args = json!({"steps": [
            {"op":"fill","target":"#pwd","value":"${secret:login_pwd}"}
        ]});
        assert!(batch_looks_like_login(&args));
    }

    #[test]
    fn test_batch_looks_like_login_false_for_plain_click() {
        let args = json!({"steps": [{"op":"click","target":"#submit"}]});
        assert!(!batch_looks_like_login(&args));
    }

    #[test]
    fn test_probe_result_reports_empty_as_fact_not_error() {
        // 空下拉：必须明确表达"空数据"，而不是让 Agent 以为出错去截图
        let res = json!({
            "option_count": 0, "popup_open": true, "loading": false,
            "empty_hint": "暂无数据", "source": "popup", "samples": []
        });
        let ok = probe_result_to_step_ok(&res, false);
        let text = ok.matched_text.unwrap();
        assert!(text.contains("空数据"), "应明确标注为空数据: {text}");
        assert!(text.contains("暂无数据"), "应带回页面提示文案: {text}");
    }

    #[test]
    fn test_probe_result_reports_loading_separately() {
        // 加载中不能被误报成空数据
        let res = json!({
            "option_count": 0, "popup_open": true, "loading": true,
            "empty_hint": "", "source": "popup", "samples": []
        });
        let text = probe_result_to_step_ok(&res, false).matched_text.unwrap();
        assert!(text.contains("加载中"), "应区分加载中与空数据: {text}");
    }

    #[test]
    fn test_probe_result_lists_options_when_present() {
        let res = json!({
            "option_count": 2, "popup_open": true, "loading": false,
            "empty_hint": "", "source": "popup", "samples": ["内科", "外科"]
        });
        let ok = probe_result_to_step_ok(&res, false);
        let text = ok.matched_text.unwrap();
        assert!(text.contains("2"), "应给出选项数: {text}");
        assert!(text.contains("内科"), "应带回样本: {text}");
        assert_eq!(ok.candidates.as_deref(), Some("2"));
    }

    #[test]
    fn test_render_outcome_marks_partial_when_steps_skipped() {
        // 复现实测缺陷：ok=true 但结尾断言被跳过，绝不能显示「✅ 完成」
        let outcome = BatchOutcome {
            ok: true,
            total: 3,
            executed: 1,
            skipped: 2,
            action_ms: 100,
            steps: vec![],
            failed_at: None,
            code: None,
            message: None,
            hint: None,
            page: None,
        };
        let md = render_outcome(&outcome);
        assert!(md.contains("部分完成"), "跳步时必须降级提示: {md}");
        assert!(md.contains("不可全信"), "应明确警示结论不可全信: {md}");
        assert!(!md.contains("✅ 批量操作完成"), "不得显示完全成功: {md}");
    }

    #[test]
    fn test_probe_page_script_uses_generic_class_matching() {
        // 关键：探针不能绑定具体 UI 库前缀，否则换 yh-/el-/ant- 就失效
        let s = super::super::dom_script::build_probe_page_script();
        assert!(s.contains("[class*=\"dialog\"]"), "弹窗应通用匹配");
        assert!(s.contains("[class*=\"form-item\"]"), "表单项应通用匹配");
        assert!(s.contains("elementFromPoint"), "按钮需做遮挡检测");
        assert!(s.contains("blocked_by"), "应报告遮挡者");
        assert!(s.contains("notices"), "应收集提示消息");
    }

    #[test]
    fn test_render_probe_page_reports_blocked_button() {
        // 复现实测：保存按钮点不到，探针必须直接指出"被谁挡住"
        let res = json!({
            "url": "http://x/#/a",
            "overlays": [{"selector": "div.yh-dialog", "size": "800x600", "title": "新增"}],
            "forms": [{"tag":"input","label":"事件性质","placeholder":"请选择","value":null,
                       "disabled":false,"clickable":true,"blocked_by":null}],
            "buttons": [
                {"text":"保存","disabled":false,"clickable":false,"blocked_by":"div.loading-mask"},
                {"text":"取消","disabled":false,"clickable":true,"blocked_by":null}
            ],
            "notices": ["[空数据] 暂无数据"]
        });
        let md = render_probe_page(&res);
        assert!(md.contains("div.yh-dialog"), "应列出可见弹窗: {md}");
        assert!(md.contains("事件性质"), "应推断出表单项标签: {md}");
        assert!(md.contains("被 div.loading-mask 遮挡"), "应指出遮挡者: {md}");
        assert!(md.contains("[可点击]"), "应标注可点击按钮: {md}");
        assert!(md.contains("暂无数据"), "应带出提示消息: {md}");
    }

    #[test]
    fn test_render_probe_page_marks_disabled_button() {
        let res = json!({
            "overlays": [], "forms": [], "notices": [],
            "buttons": [{"text":"保存","disabled":true,"clickable":true,"blocked_by":null}]
        });
        let md = render_probe_page(&res);
        assert!(md.contains("disabled"), "应明确标注按钮不可用: {md}");
        assert!(md.contains("点不动"), "应用人类可读措辞: {md}");
    }

    /// 复现「0ms 虚假成功」缺陷：在途请求未归零时，静默窗口必须持续重置，
    /// 不允许一上来就判定 idle。
    #[test]
    fn test_quiet_window_never_idle_while_inflight() {
        let t0 = std::time::Instant::now();
        let mut since = None;
        // 连续 10 次采样都有在途请求，即使跨越静默窗口时长也不能判定 idle
        for i in 0..10u64 {
            let now = t0 + Duration::from_millis(i * 100);
            assert!(
                !advance_quiet_window(&mut since, 2, now, 300),
                "有在途请求时绝不可判定网络空闲（第 {i} 次）"
            );
            assert!(since.is_none(), "在途请求应重置静默窗口");
        }
    }

    #[test]
    fn test_quiet_window_requires_continuous_silence() {
        let t0 = std::time::Instant::now();
        let mut since = None;
        // 第一次空闲：仅记录起点，不判定
        assert!(!advance_quiet_window(&mut since, 0, t0, 300));
        // 100ms 后仍空闲：未满 300ms，不判定
        assert!(!advance_quiet_window(&mut since, 0, t0 + Duration::from_millis(100), 300));
        // 此时插入一个新请求 → 窗口重置
        assert!(!advance_quiet_window(&mut since, 1, t0 + Duration::from_millis(200), 300));
        // 再来空闲：必须重新计时，200ms 后仍不判定
        assert!(!advance_quiet_window(&mut since, 0, t0 + Duration::from_millis(300), 300));
        assert!(!advance_quiet_window(&mut since, 0, t0 + Duration::from_millis(500), 300));
        // 累计静默 ≥ 300ms 才判定 idle
        assert!(advance_quiet_window(&mut since, 0, t0 + Duration::from_millis(600), 300));
    }

    #[test]
    fn test_wait_for_selector_script_prefers_visible_instance() {
        // 复现实测：`.yh-dialog` 有历史隐藏实例排在前面，旧实现直接判「元素不可见」
        let s = super::super::dom_script::build_interact_script("wait_for_selector", "div.yh-dialog", None);
        assert!(s.contains("findBestVisible"), "应走多实例择优: {s}");
        assert!(s.contains("visible_count"), "应回传可见实例数供诊断: {s}");
        assert!(s.contains("个匹配实例均不可见"), "全部不可见时须明示实例数: {s}");
        assert!(
            !s.contains("const el = findTarget(\"div.yh-dialog\")"),
            "不得再只取第一个匹配实例"
        );
    }

    #[test]
    fn test_find_target_js_prefers_visible_among_duplicates() {
        use super::super::dom_script::FIND_TARGET_JS;
        assert!(FIND_TARGET_JS.contains("isVisibleEl"), "定位器应具备可见性判定能力");
        assert!(
            FIND_TARGET_JS.contains("优先返回第一个可见实例"),
            "多实例时应优先可见者而非 DOM 首个"
        );
    }

    #[test]
    fn test_render_outcome_flags_synthetic_click_warning() {
        // 合成点击兜底必须醒目，不能被埋没
        let outcome = BatchOutcome {
            ok: true,
            total: 1,
            executed: 1,
            skipped: 0,
            action_ms: 10,
            steps: vec![StepResult {
                i: 1,
                op: "click".to_string(),
                target: Some("#save".to_string()),
                desc: None,
                ok: true,
                ms: 10,
                echo: None,
                error: None,
                fallback: Some("js_synthetic_warned".to_string()),
                matched_text: None,
                candidates: None,
                skipped: false,
            }],
            failed_at: None,
            code: None,
            message: None,
            hint: None,
            page: None,
        };
        let md = render_outcome(&outcome);
        assert!(md.contains("合成点击-可能未生效"), "应醒目警告: {md}");
    }
}
