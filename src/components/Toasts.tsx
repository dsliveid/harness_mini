import { useStore } from "../store";
import { AlertCircle, X } from "./Icons";

export function Toasts() {
  const toasts = useStore((s) => s.toasts);
  const dismiss = useStore((s) => s.dismissToast);
  if (toasts.length === 0) return null;
  return (
    <div className="fixed top-3 right-3 z-[100] flex flex-col gap-2 max-w-md pointer-events-none">
      {toasts.map((t) => (
        <div
          key={t.id}
          onClick={() => dismiss(t.id)}
          className="pointer-events-auto bg-panel2/95 backdrop-blur border border-red-500/40 text-ink rounded-xl px-3.5 py-2.5 shadow-2xl cursor-pointer text-[13px] flex items-start gap-2.5 transition-all hover:border-red-500/70"
        >
          <AlertCircle size={16} className="text-red-400 shrink-0 mt-0.5" />
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
      ))}
    </div>
  );
}
