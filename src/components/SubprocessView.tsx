import { useState, useRef, useEffect, useMemo } from "react";
import { useStore } from "../store";
import { computeTurnMetrics } from "../types";
import { ipc } from "../ipc";
import { MessageItem } from "./MessageItem";
import {
  X,
  Square,
  RefreshCw,
  Loader2,
  CheckCircle2,
  AlertCircle,
  Coins,
  Bot,
  Sparkles,
  ArrowUp,
  Folder,
  Cpu,
  Eye,
} from "./Icons";

function formatTokens(n?: number | null): string {
  if (n == null || isNaN(n)) return "0";
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(2)}M`;
  if (n >= 10_000) return `${(n / 1_000).toFixed(1)}k`;
  return n.toLocaleString("zh-CN");
}

const ROLE_INFO: Record<string, { label: string; icon: string; color: string }> = {
  frontend: { label: "前端开发", icon: "🎨", color: "text-blue-400 bg-blue-500/10 border-blue-500/20" },
  backend: { label: "后端开发", icon: "⚙️", color: "text-emerald-400 bg-emerald-500/10 border-emerald-500/20" },
  testing: { label: "测试校验", icon: "🧪", color: "text-purple-400 bg-purple-500/10 border-purple-500/20" },
  review: { label: "代码审阅", icon: "🔍", color: "text-amber-400 bg-amber-500/10 border-amber-500/20" },
  fullstack: { label: "全栈开发", icon: "⚡", color: "text-indigo-400 bg-indigo-500/10 border-indigo-500/20" },
};

export function SubprocessView({ subprocessId }: { subprocessId: string }) {
  const currentParentId = useStore((s) => s.currentId);
  const storeSubprocesses = useStore((s) => s.subprocesses);
  const storeSubagents = useStore((s) => s.subagents);
  const storeSessions = useStore((s) => s.sessions);
  const currentMessages = useStore((s) => (currentParentId ? s.messages[currentParentId] ?? [] : []));

  // 1. 全维定位子进程 Session 对象
  const sub = useMemo(() => {
    // A: 优先在当前父会话列表查找
    const directSubs = currentParentId ? storeSubprocesses[currentParentId] ?? [] : [];
    const directAgents = currentParentId ? storeSubagents[currentParentId] ?? [] : [];
    let found = directSubs.find((s) => s.id === subprocessId) ?? directAgents.find((s) => s.id === subprocessId);
    if (found) return found;

    // B: 全局所有已缓存的子进程与会话
    const allSubs = [
      ...Object.values(storeSubprocesses).flat(),
      ...Object.values(storeSubagents).flat(),
      ...storeSessions,
    ];
    found = allSubs.find((s) => s.id === subprocessId);
    if (found) return found;

    // C: 如果 subprocessId 传过来的是 ev.id（如 evt_xxx），通过消息工具事件逆向匹配
    for (const msg of currentMessages) {
      for (const ev of msg.toolEvents ?? []) {
        if (ev.id === subprocessId || ev.params?.subprocess_id === subprocessId) {
          const title = String(ev.params?.title || "").trim();
          const task = String(ev.params?.task || "").trim();
          const matched = allSubs.find((s) => (title && s.title === title) || (task && s.subagentTask === task));
          if (matched) return matched;
        }
      }
    }

    return null;
  }, [subprocessId, currentParentId, storeSubprocesses, storeSubagents, storeSessions, currentMessages]);

  const realSubId = sub?.id || subprocessId;

  const msgs = useStore((s) => s.messages[realSubId] ?? s.messages[subprocessId] ?? []);
  const runStatus = useStore((s) => s.runStatus);
  const isRunning = runStatus[realSubId] === "running" || runStatus[subprocessId] === "running";

  const setActiveSubprocessId = useStore((s) => s.setActiveSubprocessId);
  const stopSubagent = useStore((s) => s.stopSubagent);
  const restartSubagent = useStore((s) => s.restartSubagent);
  const pushToast = useStore((s) => s.pushToast);
  const setShowTokenStatsModal = useStore((s) => s.setShowTokenStatsModal);

  const [loading, setLoading] = useState(!sub);
  const [input, setInput] = useState("");
  const [sending, setSending] = useState(false);
  const [stopping, setStopping] = useState(false);
  const boxRef = useRef<HTMLDivElement>(null);
  const taRef = useRef<HTMLTextAreaElement>(null);
  const stickRef = useRef(true);

  // 异步主动拉取重试：如果本地暂时未命中，向后端主动拉取最新子进程列表
  useEffect(() => {
    let cancelled = false;
    if (!sub && currentParentId) {
      setLoading(true);
      Promise.all([
        ipc.listSubprocesses(currentParentId),
        ipc.listSubagents(currentParentId),
      ])
        .then(([subprocs, subs]) => {
          if (cancelled) return;
          useStore.setState((st) => ({
            subprocesses: { ...st.subprocesses, [currentParentId]: subprocs },
            subagents: { ...st.subagents, [currentParentId]: subs },
          }));
        })
        .catch(() => {})
        .finally(() => {
          if (!cancelled) {
            setTimeout(() => {
              if (!cancelled) setLoading(false);
            }, 300);
          }
        });
    } else {
      setLoading(false);
    }

    return () => {
      cancelled = true;
    };
  }, [subprocessId, currentParentId, sub]);

  // 拉取真实会话的历史消息与运行快照
  useEffect(() => {
    if (realSubId) {
      ipc.getMessages(realSubId, undefined, 200).then((messages) => {
        useStore.setState((st) => ({
          messages: { ...st.messages, [realSubId]: messages },
        }));
      }).catch(() => {});
      void useStore.getState().syncSessionActiveState(realSubId);
    }
  }, [realSubId]);

  const turnMetricsMap = useMemo(() => computeTurnMetrics(msgs, isRunning), [msgs, isRunning]);

  // 实时聚合子进程 Token
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

    const finalTotal = Math.max(total, Number(sub?.totalTokens) || 0);
    const finalPrompt = Math.max(prompt, Number(sub?.promptTokens) || 0);
    const finalCompletion = Math.max(completion, Number(sub?.completionTokens) || 0);

    return {
      totalTokens: finalTotal,
      promptTokens: finalPrompt,
      completionTokens: finalCompletion,
    };
  }, [msgs, isRunning, sub?.totalTokens, sub?.promptTokens, sub?.completionTokens]);

  const resize = () => {
    const ta = taRef.current;
    if (!ta) return;
    ta.style.height = "auto";
    ta.style.height = `${Math.max(32, Math.min(ta.scrollHeight, 200))}px`;
  };

  useEffect(() => {
    resize();
  }, [realSubId, input]);

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

  const roleMeta = ROLE_INFO[sub?.subagentRole || ""] ?? {
    label: "子进程",
    icon: "⚡",
    color: "text-indigo-400 bg-indigo-500/10 border-indigo-500/20",
  };

  const handleStop = async () => {
    if (stopping || !realSubId) return;
    setStopping(true);
    try {
      await stopSubagent(realSubId);
      pushToast("已发送终止指令");
    } catch (e) {
      pushToast(`停止失败: ${e}`);
    } finally {
      setStopping(false);
    }
  };

  const handleSend = async () => {
    const text = input.trim();
    if (!text || sending || !realSubId) return;
    setInput("");
    requestAnimationFrame(resize);
    setSending(true);
    try {
      await ipc.sendMessage(realSubId, text);
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

  // 正在加载中的友好过渡态
  if (!sub && loading) {
    return (
      <div className="flex-1 flex items-center justify-center p-6 text-inkdim text-center bg-panel border-l border-edge">
        <div className="flex flex-col items-center">
          <Loader2 size={32} className="mx-auto mb-3 animate-spin text-accent" />
          <div className="text-[13px] font-medium text-ink">正在拉取子进程信息…</div>
          <div className="text-[11px] text-inkdim mt-1">正在从后端同步会话与最新执行状态</div>
        </div>
      </div>
    );
  }

  // 最终未能找到时的安全兜底
  if (!sub) {
    return (
      <div className="flex-1 flex items-center justify-center p-6 text-inkdim text-center bg-panel border-l border-edge">
        <div>
          <Bot size={32} className="mx-auto mb-2 opacity-40 text-indigo-400" />
          <div className="text-[13px] font-medium text-ink">未找到该子进程或已被清理</div>
          <div className="text-[11px] text-inkdim mt-1">该子任务可能尚未完全创建或已被系统删除</div>
          <div className="flex items-center justify-center gap-2 mt-4">
            <button
              onClick={() => {
                if (currentParentId) {
                  setLoading(true);
                  void useStore.getState().loadSubprocesses(currentParentId);
                }
              }}
              className="px-3 py-1.5 rounded-lg bg-panel2 hover:bg-panel3 text-[12px] text-ink flex items-center gap-1 border border-edge"
            >
              <RefreshCw size={12} />
              <span>重新拉取</span>
            </button>
            <button
              onClick={() => setActiveSubprocessId(null)}
              className="px-3 py-1.5 rounded-lg bg-panel2 hover:bg-panel3 text-[12px] text-ink border border-edge"
            >
              关闭分屏
            </button>
          </div>
        </div>
      </div>
    );
  }

  const lastUserMsgId = [...msgs].reverse().find((m) => m.role === "user" && !m.queued)?.id;

  return (
    <div className="flex-1 flex flex-col min-h-0 bg-panel border-l border-edge relative z-10">
      {/* Top Header */}
      <div className="h-12 border-b border-edge bg-panel2/60 backdrop-blur px-3.5 flex items-center justify-between shrink-0 select-none">
        <div className="flex items-center gap-2 min-w-0 flex-1 mr-2">
          <div className="flex items-center gap-1.5 px-1.5 py-0.5 rounded bg-indigo-500/15 text-indigo-400 border border-indigo-500/30 text-[10px] font-medium shrink-0">
            <Cpu size={11} />
            <span>子进程</span>
          </div>

          <span className={`px-2 py-0.5 rounded-md text-[11px] font-medium border flex items-center gap-1 shrink-0 ${roleMeta.color}`}>
            <span>{roleMeta.icon}</span>
            <span>{roleMeta.label}</span>
          </span>

          <span className="font-medium text-[13px] text-ink truncate" title={sub.title}>
            {sub.title}
          </span>

          {sub.workspacePath && (
            <span
              className="hidden lg:inline-flex items-center gap-1 text-[11px] text-inkdim bg-panel2/80 px-1.5 py-0.5 rounded border border-edge/60 max-w-[160px] truncate shrink-0 cursor-default"
              title={`工作区路径: ${sub.workspacePath}`}
            >
              <Folder size={11} className="shrink-0 opacity-70" />
              <span className="truncate">{sub.workspacePath.split(/[\\/]/).filter(Boolean).pop() || sub.workspacePath}</span>
            </span>
          )}

          {isRunning ? (
            <span className="flex items-center gap-1 text-[11px] text-accent font-medium shrink-0 animate-pulse">
              <Loader2 size={12} className="animate-spin" />
              <span>运行中</span>
            </span>
          ) : sub.status === "failed" ? (
            <span className="flex items-center gap-1 text-[11px] text-rose-400 font-medium shrink-0">
              <AlertCircle size={12} />
              <span>已终止</span>
            </span>
          ) : (
            <span className="flex items-center gap-1 text-[11px] text-emerald-400 font-medium shrink-0">
              <CheckCircle2 size={12} />
              <span>已完成</span>
            </span>
          )}
        </div>

        {/* Top Header Controls */}
        <div className="flex items-center gap-1.5 shrink-0">
          <span className="hidden sm:inline-flex items-center gap-1 text-[10px] text-inkdim bg-panel border border-edge/60 px-1.5 py-0.5 rounded">
            <Eye size={10} className="opacity-70" />
            <span>只读流程</span>
          </span>

          {isRunning && (
            <button
              onClick={handleStop}
              disabled={stopping}
              className="flex items-center gap-1 px-2 py-1 rounded-md text-red-400 hover:bg-red-500/10 border border-red-500/30 text-[11px] transition-colors cursor-pointer"
              title="终止该子进程运行"
            >
              <Square size={11} fill="currentColor" />
              <span>{stopping ? "停止中…" : "终止"}</span>
            </button>
          )}

          {!isRunning && (
            <button
              onClick={() => void restartSubagent(realSubId)}
              className="flex items-center gap-1 px-2 py-1 rounded-md text-inkdim hover:text-ink hover:bg-panel2 border border-edge text-[11px] transition-colors"
              title="重新启动 / 继续该子进程"
            >
              <RefreshCw size={11} />
              <span>重试</span>
            </button>
          )}

          <button
            onClick={() => setActiveSubprocessId(null)}
            className="w-7 h-7 rounded-md hover:bg-panel2 text-inkdim hover:text-ink flex items-center justify-center transition-colors ml-1"
            title="关闭子进程分屏"
          >
            <X size={15} />
          </button>
        </div>
      </div>

      {/* Task Prompt Overview Banner */}
      {sub.subagentTask && (
        <div className="px-4 py-2.5 bg-panel2/40 border-b border-edge/60 text-[12px] text-ink shrink-0">
          <div className="text-[11px] font-medium text-inkdim mb-1 flex items-center gap-1.5">
            <Bot size={12} className="text-indigo-400" />
            <span>主进程分派任务目标：</span>
          </div>
          <div className="whitespace-pre-wrap max-h-[80px] overflow-y-auto font-mono text-[11px] text-inkdim leading-relaxed bg-panel/60 p-2 rounded border border-edge/40">
            {sub.subagentTask}
          </div>
        </div>
      )}

      {/* Messages Scroll Area */}
      <div className="flex-1 overflow-y-auto px-4 py-4 space-y-4" ref={boxRef} onScroll={onScroll}>
        {msgs.length === 0 ? (
          <div className="flex flex-col items-center justify-center py-16 text-center text-inkdim">
            <div className="w-10 h-10 rounded-xl bg-panel2 border border-edge flex items-center justify-center mb-3 text-indigo-400">
              <Sparkles size={20} />
            </div>
            <div className="text-[13px] font-medium text-ink mb-1">子进程已就绪</div>
            <div className="text-[12px] max-w-[280px]">
              正在独立上下文中运行，任务结果与改动将自动汇聚至主进程。
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
                readOnly={true}
                running={isRunning}
                turnMetrics={turnMetricsMap.get(m.id)}
              />
            ))
        )}
      </div>

      {/* Bottom Auxiliary & Composer */}
      <div className="p-3 bg-panel border-t border-edge/40">
        <div className="flex items-end gap-2 bg-panel2/90 border border-edge rounded-2xl px-3.5 py-2.5 shadow-sm focus-within:border-accent/60 focus-within:ring-1 focus-within:ring-accent/20 transition-all">
          <textarea
            ref={taRef}
            rows={1}
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={onKeyDown}
            placeholder={
              isRunning
                ? "子进程自主运行中…可输入补充提示追加到队列"
                : "输入消息向该子进程下发补充指导，Enter 发送"
            }
            style={{ height: 32 }}
            className="flex-1 bg-transparent outline-none resize-none text-[14px] leading-relaxed max-h-[200px] min-h-[32px] py-1 disabled:opacity-50 placeholder:text-inkdim/60"
          />
          {isRunning ? (
            <button
              type="button"
              onClick={handleStop}
              className="shrink-0 w-8 h-8 rounded-xl bg-red-600/90 hover:bg-red-500 text-white flex items-center justify-center transition-colors shadow-sm cursor-pointer"
              title="停止子进程"
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

        {/* Bottom Status Info Bar */}
        <div className="h-6 flex items-center justify-between text-[11px] text-inkdim mt-2 px-1 select-none gap-3">
          <div className="flex items-center gap-2 min-w-0 flex-1">
            <span className="truncate max-w-[240px]" title={sub.workspacePath || "继承主工作区"}>
              {sub.workspacePath ? `工作区：${sub.workspacePath}` : "继承主工作区"}
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
                  ? `该子进程消耗: ${tokenStats.totalTokens.toLocaleString()} tokens\n输入: ${tokenStats.promptTokens.toLocaleString()} · 输出: ${tokenStats.completionTokens.toLocaleString()}\n点击打开 Token 统计看板`
                  : "子进程消耗 · 点击打开 Token 统计看板"
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
