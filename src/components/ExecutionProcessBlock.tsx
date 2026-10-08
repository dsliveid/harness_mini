import { useState, useMemo, useEffect, useRef } from "react";
import type { Message, TurnMetrics, SessionCompaction, IntentAlignmentEvent, IntentInterventionRequest } from "../types";
import { MessageItem } from "./MessageItem";
import { IntentAlignmentCard } from "./IntentAlignmentCard";
import { useStore } from "../store";
import {
  ChevronRight,
  Loader2,
  Wrench,
  Clock,
  Coins,
  Brain,
  Zap,
  ShieldCheck,
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
      intentAlignment?: IntentAlignmentEvent;
    };

/** 从 reasoning 中提取历史记录中的意图对齐门控内容（若页面刷新或重启后在历史中） */
export function extractIntentAlignmentFromReasoning(reasoning?: string | null): IntentAlignmentEvent | undefined {
  if (!reasoning || !reasoning.includes("【意图剖析与规划闸门（质检通过）】")) return undefined;
  const match = reasoning.match(
    /【意图剖析与规划闸门（质检通过）】\s*(?:•|\*|-)?\s*意图理解:\s*([\s\S]*?)\s*(?:•|\*|-)?\s*拟定方案:\s*([\s\S]*?)(?:\n\n|\n|$)/
  );
  if (!match) return undefined;
  return {
    sessionId: "",
    runId: "",
    userMessage: "",
    status: "passed",
    currentAttempt: 1,
    maxRetries: 2,
    understanding: match[1].trim(),
    plan: match[2].trim(),
    score: 1.0,
    scorePercent: 100,
    reason: "前置意图对齐与规划质检已通过",
    passed: true,
  };
}

/**
 * 将平铺的时间线消息按轮次聚合成：用户提问 -> 中间执行过程 (含前置意图对齐门控与多步工具折叠) -> 最终交付成果
 * 方案 B：执行与思考过程全内置于抽屉内流式生长，运行期间全程保持展开与稳态，任务彻底完成后自动折叠
 */
export function groupTimelineItems(
  items: (
    | { type: "message"; msg: Message }
    | { type: "compaction"; compaction: SessionCompaction }
  )[],
  turnMetricsMap: Map<string, TurnMetrics>,
  running: boolean,
  intentAlignmentsMap?: Record<string, IntentAlignmentEvent>,
  activeIntentAlignment?: IntentAlignmentEvent,
  currentSessionId?: string | null
): GroupedTimelineItem[] {
  const result: GroupedTimelineItem[] = [];
  let currentAssistantSteps: Message[] = [];

  const hasAnyTools = (msg: Message) =>
    (msg.toolEvents?.length ?? 0) > 0 ||
    (Array.isArray(msg.toolCalls) && msg.toolCalls.length > 0);

  const hasAnyReasoning = (msg: Message) =>
    Boolean(msg.reasoning && msg.reasoning.trim().length > 0);

  const findIntentAlignmentForSteps = (steps: Message[]): IntentAlignmentEvent | undefined => {
    if (steps.length === 0) return undefined;
    const firstMsg = steps[0];
    if (firstMsg.runId && intentAlignmentsMap?.[firstMsg.runId]) {
      return intentAlignmentsMap[firstMsg.runId];
    }
    for (const s of steps) {
      const extracted = extractIntentAlignmentFromReasoning(s.reasoning);
      if (extracted) return extracted;
    }
    return undefined;
  };

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
      const turnAlignment = findIntentAlignmentForSteps(currentAssistantSteps) || activeIntentAlignment;
      const hasProcessNature =
        currentAssistantSteps.length > 1 ||
        currentAssistantSteps.some((m) => hasAnyTools(m) || hasAnyReasoning(m)) ||
        Boolean(turnAlignment);

      if (hasProcessNature) {
        // 方案 B 核心：一旦具备执行、思考或前置意图门控特征，所有步骤自始至终稳定挂载在同一个抽屉内
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
          intentAlignment: turnAlignment,
        });
        currentAssistantSteps = [];
        return;
      }

      // 纯单步极简普通文本流式（无工具、无思考、无前置门控）：直接在外部作为普通消息流式呈现
      result.push({ type: "message", msg: currentAssistantSteps[0] });
      currentAssistantSteps = [];
      return;
    }

    // ----------------------------------------------------
    // 场景 2：轮次已完结态 (Completed Turn)
    // ----------------------------------------------------
    const turnAlignment = findIntentAlignmentForSteps(currentAssistantSteps);

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
          intentAlignment: turnAlignment,
        });
        if (singleMsg.content && singleMsg.content.trim().length > 0) {
          result.push({
            type: "message",
            msg: { ...singleMsg, reasoning: null, turnToolEvents: singleMsg.toolEvents },
          });
        }
      } else if (turnAlignment) {
        // 具备意图对齐门控：门控卡片放入已完成折叠抽屉，外部展示交付答复
        const metrics = turnMetricsMap.get(singleMsg.id);
        result.push({
          type: "process",
          id: `process-${singleMsg.id}`,
          steps: hasReasoning
            ? [{ ...singleMsg, id: `${singleMsg.id}-reasoning`, content: null, toolEvents: [], toolCalls: [] }]
            : [],
          turnMetrics: metrics,
          isRunning: false,
          intentAlignment: turnAlignment,
        });
        result.push({
          type: "message",
          msg: { ...singleMsg, reasoning: null },
        });
      } else if (hasReasoning && singleMsg.content && singleMsg.content.trim().length > 0) {
        // 单步同时具备思考过程与回复正文：思考过程收归抽屉并默认折叠，外部仅留干净的正文交付
        const metrics = turnMetricsMap.get(singleMsg.id);
        result.push({
          type: "process",
          id: `process-${singleMsg.id}`,
          steps: [{ ...singleMsg, id: `${singleMsg.id}-reasoning`, content: null, toolEvents: [], toolCalls: [] }],
          turnMetrics: metrics,
          isRunning: false,
          intentAlignment: turnAlignment,
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
        id: `process-${processSteps[0]?.id || lastMsg.id}`,
        steps: processSteps,
        turnMetrics: metrics,
        isRunning: false,
        intentAlignment: turnAlignment,
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
        intentAlignment: turnAlignment,
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
      // 检查 runId 边界：若新步骤与当前累积步骤的 runId 明确不同，强制切断并结算前序抽屉，杜绝跨轮次混合
      if (
        currentAssistantSteps.length > 0 &&
        currentAssistantSteps[0].runId &&
        item.msg.runId &&
        currentAssistantSteps[0].runId !== item.msg.runId
      ) {
        flushAssistantSteps(false);
      }
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

  // 若当前正在运行中，且尚未产生 assistant 消息（例如正在进行前置意图对齐与质检门控，或首步思考准备中）
  if (running && (lastAssistantIdx < 0 || lastAssistantIdx < items.length - 1)) {
    const lastItem = items[items.length - 1];
    if (lastItem && lastItem.type === "message" && lastItem.msg.role === "user") {
      // 检查 activeIntentAlignment 是否匹配当前会话与最新提问
      const isRelevantAlignment = Boolean(
        activeIntentAlignment &&
          (activeIntentAlignment.status === "analyzing" ||
            activeIntentAlignment.status === "evaluating" ||
            activeIntentAlignment.status === "retrying" ||
            activeIntentAlignment.status === "requires_intervention" ||
            activeIntentAlignment.userMessage?.trim() === lastItem.msg.content?.trim())
      );

      result.push({
        type: "process",
        id: `process-running-${lastItem.msg.id}`,
        steps: [],
        isRunning: true,
        intentAlignment: isRelevantAlignment ? activeIntentAlignment : undefined,
      });
    }
  }

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
  intentAlignment?: IntentAlignmentEvent;
  pendingIntervention?: IntentInterventionRequest | null;
}

export function ExecutionProcessBlock({
  steps,
  isRunning,
  streamingMsgId,
  turnMetrics,
  readOnly,
  turnMetricsMap,
  intentAlignment: propAlignment,
  pendingIntervention: propIntervention,
}: ExecutionProcessBlockProps) {
  const currentSessionId = useStore((s) => s.currentId);

  // 解析意图对齐门控事件数据
  const storeAlignment = useStore((s) => {
    if (propAlignment) return propAlignment;
    const runId = steps[0]?.runId;
    if (runId && s.intentAlignments[runId]) return s.intentAlignments[runId];
    if (isRunning && currentSessionId && s.intentAlignments[currentSessionId]) {
      return s.intentAlignments[currentSessionId];
    }
    return undefined;
  });

  const resolvedAlignment = useMemo(() => {
    if (propAlignment) return propAlignment;
    if (storeAlignment) return storeAlignment;
    for (const s of steps) {
      const extracted = extractIntentAlignmentFromReasoning(s.reasoning);
      if (extracted) return extracted;
    }
    return undefined;
  }, [propAlignment, storeAlignment, steps]);

  const storePendingIntervention = useStore((s) => s.pendingIntentIntervention);
  const resolvedIntervention = useMemo(() => {
    if (propIntervention !== undefined) return propIntervention;
    if (!isRunning) return null;
    if (storePendingIntervention && storePendingIntervention.sessionId === currentSessionId) {
      return storePendingIntervention;
    }
    return null;
  }, [propIntervention, isRunning, storePendingIntervention, currentSessionId]);

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
  const isReverted = useMemo(() => steps.length > 0 && steps.some((s) => !!s.revertedAt), [steps]);

  const isAlignmentPassed = Boolean(resolvedAlignment?.passed || resolvedAlignment?.status === "passed");
  const isAlignmentInProgress = Boolean(
    resolvedAlignment &&
      (resolvedAlignment.status === "analyzing" ||
        resolvedAlignment.status === "evaluating" ||
        resolvedAlignment.status === "retrying" ||
        resolvedAlignment.status === "requires_intervention")
  );

  return (
    <div
      className={`rounded-2xl border border-edge/70 bg-panel2/40 hover:border-edge transition-all shadow-sm overflow-hidden my-1 ${
        isReverted ? "opacity-60 grayscale-[30%]" : ""
      }`}
    >
      {/* 折叠栏头部 */}
      <button
        type="button"
        onClick={() => setUserToggled(!isOpen)}
        className="w-full flex items-center justify-between px-3.5 py-2.5 text-left group hover:bg-panel2/70 transition-colors select-none"
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
          ) : isAlignmentPassed && toolCount === 0 && stepCount <= 1 ? (
            <ShieldCheck size={15} className="text-emerald-400 shrink-0" />
          ) : toolCount === 0 ? (
            <Brain size={15} className="text-purple-400 shrink-0" />
          ) : (
            <Wrench size={15} className="text-accent shrink-0" />
          )}

          <span className="text-[13px] font-medium text-ink truncate">
            {isRunning
              ? stepCount === 0
                ? isAlignmentInProgress
                  ? "正在进行意图对齐与规划质检…"
                  : "正在思考与制定执行计划…"
                : toolCount === 0 && stepCount === 1
                ? "正在思考与回复…"
                : `正在执行第 ${stepCount} 步…`
              : toolCount === 0
              ? resolvedAlignment
                ? stepCount > 0
                  ? `执行过程 (含意图质检 · ${stepCount} 个步骤)`
                  : "意图质检已完成"
                : `思考过程 (${stepCount} 个步骤)`
              : `执行过程 (${stepCount} 个步骤)`}
          </span>

          {isReverted && (
            <span className="text-[11px] px-1.5 py-0.5 rounded bg-amber-500/15 text-amber-400 border border-amber-500/30 shrink-0 font-medium">
              已撤回
            </span>
          )}

          {isAlignmentPassed && (
            <span className="text-[11px] px-1.5 py-0.5 rounded bg-emerald-500/15 text-emerald-400 border border-emerald-500/30 shrink-0 font-medium">
              意图已对齐
            </span>
          )}

          {isAlignmentInProgress && (
            <span className="text-[11px] px-1.5 py-0.5 rounded bg-sky-500/15 text-sky-400 border border-sky-500/30 shrink-0 font-medium animate-pulse">
              质检中
            </span>
          )}

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
        <div className="border-t border-edge/40 px-3.5 pb-3.5 pt-2.5 flex flex-col gap-3.5 bg-panel/30 select-text">
          {/* 前置门控：意图对齐与规划质检卡片（支持展开与收起） */}
          {resolvedAlignment && (
            <IntentAlignmentCard
              alignment={resolvedAlignment}
              pendingIntervention={resolvedIntervention}
              sessionId={currentSessionId || undefined}
            />
          )}

          {/* 首步准备阶段提示 */}
          {stepCount === 0 && isRunning && (
            <div className="flex items-center gap-2.5 py-1.5 px-1 text-inkdim text-[12.5px] animate-in fade-in duration-150">
              <Loader2 size={14} className="animate-spin text-accent shrink-0" />
              <span>
                {isAlignmentPassed
                  ? "意图对齐与规划质检已通过，正在启动执行流程…"
                  : "Agent 正在深度剖析用户诉求与分步行动方案…"}
              </span>
            </div>
          )}

          {/* 分步执行明细 */}
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
