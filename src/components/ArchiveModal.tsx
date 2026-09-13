import { useEffect, useState } from "react";
import { ipc } from "../ipc";
import { useStore } from "../store";
import type { Session } from "../types";
import { askConfirm } from "./PromptModal";

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
        className="bg-panel2 border border-edge rounded-2xl w-[560px] max-h-[70vh] flex flex-col shadow-2xl"
        onMouseDown={(e) => e.stopPropagation()}
      >
        <div className="px-5 py-4 border-b border-edge font-medium">归档的对话</div>
        <div className="flex-1 overflow-y-auto px-5 py-4 flex flex-col gap-2">
          {items.map((s) => (
            <div key={s.id} className="flex items-center gap-3 bg-panel border border-edge rounded-xl px-3 py-2.5">
              <div className="flex-1 min-w-0">
                <div className="truncate text-[13px]">{s.title}</div>
                <div className="text-[11px] text-inkdim truncate">
                  {s.workspacePath}
                  {s.lastMessageAt ? ` · ${new Date(s.lastMessageAt).toLocaleString("zh-CN")}` : ""}
                </div>
              </div>
              <button
                className="text-[12px] px-2.5 py-1 rounded-lg bg-accent/15 text-accent hover:bg-accent/25 shrink-0"
                onClick={() => {
                  setShow(false);
                  void selectSession(s.id, true);
                }}
                title="只读查看，取消归档后可继续对话"
              >
                查看
              </button>
              <button
                className="text-[12px] px-2.5 py-1 rounded-lg bg-panel3 hover:bg-edge shrink-0"
                onClick={async () => {
                  try {
                    await ipc.unarchiveSession(s.id);
                    await refresh();
                  } catch (e) {
                    pushToast(String(e));
                  }
                }}
              >
                取消归档
              </button>
              <button
                className="text-[12px] px-2.5 py-1 rounded-lg bg-panel3 hover:bg-edge text-red-400 shrink-0"
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
                删除
              </button>
            </div>
          ))}
          {items.length === 0 && <div className="text-inkdim text-[13px]">暂无归档。会话条目的「…」菜单中可执行归档。</div>}
        </div>
        <div className="px-5 py-3 border-t border-edge flex justify-end">
          <button className="px-4 py-1.5 rounded-lg text-inkdim hover:bg-panel3" onClick={() => setShow(false)}>
            关闭
          </button>
        </div>
      </div>
    </div>
  );
}
