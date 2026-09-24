import { useEffect, useState } from "react";
import { ipc } from "../ipc";
import { useStore } from "../store";
import type { TokenStatsReport } from "../types";
import {
  X,
  RotateCcw,
  BarChart2,
  TrendingUp,
  FolderOpen,
  MessageSquare,
  Clock,
  Sparkles,
  ChevronRight,
  ExternalLink,
  Zap,
} from "./Icons";

function formatTokens(n?: number): string {
  if (n == null || isNaN(n)) return "0";
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(2)}M`;
  if (n >= 10_000) return `${(n / 1_000).toFixed(1)}k`;
  return n.toLocaleString("zh-CN");
}

export function TokenStatsModal() {
  const show = useStore((s) => s.showTokenStatsModal);
  const setShow = useStore((s) => s.setShowTokenStatsModal);
  const selectSession = useStore((s) => s.selectSession);
  const enterProject = useStore((s) => s.enterProject);
  const pushToast = useStore((s) => s.pushToast);

  const [tab, setTab] = useState<"project" | "time" | "session">("project");
  const [days, setDays] = useState<number>(30); // 7, 14, 30, 0 (0 = all)
  const [selectedProjectId, setSelectedProjectId] = useState<string>("all");
  const [data, setData] = useState<TokenStatsReport | null>(null);
  const [loading, setLoading] = useState<boolean>(false);
  const [hoveredBar, setHoveredBar] = useState<number | null>(null);

  const refresh = async () => {
    setLoading(true);
    try {
      const projId = selectedProjectId === "all" ? null : selectedProjectId;
      const res = await ipc.getTokenStats(projId, days);
      setData(res);
    } catch (e) {
      pushToast(`加载 Token 统计失败: ${e}`);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    if (show) {
      void refresh();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [show, days, selectedProjectId]);

  if (!show) return null;

  const summary = data?.summary;
  const byProject = data?.byProject ?? [];
  const byTime = data?.byTime ?? [];
  const bySession = data?.bySession ?? [];

  const maxProjectTokens = Math.max(...byProject.map((p) => p.totalTokens), 1);
  const maxDayTokens = Math.max(...byTime.map((d) => d.totalTokens), 1);

  return (
    <div
      className="fixed inset-0 z-[85] bg-black/60 backdrop-blur-sm flex items-center justify-center animate-in fade-in duration-150"
      onMouseDown={() => setShow(false)}
    >
      <div
        className="bg-panel2 border border-edge rounded-2xl w-[920px] max-w-[94vw] h-[86vh] flex flex-col shadow-2xl overflow-hidden animate-in zoom-in-95 duration-150"
        onMouseDown={(e) => e.stopPropagation()}
      >
        {/* 顶部标题栏 */}
        <div className="h-14 px-6 border-b border-edge/60 flex items-center justify-between shrink-0 bg-panel/70">
          <div className="flex items-center gap-3">
            <div className="w-8 h-8 rounded-xl bg-accent/15 text-accent flex items-center justify-center border border-accent/20">
              <BarChart2 size={18} />
            </div>
            <div>
              <div className="text-[15px] font-semibold text-ink flex items-center gap-2">
                <span>Token 消耗统计</span>
                {loading && <span className="text-[11px] text-accent font-normal animate-pulse">刷新中…</span>}
              </div>
              <div className="text-[11px] text-inkdim">按项目与时间全方位统计模型 Token 消耗及缓存命中指标</div>
            </div>
          </div>

          <div className="flex items-center gap-2">
            <button
              className="px-2.5 py-1.5 rounded-lg border border-edge hover:bg-panel3 text-inkdim hover:text-ink text-[12px] flex items-center gap-1.5 transition-colors"
              onClick={() => void refresh()}
              title="刷新统计数据"
            >
              <RotateCcw size={13} className={loading ? "animate-spin" : ""} />
              <span>刷新</span>
            </button>
            <button
              className="w-8 h-8 rounded-lg hover:bg-panel3 flex items-center justify-center text-inkdim hover:text-ink transition-colors"
              onClick={() => setShow(false)}
            >
              <X size={16} />
            </button>
          </div>
        </div>

        {/* 顶部指标 KPI 卡片 (5 个关键汇总) */}
        <div className="p-6 pb-4 border-b border-edge/40 bg-panel/30 shrink-0">
          <div className="grid grid-cols-2 sm:grid-cols-3 lg:grid-cols-5 gap-3">
            {/* 总消耗 Token */}
            <div className="bg-panel border border-edge/70 rounded-xl p-3.5 shadow-sm">
              <div className="flex items-center justify-between text-inkdim text-[11px] mb-1">
                <span>总消耗 Token</span>
                <Sparkles size={13} className="text-accent" />
              </div>
              <div className="text-xl font-bold text-ink tracking-tight">
                {summary ? formatTokens(summary.totalTokens) : "0"}
              </div>
              <div className="text-[11px] text-inkdim mt-1 truncate" title={`输入: ${summary?.totalPromptTokens?.toLocaleString() ?? 0} (缓存命中: ${summary?.totalCachedTokens?.toLocaleString() ?? 0}) · 输出: ${summary?.totalCompletionTokens?.toLocaleString() ?? 0}`}>
                输入 {formatTokens(summary?.totalPromptTokens)} · 输出 {formatTokens(summary?.totalCompletionTokens)}
              </div>
            </div>

            {/* 全局缓存命中率 */}
            <div className="bg-panel border border-edge/70 rounded-xl p-3.5 shadow-sm">
              <div className="flex items-center justify-between text-inkdim text-[11px] mb-1">
                <span>全局缓存命中率</span>
                <Zap size={13} className="text-cyan-400" />
              </div>
              <div className="text-xl font-bold text-cyan-400 tracking-tight">
                {summary?.overallCacheHitRate != null ? `${summary.overallCacheHitRate.toFixed(1)}%` : "0.0%"}
              </div>
              <div className="text-[11px] text-inkdim mt-1 truncate" title={`累计命中: ${summary?.totalCachedTokens?.toLocaleString() ?? 0} tokens · 今日命中: ${summary?.todayCachedTokens?.toLocaleString() ?? 0} tokens`}>
                命中 {formatTokens(summary?.totalCachedTokens)} · 今日 {summary?.todayCacheHitRate != null ? `${summary.todayCacheHitRate.toFixed(1)}%` : "0.0%"}
              </div>
            </div>

            {/* 今日消耗 */}
            <div className="bg-panel border border-edge/70 rounded-xl p-3.5 shadow-sm">
              <div className="flex items-center justify-between text-inkdim text-[11px] mb-1">
                <span>今日消耗 Token</span>
                <Clock size={13} className="text-emerald-400" />
              </div>
              <div className="text-xl font-bold text-emerald-400 tracking-tight">
                {summary ? formatTokens(summary.todayTokens) : "0"}
              </div>
              <div className="text-[11px] text-inkdim mt-1 truncate" title={`今日输入: ${summary?.todayPromptTokens?.toLocaleString() ?? 0} (缓存命中: ${summary?.todayCachedTokens?.toLocaleString() ?? 0}) · 输出: ${summary?.todayCompletionTokens?.toLocaleString() ?? 0}`}>
                输入 {formatTokens(summary?.todayPromptTokens)} · 输出 {formatTokens(summary?.todayCompletionTokens)}
              </div>
            </div>

            {/* 关联项目数 */}
            <div className="bg-panel border border-edge/70 rounded-xl p-3.5 shadow-sm">
              <div className="flex items-center justify-between text-inkdim text-[11px] mb-1">
                <span>项目总数</span>
                <FolderOpen size={13} className="text-amber-400" />
              </div>
              <div className="text-xl font-bold text-ink tracking-tight">
                {byProject.filter((p) => p.projectId != null).length} <span className="text-[12px] font-normal text-inkdim">个</span>
              </div>
              <div className="text-[11px] text-inkdim mt-1 truncate">
                {byProject.filter((p) => p.totalTokens > 0).length} 个项目产生消耗
              </div>
            </div>

            {/* 累计会话与对话 */}
            <div className="bg-panel border border-edge/70 rounded-xl p-3.5 shadow-sm col-span-2 sm:col-span-1">
              <div className="flex items-center justify-between text-inkdim text-[11px] mb-1">
                <span>会话与轮次</span>
                <MessageSquare size={13} className="text-purple-400" />
              </div>
              <div className="text-xl font-bold text-ink tracking-tight">
                {summary?.totalSessions ?? 0} <span className="text-[12px] font-normal text-inkdim">会话</span>
              </div>
              <div className="text-[11px] text-inkdim mt-1 truncate">
                累计 {summary?.totalMessages ?? 0} 轮对话生成
              </div>
            </div>
          </div>
        </div>

        {/* 选项卡栏与筛选器 */}
        <div className="px-6 py-2.5 border-b border-edge/40 flex items-center justify-between gap-4 shrink-0 bg-panel/50 select-none">
          {/* Tab 切换 */}
          <div className="flex bg-panel2 p-0.5 rounded-lg border border-edge">
            <button
              className={`px-3 py-1.5 rounded-md text-[12px] font-medium transition-colors flex items-center gap-1.5 ${
                tab === "project" ? "bg-accent/15 text-accent shadow-sm" : "text-inkdim hover:text-ink"
              }`}
              onClick={() => setTab("project")}
            >
              <FolderOpen size={13} />
              <span>按项目统计</span>
            </button>
            <button
              className={`px-3 py-1.5 rounded-md text-[12px] font-medium transition-colors flex items-center gap-1.5 ${
                tab === "time" ? "bg-accent/15 text-accent shadow-sm" : "text-inkdim hover:text-ink"
              }`}
              onClick={() => setTab("time")}
            >
              <TrendingUp size={13} />
              <span>按时间统计</span>
            </button>
            <button
              className={`px-3 py-1.5 rounded-md text-[12px] font-medium transition-colors flex items-center gap-1.5 ${
                tab === "session" ? "bg-accent/15 text-accent shadow-sm" : "text-inkdim hover:text-ink"
              }`}
              onClick={() => setTab("session")}
            >
              <MessageSquare size={13} />
              <span>会话消耗排行</span>
            </button>
          </div>

          {/* 筛选条件（仅在按时间或会话排行时展示细化选项） */}
          <div className="flex items-center gap-2">
            {tab === "time" && (
              <div className="flex bg-panel2 p-0.5 rounded-lg border border-edge text-[11px]">
                {[
                  { label: "近 7 天", val: 7 },
                  { label: "近 14 天", val: 14 },
                  { label: "近 30 天", val: 30 },
                  { label: "全部", val: 0 },
                ].map((item) => (
                  <button
                    key={item.val}
                    className={`px-2.5 py-1 rounded-md transition-colors ${
                      days === item.val ? "bg-panel3 text-ink font-medium shadow-sm" : "text-inkdim hover:text-ink"
                    }`}
                    onClick={() => setDays(item.val)}
                  >
                    {item.label}
                  </button>
                ))}
              </div>
            )}

            {tab !== "project" && (
              <select
                className="bg-panel2 border border-edge rounded-lg px-2.5 py-1 text-[12px] text-ink outline-none cursor-pointer hover:border-accent/40 transition-colors"
                value={selectedProjectId}
                onChange={(e) => setSelectedProjectId(e.target.value)}
              >
                <option value="all">全部项目</option>
                {byProject
                  .filter((p) => p.projectId != null)
                  .map((p) => (
                    <option key={p.projectId!} value={p.projectId!}>
                      {p.projectName}
                    </option>
                  ))}
                <option value="unassigned">未归类 / 纯对话</option>
              </select>
            )}
          </div>
        </div>

        {/* 内容展示区 */}
        <div className="flex-1 overflow-y-auto p-6">
          {/* TAB 1: 按项目统计 */}
          {tab === "project" && (
            <div className="flex flex-col gap-3">
              {byProject.length === 0 ? (
                <div className="py-16 text-center text-inkdim text-[13px]">暂无项目 Token 消耗记录</div>
              ) : (
                byProject.map((item) => {
                  const pct = summary?.totalTokens ? Math.round((item.totalTokens / summary.totalTokens) * 100) : 0;
                  const barPct = Math.max(Math.round((item.totalTokens / maxProjectTokens) * 100), item.totalTokens > 0 ? 3 : 0);

                  return (
                    <div
                      key={item.projectId ?? "unassigned"}
                      className="bg-panel border border-edge rounded-xl p-4 hover:border-edge/90 transition-all flex flex-col gap-3 shadow-sm"
                    >
                      <div className="flex items-start justify-between gap-4">
                        <div className="flex-1 min-w-0">
                          <div className="flex items-center gap-2">
                            <span className="font-semibold text-[14px] text-ink truncate">{item.projectName}</span>
                            {item.projectId == null && (
                              <span className="px-1.5 py-0.5 rounded text-[10px] bg-zinc-500/15 text-zinc-400 border border-zinc-500/25">
                                纯对话
                              </span>
                            )}
                            <span className="text-[12px] text-inkdim ml-auto sm:ml-2">
                              占比 <b className="text-accent font-medium">{pct}%</b>
                            </span>
                          </div>
                          {item.projectPath && (
                            <div className="text-[11px] text-inkdim/80 truncate font-mono mt-0.5">
                              {item.projectPath}
                            </div>
                          )}
                        </div>

                        {item.projectId && (
                          <button
                            className="px-2.5 py-1 rounded-lg bg-panel2 hover:bg-panel3 border border-edge text-[11px] text-inkdim hover:text-ink shrink-0 flex items-center gap-1 transition-colors"
                            onClick={() => {
                              setShow(false);
                              enterProject(item.projectId!);
                            }}
                            title="进入该项目并新建对话"
                          >
                            <span>进入项目</span>
                            <ChevronRight size={12} />
                          </button>
                        )}
                      </div>

                      {/* 进度条 */}
                      <div className="w-full bg-panel3 h-2 rounded-full overflow-hidden flex">
                        <div
                          className="bg-gradient-to-r from-accent to-blue-400 h-full rounded-full transition-all duration-300"
                          style={{ width: `${barPct}%` }}
                        />
                      </div>

                      {/* 指标矩阵 */}
                      <div className="grid grid-cols-2 sm:grid-cols-5 gap-2 pt-1 border-t border-edge/30 text-[12px]">
                        <div>
                          <span className="text-inkdim text-[11px]">总消耗: </span>
                          <span className="font-semibold text-ink font-mono">{item.totalTokens.toLocaleString()}</span>
                        </div>
                        <div>
                          <span className="text-inkdim text-[11px]">输入 Token: </span>
                          <span className="text-inkdim font-mono">{item.promptTokens.toLocaleString()}</span>
                        </div>
                        <div>
                          <span className="text-inkdim text-[11px]">缓存命中: </span>
                          <span className="text-cyan-400 font-mono font-medium">
                            {(item.cachedTokens ?? 0).toLocaleString()}
                          </span>
                          <span className="text-[10px] text-cyan-400/80 ml-1">
                            ({(item.cacheHitRate ?? 0).toFixed(1)}%)
                          </span>
                        </div>
                        <div>
                          <span className="text-inkdim text-[11px]">输出 Token: </span>
                          <span className="text-inkdim font-mono">{item.completionTokens.toLocaleString()}</span>
                        </div>
                        <div className="text-right sm:text-left">
                          <span className="text-inkdim text-[11px]">涉及会话: </span>
                          <span className="text-ink font-medium">{item.sessionCount}</span>
                          <span className="text-inkdim text-[11px]"> 个 · {item.messageCount} 轮</span>
                        </div>
                      </div>
                    </div>
                  );
                })
              )}
            </div>
          )}

          {/* TAB 2: 按时间统计（每日图表 + 表格） */}
          {tab === "time" && (
            <div className="flex flex-col gap-6">
              {byTime.length === 0 ? (
                <div className="py-16 text-center text-inkdim text-[13px]">选定时间范围内暂无消耗数据</div>
              ) : (
                <>
                  {/* SVG 柱状图 */}
                  <div className="bg-panel border border-edge rounded-xl p-5 shadow-sm">
                    <div className="flex items-center justify-between mb-4">
                      <div className="text-[13px] font-semibold text-ink flex items-center gap-1.5">
                        <TrendingUp size={15} className="text-accent" />
                        <span>每日 Token 消耗走势</span>
                      </div>
                      <div className="flex items-center gap-4 text-[11px] text-inkdim">
                        <span className="flex items-center gap-1.5">
                          <span className="w-2.5 h-2.5 rounded-sm bg-blue-500 inline-block" />
                          <span>非缓存输入</span>
                        </span>
                        <span className="flex items-center gap-1.5">
                          <span className="w-2.5 h-2.5 rounded-sm bg-cyan-400 inline-block" />
                          <span>缓存命中</span>
                        </span>
                        <span className="flex items-center gap-1.5">
                          <span className="w-2.5 h-2.5 rounded-sm bg-emerald-400 inline-block" />
                          <span>输出 Token</span>
                        </span>
                      </div>
                    </div>

                    {/* SVG 渲染 */}
                    <div className="h-56 w-full relative">
                      <svg className="w-full h-full overflow-visible" preserveAspectRatio="none">
                        {/* 辅助水平网格线 */}
                        {[0.25, 0.5, 0.75, 1].map((ratio) => (
                          <line
                            key={ratio}
                            x1="0"
                            x2="100%"
                            y1={`${(1 - ratio) * 85 + 5}%`}
                            y2={`${(1 - ratio) * 85 + 5}%`}
                            stroke="currentColor"
                            className="text-edge/40"
                            strokeDasharray="4 4"
                          />
                        ))}

                        {/* 柱形 */}
                        {byTime.map((d, i) => {
                          const count = byTime.length;
                          const barWidthPct = Math.min(Math.max(60 / count, 1.2), 6);
                          const gap = 100 / count;
                          const xCenter = i * gap + gap / 2;
                          const x = xCenter - barWidthPct / 2;

                          const heightRatio = d.totalTokens / maxDayTokens;
                          const barH = heightRatio * 85;

                          const cachedTokens = d.cachedTokens ?? 0;
                          const promptTokens = d.promptTokens;
                          const uncachedPromptTokens = Math.max(0, promptTokens - cachedTokens);
                          const compTokens = d.completionTokens;
                          const total = d.totalTokens || 1;

                          const compH = (compTokens / total) * barH;
                          const cachedH = (cachedTokens / total) * barH;
                          const uncachedH = Math.max(0, barH - compH - cachedH);

                          const yTotal = 90 - barH;
                          const yComp = yTotal;
                          const yCached = yComp + compH;
                          const yUncached = yCached + cachedH;

                          const isHover = hoveredBar === i;

                          return (
                            <g
                              key={d.date}
                              onMouseEnter={() => setHoveredBar(i)}
                              onMouseLeave={() => setHoveredBar(null)}
                              className="cursor-pointer transition-opacity"
                            >
                              {/* 输出 Token 柱 (顶部绿色) */}
                              <rect
                                x={`${x}%`}
                                y={`${yComp}%`}
                                width={`${barWidthPct}%`}
                                height={`${Math.max(compH, compTokens > 0 ? 0.8 : 0)}%`}
                                rx="2"
                                className={`${isHover ? "fill-emerald-300" : "fill-emerald-400"} transition-colors`}
                              />
                              {/* 缓存命中 柱 (中间青色) */}
                              {cachedTokens > 0 && (
                                <rect
                                  x={`${x}%`}
                                  y={`${yCached}%`}
                                  width={`${barWidthPct}%`}
                                  height={`${Math.max(cachedH, 0.8)}%`}
                                  rx="2"
                                  className={`${isHover ? "fill-cyan-300" : "fill-cyan-400"} transition-colors`}
                                />
                              )}
                              {/* 非缓存输入 柱 (底部蓝色) */}
                              <rect
                                x={`${x}%`}
                                y={`${yUncached}%`}
                                width={`${barWidthPct}%`}
                                height={`${Math.max(uncachedH, uncachedPromptTokens > 0 ? 0.8 : 0)}%`}
                                rx="2"
                                className={`${isHover ? "fill-blue-400" : "fill-blue-500"} transition-colors`}
                              />
                            </g>
                          );
                        })}
                      </svg>

                      {/* 鼠标悬停 Tooltip 浮层 */}
                      {hoveredBar != null && byTime[hoveredBar] && (
                        <div
                          className="absolute pointer-events-none -top-2 z-20 bg-panel3 border border-edge/80 rounded-xl px-3 py-2 shadow-xl text-[11px] transform -translate-x-1/2 -translate-y-full animate-in fade-in zoom-in-95 duration-100"
                          style={{
                            left: `${(hoveredBar * (100 / byTime.length)) + (50 / byTime.length)}%`,
                          }}
                        >
                          <div className="font-semibold text-ink mb-1">{byTime[hoveredBar].date}</div>
                          <div className="text-emerald-400 font-mono">总计: {byTime[hoveredBar].totalTokens.toLocaleString()} tokens</div>
                          <div className="text-inkdim mt-0.5">
                            输入: {byTime[hoveredBar].promptTokens.toLocaleString()}
                            <span className="text-cyan-400 ml-1">
                              (缓存命中: {(byTime[hoveredBar].cachedTokens ?? 0).toLocaleString()} · {(byTime[hoveredBar].cacheHitRate ?? 0).toFixed(1)}%)
                            </span>
                          </div>
                          <div className="text-inkdim mt-0.5">输出: {byTime[hoveredBar].completionTokens.toLocaleString()} tokens</div>
                          <div className="text-inkdim mt-0.5">对话轮次: {byTime[hoveredBar].messageCount} 次</div>
                        </div>
                      )}
                    </div>

                    {/* 日期 X 轴简标 */}
                    <div className="flex justify-between text-[10px] text-inkdim/60 mt-2 pt-2 border-t border-edge/30 font-mono">
                      <span>{byTime[0]?.date}</span>
                      {byTime.length > 2 && <span>{byTime[Math.floor(byTime.length / 2)]?.date}</span>}
                      <span>{byTime[byTime.length - 1]?.date}</span>
                    </div>
                  </div>

                  {/* 每日明细列表 */}
                  <div className="bg-panel border border-edge rounded-xl overflow-hidden shadow-sm">
                    <div className="px-4 py-2.5 border-b border-edge/40 text-[12px] font-semibold text-ink bg-panel/50">
                      每日消耗清单明细
                    </div>
                    <div className="overflow-x-auto">
                      <table className="w-full text-left text-[12px]">
                        <thead className="bg-panel2/50 text-inkdim border-b border-edge/40 text-[11px]">
                          <tr>
                            <th className="py-2 px-4 font-medium">日期</th>
                            <th className="py-2 px-4 font-medium">总消耗 (Tokens)</th>
                            <th className="py-2 px-4 font-medium">输入 (Prompt)</th>
                            <th className="py-2 px-4 font-medium">缓存命中 (Rate)</th>
                            <th className="py-2 px-4 font-medium">输出 (Completion)</th>
                            <th className="py-2 px-4 font-medium">对话轮次</th>
                            <th className="py-2 px-4 font-medium text-right">占比</th>
                          </tr>
                        </thead>
                        <tbody className="divide-y divide-edge/30">
                          {[...byTime].reverse().map((d) => {
                            const pct = Math.round((d.totalTokens / maxDayTokens) * 100);
                            return (
                              <tr key={d.date} className="hover:bg-panel2/40 transition-colors">
                                <td className="py-2.5 px-4 font-mono text-ink font-medium">{d.date}</td>
                                <td className="py-2.5 px-4 font-mono font-semibold text-accent">{d.totalTokens.toLocaleString()}</td>
                                <td className="py-2.5 px-4 font-mono text-inkdim">{d.promptTokens.toLocaleString()}</td>
                                <td className="py-2.5 px-4 font-mono">
                                  <span className="text-cyan-400">{(d.cachedTokens ?? 0).toLocaleString()}</span>
                                  <span className="text-inkdim text-[10px] ml-1">({(d.cacheHitRate ?? 0).toFixed(1)}%)</span>
                                </td>
                                <td className="py-2.5 px-4 font-mono text-inkdim">{d.completionTokens.toLocaleString()}</td>
                                <td className="py-2.5 px-4 text-inkdim">{d.messageCount} 轮</td>
                                <td className="py-2.5 px-4 text-right">
                                  <div className="inline-block w-20 bg-panel3 h-1.5 rounded-full overflow-hidden align-middle ml-2">
                                    <div className="bg-accent h-full rounded-full" style={{ width: `${pct}%` }} />
                                  </div>
                                </td>
                              </tr>
                            );
                          })}
                        </tbody>
                      </table>
                    </div>
                  </div>
                </>
              )}
            </div>
          )}

          {/* TAB 3: 会话消耗排行 */}
          {tab === "session" && (
            <div className="flex flex-col gap-2.5">
              {bySession.length === 0 ? (
                <div className="py-16 text-center text-inkdim text-[13px]">暂无会话消耗记录</div>
              ) : (
                bySession.map((s, idx) => (
                  <div
                    key={s.sessionId}
                    className="bg-panel border border-edge rounded-xl p-3.5 hover:border-edge/90 transition-all flex items-center gap-3.5 shadow-sm"
                  >
                    {/* 序号徽章 */}
                    <div
                      className={`w-6 h-6 rounded-lg text-[11px] font-bold flex items-center justify-center shrink-0 ${
                        idx < 3
                          ? "bg-amber-500/15 text-amber-400 border border-amber-500/20"
                          : "bg-panel2 text-inkdim border border-edge"
                      }`}
                    >
                      {idx + 1}
                    </div>

                    <div className="flex-1 min-w-0">
                      <div className="flex items-center gap-2">
                        <span className="font-semibold text-[13px] text-ink truncate" title={s.title}>
                          {s.title}
                        </span>
                        {s.projectName && (
                          <span className="px-1.5 py-0.5 rounded text-[10px] bg-accent/10 text-accent font-medium border border-accent/20 truncate max-w-[120px]">
                            {s.projectName}
                          </span>
                        )}
                        {(s.cacheHitRate ?? 0) > 0 && (
                          <span className="px-1.5 py-0.5 rounded text-[10px] bg-cyan-500/10 text-cyan-400 font-medium border border-cyan-500/20 truncate inline-flex items-center gap-1">
                            <Zap size={10} className="shrink-0" />
                            <span>缓存 {(s.cacheHitRate ?? 0).toFixed(1)}%</span>
                          </span>
                        )}
                      </div>
                      <div className="flex flex-wrap items-center gap-x-3 gap-y-0.5 text-[11px] text-inkdim mt-0.5">
                        <span>输入 {s.promptTokens.toLocaleString()}</span>
                        <span>·</span>
                        {(s.cachedTokens ?? 0) > 0 && (
                          <>
                            <span className="text-cyan-400 font-mono">
                              缓存命中 {(s.cachedTokens ?? 0).toLocaleString()} ({(s.cacheHitRate ?? 0).toFixed(1)}%)
                            </span>
                            <span>·</span>
                          </>
                        )}
                        <span>输出 {s.completionTokens.toLocaleString()}</span>
                        <span>·</span>
                        <span>{s.messageCount} 轮对话</span>
                      </div>
                    </div>

                    {/* Token 数值 */}
                    <div className="text-right shrink-0">
                      <div className="text-[14px] font-bold font-mono text-ink">
                        {formatTokens(s.totalTokens)}
                      </div>
                      <div className="text-[10px] text-inkdim font-mono">{s.totalTokens.toLocaleString()} tokens</div>
                    </div>

                    {/* 跳转查看按钮 */}
                    <button
                      className="px-2.5 py-1.5 rounded-lg bg-panel2 hover:bg-panel3 border border-edge text-[12px] text-inkdim hover:text-ink shrink-0 flex items-center gap-1 transition-colors"
                      onClick={() => {
                        setShow(false);
                        void selectSession(s.sessionId);
                      }}
                      title="打开该会话"
                    >
                      <span>查看</span>
                      <ExternalLink size={12} />
                    </button>
                  </div>
                ))
              )}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
