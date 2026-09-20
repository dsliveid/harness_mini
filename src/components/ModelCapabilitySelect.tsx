import React from "react";
import type { ModelCapability, Provider, Settings } from "../types";
import { hasModelCapability, MODEL_CAPABILITY_METAS } from "../types";

export interface ModelCapabilitySelectProps {
  capability: ModelCapability;
  value: string; // "providerId::modelId" 或 ""
  onChange: (value: string) => void;
  providers: Provider[];
  settings: Settings;
  allowInherit?: boolean;
  inheritLabel?: string;
  className?: string;
  disabled?: boolean;
}

export function ModelCapabilitySelect({
  capability,
  value,
  onChange,
  providers,
  settings,
  allowInherit = true,
  inheritLabel = "跟随系统全局默认",
  className = "",
  disabled = false,
}: ModelCapabilitySelectProps) {
  const meta = MODEL_CAPABILITY_METAS[capability] || {
    id: capability,
    label: capability,
    icon: "⚙️",
  };

  // 检查当前选中项是否具备能力
  let isCurrentValid = true;
  if (value) {
    const parts = value.split("::");
    if (parts.length === 2) {
      isCurrentValid = hasModelCapability(settings, parts[0], parts[1], capability);
    }
  }

  return (
    <div className="w-full">
      <select
        value={value}
        disabled={disabled}
        onChange={(e) => onChange(e.target.value)}
        className={`w-full px-2.5 py-1.5 rounded-lg bg-panel2 border text-[12px] focus:outline-none transition-colors truncate ${
          !isCurrentValid
            ? "border-rose-500/60 text-rose-300 focus:border-rose-400"
            : "border-edge text-ink focus:border-accent"
        } ${className}`}
      >
        {allowInherit && (
          <option value="" className="text-inkdim bg-panel2">
            {inheritLabel}
          </option>
        )}

        {providers.map((p) => {
          if (!p.models || p.models.length === 0) return null;
          return (
            <optgroup key={p.id} label={p.name} className="bg-panel2 text-ink font-semibold">
              {p.models.map((m) => {
                const isSupported = hasModelCapability(settings, p.id, m, capability);
                const optKey = `${p.id}::${m}`;
                return (
                  <option
                    key={optKey}
                    value={optKey}
                    disabled={!isSupported}
                    className={
                      isSupported
                        ? "text-ink bg-panel2 font-normal"
                        : "text-zinc-500 bg-zinc-900/60 font-normal italic"
                    }
                  >
                    {m} {isSupported ? "" : `(不支持${meta.label})`}
                  </option>
                );
              })}
            </optgroup>
          );
        })}
      </select>

      {!isCurrentValid && value && (
        <div className="text-[10.5px] text-rose-400 mt-1 flex items-center gap-1">
          <span>⚠️</span>
          <span>当前选中的模型不支持【{meta.label}】能力，请更换为可选模型</span>
        </div>
      )}
    </div>
  );
}
