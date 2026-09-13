import { ipc } from "../ipc";
import { currentQueue, useStore } from "../store";

export function PendingQueue() {
  const queue = useStore((s) => currentQueue(s));
  const currentId = useStore((s) => s.currentId);
  if (!currentId || queue.length === 0 || currentId === "draft") return null;

  return (
    <div className="border-t border-edge bg-panel2 px-4 py-2">
      <div className="text-[11px] text-inkdim mb-1">待执行（{queue.length}）— 引导将立即介入当前对话，否则在当前对话结束后自动依次执行</div>
      <div className="flex flex-col gap-1 max-h-32 overflow-y-auto">
        {queue.map((q) => (
          <div key={q.id} className="flex items-center gap-2 group">
            <div className="flex-1 min-w-0 text-[13px] text-inkdim truncate">{q.content}</div>
            <button
              className="text-[12px] px-2 py-0.5 rounded bg-accent/15 text-accent hover:bg-accent/25 shrink-0"
              onClick={() => ipc.guideMessage(currentId, q.id)}
              title="立即写入正在进行的对话，引导其方向"
            >
              引导
            </button>
            <button
              className="text-[12px] px-2 py-0.5 rounded bg-panel3 text-inkdim hover:text-red-400 shrink-0"
              onClick={() => ipc.deleteQueuedMessage(currentId, q.id)}
            >
              删除
            </button>
          </div>
        ))}
      </div>
    </div>
  );
}
