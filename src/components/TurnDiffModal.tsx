import { useState, useEffect } from "react";
import { createPortal } from "react-dom";
import { ipc } from "../ipc";
import { useStore } from "../store";
import type { SnapshotFileDiff } from "../types";
import { X, Copy, Check, ExternalLink, GitCompare } from "./Icons";

interface TurnDiffModalProps {
  messageId: string;
  isReverted?: boolean;
  onClose: () => void;
  onReverted?: () => void;
  onReapplied?: () => void;
}

export function TurnDiffModal({
  messageId,
  isReverted = false,
  onClose,
  onReverted: _onReverted,
  onReapplied: _onReapplied,
}: TurnDiffModalProps) {
  const [diffs, setDiffs] = useState<SnapshotFileDiff[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [selectedIdx, setSelectedIdx] = useState(0);
  const [copied, setCopied] = useState(false);

  const pushToast = useStore((s) => s.pushToast);
  const currentWorkspace = useStore(
    (s) =>
      s.sessions.find((x) => x.id === s.currentId)?.workspacePath ||
      s.draft?.workspacePath ||
      s.settings.lastWorkspacePath ||
      ""
  );

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);
    ipc
      .getTurnDiff(messageId)
      .then((res) => {
        if (!cancelled) {
          setDiffs(res);
          setLoading(false);
        }
      })
      .catch((err) => {
        if (!cancelled) {
          setError(String(err));
          setLoading(false);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [messageId]);

  const selectedDiff = diffs[selectedIdx] || null;

  const handleCopyDiff = () => {
    if (!selectedDiff) return;
    navigator.clipboard.writeText(selectedDiff.diffText).then(
      () => {
        setCopied(true);
        setTimeout(() => setCopied(false), 1500);
        pushToast("Diff 已复制到剪贴板");
      },
      () => pushToast("复制失败")
    );
  };

  const handleOpenEditor = (filePath: string) => {
    if (!filePath) return;
    const cleanRel = filePath.replace(/\\/g, "/");
    const absPath =
      /^[a-zA-Z]:\//.test(cleanRel) || cleanRel.startsWith("/")
        ? cleanRel
        : currentWorkspace
        ? `${currentWorkspace.replace(/\\/g, "/")}/${cleanRel.replace(/^\.\//, "")}`
        : cleanRel;
    ipc.openInExternalEditor(absPath);
  };

  const modalContent = (
    <div
      className="fixed inset-0 z-[100] bg-black/75 backdrop-blur-sm flex items-center justify-center p-4 animate-in fade-in duration-200"
      onClick={onClose}
    >
      <div
        className="w-full max-w-5xl h-[85vh] bg-panel rounded-2xl border border-edge flex flex-col shadow-2xl overflow-hidden"
        onClick={(e) => e.stopPropagation()}
      >
        {/* Modal Header */}
        <div className="flex items-center justify-between px-5 py-3.5 border-b border-edge/80 bg-panel2/60 shrink-0">
          <div className="flex items-center gap-2.5">
            <div className="p-1.5 rounded-lg bg-accent/10 text-accent">
              <GitCompare size={18} />
            </div>
            <div>
              <div className="flex items-center gap-2">
                <span className="font-semibold text-sm text-ink">本轮文件修改审查 (Diff)</span>
                {isReverted && (
                  <span className="text-[11px] px-2 py-0.5 rounded-full bg-amber-500/15 text-amber-400 border border-amber-500/30">
                    当前状态：已撤回
                  </span>
                )}
              </div>
              <p className="text-[12px] text-inkdim mt-0.5">
                共产生 {diffs.length} 个文件变动 · 基于 CAS 零 Git 污染影子快照
              </p>
            </div>
          </div>

          <div className="flex items-center gap-2">
            <button
              type="button"
              className="p-1.5 rounded-lg hover:bg-panel3 text-inkdim hover:text-ink transition-colors ml-1 cursor-pointer"
              onClick={onClose}
              title="关闭"
            >
              <X size={18} />
            </button>
          </div>
        </div>

        {/* Modal Body */}
        <div className="flex-1 flex min-h-0 overflow-hidden">
          {loading ? (
            <div className="flex-1 flex items-center justify-center text-inkdim text-sm">
              正在加载影子快照差异...
            </div>
          ) : error ? (
            <div className="flex-1 flex items-center justify-center text-red-400 text-sm p-4">
              加载快照失败: {error}
            </div>
          ) : diffs.length === 0 ? (
            <div className="flex-1 flex items-center justify-center text-inkdim text-sm">
              本轮未检测到任何代码文件修改快照
            </div>
          ) : (
            <>
              {/* Left Column: File List */}
              <div className="w-64 border-r border-edge/80 bg-panel/40 flex flex-col shrink-0">
                <div className="p-2.5 text-xs text-inkdim font-medium border-b border-edge/60">
                  修改文件列表 ({diffs.length})
                </div>
                <div className="flex-1 overflow-y-auto p-1.5 flex flex-col gap-0.5">
                  {diffs.map((d, idx) => {
                    const isSel = idx === selectedIdx;
                    const fileName = d.filePath.split(/[/\\]/).pop() || d.filePath;
                    return (
                      <div
                        key={d.filePath}
                        onClick={() => setSelectedIdx(idx)}
                        className={`flex items-center justify-between px-2.5 py-1.5 rounded-lg cursor-pointer text-xs font-mono transition-colors ${
                          isSel
                            ? "bg-accent/15 text-accent font-medium border border-accent/25"
                            : "hover:bg-panel2/60 text-ink/90 border border-transparent"
                        }`}
                      >
                        <span className="truncate mr-2" title={d.filePath}>
                          {fileName}
                        </span>
                        <div className="flex items-center gap-1 text-[10px] shrink-0 font-mono">
                          {d.isNewFile ? (
                            <span className="text-emerald-400 bg-emerald-500/10 px-1 rounded">新建</span>
                          ) : (
                            <>
                              <span className="text-emerald-400">+{d.added}</span>
                              <span className="text-rose-400">-{d.removed}</span>
                            </>
                          )}
                        </div>
                      </div>
                    );
                  })}
                </div>
              </div>

              {/* Right Column: Diff Preview */}
              <div className="flex-1 flex flex-col min-w-0 bg-panel2/30">
                {selectedDiff ? (
                  <>
                    <div className="flex items-center justify-between px-4 py-2 border-b border-edge/60 bg-panel3/40 shrink-0">
                      <div className="flex items-center gap-2 font-mono text-xs text-ink min-w-0 truncate">
                        <span>{selectedDiff.filePath}</span>
                        <span className="text-emerald-400 text-[11px]">+{selectedDiff.added}</span>
                        <span className="text-rose-400 text-[11px]">-{selectedDiff.removed}</span>
                      </div>
                      <div className="flex items-center gap-1.5 shrink-0">
                        <button
                          type="button"
                          className="inline-flex items-center gap-1 px-2 py-1 rounded hover:bg-panel3 text-inkdim hover:text-ink text-xs transition-colors cursor-pointer"
                          onClick={handleCopyDiff}
                          title="复制统一 Diff 文本"
                        >
                          {copied ? <Check size={12} className="text-green-400" /> : <Copy size={12} />}
                          <span>{copied ? "已复制" : "复制"}</span>
                        </button>
                        <button
                          type="button"
                          className="inline-flex items-center gap-1 px-2 py-1 rounded hover:bg-panel3 text-inkdim hover:text-ink text-xs transition-colors cursor-pointer"
                          onClick={() => handleOpenEditor(selectedDiff.filePath)}
                          title="在外部代码编辑器中打开"
                        >
                          <ExternalLink size={12} />
                          <span>在编辑器中打开</span>
                        </button>
                      </div>
                    </div>

                    <div className="flex-1 overflow-auto p-4 font-mono text-xs leading-relaxed select-text">
                      <pre className="whitespace-pre-wrap break-all text-ink/90">
                        {selectedDiff.diffText || "（文件内容未发生实质变更）"}
                      </pre>
                    </div>
                  </>
                ) : null}
              </div>
            </>
          )}
        </div>
      </div>
    </div>
  );

  return createPortal(modalContent, document.body);
}
