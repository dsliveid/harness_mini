import { useState } from "react";
import { ipc } from "../ipc";
import { useStore } from "../store";
import type { ToolEvent } from "../types";
import { Markdown } from "./Markdown";

function statusBadge(status: string) {
  switch (status) {
    case "pending_approval":
      return <span className="text-[11px] px-1.5 py-0.5 rounded bg-amber-500/15 text-amber-400">待审批</span>;
    case "running":
      return <span className="text-[11px] px-1.5 py-0.5 rounded bg-blue-500/15 text-blue-400 animate-pulse">执行中</span>;
    case "success":
      return <span className="text-[11px] px-1.5 py-0.5 rounded bg-green-500/15 text-green-400">完成</span>;
    case "failed":
      return <span className="text-[11px] px-1.5 py-0.5 rounded bg-red-500/15 text-red-400">失败</span>;
    case "denied":
      return <span className="text-[11px] px-1.5 py-0.5 rounded bg-zinc-500/20 text-zinc-400">已拒绝</span>;
    case "timeout":
      return <span className="text-[11px] px-1.5 py-0.5 rounded bg-red-500/15 text-red-400">超时</span>;
    default:
      return <span className="text-[11px] px-1.5 py-0.5 rounded bg-panel3 text-inkdim">{status}</span>;
  }
}

function paramTitle(ev: ToolEvent): string {
  const p = ev.params ?? {};
  switch (ev.toolName) {
    case "run_command":
      return String(p.command ?? "");
    case "glob":
    case "grep":
      return `${p.pattern ?? ""}${p.path ? `（${p.path}）` : ""}`;
    case "todo":
      return "任务清单";
    default:
      return String(p.path ?? "");
  }
}

const TOOL_LABELS: Record<string, string> = {
  read_file: "读取文件",
  list_dir: "列出目录",
  glob: "查找文件",
  grep: "搜索内容",
  write_file: "写入文件",
  edit_file: "编辑文件",
  run_command: "执行命令",
  todo: "任务清单",
  temp_status: "临时空间状态",
  temp_changes: "临时空间变更",
  temp_diff: "临时空间 diff",
  temp_snapshot: "保存备份快照",
  temp_restore: "临时空间恢复",
  temp_merge: "合并到原目录",
};

function TodoList({ todos }: { todos: any[] }) {
  return (
    <div className="flex flex-col gap-1 py-1">
      {todos.map((t, i) => (
        <div key={i} className="flex items-center gap-2 text-[13px]">
          <span>{t.status === "done" ? "✅" : t.status === "in_progress" ? "🔄" : "⬜"}</span>
          <span className={t.status === "done" ? "text-inkdim line-through" : ""}>{t.content}</span>
        </div>
      ))}
    </div>
  );
}

function ApprovalSection({ ev }: { ev: ToolEvent }) {
  const req = useStore((s) => s.approvals[ev.id]);
  const approvalDone = useStore((s) => s.approvalDone);
  const pushToast = useStore((s) => s.pushToast);
  const [denyOpen, setDenyOpen] = useState(false);
  const [reason, setReason] = useState("");

  if (!req) return null;
  const riskLabel =
    req.toolName === "temp_merge"
      ? "把临时空间变更合并写回你的原始目录"
      : req.toolName === "temp_restore"
        ? "恢复临时空间（丢弃之后的修改）"
        : req.risk === "execute"
          ? "执行命令"
          : req.risk === "path"
            ? "访问工作区之外的路径"
            : "写入文件";

  const respond = async (decision: string, r?: string) => {
    approvalDone(ev.id);
    try {
      await ipc.respondApproval(ev.id, decision, r);
    } catch (e) {
      pushToast(String(e));
    }
  };

  return (
    <div className="mt-2 border border-amber-500/40 bg-amber-500/5 rounded-lg p-3">
      <div className="text-[13px] font-medium text-amber-400 mb-1">需要你的确认：{riskLabel}</div>
      <pre className="text-[12px] font-mono whitespace-pre-wrap bg-panel rounded-md p-2 max-h-48 overflow-y-auto text-ink">
        {req.preview}
      </pre>
      {req.forceOnce && (
        <div className="text-[11px] text-amber-500/80 mt-1">检测到高危命令，仅允许逐次确认，不可记忆放行。</div>
      )}
      <div className="flex items-center gap-2 mt-2 flex-wrap">
        <button
          className="px-3 py-1.5 rounded-lg bg-green-600/90 hover:bg-green-500 text-white text-[13px]"
          onClick={() => respond("allow_once")}
        >
          允许一次
        </button>
        {!req.forceOnce && (
          <button
            className="px-3 py-1.5 rounded-lg bg-panel3 hover:bg-edge text-ink text-[13px]"
            title="记住该规则：仅对当前对话生效，重启后仍保留；可在会话设置中删除"
            onClick={() => respond("allow_session")}
          >
            本会话允许
          </button>
        )}
        <button
          className="px-3 py-1.5 rounded-lg bg-panel3 hover:bg-edge text-red-400 text-[13px]"
          onClick={() => setDenyOpen(!denyOpen)}
        >
          拒绝
        </button>
      </div>
      {denyOpen && (
        <div className="flex gap-2 mt-2">
          <input
            className="flex-1 bg-panel border border-edge rounded-lg px-2 py-1.5 text-[13px] outline-none focus:border-accent"
            placeholder="拒绝原因（可选，将告知 Agent 以便调整方案）"
            value={reason}
            onChange={(e) => setReason(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") respond("deny", reason || undefined);
            }}
          />
          <button
            className="px-3 py-1.5 rounded-lg bg-red-600/80 hover:bg-red-500 text-white text-[13px]"
            onClick={() => respond("deny", reason || undefined)}
          >
            确认拒绝
          </button>
        </div>
      )}
    </div>
  );
}

function CollapsibleResult({ text }: { text: string }) {
  const lines = text.split("\n").length;
  const [expanded, setExpanded] = useState(lines <= 20);
  return (
    <div className="mt-1">
      <div className={`overflow-hidden ${expanded ? "" : "max-h-[240px]"}`}>
        <div className={expanded ? "" : "relative"}>
          <Markdown content={text} />
          {!expanded && <div className="absolute bottom-0 left-0 right-0 h-16 bg-gradient-to-t from-panel to-transparent" />}
        </div>
      </div>
      {lines > 20 && (
        <button className="text-[12px] text-accent mt-1" onClick={() => setExpanded(!expanded)}>
          {expanded ? "收起" : "展开全部"}
        </button>
      )}
    </div>
  );
}

export function ToolCard({ ev }: { ev: ToolEvent }) {
  const output = useStore((s) => s.toolOutputs[ev.id]);
  const title = paramTitle(ev);
  const label = TOOL_LABELS[ev.toolName] ?? ev.toolName;
  // 默认收起为一行（标题 + 状态），点击展开查看命令输出/文件内容
  const [expanded, setExpanded] = useState(false);

  const showOutput = ev.status === "running" && output !== undefined;
  const showResult = !!ev.resultText && (ev.status === "success" || ev.status === "failed" || ev.status === "timeout" || ev.status === "denied");

  return (
    <div
      className={`border rounded-xl bg-panel2 px-3 py-2.5 ${
        ev.status === "pending_approval" ? "border-amber-500/40" : "border-edge"
      }`}
    >
      <button
        className="w-full flex items-center gap-2 text-left"
        onClick={() => setExpanded(!expanded)}
        title={expanded ? "收起" : "展开"}
      >
        <span className="text-[11px] text-inkdim w-3 shrink-0">{expanded ? "▾" : "▸"}</span>
        <span className="text-[12px] text-inkdim shrink-0">🛠 {label}</span>
        {ev.toolName !== "todo" && (
          <span className="text-[13px] font-mono truncate flex-1 min-w-0">{title}</span>
        )}
        {statusBadge(ev.status)}
        {ev.approvalScope && ev.approvalScope !== "none" && ev.status !== "pending_approval" && (
          <span className="text-[10px] text-inkdim shrink-0">
            ({ev.approvalScope === "mode" ? "完全访问" : ev.approvalScope === "once" ? "一次放行" : "会话规则"})
          </span>
        )}
      </button>

      {ev.toolName === "todo" && Array.isArray(ev.params?.todos) && <TodoList todos={ev.params.todos} />}

      {/* 审批条是交互入口，保持常显 */}
      {ev.status === "pending_approval" && <ApprovalSection ev={ev} />}

      {expanded && showOutput && (
        <pre className="mt-2 text-[12px] font-mono whitespace-pre-wrap bg-[#111114] rounded-md p-2 max-h-40 overflow-y-auto text-inkdim">
          {output.split("\n").slice(-200).join("\n")}
        </pre>
      )}

      {expanded && showResult && <CollapsibleResult text={ev.resultText!} />}
    </div>
  );
}
