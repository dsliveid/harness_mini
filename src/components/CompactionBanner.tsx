import { useState, useEffect } from "react";
import { ipc } from "../ipc";
import { useStore } from "../store";
import { Markdown } from "./Markdown";
import type { SessionCompaction } from "../types";
import {
  Sparkles,
  Check,
  Edit2,
  Eye,
  ChevronDown,
  ChevronRight,
  RotateCw,
  Clock,
  X,
} from "./Icons";

export function CompactionBanner() {
  const currentId = useStore((s) => s.currentId);
  const pendingMap = useStore((s) => s.pendingCompactions);
  const compactionDone = useStore((s) => s.compactionDone);
  const onCompactionTimeout = useStore((s) => s.onCompactionTimeout);
  const pushToast = useStore((s) => s.pushToast);

  const pending = currentId ? Object.values(pendingMap).find((r) => r.sessionId === currentId) : null;

  const [mode, setMode] = useState<"preview" | "edit">("preview");
  const [draftSummary, setDraftSummary] = useState<string>("");
  const [busy, setBusy] = useState<boolean>(false);

  const timeoutTotal = pending?.timeoutSeconds ?? 30;
  const isTimedOut = !!pending?.timedOut;

  const [remainingSecs, setRemainingSecs] = useState<number>(() => {
    if (!pending) return 0;
    if (pending.timedOut) return 0;
    const elapsed = Math.floor((Date.now() - (pending.createdAt ?? Date.now())) / 1000);
    return Math.max(0, timeoutTotal - elapsed);
  });

  useEffect(() => {
    if (pending) {
      setDraftSummary(pending.summary);
      // 若已超时，默认进入 preview 查看模式
      if (pending.timedOut) {
        setMode("preview");
      }
    }
  }, [pending?.eventId, pending?.summary, pending?.timedOut]);

  // 30秒无操作倒计时
  useEffect(() => {
    if (!pending || pending.timedOut) {
      setRemainingSecs(0);
      return;
    }
    const updateCountdown = () => {
      const elapsed = Math.floor((Date.now() - (pending.createdAt ?? Date.now())) / 1000);
      const rem = Math.max(0, timeoutTotal - elapsed);
      setRemainingSecs(rem);
      if (rem <= 0 && !pending.timedOut) {
        onCompactionTimeout(pending.eventId);
      }
    };
    updateCountdown();
    const timer = setInterval(updateCountdown, 1000);
    return () => clearInterval(timer);
  }, [pending?.eventId, pending?.timedOut, pending?.createdAt, timeoutTotal, onCompactionTimeout]);

  if (!pending) return null;

  const handleConfirm = async () => {
    setBusy(true);
    try {
      await ipc.respondCompaction(pending.eventId, true, draftSummary);
      compactionDone(pending.eventId);
      pushToast("已确认并应用上下文压缩，Agent 继续执行下一步");
    } catch (e) {
      pushToast(`确认压缩失败: ${e}`);
    } finally {
      setBusy(false);
    }
  };

  const handleSkip = async () => {
    setBusy(true);
    try {
      await ipc.respondCompaction(pending.eventId, false, "");
      compactionDone(pending.eventId);
      pushToast("已跳过本次上下文压缩");
    } catch (e) {
      pushToast(`跳过压缩失败: ${e}`);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className={`mx-4 my-3 p-4 rounded-2xl bg-panel2 border-2 ${
      isTimedOut ? "border-amber-500/40" : "border-accent/40"
    } shadow-xl animate-in fade-in slide-in-from-top-3 duration-200`}>
      {/* 头部标题与说明 */}
      <div className="flex items-start justify-between gap-3 mb-3">
        <div className="flex items-center gap-2.5">
          <div className={`w-8 h-8 rounded-xl ${
            isTimedOut ? "bg-amber-500/15 text-amber-400 border border-amber-500/25" : "bg-accent/15 text-accent border border-accent/25"
          } flex items-center justify-center shrink-0`}>
            {isTimedOut ? <Clock size={17} /> : <Sparkles size={17} />}
          </div>
          <div>
            <div className="text-[14px] font-semibold text-ink flex items-center gap-2 flex-wrap">
              <span>{isTimedOut ? "上下文已自动压缩（执行中）" : "上下文自动压缩确认"}</span>
              {isTimedOut ? (
                <span className="text-[11px] px-2 py-0.5 rounded-full bg-amber-500/10 text-amber-400 border border-amber-500/20 font-medium">
                  30秒无操作已自动应用
                </span>
              ) : (
                <span className="text-[11px] px-2 py-0.5 rounded-full bg-accent/10 text-accent border border-accent/20 font-mono flex items-center gap-1">
                  <Clock size={11} />
                  <span>30s 无操作将自动继续 (剩余 {remainingSecs}s)</span>
                </span>
              )}
            </div>
            <div className="text-[12px] text-inkdim mt-0.5">
              {isTimedOut
                ? "由于 30 秒无操作，系统已自动应用生成的 Markdown 备忘录并继续后续对话流程。通知保持展示供您查看压缩内容。"
                : "当前对话历史已接近设定上限，系统已自动提炼 Markdown 备忘录以释放空间并保留长远记忆。"}
            </div>
          </div>
        </div>

        {/* 右上角操作区域 */}
        <div className="flex items-center gap-1.5 shrink-0">
          {!isTimedOut ? (
            /* 模式切换：预览 / 补充编辑 */
            <div className="flex items-center gap-1 bg-panel p-1 rounded-lg border border-edge">
              <button
                type="button"
                className={`px-2.5 py-1 rounded text-[12px] flex items-center gap-1.5 transition-colors ${
                  mode === "preview"
                    ? "bg-panel2 text-accent font-medium shadow-xs"
                    : "text-inkdim hover:text-ink"
                }`}
                onClick={() => setMode("preview")}
              >
                <Eye size={13} />
                <span>查看内容</span>
              </button>
              <button
                type="button"
                className={`px-2.5 py-1 rounded text-[12px] flex items-center gap-1.5 transition-colors ${
                  mode === "edit"
                    ? "bg-panel2 text-accent font-medium shadow-xs"
                    : "text-inkdim hover:text-ink"
                }`}
                onClick={() => setMode("edit")}
              >
                <Edit2 size={13} />
                <span>编辑 / 补充</span>
              </button>
            </div>
          ) : (
            /* 超时后关闭按钮 */
            <button
              type="button"
              onClick={() => compactionDone(pending.eventId)}
              className="p-1 rounded-lg text-inkdim hover:text-ink hover:bg-panel transition-colors"
              title="关闭通知"
            >
              <X size={16} />
            </button>
          )}
        </div>
      </div>

      {/* 压缩范围可视化指示条 */}
      <div className="grid grid-cols-1 sm:grid-cols-3 gap-2 p-2.5 mb-3 rounded-xl bg-panel/60 border border-edge text-[12px]">
        <div className="flex flex-col">
          <span className="text-inkdim text-[11px]">压缩起止范围</span>
          <span className="font-mono text-ink font-medium mt-0.5">
            第 {pending.startSeq} 条 ➔ 第 {pending.endSeq} 条消息
          </span>
        </div>
        <div className="flex flex-col">
          <span className="text-inkdim text-[11px]">消息条数与类型</span>
          <span className="text-ink mt-0.5">
            共 {pending.messageCount} 条（含往期工具调用与交互）
          </span>
        </div>
        <div className="flex flex-col">
          <span className="text-inkdim text-[11px]">原占用 Token 预估</span>
          <span className="text-accent font-mono font-medium mt-0.5">
            约 {pending.tokensBefore.toLocaleString()} tokens
          </span>
        </div>
      </div>

      {/* 消息范围内容预览 */}
      <div className="text-[11px] text-inkdim mb-2 flex items-center gap-2 px-1">
        <span className="shrink-0 text-ink/70">范围起点：</span>
        <span className="truncate italic">"{pending.startPreview}"</span>
        <span className="shrink-0 text-ink/70 ml-2">范围终点：</span>
        <span className="truncate italic">"{pending.endPreview}"</span>
      </div>

      {/* 内容区域：Markdown 渲染 或 补充编辑框 */}
      <div className="rounded-xl border border-edge bg-panel overflow-hidden mb-3">
        {mode === "preview" || isTimedOut ? (
          <div className="p-3.5 max-h-[320px] overflow-y-auto text-[13px] leading-relaxed">
            <Markdown content={draftSummary} />
          </div>
        ) : (
          <div className="flex flex-col">
            <div className="px-3 py-1.5 bg-panel2/60 border-b border-edge/60 text-[11px] text-inkdim flex items-center justify-between">
              <span>您可以在下方 Markdown 中直接修改或在末尾追加人工补充说明：</span>
              <span>支持标准 Markdown 语法</span>
            </div>
            <textarea
              className="w-full p-3 bg-transparent text-[13px] font-mono outline-none resize-y min-h-[220px] max-h-[420px] text-ink"
              value={draftSummary}
              onChange={(e) => setDraftSummary(e.target.value)}
              placeholder="在此输入或补充备忘录内容..."
            />
          </div>
        )}
      </div>

      {/* 底部操作区域 */}
      <div className="flex items-center justify-between pt-1">
        <div className="text-[12px] text-inkdim">
          {isTimedOut
            ? "压缩备忘录已注入后续上下文，流程已继续执行。查阅完毕后可关闭此通知。"
            : mode === "edit"
            ? "已开启补充模式，修改将在确认后注入后续上下文。"
            : "预览完毕后点击确认即可继续下一步，或在 30 秒倒计时结束前补充。"}
        </div>
        <div className="flex items-center gap-2.5">
          {isTimedOut ? (
            /* 超时自动应用后：没有继续下一步按钮，仅提供关闭/知道了按钮 */
            <button
              type="button"
              className="px-4 py-1.5 rounded-lg bg-panel hover:bg-panel3 border border-edge text-ink text-[12px] font-medium transition-colors shadow-xs"
              onClick={() => compactionDone(pending.eventId)}
            >
              我知道了
            </button>
          ) : (
            /* 未超时：展示跳过与确认继续按钮 */
            <>
              <button
                type="button"
                disabled={busy}
                className="px-3.5 py-1.5 rounded-lg border border-edge hover:bg-panel3 text-inkdim hover:text-ink text-[12px] font-medium transition-colors"
                onClick={handleSkip}
              >
                跳过本次压缩
              </button>
              <button
                type="button"
                disabled={busy}
                className="px-4 py-1.5 rounded-lg bg-accent hover:bg-blue-500 text-white text-[12px] font-medium transition-colors flex items-center gap-1.5 shadow-sm"
                onClick={handleConfirm}
              >
                {busy ? <RotateCw size={13} className="animate-spin" /> : <Check size={14} />}
                <span>确认并应用（继续执行）</span>
              </button>
            </>
          )}
        </div>
      </div>
    </div>
  );
}

/** 渲染已完成压缩的历史折叠卡片 */
export function CompactedHistoryCard({
  compaction,
}: {
  compaction: SessionCompaction;
}) {
  const [expanded, setExpanded] = useState(false);

  return (
    <div className="my-2 mx-auto w-full max-w-[820px] rounded-xl border border-edge/80 bg-panel2/70 overflow-hidden shadow-xs">
      <button
        type="button"
        className="w-full px-4 py-2.5 flex items-center justify-between text-left hover:bg-panel3/50 transition-colors"
        onClick={() => setExpanded(!expanded)}
      >
        <div className="flex items-center gap-2.5 text-[12px]">
          <div className="w-5 h-5 rounded-md bg-accent/15 text-accent flex items-center justify-center">
            <Sparkles size={12} />
          </div>
          <span className="font-medium text-ink">
            历史对话已压缩（第 {compaction.startSeq} ~ {compaction.endSeq} 条消息）
          </span>
          <span className="text-inkdim text-[11px]">
            · 释放约 {compaction.tokensBefore.toLocaleString()} tokens
          </span>
        </div>
        <div className="flex items-center gap-1 text-[11px] text-accent">
          <span>{expanded ? "收起备忘" : "查看备忘录"}</span>
          {expanded ? <ChevronDown size={13} /> : <ChevronRight size={13} />}
        </div>
      </button>

      {expanded && (
        <div className="px-4 pb-4 pt-2 border-t border-edge/60 bg-panel/50 text-[13px] leading-relaxed animate-in fade-in duration-150">
          <div className="text-[11px] text-inkdim mb-2 font-mono">
            压缩生成时间：{new Date(compaction.createdAt).toLocaleString("zh-CN")}
          </div>
          <Markdown content={compaction.summaryMarkdown} />
        </div>
      )}
    </div>
  );
}
