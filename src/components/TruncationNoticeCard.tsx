import { useState } from "react";
import { useStore } from "../store";
import type { TruncationNotice } from "../types";
import { AlertTriangle, ChevronDown, ChevronRight, Scissors, X } from "./Icons";

export function TruncationNoticeList() {
  const currentId = useStore((s) => s.currentId);
  const notices = useStore((s) =>
    currentId ? s.truncationNotices.filter((n) => n.sessionId === currentId) : []
  );
  const dismissNotice = useStore((s) => s.dismissTruncationNotice);
  const clearSessionNotices = useStore((s) => s.clearSessionTruncationNotices);

  if (notices.length === 0) return null;

  return (
    <div className="border-b border-amber-500/20 bg-amber-500/5 px-4 py-2.5 flex flex-col gap-2.5 shrink-0 animate-in fade-in duration-200">
      <div className="max-w-[820px] mx-auto w-full flex flex-col gap-2">
        {notices.length > 1 && (
          <div className="flex items-center justify-between text-xs text-amber-300/80 px-1">
            <span>共有 {notices.length} 条上下文截断记录</span>
            <button
              onClick={() => currentId && clearSessionNotices(currentId)}
              className="text-amber-400 hover:text-amber-300 underline text-xs transition-colors"
            >
              全部关闭
            </button>
          </div>
        )}
        {notices.map((notice) => (
          <TruncationNoticeItem
            key={notice.id}
            notice={notice}
            onDismiss={() => dismissNotice(notice.id)}
          />
        ))}
      </div>
    </div>
  );
}

function TruncationNoticeItem({
  notice,
  onDismiss,
}: {
  notice: TruncationNotice;
  onDismiss: () => void;
}) {
  const [expanded, setExpanded] = useState(false);

  return (
    <div className="bg-panel border border-amber-500/30 rounded-xl p-3 shadow-md text-xs flex flex-col gap-2">
      <div className="flex items-start justify-between gap-3">
        <div className="flex items-center gap-2">
          <div className="p-1 rounded-lg bg-amber-500/10 text-amber-400 shrink-0">
            <AlertTriangle size={15} />
          </div>
          <div className="flex items-center gap-2 flex-wrap">
            <span className="font-semibold text-amber-300 text-sm">
              触发原子轮次截断
            </span>
            <span className="inline-flex items-center gap-1 px-1.5 py-0.5 rounded bg-amber-500/15 text-amber-400 border border-amber-500/20 font-mono">
              <Scissors size={11} />
              已移出 {notice.droppedTurns} 轮 ({notice.droppedMessages} 条消息)
            </span>
          </div>
        </div>

        <div className="flex items-center gap-1 shrink-0">
          <button
            onClick={onDismiss}
            className="p-1 rounded-md text-inkdim hover:text-ink hover:bg-panel2 transition-colors"
            title="关闭此提醒"
            aria-label="关闭此提醒"
          >
            <X size={15} />
          </button>
        </div>
      </div>

      <p className="text-inkdim leading-relaxed">
        由于对话历史超出上下文限制（预估{" "}
        <span className="text-amber-300 font-mono font-medium">
          {notice.estTokensBefore.toLocaleString()}
        </span>{" "}
        / 上限{" "}
        <span className="text-ink font-mono font-medium">
          {notice.tokenLimit.toLocaleString()}
        </span>{" "}
        Tokens），系统已按完整轮次安全移出最早历史对话，释放约{" "}
        <span className="text-amber-300 font-mono font-medium">
          {notice.droppedTokens.toLocaleString()}
        </span>{" "}
        Tokens。当前对话不受影响，继续正常响应。
      </p>

      {/* 简要指标概览 */}
      <div className="grid grid-cols-3 gap-2 py-1.5 px-2.5 rounded-lg bg-panel2/60 border border-border/50 text-center font-mono">
        <div>
          <span className="text-inkdim block text-[10px]">截断前预估</span>
          <span className="text-ink text-[11px]">
            {notice.estTokensBefore.toLocaleString()}
          </span>
        </div>
        <div>
          <span className="text-amber-400/90 block text-[10px]">移出释放</span>
          <span className="text-amber-400 font-medium text-[11px]">
            -{notice.droppedTokens.toLocaleString()}
          </span>
        </div>
        <div>
          <span className="text-emerald-400/90 block text-[10px]">当前上下文</span>
          <span className="text-emerald-400 font-medium text-[11px]">
            {notice.estTokensAfter.toLocaleString()}
          </span>
        </div>
      </div>

      {/* 可折叠详情：最早被淘汰的内容预览 */}
      {notice.firstPreview && (
        <div className="border-t border-border/40 pt-1.5 flex flex-col gap-1">
          <button
            onClick={() => setExpanded((v) => !v)}
            className="flex items-center gap-1 text-[11px] text-inkdim hover:text-ink transition-colors self-start"
          >
            {expanded ? <ChevronDown size={13} /> : <ChevronRight size={13} />}
            <span>{expanded ? "收起移出内容预览" : "查看最早被移出的消息预览"}</span>
          </button>
          {expanded && (
            <div className="p-2 rounded bg-panel2/80 text-inkdim text-[11px] font-mono leading-relaxed max-h-24 overflow-y-auto break-all whitespace-pre-wrap border border-border/40">
              {notice.firstPreview}
            </div>
          )}
        </div>
      )}

      <div className="flex items-center justify-between pt-1 border-t border-border/30 text-[11px] text-inkdim">
        <span>{new Date(notice.createdAt).toLocaleTimeString()}</span>
        <button
          onClick={onDismiss}
          className="px-2.5 py-1 rounded bg-panel2 hover:bg-panel border border-border text-ink hover:text-accent transition-colors font-medium"
        >
          知道了
        </button>
      </div>
    </div>
  );
}
