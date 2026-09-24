import { useEffect, useState } from "react";
import {
  MODEL_CONTEXT_PRESETS,
  inferModelContextLimit,
  formatTokens,
} from "../types";
import { ModalClose } from "./ModalActions";
import { Sparkles, Sliders, RotateCcw } from "./Icons";

export interface ModelContextModalProps {
  open: boolean;
  providerId: string;
  providerName?: string;
  modelName: string;
  currentLimit: number;
  isSessionScope?: boolean;
  onSave: (newLimit: number | null) => void;
  onClose: () => void;
}

export function ModelContextModal({
  open,
  providerName,
  modelName,
  currentLimit,
  isSessionScope = false,
  onSave,
  onClose,
}: ModelContextModalProps) {
  const [val, setVal] = useState<number>(currentLimit);

  useEffect(() => {
    if (open) {
      setVal(currentLimit);
    }
  }, [open, currentLimit]);

  if (!open) return null;

  const inferred = inferModelContextLimit(modelName);
  const compactionThreshold = Math.round(val * 0.75);
  const isDefaultInferred = val === inferred;

  const categories = Array.from(new Set(MODEL_CONTEXT_PRESETS.map((p) => p.category)));

  const handleApplyPreset = (limit: number) => {
    if (limit > 0) {
      setVal(limit);
    }
  };

  const handleResetToInferred = () => {
    setVal(inferred);
  };

  const handleConfirm = () => {
    const finalVal = Math.max(2000, Math.min(val || inferred, 10_000_000));
    onSave(finalVal);
    onClose();
  };

  const inputCls =
    "bg-panel border border-edge rounded-lg px-2.5 py-1.5 text-[13px] outline-none focus:border-accent w-full font-mono";

  return (
    <div className="fixed inset-0 z-[90] bg-black/55 flex items-center justify-center animate-in fade-in duration-150">
      <div className="bg-panel2 border border-edge rounded-2xl w-[480px] max-w-[92vw] flex flex-col shadow-2xl overflow-hidden">
        {/* 标题栏 */}
        <div className="h-12 px-5 border-b border-edge/60 font-medium flex items-center justify-between shrink-0 bg-panel3/30">
          <div className="flex items-center gap-2 min-w-0">
            <Sliders size={15} className={isSessionScope ? "text-amber-400 shrink-0" : "text-accent shrink-0"} />
            <span className="text-[14px]">{isSessionScope ? "调整本次对话上下文上限" : "配置上下文 Token 上限"}</span>
            {isSessionScope && (
              <span className="text-[11px] font-normal text-amber-400 bg-amber-500/10 border border-amber-500/20 px-2 py-0.5 rounded-full shrink-0">
                仅当前对话生效
              </span>
            )}
          </div>
          <ModalClose onClick={onClose} />
        </div>

        {/* 主内容 */}
        <div className="p-5 flex flex-col gap-4 text-[13px]">
          {isSessionScope && (
            <div className="flex items-center gap-2 px-3 py-2 rounded-xl bg-amber-500/10 border border-amber-500/25 text-amber-300 text-[11px] leading-relaxed">
              <Sparkles size={13} className="shrink-0 text-amber-400" />
              <span>调整仅针对本次对话生效，不影响模型的全局预设或其他对话。</span>
            </div>
          )}

          {/* 模型信息横条 */}
          <div className="flex items-center justify-between px-3 py-2 rounded-xl bg-panel3/60 border border-edge/60">
            <div className="flex flex-col min-w-0">
              <span className="text-[11px] text-inkdim">当前对话使用模型</span>
              <span className="font-mono font-medium text-ink truncate max-w-[280px]" title={modelName}>
                {modelName}
              </span>
            </div>
            {providerName && (
              <span className="text-[11px] px-2 py-0.5 rounded-full bg-panel border border-edge text-inkdim shrink-0">
                {providerName}
              </span>
            )}
          </div>

          {/* 快捷规格标签 */}
          <div className="flex flex-col gap-1.5">
            <label className="text-[12px] font-medium text-ink flex items-center justify-between">
              <span>快捷常用规格</span>
              <span className="text-[11px] text-inkdim font-normal">一键快速填入推荐上限</span>
            </label>
            <div className="flex flex-wrap items-center gap-1.5">
              {[
                { label: "1M (800k)", val: 800_000, desc: "GPT-5.5 / Opus 5 / Gemini 3.8 / DeepSeek-v4 / Qwen3.8 / GLM-5.3" },
                { label: "500k (400k)", val: 400_000, desc: "长文本超长上下文" },
                { label: "200k (180k)", val: 180_000, desc: "Claude 3.5 / o1 / Kimi" },
                { label: "128k (110k)", val: 110_000, desc: "GPT-4o / Qwen 2.5 / GLM-4" },
                { label: "64k (56k)", val: 56_000, desc: "经典 64K" },
                { label: "32k (28k)", val: 28_000, desc: "本地 32K" },
                { label: "16k (14k)", val: 14_000, desc: "本地 16K" },
                { label: "8k (7k)", val: 7_000, desc: "本地 8K" },
              ].map((chip) => (
                <button
                  key={chip.val}
                  type="button"
                  title={`${chip.desc} · 推荐上限 ${chip.val.toLocaleString()} tokens`}
                  className={`text-[11px] px-2 py-0.5 rounded-lg border font-mono transition-all cursor-pointer ${
                    val === chip.val
                      ? "bg-accent/20 border-accent text-accent font-semibold"
                      : "bg-panel border-edge hover:border-accent/40 text-inkdim hover:text-ink"
                  }`}
                  onClick={() => setVal(chip.val)}
                >
                  {chip.label}
                </button>
              ))}
            </div>
          </div>

          {/* 下拉预设选择 */}
          <div className="flex flex-col gap-1.5">
            <label className="text-[12px] font-medium text-ink flex items-center justify-between">
              <span>从完整厂商预设列表选择</span>
              <span className="text-[11px] text-inkdim font-normal">包含代表模型规格</span>
            </label>
            <select
              className={`${inputCls} text-[12px] cursor-pointer`}
              value=""
              onChange={(e) => handleApplyPreset(Number(e.target.value))}
            >
              <option value="" disabled>
                从主流模型规格列表中选择...
              </option>
              {categories.map((cat) => (
                <optgroup key={cat} label={cat}>
                  {MODEL_CONTEXT_PRESETS.filter((p) => p.category === cat).map((p) => (
                    <option key={p.id} value={p.recommendedLimit}>
                      {p.name}（{p.desc}）
                    </option>
                  ))}
                </optgroup>
              ))}
            </select>
          </div>

          {/* 手动数字微调输入框 */}
          <div className="flex flex-col gap-1.5">
            <label className="text-[12px] font-medium text-ink flex items-center justify-between">
              <span>手动精确数值（Tokens）</span>
              <span className="text-[11px] font-mono text-accent font-medium">
                {formatTokens(val || 0)} ({(val || 0).toLocaleString()} tokens)
              </span>
            </label>
            <div className="flex items-center gap-2">
              <input
                className={inputCls}
                type="number"
                min={2000}
                max={10000000}
                step={1000}
                value={val || ""}
                placeholder={String(inferred)}
                onChange={(e) => setVal(Number(e.target.value) || 0)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") handleConfirm();
                }}
                autoFocus
              />
              <span className="text-[12px] text-inkdim shrink-0">tokens</span>
            </div>
          </div>

          {/* 实时参数反馈卡片 */}
          <div className="p-3 rounded-xl bg-panel/60 border border-edge/50 flex flex-col gap-1.5 text-[11px] text-inkdim">
            <div className="flex items-center justify-between">
              <span>自动智能压缩阈值：</span>
              <span className="font-mono text-ink font-medium">
                约 {(compactionThreshold || 0).toLocaleString()} tokens (~75%)
              </span>
            </div>
            <div className="flex items-center justify-between">
              <span>配置识别状态：</span>
              {isDefaultInferred ? (
                <span className="text-emerald-400 flex items-center gap-1">
                  <Sparkles size={11} />
                  <span>智能推断标准值</span>
                </span>
              ) : (
                <span className="text-accent">
                  {isSessionScope ? "本会话专属自定义数值" : "用户手动自定义数值"}
                </span>
              )}
            </div>
            <div className="text-[10px] text-inkdim/70 leading-relaxed mt-0.5 border-t border-edge/30 pt-1.5">
              未压缩历史消息达到 75% 阈值时系统将自动提炼 Markdown 备忘录并触发确认压缩；超出上限将触发原子成对淘汰兜底。
            </div>
          </div>
        </div>

        {/* 底部按钮栏 */}
        <div className="px-5 py-3 border-t border-edge/60 bg-panel3/30 flex items-center justify-between shrink-0">
          {isSessionScope ? (
            <button
              type="button"
              className="text-[12px] px-2.5 py-1.5 rounded-lg bg-panel hover:bg-edge text-inkdim hover:text-ink flex items-center gap-1.5 transition-colors border border-edge"
              onClick={() => {
                onSave(null);
                onClose();
              }}
              title="清除本对话专属上限，恢复跟随模型配置与全局默认"
            >
              <RotateCcw size={12} />
              <span>跟随模型默认</span>
            </button>
          ) : (
            <button
              type="button"
              className="text-[12px] px-2.5 py-1.5 rounded-lg bg-panel hover:bg-edge text-inkdim hover:text-ink flex items-center gap-1.5 transition-colors border border-edge"
              onClick={handleResetToInferred}
              title={`恢复为智能推断推荐值 (${(inferred || 0).toLocaleString()} tokens)`}
            >
              <RotateCcw size={12} />
              <span>恢复推荐值</span>
            </button>
          )}

          <div className="flex items-center gap-2">
            <button
              type="button"
              className="text-[12px] px-3.5 py-1.5 rounded-lg bg-panel hover:bg-edge text-inkdim hover:text-ink transition-colors border border-edge"
              onClick={onClose}
            >
              取消
            </button>
            <button
              type="button"
              className="text-[12px] px-4 py-1.5 rounded-lg bg-accent hover:bg-accent/90 text-white font-medium transition-colors shadow-sm"
              onClick={handleConfirm}
            >
              {isSessionScope ? "应用到本次对话" : "确定保存"}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
