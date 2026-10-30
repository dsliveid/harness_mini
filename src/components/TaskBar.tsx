import { useEffect, useState, useMemo } from "react";
import { useStore } from "../store";
import { DRAFT_ID } from "../types";
import {
  Target,
  Pause,
  Play,
  Square,
  X,
  Clock,
  Zap,
  Coins,
} from "./Icons";

export function TaskBar() {
  const currentId = useStore((s) => s.currentId);
  const activeTasks = useStore((s) => s.activeTasks);
  const task = currentId && currentId !== DRAFT_ID ? activeTasks[currentId] ?? null : null;
  const pauseLongTask = useStore((s) => s.pauseLongTask);
  const resumeLongTask = useStore((s) => s.resumeLongTask);
  const cancelLongTask = useStore((s) => s.cancelLongTask);

  const [elapsed, setElapsed] = useState(0);

  // 长任务判定：是否存在未完成的阶段路线图
  const hasUnfinishedLongSubtasks = useMemo(() => {
    if (!task || task.status === "cancelled") return false;
    const subs = task.subtasks || [];
    if (task.status !== "completed") return true;
    return subs.some((s) => s.status !== "completed");
  }, [task]);

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

  // 若无长任务或长任务未激活，完全不占用垂直空间（返回 null）
  if (!currentId || currentId === DRAFT_ID || !task || !hasUnfinishedLongSubtasks) {
    return null;
  }

  const formatSeconds = (sec: number) => {
    const m = Math.floor(sec / 60);
    const s = sec % 60;
    return `${m.toString().padStart(2, "0")}:${s.toString().padStart(2, "0")}`;
  };

  const subtasks = task.subtasks || [];
  const completedCount = subtasks.filter((s) => s.status === "completed").length;
  const totalCount = subtasks.length;
  const pct =
    totalCount > 0
      ? Math.round((completedCount / totalCount) * 100)
      : task.status === "planning"
      ? 15
      : 0;

  const isRunning = task.status === "running";

  return (
    <div
      className="h-10 border-b border-edge/80 bg-panel/75 backdrop-blur px-3 flex items-center gap-3 select-none shrink-0 text-[12px] shadow-sm overflow-hidden animate-in fade-in slide-in-from-top-1 duration-200"
      title={`长任务总目标: ${task.goal}`}
    >
      {/* 1. 进度条 */}
      <div className="flex items-center gap-2 flex-1 min-w-[120px]">
        <div className="p-1 rounded bg-purple-500/10 text-purple-400 border border-purple-500/20 shrink-0">
          <Target size={13} />
        </div>
        <div className="flex-1 h-2 bg-panel3 rounded-full overflow-hidden border border-edge/40 min-w-[40px]">
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
          {totalCount > 0 ? `${completedCount}/${totalCount} (${pct}%)` : task.status === "planning" ? "拆解中" : "准备中"}
        </span>
      </div>

      {/* 2. 消耗时间 */}
      <div className="flex items-center gap-1 font-mono text-[11px] text-inkdim shrink-0" title="已消耗时间">
        <Clock size={12} className="text-inkdim/70" />
        <span>{formatSeconds(elapsed)}</span>
      </div>

      {/* 3. tokens & 缓存命中率 */}
      <div
        className="flex items-center gap-1.5 shrink-0"
        title={`长任务累计消耗: ${(task.totalTokensUsed ?? 0).toLocaleString()} tokens${
          (task.promptTokensUsed ?? 0) > 0 || (task.completionTokensUsed ?? 0) > 0
            ? `\n输入: ${(task.promptTokensUsed ?? 0).toLocaleString()}${
                (task.cachedTokensUsed ?? 0) > 0
                  ? ` (缓存命中: ${(task.cachedTokensUsed ?? 0).toLocaleString()} · ${(task.cacheHitRate ?? 0).toFixed(1)}%)`
                  : ""
              } · 输出: ${(task.completionTokensUsed ?? 0).toLocaleString()}`
            : ""
        }`}
      >
        <span className="px-1.5 py-0.5 rounded bg-panel2 border border-edge/60 text-[10px] font-mono text-inkdim inline-flex items-center gap-1">
          <Coins size={10} className="text-amber-400/80 shrink-0" />
          <span>
            {(task.totalTokensUsed ?? 0) >= 1000
              ? `${((task.totalTokensUsed ?? 0) / 1000).toFixed(1)}k tokens`
              : `${task.totalTokensUsed ?? 0} tokens`}
          </span>
        </span>
        {(task.cacheHitRate ?? 0) > 0 && (
          <span className="px-1.5 py-0.5 rounded text-[10px] bg-cyan-500/10 text-cyan-400 font-medium border border-cyan-500/20 inline-flex items-center gap-1">
            <Zap size={10} className="shrink-0" />
            <span>{(task.cacheHitRate ?? 0).toFixed(1)}%</span>
          </span>
        )}
      </div>

      {/* 4. 暂停 / 恢复 */}
      <div className="flex items-center gap-1.5 shrink-0">
        {isRunning ? (
          <button
            onClick={() => pauseLongTask(task.id)}
            title="暂停当前长任务"
            className="flex items-center gap-1 px-2.5 py-1 rounded-md bg-amber-500/10 hover:bg-amber-500/20 text-amber-400 border border-amber-500/30 transition-colors cursor-pointer text-[11px]"
          >
            <Pause size={12} />
            <span>暂停</span>
          </button>
        ) : (
          <button
            onClick={async () => {
              await resumeLongTask(task.id);
            }}
            title="恢复推进长任务"
            className="flex items-center gap-1 px-2.5 py-1 rounded-md bg-emerald-500/15 hover:bg-emerald-500/25 text-emerald-400 border border-emerald-500/35 transition-all font-medium cursor-pointer text-[11px] shadow-xs"
          >
            <Play size={12} fill="currentColor" />
            <span>恢复</span>
          </button>
        )}

        {/* 5. 终止 */}
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
