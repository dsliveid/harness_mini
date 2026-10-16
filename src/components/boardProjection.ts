import type {
  Message,
  PlanSummary,
  TaskStatus,
  TimelineTask,
  TimelineTaskItem,
} from "../types";

/**
 * 看板（Board）：任务与计划看板的唯一投影产物。
 * 不变量：可见消息流（!revertedAt && !queued && role !== "tool"）是唯一真相源，
 * Board 是其纯函数投影，不存在任何独立实体集合或兜底分支。
 */
export interface Board extends TimelineTask {
  /** 来源消息的 seq：投影排序的唯一依据（弃用 createdAt 字符串比较） */
  sourceSeq: number;
}

/** 投影所需的外部运行态（不参与归属判定，仅决定条目展示状态） */
export interface ProjectionContext {
  /** 当前会话是否正在运行（用于最新轮次条目状态） */
  isRunning?: boolean;
  /** 当前会话最近一次运行结局（cancelled/interrupted/failed/error 视为异常停止） */
  lastRunOutcome?: string;
  /** 允许纳入的方案会话边界（含当前会话，以及关联协作者/父子会话） */
  relatedSessionIds?: ReadonlySet<string>;
}

const ABNORMAL_OUTCOMES = new Set(["cancelled", "interrupted", "failed", "error"]);
const TODO_TITLE = "任务清单";
const PLAN_TITLE_FALLBACK = "任务计划方案";

/**
 * 唯一投影函数：可见消息流 + 方案清单 -> 看板数组（按来源消息 seq 升序）。
 * - todo：按 role === "user" 划分轮次，每轮取最后一个 params.todos 非空的 todo 工具事件快照
 * - plan：按落库归属字段（source_message_id / source_tool_event_id）直取来源消息与关联事件；
 *         **归属为空的方案不进入结果**（老方案文件缺少归属字段，属一次性迁移代价，由后端回填，
 *         此处不做任何兜底或猜测）
 * - 无任何"会话级 todos 顶替消息流"的兜底
 */
export function deriveBoards(
  msgs: Message[],
  plans: PlanSummary[],
  ctx: ProjectionContext = {}
): Board[] {
  const visible = msgs.filter((m) => !m.revertedAt && !m.queued && m.role !== "tool");
  if (visible.length === 0) return [];

  const boards = [...todoBoards(visible, ctx), ...planBoards(visible, plans, ctx)];
  boards.sort(
    (a, b) =>
      a.sourceSeq - b.sourceSeq ||
      (a.type === b.type ? 0 : a.type === "plan" ? -1 : 1)
  );
  return boards;
}

/** todo 投影：按 user 消息分轮，每轮取最后一个有效 todo 快照 */
function todoBoards(visible: Message[], ctx: ProjectionContext): Board[] {
  const turns: Message[][] = [];
  let turn: Message[] = [];
  for (const m of visible) {
    if (m.role === "user") {
      if (turn.length > 0) turns.push(turn);
      turn = [m];
    } else if (m.role === "assistant") {
      turn.push(m);
    }
  }
  if (turn.length > 0) turns.push(turn);

  const isAbnormalStop = ABNORMAL_OUTCOMES.has(ctx.lastRunOutcome ?? "");
  const boards: Board[] = [];

  for (let i = 0; i < turns.length; i++) {
    const isLatestTurn = i === turns.length - 1;
    let lastEv: any = null;
    let lastMsg: Message | null = null;
    const eventIds: string[] = [];

    for (const a of turns[i]) {
      if (a.role !== "assistant") continue;
      for (const ev of a.toolEvents ?? []) {
        if (ev.toolName !== "todo") continue;
        const todos = ev.params?.todos;
        if (!Array.isArray(todos) || todos.length === 0) continue;
        eventIds.push(ev.id);
        lastEv = ev;
        lastMsg = a;
      }
    }
    if (!lastEv || !lastMsg) continue;

    const isTaskRunning = !!ctx.isRunning && isLatestTurn;
    const items: TimelineTaskItem[] = lastEv.params.todos.map((t: any, idx: number) => {
      let status: TaskStatus;
      if (isTaskRunning) {
        status = t.status || "pending";
      } else if (isAbnormalStop && isLatestTurn) {
        status = t.status === "in_progress" ? "pending" : t.status || "pending";
      } else {
        // 正常交付结束（或历史轮次已沉淀）：未完成项一律视为已完成
        status = !t.status || t.status === "in_progress" || t.status === "pending" ? "done" : t.status;
      }
      return { index: idx + 1, content: String(t.content || ""), status };
    });

    const allDone = items.every((t) => t.status === "done");
    boards.push({
      id: `todo-${lastEv.id}`,
      type: "todo",
      title: TODO_TITLE,
      status: isTaskRunning ? "in_progress" : allDone ? "completed" : "pending",
      items,
      messageId: lastMsg.id,
      toolEventId: lastEv.id,
      toolEventIds: eventIds,
      sourceSeq: lastMsg.seq,
    });
  }

  return boards;
}

/** plan 投影：按归属字段直取来源消息与关联事件；无归属或来源不可见即丢弃 */
function planBoards(
  visible: Message[],
  plans: PlanSummary[],
  ctx: ProjectionContext
): Board[] {
  const boards: Board[] = [];
  for (const p of plans) {
    if (p.session_id && ctx.relatedSessionIds && !ctx.relatedSessionIds.has(p.session_id)) {
      continue;
    }
    // 归属为空（null）的方案不进入看板：老方案文件因缺少归属字段而不显示，
    // 这是已知的一次性迁移代价（由后端另行回填），此处不加任何兜底或猜测。
    if (!p.source_message_id) continue;
    const source = visible.find((m) => m.id === p.source_message_id);
    // 来源消息已被撤回/不可见 -> 该方案不显示（撤回后展示自动回退到上一条有效看板）
    if (!source) continue;

    const toolEventId = p.source_tool_event_id ?? undefined;
    boards.push({
      id: p.id,
      type: "plan",
      title: p.title || PLAN_TITLE_FALLBACK,
      version: p.version,
      filename: p.filename,
      status: p.status as TaskStatus,
      items: p.steps
        ? p.steps.map((s) => ({
            index: s.index,
            content: `步骤 ${s.index}：${s.content}`,
            status: s.status,
          }))
        : [],
      body: p.body || "",
      messageId: source.id,
      toolEventId,
      toolEventIds: toolEventId ? [toolEventId] : [],
      sourceSeq: source.seq,
    });
  }
  return boards;
}
