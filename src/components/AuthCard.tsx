import { useState } from "react";
import { ipc } from "../ipc";
import { useStore } from "../store";
import {
  ShieldAlert,
  ShieldCheck,
  Play,
  Monitor,
  KeyRound,
  Loader2,
  RefreshCw,
  ExternalLink,
} from "./Icons";

interface AuthCardProps {
  sessionId: string;
  targetUrl?: string;
  reason?: string;
  onHealed?: () => void;
}

export function AuthCard({ sessionId, targetUrl, reason, onHealed }: AuthCardProps) {
  const [runningPipeline, setRunningPipeline] = useState(false);
  const [healed, setHealed] = useState(false);
  const pushToast = useStore((s) => s.pushToast);
  const setBrowserInspectorTab = useStore((s) => s.setBrowserInspectorTab);

  const handleRunPipeline = async () => {
    if (!sessionId || runningPipeline) return;
    setRunningPipeline(true);
    try {
      pushToast("正在启动受控浏览器并执行声明式免登流水线...", "info");
      await ipc.executeBrowserAuthPipeline(sessionId);
      setHealed(true);
      pushToast("登录流水线执行成功，页面鉴权已就绪", "success");
      onHealed?.();
    } catch (err: any) {
      pushToast(`流水线执行未成功: ${err}`, "error");
    } finally {
      setRunningPipeline(false);
    }
  };

  const handleOpenInspector = (tab: "view" | "auth") => {
    setBrowserInspectorTab(tab);
  };

  return (
    <div className="my-2 p-3.5 rounded-xl border border-amber-500/30 bg-amber-500/5 flex flex-col gap-3 text-[12px] shadow-xs">
      <div className="flex items-start justify-between gap-3">
        <div className="flex items-center gap-2">
          {healed ? (
            <ShieldCheck size={18} className="text-emerald-400 shrink-0" />
          ) : (
            <ShieldAlert size={18} className="text-amber-400 shrink-0" />
          )}
          <div>
            <div className="font-semibold text-ink flex items-center gap-2">
              <span>{healed ? "鉴权自愈已完成" : "浏览器鉴权拦截 (Auth Gateway)"}</span>
              {healed ? (
                <span className="text-[10.5px] px-2 py-0.5 rounded-full bg-emerald-500/15 text-emerald-400 border border-emerald-500/30 font-normal">
                  Ready · 鉴权就绪
                </span>
              ) : (
                <span className="text-[10.5px] px-2 py-0.5 rounded-full bg-amber-500/15 text-amber-400 border border-amber-500/30 font-normal">
                  需要登录态 / HITL
                </span>
              )}
            </div>
            <div className="text-[11.5px] text-inkdim mt-0.5">
              {reason
                ? `页面处于未鉴权态 (检测原因: ${reason})`
                : "目标页面检测到未登录重定向或 Token 失效，请选择自动流水线或人工协助。"}
              {targetUrl && (
                <span className="ml-1 font-mono text-ink/80 text-[11px]">[{targetUrl}]</span>
              )}
            </div>
          </div>
        </div>
      </div>

      <div className="flex flex-wrap items-center gap-2 pt-1 border-t border-edge/40">
        <button
          onClick={handleRunPipeline}
          disabled={runningPipeline}
          className="px-3 py-1.5 rounded-lg bg-accent text-white font-medium text-[11.5px] flex items-center gap-1.5 hover:bg-accent/90 disabled:opacity-50 transition-colors cursor-pointer shadow-xs"
        >
          {runningPipeline ? (
            <Loader2 size={13} className="animate-spin" />
          ) : (
            <Play size={13} />
          )}
          <span>{runningPipeline ? "正在自愈登录..." : "⚡ 执行免登流水线"}</span>
        </button>

        <button
          onClick={() => handleOpenInspector("view")}
          className="px-3 py-1.5 rounded-lg border border-edge bg-panel hover:bg-panel3 text-ink text-[11.5px] font-medium flex items-center gap-1.5 transition-colors cursor-pointer"
        >
          <Monitor size={13} className="text-blue-400" />
          <span>🚀 立即协助登录 (打开视窗)</span>
        </button>

        <button
          onClick={() => handleOpenInspector("auth")}
          className="px-3 py-1.5 rounded-lg border border-edge bg-panel hover:bg-panel3 text-inkdim hover:text-ink text-[11.5px] flex items-center gap-1.5 transition-colors cursor-pointer"
        >
          <KeyRound size={13} className="text-amber-400" />
          <span>⚙️ 配置凭据契约</span>
        </button>
      </div>
    </div>
  );
}
