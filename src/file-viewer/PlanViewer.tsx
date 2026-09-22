import React, { useEffect, useState } from "react";
import { ipc } from "../ipc";
import type { PlanViewerTab, ActivePlanDetail } from "./types";
import { Markdown } from "../components/Markdown";
import {
  CheckSquare,
  RotateCcw,
  Copy,
  Check,
  FolderOpen,
  Circle,
  Clock,
  CheckCircle2,
  AlertCircle,
} from "../components/Icons";

const STATUS_BADGE: Record<string, { label: string; cls: string }> = {
  in_progress: { label: "执行中", cls: "bg-emerald-500/15 text-emerald-400 border-emerald-500/30" },
  completed: { label: "已结案", cls: "bg-green-500/15 text-green-400 border-green-500/30" },
  drafting: { label: "拟订中", cls: "bg-blue-500/15 text-blue-400 border-blue-500/30" },
  suspended: { label: "已挂起", cls: "bg-amber-500/15 text-amber-400 border-amber-500/30" },
  archived: { label: "已归档", cls: "bg-gray-500/15 text-gray-400 border-gray-500/30" },
};

export function PlanViewer({ tab }: { tab: PlanViewerTab }) {
  const [data, setData] = useState<ActivePlanDetail | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  const loadPlan = async () => {
    setLoading(true);
    setError(null);
    try {
      const res = await ipc.getPlanDetail(
        tab.workspacePath || "",
        tab.sessionId,
        tab.planId
      );
      if (res) {
        setData(res);
      } else {
        setError("未找到该任务方案文件或尚未生成方案");
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    void loadPlan();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tab.planId, tab.sessionId, tab.workspacePath]);

  const handleCopy = () => {
    if (!data?.body) return;
    navigator.clipboard.writeText(data.body).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    });
  };

  const handleOpenPlanDir = () => {
    if (!tab.workspacePath) return;
    const planDir = `${tab.workspacePath.replace(/\\/g, "/")}/.harness/plans`;
    ipc.openDir(planDir);
  };

  const statusInfo = STATUS_BADGE[data?.meta.status || "in_progress"] || STATUS_BADGE.in_progress;
  const completedSteps = data?.steps.filter((s) => s.status === "done").length || 0;
  const totalSteps = data?.steps.length || 0;
  const progressPercent = totalSteps > 0 ? Math.round((completedSteps / totalSteps) * 100) : 0;

  const handleStepStatusChange = async (
    stepIndex: number,
    newStatus: "pending" | "in_progress" | "done"
  ) => {
    if (!data) return;

    // 1. 乐观更新本地状态
    const oldSteps = data.steps;
    const updatedSteps = oldSteps.map((s) =>
      s.index === stepIndex ? { ...s, status: newStatus } : s
    );
    setData({
      ...data,
      steps: updatedSteps,
    });

    // 2. 调用后端持久化到 Markdown 文件与 session todo
    try {
      await ipc.updatePlanStepStatus(
        tab.workspacePath || "",
        data.meta.id || tab.planId,
        stepIndex,
        newStatus,
        tab.sessionId
      );
    } catch (e) {
      console.error("更新步骤状态失败:", e);
      // 失败时回滚
      setData({
        ...data,
        steps: oldSteps,
      });
    }
  };

  const cycleStepStatus = (stepIndex: number, currentStatus: string) => {
    const nextStatus: "pending" | "in_progress" | "done" =
      currentStatus === "pending"
        ? "in_progress"
        : currentStatus === "in_progress"
        ? "done"
        : "pending";
    void handleStepStatusChange(stepIndex, nextStatus);
  };

  return (
    <div className="flex-1 flex flex-col min-h-0 bg-panel text-ink overflow-hidden">
      {/* 头部元数据栏 */}
      <div className="px-5 py-3 border-b border-edge/60 bg-panel2/60 shrink-0">
        <div className="flex items-center justify-between gap-3">
          <div className="flex items-center gap-2.5 min-w-0">
            <CheckSquare size={16} className="text-emerald-400 shrink-0" />
            <span className="font-semibold text-[14px] text-ink truncate">
              {data?.meta.title || tab.title}
            </span>
            {data?.meta && (
              <span className="text-[11px] font-mono px-2 py-0.5 rounded-full bg-emerald-500/10 text-emerald-400 border border-emerald-500/30">
                v{data.meta.version}
              </span>
            )}
            <span className={`text-[11px] px-2 py-0.5 rounded-full border ${statusInfo.cls}`}>
              {statusInfo.label}
            </span>
          </div>

          <div className="flex items-center gap-1.5 shrink-0 text-[12px]">
            <button
              type="button"
              className="p-1.5 rounded-lg text-inkdim hover:text-ink hover:bg-panel3 transition-colors"
              title="刷新方案"
              onClick={loadPlan}
            >
              <RotateCcw size={14} />
            </button>
            <button
              type="button"
              className="flex items-center gap-1 px-2.5 py-1 rounded-lg text-inkdim hover:text-ink hover:bg-panel3 transition-colors text-[11px]"
              title="复制方案正文"
              onClick={handleCopy}
            >
              {copied ? <Check size={13} className="text-green-400" /> : <Copy size={13} />}
              <span>{copied ? "已复制" : "复制方案"}</span>
            </button>
            <button
              type="button"
              className="p-1.5 rounded-lg text-inkdim hover:text-ink hover:bg-panel3 transition-colors"
              title="打开方案存储目录 (.harness/plans/)"
              onClick={handleOpenPlanDir}
            >
              <FolderOpen size={14} />
            </button>
          </div>
        </div>

        {/* 方案文件与更新时间 */}
        <div className="flex items-center justify-between mt-1.5 text-[11px] text-inkdim">
          <div className="flex items-center gap-3 truncate min-w-0">
            {data?.filename && (
              <span className="font-mono truncate" title={data.filename}>
                文件: .harness/plans/{data.filename}
              </span>
            )}
            {data?.meta.updated_at && (
              <span>更新于: {new Date(data.meta.updated_at).toLocaleString("zh-CN")}</span>
            )}
          </div>
          <span className="text-[10.5px] text-emerald-400/80 font-mono shrink-0">
            ● 状态修改自动同步至磁盘
          </span>
        </div>
      </div>

      {/* 滚动内容区 */}
      <div className="flex-1 overflow-auto p-5 space-y-5">
        {loading && (
          <div className="flex items-center justify-center h-48 text-inkdim text-[13px]">
            加载任务方案中…
          </div>
        )}

        {error && (
          <div className="p-6 flex flex-col items-center justify-center text-red-400 gap-2">
            <AlertCircle size={24} />
            <div className="text-[13px] font-medium">{error}</div>
            <button
              className="mt-2 px-3 py-1 bg-panel3 rounded-lg text-ink text-[12px] hover:bg-edge"
              onClick={loadPlan}
            >
              重新加载
            </button>
          </div>
        )}

        {data && (
          <>
            {/* 分步任务执行看板 */}
            {data.steps && data.steps.length > 0 && (
              <div className="bg-panel2/60 border border-edge/80 rounded-xl p-4 shadow-2xs">
                <div className="flex items-center justify-between mb-3 text-[12px]">
                  <span className="font-semibold text-ink flex items-center gap-1.5">
                    <span>实施步骤清单</span>
                    <span className="text-inkdim font-normal">
                      ({completedSteps} / {totalSteps} 步完成)
                    </span>
                  </span>
                  <div className="flex items-center gap-2">
                    <span className="text-[11px] text-inkdim font-mono">点击图标切换状态</span>
                    <span className="font-mono text-emerald-400 font-medium">{progressPercent}%</span>
                  </div>
                </div>

                {/* 进度条 */}
                <div className="w-full h-1.5 bg-panel3 rounded-full overflow-hidden mb-3.5">
                  <div
                    className="h-full bg-emerald-500 rounded-full transition-all duration-300"
                    style={{ width: `${progressPercent}%` }}
                  />
                </div>

                <div className="space-y-1.5 text-[12.5px]">
                  {data.steps.map((step) => {
                    const isDone = step.status === "done";
                    const isRunning = step.status === "in_progress";
                    return (
                      <div
                        key={step.index}
                        className={`group flex items-start gap-2.5 p-2 rounded-lg transition-all ${
                          isDone
                            ? "bg-green-500/5 text-inkdim"
                            : isRunning
                            ? "bg-emerald-500/10 text-emerald-300 font-medium border border-emerald-500/20"
                            : "bg-panel3/30 text-ink hover:bg-panel3/60"
                        }`}
                      >
                        {/* 状态图标（点击快速轮换） */}
                        <button
                          type="button"
                          className="shrink-0 mt-0.5 cursor-pointer hover:scale-115 transition-transform"
                          title={`当前状态: ${step.status}，点击切换`}
                          onClick={() => cycleStepStatus(step.index, step.status)}
                        >
                          {isDone ? (
                            <CheckCircle2 size={16} className="text-green-400" />
                          ) : isRunning ? (
                            <Clock size={16} className="text-emerald-400 animate-spin" />
                          ) : (
                            <Circle size={16} className="text-inkdim/60 hover:text-ink" />
                          )}
                        </button>

                        <span
                          className={`flex-1 leading-relaxed cursor-pointer select-text ${
                            isDone ? "line-through opacity-70" : ""
                          }`}
                          onClick={() => cycleStepStatus(step.index, step.status)}
                        >
                          {step.content}
                        </span>

                        {/* 悬浮快捷切换状态选项 */}
                        <div className="opacity-0 group-hover:opacity-100 flex items-center gap-1 shrink-0 text-[11px] font-mono transition-opacity">
                          <button
                            type="button"
                            className={`px-1.5 py-0.5 rounded cursor-pointer transition-colors ${
                              step.status === "pending"
                                ? "bg-panel border border-edge text-ink"
                                : "text-inkdim hover:text-ink hover:bg-panel3"
                            }`}
                            onClick={() => handleStepStatusChange(step.index, "pending")}
                          >
                            待办
                          </button>
                          <button
                            type="button"
                            className={`px-1.5 py-0.5 rounded cursor-pointer transition-colors ${
                              step.status === "in_progress"
                                ? "bg-emerald-500/20 text-emerald-300 border border-emerald-500/30"
                                : "text-inkdim hover:text-emerald-400 hover:bg-emerald-500/10"
                            }`}
                            onClick={() => handleStepStatusChange(step.index, "in_progress")}
                          >
                            进行中
                          </button>
                          <button
                            type="button"
                            className={`px-1.5 py-0.5 rounded cursor-pointer transition-colors ${
                              step.status === "done"
                                ? "bg-green-500/20 text-green-300 border border-green-500/30"
                                : "text-inkdim hover:text-green-400 hover:bg-green-500/10"
                            }`}
                            onClick={() => handleStepStatusChange(step.index, "done")}
                          >
                            已完成
                          </button>
                        </div>
                      </div>
                    );
                  })}
                </div>
              </div>
            )}


            {/* Markdown 方案正文 */}
            <div className="bg-panel2/40 border border-edge/60 rounded-xl p-5 shadow-2xs">
              <div className="text-[12px] font-semibold text-inkdim uppercase tracking-wider mb-3">
                方案文档正文
              </div>
              <Markdown content={data.body} workspacePath={tab.workspacePath} />
            </div>
          </>
        )}
      </div>
    </div>
  );
}
