import React, { useEffect, useState, useRef, useMemo } from "react";
import { ipc } from "../ipc";
import type { FileViewerTab, FileTextContent, FileOutlineItem } from "./types";
import {
  Copy,
  Check,
  FolderOpen,
  RotateCcw,
  Search,
  X,
  AlertCircle,
  ExternalLink,
  List,
  Edit3,
  Save,
  Eye,
  CheckSquare,
} from "../components/Icons";
import hljs from "highlight.js";

const KIND_BADGES: Record<string, { label: string; cls: string }> = {
  fn: { label: "fn", cls: "bg-blue-500/15 text-blue-400 border-blue-500/30" },
  class: { label: "class", cls: "bg-purple-500/15 text-purple-400 border-purple-500/30" },
  interface: { label: "iface", cls: "bg-emerald-500/15 text-emerald-400 border-emerald-500/30" },
  struct: { label: "struct", cls: "bg-amber-500/15 text-amber-400 border-amber-500/30" },
  enum: { label: "enum", cls: "bg-cyan-500/15 text-cyan-400 border-cyan-500/30" },
  type: { label: "type", cls: "bg-indigo-500/15 text-indigo-400 border-indigo-500/30" },
  impl: { label: "impl", cls: "bg-gray-500/15 text-gray-400 border-gray-500/30" },
  heading: { label: "H", cls: "bg-green-500/15 text-green-400 border-green-500/30" },
  other: { label: "sym", cls: "bg-panel3 text-inkdim border-edge" },
};

export function CodeViewer({ tab }: { tab: FileViewerTab }) {
  const [data, setData] = useState<FileTextContent | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const [searchQuery, setSearchQuery] = useState("");
  const [showSearch, setShowSearch] = useState(false);

  // 编辑模式与手动修改状态
  const [isEditing, setIsEditing] = useState(false);
  const [editContent, setEditContent] = useState("");
  const [isSaving, setIsSaving] = useState(false);
  const [saved, setSaved] = useState(false);

  // 判定是否是任务方案 Markdown 文档
  const isPlanDoc = useMemo(() => {
    const p = tab.path.replace(/\\/g, "/");
    return p.includes("/.harness/plans/") || p.startsWith(".harness/plans/");
  }, [tab.path]);

  const isDirty = useMemo(() => {
    return data !== null && editContent !== data.content;
  }, [data, editContent]);

  // 符号大纲状态
  const [outline, setOutline] = useState<FileOutlineItem[]>([]);
  const [showOutline, setShowOutline] = useState(false);
  const [outlineQuery, setOutlineQuery] = useState("");
  const [jumpedLine, setJumpedLine] = useState<number | null>(null);

  const containerRef = useRef<HTMLDivElement>(null);
  const highlightedLineRef = useRef<HTMLTableRowElement>(null);

  const targetRange = useMemo(() => {
    if (jumpedLine !== null) {
      return { start: jumpedLine, end: jumpedLine };
    }
    if (tab.highlightRange) {
      return {
        start: tab.highlightRange.start,
        end: tab.highlightRange.end ?? tab.highlightRange.start,
      };
    }
    if (tab.highlightLine) {
      return {
        start: tab.highlightLine,
        end: tab.highlightLine,
      };
    }
    return null;
  }, [tab.highlightLine, tab.highlightRange, jumpedLine]);

  const loadFile = async () => {
    setLoading(true);
    setError(null);
    try {
      const res = await ipc.readTextFile(tab.path);
      setData(res);
      setEditContent(res.content);
      // 同时提取符号大纲
      void loadOutline();
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  const loadOutline = async () => {
    try {
      const items = await ipc.getFileOutline(tab.path);
      setOutline(items || []);
    } catch {
      setOutline([]);
    }
  };

  const handleSave = async () => {
    if (isSaving || !data || data.isBinary) return;
    setIsSaving(true);
    try {
      await ipc.saveTextFile(tab.path, editContent, tab.sessionId);
      setData((prev) =>
        prev
          ? {
              ...prev,
              content: editContent,
              lines_count: editContent.split("\n").length,
              size_bytes: new TextEncoder().encode(editContent).length,
            }
          : prev
      );
      setSaved(true);
      setTimeout(() => setSaved(false), 2000);
      void loadOutline();
    } catch (e) {
      alert(`保存文件失败: ${e}`);
    } finally {
      setIsSaving(false);
    }
  };

  useEffect(() => {
    setJumpedLine(null);
    void loadFile();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tab.path]);

  // 监听快捷键：Ctrl+S (保存) / Ctrl+F (搜索) / Ctrl+R or F5 (刷新)
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && (e.key === "s" || e.key === "S")) {
        e.preventDefault();
        void handleSave();
      } else if ((e.ctrlKey || e.metaKey) && (e.key === "f" || e.key === "F")) {
        e.preventDefault();
        setShowSearch((s) => !s);
      } else if (e.key === "Escape") {
        setShowSearch(false);
      } else if ((e.ctrlKey || e.metaKey) && (e.key === "r" || e.key === "R")) {
        e.preventDefault();
        void loadFile();
      } else if (e.key === "F5") {
        e.preventDefault();
        void loadFile();
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [data, editContent, isSaving]);

  // 跳转到高亮行或行范围首行
  useEffect(() => {
    if (targetRange && highlightedLineRef.current) {
      highlightedLineRef.current.scrollIntoView({ behavior: "smooth", block: "center" });
    }
  }, [data, targetRange]);

  const handleCopyContent = () => {
    if (!data?.content) return;
    navigator.clipboard.writeText(data.content).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    });
  };

  const handleCopyPath = () => {
    navigator.clipboard.writeText(tab.path);
  };

  const handleOpenDir = () => {
    ipc.openDir(tab.path);
  };

  const handleOpenExternalEditor = () => {
    const line = targetRange ? targetRange.start : 1;
    ipc.openInExternalEditor(tab.path, line);
  };

  const handleJumpToSymbol = (lineno: number) => {
    setJumpedLine(lineno);
    const elem = document.getElementById(`code-line-${lineno}`);
    if (elem) {
      elem.scrollIntoView({ behavior: "smooth", block: "center" });
    }
  };

  // 代码行语法着色与分割
  const lines = useMemo(() => {
    if (!data || data.isBinary || !data.content) return [];
    try {
      const lang = data.language && hljs.getLanguage(data.language) ? data.language : undefined;
      const highlightedHtml = lang
        ? hljs.highlight(data.content, { language: lang, ignoreIllegals: true }).value
        : hljs.highlightAuto(data.content).value;
      return highlightedHtml.split("\n");
    } catch {
      return data.content.split("\n");
    }
  }, [data]);

  // 过滤符号大纲列表
  const filteredOutline = useMemo(() => {
    if (!outlineQuery.trim()) return outline;
    const q = outlineQuery.toLowerCase();
    return outline.filter((item) => item.symbol.toLowerCase().includes(q));
  }, [outline, outlineQuery]);

  return (
    <div className="flex-1 flex flex-col min-h-0 bg-panel text-ink overflow-hidden">
      {/* 顶部工具栏 */}
      <div className="h-10 px-4 border-b border-edge/60 bg-panel2/60 flex items-center justify-between shrink-0 text-[12px]">
        <div className="flex items-center gap-2 min-w-0">
          <span className="font-semibold text-ink truncate">{tab.title}</span>
          <span className="text-inkdim font-mono text-[11px] truncate max-w-[340px]" title={tab.path}>
            {tab.path}
          </span>
          {data?.language && (
            <span className="text-[10px] px-1.5 py-0.5 rounded bg-panel3 border border-edge text-inkdim font-mono uppercase">
              {data.language}
            </span>
          )}
        </div>

        <div className="flex items-center gap-1.5 shrink-0">
          {/* 编辑 / 预览模式切换 */}
          {!data?.isBinary && (
            <button
              type="button"
              className={`flex items-center gap-1 px-2.5 py-1 rounded-lg transition-colors text-[11.5px] cursor-pointer ${
                isEditing
                  ? "bg-amber-500/20 text-amber-300 border border-amber-500/40 font-medium"
                  : "text-inkdim hover:text-ink hover:bg-panel3"
              }`}
              title={isEditing ? "切换回语法高亮预览" : "切换到编辑模式（支持修改方案与代码）"}
              onClick={() => {
                if (!isEditing && data) {
                  setEditContent(data.content);
                }
                setIsEditing(!isEditing);
              }}
            >
              {isEditing ? <Eye size={13} /> : <Edit3 size={13} />}
              <span>{isEditing ? "预览" : "编辑"}</span>
            </button>
          )}

          {/* 保存按钮 (Ctrl+S) */}
          {!data?.isBinary && (
            <button
              type="button"
              className={`flex items-center gap-1 px-2.5 py-1 rounded-lg transition-colors text-[11.5px] cursor-pointer ${
                isDirty
                  ? "bg-amber-500 text-black font-semibold shadow-xs"
                  : "text-inkdim hover:text-ink hover:bg-panel3"
              }`}
              title="保存文件 (快捷键 Ctrl+S)"
              onClick={handleSave}
              disabled={isSaving}
            >
              {saved ? <Check size={13} className="text-green-400" /> : <Save size={13} />}
              <span>{isSaving ? "保存中…" : saved ? "已保存" : "保存"}</span>
            </button>
          )}

          <div className="h-4 w-px bg-edge/60 mx-0.5" />

          {/* 大纲抽屉切换 */}
          <button
            type="button"
            className={`flex items-center gap-1 px-2 py-1 rounded-lg transition-colors text-[11.5px] cursor-pointer ${
              showOutline
                ? "bg-accent/20 text-accent border border-accent/40 font-medium"
                : "text-inkdim hover:text-ink hover:bg-panel3"
            }`}
            title="代码符号大纲 (函数/类/接口)"
            onClick={() => setShowOutline(!showOutline)}
          >
            <List size={13} />
            <span>大纲</span>
            {outline.length > 0 && (
              <span className="text-[10px] px-1 rounded-full bg-panel3 text-inkdim font-mono">
                {outline.length}
              </span>
            )}
          </button>

          <button
            type="button"
            className="p-1.5 rounded-lg text-inkdim hover:text-ink hover:bg-panel3 transition-colors cursor-pointer"
            title="在文件中查找 (Ctrl+F)"
            onClick={() => setShowSearch(!showSearch)}
          >
            <Search size={14} />
          </button>
          <button
            type="button"
            className="p-1.5 rounded-lg text-inkdim hover:text-ink hover:bg-panel3 transition-colors cursor-pointer"
            title="刷新 (Ctrl+R / F5)"
            onClick={loadFile}
          >
            <RotateCcw size={14} />
          </button>
          <button
            type="button"
            className="p-1.5 rounded-lg text-inkdim hover:text-ink hover:bg-panel3 transition-colors cursor-pointer"
            title="复制文件路径"
            onClick={handleCopyPath}
          >
            <span className="text-[11px] px-1">路径</span>
          </button>
          <button
            type="button"
            className="flex items-center gap-1 px-2 py-1 rounded-lg text-inkdim hover:text-ink hover:bg-panel3 transition-colors text-[11px] cursor-pointer"
            title="复制代码全文"
            onClick={handleCopyContent}
          >
            {copied ? <Check size={13} className="text-green-400" /> : <Copy size={13} />}
            <span>{copied ? "已复制" : "复制"}</span>
          </button>
          <button
            type="button"
            className="flex items-center gap-1 px-2 py-1 rounded-lg text-inkdim hover:text-ink hover:bg-panel3 transition-colors text-[11px] cursor-pointer"
            title="在外部编辑器打开并定位 (VS Code / Cursor)"
            onClick={handleOpenExternalEditor}
          >
            <ExternalLink size={13} className="text-blue-400" />
            <span>外部打开</span>
          </button>
          <button
            type="button"
            className="p-1.5 rounded-lg text-inkdim hover:text-ink hover:bg-panel3 transition-colors cursor-pointer"
            title="在资源管理器中打开"
            onClick={handleOpenDir}
          >
            <FolderOpen size={14} />
          </button>
        </div>
      </div>

      {/* 搜索栏 */}
      {showSearch && (
        <div className="px-4 py-2 border-b border-edge/60 bg-panel3/40 flex items-center gap-2 shrink-0 animate-in slide-in-from-top-1 duration-100">
          <Search size={13} className="text-inkdim" />
          <input
            autoFocus
            className="flex-1 bg-panel border border-edge rounded-lg px-2.5 py-1 text-[12px] text-ink outline-none focus:border-accent"
            placeholder="在当前文件中搜索..."
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
          />
          <button
            type="button"
            className="p-1 rounded text-inkdim hover:text-ink hover:bg-panel cursor-pointer"
            onClick={() => {
              setSearchQuery("");
              setShowSearch(false);
            }}
          >
            <X size={13} />
          </button>
        </div>
      )}

      {/* 状态通知 */}
      {data?.isTruncated && (
        <div className="px-4 py-1.5 bg-amber-500/10 border-b border-amber-500/20 text-amber-300 text-[11.5px] flex items-center gap-2 shrink-0">
          <AlertCircle size={14} className="shrink-0" />
          <span>文件超过预览限制，仅展示前 2MB 内容。如需完整编辑请在外部编辑器中查看。</span>
        </div>
      )}

      {/* 主视图（代码区 + 可折叠符号大纲 / 文本编辑器） */}
      <div className="flex-1 min-h-0 flex flex-row overflow-hidden">
        {isEditing ? (
          <div className="flex-1 flex flex-col min-h-0 bg-[#0c0c0e] p-3">
            <div className="flex items-center justify-between text-[11px] text-inkdim px-1 mb-1.5 select-none font-mono">
              <span className="flex items-center gap-1.5">
                <span className="text-accent font-semibold">EDIT:</span>
                <span>快捷键 Ctrl+S 实时保存修改</span>
                {isPlanDoc && (
                  <span className="text-emerald-400 font-sans ml-1">
                    （修改后按 Ctrl+S 保存即可，可在对话框中直接告知 Agent 继续执行）
                  </span>
                )}
              </span>
              {isDirty && (
                <span className="text-amber-400 font-medium">● 存在未保存改动</span>
              )}
            </div>
            <textarea
              className="flex-1 w-full bg-[#121216] border border-edge/80 rounded-lg p-3 text-ink font-mono text-[12.5px] leading-relaxed outline-none focus:border-accent/60 resize-none selection:bg-accent/30 selection:text-white"
              value={editContent}
              onChange={(e) => setEditContent(e.target.value)}
              placeholder="在此输入或调整方案内容..."
              autoFocus
              spellCheck={false}
              onKeyDown={(e) => {
                if (e.key === "Tab") {
                  e.preventDefault();
                  const target = e.currentTarget;
                  const start = target.selectionStart;
                  const end = target.selectionEnd;
                  const val = editContent;
                  setEditContent(val.substring(0, start) + "  " + val.substring(end));
                  setTimeout(() => {
                    target.selectionStart = target.selectionEnd = start + 2;
                  }, 0);
                }
              }}
            />
          </div>
        ) : (
          /* 代码内容区域 */
          <div ref={containerRef} className="flex-1 overflow-auto font-mono text-[12px] leading-[1.6] select-text">
            {loading && (
              <div className="flex items-center justify-center h-48 text-inkdim text-[13px]">
                读取文件中…
              </div>
            )}

            {error && (
              <div className="p-6 flex flex-col items-center justify-center text-red-400 gap-2">
                <AlertCircle size={24} />
                <div className="text-[13px] font-medium">{error}</div>
                <button
                  className="mt-2 px-3 py-1 bg-panel3 rounded-lg text-ink text-[12px] hover:bg-edge"
                  onClick={loadFile}
                >
                  重新加载
                </button>
              </div>
            )}

            {data?.isBinary && (
              <div className="flex flex-col items-center justify-center h-64 text-inkdim gap-3">
                <div className="text-[14px]">二进制文件，不支持纯文本代码预览</div>
                <button
                  className="px-3.5 py-1.5 rounded-lg bg-panel3 hover:bg-edge text-ink text-[12px] flex items-center gap-1.5 transition-colors cursor-pointer"
                  onClick={handleOpenDir}
                >
                  <FolderOpen size={14} />
                  <span>在文件管理器中打开</span>
                </button>
              </div>
            )}

            {data && !data.isBinary && lines.length === 0 && (
              <div className="p-6 text-inkdim text-center">空文件</div>
            )}

            {data && !data.isBinary && lines.length > 0 && (
              <table className="w-full border-collapse">
                <tbody>
                  {lines.map((lineHtml, idx) => {
                    const lineNum = idx + 1;
                    const isTarget = targetRange
                      ? lineNum >= targetRange.start && lineNum <= targetRange.end
                      : false;
                    const isFirstTarget = targetRange?.start === lineNum;
                    const isMatch = searchQuery && lineHtml.toLowerCase().includes(searchQuery.toLowerCase());

                    return (
                      <tr
                        key={lineNum}
                        id={`code-line-${lineNum}`}
                        ref={isFirstTarget ? highlightedLineRef : undefined}
                        className={`hover:bg-panel3/50 transition-colors ${
                          isTarget
                            ? "bg-amber-500/15 border-l-2 border-amber-400"
                            : isMatch
                            ? "bg-yellow-500/15"
                            : ""
                        }`}
                      >
                        <td className={`w-12 select-none text-right pr-3 pl-2 font-mono text-[11px] align-top border-r border-edge/40 ${
                          isTarget ? "text-amber-300 font-bold" : "text-inkdim/60"
                        }`}>
                          {lineNum}
                        </td>
                        <td
                          className="px-3 whitespace-pre-wrap break-all text-ink select-text font-mono hljs bg-transparent"
                          dangerouslySetInnerHTML={{ __html: lineHtml || "&nbsp;" }}
                        />
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            )}
          </div>
        )}

        {/* 侧边可折叠代码大纲抽屉 */}
        {showOutline && (
          <div className="w-68 border-l border-edge/60 bg-panel2/80 flex flex-col shrink-0 animate-in slide-in-from-right-1 duration-100 select-none">
            <div className="p-2.5 border-b border-edge/60 flex items-center justify-between">
              <span className="font-semibold text-[12px] text-ink flex items-center gap-1.5">
                <List size={13} className="text-accent" />
                <span>代码符号大纲</span>
              </span>
              <button
                type="button"
                className="p-1 rounded text-inkdim hover:text-ink hover:bg-panel3 cursor-pointer"
                onClick={() => setShowOutline(false)}
              >
                <X size={12} />
              </button>
            </div>

            {/* 符号过滤输入框 */}
            <div className="p-2 border-b border-edge/40">
              <div className="flex items-center gap-1.5 bg-panel border border-edge rounded-lg px-2 py-1 text-[11.5px]">
                <Search size={12} className="text-inkdim shrink-0" />
                <input
                  className="w-full bg-transparent outline-none text-ink placeholder:text-inkdim/60 font-mono"
                  placeholder="过滤符号..."
                  value={outlineQuery}
                  onChange={(e) => setOutlineQuery(e.target.value)}
                />
                {outlineQuery && (
                  <button
                    type="button"
                    className="text-inkdim hover:text-ink"
                    onClick={() => setOutlineQuery("")}
                  >
                    <X size={11} />
                  </button>
                )}
              </div>
            </div>

            {/* 符号列表 */}
            <div className="flex-1 overflow-y-auto p-1.5 space-y-0.5 font-mono text-[11.5px]">
              {filteredOutline.length === 0 && (
                <div className="p-4 text-center text-inkdim text-[11px]">
                  {outline.length === 0 ? "未识别到符号定义" : "无匹配符号"}
                </div>
              )}

              {filteredOutline.map((item, idx) => {
                const badge = KIND_BADGES[item.kind] || KIND_BADGES.other;
                const isSelected = targetRange?.start === item.line;

                return (
                  <div
                    key={`${item.line}-${idx}`}
                    role="button"
                    tabIndex={0}
                    onClick={() => handleJumpToSymbol(item.line)}
                    className={`flex items-center gap-1.5 px-2 py-1.5 rounded-md cursor-pointer transition-colors ${
                      isSelected
                        ? "bg-accent/20 text-accent font-medium"
                        : "text-inkdim hover:text-ink hover:bg-panel3/70"
                    }`}
                    title={`第 ${item.line} 行: ${item.symbol}`}
                  >
                    <span className="text-[10.5px] text-inkdim/60 w-8 shrink-0 text-right">
                      {item.line}
                    </span>
                    <span className={`text-[9px] px-1 py-0.2 rounded border font-mono shrink-0 uppercase ${badge.cls}`}>
                      {badge.label}
                    </span>
                    <span className="truncate flex-1 text-ink/90">{item.symbol}</span>
                  </div>
                );
              })}
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
