import { create } from "zustand";
import { listen } from "@tauri-apps/api/event";
import { ipc } from "../ipc";
import type { ViewerTabItem, FileViewerStoreState } from "./types";

const STORAGE_KEY_PREFIX = "harness_file_viewer_tabs";

function getStorageKey(workspacePath?: string): string {
  if (!workspacePath) return `${STORAGE_KEY_PREFIX}_default`;
  // Simple hash for workspace path
  let hash = 0;
  for (let i = 0; i < workspacePath.length; i++) {
    hash = (hash << 5) - hash + workspacePath.charCodeAt(i);
    hash |= 0;
  }
  return `${STORAGE_KEY_PREFIX}_${Math.abs(hash)}`;
}

function loadPersistedTabs(workspacePath?: string): { tabs: ViewerTabItem[]; activeTabId: string | null } {
  try {
    const raw = localStorage.getItem(getStorageKey(workspacePath));
    if (!raw) return { tabs: [], activeTabId: null };
    const parsed = JSON.parse(raw);
    if (Array.isArray(parsed.tabs)) {
      return {
        tabs: parsed.tabs,
        activeTabId: parsed.activeTabId || (parsed.tabs[0]?.id ?? null),
      };
    }
  } catch (e) {
    console.warn("Failed to load persisted tabs:", e);
  }
  return { tabs: [], activeTabId: null };
}

function savePersistedTabs(workspacePath: string, tabs: ViewerTabItem[], activeTabId: string | null) {
  try {
    const payload = JSON.stringify({ tabs, activeTabId });
    localStorage.setItem(getStorageKey(workspacePath), payload);
  } catch (e) {
    console.warn("Failed to persist tabs:", e);
  }
}

const IMAGE_EXTENSIONS = new Set(["png", "jpg", "jpeg", "gif", "svg", "webp", "ico", "bmp"]);

export function isImageFile(filePath?: string): boolean {
  if (!filePath) return false;
  const ext = filePath.split(".").pop()?.toLowerCase();
  return ext ? IMAGE_EXTENSIONS.has(ext) : false;
}

export const useFileViewerStore = create<FileViewerStoreState>((set, get) => {
  const initial = loadPersistedTabs();

  return {
    workspacePath: "",
    tabs: initial.tabs,
    activeTabId: initial.activeTabId,

    setWorkspacePath: (path: string) => {
      const currentWs = get().workspacePath;
      if (currentWs === path) return;
      const loaded = loadPersistedTabs(path);
      set({
        workspacePath: path,
        tabs: loaded.tabs,
        activeTabId: loaded.activeTabId,
      });
    },

    openTab: (incomingTab: ViewerTabItem) => {
      const tab: ViewerTabItem =
        incomingTab.type === "file" && isImageFile(incomingTab.path)
          ? { ...incomingTab, type: "image" }
          : incomingTab;

      const { tabs, workspacePath } = get();
      const existingIdx = tabs.findIndex((t) => t.id === tab.id);

      const tabWithNonce: ViewerTabItem = {
        ...tab,
        reloadNonce: Date.now(),
      };

      let newTabs: ViewerTabItem[];
      if (existingIdx !== -1) {
        // 更新现有 tab（如参数发生变化）并激活，同时更新 reloadNonce 触发重新拉取
        newTabs = [...tabs];
        newTabs[existingIdx] = { ...newTabs[existingIdx], ...tabWithNonce };
      } else {
        newTabs = [...tabs, tabWithNonce];
      }

      set({ tabs: newTabs, activeTabId: tab.id });
      savePersistedTabs(workspacePath || tab.workspacePath || "", newTabs, tab.id);
    },

    closeTab: (tabId: string) => {
      const { tabs, activeTabId, workspacePath } = get();
      const targetIdx = tabs.findIndex((t) => t.id === tabId);
      if (targetIdx === -1) return;

      const newTabs = tabs.filter((t) => t.id !== tabId);
      let newActiveId = activeTabId;

      if (activeTabId === tabId) {
        if (newTabs.length === 0) {
          newActiveId = null;
        } else if (targetIdx > 0) {
          newActiveId = newTabs[targetIdx - 1].id;
        } else {
          newActiveId = newTabs[0].id;
        }
      }

      set({ tabs: newTabs, activeTabId: newActiveId });
      savePersistedTabs(workspacePath, newTabs, newActiveId);
    },

    closeOtherTabs: (tabId: string) => {
      const { tabs, workspacePath } = get();
      const newTabs = tabs.filter((t) => t.id === tabId || t.pinned);
      set({ tabs: newTabs, activeTabId: tabId });
      savePersistedTabs(workspacePath, newTabs, tabId);
    },

    closeAllTabs: () => {
      const { tabs, workspacePath } = get();
      const newTabs = tabs.filter((t) => t.pinned);
      const newActiveId = newTabs[0]?.id || null;
      set({ tabs: newTabs, activeTabId: newActiveId });
      savePersistedTabs(workspacePath, newTabs, newActiveId);
    },

    setActiveTab: (tabId: string) => {
      const { tabs, workspacePath } = get();
      const targetIdx = tabs.findIndex((t) => t.id === tabId);
      if (targetIdx !== -1) {
        const newTabs = [...tabs];
        newTabs[targetIdx] = { ...newTabs[targetIdx], reloadNonce: Date.now() };
        set({ tabs: newTabs, activeTabId: tabId });
        savePersistedTabs(workspacePath, newTabs, tabId);
      }
    },

    togglePinTab: (tabId: string) => {
      const { tabs, activeTabId, workspacePath } = get();
      const newTabs = tabs.map((t) => (t.id === tabId ? { ...t, pinned: !t.pinned } : t));
      set({ tabs: newTabs });
      savePersistedTabs(workspacePath, newTabs, activeTabId);
    },

    reorderTabs: (fromIndex: number, toIndex: number) => {
      const { tabs, activeTabId, workspacePath } = get();
      if (
        fromIndex < 0 ||
        fromIndex >= tabs.length ||
        toIndex < 0 ||
        toIndex >= tabs.length ||
        fromIndex === toIndex
      ) {
        return;
      }
      const newTabs = [...tabs];
      const [moved] = newTabs.splice(fromIndex, 1);
      newTabs.splice(toIndex, 0, moved);
      set({ tabs: newTabs });
      savePersistedTabs(workspacePath, newTabs, activeTabId);
    },

    notifyFileChanged: (filePath: string) => {
      const { tabs, workspacePath, activeTabId } = get();
      const normalized = filePath.replace(/\\/g, "/").toLowerCase();
      let hasChanges = false;
      const newTabs = tabs.map((t) => {
        const p = "path" in t && typeof t.path === "string" ? t.path.replace(/\\/g, "/").toLowerCase() : "";
        if (p && (p === normalized || normalized.endsWith(p) || p.endsWith(normalized))) {
          hasChanges = true;
          return { ...t, reloadNonce: Date.now() };
        }
        return t;
      });
      if (hasChanges) {
        set({ tabs: newTabs });
        savePersistedTabs(workspacePath, newTabs, activeTabId);
      }
    },
  };
});


/** 初始化监听主窗口传递的打开 tab 事件及初始数据 */
export function initFileViewerListeners() {
  const store = useFileViewerStore.getState();

  // 1. 获取启动时预存的初始 tab
  ipc.getFileViewerInitTab().then((initTab) => {
    if (initTab) {
      if (initTab.workspacePath) {
        store.setWorkspacePath(initTab.workspacePath);
      }
      store.openTab(initTab);
    }
  }).catch((err) => console.warn("Failed to get initial tab:", err));

  // 2. 监听后续主窗口派发的打开 tab 事件
  const unlistenOpenTabPromise = listen<ViewerTabItem>("file_viewer:open_tab", (event) => {
    if (event.payload) {
      const payload = event.payload;
      if (payload.workspacePath) {
        useFileViewerStore.getState().setWorkspacePath(payload.workspacePath);
      }
      useFileViewerStore.getState().openTab(payload);
    }
  });

  // 3. 监听全局文件变更广播 (Agent 写文件或外部同步)
  const unlistenChangedPromise = listen<{ path?: string } | string>("file_viewer:file_changed", (event) => {
    if (event.payload) {
      const path = typeof event.payload === "string" ? event.payload : event.payload.path;
      if (path) {
        useFileViewerStore.getState().notifyFileChanged(path);
      }
    }
  });

  return () => {
    unlistenOpenTabPromise.then((unlisten) => unlisten());
    unlistenChangedPromise.then((unlisten) => unlisten());
  };
}
