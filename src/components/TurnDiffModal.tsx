import { useState, useEffect } from "react";
import { createPortal } from "react-dom";
import { ipc } from "../ipc";
import { useStore } from "../store";
import type { SnapshotFileDiff } from "../types";
import { X, Copy, Check, ExternalLink, GitCompare, RotateCcw, RotateCw } from "./Icons";

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

  const safeIdx = Math.min(selectedIdx, Math.max(0, diffs.length - 1));
  const selectedDiff = diffs[safeIdx] || null;

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

  const [revertingPath, setRevertingPath] = useState<string | null>(null);

  const reloadDiffs = async () => {
    try {
      const res = await ipc.getTurnDiff(messageId);
      setDiffs(res);
    } catch (err) {
      console.error("重新加载 Diff 失败:", err);
    }
  };

  const handleOpenDiffViewer = (diff: SnapshotFileDiff) => {
    if (!diff?.filePath) return;
    const cleanRel = diff.filePath.replace(/\\/g, "/");
    const fileName = cleanRel.split("/").pop() || "文件";
    const absPath =
      /^[a-zA-Z]:\//.test(cleanRel) || cleanRel.startsWith("/")
        ? cleanRel
        : currentWorkspace
        ? `${currentWorkspace.replace(/\\/g, "/")}/${cleanRel.replace(/^\.\//, "")}`
        : cleanRel;

    ipc.openFileViewer({
      id: `diff:${absPath}`,
      type: "diff",
      title: `Diff: ${fileName}`,
      subtitle: cleanRel,
      path: absPath,
      diffSource: "git",
      oldContent: diff.beforeContent || "",
      newContent: diff.afterContent,
      workspacePath: currentWorkspace,
    });
  };

  const handleRevertFile = async (diff: SnapshotFileDiff) => {
    if (!diff.filePath || revertingPath) return;
    setRevertingPath(diff.filePath);
    try {
      const res = await ipc.revertTurnFile(messageId, diff.filePath, false);
      if (!res.success && res.hasConflict) {
        const proceed = window.confirm(`${res.message}\n\n是否强制覆盖撤回该文件的全部修改？`);
        if (proceed) {
          const forceRes = await ipc.revertTurnFile(messageId, diff.filePath, true);
          if (forceRes.success) {
            pushToast("该文件修改已强制撤回");
          } else {
            pushToast(forceRes.message || "撤回失败", "error");
          }
        }
      } else if (res.success) {
        pushToast("该文件修改已成功撤回");
      } else {
        pushToast(res.message || "撤回失败", "error");
      }
      const curId = useStore.getState().currentId;
      if (curId) await useStore.getState().reloadMessages(curId);
      await reloadDiffs();
    } catch (err: any) {
      pushToast(String(err) || "撤回文件修改失败", "error");
    } finally {
      setRevertingPath(null);
    }
  };

  const handleReapplyFile = async (diff: SnapshotFileDiff) => {
    if (!diff.filePath || revertingPath) return;
    setRevertingPath(diff.filePath);
    try {
      const res = await ipc.reapplyTurnFile(messageId, diff.filePath, false);
      if (!res.success && res.hasConflict) {
        const proceed = window.confirm(`${res.message}\n\n是否强制覆盖重新应用该文件的修改？`);
        if (proceed) {
          const forceRes = await ipc.reapplyTurnFile(messageId, diff.filePath, true);
          if (forceRes.success) {
            pushToast("该文件修改已强制重新应用");
          } else {
            pushToast(forceRes.message || "重新应用失败", "error");
          }
        }
      } else if (res.success) {
        pushToast("已成功重新应用该文件修改");
      } else {
        pushToast(res.message || "重新应用失败", "error");
      }
      const curId = useStore.getState().currentId;
      if (curId) await useStore.getState().reloadMessages(curId);
      await reloadDiffs();
    } catch (err: any) {
      pushToast(String(err) || "重新应用文件修改失败", "error");
    } finally {
      setRevertingPath(null);
    }
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
                    const isSel = idx === safeIdx;
                    const fileName = d.filePath.split(/[/\\]/).pop() || d.filePath;
                    return (
                      <div
                        key={d.filePath}
                        onClick={() => setSelectedIdx(idx)}
                        className={`flex items-center justify-between px-2.5 py-1.5 rounded-lg cursor-pointer text-xs font-mono transition-colors ${
                          d.revertedAt ? "opacity-60 " : ""
                        }${
                          isSel
                            ? "bg-accent/15 text-accent font-medium border border-accent/25"
                            : "hover:bg-panel2/60 text-ink/90 border border-transparent"
                        }`}
                      >
                        <div className="flex items-center gap-1.5 truncate mr-2 min-w-0" title={d.filePath}>
                          <span className="truncate">{fileName}</span>
                          {d.modifyCount && d.modifyCount > 1 ? (
                            <span
                              className="text-[10px] text-inkdim bg-panel3 px-1 py-0.2 rounded border border-edge/60 shrink-0 font-sans"
                              title={`本轮对话对该文件进行了 ${d.modifyCount} 次修改`}
                            >
                              {d.modifyCount}次修改
                            </span>
                          ) : null}
                        </div>
                        <div className="flex items-center gap-1 text-[10px] shrink-0 font-mono">
                          {d.revertedAt ? (
                            <span className="text-amber-400 bg-amber-500/15 border border-amber-500/30 px-1 rounded">已撤回</span>
                          ) : d.isNewFile ? (
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
                        {selectedDiff.revertedAt && (
                          <span className="text-[11px] px-1.5 py-0.5 rounded-full bg-amber-500/15 text-amber-400 border border-amber-500/30 font-sans">
                            已撤回
                          </span>
                        )}
                      </div>
                      <div className="flex items-center gap-1.5 shrink-0">
                        <button
                          type="button"
                          className="inline-flex items-center gap-1 px-2 py-1 rounded bg-amber-500/10 hover:bg-amber-500/20 text-amber-300 text-xs transition-colors cursor-pointer"
                          onClick={() => handleOpenDiffViewer(selectedDiff)}
                          title="在独立窗体中查看文件变更对比 (Diff)"
                        >
                          <GitCompare size={12} />
                          <span>对比 Diff</span>
                        </button>
                        {!selectedDiff.revertedAt ? (
                          <button
                            type="button"
                            className="inline-flex items-center gap-1 px-2 py-1 rounded bg-rose-500/10 hover:bg-rose-500/20 text-rose-300 text-xs transition-colors cursor-pointer disabled:opacity-50"
                            title="撤回本轮对话对该文件的全部修改（还原至初始状态）"
                            disabled={revertingPath === selectedDiff.filePath}
                            onClick={() => handleRevertFile(selectedDiff)}
                          >
                            <RotateCcw size={12} className={revertingPath === selectedDiff.filePath ? "animate-spin" : ""} />
                            <span>{revertingPath === selectedDiff.filePath ? "撤回中…" : "撤回该文件"}</span>
                          </button>
                        ) : (
                          <button
                            type="button"
                            className="inline-flex items-center gap-1 px-2 py-1 rounded bg-emerald-500/10 hover:bg-emerald-500/20 text-emerald-300 text-xs transition-colors cursor-pointer disabled:opacity-50"
                            title="重新应用本轮对话对该文件的修改"
                            disabled={revertingPath === selectedDiff.filePath}
                            onClick={() => handleReapplyFile(selectedDiff)}
                          >
                            <RotateCw size={12} className={revertingPath === selectedDiff.filePath ? "animate-spin" : ""} />
                            <span>{revertingPath === selectedDiff.filePath ? "应用中…" : "重新应用"}</span>
                          </button>
                        )}
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

                    {selectedDiff.revertedAt && (
                      <div className="bg-amber-500/10 border-b border-amber-500/20 px-4 py-1.5 flex items-center gap-2 text-[11.5px] text-amber-300 select-none shrink-0">
                        <RotateCcw size={12} className="shrink-0 text-amber-400" />
                        <span>该文件在本次会话中的修改已被撤回（代码已还原）。点击“重新应用”可再次生效。</span>
                      </div>
                    )}

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
