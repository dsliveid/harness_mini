import { listen, UnlistenFn } from "@tauri-apps/api/event";
import { useEffect } from "react";
import { useStore } from "./store";
import type { Message, Session } from "./types";

export function useAppEvents() {
  useEffect(() => {
    const s = useStore.getState();
    const unlisteners: UnlistenFn[] = [];
    // StrictMode 下 effect 会立即 cleanup 一次，而注销函数要等 Promise 返回后才入队；
    // 若 cleanup 先发生，必须在 Promise 返回时直接注销，否则监听器残留导致
    // 每个事件被处理两次（表现为流式输出内容重复）
    let cancelled = false;
    const regs: Promise<UnlistenFn>[] = [
      listen<any>("message:delta", (e) => s.onMessageDelta(e.payload)),
      listen<any>("message:reasoning:delta", (e) => s.onMessageReasoningDelta(e.payload)),
      listen<Message>("message:final", (e) => s.onMessageFinal(e.payload as Message)),
      listen<any>("tool:update", (e) => s.onToolUpdate(e.payload)),
      listen<any>("tool:output", (e) => s.onToolOutput(e.payload)),
      listen<any>("approval:request", (e) => s.onApprovalRequest(e.payload)),
      // 切到「完全访问」时后端会放行挂起的审批条，前端据此收起审批卡片
      listen<any>("approval:resolved", (e) => s.approvalDone(e.payload?.eventId)),
      listen<any>("run:status", (e) => s.onRunStatus(e.payload)),
      listen<any>("queue:update", (e) => s.onQueueUpdate(e.payload)),
      listen<Session>("session:update", (e) => s.onSessionUpdate(e.payload as Session)),
      listen<any>("settings:changed", (e) => s.onSettingsChanged(e.payload)),
      listen<any>("session:rules", (e) => s.onSessionRules(e.payload)),
      listen<any>("sessions:changed", (e) => s.onSessionsChanged(e.payload)),
      listen<any>("projects:changed", () => s.onProjectsChanged()),
      listen<any>("temp:update", (e) => s.onTempUpdate(e.payload)),
      listen<any>("messages:changed", (e) => s.reloadMessages(e.payload?.sessionId)),
      listen<any>("error", (ev) => s.onError(ev.payload)),
    ];
    Promise.all(regs).then((ls) => {
      if (cancelled) {
        ls.forEach((u) => u());
        return;
      }
      unlisteners.push(...ls);
    });
    return () => {
      cancelled = true;
      unlisteners.splice(0).forEach((u) => u());
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
}
