import { Fragment, useEffect, useState } from "react";
import { ipc } from "../ipc";
import { currentSession, useStore } from "../store";
import type { DiffHunk, DiffLine, TempChangeFile, TempChanges, TempFileDiff } from "../types";
import { RotateCcw, Package, X } from "./Icons";

const CHANGE_BADGE: Record<string, { label: string; letter: string; badge: string; del?: string; add?: string }> = {
  added: { label: "新增", letter: "+", badge: "bg-green-500/15 text-green-400 border-green-500/30" },
  modified: { label: "修改", letter: "M", badge: "bg-amber-500/15 text-amber-400 border-amber-500/30" },
  deleted: { label: "删除", letter: "−", badge: "bg-red-500/15 text-red-400 border-red-500/30" },
};

type ViewMode = "unified" | "split";

/** 并排对比行：同一行内左右单元格可为空 */
interface SplitRow {
  left?: DiffLine;
  right?: DiffLine;
}

/** 把分块行组装为并排行：相邻的删除/新增行逐行配对，余量留空 */
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

function HunkHeader({ hunk }: { hunk: DiffHunk }) {
  return (
    <div className="px-3 py-1 bg-panel3/60 text-[11px] font-mono text-inkdim select-none">
      @@ -{hunk.oldStart},{hunk.oldLines} +{hunk.newStart},{hunk.newLines} @@
    </div>
  );
}

function LineText({ text }: { text: string }) {
  return <span className="flex-1 whitespace-pre-wrap break-all">{text || " "}</span>;
}

function UnifiedView({ diff }: { diff: TempFileDiff }) {
  return (
    <div className="font-mono text-[12px] leading-[1.55]">
      {diff.hunks.map((h, hi) => (
        <div key={hi}>
          <HunkHeader hunk={h} />
          {h.lines.map((l, li) => (
            <div
              key={li}
              className={`flex ${
                l.tag === "del" ? "bg-red-500/10" : l.tag === "add" ? "bg-green-500/10" : ""
              }`}
            >
              <span className="w-11 shrink-0 text-right pr-1.5 text-inkdim/70 select-none">{l.oldNo ?? ""}</span>
              <span className="w-11 shrink-0 text-right pr-1.5 text-inkdim/70 select-none border-r border-edge mr-2">{l.newNo ?? ""}</span>
              <span
                className={`shrink-0 w-4 select-none ${
                  l.tag === "del" ? "text-red-400" : l.tag === "add" ? "text-green-400" : "text-transparent"
                }`}
              >
                {l.tag === "del" ? "−" : l.tag === "add" ? "+" : " "}
              </span>
              <LineText text={l.text} />
            </div>
          ))}
        </div>
      ))}
    </div>
  );
}

function SplitView({ diff }: { diff: TempFileDiff }) {
  return (
    <div className="font-mono text-[12px] leading-[1.55]">
      {diff.hunks.map((h, hi) => {
        const rows = hunkRows(h.lines);
        return (
          <div key={hi}>
            <HunkHeader hunk={h} />
            <div className="grid grid-cols-2">
              {rows.map((r, ri) => (
                <Fragment key={ri}>
                  <div
                    className={`flex border-r border-edge ${
                      r.left?.tag === "del" ? "bg-red-500/10" : ""
                    }`}
                  >
                    <span className="w-11 shrink-0 text-right pr-1.5 text-inkdim/70 select-none">{r.left?.oldNo ?? ""}</span>
                    {r.left ? <LineText text={r.left.text} /> : <span className="flex-1 bg-black/20">&nbsp;</span>}
                  </div>
                  <div className={`flex ${r.right?.tag === "add" ? "bg-green-500/10" : ""}`}>
                    <span className="w-11 shrink-0 text-right pr-1.5 text-inkdim/70 select-none">{r.right?.newNo ?? ""}</span>
                    {r.right ? <LineText text={r.right.text} /> : <span className="flex-1 bg-black/20">&nbsp;</span>}
                  </div>
                </Fragment>
              ))}
            </div>
          </div>
        );
      })}
    </div>
  );
}

/**
 * 临时空间变更列表弹窗：左侧为按项目分组的变更文件列表（文件名 + 路径 + 增删统计），
 * 右侧为文件内容与变更点展示，支持「统一视图 / 并排对比（变更前 | 变更后）」切换。
 */
export function DiffModal() {
  const show = useStore((s) => s.showChanges);
  const setShow = useStore((s) => s.setShowChanges);
  const session = useStore((s) => currentSession(s));
  const pushToast = useStore((s) => s.pushToast);
  const [data, setData] = useState<TempChanges | null>(null);
  const [selected, setSelected] = useState<string | null>(null); // `${key}\n${path}`
  const [diff, setDiff] = useState<TempFileDiff | null>(null);
  const [diffLoading, setDiffLoading] = useState(false);
  const [mode, setMode] = useState<ViewMode>("split");

  const sessionId = session?.id;

  const load = async () => {
    if (!sessionId) return;
    setData(null);
    setSelected(null);
    setDiff(null);
    try {
      setData(await ipc.listTempChanges(sessionId));
    } catch (e) {
      pushToast(String(e));
    }
  };

  useEffect(() => {
    if (show) void load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [show, sessionId]);

  const openFile = async (key: string, file: TempChangeFile) => {
    if (!sessionId) return;
    setSelected(`${key}\n${file.path}`);
    setDiff(null);
    setDiffLoading(true);
    try {
      setDiff(await ipc.getTempChangeDiff(sessionId, key, file.path));
    } catch (e) {
      pushToast(String(e));
    } finally {
      setDiffLoading(false);
    }
  };

  // 列表加载完成后默认展开第一个文件
  useEffect(() => {
    const first = data?.projects[0]?.files[0];
    if (data && data.projects.length > 0 && first) void openFile(data.projects[0].key, first);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [data]);

  if (!show || !session?.isTemp) return null;

  const badge = (change: string) => CHANGE_BADGE[change] ?? CHANGE_BADGE.modified;
  const selFile: TempChangeFile | null = (() => {
    for (const p of data?.projects ?? []) {
      for (const f of p.files) {
        if (`${p.key}\n${f.path}` === selected) return f;
      }
    }
    return null;
  })();

  return (
    <div className="fixed inset-0 z-[80] bg-black/50 flex items-center justify-center" onMouseDown={() => setShow(false)}>
      <div
        className="bg-panel2 border border-edge rounded-2xl w-[960px] max-w-[94vw] h-[78vh] flex flex-col shadow-2xl overflow-hidden"
        onMouseDown={(e) => e.stopPropagation()}
      >
        {/* 头部 */}
        <div className="px-5 py-3.5 border-b border-edge flex items-center gap-3 shrink-0">
          <div className="font-medium">变更列表</div>
          {data && (
            <span className="text-[12px] text-inkdim">
              {data.totalFiles} 个文件 · <span className="text-green-400">+{data.projects.reduce((n, p) => n + p.files.reduce((m, f) => m + f.added, 0), 0)}</span>{" "}
              <span className="text-red-400">−{data.projects.reduce((n, p) => n + p.files.reduce((m, f) => m + f.removed, 0), 0)}</span>
            </span>
          )}
          <button
            className="ml-auto text-[12px] px-2.5 py-1 rounded-lg text-inkdim hover:text-ink hover:bg-panel3 flex items-center gap-1.5 transition-colors"
            title="重新扫描变更"
            onClick={() => void load()}
          >
            <RotateCcw size={12} />
            <span>刷新</span>
          </button>
          <button
            className="text-[12px] px-2.5 py-1 rounded-lg text-inkdim hover:text-ink hover:bg-panel3 transition-colors"
            onClick={() => setShow(false)}
          >
            关闭
          </button>
        </div>

        <div className="flex-1 flex min-h-0">
          {/* 左侧：变更文件列表 */}
          <div className="w-[300px] shrink-0 border-r border-edge overflow-y-auto py-2">
            {!data && <div className="px-4 py-3 text-[13px] text-inkdim">扫描变更中…</div>}
            {data && data.totalFiles === 0 && (
              <div className="px-4 py-3 text-[13px] text-inkdim">暂无变更内容（相对基线提交）。</div>
            )}
            {data?.projects.map((p) => (
              <div key={p.key} className="mb-2">
                <div className="px-3 py-1.5 text-[11px] text-inkdim flex items-center gap-1.5" title={p.source}>
                  <Package size={13} className="text-inkdim shrink-0" />
                  <span className="truncate font-medium">{p.name}</span>
                  <span className="shrink-0">({p.files.length})</span>
                </div>
                {p.files.map((f) => {
                  const active = `${p.key}\n${f.path}` === selected;
                  const b = badge(f.change);
                  return (
                    <button
                      key={f.path}
                      className={`w-full text-left px-3 py-1.5 flex items-start gap-2 ${
                        active ? "bg-panel3" : "hover:bg-panel"
                      }`}
                      onClick={() => void openFile(p.key, f)}
                    >
                      <span
                        className={`shrink-0 mt-[1px] w-[18px] h-[18px] rounded flex items-center justify-center text-[11px] leading-none border ${b.badge}`}
                        title={b.label}
                      >
                        {b.letter}
                      </span>
                      <span className="flex-1 min-w-0">
                        <span className="block truncate text-[13px]">{f.name}</span>
                        <span className="block truncate text-[11px] text-inkdim" title={f.path}>
                          {f.path}
                        </span>
                      </span>
                      {!f.binary && !f.tooLarge && (f.added > 0 || f.removed > 0) && (
                        <span className="shrink-0 mt-[2px] text-[11px] font-mono">
                          {f.added > 0 && <span className="text-green-400">+{f.added}</span>}
                          {f.added > 0 && f.removed > 0 && " "}
                          {f.removed > 0 && <span className="text-red-400">−{f.removed}</span>}
                        </span>
                      )}
                    </button>
                  );
                })}
              </div>
            ))}
          </div>

          {/* 右侧：文件内容与变更点 */}
          <div className="flex-1 flex flex-col min-w-0">
            {selFile ? (
              <>
                <div className="px-4 py-2.5 border-b border-edge flex items-center gap-2 shrink-0">
                  <span
                    className={`shrink-0 px-1.5 py-[1px] rounded text-[11px] border ${badge(selFile.change).badge}`}
                  >
                    {badge(selFile.change).label}
                  </span>
                  <div className="min-w-0 flex-1">
                    <div className="text-[13px] truncate">{selFile.name}</div>
                    <div className="text-[11px] text-inkdim truncate" title={selFile.path}>
                      {selFile.path}
                    </div>
                  </div>
                  {!selFile.binary && !selFile.tooLarge && (
                    <span className="shrink-0 text-[12px] font-mono">
                      <span className="text-green-400">+{selFile.added}</span>{" "}
                      <span className="text-red-400">−{selFile.removed}</span>
                    </span>
                  )}
                  {/* 视图切换：统一 / 并排对比（变更前 | 变更后） */}
                  <div className="shrink-0 flex bg-panel rounded-lg p-0.5 border border-edge">
                    {(["unified", "split"] as const).map((m) => (
                      <button
                        key={m}
                        className={`px-2.5 py-1 rounded-md text-[12px] ${
                          mode === m ? "bg-panel3 text-ink" : "text-inkdim hover:text-ink"
                        }`}
                        onClick={() => setMode(m)}
                      >
                        {m === "unified" ? "统一视图" : "并排对比"}
                      </button>
                    ))}
                  </div>
                </div>
                <div className="flex-1 overflow-auto bg-panel">
                  {diffLoading && <div className="px-4 py-3 text-[13px] text-inkdim">加载 diff…</div>}
                  {diff?.binary && (
                    <div className="px-4 py-3 text-[13px] text-inkdim">二进制文件，无法展示内容差异。</div>
                  )}
                  {diff?.tooLarge && (
                    <div className="px-4 py-3 text-[13px] text-inkdim">文件超过 1MB，请在外部编辑器中查看。</div>
                  )}
                  {diff && !diff.binary && !diff.tooLarge && (
                    <>
                      {diff.truncated && (
                        <div className="px-4 py-2 text-[12px] text-amber-400 bg-amber-500/10">
                          变更行数过多，仅展示前 4000 行。
                        </div>
                      )}
                      {mode === "unified" ? <UnifiedView diff={diff} /> : <SplitView diff={diff} />}
                    </>
                  )}
                </div>
              </>
            ) : (
              <div className="flex-1 flex items-center justify-center text-inkdim text-[13px]">
                {data && data.totalFiles > 0 ? "在左侧选择文件查看变更" : "无变更内容"}
              </div>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
