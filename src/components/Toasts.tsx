import React from "react";
import { useStore, type ToastType } from "../store";
import { CheckCircle2, XCircle, AlertTriangle, Info, X } from "./Icons";

interface ToastStyle {
  border: string;
  icon: React.ComponentType<{ size?: number | string; className?: string }>;
  iconClass: string;
  shadow: string;
}

const TOAST_THEMES: Record<ToastType, ToastStyle> = {
  success: {
    border: "border-emerald-500/40 hover:border-emerald-500/70",
    icon: CheckCircle2,
    iconClass: "text-emerald-400",
    shadow: "shadow-[0_8px_30px_rgba(16,185,129,0.15)]",
  },
  error: {
    border: "border-red-500/40 hover:border-red-500/70",
    icon: XCircle,
    iconClass: "text-red-400",
    shadow: "shadow-[0_8px_30px_rgba(239,68,68,0.15)]",
  },
  warning: {
    border: "border-amber-500/40 hover:border-amber-500/70",
    icon: AlertTriangle,
    iconClass: "text-amber-400",
    shadow: "shadow-[0_8px_30px_rgba(245,158,11,0.15)]",
  },
  info: {
    border: "border-sky-500/40 hover:border-sky-500/70",
    icon: Info,
    iconClass: "text-sky-400",
    shadow: "shadow-[0_8px_30px_rgba(14,165,233,0.15)]",
  },
};

export function Toasts() {
  const toasts = useStore((s) => s.toasts);
  const dismiss = useStore((s) => s.dismissToast);
  if (toasts.length === 0) return null;
  return (
    <div className="fixed top-3 right-3 z-[100] flex flex-col gap-2 max-w-md pointer-events-none">
      {toasts.map((t) => {
        const theme = TOAST_THEMES[t.type] || TOAST_THEMES.info;
        const IconComponent = theme.icon;
        return (
          <div
            key={t.id}
            onClick={() => dismiss(t.id)}
            className={`pointer-events-auto bg-panel2/95 backdrop-blur border ${theme.border} text-ink rounded-xl px-3.5 py-2.5 shadow-2xl ${theme.shadow} cursor-pointer text-[13px] flex items-start gap-2.5 transition-all`}
          >
            <IconComponent size={16} className={`${theme.iconClass} shrink-0 mt-0.5`} />
            <span className="flex-1 leading-snug">{t.text}</span>
            <button
              onClick={(e) => {
                e.stopPropagation();
                dismiss(t.id);
              }}
              className="text-inkdim hover:text-ink shrink-0 p-0.5 rounded hover:bg-panel3 transition-colors"
            >
              <X size={13} />
            </button>
          </div>
        );
      })}
    </div>
  );
}
