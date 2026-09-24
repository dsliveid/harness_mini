import { useState, useEffect, useCallback, useMemo, useRef } from "react";
import { currentMessages, currentSession, useStore } from "../store";
import { ipc } from "../ipc";
import {
  type ActivePlanDetail,
  type PlanSummary,
  type TimelineTask,
  type TimelineTaskItem,
  type TaskStatus,
  type TodoItem,
  type ToolEvent,
  type FocusFloatingTarget,
  DRAFT_ID,
} from "../types";
import { Markdown } from "./Markdown";
import {
  CheckSquare,
  CheckCircle2,
  Loader2,
  Circle,
  Terminal,
  ChevronLeft,
  ChevronRight,
  Copy,
  Check,
  X,
  RotateCcw,
  ClipboardList,
  Zap,
} from "./Icons";

/** 从当前会话消息中提取所有正在执行的 run_command 工具事件 */
function useRunningCommands(): ToolEvent[] {
  const msgs = useStore((s) => currentMessages(s));
  const result: ToolEvent[] = [];
  for (const m of msgs) {
    if (!m.toolEvents) continue;
    for (const ev of m.toolEvents) {
      if (ev.toolName === "run_command" && ev.status === "running") {
        result.push(ev);
      }
    }
  }
  return result;
}

function TodoSection({
  todos,
  isRunning,
  onToggle,
}: {
  todos: { index: number; content: string; status: string }[];
  isRunning: boolean;
  onToggle?: (index: number, currentStatus: string) => void;
}) {
  const done = todos.filter((t) => t.status === "done").length;
  const total = todos.length;
  const pct = total > 0 ? Math.round((done / total) * 100) : 0;
  return (
    <div className="flex flex-col gap-1.5">
      <div className="flex items-center gap-2 text-[11px] text-inkdim">
        <span className="flex items-center gap-1 font-medium">
          <CheckSquare size={13} className="text-purple-400" />
          <span>任务步骤清单</span>
        </span>
        <span className="font-mono">{done}/{total}</span>
        <div className="flex-1 h-1 bg-panel3 rounded-full overflow-hidden">
          <div
            className="h-full bg-green-500 rounded-full transition-all duration-300"
            style={{ width: `${pct}%` }}
          />
        </div>
      </div>
      <div className="flex flex-col gap-1 max-h-[180px] overflow-y-auto pr-1">
        {todos.map((t) => (
          <div
            key={t.index}
            className={`group flex items-start gap-1.5 text-[12px] leading-snug p-0.5 rounded transition-colors ${
              onToggle ? "hover:bg-panel3/60 cursor-pointer select-none" : ""
            }`}
            onClick={() => onToggle?.(t.index, t.status)}
            title={onToggle ? "点击切换完成状态" : undefined}
          >
            <span className="shrink-0 mt-0.5 group-hover:scale-110 transition-transform">
              {t.status === "done" ? (
                <CheckCircle2 size={13} className="text-green-400" />
              ) : t.status === "in_progress" && isRunning ? (
                <Loader2 size={13} className="text-blue-400 animate-spin" />
              ) : (
                <Circle size={13} className="text-inkdim/50 group-hover:text-inkdim" />
              )}
            </span>
            <span className={t.status === "done" ? "text-inkdim line-through" : "text-ink"}>
              {t.content}
            </span>
          </div>
        ))}
      </div>
    </div>
  );
}

function RunningCommandCard({ ev }: { ev: ToolEvent }) {
  const output = useStore((s) => s.toolOutputs[ev.id]);
  const killCommand = useStore((s) => s.killCommand);
  const pushToast = useStore((s) => s.pushToast);
  const [copied, setCopied] = useState(false);
  const command = ev.params?.command ?? "";
  const lastLine = output?.trim().split("\n").pop() ?? "";
  const [terminating, setTerminating] = useState(false);

  const handleKill = async () => {
    if (terminating) return;
    setTerminating(true);
    try {
      await killCommand(ev.id);
    } catch {
      setTerminating(false);
    }
  };

  const handleCopy = () => {
    if (!command) return;
    navigator.clipboard.writeText(command).then(
      () => {
        setCopied(true);
        setTimeout(() => setCopied(false), 1500);
        pushToast("命令行已复制到剪贴板");
      },
      () => pushToast("复制失败")
    );
  };

  return (
    <div className="flex flex-col gap-1">
      <div className="flex items-center gap-1.5">
        <span
          className={`w-1.5 h-1.5 rounded-full shrink-0 ${
            terminating ? "bg-amber-400 animate-ping" : "bg-green-400 animate-pulse"
          }`}
        />
        <span className="text-[11px] text-inkdim truncate flex-1 min-w-0 font-mono" title={command}>
          {command.length > 40 ? command.slice(0, 40) + "…" : command}
        </span>
        <button
          className="shrink-0 p-0.5 rounded hover:bg-panel3 text-inkdim hover:text-ink transition-colors cursor-pointer"
          title="复制命令行"
          onClick={handleCopy}
        >
          {copied ? <Check size={11} className="text-green-400" /> : <Copy size={11} />}
        </button>
        <button
          className="shrink-0 text-[10px] px-1.5 py-0.5 rounded bg-red-600/80 hover:bg-red-500 text-white disabled:opacity-50 disabled:cursor-not-allowed transition-all cursor-pointer"
          title={terminating ? "正在终止进程…" : "终止进程"}
          disabled={terminating}
          onClick={handleKill}
        >
          {terminating ? "终止中…" : "终止"}
        </button>
      </div>
      {lastLine && (
        <pre className="text-[10px] font-mono text-inkdim truncate max-w-full" title={lastLine}>
          {lastLine.length > 60 ? lastLine.slice(-60) : lastLine}
        </pre>
      )}
    </div>
  );
}

export function FloatingTaskPanel() {
  const currentId = useStore((s) => s.currentId);
  const currentWorkspace = useStore(
    (s) =>
      s.sessions.find((x) => x.id === s.currentId)?.workspacePath ||
      s.draft?.workspacePath ||
      s.projects.find(
        (p) =>
          p.id ===
          (s.sessions.find((sess) => sess.id === s.currentId)?.projectId || s.draft?.projectId)
      )?.path ||
      ""
  );
  const isRunning = useStore((s) => (s.currentId ? s.runStatus[s.currentId] === "running" : false));
  const rawTodos: TodoItem[] = useStore((s) => (s.currentId ? s.sessionTodos[s.currentId] ?? [] : []));
  const runningCmds = useRunningCommands();
  const msgs = useStore((s) => currentMessages(s));
  const pushToast = useStore((s) => s.pushToast);
  const focusFloatingTaskId = useStore((s) => s.focusFloatingTaskId);
  const setFocusFloatingTaskId = useStore((s) => s.setFocusFloatingTaskId);

  // 会话隔离上下文：当前会话及其关联协作者/父子会话
  const currentSessionObj = useStore((s) => currentSession(s));
  const collabs = useStore((s) => (s.currentId ? s.collaborators[s.currentId] ?? [] : []));
  const subprocesses = useStore((s) => (s.currentId ? s.subprocesses[s.currentId] ?? [] : []));
  const subagents = useStore((s) => (s.currentId ? s.subagents[s.currentId] ?? [] : []));

  const relatedSessionIds = useMemo(() => {
    const set = new Set<string>();
    if (currentId && currentId !== DRAFT_ID) set.add(currentId);
    if (currentSessionObj?.parentSessionId) set.add(currentSessionObj.parentSessionId);
    collabs.forEach((c) => set.add(c.id));
    subprocesses.forEach((p) => set.add(p.id));
    subagents.forEach((a) => set.add(a.id));
    return set;
  }, [currentId, currentSessionObj?.parentSessionId, collabs, subprocesses, subagents]);

  const [collapsed, setCollapsed] = useState(false);
  const [plans, setPlans] = useState<PlanSummary[]>([]);
  const [planDetails, setPlanDetails] = useState<Record<string, ActivePlanDetail>>({});
  const [selectedTaskId, setSelectedTaskId] = useState<string | null>(null);
  const [showPlanModal, setShowPlanModal] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const [highlightedTaskId, setHighlightedTaskId] = useState<string | null>(null);
  const retryCountRef = useRef(0);

  // 1. 获取当前会话关联的所有方案文档与详细信息
  const fetchPlans = useCallback(async () => {
    if (!currentId || currentId === DRAFT_ID) {
      setPlans([]);
      setPlanDetails({});
      return;
    }
    setRefreshing(true);
    try {
      const summaries = currentWorkspace
        ? await ipc.listWorkspacePlans(currentWorkspace, currentId, true)
        : [];

      // 前端二次安全过滤：确保仅包含归属于当前会话或关联会话的方案
      const filteredSummaries = summaries.filter(
        (p) => !p.session_id || p.session_id === currentId || relatedSessionIds.has(p.session_id)
      );

      const detailsMap: Record<string, ActivePlanDetail> = {};
      for (const p of filteredSummaries) {
        try {
          const detail = await ipc.getPlanDetail(currentWorkspace, currentId, p.id);
          if (detail) {
            detailsMap[p.id] = detail;
          }
        } catch {
          // ignore error for single plan
        }
      }

      // 兜底：若未列出，尝试直接拉取活动计划
      if (filteredSummaries.length === 0) {
        try {
          const active = await ipc.getActivePlan(currentId);
          if (
            active &&
            active.meta &&
            (!active.meta.session_id ||
              active.meta.session_id === currentId ||
              relatedSessionIds.has(active.meta.session_id))
          ) {
            filteredSummaries.push({
              id: active.meta.id,
              title: active.meta.title,
              status: active.meta.status,
              version: active.meta.version,
              filename: active.filename,
              created_at: active.meta.created_at,
              updated_at: active.meta.updated_at,
              session_id: active.meta.session_id,
              total_steps: active.steps?.length ?? 0,
              completed_steps: active.steps?.filter((s) => s.status === "done").length ?? 0,
              is_active: active.meta.status === "in_progress",
            });
            detailsMap[active.meta.id] = active;
          }
        } catch {
          // ignore
        }
      }

      setPlans(filteredSummaries);
      setPlanDetails(detailsMap);
    } catch (err) {
      console.error("加载方案清单失败:", err);
    } finally {
      setRefreshing(false);
    }
  }, [currentId, currentWorkspace, relatedSessionIds]);

  useEffect(() => {
    fetchPlans();
  }, [currentId, isRunning, fetchPlans]);

  // 监听消息流中最近发生的方案工具更新（create_plan / update_plan / switch_plan）
  const lastPlanEventKey = useMemo(() => {
    for (let i = msgs.length - 1; i >= 0; i--) {
      const evs = msgs[i].toolEvents;
      if (!evs) continue;
      const found = evs.find((e) => ["create_plan", "update_plan", "switch_plan"].includes(e.toolName));
      if (found) return `${found.id}-${found.status}`;
    }
    return null;
  }, [msgs]);

  useEffect(() => {
    if (lastPlanEventKey) {
      fetchPlans();
    }
  }, [lastPlanEventKey, fetchPlans]);

  // 2. 从消息流中提取所有独立的 todo 任务清单（按轮次/消息聚合）
  const todoTasks = useMemo(() => {
    const list: TimelineTask[] = [];
    if (!currentId || currentId === DRAFT_ID) return list;

    let turnCount = 0;
    for (let i = 0; i < msgs.length; i++) {
      const m = msgs[i];
      if (m.role !== "assistant" || !m.toolEvents) continue;

      const todoEvs = m.toolEvents.filter((ev) => ev.toolName === "todo" && Array.isArray(ev.params?.todos));
      if (todoEvs.length === 0) continue;
      const lastTodoEv = todoEvs[todoEvs.length - 1];
      const rawItems = lastTodoEv.params?.todos ?? [];
      if (!Array.isArray(rawItems) || rawItems.length === 0) continue;

      turnCount += 1;
      const isLatestMsg = i === msgs.length - 1;
      const isTaskRunning = isRunning && isLatestMsg;

      const items: TimelineTaskItem[] = rawItems.map((t: any, idx: number) => ({
        index: idx + 1,
        content: String(t.content || ""),
        status: isTaskRunning
          ? (t.status || "pending")
          : t.status === "in_progress"
          ? "done"
          : t.status || "done",
      }));

      const allDone = items.every((t) => t.status === "done");
      const status: TaskStatus = isTaskRunning ? "in_progress" : allDone ? "completed" : "pending";

      list.push({
        id: `todo-${lastTodoEv.id}`,
        type: "todo",
        title: `第 ${turnCount} 轮任务清单`,
        status,
        items,
        messageId: m.id,
        toolEventId: lastTodoEv.id,
        createdAt: m.createdAt || "",
      });
    }

    // 兜底：若消息流中未解析出 todo 任务，但 store 中有活跃的 rawTodos
    if (list.length === 0 && rawTodos.length > 0 && currentId !== DRAFT_ID) {
      const items: TimelineTaskItem[] = rawTodos.map((t, idx) => ({
        index: idx + 1,
        content: t.content,
        status: isRunning ? t.status : t.status === "in_progress" ? "done" : t.status,
      }));
      list.push({
        id: "raw-session-todos",
        type: "todo",
        title: "当前任务清单",
        status: isRunning ? "in_progress" : "completed",
        items,
      });
    }

    return list;
  }, [currentId, msgs, rawTodos, isRunning]);

  // 3. 将物理方案转换为 TimelineTask，并关联其在对话消息流中的出处位置（收集所有关联事件，支持多事件反查）
  const planTasks = useMemo(() => {
    const list: TimelineTask[] = [];
    if (!currentId || currentId === DRAFT_ID) return list;

    for (const p of plans) {
      // 严格会话边界隔离：方案所属的 session_id 必须与当前会话（或关联协作者/父子会话）相匹配
      if (p.session_id && p.session_id !== currentId && !relatedSessionIds.has(p.session_id)) {
        continue;
      }

      const detail = planDetails[p.id];
      const items: TimelineTaskItem[] = detail?.steps
        ? detail.steps.map((s) => ({
            index: s.index,
            content: `步骤 ${s.index}：${s.content}`,
            status: s.status,
          }))
        : [];

      // 在消息流中寻找生成/更新该方案的所有工具事件
      let foundMsgId: string | undefined;
      let foundEventId: string | undefined;
      const matchedEventIds: string[] = [];

      for (const m of msgs) {
        if (!m.toolEvents) continue;
        for (const ev of m.toolEvents) {
          if (["create_plan", "update_plan", "switch_plan", "read_plan"].includes(ev.toolName)) {
            const evTitle = typeof ev.params?.title === "string" ? ev.params.title.trim().toLowerCase() : "";
            const planTitle = (p.title || "").trim().toLowerCase();
            const evPlanId = typeof ev.params?.plan_id === "string" ? ev.params.plan_id.trim() : "";
            const pFilename = p.filename || "";

            const matchesTitle = evTitle && planTitle && (evTitle === planTitle || planTitle.includes(evTitle) || evTitle.includes(planTitle));
            const matchesId = evPlanId && (evPlanId === p.id || p.id.includes(evPlanId));
            const matchesFilename = pFilename && ev.resultText && ev.resultText.includes(pFilename);
            const matchesResultPlanId = p.id && ev.resultText && ev.resultText.includes(p.id);

            if (matchesTitle || matchesId || matchesFilename || matchesResultPlanId) {
              if (!foundMsgId) foundMsgId = m.id;
              if (!foundEventId) foundEventId = ev.id;
              matchedEventIds.push(ev.id);
            }
          }
        }
      }

      // 兜底容错：若当前只有 1 个方案且未通过精确条件匹配，将对话中唯一的方案类事件关联上
      if (matchedEventIds.length === 0 && plans.length === 1) {
        for (const m of msgs) {
          if (!m.toolEvents) continue;
          for (const ev of m.toolEvents) {
            if (["create_plan", "update_plan", "switch_plan"].includes(ev.toolName)) {
              if (!foundMsgId) foundMsgId = m.id;
              if (!foundEventId) foundEventId = ev.id;
              matchedEventIds.push(ev.id);
            }
          }
        }
      }

      list.push({
        id: p.id,
        type: "plan",
        title: p.title || "任务计划方案",
        version: p.version,
        filename: p.filename,
        status: p.status as TaskStatus,
        items,
        body: detail?.body || "",
        messageId: foundMsgId,
        toolEventId: foundEventId,
        toolEventIds: matchedEventIds,
        createdAt: p.created_at,
        updatedAt: p.updated_at,
      });
    }
    return list;
  }, [plans, planDetails, msgs, currentId, relatedSessionIds]);

  // 4. 统一任务时间线（按时间升序排列，最新项排在末尾）
  const timeline: TimelineTask[] = useMemo(() => {
    const combined = [...planTasks, ...todoTasks];
    combined.sort((a, b) => {
      const tA = a.createdAt ? new Date(a.createdAt).getTime() : 0;
      const tB = b.createdAt ? new Date(b.createdAt).getTime() : 0;
      return tA - tB;
    });
    return combined;
  }, [planTasks, todoTasks]);

  // 匹配特定任务/方案的逻辑（支持按 eventId, planId, filename, title 综合判定）
  const matchTarget = useCallback((targetInfo: string | FocusFloatingTarget, t: TimelineTask): boolean => {
    if (typeof targetInfo === "string") {
      const q = targetInfo.trim();
      if (!q) return false;
      return (
        t.id === q ||
        t.toolEventId === q ||
        (t.toolEventIds?.includes(q) ?? false) ||
        t.id === `todo-${q}` ||
        (t.type === "plan" && ((t.filename && t.filename === q) || (t.filename?.includes(q) ?? false) || t.id.includes(q)))
      );
    }

    const { eventId, planId, filename, title } = targetInfo;
    // 1. 按 eventId 匹配
    if (eventId) {
      if (
        t.id === eventId ||
        t.toolEventId === eventId ||
        (t.toolEventIds?.includes(eventId) ?? false) ||
        t.id === `todo-${eventId}`
      ) {
        return true;
      }
    }
    // 2. 按 planId 匹配
    if (planId && t.type === "plan") {
      if (t.id === planId || t.id.includes(planId) || (t.filename && t.filename.includes(planId))) {
        return true;
      }
    }
    // 3. 按 filename 匹配
    if (filename && t.type === "plan" && t.filename) {
      const cleanF = filename.replace(/\\/g, "/").split("/").pop() || filename;
      const cleanT = t.filename.replace(/\\/g, "/").split("/").pop() || t.filename;
      if (cleanF === cleanT || cleanT.includes(cleanF) || cleanF.includes(cleanT)) {
        return true;
      }
    }
    // 4. 按 title 匹配
    if (title) {
      const normQuery = title.trim().toLowerCase();
      const normTarget = t.title.trim().toLowerCase();
      if (normQuery && normTarget && (normQuery === normTarget || normTarget.includes(normQuery) || normQuery.includes(normTarget))) {
        return true;
      }
    }
    return false;
  }, []);

  // 5. 响应外部（如 ToolCard 点击「浮窗查看」）的聚焦指令（带异步加载防抖重试与视觉高亮）
  useEffect(() => {
    if (!focusFloatingTaskId) {
      retryCountRef.current = 0;
      return;
    }

    const found = timeline.find((t) => matchTarget(focusFloatingTaskId, t));
    if (found) {
      setSelectedTaskId(found.id);
      setCollapsed(false);
      setHighlightedTaskId(found.id);
      setTimeout(() => {
        setHighlightedTaskId((prev) => (prev === found.id ? null : prev));
      }, 2000);
      setFocusFloatingTaskId(null);
      retryCountRef.current = 0;
      return;
    }

    // 若未直接命中，且重试次数小于 3 次，主动触发 fetchPlans 并在异步返回后由 timeline 变化重新触发本 effect
    if (retryCountRef.current < 3) {
      retryCountRef.current += 1;
      fetchPlans();
      const timer = setTimeout(() => {
        // 等待下一渲染帧重新触发
      }, 250);
      return () => clearTimeout(timer);
    } else {
      // 达到重试上限仍未找到具体项，若时间线非空则兜底展开浮窗
      if (timeline.length > 0) {
        setCollapsed(false);
      }
      setFocusFloatingTaskId(null);
      retryCountRef.current = 0;
    }
  }, [focusFloatingTaskId, timeline, matchTarget, fetchPlans, setFocusFloatingTaskId]);

  // 6. 默认选中逻辑：若未选中或旧项失效，优先选处于进行中的任务，否则选最新任务（末尾项）
  useEffect(() => {
    if (timeline.length === 0) {
      setSelectedTaskId(null);
      return;
    }
    setSelectedTaskId((prev) => {
      if (prev && timeline.some((t) => t.id === prev)) {
        return prev;
      }
      const inProgressTask = timeline.find((t) => t.status === "in_progress");
      return inProgressTask ? inProgressTask.id : timeline[timeline.length - 1].id;
    });
  }, [currentId, timeline]);

  // 当前激活的任务
  const currentTask = useMemo(() => {
    if (timeline.length === 0) return null;
    return timeline.find((t) => t.id === selectedTaskId) || timeline[timeline.length - 1];
  }, [timeline, selectedTaskId]);

  const currentIndex = useMemo(() => {
    if (!currentTask) return -1;
    return timeline.findIndex((t) => t.id === currentTask.id);
  }, [timeline, currentTask]);

  // 步骤勾选切换处理
  const handleToggleStep = async (stepIndex: number, currentStatus: string) => {
    if (!currentTask) return;
    const nextStatus = currentStatus === "done" ? "pending" : "done";

    if (currentTask.type === "plan" && currentWorkspace) {
      const planDetail = planDetails[currentTask.id];
      if (planDetail) {
        const oldSteps = planDetail.steps;
        const updatedSteps = oldSteps.map((s) =>
          s.index === stepIndex ? { ...s, status: nextStatus as "pending" | "done" } : s
        );
        setPlanDetails((prev) => ({
          ...prev,
          [currentTask.id]: { ...planDetail, steps: updatedSteps },
        }));

        try {
          await ipc.updatePlanStepStatus(
            currentWorkspace,
            currentTask.id,
            stepIndex,
            nextStatus,
            currentId || undefined
          );
        } catch (err) {
          console.error("更新计划步骤状态失败:", err);
          setPlanDetails((prev) => ({
            ...prev,
            [currentTask.id]: { ...planDetail, steps: oldSteps },
          }));
        }
      }
    } else if (currentTask.type === "todo" && currentId) {
      const nextRaw = rawTodos.map((t, i) =>
        i + 1 === stepIndex ? { ...t, status: nextStatus as "pending" | "done" } : t
      );
      useStore.setState((st) => ({
        sessionTodos: { ...st.sessionTodos, [currentId]: nextRaw },
      }));
    }
  };

  // 切换方案为活动方案
  const handleSwitchPlan = async (planId: string) => {
    if (!currentWorkspace || !currentId) return;
    try {
      await ipc.switchPlan(currentWorkspace, currentId, planId);
      pushToast("已切换为当前活动方案", "success");
      await fetchPlans();
      setSelectedTaskId(planId);
    } catch (err) {
      console.error("切换活动方案失败:", err);
      pushToast(`切换方案失败: ${err}`, "error");
    }
  };

  // 定位到主对话区中的消息位置
  const handleLocateMessage = (msgId?: string, toolEventId?: string) => {
    if (!msgId && !toolEventId) {
      pushToast("未关联具体的对话消息位置");
      return;
    }
    let el: Element | null = null;
    if (toolEventId) {
      el =
        document.querySelector(`[data-tool-event-id="${toolEventId}"]`) ||
        document.getElementById(`tool-${toolEventId}`);
    }
    if (!el && msgId) {
      el =
        document.querySelector(`[data-message-id="${msgId}"]`) ||
        document.getElementById(`msg-${msgId}`);
    }
    if (el) {
      el.scrollIntoView({ behavior: "smooth", block: "center" });
      el.classList.add("ring-2", "ring-accent", "ring-offset-2", "transition-all", "duration-500");
      setTimeout(() => {
        el?.classList.remove("ring-2", "ring-accent", "ring-offset-2");
      }, 2000);
    } else {
      pushToast("未在主对话流中找到对应消息");
    }
  };

  const hasTasks = timeline.length > 0;
  const hasCmds = runningCmds.length > 0;

  if (!currentId || currentId === DRAFT_ID || (!hasTasks && !hasCmds)) return null;

  // 收起态：靠右吸附抽屉标签
  if (collapsed) {
    const doneCnt = currentTask ? currentTask.items.filter((t) => t.status === "done").length : 0;
    const totalCnt = currentTask ? currentTask.items.length : 0;
    return (
      <div className="absolute top-3 right-0 z-20 select-none">
        <button
          className="group flex items-center gap-2 pl-2.5 pr-2 py-1.5 rounded-l-xl bg-panel2/90 hover:bg-panel2 backdrop-blur-md border-y border-l border-edge hover:border-accent/50 shadow-lg text-[11px] text-inkdim hover:text-ink transition-all cursor-pointer"
          onClick={() => setCollapsed(false)}
          title="展开任务与进程面板"
        >
          <ChevronLeft
            size={13}
            className="text-inkdim group-hover:text-accent transition-transform duration-150 group-hover:-translate-x-0.5 shrink-0"
          />
          {currentTask?.type === "plan" && (
            <span
              className={`text-[10px] px-1.5 py-0.5 rounded font-medium inline-flex items-center gap-1 ${
                currentTask.status === "completed"
                  ? "bg-green-500/15 text-green-400"
                  : currentTask.status === "suspended"
                  ? "bg-amber-500/15 text-amber-400"
                  : "bg-emerald-500/15 text-emerald-400"
              }`}
            >
              <ClipboardList size={11} className="shrink-0" />
              <span>
                {currentTask.version ? `v${currentTask.version} ` : ""}
                {currentTask.status === "completed" ? "已结案" : currentTask.status === "suspended" ? "已挂起" : "计划"}
              </span>
            </span>
          )}
          {currentTask?.type === "todo" && (
            <span className="text-[10px] px-1.5 py-0.5 rounded font-medium bg-purple-500/15 text-purple-400 inline-flex items-center gap-1">
              <CheckSquare size={11} className="shrink-0" />
              <span>清单</span>
            </span>
          )}
          {hasCmds && <span className="w-2 h-2 rounded-full bg-green-400 animate-pulse shrink-0" />}
          {totalCnt > 0 && (
            <span className="flex items-center gap-1 font-medium">
              <CheckSquare
                size={13}
                className={currentTask?.type === "plan" ? "text-emerald-400" : "text-purple-400"}
              />
              <span className="font-mono text-[11px]">{doneCnt}/{totalCnt}</span>
            </span>
          )}
          {timeline.length > 1 && (
            <span className="font-mono text-[10px] text-ink font-semibold bg-panel3 px-1.5 py-0.5 rounded border border-edge">
              {currentIndex + 1}/{timeline.length}
            </span>
          )}
          {hasCmds && !currentTask && (
            <span className="flex items-center gap-1 text-green-400 font-mono">
              <Terminal size={13} className="shrink-0" />
              <span>{runningCmds.length} 进程</span>
            </span>
          )}
        </button>
      </div>
    );
  }

  // 展开态：宽度优化为 325px，双层结构清晰布局
  return (
    <>
      <div className="absolute top-3 right-0 z-20 w-[325px] max-w-[calc(100%-1rem)] rounded-l-2xl bg-panel2/95 backdrop-blur-md border-y border-l border-edge shadow-2xl overflow-hidden select-none animate-in fade-in slide-in-from-right-2 duration-150">
        {/* 第一层：面板总控栏（标题、阶段数、全局动作） */}
        <div className="flex items-center justify-between px-3.5 py-2.5 border-b border-edge bg-panel/60">
          <div className="flex items-center gap-1.5 min-w-0">
            <CheckSquare size={14} className="text-emerald-400 shrink-0" />
            <span className="text-[12px] font-semibold text-ink tracking-tight">任务与计划看板</span>
            {timeline.length > 0 && (
              <span className="px-1.5 py-0.2 rounded-full text-[10px] font-mono font-medium bg-emerald-500/10 text-emerald-400 border border-emerald-500/20 shrink-0">
                {timeline.length} 阶段
              </span>
            )}
          </div>
          <div className="flex items-center gap-1 shrink-0">
            {currentTask && (currentTask.messageId || currentTask.toolEventId) && (
              <button
                type="button"
                className="flex items-center gap-1 text-[11px] text-inkdim hover:text-ink px-1.5 py-0.5 rounded-md hover:bg-panel3 transition-colors cursor-pointer"
                onClick={() => handleLocateMessage(currentTask.messageId, currentTask.toolEventId)}
                title="定位到主对话中生成此任务的消息位置"
              >
                <span>定位出处</span>
              </button>
            )}
            <button
              type="button"
              className="p-1 text-inkdim hover:text-ink rounded-md hover:bg-panel3 transition-colors cursor-pointer"
              onClick={fetchPlans}
              title="重新从工作区与数据库读取方案与任务进度"
            >
              <RotateCcw size={12} className={refreshing ? "animate-spin text-emerald-400" : ""} />
            </button>
            <button
              type="button"
              className="flex items-center gap-0.5 text-[11px] text-inkdim hover:text-ink pl-1.5 pr-1 py-0.5 rounded-md hover:bg-panel3 transition-colors cursor-pointer"
              onClick={() => setCollapsed(true)}
              title="收起至右侧"
            >
              <span>收起</span>
              <ChevronRight size={12} />
            </button>
          </div>
        </div>

        {/* 第二层：多任务选择器与高对比度独立翻页栏（彻底杜绝与标题挤压重合） */}
        {hasTasks && (
          <div className="px-3 py-2 bg-panel3/30 border-b border-edge/60 flex items-center justify-between gap-2 select-none">
            {/* 任务下拉选择菜单 */}
            <div className="flex items-center gap-1.5 flex-1 min-w-0 bg-panel border border-edge rounded-lg px-2 py-1 shadow-xs hover:border-accent/40 transition-colors">
              <span className="shrink-0">
                {currentTask?.type === "plan" ? (
                  <span className="text-[10px] px-1 py-0.2 rounded bg-emerald-500/15 text-emerald-400 font-mono font-medium">方案</span>
                ) : (
                  <span className="text-[10px] px-1 py-0.2 rounded bg-purple-500/15 text-purple-400 font-mono font-medium">清单</span>
                )}
              </span>
              <select
                className="bg-transparent text-[11px] font-medium text-ink truncate outline-none cursor-pointer flex-1 min-w-0"
                value={currentTask?.id ?? ""}
                onChange={(e) => setSelectedTaskId(e.target.value)}
                title="选择查看不同轮次的任务清单或方案文档"
              >
                {timeline.map((t, idx) => {
                  const statusLabel =
                    t.status === "completed" || t.status === "done"
                      ? "已完成"
                      : t.status === "in_progress"
                      ? "执行中"
                      : t.status === "suspended"
                      ? "已挂起"
                      : "待推进";
                  return (
                    <option key={t.id} value={t.id} className="bg-panel text-ink py-1">
                      {idx + 1}. {t.title} ({statusLabel})
                    </option>
                  );
                })}
              </select>
            </div>

            {/* 高对比度、独立清晰的上一页/下一页翻页器 */}
            {timeline.length > 1 && (
              <div className="flex items-center rounded-lg border border-edge/80 bg-panel shadow-xs shrink-0 overflow-hidden">
                <button
                  type="button"
                  disabled={currentIndex <= 0}
                  onClick={() => setSelectedTaskId(timeline[currentIndex - 1].id)}
                  className="p-1.5 text-ink hover:text-accent hover:bg-panel3 disabled:text-inkdim/30 disabled:hover:bg-transparent disabled:cursor-not-allowed cursor-pointer transition-colors"
                  title="查看上一份任务/方案 (上翻)"
                >
                  <ChevronLeft size={13} />
                </button>
                <span className="px-2 py-0.5 text-[11px] font-mono text-ink font-semibold border-x border-edge/60 select-none bg-panel2/50">
                  {currentIndex + 1} / {timeline.length}
                </span>
                <button
                  type="button"
                  disabled={currentIndex >= timeline.length - 1}
                  onClick={() => setSelectedTaskId(timeline[currentIndex + 1].id)}
                  className="p-1.5 text-ink hover:text-accent hover:bg-panel3 disabled:text-inkdim/30 disabled:hover:bg-transparent disabled:cursor-not-allowed cursor-pointer transition-colors"
                  title="查看下一份任务/方案 (下翻)"
                >
                  <ChevronRight size={13} />
                </button>
              </div>
            )}
          </div>
        )}

        {/* 内容展示区 */}
        <div className="px-3.5 py-2.5 flex flex-col gap-2.5 max-h-[min(70vh,480px)] overflow-y-auto">
          {/* 当前选中的计划方案卡片 */}
          {currentTask && currentTask.type === "plan" && (
            <div
              className={`rounded-xl border p-2.5 text-[11.5px] space-y-1.5 transition-all duration-300 ${
                highlightedTaskId === currentTask.id
                  ? "ring-2 ring-purple-400 ring-offset-1 ring-offset-panel shadow-lg shadow-purple-500/20"
                  : ""
              } ${
                currentTask.status === "completed"
                  ? "bg-green-500/10 border-green-500/30"
                  : currentTask.status === "suspended"
                  ? "bg-amber-500/10 border-amber-500/30"
                  : "bg-emerald-500/10 border-emerald-500/20"
              }`}
            >
              <div className="flex items-center justify-between gap-1.5">
                <span
                  className={`font-semibold truncate flex-1 min-w-0 inline-flex items-center gap-1.5 ${
                    currentTask.status === "completed"
                      ? "text-green-400"
                      : currentTask.status === "suspended"
                      ? "text-amber-400"
                      : "text-emerald-400"
                  }`}
                  title={currentTask.title}
                >
                  <ClipboardList size={13} className="shrink-0 text-emerald-400" />
                  <span className="truncate">{currentTask.title}</span>
                </span>
                <div className="flex items-center gap-1 shrink-0">
                  {currentTask.version && (
                    <span className="font-mono text-[10px] text-emerald-300 bg-emerald-500/20 px-1.5 py-0.5 rounded">
                      v{currentTask.version}
                    </span>
                  )}
                  <span
                    className={`text-[10px] px-1.5 py-0.5 rounded font-medium ${
                      currentTask.status === "completed"
                        ? "bg-green-500/20 text-green-300"
                        : currentTask.status === "suspended"
                        ? "bg-amber-500/20 text-amber-300"
                        : "bg-blue-500/20 text-blue-300"
                    }`}
                  >
                    {currentTask.status === "completed"
                      ? "已结案"
                      : currentTask.status === "suspended"
                      ? "已挂起"
                      : "执行中"}
                  </span>
                </div>
              </div>

              <div className="text-[11px] text-inkdim flex items-center justify-between gap-1">
                <span className="truncate font-mono text-[10px]" title={currentTask.filename}>
                  {currentTask.filename || "方案文档.md"}
                </span>
                <div className="flex items-center gap-2 shrink-0">
                  {currentTask.status === "suspended" && (
                    <button
                      type="button"
                      className="text-amber-400 hover:text-amber-300 font-medium hover:underline cursor-pointer inline-flex items-center gap-1"
                      onClick={() => handleSwitchPlan(currentTask.id)}
                      title="将此挂起的方案重新激活为当前会话的主活动方案"
                    >
                      <Zap size={11} className="shrink-0" />
                      <span>设为活动方案</span>
                    </button>
                  )}
                  {currentTask.filename && (
                    <button
                      type="button"
                      className="text-emerald-400 hover:text-emerald-300 font-medium hover:underline cursor-pointer"
                      onClick={() => {
                        const cleanRel = `.harness/plans/${currentTask.filename}`;
                        const absPath = currentWorkspace
                          ? `${currentWorkspace.replace(/\\/g, "/").replace(/\/$/, "")}/${cleanRel}`
                          : cleanRel;
                        ipc.openFileViewer({
                          id: `file:${absPath}`,
                          type: "file",
                          title: currentTask.filename || "方案文档.md",
                          path: absPath,
                          workspacePath: currentWorkspace,
                          sessionId: currentId || undefined,
                        });
                      }}
                      title="以 Markdown 文件形式在独立窗体中查看与编辑方案"
                    >
                      方案文档 (MD)
                    </button>
                  )}
                  {currentTask.body && (
                    <button
                      type="button"
                      className="text-inkdim hover:text-ink hover:underline cursor-pointer"
                      onClick={() => setShowPlanModal(true)}
                    >
                      弹窗
                    </button>
                  )}
                </div>
              </div>
            </div>
          )}

          {/* 当前选中的轻量 Todo 标题卡片 */}
          {currentTask && currentTask.type === "todo" && (
            <div
              className={`rounded-xl border p-2.5 text-[11.5px] space-y-1 transition-all duration-300 ${
                highlightedTaskId === currentTask.id
                  ? "ring-2 ring-purple-400 ring-offset-1 ring-offset-panel shadow-lg shadow-purple-500/20 bg-purple-500/20 border-purple-500/40"
                  : "bg-purple-500/10 border-purple-500/20"
              }`}
            >
              <div className="flex items-center justify-between gap-1.5">
                <span className="font-semibold text-purple-400 truncate flex-1 min-w-0 inline-flex items-center gap-1.5">
                  <CheckSquare size={13} className="shrink-0" />
                  <span className="truncate">{currentTask.title}</span>
                </span>
                <span
                  className={`text-[10px] px-1.5 py-0.5 rounded font-medium ${
                    currentTask.status === "completed" || currentTask.status === "done"
                      ? "bg-green-500/20 text-green-300"
                      : "bg-blue-500/20 text-blue-300"
                  }`}
                >
                  {currentTask.status === "completed" || currentTask.status === "done" ? "已完成" : "执行中"}
                </span>
              </div>
            </div>
          )}

          {/* 运行中的控制台进程 */}
          {hasCmds && (
            <div className="flex flex-col gap-1.5">
              <div className="text-[10px] text-inkdim font-medium flex items-center gap-1">
                <Terminal size={11} className="text-emerald-400" />
                <span>控制台进程</span>
              </div>
              {runningCmds.map((ev) => (
                <RunningCommandCard key={ev.id} ev={ev} />
              ))}
            </div>
          )}

          {/* 分隔线 */}
          {hasCmds && currentTask && currentTask.items.length > 0 && <div className="border-t border-edge" />}

          {/* 任务分步 Checklist */}
          {currentTask && currentTask.items.length > 0 && (
            <div
              className={`rounded-xl transition-all duration-300 ${
                highlightedTaskId === currentTask.id
                  ? "ring-2 ring-purple-400/70 p-1 bg-purple-500/5 rounded-xl"
                  : ""
              }`}
            >
              <TodoSection
                todos={currentTask.items}
                isRunning={isRunning && currentTask.status === "in_progress"}
                onToggle={handleToggleStep}
              />
            </div>
          )}
        </div>
      </div>

      {/* 完整方案预览弹窗 */}
      {showPlanModal && currentTask && currentTask.body && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm p-4 animate-in fade-in duration-150">
          <div className="bg-panel rounded-2xl border border-edge shadow-2xl max-w-2xl w-full max-h-[85vh] flex flex-col overflow-hidden">
            <div className="flex items-center justify-between px-5 py-3.5 border-b border-edge bg-panel2/60">
              <div className="flex items-center gap-2 min-w-0">
                <span className="text-sm font-semibold text-ink truncate">
                  {currentTask.title}
                </span>
                {currentTask.version && (
                  <span className="text-[11px] font-mono px-2 py-0.5 rounded bg-emerald-500/10 text-emerald-400 border border-emerald-500/20">
                    v{currentTask.version}
                  </span>
                )}
                <span
                  className={`text-[10px] px-1.5 py-0.5 rounded font-medium ${
                    currentTask.status === "completed"
                      ? "bg-green-500/20 text-green-300"
                      : currentTask.status === "suspended"
                      ? "bg-amber-500/20 text-amber-300"
                      : "bg-blue-500/20 text-blue-300"
                  }`}
                >
                  {currentTask.status === "completed"
                    ? "已结案"
                    : currentTask.status === "suspended"
                    ? "已挂起"
                    : "执行中"}
                </span>
              </div>
              <button
                type="button"
                className="p-1 rounded-lg hover:bg-panel3 text-inkdim hover:text-ink transition-colors cursor-pointer"
                onClick={() => setShowPlanModal(false)}
              >
                <X size={16} />
              </button>
            </div>
            <div className="p-5 overflow-y-auto flex-1 select-text space-y-3">
              <Markdown content={currentTask.body} />
            </div>
            <div className="px-5 py-3 border-t border-edge bg-panel2/40 flex justify-between items-center text-[11px] text-inkdim">
              <span className="font-mono">
                {currentTask.filename ? `.harness/plans/${currentTask.filename}` : "方案文档"}
              </span>
              <button
                type="button"
                className="px-3 py-1.5 rounded-lg bg-panel3 hover:bg-edge text-ink text-xs transition-colors cursor-pointer"
                onClick={() => setShowPlanModal(false)}
              >
                关闭
              </button>
            </div>
          </div>
        </div>
      )}
    </>
  );
}
