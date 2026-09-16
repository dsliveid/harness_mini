import { useState } from "react";
import { currentMessages, useStore } from "../store";
import type { TodoItem, ToolEvent } from "../types";

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
        <span>📋 任务清单</span>
        <span>{done}/{total}</span>
        <div className="flex-1 h-1 bg-panel3 rounded-full overflow-hidden">
          <div className="h-full bg-green-500 rounded-full transition-all" style={{ width: `${pct}%` }} />
        </div>
      </div>
      <div className="flex flex-col gap-0.5 max-h-[180px] overflow-y-auto">
        {todos.map((t, i) => (
          <div key={i} className="flex items-start gap-1.5 text-[12px] leading-snug">
            <span className="shrink-0 mt-px">
              {t.status === "done" ? "✅" : t.status === "in_progress" ? "🔄" : "⬜"}
            </span>
            <span className={t.status === "done" ? "text-inkdim line-through" : ""}>
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

  return (
    <div className="flex flex-col gap-1">
      <div className="flex items-center gap-1.5">
        <span className="w-1.5 h-1.5 rounded-full bg-green-400 animate-pulse shrink-0" />
        <span className="text-[11px] text-inkdim truncate flex-1 min-w-0 font-mono" title={command}>
          {command.length > 40 ? command.slice(0, 40) + "…" : command}
        </span>
        <button
          className="shrink-0 text-[10px] px-1.5 py-0.5 rounded bg-red-600/80 hover:bg-red-500 text-white"
          title="终止进程"
          onClick={() => killCommand(ev.id)}
        >
          终止
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

  // 收起态：紧凑胶囊
  if (collapsed) {
    const doneCnt = todos.filter((t) => t.status === "done").length;
    return (
      <div className="sticky top-3 z-20 float-right mr-4 mt-3 clear-both">
        <button
          className="flex items-center gap-1.5 px-2.5 py-1.5 rounded-lg bg-panel2/95 backdrop-blur border border-edge shadow-lg text-[11px] text-inkdim hover:text-ink hover:border-accent/40 transition-colors"
          onClick={() => setCollapsed(false)}
          title="展开任务面板"
        >
          {hasCmds && <span className="w-1.5 h-1.5 rounded-full bg-green-400 animate-pulse" />}
          {hasTodos && <span>📋 {doneCnt}/{todos.length}</span>}
          {hasCmds && !hasTodos && <span>⚡ {runningCmds.length} 进程</span>}
          <span className="text-[10px]">▼</span>
        </button>
      </div>
    );
  }

  // 展开态
  return (
    <div className="sticky top-3 z-20 float-right mr-4 mt-3 clear-both w-[280px] rounded-xl bg-panel2/95 backdrop-blur border border-edge shadow-xl">
      {/* 标题栏 */}
      <div className="flex items-center justify-between px-3 py-2 border-b border-edge">
        <span className="text-[11px] text-inkdim font-medium">
          {hasTodos && hasCmds ? "任务 & 进程" : hasTodos ? "任务清单" : "执行中的进程"}
        </span>
        <button
          className="text-[10px] text-inkdim hover:text-ink px-1.5 py-0.5 rounded hover:bg-panel3"
          onClick={() => setCollapsed(true)}
          title="收起"
        >
          ▲ 收起
        </button>
      </div>

      <div className="px-3 py-2 flex flex-col gap-2.5">
        {/* 运行中的控制台进程 */}
        {hasCmds && (
          <div className="flex flex-col gap-1.5">
            {runningCmds.length > 0 && hasTodos && (
              <div className="text-[10px] text-inkdim font-medium">⚡ 控制台进程</div>
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
