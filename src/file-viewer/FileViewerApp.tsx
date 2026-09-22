import React, { useEffect } from "react";
import { WindowHeader } from "../components/WindowHeader";
import { TabBar } from "./TabBar";
import { CodeViewer } from "./CodeViewer";
import { PlanViewer } from "./PlanViewer";
import { DiffViewer } from "./DiffViewer";
import { ImageViewer } from "./ImageViewer";
import { useFileViewerStore, initFileViewerListeners } from "./store";
import { Files, Layers } from "../components/Icons";

export function FileViewerApp() {
  const tabs = useFileViewerStore((s) => s.tabs);
  const activeTabId = useFileViewerStore((s) => s.activeTabId);
  const workspacePath = useFileViewerStore((s) => s.workspacePath);
  const closeTab = useFileViewerStore((s) => s.closeTab);
  const setActiveTab = useFileViewerStore((s) => s.setActiveTab);

  useEffect(() => {
    const unbind = initFileViewerListeners();
    return () => unbind();
  }, []);

  // 全局快捷键：Ctrl+W 关闭当前页签，Ctrl+Tab 循环切换
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && (e.key === "w" || e.key === "W")) {
        e.preventDefault();
        if (activeTabId) {
          closeTab(activeTabId);
        }
      } else if (e.ctrlKey && e.key === "Tab") {
        e.preventDefault();
        if (tabs.length > 1) {
          const curIdx = tabs.findIndex((t) => t.id === activeTabId);
          const nextIdx = e.shiftKey
            ? (curIdx - 1 + tabs.length) % tabs.length
            : (curIdx + 1) % tabs.length;
          setActiveTab(tabs[nextIdx].id);
        }
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [tabs, activeTabId, closeTab, setActiveTab]);

  const activeTab = tabs.find((t) => t.id === activeTabId);

  return (
    <div className="h-full flex flex-col bg-panel text-ink text-[13px] select-none">
      {/* 独立窗口自定义标题栏 */}
      <WindowHeader
        title="文件与变更查看器"
        subtitle={activeTab ? activeTab.title : undefined}
      />

      {/* 多页签栏 */}
      <TabBar />

      {/* 主工作区 */}
      <div className="flex-1 min-h-0 flex flex-col relative overflow-hidden">
        {activeTab ? (
          <>
            {activeTab.type === "file" && <CodeViewer tab={activeTab} />}
            {activeTab.type === "plan" && <PlanViewer tab={activeTab} />}
            {activeTab.type === "diff" && <DiffViewer tab={activeTab} />}
            {activeTab.type === "image" && <ImageViewer tab={activeTab} />}
          </>
        ) : (

          <div className="flex-1 flex flex-col items-center justify-center text-inkdim gap-3 p-8">
            <div className="w-12 h-12 rounded-2xl bg-panel3/60 border border-edge/80 flex items-center justify-center text-inkdim">
              <Files size={24} />
            </div>
            <div className="text-[14px] font-medium text-ink">暂未打开任何文件</div>
            <div className="text-[12px] text-inkdim/80 text-center max-w-sm leading-relaxed">
              在主对话窗口中，点击 Agent 提到的文件路径、任务计划卡片或变更对比按钮，即可在此独立查看。
            </div>
          </div>
        )}
      </div>

      {/* 底部状态栏 */}
      <div className="h-6 px-3 bg-panel2/80 border-t border-edge/60 flex items-center justify-between text-[11px] text-inkdim shrink-0 font-mono">
        <div className="flex items-center gap-3 truncate min-w-0">
          {activeTab && (
            <span className="flex items-center gap-1 text-ink/80 truncate">
              <span className="capitalize text-accent font-semibold">{activeTab.type}:</span>
              <span className="truncate" title={activeTab.subtitle || activeTab.title}>
                {activeTab.title}
              </span>
            </span>
          )}
        </div>

        <div className="flex items-center gap-3 shrink-0">
          {workspacePath && (
            <span className="truncate max-w-[240px]" title={`当前工作区: ${workspacePath}`}>
              {workspacePath.replace(/\\/g, "/").split("/").pop()}
            </span>
          )}
          <span className="flex items-center gap-1">
            <Layers size={11} />
            <span>{tabs.length} 个页签</span>
          </span>
        </div>
      </div>
    </div>
  );
}
