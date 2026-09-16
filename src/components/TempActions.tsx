import { ipc } from "../ipc";
import { currentSession, useStore } from "../store";
import { DRAFT_ID } from "../types";
import { askConfirm } from "./PromptModal";
import { GitCompare, FolderOpen, GitMerge, Trash2 } from "./Icons";

/**
 * 临时空间浮动操作组：悬浮在输入框上方（左对齐），仅临时空间对话（含未落库草稿）显示。
 * - 变更：查看变更列表（相对基线），无变更 / 目录不存在时禁用
 * - 临时目录：打开临时空间根目录，目录不存在时禁用
 * - 合并：把本次变更写回原项目目录；无变更 / 已合并时禁用
 * - 清空空间：删除整个临时空间目录，目录不存在时禁用
 */
export function TempActions() {
  const currentId = useStore((s) => s.currentId);
  const session = useStore((s) => currentSession(s));
  const draft = useStore((s) => s.draft);
  const tempInfo = useStore((s) => s.tempInfo);
  const setShowChanges = useStore((s) => s.setShowChanges);
  const pushToast = useStore((s) => s.pushToast);

  const isTempDraft = currentId === DRAFT_ID && !!draft?.temp;
  if (!isTempDraft && !session?.isTemp) return null;

  const info = session ? tempInfo[session.id] : undefined;
  // 草稿尚未落库：临时空间必然不存在
  const exists = session ? info?.exists ?? false : false;
  const hasChanges = info?.hasChanges ?? false;
  const mergedPending = info?.mergedPending ?? false;
  const root = info?.tempRoot ?? draft?.temp?.root ?? "";

  const onMerge = async () => {
    if (!session) return;
    const n = info?.changedCount ?? 0;
    if (!(await askConfirm(`确认把本次变更（${n} 个文件）合并回原项目目录？\n此操作会直接写入原目录文件。`, "合并"))) {
      return;
    }
    try {
      const r = await ipc.mergeTempSpace(session.id);
      const done = r.totalApplied + r.totalAiMerged;
      pushToast(
        `合并完成：${done} 个文件已写回（AI 智能合并 ${r.totalAiMerged}）` +
          (r.totalSkipped > 0 ? `，${r.totalSkipped} 个需人工处理` : "")
      );
    } catch (e) {
      pushToast(String(e));
    }
  };

  const onClear = async () => {
    if (!session) return;
    if (!(await askConfirm(`确认删除临时空间目录？\n${root}`, "删除"))) return;
    // 删除整棵临时目录可能耗时：期间在对话区显示「删除中」遮罩并屏蔽交互
    const setTempClearing = useStore.getState().setTempClearing;
    setTempClearing(true);
    try {
      await ipc.clearTempSpace(session.id);
      pushToast("临时空间已清空，可继续发送消息");
    } catch (e) {
      pushToast(String(e));
    } finally {
      setTempClearing(false);
    }
  };

  const cls = (disabled: boolean) =>
    `px-2.5 py-1.5 rounded-lg text-[12px] border border-edge flex items-center gap-1.5 transition-colors ${
      disabled ? "opacity-40 cursor-not-allowed text-inkdim" : "text-ink hover:bg-panel3 shadow-sm"
    }`;

  return (
    <div className="absolute left-5 bottom-full mb-1.5 z-30 flex items-center gap-1.5 bg-panel2/95 backdrop-blur-md border border-edge rounded-xl px-2 py-1.5 shadow-xl select-none animate-in fade-in slide-in-from-bottom-2 duration-150">
      <button
        className={cls(!exists || !hasChanges)}
        disabled={!exists || !hasChanges}
        title={hasChanges ? `查看变更列表（${info?.changedCount ?? 0} 个文件）` : "暂无变更内容"}
        onClick={() => setShowChanges(true)}
      >
        <GitCompare size={13} className="text-accent shrink-0" />
        <span>变更</span>
        {hasChanges && (
          <span className="text-[10px] px-1 rounded-full bg-accent/20 text-accent font-medium">
            {info?.changedCount ?? 0}
          </span>
        )}
      </button>
      <button
        className={cls(!exists)}
        disabled={!exists}
        title={exists ? `打开临时空间目录\n${root}` : "临时空间尚未创建（发送首条消息后创建）"}
        onClick={() => ipc.openDir(root).catch((e) => pushToast(String(e)))}
      >
        <FolderOpen size={13} className="text-amber-400 shrink-0" />
        <span>临时目录</span>
      </button>
      <button
        className={cls(!exists || !hasChanges || mergedPending)}
        disabled={!exists || !hasChanges || mergedPending}
        title={
          mergedPending
            ? "本次变更已合并，清空临时空间后可继续"
            : hasChanges
            ? "把本次变更合并回原项目目录（冲突由 AI 智能合并）"
            : "暂无变更内容"
        }
        onClick={() => void onMerge()}
      >
        <GitMerge size={13} className="text-green-400 shrink-0" />
        <span>合并</span>
      </button>
      <button
        className={`${cls(!exists)} ${exists ? "text-red-400 hover:text-red-300" : ""}`}
        disabled={!exists}
        title={exists ? `删除临时空间：\n${root}` : "临时空间尚未创建"}
        onClick={() => void onClear()}
      >
        <Trash2 size={13} className="shrink-0" />
        <span>清空空间</span>
      </button>
    </div>
  );
}
