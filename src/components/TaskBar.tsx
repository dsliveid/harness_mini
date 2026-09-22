import { useEffect, useState } from "react";
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
} from "./Icons";

export function TaskBar() {
  const currentId = useStore((s) => s.currentId);
  const activeTasks = useStore((s) => s.activeTasks);
  const task = currentId && currentId !== DRAFT_ID ? activeTasks[currentId] ?? null : null;
  const pauseLongTask = useStore((s) => s.pauseLongTask);
  const resumeLongTask = useStore((s) => s.resumeLongTask);
  const cancelLongTask = useStore((s) => s.cancelLongTask);
  const setShowTaskDetailModal = useStore((s) => s.setShowTaskDetailModal);
  const [elapsed, setElapsed] = useState(0);

  // 计时器：任务运行或规划中时累计已耗时
  useEffect(() => {
    if (!task || task.status === "completed" || task.status === "cancelled" || task.status === "paused") {
      return;
    }
    const timer = setInterval(() => {
      setElapsed((e) => e + 1);
    }, 1000);
    return () => clearInterval(timer);
  }, [task?.status, task?.id]);

  if (!task || task.status === "cancelled") {
    return null;
  }

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

  const formatSeconds = (sec: number) => {
    const m = Math.floor(sec / 60);
    const s = sec % 60;
    return `${m.toString().padStart(2, "0")}:${s.toString().padStart(2, "0")}`;
  };

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
      case "completed":
        return (
          <span className="flex items-center gap-1 text-emerald-400 font-medium">
            <CheckCircle2 size={13} />
            <span>任务已达成</span>
          </span>
        );
      case "failed":
        return (
          <span className="flex items-center gap-1 text-red-400 font-medium">
            <AlertCircle size={13} />
            <span>执行遇阻</span>
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

  return (
    <div className="h-10 border-b border-edge/80 bg-panel/70 backdrop-blur px-3 flex items-center gap-3 select-none shrink-0 text-[12px] shadow-sm animate-in fade-in slide-in-from-top-1 duration-200">
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
          className="font-medium text-ink truncate max-w-[150px]"
          title={`总目标: ${task.goal}`}
        >
          {task.goal}
        </span>
        {curSub && (
          <span
            className="px-1.5 py-0.5 rounded bg-panel2 border border-edge text-[11px] text-inkdim truncate max-w-[120px]"
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
                : task.status === "paused"
                ? "bg-amber-400"
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
        {task.status === "running" && (
          <button
            onClick={() => pauseLongTask(task.id)}
            title="暂停当前长任务"
            className="flex items-center gap-1 px-2 py-1 rounded-md bg-amber-500/10 hover:bg-amber-500/20 text-amber-400 border border-amber-500/30 transition-colors cursor-pointer text-[11px]"
          >
            <Pause size={12} />
            <span>暂停</span>
          </button>
        )}

        {(task.status === "paused" || task.status === "failed") && (
          <button
            onClick={() => resumeLongTask(task.id)}
            title="继续推进长任务"
            className="flex items-center gap-1 px-2 py-1 rounded-md bg-emerald-500/10 hover:bg-emerald-500/20 text-emerald-400 border border-emerald-500/30 transition-colors cursor-pointer text-[11px]"
          >
            <Play size={12} />
            <span>继续</span>
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
