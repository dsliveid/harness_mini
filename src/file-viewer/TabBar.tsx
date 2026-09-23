import React, { useRef, useState, useEffect } from "react";
import { useFileViewerStore } from "./store";
import type { ViewerTabItem } from "./types";
import {
  FileText,
  FileCode,
  GitCompare,
  CheckSquare,
  Image as ImageIcon,
  BookOpen,
  X,
  Copy,
  FolderOpen,
  Pin,
} from "../components/Icons";
import { ipc } from "../ipc";

interface ContextMenuState {
  x: number;
  y: number;
  tab: ViewerTabItem;
}

function TabIcon({ tab }: { tab: ViewerTabItem }) {
  if (tab.type === "plan") {
    return <CheckSquare size={13} className="text-emerald-400 shrink-0" />;
  }
  if (tab.type === "diff") {
    return <GitCompare size={13} className="text-amber-400 shrink-0" />;
  }
  if (tab.type === "image") {
    return <ImageIcon size={13} className="text-purple-400 shrink-0" />;
  }
  const tabPath = "path" in tab ? String(tab.path || "") : "";
  if (tabPath.includes("/.harness/memory/") || tabPath.includes("\\.harness\\memory\\")) {
    return <BookOpen size={13} className="text-teal-400 shrink-0" />;
  }
  const ext = tab.title.split(".").pop()?.toLowerCase();
  if (["ts", "tsx", "js", "jsx", "rs", "py", "go", "java", "c", "cpp", "json"].includes(ext || "")) {
    return <FileCode size={13} className="text-blue-400 shrink-0" />;
  }
  if (["md", "markdown", "mdown", "mkdn"].includes(ext || "")) {
    return <FileText size={13} className="text-sky-400 shrink-0" />;
  }
  return <FileText size={13} className="text-inkdim shrink-0" />;
}

export function TabBar() {
  const tabs = useFileViewerStore((s) => s.tabs);
  const activeTabId = useFileViewerStore((s) => s.activeTabId);
  const setActiveTab = useFileViewerStore((s) => s.setActiveTab);
  const closeTab = useFileViewerStore((s) => s.closeTab);
  const closeOtherTabs = useFileViewerStore((s) => s.closeOtherTabs);
  const closeAllTabs = useFileViewerStore((s) => s.closeAllTabs);
  const togglePinTab = useFileViewerStore((s) => s.togglePinTab);
  const reorderTabs = useFileViewerStore((s) => s.reorderTabs);

  const [menu, setMenu] = useState<ContextMenuState | null>(null);
  const [draggedIndex, setDraggedIndex] = useState<number | null>(null);
  const [dropTargetIndex, setDropTargetIndex] = useState<number | null>(null);
  const scrollRef = useRef<HTMLDivElement>(null);

  // 支持滚轮水平滚动页签栏
  const handleWheel = (e: React.WheelEvent) => {
    if (scrollRef.current) {
      scrollRef.current.scrollLeft += e.deltaY;
    }
  };

  // 点击外部关闭右键菜单
  useEffect(() => {
    const handleCloseMenu = () => setMenu(null);
    window.addEventListener("click", handleCloseMenu);
    return () => window.removeEventListener("click", handleCloseMenu);
  }, []);

  return (
    <div className="relative border-b border-edge/60 bg-panel3/40 shrink-0 select-none">
      <div
        ref={scrollRef}
        onWheel={handleWheel}
        className="flex items-center overflow-x-auto scrollbar-none h-9 text-[12px] px-1 gap-0.5"
      >
        {tabs.map((tab, index) => {
          const isActive = tab.id === activeTabId;
          const isDropTarget = dropTargetIndex === index && draggedIndex !== index;
          const isBeingDragged = draggedIndex === index;

          return (
            <div
              key={tab.id}
              role="button"
              tabIndex={0}
              title={tab.subtitle || tab.title}
              draggable={!tab.pinned}
              onDragStart={(e) => {
                e.dataTransfer.setData("text/plain", String(index));
                e.dataTransfer.effectAllowed = "move";
                setDraggedIndex(index);
              }}
              onDragOver={(e) => {
                e.preventDefault();
                e.dataTransfer.dropEffect = "move";
                if (dropTargetIndex !== index) {
                  setDropTargetIndex(index);
                }
              }}
              onDragLeave={() => {
                if (dropTargetIndex === index) {
                  setDropTargetIndex(null);
                }
              }}
              onDrop={(e) => {
                e.preventDefault();
                if (draggedIndex !== null && draggedIndex !== index) {
                  reorderTabs(draggedIndex, index);
                }
                setDraggedIndex(null);
                setDropTargetIndex(null);
              }}
              onDragEnd={() => {
                setDraggedIndex(null);
                setDropTargetIndex(null);
              }}
              onClick={() => setActiveTab(tab.id)}
              onAuxClick={(e) => {
                // 中键关闭页签
                if (e.button === 1) {
                  e.preventDefault();
                  closeTab(tab.id);
                }
              }}
              onContextMenu={(e) => {
                e.preventDefault();
                setMenu({ x: e.clientX, y: e.clientY, tab });
              }}
              className={`group flex items-center gap-1.5 h-7 px-2.5 rounded-md cursor-pointer transition-all max-w-[200px] border ${
                isDropTarget
                  ? "border-accent bg-accent/15 text-accent"
                  : isBeingDragged
                  ? "opacity-40 border-dashed border-edge"
                  : isActive
                  ? "bg-panel border-edge/80 text-ink shadow-2xs font-medium"
                  : "border-transparent text-inkdim hover:text-ink hover:bg-panel/50"
              }`}
            >
              <TabIcon tab={tab} />
              <span className="truncate flex-1 min-w-0">{tab.title}</span>
              {tab.pinned && <Pin size={11} className="text-accent shrink-0 rotate-45" />}
              <button
                type="button"
                className={`w-4 h-4 rounded flex items-center justify-center text-inkdim hover:text-ink hover:bg-panel3 transition-all shrink-0 cursor-pointer ${
                  isActive ? "opacity-70 hover:opacity-100" : "opacity-0 group-hover:opacity-100"
                }`}
                title="关闭页签 (Ctrl+W)"
                onClick={(e) => {
                  e.stopPropagation();
                  closeTab(tab.id);
                }}
              >
                <X size={12} />
              </button>
            </div>
          );
        })}
        {tabs.length === 0 && (
          <div className="text-[12px] text-inkdim/60 px-3 py-1">暂无打开的文件页签</div>
        )}
      </div>


      {/* 右键上下文菜单 */}
      {menu && (
        <div
          style={{ top: `${menu.y}px`, left: `${menu.x}px` }}
          className="fixed z-[100] w-48 bg-panel2/98 backdrop-blur-md border border-edge rounded-xl shadow-2xl py-1 text-[12px] animate-in fade-in zoom-in-95 duration-75 select-none"
          onClick={(e) => e.stopPropagation()}
        >
          <button
            className="w-full px-3 py-1.5 text-left hover:bg-panel3 flex items-center gap-2 text-ink transition-colors cursor-pointer"
            onClick={() => {
              closeTab(menu.tab.id);
              setMenu(null);
            }}
          >
            <X size={13} className="text-inkdim" />
            <span>关闭当前页签</span>
          </button>
          <button
            className="w-full px-3 py-1.5 text-left hover:bg-panel3 flex items-center gap-2 text-ink transition-colors cursor-pointer"
            onClick={() => {
              closeOtherTabs(menu.tab.id);
              setMenu(null);
            }}
          >
            <span>关闭其他页签</span>
          </button>
          <button
            className="w-full px-3 py-1.5 text-left hover:bg-panel3 flex items-center gap-2 text-ink transition-colors cursor-pointer"
            onClick={() => {
              closeAllTabs();
              setMenu(null);
            }}
          >
            <span>关闭所有页签</span>
          </button>
          <div className="border-t border-edge/60 my-1" />
          <button
            className="w-full px-3 py-1.5 text-left hover:bg-panel3 flex items-center gap-2 text-ink transition-colors cursor-pointer"
            onClick={() => {
              togglePinTab(menu.tab.id);
              setMenu(null);
            }}
          >
            <Pin size={13} className="text-inkdim" />
            <span>{menu.tab.pinned ? "取消固定页签" : "固定页签"}</span>
          </button>
          {"path" in menu.tab && menu.tab.path && (
            <>
              <div className="border-t border-edge/60 my-1" />
              <button
                className="w-full px-3 py-1.5 text-left hover:bg-panel3 flex items-center gap-2 text-ink transition-colors cursor-pointer"
                onClick={() => {
                  navigator.clipboard.writeText((menu.tab as any).path);
                  setMenu(null);
                }}
              >
                <Copy size={13} className="text-inkdim" />
                <span>复制文件完整路径</span>
              </button>
              <button
                className="w-full px-3 py-1.5 text-left hover:bg-panel3 flex items-center gap-2 text-ink transition-colors cursor-pointer"
                onClick={() => {
                  ipc.openDir((menu.tab as any).path);
                  setMenu(null);
                }}
              >
                <FolderOpen size={13} className="text-inkdim" />
                <span>在文件管理器中定位</span>
              </button>
            </>
          )}
        </div>
      )}
    </div>
  );
}
