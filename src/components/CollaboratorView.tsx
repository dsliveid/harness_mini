import { useState, useRef, useEffect, useMemo } from "react";
import { useStore } from "../store";
import { computeTurnMetrics, type Session, type Message } from "../types";
import { ipc } from "../ipc";
import { MessageItem } from "./MessageItem";
import { ExecutionProcessBlock, groupTimelineItems } from "./ExecutionProcessBlock";
import { askConfirm } from "./PromptModal";
import {
  X,
  Square,
  RefreshCw,
  Trash2,
  Send,
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
  Pencil,
  MoreHorizontal,
} from "./Icons";

function formatTokens(n?: number | null): string {
  if (n == null || isNaN(n)) return "0";
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(2)}M`;
  if (n >= 10_000) return `${(n / 1_000).toFixed(1)}k`;
  return n.toLocaleString("zh-CN");
}

const EMPTY_SESSIONS_LIST: Session[] = [];
const EMPTY_MESSAGES_LIST: Message[] = [];

export function CollaboratorView({ collaboratorId }: { collaboratorId: string }) {
  const currentParentId = useStore((s) => s.currentId);
  const allCollabs = useStore((s) => s.collaborators);
  const allSubprocs = useStore((s) => s.subprocesses);
  const allSubagents = useStore((s) => s.subagents);
  const allSessions = useStore((s) => s.sessions);

  const collabs = useMemo(
    () => (currentParentId ? allCollabs[currentParentId] ?? EMPTY_SESSIONS_LIST : EMPTY_SESSIONS_LIST),
    [allCollabs, currentParentId]
  );
  const subprocs = useMemo(
    () => (currentParentId ? allSubprocs[currentParentId] ?? EMPTY_SESSIONS_LIST : EMPTY_SESSIONS_LIST),
    [allSubprocs, currentParentId]
  );
  const subagents = useMemo(
    () => (currentParentId ? allSubagents[currentParentId] ?? EMPTY_SESSIONS_LIST : EMPTY_SESSIONS_LIST),
    [allSubagents, currentParentId]
  );

  // 全维查找对应的会话实体（协作者或子进程）
  const collab = useMemo(() => {
    return (
      collabs.find((s) => s.id === collaboratorId) ??
      subprocs.find((s) => s.id === collaboratorId) ??
      subagents.find((s) => s.id === collaboratorId) ??
      allSessions.find((s) => s.id === collaboratorId) ??
      null
    );
  }, [collabs, subprocs, subagents, allSessions, collaboratorId]);

  const isSubprocess = collab?.sessionType === "subprocess" || collab?.sessionType === "subagent";

  const allMessages = useStore((s) => s.messages);
  const msgs = useMemo(() => allMessages[collaboratorId] ?? EMPTY_MESSAGES_LIST, [allMessages, collaboratorId]);
  const runStatus = useStore((s) => s.runStatus);
  const isRunning = runStatus[collaboratorId] === "running";

  const setActiveCollaboratorId = useStore((s) => s.setActiveCollaboratorId);
  const setActiveSubprocessId = useStore((s) => s.setActiveSubprocessId);
  const setCollaboratorAutoReport = useStore((s) => s.setCollaboratorAutoReport);
  const reportCollaboratorIncrement = useStore((s) => s.reportCollaboratorIncrement);
  const stopSubagent = useStore((s) => s.stopSubagent);
  const restartSubagent = useStore((s) => s.restartSubagent);
  const deleteCollaborator = useStore((s) => s.deleteCollaborator);
  const pushToast = useStore((s) => s.pushToast);
  const setShowTokenStatsModal = useStore((s) => s.setShowTokenStatsModal);

  const [loading, setLoading] = useState(!collab);
  const [input, setInput] = useState("");
  const [sending, setSending] = useState(false);
  const [reporting, setReporting] = useState(false);
  const boxRef = useRef<HTMLDivElement>(null);
  const taRef = useRef<HTMLTextAreaElement>(null);
  const stickRef = useRef(true);
  const headerRef = useRef<HTMLDivElement>(null);
  const [headerWidth, setHeaderWidth] = useState(500);
  const [openMoreMenu, setOpenMoreMenu] = useState(false);

  useEffect(() => {
    const el = headerRef.current;
    if (!el) return;
    const ro = new ResizeObserver((entries) => {
      for (const entry of entries) {
        setHeaderWidth(entry.contentRect.width);
      }
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const isWide = headerWidth >= 520;
  const isCompact = headerWidth < 520;
  const isAutoReportVisible = headerWidth >= 390;
  const isVeryCompact = headerWidth < 380;

  // 异步自动拉取与状态同步：若本地尚未命中，立即从后端重试同步
  useEffect(() => {
    let cancelled = false;
    if (!collab && currentParentId) {
      setLoading(true);
      Promise.all([
        ipc.listCollaborators(currentParentId),
        ipc.listSubprocesses(currentParentId),
        ipc.listSubagents(currentParentId),
      ])
        .then(([cList, spList, saList]) => {
          if (cancelled) return;
          useStore.setState((st) => ({
            collaborators: { ...st.collaborators, [currentParentId]: cList },
            subprocesses: { ...st.subprocesses, [currentParentId]: spList },
            subagents: { ...st.subagents, [currentParentId]: saList },
          }));
        })
        .catch(() => {})
        .finally(() => {
          if (!cancelled) setLoading(false);
        });
    } else {
      setLoading(false);
    }
    return () => {
      cancelled = true;
    };
  }, [collaboratorId, currentParentId, collab]);

  // 打开时自动拉取该会话的消息与快照
  useEffect(() => {
    if (collaboratorId && !useStore.getState().messages[collaboratorId]) {
      ipc.getMessages(collaboratorId, undefined, 200).then((messages) => {
        useStore.setState((st) => ({
          messages: { ...st.messages, [collaboratorId]: messages },
        }));
      }).catch(() => {});
      void useStore.getState().syncSessionActiveState(collaboratorId);
    }
  }, [collaboratorId]);

  const turnMetricsMap = useMemo(() => computeTurnMetrics(msgs, isRunning), [msgs, isRunning]);
  const groupedItems = useMemo(() => {
    const visible = msgs.filter((m) => m.role !== "tool").map((m) => ({ type: "message" as const, msg: m }));
    return groupTimelineItems(visible, turnMetricsMap, isRunning);
  }, [msgs, turnMetricsMap, isRunning]);

  // 实时聚合会话 Token（结合已完结轮次、历史持久化值与流式中的动态粗估）
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
        return {
          label: isSubprocess ? "子任务" : "协作者",
          icon: isSubprocess ? "⚡" : "🤝",
          color: isSubprocess ? "text-indigo-400 bg-indigo-500/10 border-indigo-500/20" : "text-zinc-400 bg-zinc-500/10 border-zinc-500/20",
        };
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
    const title = collab?.title ?? (isSubprocess ? "当前子任务" : "当前协作者");
    const confirmed = await askConfirm(
      `确定移除${isSubprocess ? "子进程" : "协作者"}「${title}」及其所有历史记录？`,
      "移除"
    );
    if (confirmed) {
      await deleteCollaborator(collaboratorId);
      setActiveCollaboratorId(null);
      setActiveSubprocessId(null);
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

  // 正在加载中时的过渡态
  if (!collab && loading) {
    return (
      <div className="flex-1 flex items-center justify-center p-6 text-inkdim text-center bg-panel border-l border-edge">
        <div className="flex flex-col items-center">
          <Loader2 size={30} className="mx-auto mb-2 animate-spin text-accent" />
          <div className="text-[13px] font-medium text-ink">正在拉取会话详情…</div>
          <div className="text-[11px] text-inkdim mt-1">请稍候，正在同步执行数据与最新快照</div>
        </div>
      </div>
    );
  }

  if (!collab) {
    return (
      <div className="flex-1 flex items-center justify-center p-6 text-inkdim text-center bg-panel border-l border-edge">
        <div>
          <Bot size={32} className="mx-auto mb-2 opacity-40" />
          <div className="text-[13px]">未找到该会话或已被移除</div>
          <button
            onClick={() => {
              setActiveCollaboratorId(null);
              setActiveSubprocessId(null);
            }}
            className="mt-3 px-3 py-1.5 rounded-lg bg-panel2 hover:bg-panel3 text-[12px] text-ink border border-edge"
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
    <div className="flex-1 flex flex-col min-h-0 w-full bg-panel border-l border-edge relative z-10">
      {/* Top Header */}
      <div
        ref={headerRef}
        className="relative z-30 h-12 border-b border-edge/60 bg-panel px-3 sm:px-3.5 flex items-center justify-between shrink-0 select-none gap-2"
      >
        {/* Left: Role info + Title + Status */}
        <div className="flex items-center gap-1.5 min-w-0 shrink overflow-hidden">
          {isSubprocess && !isCompact && (
            <div className="flex items-center gap-1 h-8 px-2 rounded-lg bg-indigo-500/15 text-indigo-400 border border-indigo-500/30 text-[11px] font-medium shrink-0">
              <Cpu size={12} />
              <span>子进程</span>
            </div>
          )}

          <span
            className={`h-8 ${isCompact ? "px-2" : "px-2.5"} rounded-lg text-[12px] font-medium border flex items-center gap-1.5 shrink-0 ${roleInfo.color}`}
            title={`角色: ${roleInfo.label}`}
          >
            <span className="text-[12px]">{roleInfo.icon}</span>
            {!isCompact && <span className="whitespace-nowrap">{roleInfo.label}</span>}
          </span>

          <span
            className="font-medium text-[12.5px] text-ink truncate min-w-0 max-w-[80px] sm:max-w-[130px]"
            title={collab.title}
          >
            {collab.title}
          </span>

          {isRunning ? (
            <div
              className="flex items-center gap-1.5 h-8 px-2 sm:px-2.5 rounded-lg text-[12px] shrink-0 select-none bg-emerald-500/10 text-emerald-400 font-medium border border-emerald-500/20"
              title="任务执行中"
            >
              <span className="w-1.5 h-1.5 rounded-full shrink-0 bg-emerald-400 animate-pulse shadow-[0_0_6px_rgba(52,211,153,0.8)]" />
              {!isCompact && <span>运行中</span>}
            </div>
          ) : collab.status === "failed" ? (
            <div
              className="flex items-center gap-1.5 h-8 px-2 sm:px-2.5 rounded-lg text-[12px] shrink-0 select-none bg-rose-500/10 text-rose-400 font-medium border border-rose-500/20"
              title="执行异常"
            >
              <span className="w-1.5 h-1.5 rounded-full shrink-0 bg-rose-400" />
              {!isCompact && <span>异常</span>}
            </div>
          ) : (
            <div
              className="flex items-center gap-1.5 h-8 px-2 rounded-lg text-[12px] shrink-0 select-none text-inkdim/60"
              title="当前空闲待命"
            >
              <span className="w-1.5 h-1.5 rounded-full shrink-0 bg-inkdim/40" />
              {!isCompact && <span>就绪</span>}
            </div>
          )}
        </div>

        {/* Right Header Action Buttons */}
        <div className="flex items-center gap-1.5 shrink-0 select-none ml-auto">
          {/* 子进程专属：只读流程提示标签（宽屏展示） */}
          {isSubprocess && !isCompact && (
            <span className="inline-flex items-center gap-1 h-8 px-2 rounded-lg text-[11px] text-inkdim bg-panel2/60 border border-edge/60 shrink-0">
              <Eye size={11} className="opacity-70" />
              <span>只读</span>
            </span>
          )}

          {/* 协作者专属：自动汇报开关（渐进式缩放：>= 520px 显示「自动汇报: 开/关」，390px~520px 缩放为「汇报: 开/关」，< 390px 收进更多操作） */}
          {!isSubprocess && isAutoReportVisible && (
            <button
              type="button"
              onClick={handleToggleAutoReport}
              className={`flex items-center gap-1.5 h-8 ${isWide ? "px-2.5" : "px-2"} rounded-lg text-[12px] font-medium border transition-colors cursor-pointer shrink-0 ${
                isAutoReport
                  ? "bg-accent/10 border-accent/30 text-accent hover:bg-accent/20"
                  : "bg-panel2/80 hover:bg-panel2 border-edge text-inkdim hover:text-ink"
              }`}
              title={isAutoReport ? "自动汇报：开启（任务完成后自动提交增量汇报）\n点击切换" : "自动汇报：关闭（需手动点击「汇报成果」）\n点击切换"}
            >
              <span className={`w-1.5 h-1.5 rounded-full ${isAutoReport ? "bg-accent" : "bg-zinc-500"}`} />
              <span>{isWide ? `自动汇报: ${isAutoReport ? "开" : "关"}` : `汇报: ${isAutoReport ? "开" : "关"}`}</span>
            </button>
          )}

          {/* Stop Button */}
          {isRunning && (
            <button
              type="button"
              onClick={() => void stopSubagent(collaboratorId)}
              className="flex items-center gap-1 h-8 px-2.5 rounded-lg text-amber-400 hover:bg-amber-500/10 border border-amber-500/20 text-[12px] font-medium transition-colors cursor-pointer shrink-0"
              title={isSubprocess ? "终止当前子进程运行" : "停止当前协作者"}
            >
              <Square size={11} fill="currentColor" />
              {!isVeryCompact && <span>停止</span>}
            </button>
          )}

          {/* 协作者专属：手动增量汇报按钮 */}
          {!isSubprocess && !isRunning && msgs.some((m) => m.role === "assistant") && (
            <button
              type="button"
              onClick={handleReport}
              disabled={reporting}
              className="flex items-center gap-1 h-8 px-2 sm:px-2.5 rounded-lg text-emerald-400 hover:bg-emerald-500/15 border border-emerald-500/20 text-[12px] font-medium transition-colors cursor-pointer shrink-0 disabled:opacity-50"
              title="手动将自上次汇报以来的增量产出与改动汇报给主会话"
            >
              {reporting ? <Loader2 size={12} className="animate-spin" /> : <Send size={12} />}
              {!isVeryCompact && <span>{reporting ? "汇报中…" : "汇报成果"}</span>}
            </button>
          )}

          {/* Restart Button */}
          {!isRunning && (
            <button
              type="button"
              onClick={() => void restartSubagent(collaboratorId)}
              className="flex items-center gap-1 h-8 px-2.5 rounded-lg text-inkdim hover:text-ink hover:bg-panel2 border border-edge text-[12px] font-medium transition-colors cursor-pointer shrink-0"
              title={isSubprocess ? "重新启动 / 继续该子进程" : "重启 / 继续当前协作者"}
            >
              <RefreshCw size={11} />
              {!isVeryCompact && <span>重启</span>}
            </button>
          )}

          {/* 宽屏直接展示：编辑与删除按钮 */}
          {!isCompact && (
            <>
              {!isSubprocess && (
                <button
                  type="button"
                  onClick={() => {
                    useStore.getState().setEditingCollaboratorId(collaboratorId);
                    useStore.getState().setShowEditCollaboratorModal(true);
                  }}
                  className="w-8 h-8 rounded-lg border border-edge/60 text-inkdim hover:text-ink hover:bg-panel2 flex items-center justify-center transition-colors cursor-pointer shrink-0"
                  title="编辑协作者配置（名称、角色、驱动模型、生图模型、调度规则等）"
                >
                  <Pencil size={12} />
                </button>
              )}

              <button
                type="button"
                onClick={handleDelete}
                className="w-8 h-8 rounded-lg hover:bg-rose-500/10 hover:text-rose-400 text-inkdim flex items-center justify-center transition-colors shrink-0 cursor-pointer"
                title={isSubprocess ? "移除子进程" : "移除协作者"}
              >
                <Trash2 size={13} />
              </button>
            </>
          )}

          {/* 窄屏/缩放时收起为：更多操作下拉菜单（包含自动汇报切换、编辑、移除） */}
          {isCompact && (
            <div className="relative shrink-0">
              <button
                type="button"
                onClick={() => setOpenMoreMenu((v) => !v)}
                className={`w-8 h-8 rounded-lg border border-edge/60 hover:bg-panel2 flex items-center justify-center transition-colors cursor-pointer shrink-0 ${
                  openMoreMenu ? "bg-panel2 border-accent/50 text-accent ring-1 ring-accent/20" : "text-inkdim hover:text-ink"
                }`}
                title="更多操作（自动汇报设置、配置编辑、移除）"
              >
                <MoreHorizontal size={14} />
              </button>

              {openMoreMenu && (
                <>
                  <div
                    className="fixed inset-0 z-40"
                    onMouseDown={(e) => {
                      e.stopPropagation();
                      setOpenMoreMenu(false);
                    }}
                    onClick={(e) => e.stopPropagation()}
                  />
                  <div
                    className="absolute right-0 top-10 z-50 w-[180px] bg-panel2 border border-edge/80 rounded-xl shadow-2xl p-1 animate-in fade-in zoom-in-95 duration-100 select-none"
                    onClick={(e) => e.stopPropagation()}
                  >
                    <div className="px-2 py-1 text-[10.5px] font-medium text-inkdim">更多操作</div>
                    {!isSubprocess && (
                      <>
                        <button
                          type="button"
                          onClick={() => {
                            void handleToggleAutoReport();
                            setOpenMoreMenu(false);
                          }}
                          className="w-full text-left px-2.5 py-1.5 rounded-lg text-[12px] hover:bg-panel3 transition-colors flex items-center justify-between text-ink/90 hover:text-ink cursor-pointer"
                          title="执行完毕后自动提交增量汇报给主任务"
                        >
                          <div className="flex items-center gap-2 min-w-0">
                            <span className={`w-1.5 h-1.5 rounded-full ${isAutoReport ? "bg-accent" : "bg-zinc-500"}`} />
                            <span>自动汇报</span>
                          </div>
                          <span className={`text-[11px] font-medium ${isAutoReport ? "text-accent" : "text-inkdim"}`}>
                            {isAutoReport ? "开" : "关"}
                          </span>
                        </button>

                        {!isRunning && msgs.some((m) => m.role === "assistant") && (
                          <button
                            type="button"
                            onClick={() => {
                              setOpenMoreMenu(false);
                              void handleReport();
                            }}
                            disabled={reporting}
                            className="w-full text-left px-2.5 py-1.5 rounded-lg text-[12px] hover:bg-panel3 transition-colors flex items-center gap-2 text-emerald-400 hover:text-emerald-300 cursor-pointer disabled:opacity-50"
                            title="手动将自上次汇报以来的增量产出与改动汇报给主会话"
                          >
                            {reporting ? <Loader2 size={12} className="animate-spin" /> : <Send size={12} />}
                            <span>{reporting ? "汇报中…" : "汇报成果"}</span>
                          </button>
                        )}

                        <button
                          type="button"
                          onClick={() => {
                            useStore.getState().setEditingCollaboratorId(collaboratorId);
                            useStore.getState().setShowEditCollaboratorModal(true);
                            setOpenMoreMenu(false);
                          }}
                          className="w-full text-left px-2.5 py-1.5 rounded-lg text-[12px] hover:bg-panel3 transition-colors flex items-center gap-2 text-ink/90 hover:text-ink cursor-pointer"
                          title="编辑角色、模型分配与调度规则"
                        >
                          <Pencil size={12} className="text-inkdim" />
                          <span>编辑配置</span>
                        </button>
                        <div className="my-1 border-t border-edge/50" />
                      </>
                    )}

                    <button
                      type="button"
                      onClick={() => {
                        void handleDelete();
                        setOpenMoreMenu(false);
                      }}
                      className="w-full text-left px-2.5 py-1.5 rounded-lg text-[12px] hover:bg-rose-500/10 text-rose-400 hover:text-rose-300 transition-colors flex items-center gap-2 cursor-pointer"
                      title={isSubprocess ? "移除该子进程及其执行数据" : "移除该协作者及其执行数据"}
                    >
                      <Trash2 size={12} />
                      <span>{isSubprocess ? "移除子进程" : "移除协作者"}</span>
                    </button>
                  </div>
                </>
              )}
            </div>
          )}

          {/* Divider */}
          <div className="h-4 w-px bg-edge/80 shrink-0 mx-0.5" />

          {/* Close Panel Button */}
          <button
            type="button"
            onClick={() => {
              setActiveCollaboratorId(null);
              setActiveSubprocessId(null);
            }}
            className="w-8 h-8 rounded-lg hover:bg-panel2 text-inkdim hover:text-ink flex items-center justify-center transition-colors shrink-0 cursor-pointer"
            title="关闭分屏面板"
          >
            <X size={14} />
          </button>
        </div>
      </div>

      {/* Task Prompt Overview Banner */}
      {(collab.subagentTask || collab.dispatchRule || collab.dispatch_rule || collab.modelId || collab.model_id) && (
        <div className="px-3.5 py-2 bg-panel2/30 border-b border-edge/60 text-[12px] flex flex-col gap-1 shrink-0 select-none">
          <div className="flex items-center gap-2">
            <Sparkles size={12} className="text-accent shrink-0" />
            <span className="text-ink font-medium shrink-0 text-[11.5px]">
              {isSubprocess ? "子任务目标：" : "协作职责："}
            </span>
            <span className="truncate text-inkdim flex-1 text-[11.5px]" title={collab.subagentTask || ""}>
              {collab.subagentTask || "未填写职责"}
            </span>
            {/* Model Badges */}
            <div className="flex items-center gap-1.5 shrink-0 text-[10.5px]">
              {(collab.modelId || collab.model_id) && (
                <span className="px-1.5 py-0.5 rounded bg-panel3 border border-edge text-inkdim font-mono flex items-center gap-1" title="对话思考驱动模型">
                  <Cpu size={10} className="text-accent" />
                  <span>{collab.modelId || collab.model_id}</span>
                </span>
              )}
              {(collab.imageModelId || collab.image_model_id) && (
                <span className="px-1.5 py-0.5 rounded bg-purple-500/10 border border-purple-500/30 text-purple-300 font-mono flex items-center gap-1" title="专属生图执行模型">
                  <span>🎨</span>
                  <span>{collab.imageModelId || collab.image_model_id}</span>
                </span>
              )}
            </div>
          </div>
          {(collab.dispatchRule || collab.dispatch_rule) && (
            <div className="flex items-center gap-1.5 text-[11px] text-inkdim/80 pl-5">
              <span className="text-accent font-medium shrink-0">触发规则:</span>
              <span className="truncate flex-1" title={collab.dispatchRule || collab.dispatch_rule || ""}>
                {collab.dispatchRule || collab.dispatch_rule}
              </span>
            </div>
          )}
        </div>
      )}

      {/* Messages Scroll Area */}
      <div className="flex-1 overflow-y-auto px-4 py-4 space-y-4" ref={boxRef} onScroll={onScroll}>
        {msgs.length === 0 ? (
          <div className="flex flex-col items-center justify-center py-16 text-center text-inkdim">
            <div className="w-10 h-10 rounded-xl bg-panel2 border border-edge flex items-center justify-center mb-3 text-accent">
              <Sparkles size={20} />
            </div>
            <div className="text-[13px] font-medium text-ink mb-1">
              {isSubprocess ? "子进程已就绪" : "协作者已就绪"}
            </div>
            <div className="text-[12px] max-w-[280px]">
              {collab.subagentTask
                ? `任务目标：${collab.subagentTask}`
                : isSubprocess
                ? "正在独立上下文中运行，任务结果与产出将自动汇聚至主进程。"
                : "已加入协作团队。主进程在安排相关任务时将优先委派，您也可以在下方直接向其派发指令。"}
            </div>
          </div>
        ) : (
          groupedItems.map((item) => {
            if (item.type === "compaction") return null;
            if (item.type === "process") {
              return (
                <ExecutionProcessBlock
                  key={item.id}
                  steps={item.steps}
                  isRunning={Boolean(item.isRunning ?? (isRunning && item.turnMetrics?.isCurrentRunningTurn))}
                  sessionWorkspace={collab.workspacePath}
                  streamingMsgId={isRunning ? msgs[msgs.length - 1]?.id : undefined}
                  turnMetrics={item.turnMetrics}
                  readOnly={isSubprocess}
                  turnMetricsMap={turnMetricsMap}
                />
              );
            }
            const m = item.msg;
            return (
              <MessageItem
                key={m.id}
                msg={m}
                isLastUser={m.id === lastUserMsgId}
                streaming={isRunning && m.role === "assistant" && m.id === msgs[msgs.length - 1]?.id}
                readOnly={isSubprocess}
                running={isRunning}
                turnMetrics={turnMetricsMap.get(m.id)}
              />
            );
          })
        )}
      </div>

      {/* Bottom Area: 协作者展示输入框，子进程不需要发消息展示只读状态条 */}
      {!isSubprocess ? (
        <div className="p-3">
          <div className="flex items-end gap-2 bg-panel2/90 border border-edge rounded-2xl px-3.5 py-2.5 shadow-sm focus-within:border-accent/60 focus-within:ring-1 focus-within:ring-accent/20 transition-all">
            <textarea
              ref={taRef}
              rows={1}
              value={input}
              onChange={(e) => setInput(e.target.value)}
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
            <div className="flex items-center gap-1.5 min-w-0 flex-1">
              <Folder size={11} className="shrink-0 opacity-70" />
              <span className="truncate max-w-[280px]" title={collab.workspacePath || "继承主项目工作区"}>
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
      ) : (
        /* 子进程专属底栏：无需输入框发消息，展示工作区与实时 Token 消耗 */
        <div className="p-3 border-t border-edge/40 bg-panel2/30">
          <div className="h-6 flex items-center justify-between text-[11px] text-inkdim select-none gap-3">
            <div className="flex items-center gap-2 min-w-0 flex-1">
              <span className="inline-flex items-center gap-1 text-inkdim">
                <Bot size={12} className="text-indigo-400 shrink-0" />
                <span>临时子任务由 Agent 自主运行，执行产出自动汇聚交割</span>
              </span>
              {collab.workspacePath && (
                <span className="truncate max-w-[200px] text-inkdim/70 font-mono hidden md:inline" title={collab.workspacePath}>
                  · {collab.workspacePath.split(/[\\/]/).filter(Boolean).pop()}
                </span>
              )}
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
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
