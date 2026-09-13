import { useEffect } from "react";
import { useAppEvents } from "./events";
import { useStore } from "./store";
import { ArchiveModal } from "./components/ArchiveModal";
import { ChatView } from "./components/ChatView";
import { Composer } from "./components/Composer";
import { DataDirGate } from "./components/DataDirGate";
import { DiffModal } from "./components/DiffModal";
import { PendingQueue } from "./components/PendingQueue";
import { PromptHost } from "./components/PromptModal";
import { ProjectSettingsModal } from "./components/ProjectSettingsModal";
import { SettingsModal } from "./components/SettingsModal";
import { Sidebar } from "./components/Sidebar";
import { TempActions } from "./components/TempActions";
import { Toasts } from "./components/Toasts";
import { TopBar } from "./components/TopBar";

export default function App() {
  useAppEvents();
  const bootstrap = useStore((s) => s.bootstrap);
  const ready = useStore((s) => s.ready);

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
      <div className="flex-1 flex flex-col min-w-0">
        <TopBar />
        <ChatView />
        <PendingQueue />
        <div className="relative">
          <Composer />
          {/* 临时空间操作（变更 / 临时目录 / 合并 / 清空空间）：悬浮在输入框上方左对齐 */}
          <TempActions />
        </div>
      </div>
      <SettingsModal />
      <ProjectSettingsModal />
      <ArchiveModal />
      <DiffModal />
      <PromptHost />
      <Toasts />
      <DataDirGate />
    </div>
  );
}
