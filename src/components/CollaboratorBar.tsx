import { useRef, useMemo, useState, useEffect } from "react";
import { useStore, currentSession } from "../store";
import { DRAFT_ID, type Session } from "../types";
import {
  Users,
  Loader2,
  CheckCircle2,
  AlertCircle,
  Layout,
  Server,
  ClipboardList,
  ListTodo,
  Eye,
  Palette,
  FlaskConical,
  Search,
  Layers,
  FileText,
  Bug,
  RotateCw,
  Zap,
} from "./Icons";

const EMPTY_COLLABS_BAR: Session[] = [];

const getRoleBadge = (role?: string | null) => {
  switch (role) {
    case "frontend":
      return { label: "前端", Icon: Layout };
    case "backend":
      return { label: "后端", Icon: Server };
    case "pm":
      return { label: "产品", Icon: ClipboardList };
    case "pmo":
      return { label: "项目", Icon: ListTodo };
    case "vision":
      return { label: "视觉", Icon: Eye };
    case "image_gen":
      return { label: "生图", Icon: Palette };
    case "testing":
      return { label: "测试", Icon: FlaskConical };
    case "review":
      return { label: "审阅", Icon: Search };
    case "fullstack":
      return { label: "全栈", Icon: Layers };
    default:
      return { label: "协作者", Icon: Users };
  }
};

const getSubtaskBadge = (role?: string | null) => {
  const r = (role || "").toLowerCase();
  if (r.includes("front") || r.includes("ui") || r.includes("前端")) return { label: role || "前端", Icon: Layout };
  if (r.includes("back") || r.includes("后端") || r.includes("api")) return { label: role || "后端", Icon: Server };
  if (r.includes("test") || r.includes("测试")) return { label: role || "测试", Icon: FlaskConical };
  if (r.includes("review") || r.includes("审阅") || r.includes("审查")) return { label: role || "审阅", Icon: Search };
  if (r.includes("search") || r.includes("research") || r.includes("调研") || r.includes("探查")) return { label: role || "调研", Icon: Search };
  if (r.includes("doc") || r.includes("文档")) return { label: role || "文档", Icon: FileText };
  if (r.includes("bug") || r.includes("debug") || r.includes("排查") || r.includes("修复")) return { label: role || "排查", Icon: Bug };
  if (r.includes("refactor") || r.includes("重构")) return { label: role || "重构", Icon: RotateCw };
  return { label: role || "子任务", Icon: Zap };
};

export function CollaboratorBar() {
  const currentId = useStore((s) => s.currentId);
  const session = useStore((s) => currentSession(s));

  // 常驻协作者列表
  const allCollaborators = useStore((s) => s.collaborators);
  const collaborators = useMemo(
    () => (currentId && currentId !== DRAFT_ID ? allCollaborators[currentId] ?? EMPTY_COLLABS_BAR : EMPTY_COLLABS_BAR),
    [allCollaborators, currentId]
  );

  // 派生子进程/子任务列表
  const allSubprocesses = useStore((s) => s.subprocesses);
  const allSubagents = useStore((s) => s.subagents);

  // 汇聚属于当前主会话的子任务实体（排除已作为协作者常驻的实体）
  const rawSubprocesses = useMemo(() => {
    if (!currentId || currentId === DRAFT_ID) return [];
    const procs = allSubprocesses[currentId] ?? [];
    const ags = allSubagents[currentId] ?? [];
    const collabIds = new Set(collaborators.map((c) => c.id));

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
  }, [allSubprocesses, allSubagents, collaborators, currentId]);

  const activeCollaboratorId = useStore((s) => s.activeCollaboratorId);
  const activeSubprocessId = useStore((s) => s.activeSubprocessId);
  const setActiveCollaboratorId = useStore((s) => s.setActiveCollaboratorId);
  const setActiveSubprocessId = useStore((s) => s.setActiveSubprocessId);
  const setShowCreateCollaboratorModal = useStore((s) => s.setShowCreateCollaboratorModal);
  const runStatus = useStore((s) => s.runStatus);
  const scrollRef = useRef<HTMLDivElement>(null);

  // 状态追踪与 2.5 秒优雅退场机制
  const prevStatusesRef = useRef<Map<string, string>>(new Map());
  const [exitingSubtaskIds, setExitingSubtaskIds] = useState<Set<string>>(new Set());
  const timerMapRef = useRef<Map<string, ReturnType<typeof setTimeout>>>(new Map());

  // 切换会话时重置退场状态与清理计时器
  useEffect(() => {
    timerMapRef.current.forEach((t) => clearTimeout(t));
    timerMapRef.current.clear();
    prevStatusesRef.current.clear();
    setExitingSubtaskIds(new Set());
  }, [currentId]);

  // 组件卸载时清理定时器
  useEffect(() => {
    return () => {
      timerMapRef.current.forEach((t) => clearTimeout(t));
      timerMapRef.current.clear();
    };
  }, []);

  // 监听子任务状态变化，捕捉由非完成态转变为 completed 的事件，赋予 2.5 秒成果展示缓冲期
  useEffect(() => {
    const prevMap = prevStatusesRef.current;
    for (const sub of rawSubprocesses) {
      const prevStatus = prevMap.get(sub.id);
      const curStatus = sub.status;

      // 仅当之前状态已记录且不为 completed，而现在变为 completed 时，赋予 2.5 秒退场倒计时
      if (prevStatus && prevStatus !== "completed" && curStatus === "completed") {
        if (!timerMapRef.current.has(sub.id)) {
          setExitingSubtaskIds((prev) => new Set(prev).add(sub.id));
          const t = setTimeout(() => {
            setExitingSubtaskIds((prev) => {
              const next = new Set(prev);
              next.delete(sub.id);
              return next;
            });
            timerMapRef.current.delete(sub.id);
          }, 2500);
          timerMapRef.current.set(sub.id, t);
        }
      }
      prevMap.set(sub.id, curStatus);
    }
  }, [rawSubprocesses]);

  // 筛选活跃展示的子任务：未完成的，或者处于 2.5 秒成果展示过渡期的
  const activeSubprocesses = useMemo(() => {
    return rawSubprocesses.filter((sub) => {
      if (sub.status !== "completed") return true;
      return exitingSubtaskIds.has(sub.id);
    });
  }, [rawSubprocesses, exitingSubtaskIds]);

  if (!currentId) {
    return null;
  }

  const handleWheel = (e: React.WheelEvent<HTMLDivElement>) => {
    if (scrollRef.current) {
      if (Math.abs(e.deltaY) > Math.abs(e.deltaX)) {
        scrollRef.current.scrollLeft += e.deltaY;
      }
    }
  };

  return (
    <div className="h-10 border-b border-edge/80 bg-panel/60 backdrop-blur px-3 flex items-center gap-2 select-none shrink-0 text-[12px]">
      {/* 标题标识、数量统计与新增按钮 */}
      <div className="flex items-center text-inkdim font-medium shrink-0 pr-2 border-r border-edge/60">
        <Users size={14} className="text-accent shrink-0 mr-1.5" />
        <span className="font-medium text-ink shrink-0">协作者</span>
        {collaborators.length > 0 && (
          <span
            className="ml-1.5 px-1.5 py-0.2 text-[10px] rounded-full bg-panel2 border border-edge text-inkdim shrink-0"
            title={`当前共 ${collaborators.length} 位常驻协作者`}
          >
            {collaborators.length}
          </span>
        )}
        {activeSubprocesses.length > 0 && (
          <span
            className="ml-1.5 px-1.5 py-0.2 text-[10px] rounded-full bg-indigo-500/10 text-indigo-400 border border-indigo-500/30 shrink-0 flex items-center gap-1 font-mono"
            title={`当前共 ${activeSubprocesses.length} 个活跃子任务`}
          >
            <Zap size={9} />
            <span>{activeSubprocesses.length}</span>
          </span>
        )}
        <button
          type="button"
          onClick={() => setShowCreateCollaboratorModal(true)}
          className="w-5 h-5 ml-2 p-0 rounded-md hover:bg-panel2 border border-edge/80 hover:border-accent/60 text-inkdim hover:text-accent inline-flex items-center justify-center transition-colors cursor-pointer shrink-0"
          title="新建常驻协作者"
        >
          <svg
            width="12"
            height="12"
            viewBox="0 0 12 12"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.8"
            strokeLinecap="round"
            strokeLinejoin="round"
            className="block"
          >
            <line x1="6" y1="2" x2="6" y2="10" />
            <line x1="2" y1="6" x2="10" y2="6" />
          </svg>
        </button>
      </div>

      {/* 水平平滚卡片区（常驻协作者 + 活跃子任务） */}
      <div
        ref={scrollRef}
        onWheel={handleWheel}
        className="flex items-center gap-1.5 flex-1 overflow-x-auto scrollbar-none py-1 [scrollbar-width:none] [-ms-overflow-style:none] [&::-webkit-scrollbar]:hidden"
      >
        {/* 1. 常驻协作者列表 */}
        {collaborators.map((collab) => {
          const isRunning = runStatus[collab.id] === "running";
          const isActive = activeCollaboratorId === collab.id;
          const roleInfo = getRoleBadge(collab.subagentRole);

          return (
            <button
              key={collab.id}
              type="button"
              onClick={() => {
                setActiveCollaboratorId(isActive ? null : collab.id);
              }}
              className={`group flex items-center gap-1.5 px-2 py-1 rounded-lg border transition-all shrink-0 max-w-[102px] text-left cursor-pointer ${
                isActive
                  ? "bg-accent/15 border-accent/60 text-ink shadow-sm ring-1 ring-accent/30"
                  : "bg-panel2/70 hover:bg-panel2 border-edge text-inkdim hover:text-ink"
              }`}
              title={`${collab.title}\n角色: ${roleInfo.label}\n模式: ${collab.autoReport ?? true ? "任务完成自动汇报" : "手动汇报"}\n状态: ${isRunning ? "运行中" : "空闲待命"}`}
            >
              <span className="shrink-0 text-accent/80 group-hover:text-accent">
                <roleInfo.Icon size={12} />
              </span>
              <span className="truncate font-medium flex-1 text-[11.5px] max-w-[50px]">{collab.title}</span>
              {isRunning ? (
                <Loader2 size={11} className="shrink-0 animate-spin text-accent" />
              ) : collab.status === "completed" ? (
                <CheckCircle2 size={11} className="shrink-0 text-emerald-400" />
              ) : collab.status === "failed" ? (
                <AlertCircle size={11} className="shrink-0 text-rose-400" />
              ) : (
                <span className="w-1.5 h-1.5 rounded-full bg-inkdim/40 shrink-0" />
              )}
            </button>
          );
        })}

        {/* 分隔微纵线：仅在两者同时存在时展示 */}
        {collaborators.length > 0 && activeSubprocesses.length > 0 && (
          <div className="h-4 w-px bg-edge/70 shrink-0 mx-0.5" />
        )}

        {/* 2. 派生动态子任务列表（完成时 2.5s 优雅退场） */}
        {activeSubprocesses.map((sub) => {
          const isRunning = runStatus[sub.id] === "running";
          const isActive = activeSubprocessId === sub.id || activeCollaboratorId === sub.id;
          const isExiting = sub.status === "completed" && exitingSubtaskIds.has(sub.id);
          const roleInfo = getSubtaskBadge(sub.subagentRole);

          return (
            <button
              key={sub.id}
              type="button"
              onClick={() => {
                setActiveSubprocessId(isActive ? null : sub.id);
              }}
              className={`group flex items-center gap-1.5 px-2 py-1 rounded-lg border transition-all duration-300 shrink-0 max-w-[130px] text-left cursor-pointer ${
                isActive
                  ? "bg-accent/15 border-accent/60 text-ink shadow-sm ring-1 ring-accent/30"
                  : isExiting
                  ? "bg-emerald-500/10 border-emerald-500/35 text-emerald-400 shadow-xs animate-in fade-in duration-200"
                  : "bg-panel2/70 hover:bg-panel2 border-edge text-inkdim hover:text-ink"
              }`}
              title={`[⚡ 派生子任务]\n名称: ${sub.title}\n角色: ${roleInfo.label}\n状态: ${
                isRunning
                  ? "运行中"
                  : sub.status === "completed"
                  ? "已完成 (即将退出)"
                  : sub.status === "failed"
                  ? "执行异常"
                  : sub.status === "cancelled" || sub.status === "interrupted"
                  ? "已中止"
                  : "待推进"
              }`}
            >
              <span className={`shrink-0 ${isExiting ? "text-emerald-400" : "text-accent/80 group-hover:text-accent"}`}>
                <roleInfo.Icon size={12} />
              </span>
              <span className="truncate font-medium flex-1 text-[11.5px] max-w-[65px]">{sub.title}</span>
              {isRunning ? (
                <Loader2 size={11} className="shrink-0 animate-spin text-accent" />
              ) : sub.status === "completed" ? (
                <CheckCircle2 size={11} className="shrink-0 text-emerald-400 animate-in zoom-in-75 duration-200" />
              ) : sub.status === "failed" ? (
                <AlertCircle size={11} className="shrink-0 text-rose-400" />
              ) : sub.status === "cancelled" || sub.status === "interrupted" ? (
                <AlertCircle size={11} className="shrink-0 text-amber-400" />
              ) : (
                <span className="w-1.5 h-1.5 rounded-full bg-inkdim/40 shrink-0" />
              )}
            </button>
          );
        })}
      </div>
    </div>
  );
}
