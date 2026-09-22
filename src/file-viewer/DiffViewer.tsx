import React, { Fragment, useEffect, useState, useRef } from "react";
import { ipc } from "../ipc";
import type { DiffViewerTab, TempFileDiff, DiffHunk, DiffLine } from "./types";
import {
  RotateCcw,
  FolderOpen,
  AlertCircle,
  Copy,
  Check,
  ChevronDown,
  ChevronUp,
  ExternalLink,
  Undo2,
} from "../components/Icons";

interface SplitRow {
  left?: DiffLine;
  right?: DiffLine;
}

/** 计算相邻删除行与新增行的行内字符差异 */
function renderWordDiff(delText: string, addText: string): { delParts: React.ReactNode; addParts: React.ReactNode } {
  let p = 0;
  while (p < delText.length && p < addText.length && delText[p] === addText[p]) {
    p++;
  }
  let s = 0;
  while (
    s < delText.length - p &&
    s < addText.length - p &&
    delText[delText.length - 1 - s] === addText[addText.length - 1 - s]
  ) {
    s++;
  }

  const delPre = delText.slice(0, p);
  const delMid = delText.slice(p, delText.length - s);
  const delSuf = delText.slice(delText.length - s);

  const addPre = addText.slice(0, p);
  const addMid = addText.slice(p, addText.length - s);
  const addSuf = addText.slice(addText.length - s);

  return {
    delParts: (
      <>
        {delPre}
        {delMid && <span className="bg-red-500/40 text-red-100 font-semibold rounded px-0.5">{delMid}</span>}
        {delSuf}
      </>
    ),
    addParts: (
      <>
        {addPre}
        {addMid && <span className="bg-green-500/40 text-green-100 font-semibold rounded px-0.5">{addMid}</span>}
        {addSuf}
      </>
    ),
  };
}

function hunkRows(lines: DiffLine[]): SplitRow[] {
  const rows: SplitRow[] = [];
  let i = 0;
  while (i < lines.length) {
    const l = lines[i];
    if (l.tag === "same") {
      rows.push({ left: l, right: l });
      i += 1;
      continue;
    }
    const dels: DiffLine[] = [];
    const adds: DiffLine[] = [];
    while (i < lines.length && lines[i].tag === "del") {
      dels.push(lines[i]);
      i += 1;
    }
    while (i < lines.length && lines[i].tag === "add") {
      adds.push(lines[i]);
      i += 1;
    }
    const n = Math.max(dels.length, adds.length);
    for (let k = 0; k < n; k += 1) {
      rows.push({ left: dels[k], right: adds[k] });
    }
  }
  return rows;
}

function HunkHeader({
  hunk,
  index,
  isActive,
  canRevert,
  onRevertHunk,
}: {
  hunk: DiffHunk;
  index: number;
  isActive: boolean;
  canRevert?: boolean;
  onRevertHunk?: (hunk: DiffHunk) => void;
}) {
  const [confirming, setConfirming] = useState(false);

  return (
    <div
      id={`diff-hunk-${index}`}
      className={`px-3 py-1 text-[11px] font-mono select-none border-y transition-colors flex items-center justify-between ${
        isActive
          ? "bg-accent/20 text-accent border-accent/40 font-semibold"
          : "bg-panel3/70 text-inkdim border-edge/40"
      }`}
    >
      <span>
        @@ -{hunk.oldStart},{hunk.oldLines} +{hunk.newStart},{hunk.newLines} @@
      </span>

      <div className="flex items-center gap-2">
        <span className="text-[10px] text-inkdim font-mono">差异块 #{index + 1}</span>

        {canRevert && onRevertHunk && (
          confirming ? (
            <div className="flex items-center gap-1 bg-panel px-1.5 py-0.5 rounded border border-edge">
              <span className="text-[10px] text-amber-300">确定撤销此块改动?</span>
              <button
                type="button"
                className="px-1.5 py-0.5 rounded bg-red-500/20 text-red-300 hover:bg-red-500/30 border border-red-500/30 text-[10px] cursor-pointer"
                onClick={() => {
                  setConfirming(false);
                  onRevertHunk(hunk);
                }}
              >
                确定
              </button>
              <button
                type="button"
                className="px-1.5 py-0.5 rounded hover:bg-panel3 text-inkdim text-[10px] cursor-pointer"
                onClick={() => setConfirming(false)}
              >
                取消
              </button>
            </div>
          ) : (
            <button
              type="button"
              className="flex items-center gap-1 px-1.5 py-0.5 rounded hover:bg-panel hover:text-ink text-inkdim text-[10.5px] transition-colors cursor-pointer border border-transparent hover:border-edge"
              title="撤销此块修改并写回磁盘文件"
              onClick={() => setConfirming(true)}
            >
              <Undo2 size={11} className="text-amber-400" />
              <span>撤销此块</span>
            </button>
          )
        )}
      </div>
    </div>
  );
}

function LineText({ content }: { content: React.ReactNode }) {
  return <span className="flex-1 whitespace-pre-wrap break-all px-1.5">{content || " "}</span>;
}

function UnifiedView({
  diff,
  activeHunkIndex,
  canRevert,
  onRevertHunk,
}: {
  diff: TempFileDiff;
  activeHunkIndex: number;
  canRevert?: boolean;
  onRevertHunk?: (hunk: DiffHunk) => void;
}) {
  return (
    <div className="font-mono text-[12px] leading-[1.6]">
      {diff.hunks.map((h, hi) => {
        const isActiveHunk = hi === activeHunkIndex;
        return (
          <div key={hi} className={isActiveHunk ? "ring-1 ring-accent/40" : ""}>
            <HunkHeader
              hunk={h}
              index={hi}
              isActive={isActiveHunk}
              canRevert={canRevert}
              onRevertHunk={onRevertHunk}
            />
            {h.lines.map((l, li) => (
              <div
                key={li}
                className={`flex items-baseline ${
                  l.tag === "del" ? "bg-red-500/10" : l.tag === "add" ? "bg-green-500/10" : ""
                }`}
              >
                <span className="w-11 shrink-0 text-right pr-2 text-inkdim/60 select-none">{l.oldNo ?? ""}</span>
                <span className="w-11 shrink-0 text-right pr-2 text-inkdim/60 select-none border-r border-edge/60 mr-2">
                  {l.newNo ?? ""}
                </span>
                <span
                  className={`shrink-0 w-4 text-center select-none ${
                    l.tag === "del" ? "text-red-400 font-bold" : l.tag === "add" ? "text-green-400 font-bold" : "text-transparent"
                  }`}
                >
                  {l.tag === "del" ? "−" : l.tag === "add" ? "+" : " "}
                </span>
                <LineText content={l.text} />
              </div>
            ))}
          </div>
        );
      })}
    </div>
  );
}

function SplitView({
  diff,
  activeHunkIndex,
  canRevert,
  onRevertHunk,
}: {
  diff: TempFileDiff;
  activeHunkIndex: number;
  canRevert?: boolean;
  onRevertHunk?: (hunk: DiffHunk) => void;
}) {
  return (
    <div className="font-mono text-[12px] leading-[1.6]">
      {diff.hunks.map((h, hi) => {
        const rows = hunkRows(h.lines);
        const isActiveHunk = hi === activeHunkIndex;
        return (
          <div key={hi} className={isActiveHunk ? "ring-1 ring-accent/40" : ""}>
            <HunkHeader
              hunk={h}
              index={hi}
              isActive={isActiveHunk}
              canRevert={canRevert}
              onRevertHunk={onRevertHunk}
            />
            <div className="grid grid-cols-2 divide-x divide-edge/60">
              {rows.map((r, ri) => {
                // 若为修改替换行，进行行内字符级细化高亮
                let leftContent: React.ReactNode = r.left?.text;
                let rightContent: React.ReactNode = r.right?.text;
                if (r.left?.tag === "del" && r.right?.tag === "add") {
                  const wd = renderWordDiff(r.left.text, r.right.text);
                  leftContent = wd.delParts;
                  rightContent = wd.addParts;
                }

                return (
                  <Fragment key={ri}>
                    <div
                      className={`flex items-baseline ${
                        r.left?.tag === "del" ? "bg-red-500/10" : ""
                      }`}
                    >
                      <span className="w-11 shrink-0 text-right pr-2 text-inkdim/60 select-none border-r border-edge/40 mr-1">
                        {r.left?.oldNo ?? ""}
                      </span>
                      {r.left ? <LineText content={leftContent} /> : <span className="flex-1 bg-black/10">&nbsp;</span>}
                    </div>
                    <div className={`flex items-baseline ${r.right?.tag === "add" ? "bg-green-500/10" : ""}`}>
                      <span className="w-11 shrink-0 text-right pr-2 text-inkdim/60 select-none border-r border-edge/40 mr-1">
                        {r.right?.newNo ?? ""}
                      </span>
                      {r.right ? <LineText content={rightContent} /> : <span className="flex-1 bg-black/10">&nbsp;</span>}
                    </div>
                  </Fragment>
                );
              })}
            </div>
          </div>
        );
      })}
    </div>
  );
}

export function DiffViewer({ tab }: { tab: DiffViewerTab }) {
  const [diff, setDiff] = useState<TempFileDiff | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [mode, setMode] = useState<"unified" | "split">(tab.defaultMode || "split");
  const [copied, setCopied] = useState(false);
  const [activeHunkIndex, setActiveHunkIndex] = useState<number>(0);
  const [revertMsg, setRevertMsg] = useState<string | null>(null);

  const loadDiff = async () => {
    setLoading(true);
    setError(null);
    try {
      if (tab.diffSource === "temp" && tab.sessionId && tab.projectKey) {
        const res = await ipc.getTempChangeDiff(tab.sessionId, tab.projectKey, tab.path);
        setDiff(res);
      } else {
        const res = await ipc.getFileDiff(tab.path, tab.oldContent, tab.newContent, tab.workspacePath);
        setDiff(res);
      }
      setActiveHunkIndex(0);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    void loadDiff();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tab.path, tab.sessionId, tab.projectKey, tab.oldContent, tab.newContent]);

  // 差异块跳转函数
  const jumpToHunk = (index: number) => {
    if (!diff || diff.hunks.length === 0) return;
    const clamped = Math.max(0, Math.min(index, diff.hunks.length - 1));
    setActiveHunkIndex(clamped);
    const elem = document.getElementById(`diff-hunk-${clamped}`);
    if (elem) {
      elem.scrollIntoView({ behavior: "smooth", block: "center" });
    }
  };

  const nextHunk = () => {
    if (!diff || diff.hunks.length === 0) return;
    jumpToHunk((activeHunkIndex + 1) % diff.hunks.length);
  };

  const prevHunk = () => {
    if (!diff || diff.hunks.length === 0) return;
    jumpToHunk((activeHunkIndex - 1 + diff.hunks.length) % diff.hunks.length);
  };

  // 监听键盘快捷键：F7 / Shift+F7 差异块跳转，Ctrl+R / F5 刷新
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "F7") {
        e.preventDefault();
        if (e.shiftKey) {
          prevHunk();
        } else {
          nextHunk();
        }
      } else if ((e.ctrlKey || e.metaKey) && (e.key === "r" || e.key === "R")) {
        e.preventDefault();
        void loadDiff();
      } else if (e.key === "F5") {
        e.preventDefault();
        void loadDiff();
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  });

  const handleCopyPath = () => {
    navigator.clipboard.writeText(tab.path).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    });
  };

  const handleOpenDir = () => {
    ipc.openDir(tab.path);
  };

  const handleOpenExternalEditor = () => {
    const hunk = diff?.hunks[activeHunkIndex];
    const line = hunk ? hunk.newStart || hunk.oldStart : 1;
    ipc.openInExternalEditor(tab.path, line);
  };

  const handleRevertHunk = async (hunk: DiffHunk) => {
    try {
      await ipc.revertFileHunk(tab.path, hunk);
      setRevertMsg("已成功撤销该差异块改动并写回文件");
      setTimeout(() => setRevertMsg(null), 3000);
      await loadDiff();
    } catch (e) {
      alert(`撤销差异块失败: ${String(e)}`);
    }
  };

  const totalHunks = diff?.hunks.length || 0;
  // 当有实际物理文件路径且非临时纯内存对比时支持差异块撤销写回
  const canRevert = Boolean(tab.path && tab.diffSource !== "custom");

  return (
    <div className="flex-1 flex flex-col min-h-0 bg-panel text-ink overflow-hidden">
      {/* 头部控制栏 */}
      <div className="h-10 px-4 border-b border-edge/60 bg-panel2/60 flex items-center justify-between shrink-0 text-[12px]">
        <div className="flex items-center gap-2.5 min-w-0">
          <span className="font-semibold text-ink truncate">{tab.title}</span>
          <span className="text-inkdim font-mono text-[11px] truncate max-w-[320px]" title={tab.path}>
            {tab.path}
          </span>
          {diff && (
            <span className="flex items-center gap-1.5 text-[11px] font-mono shrink-0 ml-1">
              <span className="text-green-400 font-medium">+{diff.added}</span>
              <span className="text-red-400 font-medium">−{diff.removed}</span>
            </span>
          )}
        </div>

        <div className="flex items-center gap-2 shrink-0">
          {/* 差异块快速跳转 (Hunk Jumper) */}
          {totalHunks > 0 && (
            <div className="flex items-center bg-panel3/80 rounded-lg border border-edge px-1.5 py-0.5 text-[11px] font-mono text-inkdim gap-1">
              <span>
                块 {activeHunkIndex + 1}/{totalHunks}
              </span>
              <button
                type="button"
                className="p-0.5 rounded hover:bg-panel hover:text-ink transition-colors cursor-pointer"
                title="上一处变更 (Shift+F7)"
                onClick={prevHunk}
              >
                <ChevronUp size={13} />
              </button>
              <button
                type="button"
                className="p-0.5 rounded hover:bg-panel hover:text-ink transition-colors cursor-pointer"
                title="下一处变更 (F7)"
                onClick={nextHunk}
              >
                <ChevronDown size={13} />
              </button>
            </div>
          )}

          {/* 统一视图 / 并排对比 切换按钮 */}
          <div className="flex bg-panel3/80 rounded-lg p-0.5 border border-edge">
            {(["split", "unified"] as const).map((m) => (
              <button
                key={m}
                type="button"
                className={`px-2.5 py-0.5 rounded-md text-[11.5px] transition-colors cursor-pointer ${
                  mode === m ? "bg-panel text-ink shadow-2xs font-medium" : "text-inkdim hover:text-ink"
                }`}
                onClick={() => setMode(m)}
              >
                {m === "split" ? "并排对比" : "统一视图"}
              </button>
            ))}
          </div>

          <button
            type="button"
            className="p-1.5 rounded-lg text-inkdim hover:text-ink hover:bg-panel3 transition-colors cursor-pointer"
            title="刷新 Diff (Ctrl+R / F5)"
            onClick={loadDiff}
          >
            <RotateCcw size={14} />
          </button>
          <button
            type="button"
            className="flex items-center gap-1 px-2 py-1 rounded-lg text-inkdim hover:text-ink hover:bg-panel3 transition-colors text-[11px] cursor-pointer"
            title="复制文件路径"
            onClick={handleCopyPath}
          >
            {copied ? <Check size={13} className="text-green-400" /> : <Copy size={13} />}
            <span>{copied ? "已复制" : "复制路径"}</span>
          </button>
          <button
            type="button"
            className="flex items-center gap-1 px-2 py-1 rounded-lg text-inkdim hover:text-ink hover:bg-panel3 transition-colors text-[11px] cursor-pointer"
            title="在外部编辑器打开并定位 (VS Code / Cursor)"
            onClick={handleOpenExternalEditor}
          >
            <ExternalLink size={13} className="text-blue-400" />
            <span>编辑器打开</span>
          </button>
          <button
            type="button"
            className="p-1.5 rounded-lg text-inkdim hover:text-ink hover:bg-panel3 transition-colors cursor-pointer"
            title="在文件资源管理器中打开"
            onClick={handleOpenDir}
          >
            <FolderOpen size={14} />
          </button>
        </div>
      </div>

      {/* 撤销成功提示 */}
      {revertMsg && (
        <div className="px-4 py-1.5 bg-emerald-500/10 border-b border-emerald-500/30 text-emerald-300 text-[11.5px] flex items-center justify-between shrink-0 animate-in slide-in-from-top-1 duration-150">
          <div className="flex items-center gap-2">
            <Check size={14} className="text-emerald-400" />
            <span>{revertMsg}</span>
          </div>
          <button
            type="button"
            className="text-inkdim hover:text-ink cursor-pointer"
            onClick={() => setRevertMsg(null)}
          >
            ×
          </button>
        </div>
      )}

      {/* 截断提醒 */}
      {diff?.truncated && (
        <div className="px-4 py-1.5 bg-amber-500/10 border-b border-amber-500/20 text-amber-300 text-[11.5px] flex items-center gap-2 shrink-0">
          <AlertCircle size={14} className="shrink-0" />
          <span>变更行数过多，已截断展示前 4000 行。</span>
        </div>
      )}

      {/* 内容区域 */}
      <div className="flex-1 overflow-auto bg-panel select-text">
        {loading && (
          <div className="flex items-center justify-center h-48 text-inkdim text-[13px]">
            计算差异并加载中…
          </div>
        )}

        {error && (
          <div className="p-6 flex flex-col items-center justify-center text-red-400 gap-2">
            <AlertCircle size={24} />
            <div className="text-[13px] font-medium">{error}</div>
            <button
              className="mt-2 px-3 py-1 bg-panel3 rounded-lg text-ink text-[12px] hover:bg-edge"
              onClick={loadDiff}
            >
              重新加载
            </button>
          </div>
        )}

        {diff?.binary && (
          <div className="flex items-center justify-center h-48 text-inkdim text-[13px]">
            二进制文件，无法展示内容差异。
          </div>
        )}

        {diff && !diff.binary && diff.hunks.length === 0 && (
          <div className="flex items-center justify-center h-48 text-inkdim text-[13px]">
            文件未检测到变更内容（与基准一致）。
          </div>
        )}

        {diff && !diff.binary && diff.hunks.length > 0 && (
          <div className="py-1">
            {mode === "unified" ? (
              <UnifiedView
                diff={diff}
                activeHunkIndex={activeHunkIndex}
                canRevert={canRevert}
                onRevertHunk={handleRevertHunk}
              />
            ) : (
              <SplitView
                diff={diff}
                activeHunkIndex={activeHunkIndex}
                canRevert={canRevert}
                onRevertHunk={handleRevertHunk}
              />
            )}
          </div>
        )}
      </div>
    </div>
  );
}
