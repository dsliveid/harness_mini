import React, { useState } from "react";
import {
  CheckSquare,
  Clock,
  Copy,
  Check,
  ChevronDown,
  ChevronUp,
  Code,
  ListTodo,
  FileText,
  Tag,
} from "./Icons";
import { formatTimestamp } from "../utils/frontmatter";
import { ipc } from "../ipc";

export interface FrontmatterCardProps {
  meta: Record<string, any>;
  rawYaml: string;
  isPlan: boolean;
  workspacePath?: string | null;
  currentDocPath?: string | null;
  onOpenPlanViewer?: () => void;
}

const PLAN_STATUS_MAP: Record<
  string,
  { label: string; cls: string; dotCls: string }
> = {
  in_progress: {
    label: "执行中",
    cls: "bg-emerald-500/15 text-emerald-400 border-emerald-500/30",
    dotCls: "bg-emerald-400 animate-pulse",
  },
  completed: {
    label: "已结案",
    cls: "bg-green-500/15 text-green-400 border-green-500/30",
    dotCls: "bg-green-400",
  },
  drafting: {
    label: "拟订中",
    cls: "bg-blue-500/15 text-blue-400 border-blue-500/30",
    dotCls: "bg-blue-400",
  },
  suspended: {
    label: "已挂起",
    cls: "bg-amber-500/15 text-amber-400 border-amber-500/30",
    dotCls: "bg-amber-400",
  },
  archived: {
    label: "已归档",
    cls: "bg-gray-500/15 text-gray-400 border-gray-500/30",
    dotCls: "bg-gray-400",
  },
};

export function FrontmatterCard({
  meta,
  rawYaml,
  isPlan,
  workspacePath,
  onOpenPlanViewer,
}: FrontmatterCardProps) {
  const [collapsed, setCollapsed] = useState(false);
  const [showRawYaml, setShowRawYaml] = useState(false);
  const [copiedKey, setCopiedKey] = useState<string | null>(null);

  const copyToClipboard = (text: string, keyName: string) => {
    if (!text) return;
    navigator.clipboard.writeText(text).then(() => {
      setCopiedKey(keyName);
      setTimeout(() => setCopiedKey(null), 1500);
    });
  };

  const handleOpenPlanViewer = () => {
    if (onOpenPlanViewer) {
      onOpenPlanViewer();
      return;
    }
    const planId = (meta.id || "").trim();
    if (!planId) return;
    ipc.openFileViewer({
      id: `plan:${planId}`,
      type: "plan",
      title: meta.title || planId,
      planId: planId,
      workspacePath: workspacePath || undefined,
      sessionId: meta.session_id || undefined,
    });
  };

  // 1. 计划文档专用高质感元数据横幅（Plan Meta Banner）
  if (isPlan) {
    const statusKey = String(meta.status || "in_progress").toLowerCase();
    const statusInfo =
      PLAN_STATUS_MAP[statusKey] || PLAN_STATUS_MAP.in_progress;
    const planTitle = meta.title || "任务实施计划方案";
    const versionStr = meta.version !== undefined ? `v${meta.version}` : null;
    const createdAtStr = formatTimestamp(meta.created_at);
    const updatedAtStr = formatTimestamp(meta.updated_at);
    const planId = meta.id ? String(meta.id) : "";
    const sessionId = meta.session_id ? String(meta.session_id) : "";

    return (
      <div className="mb-6 rounded-xl border border-emerald-500/30 bg-emerald-500/[0.04] shadow-2xs overflow-hidden select-text transition-all">
        {/* 顶部标题栏与徽标 */}
        <div className="px-4 py-3 bg-panel2/60 border-b border-edge/60 flex items-center justify-between gap-3 flex-wrap">
          <div className="flex items-center gap-2.5 min-w-0 flex-1">
            <div className="w-7 h-7 rounded-lg bg-emerald-500/15 border border-emerald-500/30 flex items-center justify-center shrink-0">
              <CheckSquare size={15} className="text-emerald-400" />
            </div>
            <div className="flex items-center gap-2 min-w-0 flex-wrap">
              <span className="font-semibold text-[14px] text-ink truncate">
                {planTitle}
              </span>
              {versionStr && (
                <span className="text-[11px] font-mono px-2 py-0.5 rounded-full bg-emerald-500/15 text-emerald-400 border border-emerald-500/30 shrink-0 font-medium">
                  {versionStr}
                </span>
              )}
              <span
                className={`inline-flex items-center gap-1.5 text-[11px] px-2 py-0.5 rounded-full border shrink-0 ${statusInfo.cls}`}
              >
                <span className={`w-1.5 h-1.5 rounded-full ${statusInfo.dotCls}`} />
                <span>{statusInfo.label}</span>
              </span>
            </div>
          </div>

          {/* 右侧快捷动作按钮 */}
          <div className="flex items-center gap-1.5 shrink-0 text-[11px] font-sans">
            <button
              type="button"
              className="flex items-center gap-1 px-2.5 py-1 rounded-lg bg-emerald-500/15 hover:bg-emerald-500/25 border border-emerald-500/30 text-emerald-300 transition-colors cursor-pointer"
              title="切换为分步交互式任务清单看板"
              onClick={handleOpenPlanViewer}
            >
              <ListTodo size={12} />
              <span>任务清单看板</span>
            </button>

            <button
              type="button"
              className={`flex items-center gap-1 px-2 py-1 rounded-lg border transition-colors cursor-pointer ${
                showRawYaml
                  ? "bg-panel3 text-accent border-accent/40 font-medium"
                  : "text-inkdim hover:text-ink hover:bg-panel3 border-transparent"
              }`}
              title="查看原始 YAML 头部文本"
              onClick={() => setShowRawYaml(!showRawYaml)}
            >
              <Code size={12} />
              <span>YAML</span>
            </button>

            <button
              type="button"
              className="p-1 rounded-lg text-inkdim hover:text-ink hover:bg-panel3 transition-colors cursor-pointer"
              title={collapsed ? "展开元信息" : "收起元信息"}
              onClick={() => setCollapsed(!collapsed)}
            >
              {collapsed ? <ChevronDown size={14} /> : <ChevronUp size={14} />}
            </button>
          </div>
        </div>

        {/* 展开的详情网格 */}
        {!collapsed && (
          <div className="p-4 space-y-3 text-[12px] bg-panel/30">
            <div className="grid grid-cols-1 md:grid-cols-2 gap-3 text-inkdim">
              {createdAtStr && (
                <div className="flex items-center gap-2">
                  <Clock size={13} className="text-inkdim/70 shrink-0" />
                  <span className="text-inkdim/80">创建时间:</span>
                  <span className="font-mono text-ink text-[11.5px]">
                    {createdAtStr}
                  </span>
                </div>
              )}
              {updatedAtStr && (
                <div className="flex items-center gap-2">
                  <Clock size={13} className="text-inkdim/70 shrink-0" />
                  <span className="text-inkdim/80">更新时间:</span>
                  <span className="font-mono text-ink text-[11.5px]">
                    {updatedAtStr}
                  </span>
                </div>
              )}
              {planId && (
                <div className="flex items-center gap-2 min-w-0 md:col-span-2">
                  <span className="text-inkdim/80 shrink-0 font-mono text-[11px] bg-panel3 px-1.5 py-0.5 rounded border border-edge/60">
                    ID
                  </span>
                  <span
                    className="font-mono text-[11px] text-ink truncate select-all flex-1 min-w-0"
                    title={planId}
                  >
                    {planId}
                  </span>
                  <button
                    type="button"
                    className="flex items-center gap-1 text-[11px] text-emerald-400 hover:text-emerald-300 shrink-0 cursor-pointer"
                    title="复制方案 ID"
                    onClick={() => copyToClipboard(planId, "planId")}
                  >
                    {copiedKey === "planId" ? (
                      <>
                        <Check size={11} className="text-green-400" />
                        <span className="text-green-400">已复制</span>
                      </>
                    ) : (
                      <>
                        <Copy size={11} />
                        <span>复制</span>
                      </>
                    )}
                  </button>
                </div>
              )}
              {sessionId && (
                <div className="flex items-center gap-2 min-w-0 md:col-span-2">
                  <span className="text-inkdim/80 shrink-0 font-mono text-[11px] bg-panel3 px-1.5 py-0.5 rounded border border-edge/60">
                    会话
                  </span>
                  <span
                    className="font-mono text-[11px] text-inkdim truncate select-all flex-1 min-w-0"
                    title={sessionId}
                  >
                    {sessionId}
                  </span>
                  <button
                    type="button"
                    className="flex items-center gap-1 text-[11px] text-inkdim hover:text-ink shrink-0 cursor-pointer"
                    title="复制关联会话 ID"
                    onClick={() => copyToClipboard(sessionId, "sessionId")}
                  >
                    {copiedKey === "sessionId" ? (
                      <>
                        <Check size={11} className="text-green-400" />
                        <span className="text-green-400">已复制</span>
                      </>
                    ) : (
                      <>
                        <Copy size={11} />
                        <span>复制</span>
                      </>
                    )}
                  </button>
                </div>
              )}
            </div>

            {/* 原始 YAML 抽屉 */}
            {showRawYaml && rawYaml && (
              <div className="mt-3 pt-3 border-t border-edge/60">
                <div className="flex items-center justify-between mb-1.5 text-[11px] text-inkdim">
                  <span className="font-mono">原始 Frontmatter (YAML):</span>
                  <button
                    type="button"
                    className="flex items-center gap-1 hover:text-ink cursor-pointer"
                    onClick={() => copyToClipboard(rawYaml, "rawYaml")}
                  >
                    {copiedKey === "rawYaml" ? (
                      <Check size={11} className="text-green-400" />
                    ) : (
                      <Copy size={11} />
                    )}
                    <span>复制 YAML</span>
                  </button>
                </div>
                <pre className="p-2.5 rounded-lg bg-[#0e0e12] border border-edge/80 text-[11.5px] font-mono text-zinc-300 overflow-x-auto leading-relaxed">
                  {rawYaml.trim()}
                </pre>
              </div>
            )}
          </div>
        )}
      </div>
    );
  }

  // 2. 通用文档元数据属性面板（Generic Document Properties）
  const metaKeys = Object.keys(meta);
  if (metaKeys.length === 0) return null;

  return (
    <div className="mb-6 rounded-xl border border-edge/70 bg-panel2/40 shadow-2xs overflow-hidden select-text transition-all text-[12px]">
      <div className="px-3.5 py-2.5 bg-panel2/70 border-b border-edge/60 flex items-center justify-between gap-2">
        <div className="flex items-center gap-2 text-inkdim min-w-0">
          <FileText size={14} className="text-accent shrink-0" />
          <span className="font-medium text-ink text-[12.5px]">文档元属性</span>
          <span className="text-[10.5px] px-1.5 py-0.2 rounded-full bg-panel3 text-inkdim font-mono">
            {metaKeys.length} 项
          </span>
        </div>

        <div className="flex items-center gap-1 text-[11px]">
          <button
            type="button"
            className={`flex items-center gap-1 px-2 py-0.5 rounded-md border transition-colors cursor-pointer ${
              showRawYaml
                ? "bg-panel3 text-accent border-accent/40 font-medium"
                : "text-inkdim hover:text-ink hover:bg-panel3 border-transparent"
            }`}
            title="查看原始 YAML"
            onClick={() => setShowRawYaml(!showRawYaml)}
          >
            <Code size={12} />
            <span>YAML</span>
          </button>
          <button
            type="button"
            className="p-1 rounded-md text-inkdim hover:text-ink hover:bg-panel3 transition-colors cursor-pointer"
            onClick={() => setCollapsed(!collapsed)}
          >
            {collapsed ? <ChevronDown size={14} /> : <ChevronUp size={14} />}
          </button>
        </div>
      </div>

      {!collapsed && (
        <div className="p-3.5 space-y-2">
          <div className="grid grid-cols-1 sm:grid-cols-2 gap-x-4 gap-y-2">
            {metaKeys.map((key) => {
              const val = meta[key];
              const isArray = Array.isArray(val);
              const isDate =
                typeof val === "string" &&
                /\d{4}-\d{2}-\d{2}/.test(val) &&
                !isNaN(Date.parse(val));
              const displayVal = isDate ? formatTimestamp(val) : val;

              return (
                <div key={key} className="flex items-start gap-2 min-w-0 py-0.5">
                  <span className="font-mono text-[11px] text-inkdim/80 shrink-0 min-w-[70px]">
                    {key}:
                  </span>
                  <div className="flex-1 min-w-0">
                    {isArray ? (
                      <div className="flex flex-wrap gap-1">
                        {val.map((item: any, idx: number) => (
                          <span
                            key={idx}
                            className="inline-flex items-center gap-1 px-1.5 py-0.5 rounded bg-accent/10 border border-accent/25 text-accent text-[11px]"
                          >
                            <Tag size={10} />
                            <span>{String(item)}</span>
                          </span>
                        ))}
                      </div>
                    ) : (
                      <span className="text-ink font-mono text-[11.5px] break-all select-all">
                        {String(displayVal)}
                      </span>
                    )}
                  </div>
                </div>
              );
            })}
          </div>

          {showRawYaml && rawYaml && (
            <div className="mt-2.5 pt-2.5 border-t border-edge/60">
              <pre className="p-2 rounded-lg bg-[#0e0e12] border border-edge/80 text-[11px] font-mono text-zinc-300 overflow-x-auto leading-relaxed">
                {rawYaml.trim()}
              </pre>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
