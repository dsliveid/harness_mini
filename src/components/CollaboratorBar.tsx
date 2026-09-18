import { useRef } from "react";
import { useStore, currentSession } from "../store";
import { DRAFT_ID } from "../types";
import { Users, Plus, Square, RefreshCw, Loader2, CheckCircle2, AlertCircle } from "./Icons";

export function CollaboratorBar() {
  const currentId = useStore((s) => s.currentId);
  const session = useStore((s) => currentSession(s));
  const collaborators = useStore((s) => (s.currentId ? s.collaborators[s.currentId] ?? [] : []));
  const activeCollaboratorId = useStore((s) => s.activeCollaboratorId);
  const setActiveCollaboratorId = useStore((s) => s.setActiveCollaboratorId);
  const setShowCreateCollaboratorModal = useStore((s) => s.setShowCreateCollaboratorModal);
  const restartAllSubagents = useStore((s) => s.restartAllSubagents);
  const stopSubagent = useStore((s) => s.stopSubagent);
  const runStatus = useStore((s) => s.runStatus);
  const scrollRef = useRef<HTMLDivElement>(null);

  if (!currentId || currentId === DRAFT_ID || !session) {
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
        return { label: "前端", icon: "🎨" };
      case "backend":
        return { label: "后端", icon: "⚙️" };
      case "testing":
        return { label: "测试", icon: "🧪" };
      case "review":
        return { label: "审阅", icon: "🔍" };
      case "fullstack":
        return { label: "全栈", icon: "⚡" };
      default:
        return { label: "协作者", icon: "🤖" };
    }
  };

  return (
    <div className="h-10 border-b border-edge/80 bg-panel/60 backdrop-blur px-3 flex items-center gap-2 select-none shrink-0 text-[12px]">
      {/* 标题标识 */}
      <div className="flex items-center gap-1.5 text-inkdim font-medium shrink-0 pr-2 border-r border-edge/60">
        <Users size={14} className="text-accent" />
        <span className="font-medium text-ink">项目协作者</span>
        {collaborators.length > 0 && (
          <span className="px-1.5 py-0.2 text-[10px] rounded-full bg-panel2 border border-edge text-inkdim">
            {collaborators.length}
          </span>
        )}
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
              className={`group flex items-center gap-1.5 px-2.5 py-1 rounded-lg border transition-all shrink-0 max-w-[200px] text-left cursor-pointer ${
                isActive
                  ? "bg-accent/15 border-accent/60 text-ink shadow-sm ring-1 ring-accent/30"
                  : "bg-panel2/70 hover:bg-panel2 border-edge text-inkdim hover:text-ink"
              }`}
              title={`${collab.title}\n角色: ${roleInfo.label}\n模式: ${collab.autoReport ?? true ? "任务完成自动汇报" : "手动汇报"}\n状态: ${isRunning ? "运行中" : "空闲待命"}`}
            >
              <span className="text-[12px]">{roleInfo.icon}</span>
              <span className="truncate font-medium flex-1">{collab.title}</span>
              {isRunning ? (
                <Loader2 size={12} className="shrink-0 animate-spin text-accent" />
              ) : collab.status === "completed" ? (
                <CheckCircle2 size={12} className="shrink-0 text-emerald-400" />
              ) : collab.status === "failed" ? (
                <AlertCircle size={12} className="shrink-0 text-rose-400" />
              ) : (
                <span className="w-1.5 h-1.5 rounded-full bg-inkdim/40 shrink-0" />
              )}
            </button>
          );
        })}

        <button
          onClick={() => setShowCreateCollaboratorModal(true)}
          className="flex items-center gap-1 px-2.5 py-1 rounded-lg border border-dashed border-edge hover:border-accent/60 hover:bg-accent/10 hover:text-accent text-inkdim transition-all shrink-0 cursor-pointer"
          title="手动创建并配置新的常驻项目协作者"
        >
          <Plus size={13} strokeWidth={2} />
          <span>新建协作者</span>
        </button>
      </div>

      {collaborators.length > 0 && (
        <div className="flex items-center gap-1 shrink-0 pl-1 border-l border-edge/60">
          {anyRunning ? (
            <button
              onClick={handleStopAll}
              className="flex items-center gap-1 px-2 py-1 rounded-md text-amber-400 hover:bg-amber-500/10 border border-amber-500/20 transition-colors cursor-pointer"
              title="一键停止所有正在运行的协作者"
            >
              <Square size={11} fill="currentColor" />
              <span>全部停止</span>
            </button>
          ) : (
            <button
              onClick={() => void restartAllSubagents(currentId)}
              className="flex items-center gap-1 px-2 py-1 rounded-md text-inkdim hover:text-ink hover:bg-panel2 border border-edge transition-colors cursor-pointer"
              title="一键唤醒并重启协作者"
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
