import { useState } from "react";
import { currentMessages, useStore } from "../store";
import type { TodoItem, ToolEvent } from "../types";
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

function TodoSection({ todos }: { todos: TodoItem[] }) {
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
        {todos.map((t, i) => (
          <div key={i} className="flex items-start gap-1.5 text-[12px] leading-snug">
            <span className="shrink-0 mt-0.5">
              {t.status === "done" ? (
                <CheckCircle2 size={13} className="text-green-400" />
              ) : t.status === "in_progress" ? (
                <Loader2 size={13} className="text-blue-400 animate-spin" />
              ) : (
                <Circle size={13} className="text-inkdim/50" />
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
  const todos: TodoItem[] = useStore((s) => (s.currentId ? s.sessionTodos[s.currentId] ?? [] : []));
  const runningCmds = useRunningCommands();
  const [collapsed, setCollapsed] = useState(false);

  const hasTodos = todos.length > 0;
  const hasCmds = runningCmds.length > 0;

  if (!currentId || (!hasTodos && !hasCmds)) return null;

  // 收起态：靠右吸附抽屉标签
  if (collapsed) {
    const doneCnt = todos.filter((t) => t.status === "done").length;
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
          {hasCmds && <span className="w-2 h-2 rounded-full bg-green-400 animate-pulse shrink-0" />}
          {hasTodos && (
            <span className="flex items-center gap-1 font-medium">
              <CheckSquare size={13} className="text-purple-400 shrink-0" />
              <span className="font-mono text-[11px]">{doneCnt}/{todos.length}</span>
            </span>
          )}
          {hasCmds && !hasTodos && (
            <span className="flex items-center gap-1 text-green-400 font-mono">
              <Terminal size={13} className="shrink-0" />
              <span>{runningCmds.length} 进程</span>
            </span>
          )}
          <span className="text-[10px] text-inkdim/60 group-hover:text-inkdim/90 transition-colors">
            {hasTodos && hasCmds ? "任务·进程" : hasTodos ? "任务" : "进程"}
          </span>
        </button>
      </div>
    );
  }

  // 展开态：靠右贴边抽屉面板
  return (
    <div className="absolute top-3 right-0 z-20 w-[300px] max-w-[calc(100%-1rem)] rounded-l-2xl bg-panel2/95 backdrop-blur-md border-y border-l border-edge shadow-2xl overflow-hidden select-none animate-in fade-in slide-in-from-right-2 duration-150">
      {/* 标题栏 */}
      <div className="flex items-center justify-between px-3.5 py-2.5 border-b border-edge bg-panel/50">
        <span className="text-[11px] text-ink font-medium flex items-center gap-1.5">
          {hasTodos && hasCmds ? (
            <>
              <CheckSquare size={13} className="text-purple-400" />
              <span>任务 & 进程</span>
            </>
          ) : hasTodos ? (
            <>
              <CheckSquare size={13} className="text-purple-400" />
              <span>任务清单</span>
            </>
          ) : (
            <>
              <Terminal size={13} className="text-emerald-400" />
              <span>执行中的进程</span>
            </>
          )}
        </span>
        <button
          className="flex items-center gap-1 text-[10px] text-inkdim hover:text-ink px-2 py-0.5 rounded-md hover:bg-panel3 transition-colors cursor-pointer"
          onClick={() => setCollapsed(true)}
          title="收起至右侧"
        >
          <span>收起</span>
          <ChevronRight size={12} />
        </button>
      </div>

      <div className="px-3.5 py-2.5 flex flex-col gap-2.5 max-h-[min(70vh,480px)] overflow-y-auto">
        {/* 运行中的控制台进程 */}
        {hasCmds && (
          <div className="flex flex-col gap-1.5">
            {runningCmds.length > 0 && hasTodos && (
              <div className="text-[10px] text-inkdim font-medium flex items-center gap-1">
                <Terminal size={11} className="text-emerald-400" />
                <span>控制台进程</span>
              </div>
            )}
            {runningCmds.map((ev) => (
              <RunningCommandCard key={ev.id} ev={ev} />
            ))}
          </div>
        )}

        {/* 分隔线 */}
        {hasCmds && hasTodos && <div className="border-t border-edge" />}

        {/* 任务清单 */}
        {hasTodos && <TodoSection todos={todos} />}
      </div>
    </div>
  );
}
