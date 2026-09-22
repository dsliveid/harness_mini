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
      listen<any>("compaction:request", (e) => s.onCompactionRequest(e.payload)),
      listen<any>("compaction:resolved", (e) => s.compactionDone(e.payload?.eventId)),
      listen<any>("compaction:timeout", (e) => s.onCompactionTimeout(e.payload?.eventId)),
      listen<any>("compaction:applied", (e) => s.onCompactionApplied(e.payload)),
      listen<any>("session:compacted", (e) => s.onSessionCompacted(e.payload?.sessionId)),
      listen<any>("context:truncated", (e) => s.onTruncationNotice(e.payload)),
      listen<any>("run:status", (e) => s.onRunStatus(e.payload)),
      listen<any>("queue:update", (e) => s.onQueueUpdate(e.payload)),
      listen<Session>("session:update", (e) => s.onSessionUpdate(e.payload as Session)),
      listen<any>("settings:changed", (e) => s.onSettingsChanged(e.payload)),
      listen<any>("session:rules", (e) => s.onSessionRules(e.payload)),
      listen<any>("session:todos", (e) => s.onSessionTodos(e.payload)),
      listen<any>("sessions:changed", (e) => s.onSessionsChanged(e.payload)),
      listen<any>("projects:changed", () => s.onProjectsChanged()),
      listen<any>("temp:update", (e) => s.onTempUpdate(e.payload)),
      listen<any>("messages:changed", (e) => s.reloadMessages(e.payload?.sessionId)),
      listen<any>("growth:proposed", (e) => s.onGrowthProposed(e.payload)),
      listen<any>("growth:status", (e) => s.onGrowthStatus(e.payload)),
      listen<any>("growth:updated", (e) => s.onGrowthUpdated(e.payload)),
      listen<any>("growth:deleted", (e) => s.onGrowthDeleted(e.payload?.id)),
      listen<any>("sop:status", (e) => s.onSopStatus(e.payload)),
      listen<any>("tool:retry_guidance", (e) => s.onToolRetryGuidance(e.payload)),
      listen<any>("collaborators:changed", (e) => s.onCollaboratorsChanged(e.payload)),
      listen<Session>("collaborator:created", (e) => s.onCollaboratorCreated(e.payload as Session)),
      listen<any>("collaborator:update", (e) => s.onCollaboratorUpdate(e.payload)),
      listen<any>("collaborator:updated", (e) => s.onCollaboratorUpdate(e.payload)),
      listen<any>("collaborator:reported", (e) => s.onCollaboratorReported(e.payload)),
      listen<any>("subprocesses:changed", (e) => s.loadSubprocesses(e.payload?.parentId || e.payload?.parentSessionId)),
      listen<any>("subprocess:created", (e) => {
        const pid = e.payload?.parentId || e.payload?.parentSessionId;
        const sub = e.payload?.subprocess;
        const toolEventId = e.payload?.toolEventId;
        if (pid && sub) {
          useStore.setState((st) => {
            const existing = st.subprocesses[pid] ?? [];
            const updated = existing.some((s) => s.id === sub.id)
              ? existing.map((s) => (s.id === sub.id ? sub : s))
              : [...existing, sub];

            // 若有对应的工具调用事件，立即打上强关联
            let nextMessages = st.messages;
            if (toolEventId && st.messages[pid]) {
              const msgs = st.messages[pid].map((m) => {
                if (!m.toolEvents || m.toolEvents.length === 0) return m;
                const updatedEvs = m.toolEvents.map((ev) => {
                  if (ev.id === toolEventId) {
                    return {
                      ...ev,
                      subprocessId: sub.id,
                      params: { ...(ev.params || {}), subprocess_id: sub.id },
                    };
                  }
                  return ev;
                });
                return { ...m, toolEvents: updatedEvs };
              });
              nextMessages = { ...st.messages, [pid]: msgs };
            }

            return {
              subprocesses: { ...st.subprocesses, [pid]: updated },
              messages: nextMessages,
            };
          });
        }
        if (pid) void s.loadSubprocesses(pid);
      }),
      listen<any>("subagents:changed", (e) => s.onSubagentsChanged(e.payload)),
      listen<Session>("subagent:created", (e) => s.onSubagentCreated(e.payload as Session)),
      listen<any>("subagent:update", (e) => s.onSubagentUpdate(e.payload)),
      listen<any>("task:update", (e) => s.onTaskUpdate(e.payload)),
      listen<any>("task:checkpoint", (e) => s.onTaskCheckpoint(e.payload)),
      listen<any>("task:finished", () => s.pushToast("🎉 长任务已圆满完成！", "success")),
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
