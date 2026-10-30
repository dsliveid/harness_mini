import { useState, useEffect, useCallback } from "react";
import { currentMessages, currentSession, useStore } from "../store";
import { ipc } from "../ipc";
import {
  type CurrentBoard,
  type ToolEvent,
  type LongTask,
  DRAFT_ID,
} from "../types";
import { Markdown } from "./Markdown";
import {
  CheckSquare,
  CheckCircle2,
  Loader2,
  Circle,
  Terminal,
  ChevronLeft,
  ChevronRight,
  Copy,
  Check,
  X,
  RotateCcw,
  ClipboardList,
  Target,
  Eye,
} from "./Icons";

/** 从当前会话消息中提取所有正在执行的 run_command 工具事件 */
function useRunningCommands(): ToolEvent[] {
  const msgs = useStore((s) => currentMessages(s));
  const result: ToolEvent[] = [];
  for (const m of msgs) {
    if (!m.toolEvents) continue;
    for (const ev of m.toolEvents) {
      if (ev.toolName === "run_command" && ev.status === "running") {
        result.push(ev);
      }
    }
  }
  return result;
}

function TodoSection({
  todos,
  isRunning,
  onToggle,
}: {
  todos: { index: number; content: string; status: string }[];
  isRunning: boolean;
  onToggle?: (index: number, currentStatus: string) => void;
}) {
  const done = todos.filter((t) => t.status === "done").length;
  const total = todos.length;
  const pct = total > 0 ? Math.round((done / total) * 100) : 0;
  return (
    <div className="flex flex-col gap-1.5">
      <div className="flex items-center gap-2 text-[11px] text-inkdim">
        <span className="flex items-center gap-1 font-medium">
          <CheckSquare size={13} className="text-purple-400" />
          <span>任务步骤清单</span>
        </span>
        <span className="font-mono">{done}/{total}</span>
        <div className="flex-1 h-1 bg-panel3 rounded-full overflow-hidden">
          <div
            className="h-full bg-green-500 rounded-full transition-all duration-300"
            style={{ width: `${pct}%` }}
          />
        </div>
      </div>
      <div className="flex flex-col gap-1 max-h-[220px] overflow-y-auto pr-1">
        {todos.map((t) => (
          <div
            key={t.index}
            className={`group flex items-start gap-1.5 text-[12px] leading-snug p-0.5 rounded transition-colors ${
              onToggle ? "hover:bg-panel3/60 cursor-pointer select-none" : ""
            }`}
            onClick={() => onToggle?.(t.index, t.status)}
            title={onToggle ? "点击切换完成状态" : undefined}
          >
            <span className="shrink-0 mt-0.5 group-hover:scale-110 transition-transform">
              {t.status === "done" ? (
                <CheckCircle2 size={13} className="text-green-400" />
              ) : t.status === "in_progress" && isRunning ? (
                <Loader2 size={13} className="text-blue-400 animate-spin" />
              ) : (
                <Circle size={13} className="text-inkdim/50 group-hover:text-inkdim" />
              )}
            </span>
            <span className={t.status === "done" ? "text-inkdim line-through" : "text-ink"}>
              {t.content}
            </span>
          </div>
        ))}
      </div>
    </div>
  );
}

function LongTaskSection({
  task,
  activeSideId,
  onOpenSubagent,
  onOpenDetailModal,
}: {
  task: LongTask;
  activeSideId: string | null;
  onOpenSubagent: (subId: string) => void;
  onOpenDetailModal: () => void;
}) {
  const subtasks = task.subtasks || [];
  const done = subtasks.filter((s) => s.status === "completed").length;
  const total = subtasks.length;
  const pct = total > 0 ? Math.round((done / total) * 100) : 0;

  return (
    <div className="flex flex-col gap-2">
      {/* 标题与状态 */}
      <div className="rounded-xl border p-2.5 text-[11.5px] space-y-1.5 bg-purple-500/10 border-purple-500/20">
        <div className="flex items-center justify-between gap-1.5">
          <span className="font-semibold text-purple-400 truncate flex-1 min-w-0 inline-flex items-center gap-1.5" title={task.goal}>
            <Target size={13} className="shrink-0" />
            <span className="truncate">{task.goal}</span>
          </span>
          <span
            className={`text-[10px] px-1.5 py-0.5 rounded font-medium shrink-0 ${
              task.status === "completed"
                ? "bg-green-500/20 text-green-300"
                : task.status === "paused" || task.status === "interrupted"
                ? "bg-amber-500/20 text-amber-300"
                : task.status === "failed"
                ? "bg-red-500/20 text-red-300"
                : "bg-blue-500/20 text-blue-300"
            }`}
          >
            {task.status === "completed"
              ? "已达成"
              : task.status === "paused"
              ? "已暂停"
              : task.status === "interrupted"
              ? "已中断"
              : task.status === "failed"
              ? "遇阻"
              : "推进中"}
          </span>
        </div>

        {/* 阶段进度条 */}
        <div className="flex items-center gap-2 text-[11px] text-inkdim pt-0.5">
          <span className="font-mono text-[10px]">{done}/{total} 阶段</span>
          <div className="flex-1 h-1.5 bg-panel3 rounded-full overflow-hidden">
            <div
              className="h-full bg-gradient-to-r from-purple-500 to-indigo-400 rounded-full transition-all duration-300"
              style={{ width: `${pct}%` }}
            />
          </div>
          <span className="font-mono text-[10px]">{pct}%</span>
        </div>
      </div>

      {/* 阶段步骤列表（可点击跳转查看透明子进程） */}
      <div className="flex flex-col gap-1 max-h-[220px] overflow-y-auto pr-1">
        {subtasks.map((sub) => {
          const isDone = sub.status === "completed";
          const isRunning = sub.status === "in_progress" || sub.status === "verifying";
          const isFailed = sub.status === "failed";
          const isCurrentActiveSide = !!sub.id && activeSideId === sub.id;
          const hasRealSession = !!sub.id && !sub.id.startsWith("sub-replan") && !sub.id.startsWith("sub-");

          return (
            <div
              key={sub.id || sub.index}
              onClick={() => {
                if (hasRealSession) {
                  onOpenSubagent(sub.id);
                }
              }}
              className={`group flex items-start gap-1.5 text-[12px] leading-snug p-1.5 rounded-lg border transition-all ${
                hasRealSession ? "cursor-pointer" : ""
              } ${
                isCurrentActiveSide
                  ? "bg-accent/15 border-accent/60 text-ink shadow-xs ring-1 ring-accent/30"
                  : "bg-panel2/60 hover:bg-panel2/90 border-edge/60 text-ink"
              }`}
              title={hasRealSession ? "点击在分屏中透明查看本阶段沙箱执行过程" : undefined}
            >
              <span className="shrink-0 mt-0.5">
                {isDone ? (
                  <CheckCircle2 size={13} className="text-green-400" />
                ) : isRunning ? (
                  <Loader2 size={13} className="text-blue-400 animate-spin" />
                ) : isFailed ? (
                  <X size={13} className="text-red-400" />
                ) : (
                  <Circle size={13} className="text-inkdim/50 group-hover:text-inkdim" />
                )}
              </span>

              <div className="flex-1 min-w-0 flex flex-col gap-0.5">
                <div className="flex items-center justify-between gap-1">
                  <span className={`font-medium truncate ${isDone ? "text-inkdim line-through" : "text-ink"}`}>
                    {sub.index}. {sub.title}
                  </span>
                  {sub.verifyCommand && (
                    <span className="text-[9px] px-1 py-0.2 rounded bg-panel3 text-inkdim font-mono shrink-0" title={`验证: ${sub.verifyCommand}`}>
                      门禁
                    </span>
                  )}
                </div>
                {sub.description && (
                  <p className="text-[11px] text-inkdim line-clamp-1">
                    {sub.description}
                  </p>
                )}
              </div>

              {hasRealSession && (
                <button
                  type="button"
                  onClick={(e) => {
                    e.stopPropagation();
                    onOpenSubagent(sub.id);
                  }}
                  className={`shrink-0 p-1 rounded-md transition-colors cursor-pointer ${
                    isCurrentActiveSide ? "text-accent bg-accent/20" : "text-inkdim hover:text-ink hover:bg-panel3"
                  }`}
                  title="在分屏查看子进程"
                >
                  <Eye size={12} />
                </button>
              )}
            </div>
          );
        })}
      </div>

      {/* 底部详细看板入口 */}
      <div className="flex items-center justify-between text-[11px] text-inkdim pt-1 border-t border-edge/60">
        <span className="text-[10px]">透明沙箱执行</span>
        <button
          type="button"
          onClick={onOpenDetailModal}
          className="text-purple-400 hover:text-purple-300 font-medium hover:underline cursor-pointer flex items-center gap-1"
        >
          <span>查看完整路线图与检查点</span>
          <ChevronRight size={11} />
        </button>
      </div>
    </div>
  );
}

function RunningCommandCard({ ev }: { ev: ToolEvent }) {
  const output = useStore((s) => s.toolOutputs[ev.id]);
  const killCommand = useStore((s) => s.killCommand);
  const pushToast = useStore((s) => s.pushToast);
  const [copied, setCopied] = useState(false);
  const command = ev.params?.command ?? "";
  const lastLine = output?.trim().split("\n").pop() ?? "";
  const [terminating, setTerminating] = useState(false);

  const handleKill = async () => {
    if (terminating) return;
    setTerminating(true);
    try {
      await killCommand(ev.id);
    } catch {
      setTerminating(false);
    }
  };

  const handleCopy = () => {
    if (!command) return;
    navigator.clipboard.writeText(command).then(
      () => {
        setCopied(true);
        setTimeout(() => setCopied(false), 1500);
        pushToast("命令行已复制到剪贴板");
      },
      () => pushToast("复制失败")
    );
  };

  return (
    <div className="flex flex-col gap-1">
      <div className="flex items-center gap-1.5">
        <span
          className={`w-1.5 h-1.5 rounded-full shrink-0 ${
            terminating ? "bg-amber-400 animate-ping" : "bg-green-400 animate-pulse"
          }`}
        />
        <span className="text-[11px] text-inkdim truncate flex-1 min-w-0 font-mono" title={command}>
          {command.length > 40 ? command.slice(0, 40) + "…" : command}
        </span>
        <button
          className="shrink-0 p-0.5 rounded hover:bg-panel3 text-inkdim hover:text-ink transition-colors cursor-pointer"
          title="复制命令行"
          onClick={handleCopy}
        >
          {copied ? <Check size={11} className="text-green-400" /> : <Copy size={11} />}
        </button>
        <button
          className="shrink-0 text-[10px] px-1.5 py-0.5 rounded bg-red-600/80 hover:bg-red-500 text-white disabled:opacity-50 disabled:cursor-not-allowed transition-all cursor-pointer"
          title={terminating ? "正在终止进程…" : "终止进程"}
          disabled={terminating}
          onClick={handleKill}
        >
          {terminating ? "终止中…" : "终止"}
        </button>
      </div>
      {lastLine && (
        <pre className="text-[10px] font-mono text-inkdim truncate max-w-full" title={lastLine}>
          {lastLine.length > 60 ? lastLine.slice(-60) : lastLine}
        </pre>
      )}
    </div>
  );
}

export function FloatingTaskPanel() {
  const currentId = useStore((s) => s.currentId);
  const currentWorkspace = useStore(
    (s) =>
      s.sessions.find((x) => x.id === s.currentId)?.workspacePath ||
      s.draft?.workspacePath ||
      s.projects.find(
        (p) =>
          p.id ===
          (s.sessions.find((sess) => sess.id === s.currentId)?.projectId || s.draft?.projectId)
      )?.path ||
      ""
  );
  const isRunning = useStore((s) => (s.currentId ? s.runStatus[s.currentId] === "running" : false));
  const currentBoard = useStore((s) => (s.currentId ? s.currentBoards[s.currentId] ?? null : null));
  const loadCurrentBoard = useStore((s) => s.loadCurrentBoard);
  const runningCmds = useRunningCommands();
  const pushToast = useStore((s) => s.pushToast);
  const focusFloatingTaskId = useStore((s) => s.focusFloatingTaskId);
  const setFocusFloatingTaskId = useStore((s) => s.setFocusFloatingTaskId);
  const activeTask = useStore((s) => (s.currentId ? s.activeTasks[s.currentId] ?? null : null));
  const setActiveCollaboratorId = useStore((s) => s.setActiveCollaboratorId);
  const activeCollaboratorId = useStore((s) => s.activeCollaboratorId);
  const activeSubprocessId = useStore((s) => s.activeSubprocessId);
  const activeSideId = activeCollaboratorId || activeSubprocessId;
  const setShowTaskDetailModal = useStore((s) => s.setShowTaskDetailModal);

  const [collapsed, setCollapsed] = useState(false);
  const [showPlanModal, setShowPlanModal] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const [highlightedTaskId, setHighlightedTaskId] = useState<string | null>(null);

  // 会话初次挂载时按需加载当前权威看板
  useEffect(() => {
    if (currentId && currentId !== DRAFT_ID) {
      void loadCurrentBoard(currentId);
    }
  }, [currentId, loadCurrentBoard]);

  // 响应外部（如 ToolCard 点击「浮窗查看」）的聚焦指令
  useEffect(() => {
    if (!focusFloatingTaskId) return;
    if (currentBoard) {
      setCollapsed(false);
      setHighlightedTaskId(currentBoard.id);
      const timer = setTimeout(() => {
        setHighlightedTaskId(null);
      }, 2000);
      setFocusFloatingTaskId(null);
      return () => clearTimeout(timer);
    }
    setFocusFloatingTaskId(null);
  }, [focusFloatingTaskId, currentBoard, setFocusFloatingTaskId]);

  // 步骤勾选切换处理（仅方案类型可写回；todo 快照来自消息流为只读展示）
  const handleToggleStep = async (stepIndex: number, currentStatus: string) => {
    if (!currentBoard || currentBoard.board_type !== "plan" || !currentWorkspace || !currentId) return;
    const nextStatus = currentStatus === "done" ? "pending" : "done";

    // 前端乐观更新瞬时响应
    const oldItems = currentBoard.items;
    const updatedItems = oldItems.map((s) =>
      s.index === stepIndex ? { ...s, status: nextStatus } : s
    );
    const completed = updatedItems.filter((s) => s.status === "done").length;
    const pct = updatedItems.length > 0 ? Math.round((completed * 100) / updatedItems.length) : 0;
    useStore.setState((st) => ({
      currentBoards: {
        ...st.currentBoards,
        [currentId]: {
          ...currentBoard,
          items: updatedItems,
          completed_steps: completed,
          progress_pct: pct,
        },
      },
    }));

    try {
      await ipc.updatePlanStepStatus(currentWorkspace, currentBoard.id, stepIndex, nextStatus, currentId);
    } catch (err) {
      console.error("更新计划步骤状态失败:", err);
      // 回滚
      useStore.setState((st) => ({
        currentBoards: {
          ...st.currentBoards,
          [currentId]: {
            ...currentBoard,
            items: oldItems,
          },
        },
      }));
    }
  };

  // 定位到主对话区中的消息位置
  const handleLocateMessage = (msgId?: string, toolEventId?: string) => {
    if (!msgId && !toolEventId) {
      pushToast("未关联具体的对话消息位置");
      return;
    }
    let el: Element | null = null;
    if (toolEventId) {
      el =
        document.querySelector(`[data-tool-event-id="${toolEventId}"]`) ||
        document.getElementById(`tool-${toolEventId}`);
    }
    if (!el && msgId) {
      el =
        document.querySelector(`[data-message-id="${msgId}"]`) ||
        document.getElementById(`msg-${msgId}`);
    }
    if (el) {
      el.scrollIntoView({ behavior: "smooth", block: "center" });
      el.classList.add("ring-2", "ring-accent", "ring-offset-2", "transition-all", "duration-500");
      setTimeout(() => {
        el?.classList.remove("ring-2", "ring-accent", "ring-offset-2");
      }, 2000);
    } else {
      pushToast("未在主对话流中找到对应消息");
    }
  };

  const hasTasks = !!currentBoard && currentBoard.items.length > 0;
  const hasCmds = runningCmds.length > 0;
  const hasLongTask = !!activeTask && !!activeTask.subtasks && activeTask.subtasks.length > 0 && activeTask.status !== "cancelled";

  if (!currentId || currentId === DRAFT_ID || (!hasTasks && !hasCmds && !hasLongTask)) return null;

  // 收起态：靠右吸附抽屉标签
  if (collapsed) {
    const doneCnt = currentBoard ? currentBoard.completed_steps : 0;
    const totalCnt = currentBoard ? currentBoard.total_steps : 0;
    const longDoneCnt = activeTask ? (activeTask.subtasks || []).filter((s) => s.status === "completed").length : 0;
    const longTotalCnt = activeTask ? (activeTask.subtasks || []).length : 0;
    return (
      <div className="absolute top-3 right-0 z-20 select-none">
        <button
          className="group flex items-center gap-2 pl-2.5 pr-2 py-1.5 rounded-l-xl bg-panel2/90 hover:bg-panel2 backdrop-blur-md border-y border-l border-edge hover:border-accent/50 shadow-lg text-[11px] text-inkdim hover:text-ink transition-all cursor-pointer"
          onClick={() => setCollapsed(false)}
          title="展开任务看板"
        >
          <ChevronLeft
            size={13}
            className="text-inkdim group-hover:text-accent transition-transform duration-150 group-hover:-translate-x-0.5 shrink-0"
          />
          {hasLongTask && (
            <span className="text-[10px] px-1.5 py-0.5 rounded font-medium bg-purple-500/15 text-purple-400 inline-flex items-center gap-1">
              <Target size={11} className="shrink-0" />
              <span>长任务</span>
            </span>
          )}
          {currentBoard?.board_type === "plan" && (
            <span
              className={`text-[10px] px-1.5 py-0.5 rounded font-medium inline-flex items-center gap-1 ${
                currentBoard.status === "completed"
                  ? "bg-green-500/15 text-green-400"
                  : currentBoard.status === "suspended"
                  ? "bg-amber-500/15 text-amber-400"
                  : "bg-emerald-500/15 text-emerald-400"
              }`}
            >
              <ClipboardList size={11} className="shrink-0" />
              <span>
                {currentBoard.version ? `v${currentBoard.version} ` : ""}
                {currentBoard.status === "completed" ? "已结案" : currentBoard.status === "suspended" ? "已挂起" : "方案"}
              </span>
            </span>
          )}
          {currentBoard?.board_type === "todo" && (
            <span className="text-[10px] px-1.5 py-0.5 rounded font-medium bg-purple-500/15 text-purple-400 inline-flex items-center gap-1">
              <CheckSquare size={11} className="shrink-0" />
              <span>清单</span>
            </span>
          )}
          {hasCmds && <span className="w-2 h-2 rounded-full bg-green-400 animate-pulse shrink-0" />}
          {totalCnt > 0 && (
            <span className="flex items-center gap-1 font-medium">
              <CheckSquare
                size={13}
                className={currentBoard?.board_type === "plan" ? "text-emerald-400" : "text-purple-400"}
              />
              <span className="font-mono text-[11px]">{doneCnt}/{totalCnt}</span>
            </span>
          )}
          {longTotalCnt > 0 && !currentBoard && (
            <span className="flex items-center gap-1 font-medium text-purple-400">
              <Target size={12} />
              <span className="font-mono text-[11px]">{longDoneCnt}/{longTotalCnt}</span>
            </span>
          )}
          {hasCmds && !currentBoard && !hasLongTask && (
            <span className="flex items-center gap-1 text-green-400 font-mono">
              <Terminal size={13} className="shrink-0" />
              <span>{runningCmds.length} 进程</span>
            </span>
          )}
        </button>
      </div>
    );
  }

  // 展开态：宽度 325px，单一权威看板呈现
  return (
    <>
      <div className="absolute top-3 right-0 z-20 w-[325px] max-w-[calc(100%-1rem)] rounded-l-2xl bg-panel2/95 backdrop-blur-md border-y border-l border-edge shadow-2xl overflow-hidden select-none animate-in fade-in slide-in-from-right-2 duration-150">
        {/* 第一层：面板总控栏（标题、定位出处、刷新、收起） */}
        <div className="flex items-center justify-between px-3.5 py-2.5 border-b border-edge bg-panel/60">
          <div className="flex items-center gap-1.5 min-w-0">
            {hasLongTask && !currentBoard ? (
              <Target size={14} className="text-purple-400 shrink-0" />
            ) : (
              <CheckSquare size={14} className="text-emerald-400 shrink-0" />
            )}
            <span className="text-[12px] font-semibold text-ink tracking-tight truncate">
              {hasLongTask && !currentBoard ? "长任务阶段看板" : "任务与计划看板"}
            </span>
            {hasLongTask && (
              <span className="text-[10px] px-1.5 py-0.2 rounded font-mono font-medium shrink-0 bg-purple-500/15 text-purple-400 border border-purple-500/20">
                长任务
              </span>
            )}
            {currentBoard && (
              <span
                className={`text-[10px] px-1.5 py-0.2 rounded font-mono font-medium shrink-0 ${
                  currentBoard.board_type === "plan"
                    ? "bg-emerald-500/15 text-emerald-400 border border-emerald-500/20"
                    : "bg-purple-500/15 text-purple-400 border border-purple-500/20"
                }`}
              >
                {currentBoard.board_type === "plan" ? "方案" : "清单"}
              </span>
            )}
          </div>
          <div className="flex items-center gap-1 shrink-0">
            {currentBoard && (currentBoard.message_id || currentBoard.tool_event_id) && (
              <button
                type="button"
                className="flex items-center gap-1 text-[11px] text-inkdim hover:text-ink px-1.5 py-0.5 rounded-md hover:bg-panel3 transition-colors cursor-pointer"
                onClick={() => handleLocateMessage(currentBoard.message_id, currentBoard.tool_event_id)}
                title="定位到主对话中生成此任务的消息位置"
              >
                <span>定位出处</span>
              </button>
            )}
            <button
              type="button"
              className="p-1 text-inkdim hover:text-ink rounded-md hover:bg-panel3 transition-colors cursor-pointer"
              onClick={async () => {
                if (currentId && currentId !== DRAFT_ID) {
                  setRefreshing(true);
                  try {
                    await loadCurrentBoard(currentId);
                  } finally {
                    setRefreshing(false);
                  }
                }
              }}
              title="重新从后端同步最新权威看板状态"
            >
              <RotateCcw size={12} className={refreshing ? "animate-spin text-emerald-400" : ""} />
            </button>
            <button
              type="button"
              className="flex items-center gap-0.5 text-[11px] text-inkdim hover:text-ink pl-1.5 pr-1 py-0.5 rounded-md hover:bg-panel3 transition-colors cursor-pointer"
              onClick={() => setCollapsed(true)}
              title="收起至右侧"
            >
              <span>收起</span>
              <ChevronRight size={12} />
            </button>
          </div>
        </div>

        {/* 内容展示区 */}
        <div className="px-3.5 py-2.5 flex flex-col gap-2.5 max-h-[min(70vh,480px)] overflow-y-auto">
          {/* 长任务卡片与阶段分步 */}
          {hasLongTask && activeTask && (
            <LongTaskSection
              task={activeTask}
              activeSideId={activeSideId}
              onOpenSubagent={(subId) => setActiveCollaboratorId(activeCollaboratorId === subId ? null : subId)}
              onOpenDetailModal={() => setShowTaskDetailModal(true)}
            />
          )}

          {/* 分隔线 */}
          {hasLongTask && (currentBoard || hasCmds) && <div className="border-t border-edge" />}

          {/* 方案标题与状态卡片 */}
          {currentBoard && currentBoard.board_type === "plan" && (
            <div
              className={`rounded-xl border p-2.5 text-[11.5px] space-y-1.5 transition-all duration-300 ${
                highlightedTaskId === currentBoard.id
                  ? "ring-2 ring-purple-400 ring-offset-1 ring-offset-panel shadow-lg shadow-purple-500/20"
                  : ""
              } ${
                currentBoard.status === "completed"
                  ? "bg-green-500/10 border-green-500/30"
                  : currentBoard.status === "suspended"
                  ? "bg-amber-500/10 border-amber-500/30"
                  : "bg-emerald-500/10 border-emerald-500/20"
              }`}
            >
              <div className="flex items-center justify-between gap-1.5">
                <span
                  className={`font-semibold truncate flex-1 min-w-0 inline-flex items-center gap-1.5 ${
                    currentBoard.status === "completed"
                      ? "text-green-400"
                      : currentBoard.status === "suspended"
                      ? "text-amber-400"
                      : "text-emerald-400"
                  }`}
                  title={currentBoard.title}
                >
                  <ClipboardList size={13} className="shrink-0 text-emerald-400" />
                  <span className="truncate">{currentBoard.title}</span>
                </span>
                <div className="flex items-center gap-1 shrink-0">
                  {currentBoard.version && (
                    <span className="font-mono text-[10px] text-emerald-300 bg-emerald-500/20 px-1.5 py-0.5 rounded">
                      v{currentBoard.version}
                    </span>
                  )}
                  <span
                    className={`text-[10px] px-1.5 py-0.5 rounded font-medium ${
                      currentBoard.status === "completed"
                        ? "bg-green-500/20 text-green-300"
                        : currentBoard.status === "suspended"
                        ? "bg-amber-500/20 text-amber-300"
                        : "bg-blue-500/20 text-blue-300"
                    }`}
                  >
                    {currentBoard.status === "completed"
                      ? "已结案"
                      : currentBoard.status === "suspended"
                      ? "已挂起"
                      : "执行中"}
                  </span>
                </div>
              </div>

              <div className="text-[11px] text-inkdim flex items-center justify-between gap-1">
                <span className="truncate font-mono text-[10px]" title={currentBoard.filename}>
                  {currentBoard.filename || "方案文档.md"}
                </span>
                <div className="flex items-center gap-2 shrink-0">
                  {currentBoard.filename && (
                    <button
                      type="button"
                      className="text-emerald-400 hover:text-emerald-300 font-medium hover:underline cursor-pointer"
                      onClick={() => {
                        const cleanRel = `.harness/plans/${currentBoard.filename}`;
                        const absPath = currentWorkspace
                          ? `${currentWorkspace.replace(/\\/g, "/").replace(/\/$/, "")}/${cleanRel}`
                          : cleanRel;
                        ipc.openFileViewer({
                          id: `file:${absPath}`,
                          type: "file",
                          title: currentBoard.filename || "方案文档.md",
                          path: absPath,
                          workspacePath: currentWorkspace,
                          sessionId: currentId || undefined,
                        });
                      }}
                      title="在独立编辑器中查看/修改方案 Markdown 原文"
                    >
                      方案文档 (MD)
                    </button>
                  )}
                  {currentBoard.body && (
                    <button
                      type="button"
                      className="text-inkdim hover:text-ink hover:underline cursor-pointer"
                      onClick={() => setShowPlanModal(true)}
                    >
                      预览全文
                    </button>
                  )}
                </div>
              </div>
            </div>
          )}

          {/* 最新轻量 Todo 任务标题卡片 */}
          {currentBoard && currentBoard.board_type === "todo" && (
            <div
              className={`rounded-xl border p-2.5 text-[11.5px] space-y-1 transition-all duration-300 ${
                highlightedTaskId === currentBoard.id
                  ? "ring-2 ring-purple-400 ring-offset-1 ring-offset-panel shadow-lg shadow-purple-500/20 bg-purple-500/20 border-purple-500/40"
                  : "bg-purple-500/10 border-purple-500/20"
              }`}
            >
              <div className="flex items-center justify-between gap-1.5">
                <span className="font-semibold text-purple-400 truncate flex-1 min-w-0 inline-flex items-center gap-1.5">
                  <CheckSquare size={13} className="shrink-0" />
                  <span className="truncate">{currentBoard.title}</span>
                </span>
                <span
                  className={`text-[10px] px-1.5 py-0.5 rounded font-medium ${
                    currentBoard.status === "completed"
                      ? "bg-green-500/20 text-green-300"
                      : "bg-blue-500/20 text-blue-300"
                  }`}
                >
                  {currentBoard.status === "completed" ? "已完成" : "执行中"}
                </span>
              </div>
            </div>
          )}

          {/* 运行中的控制台进程 */}
          {hasCmds && (
            <div className="flex flex-col gap-1.5">
              <div className="text-[10px] text-inkdim font-medium flex items-center gap-1">
                <Terminal size={11} className="text-emerald-400" />
                <span>控制台进程</span>
              </div>
              {runningCmds.map((ev) => (
                <RunningCommandCard key={ev.id} ev={ev} />
              ))}
            </div>
          )}

          {/* 分隔线 */}
          {hasCmds && currentBoard && currentBoard.items.length > 0 && <div className="border-t border-edge" />}

          {/* 任务分步 Checklist */}
          {currentBoard && currentBoard.items.length > 0 && (
            <div
              className={`rounded-xl transition-all duration-300 ${
                highlightedTaskId === currentBoard.id
                  ? "ring-2 ring-purple-400/70 p-1 bg-purple-500/5 rounded-xl"
                  : ""
              }`}
            >
              <TodoSection
                todos={currentBoard.items}
                isRunning={isRunning && currentBoard.status === "in_progress"}
                onToggle={currentBoard.board_type === "plan" ? handleToggleStep : undefined}
              />
            </div>
          )}
        </div>
      </div>

      {/* 完整方案预览弹窗 */}
      {showPlanModal && currentBoard && currentBoard.body && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm p-4 animate-in fade-in duration-150">
          <div className="bg-panel rounded-2xl border border-edge shadow-2xl max-w-2xl w-full max-h-[85vh] flex flex-col overflow-hidden">
            <div className="flex items-center justify-between px-5 py-3.5 border-b border-edge bg-panel2/60">
              <div className="flex items-center gap-2 min-w-0">
                <span className="text-sm font-semibold text-ink truncate">
                  {currentBoard.title}
                </span>
                {currentBoard.version && (
                  <span className="text-[11px] font-mono px-2 py-0.5 rounded bg-emerald-500/10 text-emerald-400 border border-emerald-500/20">
                    v{currentBoard.version}
                  </span>
                )}
                <span
                  className={`text-[10px] px-1.5 py-0.5 rounded font-medium ${
                    currentBoard.status === "completed"
                      ? "bg-green-500/20 text-green-300"
                      : currentBoard.status === "suspended"
                      ? "bg-amber-500/20 text-amber-300"
                      : "bg-blue-500/20 text-blue-300"
                  }`}
                >
                  {currentBoard.status === "completed"
                    ? "已结案"
                    : currentBoard.status === "suspended"
                    ? "已挂起"
                    : "执行中"}
                </span>
              </div>
              <button
                type="button"
                className="p-1 rounded-lg hover:bg-panel3 text-inkdim hover:text-ink transition-colors cursor-pointer"
                onClick={() => setShowPlanModal(false)}
              >
                <X size={16} />
              </button>
            </div>
            <div className="p-5 overflow-y-auto flex-1 select-text space-y-3">
              <Markdown content={currentBoard.body} />
            </div>
            <div className="px-5 py-3 border-t border-edge bg-panel2/40 flex justify-between items-center text-[11px] text-inkdim">
              <span className="font-mono">
                {currentBoard.filename ? `.harness/plans/${currentBoard.filename}` : "方案文档"}
              </span>
              <button
                type="button"
                className="px-3 py-1.5 rounded-lg bg-panel3 hover:bg-edge text-ink text-xs transition-colors cursor-pointer"
                onClick={() => setShowPlanModal(false)}
              >
                关闭
              </button>
            </div>
          </div>
        </div>
      )}
    </>
  );
}
