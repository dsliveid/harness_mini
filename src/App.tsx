import { useEffect } from "react";
import { useAppEvents } from "./events";
import { useStore, SUBAGENT_MIN_PANEL_WIDTH, MAIN_PANEL_MIN_WIDTH } from "./store";
import { ArchiveModal } from "./components/ArchiveModal";
import { ChatView } from "./components/ChatView";
import { Composer } from "./components/Composer";
import { DataDirGate } from "./components/DataDirGate";
import { DiffModal } from "./components/DiffModal";
import { GrowthModal } from "./components/GrowthModal";
import { PendingQueue } from "./components/PendingQueue";
import { PromptHost } from "./components/PromptModal";
import { ProjectSettingsModal } from "./components/ProjectSettingsModal";
import { SettingsModal } from "./components/SettingsModal";
import { SessionSettingsModal } from "./components/SessionSettingsModal";
import { Sidebar } from "./components/Sidebar";
import { TempActions } from "./components/TempActions";
import { Toasts } from "./components/Toasts";
import { TokenStatsModal } from "./components/TokenStatsModal";
import { TopBar } from "./components/TopBar";
import { TaskBar } from "./components/TaskBar";
import { TaskDetailModal } from "./components/TaskDetailModal";
import { CollaboratorBar } from "./components/CollaboratorBar";
import { CollaboratorView } from "./components/CollaboratorView";
import { SubagentResizeHandle } from "./components/SubagentResizeHandle";
import { CreateCollaboratorModal } from "./components/CreateCollaboratorModal";
import { EditCollaboratorModal } from "./components/EditCollaboratorModal";
import { ModelMatrixModal } from "./components/ModelMatrixModal";
import { ImageLightboxModal } from "./components/ImageLightboxModal";
import { WindowHeader } from "./components/WindowHeader";

import { DRAFT_ID } from "./types";

const EMPTY_COLLABS: any[] = [];

export default function App() {
  useAppEvents();
  const bootstrap = useStore((s) => s.bootstrap);
  const ready = useStore((s) => s.ready);
  const tempClearing = useStore((s) => s.tempClearing);
  const activeCollaboratorId = useStore((s) => s.activeCollaboratorId);
  const activeSubprocessId = useStore((s) => s.activeSubprocessId);
  const activeSubagentId = useStore((s) => s.activeSubagentId);
  const currentParentId = useStore((s) => s.currentId);
  const allCollabs = useStore((s) => s.collaborators);
  const collabs = currentParentId ? allCollabs[currentParentId] ?? EMPTY_COLLABS : EMPTY_COLLABS;
  const subagentPanelWidth = useStore((s) => s.subagentPanelWidth);

  // 判定右侧分屏当前要激活的面板类型与 ID（新对话草稿或无会话时不开启右侧分屏）
  const hasValidParent = !!currentParentId && currentParentId !== DRAFT_ID;
  const activeSideId = hasValidParent
    ? activeSubprocessId || activeCollaboratorId || activeSubagentId
    : null;
  const isCollaborator = activeCollaboratorId
    ? true
    : activeSubprocessId
    ? false
    : activeSubagentId
    ? collabs.some((c) => c.id === activeSubagentId)
    : false;

  useEffect(() => {
    void bootstrap();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  if (!ready) {
    return (
      <div className="h-full flex flex-col text-[14px]">
        <WindowHeader />
        <div className="flex-1 flex items-center justify-center text-inkdim">加载中…</div>
      </div>
    );
  }

  return (
    <div className="h-full flex flex-col text-[14px]">
      <WindowHeader />
      <div className="flex-1 min-h-0 flex overflow-x-auto scrollbar-none">
        <Sidebar />
        <div
          style={{ minWidth: `${MAIN_PANEL_MIN_WIDTH}px` }}
          className="flex-1 flex flex-col relative overflow-hidden"
        >
          <TopBar />
          <TaskBar />
          <CollaboratorBar />
          <ChatView />
          <PendingQueue />
          <div className="relative">
            <Composer />
            {/* 临时空间操作（变更 / 临时目录 / 合并 / 清空空间）：悬浮在输入框上方左对齐 */}
            <TempActions />
          </div>
          {tempClearing && (
            <div className="absolute inset-0 z-40 bg-black/40 flex items-center justify-center">
              <div className="bg-panel2 border border-edge rounded-xl px-6 py-5 flex flex-col items-center gap-3 shadow-xl">
                <span className="w-6 h-6 rounded-full border-2 border-edge border-t-accent animate-spin" />
                <span className="text-[13px] text-inkdim">正在删除临时空间…</span>
              </div>
            </div>
          )}
        </div>
        {activeSideId && (
          <>
            <SubagentResizeHandle />
            <div
              style={{
                width: `${Math.max(SUBAGENT_MIN_PANEL_WIDTH, Math.min(subagentPanelWidth, window.innerWidth - 240 - MAIN_PANEL_MIN_WIDTH))}px`,
                minWidth: `${SUBAGENT_MIN_PANEL_WIDTH}px`,
              }}
              className="shrink-0 flex flex-col min-h-0 overflow-hidden"
            >
              <CollaboratorView collaboratorId={activeSideId} />
            </div>
          </>
        )}
      </div>
      <SettingsModal />
      <SessionSettingsModal />
      <ProjectSettingsModal />
      <ArchiveModal />
      <DiffModal />
      <GrowthModal />
      <TokenStatsModal />
      <TaskDetailModal />
      <CreateCollaboratorModal />
      <EditCollaboratorModal />
      <ModelMatrixModal />
      <ImageLightboxModal />
      <PromptHost />
      <Toasts />
      <DataDirGate />
    </div>
  );
}
