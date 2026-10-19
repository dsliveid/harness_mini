import { useEffect, useState, useRef } from "react";
import { ipc } from "../ipc";
import { useStore, currentSession } from "../store";
import type { BrowserDebugState, InteractiveElement, AuthMacroStatus, AuthStep } from "../types";
import {
  Globe,
  X,
  RefreshCw,
  Camera,
  Trash2,
  Monitor,
  MousePointer,
  AlertTriangle,
  Bug,
  Activity,
  Terminal,
  Search,
  Sparkles,
  ExternalLink,
  ChevronRight,
  Send,
  Loader2,
  List,
  KeyRound,
  Play,
  Square,
  Circle,
  ShieldCheck,
  CheckCircle2,
} from "./Icons";

type ActiveTab = "view" | "elements" | "data" | "console" | "network" | "auth";

export function BrowserInspector() {
  const show = useStore((s) => s.showBrowserInspector);
  const setShow = useStore((s) => s.setShowBrowserInspector);
  const session = useStore((s) => currentSession(s));
  const currentId = useStore((s) => s.currentId);
  const pushToast = useStore((s) => s.pushToast);

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

  // 登录流程录制与免登宏状态
  const [macroStatus, setMacroStatus] = useState<AuthMacroStatus | null>(null);
  const [isRecording, setIsRecording] = useState(false);
  const [recordedSteps, setRecordedSteps] = useState<AuthStep[]>([]);
  const [replaying, setReplaying] = useState(false);
  const [savingMacro, setSavingMacro] = useState(false);

  const viewportRef = useRef<HTMLDivElement>(null);

  // 获取免登宏状态
  const fetchMacroStatus = async () => {
    if (!sessionId) return;
    try {
      const status = await ipc.getAuthMacroStatus(sessionId);
      setMacroStatus(status);
    } catch {}
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
    void fetchMacroStatus();

    // 弹窗开启时，每 2.5 秒静默轮询一次最新页面快照与状态
    const timer = setInterval(() => {
      void fetchState(true);
      void fetchMacroStatus();
    }, 2500);

    return () => clearInterval(timer);
  }, [show, sessionId]);

  // 录制模式下，每 1 秒同步最新捕获的操作步骤
  useEffect(() => {
    if (!isRecording || !sessionId) return;
    const interval = setInterval(async () => {
      try {
        const steps = await ipc.getRecordedBrowserSteps(sessionId);
        if (steps && steps.length > 0) {
          setRecordedSteps(steps);
        }
      } catch {}
    }, 1000);
    return () => clearInterval(interval);
  }, [isRecording, sessionId]);

  // 开启录制
  const handleStartRecording = async () => {
    if (!sessionId) return;
    try {
      await ipc.startBrowserRecording(sessionId);
      setIsRecording(true);
      setRecordedSteps([]);
      pushToast("已开启登录流程录制，请在视窗内进行输入与点击交互，操作将被实时捕获", "info");
    } catch (err: any) {
      pushToast(`启动录制失败: ${err}`, "error");
    }
  };

  // 完成录制并保存
  const handleStopRecordingAndSave = async () => {
    if (!sessionId) return;
    setSavingMacro(true);
    try {
      const macro = await ipc.stopBrowserRecordingAndSave(sessionId, recordedSteps);
      setIsRecording(false);
      pushToast(`登录宏已保存 (共 ${macro.steps.length} 步)，终态 Cookies & LocalStorage 已成功固化`, "success");
      void fetchMacroStatus();
      void fetchState(true);
    } catch (err: any) {
      pushToast(`保存录制宏失败: ${err}`, "error");
    } finally {
      setSavingMacro(false);
    }
  };

  // 取消录制
  const handleCancelRecording = () => {
    setIsRecording(false);
    pushToast("已取消本次录制", "info");
  };

  // 重放登录宏
  const handleReplayMacro = async () => {
    if (!sessionId || replaying) return;
    setReplaying(true);
    try {
      pushToast("正在后台重放免登宏与登录流程...", "info");
      await ipc.replayBrowserAuthMacro(sessionId);
      pushToast("免登宏重放成功，已刷新系统登录态与指定机构", "success");
      void fetchMacroStatus();
      void fetchState(true);
    } catch (err: any) {
      pushToast(`重放失败: ${err}`, "error");
    } finally {
      setReplaying(false);
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

    // 网页截图实际尺寸与渲染尺寸缩放比例
    const scaleX = img.naturalWidth / rect.width;
    const scaleY = img.naturalHeight / rect.height;

    const clickX = (e.clientX - rect.left) * scaleX;
    const clickY = (e.clientY - rect.top) * scaleY;

    // 显示点击涟漪动画
    setClickRipple({ x: e.clientX - rect.left, y: e.clientY - rect.top });
    setTimeout(() => setClickRipple(null), 600);

    setInteracting(true);
    try {
      await ipc.browserUserInteract(sessionId, "click", clickX, clickY);
      pushToast(`已人工点击坐标 (${Math.round(clickX)}, ${Math.round(clickY)})`, "info");
      // 延迟刷新
      setTimeout(() => void fetchState(true), 500);
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
              <div className="text-[11px] text-inkdim flex items-center gap-1.5 mt-0.5">
                <span>会话 ID:</span>
                <span className="font-mono text-ink">{sessionId.slice(0, 16)}...</span>
              </div>
            </div>
          </div>

          <div className="flex items-center gap-2">
            {/* 登录流程录制控制区 */}
            {isRecording ? (
              <div className="flex items-center gap-1.5 bg-red-500/10 border border-red-500/30 px-2.5 py-1 rounded-lg">
                <span className="w-2 h-2 rounded-full bg-red-500 animate-ping" />
                <span className="text-[11.5px] font-medium text-red-400">
                  录制中 ({recordedSteps.length} 步)
                </span>
                <button
                  onClick={() => void handleStopRecordingAndSave()}
                  disabled={savingMacro}
                  className="px-2 py-0.5 rounded bg-emerald-500/20 text-emerald-400 border border-emerald-500/40 hover:bg-emerald-500/30 text-[11px] font-medium flex items-center gap-1 cursor-pointer ml-1"
                  title="完成录制并锁定当前终态 (Cookies + LocalStorage)"
                >
                  <CheckCircle2 size={12} />
                  <span>{savingMacro ? "保存中…" : "完成录制"}</span>
                </button>
                <button
                  onClick={handleCancelRecording}
                  className="p-1 rounded text-inkdim hover:text-ink hover:bg-panel3 cursor-pointer"
                  title="取消录制"
                >
                  <X size={12} />
                </button>
              </div>
            ) : (
              <>
                {macroStatus?.has_macro ? (
                  <div className="flex items-center gap-1.5 bg-emerald-500/10 border border-emerald-500/30 px-2.5 py-1 rounded-lg text-[11.5px]">
                    <ShieldCheck size={13} className="text-emerald-400" />
                    <span className="text-emerald-400 font-medium">免登宏 ({macroStatus.step_count}步)</span>
                    <button
                      onClick={() => void handleReplayMacro()}
                      disabled={replaying}
                      className="p-1 rounded text-emerald-400 hover:bg-emerald-500/20 cursor-pointer"
                      title="立即测试重放登录宏"
                    >
                      <Play size={11} className={replaying ? "animate-spin" : ""} />
                    </button>
                    <button
                      onClick={() => void handleStartRecording()}
                      className="text-[11px] text-inkdim hover:text-ink hover:underline ml-0.5 cursor-pointer"
                      title="重新录制登录流程"
                    >
                      重新录制
                    </button>
                  </div>
                ) : (
                  <button
                    onClick={() => void handleStartRecording()}
                    className="px-2.5 py-1.5 rounded-lg border border-accent/40 bg-accent/10 hover:bg-accent/20 text-accent text-[12px] flex items-center gap-1.5 transition-colors cursor-pointer"
                    title="录制 1~N 步登录流程（输入账号密码、选择机构等），录制完成后手动点击完成"
                  >
                    <Circle size={10} className="fill-accent text-accent" />
                    <span>录制登录流程</span>
                  </button>
                )}
              </>
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
                onClick={() => setActiveTab("auth")}
                className={`px-3 py-1 rounded text-[11.5px] font-medium transition-colors flex items-center gap-1.5 cursor-pointer ${
                  activeTab === "auth"
                    ? "bg-accent/20 text-accent shadow-xs"
                    : "text-inkdim hover:text-ink hover:bg-panel3"
                }`}
              >
                <KeyRound size={12} />
                <span>免登宏与自愈</span>
                {macroStatus?.has_macro && (
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
                  {/* 录制中提示与快速完成条 */}
                  {isRecording && (
                    <div className="p-3 rounded-xl bg-red-500/10 border border-red-500/30 flex items-center justify-between gap-3 text-[12px] animate-in fade-in duration-150">
                      <div className="flex items-center gap-2 text-red-400 font-medium">
                        <span className="w-2.5 h-2.5 rounded-full bg-red-500 animate-pulse" />
                        <span>正在录制登录流程：请直接在下方网页视窗中输入、点击并选定目标机构，系统正实时捕获操作 (已捕获 {recordedSteps.length} 步)</span>
                      </div>
                      <div className="flex items-center gap-2 shrink-0">
                        <button
                          onClick={() => void handleStopRecordingAndSave()}
                          disabled={savingMacro}
                          className="px-3.5 py-1.5 rounded-lg bg-emerald-500 text-white hover:bg-emerald-600 font-medium text-[11.5px] flex items-center gap-1.5 cursor-pointer shadow-xs transition-colors"
                        >
                          <CheckCircle2 size={13} />
                          <span>{savingMacro ? "固化中…" : "完成录制并保存"}</span>
                        </button>
                        <button
                          onClick={handleCancelRecording}
                          className="px-2.5 py-1.5 rounded-lg border border-edge text-inkdim hover:text-ink text-[11.5px] cursor-pointer hover:bg-panel3 transition-colors"
                        >
                          取消
                        </button>
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

                  {/* 屏幕快照与交互视窗 */}
                  <div
                    ref={viewportRef}
                    className="flex-1 min-h-[300px] bg-black/40 border border-edge rounded-xl overflow-auto relative flex items-center justify-center p-2"
                  >
                    {screenshotUrl ? (
                      <div className="relative inline-block shadow-xl rounded overflow-hidden select-none cursor-crosshair">
                        <img
                          src={screenshotUrl}
                          alt="Browser Viewport"
                          onClick={handleViewportClick}
                          className="max-w-full max-h-[60vh] object-contain block bg-white"
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

              {/* Tab 5: 免登宏与状态自愈 */}
              {activeTab === "auth" && (
                <div className="flex flex-col h-full gap-4 max-w-4xl mx-auto w-full">
                  {/* 状态总览与快捷操作卡片 */}
                  <div className="p-4 rounded-xl border border-edge bg-panel2 flex flex-col sm:flex-row sm:items-center justify-between gap-4 shadow-sm">
                    <div>
                      <div className="flex items-center gap-2">
                        <KeyRound size={16} className="text-accent" />
                        <span className="font-semibold text-[13.5px]">登录宏与状态自愈控制台</span>
                        {macroStatus?.has_macro ? (
                          <span className="text-[11px] font-mono px-2 py-0.5 rounded-full bg-emerald-500/15 text-emerald-400 border border-emerald-500/30">
                            已就绪 · {macroStatus.step_count} 个重放步骤
                          </span>
                        ) : (
                          <span className="text-[11px] font-mono px-2 py-0.5 rounded-full bg-amber-500/15 text-amber-400 border border-amber-500/30">
                            未配置录制宏
                          </span>
                        )}
                      </div>
                      <div className="text-[11.5px] text-inkdim mt-2 space-y-1">
                        <div>• 步骤指令: <code className="font-mono text-ink bg-panel px-1.5 py-0.5 rounded">.harness/auth_macro.json</code> (密码字段经本地 AES-256-GCM 强加密)</div>
                        <div>• 终态快照: <code className="font-mono text-ink bg-panel px-1.5 py-0.5 rounded">.harness/storage_state.json</code> (包含 Cookies 与 LocalStorage，日常测试 0 毫秒直达)</div>
                        {macroStatus?.last_updated && (
                          <div>• 最近快照更新: <span className="font-mono text-ink">{macroStatus.last_updated}</span></div>
                        )}
                      </div>
                    </div>

                    <div className="flex items-center gap-2 shrink-0">
                      {isRecording ? (
                        <button
                          onClick={() => void handleStopRecordingAndSave()}
                          disabled={savingMacro}
                          className="px-3.5 py-2 rounded-lg bg-emerald-500 text-white hover:bg-emerald-600 font-medium text-[12px] flex items-center gap-1.5 cursor-pointer shadow-sm transition-colors"
                        >
                          <CheckCircle2 size={14} />
                          <span>{savingMacro ? "固化中…" : "完成录制并锁定"}</span>
                        </button>
                      ) : (
                        <>
                          <button
                            onClick={() => void handleStartRecording()}
                            className="px-3 py-1.5 rounded-lg border border-accent/40 bg-accent/10 hover:bg-accent/20 text-accent font-medium text-[12px] flex items-center gap-1.5 cursor-pointer transition-colors"
                          >
                            <Circle size={10} className="fill-accent text-accent" />
                            <span>{macroStatus?.has_macro ? "重新录制流程" : "开始录制流程"}</span>
                          </button>
                          {macroStatus?.has_macro && (
                            <button
                              onClick={() => void handleReplayMacro()}
                              disabled={replaying}
                              className="px-3 py-1.5 rounded-lg border border-edge hover:bg-panel3 text-ink text-[12px] flex items-center gap-1.5 cursor-pointer transition-colors"
                            >
                              <Play size={12} className={replaying ? "animate-spin" : ""} />
                              <span>{replaying ? "重放中…" : "立即测试重放"}</span>
                            </button>
                          )}
                        </>
                      )}
                    </div>
                  </div>

                  {/* 步骤序列清单 */}
                  <div className="flex-1 min-h-0 border border-edge rounded-xl bg-black/20 p-4 flex flex-col gap-3 overflow-hidden">
                    <div className="flex items-center justify-between text-[12px] text-inkdim">
                      <span className="font-medium text-ink">
                        {isRecording ? `当前已捕获步骤 (${recordedSteps.length})` : "已固化动作序列"}
                      </span>
                      {isRecording && (
                        <span className="text-red-400 text-[11px] flex items-center gap-1">
                          <span className="w-1.5 h-1.5 rounded-full bg-red-400 animate-ping" />
                          实时监听中：在视窗中打字或点击均会自动记录
                        </span>
                      )}
                    </div>

                    <div className="flex-1 min-h-0 overflow-y-auto space-y-2">
                      {(isRecording ? recordedSteps : []).length === 0 && !macroStatus?.has_macro ? (
                        <div className="h-44 flex flex-col items-center justify-center text-inkdim text-[12px] gap-2 text-center">
                          <KeyRound size={30} className="text-inkdim/40" />
                          <span className="font-medium text-ink">暂无已录制的登录步骤</span>
                          <span className="text-[11.5px] text-inkdim/70 max-w-md">
                            点击右上角【开始录制流程】，切换至“视窗与接管”正常输入账号密码、点击登录并选择机构，完成后点击“完成录制”即可永久自动免登。
                          </span>
                        </div>
                      ) : (
                        (isRecording ? recordedSteps : []).map((step, idx) => (
                          <div
                            key={idx}
                            className="p-2.5 rounded-lg border border-edge bg-panel/70 flex items-center justify-between gap-3 text-[12px] font-mono hover:border-accent/40 transition-colors"
                          >
                            <div className="flex items-center gap-2.5 min-w-0">
                              <span className="w-5 h-5 rounded-full bg-accent/15 text-accent flex items-center justify-center text-[10.5px] font-bold shrink-0">
                                {step.order || idx + 1}
                              </span>
                              <span className="px-1.5 py-0.5 rounded text-[10.5px] bg-panel3 text-ink font-semibold shrink-0">
                                {step.action}
                              </span>
                              <span className="truncate text-ink font-normal">{step.desc || step.selector}</span>
                            </div>
                            <div className="text-[11px] text-inkdim shrink-0 flex items-center gap-2">
                              {step.is_encrypted ? (
                                <span className="px-1.5 py-0.2 rounded bg-amber-500/10 text-amber-400 border border-amber-500/20 text-[10px]">
                                  🔒 已加密 (AES-256)
                                </span>
                              ) : step.value ? (
                                <span className="max-w-[140px] truncate text-ink/80">"{step.value}"</span>
                              ) : null}
                            </div>
                          </div>
                        ))
                      )}
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
