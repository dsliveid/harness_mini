import { useState, useEffect, useMemo, useRef } from "react";
import { useStore } from "../store";
import { ipc } from "../ipc";
import { MessageItem } from "./MessageItem";
import { computeTurnMetrics } from "../types";
import {
  ChevronRight,
  Loader2,
  CheckCircle2,
  AlertCircle,
  Square,
  Eye,
  Bot,
  Coins,
  Cpu,
} from "./Icons";
import type { ToolEvent } from "../types";

function formatTokens(n?: number | null): string {
  if (n == null || isNaN(n)) return "0";
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(2)}M`;
  if (n >= 10_000) return `${(n / 1_000).toFixed(1)}k`;
  return n.toLocaleString("zh-CN");
}

const ROLE_INFO: Record<string, { label: string; icon: string; color: string }> = {
  frontend: { label: "前端开发", icon: "🎨", color: "text-blue-400 bg-blue-500/10 border-blue-500/20" },
  backend: { label: "后端开发", icon: "⚙️", color: "text-emerald-400 bg-emerald-500/10 border-emerald-500/20" },
  testing: { label: "测试校验", icon: "🧪", color: "text-purple-400 bg-purple-500/10 border-purple-500/20" },
  review: { label: "代码审阅", icon: "🔍", color: "text-amber-400 bg-amber-500/10 border-amber-500/20" },
  fullstack: { label: "全栈开发", icon: "⚡", color: "text-indigo-400 bg-indigo-500/10 border-indigo-500/20" },
};

export function SubprocessCard({ ev, subprocessId: propSubId }: { ev?: ToolEvent; subprocessId?: string }) {
  const currentParentId = useStore((s) => s.currentId);
  const subprocesses = useStore((s) => (currentParentId ? s.subprocesses[currentParentId] ?? [] : []));
  const runStatus = useStore((s) => s.runStatus);
  const stopSubagent = useStore((s) => s.stopSubagent);

  // 从传入 prop 或从 ev.resultText / ev.params 中解析子进程 ID
  const subId = useMemo(() => {
    if (propSubId) return propSubId;
    if (ev?.params?.subprocess_id) return String(ev.params.subprocess_id);
    if (ev?.params?.subagent_id) return String(ev.params.subagent_id);
    if (ev?.resultText) {
      const match = ev.resultText.match(/- ID:\s*`([^`]+)`/);
      if (match) return match[1];
    }
    return null;
  }, [propSubId, ev]);

  const sub = useMemo(() => {
    if (!subId) return null;
    return subprocesses.find((s) => s.id === subId) ?? null;
  }, [subprocesses, subId]);

  const msgs = useStore((s) => (subId ? s.messages[subId] ?? [] : []));
  const isRunning = subId ? runStatus[subId] === "running" : false;

  const [expanded, setExpanded] = useState(false);
  const [stopping, setStopping] = useState(false);
  const loadedRef = useRef(false);

  // 展开或有活跃 ID 时，按需拉取子进程历史消息
  useEffect(() => {
    if (subId && (expanded || isRunning) && !loadedRef.current) {
      loadedRef.current = true;
      ipc.getMessages(subId, undefined, 100).then((messages) => {
        useStore.setState((st) => ({
          messages: { ...st.messages, [subId]: messages },
        }));
      }).catch(() => {});
    }
  }, [subId, expanded, isRunning]);

  const turnMetricsMap = useMemo(() => computeTurnMetrics(msgs, isRunning), [msgs, isRunning]);

  const roleKey = sub?.subagentRole || String(ev?.params?.role || "custom");
  const roleMeta = ROLE_INFO[roleKey] ?? {
    label: "子进程",
    icon: "⚡",
    color: "text-zinc-400 bg-zinc-500/10 border-zinc-500/20",
  };

  const title = sub?.title || String(ev?.params?.title || "临时执行子进程");
  const taskText = sub?.subagentTask || String(ev?.params?.task || "");

  const handleStop = async (e: React.MouseEvent) => {
    e.stopPropagation();
    if (!subId || stopping) return;
    setStopping(true);
    try {
      await stopSubagent(subId);
    } finally {
      setStopping(false);
    }
  };

  const totalTokens = sub?.totalTokens ?? 0;

  return (
    <div className="rounded-xl border border-indigo-500/25 bg-panel2/70 overflow-hidden my-2 shadow-xs transition-all">
      {/* Card Header */}
      <div
        role="button"
        tabIndex={0}
        onClick={() => setExpanded(!expanded)}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            setExpanded(!expanded);
          }
        }}
        className="flex items-center justify-between px-3.5 py-2.5 bg-panel2/90 hover:bg-panel3/60 cursor-pointer select-none transition-colors border-b border-edge/40"
      >
        <div className="flex items-center gap-2 min-w-0 flex-1 mr-2">
          <ChevronRight
            size={13}
            className={`transition-transform duration-150 text-inkdim shrink-0 ${expanded ? "rotate-90" : ""}`}
          />
          <div className="flex items-center gap-1.5 px-1.5 py-0.5 rounded bg-indigo-500/15 text-indigo-400 border border-indigo-500/30 text-[10px] font-medium shrink-0">
            <Cpu size={11} />
            <span>子进程</span>
          </div>

          <span className={`px-1.5 py-0.5 rounded text-[10px] font-medium border flex items-center gap-1 shrink-0 ${roleMeta.color}`}>
            <span>{roleMeta.icon}</span>
            <span>{roleMeta.label}</span>
          </span>

          <span className="font-medium text-[13px] text-ink truncate" title={title}>
            {title}
          </span>
        </div>

        {/* Right Status & Controls */}
        <div className="flex items-center gap-2 shrink-0">
          <span className="inline-flex items-center gap-1 text-[10px] text-inkdim bg-panel border border-edge/60 px-1.5 py-0.5 rounded">
            <Eye size={10} className="opacity-70" />
            <span>只读执行</span>
          </span>

          {totalTokens > 0 && (
            <span className="text-[11px] font-mono text-inkdim flex items-center gap-1">
              <Coins size={11} className="text-amber-400" />
              <span>{formatTokens(totalTokens)}</span>
            </span>
          )}

          {isRunning ? (
            <span className="flex items-center gap-1 text-[11px] text-accent font-medium animate-pulse">
              <Loader2 size={12} className="animate-spin" />
              <span>执行中</span>
            </span>
          ) : sub?.status === "failed" ? (
            <span className="flex items-center gap-1 text-[11px] text-rose-400 font-medium">
              <AlertCircle size={12} />
              <span>已终止</span>
            </span>
          ) : (
            <span className="flex items-center gap-1 text-[11px] text-emerald-400 font-medium">
              <CheckCircle2 size={12} />
              <span>已完成</span>
            </span>
          )}

          {isRunning && subId && (
            <button
              type="button"
              onClick={handleStop}
              disabled={stopping}
              className="text-[10px] px-2 py-0.5 rounded bg-red-600/80 hover:bg-red-500 text-white disabled:opacity-50 transition-colors shadow-xs"
              title="强制终止该子进程"
            >
              <span className="flex items-center gap-1">
                <Square size={9} fill="currentColor" />
                <span>{stopping ? "停止中…" : "终止"}</span>
              </span>
            </button>
          )}
        </div>
      </div>

      {/* Collapsed Preview */}
      {!expanded && taskText && (
        <div className="px-3.5 py-1.5 text-[12px] text-inkdim truncate bg-panel/40 font-mono">
          任务目标：{taskText}
        </div>
      )}

      {/* Expanded Timeline View (View-only) */}
      {expanded && (
        <div className="p-3.5 bg-panel/30 space-y-3">
          {taskText && (
            <div className="p-2.5 rounded-lg bg-panel2/60 border border-edge/60 text-[12px] text-ink leading-relaxed">
              <div className="text-[11px] font-medium text-inkdim mb-1 flex items-center gap-1">
                <Bot size={12} className="text-indigo-400" />
                <span>子进程分派任务说明（只读）：</span>
              </div>
              <div className="whitespace-pre-wrap">{taskText}</div>
            </div>
          )}

          {/* Subprocess Messages Timeline */}
          {msgs.length === 0 ? (
            <div className="py-4 text-center text-inkdim text-[12px] flex items-center justify-center gap-2">
              <Loader2 size={13} className="animate-spin text-accent" />
              <span>正在加载子进程执行轨迹…</span>
            </div>
          ) : (
            <div className="space-y-3 max-h-[380px] overflow-y-auto pr-1">
              {msgs
                .filter((m) => m.role !== "tool")
                .map((m) => (
                  <MessageItem
                    key={m.id}
                    msg={m}
                    isLastUser={false}
                    streaming={isRunning && m.role === "assistant" && m.id === msgs[msgs.length - 1]?.id}
                    readOnly={true}
                    running={isRunning}
                    turnMetrics={turnMetricsMap.get(m.id)}
                  />
                ))}
            </div>
          )}

          <div className="text-[11px] text-inkdim/60 pt-1 border-t border-edge/40 flex items-center justify-between">
            <span>🔒 子进程由 Agent 自动创建与执行，外部不可交互输入，产出将自动向主会话报告。</span>
            {subId && <span className="font-mono">ID: {subId.slice(0, 8)}...</span>}
          </div>
        </div>
      )}
    </div>
  );
}
