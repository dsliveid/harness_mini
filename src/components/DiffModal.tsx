import { Fragment, useEffect, useState } from "react";
import { ipc } from "../ipc";
import { currentSession, useStore } from "../store";
import type { DiffHunk, DiffLine, TempChangeFile, TempChangeProject, TempChanges, TempFileDiff } from "../types";
import { RotateCcw, Package, Folder, FolderOpen, MoreHorizontal, Copy } from "./Icons";

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

function getFileDir(relativePath: string): string {
  const norm = relativePath.replace(/\\/g, "/");
  const lastSlash = norm.lastIndexOf("/");
  if (lastSlash === -1) return "";
  return norm.slice(0, lastSlash);
}

function resolveFullPath(baseDir: string, relativePath: string): string {
  const isWin = /^[a-zA-Z]:/.test(baseDir) || baseDir.includes("\\");
  const sep = isWin ? "\\" : "/";
  const cleanBase = baseDir.replace(/[/\\]+$/, "");
  const cleanRel = relativePath.replace(/^[/\\]+/, "");
  if (!cleanRel) return isWin ? cleanBase.replace(/\//g, "\\") : cleanBase.replace(/\\/g, "/");
  const formattedRel = isWin ? cleanRel.replace(/\//g, "\\") : cleanRel.replace(/\\/g, "/");
  return `${cleanBase}${sep}${formattedRel}`;
}

interface MenuState {
  anchor: { x: number; y: number };
  project: TempChangeProject;
  file: TempChangeFile;
}

function FileActionMenu({
  state,
  onClose,
  onOpenDir,
  onCopy,
}: {
  state: MenuState;
  onClose: () => void;
  onOpenDir: (path: string) => void;
  onCopy: (text: string, label: string) => void;
}) {
  const { anchor, project, file } = state;
  const fileDir = getFileDir(file.path);
  const sourceDir = resolveFullPath(project.source, fileDir);
  const tempDir = resolveFullPath(project.temp, fileDir);
  const sourceFile = resolveFullPath(project.source, file.path);
  const tempFile = resolveFullPath(project.temp, file.path);

  // 计算菜单位置，避免溢出屏幕
  const menuWidth = 280;
  let left = anchor.x - menuWidth + 24;
  if (left < 10) left = 10;
  if (left + menuWidth > window.innerWidth - 10) {
    left = window.innerWidth - menuWidth - 10;
  }

  const menuHeight = 230;
  let top = anchor.y + 6;
  if (top + menuHeight > window.innerHeight - 10) {
    top = Math.max(10, anchor.y - menuHeight - 30);
  }

  return (
    <>
      <div
        className="fixed inset-0 z-[100]"
        onMouseDown={(e) => {
          e.stopPropagation();
          onClose();
        }}
        onClick={(e) => {
          e.stopPropagation();
          onClose();
        }}
      />
      <div
        style={{ left: `${left}px`, top: `${top}px` }}
        className="fixed z-[101] w-[280px] bg-panel2/98 backdrop-blur-md border border-edge rounded-xl shadow-2xl py-1.5 text-[12px] animate-in fade-in zoom-in-95 duration-100 select-none overflow-hidden"
        onMouseDown={(e) => e.stopPropagation()}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="px-3 py-1 text-[11px] text-inkdim border-b border-edge/60 truncate font-mono" title={file.path}>
          {file.name}
        </div>

        <div className="py-1">
          <button
            className="w-full px-3 py-1.5 text-left hover:bg-panel3 flex items-start gap-2 text-ink transition-colors group cursor-pointer"
            onClick={(e) => {
              e.stopPropagation();
              onOpenDir(sourceDir);
              onClose();
            }}
            title={`在文件管理器中打开文件所在目录:\n${sourceDir}`}
          >
            <Folder size={14} className="text-blue-400 shrink-0 mt-0.5 group-hover:scale-110 transition-transform" />
            <div className="min-w-0 flex-1">
              <div className="font-medium text-[12px] text-ink">打开文件所在目录</div>
              <div className="text-[10px] text-inkdim font-mono truncate" title={sourceDir}>{sourceDir}</div>
            </div>
          </button>

          <button
            className="w-full px-3 py-1.5 text-left hover:bg-panel3 flex items-start gap-2 text-ink transition-colors group cursor-pointer"
            onClick={(e) => {
              e.stopPropagation();
              onOpenDir(tempDir);
              onClose();
            }}
            title={`在文件管理器中打开临时空间对应目录:\n${tempDir}`}
          >
            <FolderOpen size={14} className="text-amber-400 shrink-0 mt-0.5 group-hover:scale-110 transition-transform" />
            <div className="min-w-0 flex-1">
              <div className="font-medium text-[12px] text-ink">打开临时空间目录</div>
              <div className="text-[10px] text-inkdim font-mono truncate" title={tempDir}>{tempDir}</div>
            </div>
          </button>
        </div>

        <div className="border-t border-edge/60 my-1" />

        <div className="py-0.5">
          <button
            className="w-full px-3 py-1 text-left hover:bg-panel3 flex items-center gap-2 text-inkdim hover:text-ink transition-colors text-[12px] cursor-pointer"
            onClick={(e) => {
              e.stopPropagation();
              onCopy(sourceDir, "文件目录路径");
              onClose();
            }}
          >
            <Copy size={13} className="shrink-0" />
            <span>复制文件目录路径</span>
          </button>

          <button
            className="w-full px-3 py-1 text-left hover:bg-panel3 flex items-center gap-2 text-inkdim hover:text-ink transition-colors text-[12px] cursor-pointer"
            onClick={(e) => {
              e.stopPropagation();
              onCopy(sourceFile, "文件完整路径");
              onClose();
            }}
          >
            <Copy size={13} className="shrink-0" />
            <span>复制文件完整路径</span>
          </button>

          <button
            className="w-full px-3 py-1 text-left hover:bg-panel3 flex items-center gap-2 text-inkdim hover:text-ink transition-colors text-[12px] cursor-pointer"
            onClick={(e) => {
              e.stopPropagation();
              onCopy(file.path, "相对路径");
              onClose();
            }}
          >
            <Copy size={13} className="shrink-0" />
            <span>复制相对路径</span>
          </button>
        </div>
      </div>
    </>
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

  const [menuState, setMenuState] = useState<MenuState | null>(null);

  const sessionId = session?.id;

  const load = async () => {
    if (!sessionId) return;
    setData(null);
    setSelected(null);
    setDiff(null);
    setMenuState(null);
    try {
      setData(await ipc.listTempChanges(sessionId));
    } catch (e) {
      pushToast(String(e));
    }
  };

  const handleOpenDir = (path: string) => {
    if (!path) return;
    ipc.openDir(path).catch((e) => pushToast(String(e)));
  };

  const handleCopy = (text: string, label: string) => {
    navigator.clipboard.writeText(text).then(
      () => pushToast(`已复制${label}`),
      (e) => pushToast(`复制失败: ${e}`)
    );
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
    <div
      className="fixed inset-0 z-[80] bg-black/50 flex items-center justify-center"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) setShow(false);
      }}
    >
      <div
        className="bg-panel2 border border-edge rounded-2xl w-[960px] max-w-[94vw] h-[78vh] flex flex-col shadow-2xl overflow-hidden"
        onMouseDown={(e) => e.stopPropagation()}
      >
        {/* 头部 */}
        <div className="h-12 px-5 border-b border-edge/60 flex items-center gap-3 shrink-0">
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
                <div
                  className="px-3 py-1.5 text-[11px] text-inkdim flex items-center gap-1.5 select-none"
                  title={`项目: ${p.name}\n原项目目录: ${p.source}\n临时空间目录: ${p.temp}`}
                >
                  <Package size={13} className="text-inkdim shrink-0" />
                  <span className="truncate font-medium text-ink/90 flex-1 min-w-0" title={p.name}>
                    {p.name}
                  </span>
                  <span
                    className="ml-auto shrink-0 text-[10px] px-1.5 py-[2px] rounded-full bg-panel3 border border-edge/80 text-inkdim font-medium leading-none"
                    title={`包含 ${p.files.length} 个变更文件`}
                  >
                    {p.files.length} 个文件
                  </span>
                </div>
                {p.files.map((f) => {
                  const active = `${p.key}\n${f.path}` === selected;
                  const b = badge(f.change);
                  const fileDir = getFileDir(f.path);
                  const sourceDir = resolveFullPath(p.source, fileDir);
                  const tempDir = resolveFullPath(p.temp, fileDir);
                  const isMenuOpen = menuState?.project.key === p.key && menuState?.file.path === f.path;

                  return (
                    <div
                      key={f.path}
                      className={`group relative w-full text-left px-3 py-1.5 flex items-start gap-2 cursor-pointer transition-colors select-none ${
                        active ? "bg-panel3" : "hover:bg-panel"
                      }`}
                      onClick={() => void openFile(p.key, f)}
                      title={`变更文件: ${f.name}\n文件目录: ${sourceDir}\n相对路径: ${f.path}`}
                    >
                      <span
                        className={`shrink-0 mt-[1px] w-[18px] h-[18px] rounded flex items-center justify-center text-[11px] leading-none border ${b.badge}`}
                        title={b.label}
                      >
                        {b.letter}
                      </span>
                      <span className="flex-1 min-w-0">
                        <span className="block truncate text-[13px] text-ink">{f.name}</span>
                        <span
                          className="block truncate text-[11px] text-inkdim font-mono"
                          title={`相对位置: ${f.path}\n文件所在目录: ${sourceDir}`}
                        >
                          {f.path}
                        </span>
                      </span>
                      {!f.binary && !f.tooLarge && (f.added > 0 || f.removed > 0) && (
                        <span className="shrink-0 mt-[2px] text-[11px] font-mono group-hover:opacity-40 transition-opacity">
                          {f.added > 0 && <span className="text-green-400">+{f.added}</span>}
                          {f.added > 0 && f.removed > 0 && " "}
                          {f.removed > 0 && <span className="text-red-400">−{f.removed}</span>}
                        </span>
                      )}
                      <button
                        className={`shrink-0 mt-[1px] w-6 h-6 rounded flex items-center justify-center text-inkdim hover:text-ink hover:bg-panel border border-transparent hover:border-edge transition-all cursor-pointer ${
                          isMenuOpen ? "opacity-100 bg-panel border-edge text-ink" : "opacity-0 group-hover:opacity-100"
                        }`}
                        title="更多操作（打开文件目录 / 复制路径）"
                        onClick={(e) => {
                          e.stopPropagation();
                          if (isMenuOpen) {
                            setMenuState(null);
                          } else {
                            const rect = e.currentTarget.getBoundingClientRect();
                            setMenuState({
                              anchor: { x: rect.right, y: rect.bottom },
                              project: p,
                              file: f,
                            });
                          }
                        }}
                      >
                        <MoreHorizontal size={14} />
                      </button>
                    </div>
                  );
                })}
              </div>
            ))}
          </div>

          {/* 右侧：文件内容与变更点 */}
          <div className="flex-1 flex flex-col min-w-0">
            {selFile ? (
              <>
                <div className="h-12 px-4 border-b border-edge flex items-center gap-3 shrink-0 bg-panel2">
                  <span
                    className={`shrink-0 px-1.5 py-[1px] rounded text-[11px] border ${badge(selFile.change).badge}`}
                  >
                    {badge(selFile.change).label}
                  </span>
                  <div className="min-w-0 flex-1 flex items-baseline gap-2">
                    <span className="text-[13px] font-medium truncate text-ink">{selFile.name}</span>
                    <span className="text-[11px] text-inkdim font-mono truncate" title={selFile.path}>
                      {selFile.path}
                    </span>
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

      {menuState && (
        <FileActionMenu
          state={menuState}
          onClose={() => setMenuState(null)}
          onOpenDir={handleOpenDir}
          onCopy={handleCopy}
        />
      )}
    </div>
  );
}
