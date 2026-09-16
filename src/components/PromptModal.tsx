import React from "react";
import { create } from "zustand";
import { AlertTriangle } from "./Icons";

interface PromptOpts {
  title: string;
  value?: string;
  placeholder?: string;
  confirmText?: string;
  danger?: boolean;
  withInput?: boolean; // false = 仅确认框
}

interface PromptState {
  open: boolean;
  title: string;
  value: string;
  placeholder: string;
  confirmText: string;
  danger: boolean;
  withInput: boolean;
  resolve?: (v: string | null) => void;
  ask: (opts: PromptOpts) => Promise<string | null>;
  close: (v: string | null) => void;
  setValue: (v: string) => void;
}

export const usePrompt = create<PromptState>((set, get) => ({
  open: false,
  title: "",
  value: "",
  placeholder: "",
  confirmText: "确定",
  danger: false,
  withInput: true,
  ask: (opts) =>
    new Promise((resolve) => {
      set({
        open: true,
        title: opts.title,
        value: opts.value ?? "",
        placeholder: opts.placeholder ?? "",
        confirmText: opts.confirmText ?? "确定",
        danger: opts.danger ?? false,
        withInput: opts.withInput ?? true,
        resolve,
      });
    }),
  close(v) {
    const r = get().resolve;
    set({ open: false, resolve: undefined });
    r?.(v);
  },
  setValue(v) {
    set({ value: v });
  },
}));

export function askPrompt(opts: PromptOpts) {
  return usePrompt.getState().ask(opts);
}

/** 确认框：确定返回 true，取消返回 false（注意 ask 返回 "" 表示确认，必须转布尔） */
export async function askConfirm(message: string, confirmText = "删除"): Promise<boolean> {
  const r = await usePrompt.getState().ask({ title: message, withInput: false, confirmText, danger: true });
  return r !== null;
}

export function PromptHost() {
  const { open, title, value, placeholder, confirmText, danger, withInput } = usePrompt();
  const close = usePrompt((s) => s.close);
  const setValue = usePrompt((s) => s.setValue);

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Enter") {
      e.preventDefault();
      close(withInput ? value : "");
    } else if (e.key === "Escape") {
      e.preventDefault();
      close(null);
    }
  };

  if (!open) return null;
  return (
    <div className="fixed inset-0 z-[90] bg-black/50 flex items-center justify-center p-4" onMouseDown={() => close(null)}>
      <div
        className="bg-panel2 border border-edge rounded-2xl p-5 w-[420px] max-w-[92vw] shadow-2xl"
        onMouseDown={(e) => e.stopPropagation()}
      >
        <div className="text-[15px] font-medium mb-3 flex items-center gap-2 text-ink">
          {danger && <AlertTriangle size={18} className="text-red-400 shrink-0" />}
          <span className="leading-snug">{title}</span>
        </div>
        {withInput && (
          <input
            autoFocus
            className="w-full bg-panel border border-edge rounded-lg px-3 py-2 mb-4 outline-none focus:border-accent"
            value={value}
            placeholder={placeholder}
            onChange={(e) => setValue(e.target.value)}
            onKeyDown={onKeyDown}
          />
        )}
        <div className="flex justify-end gap-2">
          <button className="px-3 py-1.5 rounded-lg hover:bg-panel3 text-inkdim" onClick={() => close(null)}>
            取消
          </button>
          <button
            className={`px-3 py-1.5 rounded-lg text-white ${danger ? "bg-red-600 hover:bg-red-500" : "bg-accent hover:bg-blue-500"}`}
            onClick={() => close(withInput ? value : "")}
          >
            {confirmText}
          </button>
        </div>
      </div>
    </div>
  );
}
