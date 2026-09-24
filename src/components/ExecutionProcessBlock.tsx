import { useState, useMemo, useEffect, useRef } from "react";
import type { Message, TurnMetrics, SessionCompaction } from "../types";
import { MessageItem } from "./MessageItem";
import {
  ChevronRight,
  Loader2,
  Wrench,
  Clock,
  Coins,
  Brain,
  Zap,
} from "./Icons";

export type GroupedTimelineItem =
  | { type: "compaction"; compaction: SessionCompaction }
  | { type: "message"; msg: Message }
  | {
      type: "process";
      id: string;
      steps: Message[];
      turnMetrics?: TurnMetrics;
      isRunning?: boolean;
    };

/**
 * 将平铺的时间线消息按轮次聚合成：用户提问 -> 中间执行过程 (多步折叠) -> 最终交付成果
 * 方案 B：执行与思考过程全内置于抽屉内流式生长，运行期间全程保持展开与稳态，任务彻底完成后自动折叠
 */
export function groupTimelineItems(
  items: (
    | { type: "message"; msg: Message }
    | { type: "compaction"; compaction: SessionCompaction }
  )[],
  turnMetricsMap: Map<string, TurnMetrics>,
  running: boolean
): GroupedTimelineItem[] {
  const result: GroupedTimelineItem[] = [];
  let currentAssistantSteps: Message[] = [];

  const hasAnyTools = (msg: Message) =>
    (msg.toolEvents?.length ?? 0) > 0 ||
    (Array.isArray(msg.toolCalls) && msg.toolCalls.length > 0);

  const hasAnyReasoning = (msg: Message) =>
    Boolean(msg.reasoning && msg.reasoning.trim().length > 0);

  // 定位整个时间线中最后一条 assistant 消息的绝对索引
  let lastAssistantIdx = -1;
  for (let i = items.length - 1; i >= 0; i--) {
    const it = items[i];
    if (it.type === "message" && it.msg.role === "assistant") {
      lastAssistantIdx = i;
      break;
    }
  }

  const flushAssistantSteps = (isCurrentRunningTurn: boolean) => {
    if (currentAssistantSteps.length === 0) return;

    // ----------------------------------------------------
    // 场景 1：当前轮次正在运行中 (running === true 且是当前活跃尾部轮次)
    // ----------------------------------------------------
    if (isCurrentRunningTurn) {
      const hasProcessNature =
        currentAssistantSteps.length > 1 ||
        currentAssistantSteps.some((m) => hasAnyTools(m) || hasAnyReasoning(m));

      if (hasProcessNature) {
        // 方案 B 核心：一旦具备执行或思考特征，所有步骤自始至终稳定挂载在同一个抽屉内，流式打字与步骤递增全部内置，绝不在外面闪现
        const firstId = currentAssistantSteps[0].id;
        const lastMsg = currentAssistantSteps[currentAssistantSteps.length - 1];
        const metrics =
          turnMetricsMap.get(lastMsg.id) ?? turnMetricsMap.get(firstId);

        result.push({
          type: "process",
          id: `process-${firstId}`,
          steps: currentAssistantSteps,
          turnMetrics: metrics,
          isRunning: true,
        });
        currentAssistantSteps = [];
        return;
      }

      // 纯单步极简普通文本流式（无工具、无思考）：直接在外部作为普通消息流式呈现
      result.push({ type: "message", msg: currentAssistantSteps[0] });
      currentAssistantSteps = [];
      return;
    }

    // ----------------------------------------------------
    // 场景 2：轮次已完结态 (Completed Turn)
    // ----------------------------------------------------
    // 2.1 单步 assistant 消息
    if (currentAssistantSteps.length === 1) {
      const singleMsg = currentAssistantSteps[0];
      const hasTools = hasAnyTools(singleMsg);
      const hasReasoning = hasAnyReasoning(singleMsg);

      if (hasTools) {
        // 单步工具调用：放入已完成折叠抽屉
        const metrics = turnMetricsMap.get(singleMsg.id);
        result.push({
          type: "process",
          id: `process-${singleMsg.id}`,
          steps: [singleMsg],
          turnMetrics: metrics,
          isRunning: false,
        });
        if (singleMsg.content && singleMsg.content.trim().length > 0) {
          result.push({
            type: "message",
            msg: { ...singleMsg, reasoning: null, turnToolEvents: singleMsg.toolEvents },
          });
        }
      } else if (hasReasoning && singleMsg.content && singleMsg.content.trim().length > 0) {
        // 单步同时具备思考过程与回复正文：思考过程收归抽屉并默认折叠，外部仅留干净的正文交付
        const metrics = turnMetricsMap.get(singleMsg.id);
        result.push({
          type: "process",
          id: `process-${singleMsg.id}`,
          steps: [{ ...singleMsg, id: `${singleMsg.id}-reasoning`, content: null, toolEvents: [], toolCalls: [] }],
          turnMetrics: metrics,
          isRunning: false,
        });
        result.push({
          type: "message",
          msg: { ...singleMsg, reasoning: null },
        });
      } else {
        // 纯单步普通文本答复
        result.push({ type: "message", msg: singleMsg });
      }
      currentAssistantSteps = [];
      return;
    }

    // 2.2 多步 assistant 消息
    const lastMsg = currentAssistantSteps[currentAssistantSteps.length - 1];
    const lastHasTools = hasAnyTools(lastMsg);
    const lastHasReasoning = hasAnyReasoning(lastMsg);

    const allTurnToolEvents = currentAssistantSteps.flatMap((m) => m.toolEvents || []);
    const turnRevertedAt = currentAssistantSteps.find((m) => m.revertedAt)?.revertedAt || lastMsg.revertedAt;

    if (!lastHasTools) {
      // 最后一条是无工具的交付答复：前序所有步骤（以及最后一条的思考过程，若有）收拢入执行过程，正文作为外部交付成果
      const processSteps = currentAssistantSteps.slice(0, currentAssistantSteps.length - 1);
      if (lastHasReasoning) {
        processSteps.push({
          ...lastMsg,
          id: `${lastMsg.id}-reasoning`,
          content: null,
          toolEvents: [],
          toolCalls: [],
        });
      }

      const metrics =
        turnMetricsMap.get(lastMsg.id) ?? turnMetricsMap.get(processSteps[0]?.id);

      result.push({
        type: "process",
        id: `process-${processSteps[0].id}`,
        steps: processSteps,
        turnMetrics: metrics,
        isRunning: false,
      });

      result.push({
        type: "message",
        msg: {
          ...lastMsg,
          reasoning: null,
          turnToolEvents: allTurnToolEvents,
          revertedAt: turnRevertedAt,
        },
      });
    } else {
      // 整轮所有步骤均包含工具调用（如中断在工具执行）
      const metrics =
        turnMetricsMap.get(lastMsg.id) ??
        turnMetricsMap.get(currentAssistantSteps[0]?.id);

      result.push({
        type: "process",
        id: `process-${currentAssistantSteps[0].id}`,
        steps: currentAssistantSteps,
        turnMetrics: metrics,
        isRunning: false,
      });

      if (lastMsg.content && lastMsg.content.trim().length > 0) {
        result.push({
          type: "message",
          msg: {
            ...lastMsg,
            reasoning: null,
            turnToolEvents: allTurnToolEvents,
            revertedAt: turnRevertedAt,
          },
        });
      }
    }

    currentAssistantSteps = [];
  };

  for (let i = 0; i < items.length; i++) {
    const item = items[i];
    if (item.type === "compaction") {
      flushAssistantSteps(false);
      result.push(item);
    } else if (item.msg.role === "assistant") {
      currentAssistantSteps.push(item.msg);
    } else {
      // user 消息或其他非 assistant 角色
      flushAssistantSteps(false);
      result.push(item);
    }
  }

  // 处理时间线尾部的当前轮次
  const isTailActive = running && lastAssistantIdx >= 0;
  flushAssistantSteps(isTailActive);
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

  const { promptTokens, completionTokens, cachedTokens, isEstimated } = useMemo(() => {
    let p = 0;
    let c = 0;
    let ck = 0;
    let est = false;
    for (const s of steps) {
      const pt = s.promptTokens ?? (s.usage?.promptTokens || s.usage?.inputEst) ?? 0;
      const ct = s.completionTokens ?? (s.usage?.completionTokens || s.usage?.outputEst) ?? 0;
      const ckt = s.cachedTokens ?? (s.usage?.cachedTokens || s.usage?.promptCacheHitTokens || s.usage?.cacheReadInputTokens) ?? 0;
      const isEst = s.isEstimated ?? (s.usage?.isEstimated || (s.usage?.inputEst != null && s.usage?.promptTokens == null)) ?? false;
      p += Number(pt) || 0;
      c += Number(ct) || 0;
      ck += Number(ckt) || 0;
      if (isEst) est = true;
    }
    return { promptTokens: p, completionTokens: c, cachedTokens: ck, isEstimated: est };
  }, [steps]);

  const cacheHitRate = promptTokens > 0 ? Math.round((cachedTokens / promptTokens) * 1000) / 10 : 0;

  const turnModifiedFilesCount = useMemo(() => {
    const paths = new Set<string>();
    for (const s of steps) {
      for (const ev of s.toolEvents ?? []) {
        if (
          (ev.toolName === "write_file" || ev.toolName === "edit_file") &&
          (ev.status === "success" || s.revertedAt)
        ) {
          const p = ev.params?.path ? String(ev.params.path) : "";
          if (p) paths.add(p);
        }
      }
    }
    return paths.size;
  }, [steps]);

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
          ) : toolCount === 0 ? (
            <Brain size={15} className="text-purple-400 shrink-0" />
          ) : (
            <Wrench size={15} className="text-accent shrink-0" />
          )}
          <span className="text-[13px] font-medium text-ink truncate">
            {isRunning
              ? (toolCount === 0 && stepCount === 1 ? "正在思考与回复…" : `正在执行第 ${stepCount} 步…`)
              : toolCount === 0
              ? `思考过程 (${stepCount} 个步骤)`
              : `执行过程 (${stepCount} 个步骤)`}
          </span>
          {toolCount > 0 && (
            <span className="text-[11px] px-1.5 py-0.5 rounded bg-panel3 text-inkdim border border-edge/40 shrink-0">
              {toolCount} 次工具调用
            </span>
          )}
          {turnModifiedFilesCount > 0 && (
            <span className="text-[11px] px-1.5 py-0.5 rounded bg-amber-500/15 text-amber-400 border border-amber-500/30 shrink-0 font-medium">
              改动 {turnModifiedFilesCount} 个文件
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
            <span
              className="hidden md:inline-flex items-center gap-1 font-mono"
              title={`执行过程总消耗: ${tokens.toLocaleString()} tokens ${isEstimated ? "(基于字符估算)" : "(模型实际返回)"}\n输入: ${promptTokens.toLocaleString()}${cachedTokens > 0 ? ` (缓存命中: ${cachedTokens.toLocaleString()} · ${cacheHitRate.toFixed(1)}%)` : ""} · 输出: ${completionTokens.toLocaleString()}`}
            >
              <Coins size={11} className="text-inkdim/60" />
              <span>{tokensStr}</span>
              {cacheHitRate > 0 && (
                <span className="px-1.5 py-0.5 rounded text-[10px] bg-cyan-500/10 text-cyan-400 font-medium border border-cyan-500/20 inline-flex items-center gap-1">
                  <Zap size={10} className="shrink-0" />
                  <span>{cacheHitRate.toFixed(1)}% 缓存</span>
                </span>
              )}
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
