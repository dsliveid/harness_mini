import { useState, useMemo, useRef, useEffect } from "react";
import { ipc } from "../ipc";
import { useStore } from "../store";
import {
  DRAFT_ID,
  formatTokens,
  resolveActiveModel,
  resolveModelContextLimit,
  type Message,
  type Session,
  type SessionCompaction,
  type Settings,
} from "../types";
import { ModelContextModal } from "./ModelContextModal";
import { Brain, Sliders, Sparkles } from "./Icons";

/**
 * 启发式估算文本 Token 数（与后端 estimate_tokens 逻辑一致）
 * 非 ASCII 字符乘 0.7，ASCII 字符乘 0.3
 */
export function estimateTokens(s: string): number {
  if (!s) return 0;
  let cjk = 0;
  let other = 0;
  for (const ch of s) {
    if (ch.charCodeAt(0) <= 0x7f) other++;
    else cjk++;
  }
  return Math.ceil(cjk * 0.7 + other * 0.3);
}

export interface ContextUsageInfo {
  activeTokens: number;
  limit: number;
  percentage: number;
  remainingTokens: number;
  compactionThreshold: number;
  tokensToCompaction: number;
  hasCompacted: boolean;
  compactionCount: number;
  isEstimated: boolean;
  modelName: string;
  providerName: string;
  providerId: string;
  status: "safe" | "moderate" | "critical";
}

/**
 * 计算当前会话瞬时活跃上下文 Token 负载
 */
export function computeContextUsage(
  msgs: Message[],
  compactions: SessionCompaction[],
  session: Session | null,
  settings: Settings,
  draftLimit?: number | null
): ContextUsageInfo {
  const active = resolveActiveModel(settings);
  const modelName = active?.model ?? "default";
  const providerName = active?.provider.name ?? "";
  const providerId = active?.provider.id ?? "";

  const defaultLimit = active
    ? resolveModelContextLimit(settings, providerId, modelName)
    : settings.contextTokenLimit || 64_000;

  const limit = session?.contextTokenLimit ?? draftLimit ?? defaultLimit;
  const compactionThreshold = Math.round(limit * 0.75);

  const maxCompactedSeq = compactions.reduce((max, c) => Math.max(max, c.endSeq), 0);
  const compactionCount = compactions.length;
  const hasCompacted = compactionCount > 0;

  let activeTokens = 0;
  let isEstimated = false;

  // 倒序定位最后一条已完成并具备 promptTokens 的 assistant 消息
  let lastAssistantIdx = -1;
  for (let i = msgs.length - 1; i >= 0; i--) {
    const m = msgs[i];
    if (m.role === "assistant" && !m.queued && m.seq > maxCompactedSeq) {
      if (
        (m.promptTokens != null && m.promptTokens > 0) ||
        (m.usage?.promptTokens != null && m.usage.promptTokens > 0)
      ) {
        lastAssistantIdx = i;
        break;
      }
    }
  }

  if (lastAssistantIdx >= 0) {
    const lastAsst = msgs[lastAssistantIdx];
    const pt = lastAsst.promptTokens ?? lastAsst.usage?.promptTokens ?? 0;
    const ct = lastAsst.completionTokens ?? lastAsst.usage?.completionTokens ?? 0;
    activeTokens = pt + ct;
    isEstimated =
      lastAsst.isEstimated ??
      (lastAsst.usage?.isEstimated ||
        (lastAsst.usage?.inputEst != null && lastAsst.usage?.promptTokens == null)) ??
      false;

    // 累加最后一条 assistant 之后的后续增量消息（如用户新输入或追加内容）
    for (let i = lastAssistantIdx + 1; i < msgs.length; i++) {
      const m = msgs[i];
      if (m.queued || m.seq <= maxCompactedSeq) continue;
      const text = (m.content || "") + (m.reasoning || "");
      activeTokens += estimateTokens(text);
      if (m.attachments) {
        for (const att of m.attachments) {
          activeTokens += att.is_image ? 1000 : 200;
        }
      }
    }
  } else {
    // 尚无真实 assistant 回复：系统提示词 (~1,500) + 未压缩消息内容估算
    let est = msgs.length > 0 ? 1500 : 0;
    for (const m of msgs) {
      if (m.queued || m.seq <= maxCompactedSeq) continue;
      const text = (m.content || "") + (m.reasoning || "");
      est += estimateTokens(text);
      if (m.attachments) {
        for (const att of m.attachments) {
          est += att.is_image ? 1000 : 200;
        }
      }
    }
    if (hasCompacted) {
      for (const c of compactions) {
        est += estimateTokens(c.summaryMarkdown || "");
      }
    }
    activeTokens = est;
    isEstimated = true;
  }

  const percentage =
    limit > 0 ? Math.min(100, Math.round((activeTokens / limit) * 1000) / 10) : 0;
  const remainingTokens = Math.max(0, limit - activeTokens);
  const tokensToCompaction = Math.max(0, compactionThreshold - activeTokens);

  let status: "safe" | "moderate" | "critical" = "safe";
  if (percentage >= 75) {
    status = "critical";
  } else if (percentage >= 60) {
    status = "moderate";
  }

  return {
    activeTokens,
    limit,
    percentage,
    remainingTokens,
    compactionThreshold,
    tokensToCompaction,
    hasCompacted,
    compactionCount,
    isEstimated,
    modelName,
    providerName,
    providerId,
    status,
  };
}

export interface ContextUsageGaugeProps {
  session?: Session | null;
  className?: string;
}

export function ContextUsageGauge({ session, className = "" }: ContextUsageGaugeProps) {
  const currentId = useStore((s) => s.currentId);
  const draft = useStore((s) => s.draft);
  const settings = useStore((s) => s.settings);
  const pushToast = useStore((s) => s.pushToast);
  const setDraftContextTokenLimit = useStore((s) => s.setDraftContextTokenLimit);

  const sessionId = session?.id ?? currentId ?? DRAFT_ID;
  const messages = useStore((s) => (sessionId ? s.messages[sessionId] ?? [] : []));
  const compactions = useStore((s) => (sessionId ? s.sessionCompactions[sessionId] ?? [] : []));

  const [openModal, setOpenModal] = useState(false);
  const [showTooltip, setShowTooltip] = useState(false);
  const closeTimerRef = useRef<number | null>(null);

  const info = useMemo(() => {
    return computeContextUsage(
      messages,
      compactions,
      session ?? null,
      settings,
      sessionId === DRAFT_ID ? draft?.contextTokenLimit ?? null : null
    );
  }, [messages, compactions, session, settings, sessionId, draft?.contextTokenLimit]);

  const handleMouseEnter = () => {
    if (closeTimerRef.current) {
      clearTimeout(closeTimerRef.current);
      closeTimerRef.current = null;
    }
    setShowTooltip(true);
  };

  const handleMouseLeave = () => {
    closeTimerRef.current = window.setTimeout(() => {
      setShowTooltip(false);
    }, 150);
  };

  useEffect(() => {
    return () => {
      if (closeTimerRef.current) {
        clearTimeout(closeTimerRef.current);
      }
    };
  }, []);

  const handleSaveLimit = async (newLimit: number | null) => {
    if (session) {
      try {
        await ipc.setSessionContextLimit(session.id, newLimit);
      } catch (err) {
        pushToast(String(err));
      }
    } else if (currentId === DRAFT_ID) {
      setDraftContextTokenLimit(newLimit);
    }
  };

  // SVG 环形进度参数
  // 周长 = 2 * PI * r = 2 * 3.14159 * 6.2 ≈ 38.96
  const CIRCLE_R = 6.2;
  const CIRCLE_C = 2 * Math.PI * CIRCLE_R;
  const offset = CIRCLE_C * (1 - Math.min(info.percentage, 100) / 100);

  // 颜色映射
  let ringColorClass = "text-emerald-400";
  let buttonBorderClass = "hover:border-emerald-400/40";
  let statusBadge = (
    <span className="px-1.5 py-0.5 rounded text-[10px] bg-emerald-500/15 text-emerald-400 border border-emerald-500/30 font-medium">
      空间充裕
    </span>
  );

  if (info.status === "critical") {
    ringColorClass = "text-rose-400";
    buttonBorderClass = "border-rose-500/30 hover:border-rose-400/60 bg-rose-500/10 text-rose-300";
    statusBadge = (
      <span className="px-1.5 py-0.5 rounded text-[10px] bg-rose-500/15 text-rose-400 border border-rose-500/30 font-medium animate-pulse">
        接近压缩阈值
      </span>
    );
  } else if (info.status === "moderate") {
    ringColorClass = "text-amber-400";
    buttonBorderClass = "hover:border-amber-400/40";
    statusBadge = (
      <span className="px-1.5 py-0.5 rounded text-[10px] bg-amber-500/15 text-amber-400 border border-amber-500/30 font-medium">
        负载适中
      </span>
    );
  }

  return (
    <div
      className="relative inline-flex items-center"
      onMouseEnter={handleMouseEnter}
      onMouseLeave={handleMouseLeave}
    >
      <button
        type="button"
        className={`inline-flex items-center gap-1.5 px-2 py-0.5 rounded-md text-[11px] transition-all group font-mono cursor-pointer border ${
          info.status === "critical"
            ? buttonBorderClass
            : `bg-panel2/80 hover:bg-panel3 border-edge/80 ${buttonBorderClass} text-ink shadow-xs`
        } ${className}`}
        onClick={() => {
          setShowTooltip(false);
          setOpenModal(true);
        }}
        title="点击配置上下文 Token 上限规格"
      >
        {/* SVG 圆圈进度条 */}
        <svg className="w-3.5 h-3.5 -rotate-90 shrink-0" viewBox="0 0 16 16">
          <circle
            cx="8"
            cy="8"
            r={CIRCLE_R}
            fill="none"
            stroke="currentColor"
            className="text-edge/60"
            strokeWidth="2.2"
          />
          <circle
            cx="8"
            cy="8"
            r={CIRCLE_R}
            fill="none"
            stroke="currentColor"
            className={`${ringColorClass} transition-all duration-300`}
            strokeWidth="2.2"
            strokeDasharray={CIRCLE_C}
            strokeDashoffset={offset}
            strokeLinecap="round"
          />
        </svg>

        <span className="font-medium text-ink tracking-tight">{info.percentage}%</span>
        <span className="text-[10px] text-inkdim hidden sm:inline">上下文</span>
      </button>

      {/* 鼠标悬停详情浮层 (Popover) */}
      {showTooltip && (
        <div
          className="absolute bottom-full right-0 mb-2 z-50 w-[310px] bg-panel2/95 border border-edge/80 rounded-xl p-3.5 shadow-2xl backdrop-blur-md animate-in fade-in zoom-in-95 duration-100 text-[12px] cursor-default select-none pointer-events-auto"
          onMouseEnter={handleMouseEnter}
          onMouseLeave={handleMouseLeave}
        >
          {/* 标题栏 */}
          <div className="flex items-center justify-between pb-2 border-b border-edge/40">
            <div className="flex items-center gap-1.5">
              <Brain size={14} className="text-purple-400 shrink-0" />
              <span className="font-semibold text-ink text-[12.5px]">上下文窗口负载</span>
            </div>
            {statusBadge}
          </div>

          {/* 模型与厂商 */}
          <div className="flex items-center justify-between text-[11px] text-inkdim mt-2 font-mono">
            <span className="truncate max-w-[190px]" title={info.modelName}>
              模型: <span className="text-ink font-medium">{info.modelName}</span>
            </span>
            {info.providerName && (
              <span className="truncate max-w-[95px] text-inkdim/80 text-[10.5px]">
                {info.providerName}
              </span>
            )}
          </div>

          {/* Token 数值与比例 */}
          <div className="mt-2.5 flex items-baseline justify-between font-mono">
            <div>
              <span className="text-[16px] font-bold text-ink">
                {info.activeTokens.toLocaleString()}
              </span>
              <span className="text-[10.5px] text-inkdim ml-1">tokens</span>
            </div>
            <div className="text-[11.5px] text-inkdim">
              上限 {formatTokens(info.limit)} (
              <span className={`font-semibold ${info.status === "critical" ? "text-rose-400" : "text-ink"}`}>
                {info.percentage}%
              </span>
              )
            </div>
          </div>

          {/* 线性水平进度条，附带 75% 压缩触发线 */}
          <div className="w-full bg-panel3 h-2 rounded-full overflow-hidden relative mt-1.5 border border-edge/40">
            <div
              className={`h-full rounded-full transition-all duration-300 ${
                info.status === "critical"
                  ? "bg-rose-400"
                  : info.status === "moderate"
                  ? "bg-amber-400"
                  : "bg-emerald-400"
              }`}
              style={{ width: `${Math.min(info.percentage, 100)}%` }}
            />
            {/* 75% 压缩警戒标记 */}
            <div
              className="absolute top-0 bottom-0 w-[1.5px] bg-rose-400/90 z-10"
              style={{ left: "75%" }}
              title="75% 智能语义压缩触发线"
            />
          </div>

          {/* 详细指标明细 */}
          <div className="mt-3 space-y-1.5 text-[11px] text-inkdim border-t border-edge/30 pt-2 font-mono">
            <div className="flex items-center justify-between">
              <span>剩余可用空间:</span>
              <span className="text-ink font-medium">{info.remainingTokens.toLocaleString()} tokens</span>
            </div>
            <div className="flex items-center justify-between">
              <span>智能压缩阈值 (75%):</span>
              <span className="text-inkdim">{formatTokens(info.compactionThreshold)}</span>
            </div>
            <div className="flex items-center justify-between">
              <span>距触发自动压缩:</span>
              <span className={info.tokensToCompaction === 0 ? "text-rose-400 font-medium" : "text-inkdim"}>
                {info.tokensToCompaction > 0
                  ? `剩余 ${info.tokensToCompaction.toLocaleString()} tokens`
                  : "已达阈值 / 压缩生效中"}
              </span>
            </div>
            <div className="flex items-center justify-between">
              <span>历史压缩状态:</span>
              <span className="text-inkdim">
                {info.hasCompacted ? `已压缩 ${info.compactionCount} 次历史` : "未发生压缩 (历史完整)"}
              </span>
            </div>
          </div>

          {/* 底部点击引导 */}
          <div
            className="mt-3 pt-2 border-t border-edge/40 flex items-center justify-between text-[11px] text-accent hover:underline cursor-pointer transition-colors"
            onClick={() => {
              setShowTooltip(false);
              setOpenModal(true);
            }}
          >
            <span className="flex items-center gap-1 font-medium">
              <Sliders size={12} />
              <span>调整本次对话上下文上限</span>
            </span>
            <span className="text-[10px] text-inkdim font-mono">{formatTokens(info.limit)}</span>
          </div>
        </div>
      )}

      {/* 上下文规格调整对话框 */}
      {openModal && (
        <ModelContextModal
          open={openModal}
          providerId={info.providerId}
          providerName={info.providerName}
          modelName={info.modelName}
          currentLimit={info.limit}
          isSessionScope={true}
          onSave={handleSaveLimit}
          onClose={() => setOpenModal(false)}
        />
      )}
    </div>
  );
}
