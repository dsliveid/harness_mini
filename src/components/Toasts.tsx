import { useStore } from "../store";

export function Toasts() {
  const toasts = useStore((s) => s.toasts);
  const dismiss = useStore((s) => s.dismissToast);
  if (toasts.length === 0) return null;
  return (
    <div className="fixed top-3 right-3 z-[100] flex flex-col gap-2 max-w-md">
      {toasts.map((t) => (
        <div
          key={t.id}
          onClick={() => dismiss(t.id)}
          className="bg-panel3 border border-red-500/40 text-ink rounded-lg px-4 py-2 shadow-lg cursor-pointer text-[13px]"
        >
          {t.text}
        </div>
      ))}
    </div>
  );
}
