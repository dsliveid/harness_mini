import { useEffect, useState, useMemo } from "react";
import { ipc } from "../ipc";
import { useStore } from "../store";
import type {
  ToolLogFilter,
  ToolLogItem,
  ToolLogQueryResult,
  TopErrorSummary,
  JevDecisionEvent,
  JevEventFilter,
  JevEventQueryResult,
  JevStatsSummary,
} from "../types";
import {
  X,
  RotateCcw,
  ScrollText,
  Search,
  Filter,
  ChevronLeft,
  ChevronRight,
  Copy,
  Check,
  ExternalLink,
  AlertTriangle,
  CheckCircle2,
  XCircle,
  Clock,
  Calendar,
  Layers,
  Wrench,
  Brain,
  MessageSquare,
  Sparkles,
  Bug,
  Zap,
  ShieldAlert,
} from "./Icons";

function formatTimestamp(isoStr?: string | null): string {
  if (!isoStr) return "-";
  try {
    const d = new Date(isoStr);
    if (isNaN(d.getTime())) return isoStr;
    const pad = (n: number) => n.toString().padStart(2, "0");
    const month = pad(d.getMonth() + 1);
    const day = pad(d.getDate());
    const hours = pad(d.getHours());
    const minutes = pad(d.getMinutes());
    const seconds = pad(d.getSeconds());
    return `${month}-${day} ${hours}:${minutes}:${seconds}`;
  } catch {
    return isoStr;
  }
}

function statusBadge(status: string) {
  switch (status) {
    case "success":
      return (
        <span className="inline-flex items-center gap-1 text-[11px] px-2 py-0.5 rounded-full bg-emerald-500/15 text-emerald-400 border border-emerald-500/30 shrink-0 font-medium">
          <CheckCircle2 size={11} />
          <span>成功</span>
        </span>
      );
    case "failed":
      return (
        <span className="inline-flex items-center gap-1 text-[11px] px-2 py-0.5 rounded-full bg-red-500/15 text-red-400 border border-red-500/30 shrink-0 font-medium">
          <XCircle size={11} />
          <span>失败</span>
        </span>
      );
    case "denied":
      return (
        <span className="inline-flex items-center gap-1 text-[11px] px-2 py-0.5 rounded-full bg-zinc-500/20 text-zinc-300 border border-zinc-500/30 shrink-0 font-medium">
          <AlertTriangle size={11} />
          <span>被拦截/拒绝</span>
        </span>
      );
    case "timeout":
      return (
        <span className="inline-flex items-center gap-1 text-[11px] px-2 py-0.5 rounded-full bg-amber-500/15 text-amber-400 border border-amber-500/30 shrink-0 font-medium">
          <Clock size={11} />
          <span>超时</span>
        </span>
      );
    case "running":
      return (
        <span className="inline-flex items-center gap-1 text-[11px] px-2 py-0.5 rounded-full bg-blue-500/15 text-blue-400 border border-blue-500/30 shrink-0 font-medium animate-pulse">
          <span>执行中</span>
        </span>
      );
    case "pending_approval":
      return (
        <span className="inline-flex items-center gap-1 text-[11px] px-2 py-0.5 rounded-full bg-amber-500/15 text-amber-400 border border-amber-500/30 shrink-0 font-medium">
          <span>待审批</span>
        </span>
      );
    default:
      return (
        <span className="text-[11px] px-2 py-0.5 rounded-full bg-panel3 text-inkdim shrink-0">
          {status}
        </span>
      );
  }
}

function jevSceneBadge(scene: string) {
  switch (scene) {
    case "task_complexity":
      return (
        <span className="inline-flex items-center gap-1 text-[11px] px-2 py-0.5 rounded-full bg-amber-500/15 text-amber-300 border border-amber-500/30 shrink-0 font-medium">
          <Zap size={11} className="text-amber-400 fill-amber-400" />
          <span>任务复杂度</span>
        </span>
      );
    case "command_guard":
      return (
        <span className="inline-flex items-center gap-1 text-[11px] px-2 py-0.5 rounded-full bg-red-500/15 text-red-300 border border-red-500/30 shrink-0 font-medium">
          <ShieldAlert size={11} className="text-red-400" />
          <span>命令风控</span>
        </span>
      );
    case "memory_gate":
      return (
        <span className="inline-flex items-center gap-1 text-[11px] px-2 py-0.5 rounded-full bg-teal-500/15 text-teal-300 border border-teal-500/30 shrink-0 font-medium">
          <Brain size={11} className="text-teal-400" />
          <span>记忆质检</span>
        </span>
      );
    case "auto_distill":
      return (
        <span className="inline-flex items-center gap-1 text-[11px] px-2 py-0.5 rounded-full bg-purple-500/15 text-purple-300 border border-purple-500/30 shrink-0 font-medium">
          <Sparkles size={11} className="text-purple-400" />
          <span>经验提炼</span>
        </span>
      );
    case "plan_review":
      return (
        <span className="inline-flex items-center gap-1 text-[11px] px-2 py-0.5 rounded-full bg-emerald-500/15 text-emerald-300 border border-emerald-500/30 shrink-0 font-medium">
          <CheckCircle2 size={11} className="text-emerald-400" />
          <span>方案审查</span>
        </span>
      );
    default:
      return (
        <span className="text-[11px] px-2 py-0.5 rounded-full bg-panel3 text-inkdim shrink-0">
          {scene}
        </span>
      );
  }
}

function jevVerdictBadge(verdict: string) {
  switch (verdict) {
    case "allow":
      return (
        <span className="inline-flex items-center gap-1 text-[11px] px-2 py-0.5 rounded-full bg-emerald-500/15 text-emerald-400 border border-emerald-500/30 shrink-0 font-medium">
          <CheckCircle2 size={11} />
          <span>放行</span>
        </span>
      );
    case "deny":
      return (
        <span className="inline-flex items-center gap-1 text-[11px] px-2 py-0.5 rounded-full bg-red-500/15 text-red-400 border border-red-500/30 shrink-0 font-medium">
          <XCircle size={11} />
          <span>拦截</span>
        </span>
      );
    case "abstain":
      return (
        <span className="inline-flex items-center gap-1 text-[11px] px-2 py-0.5 rounded-full bg-zinc-500/20 text-zinc-300 border border-zinc-500/30 shrink-0 font-medium">
          <span>弃权</span>
        </span>
      );
    default:
      return (
        <span className="inline-flex items-center gap-1 text-[11px] px-2 py-0.5 rounded-full bg-cyan-500/15 text-cyan-300 border border-cyan-500/30 shrink-0 font-medium">
          <span>{verdict}</span>
        </span>
      );
  }
}

export function LogViewerModal() {
  const show = useStore((s) => s.showLogViewerModal);
  const fromSettings = useStore((s) => s.logViewerFromSettings);
  const initialFilter = useStore((s) => s.logViewerFilter);
  const closeLogViewer = useStore((s) => s.closeLogViewer);
  const projects = useStore((s) => s.projects);
  const selectSession = useStore((s) => s.selectSession);
  const pushToast = useStore((s) => s.pushToast);

  // 状态筛选
  const [selectedProjectId, setSelectedProjectId] = useState<string>("all");
  const [statusFilter, setStatusFilter] = useState<string>("error_only"); // 默认展示异常
  const [toolNameFilter, setToolNameFilter] = useState<string>("all");
  const [keyword, setKeyword] = useState<string>("");
  const [searchInput, setSearchInput] = useState<string>("");
  const [page, setPage] = useState<number>(1);
  const pageSize = 30;

  // 日期范围筛选
  const [datePreset, setDatePreset] = useState<"all" | "today" | "7d" | "30d" | "custom">("all");
  const [startDate, setStartDate] = useState<string>("");
  const [endDate, setEndDate] = useState<string>("");

  // 视图模式：'stream' (调用明细流) | 'clusters' (同款异常聚类) | 'jev' (Jev 决策流水)
  const [activeTab, setActiveTab] = useState<"stream" | "clusters" | "jev">("stream");

  // Jev 数据集与筛选
  const [jevSceneFilter, setJevSceneFilter] = useState<string>("all");
  const [jevVerdictFilter, setJevVerdictFilter] = useState<string>("all");
  const [jevPage, setJevPage] = useState<number>(1);
  const [jevQueryResult, setJevQueryResult] = useState<JevEventQueryResult | null>(null);
  const [jevLoading, setJevLoading] = useState<boolean>(false);
  const [selectedJevId, setSelectedJevId] = useState<string | null>(null);
  const [copiedJevJson, setCopiedJevJson] = useState<boolean>(false);

  // 数据集
  const [queryResult, setQueryResult] = useState<ToolLogQueryResult | null>(null);
  const [loading, setLoading] = useState<boolean>(false);
  const [selectedItemId, setSelectedItemId] = useState<string | null>(null);

  // 复制提示
  const [copiedType, setCopiedType] = useState<"params" | "result" | null>(null);

  // 初始化参数应用
  useEffect(() => {
    if (show && initialFilter) {
      if (initialFilter.projectId) setSelectedProjectId(initialFilter.projectId);
      if (initialFilter.status) setStatusFilter(initialFilter.status);
      if (initialFilter.toolName) setToolNameFilter(initialFilter.toolName);
      if (initialFilter.startDate) setStartDate(initialFilter.startDate);
      if (initialFilter.endDate) setEndDate(initialFilter.endDate);
      if (initialFilter.startDate || initialFilter.endDate) setDatePreset("custom");
      if (initialFilter.keyword) {
        setKeyword(initialFilter.keyword);
        setSearchInput(initialFilter.keyword);
      }
    }
  }, [show, initialFilter]);

  // 日期快捷选项处理
  const handleDatePreset = (preset: "all" | "today" | "7d" | "30d") => {
    setDatePreset(preset);
    const now = new Date();
    const pad = (n: number) => n.toString().padStart(2, "0");
    const formatDate = (d: Date) => `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;

    if (preset === "all") {
      setStartDate("");
      setEndDate("");
    } else if (preset === "today") {
      const todayStr = formatDate(now);
      setStartDate(todayStr);
      setEndDate(todayStr);
    } else if (preset === "7d") {
      const past = new Date();
      past.setDate(past.getDate() - 6);
      setStartDate(formatDate(past));
      setEndDate(formatDate(now));
    } else if (preset === "30d") {
      const past = new Date();
      past.setDate(past.getDate() - 29);
      setStartDate(formatDate(past));
      setEndDate(formatDate(now));
    }
    setPage(1);
  };

  const handleStartDateChange = (val: string) => {
    setStartDate(val);
    setDatePreset("custom");
    setPage(1);
  };

  const handleEndDateChange = (val: string) => {
    setEndDate(val);
    setDatePreset("custom");
    setPage(1);
  };

  const handleClearDate = () => {
    setDatePreset("all");
    setStartDate("");
    setEndDate("");
    setPage(1);
  };

  // 防抖搜索关键词
  useEffect(() => {
    const timer = setTimeout(() => {
      setKeyword(searchInput.trim());
      setPage(1);
    }, 300);
    return () => clearTimeout(timer);
  }, [searchInput]);

  // 查询数据
  const fetchData = async () => {
    setLoading(true);
    try {
      const filter: ToolLogFilter = {
        projectId: selectedProjectId === "all" ? null : selectedProjectId,
        status: statusFilter === "all" ? null : statusFilter,
        toolName: toolNameFilter === "all" ? null : toolNameFilter,
        keyword: keyword || null,
        startDate: startDate || null,
        endDate: endDate || null,
        page,
        pageSize,
      };
      const res = await ipc.queryToolLogs(filter);
      setQueryResult(res);
      // 默认选中第一项
      if (res.items.length > 0) {
        setSelectedItemId((prev) => (prev && res.items.some((i) => i.id === prev) ? prev : res.items[0].id));
      } else {
        setSelectedItemId(null);
      }
    } catch (e) {
      pushToast(`加载日志失败: ${e}`);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    if (show) {
      void fetchData();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [show, selectedProjectId, statusFilter, toolNameFilter, keyword, startDate, endDate, page]);

  // 查询 Jev 决策流水
  const fetchJevData = async () => {
    setJevLoading(true);
    try {
      const filter: JevEventFilter = {
        scene: jevSceneFilter === "all" ? undefined : jevSceneFilter,
        verdict: jevVerdictFilter === "all" ? undefined : jevVerdictFilter,
        limit: pageSize,
        offset: (jevPage - 1) * pageSize,
      };
      const res = await ipc.queryJevEvents(filter);
      setJevQueryResult(res);
      if (res.items.length > 0) {
        setSelectedJevId((prev) => (prev && res.items.some((i) => i.id === prev) ? prev : res.items[0].id));
      } else {
        setSelectedJevId(null);
      }
    } catch (e) {
      pushToast(`加载 Jev 决策流水失败: ${e}`);
    } finally {
      setJevLoading(false);
    }
  };

  useEffect(() => {
    if (show && activeTab === "jev") {
      void fetchJevData();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [show, activeTab, jevSceneFilter, jevVerdictFilter, jevPage]);

  // 当前选中的条目
  const selectedItem = useMemo(() => {
    if (!queryResult || !selectedItemId) return null;
    return queryResult.items.find((item) => item.id === selectedItemId) ?? null;
  }, [queryResult, selectedItemId]);

  const selectedJevItem = useMemo(() => {
    if (!jevQueryResult || !selectedJevId) return null;
    return jevQueryResult.items.find((item) => item.id === selectedJevId) ?? null;
  }, [jevQueryResult, selectedJevId]);

  // 常见已知工具列表备选
  const commonTools = useMemo(() => {
    const set = new Set<string>([
      "read_file",
      "edit_file",
      "write_file",
      "run_command",
      "fetch_web_page",
      "file_outline",
      "list_dir",
      "glob",
      "grep",
      "create_plan",
      "update_plan",
      "spawn_subprocess",
      "wait_subprocesses",
      "record_memory",
      "read_memory",
    ]);
    if (queryResult?.items) {
      for (const it of queryResult.items) {
        set.add(it.toolName);
      }
    }
    return Array.from(set).sort();
  }, [queryResult]);

  if (!show) return null;

  const totalCount = queryResult?.totalCount ?? 0;
  const errorCount = queryResult?.errorCount ?? 0;
  const errorRate = totalCount > 0 ? ((errorCount / totalCount) * 100).toFixed(1) : "0.0";
  const items = queryResult?.items ?? [];
  const topErrors = queryResult?.topErrors ?? [];
  const totalPages = queryResult?.totalPages ?? 1;

  const jevItems = jevQueryResult?.items ?? [];
  const totalJevCount = jevQueryResult?.total ?? 0;
  const totalJevPages = Math.max(1, Math.ceil(totalJevCount / pageSize));
  const jevStats: JevStatsSummary = jevQueryResult?.stats ?? {
    totalCount: 0,
    avgLatencyMs: 0,
    allowCount: 0,
    denyCount: 0,
    abstainCount: 0,
    todayCount: 0,
  };

  const copyText = async (text: string, type: "params" | "result") => {
    try {
      await navigator.clipboard.writeText(text);
      setCopiedType(type);
      setTimeout(() => setCopiedType(null), 1500);
    } catch {
      pushToast("复制失败");
    }
  };

  const handleJumpToSession = (sessionId: string) => {
    selectSession(sessionId);
    closeLogViewer();
    pushToast("已切换至对应对话");
  };

  const handleApplyTopErrorFilter = (toolName: string, errSummary: string) => {
    setToolNameFilter(toolName);
    // 截取前 20 个字符做关键词过滤
    const cleanKw = errSummary.replace(/[\[\]【】()（）]/g, "").trim().slice(0, 15);
    setSearchInput(cleanKw);
    setStatusFilter("error_only");
    setActiveTab("stream");
    setPage(1);
  };

  return (
    <div
      className="fixed inset-0 z-[85] bg-black/60 backdrop-blur-sm flex items-center justify-center animate-in fade-in duration-150"
      onMouseDown={closeLogViewer}
    >
      <div
        className="bg-panel2 border border-edge rounded-2xl w-[1120px] max-w-[96vw] h-[88vh] flex flex-col shadow-2xl overflow-hidden animate-in zoom-in-95 duration-150"
        onMouseDown={(e) => e.stopPropagation()}
      >
        {/* 顶部标题栏 */}
        <div className="h-14 px-6 border-b border-edge/60 flex items-center justify-between shrink-0 bg-panel/70">
          <div className="flex items-center gap-3">
            <div className="w-8 h-8 rounded-xl bg-cyan-500/15 text-cyan-400 flex items-center justify-center border border-cyan-500/25">
              <ScrollText size={18} />
            </div>
            <div>
              <div className="text-[15px] font-semibold text-ink flex items-center gap-2">
                <span>工具调用与异常日志中心</span>
                {loading && <span className="text-[11px] text-cyan-400 font-normal animate-pulse">检索中…</span>}
              </div>
              <div className="text-[11px] text-inkdim">跨会话排查工具调用失败原因、入参出参报文及同款异常模式</div>
            </div>
          </div>

          <div className="flex items-center gap-2">
            {fromSettings && (
              <button
                className="px-2.5 py-1.5 rounded-lg border border-edge hover:bg-panel3 text-inkdim hover:text-ink text-[12px] flex items-center gap-1.5 transition-colors cursor-pointer"
                onClick={closeLogViewer}
                title="返回程序设置"
              >
                <ChevronLeft size={14} />
                <span>返回设置</span>
              </button>
            )}
            <button
              className="px-2.5 py-1.5 rounded-lg border border-edge hover:bg-panel3 text-inkdim hover:text-ink text-[12px] flex items-center gap-1.5 transition-colors cursor-pointer"
              onClick={() => {
                if (activeTab === "jev") void fetchJevData();
                else void fetchData();
              }}
              title={activeTab === "jev" ? "刷新 Jev 决策流水" : "刷新日志数据"}
            >
              <RotateCcw size={13} className={(activeTab === "jev" ? jevLoading : loading) ? "animate-spin" : ""} />
              <span>刷新</span>
            </button>
            <button
              className="w-8 h-8 rounded-lg hover:bg-panel3 flex items-center justify-center text-inkdim hover:text-ink transition-colors cursor-pointer"
              onClick={closeLogViewer}
              title={fromSettings ? "关闭并返回设置" : "关闭"}
            >
              <X size={16} />
            </button>
          </div>
        </div>

        {/* 顶部 KPI 概览卡片 */}
        <div className="px-6 py-3 border-b border-edge/40 bg-panel/30 shrink-0">
          {activeTab === "jev" ? (
            <div className="grid grid-cols-2 sm:grid-cols-4 gap-3">
              <div className="bg-panel border border-edge/70 rounded-xl p-3 shadow-sm">
                <div className="flex items-center justify-between text-inkdim text-[11px] mb-0.5">
                  <span>Jev 决策总次数</span>
                  <Zap size={13} className="text-amber-400" />
                </div>
                <div className="text-xl font-bold text-ink">{jevStats.totalCount.toLocaleString()}</div>
              </div>

              <div className="bg-panel border border-edge/70 rounded-xl p-3 shadow-sm">
                <div className="flex items-center justify-between text-inkdim text-[11px] mb-0.5">
                  <span>平均响应延迟</span>
                  <Clock size={13} className="text-blue-400" />
                </div>
                <div className="text-xl font-bold text-blue-400">{jevStats.avgLatencyMs} ms</div>
              </div>

              <div className="bg-panel border border-edge/70 rounded-xl p-3 shadow-sm">
                <div className="flex items-center justify-between text-inkdim text-[11px] mb-0.5">
                  <span>风控拦截阻断</span>
                  <ShieldAlert size={13} className="text-red-400" />
                </div>
                <div className="text-xl font-bold text-red-400">
                  {jevStats.denyCount} 次
                  {jevStats.totalCount > 0 && (
                    <span className="text-[12px] font-normal text-red-400/80 ml-1.5">
                      ({((jevStats.denyCount / jevStats.totalCount) * 100).toFixed(1)}%)
                    </span>
                  )}
                </div>
              </div>

              <div className="bg-panel border border-edge/70 rounded-xl p-3 shadow-sm">
                <div className="flex items-center justify-between text-inkdim text-[11px] mb-0.5">
                  <span>今日决策 / 弃权数</span>
                  <Sparkles size={13} className="text-purple-400" />
                </div>
                <div className="text-xl font-bold text-purple-400">
                  {jevStats.todayCount} 次今日 · {jevStats.abstainCount} 弃权
                </div>
              </div>
            </div>
          ) : (
            <div className="grid grid-cols-2 sm:grid-cols-4 gap-3">
              <div className="bg-panel border border-edge/70 rounded-xl p-3 shadow-sm">
                <div className="flex items-center justify-between text-inkdim text-[11px] mb-0.5">
                  <span>符合条件总调用</span>
                  <Layers size={13} className="text-accent" />
                </div>
                <div className="text-xl font-bold text-ink">{totalCount.toLocaleString()}</div>
              </div>

              <div className="bg-panel border border-edge/70 rounded-xl p-3 shadow-sm">
                <div className="flex items-center justify-between text-inkdim text-[11px] mb-0.5">
                  <span>异常调用总数</span>
                  <AlertTriangle size={13} className="text-red-400" />
                </div>
                <div className="text-xl font-bold text-red-400">{errorCount.toLocaleString()}</div>
              </div>

              <div className="bg-panel border border-edge/70 rounded-xl p-3 shadow-sm">
                <div className="flex items-center justify-between text-inkdim text-[11px] mb-0.5">
                  <span>异常率</span>
                  <Clock size={13} className="text-amber-400" />
                </div>
                <div className={`text-xl font-bold ${Number(errorRate) > 10 ? "text-amber-400" : "text-emerald-400"}`}>
                  {errorRate}%
                </div>
              </div>

              <div className="bg-panel border border-edge/70 rounded-xl p-3 shadow-sm">
                <div className="flex items-center justify-between text-inkdim text-[11px] mb-0.5">
                  <span>高频同款异常数</span>
                  <Bug size={13} className="text-cyan-400" />
                </div>
                <div className="text-xl font-bold text-cyan-400">{topErrors.length} 组模式</div>
              </div>
            </div>
          )}
        </div>

        {/* 多维筛选控制条 */}
        <div className="px-6 py-2 border-b border-edge/40 bg-panel/50 shrink-0 flex flex-col gap-2">
          {activeTab === "jev" ? (
            <div className="flex flex-wrap items-center justify-between gap-3">
              <div className="flex flex-wrap items-center gap-2.5">
                {/* 场景过滤 */}
                <div className="flex items-center gap-1.5 text-[12px] text-inkdim">
                  <span className="shrink-0">决策场景:</span>
                  <select
                    className="bg-panel2 border border-edge rounded-lg px-2.5 py-1 text-[12px] text-ink outline-none hover:bg-panel3 focus:border-accent cursor-pointer"
                    value={jevSceneFilter}
                    onChange={(e) => {
                      setJevSceneFilter(e.target.value);
                      setJevPage(1);
                    }}
                  >
                    <option value="all">全部场景</option>
                    <option value="task_complexity">⚡ 任务复杂度评估</option>
                    <option value="command_guard">🛡️ 破坏性命令风控</option>
                    <option value="memory_gate">🧠 长期记忆质检</option>
                    <option value="auto_distill">✨ 经验自动提炼</option>
                    <option value="plan_review">📋 方案先行审查</option>
                  </select>
                </div>

                {/* 结果过滤 */}
                <div className="flex items-center bg-panel2 border border-edge rounded-lg p-0.5 text-[11.5px]">
                  <button
                    className={`px-2.5 py-0.8 rounded-md transition-colors cursor-pointer ${
                      jevVerdictFilter === "all" ? "bg-panel3 text-ink font-medium" : "text-inkdim hover:text-ink"
                    }`}
                    onClick={() => {
                      setJevVerdictFilter("all");
                      setJevPage(1);
                    }}
                  >
                    全部结果
                  </button>
                  <button
                    className={`px-2.5 py-0.8 rounded-md transition-colors cursor-pointer ${
                      jevVerdictFilter === "allow" ? "bg-panel3 text-emerald-400 font-medium" : "text-inkdim hover:text-ink"
                    }`}
                    onClick={() => {
                      setJevVerdictFilter("allow");
                      setJevPage(1);
                    }}
                  >
                    放行
                  </button>
                  <button
                    className={`px-2.5 py-0.8 rounded-md transition-colors cursor-pointer ${
                      jevVerdictFilter === "deny" ? "bg-panel3 text-red-400 font-medium" : "text-inkdim hover:text-ink"
                    }`}
                    onClick={() => {
                      setJevVerdictFilter("deny");
                      setJevPage(1);
                    }}
                  >
                    拦截
                  </button>
                  <button
                    className={`px-2.5 py-0.8 rounded-md transition-colors cursor-pointer ${
                      jevVerdictFilter === "abstain" ? "bg-panel3 text-zinc-300 font-medium" : "text-inkdim hover:text-ink"
                    }`}
                    onClick={() => {
                      setJevVerdictFilter("abstain");
                      setJevPage(1);
                    }}
                  >
                    弃权
                  </button>
                </div>
              </div>

              {/* 视图 Tab 切换 */}
              <div className="flex items-center bg-panel2 border border-edge rounded-lg p-0.5 text-[12px]">
                <button
                  className="px-2.5 py-0.8 rounded-md transition-colors cursor-pointer text-inkdim hover:text-ink"
                  onClick={() => setActiveTab("stream")}
                >
                  调用列表 ({totalCount})
                </button>
                <button
                  className="px-2.5 py-0.8 rounded-md transition-colors cursor-pointer flex items-center gap-1 text-inkdim hover:text-ink"
                  onClick={() => setActiveTab("clusters")}
                >
                  <span>同款异常聚合</span>
                  {topErrors.length > 0 && (
                    <span className="px-1.5 py-0.2 rounded-full text-[10px] bg-red-500/20 text-red-400 border border-red-500/30">
                      {topErrors.length}
                    </span>
                  )}
                </button>
                <button
                  className="px-2.5 py-0.8 rounded-md transition-colors cursor-pointer flex items-center gap-1 font-medium bg-amber-500/20 text-amber-300 border border-amber-500/30"
                  onClick={() => setActiveTab("jev")}
                >
                  <Zap size={11} className="text-amber-400 fill-amber-400" />
                  <span>⚡ Jev 决策流水</span>
                  {totalJevCount > 0 && (
                    <span className="px-1.5 py-0.2 rounded-full text-[10px] bg-amber-500/15 text-amber-300 border border-amber-500/25">
                      {totalJevCount}
                    </span>
                  )}
                </button>
              </div>
            </div>
          ) : (
            <>
              {/* 第一行：范围/工具/状态/搜索/Tab */}
              <div className="flex flex-wrap items-center justify-between gap-3">
                <div className="flex flex-wrap items-center gap-2.5">
                  {/* 项目范围选择 */}
                  <div className="flex items-center gap-1.5 text-[12px] text-inkdim">
                    <span className="shrink-0">项目:</span>
                    <select
                      className="bg-panel2 border border-edge rounded-lg px-2.5 py-1 text-[12px] text-ink outline-none hover:bg-panel3 focus:border-accent cursor-pointer"
                      value={selectedProjectId}
                      onChange={(e) => {
                        setSelectedProjectId(e.target.value);
                        setPage(1);
                      }}
                    >
                      <option value="all">全部项目 ({projects.length})</option>
                      {projects.map((p) => (
                        <option key={p.id} value={p.id}>
                          {p.name || p.path || "未命名项目"}
                        </option>
                      ))}
                      <option value="unassigned">未归属项目会话</option>
                    </select>
                  </div>

                  {/* 工具名称过滤 */}
                  <div className="flex items-center gap-1.5 text-[12px] text-inkdim">
                    <span className="shrink-0">工具:</span>
                    <select
                      className="bg-panel2 border border-edge rounded-lg px-2.5 py-1 text-[12px] text-ink outline-none hover:bg-panel3 focus:border-accent cursor-pointer max-w-[150px]"
                      value={toolNameFilter}
                      onChange={(e) => {
                        setToolNameFilter(e.target.value);
                        setPage(1);
                      }}
                    >
                      <option value="all">全部工具</option>
                      {commonTools.map((t) => (
                        <option key={t} value={t}>
                          {t}
                        </option>
                      ))}
                    </select>
                  </div>

                  {/* 状态过滤选项 */}
                  <div className="flex items-center bg-panel2 border border-edge rounded-lg p-0.5 text-[11.5px]">
                    <button
                      className={`px-2 py-0.8 rounded-md transition-colors cursor-pointer ${
                        statusFilter === "error_only"
                          ? "bg-red-500/20 text-red-300 font-medium"
                          : "text-inkdim hover:text-ink"
                      }`}
                      onClick={() => {
                        setStatusFilter("error_only");
                        setPage(1);
                      }}
                      title="仅看失败、被拦截和超时的调用"
                    >
                      仅看异常
                    </button>
                    <button
                      className={`px-2 py-0.8 rounded-md transition-colors cursor-pointer ${
                        statusFilter === "all" ? "bg-panel3 text-ink font-medium" : "text-inkdim hover:text-ink"
                      }`}
                      onClick={() => {
                        setStatusFilter("all");
                        setPage(1);
                      }}
                    >
                      全部状态
                    </button>
                    <button
                      className={`px-2 py-0.8 rounded-md transition-colors cursor-pointer ${
                        statusFilter === "failed" ? "bg-panel3 text-red-400 font-medium" : "text-inkdim hover:text-ink"
                      }`}
                      onClick={() => {
                        setStatusFilter("failed");
                        setPage(1);
                      }}
                    >
                      失败
                    </button>
                    <button
                      className={`px-2 py-0.8 rounded-md transition-colors cursor-pointer ${
                        statusFilter === "denied" ? "bg-panel3 text-zinc-300 font-medium" : "text-inkdim hover:text-ink"
                      }`}
                      onClick={() => {
                        setStatusFilter("denied");
                        setPage(1);
                      }}
                    >
                      拦截
                    </button>
                    <button
                      className={`px-2 py-0.8 rounded-md transition-colors cursor-pointer ${
                        statusFilter === "success" ? "bg-panel3 text-green-400 font-medium" : "text-inkdim hover:text-ink"
                      }`}
                      onClick={() => {
                        setStatusFilter("success");
                        setPage(1);
                      }}
                    >
                      成功
                    </button>
                  </div>
                </div>

                <div className="flex items-center gap-3">
                  {/* 关键字模糊搜索 */}
                  <div className="relative">
                    <Search size={13} className="absolute left-2.5 top-1/2 -translate-y-1/2 text-inkdim" />
                    <input
                      type="text"
                      placeholder="搜索入参/出参/错误文本..."
                      className="bg-panel2 border border-edge rounded-lg pl-7 pr-7 py-1 text-[12px] text-ink placeholder:text-inkdim/50 outline-none hover:bg-panel3 focus:border-accent w-56"
                      value={searchInput}
                      onChange={(e) => setSearchInput(e.target.value)}
                    />
                    {searchInput && (
                      <button
                        className="absolute right-2 top-1/2 -translate-y-1/2 text-inkdim hover:text-ink cursor-pointer"
                        onClick={() => setSearchInput("")}
                      >
                        <X size={12} />
                      </button>
                    )}
                  </div>

                  {/* 视图 Tab 切换 */}
                  <div className="flex items-center bg-panel2 border border-edge rounded-lg p-0.5 text-[12px]">
                    <button
                      className={`px-2.5 py-0.8 rounded-md transition-colors cursor-pointer ${
                        activeTab === "stream" ? "bg-accent/15 text-accent font-medium" : "text-inkdim hover:text-ink"
                      }`}
                      onClick={() => setActiveTab("stream")}
                    >
                      调用列表 ({totalCount})
                    </button>
                    <button
                      className={`px-2.5 py-0.8 rounded-md transition-colors cursor-pointer flex items-center gap-1 ${
                        activeTab === "clusters" ? "bg-accent/15 text-accent font-medium" : "text-inkdim hover:text-ink"
                      }`}
                      onClick={() => setActiveTab("clusters")}
                    >
                      <span>同款异常聚合</span>
                      {topErrors.length > 0 && (
                        <span className="px-1.5 py-0.2 rounded-full text-[10px] bg-red-500/20 text-red-400 border border-red-500/30">
                          {topErrors.length}
                        </span>
                      )}
                    </button>
                    <button
                      className="px-2.5 py-0.8 rounded-md transition-colors cursor-pointer flex items-center gap-1 font-medium text-inkdim hover:text-ink"
                      onClick={() => setActiveTab("jev")}
                    >
                      <Zap size={11} className="text-inkdim" />
                      <span>⚡ Jev 决策流水</span>
                      {totalJevCount > 0 && (
                        <span className="px-1.5 py-0.2 rounded-full text-[10px] bg-amber-500/15 text-amber-300 border border-amber-500/25">
                          {totalJevCount}
                        </span>
                      )}
                    </button>
                  </div>
                </div>
              </div>

              {/* 第二行：时间范围与日期选择 */}
              <div className="flex flex-wrap items-center justify-between gap-2 pt-1.5 border-t border-edge/30 text-[12px]">
                <div className="flex flex-wrap items-center gap-2.5">
                  <div className="flex items-center gap-1.5 text-inkdim">
                    <Calendar size={13} className="text-accent" />
                    <span className="shrink-0 text-[11.5px]">时间范围:</span>
                  </div>

                  {/* 快捷预设 */}
                  <div className="flex bg-panel2 p-0.5 rounded-lg border border-edge text-[11px]">
                    {[
                      { label: "全部时间", val: "all" as const },
                      { label: "今天", val: "today" as const },
                      { label: "近 7 天", val: "7d" as const },
                      { label: "近 30 天", val: "30d" as const },
                    ].map((item) => (
                      <button
                        key={item.val}
                        className={`px-2 py-0.8 rounded-md transition-colors cursor-pointer ${
                          datePreset === item.val
                            ? "bg-panel3 text-ink font-medium shadow-sm"
                            : "text-inkdim hover:text-ink"
                        }`}
                        onClick={() => handleDatePreset(item.val)}
                      >
                        {item.label}
                      </button>
                    ))}
                  </div>

                  {/* 自定义起止日期 */}
                  <div className="flex items-center gap-1.5 text-[11.5px] text-inkdim">
                    <span className="text-[11px]">从</span>
                    <input
                      type="date"
                      value={startDate}
                      onChange={(e) => handleStartDateChange(e.target.value)}
                      className="bg-panel2 border border-edge rounded-lg px-2 py-0.5 text-[11.5px] text-ink outline-none hover:bg-panel3 focus:border-accent cursor-pointer"
                      title="起始日期"
                    />
                    <span className="text-[11px]">至</span>
                    <input
                      type="date"
                      value={endDate}
                      onChange={(e) => handleEndDateChange(e.target.value)}
                      className="bg-panel2 border border-edge rounded-lg px-2 py-0.5 text-[11.5px] text-ink outline-none hover:bg-panel3 focus:border-accent cursor-pointer"
                      title="结束日期"
                    />
                    {(startDate || endDate) && (
                      <button
                        className="p-1 rounded-md hover:bg-panel3 text-inkdim hover:text-ink transition-colors cursor-pointer ml-0.5"
                        onClick={handleClearDate}
                        title="重置日期范围"
                      >
                        <X size={12} />
                      </button>
                    )}
                  </div>
                </div>

                {/* 提示或快速状态 */}
                {(startDate || endDate) && (
                  <div className="text-[11px] text-accent/80 flex items-center gap-1">
                    <span>
                      已过滤区间: {startDate || "最早"} ~ {endDate || "今天"}
                    </span>
                  </div>
                )}
              </div>
            </>
          )}
        </div>

        {/* 主体内容视图 */}
        <div className="flex-1 min-h-0 flex overflow-hidden">
          {activeTab === "jev" ? (
            /* 视图 3：Jev 决策流水双栏布局 */
            <div className="flex-1 flex min-h-0 overflow-hidden">
              {/* 左侧列表 */}
              <div className="w-[45%] border-r border-edge flex flex-col min-h-0 bg-panel/30">
                <div className="flex-1 overflow-y-auto divide-y divide-edge/40">
                  {jevItems.length === 0 ? (
                    <div className="h-48 flex flex-col items-center justify-center text-inkdim text-[13px]">
                      <span>暂无匹配的 Jev 决策流水</span>
                    </div>
                  ) : (
                    jevItems.map((item) => {
                      const isSelected = item.id === selectedJevId;
                      return (
                        <div
                          key={item.id}
                          className={`p-3.5 cursor-pointer transition-colors ${
                            isSelected
                              ? "bg-amber-500/10 border-l-2 border-l-amber-400"
                              : "hover:bg-panel3/50"
                          }`}
                          onClick={() => setSelectedJevId(item.id)}
                        >
                          <div className="flex items-center justify-between gap-2 mb-1.5 flex-wrap">
                            <div className="flex items-center gap-1.5">
                              {jevSceneBadge(item.scene)}
                              {jevVerdictBadge(item.verdict)}
                            </div>
                            <span className="text-[11px] text-inkdim shrink-0 font-mono">
                              {formatTimestamp(item.createdAt)}
                            </span>
                          </div>

                          <div className="flex items-center gap-2 text-[11.5px] text-inkdim mb-1 font-mono flex-wrap">
                            <span className="text-amber-400 font-semibold">{item.latencyMs}ms</span>
                            <span>·</span>
                            <span>置信度: {(item.confidence * 100).toFixed(0)}%</span>
                            {item.decisionValue && (
                              <>
                                <span>·</span>
                                <span className="text-ink font-sans">值: {item.decisionValue}</span>
                              </>
                            )}
                            {item.adaptedEffort && (
                              <span className="px-1 rounded bg-blue-500/15 text-blue-300 text-[10px]">
                                {item.adaptedEffort}
                              </span>
                            )}
                          </div>

                          {item.reason ? (
                            <div className="text-[11px] text-inkdim font-sans truncate bg-panel/50 px-2 py-1 rounded border border-edge/30">
                              {item.reason}
                            </div>
                          ) : item.promptSummary ? (
                            <div className="text-[11px] text-inkdim/70 font-mono truncate">
                              {item.promptSummary}
                            </div>
                          ) : null}
                        </div>
                      );
                    })
                  )}
                </div>

                {/* 分页栏 */}
                <div className="h-11 px-4 border-t border-edge/60 bg-panel/70 flex items-center justify-between shrink-0 text-[12px] text-inkdim">
                  <span>
                    第 {jevPage} / {totalJevPages} 页 (共 {totalJevCount} 条)
                  </span>
                  <div className="flex items-center gap-1">
                    <button
                      className="px-2 py-1 rounded border border-edge hover:bg-panel2 disabled:opacity-30 disabled:cursor-not-allowed cursor-pointer"
                      disabled={jevPage <= 1}
                      onClick={() => setJevPage((p) => Math.max(1, p - 1))}
                    >
                      上一页
                    </button>
                    <button
                      className="px-2 py-1 rounded border border-edge hover:bg-panel2 disabled:opacity-30 disabled:cursor-not-allowed cursor-pointer"
                      disabled={jevPage >= totalJevPages}
                      onClick={() => setJevPage((p) => Math.min(totalJevPages, p + 1))}
                    >
                      下一页
                    </button>
                  </div>
                </div>
              </div>

              {/* 右侧详情抽屉 */}
              <div className="w-[55%] flex flex-col min-h-0 bg-panel2/40 overflow-y-auto p-5 space-y-4">
                {selectedJevItem ? (
                  <>
                    <div className="bg-panel border border-edge rounded-xl p-4 shadow-sm">
                      <div className="flex items-center justify-between mb-3 flex-wrap gap-2">
                        <div className="flex items-center gap-2">
                          {jevSceneBadge(selectedJevItem.scene)}
                          {jevVerdictBadge(selectedJevItem.verdict)}
                        </div>

                        <div className="flex items-center gap-2">
                          <button
                            className="px-2.5 py-1 rounded-lg text-[11.5px] bg-accent/15 text-accent hover:bg-accent/25 border border-accent/20 transition-colors flex items-center gap-1 cursor-pointer"
                            onClick={() => handleJumpToSession(selectedJevItem.sessionId)}
                            title="跳转至产生该决策的会话页面"
                          >
                            <ExternalLink size={12} />
                            <span>跳转至该对话</span>
                          </button>
                          <button
                            className="px-2.5 py-1 rounded-lg text-[11.5px] border border-edge hover:bg-panel3 text-inkdim hover:text-ink transition-colors flex items-center gap-1 cursor-pointer"
                            onClick={() => {
                              navigator.clipboard.writeText(JSON.stringify(selectedJevItem, null, 2));
                              setCopiedJevJson(true);
                              setTimeout(() => setCopiedJevJson(false), 1500);
                              pushToast("已复制原始 JSON 数据");
                            }}
                            title="复制完整决策 JSON 数据"
                          >
                            {copiedJevJson ? <Check size={12} className="text-green-400" /> : <Copy size={12} />}
                            <span>{copiedJevJson ? "已复制" : "复制 JSON"}</span>
                          </button>
                        </div>
                      </div>

                      <div className="grid grid-cols-2 gap-2.5 text-[12px] pt-3 border-t border-edge/40">
                        <div>
                          <span className="text-inkdim/60">响应耗时: </span>
                          <span className="font-mono text-amber-300 font-bold">{selectedJevItem.latencyMs} ms</span>
                        </div>
                        <div>
                          <span className="text-inkdim/60">模型置信度: </span>
                          <span className="font-mono text-ink font-semibold">{(selectedJevItem.confidence * 100).toFixed(1)}%</span>
                        </div>
                        {selectedJevItem.decisionValue && (
                          <div>
                            <span className="text-inkdim/60">决策判定值: </span>
                            <span className="font-mono text-cyan-300 font-medium">{selectedJevItem.decisionValue}</span>
                          </div>
                        )}
                        {selectedJevItem.adaptedEffort && (
                          <div>
                            <span className="text-inkdim/60">自适应推理深度: </span>
                            <span className="font-mono text-blue-300 font-medium">{selectedJevItem.adaptedEffort}</span>
                          </div>
                        )}
                        {selectedJevItem.needsResearch != null && (
                          <div>
                            <span className="text-inkdim/60">知识调研诉求: </span>
                            <span className={selectedJevItem.needsResearch ? "text-purple-300 font-medium" : "text-inkdim"}>
                              {selectedJevItem.needsResearch ? "需要调研探索" : "无需调研"}
                            </span>
                          </div>
                        )}
                        <div>
                          <span className="text-inkdim/60">决策时间: </span>
                          <span className="font-mono text-ink text-[11px]">{selectedJevItem.createdAt}</span>
                        </div>
                        <div className="col-span-2 truncate font-mono text-[11px] text-inkdim/60">
                          会话 ID: {selectedJevItem.sessionId}
                        </div>
                        {selectedJevItem.runId && (
                          <div className="col-span-2 truncate font-mono text-[11px] text-inkdim/60">
                            Run ID: {selectedJevItem.runId}
                          </div>
                        )}
                        <div className="col-span-2 truncate font-mono text-[10.5px] text-inkdim/50">
                          Event ID: {selectedJevItem.id}
                        </div>
                      </div>
                    </div>

                    {/* 决策归因与思考理由 */}
                    {selectedJevItem.reason && (
                      <div className="bg-panel border border-edge rounded-xl p-4 shadow-sm flex flex-col gap-2">
                        <div className="flex items-center gap-1.5 text-[12.5px] font-semibold text-amber-300">
                          <Zap size={14} className="fill-amber-400" />
                          <span>Jev 快思考决策归因理由 (Reasoning)</span>
                        </div>
                        <div className="text-[12px] text-ink font-sans bg-panel2/80 rounded-lg p-3 whitespace-pre-wrap leading-relaxed border border-edge/50 select-text">
                          {selectedJevItem.reason}
                        </div>
                      </div>
                    )}

                    {/* 入参上下文摘要 */}
                    {selectedJevItem.promptSummary && (
                      <div className="bg-panel border border-edge rounded-xl p-4 shadow-sm flex flex-col gap-2">
                        <div className="flex items-center gap-1.5 text-[12px] font-medium text-inkdim">
                          <ScrollText size={13} />
                          <span>决策评估入参摘要 (Prompt Summary)</span>
                        </div>
                        <div className="text-[11.5px] text-inkdim font-mono bg-panel2/80 rounded-lg p-3 max-h-48 overflow-y-auto whitespace-pre-wrap leading-relaxed border border-edge/50 select-text">
                          {selectedJevItem.promptSummary}
                        </div>
                      </div>
                    )}
                  </>
                ) : (
                  <div className="h-full flex flex-col items-center justify-center text-inkdim text-[13px]">
                    <Zap size={24} className="text-inkdim/40 mb-2" />
                    <span>请在左侧选择一条 Jev 决策记录查看全量归因细节</span>
                  </div>
                )}
              </div>
            </div>
          ) : activeTab === "clusters" ? (
            /* 视图 1：同款异常聚类看板 */
            <div className="flex-1 overflow-y-auto p-6 space-y-3">
              <div className="text-[13px] text-ink font-medium flex items-center justify-between mb-2">
                <div className="flex items-center gap-2">
                  <Bug size={16} className="text-amber-400" />
                  <span>跨对话高频同款调用错误统计分析</span>
                </div>
                <div className="text-[12px] text-inkdim">共聚类出 {topErrors.length} 组频繁发生的相同错误模式</div>
              </div>

              {topErrors.length === 0 ? (
                <div className="h-48 flex flex-col items-center justify-center text-inkdim text-[13px] border border-dashed border-edge/60 rounded-xl">
                  <span>当前筛选范围内未发现同款异常错误</span>
                </div>
              ) : (
                <div className="grid grid-cols-1 md:grid-cols-2 gap-3.5">
                  {topErrors.map((te, idx) => (
                    <div
                      key={idx}
                      className="bg-panel border border-edge rounded-xl p-4 flex flex-col justify-between hover:border-accent/40 transition-all shadow-sm"
                    >
                      <div>
                        <div className="flex items-center justify-between mb-2">
                          <div className="flex items-center gap-2">
                            <span className="text-[11px] font-mono px-2 py-0.5 rounded bg-cyan-500/10 text-cyan-400 border border-cyan-500/20 font-medium">
                              {te.toolName}
                            </span>
                            <span className="text-[11px] text-inkdim flex items-center gap-1">
                              <Clock size={11} />
                              {formatTimestamp(te.latestAt)}
                            </span>
                          </div>
                          <div className="flex items-center gap-2">
                            <span className="text-[11px] px-2 py-0.5 rounded-full bg-red-500/15 text-red-400 border border-red-500/30 font-semibold">
                              发生 {te.count} 次
                            </span>
                            <span className="text-[11px] px-2 py-0.5 rounded-full bg-amber-500/15 text-amber-400 border border-amber-500/30">
                              波及 {te.sessionCount} 个对话
                            </span>
                          </div>
                        </div>

                        <div className="text-[12.5px] text-ink/90 font-mono bg-panel2/60 border border-edge/60 rounded-lg p-2.5 my-2 break-all line-clamp-3 leading-relaxed">
                          {te.errorSummary}
                        </div>
                      </div>

                      <div className="flex items-center justify-end mt-2 pt-2 border-t border-edge/40">
                        <button
                          className="px-2.5 py-1 rounded-lg text-[11.5px] bg-accent/15 text-accent hover:bg-accent/25 border border-accent/20 transition-colors flex items-center gap-1 cursor-pointer"
                          onClick={() => handleApplyTopErrorFilter(te.toolName, te.errorSummary)}
                        >
                          <Filter size={12} />
                          <span>筛选查看相关调用</span>
                        </button>
                      </div>
                    </div>
                  ))}
                </div>
              )}
            </div>
          ) : (
            /* 视图 2：调用列表 + 侧边详情抽屉双栏布局 */
            <div className="flex-1 flex min-h-0 overflow-hidden">
              {/* 左侧日志列表 */}
              <div className="w-[45%] border-r border-edge flex flex-col min-h-0 bg-panel/30">
                <div className="flex-1 overflow-y-auto divide-y divide-edge/40">
                  {items.length === 0 ? (
                    <div className="h-48 flex flex-col items-center justify-center text-inkdim text-[13px]">
                      <span>暂无匹配的调用记录</span>
                    </div>
                  ) : (
                    items.map((item) => {
                      const isSelected = item.id === selectedItemId;
                      return (
                        <div
                          key={item.id}
                          className={`p-3.5 cursor-pointer transition-colors ${
                            isSelected
                              ? "bg-accent/10 border-l-2 border-l-accent"
                              : "hover:bg-panel3/50"
                          }`}
                          onClick={() => setSelectedItemId(item.id)}
                        >
                          <div className="flex items-center justify-between gap-2 mb-1.5">
                            <div className="flex items-center gap-1.5 min-w-0 flex-1">
                              {statusBadge(item.status)}
                              <span className="font-mono text-[12px] font-semibold text-ink truncate">
                                {item.toolName}
                              </span>
                            </div>
                            <span className="text-[11px] text-inkdim shrink-0">
                              {formatTimestamp(item.createdAt)}
                            </span>
                          </div>

                          <div className="text-[11.5px] text-inkdim truncate flex items-center gap-1 mb-1">
                            <MessageSquare size={11} className="shrink-0" />
                            <span className="truncate">{item.sessionTitle || "无标题会话"}</span>
                            {item.projectName && (
                              <span className="text-[10px] px-1.5 py-0.2 rounded bg-panel3 text-inkdim/80 shrink-0">
                                {item.projectName}
                              </span>
                            )}
                          </div>

                          {/* 简要报错或入参 */}
                          {item.status !== "success" && item.resultText ? (
                            <div className="text-[11px] text-red-400 font-mono truncate bg-red-500/5 px-1.5 py-0.5 rounded border border-red-500/15">
                              {item.resultText}
                            </div>
                          ) : (
                            <div className="text-[11px] text-inkdim/70 font-mono truncate">
                              {JSON.stringify(item.params)}
                            </div>
                          )}
                        </div>
                      );
                    })
                  )}
                </div>

                {/* 分页控制栏 */}
                <div className="h-11 px-4 border-t border-edge/60 bg-panel/70 flex items-center justify-between shrink-0 text-[12px] text-inkdim">
                  <span>
                    第 {page} / {totalPages} 页 (共 {totalCount} 条)
                  </span>
                  <div className="flex items-center gap-1">
                    <button
                      className="px-2 py-1 rounded border border-edge hover:bg-panel2 disabled:opacity-30 disabled:cursor-not-allowed cursor-pointer"
                      disabled={page <= 1}
                      onClick={() => setPage((p) => Math.max(1, p - 1))}
                    >
                      上一页
                    </button>
                    <button
                      className="px-2 py-1 rounded border border-edge hover:bg-panel2 disabled:opacity-30 disabled:cursor-not-allowed cursor-pointer"
                      disabled={page >= totalPages}
                      onClick={() => setPage((p) => Math.min(totalPages, p + 1))}
                    >
                      下一页
                    </button>
                  </div>
                </div>
              </div>

              {/* 右侧详细报文与上下文抽屉 */}
              <div className="w-[55%] flex flex-col min-h-0 bg-panel2/40 overflow-y-auto p-5 space-y-4">
                {selectedItem ? (
                  <>
                    {/* 详情标题与元数据 */}
                    <div className="bg-panel border border-edge rounded-xl p-4 shadow-sm">
                      <div className="flex items-center justify-between mb-2.5">
                        <div className="flex items-center gap-2">
                          {statusBadge(selectedItem.status)}
                          <span className="font-mono text-[14px] font-bold text-ink">
                            {selectedItem.toolName}
                          </span>
                        </div>

                        <button
                          className="px-2.5 py-1 rounded-lg text-[11.5px] bg-accent/15 text-accent hover:bg-accent/25 border border-accent/20 transition-colors flex items-center gap-1 cursor-pointer"
                          onClick={() => handleJumpToSession(selectedItem.sessionId)}
                          title="跳转至产生该工具调用的会话页面"
                        >
                          <ExternalLink size={12} />
                          <span>跳转至该对话</span>
                        </button>
                      </div>

                      <div className="grid grid-cols-2 gap-2 text-[11.5px] text-inkdim pt-2 border-t border-edge/40">
                        <div>
                          <span className="text-inkdim/60">发生时间: </span>
                          <span className="font-mono text-ink">{selectedItem.createdAt}</span>
                        </div>
                        <div>
                          <span className="text-inkdim/60">关联项目: </span>
                          <span className="text-ink">{selectedItem.projectName || "未归属项目"}</span>
                        </div>
                        <div className="col-span-2 truncate">
                          <span className="text-inkdim/60">会话标题: </span>
                          <span className="text-ink">{selectedItem.sessionTitle}</span>
                        </div>
                        {selectedItem.subprocessId && (
                          <div className="col-span-2 truncate">
                            <span className="text-inkdim/60">子进程/子代理 ID: </span>
                            <span className="font-mono text-cyan-400">{selectedItem.subprocessId}</span>
                          </div>
                        )}
                        <div className="col-span-2 truncate font-mono text-[10.5px] text-inkdim/60">
                          Event ID: {selectedItem.id}
                        </div>
                      </div>
                    </div>

                    {/* 模型思考过程上下文 (Reasoning Context) */}
                    {selectedItem.reasoning && (
                      <div className="bg-panel border border-edge rounded-xl p-3.5 shadow-sm">
                        <div className="flex items-center gap-1.5 text-[12px] font-medium text-purple-400 mb-2">
                          <Brain size={14} />
                          <span>触发本次调用的模型思考过程 (Reasoning Context)</span>
                        </div>
                        <div className="text-[12px] text-inkdim font-mono bg-panel2/80 rounded-lg p-2.5 max-h-36 overflow-y-auto whitespace-pre-wrap leading-relaxed border border-edge/50">
                          {selectedItem.reasoning}
                        </div>
                      </div>
                    )}

                    {/* 出参与报错结果 (Result / Error) */}
                    <div className="bg-panel border border-edge rounded-xl p-4 shadow-sm flex flex-col">
                      <div className="flex items-center justify-between mb-2">
                        <div className="flex items-center gap-1.5 text-[12.5px] font-medium text-ink">
                          {selectedItem.status !== "success" ? (
                            <>
                              <XCircle size={14} className="text-red-400" />
                              <span className="text-red-400 font-semibold">执行出参 / 错误原因 (Error / Result)</span>
                            </>
                          ) : (
                            <>
                              <CheckCircle2 size={14} className="text-emerald-400" />
                              <span>执行结果出参 (Result Output)</span>
                            </>
                          )}
                        </div>

                        {selectedItem.resultText && (
                          <button
                            className="text-[11.5px] text-inkdim hover:text-ink flex items-center gap-1 transition-colors cursor-pointer"
                            onClick={() => copyText(selectedItem.resultText ?? "", "result")}
                          >
                            {copiedType === "result" ? (
                              <>
                                <Check size={12} className="text-emerald-400" />
                                <span className="text-emerald-400">已复制</span>
                              </>
                            ) : (
                              <>
                                <Copy size={12} />
                                <span>复制出参</span>
                              </>
                            )}
                          </button>
                        )}
                      </div>

                      <pre
                        className={`text-[12px] font-mono rounded-lg p-3 max-h-60 overflow-y-auto whitespace-pre-wrap leading-relaxed border ${
                          selectedItem.status !== "success"
                            ? "bg-red-500/5 text-red-300 border-red-500/20"
                            : "bg-panel2 text-ink border-edge"
                        }`}
                      >
                        {selectedItem.resultText || "[无返回内容]"}
                      </pre>
                    </div>

                    {/* 完整入参 (Input Parameters JSON) */}
                    <div className="bg-panel border border-edge rounded-xl p-4 shadow-sm flex flex-col">
                      <div className="flex items-center justify-between mb-2">
                        <div className="flex items-center gap-1.5 text-[12.5px] font-medium text-ink">
                          <Wrench size={14} className="text-accent" />
                          <span>完整调用入参 (Input Params JSON)</span>
                        </div>

                        <button
                          className="text-[11.5px] text-inkdim hover:text-ink flex items-center gap-1 transition-colors cursor-pointer"
                          onClick={() => copyText(JSON.stringify(selectedItem.params, null, 2), "params")}
                        >
                          {copiedType === "params" ? (
                            <>
                              <Check size={12} className="text-emerald-400" />
                              <span className="text-emerald-400">已复制</span>
                            </>
                          ) : (
                            <>
                              <Copy size={12} />
                              <span>复制入参 JSON</span>
                            </>
                          )}
                        </button>
                      </div>

                      <pre className="text-[12px] font-mono text-cyan-300 bg-panel2 border border-edge rounded-lg p-3 max-h-64 overflow-y-auto whitespace-pre-wrap leading-relaxed">
                        {JSON.stringify(selectedItem.params, null, 2)}
                      </pre>
                    </div>
                  </>
                ) : (
                  <div className="h-full flex flex-col items-center justify-center text-inkdim text-[13px]">
                    <ScrollText size={32} className="text-inkdim/40 mb-2" />
                    <span>请在左侧列表中选择一条调用日志查看详情</span>
                  </div>
                )}
              </div>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
