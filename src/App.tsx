import { useEffect } from "react";
import { useAppEvents } from "./events";
import { useStore } from "./store";
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
import { TopBar } from "./components/TopBar";

export default function App() {
  useAppEvents();
  const bootstrap = useStore((s) => s.bootstrap);
  const ready = useStore((s) => s.ready);
  const tempClearing = useStore((s) => s.tempClearing);

  useEffect(() => {
    void bootstrap();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  if (!ready) {
    return <div className="h-full flex items-center justify-center text-inkdim">加载中…</div>;
  }

  return (
    <div className="h-full flex text-[14px]">
      <Sidebar />
      <div className="flex-1 flex flex-col min-w-0 relative">
        <TopBar />
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
      <SettingsModal />
      <SessionSettingsModal />
      <ProjectSettingsModal />
      <ArchiveModal />
      <DiffModal />
      <GrowthModal />
      <PromptHost />
      <Toasts />
      <DataDirGate />
    </div>
  );
}
