import { useState, useMemo, useEffect } from "react";
import { ipc } from "../ipc";
import { useStore } from "../store";
import type { ToolEvent } from "../types";
import { Markdown } from "./Markdown";
import { SubprocessBranchTree } from "./SubprocessBranchTree";
import { SafeImage } from "./SafeImage";
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
  Cpu,
  Users,
  Image,
  ZoomIn,
  Link2,
  Sparkles,
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
  if (name === "generate_image") {
    return (
      <span className="text-[10px] px-1.5 py-[0.5px] rounded bg-pink-500/10 text-pink-400 border border-pink-500/20 shrink-0">
        多模态生图
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
  if (["create_plan", "update_plan", "switch_plan", "read_plan", "list_plans"].includes(name)) {
    return (
      <span className="text-[10px] px-1.5 py-[0.5px] rounded bg-emerald-500/10 text-emerald-400 border border-emerald-500/20 shrink-0">
        任务计划
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
    case "create_plan":
    case "update_plan":
    case "switch_plan":
    case "read_plan":
    case "list_plans":
      return <CheckSquare size={14} className="text-emerald-400 shrink-0" />;
    case "generate_image":
      return <Image size={14} className="text-pink-400 shrink-0" />;
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
    case "spawn_subprocess":
    case "spawn_subagent":
    case "stop_subprocess":
    case "stop_subagent":
      return <Cpu size={14} className="text-indigo-400 shrink-0" />;
    case "dispatch_collaborator":
    case "wait_collaborators":
    case "get_collaborators":
      return <Users size={14} className="text-accent shrink-0" />;
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
    case "generate_image":
      return String(p.prompt ?? "");
    case "run_skill":
    case "save_skill":
      return String(p.name ?? "");
    case "list_skills":
      return "技能工具库";
    case "create_plan":
      return String(p.title ?? "创建任务方案");
    case "update_plan":
      return `${p.status === "completed" ? "【已结案】" : ""}${p.reason ?? "更新任务计划"}`;
    case "switch_plan":
      return `激活计划 [${p.plan_id ?? ""}]`;
    case "read_plan":
      return String(p.plan_id ?? "当前活动计划");
    case "list_plans":
      return "项目计划清单";
    case "wait_subprocesses":
    case "wait_subagents":
      return "等待子进程完成汇聚";
    case "dispatch_collaborator":
      return `${p.collaborator_id ? `[${p.collaborator_id}] ` : ""}${p.task ?? ""}`;
    case "wait_collaborators":
      return "等待目标协作者汇报产出";
    case "get_collaborators":
      return "查询项目可用协作者名录";
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
  create_plan: "创建计划",
  update_plan: "更新计划",
  switch_plan: "切换计划",
  read_plan: "查阅计划",
  list_plans: "计划清单",
  generate_image: "生成图片",
  list_skills: "查询技能库",
  save_skill: "固化项目技能",
  run_skill: "执行项目技能",
  temp_status: "临时空间状态",
  temp_changes: "临时空间变更",
  temp_diff: "临时空间 diff",
  temp_snapshot: "保存备份快照",
  temp_restore: "临时空间恢复",
  temp_merge: "合并到原目录",
  spawn_subprocess: "派生子进程",
  spawn_subagent: "派生子进程",
  wait_subprocesses: "等待子进程汇聚",
  wait_subagents: "等待子进程",
  stop_subprocess: "停止子进程",
  stop_subagent: "停止子进程",
  dispatch_collaborator: "委派协作者任务",
  wait_collaborators: "等待协作者汇报",
  get_collaborators: "查询项目协作者",
};

function PlanCardView({
  ev,
  onOpenPlan,
  planFilePath,
}: {
  ev: ToolEvent;
  onOpenPlan?: (e: React.MouseEvent) => void;
  planFilePath?: string | null;
}) {
  const p = ev.params || {};
  const isCreate = ev.toolName === "create_plan";
  const isUpdate = ev.toolName === "update_plan";
  const isSwitch = ev.toolName === "switch_plan";

  return (
    <div className="mt-2 text-[12px] bg-panel3/30 border border-edge/60 rounded-lg p-2.5 space-y-1.5">
      {isCreate && (
        <>
          <div className="flex items-center gap-2">
            <span className="font-semibold text-emerald-400">{p.title || "任务计划"}</span>
            <span className="text-[10px] px-1.5 py-0.5 rounded bg-emerald-500/10 text-emerald-300 border border-emerald-500/20 font-mono">
              v1 (执行中)
            </span>
          </div>
          {p.goals && (
            <div className="text-ink text-[11.5px] leading-relaxed">
              <span className="text-inkdim font-medium">目标：</span>{p.goals}
            </div>
          )}
          {p.architecture && (
            <div className="text-ink text-[11.5px] leading-relaxed">
              <span className="text-inkdim font-medium">方案：</span>{p.architecture}
            </div>
          )}
          {Array.isArray(p.steps) && p.steps.length > 0 && (
            <div className="mt-1 pt-1.5 border-t border-edge/40 space-y-1">
              <div className="text-[11px] font-medium text-inkdim">分步实施清单 ({p.steps.length} 步)：</div>
              <div className="space-y-0.5">
                {p.steps.map((s: string, idx: number) => (
                  <div key={idx} className="flex items-start gap-1.5 text-ink text-[11.5px]">
                    <Circle size={12} className="text-inkdim/60 shrink-0 mt-0.5" />
                    <span>{s}</span>
                  </div>
                ))}
              </div>
            </div>
          )}

          {/* 方案文档落盘状态与快捷操作条 */}
          <div className="mt-2 pt-2 border-t border-edge/40 flex items-center justify-between gap-2 flex-wrap text-[11.5px]">
            <div className="flex items-center gap-1.5 text-inkdim font-mono text-[11px] truncate max-w-[260px]">
              {planFilePath ? (
                <span title={planFilePath} className="truncate text-emerald-400/90 font-mono">
                  📄 {planFilePath}
                </span>
              ) : ev.status === "running" ? (
                <span className="text-amber-400/90 animate-pulse font-sans">⏳ 方案文档生成中...</span>
              ) : (
                <span className="font-sans text-emerald-400/90">📄 方案已物理落盘</span>
              )}
            </div>

            <div className="flex items-center gap-2 ml-auto">
              <button
                type="button"
                className={`inline-flex items-center gap-1 px-2.5 py-1 rounded-md transition-colors text-[11px] cursor-pointer ${
                  ev.status === "running"
                    ? "bg-panel3 text-inkdim/50 cursor-not-allowed border border-edge/40"
                    : "bg-emerald-500/15 hover:bg-emerald-500/25 text-emerald-300 border border-emerald-500/30"
                }`}
                onClick={onOpenPlan}
                disabled={ev.status === "running"}
                title={ev.status === "running" ? "方案生成中，落盘后即可打开" : "以 Markdown 文件打开并可手动编辑调整"}
              >
                <CheckSquare size={12} />
                <span>{ev.status === "running" ? "生成中..." : "查看/编辑方案 (MD)"}</span>
              </button>
            </div>
          </div>
          <div className="text-[11px] text-inkdim/80 bg-panel/40 px-2 py-1 rounded border border-edge/30">
            💡 提示：方案已物理落盘（可点击上方按钮查看/修改）。您可直接在下方对话框中告知 Agent 确认开始执行或提出调整建议。
          </div>
        </>
      )}
      {isUpdate && (
        <>
          <div className="flex items-center gap-2">
            <span className="font-medium text-amber-400">计划动态更新</span>
            {p.status === "completed" && (
              <span className="text-[10px] px-1.5 py-0.5 rounded bg-green-500/10 text-green-400 border border-green-500/20">
                已结案完成
              </span>
            )}
            {p.status === "in_progress" && (
              <span className="text-[10px] px-1.5 py-0.5 rounded bg-blue-500/10 text-blue-400 border border-blue-500/20">
                推进中
              </span>
            )}
          </div>
          <div className="text-ink text-[11.5px]"><span className="text-inkdim font-medium">调整原因：</span>{p.reason}</div>
          {p.revision_note && (
            <div className="text-ink text-[11.5px]"><span className="text-inkdim font-medium">变更记录：</span>{p.revision_note}</div>
          )}
        </>
      )}
      {isSwitch && (
        <div className="text-ink text-[11.5px]">
          已将当前活动计划切换为：<span className="font-mono text-emerald-400">[{p.plan_id}]</span>
        </div>
      )}
    </div>
  );
}

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

function ImageToolView({
  ev,
  imagePath,
  prompt,
  metadata,
}: {
  ev: ToolEvent;
  imagePath: string | null;
  prompt: string;
  metadata: {
    imagePath?: string | null;
    model?: string | null;
    resolution?: string | null;
    fileSize?: string | null;
  } | null;
}) {
  const setLightboxImage = useStore((s) => s.setLightboxImage);
  const pushToast = useStore((s) => s.pushToast);
  const [copiedPrompt, setCopiedPrompt] = useState(false);
  const [copiedPath, setCopiedPath] = useState(false);
  const [expandedPrompt, setExpandedPrompt] = useState(false);

  const handleCopyPrompt = (e: React.MouseEvent) => {
    e.stopPropagation();
    if (!prompt) return;
    navigator.clipboard.writeText(prompt).then(
      () => {
        setCopiedPrompt(true);
        setTimeout(() => setCopiedPrompt(false), 1500);
        pushToast("提示词已复制到剪贴板");
      },
      () => pushToast("复制失败")
    );
  };

  const handleCopyPath = (e: React.MouseEvent) => {
    e.stopPropagation();
    const p = metadata?.imagePath || imagePath;
    if (!p) return;
    navigator.clipboard.writeText(p).then(
      () => {
        setCopiedPath(true);
        setTimeout(() => setCopiedPath(false), 1500);
        pushToast("图片路径已复制到剪贴板");
      },
      () => pushToast("复制失败")
    );
  };

  // 1. 执行中状态：骨架加载与提示词引导
  if (ev.status === "running") {
    return (
      <div className="mt-2.5 rounded-xl border border-pink-500/20 bg-pink-500/5 p-6 flex flex-col items-center justify-center gap-2.5 text-inkdim select-none">
        <div className="relative flex items-center justify-center">
          <Loader2 size={26} className="animate-spin text-pink-400" />
          <Sparkles size={12} className="absolute text-pink-300 animate-pulse" />
        </div>
        <div className="text-[13px] font-medium text-ink">正在绘制图像，请稍候…</div>
        {prompt && (
          <div className="text-[11.5px] text-inkdim/80 text-center max-w-md line-clamp-2 italic px-2">
            “{prompt}”
          </div>
        )}
      </div>
    );
  }

  // 2. 失败/拒绝/超时状态：展示具体报错
  if (ev.status === "failed" || ev.status === "denied" || ev.status === "timeout") {
    return (
      <div className="mt-2.5 rounded-xl border border-red-500/30 bg-red-500/10 p-3.5 text-red-300">
        <div className="font-medium text-[12.5px] flex items-center gap-1.5 mb-1.5 text-red-400">
          <ShieldAlert size={15} />
          <span>图片生成失败</span>
        </div>
        <div className="text-[12px] font-mono whitespace-pre-wrap text-red-300/90 max-h-40 overflow-y-auto selection:bg-red-500/30">
          {ev.resultText || "未知错误"}
        </div>
      </div>
    );
  }

  // 3. 成功状态但未获取到图片路径：兜底展示返回文本
  if (!imagePath) {
    return (
      <div className="mt-2.5 rounded-xl border border-edge/80 bg-panel3/30 p-3 text-[12.5px] text-ink/90">
        {ev.resultText || "图片已生成，但未获取到展示路径"}
      </div>
    );
  }

  const isLongPrompt = prompt.length > 120;

  return (
    <div className="mt-2.5 rounded-xl overflow-hidden border border-edge/80 bg-panel3/25 shadow-sm">
      {/* 图片展示画布 */}
      <div
        className="relative group cursor-zoom-in flex items-center justify-center bg-black/40 py-2.5 px-3 overflow-hidden min-h-[180px]"
        onClick={() =>
          setLightboxImage({
            src: imagePath,
            title: prompt || "生成图片",
          })
        }
      >
        <SafeImage
          src={imagePath}
          alt={prompt || "生成图片"}
          className="max-h-[380px] w-auto max-w-full rounded-lg object-contain shadow-lg group-hover:scale-[1.01] transition-transform duration-200"
        />

        {/* 悬停快捷操作栏 */}
        <div
          className="absolute top-2.5 right-2.5 opacity-0 group-hover:opacity-100 transition-opacity flex items-center gap-1.5 bg-black/75 backdrop-blur-md rounded-lg p-1 shadow-md border border-white/10"
          onClick={(e) => e.stopPropagation()}
        >
          <button
            type="button"
            className="p-1 rounded hover:bg-white/20 text-white/80 hover:text-white transition-colors cursor-pointer"
            title="放大预览"
            onClick={() =>
              setLightboxImage({
                src: imagePath,
                title: prompt || "生成图片",
              })
            }
          >
            <ZoomIn size={14} />
          </button>
          <button
            type="button"
            className="p-1 rounded hover:bg-white/20 text-white/80 hover:text-white transition-colors cursor-pointer"
            title="复制本地路径"
            onClick={handleCopyPath}
          >
            {copiedPath ? <Check size={14} className="text-emerald-400" /> : <Link2 size={14} />}
          </button>
          <button
            type="button"
            className="p-1 rounded hover:bg-white/20 text-white/80 hover:text-white transition-colors cursor-pointer"
            title="复制提示词"
            onClick={handleCopyPrompt}
          >
            {copiedPrompt ? <Check size={14} className="text-emerald-400" /> : <Copy size={14} />}
          </button>
        </div>

        {/* 悬停指引 */}
        <div className="absolute bottom-2 inset-x-0 flex justify-center pointer-events-none opacity-0 group-hover:opacity-100 transition-opacity duration-200">
          <span className="px-2.5 py-1 rounded-full bg-black/75 backdrop-blur-md text-[11px] text-white/90 shadow-md flex items-center gap-1.5 border border-white/10">
            <ZoomIn size={12} />
            <span>点击查看大图</span>
          </span>
        </div>
      </div>

      {/* 提示词与元数据信息区 */}
      <div className="p-3 border-t border-edge/60 space-y-2.5 bg-panel2/50">
        <div className="flex items-center justify-between gap-2 flex-wrap">
          <div className="flex items-center gap-1.5 flex-wrap">
            <span className="inline-flex items-center gap-1 text-[11px] font-medium text-pink-400 bg-pink-500/10 border border-pink-500/20 px-2 py-0.5 rounded-md">
              <Sparkles size={11} />
              <span>提示词</span>
            </span>
            {metadata?.model && (
              <span className="text-[10.5px] font-mono text-inkdim bg-panel3/80 border border-edge/60 px-1.5 py-0.5 rounded-md" title="生图模型">
                {metadata.model}
              </span>
            )}
            {metadata?.resolution && (
              <span className="text-[10.5px] font-mono text-inkdim bg-panel3/80 border border-edge/60 px-1.5 py-0.5 rounded-md" title="图片分辨率">
                {metadata.resolution}
              </span>
            )}
            {metadata?.fileSize && (
              <span className="text-[10.5px] font-mono text-inkdim bg-panel3/80 border border-edge/60 px-1.5 py-0.5 rounded-md" title="文件体积">
                {metadata.fileSize}
              </span>
            )}
          </div>

          <div className="flex items-center gap-1.5 shrink-0 ml-auto">
            {metadata?.imagePath && (
              <button
                type="button"
                className="inline-flex items-center gap-1 px-2 py-1 rounded-md bg-panel3 hover:bg-edge text-inkdim hover:text-ink transition-colors text-[11px] cursor-pointer"
                onClick={handleCopyPath}
                title="复制图片存储路径"
              >
                {copiedPath ? <Check size={11} className="text-emerald-400" /> : <Copy size={11} />}
                <span>{copiedPath ? "已复制路径" : "复制路径"}</span>
              </button>
            )}
            {prompt && (
              <button
                type="button"
                className="inline-flex items-center gap-1 px-2 py-1 rounded-md bg-panel3 hover:bg-edge text-inkdim hover:text-ink transition-colors text-[11px] cursor-pointer"
                onClick={handleCopyPrompt}
                title="复制完整提示词"
              >
                {copiedPrompt ? <Check size={11} className="text-emerald-400" /> : <Copy size={11} />}
                <span>{copiedPrompt ? "已复制提示词" : "复制提示词"}</span>
              </button>
            )}
          </div>
        </div>

        {/* 提示词正文 */}
        {prompt && (
          <div className="relative rounded-lg bg-panel3/40 border border-edge/40 p-2.5 text-[12.5px] text-ink/90 leading-relaxed font-sans select-text break-words">
            <div className={!expandedPrompt && isLongPrompt ? "line-clamp-3" : ""}>
              {prompt}
            </div>
            {isLongPrompt && (
              <div className="mt-1 flex justify-end">
                <button
                  type="button"
                  onClick={() => setExpandedPrompt(!expandedPrompt)}
                  className="text-[11px] text-accent hover:underline cursor-pointer select-none"
                >
                  {expandedPrompt ? "收起提示词" : "展开完整提示词"}
                </button>
              </div>
            )}
          </div>
        )}

        {/* 本地路径单行展示 */}
        {metadata?.imagePath && (
          <div className="flex items-center justify-between text-[11px] text-inkdim/70 font-mono px-0.5 pt-0.5">
            <span className="truncate flex-1 min-w-0" title={metadata.imagePath}>
              保存至: {metadata.imagePath}
            </span>
          </div>
        )}
      </div>
    </div>
  );
}

export function ToolCard({ ev }: { ev: ToolEvent }) {
  if (ev.toolName === "spawn_subprocess" || ev.toolName === "spawn_subagent") {
    return <SubprocessBranchTree event={ev} />;
  }

  const output = useStore((s) => s.toolOutputs[ev.id]);
  const killCommand = useStore((s) => s.killCommand);
  const pushToast = useStore((s) => s.pushToast);
  const setLightboxImage = useStore((s) => s.setLightboxImage);
  const isRunning = useStore((s) => (s.currentId ? s.runStatus[s.currentId] === "running" : false));
  const [terminating, setTerminating] = useState(false);
  const [copied, setCopied] = useState(false);
  const title = paramTitle(ev);
  const label = TOOL_LABELS[ev.toolName] ?? ev.toolName;
  const isCommandTool = ev.toolName === "run_command";
  const commandText = isCommandTool ? String(ev.params?.command ?? "") : "";
  const isImageTool = ev.toolName === "generate_image";
  const imagePrompt = isImageTool ? String(ev.params?.prompt ?? "") : "";
  // 生图工具生成成功默认展开，生成失败默认收起；其他工具默认收起
  const [expanded, setExpanded] = useState(() => {
    if (!isImageTool) return false;
    return ev.status !== "failed" && ev.status !== "timeout" && ev.status !== "denied";
  });

  // 当生图状态在执行或流转中变为失败时，自动收起卡片；成功时展开
  useEffect(() => {
    if (!isImageTool) return;
    if (ev.status === "failed" || ev.status === "timeout" || ev.status === "denied") {
      setExpanded(false);
    } else if (ev.status === "success") {
      setExpanded(true);
    }
  }, [isImageTool, ev.status]);

  const generatedImagePath = useMemo(() => {
    if (ev.toolName !== "generate_image" || !ev.resultText) return null;
    const match = ev.resultText.match(/!\[.*?\]\((.*?)\)/);
    if (match) return match[1];
    const pathMatch = ev.resultText.match(/保存路径:\s*`?([^`\n]+)`?/);
    return pathMatch ? pathMatch[1].trim() : null;
  }, [ev.toolName, ev.resultText]);

  const imageMetadata = useMemo(() => {
    if (!isImageTool || !ev.resultText) return null;
    const text = ev.resultText;

    const pathMatch = text.match(/保存路径:\s*`?([^`\n]+)`?/);
    const modelMatch = text.match(/使用模型:\s*`?([^`\n]+)`?/);
    const sizeMatch = text.match(/分辨率:\s*([^\n]+)/);
    const kbMatch = text.match(/大小:\s*([^\n]+)/);

    return {
      imagePath: generatedImagePath || (pathMatch ? pathMatch[1].trim() : null),
      model: modelMatch ? modelMatch[1].trim() : null,
      resolution: sizeMatch ? sizeMatch[1].trim() : (ev.params?.size ? String(ev.params.size) : null),
      fileSize: kbMatch ? kbMatch[1].trim() : null,
    };
  }, [isImageTool, ev.resultText, generatedImagePath, ev.params]);

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

  const handleCopyPrompt = (e: React.MouseEvent) => {
    e.stopPropagation();
    const textToCopy = imagePrompt || title;
    if (!textToCopy) return;
    navigator.clipboard.writeText(textToCopy).then(
      () => {
        setCopied(true);
        setTimeout(() => setCopied(false), 1500);
        pushToast("提示词已复制到剪贴板");
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

  const currentWorkspace = useStore(
    (s) =>
      s.sessions.find((x) => x.id === s.currentId)?.workspacePath ||
      s.draft?.workspacePath ||
      s.settings.lastWorkspacePath ||
      ""
  );
  const filePath = ev.params?.path ? String(ev.params.path) : "";

  const handleOpenFileViewer = (e: React.MouseEvent) => {
    e.stopPropagation();
    if (!filePath) return;
    const cleanRel = filePath.replace(/\\/g, "/");
    const fileName = cleanRel.split("/").pop() || "文件";
    const absPath =
      /^[a-zA-Z]:\//.test(cleanRel) || cleanRel.startsWith("/")
        ? cleanRel
        : currentWorkspace
        ? `${currentWorkspace.replace(/\\/g, "/")}/${cleanRel.replace(/^\.\//, "")}`
        : cleanRel;

    ipc.openFileViewer({
      id: `file:${absPath}`,
      type: "file",
      title: fileName,
      subtitle: cleanRel,
      path: absPath,
      workspacePath: currentWorkspace,
    });
  };

  const handleOpenDiffViewer = (e: React.MouseEvent) => {
    e.stopPropagation();
    if (!filePath) return;
    const cleanRel = filePath.replace(/\\/g, "/");
    const fileName = cleanRel.split("/").pop() || "文件";
    const absPath =
      /^[a-zA-Z]:\//.test(cleanRel) || cleanRel.startsWith("/")
        ? cleanRel
        : currentWorkspace
        ? `${currentWorkspace.replace(/\\/g, "/")}/${cleanRel.replace(/^\.\//, "")}`
        : cleanRel;

    ipc.openFileViewer({
      id: `diff:${absPath}`,
      type: "diff",
      title: `Diff: ${fileName}`,
      subtitle: cleanRel,
      path: absPath,
      diffSource: "git",
      oldContent: ev.params?.old_string ? String(ev.params.old_string) : undefined,
      newContent: ev.params?.new_string ? String(ev.params.new_string) : undefined,
      workspacePath: currentWorkspace,
    });
  };

  // 提取方案文件的物理路径（.harness/plans/*.md）
  const resolvedPlanFilePath = useMemo(() => {
    if (!["create_plan", "update_plan", "switch_plan", "read_plan"].includes(ev.toolName)) {
      return null;
    }
    // 1. 显式参数传递（如 params 中注入 plan_file_path / planFilePath / path）
    if (ev.params?.plan_file_path) return String(ev.params.plan_file_path);
    if (ev.params?.planFilePath) return String(ev.params.planFilePath);
    if (typeof ev.params?.path === "string" && (ev.params.path.includes(".harness/plans/") || ev.params.path.includes(".harness\\plans\\"))) {
      return ev.params.path.replace(/\\/g, "/");
    }
    // 2. 从 resultText 中提取，支持 Windows 反斜杠、全路径、相对路径
    if (ev.resultText) {
      const normalized = ev.resultText.replace(/\\/g, "/");
      const match = normalized.match(/(?:\.harness\/plans\/|plans\/)([^`\s\n"'<>]+\.md)/i);
      if (match) return `.harness/plans/${match[1]}`;
    }
    return null;
  }, [ev.toolName, ev.resultText, ev.params]);

  const currentSessionId = useStore((s) => s.currentId);

  const handleOpenPlanFile = async (e: React.MouseEvent) => {
    e.stopPropagation();
    if (ev.status === "running") {
      pushToast("方案文档生成中，落盘后即可打开查看");
      return;
    }
    let targetRel = resolvedPlanFilePath;
    // 3. 兜底策略：若未能静态解析出路径，异步查询当前会话的活动计划文件
    if (!targetRel && currentSessionId) {
      try {
        const active = await ipc.getActivePlan(currentSessionId);
        if (active?.filename) {
          targetRel = `.harness/plans/${active.filename}`;
        }
      } catch {
        // ignore
      }
    }
    if (!targetRel) {
      pushToast("未找到生成的方案文件路径，请确认计划是否已成功创建");
      return;
    }
    const cleanRel = targetRel.replace(/\\/g, "/");
    const absPath =
      /^[a-zA-Z]:\//.test(cleanRel) || cleanRel.startsWith("/")
        ? cleanRel
        : currentWorkspace
          ? `${currentWorkspace.replace(/\\/g, "/").replace(/\/$/, "")}/${cleanRel.replace(/^\//, "")}`
          : cleanRel;
    const fileName = cleanRel.split("/").pop() || "任务方案.md";

    ipc.openFileViewer({
      id: `file:${absPath}`,
      type: "file",
      title: String(ev.params?.title ? `${ev.params.title}.md` : fileName),
      path: absPath,
      workspacePath: currentWorkspace,
      sessionId: currentSessionId || undefined,
    });
  };

  // 提取子任务/协作者交付报告工件的物理路径（.harness/subtasks/*_report.md）
  const resolvedSubtaskReportPath = useMemo(() => {
    if (!["wait_subprocesses", "wait_subagents", "wait_collaborators"].includes(ev.toolName)) {
      return null;
    }
    if (ev.resultText) {
      const normalized = ev.resultText.replace(/\\/g, "/");
      const match = normalized.match(/(?:\.harness\/subtasks\/)([^`\s\n"'<>]+\.md)/i);
      if (match) return `.harness/subtasks/${match[1]}`;
    }
    return null;
  }, [ev.toolName, ev.resultText]);

  const handleOpenSubtaskReport = (e: React.MouseEvent) => {
    e.stopPropagation();
    if (!resolvedSubtaskReportPath) return;
    const cleanRel = resolvedSubtaskReportPath.replace(/\\/g, "/");
    const absPath =
      /^[a-zA-Z]:\//.test(cleanRel) || cleanRel.startsWith("/")
        ? cleanRel
        : currentWorkspace
          ? `${currentWorkspace.replace(/\\/g, "/").replace(/\/$/, "")}/${cleanRel.replace(/^\//, "")}`
          : cleanRel;
    const fileName = cleanRel.split("/").pop() || "子任务交付报告.md";

    ipc.openFileViewer({
      id: `file:${absPath}`,
      type: "file",
      title: `交付报告: ${fileName}`,
      path: absPath,
      workspacePath: currentWorkspace,
      sessionId: currentSessionId || undefined,
    });
  };

  const showOutput = ev.status === "running" && output !== undefined;
  const showResult = !isImageTool && !!ev.resultText && (ev.status === "success" || ev.status === "failed" || ev.status === "timeout" || ev.status === "denied");

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
          {isImageTool && (imagePrompt || title) && (
            <button
              type="button"
              className="p-1 rounded hover:bg-panel3 text-inkdim hover:text-ink transition-colors"
              title="复制提示词"
              onClick={handleCopyPrompt}
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
          {ev.toolName === "read_file" && filePath && (
            <button
              type="button"
              className="inline-flex items-center gap-1 text-[11px] px-2 py-0.5 rounded bg-blue-500/10 hover:bg-blue-500/20 text-blue-300 transition-colors"
              title="在独立窗体中查看文件全文"
              onClick={handleOpenFileViewer}
            >
              <FileText size={11} />
              <span>查看</span>
            </button>
          )}
          {(ev.toolName === "write_file" || ev.toolName === "edit_file") && filePath && (
            <button
              type="button"
              className="inline-flex items-center gap-1 text-[11px] px-2 py-0.5 rounded bg-amber-500/10 hover:bg-amber-500/20 text-amber-300 transition-colors"
              title="在独立窗体中查看文件变更对比 (Diff)"
              onClick={handleOpenDiffViewer}
            >
              <GitCompare size={11} />
              <span>对比 Diff</span>
            </button>
          )}
          {["create_plan", "update_plan", "switch_plan", "read_plan"].includes(ev.toolName) && (
            <button
              type="button"
              className={`inline-flex items-center gap-1 text-[11px] px-2 py-0.5 rounded transition-colors ${
                ev.status === "running"
                  ? "bg-panel3 text-inkdim/50 cursor-not-allowed"
                  : "bg-emerald-500/10 hover:bg-emerald-500/20 text-emerald-300 cursor-pointer"
              }`}
              title={
                ev.status === "running"
                  ? "方案文档生成中，落盘后即可打开"
                  : "以 Markdown 文件打开并可手动调整方案"
              }
              onClick={handleOpenPlanFile}
              disabled={ev.status === "running"}
            >
              <CheckSquare size={11} />
              <span>{ev.status === "running" ? "方案生成中..." : "方案文档 (MD)"}</span>
            </button>
          )}
          {["wait_subprocesses", "wait_subagents", "wait_collaborators"].includes(ev.toolName) && resolvedSubtaskReportPath && (
            <button
              type="button"
              className="inline-flex items-center gap-1 text-[11px] px-2 py-0.5 rounded bg-indigo-500/10 hover:bg-indigo-500/20 text-indigo-300 transition-colors cursor-pointer border border-indigo-500/30"
              title="在独立窗体中查看完整子任务详细报告"
              onClick={handleOpenSubtaskReport}
            >
              <FileText size={11} />
              <span>报告工件 (MD)</span>
            </button>
          )}
          {statusBadge(ev.status)}
          {ev.approvalScope && ev.approvalScope !== "none" && ev.status !== "pending_approval" && (
            <span className="text-[10px] text-inkdim shrink-0">
              ({ev.approvalScope === "mode" ? "完全访问" : ev.approvalScope === "once" ? "一次放行" : ev.approvalScope === "harness_whitelist" ? "系统白名单" : "会话规则"})
            </span>
          )}
        </div>
      </div>

      {ev.toolName === "todo" && Array.isArray(ev.params?.todos) && (
        <TodoList todos={ev.params.todos} isRunning={isRunning} />
      )}

      {["create_plan", "update_plan", "switch_plan"].includes(ev.toolName) && (
        <PlanCardView
          ev={ev}
          onOpenPlan={handleOpenPlanFile}
          planFilePath={resolvedPlanFilePath}
        />
      )}

      {/* 审批条是交互入口，保持常显 */}
      {ev.status === "pending_approval" && <ApprovalSection ev={ev} />}

      {/* 展开时：若是生图工具，展示专用生图成果与提示词卡片 */}
      {expanded && isImageTool && (
        <ImageToolView
          ev={ev}
          imagePath={generatedImagePath}
          prompt={imagePrompt || title}
          metadata={imageMetadata}
        />
      )}

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
