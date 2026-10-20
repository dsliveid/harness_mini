import { useEffect, useState, useRef } from "react";
import { ipc } from "../ipc";
import { useStore, currentSession } from "../store";
import type { BrowserDebugState, InteractiveElement, AuthStep, AuthContract, AuthAction, DistillTrace } from "../types";
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
  KeyRound,
  Play,
  Square,
  Circle,
  ShieldCheck,
  ShieldAlert,
  CheckCircle2,
  Maximize2,
  Lock,
  Save,
  Database,
  Layers,
  Plus,
  ArrowUp,
  Code,
  FileText,
  Copy,
  Edit3,
} from "./Icons";

type ActiveTab = "view" | "elements" | "data" | "console" | "network" | "auth";

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

  // 登录流程录制轨迹状态 (交互探针)
  const [isRecording, setIsRecording] = useState(false);
  const [recordedSteps, setRecordedSteps] = useState<AuthStep[]>([]);
  const [showRecordedSteps, setShowRecordedSteps] = useState(false);

  // 声明式凭据契约与大模型提炼状态
  const [authContract, setAuthContract] = useState<AuthContract | null>(null);
  const [contractSystemName, setContractSystemName] = useState("");
  const [contractLoginUrl, setContractLoginUrl] = useState("");
  const [contractUsername, setContractUsername] = useState("");
  const [contractPassword, setContractPassword] = useState("");
  const [contractAssertUrl, setContractAssertUrl] = useState("");
  const [contractActions, setContractActions] = useState<AuthAction[]>([]);
  const [distilling, setDistilling] = useState(false);
  const [distilledPreview, setDistilledPreview] = useState<AuthContract | null>(null);
  const [distillTrace, setDistillTrace] = useState<DistillTrace | null>(null);
  const [showDistillTrace, setShowDistillTrace] = useState(false);
  const [refinePrompt, setRefinePrompt] = useState("");
  const [customSystemPrompt, setCustomSystemPrompt] = useState("");
  const [editingActionIndex, setEditingActionIndex] = useState<number | null>(null);
  const [savingContract, setSavingContract] = useState(false);
  const [runningPipeline, setRunningPipeline] = useState(false);

  // 预载默认系统提示词
  useEffect(() => {
    if (show && !customSystemPrompt) {
      void ipc.getDefaultAuthDistillSystemPrompt().then((prompt) => {
        setCustomSystemPrompt(prompt);
      }).catch(() => {});
    }
  }, [show]);

  const storeTab = useStore((s) => s.browserInspectorTab);
  useEffect(() => {
    if (storeTab) setActiveTab(storeTab);
  }, [storeTab]);

  const [zoomMode, setZoomMode] = useState<"fit" | "original">("fit");
  const [scrolling, setScrolling] = useState(false);
  const [capturingFull, setCapturingFull] = useState(false);

  const viewportRef = useRef<HTMLDivElement>(null);
  const wheelThrottleTimer = useRef<number | null>(null);
  const wheelAccumRef = useRef<{ dx: number; dy: number }>({ dx: 0, dy: 0 });

  // 获取声明式凭据契约
  const fetchAuthContract = async () => {
    if (!sessionId) return;
    try {
      const contract = await ipc.getAuthContract(sessionId);
      if (contract) {
        setAuthContract(contract);
        setContractSystemName(contract.system_name || "");
        setContractLoginUrl(contract.login_url || "");
        setContractUsername(contract.credentials?.username || "");
        setContractAssertUrl(contract.post_login?.success_assert_url || "");
        setContractActions(contract.post_login?.actions || []);
      }
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
    void fetchAuthContract();

    // 弹窗开启时，每 2.5 秒静默轮询一次最新页面快照与状态
    const timer = setInterval(() => {
      void fetchState(true);
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

  // 仅停止录制（保留捕获轨迹供审阅）
  const handleStopRecordingOnly = async () => {
    if (!sessionId) return;
    try {
      const steps = await ipc.stopBrowserRecording(sessionId).catch(() => []);
      if (steps && steps.length > 0) {
        setRecordedSteps(steps);
      }
      setIsRecording(false);
      setShowRecordedSteps(true);
      pushToast(`录制已停止 (已捕获 ${steps.length || recordedSteps.length} 步操作)，可展开下方列表查看`, "info");
    } catch (err: any) {
      pushToast(`停止录制失败: ${err}`, "error");
    }
  };

  // 停止录制并交由大模型智能提炼通用免登契约
  const handleStopAndDistill = async () => {
    if (!sessionId || distilling) return;
    setDistilling(true);
    try {
      pushToast("正在停止录制并交由大模型智能提炼通用免登契约...", "info");
      const steps = await ipc.stopBrowserRecording(sessionId).catch(() => []);
      if (steps && steps.length > 0) {
        setRecordedSteps(steps);
      }
      setIsRecording(false);

      const stepsToFeed = (steps && steps.length > 0) ? steps : recordedSteps;
      const res = await ipc.distillAuthContract(
        sessionId,
        stepsToFeed.length > 0 ? stepsToFeed : undefined,
        refinePrompt.trim() || undefined,
        customSystemPrompt.trim() || undefined
      );
      setDistilledPreview(res.contract);
      setDistillTrace(res.trace);
      setShowDistillTrace(true);
      setAuthContract(res.contract);
      setContractSystemName(res.contract.system_name || "");
      setContractLoginUrl(res.contract.login_url || "");
      setContractUsername(res.contract.credentials?.username || "");
      setContractAssertUrl(res.contract.post_login?.success_assert_url || "");
      setContractActions(res.contract.post_login?.actions || []);
      if (res.trace.system_prompt) {
        setCustomSystemPrompt(res.trace.system_prompt);
      }

      setActiveTab("auth");
      pushToast("✨ 大模型已成功提炼通用免登契约，请审阅确认", "success");
      void fetchState(true);
    } catch (err: any) {
      pushToast(`AI 提炼契约失败: ${err}`, "error");
    } finally {
      setDistilling(false);
    }
  };

  // 携带针对性补充提示词或更新后的系统提示词重新交由大模型提炼契约
  const handleRedistill = async (options?: { refine?: string; systemPrompt?: string }) => {
    if (!sessionId || distilling) return;
    const stepsToFeed = recordedSteps.length > 0 ? recordedSteps : undefined;
    setDistilling(true);
    try {
      const promptToUse = options?.refine !== undefined ? options.refine : refinePrompt;
      const sysToUse = options?.systemPrompt !== undefined ? options.systemPrompt : customSystemPrompt;
      pushToast("正在携带补充提示词要求大模型重新提炼通用契约...", "info");
      const res = await ipc.distillAuthContract(
        sessionId,
        stepsToFeed,
        promptToUse.trim() || undefined,
        sysToUse.trim() || undefined
      );
      setDistilledPreview(res.contract);
      setDistillTrace(res.trace);
      setAuthContract(res.contract);
      setContractSystemName(res.contract.system_name || "");
      setContractLoginUrl(res.contract.login_url || "");
      setContractUsername(res.contract.credentials?.username || "");
      setContractAssertUrl(res.contract.post_login?.success_assert_url || "");
      setContractActions(res.contract.post_login?.actions || []);
      if (res.trace.system_prompt) {
        setCustomSystemPrompt(res.trace.system_prompt);
      }
      pushToast("✨ 大模型已依据新提示词重新提炼契约，请审阅", "success");
    } catch (err: any) {
      pushToast(`重新提炼契约失败: ${err}`, "error");
    } finally {
      setDistilling(false);
    }
  };

  // 恢复默认系统提示词
  const handleResetDefaultSystemPrompt = async () => {
    try {
      const def = await ipc.getDefaultAuthDistillSystemPrompt();
      setCustomSystemPrompt(def);
      pushToast("已恢复为默认契约提炼系统提示词", "info");
    } catch (err: any) {
      pushToast(`恢复默认提示词失败: ${err}`, "error");
    }
  };

  // 取消录制
  const handleCancelRecording = () => {
    setIsRecording(false);
    pushToast("已取消本次录制", "info");
  };

  // 添加新的后置通用动作
  const handleAddAction = () => {
    const newAct: AuthAction = {
      step: contractActions.length + 1,
      desc: "",
      action_type: "select_modal_item",
      dialog_title: "",
      match_text: "",
      selector: "",
      confirm_btn: "确定",
    };
    const updated = [...contractActions, newAct];
    setContractActions(updated);
    setEditingActionIndex(updated.length - 1);
  };

  // 修改特定动作项字段
  const handleUpdateAction = (index: number, updates: Partial<AuthAction>) => {
    setContractActions((prev) =>
      prev.map((act, i) => (i === index ? { ...act, ...updates } : act))
    );
  };

  // 删除动作
  const handleDeleteAction = (index: number) => {
    setContractActions((prev) => {
      const filtered = prev.filter((_, i) => i !== index);
      return filtered.map((act, i) => ({ ...act, step: i + 1 }));
    });
    if (editingActionIndex === index) {
      setEditingActionIndex(null);
    } else if (editingActionIndex !== null && editingActionIndex > index) {
      setEditingActionIndex(editingActionIndex - 1);
    }
  };

  // 调整动作先后顺序
  const handleMoveAction = (index: number, direction: "up" | "down") => {
    const target = direction === "up" ? index - 1 : index + 1;
    if (target < 0 || target >= contractActions.length) return;
    setContractActions((prev) => {
      const copy = [...prev];
      const temp = copy[index];
      copy[index] = copy[target];
      copy[target] = temp;
      return copy.map((act, i) => ({ ...act, step: i + 1 }));
    });
    if (editingActionIndex === index) {
      setEditingActionIndex(target);
    }
  };

  // 保存声明式凭据契约
  const handleSaveContract = async () => {
    if (!sessionId) return;
    if (!contractLoginUrl.trim()) {
      pushToast("请输入登录页面 URL", "error");
      return;
    }
    if (!contractUsername.trim()) {
      pushToast("请输入登录账号/用户名", "error");
      return;
    }
    if (!authContract && !contractPassword.trim()) {
      pushToast("首次配置契约请输入登录密码", "error");
      return;
    }
    setSavingContract(true);
    try {
      const contract: AuthContract = {
        version: "3.0",
        system_name: contractSystemName.trim() || undefined,
        login_url: contractLoginUrl.trim(),
        credentials: {
          username: contractUsername.trim(),
          password_cipher: authContract?.credentials?.password_cipher || "",
        },
        post_login: {
          actions: contractActions.length > 0 ? contractActions : undefined,
          success_assert_url: contractAssertUrl.trim() || undefined,
        },
        updated_at: new Date().toISOString(),
      };
      await ipc.saveAuthContract(sessionId, contract, contractPassword.trim() || undefined);
      setContractPassword("");
      setDistilledPreview(null);
      pushToast("声明式免登契约已保存，凭据已通过 AES-256-GCM 强加密持久化", "success");
      void fetchAuthContract();
    } catch (err: any) {
      pushToast(`保存契约失败: ${err}`, "error");
    } finally {
      setSavingContract(false);
    }
  };

  // 执行声明式免登自愈流水线
  const handleRunPipeline = async () => {
    if (!sessionId || runningPipeline) return;
    setRunningPipeline(true);
    try {
      pushToast("正在启动受控浏览器并执行声明式免登流水线...", "info");
      await ipc.executeBrowserAuthPipeline(sessionId);
      pushToast("免登自愈流水线执行完成，已刷新系统登录态与指定机构", "success");
      void fetchState(true);
    } catch (err: any) {
      pushToast(`自愈流水线执行未成功: ${err}`, "error");
    } finally {
      setRunningPipeline(false);
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
                onClick={() => setActiveTab("auth")}
                className={`px-3 py-1 rounded text-[11.5px] font-medium transition-colors flex items-center gap-1.5 cursor-pointer ${
                  activeTab === "auth"
                    ? "bg-accent/20 text-accent shadow-xs"
                    : "text-inkdim hover:text-ink hover:bg-panel3"
                }`}
              >
                <KeyRound size={12} />
                <span>免登契约与自愈</span>
                {authContract && (
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
                    <div className="flex flex-col gap-2">
                      <div className="p-3 rounded-xl bg-red-500/10 border border-red-500/30 flex items-center justify-between gap-3 text-[12px] animate-in fade-in duration-150">
                        <div className="flex items-center gap-2 text-red-400 font-medium">
                          <span className="w-2.5 h-2.5 rounded-full bg-red-500 animate-pulse" />
                          <span>正在录制登录流程：请直接在下方网页视窗中输入、点击并选定目标科室/机构 (已捕获 {recordedSteps.length} 步)</span>
                        </div>
                        <div className="flex items-center gap-2 shrink-0">
                          <button
                            type="button"
                            onClick={() => setShowRecordedSteps(!showRecordedSteps)}
                            className="px-2.5 py-1.5 rounded-lg border border-edge hover:bg-panel3 text-ink text-[11.5px] flex items-center gap-1 cursor-pointer transition-colors"
                            title="展开或收起已捕获的操作步骤"
                          >
                            <List size={13} className="text-accent" />
                            <span>{showRecordedSteps ? "收起捕获列表" : `捕获步骤 (${recordedSteps.length})`}</span>
                          </button>
                          <button
                            type="button"
                            onClick={() => void handleStopAndDistill()}
                            disabled={distilling}
                            className="px-3 py-1.5 rounded-lg bg-gradient-to-r from-purple-600 to-indigo-600 text-white hover:opacity-95 font-medium text-[11.5px] flex items-center gap-1.5 cursor-pointer shadow-xs transition-opacity"
                            title="结束录制并交由大模型智能提炼声明式免登契约（自动提取账号、密码、通用弹窗与科室/机构）"
                          >
                            {distilling ? <Loader2 size={13} className="animate-spin" /> : <Sparkles size={13} />}
                            <span>{distilling ? "AI 提炼中…" : "✨ AI 提炼免登契约"}</span>
                          </button>
                          <button
                            type="button"
                            onClick={() => void handleStopRecordingOnly()}
                            className="px-2.5 py-1.5 rounded-lg border border-edge text-inkdim hover:text-ink text-[11.5px] cursor-pointer hover:bg-panel3 transition-colors"
                            title="仅停止录制，不立即调用大模型"
                          >
                            仅停止录制
                          </button>
                          <button
                            type="button"
                            onClick={handleCancelRecording}
                            disabled={distilling}
                            className="px-2 py-1.5 rounded-lg border border-edge text-inkdim hover:text-ink text-[11.5px] cursor-pointer hover:bg-panel3 transition-colors"
                          >
                            取消
                          </button>
                        </div>
                      </div>

                      {/* 实时录制操作列表抽屉 */}
                      {showRecordedSteps && (
                        <div className="p-3 rounded-xl border border-edge bg-panel2 flex flex-col gap-2 max-h-56 overflow-y-auto shadow-xs text-[11.5px] animate-in fade-in duration-150">
                          <div className="flex items-center justify-between text-inkdim font-medium border-b border-edge/60 pb-1.5">
                            <span className="flex items-center gap-1.5 text-ink">
                              <List size={13} className="text-accent" />
                              <span>已捕获的原始交互轨迹 ({recordedSteps.length} 项)</span>
                            </span>
                            <span className="text-[10.5px]">每步交互均被 CDP 监听捕获</span>
                          </div>
                          {recordedSteps.length === 0 ? (
                            <div className="text-inkdim text-center py-2 text-[11px]">
                              尚未捕获到操作，请在下方网页内点击或输入
                            </div>
                          ) : (
                            <div className="space-y-1 divide-y divide-edge/30">
                              {recordedSteps.map((s, idx) => (
                                <div key={idx} className="pt-1 flex items-center justify-between gap-2">
                                  <div className="flex items-center gap-2 min-w-0">
                                    <span className="w-4 h-4 rounded-full bg-panel text-[9.5px] flex items-center justify-center font-mono font-bold text-ink shrink-0 border border-edge">
                                      {s.order || idx + 1}
                                    </span>
                                    <span className={`text-[9.5px] px-1 py-0.2 rounded font-mono uppercase font-semibold shrink-0 ${
                                      s.action === "type" ? "bg-blue-500/20 text-blue-400" :
                                      s.action === "click" ? "bg-purple-500/20 text-purple-400" :
                                      s.action === "wait" ? "bg-amber-500/20 text-amber-400" :
                                      "bg-panel text-inkdim"
                                    }`}>
                                      {s.action}
                                    </span>
                                    <span className="text-ink truncate font-mono text-[11px]">
                                      {s.selector || s.desc || "(页面整体)"}
                                    </span>
                                  </div>
                                  <div className="flex items-center gap-2 shrink-0">
                                    {s.value && (
                                      <span className="text-[10.5px] font-mono px-1.5 py-0.2 rounded bg-panel border border-edge text-emerald-400">
                                        {s.is_encrypted ? "••••••••" : s.value}
                                      </span>
                                    )}
                                    {s.ms && (
                                      <span className="text-[10px] text-inkdim font-mono">
                                        +{s.ms}ms
                                      </span>
                                    )}
                                  </div>
                                </div>
                              ))}
                            </div>
                          )}
                        </div>
                      )}
                    </div>
                  )}

                  {/* 页面语义健康度告警与自愈提示 */}
                  {debugState?.page_state && debugState.page_state.type === "Unauthenticated" && (
                    <div className="p-3 rounded-xl bg-amber-500/10 border border-amber-500/30 flex items-center justify-between gap-3 text-[12px] animate-in fade-in duration-150">
                      <div className="flex items-center gap-2 text-amber-400 font-medium">
                        <AlertTriangle size={15} className="shrink-0" />
                        <span>
                          页面处于未鉴权态（检测到重定向或登录表单，理由: <code className="font-mono bg-panel px-1 rounded">{debugState.page_state.payload.reason}</code>）
                        </span>
                      </div>
                      <div className="flex items-center gap-2 shrink-0">
                        <button
                          onClick={() => void handleRunPipeline()}
                          disabled={runningPipeline}
                          className="px-3 py-1.5 rounded-lg bg-accent text-white hover:bg-accent/90 font-medium text-[11.5px] flex items-center gap-1.5 cursor-pointer shadow-xs transition-colors"
                        >
                          {runningPipeline ? <Loader2 size={13} className="animate-spin" /> : <Play size={13} />}
                          <span>执行免登流水线</span>
                        </button>
                        <button
                          onClick={() => setActiveTab("auth")}
                          className="px-2.5 py-1.5 rounded-lg border border-edge bg-panel hover:bg-panel3 text-ink text-[11.5px] transition-colors cursor-pointer"
                        >
                          配置凭据
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

              {/* Tab 5: 声明式凭据契约与鉴权网关 */}
              {activeTab === "auth" && (
                <div className="flex flex-col h-full gap-4 max-w-4xl mx-auto w-full overflow-y-auto pr-1">
                  {/* 主卡片：声明式免登契约配置 (Auth Contract v3.0) */}
                  <div className="p-5 rounded-xl border border-edge bg-panel2 flex flex-col gap-4 shadow-sm">
                    {/* 卡片头部：契约状态与核心操作栏（AI 智能录制填入直接归属于契约配置） */}
                    <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-3 border-b border-edge/60 pb-3">
                      <div className="flex items-center gap-2 flex-wrap">
                        <ShieldCheck size={18} className="text-accent shrink-0" />
                        <span className="font-semibold text-[14px] text-ink">
                          声明式免登契约配置
                        </span>
                        <span className="text-[10px] px-2 py-0.5 rounded-full font-mono bg-panel3 text-inkdim border border-edge">
                          v3.0
                        </span>
                        {authContract ? (
                          <span className="text-[10.5px] px-2 py-0.5 rounded-full bg-emerald-500/15 text-emerald-400 border border-emerald-500/30 font-medium">
                            🟢 已配置
                          </span>
                        ) : (
                          <span className="text-[10.5px] px-2 py-0.5 rounded-full bg-amber-500/15 text-amber-400 border border-amber-500/30 font-medium">
                            🟡 未配置
                          </span>
                        )}
                        {activeModel && (
                          <span className="text-[10.5px] px-2 py-0.5 rounded-full bg-purple-500/15 text-purple-300 border border-purple-500/30 flex items-center gap-1 font-mono">
                            <Sparkles size={10} className="text-purple-400" />
                            <span>{activeModel.provider.name} · {activeModel.model}</span>
                          </span>
                        )}
                        {authContract?.updated_at && (
                          <span className="text-[11px] text-inkdim hidden sm:inline ml-1">
                            (更新于 <span className="font-mono text-ink">{authContract.updated_at}</span>)
                          </span>
                        )}
                      </div>

                      {/* 契约卡片核心操作区 */}
                      <div className="flex items-center gap-2 flex-wrap">
                        {recordedSteps.length > 0 && (
                          <button
                            type="button"
                            onClick={() => setShowRecordedSteps(!showRecordedSteps)}
                            className="px-2.5 py-1.5 rounded-lg border border-edge hover:bg-panel3 text-ink text-[11.5px] flex items-center gap-1.5 cursor-pointer transition-colors"
                            title="展开或收起已捕获的交互轨迹"
                          >
                            <List size={13} className="text-accent" />
                            <span>{showRecordedSteps ? "收起轨迹" : `捕获轨迹 (${recordedSteps.length})`}</span>
                          </button>
                        )}

                        <button
                          type="button"
                          onClick={() => {
                            setActiveTab("view");
                            void handleStartRecording();
                          }}
                          disabled={isRecording || distilling}
                          className="px-3 py-1.5 rounded-lg bg-gradient-to-r from-purple-600 to-indigo-600 hover:opacity-95 text-white font-medium text-[11.5px] flex items-center gap-1.5 cursor-pointer shadow-xs transition-opacity"
                          title="在视窗中走一遍登录流程，由大模型自动提炼并填入下方契约表单"
                        >
                          {distilling ? (
                            <>
                              <Loader2 size={13} className="animate-spin" />
                              <span>AI 提炼中…</span>
                            </>
                          ) : isRecording ? (
                            <>
                              <Circle size={11} className="text-red-400 fill-red-400 animate-pulse" />
                              <span>正在录制中…</span>
                            </>
                          ) : (
                            <>
                              <Sparkles size={13} />
                              <span>✨ AI 智能录制填入</span>
                            </>
                          )}
                        </button>

                        <button
                          onClick={() => void handleSaveContract()}
                          disabled={savingContract}
                          className="px-3 py-1.5 rounded-lg bg-emerald-600 hover:bg-emerald-700 text-white font-medium text-[11.5px] flex items-center gap-1 cursor-pointer shadow-xs transition-colors"
                          title="保存当前免登契约到 .harness/auth_contract.json (密码强加密)"
                        >
                          {savingContract ? <Loader2 size={12} className="animate-spin" /> : <Save size={12} />}
                          <span>{savingContract ? "保存中…" : "保存契约"}</span>
                        </button>

                        <button
                          onClick={() => void handleRunPipeline()}
                          disabled={runningPipeline}
                          className="px-3 py-1.5 rounded-lg bg-accent hover:bg-accent/90 text-white font-medium text-[11.5px] flex items-center gap-1 cursor-pointer shadow-xs transition-colors"
                          title="使用当前契约运行完整免登与状态自愈测试"
                        >
                          {runningPipeline ? <Loader2 size={12} className="animate-spin" /> : <Play size={12} />}
                          <span>{runningPipeline ? "运行中…" : "测试流水线"}</span>
                        </button>
                      </div>
                    </div>

                    {/* AI 提炼契约审阅与 LLM 交互追踪面板 (就地嵌入契约卡片上方) */}
                    {distilledPreview && (
                      <div className="p-3.5 rounded-xl border-2 border-purple-500/50 bg-gradient-to-b from-purple-500/10 to-panel2 flex flex-col gap-3 shadow-md animate-in fade-in duration-200">
                        <div className="flex items-center justify-between border-b border-purple-500/20 pb-2.5 flex-wrap gap-2">
                          <div className="flex items-center gap-2 text-purple-300 font-semibold text-[13px] flex-wrap">
                            <Sparkles size={15} className="text-purple-400" />
                            <span>✨ AI 大模型已成功提炼通用免登契约 (v3.0)</span>
                            {(distillTrace?.provider_name || activeModel?.provider.name) && (
                              <span className="text-[11px] px-2 py-0.5 rounded-full bg-purple-500/20 text-purple-200 font-mono border border-purple-500/30 flex items-center gap-1 font-normal">
                                <span>厂商: {distillTrace?.provider_name || activeModel?.provider.name}</span>
                                <span>·</span>
                                <span>模型: {distillTrace?.model_name || activeModel?.model}</span>
                              </span>
                            )}
                          </div>
                          <div className="flex items-center gap-2">
                            {distillTrace && (
                              <button
                                type="button"
                                onClick={() => setShowDistillTrace(!showDistillTrace)}
                                className="px-2.5 py-1 rounded-md bg-purple-500/20 hover:bg-purple-500/30 text-purple-200 text-[11px] flex items-center gap-1 cursor-pointer transition-colors border border-purple-500/30"
                                title="查看与调整大模型提炼过程的 Prompt、系统指令与原始返回"
                              >
                                <Code size={12} />
                                <span>{showDistillTrace ? "收起提示词与追踪" : `提示词调优与交互追踪 (${distillTrace.elapsed_ms}ms)`}</span>
                              </button>
                            )}
                            <span className="text-[10.5px] px-2 py-0.5 rounded-full bg-purple-500/20 text-purple-200 font-mono">
                              系统: {distilledPreview.system_name || "通用业务系统"}
                            </span>
                          </div>
                        </div>

                        {/* 针对性补充提示词微调栏 */}
                        <div className="p-2.5 rounded-lg bg-panel/90 border border-purple-500/30 flex flex-col gap-2">
                          <div className="flex items-center justify-between text-[11.5px] text-purple-300 font-medium">
                            <span className="flex items-center gap-1.5">
                              <Edit3 size={12} className="text-purple-400" />
                              <span>提炼结果针对性微调 (补充提示词 Steering Prompt)</span>
                            </span>
                            <span className="text-inkdim text-[10.5px]">输入针对性指令以指导大模型纠正登录 URL、凭据字段或弹窗选项</span>
                          </div>
                          <div className="flex items-center gap-2">
                            <input
                              type="text"
                              value={refinePrompt}
                              onChange={(e) => setRefinePrompt(e.target.value)}
                              onKeyDown={(e) => {
                                if (e.key === "Enter" && !e.shiftKey) {
                                  e.preventDefault();
                                  void handleRedistill();
                                }
                              }}
                              placeholder="例如：登录地址请使用 http://localhost:8080/#/login；弹窗中必须点选呼吸内科；账号使用 doctor_test..."
                              className="flex-1 bg-panel2 border border-edge/80 rounded-md px-3 py-1.5 text-[11.5px] text-ink outline-none focus:border-purple-400 placeholder:text-inkdim/50"
                            />
                            <button
                              type="button"
                              onClick={() => void handleRedistill()}
                              disabled={distilling}
                              className="px-3 py-1.5 rounded-md bg-purple-600 hover:bg-purple-700 text-white font-medium text-[11.5px] flex items-center gap-1.5 shrink-0 cursor-pointer shadow-xs transition-colors disabled:opacity-50"
                              title="携带此补充要求让大模型重新提炼契约"
                            >
                              {distilling ? <Loader2 size={12} className="animate-spin" /> : <Sparkles size={12} />}
                              <span>{distilling ? "提炼中…" : "携带补充提示词重新提炼"}</span>
                            </button>
                          </div>
                        </div>

                        {/* 大模型交互与调试追踪面板 (支持调整系统提示词) */}
                        {showDistillTrace && distillTrace && (
                          <div className="p-3 rounded-xl border border-purple-500/40 bg-black/40 flex flex-col gap-2.5 text-[11.5px] animate-in fade-in duration-150">
                            <div className="flex items-center justify-between text-purple-300 font-medium flex-wrap gap-2">
                              <span className="flex items-center gap-1.5">
                                <Terminal size={13} className="text-purple-400" />
                                <span>大模型交互调试追踪 (LLM Interaction Trace)</span>
                                {(distillTrace.provider_name || distillTrace.model_name) && (
                                  <span className="ml-1 text-[10.5px] px-1.5 py-0.5 rounded bg-purple-500/30 text-purple-200 font-mono">
                                    {distillTrace.provider_name} / {distillTrace.model_name}
                                  </span>
                                )}
                              </span>
                              <span className="font-mono text-[10.5px] text-purple-300/80">耗时: {distillTrace.elapsed_ms} ms</span>
                            </div>

                            <div className="space-y-2">
                              {/* 1. 提炼系统指令 (可编辑可调整) */}
                              <details className="bg-panel/70 border border-edge/60 rounded-lg p-2 text-inkdim" open>
                                <summary className="cursor-pointer font-medium text-ink flex items-center justify-between text-[11px]">
                                  <div className="flex items-center gap-1.5 text-purple-300">
                                    <Edit3 size={11} className="text-purple-400" />
                                    <span>1. 提炼系统指令 (System Prompt，支持编辑调整)</span>
                                  </div>
                                  <span className="text-[10px] text-inkdim font-mono">点击收起/展开</span>
                                </summary>
                                <div className="mt-2 flex flex-col gap-2">
                                  <textarea
                                    value={customSystemPrompt || distillTrace.system_prompt}
                                    onChange={(e) => setCustomSystemPrompt(e.target.value)}
                                    rows={8}
                                    className="w-full p-2.5 bg-black/60 rounded-md border border-edge/80 text-[11px] font-mono whitespace-pre-wrap break-all text-ink focus:border-purple-400 outline-none leading-relaxed"
                                    placeholder="编辑契约提炼规则与输出 JSON 格式规范..."
                                  />
                                  <div className="flex items-center justify-between flex-wrap gap-2">
                                    <button
                                      type="button"
                                      onClick={() => void handleResetDefaultSystemPrompt()}
                                      className="px-2.5 py-1 rounded text-[10.5px] text-inkdim hover:text-ink hover:bg-panel3 border border-edge/60 transition-colors cursor-pointer"
                                      title="恢复内置的默认免登契约提炼专家指令"
                                    >
                                      恢复默认系统提示词
                                    </button>
                                    <button
                                      type="button"
                                      onClick={() => void handleRedistill({ systemPrompt: customSystemPrompt })}
                                      disabled={distilling}
                                      className="px-3 py-1 rounded bg-purple-600 hover:bg-purple-700 text-white font-medium text-[11px] flex items-center gap-1 cursor-pointer transition-colors disabled:opacity-50 shadow-xs"
                                      title="使用当前编辑的系统提示词重新向大模型发起提炼"
                                    >
                                      {distilling ? <Loader2 size={11} className="animate-spin" /> : <Play size={11} />}
                                      <span>应用此系统提示词重新提炼</span>
                                    </button>
                                  </div>
                                </div>
                              </details>

                              <details className="bg-panel/70 border border-edge/60 rounded-lg p-2 text-inkdim" open>
                                <summary className="cursor-pointer font-medium text-ink flex items-center justify-between text-[11px]">
                                  <span>2. 输入大模型的原始轨迹与上下文 (User Prompt)</span>
                                  <span className="text-[10px] text-inkdim font-mono">点击收起</span>
                                </summary>
                                <pre className="mt-1.5 p-2 bg-black/50 rounded text-[10.5px] font-mono whitespace-pre-wrap break-all text-ink">
                                  {distillTrace.user_prompt}
                                </pre>
                              </details>

                              <details className="bg-panel/70 border border-edge/60 rounded-lg p-2 text-inkdim" open>
                                <summary className="cursor-pointer font-medium text-ink flex items-center justify-between text-[11px]">
                                  <span>3. 大模型原始返回内容 (Raw Model Response)</span>
                                  <span className="text-[10px] text-inkdim font-mono">点击收起</span>
                                </summary>
                                <pre className="mt-1.5 p-2 bg-black/50 rounded text-[10.5px] font-mono whitespace-pre-wrap break-all text-emerald-300">
                                  {distillTrace.raw_response}
                                </pre>
                              </details>
                            </div>
                          </div>
                        )}

                        <div className="grid grid-cols-1 sm:grid-cols-3 gap-2.5 text-[11.5px]">
                          <div className="p-2 rounded-lg bg-panel border border-edge/60 flex flex-col gap-0.5">
                            <span className="text-inkdim text-[10.5px]">登录地址 (login_url)</span>
                            <span className="font-mono text-ink truncate" title={distilledPreview.login_url}>{distilledPreview.login_url}</span>
                          </div>
                          <div className="p-2 rounded-lg bg-panel border border-edge/60 flex flex-col gap-0.5">
                            <span className="text-inkdim text-[10.5px]">账号用户名 (username)</span>
                            <span className="font-medium text-ink truncate">{distilledPreview.credentials?.username || "(未检测到)"}</span>
                          </div>
                          <div className="p-2 rounded-lg bg-panel border border-edge/60 flex flex-col gap-0.5">
                            <span className="text-inkdim text-[10.5px]">成功断言路由 (success_assert_url)</span>
                            <span className="font-mono text-ink truncate" title={distilledPreview.post_login?.success_assert_url || "(自动检测)"}>
                              {distilledPreview.post_login?.success_assert_url || "(自动检测)"}
                            </span>
                          </div>
                        </div>

                        {/* 通用后置动作预览 */}
                        {distilledPreview.post_login?.actions && distilledPreview.post_login.actions.length > 0 && (
                          <div className="flex flex-col gap-1.5 text-[11.5px]">
                            <span className="font-medium text-inkdim flex items-center gap-1.5">
                              <Layers size={13} className="text-purple-400" />
                              <span>提炼的登录后置动作序列 ({distilledPreview.post_login.actions.length} 项)</span>
                            </span>
                            <div className="space-y-1.5 max-h-40 overflow-y-auto pr-1">
                              {distilledPreview.post_login.actions.map((act, i) => (
                                <div key={i} className="p-2 rounded-lg bg-panel/80 border border-edge/80 flex items-center justify-between gap-2">
                                  <div className="flex items-center gap-2 min-w-0">
                                    <span className="w-5 h-5 rounded-full bg-purple-500/20 text-purple-300 text-[10px] flex items-center justify-center font-mono font-bold shrink-0">
                                      {act.step || i + 1}
                                    </span>
                                    <span className="text-[10px] px-1.5 py-0.5 rounded bg-panel3 text-inkdim font-mono uppercase shrink-0">
                                      {act.action_type}
                                    </span>
                                    <span className="text-ink truncate font-medium">{act.desc || "执行操作"}</span>
                                    {act.match_text && (
                                      <span className="text-[11px] text-accent font-medium bg-accent/10 px-1.5 py-0.2 rounded shrink-0">
                                        "{act.match_text}"
                                      </span>
                                    )}
                                    {act.dialog_title && (
                                      <span className="text-[10.5px] text-inkdim truncate">
                                        (弹窗: {act.dialog_title})
                                      </span>
                                    )}
                                  </div>
                                </div>
                              ))}
                            </div>
                          </div>
                        )}

                        <div className="flex items-center justify-between pt-2 border-t border-purple-500/20">
                          <span className="text-[11px] text-purple-300/80">
                            提炼参数已就地同步至下方契约表单，可在下方直接微调动作与参数。
                          </span>
                          <div className="flex items-center gap-2">
                            <button
                              type="button"
                              onClick={() => setDistilledPreview(null)}
                              className="px-2.5 py-1.5 rounded-lg border border-edge text-inkdim hover:text-ink text-[11px] cursor-pointer hover:bg-panel3 transition-colors"
                            >
                              忽略提炼结果
                            </button>
                            <button
                              type="button"
                              onClick={() => {
                                pushToast("已确认采用 AI 提炼契约内容，可直接点击右上角【保存契约】生效", "success");
                                setDistilledPreview(null);
                              }}
                              className="px-3 py-1.5 rounded-lg bg-purple-600 hover:bg-purple-700 text-white font-medium text-[11.5px] flex items-center gap-1 cursor-pointer transition-colors shadow-xs"
                            >
                              <CheckCircle2 size={13} />
                              <span>确认采用</span>
                            </button>
                          </div>
                        </div>
                      </div>
                    )}

                    {/* 录制捕获轨迹展开面板 (就地嵌入契约卡片内) */}
                    {showRecordedSteps && recordedSteps.length > 0 && (
                      <div className="p-3.5 rounded-xl border border-edge bg-panel flex flex-col gap-3 shadow-xs animate-in fade-in duration-150 text-[12px]">
                        <div className="flex items-center justify-between border-b border-edge/60 pb-2">
                          <div className="flex items-center gap-2 font-semibold text-ink text-[12.5px]">
                            <List size={14} className="text-accent" />
                            <span>原始交互捕获轨迹 (共 {recordedSteps.length} 步操作)</span>
                          </div>
                          <div className="flex items-center gap-2">
                            <button
                              type="button"
                              onClick={() => void handleRedistill()}
                              disabled={distilling}
                              className="px-3 py-1 rounded-lg bg-gradient-to-r from-purple-600 to-indigo-600 text-white font-medium text-[11px] flex items-center gap-1 cursor-pointer hover:opacity-95 shadow-xs transition-opacity"
                              title="使用当前捕获的操作轨迹与补充提示词重新交由大模型进行提炼"
                            >
                              {distilling ? <Loader2 size={12} className="animate-spin" /> : <Sparkles size={12} />}
                              <span>重新交由 AI 提炼</span>
                            </button>
                            <button
                              type="button"
                              onClick={() => setShowRecordedSteps(false)}
                              className="text-inkdim hover:text-ink text-[11px] px-2 py-1 rounded border border-edge cursor-pointer hover:bg-panel3"
                            >
                              收起
                            </button>
                          </div>
                        </div>

                        <div className="max-h-56 overflow-y-auto space-y-1.5 pr-1 divide-y divide-edge/30">
                          {recordedSteps.map((s, idx) => (
                            <div key={idx} className="pt-1.5 flex items-center justify-between gap-3 text-[11.5px]">
                              <div className="flex items-center gap-2 min-w-0">
                                <span className="w-5 h-5 rounded-full bg-panel2 text-[10px] flex items-center justify-center font-mono font-bold text-ink shrink-0 border border-edge">
                                  {s.order || idx + 1}
                                </span>
                                <span className={`text-[10px] px-1.5 py-0.5 rounded font-mono uppercase font-semibold shrink-0 ${
                                  s.action === "type" ? "bg-blue-500/20 text-blue-400" :
                                  s.action === "click" ? "bg-purple-500/20 text-purple-400" :
                                  s.action === "wait" ? "bg-amber-500/20 text-amber-400" :
                                  "bg-panel2 text-inkdim"
                                }`}>
                                  {s.action}
                                </span>
                                <span className="text-ink truncate font-mono text-[11px]" title={s.selector || s.desc || "(页面整体)"}>
                                  {s.selector || s.desc || "(页面整体)"}
                                </span>
                              </div>
                              <div className="flex items-center gap-2 shrink-0">
                                {s.value && (
                                  <span className="text-[11px] font-mono px-2 py-0.5 rounded bg-panel2 border border-edge text-emerald-400">
                                    {s.is_encrypted ? "•••••••• (已掩码)" : s.value}
                                  </span>
                                )}
                                {s.ms && (
                                  <span className="text-[10px] text-inkdim font-mono">
                                    +{s.ms}ms
                                  </span>
                                )}
                              </div>
                            </div>
                          ))}
                        </div>
                      </div>
                    )}

                    {/* 表单字段区 */}
                    <div className="grid grid-cols-1 sm:grid-cols-2 gap-4 text-[12px]">
                      {/* 系统名称/标识 */}
                      <div className="flex flex-col gap-1.5 sm:col-span-2">
                        <label className="text-ink font-medium flex items-center justify-between">
                          <span>业务系统名称 / 标识 (system_name, 可选)</span>
                          <span className="text-[11px] text-inkdim font-normal">例如: 智慧医务管理系统、HIS门诊系统</span>
                        </label>
                        <input
                          type="text"
                          value={contractSystemName}
                          onChange={(e) => setContractSystemName(e.target.value)}
                          placeholder="例如: 智慧医务管理平台"
                          className="bg-panel border border-edge focus:border-accent rounded-lg px-3 py-2 text-ink outline-none text-[12px]"
                        />
                      </div>

                      {/* 登录起点 URL */}
                      <div className="flex flex-col gap-1.5 sm:col-span-2">
                        <label className="text-ink font-medium flex items-center justify-between">
                          <span>登录地址 (login_url) *</span>
                          <span className="text-[11px] text-inkdim font-normal">流水线自愈起点，绝不使用业务终态首页</span>
                        </label>
                        <input
                          type="text"
                          value={contractLoginUrl}
                          onChange={(e) => setContractLoginUrl(e.target.value)}
                          placeholder="例如: http://127.0.0.1:8080/#/login"
                          className="bg-panel border border-edge focus:border-accent rounded-lg px-3 py-2 text-ink font-mono outline-none text-[12px]"
                        />
                      </div>

                      {/* 账号 */}
                      <div className="flex flex-col gap-1.5">
                        <label className="text-ink font-medium">
                          登录用户名 / 账号 (username) *
                        </label>
                        <input
                          type="text"
                          value={contractUsername}
                          onChange={(e) => setContractUsername(e.target.value)}
                          placeholder="例如: admin 或 zyt"
                          className="bg-panel border border-edge focus:border-accent rounded-lg px-3 py-2 text-ink outline-none text-[12px]"
                        />
                      </div>

                      {/* 密码 */}
                      <div className="flex flex-col gap-1.5">
                        <label className="text-ink font-medium flex items-center justify-between">
                          <span>登录密码 (password) *</span>
                          {authContract && (
                            <span className="text-[10.5px] text-emerald-400 font-normal">
                              已加密配置 (留空保持不变)
                            </span>
                          )}
                        </label>
                        <input
                          type="password"
                          value={contractPassword}
                          onChange={(e) => setContractPassword(e.target.value)}
                          placeholder={authContract ? "•••••••• (已通过 AES-256 加密保存)" : "输入登录密码"}
                          className="bg-panel border border-edge focus:border-accent rounded-lg px-3 py-2 text-ink outline-none text-[12px]"
                        />
                      </div>

                      {/* 通用后置动作序列 (可视化查看与手动编辑) */}
                      <div className="flex flex-col gap-2.5 sm:col-span-2 p-3.5 rounded-xl bg-panel/70 border border-edge">
                        <div className="flex items-center justify-between">
                          <label className="text-ink font-medium flex items-center gap-1.5 text-[12.5px]">
                            <Layers size={14} className="text-accent" />
                            <span>通用后置动作序列 (actions, 支持科室/机构/弹窗处理)</span>
                          </label>
                          <div className="flex items-center gap-2">
                            <button
                              type="button"
                              onClick={handleAddAction}
                              className="px-2.5 py-1 rounded-md bg-accent/15 hover:bg-accent/25 text-accent text-[11px] font-medium flex items-center gap-1 cursor-pointer transition-colors border border-accent/30"
                              title="手动添加一个通用后置动作"
                            >
                              <Plus size={12} />
                              <span>添加动作</span>
                            </button>
                            {contractActions.length > 0 && (
                              <button
                                type="button"
                                onClick={() => setContractActions([])}
                                className="text-[10.5px] text-red-400 hover:text-red-300 flex items-center gap-1 cursor-pointer px-2 py-1 rounded hover:bg-red-500/10 transition-colors"
                              >
                                <Trash2 size={11} />
                                <span>清空全部</span>
                              </button>
                            )}
                          </div>
                        </div>

                        {contractActions.length === 0 ? (
                          <div className="text-[11.5px] text-inkdim py-4 text-center border border-dashed border-edge/60 rounded-lg flex flex-col items-center gap-2">
                            <span>暂无通用后置动作序列。</span>
                            <div className="flex items-center gap-2">
                              <button
                                type="button"
                                onClick={handleAddAction}
                                className="px-3 py-1 rounded-md border border-edge hover:bg-panel3 text-ink text-[11px] cursor-pointer"
                              >
                                + 手动添加动作
                              </button>
                              <span>或点击卡片右上角【✨ AI 智能录制填入】由大模型自动提炼</span>
                            </div>
                          </div>
                        ) : (
                          <div className="space-y-2">
                            {contractActions.map((act, idx) => {
                              const isEditing = editingActionIndex === idx;
                              return (
                                <div key={idx} className="rounded-lg bg-panel border border-edge overflow-hidden transition-all shadow-xs">
                                  {/* 动作头部摘要 */}
                                  <div className="p-2.5 flex items-center justify-between gap-2 text-[11.5px] bg-panel2/60">
                                    <div className="flex items-center gap-2 min-w-0 flex-wrap">
                                      <span className="w-5 h-5 rounded-full bg-accent/20 text-accent text-[10px] flex items-center justify-center font-mono font-bold shrink-0">
                                        #{act.step || idx + 1}
                                      </span>
                                      <span className={`text-[9.5px] px-1.5 py-0.5 rounded font-mono uppercase font-semibold shrink-0 ${
                                        act.action_type === "select_modal_item" ? "bg-blue-500/20 text-blue-400" :
                                        act.action_type === "dismiss_notice" ? "bg-amber-500/20 text-amber-400" :
                                        act.action_type === "click" ? "bg-purple-500/20 text-purple-400" :
                                        "bg-panel3 text-inkdim"
                                      }`}>
                                        {act.action_type}
                                      </span>
                                      <span className="text-ink font-medium truncate max-w-[200px]" title={act.desc || `动作 #${act.step}`}>
                                        {act.desc || `步骤 #${act.step}`}
                                      </span>
                                      {act.match_text && (
                                        <span className="text-[11px] text-accent font-medium bg-accent/10 px-1.5 py-0.2 rounded shrink-0">
                                          目标: "{act.match_text}"
                                        </span>
                                      )}
                                      {act.dialog_title && (
                                        <span className="text-[10.5px] text-inkdim truncate max-w-[150px]">
                                          (弹窗: {act.dialog_title})
                                        </span>
                                      )}
                                      {act.confirm_btn && (
                                        <span className="text-[10.5px] text-emerald-400 bg-emerald-500/10 px-1.5 py-0.2 rounded shrink-0">
                                          确认键: "{act.confirm_btn}"
                                        </span>
                                      )}
                                    </div>

                                    {/* 动作控制按钮 */}
                                    <div className="flex items-center gap-1 shrink-0">
                                      <button
                                        type="button"
                                        onClick={() => setEditingActionIndex(isEditing ? null : idx)}
                                        className={`px-2 py-0.5 rounded text-[10.5px] font-medium transition-colors cursor-pointer ${
                                          isEditing ? "bg-accent text-white" : "text-inkdim hover:text-ink hover:bg-panel3"
                                        }`}
                                      >
                                        {isEditing ? "收起" : "编辑"}
                                      </button>
                                      <button
                                        type="button"
                                        onClick={() => handleMoveAction(idx, "up")}
                                        disabled={idx === 0}
                                        className="p-1 text-inkdim hover:text-ink disabled:opacity-30 cursor-pointer"
                                        title="上移"
                                      >
                                        <ChevronUp size={13} />
                                      </button>
                                      <button
                                        type="button"
                                        onClick={() => handleMoveAction(idx, "down")}
                                        disabled={idx === contractActions.length - 1}
                                        className="p-1 text-inkdim hover:text-ink disabled:opacity-30 cursor-pointer"
                                        title="下移"
                                      >
                                        <ChevronDown size={13} />
                                      </button>
                                      <button
                                        type="button"
                                        onClick={() => handleDeleteAction(idx)}
                                        className="p-1 text-inkdim hover:text-red-400 cursor-pointer transition-colors"
                                        title="删除此步骤"
                                      >
                                        <X size={13} />
                                      </button>
                                    </div>
                                  </div>

                                  {/* 展开的编辑表单 */}
                                  {isEditing && (
                                    <div className="p-3 border-t border-edge/60 bg-panel/90 grid grid-cols-1 sm:grid-cols-2 gap-3 text-[11.5px] animate-in fade-in duration-100">
                                      <div className="flex flex-col gap-1">
                                        <label className="text-ink font-medium">动作类型 (action_type) *</label>
                                        <select
                                          value={act.action_type}
                                          onChange={(e) => handleUpdateAction(idx, { action_type: e.target.value })}
                                          className="bg-panel2 border border-edge rounded-md px-2.5 py-1.5 text-ink outline-none"
                                        >
                                          <option value="select_modal_item">🏢 select_modal_item (弹窗选择科室/机构/角色)</option>
                                          <option value="dismiss_notice">📜 dismiss_notice (关闭提示/公告/协议)</option>
                                          <option value="click">🖱️ click (原生点击特定按钮或元素)</option>
                                          <option value="wait">⏳ wait (等待页面加载与动画沉降)</option>
                                        </select>
                                      </div>

                                      <div className="flex flex-col gap-1">
                                        <label className="text-ink font-medium">动作意图描述 (desc)</label>
                                        <input
                                          type="text"
                                          value={act.desc || ""}
                                          onChange={(e) => handleUpdateAction(idx, { desc: e.target.value })}
                                          placeholder="例如: 在科室弹窗中选中心内科"
                                          className="bg-panel2 border border-edge rounded-md px-2.5 py-1.5 text-ink outline-none"
                                        />
                                      </div>

                                      <div className="flex flex-col gap-1">
                                        <label className="text-ink font-medium">所属弹窗标题 (dialog_title, 可选)</label>
                                        <input
                                          type="text"
                                          value={act.dialog_title || ""}
                                          onChange={(e) => handleUpdateAction(idx, { dialog_title: e.target.value })}
                                          placeholder="例如: 选择科室、机构切换、系统须知"
                                          className="bg-panel2 border border-edge rounded-md px-2.5 py-1.5 text-ink outline-none"
                                        />
                                      </div>

                                      <div className="flex flex-col gap-1">
                                        <label className="text-ink font-medium">目标匹配文本 (match_text, 可选)</label>
                                        <input
                                          type="text"
                                          value={act.match_text || ""}
                                          onChange={(e) => handleUpdateAction(idx, { match_text: e.target.value })}
                                          placeholder="例如: 心内科、扬州市江都区卫生健康委员会"
                                          className="bg-panel2 border border-edge rounded-md px-2.5 py-1.5 text-ink outline-none font-mono"
                                        />
                                      </div>

                                      <div className="flex flex-col gap-1">
                                        <label className="text-ink font-medium">随附确认按钮 (confirm_btn, 可选)</label>
                                        <input
                                          type="text"
                                          value={act.confirm_btn || ""}
                                          onChange={(e) => handleUpdateAction(idx, { confirm_btn: e.target.value })}
                                          placeholder="例如: 确定、我知道了、下一步"
                                          className="bg-panel2 border border-edge rounded-md px-2.5 py-1.5 text-ink outline-none"
                                        />
                                      </div>

                                      <div className="flex flex-col gap-1">
                                        <label className="text-ink font-medium">DOM 选择器 (selector, 可选)</label>
                                        <input
                                          type="text"
                                          value={act.selector || ""}
                                          onChange={(e) => handleUpdateAction(idx, { selector: e.target.value })}
                                          placeholder="例如: .submit-btn 或 #deptSelect"
                                          className="bg-panel2 border border-edge rounded-md px-2.5 py-1.5 text-ink font-mono outline-none"
                                        />
                                      </div>

                                      <div className="sm:col-span-2 flex justify-end pt-1">
                                        <button
                                          type="button"
                                          onClick={() => setEditingActionIndex(null)}
                                          className="px-3 py-1 rounded bg-panel3 hover:bg-panel text-ink text-[11px] font-medium cursor-pointer transition-colors border border-edge"
                                        >
                                          ✓ 完成本步修改
                                        </button>
                                      </div>
                                    </div>
                                  )}
                                </div>
                              );
                            })}
                          </div>
                        )}
                      </div>

                      {/* 成功断言路由（可选） */}
                      <div className="flex flex-col gap-1.5 sm:col-span-2">
                        <label className="text-ink font-medium flex items-center justify-between">
                          <span>成功断言路由规则 (success_assert_url, 可选)</span>
                          <span className="text-[11px] text-inkdim font-normal">支持正则或管道符匹配，留空则自动检测</span>
                        </label>
                        <input
                          type="text"
                          value={contractAssertUrl}
                          onChange={(e) => setContractAssertUrl(e.target.value)}
                          placeholder="例如: #/index|#/medicalAffairManage"
                          className="bg-panel border border-edge focus:border-accent rounded-lg px-3 py-2 text-ink font-mono outline-none text-[12px]"
                        />
                      </div>
                    </div>

                    {/* 表单底部操作栏 */}
                    <div className="flex items-center justify-between pt-2 border-t border-edge/60 mt-1">
                      <div className="text-[11.5px] text-inkdim flex items-center gap-1.5">
                        <Lock size={12} className="text-emerald-400 shrink-0" />
                        <span>凭据在工作区本地加密落盘 (.harness/auth_contract.json)，密码 AES-256 加密，自动加入 .gitignore</span>
                      </div>

                      <div className="flex items-center gap-2">
                        <button
                          onClick={() => void handleSaveContract()}
                          disabled={savingContract}
                          className="px-3.5 py-1.5 rounded-lg bg-emerald-600 text-white hover:bg-emerald-700 font-medium text-[12px] flex items-center gap-1.5 cursor-pointer shadow-xs transition-colors"
                        >
                          {savingContract ? <Loader2 size={13} className="animate-spin" /> : <Save size={13} />}
                          <span>{savingContract ? "保存中…" : "保存凭据契约"}</span>
                        </button>

                        <button
                          onClick={() => void handleRunPipeline()}
                          disabled={runningPipeline}
                          className="px-3.5 py-1.5 rounded-lg bg-accent text-white hover:bg-accent/90 font-medium text-[12px] flex items-center gap-1.5 cursor-pointer shadow-xs transition-colors"
                        >
                          {runningPipeline ? <Loader2 size={13} className="animate-spin" /> : <Play size={13} />}
                          <span>{runningPipeline ? "正在执行…" : "测试免登流水线"}</span>
                        </button>
                      </div>
                    </div>
                  </div>

                  {/* 状态辅助诊断卡片（置于契约卡片下方作为状态透传与健康度指标） */}
                  <div className="grid grid-cols-1 sm:grid-cols-3 gap-3">
                    {/* 卡片 1: 原生持久化 Profile */}
                    <div className="p-3.5 rounded-xl border border-edge bg-panel2 flex flex-col gap-1.5 shadow-xs">
                      <div className="flex items-center justify-between">
                        <span className="text-[12px] font-semibold text-ink flex items-center gap-1.5">
                          <Database size={14} className="text-blue-400" />
                          <span>原生持久化 Profile</span>
                        </span>
                        <span className="text-[10px] px-2 py-0.5 rounded-full bg-emerald-500/15 text-emerald-400 border border-emerald-500/30 font-medium">
                          🟢 已启用
                        </span>
                      </div>
                      <div className="text-[11px] text-inkdim leading-relaxed">
                        基于 Chromium <code className="font-mono bg-panel px-1 rounded">--user-data-dir</code> 原生存储，Cookies、LocalStorage、IndexedDB 物理落盘，零 JS 注水污染。
                      </div>
                    </div>

                    {/* 卡片 2: 声明式登录契约 */}
                    <div className="p-3.5 rounded-xl border border-edge bg-panel2 flex flex-col gap-1.5 shadow-xs">
                      <div className="flex items-center justify-between">
                        <span className="text-[12px] font-semibold text-ink flex items-center gap-1.5">
                          <KeyRound size={14} className="text-amber-400" />
                          <span>声明式登录契约</span>
                        </span>
                        {authContract ? (
                          <span className="text-[10px] px-2 py-0.5 rounded-full bg-emerald-500/15 text-emerald-400 border border-emerald-500/30 font-medium">
                            🟢 已配置 ({authContract.version || "v3.0"})
                          </span>
                        ) : (
                          <span className="text-[10px] px-2 py-0.5 rounded-full bg-amber-500/15 text-amber-400 border border-amber-500/30 font-medium">
                            🟡 未配置
                          </span>
                        )}
                      </div>
                      <div className="text-[11px] text-inkdim leading-relaxed">
                        声明式定义账号密码、科室/机构多阶动作。<code className="font-mono bg-panel px-1 rounded">.harness/auth_contract.json</code> 密码字段采用 AES-256-GCM 强加密。
                      </div>
                    </div>

                    {/* 卡片 3: 语义页面状态机 */}
                    <div className="p-3.5 rounded-xl border border-edge bg-panel2 flex flex-col gap-1.5 shadow-xs">
                      <div className="flex items-center justify-between">
                        <span className="text-[12px] font-semibold text-ink flex items-center gap-1.5">
                          <Activity size={14} className="text-emerald-400" />
                          <span>页面语义健康度</span>
                        </span>
                        {debugState?.page_state?.type === "Ready" ? (
                          <span className="text-[10px] px-2 py-0.5 rounded-full bg-emerald-500/15 text-emerald-400 border border-emerald-500/30 font-medium">
                            🟢 Ready
                          </span>
                        ) : debugState?.page_state?.type === "Unauthenticated" ? (
                          <span className="text-[10px] px-2 py-0.5 rounded-full bg-amber-500/15 text-amber-400 border border-amber-500/30 font-medium">
                            🟡 Unauthenticated
                          </span>
                        ) : debugState?.page_state?.type === "ActionBlocked" ? (
                          <span className="text-[10px] px-2 py-0.5 rounded-full bg-purple-500/15 text-purple-300 border border-purple-500/30 font-medium">
                            🟣 ActionBlocked
                          </span>
                        ) : (
                          <span className="text-[10px] px-2 py-0.5 rounded-full bg-panel3 text-inkdim border border-edge font-medium">
                            ⚪ 未连接
                          </span>
                        )}
                      </div>
                      <div className="text-[11px] text-inkdim leading-relaxed">
                        解耦 CDP 状态码，直接深度扫描 SPA 路由、未授权弹窗遮罩与组织选择弹窗，为自愈与 HITL 提供确定性输入。
                      </div>
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
