import { useState } from "react";
import { ipc } from "../ipc";
import { useStore } from "../store";
import {
  ShieldAlert,
  ShieldCheck,
  Monitor,
  Database,
  Loader2,
  RefreshCw,
} from "./Icons";

interface AuthCardProps {
  sessionId: string;
  targetUrl?: string;
  reason?: string;
  onHealed?: () => void;
}

export function AuthCard({ sessionId, targetUrl, reason, onHealed }: AuthCardProps) {
  const [restoring, setRestoring] = useState(false);
  const [healed, setHealed] = useState(false);
  const pushToast = useStore((s) => s.pushToast);
  const setBrowserInspectorTab = useStore((s) => s.setBrowserInspectorTab);

  const handleRestoreStorageState = async () => {
    if (!sessionId || restoring) return;
    setRestoring(true);
    try {
      pushToast("正在尝试从本地持久化快照注入登录态凭据...", "info");
      const ok = await ipc.restoreBrowserStorageState(sessionId);
      if (ok) {
        setHealed(true);
        pushToast("登录态快照注入成功，页面已重载", "success");
        onHealed?.();
      } else {
        pushToast("未找到当前站点的登录态快照，请点击下方按钮在视窗内首次登录并固化", "warning");
      }
    } catch (err: any) {
      pushToast(`快照注入失败: ${err}`, "error");
    } finally {
      setRestoring(false);
    }
  };

  const handleOpenInspector = (tab: "view" | "storage") => {
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
              <span>{healed ? "登录态恢复已完成" : "浏览器鉴权拦截 (StorageState Gateway)"}</span>
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
                ? `页面处于未登录态 (检测原因: ${reason})`
                : "目标页面检测到未登录重定向或 Token 失效，可一键注入已固化快照或打开视窗人工完成首次登录。"}
              {targetUrl && (
                <span className="ml-1 font-mono text-ink/80 text-[11px]">[{targetUrl}]</span>
              )}
            </div>
          </div>
        </div>
      </div>

      <div className="flex flex-wrap items-center gap-2 pt-1 border-t border-edge/40">
        <button
          onClick={handleRestoreStorageState}
          disabled={restoring}
          className="px-3 py-1.5 rounded-lg bg-blue-600 hover:bg-blue-500 text-white font-medium text-[11.5px] flex items-center gap-1.5 disabled:opacity-50 transition-colors cursor-pointer shadow-xs"
        >
          {restoring ? (
            <Loader2 size={13} className="animate-spin" />
          ) : (
            <RefreshCw size={13} />
          )}
          <span>{restoring ? "正在恢复登录态..." : "🔄 注入登录态快照"}</span>
        </button>

        <button
          onClick={() => handleOpenInspector("view")}
          className="px-3 py-1.5 rounded-lg border border-edge bg-panel hover:bg-panel3 text-ink text-[11.5px] font-medium flex items-center gap-1.5 transition-colors cursor-pointer"
        >
          <Monitor size={13} className="text-blue-400" />
          <span>🚀 视窗协助登录</span>
        </button>

        <button
          onClick={() => handleOpenInspector("storage")}
          className="px-3 py-1.5 rounded-lg border border-edge bg-panel hover:bg-panel3 text-inkdim hover:text-ink text-[11.5px] flex items-center gap-1.5 transition-colors cursor-pointer"
        >
          <Database size={13} className="text-emerald-400" />
          <span>💾 登录态快照管理</span>
        </button>
      </div>
    </div>
  );
}
