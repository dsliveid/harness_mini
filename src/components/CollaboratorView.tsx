import { useState, useRef, useEffect, useMemo } from "react";
import { useStore } from "../store";
import { computeTurnMetrics } from "../types";
import { ipc } from "../ipc";
import { MessageItem } from "./MessageItem";
import { askConfirm } from "./PromptModal";
import {
  X,
  Square,
  RefreshCw,
  Trash2,
  Send,
  Loader2,
  CheckCircle2,
  Coins,
  Bot,
  Sparkles,
  ArrowUp,
  Folder,
} from "./Icons";

function formatTokens(n?: number | null): string {
  if (n == null || isNaN(n)) return "0";
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(2)}M`;
  if (n >= 10_000) return `${(n / 1_000).toFixed(1)}k`;
  return n.toLocaleString("zh-CN");
}

export function CollaboratorView({ collaboratorId }: { collaboratorId: string }) {
  const currentParentId = useStore((s) => s.currentId);
  const collabs = useStore((s) => (currentParentId ? s.collaborators[currentParentId] ?? [] : []));
  const collab = collabs.find((s) => s.id === collaboratorId);
  const msgs = useStore((s) => s.messages[collaboratorId] ?? []);
  const runStatus = useStore((s) => s.runStatus);
  const isRunning = runStatus[collaboratorId] === "running";

  const setActiveCollaboratorId = useStore((s) => s.setActiveCollaboratorId);
  const setCollaboratorAutoReport = useStore((s) => s.setCollaboratorAutoReport);
  const reportCollaboratorIncrement = useStore((s) => s.reportCollaboratorIncrement);
  const stopSubagent = useStore((s) => s.stopSubagent);
  const restartSubagent = useStore((s) => s.restartSubagent);
  const deleteSubagent = useStore((s) => s.deleteSubagent);
  const pushToast = useStore((s) => s.pushToast);
  const setShowTokenStatsModal = useStore((s) => s.setShowTokenStatsModal);

  const [input, setInput] = useState("");
  const [sending, setSending] = useState(false);
  const [reporting, setReporting] = useState(false);
  const boxRef = useRef<HTMLDivElement>(null);
  const taRef = useRef<HTMLTextAreaElement>(null);
  const stickRef = useRef(true);

  const turnMetricsMap = useMemo(() => computeTurnMetrics(msgs, isRunning), [msgs, isRunning]);

  // 实时聚合协作者会话 Token（结合已完结轮次、历史持久化值与流式中的动态粗估）
  const tokenStats = useMemo(() => {
    let total = 0;
    let prompt = 0;
    let completion = 0;

    for (const msg of msgs) {
      const tt = msg.totalTokens ?? (msg.usage?.totalTokens || (msg.usage?.inputEst || 0) + (msg.usage?.outputEst || 0)) ?? 0;
      const pt = msg.promptTokens ?? (msg.usage?.promptTokens || msg.usage?.inputEst) ?? 0;
      const ct = msg.completionTokens ?? (msg.usage?.completionTokens || msg.usage?.outputEst) ?? 0;

      if (tt > 0 || pt > 0 || ct > 0) {
        total += Number(tt) || 0;
        prompt += Number(pt) || 0;
        completion += Number(ct) || 0;
      } else if (msg.role === "assistant" && isRunning && msg.id === msgs[msgs.length - 1]?.id) {
        const streamedChars = (msg.content?.length || 0) + (msg.reasoning?.length || 0);
        if (streamedChars > 0) {
          const estOutput = Math.max(1, Math.round(streamedChars / 3));
          completion += estOutput;
          total += estOutput;
        }
      }
    }

    const finalTotal = Math.max(total, Number(collab?.totalTokens) || 0);
    const finalPrompt = Math.max(prompt, Number(collab?.promptTokens) || 0);
    const finalCompletion = Math.max(completion, Number(collab?.completionTokens) || 0);

    return {
      totalTokens: finalTotal,
      promptTokens: finalPrompt,
      completionTokens: finalCompletion,
    };
  }, [msgs, isRunning, collab?.totalTokens, collab?.promptTokens, collab?.completionTokens]);

  const resize = () => {
    const ta = taRef.current;
    if (!ta) return;
    ta.style.height = "auto";
    ta.style.height = `${Math.max(32, Math.min(ta.scrollHeight, 200))}px`;
  };

  useEffect(() => {
    resize();
  }, [collaboratorId, input]);

  useEffect(() => {
    const el = boxRef.current;
    if (el && stickRef.current) {
      el.scrollTop = el.scrollHeight;
    }
  }, [msgs, isRunning]);

  const onScroll = () => {
    const el = boxRef.current;
    if (!el) return;
    stickRef.current = el.scrollHeight - el.scrollTop - el.clientHeight < 80;
  };

  const getRoleInfo = (role?: string | null) => {
    switch (role) {
      case "frontend":
        return { label: "前端开发", icon: "🎨", color: "text-blue-400 bg-blue-500/10 border-blue-500/20" };
      case "backend":
        return { label: "后端开发", icon: "⚙️", color: "text-emerald-400 bg-emerald-500/10 border-emerald-500/20" };
      case "testing":
        return { label: "测试校验", icon: "🧪", color: "text-purple-400 bg-purple-500/10 border-purple-500/20" };
      case "review":
        return { label: "代码审阅", icon: "🔍", color: "text-amber-400 bg-amber-500/10 border-amber-500/20" };
      case "fullstack":
        return { label: "全栈开发", icon: "⚡", color: "text-indigo-400 bg-indigo-500/10 border-indigo-500/20" };
      default:
        return { label: "协作者", icon: "🤝", color: "text-zinc-400 bg-zinc-500/10 border-zinc-500/20" };
    }
  };

  const roleInfo = getRoleInfo(collab?.subagentRole);

  const handleSend = async () => {
    const text = input.trim();
    if (!text || sending) return;
    setInput("");
    requestAnimationFrame(resize);
    setSending(true);
    try {
      await ipc.sendMessage(collaboratorId, text);
    } catch (e) {
      pushToast(`发送失败: ${e}`);
      setInput(text);
      requestAnimationFrame(resize);
    } finally {
      setSending(false);
    }
  };

  const onKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key !== "Enter") return;
    if ((e.nativeEvent as unknown as { isComposing?: boolean }).isComposing) return;
    if (e.altKey || e.shiftKey) {
      return;
    }
    e.preventDefault();
    void handleSend();
  };

  const handleDelete = async () => {
    const confirmed = await askConfirm(
      `确定移除协作者「${collab?.title ?? "当前协作者"}」及其所有历史记录？`,
      "移除"
    );
    if (confirmed) {
      await deleteSubagent(collaboratorId);
      setActiveCollaboratorId(null);
    }
  };

  const handleReport = async () => {
    if (reporting) return;
    setReporting(true);
    try {
      await reportCollaboratorIncrement(collaboratorId);
    } finally {
      setReporting(false);
    }
  };

  const handleToggleAutoReport = async () => {
    const nextVal = !(collab?.autoReport ?? true);
    await setCollaboratorAutoReport(collaboratorId, nextVal);
  };

  if (!collab) {
    return (
      <div className="flex-1 flex items-center justify-center p-6 text-inkdim text-center">
        <div>
          <Bot size={32} className="mx-auto mb-2 opacity-40" />
          <div className="text-[13px]">未找到该协作者或已被移除</div>
          <button
            onClick={() => setActiveCollaboratorId(null)}
            className="mt-3 px-3 py-1.5 rounded-lg bg-panel2 hover:bg-panel3 text-[12px] text-ink"
          >
            关闭面板
          </button>
        </div>
      </div>
    );
  }

  const lastUserMsgId = [...msgs].reverse().find((m) => m.role === "user" && !m.queued)?.id;
  const isAutoReport = collab.autoReport ?? true;

  return (
    <div className="flex-1 flex flex-col min-h-0 bg-panel border-l border-edge relative z-10">
      {/* Top Header */}
      <div className="h-12 border-b border-edge bg-panel2/60 backdrop-blur px-3.5 flex items-center justify-between shrink-0 select-none">
        <div className="flex items-center gap-2 min-w-0">
          <span className={`px-2 py-0.5 rounded-md text-[11px] font-medium border flex items-center gap-1 shrink-0 ${roleInfo.color}`}>
            <span>{roleInfo.icon}</span>
            <span>{roleInfo.label}</span>
          </span>
          <span className="font-medium text-[13px] text-ink truncate" title={collab.title}>
            {collab.title}
          </span>
          {collab.workspacePath && (
            <span
              className="hidden lg:inline-flex items-center gap-1 text-[11px] text-inkdim bg-panel2/80 px-1.5 py-0.5 rounded border border-edge/60 max-w-[180px] truncate shrink-0 cursor-default"
              title={`工作区根目录: ${collab.workspacePath}`}
            >
              <Folder size={11} className="shrink-0 opacity-70" />
              <span className="truncate">{collab.workspacePath.split(/[\\/]/).filter(Boolean).pop() || collab.workspacePath}</span>
            </span>
          )}
          {isRunning ? (
            <span className="flex items-center gap-1 text-[11px] text-accent font-medium shrink-0 animate-pulse">
              <Loader2 size={12} className="animate-spin" />
              <span>运行中</span>
            </span>
          ) : (
            <span className="flex items-center gap-1 text-[11px] text-emerald-400 font-medium shrink-0">
              <CheckCircle2 size={12} />
              <span>空闲就绪</span>
            </span>
          )}
        </div>

        {/* Header Action Buttons */}
        <div className="flex items-center gap-1.5 shrink-0">
          {/* Auto-Report Toggle Switch */}
          <button
            type="button"
            onClick={handleToggleAutoReport}
            className={`flex items-center gap-1.5 px-2 py-1 rounded-md text-[11px] border transition-colors cursor-pointer ${
              isAutoReport
                ? "bg-accent/10 border-accent/30 text-accent hover:bg-accent/20"
                : "bg-panel2 border-edge text-inkdim hover:text-ink"
            }`}
            title={isAutoReport ? "已开启：协作者执行完毕后将自动向主任务提交增量汇报（点击切换）" : "已关闭：需手动点击「汇报成果」向主任务汇报（点击切换）"}
          >
            <span className={`w-1.5 h-1.5 rounded-full ${isAutoReport ? "bg-accent" : "bg-zinc-500"}`} />
            <span>自动汇报: {isAutoReport ? "开" : "关"}</span>
          </button>

          {/* Stop Button */}
          {isRunning && (
            <button
              onClick={() => void stopSubagent(collaboratorId)}
              className="flex items-center gap-1 px-2 py-1 rounded-md text-amber-400 hover:bg-amber-500/10 border border-amber-500/30 text-[11px] transition-colors"
              title="停止当前协作者"
            >
              <Square size={11} fill="currentColor" />
              <span>停止</span>
            </button>
          )}

          {/* Manual Incremental Report Button */}
          {!isRunning && msgs.some((m) => m.role === "assistant") && (
            <button
              onClick={handleReport}
              disabled={reporting}
              className="flex items-center gap-1 px-2.5 py-1 rounded-md text-emerald-400 hover:bg-emerald-500/15 border border-emerald-500/30 text-[11px] font-medium transition-colors cursor-pointer"
              title="手动将自上次汇报以来的增量产出与改动汇报给主会话"
            >
              <Send size={11} />
              <span>{reporting ? "汇报中…" : "汇报成果"}</span>
            </button>
          )}

          {/* Restart Button */}
          {!isRunning && (
            <button
              onClick={() => void restartSubagent(collaboratorId)}
              className="flex items-center gap-1 px-2 py-1 rounded-md text-inkdim hover:text-ink hover:bg-panel2 border border-edge text-[11px] transition-colors"
              title="重启 / 继续当前协作者"
            >
              <RefreshCw size={11} />
              <span>重启</span>
            </button>
          )}

          {/* Delete Button */}
          <button
            onClick={handleDelete}
            className="w-7 h-7 rounded-md hover:bg-rose-500/10 hover:text-rose-400 text-inkdim flex items-center justify-center transition-colors"
            title="移除协作者"
          >
            <Trash2 size={13} />
          </button>

          {/* Close Panel Button */}
          <button
            onClick={() => setActiveCollaboratorId(null)}
            className="w-7 h-7 rounded-md hover:bg-panel2 text-inkdim hover:text-ink flex items-center justify-center transition-colors ml-1"
            title="关闭协作者面板"
          >
            <X size={15} />
          </button>
        </div>
      </div>

      {/* Messages Scroll Area */}
      <div className="flex-1 overflow-y-auto px-4 py-4 space-y-4" ref={boxRef} onScroll={onScroll}>
        {msgs.length === 0 ? (
          <div className="flex flex-col items-center justify-center py-12 text-center text-inkdim">
            <div className="w-10 h-10 rounded-xl bg-panel2 border border-edge flex items-center justify-center mb-3 text-accent">
              <Sparkles size={20} />
            </div>
            <div className="text-[13px] font-medium text-ink mb-1">协作者已就绪</div>
            <div className="text-[12px] max-w-[280px]">
              {collab.subagentTask ? `专业职责：${collab.subagentTask}` : "已加入协作团队。主进程在安排相关任务时将优先委派，您也可以在下方直接向其派发指令。"}
            </div>
          </div>
        ) : (
          msgs
            .filter((m) => m.role !== "tool")
            .map((m) => (
              <MessageItem
                key={m.id}
                msg={m}
                isLastUser={m.id === lastUserMsgId}
                streaming={isRunning && m.role === "assistant" && m.id === msgs[msgs.length - 1]?.id}
                readOnly={false}
                running={isRunning}
                turnMetrics={turnMetricsMap.get(m.id)}
              />
            ))
        )}
      </div>

      {/* Bottom Composer for Collaborator */}
      <div className="p-3">
        <div className="flex items-end gap-2 bg-panel2/90 border border-edge rounded-2xl px-3.5 py-2.5 shadow-sm focus-within:border-accent/60 focus-within:ring-1 focus-within:ring-accent/20 transition-all">
          <textarea
            ref={taRef}
            rows={1}
            value={input}
            onChange={(e) => {
              setInput(e.target.value);
            }}
            onKeyDown={onKeyDown}
            placeholder={
              isRunning
                ? "协作者运行中…输入指令追加到待执行队列"
                : "向该协作者手动安排任务或发送补充指令，Enter 发送"
            }
            style={{ height: 32 }}
            className="flex-1 bg-transparent outline-none resize-none text-[14px] leading-relaxed max-h-[200px] min-h-[32px] py-1 disabled:opacity-50 placeholder:text-inkdim/60"
          />
          {isRunning ? (
            <button
              type="button"
              onClick={() => void stopSubagent(collaboratorId)}
              className="shrink-0 w-8 h-8 rounded-xl bg-red-600/90 hover:bg-red-500 text-white flex items-center justify-center transition-colors shadow-sm cursor-pointer"
              title="停止协作者"
            >
              <Square size={13} className="fill-current" />
            </button>
          ) : (
            <button
              type="button"
              disabled={!input.trim() || sending}
              onClick={handleSend}
              className="shrink-0 w-8 h-8 rounded-xl bg-accent hover:bg-blue-500 disabled:opacity-30 disabled:cursor-not-allowed text-white flex items-center justify-center transition-all shadow-sm cursor-pointer"
              title="发送（Enter）"
            >
              <ArrowUp size={16} strokeWidth={2.4} />
            </button>
          )}
        </div>

        {/* Bottom Status Bar */}
        <div className="h-6 flex items-center justify-between text-[11px] text-inkdim mt-2 px-1 select-none gap-3">
          <div className="flex items-center gap-2 min-w-0 flex-1">
            <span className="truncate max-w-[240px]" title={collab.workspacePath || "继承主项目工作区"}>
              {collab.workspacePath ? `工作区：${collab.workspacePath}` : "继承主工作区"}
            </span>
          </div>

          <div className="flex items-center gap-2.5 shrink-0">
            <button
              type="button"
              className={`inline-flex items-center gap-1.5 px-2 py-0.5 rounded-md text-[11px] transition-all font-mono border cursor-pointer ${
                tokenStats.totalTokens > 0
                  ? "bg-panel2/80 hover:bg-panel3 border-edge/80 hover:border-amber-400/40 text-ink shadow-xs"
                  : "hover:bg-panel2 text-inkdim hover:text-ink border-transparent hover:border-edge/50"
              }`}
              onClick={() => setShowTokenStatsModal(true)}
              title={
                tokenStats.totalTokens > 0
                  ? `该协作者累计消耗: ${tokenStats.totalTokens.toLocaleString()} tokens\n输入: ${tokenStats.promptTokens.toLocaleString()} · 输出: ${tokenStats.completionTokens.toLocaleString()}\n点击打开 Token 消耗统计看板`
                  : "协作者独立会话消耗 · 点击打开 Token 统计看板"
              }
            >
              <Coins
                size={12}
                className={
                  tokenStats.totalTokens > 0
                    ? "text-amber-400 shrink-0"
                    : "text-inkdim group-hover:text-amber-400 transition-colors shrink-0"
                }
              />
              <span className={tokenStats.totalTokens > 0 ? "font-medium text-ink" : "text-inkdim"}>
                {formatTokens(tokenStats.totalTokens)}
              </span>
              <span className="text-[10px] text-inkdim">tokens</span>
            </button>

            <span className="hidden sm:inline-block opacity-65">
              <kbd className="px-1 py-0.5 rounded bg-panel3 border border-edge text-[10px]">Enter</kbd> 发送
            </span>
          </div>
        </div>
      </div>
    </div>
  );
}
