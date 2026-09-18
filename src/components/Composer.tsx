import { useEffect, useRef } from "react";
import { ipc } from "../ipc";
import { useStore } from "../store";
import { DRAFT_ID } from "../types";
import { ArrowUp, Square, Coins } from "./Icons";

function formatTokens(n?: number | null): string {
  if (n == null || isNaN(n)) return "0";
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(2)}M`;
  if (n >= 10_000) return `${(n / 1_000).toFixed(1)}k`;
  return n.toLocaleString("zh-CN");
}

export function Composer() {
  const currentId = useStore((s) => s.currentId);
  const activeId = currentId ?? DRAFT_ID;
  const text = useStore((s) => s.sessionDrafts[activeId] ?? "");
  const setSessionDraft = useStore((s) => s.setSessionDraft);
  const running = useStore((s) => (s.currentId ? s.runStatus[s.currentId] === "running" : false));
  const readOnly = useStore((s) => s.readOnly);
  const session = useStore((s) => (s.currentId && s.currentId !== DRAFT_ID ? s.sessions.find((x) => x.id === s.currentId) ?? null : null));
  const draft = useStore((s) => s.draft);
  const tempInfo = useStore((s) => s.tempInfo);
  const selectSession = useStore((s) => s.selectSession);
  const pushToast = useStore((s) => s.pushToast);
  const setShowTokenStatsModal = useStore((s) => s.setShowTokenStatsModal);

  const taRef = useRef<HTMLTextAreaElement>(null);

  // 临时空间对话：已合并且临时空间未清空期间禁止发送（清空后可继续）
  const tempBlocked = !!(
    session?.mergedPending &&
    session.isTemp &&
    (tempInfo[session.id]?.exists ?? false)
  );

  // 空对话（未选中会话/临时对话）也可发送：首条消息发送后由后端自动落库为新会话
  const canSend = !readOnly && !tempBlocked;
  const placeholder = tempBlocked
    ? "已合并到原项目，清空临时空间后可继续发送消息"
    : readOnly
    ? "已归档会话为只读，取消归档后可继续对话"
    : running
    ? "Agent 运行中…输入消息回车将加入待执行列表"
    : "输入消息，Enter 发送，Alt+Enter / Shift+Enter 换行";

  const resize = () => {
    const ta = taRef.current;
    if (!ta) return;
    ta.style.height = "auto";
    ta.style.height = `${Math.max(32, Math.min(ta.scrollHeight, 200))}px`;
  };

  useEffect(() => {
    resize();
  }, [activeId, text]);

  const insertNewline = () => {
    const ta = taRef.current;
    if (!ta) return;
    const start = ta.selectionStart;
    const end = ta.selectionEnd;
    const next = text.slice(0, start) + "\n" + text.slice(end);
    setSessionDraft(activeId, next);
    requestAnimationFrame(() => {
      ta.selectionStart = ta.selectionEnd = start + 1;
      resize();
    });
  };

  const doSend = async () => {
    const t = text.trim();
    if (!t || !canSend) return;
    const targetId = activeId;
    setSessionDraft(targetId, "");
    requestAnimationFrame(resize);
    try {
      const st = useStore.getState();
      // 未选中会话（含临时对话）时按新对话发送，后端自动创建会话保存
      const isDraftLike = !currentId || currentId === DRAFT_ID;
      const draft = st.draft;
      const res = await ipc.sendMessage(
        isDraftLike ? null : currentId,
        t,
        isDraftLike ? draft?.workspacePath ?? undefined : undefined,
        isDraftLike ? draft?.projectId ?? st.currentProjectId ?? undefined : undefined,
        isDraftLike ? draft?.temp ?? undefined : undefined,
        // 访问模式为会话级：新对话落库时带上草稿上的取值（已从“上一条对话”继承）
        isDraftLike ? draft?.accessMode ?? undefined : undefined,
        isDraftLike ? draft?.contextTokenLimit ?? undefined : undefined
      );
      const st2 = useStore.getState();
      // 后端随结果带回会话实体：切换前先入列，避免「currentId 已切换、会话事件未到达」
      // 期间顶栏/输入框按未保存草稿渲染造成闪现
      if (res.session) st2.onSessionUpdate(res.session);
      // 已触发运行：立即进入运行态，停止按钮不再等待 run:status 事件（事件稍后覆盖为同一状态）
      if (!res.queued && res.sessionId && st2.runStatus[res.sessionId] === undefined) {
        st2.onRunStatus({ sessionId: res.sessionId, status: "running" });
      }
      if (isDraftLike && res.sessionId) {
        await st2.selectSession(res.sessionId);
      }
    } catch (e) {
      pushToast(String(e));
      setSessionDraft(targetId, t); // 发送失败恢复原会话草稿内容
    }
  };

  const onKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key !== "Enter") return;
    // 中文输入法组合态回车不发送
    if ((e.nativeEvent as unknown as { isComposing?: boolean }).isComposing) return;
    if (e.altKey || e.shiftKey) {
      // Alt+Enter / Shift+Enter 换行
      e.preventDefault();
      insertNewline();
      return;
    }
    e.preventDefault();
    void doSend();
  };

  const effectiveWorkspace = session?.workspacePath ?? draft?.workspacePath ?? "";

  return (
    <div className="p-3">
      <div className="flex items-end gap-2 bg-panel2/90 border border-edge rounded-2xl px-3.5 py-2.5 shadow-sm focus-within:border-accent/60 focus-within:ring-1 focus-within:ring-accent/20 transition-all">
        <textarea
          ref={taRef}
          style={{ height: 32 }}
          className="flex-1 bg-transparent outline-none resize-none text-[14px] leading-relaxed max-h-[200px] min-h-[32px] py-1 disabled:opacity-50 placeholder:text-inkdim/60"
          rows={1}
          value={text}
          placeholder={placeholder}
          disabled={!canSend}
          onChange={(e) => {
            setSessionDraft(activeId, e.target.value);
          }}
          onKeyDown={onKeyDown}
        />
        {running ? (
          <button
            className="shrink-0 w-8 h-8 rounded-xl bg-red-600/90 hover:bg-red-500 text-white flex items-center justify-center transition-colors shadow-sm"
            title="停止生成"
            onClick={() => currentId && useStore.getState().stopRun(currentId)}
          >
            <Square size={13} className="fill-current" />
          </button>
        ) : (
          <button
            className="shrink-0 w-8 h-8 rounded-xl bg-accent hover:bg-blue-500 text-white flex items-center justify-center disabled:opacity-30 disabled:cursor-not-allowed transition-all shadow-sm"
            title="发送（Enter）"
            disabled={!canSend || !text.trim()}
            onClick={() => void doSend()}
          >
            <ArrowUp size={16} strokeWidth={2.4} />
          </button>
        )}
      </div>
      <div className="h-6 flex items-center justify-between text-[11px] text-inkdim mt-2 px-1 select-none gap-3">
        <div className="flex items-center gap-2 min-w-0 flex-1">
          <span className="truncate max-w-[320px] md:max-w-[440px]" title={effectiveWorkspace || undefined}>
            {effectiveWorkspace || "未绑定工作区 · 可直接对话（文件/命令工具不可用）"}
          </span>
        </div>

        <div className="flex items-center gap-2.5 shrink-0">
          {/* 会话 Token 消耗徽章（点击直达统计看板） */}
          {session ? (
            <button
              type="button"
              className={`inline-flex items-center gap-1.5 px-2 py-0.5 rounded-md text-[11px] transition-all group font-mono cursor-pointer border ${
                (session.totalTokens ?? 0) > 0
                  ? "bg-panel2/80 hover:bg-panel3 border-edge/80 hover:border-amber-400/40 text-ink shadow-xs"
                  : "hover:bg-panel2 text-inkdim hover:text-ink border-transparent hover:border-edge/50"
              }`}
              onClick={() => setShowTokenStatsModal(true)}
              title={`该会话累计消耗: ${(session.totalTokens ?? 0).toLocaleString()} tokens\n输入: ${(session.promptTokens ?? 0).toLocaleString()} · 输出: ${(session.completionTokens ?? 0).toLocaleString()}\n点击打开 Token 消耗统计看板`}
            >
              <Coins
                size={12}
                className={
                  (session.totalTokens ?? 0) > 0
                    ? "text-amber-400 group-hover:scale-110 transition-transform shrink-0"
                    : "text-inkdim group-hover:text-amber-400 transition-colors shrink-0"
                }
              />
              <span className={(session.totalTokens ?? 0) > 0 ? "font-medium text-ink" : "text-inkdim"}>
                {formatTokens(session.totalTokens ?? 0)}
              </span>
              <span className="text-[10px] text-inkdim">tokens</span>
            </button>
          ) : (
            <button
              type="button"
              className="inline-flex items-center gap-1 px-1.5 py-0.5 rounded-md hover:bg-panel2 text-[11px] text-inkdim hover:text-ink transition-colors font-mono cursor-pointer border border-transparent hover:border-edge/50"
              onClick={() => setShowTokenStatsModal(true)}
              title="点击打开 Token 消耗统计看板"
            >
              <Coins size={12} className="text-inkdim group-hover:text-amber-400 transition-colors shrink-0" />
              <span>Token 统计</span>
            </button>
          )}

          <span className="hidden sm:inline-block opacity-65">
            <kbd className="px-1 py-0.5 rounded bg-panel3 border border-edge text-[10px]">Enter</kbd> 发送 ·{" "}
            <kbd className="px-1 py-0.5 rounded bg-panel3 border border-edge text-[10px]">Shift+Enter</kbd> 换行
          </span>
        </div>
      </div>
    </div>
  );
}
