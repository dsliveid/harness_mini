import { useState, useEffect, useMemo } from "react";
import { useStore } from "../store";
import {
  ShieldCheck,
  AlertTriangle,
  Loader2,
  ChevronRight,
  Send,
  Lightbulb,
  Brain,
  Copy,
  Check,
  CheckCircle2,
} from "./Icons";
import type { IntentAlignmentEvent, IntentInterventionRequest } from "../types";

export interface IntentAlignmentCardProps {
  alignment?: IntentAlignmentEvent;
  pendingIntervention?: IntentInterventionRequest | null;
  sessionId?: string;
  defaultExpanded?: boolean;
  className?: string;
}

export function IntentAlignmentCard(props?: IntentAlignmentCardProps) {
  const currentId = useStore((s) => s.currentId);
  const effectiveSessionId = props?.sessionId || currentId;
  const storeAlignment = useStore((s) => (effectiveSessionId ? s.intentAlignments[effectiveSessionId] : undefined));
  const storePending = useStore((s) => s.pendingIntentIntervention);
  const pushToast = useStore((s) => s.pushToast);
  const respondIntentIntervention = useStore((s) => s.respondIntentIntervention);

  const alignment = props?.alignment ?? storeAlignment;
  const pendingIntervention =
    props?.pendingIntervention !== undefined
      ? props.pendingIntervention
      : storePending?.sessionId === effectiveSessionId
      ? storePending
      : null;

  // 状态判定
  const isRequiresIntervention =
    Boolean(pendingIntervention && pendingIntervention.sessionId === effectiveSessionId) ||
    alignment?.status === "requires_intervention";
  const isPassed = alignment?.status === "passed" || alignment?.passed === true;
  const isSkipped = alignment?.status === "skipped";
  const isAnalyzing = alignment?.status === "analyzing";
  const isEvaluating = alignment?.status === "evaluating";
  const inProgress = isAnalyzing || isEvaluating || isRequiresIntervention;

  // 用户折叠状态切换
  const [userToggled, setUserToggled] = useState<boolean | null>(null);
  const [hintText, setHintText] = useState("");
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [showHintInput, setShowHintInput] = useState(false);
  const [copiedKey, setCopiedKey] = useState<string | null>(null);

  useEffect(() => {
    if (isRequiresIntervention) {
      setUserToggled(true);
    }
  }, [isRequiresIntervention]);

  const expanded = userToggled !== null ? userToggled : (isRequiresIntervention || (props?.defaultExpanded ?? false));

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

  if (!effectiveSessionId || (!alignment && !pendingIntervention)) return null;

  const rawScore = alignment?.scorePercent ?? (alignment?.score ? Math.round(alignment.score * 100) : 0);
  const scorePct = pendingIntervention
    ? Math.round(pendingIntervention.score > 1 ? pendingIntervention.score : pendingIntervention.score * 100)
    : rawScore;

  const activeInterventionId = pendingIntervention?.id || alignment?.interventionId || undefined;

  const handleAdopt = async () => {
    setIsSubmitting(true);
    try {
      await respondIntentIntervention("adopt", undefined, activeInterventionId);
    } finally {
      setIsSubmitting(false);
    }
  };

  const handleAdoptPlan1 = async () => {
    setIsSubmitting(true);
    try {
      await respondIntentIntervention("adopt_plan_1", undefined, activeInterventionId);
    } finally {
      setIsSubmitting(false);
    }
  };

  const handleAdoptPlan2 = async () => {
    setIsSubmitting(true);
    try {
      await respondIntentIntervention("adopt_plan_2", undefined, activeInterventionId);
    } finally {
      setIsSubmitting(false);
    }
  };

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

  const understandingContent =
    pendingIntervention?.understanding ||
    alignment?.understanding ||
    "";

  const planContent =
    pendingIntervention?.plan ||
    alignment?.plan ||
    "";

  const critiqueContent =
    pendingIntervention?.critique ||
    alignment?.reason ||
    "";

  const hasDualPlans = Boolean(pendingIntervention?.plan1 && pendingIntervention?.plan2);

  const fullContentToCopy = useMemo(() => {
    const parts: string[] = [];
    if (understandingContent) parts.push(`【意图】${understandingContent}`);
    if (planContent) parts.push(`【动作】\n${planContent}`);
    if (critiqueContent) parts.push(`【提示】${critiqueContent}`);
    return parts.join("\n\n");
  }, [understandingContent, planContent, critiqueContent]);

  // 1. 分析与评估中的轻量状态条
  if (isAnalyzing || isEvaluating) {
    return (
      <div
        className={`rounded-lg border border-sky-500/30 bg-sky-500/5 px-3 py-2 flex items-center justify-between text-[12px] text-sky-300 animate-pulse ${
          props?.className || ""
        }`}
      >
        <div className="flex items-center gap-2">
          {isEvaluating ? (
            <Loader2 size={13} className="text-purple-400 animate-spin shrink-0" />
          ) : (
            <Brain size={13} className="text-sky-400 shrink-0" />
          )}
          <span>
            {isEvaluating ? "决策模型正在评估意图契合度…" : "大模型正在理解输入与规划行动…"}
          </span>
        </div>
        <button
          type="button"
          onClick={handleSkip}
          className="text-[11px] text-inkdim hover:text-ink underline cursor-pointer select-none"
        >
          直接执行
        </button>
      </div>
    );
  }

  // 2. 已跳过状态
  if (isSkipped) {
    return (
      <div
        className={`rounded-lg border border-edge/60 bg-panel2/40 px-3 py-1.5 text-[11.5px] text-inkdim flex items-center justify-between select-none ${
          props?.className || ""
        }`}
      >
        <span>已跳过前置意图评估，按原定逻辑直接执行</span>
      </div>
    );
  }

  // 3. 已通过状态 (评分 >= 90)
  if (isPassed) {
    return (
      <div
        className={`rounded-lg border border-emerald-500/25 bg-emerald-500/5 px-3 py-1.5 transition-all text-[12px] select-text ${
          props?.className || ""
        }`}
      >
        <div
          role="button"
          tabIndex={0}
          className="flex items-center justify-between gap-2 cursor-pointer select-none group"
          onClick={() => setUserToggled(!expanded)}
          onKeyDown={(e) => {
            if (e.key === "Enter" || e.key === " ") {
              e.preventDefault();
              setUserToggled(!expanded);
            }
          }}
          title={expanded ? "点击收起详情" : "点击查看行动要点"}
        >
          <div className="flex items-center gap-2 min-w-0 flex-1">
            <ShieldCheck size={13} className="text-emerald-400 shrink-0" />
            <span className="font-medium text-emerald-400 shrink-0">意图已对齐</span>
            {scorePct > 0 && (
              <span className="text-[10.5px] font-mono px-1.5 py-0.2 rounded bg-emerald-500/15 text-emerald-300 shrink-0">
                {scorePct}%
              </span>
            )}
            <span className="text-inkdim truncate flex-1 font-mono text-[11.5px]">
              {understandingContent || "意图明确，直接执行"}
            </span>
          </div>
          <div className="flex items-center gap-1.5 shrink-0">
            {fullContentToCopy && (
              <button
                type="button"
                className="p-1 rounded hover:bg-panel3 text-inkdim hover:text-ink transition-colors cursor-pointer"
                title="复制意图与动作要点"
                onClick={(e) => handleCopy(e, "all", fullContentToCopy, "意图与动作")}
              >
                {copiedKey === "all" ? <Check size={11} className="text-emerald-400" /> : <Copy size={11} />}
              </button>
            )}
            <ChevronRight
              size={12}
              className={`text-inkdim transition-transform duration-150 ${expanded ? "rotate-90" : ""}`}
            />
          </div>
        </div>

        {expanded && planContent && (
          <div className="mt-2 pt-2 border-t border-emerald-500/15 text-[11.5px] text-inkdim leading-relaxed whitespace-pre-wrap font-mono animate-in fade-in duration-150">
            {planContent}
          </div>
        )}
      </div>
    );
  }

  // 4. 待用户决策状态 (< 90 分)：极简内联卡片，不超 4 行，直观二选一
  return (
    <div
      className={`rounded-xl border border-amber-500/40 bg-amber-500/5 px-3.5 py-3 space-y-2.5 transition-all shadow-sm select-text ${
        props?.className || ""
      }`}
    >
      {/* 头部：提示与简明评分 */}
      <div className="flex items-center justify-between select-none">
        <div className="flex items-center gap-1.5">
          <AlertTriangle size={14} className="text-amber-400 shrink-0" />
          <span className="font-medium text-[12.5px] text-amber-300">
            意图契合度待确认
          </span>
          {scorePct > 0 && (
            <span className="text-[11px] font-mono px-1.5 py-0.2 rounded bg-amber-500/20 text-amber-300 border border-amber-500/30">
              {scorePct}分
            </span>
          )}
        </div>
        <button
          type="button"
          onClick={handleSkip}
          disabled={isSubmitting}
          className="text-[11px] text-inkdim hover:text-ink underline cursor-pointer"
          title="跳过对齐，按大模型原定动作直接执行"
        >
          直接执行
        </button>
      </div>

      {/* 极简内容区（每项 1~2 行，清晰易读） */}
      <div className="bg-panel2/70 border border-edge/40 rounded-lg p-2.5 space-y-1.5 text-[12px] leading-relaxed select-text">
        {understandingContent && (
          <div className="flex items-start gap-1.5">
            <span className="text-amber-400/90 font-medium shrink-0">🎯 理解:</span>
            <span className="text-ink font-mono">{understandingContent}</span>
          </div>
        )}
        {planContent && (
          <div className="flex items-start gap-1.5">
            <span className="text-cyan-400/90 font-medium shrink-0">📋 动作:</span>
            <span className="text-inkdim font-mono whitespace-pre-wrap">{planContent}</span>
          </div>
        )}
        {critiqueContent && (
          <div className="flex items-start gap-1.5 pt-1 border-t border-edge/30">
            <span className="text-amber-300/90 font-medium shrink-0">💡 提示:</span>
            <span className="text-amber-200/90 font-mono">{critiqueContent}</span>
          </div>
        )}
      </div>

      {/* 操作按钮区：单轮直观二选一 */}
      <div className="select-none pt-0.5">
        {showHintInput ? (
          <div className="space-y-2">
            <textarea
              className="w-full bg-panel border border-amber-500/40 focus:border-amber-400 rounded-lg p-2 text-[12px] text-ink outline-none resize-none min-h-[56px] select-text cursor-text"
              placeholder="请输入调整提示（例如：'请不要修改配置文件，只修复单元测试'）..."
              value={hintText}
              onChange={(e) => setHintText(e.target.value)}
              disabled={isSubmitting}
              autoFocus
            />
            <div className="flex items-center justify-end gap-2">
              <button
                type="button"
                className="px-2.5 py-1 rounded-md border border-edge hover:bg-panel3 text-[11.5px] text-inkdim hover:text-ink cursor-pointer transition-colors"
                onClick={() => setShowHintInput(false)}
                disabled={isSubmitting}
              >
                取消
              </button>
              <button
                type="button"
                className="px-3 py-1 rounded-md bg-amber-500 hover:bg-amber-400 text-zinc-950 font-medium text-[11.5px] flex items-center gap-1.5 cursor-pointer transition-colors shadow-sm disabled:opacity-50"
                onClick={handleSendHint}
                disabled={isSubmitting || !hintText.trim()}
              >
                {isSubmitting ? <Loader2 size={12} className="animate-spin" /> : <Send size={12} />}
                <span>提交并重新评估</span>
              </button>
            </div>
          </div>
        ) : (
          <div className="flex items-center gap-2">
            {hasDualPlans ? (
              <>
                <button
                  type="button"
                  className="px-3 py-1.5 rounded-lg bg-emerald-600 hover:bg-emerald-500 text-white font-medium text-[12px] flex items-center gap-1.5 cursor-pointer transition-colors shadow-sm disabled:opacity-50"
                  onClick={handleAdoptPlan1}
                  disabled={isSubmitting}
                >
                  <CheckCircle2 size={13} />
                  <span>采用决策 (1)</span>
                </button>
                <button
                  type="button"
                  className="px-3 py-1.5 rounded-lg bg-emerald-600 hover:bg-emerald-500 text-white font-medium text-[12px] flex items-center gap-1.5 cursor-pointer transition-colors shadow-sm disabled:opacity-50"
                  onClick={handleAdoptPlan2}
                  disabled={isSubmitting}
                >
                  <CheckCircle2 size={13} />
                  <span>采用决策 (2)</span>
                </button>
              </>
            ) : (
              <button
                type="button"
                className="px-3.5 py-1.5 rounded-lg bg-emerald-600 hover:bg-emerald-500 text-white font-medium text-[12px] flex items-center gap-1.5 cursor-pointer transition-colors shadow-sm disabled:opacity-50"
                onClick={handleAdopt}
                disabled={isSubmitting}
              >
                <CheckCircle2 size={13} />
                <span>采用该决策</span>
              </button>
            )}

            <button
              type="button"
              className="px-3.5 py-1.5 rounded-lg bg-amber-500 hover:bg-amber-400 text-zinc-950 font-medium text-[12px] flex items-center gap-1.5 cursor-pointer transition-colors shadow-sm disabled:opacity-50"
              onClick={() => setShowHintInput(true)}
              disabled={isSubmitting}
            >
              <Lightbulb size={13} />
              <span>输入提示，调整方向</span>
            </button>
          </div>
        )}
      </div>
    </div>
  );
}
