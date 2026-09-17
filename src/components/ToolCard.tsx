import { useState } from "react";
import { ipc } from "../ipc";
import { useStore } from "../store";
import type { ToolEvent } from "../types";
import { Markdown } from "./Markdown";
import {
  FileText,
  FileEdit,
  Terminal,
  Folder,
  Search,
  CheckSquare,
  GitCompare,
  GitMerge,
  Wrench,
  CheckCircle2,
  Loader2,
  Circle,
  ChevronRight,
  ShieldAlert,
  Zap,
  Copy,
  Check,
} from "./Icons";

function statusBadge(status: string) {
  switch (status) {
    case "pending_approval":
      return <span className="text-[11px] px-2 py-0.5 rounded-full bg-amber-500/15 text-amber-400 border border-amber-500/30 font-medium">待审批</span>;
    case "running":
      return <span className="text-[11px] px-2 py-0.5 rounded-full bg-blue-500/15 text-blue-400 border border-blue-500/30 animate-pulse font-medium">执行中</span>;
    case "success":
      return <span className="text-[11px] px-2 py-0.5 rounded-full bg-green-500/15 text-green-400 border border-green-500/30">完成</span>;
    case "failed":
      return <span className="text-[11px] px-2 py-0.5 rounded-full bg-red-500/15 text-red-400 border border-red-500/30">失败</span>;
    case "denied":
      return <span className="text-[11px] px-2 py-0.5 rounded-full bg-zinc-500/20 text-zinc-400 border border-zinc-500/30">已拒绝</span>;
    case "timeout":
      return <span className="text-[11px] px-2 py-0.5 rounded-full bg-red-500/15 text-red-400 border border-red-500/30">超时</span>;
    default:
      return <span className="text-[11px] px-2 py-0.5 rounded-full bg-panel3 text-inkdim">{status}</span>;
  }
}

function toolCategoryBadge(name: string) {
  if (name.startsWith("temp_")) {
    return (
      <span className="text-[10px] px-1.5 py-[0.5px] rounded bg-amber-500/10 text-amber-400 border border-amber-500/20 shrink-0">
        临时空间
      </span>
    );
  }
  if (["list_skills", "save_skill", "run_skill"].includes(name)) {
    return (
      <span className="text-[10px] px-1.5 py-[0.5px] rounded bg-purple-500/10 text-purple-400 border border-purple-500/20 shrink-0">
        自演化
      </span>
    );
  }
  return (
    <span className="text-[10px] px-1.5 py-[0.5px] rounded bg-blue-500/10 text-blue-400 border border-blue-500/20 shrink-0">
      内置
    </span>
  );
}

function ToolIcon({ name }: { name: string }) {
  switch (name) {
    case "read_file":
      return <FileText size={14} className="text-blue-400 shrink-0" />;
    case "write_file":
    case "edit_file":
      return <FileEdit size={14} className="text-amber-400 shrink-0" />;
    case "run_command":
      return <Terminal size={14} className="text-emerald-400 shrink-0" />;
    case "list_dir":
    case "glob":
      return <Folder size={14} className="text-cyan-400 shrink-0" />;
    case "grep":
      return <Search size={14} className="text-indigo-400 shrink-0" />;
    case "todo":
      return <CheckSquare size={14} className="text-purple-400 shrink-0" />;
    case "list_skills":
    case "save_skill":
    case "run_skill":
      return <Zap size={14} className="text-purple-400 shrink-0" />;
    case "temp_status":
    case "temp_changes":
    case "temp_diff":
      return <GitCompare size={14} className="text-amber-400 shrink-0" />;
    case "temp_merge":
      return <GitMerge size={14} className="text-green-400 shrink-0" />;
    default:
      return <Wrench size={14} className="text-inkdim shrink-0" />;
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
    case "run_skill":
    case "save_skill":
      return String(p.name ?? "");
    case "list_skills":
      return "技能工具库";
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
  list_skills: "查询技能库",
  save_skill: "固化项目技能",
  run_skill: "执行项目技能",
  temp_status: "临时空间状态",
  temp_changes: "临时空间变更",
  temp_diff: "临时空间 diff",
  temp_snapshot: "保存备份快照",
  temp_restore: "临时空间恢复",
  temp_merge: "合并到原目录",
};

function TodoList({ todos, isRunning }: { todos: any[]; isRunning: boolean }) {
  return (
    <div className="flex flex-col gap-1.5 py-1.5 pl-5">
      {todos.map((t, i) => {
        const isDone = t.status === "done" || (!isRunning && t.status === "in_progress");
        const inProgress = t.status === "in_progress" && isRunning;
        return (
          <div key={i} className="flex items-center gap-2 text-[13px]">
            {isDone ? (
              <CheckCircle2 size={14} className="text-green-400 shrink-0" />
            ) : inProgress ? (
              <Loader2 size={14} className="text-blue-400 animate-spin shrink-0" />
            ) : (
              <Circle size={14} className="text-inkdim/60 shrink-0" />
            )}
            <span className={isDone ? "text-inkdim line-through" : "text-ink"}>{t.content}</span>
          </div>
        );
      })}
    </div>
  );
}

function ApprovalSection({ ev }: { ev: ToolEvent }) {
  const req = useStore((s) => s.approvals[ev.id]);
  const approvalDone = useStore((s) => s.approvalDone);
  const pushToast = useStore((s) => s.pushToast);
  const [denyOpen, setDenyOpen] = useState(false);
  const [reason, setReason] = useState("");
  const [copied, setCopied] = useState(false);

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

  const handleCopy = () => {
    const textToCopy = ev.params?.command ? String(ev.params.command) : req.preview.replace(/^\$\s*/, "");
    navigator.clipboard.writeText(textToCopy).then(
      () => {
        setCopied(true);
        setTimeout(() => setCopied(false), 1500);
        pushToast("已复制到剪贴板");
      },
      () => pushToast("复制失败")
    );
  };

  return (
    <div className="mt-2.5 border border-amber-500/40 bg-amber-500/5 rounded-xl p-3 shadow-sm animate-in fade-in duration-150">
      <div className="text-[13px] font-medium text-amber-400 mb-1.5 flex items-center justify-between gap-1.5">
        <div className="flex items-center gap-1.5">
          <ShieldAlert size={16} className="shrink-0 text-amber-400" />
          <span>需要你的确认：{riskLabel}</span>
        </div>
        <button
          type="button"
          className="inline-flex items-center gap-1 text-[11px] px-2 py-0.5 rounded bg-amber-500/10 hover:bg-amber-500/20 text-amber-300 transition-colors"
          onClick={handleCopy}
          title="复制命令内容"
        >
          {copied ? <Check size={12} className="text-green-400" /> : <Copy size={12} />}
          <span>{copied ? "已复制" : "复制"}</span>
        </button>
      </div>
      <pre className="text-[12px] font-mono whitespace-pre-wrap break-all bg-panel border border-edge/60 rounded-lg p-2.5 max-h-48 overflow-y-auto text-ink select-text">
        {req.preview}
      </pre>
      {req.forceOnce && (
        <div className="text-[11px] text-amber-500/80 mt-1">检测到高危命令，仅允许逐次确认，不可记忆放行。</div>
      )}
      <div className="flex items-center gap-2 mt-2 flex-wrap">
        <button
          className="px-3 py-1.5 rounded-lg bg-green-600/90 hover:bg-green-500 text-white text-[13px] transition-colors shadow-sm font-medium"
          onClick={() => respond("allow_once")}
        >
          允许一次
        </button>
        {!req.forceOnce && (
          <button
            className="px-3 py-1.5 rounded-lg bg-panel3 hover:bg-edge text-ink text-[13px] transition-colors"
            title="记住该规则：仅对当前对话生效，重启后仍保留；可在会话设置中删除"
            onClick={() => respond("allow_session")}
          >
            本会话允许
          </button>
        )}
        <button
          className="px-3 py-1.5 rounded-lg bg-panel3 hover:bg-edge text-red-400 text-[13px] transition-colors"
          onClick={() => setDenyOpen(!denyOpen)}
        >
          拒绝
        </button>
      </div>
      {denyOpen && (
        <div className="flex gap-2 mt-2">
          <input
            className="flex-1 bg-panel border border-edge rounded-lg px-2.5 py-1.5 text-[13px] outline-none focus:border-accent"
            placeholder="拒绝原因（可选，将告知 Agent 以便调整方案）"
            value={reason}
            onChange={(e) => setReason(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") respond("deny", reason || undefined);
            }}
          />
          <button
            className="px-3 py-1.5 rounded-lg bg-red-600/80 hover:bg-red-500 text-white text-[13px] transition-colors shadow-sm"
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
        <button className="text-[12px] text-accent mt-1 hover:underline" onClick={() => setExpanded(!expanded)}>
          {expanded ? "收起" : "展开全部"}
        </button>
      )}
    </div>
  );
}

export function ToolCard({ ev }: { ev: ToolEvent }) {
  const output = useStore((s) => s.toolOutputs[ev.id]);
  const killCommand = useStore((s) => s.killCommand);
  const pushToast = useStore((s) => s.pushToast);
  const isRunning = useStore((s) => (s.currentId ? s.runStatus[s.currentId] === "running" : false));
  const [terminating, setTerminating] = useState(false);
  const [copied, setCopied] = useState(false);
  const title = paramTitle(ev);
  const label = TOOL_LABELS[ev.toolName] ?? ev.toolName;
  const isCommandTool = ev.toolName === "run_command";
  const commandText = isCommandTool ? String(ev.params?.command ?? "") : "";
  // 默认收起为一行（标题 + 状态），点击展开查看命令输出/文件内容
  const [expanded, setExpanded] = useState(false);

  const handleCopyCommand = (e: React.MouseEvent) => {
    e.stopPropagation();
    const textToCopy = commandText || title;
    if (!textToCopy) return;
    navigator.clipboard.writeText(textToCopy).then(
      () => {
        setCopied(true);
        setTimeout(() => setCopied(false), 1500);
        pushToast("命令行已复制到剪贴板");
      },
      () => pushToast("复制失败")
    );
  };

  const handleKill = async (e: React.MouseEvent) => {
    e.stopPropagation();
    if (terminating) return;
    setTerminating(true);
    try {
      await killCommand(ev.id);
    } catch {
      setTerminating(false);
    }
  };

  const showOutput = ev.status === "running" && output !== undefined;
  const showResult = !!ev.resultText && (ev.status === "success" || ev.status === "failed" || ev.status === "timeout" || ev.status === "denied");

  return (
    <div
      className={`border rounded-xl bg-panel2/80 hover:bg-panel2 px-3 py-2.5 transition-colors shadow-sm ${
        ev.status === "pending_approval" ? "border-amber-500/50 ring-1 ring-amber-500/20" : "border-edge/80"
      }`}
    >
      <div
        role="button"
        tabIndex={0}
        className="w-full flex items-center gap-2 text-left select-none cursor-pointer"
        onClick={() => setExpanded(!expanded)}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            setExpanded(!expanded);
          }
        }}
        title={expanded ? "收起" : "展开"}
      >
        <ChevronRight
          size={13}
          className={`transition-transform duration-150 text-inkdim shrink-0 ${expanded ? "rotate-90" : ""}`}
        />
        <ToolIcon name={ev.toolName} />
        <div className="flex items-center gap-1.5 shrink-0">
          <span className="text-[12px] text-ink font-medium">{label}</span>
          {toolCategoryBadge(ev.toolName)}
        </div>
        {ev.toolName !== "todo" && (
          <span
            className="text-[12px] font-mono text-inkdim truncate flex-1 min-w-0"
            title={title}
          >
            {title}
          </span>
        )}
        <div className="ml-auto flex items-center gap-1.5 shrink-0">
          {isCommandTool && commandText && (
            <button
              type="button"
              className="p-1 rounded hover:bg-panel3 text-inkdim hover:text-ink transition-colors"
              title="复制命令行"
              onClick={handleCopyCommand}
            >
              {copied ? <Check size={12} className="text-green-400" /> : <Copy size={12} />}
            </button>
          )}
          {ev.toolName === "run_command" && ev.status === "running" && (
            <button
              type="button"
              className="text-[10px] px-2 py-0.5 rounded bg-red-600/80 hover:bg-red-500 text-white disabled:opacity-50 disabled:cursor-not-allowed transition-all"
              title={terminating ? "正在终止进程…" : "终止进程"}
              disabled={terminating}
              onClick={handleKill}
            >
              {terminating ? "终止中…" : "终止"}
            </button>
          )}
          {statusBadge(ev.status)}
          {ev.approvalScope && ev.approvalScope !== "none" && ev.status !== "pending_approval" && (
            <span className="text-[10px] text-inkdim shrink-0">
              ({ev.approvalScope === "mode" ? "完全访问" : ev.approvalScope === "once" ? "一次放行" : "会话规则"})
            </span>
          )}
        </div>
      </div>

      {ev.toolName === "todo" && Array.isArray(ev.params?.todos) && (
        <TodoList todos={ev.params.todos} isRunning={isRunning} />
      )}

      {/* 审批条是交互入口，保持常显 */}
      {ev.status === "pending_approval" && <ApprovalSection ev={ev} />}

      {/* 展开时：若是执行命令，先展示完整的命令行（带复制按钮与完整换行支持） */}
      {expanded && isCommandTool && commandText && (
        <div className="mt-2.5 rounded-lg border border-edge/60 bg-[#111114] overflow-hidden">
          <div className="flex items-center justify-between px-3 py-1.5 bg-panel3/40 border-b border-edge/40 text-[11px] text-inkdim select-none">
            <span className="flex items-center gap-1.5 font-medium text-inkdim">
              <Terminal size={12} className="text-emerald-400" />
              <span>完整命令行</span>
            </span>
            <button
              type="button"
              className="inline-flex items-center gap-1 px-2 py-0.5 rounded bg-panel3 hover:bg-edge hover:text-ink text-inkdim transition-colors text-[11px]"
              onClick={handleCopyCommand}
              title="复制命令行"
            >
              {copied ? <Check size={12} className="text-green-400" /> : <Copy size={12} />}
              <span>{copied ? "已复制" : "复制"}</span>
            </button>
          </div>
          <pre className="p-3 text-[12px] font-mono text-emerald-300/90 whitespace-pre-wrap break-all select-text max-h-48 overflow-y-auto selection:bg-emerald-500/30">
            {commandText}
          </pre>
        </div>
      )}

      {expanded && showOutput && (
        <pre className="mt-2 text-[12px] font-mono whitespace-pre-wrap bg-[#111114] rounded-md p-2 max-h-40 overflow-y-auto text-inkdim">
          {output.split("\n").slice(-200).join("\n")}
        </pre>
      )}

      {expanded && showResult && <CollapsibleResult text={ev.resultText!} />}
    </div>
  );
}
