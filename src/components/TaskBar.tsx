import { useEffect, useState, useMemo, useRef } from "react";
import { useStore } from "../store";
import { DRAFT_ID } from "../types";
import {
  Target,
  Loader2,
  Pause,
  Play,
  Square,
  CheckCircle2,
  AlertCircle,
  Sparkles,
  List,
  X,
  Clock,
  Layers,
  RotateCcw,
} from "./Icons";

export function TaskBar() {
  const currentId = useStore((s) => s.currentId);
  const activeTasks = useStore((s) => s.activeTasks);
  const task = currentId && currentId !== DRAFT_ID ? activeTasks[currentId] ?? null : null;
  const pauseLongTask = useStore((s) => s.pauseLongTask);
  const resumeLongTask = useStore((s) => s.resumeLongTask);
  const cancelLongTask = useStore((s) => s.cancelLongTask);
  const setShowTaskDetailModal = useStore((s) => s.setShowTaskDetailModal);

  const allSubprocesses = useStore((s) => s.subprocesses);
  const allSubagents = useStore((s) => s.subagents);
  const subprocesses = useMemo(() => {
    if (!currentId || currentId === DRAFT_ID) return [];
    const procs = allSubprocesses[currentId] ?? [];
    const ags = allSubagents[currentId] ?? [];
    const map = new Map<string, typeof procs[0]>();
    for (const p of procs) map.set(p.id, p);
    for (const a of ags) {
      if (!map.has(a.id)) {
        map.set(a.id, a);
      } else {
        map.set(a.id, { ...map.get(a.id)!, ...a });
      }
    }
    return Array.from(map.values());
  }, [allSubprocesses, allSubagents, currentId]);
  const restartAllSubagents = useStore((s) => s.restartAllSubagents);
  const stopSubagent = useStore((s) => s.stopSubagent);
  const continueTurn = useStore((s) => s.continueTurn);
  const runStatus = useStore((s) => s.runStatus);

  const [elapsed, setElapsed] = useState(0);
  const [justCompleted, setJustCompleted] = useState(false);
  const [dismissedSessionId, setDismissedSessionId] = useState<string | null>(null);
  const wasUncompletedRef = useRef<Record<string, boolean>>({});

  // 长任务判定：是否存在未完成的子任务
  const hasUnfinishedLongSubtasks = useMemo(() => {
    if (!task || task.status === "cancelled") return false;
    const subs = task.subtasks || [];
    // 只要有任意子任务状态不是 completed，或者长任务状态本身处于 planning/running/paused/interrupted/failed，均视为未完成
    if (task.status !== "completed") return true;
    return subs.some((s) => s.status !== "completed");
  }, [task]);

  // 派生子进程/子任务判定：是否存在未真正完成的子任务（包括执行中、待推进、已中断、异常失败）
  const uncompletedSubprocesses = useMemo(() => {
    return subprocesses.filter((s) => s.status !== "completed");
  }, [subprocesses]);

  const totalSubCount = subprocesses.length;
  const completedSubCount = subprocesses.filter((s) => s.status === "completed").length;
  const allSubsCompleted = totalSubCount > 0 && completedSubCount === totalSubCount;

  // 只要检测到该会话有正在推进/未完成的子任务，记录标记
  useEffect(() => {
    if (!currentId || currentId === DRAFT_ID) return;
    if (uncompletedSubprocesses.length > 0) {
      wasUncompletedRef.current[currentId] = true;
      setDismissedSessionId(null);
    }
  }, [currentId, uncompletedSubprocesses.length]);

  // 仅当子任务由“未完成”真正变为“全部完成 (completed)”时，才触发短暂就绪庆祝态，随后自动消失
  useEffect(() => {
    if (!currentId || currentId === DRAFT_ID) return;
    if (
      wasUncompletedRef.current[currentId] &&
      totalSubCount > 0 &&
      allSubsCompleted &&
      !hasUnfinishedLongSubtasks
    ) {
      setJustCompleted(true);
      const timer = setTimeout(() => {
        setJustCompleted(false);
        wasUncompletedRef.current[currentId] = false;
      }, 3000);
      return () => clearTimeout(timer);
    } else {
      setJustCompleted(false);
    }
  }, [currentId, totalSubCount, allSubsCompleted, hasUnfinishedLongSubtasks]);

  // 计时器：任务运行或规划中时累计已耗时
  useEffect(() => {
    if (
      !task ||
      task.status === "completed" ||
      task.status === "cancelled" ||
      task.status === "paused" ||
      task.status === "interrupted" ||
      task.status === "failed"
    ) {
      return;
    }
    const timer = setInterval(() => {
      setElapsed((e) => e + 1);
    }, 1000);
    return () => clearInterval(timer);
  }, [task?.status, task?.id]);

  // 核心显示条件：只要存在未完成的长任务子任务，或者存在未完成/刚完成的协作子任务，就显示该栏
  if (!currentId || currentId === DRAFT_ID) {
    return null;
  }

  const isDismissed = dismissedSessionId === currentId;
  const shouldShow =
    !isDismissed &&
    (hasUnfinishedLongSubtasks || uncompletedSubprocesses.length > 0 || justCompleted);
  if (!shouldShow) {
    return null;
  }

  const formatSeconds = (sec: number) => {
    const m = Math.floor(sec / 60);
    const s = sec % 60;
    return `${m.toString().padStart(2, "0")}:${s.toString().padStart(2, "0")}`;
  };

  // 场景 A：当前会话存在长任务
  if (task && hasUnfinishedLongSubtasks) {
    const subtasks = task.subtasks || [];
    const completedCount = subtasks.filter((s) => s.status === "completed").length;
    const totalCount = subtasks.length;
    const pct =
      totalCount > 0
        ? Math.round((completedCount / totalCount) * 100)
        : task.status === "planning"
        ? 15
        : 0;

    const curSub =
      subtasks[task.currentSubtaskIndex] ??
      (subtasks.length > 0 ? subtasks[subtasks.length - 1] : null);

    const getStatusBadge = () => {
      switch (task.status) {
        case "planning":
          return (
            <span className="flex items-center gap-1 text-amber-400 font-medium">
              <Sparkles size={13} className="animate-pulse" />
              <span>规划拆解中</span>
            </span>
          );
        case "running":
          return (
            <span className="flex items-center gap-1 text-purple-400 font-medium">
              <Loader2 size={13} className="animate-spin" />
              <span>自主推进中</span>
            </span>
          );
        case "paused":
          return (
            <span className="flex items-center gap-1 text-amber-400 font-medium">
              <Pause size={13} />
              <span>已暂停</span>
            </span>
          );
        case "interrupted":
          return (
            <span className="flex items-center gap-1 text-amber-400 font-medium animate-pulse">
              <AlertCircle size={13} />
              <span>已中断 · 待恢复</span>
            </span>
          );
        case "failed":
          return (
            <span className="flex items-center gap-1 text-red-400 font-medium">
              <AlertCircle size={13} />
              <span>执行遇阻</span>
            </span>
          );
        case "completed":
          return (
            <span className="flex items-center gap-1 text-emerald-400 font-medium">
              <CheckCircle2 size={13} />
              <span>任务已达成</span>
            </span>
          );
        default:
          return (
            <span className="flex items-center gap-1 text-inkdim font-medium">
              <span>{task.status}</span>
            </span>
          );
      }
    };

    const isRunning = task.status === "running";

    return (
      <div className="h-10 border-b border-edge/80 bg-panel/75 backdrop-blur px-3 flex items-center gap-3 select-none shrink-0 text-[12px] shadow-sm animate-in fade-in slide-in-from-top-1 duration-200">
        {/* 状态徽章与标识 */}
        <div className="flex items-center gap-2 shrink-0 pr-2 border-r border-edge/60">
          <div className="p-1 rounded bg-purple-500/10 text-purple-400 border border-purple-500/20">
            <Target size={13} />
          </div>
          {getStatusBadge()}
        </div>

        {/* 任务目标与当前子任务 */}
        <div className="flex items-center gap-2 min-w-0 max-w-[280px] shrink-0">
          <span
            className="font-medium text-ink truncate max-w-[140px]"
            title={`总目标: ${task.goal}`}
          >
            {task.goal}
          </span>
          {curSub && (
            <span
              className="px-1.5 py-0.5 rounded bg-panel2 border border-edge text-[11px] text-inkdim truncate max-w-[130px]"
              title={`当前子任务 [${curSub.index}/${totalCount}]: ${curSub.title}`}
            >
              [{curSub.index}/{totalCount}] {curSub.title}
            </span>
          )}
        </div>

        {/* 紧凑型进度条 */}
        <div className="flex items-center gap-2 flex-1 min-w-[120px] max-w-[320px]">
          <div className="flex-1 h-1.5 bg-panel3 rounded-full overflow-hidden border border-edge/40">
            <div
              className={`h-full rounded-full transition-all duration-500 ${
                task.status === "completed"
                  ? "bg-emerald-400"
                  : task.status === "paused" || task.status === "interrupted"
                  ? "bg-amber-400"
                  : task.status === "failed"
                  ? "bg-red-400"
                  : "bg-gradient-to-r from-purple-500 to-indigo-400"
              }`}
              style={{ width: `${pct}%` }}
            />
          </div>
          <span className="text-[11px] font-mono text-inkdim shrink-0">
            {totalCount > 0 ? `${completedCount}/${totalCount} (${pct}%)` : "准备中"}
          </span>
        </div>

        {/* 耗时与 Token 统计 */}
        <div className="flex items-center gap-2.5 text-[11px] text-inkdim shrink-0 ml-auto pr-1">
          {elapsed > 0 && (
            <span className="flex items-center gap-1 font-mono">
              <Clock size={12} className="text-inkdim/70" />
              {formatSeconds(elapsed)}
            </span>
          )}
          {task.totalTokensUsed > 0 && (
            <span className="px-1.5 py-0.5 rounded bg-panel2 border border-edge/60 text-[10px] font-mono">
              {task.totalTokensUsed >= 1000
                ? `${(task.totalTokensUsed / 1000).toFixed(1)}k tokens`
                : `${task.totalTokensUsed} tokens`}
            </span>
          )}
        </div>

        {/* 控制操作按钮组 */}
        <div className="flex items-center gap-1.5 shrink-0">
          {isRunning ? (
            <button
              onClick={() => pauseLongTask(task.id)}
              title="暂停当前长任务"
              className="flex items-center gap-1 px-2 py-1 rounded-md bg-amber-500/10 hover:bg-amber-500/20 text-amber-400 border border-amber-500/30 transition-colors cursor-pointer text-[11px]"
            >
              <Pause size={12} />
              <span>暂停</span>
            </button>
          ) : (
            <button
              onClick={async () => {
                await resumeLongTask(task.id);
                if (currentId && runStatus[currentId] !== "running") {
                  void continueTurn(currentId);
                }
              }}
              title={`一键恢复推进未完成子任务并协同恢复主会话: ${curSub ? curSub.title : task.goal}`}
              className="flex items-center gap-1.5 px-2.5 py-1 rounded-md bg-emerald-500/15 hover:bg-emerald-500/25 text-emerald-400 border border-emerald-500/35 transition-all font-medium cursor-pointer text-[11px] shadow-xs"
            >
              <Play size={12} fill="currentColor" />
              <span>恢复子任务</span>
            </button>
          )}

          {task.status !== "completed" && (
            <button
              onClick={() => cancelLongTask(task.id)}
              title="终止长任务"
              className="flex items-center gap-1 px-2 py-1 rounded-md bg-red-500/10 hover:bg-red-500/20 text-red-400 border border-red-500/30 transition-colors cursor-pointer text-[11px]"
            >
              <Square size={12} />
              <span>终止</span>
            </button>
          )}

          {/* 展开详细看板按钮 */}
          <button
            onClick={() => setShowTaskDetailModal(true)}
            title="展开任务路线图与阶段检查点"
            className="flex items-center gap-1 px-2 py-1 rounded-md bg-panel2 hover:bg-panel3 text-inkdim hover:text-ink border border-edge transition-colors cursor-pointer text-[11px]"
          >
            <List size={12} />
            <span>任务看板</span>
          </button>

          {task.status === "completed" && (
            <button
              onClick={() => cancelLongTask(task.id)}
              title="关闭长任务状态栏"
              className="p-1 rounded hover:bg-panel3 text-inkdim hover:text-ink transition-colors cursor-pointer"
            >
              <X size={12} />
            </button>
          )}
        </div>
      </div>
    );
  }

  // 场景 B：当前会话无长任务，但存在派生协作子任务/子进程
  const anySubRunning = subprocesses.some((s) => runStatus[s.id] === "running");
  const anySubInterrupted = subprocesses.some(
    (s) => s.status === "cancelled" || s.status === "stopped" || s.status === "interrupted"
  );
  const anySubFailed = subprocesses.some((s) => s.status === "failed");
  const subPct = totalSubCount > 0 ? Math.round((completedSubCount / totalSubCount) * 100) : 0;
  const activeSub = uncompletedSubprocesses[0];

  const handleStopAllSubs = async () => {
    for (const sub of subprocesses) {
      if (runStatus[sub.id] === "running") {
        void stopSubagent(sub.id);
      }
    }
  };

  return (
    <div className="h-10 border-b border-edge/80 bg-panel/75 backdrop-blur px-3 flex items-center gap-3 select-none shrink-0 text-[12px] shadow-sm animate-in fade-in slide-in-from-top-1 duration-200">
      <div className="flex items-center gap-2 shrink-0 pr-2 border-r border-edge/60">
        <div className="p-1 rounded bg-indigo-500/10 text-indigo-400 border border-indigo-500/20">
          <Layers size={13} />
        </div>
        {anySubRunning ? (
          <span className="flex items-center gap-1 text-indigo-400 font-medium">
            <Loader2 size={13} className="animate-spin" />
            <span>子任务执行中</span>
          </span>
        ) : allSubsCompleted ? (
          <span className="flex items-center gap-1 text-emerald-400 font-medium">
            <CheckCircle2 size={13} />
            <span>子任务已全部完成</span>
          </span>
        ) : anySubInterrupted ? (
          <span className="flex items-center gap-1 text-amber-400 font-medium">
            <AlertCircle size={13} />
            <span>子任务已中止</span>
          </span>
        ) : anySubFailed ? (
          <span className="flex items-center gap-1 text-rose-400 font-medium">
            <AlertCircle size={13} />
            <span>子任务执行异常</span>
          </span>
        ) : (
          <span className="flex items-center gap-1 text-inkdim font-medium">
            <Clock size={13} />
            <span>子任务待推进</span>
          </span>
        )}
      </div>

      <div className="flex items-center gap-2 min-w-0 max-w-[280px] shrink-0">
        <span className="font-medium text-ink truncate max-w-[130px]">
          协作子任务
        </span>
        {activeSub && (
          <span
            className="px-1.5 py-0.5 rounded bg-panel2 border border-edge text-[11px] text-inkdim truncate max-w-[140px]"
            title={`当前子任务: ${activeSub.title}`}
          >
            ⚡ {activeSub.title}
          </span>
        )}
      </div>

      <div className="flex items-center gap-2 flex-1 min-w-[120px] max-w-[320px]">
        <div className="flex-1 h-1.5 bg-panel3 rounded-full overflow-hidden border border-edge/40">
          <div
            className={`h-full rounded-full transition-all duration-500 ${
              allSubsCompleted
                ? "bg-emerald-400"
                : anySubRunning
                ? "bg-gradient-to-r from-indigo-500 to-purple-400"
                : anySubFailed
                ? "bg-rose-400"
                : anySubInterrupted
                ? "bg-amber-400"
                : "bg-inkdim/50"
            }`}
            style={{ width: `${allSubsCompleted ? 100 : subPct}%` }}
          />
        </div>
        <span className="text-[11px] font-mono text-inkdim shrink-0">
          {totalSubCount > 0
            ? allSubsCompleted
              ? `${totalSubCount}/${totalSubCount} (100%)`
              : `${completedSubCount}/${totalSubCount} (${subPct}%)`
            : "准备中"}
        </span>
      </div>

      <div className="flex items-center gap-1.5 shrink-0 ml-auto">
        {anySubRunning ? (
          <button
            onClick={handleStopAllSubs}
            className="flex items-center gap-1 px-2.5 py-1 rounded-md text-amber-400 hover:bg-amber-500/10 border border-amber-500/30 transition-colors cursor-pointer text-[11px]"
            title="停止所有正在运行的子任务进程"
          >
            <Square size={11} fill="currentColor" />
            <span>全部停止</span>
          </button>
        ) : allSubsCompleted ? (
          <span className="text-[11px] text-emerald-400/90 font-medium px-2 py-0.5 rounded bg-emerald-500/10 border border-emerald-500/20">
            已就绪
          </span>
        ) : (
          <button
            onClick={async () => {
              await restartAllSubagents(currentId);
              if (currentId && runStatus[currentId] !== "running") {
                void continueTurn(currentId);
              }
            }}
            className="flex items-center gap-1.5 px-2.5 py-1 rounded-md bg-emerald-500/15 hover:bg-emerald-500/25 text-emerald-400 border border-emerald-500/35 transition-all font-medium cursor-pointer text-[11px] shadow-xs"
            title="一键恢复推进所有未完成子任务并协同恢复主会话"
          >
            <Play size={12} fill="currentColor" />
            <span>恢复子任务</span>
          </button>
        )}

        {/* 手动关闭状态栏 */}
        {(!anySubRunning || allSubsCompleted) && (
          <button
            onClick={() => setDismissedSessionId(currentId)}
            title="关闭子任务状态栏"
            className="p-1 rounded hover:bg-panel3 text-inkdim hover:text-ink transition-colors cursor-pointer ml-1"
          >
            <X size={12} />
          </button>
        )}
      </div>
    </div>
  );
}
