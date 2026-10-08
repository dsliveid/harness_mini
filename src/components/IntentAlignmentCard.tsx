import { useState, useEffect, useMemo } from "react";
import { useStore } from "../store";
import {
  CheckCircle2,
  XCircle,
  AlertTriangle,
  Loader2,
  ChevronRight,
  Send,
  Play,
  Lightbulb,
  ListTodo,
  Brain,
  RefreshCw,
  ShieldCheck,
  Copy,
  Check,
} from "./Icons";
import type { IntentAlignmentEvent, IntentAlignmentRound, IntentInterventionRequest } from "../types";

export interface IntentAlignmentCardProps {
  alignment?: IntentAlignmentEvent;
  pendingIntervention?: IntentInterventionRequest | null;
  sessionId?: string;
  defaultExpanded?: boolean;
  className?: string;
}

function alignmentStatusBadge(
  status: string,
  passed: boolean,
  isRequiresIntervention: boolean,
  inProgress: boolean
) {
  if (isRequiresIntervention) {
    return (
      <span className="text-[11px] px-2 py-0.5 rounded-full bg-amber-500/15 text-amber-400 border border-amber-500/30 font-medium">
        待干预
      </span>
    );
  }
  if (status === "analyzing") {
    return (
      <span className="text-[11px] px-2 py-0.5 rounded-full bg-sky-500/15 text-sky-400 border border-sky-500/30 animate-pulse font-medium">
        分析中
      </span>
    );
  }
  if (status === "evaluating") {
    return (
      <span className="text-[11px] px-2 py-0.5 rounded-full bg-purple-500/15 text-purple-400 border border-purple-500/30 animate-pulse font-medium">
        质检中
      </span>
    );
  }
  if (status === "retrying") {
    return (
      <span className="text-[11px] px-2 py-0.5 rounded-full bg-amber-500/15 text-amber-400 border border-amber-500/30 animate-pulse font-medium">
        修正中
      </span>
    );
  }
  if (status === "passed" || passed) {
    return (
      <span className="text-[11px] px-2 py-0.5 rounded-full bg-green-500/15 text-green-400 border border-green-500/30 font-medium">
        已通过
      </span>
    );
  }
  if (status === "skipped") {
    return (
      <span className="text-[11px] px-2 py-0.5 rounded-full bg-panel3 text-inkdim border border-edge/40">
        已跳过
      </span>
    );
  }
  return null;
}

export function IntentAlignmentCard(props?: IntentAlignmentCardProps) {
  const currentId = useStore((s) => s.currentId);
  const effectiveSessionId = props?.sessionId || currentId;
  const storeAlignment = useStore((s) => (effectiveSessionId ? s.intentAlignments[effectiveSessionId] : undefined));
  const storePending = useStore((s) => s.pendingIntentIntervention);
  const pushToast = useStore((s) => s.pushToast);

  const alignment = props?.alignment ?? storeAlignment;
  const pendingIntervention =
    props?.pendingIntervention !== undefined
      ? props.pendingIntervention
      : storePending?.sessionId === effectiveSessionId
      ? storePending
      : null;

  const respondIntentIntervention = useStore((s) => s.respondIntentIntervention);

  // 状态判定
  const isRequiresIntervention =
    Boolean(pendingIntervention && pendingIntervention.sessionId === effectiveSessionId) ||
    alignment?.status === "requires_intervention";
  const isPassed = alignment?.status === "passed" || alignment?.passed === true;
  const isSkipped = alignment?.status === "skipped";
  const isAnalyzing = alignment?.status === "analyzing";
  const isEvaluating = alignment?.status === "evaluating";
  const isRetrying = alignment?.status === "retrying";
  const inProgress = isAnalyzing || isEvaluating || isRetrying || isRequiresIntervention;

  // 用户折叠状态切换：运行与待干预中默认展开，已通过或已跳过后默认收起（和工具卡片一致）
  const [userToggled, setUserToggled] = useState<boolean | null>(null);

  // 当进入待干预状态时，自动展开提醒用户
  useEffect(() => {
    if (isRequiresIntervention) {
      setUserToggled(true);
    }
  }, [isRequiresIntervention]);

  const defaultExp = props?.defaultExpanded !== undefined ? props.defaultExpanded : inProgress;
  const expanded = userToggled !== null ? userToggled : defaultExp;

  const [hintText, setHintText] = useState("");
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [showHintInput, setShowHintInput] = useState(false);
  const [selectedRoundIndex, setSelectedRoundIndex] = useState<number | null>(null);
  const [copiedKey, setCopiedKey] = useState<string | null>(null);

  const handleCopy = (e: React.MouseEvent, key: string, text: string, label: string) => {
    e.stopPropagation();
    if (!text) return;
    navigator.clipboard.writeText(text).then(
      () => {
        setCopiedKey(key);
        setTimeout(() => setCopiedKey(null), 1500);
        pushToast(`已复制${label}到剪贴板`);
      },
      () => pushToast("复制失败")
    );
  };

  // 如果没有与当前会话匹配的门控事件与干预事件，不渲染
  if (!effectiveSessionId || (!alignment && !pendingIntervention)) return null;

  const currentAttempt = alignment?.currentAttempt || alignment?.attempt || 1;
  const rawScore = alignment?.scorePercent ?? (alignment?.score ? Math.round(alignment.score * 100) : 0);
  const scorePct = pendingIntervention
    ? Math.round(pendingIntervention.score > 1 ? pendingIntervention.score : pendingIntervention.score * 100)
    : rawScore;

  // 汇总所有轮次历史记录
  const historyRounds: IntentAlignmentRound[] = [...(alignment?.history || [])];

  // 如果 history 为空但当前已有提炼结果或正在分析，兜底构造当前轮次以便呈现
  if (
    historyRounds.length === 0 &&
    (alignment?.understanding || alignment?.plan || pendingIntervention?.understanding || inProgress)
  ) {
    historyRounds.push({
      attempt: currentAttempt,
      understanding: pendingIntervention?.understanding || alignment?.understanding || "",
      plan: pendingIntervention?.plan || alignment?.plan || "",
      score: pendingIntervention?.score || alignment?.score || 0,
      scorePercent: scorePct,
      critique: pendingIntervention?.critique || alignment?.reason || "",
      passed: isPassed,
    });
  }

  // 计算当前查看的轮次
  const activeRoundIndex =
    selectedRoundIndex !== null && selectedRoundIndex < historyRounds.length
      ? selectedRoundIndex
      : Math.max(0, historyRounds.length - 1);
  const activeRound = historyRounds[activeRoundIndex];

  // 活动的干预通道 ID（优先使用 pendingIntervention，其次 alignment.interventionId）
  const activeInterventionId = pendingIntervention?.id || alignment?.interventionId || undefined;

  const handleSkip = async () => {
    setIsSubmitting(true);
    try {
      await respondIntentIntervention("skip", undefined, activeInterventionId);
    } finally {
      setIsSubmitting(false);
    }
  };

  const handleSendHint = async () => {
    if (!hintText.trim()) return;
    setIsSubmitting(true);
    try {
      await respondIntentIntervention("hint", hintText.trim(), activeInterventionId);
      setHintText("");
      setShowHintInput(false);
    } finally {
      setIsSubmitting(false);
    }
  };

  // 提取当前轮次的文本内容供拷贝与展示
  const understandingContent =
    activeRound?.understanding ||
    alignment?.understanding ||
    pendingIntervention?.understanding ||
    "";

  const planContent =
    activeRound?.plan ||
    alignment?.plan ||
    pendingIntervention?.plan ||
    "";

  const critiqueContent =
    activeRound?.critique ||
    alignment?.reason ||
    pendingIntervention?.critique ||
    "";

  const fullContentToCopy = useMemo(() => {
    const parts: string[] = [];
    if (understandingContent) parts.push(`【核心意图剖析与约束边界】\n${understandingContent}`);
    if (planContent) parts.push(`【拟定分步行动计划】\n${planContent}`);
    if (critiqueContent) parts.push(`【决策模型独立质检意见】\n${critiqueContent}`);
    return parts.join("\n\n");
  }, [understandingContent, planContent, critiqueContent]);

  // 摘要预览文本
  const summaryTitle = useMemo(() => {
    if (isAnalyzing) return `正在深度剖析核心诉求与分步计划 (第 ${currentAttempt} 轮)…`;
    if (isEvaluating) return `决策模型正在独立严苛质检方案可行性…`;
    if (isRetrying) return `质检未达标 (${scorePct}%)，正在第 ${currentAttempt} 轮自愈修正…`;
    if (isRequiresIntervention) return `质检未达标，等待人工指导或跳过`;
    if (isPassed) {
      if (understandingContent) {
        return `理解已对齐：${understandingContent.replace(/\s+/g, " ")}`;
      }
      return `意图对齐与规划质检已通过`;
    }
    if (isSkipped) return `已手动跳过前置质检门禁，按当前理解直接推进`;
    return "意图对齐与规划质检";
  }, [
    isAnalyzing,
    isEvaluating,
    isRetrying,
    isRequiresIntervention,
    isPassed,
    isSkipped,
    currentAttempt,
    scorePct,
    understandingContent,
  ]);

  // 样式主题配色
  const cardBorderBg = isRequiresIntervention
    ? "border-amber-500/50 bg-amber-500/10 text-amber-200 ring-1 ring-amber-500/20"
    : isPassed
    ? "border-emerald-500/30 bg-emerald-500/5 hover:border-emerald-500/50"
    : isSkipped
    ? "border-edge/70 bg-panel2/60 text-inkdim"
    : isRetrying
    ? "border-amber-500/35 bg-amber-500/5 text-amber-300"
    : isEvaluating
    ? "border-purple-500/35 bg-purple-500/5 text-purple-300"
    : "border-sky-500/35 bg-sky-500/5 text-sky-300";

  return (
    <div
      id={`intent-alignment-${alignment?.runId || "card"}`}
      className={`border rounded-xl px-3 py-2.5 transition-all shadow-sm ${cardBorderBg} ${
        props?.className || ""
      }`}
    >
      {/* 头部：与 ToolCard 一致的折叠/展开栏 */}
      <div
        role="button"
        tabIndex={0}
        className="w-full flex items-center gap-2 text-left select-none cursor-pointer group"
        onClick={() => setUserToggled(!expanded)}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            setUserToggled(!expanded);
          }
        }}
        title={expanded ? "点击收起详情" : "点击展开详情"}
      >
        <ChevronRight
          size={13}
          className={`transition-transform duration-150 text-inkdim shrink-0 ${
            expanded ? "rotate-90" : ""
          }`}
        />

        {/* 状态图标 */}
        {isAnalyzing && <Brain size={14} className="text-sky-400 shrink-0 animate-pulse" />}
        {isEvaluating && <Loader2 size={14} className="text-purple-400 shrink-0 animate-spin" />}
        {isRetrying && <RefreshCw size={14} className="text-amber-400 shrink-0 animate-spin" />}
        {isRequiresIntervention && <AlertTriangle size={14} className="text-amber-400 shrink-0 animate-pulse" />}
        {isPassed && <ShieldCheck size={14} className="text-emerald-400 shrink-0" />}
        {isSkipped && <Play size={13} className="text-inkdim shrink-0" />}

        {/* 卡片标签与前置门控标识 */}
        <div className="flex items-center gap-1.5 shrink-0">
          <span className="text-[12px] text-ink font-medium">意图对齐与规划质检</span>
          <span className="text-[10px] px-1.5 py-[0.5px] rounded bg-purple-500/10 text-purple-400 border border-purple-500/20 shrink-0">
            前置门控
          </span>
        </div>

        {/* 摘要与进度预览 */}
        <span
          className="text-[12px] font-mono text-inkdim truncate flex-1 min-w-0"
          title={summaryTitle}
        >
          {summaryTitle}
        </span>

        {/* 复制全部按钮、契合度与状态徽标 */}
        <div className="ml-auto flex items-center gap-1.5 shrink-0">
          {fullContentToCopy && (
            <button
              type="button"
              className="p-1 rounded hover:bg-panel3 text-inkdim hover:text-ink transition-colors cursor-pointer"
              title="复制卡片全部对齐内容"
              onClick={(e) => handleCopy(e, "all", fullContentToCopy, "完整方案")}
            >
              {copiedKey === "all" ? <Check size={12} className="text-emerald-400" /> : <Copy size={12} />}
            </button>
          )}

          {scorePct > 0 && !isAnalyzing && (
            <span
              className={`text-[10px] font-mono px-1.5 py-0.5 rounded-full border font-bold ${
                isPassed
                  ? "bg-emerald-500/15 text-emerald-300 border-emerald-500/30"
                  : "bg-amber-500/20 text-amber-200 border-amber-500/35"
              }`}
            >
              契合度 {scorePct}%
            </span>
          )}
          {alignmentStatusBadge(alignment?.status || "", isPassed, isRequiresIntervention, inProgress)}
        </div>
      </div>

      {/* 展开的详情面板：明确支持 select-text 与文本选择 */}
      {expanded && (
        <div className="mt-2.5 pt-2.5 border-t border-edge/40 text-[12px] space-y-3 animate-in fade-in duration-150 select-text">
          {/* 多轮审查历史切换 Tabs（如果有多个轮次） */}
          {historyRounds.length > 1 && (
            <div className="flex items-center gap-1.5 overflow-x-auto pb-1 text-[11px] select-none">
              <span className="text-inkdim font-medium mr-1 shrink-0">质检迭代轮次:</span>
              {historyRounds.map((r, idx) => {
                const isSelected = idx === activeRoundIndex;
                const pct = r.scorePercent || Math.round(r.score * 100);
                return (
                  <button
                    key={idx}
                    type="button"
                    onClick={(e) => {
                      e.stopPropagation();
                      setSelectedRoundIndex(idx);
                    }}
                    className={`px-2.5 py-1 rounded-md border flex items-center gap-1.5 transition-colors cursor-pointer shrink-0 ${
                      isSelected
                        ? "bg-accent/15 border-accent text-accent font-semibold"
                        : "bg-panel2/60 border-edge/60 text-inkdim hover:text-ink hover:bg-panel3"
                    }`}
                  >
                    <span>第 {r.attempt} 轮</span>
                    {pct > 0 && (
                      <span
                        className={`text-[10px] px-1 py-0.2 rounded font-mono ${
                          r.passed
                            ? "text-emerald-400 bg-emerald-500/10"
                            : "text-amber-400 bg-amber-500/10"
                        }`}
                      >
                        {pct}%
                      </span>
                    )}
                    {r.passed ? (
                      <CheckCircle2 size={11} className="text-emerald-400" />
                    ) : (
                      <XCircle size={11} className="text-amber-400" />
                    )}
                  </button>
                );
              })}
            </div>
          )}

          {/* 详细内容展示区 */}
          <div className="bg-panel/75 border border-edge/60 rounded-xl p-3.5 space-y-3 shadow-inner select-text">
            {/* 1. 大模型对意图的剖析 */}
            <div>
              <div className="flex items-center justify-between font-medium text-inkdim mb-1">
                <div className="flex items-center gap-1.5">
                  <Lightbulb size={13} className="text-amber-400 shrink-0" />
                  <span>大模型核心意图剖析与约束边界：</span>
                </div>
                {understandingContent && (
                  <button
                    type="button"
                    className="px-1.5 py-0.5 rounded hover:bg-panel3 text-inkdim hover:text-ink transition-colors cursor-pointer inline-flex items-center gap-1 text-[11px] select-none"
                    title="复制意图理解文本"
                    onClick={(e) => handleCopy(e, "understanding", understandingContent, "意图理解")}
                  >
                    {copiedKey === "understanding" ? <Check size={11} className="text-emerald-400" /> : <Copy size={11} />}
                    <span>{copiedKey === "understanding" ? "已复制" : "复制"}</span>
                  </button>
                )}
              </div>
              <div className="text-ink bg-panel2/80 px-3 py-2 rounded-lg border border-edge/40 font-mono text-[11.5px] leading-relaxed whitespace-pre-wrap select-text cursor-text selection:bg-accent/25">
                {understandingContent ||
                  (isAnalyzing ? "🧠 大模型正在深入剖析用户核心诉求与边界..." : "（无意图理解内容）")}
              </div>
            </div>

            {/* 2. 大模型制定的行动计划 */}
            <div>
              <div className="flex items-center justify-between font-medium text-inkdim mb-1">
                <div className="flex items-center gap-1.5">
                  <ListTodo size={13} className="text-cyan-400 shrink-0" />
                  <span>拟定下一步分步行动计划：</span>
                </div>
                {planContent && (
                  <button
                    type="button"
                    className="px-1.5 py-0.5 rounded hover:bg-panel3 text-inkdim hover:text-ink transition-colors cursor-pointer inline-flex items-center gap-1 text-[11px] select-none"
                    title="复制行动计划文本"
                    onClick={(e) => handleCopy(e, "plan", planContent, "行动计划")}
                  >
                    {copiedKey === "plan" ? <Check size={11} className="text-emerald-400" /> : <Copy size={11} />}
                    <span>{copiedKey === "plan" ? "已复制" : "复制"}</span>
                  </button>
                )}
              </div>
              <div className="text-ink bg-panel2/80 px-3 py-2 rounded-lg border border-edge/40 font-mono text-[11.5px] leading-relaxed whitespace-pre-wrap select-text cursor-text selection:bg-accent/25">
                {planContent ||
                  (isAnalyzing ? "📋 正在制定下一步分步工具调用方案..." : "（无行动方案内容）")}
              </div>
            </div>

            {/* 3. 决策模型独立质检意见与打分 */}
            {(critiqueContent || isEvaluating) && (
              <div className="pt-2 border-t border-edge/30">
                <div className="flex items-center justify-between text-amber-300 font-medium mb-1">
                  <span className="flex items-center gap-1.5">
                    <ShieldCheck size={13} className="text-amber-400 shrink-0" />
                    <span>决策模型独立质检评估与反思意见：</span>
                  </span>
                  <div className="flex items-center gap-2">
                    {(activeRound?.scorePercent || alignment?.scorePercent || pendingIntervention) && (
                      <span className="font-mono text-[11px] px-2 py-0.5 rounded bg-amber-500/20 text-amber-200 border border-amber-500/30">
                        质检得分: {activeRound?.scorePercent || alignment?.scorePercent || scorePct}%
                      </span>
                    )}
                    {critiqueContent && (
                      <button
                        type="button"
                        className="px-1.5 py-0.5 rounded hover:bg-amber-500/20 text-amber-300/90 hover:text-amber-200 transition-colors cursor-pointer inline-flex items-center gap-1 text-[11px] select-none"
                        title="复制质检意见文本"
                        onClick={(e) => handleCopy(e, "critique", critiqueContent, "质检意见")}
                      >
                        {copiedKey === "critique" ? <Check size={11} className="text-emerald-400" /> : <Copy size={11} />}
                        <span>{copiedKey === "critique" ? "已复制" : "复制"}</span>
                      </button>
                    )}
                  </div>
                </div>
                <div className="text-amber-100/90 bg-amber-500/10 px-3 py-2 rounded-lg border border-amber-500/25 leading-relaxed text-[11.5px] whitespace-pre-wrap select-text cursor-text selection:bg-amber-500/30">
                  {isEvaluating && !critiqueContent
                    ? "⚡ 决策模型正在根据评估细则对意图与方案进行全方位审阅..."
                    : critiqueContent}
                </div>
              </div>
            )}
          </div>

          {/* 全程随时可控的干预与跳过工具条（只要门控未结束或等待人工决策） */}
          {inProgress && (
            <div className="pt-1 select-none">
              {showHintInput ? (
                <div className="space-y-2 pt-1">
                  <textarea
                    className="w-full bg-panel border border-amber-500/40 focus:border-amber-400 rounded-lg p-2.5 text-[12px] text-ink outline-none resize-none min-h-[64px] select-text cursor-text"
                    placeholder="请输入纠偏补充提示（例如：'请不要修改配置文件，只关注修复测试用例'）..."
                    value={hintText}
                    onChange={(e) => setHintText(e.target.value)}
                    disabled={isSubmitting}
                    autoFocus
                  />
                  <div className="flex items-center justify-end gap-2">
                    <button
                      type="button"
                      className="px-3 py-1.5 rounded-lg border border-edge hover:bg-panel3 text-[12px] text-inkdim hover:text-ink cursor-pointer transition-colors"
                      onClick={() => setShowHintInput(false)}
                      disabled={isSubmitting}
                    >
                      取消
                    </button>
                    <button
                      type="button"
                      className="px-3.5 py-1.5 rounded-lg bg-amber-500 hover:bg-amber-400 text-zinc-950 font-medium text-[12px] flex items-center gap-1.5 cursor-pointer transition-colors shadow-sm disabled:opacity-50"
                      onClick={handleSendHint}
                      disabled={isSubmitting || !hintText.trim()}
                    >
                      {isSubmitting ? <Loader2 size={13} className="animate-spin" /> : <Send size={13} />}
                      <span>提交纠偏提示并重新评估</span>
                    </button>
                  </div>
                </div>
              ) : (
                <div className="flex items-center justify-between gap-2 pt-0.5">
                  <div className="text-[11px] text-inkdim/80">
                    💡 提示：您可在此处随时纠偏，或直接跳过对齐让大模型立即执行。
                  </div>
                  <div className="flex items-center gap-2">
                    <button
                      type="button"
                      className="px-3 py-1.5 rounded-lg bg-panel3 hover:bg-edge border border-edge text-ink text-[12px] flex items-center gap-1.5 cursor-pointer transition-colors shadow-sm"
                      onClick={handleSkip}
                      disabled={isSubmitting}
                      title="直接跳过前置意图门控，让大模型立即开始执行任务"
                    >
                      <Play size={12} className="text-inkdim" />
                      <span>跳过对齐，直接执行</span>
                    </button>
                    <button
                      type="button"
                      className="px-3.5 py-1.5 rounded-lg bg-amber-500 hover:bg-amber-400 text-zinc-950 font-medium text-[12px] flex items-center gap-1.5 cursor-pointer transition-colors shadow-sm"
                      onClick={() => setShowHintInput(true)}
                      disabled={isSubmitting}
                      title="输入补充提示或约束条件，指导大模型重新审视并制定方案"
                    >
                      <Lightbulb size={13} />
                      <span>给予补充提示</span>
                    </button>
                  </div>
                </div>
              )}
            </div>
          )}
        </div>
      )}
    </div>
  );
}
