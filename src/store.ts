import { create } from "zustand";
import { ipc } from "./ipc";
import { DRAFT_ID, samePath, type ApprovalReq, type ApprovalRule, type Attachment, type CollaboratorCreateInput, type CollaboratorUpdateInput, type CompactionReq, type DataStatus, type GrowthItem, type Message, type Project, type QueuedItem, type Session, type SessionCompaction, type SessionModelsUpdateInput, type Settings, type TempAlloc, type TempInfo, type TodoItem, type ToolEvent, type ToolRetryGuidanceEvent, type ToolRetryStatus, type TruncationNotice, type LongTask, type TaskCheckpoint, type TaskSubItem } from "./types";


export interface EditingMessageTarget {
  messageId: string;
  sessionId: string;
  text: string;
  attachments: Attachment[];
}

export type ToastType = "success" | "error" | "warning" | "info";

export interface Toast {
  id: string;
  text: string;
  type: ToastType;
}

export function inferToastType(text: string): ToastType {
  const t = text.trim();
  if (t.startsWith("✅")) return "success";
  if (t.startsWith("❌")) return "error";
  if (t.startsWith("⚠️")) return "warning";
  if (t.startsWith("ℹ️")) return "info";

  // Error patterns (checked before success/warning to avoid ambiguous matches like "删除失败")
  if (
    /失败|错误|异常|未果|\berror\b|\bfailed\b|\bfaile(d)?\b|\bexception\b|\brejected\b|\bdenied\b|\btimeout\b|\binvalid\b/i.test(
      t
    )
  ) {
    return "error";
  }

  // Warning patterns
  if (
    /警告|\bwarning\b|未通过|自愈|自省|受阻|超限|请[输选填]|未[选设]|不能|不可|已关联/i.test(
      t
    )
  ) {
    return "warning";
  }

  // Success patterns
  if (
    /成功|通过|已保存|已采纳|已启用|已停用|已删除|已终止|已重启|已创建|已复制|已恢复|已开启|已切换|已提交|已更新|已就绪|已确认|\bsuccess\b/i.test(
      t
    )
  ) {
    return "success";
  }

  if (/\b\w+Error\b|\b\w+Exception\b/i.test(t)) {
    return "error";
  }

  return "info";
}


/** 记住上次查看的会话 id：页面重载（如开发热更新）后回到原位，而不是跳到列表第一个 */
const LAST_SESSION_KEY = "harness_mini.lastSessionId";
const SESSION_DRAFTS_KEY = "harness_mini.sessionDrafts";
const SUBAGENT_WIDTH_KEY = "harness_mini.subagentPanelWidth";
export const MAIN_PANEL_MIN_WIDTH = 460;
export const SUBAGENT_MIN_PANEL_WIDTH = 380;
export const SUBAGENT_DEFAULT_PANEL_WIDTH = 480;

function loadSessionDrafts(): Record<string, string> {
  try {
    const raw = localStorage.getItem(SESSION_DRAFTS_KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw);
    return parsed && typeof parsed === "object" ? parsed : {};
  } catch {
    return {};
  }
}

let saveDraftsTimer: any = null;
function persistSessionDrafts(drafts: Record<string, string>) {
  if (saveDraftsTimer) clearTimeout(saveDraftsTimer);
  saveDraftsTimer = setTimeout(() => {
    try {
      const cleaned: Record<string, string> = {};
      for (const [k, v] of Object.entries(drafts)) {
        if (v && v.trim().length > 0) {
          cleaned[k] = v;
        }
      }
      localStorage.setItem(SESSION_DRAFTS_KEY, JSON.stringify(cleaned));
    } catch {}
  }, 300);
}

function persistSessionDraftsImmediate(drafts: Record<string, string>) {
  if (saveDraftsTimer) clearTimeout(saveDraftsTimer);
  try {
    const cleaned: Record<string, string> = {};
    for (const [k, v] of Object.entries(drafts)) {
      if (v && v.trim().length > 0) {
        cleaned[k] = v;
      }
    }
    localStorage.setItem(SESSION_DRAFTS_KEY, JSON.stringify(cleaned));
  } catch {}
}

interface Store {
  ready: boolean;
  settings: Settings;
  projects: Project[];
  sessions: Session[];
  currentId: string | null; // 会话 id 或 DRAFT_ID
  /** 各会话的未发送输入草稿，按会话 id 或 DRAFT_ID 隔离 */
  sessionDrafts: Record<string, string>;
  setSessionDraft: (id: string, text: string) => void;
  draft: {
    projectId: string | null;
    workspacePath: string | null;
    temp?: TempAlloc | null;
    /** 访问模式（会话级）：新建草稿时继承“上一条对话”，首次发送时随会话落库 */
    accessMode: "confirm" | "full_access";
    /** 会话专属上下文上限（草稿阶段设置，落库时随会话持久化） */
    contextTokenLimit?: number | null;
    /** 草稿专属生图模型配置 */
    imageProviderId?: string | null;
    imageModelId?: string | null;
    /** 草稿专属视觉感知模型配置 */
    visionProviderId?: string | null;
    visionModelId?: string | null;
  } | null;
  setDraftContextTokenLimit: (limit: number | null) => void;
  setDraftCapabilityModels: (input: {
    imageProviderId?: string | null;
    imageModelId?: string | null;
    visionProviderId?: string | null;
    visionModelId?: string | null;
  }) => void;
  /** 临时空间运行时状态（临时目录是否存在 / 是否有变更 / 合并状态），按会话 id 缓存 */
  tempInfo: Record<string, TempInfo>;
  currentProjectId: string | null; // 当前进入的项目（项目视图下发送空对话时落库到该项目）
  messages: Record<string, Message[]>;
  hasMore: Record<string, boolean>;
  queues: Record<string, QueuedItem[]>;
  runStatus: Record<string, "idle" | "running">;
  lastRunOutcome: Record<string, string>;
  approvals: Record<string, ApprovalReq>;
  pendingCompactions: Record<string, CompactionReq>;
  sessionCompactions: Record<string, SessionCompaction[]>;
  /** 上下文硬截断提醒列表（不阻塞对话，用户可手动逐条关闭或批量关闭） */
  truncationNotices: TruncationNotice[];
  toolOutputs: Record<string, string>;
  toasts: Toast[];
  readOnly: boolean;
  showSettings: boolean;
  showArchive: boolean;
  /** Token 消耗统计看板弹窗 */
  showTokenStatsModal: boolean;
  /** 成长档案看板弹窗 */
  showGrowthModal: boolean;
  growthModalProjectId: string | null;
  /** 经验反思提炼状态（按会话 id 记录） */
  growthStatus: Record<string, { status: "idle" | "analyzing" | "proposed"; message: string }>;
  /** 各会话产生的成长提案卡片，按会话 id 缓存 */
  activeProposals: Record<string, GrowthItem[]>;
  /** 所有的成长经验列表 */
  growths: GrowthItem[];
  growthsLoading: boolean;
  /** 临时空间变更列表弹窗（仅临时空间对话可用） */
  showChanges: boolean;
  /** 临时空间清空进行中：对话区显示「删除中」遮罩，期间屏蔽交互 */
  tempClearing: boolean;
  /** 项目设置弹窗当前打开的项目 id；null = 关闭 */
  projectSettingsId: string | null;
  /** 会话设置弹窗当前打开的会话 id；null = 关闭 */
  sessionSettingsId: string | null;
  /** 各对话的审批规则（会话级：仅对所属对话生效），按会话 id 缓存 */
  sessionRules: Record<string, ApprovalRule[]>;
  /** 会话任务清单，按会话 id 缓存 */
  sessionTodos: Record<string, TodoItem[]>;
  /** 程序数据目录状态；pending=true 时启动拦截对话框等待用户选择 */
  dataStatus: DataStatus | null;
  /** 项目协作者列表，按父会话 ID 缓存 */
  collaborators: Record<string, Session[]>;
  /** 当前选中的常驻协作者 ID */
  activeCollaboratorId: string | null;
  /** 是否打开「创建协作者」弹窗 */
  showCreateCollaboratorModal: boolean;
  /** 临时子进程列表，按父会话 ID 缓存 */
  subprocesses: Record<string, Session[]>;
  /** 当前在分屏中打开查看的临时子进程 ID（null = 未打开） */
  activeSubprocessId: string | null;
  /** 子 Agent 进程列表，按父会话 ID 缓存（保持向下兼容） */
  subagents: Record<string, Session[]>;
  /** 当前在分屏中打开查看的子 Agent ID（null = 未打开） */
  activeSubagentId: string | null;
  /** 协作者/子 Agent 对话窗体宽度（px），支持拖动并持久化记忆 */
  subagentPanelWidth: number;
  /** 是否打开「创建子Agent」弹窗 */
  showCreateSubagentModal: boolean;
  view: "list" | "project";
  lightboxImage: { src: string; alt?: string; title?: string } | null;
  setLightboxImage: (img: { src: string; alt?: string; title?: string } | null) => void;

  bootstrap: () => Promise<void>;
  /** 拉取全局运行中的会话，恢复各会话的“运行中”状态显示（启动/界面刷新后调用） */
  refreshRunStatus: () => Promise<void>;
  /** 同步会话的实时快照（流式文字、思考流、活跃工具事件、待审批），支持刷新重连无缝恢复 */
  syncSessionActiveState: (sessionId: string) => Promise<void>;
  /** 请求停止会话当前运行：乐观置为空闲，后端会补发 run:status 事件兜底 */
  stopRun: (sessionId: string) => void;
  /** 重试当前会话最后一步 */
  retryTurn: (sessionId: string) => Promise<void>;
  /** 继续推进当前会话任务 */
  continueTurn: (sessionId: string) => Promise<void>;
  /** 收到流式回滚重置事件 */
  onMessageReset: (p: { sessionId: string; messageId: string }) => void;
  /** 收到网络重试通知事件 */
  onRunRetry: (p: { sessionId: string; messageId: string; attempt: number; maxRetries: number; reason: string }) => void;
  /** 手动关闭指定正在运行的控制台进程 */
  killCommand: (eventId: string) => Promise<void>;
  refreshSessions: () => Promise<void>;
  refreshProjects: () => Promise<void>;
  toggleProjectPinned: (id: string) => Promise<void>;
  setView: (v: "list" | "project") => void;
  enterProject: (projectId: string) => void;
  newDraft: (projectId?: string | null, inherit?: boolean) => void;
  /** 临时空间对话：向项目要一个临时空间计划（仅地址不建目录），以草稿形式打开 */
  newTempDraft: (projectId: string) => Promise<void>;
  setDraftWorkspace: (p: string | null, projectId?: string | null) => void;
  /** 顶栏切换未保存对话的访问模式（仅改草稿，首次发送时落库） */
  setDraftAccessMode: (mode: "confirm" | "full_access") => void;
  selectSession: (id: string, readOnly?: boolean) => Promise<void>;
  forkSession: (
    sessionId: string,
    messageId: string,
    customTitle?: string,
    includeTarget?: boolean,
  ) => Promise<Session | null>;
  forkAndEditUserMessage: (
    sessionId: string,
    message: Message,
  ) => Promise<Session | null>;
  loadEarlier: (id: string) => Promise<void>;
  reloadMessages: (id: string) => Promise<void>;
  setCurrent: (id: string | null) => void;
  setShowSettings: (v: boolean) => void;
  setShowArchive: (v: boolean) => void;
  setShowTokenStatsModal: (v: boolean) => void;
  setShowGrowthModal: (v: boolean, projectId?: string | null) => void;
  loadGrowths: (projectId?: string | null, status?: string | null) => Promise<void>;
  acceptGrowth: (id: string) => Promise<void>;
  rejectGrowth: (id: string) => Promise<void>;
  toggleGrowth: (id: string, enabled: boolean) => Promise<void>;
  updateGrowthRule: (id: string, title: string, ruleContent: string, category: string) => Promise<void>;
  deleteGrowth: (id: string) => Promise<void>;
  triggerManualGrowth: (sessionId: string, userInstruction?: string) => Promise<void>;
  setShowChanges: (v: boolean) => void;
  setTempClearing: (v: boolean) => void;
  setProjectSettings: (id: string | null) => void;
  /** 打开/关闭会话设置弹窗（打开时顺带拉取该对话的审批规则） */
  setSessionSettings: (id: string | null) => void;
  refreshSessionRules: (id: string) => Promise<void>;
  onSessionRules: (p: { sessionId: string; rules: ApprovalRule[] }) => void;
  setSettingsLocal: (s: Settings) => void;
  onSettingsChanged: (s: Settings) => void;
  pushToast: (text: string, type?: ToastType) => void;
  dismissToast: (id: string) => void;

  // 事件处理
  onMessageDelta: (p: any) => void;
  onMessageReasoningDelta: (p: any) => void;
  onMessageFinal: (m: Message) => void;
  onToolUpdate: (p: any) => void;
  onToolOutput: (p: any) => void;
  onApprovalRequest: (r: ApprovalReq) => void;
  approvalDone: (eventId: string) => void;
  onCompactionRequest: (r: CompactionReq) => void;
  compactionDone: (eventId: string) => void;
  onCompactionTimeout: (eventId: string) => void;
  onCompactionApplied: (c: SessionCompaction) => void;
  onSessionCompacted: (sessionId?: string) => Promise<void>;
  refreshSessionCompactions: (sessionId: string) => Promise<void>;
  onTruncationNotice: (notice: TruncationNotice) => void;
  dismissTruncationNotice: (id: string) => void;
  clearSessionTruncationNotices: (sessionId: string) => void;
  onRunStatus: (p: any) => void;
  onQueueUpdate: (p: any) => void;
  onSessionUpdate: (s: Session) => void;
  onSessionsChanged: (p?: { deleted?: string; archived?: string; unarchived?: string; created?: string }) => Promise<void>;
  onTempUpdate: (p: { sessionId: string; info: TempInfo }) => void;
  onSessionTodos: (p: { sessionId: string; todos: TodoItem[] }) => void;
  onProjectsChanged: () => Promise<void>;
  onGrowthProposed: (item: GrowthItem) => void;
  onGrowthStatus: (p: { sessionId: string; status: "idle" | "analyzing" | "proposed"; message?: string; growthId?: string }) => void;
  onGrowthUpdated: (item: GrowthItem) => void;
  onGrowthDeleted: (id: string) => void;
  sopStatus: Record<string, { status: "checking" | "passed" | "failed" | "error"; command: string; output?: string }>;
  onSopStatus: (p: { sessionId: string; status: "checking" | "passed" | "failed" | "error"; command: string; output?: string }) => void;
  toolRetryStatus: Record<string, ToolRetryStatus>;
  dismissToolRetry: (sessionId: string) => void;
  onToolRetryGuidance: (p: ToolRetryGuidanceEvent) => void;
  loadSubagents: (parentSessionId: string) => Promise<void>;
  loadCollaborators: (parentSessionId: string) => Promise<void>;
  loadSubprocesses: (parentSessionId: string) => Promise<void>;
  setActiveSubprocessId: (id: string | null) => void;
  setActiveCollaboratorId: (id: string | null) => void;
  setShowCreateCollaboratorModal: (open: boolean) => void;
  showEditCollaboratorModal: boolean;
  setShowEditCollaboratorModal: (open: boolean) => void;
  editingCollaboratorId: string | null;
  setEditingCollaboratorId: (id: string | null) => void;
  /** 当前正在编辑重发的消息目标 */
  editingMessage: EditingMessageTarget | null;
  setEditingMessage: (target: EditingMessageTarget | null) => void;
  /** 能力模型分配矩阵弹窗 */
  showModelMatrixModal: boolean;
  modelMatrixSessionId: string | null;
  openModelMatrixModal: (sessionId?: string | null) => void;
  closeModelMatrixModal: () => void;
  setSessionModels: (input: SessionModelsUpdateInput) => Promise<boolean>;
  createCollaborator: (input: CollaboratorCreateInput) => Promise<Session | null>;
  updateCollaborator: (input: CollaboratorUpdateInput) => Promise<Session | null>;
  deleteCollaborator: (collaboratorId: string) => Promise<void>;
  setCollaboratorAutoReport: (collaboratorId: string, autoReport: boolean) => Promise<void>;
  reportCollaboratorIncrement: (collaboratorId: string) => Promise<void>;
  onCollaboratorsChanged: (p: any) => Promise<void>;
  onCollaboratorUpdate: (p: any) => void;
  onCollaboratorCreated: (collab: Session) => void;
  onCollaboratorReported: (p: any) => void;
  setActiveSubagentId: (id: string | null) => void;
  setSubagentPanelWidth: (width: number) => void;
  setShowCreateSubagentModal: (open: boolean) => void;
  createSubagent: (input: { parentSessionId: string; role: string; taskPrompt: string; title?: string; subpath?: string | null; workspacePath?: string | null }) => Promise<Session | null>;
  stopSubagent: (subagentId: string) => Promise<void>;
  restartSubagent: (subagentId: string) => Promise<void>;
  restartAllSubagents: (parentSessionId: string) => Promise<void>;
  deleteSubagent: (subagentId: string) => Promise<void>;
  reportSubagentToParent: (subagentId: string) => Promise<void>;
  onSubagentsChanged: (p: any) => Promise<void>;
  onSubagentCreated: (subagent: Session) => void;
  onSubagentUpdate: (payload: any) => void;
  onSubprocessUpdate: (payload: any) => void;

  /** 长任务（按会话 id 索引） */
  activeTasks: Record<string, LongTask | null>;
  /** 任务检查点（按 taskId 索引） */
  taskCheckpoints: Record<string, TaskCheckpoint[]>;
  /** 是否打开长任务详情弹窗 */
  showTaskDetailModal: boolean;
  setShowTaskDetailModal: (v: boolean) => void;

  startLongTask: (sessionId: string, goal: string, maxBudgetTokens?: number | null) => Promise<LongTask | null>;
  pauseLongTask: (taskId: string) => Promise<void>;
  resumeLongTask: (taskId: string) => Promise<LongTask | null>;
  cancelLongTask: (taskId: string) => Promise<void>;
  fetchActiveTask: (sessionId: string) => Promise<LongTask | null>;
  fetchTaskCheckpoints: (taskId: string) => Promise<TaskCheckpoint[]>;
  rollbackToCheckpoint: (checkpointId: string) => Promise<LongTask | null>;
  updateTaskSubtasks: (taskId: string, subtasks: TaskSubItem[]) => Promise<LongTask | null>;
  onTaskUpdate: (task: LongTask) => void;
  onTaskCheckpoint: (checkpoint: TaskCheckpoint) => void;

  onError: (p: any) => void;
}

function upsertMessage(list: Message[], m: Message): Message[] {
  const idx = list.findIndex((x) => x.id === m.id);
  if (idx >= 0) {
    const next = list.slice();
    next[idx] = { ...next[idx], ...m, toolEvents: m.toolEvents.length ? m.toolEvents : next[idx].toolEvents };
    return next;
  }
  return [...list, m];
}

function upsertToolEvent(list: Message[], ev: ToolEvent, sessionId: string): Message[] {
  let idx = list.findIndex((m) => m.id === ev.messageId);
  let msg: Message;
  if (idx < 0) {
    // 模型首次回复即工具调用（无思考/正文增量）时，tool:update 先于消息到达，
    // 以 messageId 创建占位 assistant 消息承载工具卡片
    msg = {
      id: ev.messageId,
      sessionId,
      seq: Number.MAX_SAFE_INTEGER,
      role: "assistant",
      content: "",
      reasoning: null,
      queued: false,
      createdAt: ev.createdAt ?? new Date().toISOString(),
      toolEvents: [ev],
    };
    return [...list, msg];
  }
  const next = list.slice();
  msg = { ...next[idx] };
  const evIdx = msg.toolEvents.findIndex((e) => e.id === ev.id);
  const events = msg.toolEvents.slice();
  if (evIdx >= 0) events[evIdx] = { ...events[evIdx], ...ev };
  else events.push(ev);
  msg.toolEvents = events;
  next[idx] = msg;
  return next;
}

function mergeToolEvents(localEvs: ToolEvent[] = [], fetchedEvs: ToolEvent[] = []): ToolEvent[] {
  if (!localEvs || localEvs.length === 0) return fetchedEvs || [];
  if (!fetchedEvs || fetchedEvs.length === 0) return localEvs;

  const localMap = new Map(localEvs.map((e) => [e.id, e]));
  const merged: ToolEvent[] = [];

  for (const f of fetchedEvs) {
    const l = localMap.get(f.id);
    if (!l) {
      merged.push(f);
    } else {
      localMap.delete(f.id);
      const isLocalPending = l.status === "running" || l.status === "pending_approval";
      const isFetchedTerminal = f.status !== "running" && f.status !== "pending_approval";
      if (isLocalPending && isFetchedTerminal) {
        merged.push({ ...l, ...f });
      } else {
        merged.push({ ...f, ...l });
      }
    }
  }

  for (const l of localMap.values()) {
    merged.push(l);
  }

  return merged;
}

/**
 * 用库中消息合并本地消息后作为会话消息列表：
 * - 流式占位消息的内容领先于库里定期落库的内容（且可能尚未落库），合并时取较新一侧，避免刷新瞬间文字闪断
 * - 本地独有的消息（正在流式、尚未落库）保持在末尾
 */
function mergeSessionMessages(local: Message[] | undefined, fetched: Message[]): Message[] {
  if (!local || local.length === 0) return fetched;
  const byId = new Map(local.map((m) => [m.id, m]));
  const out = fetched.map((m) => {
    const l = byId.get(m.id);
    if (!l) return m;
    return {
      ...m,
      content: (l.content?.length ?? 0) >= (m.content?.length ?? 0) ? l.content : m.content,
      reasoning: (l.reasoning?.length ?? 0) >= (m.reasoning?.length ?? 0) ? l.reasoning : m.reasoning,
      toolEvents: mergeToolEvents(l.toolEvents, m.toolEvents),
      toolCalls: (m.toolCalls?.length ?? 0) > 0 ? m.toolCalls : l.toolCalls,
    };
  });
  const fetchedIds = new Set(fetched.map((m) => m.id));
  for (const l of local) {
    if (!fetchedIds.has(l.id)) out.push(l);
  }
  return out;
}

export const useStore = create<Store>((set, get) => ({
  ready: false,
  settings: {
    providers: [],
    activeProviderId: null,
    activeModelId: null,
    activeModel: null,
    globalAccessMode: "confirm",
    maxSteps: 30,
    commandTimeoutSecs: 120,
    contextTokenLimit: 64000,
    lastWorkspacePath: null,
    disabledTools: [],
  },
  projects: [],
  sessions: [],
  currentId: null,
  sessionDrafts: loadSessionDrafts(),
  draft: null,
  tempInfo: {},
  currentProjectId: null,
  messages: {},
  hasMore: {},
  queues: {},
  runStatus: {},
  lastRunOutcome: {},
  approvals: {},
  pendingCompactions: {},
  sessionCompactions: {},
  truncationNotices: [],
  toolOutputs: {},
  toasts: [],
  readOnly: false,
  showSettings: false,
  showArchive: false,
  showTokenStatsModal: false,
  showGrowthModal: false,
  growthModalProjectId: null,
  growthStatus: {},
  activeProposals: {},
  growths: [],
  growthsLoading: false,
  sopStatus: {},
  toolRetryStatus: {},
  showChanges: false,
  tempClearing: false,
  projectSettingsId: null,
  sessionSettingsId: null,
  sessionRules: {},
  sessionTodos: {},
  dataStatus: null,
  collaborators: {},
  activeCollaboratorId: null,
  showCreateCollaboratorModal: false,
  showEditCollaboratorModal: false,
  editingCollaboratorId: null,
  editingMessage: null,
  showModelMatrixModal: false,
  modelMatrixSessionId: null,
  subprocesses: {},
  activeSubprocessId: null,
  subagents: {},
  activeSubagentId: null,
  subagentPanelWidth: Math.max(SUBAGENT_MIN_PANEL_WIDTH, Number(localStorage.getItem(SUBAGENT_WIDTH_KEY)) || SUBAGENT_DEFAULT_PANEL_WIDTH),
  showCreateSubagentModal: false,
  // 启动默认进入项目视图
  view: "project",
  lightboxImage: null,
  setLightboxImage: (img) => set({ lightboxImage: img }),

  activeTasks: {},
  taskCheckpoints: {},
  showTaskDetailModal: false,
  setShowTaskDetailModal: (v) => set({ showTaskDetailModal: v }),


  async bootstrap() {
    // 数据目录状态优先获取：pending 时启动拦截对话框要在其余数据加载前就绪
    try {
      set({ dataStatus: await ipc.getDataStatus() });
    } catch {
      // 状态获取失败不阻塞启动
    }
    try {
      const [rawSettings, sessions, projects] = await Promise.all([
        ipc.getSettings(),
        ipc.listSessions(),
        ipc.listProjects(),
      ]);
      const activeModelId = rawSettings.activeModelId ?? rawSettings.activeModel ?? null;
      const settings = { ...rawSettings, activeModelId, activeModel: activeModelId };
      set({ settings, sessions, projects, ready: true });
      // 恢复各会话的运行状态（运行状态不持久化在前端，以数据库为准）
      void get().refreshRunStatus();
      // 恢复上次查看的会话；已被删除/归档则回落到列表第一个
      const lastId = localStorage.getItem(LAST_SESSION_KEY);
      const last = lastId ? sessions.find((s) => s.id === lastId) : null;
      if (last) {
        await get().selectSession(last.id);
      } else if (sessions.length > 0) {
        await get().selectSession(sessions[0].id);
      } else {
        // 无任何会话时也保持右侧为一个可发送的空对话（发送后自动落库）
        get().newDraft(null);
      }
    } catch (e) {
      set({ ready: true });
      get().pushToast(String(e));
    }
  },

  async refreshRunStatus() {
    try {
      const runs = await ipc.listRunningSessions();
      const running: Record<string, "running"> = {};
      for (const r of runs) running[r.sessionId] = "running";
      set((st) => ({ runStatus: { ...st.runStatus, ...running } }));
      const cur = get().currentId;
      if (cur && (running[cur] || get().runStatus[cur] === "running")) {
        void get().syncSessionActiveState(cur);
      }
    } catch (e) {
      get().pushToast(String(e));
    }
  },

  async syncSessionActiveState(sessionId: string) {
    try {
      const activeState = await ipc.getSessionActiveState(sessionId);
      if (!activeState) return;

      set((st) => {
        const nextRunStatus = { ...st.runStatus };
        if (activeState.isRunning) {
          nextRunStatus[sessionId] = "running";
        }

        let nextApprovals = st.approvals;
        if (activeState.pendingApproval) {
          nextApprovals = {
            ...st.approvals,
            [activeState.pendingApproval.eventId]: activeState.pendingApproval,
          };
        }

        let nextCompactions = st.pendingCompactions;
        if (activeState.pendingCompaction) {
          nextCompactions = {
            ...st.pendingCompactions,
            [activeState.pendingCompaction.eventId]: activeState.pendingCompaction,
          };
        }

        const list = st.messages[sessionId] ? [...st.messages[sessionId]] : [];
        if (
          activeState.streamingContent ||
          activeState.streamingReasoning ||
          (activeState.activeToolEvents && activeState.activeToolEvents.length > 0)
        ) {
          const lastMsg = list.length > 0 ? list[list.length - 1] : null;
          if (
            lastMsg &&
            lastMsg.role === "assistant" &&
            (!lastMsg.runId || !activeState.activeRunId || lastMsg.runId === activeState.activeRunId)
          ) {
            const updated = { ...lastMsg };
            if (
              activeState.streamingContent &&
              (!updated.content || activeState.streamingContent.length >= updated.content.length)
            ) {
              updated.content = activeState.streamingContent;
            }
            if (
              activeState.streamingReasoning &&
              (!updated.reasoning || activeState.streamingReasoning.length >= updated.reasoning.length)
            ) {
              updated.reasoning = activeState.streamingReasoning;
            }
            if (activeState.activeToolEvents && activeState.activeToolEvents.length > 0) {
              const mergedEvents = [...updated.toolEvents];
              for (const ev of activeState.activeToolEvents) {
                const idx = mergedEvents.findIndex((e) => e.id === ev.id);
                if (idx >= 0) {
                  mergedEvents[idx] = ev;
                } else {
                  mergedEvents.push(ev);
                }
              }
              updated.toolEvents = mergedEvents;
            }
            list[list.length - 1] = updated;
          } else if (activeState.isRunning) {
            list.push({
              id: activeState.currentMessageId ?? (activeState.activeRunId ? `active-${activeState.activeRunId}` : `active-${Date.now()}`),
              sessionId,
              runId: activeState.activeRunId ?? undefined,
              seq: Number.MAX_SAFE_INTEGER,
              role: "assistant",
              content: activeState.streamingContent || "",
              reasoning: activeState.streamingReasoning || null,
              queued: false,
              createdAt: new Date().toISOString(),
              toolEvents: activeState.activeToolEvents || [],
            });
          }
        }

        return {
          runStatus: nextRunStatus,
          approvals: nextApprovals,
          pendingCompactions: nextCompactions,
          messages: { ...st.messages, [sessionId]: list },
        };
      });
    } catch {
      // 优雅降级，静默忽略快照获取失败
    }
  },

  stopRun(sessionId: string) {
    // 乐观置为空闲（按钮立即恢复“发送”）；后端收到后补发 run:status(idle) 兜底，
    // 队列未空时后端会自动继续消费并重新进入运行态；同时收敛本地 running toolEvents 避免幽灵卡片残留
    set((st) => {
      const list = st.messages[sessionId];
      let nextMessages = st.messages;
      let nextOutputs = st.toolOutputs;
      if (list) {
        let changed = false;
        const updatedList = list.map((m) => {
          if (!m.toolEvents?.some((e) => e.status === "running" || e.status === "pending_approval")) return m;
          changed = true;
          return {
            ...m,
            toolEvents: m.toolEvents.map((e) =>
              e.status === "running" || e.status === "pending_approval"
                ? { ...e, status: "failed" as const, resultText: e.resultText ?? "[任务已终止]" }
                : e
            ),
          };
        });
        if (changed) {
          nextMessages = { ...st.messages, [sessionId]: updatedList };
          const outs = { ...st.toolOutputs };
          for (const m of list) {
            m.toolEvents?.forEach((e) => {
              if (e.status === "running") delete outs[e.id];
            });
          }
          nextOutputs = outs;
        }
      }
      const nextRetry = { ...st.toolRetryStatus };
      if (nextRetry[sessionId] && nextRetry[sessionId].status === "retrying") {
        nextRetry[sessionId] = {
          ...nextRetry[sessionId],
          status: "cancelled",
          message: "会话已停止，未完成的工具自纠已取消",
        };
      }
      return {
        runStatus: { ...st.runStatus, [sessionId]: "idle" },
        lastRunOutcome: { ...st.lastRunOutcome, [sessionId]: "cancelled" },
        toolRetryStatus: nextRetry,
        messages: nextMessages,
        toolOutputs: nextOutputs,
      };
    });
    void ipc.stopRun(sessionId).catch((e) => get().pushToast(String(e)));
  },

  async retryTurn(sessionId) {
    try {
      set((st) => {
        const curMsgs = st.messages[sessionId] ?? [];
        const lastUserIdx = curMsgs.map((m) => m.role).lastIndexOf("user");
        const nextMsgs = lastUserIdx >= 0 ? curMsgs.slice(0, lastUserIdx + 1) : curMsgs;
        return {
          runStatus: { ...st.runStatus, [sessionId]: "running" },
          lastRunOutcome: { ...st.lastRunOutcome, [sessionId]: "running" },
          messages: { ...st.messages, [sessionId]: nextMsgs },
        };
      });
      await ipc.retryTurn(sessionId);
      get().pushToast("正在重新执行本轮...", "info");
    } catch (e: any) {
      get().pushToast(`重试失败: ${e}`, "error");
    }
  },

  async continueTurn(sessionId) {
    try {
      set((st) => ({
        runStatus: { ...st.runStatus, [sessionId]: "running" },
        lastRunOutcome: { ...st.lastRunOutcome, [sessionId]: "running" },
      }));
      await ipc.continueTurn(sessionId);
      get().pushToast("正在继续推进任务...", "info");
    } catch (e: any) {
      get().pushToast(`继续失败: ${e}`, "error");
    }
  },

  async killCommand(eventId: string) {
    try {
      await ipc.killCommand(eventId);
      get().pushToast("已终止控制台进程");
      // 乐观更新：将当前消息列表中的该 toolEvent 置为 failed 并清理输出，
      // 确保界面列表中该进程立刻消失，避免残留可再次点击的困惑；
      // 后续后端 tool:update 到达时会做最终状态与输出文本的对齐。
      set((st) => {
        let changed = false;
        const nextMessages = { ...st.messages };
        for (const [sid, list] of Object.entries(nextMessages)) {
          const mIdx = list.findIndex((m) => m.toolEvents?.some((e) => e.id === eventId && e.status === "running"));
          if (mIdx >= 0) {
            const m = list[mIdx];
            const nextEvents = m.toolEvents.map((e) =>
              e.id === eventId && e.status === "running"
                ? { ...e, status: "failed" as const, resultText: e.resultText ?? "[控制台进程已终止]" }
                : e
            );
            const nextList = [...list];
            nextList[mIdx] = { ...m, toolEvents: nextEvents };
            nextMessages[sid] = nextList;
            changed = true;
          }
        }
        if (!changed) return st;
        const nextOutputs = { ...st.toolOutputs };
        delete nextOutputs[eventId];
        return { messages: nextMessages, toolOutputs: nextOutputs };
      });
    } catch (e) {
      get().pushToast(String(e));
    }
  },

  async refreshSessions() {
    try {
      set({ sessions: await ipc.listSessions() });
    } catch (e) {
      get().pushToast(String(e));
    }
  },

  async refreshProjects() {
    try {
      set({ projects: await ipc.listProjects() });
    } catch (e) {
      get().pushToast(String(e));
    }
  },

  async toggleProjectPinned(id: string) {
    const target = get().projects.find((p) => p.id === id);
    if (!target) return;
    const nextPinned = !target.pinned;
    // 乐观更新：本地立即切换，无需等后端 IPC 往返和刷新页面，界面瞬间呈现置顶效果
    set((st) => ({
      projects: st.projects.map((p) => (p.id === id ? { ...p, pinned: nextPinned } : p)),
    }));
    try {
      await ipc.setProjectPinned(id, nextPinned);
    } catch (e) {
      // 失败回滚
      set((st) => ({
        projects: st.projects.map((p) => (p.id === id ? { ...p, pinned: target.pinned } : p)),
      }));
      get().pushToast(`置顶状态切换失败: ${e}`);
    }
  },

  setView(v) {
    set({ view: v, ...(v === "list" ? { currentProjectId: null } : {}) });
  },

  // 进入项目：右侧切换为该项目下的空对话，发送首条消息后自动落库到该项目
  enterProject(projectId) {
    set({ currentProjectId: projectId });
    get().newDraft(projectId);
  },

  newDraft(projectId, inherit = true) {
    // 工作区即项目：从项目新建空对话时，草稿直接绑定该项目的目录；
    // 其余新对话（inherit=true）默认继承“上一条对话”（= 当前选中的对话）的模式与工作区；
    // inherit=false 用于“新建纯对话”：工作区始终为空。
    // 没有可继承来源（当前无选中对话）时：纯对话 + 设置页的新对话默认模式。
    const st = get();
    const prev = inherit ? currentSession(st) : null;
    const explicit = projectId ? st.projects.find((p) => p.id === projectId) ?? null : null;
    let workspacePath: string | null;
    let boundProjectId: string | null;
    if (explicit) {
      workspacePath = explicit.path ?? null;
      boundProjectId = explicit.id;
    } else if (inherit) {
      // 临时空间对话的工作区是临时副本，不作为继承来源
      workspacePath = prev && !prev.isTemp ? prev.workspacePath || null : null;
      boundProjectId = workspacePath
        ? st.projects.find((p) => samePath(p.path, workspacePath))?.id ?? null
        : null;
    } else {
      workspacePath = null;
      boundProjectId = null;
    }
    localStorage.removeItem(LAST_SESSION_KEY); // 草稿不可恢复，清除记住的会话
    set({
      draft: {
        projectId: boundProjectId,
        workspacePath,
        temp: null,
        // 模式继承上一条对话；无可继承来源时用设置页的新对话默认值
        accessMode: (inherit ? prev?.accessMode : undefined) ?? st.settings.globalAccessMode,
        contextTokenLimit: inherit ? prev?.contextTokenLimit ?? null : null,
        imageProviderId: inherit ? prev?.imageProviderId ?? null : null,
        imageModelId: inherit ? prev?.imageModelId ?? null : null,
        visionProviderId: inherit ? prev?.visionProviderId ?? null : null,
        visionModelId: inherit ? prev?.visionModelId ?? null : null,
      },
      currentId: DRAFT_ID,
      readOnly: false,
      editingMessage: null,
      messages: { ...get().messages, [DRAFT_ID]: [] },
      activeCollaboratorId: null,
      activeSubprocessId: null,
      activeSubagentId: null,
    });
  },

  async newTempDraft(projectId) {
    try {
      // 后端生成唯一随机码与计划路径，不创建目录；git 缺失时在此报错提示
      const temp = await ipc.allocTempCode(projectId);
      const st = get();
      const prev = currentSession(st);
      set({
        draft: {
          projectId,
          workspacePath: temp.mainTemp,
          temp,
          // 临时空间对话同样继承上一条对话的访问模式
          accessMode: prev?.accessMode ?? st.settings.globalAccessMode,
          contextTokenLimit: prev?.contextTokenLimit ?? null,
          imageProviderId: prev?.imageProviderId ?? null,
          imageModelId: prev?.imageModelId ?? null,
          visionProviderId: prev?.visionProviderId ?? null,
          visionModelId: prev?.visionModelId ?? null,
        },
        currentId: DRAFT_ID,
        readOnly: false,
        editingMessage: null,
        messages: { ...get().messages, [DRAFT_ID]: [] },
        activeCollaboratorId: null,
        activeSubprocessId: null,
        activeSubagentId: null,
      });
    } catch (e) {
      get().pushToast(String(e));
    }
  },

  setDraftWorkspace(p, projectId) {
    const d = get().draft;
    if (!d) return;
    // projectId 未传时保持不变；显式传 null 表示脱离项目
    set({ draft: { ...d, workspacePath: p, projectId: projectId === undefined ? d.projectId : projectId } });
  },

  setDraftAccessMode(mode) {
    const d = get().draft;
    if (!d) return;
    set({ draft: { ...d, accessMode: mode } });
  },

  setDraftContextTokenLimit(limit) {
    const d = get().draft;
    if (!d) return;
    set({ draft: { ...d, contextTokenLimit: limit } });
  },

  setDraftCapabilityModels(input) {
    const d = get().draft;
    if (!d) return;
    set({
      draft: {
        ...d,
        imageProviderId: input.imageProviderId !== undefined ? input.imageProviderId : d.imageProviderId,
        imageModelId: input.imageModelId !== undefined ? input.imageModelId : d.imageModelId,
        visionProviderId: input.visionProviderId !== undefined ? input.visionProviderId : d.visionProviderId,
        visionModelId: input.visionModelId !== undefined ? input.visionModelId : d.visionModelId,
      },
    });
  },

  async selectSession(id, readOnly = false) {
    // 先取消息、后原子切换：避免「currentId 已切换、消息未到达」期间消息区整块空白的闪现
    try {
      const msgs = await ipc.getMessages(id, undefined, 200);
      set((st) => ({
        currentId: id,
        readOnly,
        editingMessage: null,
        messages: { ...st.messages, [id]: mergeSessionMessages(st.messages[id], msgs) },
        hasMore: { ...st.hasMore, [id]: msgs.length >= 200 },
      }));
      localStorage.setItem(LAST_SESSION_KEY, id);
      const q = await ipc.listQueued(id);
      set((st) => ({ queues: { ...st.queues, [id]: q.filter((m) => m.queued).map((m) => ({ id: m.id, content: m.content ?? "", createdAt: m.createdAt })) } }));
      // 临时空间会话：拉取按钮禁用态所需的运行时状态（临时目录是否存在 / 有无变更）
      if (get().sessions.find((s) => s.id === id)?.isTemp) {
        const info = await ipc.getTempInfo(id);
        set((st) => ({ tempInfo: { ...st.tempInfo, [id]: info } }));
      }
      // 拉取会话任务清单快照
      ipc.getSessionTodos(id).then((val) => {
        if (val && Array.isArray(val.todos)) {
          set((st) => ({ sessionTodos: { ...st.sessionTodos, [id]: val.todos } }));
        }
      }).catch(() => {});
      // 拉取会话历史压缩记录
      void get().refreshSessionCompactions(id);
      // 拉取该会话待审阅的成长提案
      ipc.listGrowths(undefined, "proposed").then((list) => {
        const forThis = list.filter((g) => g.sessionId === id);
        if (forThis.length > 0) {
          set((st) => ({ activeProposals: { ...st.activeProposals, [id]: forThis } }));
        }
      }).catch(() => {});
      // 即时拉取并恢复运行态快照（流式文字、思考流、活跃工具卡片、审批卡片）
      void get().syncSessionActiveState(id);
      // 拉取该会话的项目协作者与子进程列表
      void get().loadCollaborators(id);
      void get().loadSubprocesses(id);
      void get().loadSubagents(id);
      void get().fetchActiveTask(id);
      // 切换主会话时，如果分屏中的协作者/子 Agent 不属于当前会话，则关闭分屏
      const curCollabId = get().activeCollaboratorId;
      if (curCollabId) {
        const collabs = get().collaborators[id] ?? [];
        if (!collabs.some((s) => s.id === curCollabId)) {
          set({ activeCollaboratorId: null });
        }
      }
      const curSubprocId = get().activeSubprocessId;
      if (curSubprocId) {
        const subprocs = get().subprocesses[id] ?? [];
        if (!subprocs.some((s) => s.id === curSubprocId)) {
          set({ activeSubprocessId: null });
        }
      }
      const curSubId = get().activeSubagentId;
      if (curSubId) {
        const subs = get().subagents[id] ?? [];
        if (!subs.some((s) => s.id === curSubId)) {
          set({ activeSubagentId: null });
        }
      }
    } catch (e) {
      get().pushToast(String(e));
    }
  },

  async forkSession(sessionId, messageId, customTitle, includeTarget = true) {
    try {
      const newSession = await ipc.forkSessionAtMessage(sessionId, messageId, customTitle, includeTarget);
      await get().refreshSessions();
      await get().selectSession(newSession.id);
      get().pushToast(`🌿 已创建分支「${newSession.title}」`, "success");
      return newSession;
    } catch (e) {
      get().pushToast(`创建分支失败: ${e}`, "error");
      return null;
    }
  },

  async forkAndEditUserMessage(sessionId, message) {
    try {
      const newSession = await ipc.forkSessionAtMessage(sessionId, message.id, undefined, false);
      await get().refreshSessions();
      await get().selectSession(newSession.id);
      get().setSessionDraft(newSession.id, message.content ?? "");
      get().pushToast(`🌿 已创建分支，提问内容已预填入输入框`, "success");
      return newSession;
    } catch (e) {
      get().pushToast(`创建分支失败: ${e}`, "error");
      return null;
    }
  },

  async loadEarlier(id) {
    const msgs = get().messages[id] ?? [];
    if (msgs.length === 0) return;
    const oldest = msgs[0].seq;
    try {
      const earlier = await ipc.getMessages(id, oldest, 200);
      set((st) => ({
        messages: { ...st.messages, [id]: [...earlier, ...msgs] },
        hasMore: { ...st.hasMore, [id]: earlier.length >= 200 },
      }));
    } catch (e) {
      get().pushToast(String(e));
    }
  },

  setCurrent(id) {
    set({ currentId: id, readOnly: false });
  },

  async reloadMessages(id) {
    try {
      const msgs = await ipc.getMessages(id, undefined, 200);
      set((st) => ({ messages: { ...st.messages, [id]: msgs } }));
    } catch (e) {
      get().pushToast(String(e));
    }
  },
  setShowSettings(v) {
    set({ showSettings: v });
  },
  setShowArchive(v) {
    set({ showArchive: v });
  },
  setShowTokenStatsModal(v) {
    set({ showTokenStatsModal: v });
  },
  setShowGrowthModal(v, projectId) {
    set({ showGrowthModal: v, growthModalProjectId: projectId ?? null });
    if (v) void get().loadGrowths(projectId ?? undefined);
  },
  async loadGrowths(projectId, status) {
    set({ growthsLoading: true });
    try {
      const list = await ipc.listGrowths(projectId, status);
      set({ growths: list });
    } catch (e) {
      get().pushToast(String(e));
    } finally {
      set({ growthsLoading: false });
    }
  },
  async acceptGrowth(id) {
    try {
      await ipc.updateGrowthStatus(id, "accepted");
      get().pushToast("已采纳并固化为项目经验");
    } catch (e) {
      get().pushToast(String(e));
    }
  },
  async rejectGrowth(id) {
    try {
      await ipc.updateGrowthStatus(id, "rejected");
      get().pushToast("已忽略该经验");
    } catch (e) {
      get().pushToast(String(e));
    }
  },
  async toggleGrowth(id, enabled) {
    try {
      await ipc.updateGrowthStatus(id, enabled ? "accepted" : "disabled");
      get().pushToast(enabled ? "已启用该经验" : "已停用该经验");
    } catch (e) {
      get().pushToast(String(e));
    }
  },
  async updateGrowthRule(id, title, ruleContent, category) {
    try {
      await ipc.updateGrowthRule(id, title, ruleContent, category);
      get().pushToast("规则修改已保存");
    } catch (e) {
      get().pushToast(String(e));
    }
  },
  async deleteGrowth(id) {
    try {
      await ipc.deleteGrowth(id);
      get().pushToast("经验已删除");
    } catch (e) {
      get().pushToast(String(e));
    }
  },
  async triggerManualGrowth(sessionId, userInstruction) {
    try {
      await ipc.triggerGrowthReflection(sessionId, userInstruction);
    } catch (e) {
      get().pushToast(String(e));
    }
  },
  setShowChanges(v) {
    set({ showChanges: v });
  },
  setTempClearing(v) {
    set({ tempClearing: v });
  },
  setSessionDraft(id, text) {
    const next = { ...get().sessionDrafts, [id]: text };
    set({ sessionDrafts: next });
    persistSessionDrafts(next);
  },
  setProjectSettings(id) {
    set({ projectSettingsId: id });
  },
  setSessionSettings(id) {
    set({ sessionSettingsId: id });
    if (id) void get().refreshSessionRules(id);
  },
  async refreshSessionRules(id) {
    try {
      const rules = await ipc.listSessionRules(id);
      set((st) => ({ sessionRules: { ...st.sessionRules, [id]: rules } }));
    } catch (e) {
      get().pushToast(String(e));
    }
  },
  onSessionRules(p) {
    set((st) => ({ sessionRules: { ...st.sessionRules, [p.sessionId]: p.rules ?? [] } }));
  },
  setSettingsLocal(s) {
    const activeModelId = s.activeModelId ?? s.activeModel ?? null;
    set({ settings: { ...s, activeModelId, activeModel: activeModelId } });
  },

  /** 后端设置变更时同步刷新缓存 */
  onSettingsChanged(s) {
    const activeModelId = s.activeModelId ?? s.activeModel ?? null;
    set({ settings: { ...s, activeModelId, activeModel: activeModelId } });
  },

  pushToast(text, type) {
    const id = Math.random().toString(36).slice(2);
    const resolvedType = type ?? inferToastType(text);
    set((st) => ({ toasts: [...st.toasts, { id, text, type: resolvedType }] }));
    setTimeout(() => get().dismissToast(id), 6000);
  },
  dismissToast(id) {
    set((st) => ({ toasts: st.toasts.filter((t) => t.id !== id) }));
  },

  // ---------- 事件 ----------

  onMessageDelta(p) {
    const { sessionId, messageId, delta } = p;
    set((st) => {
      const list = st.messages[sessionId] ?? [];
      const idx = list.findIndex((m) => m.id === messageId);
      if (idx < 0) {
        // 流式占位消息
        const placeholder: Message = {
          id: messageId,
          sessionId,
          seq: Number.MAX_SAFE_INTEGER,
          role: "assistant",
          content: delta,
          reasoning: null,
          queued: false,
          createdAt: new Date().toISOString(),
          toolEvents: [],
        };
        return { messages: { ...st.messages, [sessionId]: [...list, placeholder] } };
      }
      const next = list.slice();
      next[idx] = { ...next[idx], content: (next[idx].content ?? "") + delta };
      return { messages: { ...st.messages, [sessionId]: next } };
    });
  },

  onMessageReasoningDelta(p) {
    const { sessionId, messageId, delta } = p;
    set((st) => {
      const list = st.messages[sessionId] ?? [];
      const idx = list.findIndex((m) => m.id === messageId);
      if (idx < 0) {
        // 流式占位消息（思考通常先于正文到达）
        const placeholder: Message = {
          id: messageId,
          sessionId,
          seq: Number.MAX_SAFE_INTEGER,
          role: "assistant",
          content: "",
          reasoning: delta,
          queued: false,
          createdAt: new Date().toISOString(),
          toolEvents: [],
        };
        return { messages: { ...st.messages, [sessionId]: [...list, placeholder] } };
      }
      const next = list.slice();
      next[idx] = { ...next[idx], reasoning: (next[idx].reasoning ?? "") + delta };
      return { messages: { ...st.messages, [sessionId]: next } };
    });
  },

  onMessageReset(p) {
    set((st) => {
      const list = st.messages[p.sessionId] ?? [];
      const idx = list.findIndex((m) => m.id === p.messageId);
      if (idx < 0) return st;
      const next = list.slice();
      next[idx] = { ...next[idx], content: "", reasoning: "" };
      return { messages: { ...st.messages, [p.sessionId]: next } };
    });
  },

  onRunRetry(p) {
    get().pushToast(`网络波动，正在进行第 ${p.attempt}/${p.maxRetries} 次自动重试...`, "warning");
  },

  onMessageFinal(m) {
    if (m.queued) return; // 待执行列表消息由 queue:update 呈现
    set((st) => {
      const list = st.messages[m.sessionId] ?? [];
      const updatedList = upsertMessage(list, m);

      // 同步更新所属会话的累计 token
      let nextSessions = st.sessions;
      const sIdx = st.sessions.findIndex((x) => x.id === m.sessionId);
      if (sIdx >= 0) {
        let total = 0;
        let prompt = 0;
        let completion = 0;
        for (const msg of updatedList) {
          const tt = msg.totalTokens ?? (msg.usage?.totalTokens || (msg.usage?.inputEst || 0) + (msg.usage?.outputEst || 0)) ?? 0;
          const pt = msg.promptTokens ?? (msg.usage?.promptTokens || msg.usage?.inputEst) ?? 0;
          const ct = msg.completionTokens ?? (msg.usage?.completionTokens || msg.usage?.outputEst) ?? 0;
          total += Number(tt) || 0;
          prompt += Number(pt) || 0;
          completion += Number(ct) || 0;
        }
        nextSessions = st.sessions.slice();
        nextSessions[sIdx] = {
          ...st.sessions[sIdx],
          totalTokens: total,
          promptTokens: prompt,
          completionTokens: completion,
        };
      }

      // 同步更新所属子 Agent 会话的累计 token
      let nextSubagents = st.subagents;
      let foundParentId: string | null = null;
      for (const [pid, subList] of Object.entries(st.subagents)) {
        if (subList.some((s) => s.id === m.sessionId)) {
          foundParentId = pid;
          break;
        }
      }
      if (foundParentId) {
        let subTotal = 0;
        let subPrompt = 0;
        let subCompletion = 0;
        for (const msg of updatedList) {
          const tt = msg.totalTokens ?? (msg.usage?.totalTokens || (msg.usage?.inputEst || 0) + (msg.usage?.outputEst || 0)) ?? 0;
          const pt = msg.promptTokens ?? (msg.usage?.promptTokens || msg.usage?.inputEst) ?? 0;
          const ct = msg.completionTokens ?? (msg.usage?.completionTokens || msg.usage?.outputEst) ?? 0;
          subTotal += Number(tt) || 0;
          subPrompt += Number(pt) || 0;
          subCompletion += Number(ct) || 0;
        }
        nextSubagents = {
          ...st.subagents,
          [foundParentId]: (st.subagents[foundParentId] ?? []).map((s) =>
            s.id === m.sessionId
              ? {
                  ...s,
                  totalTokens: subTotal,
                  promptTokens: subPrompt,
                  completionTokens: subCompletion,
                }
              : s
          ),
        };
      }

      // 同步更新所属临时子进程列表的累计 token
      let nextSubprocesses = st.subprocesses;
      let foundSubprocParentId: string | null = null;
      for (const [pid, subList] of Object.entries(st.subprocesses)) {
        if (subList.some((s) => s.id === m.sessionId)) {
          foundSubprocParentId = pid;
          break;
        }
      }
      if (foundSubprocParentId) {
        let subTotal = 0;
        let subPrompt = 0;
        let subCompletion = 0;
        for (const msg of updatedList) {
          const tt = msg.totalTokens ?? (msg.usage?.totalTokens || (msg.usage?.inputEst || 0) + (msg.usage?.outputEst || 0)) ?? 0;
          const pt = msg.promptTokens ?? (msg.usage?.promptTokens || msg.usage?.inputEst) ?? 0;
          const ct = msg.completionTokens ?? (msg.usage?.completionTokens || msg.usage?.outputEst) ?? 0;
          subTotal += Number(tt) || 0;
          subPrompt += Number(pt) || 0;
          subCompletion += Number(ct) || 0;
        }
        nextSubprocesses = {
          ...st.subprocesses,
          [foundSubprocParentId]: (st.subprocesses[foundSubprocParentId] ?? []).map((s) =>
            s.id === m.sessionId
              ? {
                  ...s,
                  totalTokens: subTotal,
                  promptTokens: subPrompt,
                  completionTokens: subCompletion,
                }
              : s
          ),
        };
      }

      return {
        messages: { ...st.messages, [m.sessionId]: updatedList },
        sessions: nextSessions,
        subagents: nextSubagents,
        subprocesses: nextSubprocesses,
      };
    });
  },

  onToolUpdate(p) {
    const ev: ToolEvent = p.event;
    set((st) => {
      const list = st.messages[p.sessionId] ?? [];
      return { messages: { ...st.messages, [p.sessionId]: upsertToolEvent(list, ev, p.sessionId) } };
    });
    if (ev.status !== "running" && ev.status !== "pending_approval") {
      set((st) => {
        const outs = { ...st.toolOutputs };
        delete outs[ev.id];
        return { toolOutputs: outs };
      });
    }
    // todo 工具更新时同步任务清单
    if (ev.toolName === "todo" && ev.status === "success" && Array.isArray(ev.params?.todos)) {
      set((st) => ({
        sessionTodos: { ...st.sessionTodos, [p.sessionId]: ev.params.todos },
      }));
    }
  },

  onToolOutput(p) {
    set((st) => ({
      toolOutputs: { ...st.toolOutputs, [p.eventId]: (st.toolOutputs[p.eventId] ?? "") + p.line + "\n" },
    }));
  },

  onApprovalRequest(r) {
    set((st) => ({ approvals: { ...st.approvals, [r.eventId]: r } }));
  },

  approvalDone(eventId) {
    set((st) => {
      const a = { ...st.approvals };
      delete a[eventId];
      return { approvals: a };
    });
  },

  onCompactionRequest(r) {
    set((st) => ({
      pendingCompactions: {
        ...st.pendingCompactions,
        [r.eventId]: {
          ...r,
          timeoutSeconds: r.timeoutSeconds ?? 30,
          createdAt: r.createdAt ?? Date.now(),
          timedOut: false,
        },
      },
    }));
  },

  compactionDone(eventId) {
    set((st) => {
      const a = { ...st.pendingCompactions };
      delete a[eventId];
      return { pendingCompactions: a };
    });
  },

  onCompactionTimeout(eventId) {
    set((st) => {
      const existing = st.pendingCompactions[eventId];
      if (!existing) return {};
      return {
        pendingCompactions: {
          ...st.pendingCompactions,
          [eventId]: {
            ...existing,
            timedOut: true,
          },
        },
      };
    });
  },

  onCompactionApplied(c) {
    set((st) => {
      const list = st.sessionCompactions[c.sessionId] ?? [];
      const nextList = [...list.filter((x) => x.id !== c.id), c].sort((a, b) => a.startSeq - b.startSeq);
      return {
        sessionCompactions: { ...st.sessionCompactions, [c.sessionId]: nextList },
      };
    });
  },

  async onSessionCompacted(sessionId) {
    if (!sessionId) return;
    await get().refreshSessionCompactions(sessionId);
    await get().reloadMessages(sessionId);
  },

  async refreshSessionCompactions(sessionId) {
    try {
      const list = await ipc.listSessionCompactions(sessionId);
      set((st) => ({
        sessionCompactions: { ...st.sessionCompactions, [sessionId]: list },
      }));
    } catch {
      // 容错
    }
  },

  onTruncationNotice(notice) {
    set((st) => ({
      truncationNotices: [notice, ...st.truncationNotices.filter((n) => n.id !== notice.id)],
    }));
  },

  dismissTruncationNotice(id) {
    set((st) => ({
      truncationNotices: st.truncationNotices.filter((n) => n.id !== id),
    }));
  },

  clearSessionTruncationNotices(sessionId) {
    set((st) => ({
      truncationNotices: st.truncationNotices.filter((n) => n.sessionId !== sessionId),
    }));
  },

  onSessionTodos(p) {
    if (!p?.sessionId || !Array.isArray(p.todos)) return;
    set((st) => ({
      sessionTodos: { ...st.sessionTodos, [p.sessionId]: p.todos },
    }));
  },

  onRunStatus(p) {
    set((st) => {
      const nextRetry = { ...st.toolRetryStatus };
      if (p.status === "running") {
        // 新一轮会话启动时，自动清除上一轮残留的已中止或已完结的自纠提示
        if (nextRetry[p.sessionId] && nextRetry[p.sessionId].status !== "retrying") {
          delete nextRetry[p.sessionId];
        }
      } else {
        if (nextRetry[p.sessionId] && nextRetry[p.sessionId].status === "retrying") {
          nextRetry[p.sessionId] = {
            ...nextRetry[p.sessionId],
            status: "cancelled",
            message: "会话运行结束，自纠已中止",
          };
        }
      }
      let nextTodos = st.sessionTodos;
      // 运行完成时增加双重兜底：若任务清单仍有 in_progress，根据是否正常完成分别置为 done 或 pending
      if (st.sessionTodos[p.sessionId]) {
        const cur = st.sessionTodos[p.sessionId];
        if (p.status === "done") {
          if (cur.some((t) => t.status === "in_progress")) {
            nextTodos = {
              ...st.sessionTodos,
              [p.sessionId]: cur.map((t) => (t.status === "in_progress" ? { ...t, status: "done" } : t)),
            };
          }
        } else if (p.status !== "running") {
          if (cur.some((t) => t.status === "in_progress")) {
            nextTodos = {
              ...st.sessionTodos,
              [p.sessionId]: cur.map((t) => (t.status === "in_progress" ? { ...t, status: "pending" } : t)),
            };
          }
        }
      }

      // 若当前会话关联长任务且运行被中止/失败，立即将长任务及进行中的子步骤置为 interrupted/failed，严禁变为 completed
      let nextActiveTasks = st.activeTasks;
      if (p.status !== "running" && p.status !== "done" && st.activeTasks[p.sessionId]) {
        const curTask = st.activeTasks[p.sessionId]!;
        if (curTask.status === "running" || curTask.status === "planning") {
          const mappedTaskStatus = p.status === "failed" ? "failed" : "interrupted";
          const updatedSubtasks = (curTask.subtasks || []).map((sub) =>
            sub.status === "running" || sub.status === "verifying"
              ? { ...sub, status: mappedTaskStatus }
              : sub
          );
          nextActiveTasks = {
            ...st.activeTasks,
            [p.sessionId]: {
              ...curTask,
              status: mappedTaskStatus,
              subtasks: updatedSubtasks,
            },
          };
        }
      }

      // 同步检查是否为某个父会话的子任务/子进程运行结束，立即同步内存状态
      let nextSubprocesses = st.subprocesses;
      let nextSubagents = st.subagents;
      if (p.status !== "running") {
        const mappedStatus =
          p.status === "done" ? "completed" : p.status === "failed" ? "failed" : "cancelled";
        for (const [pid, list] of Object.entries(st.subprocesses)) {
          if (list.some((s) => s.id === p.sessionId)) {
            nextSubprocesses = {
              ...nextSubprocesses,
              [pid]: list.map((s) => (s.id === p.sessionId ? { ...s, status: mappedStatus } : s)),
            };
            break;
          }
        }
        for (const [pid, list] of Object.entries(st.subagents)) {
          if (list.some((s) => s.id === p.sessionId)) {
            nextSubagents = {
              ...nextSubagents,
              [pid]: list.map((s) => (s.id === p.sessionId ? { ...s, status: mappedStatus } : s)),
            };
            break;
          }
        }

        // 若被中止的是父会话自身，将其下所有执行中/待推进的子任务在内存中同步置为 cancelled
        if (p.status === "cancelled" || p.status === "interrupted") {
          if (nextSubprocesses[p.sessionId]) {
            nextSubprocesses = {
              ...nextSubprocesses,
              [p.sessionId]: nextSubprocesses[p.sessionId].map((s) =>
                s.status === "running" || s.status === "pending" || s.status === "in_progress"
                  ? { ...s, status: "cancelled" }
                  : s
              ),
            };
          }
          if (nextSubagents[p.sessionId]) {
            nextSubagents = {
              ...nextSubagents,
              [p.sessionId]: nextSubagents[p.sessionId].map((s) =>
                s.status === "running" || s.status === "pending" || s.status === "in_progress"
                  ? { ...s, status: "cancelled" }
                  : s
              ),
            };
          }
        }
      }

      return {
        runStatus: { ...st.runStatus, [p.sessionId]: p.status === "running" ? "running" : "idle" },
        lastRunOutcome: { ...st.lastRunOutcome, [p.sessionId]: p.status },
        toolRetryStatus: nextRetry,
        sessionTodos: nextTodos,
        activeTasks: nextActiveTasks,
        subprocesses: nextSubprocesses,
        subagents: nextSubagents,
      };
    });

    if (p.status !== "running") {
      for (const [pid, list] of Object.entries(get().subprocesses)) {
        if (list.some((s) => s.id === p.sessionId)) {
          void get().loadSubprocesses(pid);
          void get().loadSubagents(pid);
          break;
        }
      }
    }
  },

  onQueueUpdate(p) {
    set((st) => ({ queues: { ...st.queues, [p.sessionId]: p.items ?? [] } }));
  },

  onSessionUpdate(s) {
    if (s.status === "archived") {
      set((st) => ({
        sessions: st.sessions.filter((x) => x.id !== s.id),
        subagents: Object.fromEntries(
          Object.entries(st.subagents).map(([pid, list]) => [
            pid,
            list.filter((x) => x.id !== s.id),
          ])
        ),
        subprocesses: Object.fromEntries(
          Object.entries(st.subprocesses).map(([pid, list]) => [
            pid,
            list.filter((x) => x.id !== s.id),
          ])
        ),
      }));
      return;
    }
    set((st) => {
      // 若为子 Agent 会话，同步更新至 subagents 和 subprocesses 映射中，绝不可混入主会话列表 sessions
      let parentId = s.parentSessionId;
      if (!parentId) {
        for (const [pid, list] of Object.entries(st.subagents)) {
          if (list.some((x) => x.id === s.id)) {
            parentId = pid;
            break;
          }
        }
        if (!parentId) {
          for (const [pid, list] of Object.entries(st.subprocesses)) {
            if (list.some((x) => x.id === s.id)) {
              parentId = pid;
              break;
            }
          }
        }
      }

      if (parentId || s.sessionType === "subagent" || s.sessionType === "subprocess") {
        if (!parentId) return st;
        const existingSubagents = st.subagents[parentId] ?? [];
        const nextSubagents = existingSubagents.some((x) => x.id === s.id)
          ? existingSubagents.map((x) => (x.id === s.id ? { ...x, ...s } : x))
          : [...existingSubagents, s];

        const existingSubprocs = st.subprocesses[parentId] ?? [];
        const nextSubprocs = existingSubprocs.some((x) => x.id === s.id)
          ? existingSubprocs.map((x) => (x.id === s.id ? { ...x, ...s } : x))
          : [...existingSubprocs, s];

        return {
          subagents: { ...st.subagents, [parentId]: nextSubagents },
          subprocesses: { ...st.subprocesses, [parentId]: nextSubprocs },
        };
      }

      const idx = st.sessions.findIndex((x) => x.id === s.id);
      if (idx >= 0) {
        const next = st.sessions.slice();
        next[idx] = s;
        return { sessions: next };
      }
      return { sessions: [s, ...st.sessions] };
    });
  },

  async onSessionsChanged(p) {
    await get().refreshSessions();
    const deleted = p?.deleted;
    if (!deleted) return;
    // 被删的会话可能正展示在右侧（如从归档中心删除只读查看中的会话），
    // 后端各删除入口都会广播 sessions:changed {"deleted": id}，在此统一收尾
    set((st) => {
      const messages = { ...st.messages };
      const queues = { ...st.queues };
      const runStatus = { ...st.runStatus };
      const tempInfo = { ...st.tempInfo };
      const sessionDrafts = { ...st.sessionDrafts };
      delete messages[deleted];
      delete queues[deleted];
      delete runStatus[deleted];
      delete tempInfo[deleted];
      delete sessionDrafts[deleted];
      persistSessionDraftsImmediate(sessionDrafts);
      const nextSubagents: Record<string, Session[]> = {};
      for (const [pid, list] of Object.entries(st.subagents)) {
        nextSubagents[pid] = list.filter((s) => s.id !== deleted);
      }
      const nextCollaborators: Record<string, Session[]> = {};
      for (const [pid, list] of Object.entries(st.collaborators)) {
        nextCollaborators[pid] = list.filter((s) => s.id !== deleted);
      }
      const nextSubprocesses: Record<string, Session[]> = {};
      for (const [pid, list] of Object.entries(st.subprocesses)) {
        nextSubprocesses[pid] = list.filter((s) => s.id !== deleted);
      }

      return {
        messages,
        queues,
        runStatus,
        tempInfo,
        sessionDrafts,
        subagents: nextSubagents,
        collaborators: nextCollaborators,
        subprocesses: nextSubprocesses,
        activeSubagentId: st.activeSubagentId === deleted ? null : st.activeSubagentId,
        activeCollaboratorId: st.activeCollaboratorId === deleted ? null : st.activeCollaboratorId,
        activeSubprocessId: st.activeSubprocessId === deleted ? null : st.activeSubprocessId,
        editingMessage: st.editingMessage?.sessionId === deleted ? null : st.editingMessage,
        ...(st.currentId === deleted ? { currentId: null, readOnly: false } : {}),
      };
    });
    // 当前会话被删后落回一个可发送的空对话，而不是不可输入的空白态
    if (useStore.getState().currentId === null) {
      get().newDraft(get().currentProjectId);
    }
  },

  async onProjectsChanged() {
    await get().refreshProjects();
  },

  onTempUpdate(p) {
    set((st) => ({ tempInfo: { ...st.tempInfo, [p.sessionId]: p.info } }));
  },

  onGrowthProposed(item) {
    const sid = item.sessionId ?? "";
    set((st) => {
      const currentList = st.activeProposals[sid] ?? [];
      if (currentList.some((x) => x.id === item.id)) return st;
      return {
        activeProposals: {
          ...st.activeProposals,
          [sid]: [...currentList, item],
        },
        growths: [item, ...st.growths.filter((x) => x.id !== item.id)],
      };
    });
  },

  onGrowthStatus(p) {
    set((st) => ({
      growthStatus: {
        ...st.growthStatus,
        [p.sessionId]: { status: p.status, message: p.message ?? "" },
      },
    }));
  },

  onGrowthUpdated(item) {
    const sid = item.sessionId ?? "";
    set((st) => {
      const sidProposals = (st.activeProposals[sid] ?? []).map((x) => (x.id === item.id ? item : x));
      const nextGrowths = st.growths.map((x) => (x.id === item.id ? item : x));
      return {
        activeProposals: {
          ...st.activeProposals,
          [sid]: sidProposals,
        },
        growths: nextGrowths,
      };
    });
  },

  onGrowthDeleted(id) {
    set((st) => {
      const nextProposals: Record<string, GrowthItem[]> = {};
      for (const [k, v] of Object.entries(st.activeProposals)) {
        nextProposals[k] = v.filter((x) => x.id !== id);
      }
      return {
        activeProposals: nextProposals,
        growths: st.growths.filter((x) => x.id !== id),
      };
    });
  },

  onSopStatus(p) {
    set((st) => ({
      sopStatus: {
        ...st.sopStatus,
        [p.sessionId]: { status: p.status, command: p.command, output: p.output },
      },
    }));
    if (p.status === "passed") {
      get().pushToast(`🛡️ 交付前 SOP 自检通过 (${p.command})`);
    } else if (p.status === "failed") {
      get().pushToast(`🛡️ 交付前 SOP 自检未通过 (${p.command})，Agent 正在自愈修复...`);
    }
  },

  dismissToolRetry(sessionId) {
    set((st) => {
      const nextRetry = { ...st.toolRetryStatus };
      if (nextRetry[sessionId]) {
        nextRetry[sessionId] = { ...nextRetry[sessionId], dismissed: true };
      }
      return { toolRetryStatus: nextRetry };
    });
  },

  onToolRetryGuidance(p) {
    const status = p.status ?? "retrying";
    const attempt = p.attempt ?? 1;
    const maxRetries = p.maxRetries ?? 2;
    set((st) => ({
      toolRetryStatus: {
        ...st.toolRetryStatus,
        [p.sessionId]: {
          toolName: p.toolName,
          attempt,
          maxRetries,
          status,
          error: p.error,
          message: p.message,
          dismissed: false,
        },
      },
    }));
    if (status === "retrying") {
      get().pushToast(`🔄 工具 ${p.toolName} 执行受阻，Agent 正在自省排查并重试 (${attempt}/${maxRetries})...`);
    } else if (status === "success") {
      get().pushToast(`✅ 工具 ${p.toolName} 自纠成功，已恢复执行`);
    } else if (status === "failed") {
      get().pushToast(`❌ 工具 ${p.toolName} 自纠未果（已达重试上限）`);
    }
  },

  async loadCollaborators(parentSessionId: string) {
    try {
      const list = await ipc.listCollaborators(parentSessionId);
      set((st) => ({
        collaborators: { ...st.collaborators, [parentSessionId]: list },
      }));
      const activeId = get().activeCollaboratorId;
      if (activeId) {
        if (list.some((s) => s.id === activeId)) {
          if (!get().messages[activeId]) {
            const msgs = await ipc.getMessages(activeId, undefined, 200);
            set((st) => ({
              messages: { ...st.messages, [activeId]: msgs },
            }));
          }
        } else {
          set({ activeCollaboratorId: null });
        }
      }
    } catch (e) {
      console.error("loadCollaborators error", e);
    }
  },

  async loadSubprocesses(parentSessionId: string) {
    try {
      const list = await ipc.listSubprocesses(parentSessionId);
      set((st) => ({
        subprocesses: { ...st.subprocesses, [parentSessionId]: list },
      }));
    } catch (e) {
      console.error("loadSubprocesses error", e);
    }
  },

  setActiveSubprocessId(id: string | null) {
    set({ activeSubprocessId: id, activeCollaboratorId: null, activeSubagentId: id });
    if (id) {
      ipc.getMessages(id, undefined, 200).then((msgs) => {
        set((st) => ({
          messages: { ...st.messages, [id]: mergeSessionMessages(st.messages[id], msgs) },
          hasMore: { ...st.hasMore, [id]: msgs.length >= 200 },
        }));
      }).catch((e) => get().pushToast(String(e)));
      void get().syncSessionActiveState(id);
    }
  },

  setActiveCollaboratorId(id: string | null) {
    set({ activeCollaboratorId: id, activeSubagentId: id, activeSubprocessId: null });
    if (id) {
      ipc.getMessages(id, undefined, 200).then((msgs) => {
        set((st) => ({
          messages: { ...st.messages, [id]: mergeSessionMessages(st.messages[id], msgs) },
          hasMore: { ...st.hasMore, [id]: msgs.length >= 200 },
        }));
      }).catch((e) => get().pushToast(String(e)));
      void get().syncSessionActiveState(id);
    }
  },

  setShowCreateCollaboratorModal(open: boolean) {
    set({ showCreateCollaboratorModal: open });
  },

  setShowEditCollaboratorModal(open: boolean) {
    set({ showEditCollaboratorModal: open });
  },

  setEditingCollaboratorId(id: string | null) {
    set({ editingCollaboratorId: id, showEditCollaboratorModal: id !== null });
  },

  setEditingMessage(target: EditingMessageTarget | null) {
    set({ editingMessage: target });
  },

  openModelMatrixModal(sessionId) {
    set({ showModelMatrixModal: true, modelMatrixSessionId: sessionId ?? get().currentId ?? null });
  },

  closeModelMatrixModal() {
    set({ showModelMatrixModal: false, modelMatrixSessionId: null });
  },

  async setSessionModels(input) {
    try {
      const updated = await ipc.setSessionModels(input);
      get().onSessionUpdate(updated);
      set((st) => {
        const nextCollabs: Record<string, Session[]> = {};
        for (const [pid, list] of Object.entries(st.collaborators)) {
          nextCollabs[pid] = list.map((c) => (c.id === updated.id ? updated : c));
        }
        return { collaborators: nextCollabs };
      });
      get().pushToast(`模型能力配置已生效`);
      return true;
    } catch (e) {
      get().pushToast(`更新模型配置失败: ${e}`);
      return false;
    }
  },

  async updateCollaborator(input) {
    try {
      const updated = await ipc.updateCollaborator(input);
      set((st) => {
        const nextCollabs: Record<string, Session[]> = {};
        for (const [pid, list] of Object.entries(st.collaborators)) {
          nextCollabs[pid] = list.map((c) => (c.id === updated.id ? updated : c));
        }
        return {
          collaborators: nextCollabs,
          showEditCollaboratorModal: false,
          editingCollaboratorId: null,
        };
      });
      get().pushToast(`协作者「${updated.title}」配置已更新`);
      return updated;
    } catch (e) {
      get().pushToast(`更新协作者失败: ${e}`);
      return null;
    }
  },

  async createCollaborator(input) {
    try {
      let parentId = input.parentSessionId;
      if (!parentId || parentId === DRAFT_ID) {
        // 新对话尚未保存：先创建一个默认的主对话，设置为主进程和统筹协调者
        const st = get();
        const draft = st.draft;
        const newSession = await ipc.createSession({
          workspacePath: draft?.workspacePath || undefined,
          projectId: draft?.projectId || st.currentProjectId || undefined,
          title: "主进程与统筹协调者",
          accessMode: draft?.accessMode || undefined,
          contextTokenLimit: draft?.contextTokenLimit || undefined,
          temp: draft?.temp || undefined,
        });
        st.onSessionUpdate(newSession);
        await st.selectSession(newSession.id);
        parentId = newSession.id;
        input.parentSessionId = parentId;
      }
      const created = await ipc.createCollaborator(input);
      set((st) => {
        const existing = st.collaborators[parentId] ?? [];
        const nextList = [created, ...existing.filter((s) => s.id !== created.id)];
        return {
          collaborators: { ...st.collaborators, [parentId]: nextList },
          activeCollaboratorId: created.id,
          activeSubagentId: created.id,
          showCreateCollaboratorModal: false,
        };
      });
      get().pushToast(`协作者「${created.title}」已创建并进入就绪状态`);
      void get().syncSessionActiveState(created.id);
      return created;
    } catch (e) {
      get().pushToast(`创建协作者失败: ${e}`);
      return null;
    }
  },

  async deleteCollaborator(collaboratorId: string) {
    try {
      await ipc.deleteSubagent(collaboratorId);
      set((st) => {
        const nextCollaborators: Record<string, Session[]> = {};
        for (const [pid, list] of Object.entries(st.collaborators)) {
          nextCollaborators[pid] = list.filter((s) => s.id !== collaboratorId);
        }
        const nextSubagents: Record<string, Session[]> = {};
        for (const [pid, list] of Object.entries(st.subagents)) {
          nextSubagents[pid] = list.filter((s) => s.id !== collaboratorId);
        }
        const nextSubprocesses: Record<string, Session[]> = {};
        for (const [pid, list] of Object.entries(st.subprocesses)) {
          nextSubprocesses[pid] = list.filter((s) => s.id !== collaboratorId);
        }
        const messages = { ...st.messages };
        delete messages[collaboratorId];
        return {
          collaborators: nextCollaborators,
          subagents: nextSubagents,
          subprocesses: nextSubprocesses,
          messages,
          activeCollaboratorId: st.activeCollaboratorId === collaboratorId ? null : st.activeCollaboratorId,
          activeSubagentId: st.activeSubagentId === collaboratorId ? null : st.activeSubagentId,
          activeSubprocessId: st.activeSubprocessId === collaboratorId ? null : st.activeSubprocessId,
        };
      });
      get().pushToast("已移除协作者", "success");
    } catch (e) {
      get().pushToast(`移除协作者失败: ${e}`, "error");
    }
  },

  async setCollaboratorAutoReport(collaboratorId: string, autoReport: boolean) {
    try {
      await ipc.setCollaboratorAutoReport(collaboratorId, autoReport);
      set((st) => {
        const nextCollabs: Record<string, Session[]> = {};
        for (const [pid, list] of Object.entries(st.collaborators)) {
          nextCollabs[pid] = list.map((c) => (c.id === collaboratorId ? { ...c, autoReport } : c));
        }
        return { collaborators: nextCollabs };
      });
      get().pushToast(autoReport ? "已开启任务完成自动汇报" : "已切换为手动汇报模式");
    } catch (e) {
      get().pushToast(`修改汇报设置失败: ${e}`);
    }
  },

  async reportCollaboratorIncrement(collaboratorId: string) {
    try {
      const res = await ipc.reportCollaboratorIncrement(collaboratorId);
      get().pushToast(res || "已向主会话提交增量汇报");
    } catch (e) {
      get().pushToast(`增量汇报失败: ${e}`);
    }
  },

  async onCollaboratorsChanged(p) {
    const pid = p?.parentSessionId || p?.parentId;
    if (pid) {
      await get().loadCollaborators(pid);
    }
  },

  onCollaboratorUpdate(p) {
    if (p?.id) {
      const updated = p as Session;
      set((st) => {
        const nextCollabs: Record<string, Session[]> = {};
        for (const [pid, list] of Object.entries(st.collaborators)) {
          nextCollabs[pid] = list.map((c) => (c.id === updated.id ? { ...c, ...updated } : c));
        }
        return { collaborators: nextCollabs };
      });
      return;
    }
    const cid = p?.collaboratorId || p?.subagentId;
    const status = p?.status;
    if (cid && status) {
      set((st) => {
        const nextCollabs: Record<string, Session[]> = {};
        for (const [pid, list] of Object.entries(st.collaborators)) {
          nextCollabs[pid] = list.map((c) => (c.id === cid ? { ...c, status } : c));
        }
        return {
          collaborators: nextCollabs,
          runStatus: {
            ...st.runStatus,
            [cid]: status === "running" ? "running" : "idle",
          },
        };
      });
    }
  },

  onCollaboratorCreated(collab) {
    const pid = collab.parentSessionId;
    if (!pid) return;
    set((st) => {
      const existing = st.collaborators[pid] ?? [];
      if (existing.some((c) => c.id === collab.id)) return st;
      return {
        collaborators: { ...st.collaborators, [pid]: [collab, ...existing] },
      };
    });
  },

  onCollaboratorReported(p) {
    const cid = p?.collaboratorId;
    const lastId = p?.lastReportedMsgId;
    if (cid && lastId) {
      set((st) => {
        const nextCollabs: Record<string, Session[]> = {};
        for (const [pid, list] of Object.entries(st.collaborators)) {
          nextCollabs[pid] = list.map((c) => (c.id === cid ? { ...c, lastReportedMsgId: lastId } : c));
        }
        return { collaborators: nextCollabs };
      });
    }
  },

  async loadSubagents(parentSessionId: string) {
    try {
      const list = await ipc.listSubagents(parentSessionId);
      set((st) => ({
        subagents: { ...st.subagents, [parentSessionId]: list },
      }));
      const activeId = get().activeSubagentId;
      if (activeId && list.some((s) => s.id === activeId)) {
        if (!get().messages[activeId]) {
          const msgs = await ipc.getMessages(activeId, undefined, 200);
          set((st) => ({
            messages: { ...st.messages, [activeId]: msgs },
          }));
        }
      }
    } catch (e) {
      console.error("loadSubagents error", e);
    }
  },

  setActiveSubagentId(id: string | null) {
    set({ activeSubagentId: id });
    if (id) {
      ipc.getMessages(id, undefined, 200).then((msgs) => {
        set((st) => ({
          messages: { ...st.messages, [id]: mergeSessionMessages(st.messages[id], msgs) },
          hasMore: { ...st.hasMore, [id]: msgs.length >= 200 },
        }));
      }).catch((e) => get().pushToast(String(e)));
      void get().syncSessionActiveState(id);
    }
  },

  setSubagentPanelWidth(width: number) {
    const minAllowed = SUBAGENT_MIN_PANEL_WIDTH;
    const maxAllowed = Math.max(minAllowed, window.innerWidth - 240 - MAIN_PANEL_MIN_WIDTH);
    const clamped = Math.max(minAllowed, Math.min(width, maxAllowed));
    localStorage.setItem(SUBAGENT_WIDTH_KEY, String(clamped));
    set({ subagentPanelWidth: clamped });
  },

  setShowCreateSubagentModal(open: boolean) {
    set({ showCreateSubagentModal: open });
  },

  async createSubagent(input) {
    try {
      const created = await ipc.spawnSubagent(input);
      set((st) => {
        const existing = st.subagents[input.parentSessionId] ?? [];
        const nextList = [created, ...existing.filter((s) => s.id !== created.id)];
        return {
          subagents: { ...st.subagents, [input.parentSessionId]: nextList },
          activeSubagentId: created.id,
          showCreateSubagentModal: false,
        };
      });
      get().pushToast(`子 Agent「${created.title}」已创建并开始运行`);
      void get().syncSessionActiveState(created.id);
      return created;
    } catch (e) {
      get().pushToast(`创建子 Agent 失败: ${e}`);
      return null;
    }
  },

  async stopSubagent(subagentId: string) {
    try {
      set((st) => ({
        runStatus: { ...st.runStatus, [subagentId]: "idle" },
      }));
      await ipc.stopSubagent(subagentId);
      get().pushToast("已停止子 Agent");
    } catch (e) {
      get().pushToast(`停止子 Agent 失败: ${e}`);
    }
  },

  async restartSubagent(subagentId: string) {
    try {
      set((st) => ({
        runStatus: { ...st.runStatus, [subagentId]: "running" },
      }));
      await ipc.restartSubagent(subagentId);
      get().pushToast("子 Agent 已重启");
      void get().syncSessionActiveState(subagentId);
    } catch (e) {
      get().pushToast(`重启子 Agent 失败: ${e}`);
    }
  },

  async restartAllSubagents(parentSessionId: string) {
    try {
      await ipc.restartAllSubagents(parentSessionId);
      get().pushToast("已发起所有子任务重启");
      const subs = [
        ...(get().subagents[parentSessionId] ?? []),
        ...(get().subprocesses[parentSessionId] ?? []),
      ];
      const seen = new Set<string>();
      for (const s of subs) {
        if (!seen.has(s.id)) {
          seen.add(s.id);
          void get().syncSessionActiveState(s.id);
        }
      }
    } catch (e) {
      get().pushToast(`批量重启子任务失败: ${e}`);
    }
  },

  async deleteSubagent(subagentId: string) {
    try {
      await ipc.deleteSubagent(subagentId);
      set((st) => {
        const nextSubagents: Record<string, Session[]> = {};
        for (const [pid, list] of Object.entries(st.subagents)) {
          nextSubagents[pid] = list.filter((s) => s.id !== subagentId);
        }
        const nextCollaborators: Record<string, Session[]> = {};
        for (const [pid, list] of Object.entries(st.collaborators)) {
          nextCollaborators[pid] = list.filter((s) => s.id !== subagentId);
        }
        const nextSubprocesses: Record<string, Session[]> = {};
        for (const [pid, list] of Object.entries(st.subprocesses)) {
          nextSubprocesses[pid] = list.filter((s) => s.id !== subagentId);
        }
        const messages = { ...st.messages };
        delete messages[subagentId];
        return {
          subagents: nextSubagents,
          collaborators: nextCollaborators,
          subprocesses: nextSubprocesses,
          messages,
          activeSubagentId: st.activeSubagentId === subagentId ? null : st.activeSubagentId,
          activeCollaboratorId: st.activeCollaboratorId === subagentId ? null : st.activeCollaboratorId,
          activeSubprocessId: st.activeSubprocessId === subagentId ? null : st.activeSubprocessId,
        };
      });
      get().pushToast("已删除", "success");
    } catch (e) {
      get().pushToast(`删除失败: ${e}`, "error");
    }
  },

  async reportSubagentToParent(subagentId: string) {
    try {
      const res = await ipc.reportSubagentToParent(subagentId);
      get().pushToast(res || "已向主 Agent 提交成果汇报");
    } catch (e) {
      get().pushToast(`汇报失败: ${e}`);
    }
  },

  async onSubagentsChanged(p) {
    const pid = p?.parentSessionId || p?.parentId;
    if (pid) {
      await Promise.all([get().loadSubagents(pid), get().loadSubprocesses(pid)]);
    }
  },

  onSubagentCreated(subagent) {
    const pid = subagent.parentSessionId || (subagent as any).parentId;
    if (!pid) return;
    set((st) => {
      const existing = st.subagents[pid] ?? [];
      const nextSubs = existing.some((s) => s.id === subagent.id)
        ? existing.map((s) => (s.id === subagent.id ? { ...s, ...subagent } : s))
        : [subagent, ...existing];

      const existingProcs = st.subprocesses[pid] ?? [];
      const nextProcs = existingProcs.some((s) => s.id === subagent.id)
        ? existingProcs.map((s) => (s.id === subagent.id ? { ...s, ...subagent } : s))
        : [subagent, ...existingProcs];

      return {
        subagents: { ...st.subagents, [pid]: nextSubs },
        subprocesses: { ...st.subprocesses, [pid]: nextProcs },
      };
    });
  },

  onSubprocessUpdate(payload) {
    const pid = payload?.parentSessionId || payload?.parentId;
    const sid = payload?.subprocessId || payload?.subagentId || payload?.id;
    if (!sid) return;

    let targetPid = pid;
    if (!targetPid) {
      const st = get();
      for (const [p, list] of Object.entries(st.subprocesses)) {
        if (list.some((s) => s.id === sid)) {
          targetPid = p;
          break;
        }
      }
      if (!targetPid) {
        for (const [p, list] of Object.entries(st.subagents)) {
          if (list.some((s) => s.id === sid)) {
            targetPid = p;
            break;
          }
        }
      }
    }

    if (payload.status) {
      set((st) => ({
        runStatus: {
          ...st.runStatus,
          [sid]: payload.status === "running" ? "running" : "idle",
        },
        subprocesses: targetPid
          ? {
              ...st.subprocesses,
              [targetPid]: (st.subprocesses[targetPid] ?? []).map((s) =>
                s.id === sid ? { ...s, status: payload.status } : s
              ),
            }
          : st.subprocesses,
        subagents: targetPid
          ? {
              ...st.subagents,
              [targetPid]: (st.subagents[targetPid] ?? []).map((s) =>
                s.id === sid ? { ...s, status: payload.status } : s
              ),
            }
          : st.subagents,
      }));
    }
    if (targetPid) {
      void get().loadSubprocesses(targetPid);
      void get().loadSubagents(targetPid);
    }
  },

  onSubagentUpdate(payload) {
    get().onSubprocessUpdate(payload);
  },

  async startLongTask(sessionId, goal, maxBudgetTokens) {
    try {
      const task = await ipc.startLongTask(sessionId, goal, maxBudgetTokens);
      set((st) => ({
        activeTasks: { ...st.activeTasks, [sessionId]: task },
      }));
      get().pushToast(`长任务已启动: ${goal}`, "info");
      return task;
    } catch (e: any) {
      get().pushToast(`启动长任务失败: ${e}`, "error");
      return null;
    }
  },

  async pauseLongTask(taskId) {
    try {
      await ipc.pauseLongTask(taskId);
      get().pushToast("长任务已暂停", "warning");
    } catch (e: any) {
      get().pushToast(`暂停长任务失败: ${e}`, "error");
    }
  },

  async resumeLongTask(taskId) {
    try {
      const task = await ipc.resumeLongTask(taskId);
      set((st) => ({
        activeTasks: { ...st.activeTasks, [task.sessionId]: task },
      }));
      get().pushToast("长任务已恢复继续推进", "info");
      return task;
    } catch (e: any) {
      get().pushToast(`恢复长任务失败: ${e}`, "error");
      return null;
    }
  },

  async cancelLongTask(taskId) {
    try {
      await ipc.cancelLongTask(taskId);
      get().pushToast("长任务已终止", "info");
    } catch (e: any) {
      get().pushToast(`终止长任务失败: ${e}`, "error");
    }
  },

  async fetchActiveTask(sessionId) {
    try {
      const task = await ipc.getActiveTask(sessionId);
      set((st) => ({
        activeTasks: { ...st.activeTasks, [sessionId]: task },
      }));
      if (task) {
        void get().fetchTaskCheckpoints(task.id);
      }
      return task;
    } catch {
      return null;
    }
  },

  async fetchTaskCheckpoints(taskId) {
    try {
      const list = await ipc.listTaskCheckpoints(taskId);
      set((st) => ({
        taskCheckpoints: { ...st.taskCheckpoints, [taskId]: list },
      }));
      return list;
    } catch {
      return [];
    }
  },

  async rollbackToCheckpoint(checkpointId) {
    try {
      const task = await ipc.rollbackToCheckpoint(checkpointId);
      set((st) => ({
        activeTasks: { ...st.activeTasks, [task.sessionId]: task },
      }));
      // 重新获取该任务的检查点列表
      void get().fetchTaskCheckpoints(task.id);
      get().pushToast("已成功回退到指定历史检查点", "info");
      return task;
    } catch (e: any) {
      get().pushToast(`回退快照失败: ${e}`, "error");
      return null;
    }
  },

  async updateTaskSubtasks(taskId, subtasks) {
    try {
      const task = await ipc.updateTaskSubtasks(taskId, subtasks);
      set((st) => ({
        activeTasks: { ...st.activeTasks, [task.sessionId]: task },
      }));
      get().pushToast("子任务路线图已更新", "info");
      return task;
    } catch (e: any) {
      get().pushToast(`更新子任务失败: ${e}`, "error");
      return null;
    }
  },

  onTaskUpdate(task) {
    set((st) => ({
      activeTasks: { ...st.activeTasks, [task.sessionId]: task },
    }));
  },

  onTaskCheckpoint(checkpoint) {
    set((st) => {
      const prev = st.taskCheckpoints[checkpoint.taskId] ?? [];
      const exists = prev.some((c) => c.id === checkpoint.id);
      return {
        taskCheckpoints: {
          ...st.taskCheckpoints,
          [checkpoint.taskId]: exists
            ? prev.map((c) => (c.id === checkpoint.id ? checkpoint : c))
            : [...prev, checkpoint],
        },
      };
    });
  },

  onError(p) {
    get().pushToast(`[${p.kind}] ${p.message}`);
  },
}));

export function currentSession(s: Store): Session | null {
  if (!s.currentId || s.currentId === DRAFT_ID) return null;
  return s.sessions.find((x) => x.id === s.currentId) ?? null;
}

export function currentMessages(s: Store): Message[] {
  if (!s.currentId) return [];
  return s.messages[s.currentId] ?? [];
}

export function currentQueue(s: Store): QueuedItem[] {
  if (!s.currentId) return [];
  return s.queues[s.currentId] ?? [];
}
