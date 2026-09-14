import { useEffect, useState } from "react";
import { ipc } from "../ipc";
import { useStore } from "../store";
import type { Message, ToolEvent } from "../types";
import { Markdown } from "./Markdown";
import { ToolCard } from "./ToolCard";

/** 工具事件按 assistant 消息中 tool_calls 的顺序排列 */
function orderedEvents(msg: Message): ToolEvent[] {
  const evs = msg.toolEvents ?? [];
  if (!msg.toolCalls || !Array.isArray(msg.toolCalls) || msg.toolCalls.length === 0) {
    return evs;
  }
  const byCallId = new Map(evs.map((e) => [e.toolCallId ?? "", e]));
  const ordered: ToolEvent[] = [];
  for (const tc of msg.toolCalls) {
    const ev = byCallId.get(tc?.id ?? "");
    if (ev) {
      ordered.push(ev);
      byCallId.delete(tc.id);
    }
  }
  // 兜底：未能匹配的事件按原顺序附加
  for (const e of evs) {
    if (!ordered.includes(e)) ordered.push(e);
  }
  return ordered;
}

/** 思考过程折叠卡片：流式期间默认展开，便于实时可见；完成后默认折叠 */
function ReasoningBlock({ text, streaming }: { text: string; streaming: boolean }) {
  const [open, setOpen] = useState(streaming);
  // 流式结束后自动收起，避免思考正文长期占据对话空间
  useEffect(() => {
    if (!streaming && open) setOpen(false);
  }, [streaming]); // eslint-disable-line react-hooks/exhaustive-deps
  return (
    <div className="rounded-xl border border-edge bg-panel2/60 overflow-hidden">
      <button
        className="w-full flex items-center gap-2 px-3 py-2 text-left hover:bg-panel2"
        onClick={() => setOpen(!open)}
      >
        <span className="text-[11px] text-inkdim w-3 shrink-0">{open ? "▾" : "▸"}</span>
        <span className="text-[12px] text-inkdim">🧠 思考过程</span>
        {streaming && open && (
          <span className="flex gap-1 ml-1">
            <span className="w-1 h-1 rounded-full bg-inkdim animate-bounce" style={{ animationDelay: "0ms" }} />
            <span className="w-1 h-1 rounded-full bg-inkdim animate-bounce" style={{ animationDelay: "120ms" }} />
            <span className="w-1 h-1 rounded-full bg-inkdim animate-bounce" style={{ animationDelay: "240ms" }} />
          </span>
        )}
      </button>
      {open && (
        <div className="px-3 pb-3 text-[13px] text-inkdim whitespace-pre-wrap leading-relaxed">{text}</div>
      )}
    </div>
  );
}

export function MessageItem({
  msg,
  isLastUser,
  streaming,
  readOnly,
  running,
  editBlocked,
}: {
  msg: Message;
  isLastUser: boolean;
  streaming: boolean;
  readOnly: boolean;
  running: boolean;
  /** 临时空间对话：合并点（含）之前的消息不可编辑重发 */
  editBlocked?: boolean;
}) {
  const pushToast = useStore((s) => s.pushToast);
  const [editing, setEditing] = useState(false);
  const [editText, setEditText] = useState("");

  if (msg.role === "tool") return null; // 工具结果已由 ToolCard 呈现

  if (msg.role === "system") {
    return (
      <div className="self-center max-w-[92%] border border-red-500/40 bg-red-500/10 rounded-xl px-4 py-3">
        <div className="text-[13px] text-red-300 whitespace-pre-wrap leading-relaxed">{msg.content}</div>
      </div>
    );
  }

  if (msg.role === "user") {
    if (editing) {
      return (
        <div className="flex justify-end">
          <div className="max-w-[85%] w-full">
            <div className="bg-panel2 border border-accent/60 rounded-xl p-3">
              <textarea
                autoFocus
                className="w-full bg-transparent outline-none resize-none text-[14px] leading-relaxed min-h-[60px]"
                value={editText}
                onChange={(e) => setEditText(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" && !e.altKey && !(e.nativeEvent as any).isComposing) {
                    e.preventDefault();
                    void ipc
                      .editAndResend(msg.sessionId, msg.id, editText)
                      .then(() => setEditing(false))
                      .catch((err) => {
                        pushToast(String(err));
                      });
                  } else if (e.key === "Escape") {
                    setEditing(false);
                  }
                }}
              />
              <div className="flex justify-end gap-2 mt-2">
                <button className="px-3 py-1.5 rounded-lg text-inkdim hover:bg-panel3 text-[13px]" onClick={() => setEditing(false)}>
                  取消
                </button>
                <button
                  className="px-3 py-1.5 rounded-lg bg-accent hover:bg-blue-500 text-white text-[13px]"
                  onClick={() =>
                    void ipc
                      .editAndResend(msg.sessionId, msg.id, editText)
                      .then(() => setEditing(false))
                      .catch((err) => pushToast(String(err)))
                  }
                >
                  重新发送
                </button>
              </div>
            </div>
          </div>
        </div>
      );
    }
    return (
      <div className="flex justify-end group relative">
        <div className="max-w-[85%] flex items-start gap-2">
          {isLastUser && !running && !readOnly && !editBlocked && (
            <button
              className="opacity-0 group-hover:opacity-100 mt-2 w-7 h-7 rounded-lg hover:bg-panel3 flex items-center justify-center text-inkdim shrink-0"
              title="编辑并重新发送（其后的消息将被作废）"
              onClick={() => {
                setEditText(msg.content ?? "");
                setEditing(true);
              }}
            >
              ✏️
            </button>
          )}
          <div className="bg-panel2 border border-edge rounded-xl px-4 py-2.5">
            <div className="whitespace-pre-wrap text-[14px] leading-relaxed">{msg.content}</div>
          </div>
        </div>
      </div>
    );
  }

  if (msg.role === "assistant") {
    const events = orderedEvents(msg);
    const hasContent = !!msg.content;
    const hasReasoning = !!msg.reasoning;
    return (
      <div className="flex flex-col gap-2">
        {events.map((ev) => (
          <ToolCard key={ev.id} ev={ev} />
        ))}
        {hasReasoning && <ReasoningBlock text={msg.reasoning!} streaming={streaming} />}
        {hasContent && (
          <div className="group relative">
            <Markdown content={msg.content!} />
            {streaming && <span className="stream-cursor" />}
            {!streaming && hasContent && (
              <button
                className="opacity-0 group-hover:opacity-100 text-[11px] text-inkdim hover:text-ink mt-1"
                onClick={() => {
                  navigator.clipboard.writeText(msg.content ?? "").then(
                    () => pushToast("已复制"),
                    () => pushToast("复制失败")
                  );
                }}
              >
                复制
              </button>
            )}
          </div>
        )}
      </div>
    );
  }

  return null;
}
