import { useState, useEffect, useMemo, useRef, useLayoutEffect, useCallback } from "react";
import { useStore } from "../store";
import { ipc } from "../ipc";
import {
  Loader2,
  CheckCircle2,
  AlertCircle,
  Square,
  Bot,
  Coins,
  Cpu,
  ChevronRight,
  ExternalLink,
  Sparkles,
} from "./Icons";
import type { ToolEvent, Session } from "../types";

function formatTokens(n?: number | null): string {
  if (n == null || isNaN(n)) return "0";
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(2)}M`;
  if (n >= 10_000) return `${(n / 1_000).toFixed(1)}k`;
  return n.toLocaleString("zh-CN");
}

const ROLE_INFO: Record<string, { label: string; icon: string; color: string; border: string }> = {
  frontend: { label: "前端开发", icon: "🎨", color: "text-blue-400 bg-blue-500/10", border: "border-blue-500/30" },
  backend: { label: "后端开发", icon: "⚙️", color: "text-emerald-400 bg-emerald-500/10", border: "border-emerald-500/30" },
  testing: { label: "测试校验", icon: "🧪", color: "text-purple-400 bg-purple-500/10", border: "border-purple-500/30" },
  review: { label: "代码审阅", icon: "🔍", color: "text-amber-400 bg-amber-500/10", border: "border-amber-500/30" },
  fullstack: { label: "全栈开发", icon: "⚡", color: "text-indigo-400 bg-indigo-500/10", border: "border-indigo-500/30" },
};

export interface SubprocessItemData {
  ev?: ToolEvent;
  subId: string;
  sub?: Session | null;
  title: string;
  task: string;
  role: string;
}

interface Point {
  x: number;
  y: number;
}

/**
 * 单个子任务标签卡片
 */
function SubprocessTaskTab({
  item,
  isActive,
  onToggleActive,
  onMountRef,
}: {
  item: SubprocessItemData;
  isActive: boolean;
  onToggleActive: () => void;
  onMountRef: (el: HTMLDivElement | null) => void;
}) {
  const runStatus = useStore((s) => s.runStatus);
  const stopSubagent = useStore((s) => s.stopSubagent);
  const pushToast = useStore((s) => s.pushToast);
  const isRunning = runStatus[item.subId] === "running";
  const [stopping, setStopping] = useState(false);

  const roleMeta = ROLE_INFO[item.role] ?? {
    label: "子进程",
    icon: "⚡",
    color: "text-indigo-400 bg-indigo-500/10",
    border: "border-indigo-500/30",
  };

  const handleStop = async (e: React.MouseEvent) => {
    e.stopPropagation();
    if (stopping || !item.subId) return;
    setStopping(true);
    try {
      await stopSubagent(item.subId);
      pushToast("已停止子任务");
    } finally {
      setStopping(false);
    }
  };

  const totalTokens = item.sub?.totalTokens ?? 0;

  return (
    <div
      ref={onMountRef}
      role="button"
      tabIndex={0}
      onClick={onToggleActive}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onToggleActive();
        }
      }}
      className={`group relative flex flex-col gap-1.5 p-3 rounded-xl border transition-all cursor-pointer select-none text-left ${
        isActive
          ? "bg-accent/15 border-accent shadow-sm ring-1 ring-accent/40"
          : "bg-panel2/80 hover:bg-panel3/70 border-edge hover:border-accent/50 shadow-xs"
      }`}
    >
      {/* 入线锚点（供 SVG 连线对齐） */}
      <span
        data-anchor="sub-in"
        className={`absolute -left-[5px] top-1/2 -translate-y-1/2 w-2 h-2 rounded-full border transition-colors ${
          isActive
            ? "bg-accent border-white"
            : isRunning
            ? "bg-accent border-accent/60 animate-ping"
            : "bg-edge border-inkdim/40"
        }`}
      />

      {/* 第一行：角色徽章、标题、状态指示与控制 */}
      <div className="flex items-center justify-between gap-2">
        <div className="flex items-center gap-2 min-w-0 flex-1">
          <span
            className={`px-1.5 py-0.5 rounded text-[10px] font-medium border flex items-center gap-1 shrink-0 ${roleMeta.color} ${roleMeta.border}`}
          >
            <span>{roleMeta.icon}</span>
            <span>{roleMeta.label}</span>
          </span>

          <span className="font-medium text-[13px] text-ink truncate" title={item.title}>
            {item.title}
          </span>
        </div>

        {/* 状态与动作 */}
        <div className="flex items-center gap-2 shrink-0">
          {totalTokens > 0 && (
            <span
              className="text-[11px] font-mono text-inkdim flex items-center gap-1"
              title={`累计消耗 ${totalTokens.toLocaleString()} tokens`}
            >
              <Coins size={11} className="text-amber-400" />
              <span>{formatTokens(totalTokens)}</span>
            </span>
          )}

          {item.ev?.status === "pending_approval" ? (
            <span className="flex items-center gap-1 text-[11px] text-amber-400 font-medium animate-pulse">
              <AlertCircle size={12} />
              <span>待审批</span>
            </span>
          ) : isRunning ? (
            <span className="flex items-center gap-1 text-[11px] text-accent font-medium animate-pulse">
              <Loader2 size={12} className="animate-spin" />
              <span>执行中</span>
            </span>
          ) : item.sub?.status === "failed" || item.ev?.status === "failed" ? (
            <span className="flex items-center gap-1 text-[11px] text-rose-400 font-medium">
              <AlertCircle size={12} />
              <span>已终止</span>
            </span>
          ) : (
            <span className="flex items-center gap-1 text-[11px] text-emerald-400 font-medium">
              <CheckCircle2 size={12} />
              <span>已完成</span>
            </span>
          )}

          {item.ev?.status === "pending_approval" && (
            <button
              type="button"
              onClick={async (e) => {
                e.stopPropagation();
                if (!item.ev?.id) return;
                try {
                  useStore.getState().approvalDone(item.ev.id);
                  await ipc.respondApproval(item.ev.id, "allow_session");
                  pushToast("已批准启动子任务");
                } catch (err) {
                  pushToast(String(err));
                }
              }}
              className="text-[10px] px-2 py-0.5 rounded bg-emerald-600 hover:bg-emerald-500 text-white font-medium transition-colors shadow-xs"
              title="批准启动该子任务"
            >
              批准
            </button>
          )}

          {isRunning && (
            <button
              type="button"
              onClick={handleStop}
              disabled={stopping}
              className="text-[10px] px-1.5 py-0.5 rounded bg-red-600/80 hover:bg-red-500 text-white disabled:opacity-50 transition-colors shadow-xs"
              title="终止该子进程"
            >
              <Square size={8} fill="currentColor" />
            </button>
          )}

          <span
            className={`text-[11px] flex items-center gap-0.5 font-medium transition-colors ${
              isActive ? "text-accent" : "text-inkdim group-hover:text-ink"
            }`}
          >
            <span>{isActive ? "查看中" : "查看流程"}</span>
            <ChevronRight size={12} className={`transition-transform ${isActive ? "rotate-90" : ""}`} />
          </span>
        </div>
      </div>

      {/* 第二行：任务目标预览 */}
      {item.task && (
        <div className="text-[12px] text-inkdim/90 truncate font-mono bg-panel/60 px-2 py-1 rounded border border-edge/40">
          {item.task}
        </div>
      )}
    </div>
  );
}

const EMPTY_SESSIONS: Session[] = [];

/**
 * 横向树形分支子进程组组件 (SubprocessBranchTree)
 * 将 1 个或多个并发创建的子进程以横向树状拓扑图展示
 */
export function SubprocessBranchTree({
  events,
  event,
  singleSubId,
}: {
  events?: ToolEvent[];
  event?: ToolEvent;
  singleSubId?: string;
}) {
  const currentParentId = useStore((s) => s.currentId);
  const allSubprocesses = useStore((s) => s.subprocesses);
  const allSubagents = useStore((s) => s.subagents);
  const subprocesses = useMemo(
    () => (currentParentId ? allSubprocesses[currentParentId] ?? EMPTY_SESSIONS : EMPTY_SESSIONS),
    [allSubprocesses, currentParentId]
  );
  const subagents = useMemo(
    () => (currentParentId ? allSubagents[currentParentId] ?? EMPTY_SESSIONS : EMPTY_SESSIONS),
    [allSubagents, currentParentId]
  );
  const activeCollaboratorId = useStore((s) => s.activeCollaboratorId);
  const setActiveCollaboratorId = useStore((s) => s.setActiveCollaboratorId);
  const runStatus = useStore((s) => s.runStatus);
  const pushToast = useStore((s) => s.pushToast);

  const normalizedEvents = useMemo(() => {
    if (events) return events;
    if (event) return [event];
    return null;
  }, [events, event]);

  // 解析各个子任务的信息
  const items: SubprocessItemData[] = useMemo(() => {
    const allSubprocs = [...subprocesses, ...subagents];

    if (singleSubId) {
      const sub = allSubprocs.find((s) => s.id === singleSubId) ?? null;
      return [
        {
          subId: singleSubId,
          sub,
          title: sub?.title || "临时执行子进程",
          task: sub?.subagentTask || "",
          role: sub?.subagentRole || "custom",
        },
      ];
    }

    if (!normalizedEvents || normalizedEvents.length === 0) return [];

    return normalizedEvents.map((ev) => {
      let subId = "";
      // 1. 优先读取强绑定的真实 ID
      if (ev.subprocessId) subId = String(ev.subprocessId);
      else if (ev.params?.subprocess_id) subId = String(ev.params.subprocess_id);
      else if (ev.params?.subagent_id) subId = String(ev.params.subagent_id);
      else if (ev.params?.sessionId) subId = String(ev.params.sessionId);

      // 2. 通过 triggerToolEventId 或 subId 进行精确反查
      let sub = allSubprocs.find((s) => (subId && s.id === subId) || (ev.id && s.triggerToolEventId === ev.id)) ?? null;
      if (sub && !subId) {
        subId = sub.id;
      }

      // 3. 从 resultText 中多模式提取
      if (!subId && ev.resultText) {
        const m1 = ev.resultText.match(/- ID:\s*[`"']?([a-zA-Z0-9_-]+)[`"']?/i);
        const m2 = ev.resultText.match(/ID:\s*[`"']?([a-zA-Z0-9_-]+)[`"']?/i);
        const m3 = ev.resultText.match(/子(?:进程|Agent)\s*ID:\s*[`"']?([a-zA-Z0-9_-]+)[`"']?/i);
        if (m1) subId = m1[1];
        else if (m2) subId = m2[1];
        else if (m3) subId = m3[1];
      }

      if (!sub && subId) {
        sub = allSubprocs.find((s) => s.id === subId) ?? null;
      }

      // 4. 标题与任务文本智能回退匹配
      const titleParam = String(ev.params?.title || "").trim();
      const taskParam = String(ev.params?.task || "").trim();
      if (!sub && (titleParam || taskParam)) {
        sub = allSubprocs.find((s) =>
          (titleParam && s.title === titleParam) ||
          (taskParam && s.subagentTask === taskParam)
        ) ?? null;
        if (sub) {
          subId = sub.id;
        }
      }

      const title = sub?.title || titleParam || "临时执行子进程";
      const task = sub?.subagentTask || taskParam || "";
      const role = sub?.subagentRole || String(ev.params?.role || "custom");

      return {
        ev,
        subId: subId || ev.id,
        sub,
        title,
        task,
        role,
      };
    });
  }, [normalizedEvents, singleSubId, subprocesses, subagents]);

  // 容器及锚点坐标测量
  const containerRef = useRef<HTMLDivElement>(null);
  const rootNodeRef = useRef<HTMLDivElement>(null);
  const taskRefs = useRef<(HTMLDivElement | null)[]>([]);

  const [svgPaths, setSvgPaths] = useState<{ d: string; isRunning: boolean; isActive: boolean }[]>([]);

  // 动态测量并绘制平滑横向贝塞尔曲线
  const updateLines = useCallback(() => {
    const container = containerRef.current;
    const rootNode = rootNodeRef.current;
    if (!container || !rootNode || items.length === 0) {
      setSvgPaths((prev) => (prev.length === 0 ? prev : []));
      return;
    }

    const containerRect = container.getBoundingClientRect();
    const rootRect = rootNode.getBoundingClientRect();

    // 出线点：主进程节点的右侧中心
    const startX = Math.round(rootRect.right - containerRect.left);
    const startY = Math.round(rootRect.top + rootRect.height / 2 - containerRect.top);

    const paths = items.map((item, idx) => {
      const taskEl = taskRefs.current[idx];
      if (!taskEl) {
        return { d: "", isRunning: false, isActive: false };
      }
      const taskRect = taskEl.getBoundingClientRect();
      // 入线点：子任务卡片的左侧中心
      const endX = Math.round(taskRect.left - containerRect.left);
      const endY = Math.round(taskRect.top + taskRect.height / 2 - containerRect.top);

      // 横向三次贝塞尔曲线
      const dx = Math.max(20, (endX - startX) * 0.55);
      const d = `M ${startX} ${startY} C ${Math.round(startX + dx)} ${startY}, ${Math.round(endX - dx)} ${endY}, ${endX} ${endY}`;

      const isRunning = runStatus[item.subId] === "running";
      const targetId = item.sub?.id || item.subId;
      const isActive = activeCollaboratorId === targetId;

      return { d, isRunning, isActive };
    });

    setSvgPaths((prev) => {
      if (prev.length === paths.length) {
        const isSame = prev.every(
          (p, i) =>
            p.d === paths[i].d &&
            p.isRunning === paths[i].isRunning &&
            p.isActive === paths[i].isActive
        );
        if (isSame) return prev;
      }
      return paths;
    });
  }, [items, activeCollaboratorId, runStatus]);

  useLayoutEffect(() => {
    updateLines();
    const timer = setTimeout(updateLines, 80);
    return () => clearTimeout(timer);
  }, [updateLines]);

  useEffect(() => {
    let animId: number;
    const scheduleUpdate = () => {
      cancelAnimationFrame(animId);
      animId = requestAnimationFrame(updateLines);
    };
    window.addEventListener("resize", scheduleUpdate);
    const observer = new ResizeObserver(scheduleUpdate);
    if (containerRef.current) {
      observer.observe(containerRef.current);
    }
    return () => {
      cancelAnimationFrame(animId);
      window.removeEventListener("resize", scheduleUpdate);
      observer.disconnect();
    };
  }, [updateLines]);

  if (items.length === 0) return null;

  const anyRunning = items.some((it) => runStatus[it.subId] === "running");

  const allCompleted = items.every((it) => {
    return runStatus[it.subId] !== "running" && it.sub?.status !== "failed";
  });

  return (
    <div className="rounded-2xl border border-edge/80 bg-panel2/40 overflow-hidden my-2.5 p-3.5 shadow-xs transition-all">
      {/* 头部摘要信息 */}
      <div className="flex items-center justify-between pb-3 mb-3 border-b border-edge/40">
        <div className="flex items-center gap-2">
          <div className="flex items-center gap-1.5 px-2 py-0.5 rounded-md bg-indigo-500/15 text-indigo-400 border border-indigo-500/30 text-[11px] font-medium">
            <Cpu size={12} />
            <span>{items.length > 1 ? "并行子任务群" : "派生子进程"}</span>
          </div>
          <span className="text-[12px] text-inkdim">
            从主进程派生 {items.length} 个独立执行单元
          </span>
        </div>

        <div className="flex items-center gap-2">
          {anyRunning ? (
            <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-[10px] font-medium bg-blue-500/15 text-blue-400 border border-blue-500/30 animate-pulse">
              <Loader2 size={10} className="animate-spin" />
              <span>并发执行中</span>
            </span>
          ) : allCompleted ? (
            <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-[10px] font-medium bg-emerald-500/15 text-emerald-400 border border-emerald-500/30">
              <CheckCircle2 size={10} />
              <span>全部就绪</span>
            </span>
          ) : null}
        </div>
      </div>

      {/* 横向树状拓扑图容器 */}
      <div ref={containerRef} className="relative flex items-center gap-6 sm:gap-10 py-1">
        {/* SVG 背景连线层 */}
        <svg className="absolute inset-0 w-full h-full pointer-events-none z-0 overflow-visible">
          <defs>
            <linearGradient id="tree-active-line" x1="0%" y1="0%" x2="100%" y2="0%">
              <stop offset="0%" stopColor="#6366f1" stopOpacity="0.8" />
              <stop offset="100%" stopColor="#3b82f6" stopOpacity="1" />
            </linearGradient>
            <linearGradient id="tree-normal-line" x1="0%" y1="0%" x2="100%" y2="0%">
              <stop offset="0%" stopColor="#71717a" stopOpacity="0.3" />
              <stop offset="100%" stopColor="#71717a" stopOpacity="0.6" />
            </linearGradient>
          </defs>

          {svgPaths.map((p, i) => {
            if (!p.d) return null;
            return (
              <g key={i}>
                {/* 连线底轨 */}
                <path
                  d={p.d}
                  fill="none"
                  stroke={p.isActive ? "url(#tree-active-line)" : p.isRunning ? "#3b82f6" : "url(#tree-normal-line)"}
                  strokeWidth={p.isActive ? 2.2 : p.isRunning ? 2 : 1.4}
                  strokeDasharray={p.isRunning ? "4,4" : "none"}
                  className={p.isRunning ? "animate-[dash_1.2s_linear_infinite]" : ""}
                />
              </g>
            );
          })}
        </svg>

        {/* 左侧：主进程根节点 */}
        <div
          ref={rootNodeRef}
          className={`relative z-10 shrink-0 w-28 sm:w-32 rounded-xl p-3 flex flex-col items-center justify-center text-center border transition-all ${
            anyRunning
              ? "bg-panel2/95 border-accent/60 shadow-[0_0_15px_rgba(59,130,246,0.15)] ring-1 ring-accent/30"
              : "bg-panel2/90 border-edge shadow-xs"
          }`}
        >
          <div className="w-8 h-8 rounded-lg bg-indigo-500/15 text-indigo-400 border border-indigo-500/30 flex items-center justify-center mb-1.5">
            <Bot size={18} />
          </div>
          <span className="text-[12px] font-semibold text-ink">主进程</span>
          <span className="text-[10px] text-inkdim scale-95 mt-0.5">
            {anyRunning ? "协调分派中" : "派生调度完成"}
          </span>

          {/* 出线锚点圆点 */}
          <span
            data-anchor="root-out"
            className={`absolute -right-[5px] top-1/2 -translate-y-1/2 w-2.5 h-2.5 rounded-full border transition-colors ${
              anyRunning ? "bg-accent border-white shadow-xs" : "bg-edge border-inkdim/50"
            }`}
          />
        </div>

        {/* 右侧：子任务卡片列表（纵向排布，形成 1 -> N 横向树形分支） */}
        <div className="relative z-10 flex-1 flex flex-col gap-2.5 min-w-0">
          {items.map((item, idx) => {
            const targetId = item.sub?.id || item.subId;
            const isActive = !!targetId && activeCollaboratorId === targetId;
            return (
              <SubprocessTaskTab
                key={item.subId || idx}
                item={item}
                isActive={isActive}
                onToggleActive={async () => {
                  let targetId = item.sub?.id || (item.subId !== item.ev?.id ? item.subId : null);
                  if (!targetId && currentParentId) {
                    try {
                      const [spList, saList] = await Promise.all([
                        ipc.listSubprocesses(currentParentId),
                        ipc.listSubagents(currentParentId),
                      ]);
                      useStore.setState((st) => ({
                        subprocesses: { ...st.subprocesses, [currentParentId]: spList },
                        subagents: { ...st.subagents, [currentParentId]: saList },
                      }));
                      const all = [...spList, ...saList];
                      const matched = all.find(
                        (s) =>
                          (item.ev?.id && s.triggerToolEventId === item.ev.id) ||
                          (item.title && s.title === item.title) ||
                          (item.task && s.subagentTask === item.task)
                      );
                      if (matched) {
                        targetId = matched.id;
                      }
                    } catch (e) {
                      console.error("fetch latest subprocesses error", e);
                    }
                  }

                  if (!targetId) {
                    if (item.ev?.status === "pending_approval") {
                      pushToast("该子任务正在等待审批，请先点击“批准”按钮启动");
                    } else if (item.ev?.status === "failed") {
                      pushToast("该子任务未成功启动（任务已终止）");
                    } else {
                      pushToast("子进程正在启动初始化，请稍候…");
                    }
                    return;
                  }

                  if (activeCollaboratorId === targetId) {
                    setActiveCollaboratorId(null);
                  } else {
                    setActiveCollaboratorId(targetId);
                  }
                }}
                onMountRef={(el) => {
                  taskRefs.current[idx] = el;
                }}
              />
            );
          })}
        </div>
      </div>

      {/* 底部提示 */}
      <div className="mt-3 pt-2 border-t border-edge/30 text-[11px] text-inkdim flex items-center justify-between">
        <span className="flex items-center gap-1 opacity-75">
          <Sparkles size={11} className="text-accent" />
          <span>点击上方任一子任务，可在右侧独立分屏中查阅其完整对话流程与工具卡片</span>
        </span>
        <span className="text-[10px] opacity-60">
          {items.length} 个任务分支
        </span>
      </div>
    </div>
  );
}
