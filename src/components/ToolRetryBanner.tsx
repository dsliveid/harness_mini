import { useState } from "react";
import { useStore } from "../store";
import {
  RefreshCw,
  CheckCircle2,
  AlertCircle,
  AlertTriangle,
  X,
  ChevronDown,
  ChevronUp,
  Copy,
  Check,
} from "./Icons";

export function ToolRetryBanner() {
  const currentId = useStore((s) => s.currentId);
  const toolRetry = useStore((s) => (s.currentId ? s.toolRetryStatus[s.currentId] : null));
  const dismissToolRetry = useStore((s) => s.dismissToolRetry);
  const pushToast = useStore((s) => s.pushToast);
  const [showDetail, setShowDetail] = useState(false);
  const [copied, setCopied] = useState(false);

  if (!currentId || !toolRetry || toolRetry.dismissed) return null;

  const { toolName, attempt, maxRetries, status, error, message } = toolRetry;

  const isRetrying = status === "retrying";
  const isSuccess = status === "success";
  const isFailed = status === "failed";
  const isCancelled = status === "cancelled";

  const handleCopy = (text: string) => {
    if (!text) return;
    navigator.clipboard.writeText(text).then(
      () => {
        setCopied(true);
        pushToast("错误详情已复制到剪贴板");
        setTimeout(() => setCopied(false), 2000);
      },
      (err) => pushToast(`复制失败: ${err}`)
    );
  };

  // 根据不同状态定制主题样式
  const themeClass = isSuccess
    ? "border-emerald-500/40 bg-panel2/95 text-emerald-300 shadow-[0_8px_30px_rgba(16,185,129,0.15)]"
    : isFailed
    ? "border-red-500/40 bg-panel2/95 text-red-300 shadow-[0_8px_30px_rgba(239,68,68,0.15)]"
    : isCancelled
    ? "border-zinc-500/40 bg-panel2/95 text-zinc-300 shadow-xl"
    : "border-purple-500/40 bg-panel2/95 text-purple-300 shadow-[0_8px_30px_rgba(168,85,247,0.15)]";

  return (
    <div className="absolute left-4 top-4 z-30 max-w-[480px] animate-in fade-in slide-in-from-left-2 duration-150">
      <div className={`p-3.5 rounded-2xl border backdrop-blur-md flex flex-col gap-2 ${themeClass}`}>
        {/* 顶部标题与关闭按钮 */}
        <div className="flex items-start justify-between gap-3">
          <div className="flex items-center gap-2 min-w-0">
            {isRetrying && (
              <RefreshCw size={15} className="text-purple-400 animate-spin shrink-0 select-none" />
            )}
            {isSuccess && (
              <CheckCircle2 size={16} className="text-emerald-400 shrink-0 select-none" />
            )}
            {isFailed && (
              <AlertCircle size={16} className="text-red-400 shrink-0 select-none" />
            )}
            {isCancelled && (
              <AlertTriangle size={15} className="text-zinc-400 shrink-0 select-none" />
            )}

            <div className="text-[13px] font-semibold text-ink flex items-center gap-1.5 flex-wrap select-text">
              <span>工具</span>
              <code className="bg-panel px-1.5 py-0.5 rounded text-[12px] font-mono border border-edge/60 text-accent select-text">
                {toolName}
              </code>
              <span>
                {isRetrying && `自纠重试中 (${attempt}/${maxRetries})`}
                {isSuccess && "自纠成功"}
                {isFailed && `自纠未果 (${attempt}/${maxRetries})`}
                {isCancelled && "自纠已中止"}
              </span>
            </div>
          </div>

          <button
            className="shrink-0 w-6 h-6 rounded-lg hover:bg-panel3 flex items-center justify-center text-inkdim hover:text-ink transition-colors cursor-pointer select-none"
            onClick={() => dismissToolRetry(currentId)}
            title="关闭提示"
          >
            <X size={14} />
          </button>
        </div>

        {/* 描述信息 */}
        <div className="text-[12px] text-inkdim leading-relaxed select-text cursor-text">
          {isRetrying && (
            <span>
              执行遇到问题，Agent 正在内部自省分析错误并调整参数进行重试…
            </span>
          )}
          {isSuccess && (
            <span className="text-emerald-300">
              {message || "Agent 已自动修正工具调用参数并执行成功，流程恢复正常进行。"}
            </span>
          )}
          {isFailed && (
            <span className="text-red-300">
              {message || `连续失败已达 ${maxRetries} 次上限，Agent 将终止重复尝试并向你陈述原因。`}
            </span>
          )}
          {isCancelled && (
            <span>当前会话已停止运行，未完成的工具自纠已取消。</span>
          )}
        </div>

        {/* 失败或重试时的错误详情展开 */}
        {error && (isFailed || isRetrying) && (
          <div className="flex flex-col gap-1.5 pt-1.5 border-t border-edge/40">
            <div className="flex items-center justify-between select-none">
              <button
                className="flex items-center gap-1 text-[11px] text-inkdim hover:text-ink transition-colors cursor-pointer"
                onClick={() => setShowDetail(!showDetail)}
              >
                <span>{showDetail ? "收起错误详情" : "查看失败原因详情"}</span>
                {showDetail ? <ChevronUp size={11} /> : <ChevronDown size={11} />}
              </button>

              <button
                className="flex items-center gap-1 px-1.5 py-0.5 rounded text-[11px] text-inkdim hover:text-ink hover:bg-panel3 transition-colors cursor-pointer"
                onClick={() => handleCopy(error)}
                title="复制错误内容到剪贴板"
              >
                {copied ? (
                  <>
                    <Check size={12} className="text-emerald-400" />
                    <span className="text-emerald-400 font-medium">已复制</span>
                  </>
                ) : (
                  <>
                    <Copy size={12} />
                    <span>复制错误</span>
                  </>
                )}
              </button>
            </div>

            {showDetail && (
              <pre className="max-h-48 overflow-y-auto p-2.5 rounded-lg bg-panel text-[11px] font-mono text-red-300/90 whitespace-pre-wrap break-all border border-edge/60 select-text cursor-text">
                {error}
              </pre>
            )}
          </div>
        )}
      </div>
    </div>
  );
}
