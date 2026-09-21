import { useState, useMemo, useEffect, useRef } from "react";
import type { Message, TurnMetrics, SessionCompaction } from "../types";
import { MessageItem } from "./MessageItem";
import {
  ChevronRight,
  Loader2,
  Wrench,
  Clock,
  Coins,
} from "./Icons";

export type GroupedTimelineItem =
  | { type: "compaction"; compaction: SessionCompaction }
  | { type: "message"; msg: Message }
  | {
      type: "process";
      id: string;
      steps: Message[];
      turnMetrics?: TurnMetrics;
    };

/**
 * 将平铺的时间线消息按轮次聚合成：用户提问 -> 中间执行过程 (多步折叠) -> 最终交付成果
 */
export function groupTimelineItems(
  items: (
    | { type: "message"; msg: Message }
    | { type: "compaction"; compaction: SessionCompaction }
  )[],
  turnMetricsMap: Map<string, TurnMetrics>,
  _running: boolean
): GroupedTimelineItem[] {
  const result: GroupedTimelineItem[] = [];
  let currentAssistantSteps: Message[] = [];

  const flushAssistantSteps = () => {
    if (currentAssistantSteps.length === 0) return;

    // 单步 assistant 消息：直接普通消息呈现
    if (currentAssistantSteps.length === 1) {
      result.push({ type: "message", msg: currentAssistantSteps[0] });
      currentAssistantSteps = [];
      return;
    }

    // 多步 assistant 消息：
    // 判断最后一条消息是否为没有工具调用的交付结果
    const lastMsg = currentAssistantSteps[currentAssistantSteps.length - 1];
    const lastHasTools =
      (lastMsg.toolEvents?.length ?? 0) > 0 ||
      (Array.isArray(lastMsg.toolCalls) && lastMsg.toolCalls.length > 0);

    if (!lastHasTools) {
      // 最后一条是无工具的最终答复：前面所有消息收拢进执行过程，最后一条独立作为最终结果
      const processSteps = currentAssistantSteps.slice(0, currentAssistantSteps.length - 1);
      const metrics =
        turnMetricsMap.get(lastMsg.id) ?? turnMetricsMap.get(processSteps[0]?.id);
      result.push({
        type: "process",
        id: `process-${processSteps[0].id}`,
        steps: processSteps,
        turnMetrics: metrics,
      });
      result.push({ type: "message", msg: lastMsg });
    } else {
      // 全部消息均包含工具调用（如正在多步执行中，或中断在工具调用步骤）
      const metrics =
        turnMetricsMap.get(lastMsg.id) ??
        turnMetricsMap.get(currentAssistantSteps[0]?.id);
      result.push({
        type: "process",
        id: `process-${currentAssistantSteps[0].id}`,
        steps: currentAssistantSteps,
        turnMetrics: metrics,
      });
    }

    currentAssistantSteps = [];
  };

  for (const item of items) {
    if (item.type === "compaction") {
      flushAssistantSteps();
      result.push(item);
    } else if (item.msg.role === "assistant") {
      currentAssistantSteps.push(item.msg);
    } else {
      // user 消息或其他角色
      flushAssistantSteps();
      result.push(item);
    }
  }

  flushAssistantSteps();
  return result;
}

interface ExecutionProcessBlockProps {
  steps: Message[];
  isRunning: boolean;
  sessionWorkspace?: string;
  streamingMsgId?: string;
  turnMetrics?: TurnMetrics;
  readOnly?: boolean;
  turnMetricsMap?: Map<string, TurnMetrics>;
}

export function ExecutionProcessBlock({
  steps,
  isRunning,
  streamingMsgId,
  turnMetrics,
  readOnly,
  turnMetricsMap,
}: ExecutionProcessBlockProps) {
  // 用户手动切换过展开/收起时，优先使用用户意图；否则运行中默认展开，完成后默认折叠
  const [userToggled, setUserToggled] = useState<boolean | null>(null);
  const isOpen = userToggled !== null ? userToggled : isRunning;

  // 当运行状态从 running 变为 false 时，重置用户手动干预状态，确保自动优雅折叠
  const wasRunningRef = useRef(isRunning);
  useEffect(() => {
    if (wasRunningRef.current && !isRunning) {
      setUserToggled(null);
    }
    wasRunningRef.current = isRunning;
  }, [isRunning]);

  const stepCount = steps.length;
  const toolCount = useMemo(() => {
    return steps.reduce((sum, s) => {
      const evCount = s.toolEvents?.length ?? 0;
      const tcCount = Array.isArray(s.toolCalls) ? s.toolCalls.length : 0;
      return sum + Math.max(evCount, tcCount);
    }, 0);
  }, [steps]);

  // 计算用时与 Token
  const durationMs = useMemo(() => {
    if (turnMetrics?.turnDurationMs != null && turnMetrics.turnDurationMs > 0) {
      return turnMetrics.turnDurationMs;
    }
    const sum = steps.reduce((acc, s) => acc + (s.durationMs ?? 0), 0);
    return sum > 0 ? sum : null;
  }, [turnMetrics?.turnDurationMs, steps]);

  const tokens = useMemo(() => {
    if (turnMetrics?.turnTokens != null && turnMetrics.turnTokens > 0) {
      return turnMetrics.turnTokens;
    }
    return steps.reduce((acc, s) => {
      const t =
        s.totalTokens ??
        (s.usage?.totalTokens || (s.usage?.inputEst || 0) + (s.usage?.outputEst || 0)) ??
        0;
      return acc + (Number(t) || 0);
    }, 0);
  }, [turnMetrics?.turnTokens, steps]);

  const durationStr =
    durationMs != null ? `${(durationMs / 1000).toFixed(1)}s` : null;
  const tokensStr = tokens > 0 ? `${tokens.toLocaleString()} tokens` : null;

  return (
    <div className="rounded-2xl border border-edge/70 bg-panel2/40 hover:border-edge transition-all shadow-sm overflow-hidden select-none my-1">
      {/* 折叠栏头部 */}
      <button
        type="button"
        onClick={() => setUserToggled(!isOpen)}
        className="w-full flex items-center justify-between px-3.5 py-2.5 text-left group hover:bg-panel2/70 transition-colors"
      >
        <div className="flex items-center gap-2.5 min-w-0">
          <ChevronRight
            size={14}
            className={`text-inkdim transition-transform duration-200 shrink-0 ${
              isOpen ? "rotate-90" : ""
            }`}
          />
          {isRunning ? (
            <Loader2 size={15} className="text-blue-400 animate-spin shrink-0" />
          ) : (
            <Wrench size={15} className="text-accent shrink-0" />
          )}
          <span className="text-[13px] font-medium text-ink truncate">
            {isRunning
              ? `正在执行第 ${stepCount} 步…`
              : `执行过程 (${stepCount} 个步骤)`}
          </span>
          {toolCount > 0 && (
            <span className="text-[11px] px-1.5 py-0.5 rounded bg-panel3 text-inkdim border border-edge/40 shrink-0">
              {toolCount} 次工具调用
            </span>
          )}
          {isRunning && (
            <span className="text-[10px] px-1.5 py-0.5 rounded bg-blue-500/15 text-blue-400 border border-blue-500/30 animate-pulse shrink-0 font-medium">
              运行中
            </span>
          )}
        </div>

        <div className="flex items-center gap-2 text-[11.5px] text-inkdim shrink-0">
          {durationStr && (
            <span className="hidden sm:inline-flex items-center gap-1 font-mono">
              <Clock size={11} className="text-inkdim/60" />
              {durationStr}
            </span>
          )}
          {tokensStr && (
            <span className="hidden md:inline-flex items-center gap-1 font-mono">
              <Coins size={11} className="text-inkdim/60" />
              {tokensStr}
            </span>
          )}
          <span className="text-accent group-hover:underline ml-1 font-medium text-[12px]">
            {isOpen ? "收起详情" : "展开详情"}
          </span>
        </div>
      </button>

      {/* 展开的执行步骤明细 */}
      {isOpen && (
        <div className="border-t border-edge/40 px-3.5 pb-3.5 pt-2 flex flex-col gap-3.5 bg-panel/30">
          {steps.map((step, idx) => (
            <div
              key={step.id}
              className="relative pl-3 border-l-2 border-edge/60 hover:border-accent/50 transition-colors"
            >
              <div className="flex items-center justify-between text-[11px] text-inkdim/70 font-mono mb-1.5 select-none">
                <span className="font-medium text-inkdim/90">
                  步骤 {idx + 1} / {stepCount}
                </span>
                {step.durationMs != null && step.durationMs > 0 && (
                  <span>{(step.durationMs / 1000).toFixed(1)}s</span>
                )}
              </div>
              <MessageItem
                msg={step}
                isProcessStep={true}
                streaming={isRunning && step.id === streamingMsgId}
                readOnly={readOnly}
                running={isRunning}
                turnMetrics={turnMetricsMap?.get(step.id)}
              />
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
