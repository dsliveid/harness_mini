import { useState, useMemo, useEffect, useRef } from "react";

import type { Message, TurnMetrics, SessionCompaction, IntentAlignmentEvent, IntentInterventionRequest } from "../types";

import { MessageItem } from "./MessageItem";

import { useStore } from "../store";

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

/** 清洗可能夹带【思考过程汇报】旧版兜底标记的历史正文，精确提取最终交付结论 */

function extractTargetFromPartialArgs(toolName?: string, partialArgs?: string): string | null {
  if (!partialArgs) return null;
  const match = partialArgs.match(/["'](?:Path|path|target|Target|CommandLine|command|prompt|query|filename)["']\s*:\s*["']([^"'\\]*(?:\\.[^"'\\]*)*)/i);
  if (match && match[1]) {
    try {
      return JSON.parse(`"${match[1]}"`);
    } catch {
      return match[1];
    }
  }
  return null;
}

export function cleanDeliverableContent(content?: string | null): string {

  if (!content) return "";

  const trimmed = content.trim();

  if (!trimmed.startsWith("【思考过程汇报】")) return content;

  const withoutPrefix = trimmed.replace(/^【思考过程汇报】\s*/, "");

  const match = withoutPrefix.match(

    /(?:(?:^|\n|[\s。！!])end\s*(?:\n+|[\s:：])*(?=\*\*|##|#|【|一句话|结论|核心|直接)|(?:\n\s*\n|\n)(?=(?:##|\*\*|【)?(?:先给结论|核心结论|结论先说|最终结论|结论|一句话结论|总(?:结|览)|回答|建议)(?:\*\*|】)?[:：\s])|(?:\n\s*\n)(?=(?:#+\s+|因此[，,]|综上所述[，,]|基于以上分析[，,])))/i

  );

  if (match && match.index != null) {

    const tailRaw = withoutPrefix.slice(match.index + match[0].length).trim();

    const cleanTail = tailRaw.replace(/^end[:：\s]*/i, "").trim();

    if (cleanTail.length > 0) {

      return cleanTail;

    }

  }

  return withoutPrefix;

}

/**

 * 将平铺的时间线消息按轮次聚合成：用户提问 -> 中间执行过程 (含前置意图对齐门控与多步工具折叠) -> 最终交付成果

 * 双轨恒定容器模型 (Dual-track Container Model)：

 * - 过程轨 (Execution Track)：所有中间思考与工具调用卡片收拢于过程抽屉内，无论是运行中还是已完成均自始至终在上方稳态展示

 * - 交付轨 (Deliverable Track)：最终结论与交付答复自始至终独立于过程抽屉之外，在下方流式或静态呈现，绝不被塞入抽屉导致被吞

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

  let currentAssistantSteps: Message[] = [];

  let currentTriggerUserMsg: Message | undefined;

  const flushTurn = (isCurrentRunningTurn: boolean) => {

    if (currentAssistantSteps.length === 0) return;

    const firstMsg = currentAssistantSteps[0];

    const lastMsg = currentAssistantSteps[currentAssistantSteps.length - 1];

    // 绝对稳定的轮次 Key：优先绑定触发该轮次的 userMsg.id，或 runId / 首个步骤 id

    // 整个轮次生命周期内 100% 恒定不变，绝对杜绝 React 组件在步骤流式推进中发生 Unmount/Remount

    const stableTurnKey = currentTriggerUserMsg?.id || firstMsg.runId || firstMsg.id;

    const allTurnToolEvents = currentAssistantSteps.flatMap((m) => m.toolEvents || []);

    const turnRevertedAt = currentAssistantSteps.find((m) => m.revertedAt)?.revertedAt || lastMsg.revertedAt;

    const turnAlignment =

      findIntentAlignmentForSteps(currentAssistantSteps) ||

      (isCurrentRunningTurn ? activeIntentAlignment : undefined);

    const metrics =

      (firstMsg.runId ? turnMetricsMap.get(firstMsg.runId) : undefined) ??

      turnMetricsMap.get(lastMsg.id) ??

      turnMetricsMap.get(firstMsg.id);

    const anyStepHasTools = currentAssistantSteps.some(hasAnyTools);

    const lastHasTools = hasAnyTools(lastMsg);

    const lastHasReasoning = hasAnyReasoning(lastMsg);

    // 核心不变量（方案 A 轮次领域模型）：

    // 过程抽屉的运行态严格且唯一受整个轮次的运行状态控制！

    // 只要当前轮次处于运行中，isProcessRunning 始终为 true，绝不允许被单步微观属性临时翻转！

    const isProcessRunning = isCurrentRunningTurn;

    if (anyStepHasTools || currentAssistantSteps.length > 1) {

      // 场景 A：多步工具执行轮次（ReAct Multi-step Loop）

      // 若最后一步无工具调用，则最后一步为最终交付答复（Deliverable），其前序步骤及思考过程归入过程轨

      const isLastStepDeliverable = !lastHasTools;

      let processSteps: Message[];

      let deliverableMsg: Message | undefined;

      if (isLastStepDeliverable) {

        processSteps = currentAssistantSteps.slice(0, currentAssistantSteps.length - 1);

        if (lastHasReasoning) {

          processSteps.push({

            ...lastMsg,

            id: `${lastMsg.id}-reasoning`,

            content: null,

            toolEvents: [],

            toolCalls: [],

          });

        }

        deliverableMsg = lastMsg;

      } else {

        // 最后一步仍包含工具调用（正在执行工具，或中途停止）

        processSteps = currentAssistantSteps;

        // 若已停止（非运行态）且最后一步携带了总结文本，提取作为交付

        if (!isCurrentRunningTurn) {

          deliverableMsg = lastMsg;

        }

      }

      // 过程轨发射：只要有步骤或有意图门控即呈现过程抽屉

      if (processSteps.length > 0 || Boolean(turnAlignment)) {

        result.push({

          type: "process",

          id: `process-turn-${stableTurnKey}`,

          steps: processSteps,

          turnMetrics: metrics,

          isRunning: isProcessRunning,

          intentAlignment: turnAlignment,

        });

      }

      // 交付轨发射：双轨恒定容器模型，无论运行中还是已完成均独立渲染在外部

      if (deliverableMsg) {

        const cleanContent = cleanDeliverableContent(deliverableMsg.content);

        const hasContent =

          cleanContent.length > 0 || (isCurrentRunningTurn && deliverableMsg.content != null);

        if (hasContent) {

          result.push({

            type: "message",

            msg: {

              ...deliverableMsg,

              content: cleanContent,

              reasoning: null,

              toolEvents: [],

              toolCalls: [],

              turnToolEvents: allTurnToolEvents,

              revertedAt: turnRevertedAt,

            },

          });

        }

      }

    } else {

      // 场景 B：单步助手消息（Single-step Assistant）

      const singleMsg = currentAssistantSteps[0];

      const singleHasTools = hasAnyTools(singleMsg);

      const singleHasReasoning = hasAnyReasoning(singleMsg);

      if (singleHasTools) {

        // 单步直接调用了工具

        result.push({

          type: "process",

          id: `process-turn-${stableTurnKey}`,

          steps: [singleMsg],

          turnMetrics: metrics,

          isRunning: isProcessRunning,

          intentAlignment: turnAlignment,

        });

        const cleanSingle = cleanDeliverableContent(singleMsg.content);

        if (cleanSingle.length > 0 || (isCurrentRunningTurn && singleMsg.content != null)) {

          result.push({

            type: "message",

            msg: {

              ...singleMsg,

              content: cleanSingle,

              reasoning: null,

              toolEvents: [],

              toolCalls: [],

              turnToolEvents: singleMsg.toolEvents,

              revertedAt: turnRevertedAt,

            },

          });

        }

      } else if (turnAlignment || singleHasReasoning) {

        // 单步无工具，但具备思考过程或意图门控卡片

        result.push({

          type: "process",

          id: `process-turn-${stableTurnKey}`,

          steps: singleHasReasoning

            ? [{ ...singleMsg, id: `${singleMsg.id}-reasoning`, content: null, toolEvents: [], toolCalls: [] }]

            : [],

          turnMetrics: metrics,

          isRunning: isProcessRunning,

          intentAlignment: turnAlignment,

        });

        const cleanSingle = cleanDeliverableContent(singleMsg.content);

        if (cleanSingle.length > 0 || (isCurrentRunningTurn && singleMsg.content != null)) {

          result.push({

            type: "message",

            msg: {

              ...singleMsg,

              content: cleanSingle,

              reasoning: null,

              toolEvents: [],

              toolCalls: [],

              revertedAt: turnRevertedAt,

            },

          });

        }

      } else {

        // 纯单步极简普通文本答复（无工具、无思考、无门控）：直接在外部作为普通消息呈现

        const cleanSingle = cleanDeliverableContent(singleMsg.content);

        if (cleanSingle.length > 0 || (isCurrentRunningTurn && singleMsg.content != null)) {

          result.push({

            type: "message",

            msg: {

              ...singleMsg,

              content: cleanSingle,

              toolEvents: [],

              toolCalls: [],

              revertedAt: turnRevertedAt,

            },

          });

        }

      }

    }

    currentAssistantSteps = [];

  };

  for (let i = 0; i < items.length; i++) {

    const item = items[i];

    if (item.type === "compaction") {

      flushTurn(false);

      currentTriggerUserMsg = undefined;

      result.push(item);

    } else if (item.msg.role === "assistant") {

      // 检查 runId 边界：若新步骤与当前累积步骤的 runId 明确不同，强制切断并结算前序轮次，杜绝跨轮次混合

      if (

        currentAssistantSteps.length > 0 &&

        currentAssistantSteps[0].runId &&

        item.msg.runId &&

        currentAssistantSteps[0].runId !== item.msg.runId

      ) {

        flushTurn(false);

      }

      currentAssistantSteps.push(item.msg);

    } else {

      // user 消息或其他非 assistant 角色

      flushTurn(false);

      if (item.msg.role === "user") {

        currentTriggerUserMsg = item.msg;

      } else {

        currentTriggerUserMsg = undefined;

      }

      result.push(item);

    }

  }

  // 处理时间线尾部的当前轮次

  const isTailActive = running && lastAssistantIdx >= 0;

  flushTurn(isTailActive);

  // 若当前正在运行中，且尚未产生 assistant 消息（等待模型首字响应期间：直接极简占位，杜绝首步预占抽屉闪烁）

  if (running && (lastAssistantIdx < 0 || lastAssistantIdx < items.length - 1)) {

    const lastItem = items[items.length - 1];

    if (lastItem && lastItem.type === "message" && lastItem.msg.role === "user") {

      const isRequiresIntervention = Boolean(

        activeIntentAlignment && activeIntentAlignment.status === "requires_intervention"

      );

      if (isRequiresIntervention) {

        result.push({

          type: "process",

          id: `process-turn-${lastItem.msg.id}`,

          steps: [],

          isRunning: true,

          intentAlignment: activeIntentAlignment,

        });

      } else {

        result.push({

          type: "message",

          msg: {

            id: `pending-${lastItem.msg.id}`,

            sessionId: lastItem.msg.sessionId,

            seq: Number.MAX_SAFE_INTEGER,

            role: "assistant",

            content: null,

            reasoning: null,

            queued: false,

            createdAt: new Date().toISOString(),

            toolEvents: [],

          },

        });

      }

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

  const [elapsedSec, setElapsedSec] = useState(0);

  useEffect(() => {

    if (!isRunning || steps.length > 0) {

      setElapsedSec(0);

      return;

    }

    const timer = setInterval(() => {

      setElapsedSec((s) => s + 1);

    }, 1000);

    return () => clearInterval(timer);

  }, [isRunning, steps.length]);

  const currentSessionId = useStore((s) => s.currentId);
  const sessionId = steps[0]?.sessionId || currentSessionId;
  const activeStep = useStore((s) => (sessionId ? s.activeStepMap[sessionId] : null));

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

  const isRequiresIntervention = Boolean(

    resolvedIntervention || resolvedAlignment?.status === "requires_intervention"

  );

  // 用户手动切换过展开/收起时，优先使用用户意图；否则运行中或等待干预时默认展开，完成后默认折叠

  const [userToggled, setUserToggled] = useState<boolean | null>(null);

  const isOpen = userToggled !== null ? userToggled : (isRunning || isRequiresIntervention);

  useEffect(() => {

    if (isRequiresIntervention) {

      setUserToggled(true);

    }

  }, [isRequiresIntervention]);

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

  const hasDecisionTool = useMemo(() => {

    return steps.some((s) =>

      s.toolEvents?.some((t) => ["score_decision", "judge_decision", "choice_decision"].includes(t.toolName))

    );

  }, [steps]);

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
          ) : toolCount === 0 ? (
            <Brain size={15} className="text-purple-400 shrink-0" />
          ) : (
            <Wrench size={15} className="text-accent shrink-0" />
          )}

          <span className="text-[13px] font-medium text-ink truncate">

            {isRunning

              ? stepCount === 0

                ? `正在连接模型… ${elapsedSec > 0 ? `(${elapsedSec}s)` : ""}`

                : activeStep?.phase.type === "tool_preparing"

                ? `正在准备 ${activeStep.phase.toolName}...`

                : toolCount === 0 && stepCount === 1

                ? "正在思考与回复…"

                : `正在执行第 ${stepCount} 步…`

              : toolCount === 0

              ? `思考过程 (${stepCount} 个步骤)`

              : hasDecisionTool

              ? `执行过程 (含决策 · ${stepCount} 个步骤)`

              : `执行过程 (${stepCount} 个步骤)`}

          </span>

          {isReverted && (

            <span className="text-[11px] px-1.5 py-0.5 rounded bg-amber-500/15 text-amber-400 border border-amber-500/30 shrink-0 font-medium">

              已撤回

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

              {activeStep?.phase.type === "tool_preparing"
                ? `准备参数 (${Math.round(((activeStep.phase.bytesReceived || activeStep.phase.bytes_received || 0) / 1024) * 10) / 10} KB)`
                : "运行中"}

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

          {/* 首步准备阶段提示 */}

          {stepCount === 0 && isRunning && (

            <div className="flex items-center gap-2.5 py-1.5 px-1 text-inkdim text-[12.5px] animate-in fade-in duration-150">

              <Loader2 size={14} className="animate-spin text-accent shrink-0" />

              <span>

                正在连接模型… {elapsedSec > 0 ? `(${elapsedSec}s)` : ""}

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

                streaming={Boolean(isRunning && (step.id === streamingMsgId || (streamingMsgId && (step.id.startsWith(streamingMsgId) || streamingMsgId.startsWith(step.id)))))}

                readOnly={readOnly}

                running={isRunning}

                turnMetrics={turnMetricsMap?.get(step.id)}

              />

            </div>

          ))}

          {isRunning && activeStep?.phase.type === "tool_preparing" && (
            <div className="relative pl-3 border-l-2 border-blue-500/60 animate-in fade-in duration-200">
              <div className="flex items-center gap-2.5 py-2 px-3 rounded-xl bg-blue-500/10 border border-blue-500/25 text-[12.5px] text-blue-200">
                <Loader2 size={14} className="animate-spin text-blue-400 shrink-0" />
                <div className="flex-1 min-w-0 flex items-center justify-between gap-2">
                  <div className="flex items-center gap-2 truncate">
                    <span className="font-mono font-medium text-blue-300">
                      {activeStep.phase.toolName}
                    </span>
                    <span className="truncate text-inkdim text-[12px]">
                      {(() => {
                        const target = extractTargetFromPartialArgs(
                          activeStep.phase.toolName,
                          activeStep.phase.partialArgs || activeStep.phase.partial_args
                        );
                        return target ? `目标: ${target}` : "正在接收参数...";
                      })()}
                    </span>
                  </div>
                  <span className="font-mono text-[11px] text-blue-400/90 shrink-0">
                    {(() => {
                      const bytes = activeStep.phase.bytesReceived || activeStep.phase.bytes_received || 0;
                      return bytes > 1024 ? `${(bytes / 1024).toFixed(1)} KB` : `${bytes} B`;
                    })()}
                  </span>
                </div>
              </div>
            </div>
          )}

        </div>

      )}

    </div>

  );

}
