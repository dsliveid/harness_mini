import type {
  FileViewerTab,
  ImageViewerTab,
  PlanViewerTab,
  DiffViewerTab,
  ViewerTabItem,
  ViewerTabType,
  FileTextContent,
  ActivePlanDetail,
  TempFileDiff,
  DiffHunk,
  DiffLine,
  FileOutlineItem,
} from "../types";

export type {
  FileViewerTab,
  ImageViewerTab,
  PlanViewerTab,
  DiffViewerTab,
  ViewerTabItem,
  ViewerTabType,
  FileTextContent,
  ActivePlanDetail,
  TempFileDiff,
  DiffHunk,
  DiffLine,
  FileOutlineItem,
};

export interface FileViewerStoreState {
  workspacePath: string;
  tabs: ViewerTabItem[];
  activeTabId: string | null;
  setWorkspacePath: (path: string) => void;
  openTab: (tab: ViewerTabItem) => void;
  closeTab: (tabId: string) => void;
  closeOtherTabs: (tabId: string) => void;
  closeAllTabs: () => void;
  setActiveTab: (tabId: string) => void;
  togglePinTab: (tabId: string) => void;
  reorderTabs: (fromIndex: number, toIndex: number) => void;
}

