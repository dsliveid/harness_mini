import { useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Wind, Minus, Maximize2, Copy, X } from "./Icons";

/** 安全获取 Tauri 窗口实例，环境不可用时不抛错 */
function getAppWindow() {
  try {
    if (typeof window !== "undefined" && (window as any).__TAURI_INTERNALS__) {
      return getCurrentWindow();
    }
  } catch (err) {
    console.warn("Tauri window API unavailable:", err);
  }
  return null;
}

export function WindowHeader() {
  const [isMaximized, setIsMaximized] = useState(false);

  useEffect(() => {
    const win = getAppWindow();
    if (!win) return;

    let mounted = true;
    win.isMaximized().then((m) => {
      if (mounted) setIsMaximized(m);
    }).catch(() => {});

    const unlistenPromise = win.onResized(() => {
      win.isMaximized().then((m) => {
        if (mounted) setIsMaximized(m);
      }).catch(() => {});
    });

    return () => {
      mounted = false;
      void unlistenPromise.then((unlisten) => unlisten && unlisten());
    };
  }, []);

  const handleMinimize = () => {
    const win = getAppWindow();
    if (win) {
      void win.minimize().catch((err) => console.warn("Minimize failed:", err));
    }
  };

  const handleToggleMaximize = () => {
    const win = getAppWindow();
    if (win) {
      void win.toggleMaximize().then(() => {
        void win.isMaximized().then(setIsMaximized);
      }).catch((err) => console.warn("Toggle maximize failed:", err));
    }
  };

  const handleClose = () => {
    const win = getAppWindow();
    if (win) {
      void win.close().catch((err) => console.warn("Close failed:", err));
    }
  };

  return (
    <header
      className="h-10 shrink-0 flex items-center justify-between border-b border-edge/50 bg-panel2 select-none z-50 text-[12px]"
      data-tauri-drag-region
      onDoubleClick={handleToggleMaximize}
    >
      {/* 左侧：品牌 Logo 与标题 */}
      <div className="flex items-center gap-2 px-3 pointer-events-none" data-tauri-drag-region>
        <div className="flex h-4 w-4 items-center justify-center rounded bg-accent/20 text-accent">
          <Wind size={11} strokeWidth={2.4} />
        </div>
        <span className="font-semibold tracking-wider text-[11px] text-ink/90 font-mono">HARNESS MINI</span>
      </div>

      {/* 中间：可拖拽大区域 */}
      <div className="flex-1 h-full cursor-default" data-tauri-drag-region />

      {/* 右侧：窗口控制按钮组 */}
      <div className="flex h-full items-center shrink-0">
        <button
          type="button"
          tabIndex={-1}
          className="flex h-full w-12 items-center justify-center text-inkdim hover:bg-panel3 hover:text-ink transition-colors cursor-pointer"
          title="最小化"
          aria-label="最小化窗口"
          onClick={handleMinimize}
        >
          <Minus size={13} strokeWidth={2} />
        </button>
        <button
          type="button"
          tabIndex={-1}
          className="flex h-full w-12 items-center justify-center text-inkdim hover:bg-panel3 hover:text-ink transition-colors cursor-pointer"
          title={isMaximized ? "还原" : "最大化"}
          aria-label={isMaximized ? "向下还原窗口" : "最大化窗口"}
          onClick={handleToggleMaximize}
        >
          {isMaximized ? (
            <Copy size={11} className="rotate-180 text-inkdim" strokeWidth={2} />
          ) : (
            <Maximize2 size={11} strokeWidth={2} />
          )}
        </button>
        <button
          type="button"
          tabIndex={-1}
          className="flex h-full w-12 items-center justify-center text-inkdim hover:bg-red-500 hover:text-white transition-colors cursor-pointer"
          title="关闭"
          aria-label="关闭窗口"
          onClick={handleClose}
        >
          <X size={14} strokeWidth={2} />
        </button>
      </div>
    </header>
  );
}
