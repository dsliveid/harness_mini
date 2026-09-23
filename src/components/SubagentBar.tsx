import { useStore, currentSession } from "../store";
import { DRAFT_ID } from "../types";
import {
  Layers,
  Plus,
  Square,
  RefreshCw,
  Loader2,
  CheckCircle2,
  AlertCircle,
  Layout,
  Server,
  FlaskConical,
  Search,
  Zap,
} from "./Icons";

export function SubagentBar() {
  const currentId = useStore((s) => s.currentId);
  const session = useStore((s) => currentSession(s));
  const subagents = useStore((s) => (s.currentId ? s.subagents[s.currentId] ?? [] : []));
  const activeSubagentId = useStore((s) => s.activeSubagentId);
  const setActiveSubagentId = useStore((s) => s.setActiveSubagentId);
  const setShowCreateSubagentModal = useStore((s) => s.setShowCreateSubagentModal);
  const restartAllSubagents = useStore((s) => s.restartAllSubagents);
  const stopSubagent = useStore((s) => s.stopSubagent);
  const runStatus = useStore((s) => s.runStatus);

  if (!currentId || currentId === DRAFT_ID || !session) {
    return null;
  }

  const anyRunning = subagents.some((sub) => runStatus[sub.id] === "running");

  const handleStopAll = async () => {
    for (const sub of subagents) {
      if (runStatus[sub.id] === "running") {
        void stopSubagent(sub.id);
      }
    }
  };

  const getRoleBadge = (role?: string | null) => {
    switch (role) {
      case "frontend":
        return { label: "前端", Icon: Layout };
      case "backend":
        return { label: "后端", Icon: Server };
      case "testing":
        return { label: "测试", Icon: FlaskConical };
      case "review":
        return { label: "审阅", Icon: Search };
      case "fullstack":
        return { label: "全栈", Icon: Layers };
      default:
        return { label: "子任务", Icon: Zap };
    }
  };

  return (
    <div className="h-10 border-b border-edge/80 bg-panel/60 backdrop-blur px-3 flex items-center gap-2 overflow-x-auto select-none shrink-0 scrollbar-none text-[12px]">
      <div className="flex items-center gap-1.5 text-inkdim font-medium shrink-0 pr-1 border-r border-edge/60">
        <Layers size={14} className="text-accent" />
        <span>子进程协作</span>
        {subagents.length > 0 && (
          <span className="px-1.5 py-0.2 text-[10px] rounded-full bg-panel2 border border-edge text-inkdim">
            {subagents.length}
          </span>
        )}
      </div>

      <div className="flex items-center gap-1.5 flex-1 overflow-x-auto scrollbar-none py-1">
        {subagents.map((sub) => {
          const isRunning = runStatus[sub.id] === "running";
          const isActive = activeSubagentId === sub.id;
          const roleInfo = getRoleBadge(sub.subagentRole);

          return (
            <button
              key={sub.id}
              onClick={() => setActiveSubagentId(isActive ? null : sub.id)}
              className={`group flex items-center gap-1.5 px-2 py-1 rounded-lg border transition-all shrink-0 max-w-[130px] text-left ${
                isActive
                  ? "bg-accent/15 border-accent/60 text-ink shadow-sm ring-1 ring-accent/30"
                  : "bg-panel2/70 hover:bg-panel2 border-edge text-inkdim hover:text-ink"
              }`}
              title={`${sub.title}\n角色: ${roleInfo.label}\n状态: ${isRunning ? "运行中" : sub.status}`}
            >
              <span className="shrink-0 text-accent/80 group-hover:text-accent">
                <roleInfo.Icon size={12} />
              </span>
              <span className="truncate font-medium flex-1 text-[11.5px]">{sub.title}</span>
              {isRunning ? (
                <Loader2 size={11} className="shrink-0 animate-spin text-accent" />
              ) : sub.status === "completed" ? (
                <CheckCircle2 size={11} className="shrink-0 text-emerald-400" />
              ) : sub.status === "failed" ? (
                <AlertCircle size={11} className="shrink-0 text-rose-400" />
              ) : (
                <span className="w-1.5 h-1.5 rounded-full bg-inkdim/40 shrink-0" />
              )}
            </button>
          );
        })}

        <button
          onClick={() => setShowCreateSubagentModal(true)}
          className="flex items-center gap-1 px-2.5 py-1 rounded-lg border border-dashed border-edge hover:border-accent/60 hover:bg-accent/10 hover:text-accent text-inkdim transition-all shrink-0"
          title="创建新子 Agent 协作开发进程"
        >
          <Plus size={13} strokeWidth={2} />
          <span>新建子进程</span>
        </button>
      </div>

      {subagents.length > 0 && (
        <div className="flex items-center gap-1 shrink-0 pl-1 border-l border-edge/60">
          {anyRunning ? (
            <button
              onClick={handleStopAll}
              className="flex items-center gap-1 px-2 py-1 rounded-md text-amber-400 hover:bg-amber-500/10 border border-amber-500/20 transition-colors"
              title="一键停止所有正在运行的子 Agent 进程"
            >
              <Square size={11} fill="currentColor" />
              <span>全部停止</span>
            </button>
          ) : (
            <button
              onClick={() => void restartAllSubagents(currentId)}
              className="flex items-center gap-1 px-2 py-1 rounded-md text-inkdim hover:text-ink hover:bg-panel2 border border-edge transition-colors"
              title="一键重新唤醒/继续所有子 Agent"
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
