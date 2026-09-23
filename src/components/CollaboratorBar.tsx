import { useRef, useMemo } from "react";
import { useStore, currentSession } from "../store";
import { DRAFT_ID, type Session } from "../types";
import {
  Users,
  Square,
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
} from "./Icons";

const EMPTY_COLLABS_BAR: Session[] = [];

export function CollaboratorBar() {
  const currentId = useStore((s) => s.currentId);
  const session = useStore((s) => currentSession(s));
  const allCollaborators = useStore((s) => s.collaborators);
  const collaborators = useMemo(
    () => (currentId && currentId !== DRAFT_ID ? allCollaborators[currentId] ?? EMPTY_COLLABS_BAR : EMPTY_COLLABS_BAR),
    [allCollaborators, currentId]
  );
  const activeCollaboratorId = useStore((s) => s.activeCollaboratorId);
  const setActiveCollaboratorId = useStore((s) => s.setActiveCollaboratorId);
  const setShowCreateCollaboratorModal = useStore((s) => s.setShowCreateCollaboratorModal);
  const stopSubagent = useStore((s) => s.stopSubagent);
  const runStatus = useStore((s) => s.runStatus);
  const scrollRef = useRef<HTMLDivElement>(null);

  if (!currentId) {
    return null;
  }

  const anyRunning = collaborators.some((c) => runStatus[c.id] === "running");

  const handleStopAll = async () => {
    for (const c of collaborators) {
      if (runStatus[c.id] === "running") {
        void stopSubagent(c.id);
      }
    }
  };

  const handleWheel = (e: React.WheelEvent<HTMLDivElement>) => {
    if (scrollRef.current) {
      if (Math.abs(e.deltaY) > Math.abs(e.deltaX)) {
        scrollRef.current.scrollLeft += e.deltaY;
      }
    }
  };

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

  return (
    <div className="h-10 border-b border-edge/80 bg-panel/60 backdrop-blur px-3 flex items-center gap-2 select-none shrink-0 text-[12px]">
      {/* 标题标识、数量统计与新增按钮 */}
      <div className="flex items-center text-inkdim font-medium shrink-0 pr-2 border-r border-edge/60">
        <Users size={14} className="text-accent shrink-0 mr-1.5" />
        <span className="font-medium text-ink shrink-0">协作者</span>
        {collaborators.length > 0 && (
          <span className="ml-1.5 px-1.5 py-0.2 text-[10px] rounded-full bg-panel2 border border-edge text-inkdim shrink-0">
            {collaborators.length}
          </span>
        )}
        <button
          type="button"
          onClick={() => setShowCreateCollaboratorModal(true)}
          className="w-5 h-5 ml-2 p-0 rounded-md hover:bg-panel2 border border-edge/80 hover:border-accent/60 text-inkdim hover:text-accent inline-flex items-center justify-center transition-colors cursor-pointer shrink-0"
          title="新建协作者"
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

      {/* 协作者水平滚动条（无滚动条，鼠标滚轮平滑滚动） */}
      <div
        ref={scrollRef}
        onWheel={handleWheel}
        className="flex items-center gap-1.5 flex-1 overflow-x-auto scrollbar-none py-1 [scrollbar-width:none] [-ms-overflow-style:none] [&::-webkit-scrollbar]:hidden"
      >
        {collaborators.map((collab) => {
          const isRunning = runStatus[collab.id] === "running";
          const isActive = activeCollaboratorId === collab.id;
          const roleInfo = getRoleBadge(collab.subagentRole);

          return (
            <button
              key={collab.id}
              onClick={() => setActiveCollaboratorId(isActive ? null : collab.id)}
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
      </div>

      {collaborators.length > 0 && anyRunning && (
        <div className="flex items-center gap-1 shrink-0 pl-1 border-l border-edge/60">
          <button
            onClick={handleStopAll}
            className="flex items-center gap-1 px-2.5 py-1 rounded-lg text-amber-400 hover:bg-amber-500/10 border border-amber-500/20 transition-colors cursor-pointer text-[11.5px]"
            title="一键停止所有正在运行的协作者"
          >
            <Square size={11} fill="currentColor" />
            <span>全部停止</span>
          </button>
        </div>
      )}
    </div>
  );
}
