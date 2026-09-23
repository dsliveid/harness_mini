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
  RefreshCw,
} from "./Icons";

const getSubtaskBadge = (role?: string | null) => {
  const r = (role || "").toLowerCase();
  if (r.includes("front") || r.includes("ui") || r.includes("前端")) return { label: role || "前端", icon: "🎨" };
  if (r.includes("back") || r.includes("后端") || r.includes("api")) return { label: role || "后端", icon: "⚙️" };
  if (r.includes("test") || r.includes("测试")) return { label: role || "测试", icon: "🧪" };
  if (r.includes("review") || r.includes("审阅") || r.includes("审查")) return { label: role || "审阅", icon: "🔍" };
  if (r.includes("search") || r.includes("research") || r.includes("调研") || r.includes("探查")) return { label: role || "调研", icon: "🔎" };
  if (r.includes("doc") || r.includes("文档")) return { label: role || "文档", icon: "📝" };
  if (r.includes("bug") || r.includes("debug") || r.includes("排查") || r.includes("修复")) return { label: role || "排查", icon: "🐞" };
  if (r.includes("refactor") || r.includes("重构")) return { label: role || "重构", icon: "♻️" };
  return { label: role || "子任务", icon: "⚡" };
};

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
  const allCollaborators = useStore((s) => s.collaborators);
  const activeSubprocessId = useStore((s) => s.activeSubprocessId);
  const activeCollaboratorId = useStore((s) => s.activeCollaboratorId);
  const activeSubagentId = useStore((s) => s.activeSubagentId);
  const setActiveSubprocessId = useStore((s) => s.setActiveSubprocessId);
  const setActiveCollaboratorId = useStore((s) => s.setActiveCollaboratorId);
  const scrollRef = useRef<HTMLDivElement>(null);

  const handleWheel = (e: React.WheelEvent<HTMLDivElement>) => {
    if (scrollRef.current) {
      if (Math.abs(e.deltaY) > Math.abs(e.deltaX)) {
        scrollRef.current.scrollLeft += e.deltaY;
      }
    }
  };

  const subprocesses = useMemo(() => {
    if (!currentId || currentId === DRAFT_ID) return [];
    const procs = allSubprocesses[currentId] ?? [];
    const ags = allSubagents[currentId] ?? [];
    const collabs = allCollaborators[currentId] ?? [];
    const collabIds = new Set(collabs.map((c) => c.id));

    const map = new Map<string, typeof procs[0]>();
    for (const p of procs) {
      if (p.sessionType !== "collaborator" && !collabIds.has(p.id)) {
        map.set(p.id, p);
      }
    }
    for (const a of ags) {
      if (a.sessionType !== "collaborator" && !collabIds.has(a.id)) {
        if (!map.has(a.id)) {
          map.set(a.id, a);
        } else {
          map.set(a.id, { ...map.get(a.id)!, ...a });
        }
      }
    }
    return Array.from(map.values());
  }, [allSubprocesses, allSubagents, allCollaborators, currentId]);

  const restartAllSubagents = useStore((s) => s.restartAllSubagents);
  const stopSubagent = useStore((s) => s.stopSubagent);
  const continueTurn = useStore((s) => s.continueTurn);
  const runStatus = useStore((s) => s.runStatus);

  const [elapsed, setElapsed] = useState(0);

  // 长任务判定：是否存在未完成的子任务
  const hasUnfinishedLongSubtasks = useMemo(() => {
    if (!task || task.status === "cancelled") return false;
    const subs = task.subtasks || [];
    // 只要有任意子任务状态不是 completed，或者长任务状态本身处于 planning/running/paused/interrupted/failed，均视为未完成
    if (task.status !== "completed") return true;
    return subs.some((s) => s.status !== "completed");
  }, [task]);

  const totalSubCount = subprocesses.length;
  const completedSubCount = subprocesses.filter((s) => s.status === "completed").length;
  const allSubsCompleted = totalSubCount > 0 && completedSubCount === totalSubCount;

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

  // 核心显示条件：只要存在未完成的长任务子任务，或者存在协作子任务，就显示该栏（参照协作者栏）
  if (!currentId || currentId === DRAFT_ID) {
    return null;
  }

  const shouldShow = hasUnfinishedLongSubtasks || subprocesses.length > 0;
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

  const handleStopAllSubs = async () => {
    for (const sub of subprocesses) {
      if (runStatus[sub.id] === "running") {
        void stopSubagent(sub.id);
      }
    }
  };

  return (
    <div className="h-10 border-b border-edge/80 bg-panel/60 backdrop-blur px-3 flex items-center gap-2 select-none shrink-0 text-[12px]">
      {/* 标题标识 */}
      <div className="flex items-center gap-1.5 text-inkdim font-medium shrink-0 pr-2 border-r border-edge/60">
        <Sparkles size={14} className="text-accent" />
        <span className="font-medium text-ink">协作子任务</span>
        {totalSubCount > 0 && (
          <span
            className={`px-1.5 py-0.2 text-[10px] rounded-full border ${
              allSubsCompleted
                ? "bg-emerald-500/10 text-emerald-400 border-emerald-500/30"
                : anySubRunning
                ? "bg-indigo-500/10 text-indigo-400 border-indigo-500/30"
                : "bg-panel2 border-edge text-inkdim"
            }`}
            title={`已完成 ${completedSubCount} / 共 ${totalSubCount} 个子任务`}
          >
            {completedSubCount}/{totalSubCount}
          </span>
        )}
      </div>

      {/* 子任务水平平铺滚动列表（无原生滚动条，鼠标滚轮平滑横向滚动，参照协作者） */}
      <div
        ref={scrollRef}
        onWheel={handleWheel}
        className="flex items-center gap-1.5 flex-1 overflow-x-auto scrollbar-none py-1 [scrollbar-width:none] [-ms-overflow-style:none] [&::-webkit-scrollbar]:hidden"
      >
        {subprocesses.map((sub) => {
          const isRunning = runStatus[sub.id] === "running";
          const isActive =
            activeSubprocessId === sub.id ||
            activeCollaboratorId === sub.id ||
            activeSubagentId === sub.id;
          const roleInfo = getSubtaskBadge(sub.subagentRole);

          return (
            <button
              key={sub.id}
              onClick={() => {
                if (isActive) {
                  setActiveSubprocessId(null);
                  setActiveCollaboratorId(null);
                } else {
                  setActiveSubprocessId(sub.id);
                }
              }}
              className={`group flex items-center gap-1.5 px-2.5 py-1 rounded-lg border transition-all shrink-0 max-w-[200px] text-left cursor-pointer ${
                isActive
                  ? "bg-accent/15 border-accent/60 text-ink shadow-sm ring-1 ring-accent/30"
                  : "bg-panel2/70 hover:bg-panel2 border-edge text-inkdim hover:text-ink"
              }`}
              title={`名称: ${sub.title}\n角色: ${roleInfo.label}\n状态: ${
                isRunning
                  ? "运行中"
                  : sub.status === "completed"
                  ? "已完成"
                  : sub.status === "failed"
                  ? "执行异常"
                  : sub.status === "cancelled" || sub.status === "interrupted"
                  ? "已中止"
                  : "待推进"
              }`}
            >
              <span className="text-[12px]">{roleInfo.icon}</span>
              <span className="truncate font-medium flex-1">{sub.title}</span>
              {isRunning ? (
                <Loader2 size={12} className="shrink-0 animate-spin text-accent" />
              ) : sub.status === "completed" ? (
                <CheckCircle2 size={12} className="shrink-0 text-emerald-400" />
              ) : sub.status === "failed" ? (
                <AlertCircle size={12} className="shrink-0 text-rose-400" />
              ) : sub.status === "cancelled" || sub.status === "interrupted" ? (
                <AlertCircle size={12} className="shrink-0 text-amber-400" />
              ) : (
                <span className="w-1.5 h-1.5 rounded-full bg-inkdim/40 shrink-0" />
              )}
            </button>
          );
        })}
      </div>

      {/* 控制操作按钮区：对齐 CollaboratorBar */}
      {subprocesses.length > 0 && (
        <div className="flex items-center gap-1 shrink-0 pl-1 border-l border-edge/60">
          {anySubRunning ? (
            <button
              onClick={handleStopAllSubs}
              className="flex items-center gap-1 px-2.5 py-1 rounded-lg text-amber-400 hover:bg-amber-500/10 border border-amber-500/20 transition-colors cursor-pointer text-[11.5px]"
              title="一键停止所有正在运行的子任务进程"
            >
              <Square size={11} fill="currentColor" />
              <span>全部停止</span>
            </button>
          ) : allSubsCompleted ? (
            <span className="flex items-center gap-1 text-[11px] text-emerald-400 font-medium px-2 py-0.5 rounded bg-emerald-500/10 border border-emerald-500/20">
              <CheckCircle2 size={12} />
              <span>已就绪</span>
            </span>
          ) : (
            <button
              onClick={async () => {
                await restartAllSubagents(currentId);
                if (currentId && runStatus[currentId] !== "running") {
                  void continueTurn(currentId);
                }
              }}
              className="flex items-center gap-1 px-2.5 py-1 rounded-lg text-inkdim hover:text-ink hover:bg-panel2 border border-edge transition-colors cursor-pointer text-[11.5px]"
              title="一键唤醒并恢复推进未完成子任务"
            >
              <RefreshCw size={11} />
              <span>全部重启</span>
            </button>
          )}
        </div>
      )}
    </div>
  );
}
