/**
 * 设置类弹窗的统一外壳部件：
 * - 右上角关闭叉（弹窗不再支持点击遮罩关闭，只能手动关闭）
 * - 底部统一操作条：取消（丢弃并关闭）/ 应用（只保存，不关闭）/ 保存（保存并关闭）
 */

import { X } from "./Icons";

export function ModalClose({ onClick, title = "关闭" }: { onClick: () => void; title?: string }) {
  return (
    <button
      className="ml-auto shrink-0 w-7 h-7 rounded-lg hover:bg-panel3 text-inkdim hover:text-ink flex items-center justify-center transition-colors"
      title={title}
      onClick={onClick}
    >
      <X size={15} strokeWidth={2} />
    </button>
  );
}

export function ModalActions({
  onCancel,
  onApply,
  onSave,
  canApply,
  busy,
}: {
  onCancel: () => void;
  onApply: () => void;
  onSave: () => void;
  /** 无待保存改动时禁用「应用」 */
  canApply: boolean;
  busy: boolean;
}) {
  return (
    <div className="px-5 py-3.5 border-t border-edge flex justify-end gap-2 shrink-0">
      <button className="px-4 py-1.5 rounded-lg text-inkdim hover:bg-panel3" onClick={onCancel}>
        取消
      </button>
      <button
        className="px-4 py-1.5 rounded-lg bg-panel3 hover:bg-edge text-ink disabled:opacity-40 disabled:cursor-not-allowed"
        disabled={!canApply || busy}
        title={canApply ? "保存数据，不关闭窗口" : "暂无改动"}
        onClick={onApply}
      >
        应用
      </button>
      <button
        className="px-4 py-1.5 rounded-lg bg-accent hover:bg-blue-500 text-white disabled:opacity-50"
        disabled={busy}
        onClick={onSave}
      >
        {busy ? "保存中…" : "保存"}
      </button>
    </div>
  );
}
