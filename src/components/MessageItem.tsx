import { useEffect, useState } from "react";
import { ipc } from "../ipc";
import { useStore } from "../store";
import type { Message, ToolEvent, TurnMetrics } from "../types";
import { Markdown } from "./Markdown";
import { ToolCard } from "./ToolCard";
import { Brain, ChevronRight, Pencil, Copy, Check, Clock, Zap } from "./Icons";

function formatDuration(ms?: number | null): string {
  if (ms == null || isNaN(ms)) return "";
  if (ms < 0) ms = 0;
  if (ms < 1000) return `${(ms / 1000).toFixed(1)}s`;
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)}s`;
  const mins = Math.floor(ms / 60_000);
  const secs = Math.floor((ms % 60_000) / 1000);
  return `${mins}分${secs}秒`;
}

function formatTokens(n?: number | null): string {
  if (n == null || isNaN(n)) return "0";
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(2)}M`;
  if (n >= 10_000) return `${(n / 1_000).toFixed(1)}k`;
  return n.toLocaleString("zh-CN");
}

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
    <div className="rounded-xl border border-edge/80 bg-panel2/40 overflow-hidden hover:border-edge transition-colors shadow-sm">
      <button
        className="w-full flex items-center gap-2 px-3 py-2 text-left hover:bg-panel2 transition-colors select-none"
        onClick={() => setOpen(!open)}
      >
        <ChevronRight
          size={13}
          className={`transition-transform duration-150 text-inkdim shrink-0 ${open ? "rotate-90" : ""}`}
        />
        <Brain size={14} className="text-purple-400 shrink-0" />
        <span className="text-[12px] font-medium text-inkdim">思考过程</span>
        {streaming && open && (
          <span className="flex gap-1 ml-1.5 items-center">
            <span className="w-1 h-1 rounded-full bg-purple-400 animate-bounce" style={{ animationDelay: "0ms" }} />
            <span className="w-1 h-1 rounded-full bg-purple-400 animate-bounce" style={{ animationDelay: "120ms" }} />
            <span className="w-1 h-1 rounded-full bg-purple-400 animate-bounce" style={{ animationDelay: "240ms" }} />
          </span>
        )}
      </button>
      {open && (
        <div className="px-3.5 pb-3 text-[13px] text-inkdim whitespace-pre-wrap leading-relaxed border-t border-edge/40 pt-2 font-mono text-[12px] opacity-90">{text}</div>
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
  turnMetrics,
}: {
  msg: Message;
  isLastUser: boolean;
  streaming: boolean;
  readOnly: boolean;
  running: boolean;
  /** 临时空间对话：合并点（含）之前的消息不可编辑重发 */
  editBlocked?: boolean;
  turnMetrics?: TurnMetrics;
}) {
  const pushToast = useStore((s) => s.pushToast);
  const setShowTokenStatsModal = useStore((s) => s.setShowTokenStatsModal);
  const [editing, setEditing] = useState(false);
  const [editText, setEditText] = useState("");
  const [copied, setCopied] = useState(false);
  const [streamDuration, setStreamDuration] = useState<number>(0);
  const [liveTurnDuration, setLiveTurnDuration] = useState<number>(0);

  useEffect(() => {
    if (!streaming) return;
    const start = Date.now();
    const timer = setInterval(() => {
      setStreamDuration(Date.now() - start);
    }, 100);
    return () => clearInterval(timer);
  }, [streaming]);

  // 整轮执行耗时实时计时（从用户发消息起持续计时，包含工具执行、测试命令与多步骤直至结束）
  useEffect(() => {
    if (!turnMetrics?.isCurrentRunningTurn || !turnMetrics.turnStartTime) {
      setLiveTurnDuration(0);
      return;
    }
    const update = () => {
      setLiveTurnDuration(Math.max(0, Date.now() - turnMetrics.turnStartTime));
    };
    update();
    const timer = setInterval(update, 100);
    return () => clearInterval(timer);
  }, [turnMetrics?.isCurrentRunningTurn, turnMetrics?.turnStartTime]);

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
            <div className="bg-panel2 border border-accent/60 rounded-xl p-3 shadow-md">
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
                <button className="px-3 py-1.5 rounded-lg text-inkdim hover:bg-panel3 text-[13px] transition-colors" onClick={() => setEditing(false)}>
                  取消
                </button>
                <button
                  className="px-3 py-1.5 rounded-lg bg-accent hover:bg-blue-500 text-white text-[13px] transition-colors shadow-sm"
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
              className="opacity-0 group-hover:opacity-100 mt-2 w-7 h-7 rounded-lg hover:bg-panel3 flex items-center justify-center text-inkdim hover:text-ink shrink-0 transition-opacity"
              title="编辑并重新发送（其后的消息将被作废）"
              onClick={() => {
                setEditText(msg.content ?? "");
                setEditing(true);
              }}
            >
              <Pencil size={13} />
            </button>
          )}
          <div className="bg-panel2 border border-edge rounded-2xl px-4 py-2.5 shadow-sm text-ink">
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

    const isRunningTurn = !!turnMetrics?.isCurrentRunningTurn;
    const isTurnEnd = turnMetrics?.isTurnEnd ?? true;
    const stepCount = turnMetrics?.turnStepCount ?? 1;
    const stepIndex = turnMetrics?.stepIndex ?? 1;
    const stepDuration = msg.durationMs ?? msg.usage?.durationMs ?? null;
    const turnDuration = turnMetrics?.turnDurationMs ?? stepDuration;
    const totalTokens = msg.totalTokens ?? (msg.usage?.totalTokens || (msg.usage?.inputEst || 0) + (msg.usage?.outputEst || 0)) ?? 0;
    const promptTokens = msg.promptTokens ?? (msg.usage?.promptTokens || msg.usage?.inputEst) ?? 0;
    const completionTokens = msg.completionTokens ?? (msg.usage?.completionTokens || msg.usage?.outputEst) ?? 0;
    const turnTokens = turnMetrics?.turnTokens ?? totalTokens;

    const hasTokens = (isTurnEnd ? turnTokens : totalTokens) > 0;
    const hasTurnDuration = isTurnEnd && turnDuration != null && turnDuration > 0;
    const hasStepDuration = !isTurnEnd && stepDuration != null && stepDuration > 0;
    const showDuration = isRunningTurn || streaming || hasTurnDuration || hasStepDuration;

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
          </div>
        )}

        {/* 对话状态与指标栏：复制、耗时（整轮与单步）、Token 消耗 */}
        {(hasContent || hasTokens || showDuration || isRunningTurn || streaming) && (
          <div className="flex items-center gap-2.5 text-[11px] text-inkdim select-none mt-0.5 px-0.5">
            {!streaming && hasContent && (
              <button
                className="inline-flex items-center gap-1 transition-opacity select-none px-1.5 py-0.5 rounded hover:bg-panel2 hover:text-ink"
                onClick={() => {
                  navigator.clipboard.writeText(msg.content ?? "").then(
                    () => {
                      setCopied(true);
                      setTimeout(() => setCopied(false), 1500);
                      pushToast("已复制到剪贴板");
                    },
                    () => pushToast("复制失败")
                  );
                }}
                title="复制回复正文"
              >
                {copied ? <Check size={12} className="text-green-400" /> : <Copy size={12} />}
                <span>{copied ? "已复制" : "复制"}</span>
              </button>
            )}

            {/* 耗时显示：区分整轮执行总耗时与单步耗时，正在执行中持续跳动 */}
            {isRunningTurn ? (
              <span
                className="inline-flex items-center gap-1 text-accent font-mono cursor-default select-none animate-pulse"
                title="整轮执行持续耗时：从发送消息起，持续跨越所有工具执行与计划步骤"
              >
                <Clock size={12} className="animate-spin text-accent" />
                <span>正在执行... {formatDuration(liveTurnDuration) || "0.1s"}</span>
              </span>
            ) : streaming ? (
              <span className="inline-flex items-center gap-1 text-accent font-mono">
                <Clock size={12} className="animate-spin" />
                <span>{formatDuration(streamDuration) || "0.1s"}</span>
              </span>
            ) : isTurnEnd && hasTurnDuration ? (
              <span
                className="inline-flex items-center gap-1 text-inkdim/90 hover:text-ink transition-colors font-mono cursor-default select-none"
                title={`本次完整处理耗时: ${turnDuration} ms\n从用户发送消息到计划全部执行完毕${stepCount > 1 ? `\n包含 ${stepCount} 个执行步骤` : ""}\n本次总消耗: ${(Number(turnTokens) || 0).toLocaleString()} tokens`}
              >
                <Clock size={12} className="text-emerald-400/90" />
                <span className="font-medium text-ink/90">总耗时 {formatDuration(turnDuration)}</span>
                {stepCount > 1 && (
                  <span className="text-inkdim text-[10px] opacity-75">({stepCount}步)</span>
                )}
              </span>
            ) : !isTurnEnd && hasStepDuration ? (
              <span
                className="inline-flex items-center gap-1 text-inkdim/70 hover:text-ink transition-colors font-mono cursor-default select-none"
                title={`第 ${stepIndex} 步耗时: ${stepDuration} ms\n本步骤消耗: ${(Number(totalTokens) || 0).toLocaleString()} tokens`}
              >
                <Clock size={12} className="text-inkdim/60" />
                <span>步骤 {formatDuration(stepDuration)}</span>
              </span>
            ) : null}

            {/* Token 用量显示：轮次结束展示整轮累计 Token，中间步骤展示单步 Token */}
            {!isRunningTurn && !streaming && (
              isTurnEnd ? (
                turnTokens > 0 && (
                  <span
                    role="button"
                    tabIndex={0}
                    onClick={() => setShowTokenStatsModal(true)}
                    className="inline-flex items-center gap-1 text-inkdim/80 hover:text-ink transition-colors font-mono cursor-pointer select-none"
                    title={`本次对话累计消耗: ${(Number(turnTokens) || 0).toLocaleString()} tokens${
                      (turnMetrics?.turnPromptTokens ?? 0) > 0 || (turnMetrics?.turnCompletionTokens ?? 0) > 0
                        ? `\n输入: ${(Number(turnMetrics?.turnPromptTokens ?? promptTokens) || 0).toLocaleString()} · 输出: ${(Number(turnMetrics?.turnCompletionTokens ?? completionTokens) || 0).toLocaleString()}`
                        : ""
                    }${stepCount > 1 ? `\n(已汇总本轮全部 ${stepCount} 个步骤)` : ""}\n点击打开 Token 消耗统计看板`}
                  >
                    <Zap size={12} className="text-amber-400/80 shrink-0" />
                    <span>{formatTokens(turnTokens)} tokens</span>
                    {stepCount > 1 && (
                      <span className="text-inkdim text-[10px] opacity-75">累计</span>
                    )}
                  </span>
                )
              ) : (
                totalTokens > 0 && (
                  <span
                    role="button"
                    tabIndex={0}
                    onClick={() => setShowTokenStatsModal(true)}
                    className="inline-flex items-center gap-1 text-inkdim/70 hover:text-ink transition-colors font-mono cursor-pointer select-none"
                    title={`本步骤消耗: ${(Number(totalTokens) || 0).toLocaleString()} tokens\n输入: ${(Number(promptTokens) || 0).toLocaleString()} · 输出: ${(Number(completionTokens) || 0).toLocaleString()}\n点击打开 Token 消耗统计看板`}
                  >
                    <Zap size={12} className="text-amber-400/60 shrink-0" />
                    <span>{formatTokens(totalTokens)} tokens</span>
                  </span>
                )
              )
            )}
          </div>
        )}
      </div>
    );
  }

  return null;
}
