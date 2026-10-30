import { useEffect, useState, useRef } from "react";
import { ipc } from "../ipc";
import { useStore, currentSession } from "../store";
import type { BrowserDebugState, InteractiveElement, StorageStateSummary } from "../types";
import { resolveSessionActiveModel } from "../types";
import {
  Globe,
  X,
  RefreshCw,
  Camera,
  Trash2,
  Monitor,
  MousePointer,
  AlertTriangle,
  AlertCircle,
  Bug,
  Activity,
  Terminal,
  Search,
  Sparkles,
  ExternalLink,
  ChevronRight,
  ChevronDown,
  ChevronUp,
  Send,
  Loader2,
  List,
  ShieldCheck,
  CheckCircle2,
  Maximize2,
  Lock,
  Save,
  Database,
  Layers,
  FileText,
  Copy,
} from "./Icons";

type ActiveTab = "view" | "elements" | "data" | "console" | "network" | "storage";

export function BrowserInspector() {
  const show = useStore((s) => s.showBrowserInspector);
  const setShow = useStore((s) => s.setShowBrowserInspector);
  const session = useStore((s) => currentSession(s));
  const settings = useStore((s) => s.settings);
  const currentId = useStore((s) => s.currentId);
  const pushToast = useStore((s) => s.pushToast);

  const activeModel = resolveSessionActiveModel(settings, session, null);
  const sessionId = session?.id || currentId || "";

  const [activeTab, setActiveTab] = useState<ActiveTab>("view");
  const [debugState, setDebugState] = useState<BrowserDebugState | null>(null);
  const [screenshotUrl, setScreenshotUrl] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [interacting, setInteracting] = useState(false);
  const [navigating, setNavigating] = useState(false);
  const [manualUrl, setManualUrl] = useState("");
  const [typeInput, setTypeInput] = useState("");
  const [elementSearch, setElementSearch] = useState("");
  const [clickRipple, setClickRipple] = useState<{ x: number; y: number } | null>(null);

  // 登录态快照状态 (StorageState 网关)
  const [storageSummary, setStorageSummary] = useState<StorageStateSummary | null>(null);
  const [loadingSummary, setLoadingSummary] = useState(false);
  const [savingStorage, setSavingStorage] = useState(false);
  const [restoringStorage, setRestoringStorage] = useState(false);
  const [clearingStorage, setClearingStorage] = useState(false);

  const storeTab = useStore((s) => s.browserInspectorTab);
  useEffect(() => {
    if (storeTab) {
      if ((storeTab as string) === "auth") {
        setActiveTab("storage");
      } else {
        setActiveTab(storeTab as ActiveTab);
      }
    }
  }, [storeTab]);

  const [zoomMode, setZoomMode] = useState<"fit" | "original">("fit");
  const [scrolling, setScrolling] = useState(false);
  const [capturingFull, setCapturingFull] = useState(false);

  const viewportRef = useRef<HTMLDivElement>(null);
  const wheelThrottleTimer = useRef<number | null>(null);
  const wheelAccumRef = useRef<{ dx: number; dy: number }>({ dx: 0, dy: 0 });

  // 获取当前站点的 StorageState 快照摘要
  const fetchStorageSummary = async () => {
    if (!sessionId) return;
    setLoadingSummary(true);
    try {
      const summary = await ipc.getBrowserStorageStateSummary(sessionId);
      setStorageSummary(summary);
    } catch {
      setStorageSummary(null);
    } finally {
      setLoadingSummary(false);
    }
  };

  // 加载状态
  const fetchState = async (silently = false) => {
    if (!sessionId) return;
    if (!silently) setLoading(true);
    try {
      const state = await ipc.getBrowserDebugState(sessionId);
      setDebugState(state);
      // 同时获取截图
      try {
        const shot = await ipc.browserCaptureScreenshot(sessionId);
        const dataUrl = shot.startsWith("data:") ? shot : `data:image/jpeg;base64,${shot}`;
        setScreenshotUrl(dataUrl);
      } catch {
        // ignore screenshot error if not ready
      }
    } catch (e: any) {
      if (!silently) {
        setDebugState(null);
        setScreenshotUrl(null);
      }
    } finally {
      if (!silently) setLoading(false);
    }
  };

  useEffect(() => {
    if (!show || !sessionId) return;
    void fetchState();
    void fetchStorageSummary();

    // 弹窗开启时，每 2.5 秒静默轮询一次最新页面快照与状态
    const timer = setInterval(() => {
      void fetchState(true);
    }, 2500);

    return () => clearInterval(timer);
  }, [show, sessionId]);

  // 固化全量登录态 (Storage State: Cookies + LocalStorage + SessionStorage)
  const handleSaveStorageState = async () => {
    if (!sessionId || savingStorage) return;
    setSavingStorage(true);
    try {
      const msg = await ipc.saveBrowserStorageState(sessionId);
      pushToast(msg || "已成功固化当前登录态快照（Cookies + Storage）", "success");
      void fetchStorageSummary();
      void fetchState(true);
    } catch (err: any) {
      pushToast(`固化登录态失败: ${err}`, "error");
    } finally {
      setSavingStorage(false);
    }
  };

  // 恢复并注入已固化的登录态快照
  const handleRestoreStorageState = async () => {
    if (!sessionId || restoringStorage) return;
    setRestoringStorage(true);
    try {
      const ok = await ipc.restoreBrowserStorageState(sessionId);
      if (ok) {
        pushToast("已成功注入登录态快照，页面已刷新激活！", "success");
        setTimeout(() => {
          void fetchState(true);
        }, 1200);
        void fetchStorageSummary();
        void fetchState(true);
      } else {
        pushToast("未找到对应站点的登录态快照文件", "warning");
      }
    } catch (err: any) {
      pushToast(`载入登录态失败: ${err}`, "error");
    } finally {
      setRestoringStorage(false);
    }
  };

  // 清除当前站点的登录态快照
  const handleClearStorageState = async () => {
    if (!sessionId || clearingStorage) return;
    setClearingStorage(true);
    try {
      const ok = await ipc.clearBrowserStorageState(sessionId);
      if (ok) {
        pushToast("已成功彻底清除快照文件与浏览器内存凭据，页面已重载！", "info");
      } else {
        pushToast("当前站点已无残留快照", "info");
      }
      void fetchStorageSummary();
      void fetchState(true);
    } catch (err: any) {
      pushToast(`清除快照失败: ${err}`, "error");
    } finally {
      setClearingStorage(false);
    }
  };

  // 手动导航至 URL
  const handleManualNavigate = async (targetUrl?: string) => {
    const raw = (targetUrl || manualUrl).trim();
    if (!raw || !sessionId || navigating) return;
    const u = raw.startsWith("http://") || raw.startsWith("https://") ? raw : `http://${raw}`;
    setNavigating(true);
    try {
      pushToast(`正在启动受控浏览器并导航至: ${u}`, "info");
      const state = await ipc.browserNavigateUrl(sessionId, u);
      setDebugState(state);
      setManualUrl("");
      try {
        const shot = await ipc.browserCaptureScreenshot(sessionId);
        const dataUrl = shot.startsWith("data:") ? shot : `data:image/jpeg;base64,${shot}`;
        setScreenshotUrl(dataUrl);
      } catch {}
      pushToast("网页调试会话已建立", "success");
    } catch (err: any) {
      pushToast(`打开网页失败: ${err}`, "error");
    } finally {
      setNavigating(false);
    }
  };

  // 人工接管：在屏幕上点击
  const handleViewportClick = async (e: React.MouseEvent<HTMLImageElement>) => {
    if (!sessionId || interacting) return;

    const img = e.currentTarget;
    const rect = img.getBoundingClientRect();

    // 网页截图实际尺寸与渲染尺寸缩放比例 (基于标准 1440x960 CSS 视口容错兜底)
    const natWidth = img.naturalWidth || 1440;
    const natHeight = img.naturalHeight || 960;
    const scaleX = natWidth / rect.width;
    const scaleY = natHeight / rect.height;

    const clickX = Math.max(0, Math.min(1440, Math.round((e.clientX - rect.left) * scaleX)));
    const clickY = Math.max(0, Math.min(960, Math.round((e.clientY - rect.top) * scaleY)));

    // 显示点击涟漪动画
    setClickRipple({ x: e.clientX - rect.left, y: e.clientY - rect.top });
    setTimeout(() => setClickRipple(null), 500);

    setInteracting(true);
    try {
      await ipc.browserUserInteract(sessionId, "click", clickX, clickY);
      // 快速触发快照刷新以即时呈现光标聚焦
      setTimeout(() => void fetchState(true), 250);
    } catch (err: any) {
      pushToast(`点击交互失败: ${err}`, "error");
    } finally {
      setInteracting(false);
    }
  };

  // 人工接管：文本输入
  const handleSendText = async () => {
    if (!sessionId || !typeInput || interacting) return;
    setInteracting(true);
    try {
      await ipc.browserUserInteract(sessionId, "type", undefined, undefined, typeInput);
      pushToast(`已向焦点元素输入: "${typeInput}"`, "info");
      setTypeInput("");
      setTimeout(() => void fetchState(true), 500);
    } catch (err: any) {
      pushToast(`输入失败: ${err}`, "error");
    } finally {
      setInteracting(false);
    }
  };

  // 关闭浏览器实例
  const handleCloseSession = async () => {
    if (!sessionId) return;
    try {
      await ipc.closeBrowserSession(sessionId);
      pushToast("已关闭浏览器调试实例", "success");
      setDebugState(null);
      setScreenshotUrl(null);
    } catch (err: any) {
      pushToast(`关闭浏览器失败: ${err}`, "error");
    }
  };

  // 唤起独立 Chromium DevTools 调试窗口
  const handleOpenDevtools = async () => {
    if (!sessionId) return;
    try {
      await ipc.browserOpenDevtoolsWindow(sessionId);
      pushToast("已唤起官方 Chromium DevTools 独立调试窗口（享受原生 120fps 渲染与滚动条拖拽）", "info");
    } catch (err: any) {
      pushToast(`唤起独立窗口失败: ${err}`, "error");
    }
  };

  // 滚轮事件透传
  const handleViewportWheel = (e: React.WheelEvent) => {
    if (!sessionId) return;
    wheelAccumRef.current.dx += e.deltaX;
    wheelAccumRef.current.dy += e.deltaY;

    if (wheelThrottleTimer.current === null) {
      wheelThrottleTimer.current = window.setTimeout(async () => {
        const { dx, dy } = wheelAccumRef.current;
        wheelAccumRef.current = { dx: 0, dy: 0 };
        wheelThrottleTimer.current = null;
        try {
          await ipc.browserUserWheel(sessionId, 720, 480, dx, dy);
          setTimeout(() => void fetchState(true), 250);
        } catch {}
      }, 60);
    }
  };

  // 快捷视口滚动
  const handleScrollAction = async (direction: "down" | "up" | "top" | "bottom") => {
    if (!sessionId || scrolling) return;
    setScrolling(true);
    try {
      const state = await ipc.browserScrollViewport(sessionId, direction);
      setDebugState(state);
      setTimeout(() => void fetchState(true), 250);
    } catch (err: any) {
      pushToast(`滚动失败: ${err}`, "error");
    } finally {
      setScrolling(false);
    }
  };

  // 截取完整长图
  const handleCaptureFullPage = async () => {
    if (!sessionId || capturingFull) return;
    setCapturingFull(true);
    try {
      pushToast("正在截取全页面长图 (Beyond Viewport)...", "info");
      const shot = await ipc.browserCaptureScreenshot(sessionId, true);
      const dataUrl = shot.startsWith("data:") ? shot : `data:image/jpeg;base64,${shot}`;
      setScreenshotUrl(dataUrl);
      pushToast("全页面长图已捕获完成", "success");
    } catch (err: any) {
      pushToast(`全页面截屏失败: ${err}`, "error");
    } finally {
      setCapturingFull(false);
    }
  };

  if (!show) return null;

  const filteredElements = (debugState?.elements || []).filter((el) => {
    if (!elementSearch) return true;
    const q = elementSearch.toLowerCase();
    return (
      el.ref_id.toLowerCase().includes(q) ||
      el.tag.toLowerCase().includes(q) ||
      (el.text && el.text.toLowerCase().includes(q)) ||
      (el.placeholder && el.placeholder.toLowerCase().includes(q)) ||
      (el.value && el.value.toLowerCase().includes(q))
    );
  });

  return (
    <div className="fixed inset-0 z-50 bg-black/60 backdrop-blur-xs flex items-center justify-center p-3 sm:p-6 animate-in fade-in duration-150">
      <div className="bg-panel border border-edge rounded-2xl shadow-2xl flex flex-col w-full max-w-6xl h-[90vh] overflow-hidden text-ink">
        {/* 顶栏 Header */}
        <div className="h-13 px-5 border-b border-edge flex items-center justify-between shrink-0 bg-panel2/50">
          <div className="flex items-center gap-3">
            <div className="w-8 h-8 rounded-lg bg-accent/15 border border-accent/30 flex items-center justify-center text-accent">
              <Globe size={18} />
            </div>
            <div>
              <div className="flex items-center gap-2">
                <span className="font-semibold text-[14px]">原生 Chromium 网页调试工作台</span>
                <span className="text-[11px] font-mono px-2 py-0.5 rounded-full bg-blue-500/15 text-blue-400 border border-blue-500/30">
                  CDP 驱动 · 人工接管
                </span>
              </div>
              <div className="text-[11px] text-inkdim flex items-center gap-2 mt-0.5 flex-wrap">
                <span className="flex items-center gap-1">
                  <span>会话 ID:</span>
                  <span className="font-mono text-ink">{sessionId.slice(0, 16)}...</span>
                </span>
                <span className="text-edge">·</span>
                <span className="flex items-center gap-1.5 px-2 py-0.5 rounded-md bg-panel3 border border-edge/60 text-ink text-[11px]">
                  <Sparkles size={11} className="text-purple-400" />
                  <span className="text-inkdim">提炼模型:</span>
                  <span className="font-semibold text-purple-300">
                    {activeModel ? `${activeModel.provider.name} · ${activeModel.model}` : "未配置模型"}
                  </span>
                </span>
              </div>
            </div>
          </div>

          <div className="flex items-center gap-2">
            {debugState && (
              <button
                onClick={() => void handleOpenDevtools()}
                className="px-2.5 py-1.5 rounded-lg border border-accent/40 bg-accent/10 hover:bg-accent/20 text-accent text-[12px] flex items-center gap-1.5 transition-colors cursor-pointer"
                title="在系统独立浏览器中打开官方 Chromium DevTools 调试面板（享受 100% 原生 120fps 渲染、拖拽真实滚动条、F12 控制台审查）"
              >
                <ExternalLink size={13} />
                <span>独立调试窗口</span>
              </button>
            )}

            <button
              onClick={() => void fetchState()}
              disabled={loading}
              className="px-2.5 py-1.5 rounded-lg border border-edge hover:bg-panel3 text-[12px] text-inkdim hover:text-ink flex items-center gap-1.5 transition-colors cursor-pointer"
              title="刷新调试状态与屏幕快照"
            >
              <RefreshCw size={13} className={loading ? "animate-spin text-accent" : ""} />
              <span>刷新</span>
            </button>

            {debugState && (
              <button
                onClick={() => void handleCloseSession()}
                className="px-2.5 py-1.5 rounded-lg border border-red-500/30 text-red-400 hover:bg-red-500/10 text-[12px] flex items-center gap-1.5 transition-colors cursor-pointer"
                title="关闭当前浏览器的后台独立进程"
              >
                <Trash2 size={13} />
                <span>关闭浏览器</span>
              </button>
            )}

            <button
              onClick={() => setShow(false)}
              className="p-1.5 rounded-lg text-inkdim hover:text-ink hover:bg-panel3 transition-colors cursor-pointer ml-1"
            >
              <X size={18} />
            </button>
          </div>
        </div>

        {/* 地址与状态指示条 */}
        {debugState ? (
          <div className="px-5 py-2.5 bg-panel3/30 border-b border-edge/60 flex items-center justify-between gap-4 text-[12px]">
            <div className="flex items-center gap-2.5 flex-1 min-w-0">
              <span className="text-[11px] font-semibold text-emerald-400 bg-emerald-500/15 px-2 py-0.5 rounded border border-emerald-500/30 shrink-0 flex items-center gap-1">
                <span className="w-1.5 h-1.5 rounded-full bg-emerald-400 animate-pulse" />
                已连接
              </span>
              <span className="font-medium text-ink truncate shrink-0 max-w-[170px]" title={debugState.title}>
                {debugState.title || "未命名网页"}
              </span>
              <form
                onSubmit={(e) => {
                  e.preventDefault();
                  void handleManualNavigate();
                }}
                className="flex items-center gap-1.5 flex-1 min-w-0 bg-panel px-2.5 py-1 rounded-md border border-edge/60 font-mono text-[11.5px] focus-within:border-accent"
              >
                <Globe size={12} className="shrink-0 text-accent" />
                <input
                  type="text"
                  value={manualUrl || debugState.url}
                  onChange={(e) => setManualUrl(e.target.value)}
                  placeholder="输入新地址回车跳转..."
                  className="bg-transparent border-none text-[11.5px] text-ink outline-none flex-1 truncate font-mono"
                />
                {manualUrl && manualUrl !== debugState.url && (
                  <button
                    type="submit"
                    disabled={navigating}
                    className="px-2 py-0.5 rounded text-[10.5px] bg-accent text-white hover:bg-accent/90 shrink-0 cursor-pointer"
                  >
                    {navigating ? "跳转中…" : "跳转"}
                  </button>
                )}
              </form>
            </div>

            {/* 标签页切换 */}
            <div className="flex items-center gap-1 bg-panel2 p-0.5 rounded-lg border border-edge/60 shrink-0">
              <button
                onClick={() => setActiveTab("view")}
                className={`px-3 py-1 rounded text-[11.5px] font-medium transition-colors flex items-center gap-1.5 cursor-pointer ${
                  activeTab === "view"
                    ? "bg-accent/20 text-accent shadow-xs"
                    : "text-inkdim hover:text-ink hover:bg-panel3"
                }`}
              >
                <Monitor size={12} />
                <span>视窗与接管</span>
              </button>
              <button
                onClick={() => setActiveTab("elements")}
                className={`px-3 py-1 rounded text-[11.5px] font-medium transition-colors flex items-center gap-1.5 cursor-pointer ${
                  activeTab === "elements"
                    ? "bg-accent/20 text-accent shadow-xs"
                    : "text-inkdim hover:text-ink hover:bg-panel3"
                }`}
              >
                <MousePointer size={12} />
                <span>交互元素 (@e)</span>
                <span className="text-[10px] px-1 rounded-full bg-panel3 text-inkdim">
                  {debugState.elements.length}
                </span>
              </button>
              <button
                onClick={() => setActiveTab("data")}
                className={`px-3 py-1 rounded text-[11.5px] font-medium transition-colors flex items-center gap-1.5 cursor-pointer ${
                  activeTab === "data"
                    ? "bg-accent/20 text-accent shadow-xs"
                    : "text-inkdim hover:text-ink hover:bg-panel3"
                }`}
              >
                <List size={12} />
                <span>数据与表格</span>
                {((debugState.tables?.length || 0) + (debugState.notices?.length || 0) > 0) && (
                  <span className="text-[10px] px-1 rounded-full bg-emerald-500/20 text-emerald-400 font-mono">
                    {(debugState.tables?.length || 0) + (debugState.notices?.length || 0)}
                  </span>
                )}
              </button>
              <button
                onClick={() => setActiveTab("console")}
                className={`px-3 py-1 rounded text-[11.5px] font-medium transition-colors flex items-center gap-1.5 cursor-pointer ${
                  activeTab === "console"
                    ? "bg-accent/20 text-accent shadow-xs"
                    : "text-inkdim hover:text-ink hover:bg-panel3"
                }`}
              >
                <Bug size={12} />
                <span>控制台异常</span>
                {debugState.console_errors.length > 0 && (
                  <span className="text-[10px] px-1 rounded-full bg-red-500/20 text-red-400 font-mono">
                    {debugState.console_errors.length}
                  </span>
                )}
              </button>
              <button
                onClick={() => setActiveTab("network")}
                className={`px-3 py-1 rounded text-[11.5px] font-medium transition-colors flex items-center gap-1.5 cursor-pointer ${
                  activeTab === "network"
                    ? "bg-accent/20 text-accent shadow-xs"
                    : "text-inkdim hover:text-ink hover:bg-panel3"
                }`}
              >
                <Activity size={12} />
                <span>网络请求异常</span>
                {debugState.network_errors.length > 0 && (
                  <span className="text-[10px] px-1 rounded-full bg-amber-500/20 text-amber-400 font-mono">
                    {debugState.network_errors.length}
                  </span>
                )}
              </button>
              <button
                onClick={() => {
                  setActiveTab("storage");
                  void fetchStorageSummary();
                }}
                className={`px-3 py-1 rounded text-[11.5px] font-medium transition-colors flex items-center gap-1.5 cursor-pointer ${
                  activeTab === "storage"
                    ? "bg-accent/20 text-accent shadow-xs"
                    : "text-inkdim hover:text-ink hover:bg-panel3"
                }`}
              >
                <Database size={12} />
                <span>登录态 (StorageState)</span>
                {storageSummary?.exists && (
                  <span className="w-1.5 h-1.5 rounded-full bg-emerald-400" />
                )}
              </button>
            </div>
          </div>
        ) : null}

        {/* 内容展示区 */}
        <div className="flex-1 min-h-0 overflow-y-auto p-5">
          {!debugState ? (
            <div className="h-full flex flex-col items-center justify-center text-center p-8">
              <div className="w-14 h-14 rounded-2xl bg-panel3 border border-edge flex items-center justify-center text-accent mb-4 shadow-sm">
                <Globe size={28} />
              </div>
              <div className="text-[16px] font-semibold text-ink mb-1.5">当前会话暂无活动的浏览器调试实例</div>
              <div className="text-[12.5px] text-inkdim max-w-[520px] leading-relaxed mb-6">
                当 Agent 在对话中执行 <code className="text-accent bg-panel3 px-1.5 py-0.5 rounded font-mono text-[11.5px]">browser_navigate</code> 工具自测时会自动调起 Chromium；你也可以在下方输入本地或外部网址直接手动调起调试：
              </div>

              {/* 手动打开 URL 输入框 */}
              <div className="w-full max-w-lg flex flex-col gap-3 mb-6">
                <form
                  onSubmit={(e) => {
                    e.preventDefault();
                    void handleManualNavigate();
                  }}
                  className="flex items-center gap-2 bg-panel2 border border-edge/80 focus-within:border-accent p-1.5 rounded-xl shadow-inner transition-colors"
                >
                  <Globe size={15} className="text-inkdim ml-2 shrink-0" />
                  <input
                    type="text"
                    value={manualUrl}
                    onChange={(e) => setManualUrl(e.target.value)}
                    placeholder="输入要调试的 URL，如 127.0.0.1:8080 或 localhost:5173"
                    className="flex-1 bg-transparent border-none text-[13px] text-ink outline-none px-2 font-mono"
                  />
                  <button
                    type="submit"
                    disabled={navigating || !manualUrl.trim()}
                    className="px-4 py-2 rounded-lg bg-accent text-white text-[12.5px] font-medium hover:bg-accent/90 disabled:opacity-50 transition-colors flex items-center gap-1.5 shrink-0 cursor-pointer shadow-xs"
                  >
                    {navigating ? (
                      <>
                        <Loader2 size={13} className="animate-spin" />
                        <span>启动中…</span>
                      </>
                    ) : (
                      <>
                        <ExternalLink size={13} />
                        <span>打开网页</span>
                      </>
                    )}
                  </button>
                </form>

                {/* 常用本地快捷预设 */}
                <div className="flex items-center justify-center gap-2 text-[11px] text-inkdim">
                  <span>快捷填入:</span>
                  {["http://localhost:5173", "http://127.0.0.1:8080", "http://localhost:3000"].map((preset) => (
                    <button
                      key={preset}
                      type="button"
                      onClick={() => void handleManualNavigate(preset)}
                      className="px-2 py-0.5 rounded-md bg-panel3 hover:bg-panel hover:text-ink font-mono transition-colors border border-edge/60 cursor-pointer"
                    >
                      {preset.replace("http://", "")}
                    </button>
                  ))}
                </div>
              </div>

              <div className="flex items-center gap-2">
                <button
                  onClick={() => void fetchState()}
                  disabled={loading}
                  className="px-3.5 py-1.5 rounded-lg border border-edge hover:bg-panel3 text-inkdim hover:text-ink text-[12px] transition-colors flex items-center gap-1.5 cursor-pointer"
                >
                  <RefreshCw size={13} className={loading ? "animate-spin text-accent" : ""} />
                  <span>重新检测活跃实例</span>
                </button>
              </div>
            </div>
          ) : (
            <>
              {/* Tab 1: 实时视窗与人工接管 */}
              {activeTab === "view" && (
                <div className="flex flex-col h-full gap-4">
                  {/* 页面语义未登录态提示与快照快捷操作 */}
                  {debugState?.page_state && debugState.page_state.type === "Unauthenticated" && (
                    <div className="p-3 rounded-xl bg-amber-500/10 border border-amber-500/30 flex items-center justify-between gap-3 text-[12px] animate-in fade-in duration-150">
                      <div className="flex items-center gap-2 text-amber-400 font-medium">
                        <AlertTriangle size={15} className="shrink-0" />
                        <span>
                          页面处于未登录态（检测到重定向或登录表单，理由: <code className="font-mono bg-panel px-1 rounded">{debugState.page_state.payload.reason}</code>）
                        </span>
                      </div>
                      <div className="flex items-center gap-2 shrink-0">
                        <button
                          onClick={() => void handleRestoreStorageState()}
                          disabled={restoringStorage}
                          className="px-3 py-1.5 rounded-lg bg-blue-600 hover:bg-blue-500 text-white font-medium text-[11.5px] flex items-center gap-1.5 cursor-pointer shadow-xs transition-colors disabled:opacity-50"
                          title="尝试从本地快照注入 Cookies/Storage 凭据并重载"
                        >
                          {restoringStorage ? <Loader2 size={13} className="animate-spin" /> : <RefreshCw size={13} />}
                          <span>注入登录态快照</span>
                        </button>
                        <button
                          onClick={() => {
                            setActiveTab("storage");
                            void fetchStorageSummary();
                          }}
                          className="px-2.5 py-1.5 rounded-lg border border-edge bg-panel hover:bg-panel3 text-ink text-[11.5px] transition-colors cursor-pointer"
                        >
                          登录态管理
                        </button>
                      </div>
                    </div>
                  )}

                  {debugState?.page_state && debugState.page_state.type === "ActionBlocked" && (
                    <div className="p-3 rounded-xl bg-purple-500/10 border border-purple-500/30 flex items-center justify-between gap-3 text-[12px]">
                      <div className="flex items-center gap-2 text-purple-300 font-medium">
                        <AlertCircle size={15} className="shrink-0" />
                        <span>
                          前置阻断弹窗: <strong className="text-white font-semibold">{debugState.page_state.payload.title}</strong> ({debugState.page_state.payload.description})
                        </span>
                      </div>
                      <div className="text-[11.5px] text-purple-300/80">
                        可在下方直接点击选定目标项或通过契约通用后置动作序列自动处理
                      </div>
                    </div>
                  )}

                  {debugState?.page_state && debugState.page_state.type === "Error" && (
                    <div className="p-3 rounded-xl bg-red-500/10 border border-red-500/30 flex items-center justify-between gap-3 text-[12px]">
                      <div className="flex items-center gap-2 text-red-400 font-medium">
                        <AlertCircle size={15} className="shrink-0" />
                        <span>页面不可逆异常: {debugState.page_state.payload.message} (HTTP {debugState.page_state.payload.code ?? "N/A"})</span>
                      </div>
                    </div>
                  )}

                  {/* 人工接管输入提示条 */}
                  <div className="flex items-center justify-between gap-3 bg-panel2 p-3 rounded-xl border border-edge">
                    <div className="flex items-center gap-2 text-[12px] text-inkdim">
                      <MousePointer size={14} className="text-accent shrink-0" />
                      <span>
                        <strong className="text-ink font-medium">人工接管模式：</strong>
                        直接点击下方网页快照任意位置即可派发原生鼠标点击。在右侧输入文本可直接打字至聚焦输入框。
                      </span>
                    </div>

                    <div className="flex items-center gap-2">
                      <div className="flex items-center gap-1.5 bg-panel border border-edge rounded-lg px-2.5 py-1">
                        <Terminal size={12} className="text-inkdim shrink-0" />
                        <input
                          type="text"
                          value={typeInput}
                          onChange={(e) => setTypeInput(e.target.value)}
                          onKeyDown={(e) => {
                            if (e.key === "Enter") void handleSendText();
                          }}
                          placeholder="输入文本打字..."
                          className="bg-transparent border-none text-[12px] text-ink outline-none w-44"
                        />
                      </div>
                      <button
                        onClick={() => void handleSendText()}
                        disabled={!typeInput || interacting}
                        className="px-3 py-1.5 rounded-lg bg-accent text-white text-[12px] font-medium hover:bg-accent/90 disabled:opacity-50 transition-colors flex items-center gap-1 cursor-pointer"
                      >
                        <Send size={12} />
                        <span>输入</span>
                      </button>
                    </div>
                  </div>

                  {/* 视口与长页面滚动控制条 */}
                  <div className="flex items-center justify-between gap-3 bg-panel2/60 px-3.5 py-2 rounded-xl border border-edge text-[11.5px]">
                    <div className="flex items-center gap-2 text-inkdim flex-wrap min-w-0">
                      <span className="font-semibold text-ink flex items-center gap-1 shrink-0">
                        <Monitor size={13} className="text-accent" />
                        视口概览:
                      </span>
                      {debugState.viewport ? (
                        <span className="font-mono bg-panel px-2 py-0.5 rounded border border-edge/60 text-ink shrink-0">
                          Y: {Math.round(debugState.viewport.scroll_y)}px ~ {Math.round(debugState.viewport.scroll_y + debugState.viewport.viewport_height)}px / 总高: {Math.round(debugState.viewport.total_height)}px (第 {debugState.viewport.current_page}/{debugState.viewport.total_pages} 屏)
                        </span>
                      ) : (
                        <span className="font-mono text-ink shrink-0">1440 × 960 (2x Retina)</span>
                      )}

                      {debugState.viewport?.has_more_down && (
                        <span className="text-amber-400 bg-amber-500/10 border border-amber-500/30 px-2 py-0.5 rounded flex items-center gap-1 shrink-0">
                          <AlertTriangle size={11} />
                          下方仍有未展示内容 (折叠约 {debugState.hidden_elements_count || 0} 个元素)
                        </span>
                      )}
                    </div>

                    <div className="flex items-center gap-1.5 shrink-0">
                      <div className="flex items-center bg-panel border border-edge rounded-lg p-0.5">
                        <button
                          onClick={() => void handleScrollAction("top")}
                          disabled={scrolling}
                          className="px-2 py-1 rounded text-[11px] text-inkdim hover:text-ink hover:bg-panel3 transition-colors cursor-pointer"
                          title="滚动回到页面最顶部"
                        >
                          顶部
                        </button>
                        <button
                          onClick={() => void handleScrollAction("up")}
                          disabled={scrolling}
                          className="px-2 py-1 rounded text-[11px] text-inkdim hover:text-ink hover:bg-panel3 transition-colors cursor-pointer"
                          title="向上翻动一屏"
                        >
                          上翻
                        </button>
                        <button
                          onClick={() => void handleScrollAction("down")}
                          disabled={scrolling}
                          className="px-2 py-1 rounded text-[11px] text-inkdim hover:text-ink hover:bg-panel3 transition-colors cursor-pointer flex items-center gap-1"
                          title="向下翻动一屏"
                        >
                          <span>下翻</span>
                          {debugState.viewport?.has_more_down && (
                            <span className="w-1.5 h-1.5 rounded-full bg-accent animate-pulse" />
                          )}
                        </button>
                        <button
                          onClick={() => void handleScrollAction("bottom")}
                          disabled={scrolling}
                          className="px-2 py-1 rounded text-[11px] text-inkdim hover:text-ink hover:bg-panel3 transition-colors cursor-pointer"
                          title="直达页面最底部"
                        >
                          底部
                        </button>
                      </div>

                      <button
                        onClick={() => void handleCaptureFullPage()}
                        disabled={capturingFull}
                        className="px-2.5 py-1 rounded-lg border border-edge hover:bg-panel3 text-[11px] text-inkdim hover:text-ink flex items-center gap-1 transition-colors cursor-pointer"
                        title="捕获整页长图 (Beyond Viewport)"
                      >
                        <Camera size={12} className={capturingFull ? "animate-spin text-accent" : ""} />
                        <span>{capturingFull ? "截长图中…" : "全页面长截图"}</span>
                      </button>

                      <button
                        onClick={() => setZoomMode(zoomMode === "fit" ? "original" : "fit")}
                        className="px-2.5 py-1 rounded-lg border border-edge hover:bg-panel3 text-[11px] text-inkdim hover:text-ink flex items-center gap-1 transition-colors cursor-pointer"
                        title={zoomMode === "fit" ? "切换至 100% 原始物理分辨率 (可平移查看细节)" : "切换至自适应视窗 (Fit)"}
                      >
                        <Maximize2 size={11} />
                        <span>{zoomMode === "fit" ? "1:1 像素" : "适应窗口"}</span>
                      </button>
                    </div>
                  </div>

                  {/* 屏幕快照与交互视窗 */}
                  <div
                    ref={viewportRef}
                    onWheel={handleViewportWheel}
                    className="flex-1 min-h-[300px] bg-black/40 border border-edge rounded-xl overflow-auto relative flex items-center justify-center p-2"
                  >
                    {screenshotUrl ? (
                      <div className="relative inline-block shadow-xl rounded overflow-hidden select-none cursor-crosshair">
                        <img
                          src={screenshotUrl}
                          alt="Browser Viewport"
                          onClick={handleViewportClick}
                          className={
                            zoomMode === "fit"
                              ? "max-w-full max-h-[58vh] object-contain block bg-white"
                              : "max-w-none w-[1440px] block bg-white"
                          }
                        />
                        {clickRipple && (
                          <span
                            style={{ left: clickRipple.x - 12, top: clickRipple.y - 12 }}
                            className="absolute w-6 h-6 rounded-full border-2 border-accent bg-accent/30 animate-ping pointer-events-none"
                          />
                        )}
                      </div>
                    ) : (
                      <div className="text-inkdim text-[13px] flex items-center gap-2">
                        <Loader2 size={16} className="animate-spin text-accent" />
                        <span>正在捕获浏览器快照…</span>
                        <button
                          onClick={() => void fetchState()}
                          className="ml-2 text-accent hover:underline text-[12px] cursor-pointer"
                        >
                          刷新快照
                        </button>
                      </div>
                    )}
                  </div>
                </div>
              )}

              {/* Tab 2: 交互元素 (@e1... Ref Tree) */}
              {activeTab === "elements" && (
                <div className="flex flex-col h-full gap-3">
                  <div className="flex items-center justify-between gap-3">
                    <div className="text-[12px] text-inkdim">
                      系统已将 DOM 压缩提炼为 <strong className="text-ink font-medium">Ref 标识符</strong>（如 <code className="text-accent bg-panel3 px-1 rounded font-mono">@e1</code>），Agent 可基于此进行精准、低 Token 消耗的操作。
                    </div>
                    <div className="flex items-center gap-2 bg-panel2 border border-edge rounded-lg px-2.5 py-1 w-64">
                      <Search size={13} className="text-inkdim" />
                      <input
                        type="text"
                        value={elementSearch}
                        onChange={(e) => setElementSearch(e.target.value)}
                        placeholder="搜索 Tag、Ref 或文本..."
                        className="bg-transparent border-none text-[12px] text-ink outline-none w-full"
                      />
                    </div>
                  </div>

                  <div className="flex-1 min-h-0 overflow-y-auto border border-edge rounded-xl divide-y divide-edge/60 bg-panel2/30">
                    {filteredElements.length === 0 ? (
                      <div className="text-inkdim text-[12px] p-6 text-center">暂无匹配的可交互元素</div>
                    ) : (
                      filteredElements.map((el) => (
                        <div
                          key={el.ref_id}
                          className="p-3 hover:bg-panel3/50 transition-colors flex items-start justify-between gap-3 text-[12px]"
                        >
                          <div className="flex items-start gap-2.5 min-w-0 flex-1">
                            <span className="font-mono font-semibold px-2 py-0.5 rounded bg-accent/15 text-accent border border-accent/30 text-[11px] shrink-0">
                              {el.ref_id}
                            </span>
                            <span className="font-mono text-ink font-medium uppercase px-1.5 py-0.5 rounded bg-panel3 border border-edge text-[10.5px] shrink-0">
                              {el.tag}
                            </span>
                            <div className="flex flex-col gap-0.5 min-w-0 flex-1">
                              {el.text && (
                                <div className="text-ink font-medium truncate" title={el.text}>
                                  文本: {el.text}
                                </div>
                              )}
                              {el.placeholder && (
                                <div className="text-inkdim truncate" title={el.placeholder}>
                                  占位提示: {el.placeholder}
                                </div>
                              )}
                              {el.value && (
                                <div className="text-inkdim font-mono text-[11px] truncate" title={el.value}>
                                  当前取值: {el.value}
                                </div>
                              )}
                              {el.href && (
                                <div className="text-blue-400 font-mono text-[11px] truncate flex items-center gap-1" title={el.href}>
                                  <ExternalLink size={10} className="shrink-0" />
                                  <span>{el.href}</span>
                                </div>
                              )}
                            </div>
                          </div>

                          <div className="flex items-center gap-2 shrink-0">
                            <button
                              onClick={async () => {
                                try {
                                  await ipc.browserUserInteract(sessionId, "click", undefined, undefined, el.ref_id);
                                  pushToast(`已人工触发点击 ${el.ref_id}`, "success");
                                  setTimeout(() => void fetchState(true), 500);
                                } catch (e) {
                                  pushToast(`操作失败: ${e}`, "error");
                                }
                              }}
                              className="px-2.5 py-1 rounded bg-panel hover:bg-accent hover:text-white border border-edge text-[11px] font-medium transition-colors cursor-pointer"
                            >
                              触发点击
                            </button>
                          </div>
                        </div>
                      ))
                    )}
                  </div>
                </div>
              )}

              {/* Tab: 数据与表格概览 */}
              {activeTab === "data" && (
                <div className="flex flex-col h-full gap-4 overflow-y-auto">
                  {/* 页面通知与提示 */}
                  {debugState.notices && debugState.notices.length > 0 && (
                    <div className="flex flex-col gap-2">
                      <div className="text-[12px] font-medium text-ink flex items-center gap-1.5">
                        <AlertTriangle size={14} className="text-amber-400" />
                        <span>页面通知与空状态提示 (Alerts & Notices)</span>
                      </div>
                      <div className="flex flex-col gap-1.5">
                        {debugState.notices.map((n, idx) => (
                          <div
                            key={idx}
                            className="p-2.5 rounded-lg border border-amber-500/30 bg-amber-500/10 text-amber-200 text-[12px] flex items-center gap-2"
                          >
                            <span>ℹ️</span>
                            <span>{n}</span>
                          </div>
                        ))}
                      </div>
                    </div>
                  )}

                  {/* 业务数据表格 */}
                  <div className="flex flex-col gap-2 flex-1 min-h-0">
                    <div className="text-[12px] font-medium text-ink flex items-center gap-1.5">
                      <List size={14} className="text-accent" />
                      <span>检测到的业务数据表格 (Data Tables)</span>
                    </div>

                    {!debugState.tables || debugState.tables.length === 0 ? (
                      <div className="p-8 border border-dashed border-edge rounded-xl text-center text-inkdim text-[12px]">
                        当前视口或页面内未检测到标准表格组件 (table / el-table / vxe-table)
                      </div>
                    ) : (
                      <div className="flex flex-col gap-4">
                        {debugState.tables.map((tbl, idx) => (
                          <div key={idx} className="border border-edge rounded-xl p-3 bg-panel2/50 overflow-x-auto">
                            <div className="text-[11px] font-mono text-inkdim mb-2 font-medium">
                              表格 #{idx + 1}
                            </div>
                            <pre className="font-mono text-[12px] text-ink whitespace-pre-wrap leading-relaxed">
                              {tbl}
                            </pre>
                          </div>
                        ))}
                      </div>
                    )}
                  </div>
                </div>
              )}

              {/* Tab 3: 控制台异常 */}
              {activeTab === "console" && (
                <div className="flex flex-col h-full gap-3">
                  <div className="text-[12px] text-inkdim flex items-center gap-1.5">
                    <Bug size={14} className="text-red-400" />
                    <span>捕获当前页面的 JavaScript 运行时报错、未捕获 Promise Rejection 与 console.error：</span>
                  </div>

                  <div className="flex-1 min-h-0 overflow-y-auto border border-edge rounded-xl bg-black/30 p-3 font-mono text-[12px] divide-y divide-edge/40">
                    {debugState.console_errors.length === 0 ? (
                      <div className="text-emerald-400/80 p-4 text-center">✅ 控制台未捕获到任何 JavaScript 异常</div>
                    ) : (
                      debugState.console_errors.map((err, idx) => (
                        <div key={idx} className="py-2 text-red-400 leading-relaxed whitespace-pre-wrap break-all">
                          {err}
                        </div>
                      ))
                    )}
                  </div>
                </div>
              )}

              {/* Tab 4: 网络接口异常 */}
              {activeTab === "network" && (
                <div className="flex flex-col h-full gap-3">
                  <div className="text-[12px] text-inkdim flex items-center gap-1.5">
                    <Activity size={14} className="text-amber-400" />
                    <span>捕获当前页面 HTTP 状态码 ≥ 400（如 404/500/502）或失败的网络请求：</span>
                  </div>

                  <div className="flex-1 min-h-0 overflow-y-auto border border-edge rounded-xl bg-black/30 p-3 font-mono text-[12px] divide-y divide-edge/40">
                    {debugState.network_errors.length === 0 ? (
                      <div className="text-emerald-400/80 p-4 text-center">✅ 未捕获到任何接口网络请求错误</div>
                    ) : (
                      debugState.network_errors.map((netErr, idx) => (
                        <div key={idx} className="py-2 text-amber-400 leading-relaxed whitespace-pre-wrap break-all">
                          {netErr}
                        </div>
                      ))
                    )}
                  </div>
                </div>
              )}

              {/* Tab 5: 确定性登录态快照 (StorageState 网关) */}
              {activeTab === "storage" && (
                <div className="flex flex-col h-full gap-5 max-w-4xl mx-auto w-full overflow-y-auto pr-1">
                  {/* 核心卡片 1: 登录态快照状态与核心操作区 */}
                  <div className="p-5 rounded-2xl border border-blue-500/30 bg-blue-500/5 flex flex-col gap-4 shadow-sm">
                    <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-3 border-b border-blue-500/20 pb-3">
                      <div className="flex items-center gap-2.5 flex-wrap">
                        <Database size={19} className="text-blue-400 shrink-0" />
                        <span className="font-semibold text-[15px] text-ink">
                          确定性免登快照 (StorageState)
                        </span>
                        <span className="text-[10.5px] px-2 py-0.5 rounded-full font-mono bg-blue-500/20 text-blue-300 border border-blue-500/30">
                          工业级标准 · 对齐 Playwright
                        </span>
                        {storageSummary?.exists ? (
                          <span className="text-[10.5px] px-2.5 py-0.5 rounded-full bg-emerald-500/15 text-emerald-400 border border-emerald-500/30 font-medium flex items-center gap-1">
                            <CheckCircle2 size={11} />
                            <span>快照已就绪</span>
                          </span>
                        ) : (
                          <span className="text-[10.5px] px-2.5 py-0.5 rounded-full bg-amber-500/15 text-amber-400 border border-amber-500/30 font-medium">
                            ⚪ 未固化快照
                          </span>
                        )}
                      </div>

                      {/* 核心动作按钮组 */}
                      <div className="flex items-center gap-2 flex-wrap">
                        <button
                          type="button"
                          onClick={() => void handleSaveStorageState()}
                          disabled={savingStorage}
                          className="px-3.5 py-1.5 rounded-lg bg-blue-600 hover:bg-blue-500 text-white font-medium text-[12px] flex items-center gap-1.5 cursor-pointer shadow-xs transition-colors disabled:opacity-50"
                          title="抓取当前浏览器视窗中已登录的 Cookies、LocalStorage、SessionStorage 并独立持久化"
                        >
                          {savingStorage ? <Loader2 size={13} className="animate-spin" /> : <Save size={13} />}
                          <span>固化当前登录态快照</span>
                        </button>
                        <button
                          type="button"
                          onClick={() => void handleRestoreStorageState()}
                          disabled={restoringStorage}
                          className="px-3.5 py-1.5 rounded-lg border border-blue-500/30 hover:bg-blue-500/10 text-blue-400 font-medium text-[12px] flex items-center gap-1.5 cursor-pointer transition-colors disabled:opacity-50"
                          title="将已固化的 Cookies 与 Storage 注入当前会话并重载页面"
                        >
                          {restoringStorage ? <Loader2 size={13} className="animate-spin" /> : <RefreshCw size={13} />}
                          <span>注入并恢复登录态</span>
                        </button>
                        {storageSummary?.exists && (
                          <button
                            type="button"
                            onClick={() => void handleClearStorageState()}
                            disabled={clearingStorage}
                            className="px-2.5 py-1.5 rounded-lg border border-red-500/30 hover:bg-red-500/10 text-red-400 text-[12px] flex items-center gap-1 cursor-pointer transition-colors disabled:opacity-50"
                            title="删除该站点已固化的快照文件"
                          >
                            {clearingStorage ? <Loader2 size={13} className="animate-spin" /> : <Trash2 size={13} />}
                            <span>清除快照</span>
                          </button>
                        )}
                      </div>
                    </div>

                    {/* 快照详细信息格栅 */}
                    <div className="grid grid-cols-1 sm:grid-cols-3 gap-3">
                      <div className="p-3 rounded-xl bg-panel/70 border border-edge/60 flex flex-col gap-1">
                        <div className="text-[11px] text-inkdim flex items-center justify-between">
                          <span>目标站点 Origin</span>
                          <Globe size={12} className="text-blue-400" />
                        </div>
                        <div className="font-mono text-[12.5px] text-ink font-medium truncate" title={storageSummary?.landing_url || (storageSummary?.host ? `${storageSummary.host}:${storageSummary.port || ""}` : (debugState?.url || "N/A"))}>
                          {storageSummary?.host ? `${storageSummary.host}${storageSummary.port ? `:${storageSummary.port}` : ""}` : (debugState?.url ? new URL(debugState.url).host : "未连接")}
                        </div>
                        <div className="text-[10px] text-inkdim truncate font-mono" title={storageSummary?.landing_url || "物理哈希隔离存储"}>
                          {storageSummary?.landing_url ? `落地: ${storageSummary.landing_url.replace(/^https?:\/\/[^\/]+/, "")}` : "物理哈希隔离存储"}
                        </div>
                      </div>

                      <div className="p-3 rounded-xl bg-panel/70 border border-edge/60 flex flex-col gap-1">
                        <div className="text-[11px] text-inkdim flex items-center justify-between">
                          <span>持久化凭据计数</span>
                          <Database size={12} className="text-emerald-400" />
                        </div>
                        <div className="font-mono text-[12.5px] text-ink font-semibold flex items-center gap-1.5 flex-wrap">
                          <span className="text-emerald-400">{storageSummary?.cookie_count ?? 0}</span>
                          <span className="text-[11px] font-normal text-inkdim">Cookies</span>
                          <span className="text-inkdim">/</span>
                          <span className="text-blue-400">
                            {(storageSummary?.local_storage_count ?? 0) + (storageSummary?.session_storage_count ?? 0)}
                          </span>
                          <span className="text-[11px] font-normal text-inkdim" title={`LocalStorage: ${storageSummary?.local_storage_count ?? 0}, SessionStorage: ${storageSummary?.session_storage_count ?? 0}`}>
                            Storage ({storageSummary?.local_storage_count ?? 0} LS / {storageSummary?.session_storage_count ?? 0} SS)
                          </span>
                        </div>
                        <div className="text-[10px] text-inkdim truncate">
                          包含 Cookies、LocalStorage、SessionStorage
                        </div>
                      </div>

                      <div className="p-3 rounded-xl bg-panel/70 border border-edge/60 flex flex-col gap-1">
                        <div className="text-[11px] text-inkdim flex items-center justify-between">
                          <span>最后固化时间</span>
                          <RefreshCw size={12} className="text-purple-400" />
                        </div>
                        <div className="font-mono text-[12px] text-ink truncate">
                          {storageSummary?.updated_at ? new Date(storageSummary.updated_at).toLocaleString() : "尚未保存"}
                        </div>
                        <div className="text-[10px] text-inkdim truncate" title={storageSummary?.file_path}>
                          {storageSummary?.exists ? "已加入 .gitignore 防泄露" : "等待人工首次登录固化"}
                        </div>
                      </div>
                    </div>

                    {/* Storage 键名清单（LocalStorage 与 SessionStorage，帮助确认完整捕获 Token） */}
                    {((storageSummary?.local_storage_keys && storageSummary.local_storage_keys.length > 0) ||
                      (storageSummary?.session_storage_keys && storageSummary.session_storage_keys.length > 0)) && (
                      <div className="flex flex-col gap-2 pt-1">
                        {storageSummary.local_storage_keys && storageSummary.local_storage_keys.length > 0 && (
                          <div className="flex flex-col gap-1">
                            <div className="text-[11px] text-inkdim font-medium flex items-center gap-1.5">
                              <span className="text-blue-400 font-mono text-[10px] px-1 rounded bg-blue-500/10 border border-blue-500/20">LocalStorage</span>
                              <span>已捕获 ({storageSummary.local_storage_keys.length}):</span>
                            </div>
                            <div className="flex flex-wrap gap-1.5 max-h-20 overflow-y-auto">
                              {storageSummary.local_storage_keys.map((k) => (
                                <span
                                  key={k}
                                  className="text-[10.5px] font-mono px-2 py-0.5 rounded-md bg-panel3 border border-edge text-ink"
                                >
                                  {k}
                                </span>
                              ))}
                            </div>
                          </div>
                        )}

                        {storageSummary.session_storage_keys && storageSummary.session_storage_keys.length > 0 && (
                          <div className="flex flex-col gap-1">
                            <div className="text-[11px] text-inkdim font-medium flex items-center gap-1.5">
                              <span className="text-purple-400 font-mono text-[10px] px-1 rounded bg-purple-500/10 border border-purple-500/20">SessionStorage</span>
                              <span>已捕获 ({storageSummary.session_storage_keys.length}):</span>
                            </div>
                            <div className="flex flex-wrap gap-1.5 max-h-20 overflow-y-auto">
                              {storageSummary.session_storage_keys.map((k) => (
                                <span
                                  key={k}
                                  className="text-[10.5px] font-mono px-2 py-0.5 rounded-md bg-panel3 border border-edge text-ink"
                                >
                                  {k}
                                </span>
                              ))}
                            </div>
                          </div>
                        )}
                      </div>
                    )}

                    {/* 关键安全机制与服务端注销提醒 */}
                    <div className="text-[11px] p-2.5 rounded-lg bg-amber-500/10 border border-amber-500/20 text-amber-300/90 flex items-start gap-2">
                      <span className="shrink-0 mt-0.5 text-[13px]">💡</span>
                      <div className="leading-relaxed">
                        <strong className="text-amber-200">关于服务端 Token 自动注销机制：</strong>
                        某些现代政企/医务系统（如 YWPT）在网页中点击右上角【退出】时，会发送退出请求并在后端网关将 Token 彻底拉黑注销。此时若再尝试注入该旧快照会触发 401 拦截。
                        <strong className="text-amber-200 ml-1">正确验证免登：</strong>请直接通过 Harness 的【关闭浏览器】并重新启动会话来验证免登冷启动；若凭据已在服务端作废，仅需重新登录一次并点击【固化当前登录态快照】即可更新。
                      </div>
                    </div>
                  </div>

                  {/* 核心卡片 2: 3 步闭环 SOP 标准执行流程 */}
                  <div className="p-5 rounded-2xl border border-edge bg-panel2 flex flex-col gap-4 shadow-sm">
                    <div className="flex items-center justify-between border-b border-edge/60 pb-3">
                      <div className="flex items-center gap-2">
                        <FileText size={17} className="text-accent shrink-0" />
                        <span className="font-semibold text-[14px] text-ink">
                          执行与测试标准流程 (SOP 指南)
                        </span>
                      </div>
                      <span className="text-[11px] text-inkdim">3 步完成免登接入与闭环验证</span>
                    </div>

                    <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
                      {/* Step 1 */}
                      <div className="p-4 rounded-xl border border-edge/70 bg-panel flex flex-col gap-2 relative">
                        <div className="w-6 h-6 rounded-full bg-accent/15 text-accent font-bold text-[12px] flex items-center justify-center font-mono">
                          1
                        </div>
                        <div className="font-semibold text-[13px] text-ink">
                          视窗首次人工登录
                        </div>
                        <p className="text-[11.5px] text-inkdim leading-relaxed">
                          切换到【视窗】Tab 或独立 Chromium 视窗中，正常输入账号密码。若有短信/图片验证码或科室/组织选择弹窗，直接人工点击完成，直到进入系统主界面。
                        </p>
                      </div>

                      {/* Step 2 */}
                      <div className="p-4 rounded-xl border border-blue-500/30 bg-blue-500/5 flex flex-col gap-2 relative">
                        <div className="w-6 h-6 rounded-full bg-blue-500/20 text-blue-400 font-bold text-[12px] flex items-center justify-center font-mono">
                          2
                        </div>
                        <div className="font-semibold text-[13px] text-ink">
                          一键固化快照
                        </div>
                        <p className="text-[11.5px] text-inkdim leading-relaxed">
                          登录成功后，切回本面板点击【固化当前登录态快照】。系统底层通过 CDP 协议将 Cookies、LocalStorage、SessionStorage 冻结为 JSON 并以站点域名物理哈希隔离落盘。
                        </p>
                      </div>

                      {/* Step 3 */}
                      <div className="p-4 rounded-xl border border-emerald-500/30 bg-emerald-500/5 flex flex-col gap-2 relative">
                        <div className="w-6 h-6 rounded-full bg-emerald-500/20 text-emerald-400 font-bold text-[12px] flex items-center justify-center font-mono">
                          3
                        </div>
                        <div className="font-semibold text-[13px] text-ink">
                          Agent 自动免登与闭环
                        </div>
                        <p className="text-[11.5px] text-inkdim leading-relaxed">
                          后续无论是 Agent 自动化执行任务、新启会话还是跨会话重启，只要导航至该站点，底层自动预注入全量状态，0 次大模型调用、0 次表单输入、100% 确定性免登。
                        </p>
                      </div>
                    </div>
                  </div>

                  {/* 核心卡片 3: 测试与验证指引 (Testing Protocol) */}
                  <div className="p-5 rounded-2xl border border-edge bg-panel2 flex flex-col gap-3 shadow-sm">
                    <div className="flex items-center gap-2 border-b border-edge/60 pb-2.5">
                      <ShieldCheck size={16} className="text-emerald-400 shrink-0" />
                      <span className="font-semibold text-[13.5px] text-ink">
                        快速测试与质量保证规范
                      </span>
                    </div>

                    <div className="space-y-2 text-[12px] text-inkdim leading-relaxed">
                      <div className="flex items-start gap-2">
                        <span className="text-accent font-bold mt-0.5">•</span>
                        <div>
                          <strong className="text-ink font-medium">免登恢复测试 (Restore Test)：</strong>
                          验证免登恢复时，可直接点击【注入并恢复登录态】或关闭浏览器后重新打开测试。
                          <span className="text-amber-400 font-medium ml-1">
                            ⚠️ 注意：切勿在系统网页右上角点击【退出登录】，因为主动注销会向后端发送请求拉黑/销毁该 Token，导致凭据在服务端被废弃。
                          </span>
                        </div>
                      </div>
                      <div className="flex items-start gap-2">
                        <span className="text-accent font-bold mt-0.5">•</span>
                        <div>
                          <strong className="text-ink font-medium">冷启动验证 (Cold-Start Verification)：</strong>
                          在 Agent 会话对话框输入例如：“请用浏览器访问 http://127.0.0.1:18086/ 并查看当前是否已登录”，验证 Agent 在全新无头沙箱中是否无需点击登录表单直接进入内页。
                        </div>
                      </div>
                      <div className="flex items-start gap-2">
                        <span className="text-accent font-bold mt-0.5">•</span>
                        <div>
                          <strong className="text-ink font-medium">快照清除与失效回滚 (Clear & Rollback Test)：</strong>
                          点击【清除快照】，刷新视窗，页面应立即返回登录界面；再次登录并固化，免登能力无缝恢复。
                        </div>
                      </div>
                      <div className="flex items-start gap-2">
                        <span className="text-accent font-bold mt-0.5">•</span>
                        <div>
                          <strong className="text-ink font-medium">后端自动化单测 (Rust Cargo Test)：</strong>
                          终端运行 <code className="px-1.5 py-0.5 rounded bg-panel font-mono text-accent border border-edge/60">cargo test --lib browser::storage</code>，自动验证存储路径哈希隔离、Summary 提取与 WebStorage 注入脚本生成。
                        </div>
                      </div>
                    </div>
                  </div>

                  {/* 核心卡片 4: 页面语义状态机 */}
                  <div className="p-4 rounded-xl border border-edge bg-panel2 flex flex-col gap-2 shadow-xs">
                    <div className="flex items-center justify-between">
                      <span className="text-[12.5px] font-semibold text-ink flex items-center gap-1.5">
                        <Activity size={14} className="text-emerald-400" />
                        <span>页面语义健康度 (Page State Machine)</span>
                      </span>
                      {debugState?.page_state?.type === "Ready" ? (
                        <span className="text-[10.5px] px-2 py-0.5 rounded-full bg-emerald-500/15 text-emerald-400 border border-emerald-500/30 font-medium">
                          🟢 Ready (正常业务态)
                        </span>
                      ) : debugState?.page_state?.type === "Unauthenticated" ? (
                        <span className="text-[10.5px] px-2 py-0.5 rounded-full bg-amber-500/15 text-amber-400 border border-amber-500/30 font-medium">
                          🟡 Unauthenticated (未登录态)
                        </span>
                      ) : debugState?.page_state?.type === "ActionBlocked" ? (
                        <span className="text-[10.5px] px-2 py-0.5 rounded-full bg-purple-500/15 text-purple-300 border border-purple-500/30 font-medium">
                          🟣 ActionBlocked (弹窗阻断态)
                        </span>
                      ) : (
                        <span className="text-[10.5px] px-2 py-0.5 rounded-full bg-panel3 text-inkdim border border-edge font-medium">
                          ⚪ 未连接
                        </span>
                      )}
                    </div>
                    <div className="text-[11px] text-inkdim leading-relaxed">
                      解耦 CDP HTTP 状态码，直接深度扫描 SPA 路由路径与登录遮罩，为 Agent 任务决策提供高精度语义支撑。
                    </div>
                  </div>
                </div>
              )}
            </>
          )}
        </div>
      </div>
    </div>
  );
}
