import { useState, useEffect } from "react";
import { useStore } from "../store";
import {
  Zap,
  CheckCircle2,
  XCircle,
  AlertTriangle,
  Loader2,
  ChevronDown,
  ChevronUp,
  X,
  Send,
  Play,
  Lightbulb,
  ListTodo,
  Brain,
  RefreshCw,
  ShieldCheck,
} from "./Icons";
import type { IntentAlignmentRound } from "../types";

export function IntentAlignmentCard() {
  const currentId = useStore((s) => s.currentId);
  const alignment = useStore((s) => (currentId ? s.intentAlignments[currentId] : undefined));
  const pendingIntervention = useStore((s) => s.pendingIntentIntervention);
  const respondIntentIntervention = useStore((s) => s.respondIntentIntervention);
  const clearIntentAlignment = useStore((s) => s.clearIntentAlignment);

  const [expanded, setExpanded] = useState(true);
  const [hintText, setHintText] = useState("");
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [showHintInput, setShowHintInput] = useState(false);
  const [selectedRoundIndex, setSelectedRoundIndex] = useState<number | null>(null);

  // 如果没有与当前会话匹配的门控事件与干预事件，不渲染
  if (!currentId || (!alignment && !pendingIntervention)) return null;

  // 综合判定当前状态
  const isRequiresIntervention =
    Boolean(pendingIntervention && pendingIntervention.sessionId === currentId) ||
    alignment?.status === "requires_intervention";
  const isPassed = alignment?.status === "passed" || alignment?.passed === true;
  const isSkipped = alignment?.status === "skipped";
  const isAnalyzing = alignment?.status === "analyzing";
  const isEvaluating = alignment?.status === "evaluating";
  const isRetrying = alignment?.status === "retrying";
  const inProgress = isAnalyzing || isEvaluating || isRetrying || isRequiresIntervention;

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

  // 样式主题配色
  const cardBorderBg = isPassed
    ? "border-emerald-500/30 bg-emerald-500/5 text-emerald-300"
    : isSkipped
    ? "border-edge/70 bg-panel/60 text-inkdim"
    : isRequiresIntervention
    ? "border-amber-500/40 bg-amber-500/10 text-amber-200"
    : isRetrying
    ? "border-amber-500/35 bg-amber-500/5 text-amber-300"
    : isEvaluating
    ? "border-purple-500/35 bg-purple-500/5 text-purple-300"
    : "border-sky-500/35 bg-sky-500/5 text-sky-300";

  return (
    <div
      className={`rounded-xl border p-3.5 shadow-sm transition-all animate-in fade-in duration-200 space-y-3 ${cardBorderBg}`}
    >
      {/* 顶栏：状态、进度、多轮指示及折叠/关闭控制 */}
      <div className="flex items-center justify-between gap-3">
        <div className="flex items-center gap-2.5 min-w-0">
          {isAnalyzing && <Brain size={16} className="text-sky-400 shrink-0 animate-pulse" />}
          {isEvaluating && <Loader2 size={16} className="text-purple-400 shrink-0 animate-spin" />}
          {isRetrying && <RefreshCw size={16} className="text-amber-400 shrink-0 animate-spin" />}
          {isRequiresIntervention && <AlertTriangle size={16} className="text-amber-400 shrink-0 animate-pulse" />}
          {isPassed && <CheckCircle2 size={16} className="text-emerald-400 shrink-0" />}
          {isSkipped && <Play size={14} className="text-inkdim shrink-0" />}

          <div className="min-w-0">
            <div className="font-semibold text-[13px] flex items-center gap-2 flex-wrap">
              <span>
                {isAnalyzing && `正在进行意图深度剖析与规划制定 (第 ${currentAttempt} 轮)...`}
                {isEvaluating && `大模型思考已完成，决策模型正在进行独立严苛质检打分...`}
                {isRetrying &&
                  `质检未达标 (${scorePct}% < 90%)，正在结合质检建议进行第 ${currentAttempt} 轮自愈修正...`}
                {isRequiresIntervention &&
                  `意图理解质检未达标（连续 ${pendingIntervention?.retryCount || currentAttempt} 轮低于 90%）`}
                {isPassed && `意图对齐与规划质检已通过`}
                {isSkipped && `意图对齐已手动跳过，按当前理解直接推进执行`}
              </span>

              {scorePct > 0 && !isAnalyzing && (
                <span
                  className={`text-[10.5px] font-mono px-2 py-0.5 rounded-full border font-bold ${
                    isPassed
                      ? "bg-emerald-500/15 text-emerald-300 border-emerald-500/30"
                      : "bg-amber-500/20 text-amber-200 border-amber-500/35"
                  }`}
                >
                  契合度: {scorePct}%
                </span>
              )}
            </div>

            <div className="text-[11.5px] text-inkdim/90 mt-0.5 truncate">
              {isAnalyzing && "大模型正在深刻剖析用户真实诉求、关键约束与拟调用的分步工具..."}
              {isEvaluating && "正在调用独立配置的决策模型对意图理解的深度与行动方案的可行性进行评分..."}
              {isRetrying && "大模型正在根据决策模型的质检批注进行反思迭代，修正行动方案偏差..."}
              {isRequiresIntervention && "决策模型研判大模型理解仍有偏差，请选择给予纠偏提示或直接跳过继续执行。"}
              {isPassed && "已完成双模型前置意图校验，执行方案已锁定并注入会话主流程。"}
              {isSkipped && "已跳过决策模型前置质量门禁，直接由主模型执行用户指令。"}
            </div>
          </div>
        </div>

        {/* 顶部操作区 */}
        <div className="flex items-center gap-1.5 shrink-0">
          <button
            type="button"
            className="p-1 rounded-md hover:bg-panel3 text-inkdim hover:text-ink cursor-pointer transition-colors"
            onClick={() => setExpanded((v) => !v)}
            title={expanded ? "收起详细思考与评估过程" : "展开详细思考与评估过程"}
          >
            {expanded ? <ChevronUp size={14} /> : <ChevronDown size={14} />}
          </button>
          {(isPassed || isSkipped) && (
            <button
              type="button"
              className="p-1 rounded-md hover:bg-panel3 text-inkdim hover:text-ink cursor-pointer transition-colors"
              onClick={() => clearIntentAlignment(currentId)}
              title="关闭卡片"
            >
              <X size={14} />
            </button>
          )}
        </div>
      </div>

      {/* 展开的详情面板 */}
      {expanded && (
        <div className="space-y-3 pt-1 border-t border-edge/30 text-[12px] animate-in fade-in duration-150">
          {/* 多轮审查历史切换 Tabs（如果有多个轮次） */}
          {historyRounds.length > 1 && (
            <div className="flex items-center gap-1.5 overflow-x-auto pb-1 text-[11px]">
              <span className="text-inkdim font-medium mr-1 shrink-0">质检迭代轮次:</span>
              {historyRounds.map((r, idx) => {
                const isSelected = idx === activeRoundIndex;
                const pct = r.scorePercent || Math.round(r.score * 100);
                return (
                  <button
                    key={idx}
                    type="button"
                    onClick={() => setSelectedRoundIndex(idx)}
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
          <div className="bg-panel/75 border border-edge/60 rounded-xl p-3.5 space-y-3 shadow-inner">
            {/* 1. 大模型对意图的剖析 */}
            <div>
              <div className="flex items-center gap-1.5 font-medium text-inkdim mb-1">
                <Lightbulb size={13} className="text-amber-400" />
                <span>大模型核心意图剖析与约束边界：</span>
              </div>
              <div className="text-ink bg-panel2/80 px-3 py-2 rounded-lg border border-edge/40 font-mono text-[11.5px] leading-relaxed whitespace-pre-wrap selection:bg-accent/20">
                {activeRound?.understanding ||
                  alignment?.understanding ||
                  pendingIntervention?.understanding ||
                  (isAnalyzing ? "🧠 大模型正在深入剖析用户核心诉求与边界..." : "（无意图理解内容）")}
              </div>
            </div>

            {/* 2. 大模型制定的行动计划 */}
            <div>
              <div className="flex items-center gap-1.5 font-medium text-inkdim mb-1">
                <ListTodo size={13} className="text-cyan-400" />
                <span>拟定下一步分步行动计划：</span>
              </div>
              <div className="text-ink bg-panel2/80 px-3 py-2 rounded-lg border border-edge/40 font-mono text-[11.5px] leading-relaxed whitespace-pre-wrap selection:bg-accent/20">
                {activeRound?.plan ||
                  alignment?.plan ||
                  pendingIntervention?.plan ||
                  (isAnalyzing ? "📋 正在制定下一步分步工具调用方案..." : "（无行动方案内容）")}
              </div>
            </div>

            {/* 3. 决策模型独立质检意见与打分 */}
            {(activeRound?.critique || alignment?.reason || pendingIntervention?.critique || isEvaluating) && (
              <div className="pt-2 border-t border-edge/30">
                <div className="flex items-center justify-between text-amber-300 font-medium mb-1">
                  <span className="flex items-center gap-1.5">
                    <ShieldCheck size={13} className="text-amber-400" />
                    <span>决策模型独立质检评估与反思意见：</span>
                  </span>
                  {(activeRound?.scorePercent || alignment?.scorePercent || pendingIntervention) && (
                    <span className="font-mono text-[11px] px-2 py-0.5 rounded bg-amber-500/20 text-amber-200 border border-amber-500/30">
                      质检得分: {activeRound?.scorePercent || alignment?.scorePercent || scorePct}%
                    </span>
                  )}
                </div>
                <div className="text-amber-100/90 bg-amber-500/10 px-3 py-2 rounded-lg border border-amber-500/25 leading-relaxed text-[11.5px] whitespace-pre-wrap">
                  {isEvaluating && !activeRound?.critique
                    ? "⚡ 决策模型正在根据评估细则对意图与方案进行全方位审阅..."
                    : activeRound?.critique || alignment?.reason || pendingIntervention?.critique}
                </div>
              </div>
            )}
          </div>

          {/* 全程随时可控的干预与跳过工具条（只要门控未结束或等待人工决策） */}
          {inProgress && (
            <div className="pt-1">
              {showHintInput ? (
                <div className="space-y-2 pt-1">
                  <textarea
                    className="w-full bg-panel border border-amber-500/40 focus:border-amber-400 rounded-lg p-2.5 text-[12px] text-ink outline-none resize-none min-h-[64px]"
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
