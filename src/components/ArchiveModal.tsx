import { useEffect, useState } from "react";
import { ipc } from "../ipc";
import { useStore } from "../store";
import type { Session } from "../types";
import { askConfirm } from "./PromptModal";
import { Archive, Eye, RotateCcw, Trash2, X } from "./Icons";

export function ArchiveModal() {
  const show = useStore((s) => s.showArchive);
  const setShow = useStore((s) => s.setShowArchive);
  const selectSession = useStore((s) => s.selectSession);
  const pushToast = useStore((s) => s.pushToast);
  const [items, setItems] = useState<Session[]>([]);

  const refresh = async () => {
    try {
      setItems(await ipc.listArchived());
    } catch (e) {
      pushToast(String(e));
    }
  };

  useEffect(() => {
    if (show) void refresh();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [show]);

  if (!show) return null;

  return (
    <div className="fixed inset-0 z-[80] bg-black/50 flex items-center justify-center" onMouseDown={() => setShow(false)}>
      <div
        className="bg-panel2 border border-edge rounded-2xl w-[580px] max-h-[72vh] flex flex-col shadow-2xl overflow-hidden"
        onMouseDown={(e) => e.stopPropagation()}
      >
        <div className="h-12 px-5 border-b border-edge/60 font-medium flex items-center justify-between shrink-0">
          <div className="flex items-center gap-2">
            <Archive size={16} className="text-amber-400" />
            <span>归档的对话</span>
          </div>
          <button
            className="w-7 h-7 rounded-lg hover:bg-panel3 flex items-center justify-center text-inkdim hover:text-ink transition-colors"
            onClick={() => setShow(false)}
          >
            <X size={15} />
          </button>
        </div>

        <div className="flex-1 overflow-y-auto px-5 py-4 flex flex-col gap-2.5">
          {items.map((s) => (
            <div key={s.id} className="flex items-center gap-3 bg-panel border border-edge rounded-xl px-3.5 py-2.5 hover:border-edge/80 transition-colors">
              <div className="flex-1 min-w-0">
                <div className="truncate text-[13px] font-medium text-ink">{s.title}</div>
                <div className="text-[11px] text-inkdim truncate mt-0.5 font-mono">
                  {s.workspacePath}
                  {s.lastMessageAt ? ` · ${new Date(s.lastMessageAt).toLocaleString("zh-CN")}` : ""}
                </div>
              </div>
              <button
                className="text-[12px] px-2.5 py-1 rounded-lg bg-accent/15 text-accent hover:bg-accent/25 shrink-0 flex items-center gap-1 transition-colors"
                onClick={() => {
                  setShow(false);
                  void selectSession(s.id, true);
                }}
                title="只读查看，取消归档后可继续对话"
              >
                <Eye size={12} />
                <span>查看</span>
              </button>
              <button
                className="text-[12px] px-2.5 py-1 rounded-lg bg-panel3 hover:bg-edge text-inkdim hover:text-ink shrink-0 flex items-center gap-1 transition-colors"
                onClick={async () => {
                  try {
                    await ipc.unarchiveSession(s.id);
                    await refresh();
                  } catch (e) {
                    pushToast(String(e));
                  }
                }}
              >
                <RotateCcw size={12} />
                <span>取消归档</span>
              </button>
              <button
                className="text-[12px] px-2.5 py-1 rounded-lg bg-panel3 hover:bg-red-500/20 text-red-400 hover:text-red-300 shrink-0 flex items-center gap-1 transition-colors"
                onClick={async () => {
                  if (await askConfirm(`确定删除归档对话「${s.title}」？数据不可恢复`)) {
                    try {
                      await ipc.deleteSession(s.id);
                      await refresh();
                    } catch (e) {
                      pushToast(String(e));
                    }
                  }
                }}
              >
                <Trash2 size={12} />
                <span>删除</span>
              </button>
            </div>
          ))}
          {items.length === 0 && (
            <div className="flex-1 flex flex-col items-center justify-center py-12 text-center text-inkdim">
              <div className="w-12 h-12 rounded-2xl bg-amber-500/10 border border-amber-500/20 flex items-center justify-center text-amber-400 mb-3">
                <Archive size={22} />
              </div>
              <div className="text-[14px] font-medium text-ink mb-1">暂无归档对话</div>
              <div className="text-[12px] max-w-xs text-inkdim/80">
                对话列表中点击会话条目的「…」菜单可随时将历史对话归档保存。
              </div>
            </div>
          )}
        </div>
        <div className="px-5 py-3 border-t border-edge flex justify-end shrink-0">
          <button
            className="px-4 py-1.5 rounded-lg text-inkdim hover:bg-panel3 text-[13px] transition-colors"
            onClick={() => setShow(false)}
          >
            关闭
          </button>
        </div>
      </div>
    </div>
  );
}
