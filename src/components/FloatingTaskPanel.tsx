import { useState, useEffect, useCallback } from "react";
import { currentMessages, useStore } from "../store";
import { ipc } from "../ipc";
import type { ActivePlanDetail, TodoItem, ToolEvent } from "../types";
import { Markdown } from "./Markdown";
import {
  CheckSquare,
  CheckCircle2,
  Loader2,
  Circle,
  Terminal,
  ChevronDown,
  ChevronUp,
  ChevronLeft,
  ChevronRight,
  Copy,
  Check,
  X,
  RotateCcw,
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
          <span>任务清单</span>
        </span>
        <span className="font-mono">{done}/{total}</span>
        <div className="flex-1 h-1 bg-panel3 rounded-full overflow-hidden">
          <div className="h-full bg-green-500 rounded-full transition-all duration-300" style={{ width: `${pct}%` }} />
        </div>
      </div>
      <div className="flex flex-col gap-1 max-h-[180px] overflow-y-auto pr-1">
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
          className="shrink-0 p-0.5 rounded hover:bg-panel3 text-inkdim hover:text-ink transition-colors"
          title="复制命令行"
          onClick={handleCopy}
        >
          {copied ? <Check size={11} className="text-green-400" /> : <Copy size={11} />}
        </button>
        <button
          className="shrink-0 text-[10px] px-1.5 py-0.5 rounded bg-red-600/80 hover:bg-red-500 text-white disabled:opacity-50 disabled:cursor-not-allowed transition-all"
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
  const currentWorkspace = useStore((s) => s.sessions.find((x) => x.id === s.currentId)?.workspacePath || "");
  const isRunning = useStore((s) => (s.currentId ? s.runStatus[s.currentId] === "running" : false));
  const rawTodos: TodoItem[] = useStore((s) => (s.currentId ? s.sessionTodos[s.currentId] ?? [] : []));
  const runningCmds = useRunningCommands();
  const [collapsed, setCollapsed] = useState(false);
  const [activePlan, setActivePlan] = useState<ActivePlanDetail | null>(null);
  const [showPlanModal, setShowPlanModal] = useState(false);
  const [refreshing, setRefreshing] = useState(false);

  // 获取当前会话关联的活动计划
  const fetchActivePlan = useCallback(() => {
    if (!currentId) return;
    setRefreshing(true);
    ipc.getActivePlan(currentId).then(
      (res) => {
        setActivePlan(res);
        setRefreshing(false);
      },
      () => {
        setActivePlan(null);
        setRefreshing(false);
      }
    );
  }, [currentId]);

  // 会话处于空闲/结束态时，自动将历史遗留未收尾的 in_progress 任务视为已完成
  const todos = isRunning
    ? rawTodos
    : rawTodos.map((t) => (t.status === "in_progress" ? { ...t, status: "done" } : t));

  useEffect(() => {
    // 切换会话时立即重置活动计划，消除上一会话残留
    setActivePlan(null);
    fetchActivePlan();
  }, [currentId, isRunning, todos.length, fetchActivePlan]);

  // 统一任务清单：优先使用物理方案文件中的步骤 (File-Centric)，无方案时回退到普通 todo
  const hasPlanSteps = !!(activePlan?.steps && activePlan.steps.length > 0);
  const effectiveTodos = hasPlanSteps
    ? activePlan!.steps.map((s) => ({
        index: s.index,
        content: `步骤 ${s.index}：${s.content}`,
        status: s.status,
      }))
    : todos.map((t, i) => ({
        index: i + 1,
        content: t.content,
        status: t.status,
      }));

  const handleToggleStep = async (stepIndex: number, currentStatus: string) => {
    const nextStatus = currentStatus === "done" ? "pending" : "done";
    if (hasPlanSteps && activePlan && currentWorkspace) {
      // 乐观更新 activePlan 本地状态
      const oldSteps = activePlan.steps;
      const updatedSteps = oldSteps.map((s) =>
        s.index === stepIndex ? { ...s, status: nextStatus as "pending" | "done" } : s
      );
      setActivePlan({ ...activePlan, steps: updatedSteps });
      try {
        await ipc.updatePlanStepStatus(
          currentWorkspace,
          activePlan.meta.id,
          stepIndex,
          nextStatus,
          currentId || undefined
        );
      } catch (err) {
        console.error("更新计划步骤状态失败:", err);
        setActivePlan({ ...activePlan, steps: oldSteps });
      }
    } else if (currentId) {
      const nextRaw = rawTodos.map((t, i) =>
        i + 1 === stepIndex ? { ...t, status: nextStatus as "pending" | "done" } : t
      );
      useStore.setState((st) => ({
        sessionTodos: { ...st.sessionTodos, [currentId]: nextRaw },
      }));
    }
  };

  const hasTodos = effectiveTodos.length > 0;
  const hasCmds = runningCmds.length > 0;
  const hasPlan = activePlan !== null;

  if (!currentId || (!hasTodos && !hasCmds && !hasPlan)) return null;

  // 收起态：靠右吸附抽屉标签
  if (collapsed) {
    const doneCnt = effectiveTodos.filter((t) => t.status === "done").length;
    return (
      <div className="absolute top-3 right-0 z-20 select-none">
        <button
          className="group flex items-center gap-2 pl-2.5 pr-2 py-1.5 rounded-l-xl bg-panel2/90 hover:bg-panel2 backdrop-blur-md border-y border-l border-edge hover:border-accent/50 shadow-lg text-[11px] text-inkdim hover:text-ink transition-all cursor-pointer"
          onClick={() => setCollapsed(false)}
          title="展开任务与进程面板"
        >
          <ChevronLeft
            size={13}
            className="text-inkdim group-hover:text-accent transition-transform duration-150 group-hover:-translate-x-0.5 shrink-0"
          />
          {hasPlan && (
            <span className="text-[10px] px-1.5 py-0.5 rounded bg-emerald-500/15 text-emerald-400 font-medium">
              📋 计划 v{activePlan.meta.version}
            </span>
          )}
          {hasCmds && <span className="w-2 h-2 rounded-full bg-green-400 animate-pulse shrink-0" />}
          {hasTodos && (
            <span className="flex items-center gap-1 font-medium">
              <CheckSquare size={13} className="text-purple-400 shrink-0" />
              <span className="font-mono text-[11px]">{doneCnt}/{effectiveTodos.length}</span>
            </span>
          )}
          {hasCmds && !hasTodos && !hasPlan && (
            <span className="flex items-center gap-1 text-green-400 font-mono">
              <Terminal size={13} className="shrink-0" />
              <span>{runningCmds.length} 进程</span>
            </span>
          )}
        </button>
      </div>
    );
  }

  // 展开态：靠右贴边抽屉面板
  return (
    <>
      <div className="absolute top-3 right-0 z-20 w-[310px] max-w-[calc(100%-1rem)] rounded-l-2xl bg-panel2/95 backdrop-blur-md border-y border-l border-edge shadow-2xl overflow-hidden select-none animate-in fade-in slide-in-from-right-2 duration-150">
        {/* 标题栏 */}
        <div className="flex items-center justify-between px-3.5 py-2.5 border-b border-edge bg-panel/50">
          <span className="text-[11px] text-ink font-medium flex items-center gap-1.5">
            <CheckSquare size={13} className="text-emerald-400" />
            <span>任务与计划总控</span>
          </span>
          <div className="flex items-center gap-1">
            <button
              className="flex items-center gap-1 text-[10px] text-inkdim hover:text-ink px-1.5 py-0.5 rounded-md hover:bg-panel3 transition-colors cursor-pointer"
              onClick={fetchActivePlan}
              title="重新从磁盘读取方案与任务进度"
            >
              <RotateCcw size={11} className={refreshing ? "animate-spin text-emerald-400" : ""} />
              <span>刷新</span>
            </button>
            <button
              className="flex items-center gap-1 text-[10px] text-inkdim hover:text-ink px-2 py-0.5 rounded-md hover:bg-panel3 transition-colors cursor-pointer"
              onClick={() => setCollapsed(true)}
              title="收起至右侧"
            >
              <span>收起</span>
              <ChevronRight size={12} />
            </button>
          </div>
        </div>

        <div className="px-3.5 py-2.5 flex flex-col gap-2.5 max-h-[min(70vh,480px)] overflow-y-auto">
          {/* 活动计划卡片 */}
          {activePlan && (
            <div className="rounded-xl bg-emerald-500/10 border border-emerald-500/20 p-2.5 text-[11.5px] space-y-1.5">
              <div className="flex items-center justify-between gap-1.5">
                <span className="font-semibold text-emerald-400 truncate flex-1 min-w-0" title={activePlan.meta.title}>
                  📋 {activePlan.meta.title}
                </span>
                <span className="font-mono text-[10px] text-emerald-300 bg-emerald-500/20 px-1.5 py-0.5 rounded shrink-0">
                  v{activePlan.meta.version}
                </span>
              </div>
              <div className="text-[11px] text-inkdim flex items-center justify-between gap-1">
                <span className="truncate font-mono text-[10px]">{activePlan.filename}</span>
                <div className="flex items-center gap-2 shrink-0">
                  <button
                    type="button"
                    className="text-emerald-400 hover:text-emerald-300 font-medium hover:underline cursor-pointer"
                    onClick={() => {
                      const cleanRel = `.harness/plans/${activePlan.filename}`;
                      const absPath = currentWorkspace
                        ? `${currentWorkspace.replace(/\\/g, "/").replace(/\/$/, "")}/${cleanRel}`
                        : cleanRel;
                      ipc.openFileViewer({
                        id: `file:${absPath}`,
                        type: "file",
                        title: activePlan.filename,
                        path: absPath,
                        workspacePath: currentWorkspace,
                        sessionId: currentId || undefined,
                      });
                    }}
                    title="以 Markdown 文件形式在独立窗体中查看与编辑方案"
                  >
                    方案文档 (MD)
                  </button>
                  <button
                    type="button"
                    className="text-inkdim hover:text-ink hover:underline cursor-pointer"
                    onClick={() => setShowPlanModal(true)}
                  >
                    弹窗
                  </button>
                </div>
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
          {hasCmds && hasTodos && <div className="border-t border-edge" />}

          {/* 任务清单 */}
          {hasTodos && (
            <TodoSection
              todos={effectiveTodos}
              isRunning={isRunning}
              onToggle={handleToggleStep}
            />
          )}
        </div>
      </div>

      {/* 完整方案预览弹窗 */}
      {showPlanModal && activePlan && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm p-4 animate-in fade-in duration-150">
          <div className="bg-panel rounded-2xl border border-edge shadow-2xl max-w-2xl w-full max-h-[85vh] flex flex-col overflow-hidden">
            <div className="flex items-center justify-between px-5 py-3.5 border-b border-edge bg-panel2/60">
              <div className="flex items-center gap-2 min-w-0">
                <span className="text-sm font-semibold text-ink truncate">
                  {activePlan.meta.title}
                </span>
                <span className="text-[11px] font-mono px-2 py-0.5 rounded bg-emerald-500/10 text-emerald-400 border border-emerald-500/20">
                  v{activePlan.meta.version}
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
              <Markdown content={activePlan.body} />
            </div>
            <div className="px-5 py-3 border-t border-edge bg-panel2/40 flex justify-between items-center text-[11px] text-inkdim">
              <span className="font-mono">文件：.harness/plans/{activePlan.filename}</span>
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
