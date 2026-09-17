import { create } from "zustand";
import { ipc } from "./ipc";
import { DRAFT_ID, samePath, type ApprovalReq, type ApprovalRule, type CompactionReq, type DataStatus, type GrowthItem, type Message, type Project, type QueuedItem, type Session, type SessionCompaction, type Settings, type TempAlloc, type TempInfo, type TodoItem, type ToolEvent, type ToolRetryGuidanceEvent, type ToolRetryStatus, type TruncationNotice } from "./types";

export interface Toast {
  id: string;
  text: string;
}

/** 记住上次查看的会话 id：页面重载（如开发热更新）后回到原位，而不是跳到列表第一个 */
const LAST_SESSION_KEY = "harness_mini.lastSessionId";

interface Store {
  ready: boolean;
  settings: Settings;
  projects: Project[];
  sessions: Session[];
  currentId: string | null; // 会话 id 或 DRAFT_ID
  draft: {
    projectId: string | null;
    workspacePath: string | null;
    temp?: TempAlloc | null;
    /** 访问模式（会话级）：新建草稿时继承“上一条对话”，首次发送时随会话落库 */
    accessMode: "confirm" | "full_access";
  } | null;
  /** 临时空间运行时状态（临时目录是否存在 / 是否有变更 / 合并状态），按会话 id 缓存 */
  tempInfo: Record<string, TempInfo>;
  currentProjectId: string | null; // 当前进入的项目（项目视图下发送空对话时落库到该项目）
  messages: Record<string, Message[]>;
  hasMore: Record<string, boolean>;
  queues: Record<string, QueuedItem[]>;
  runStatus: Record<string, "idle" | "running">;
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
  view: "list" | "project";

  bootstrap: () => Promise<void>;
  /** 拉取全局运行中的会话，恢复各会话的“运行中”状态显示（启动/界面刷新后调用） */
  refreshRunStatus: () => Promise<void>;
  /** 请求停止会话当前运行：乐观置为空闲，后端会补发 run:status 事件兜底 */
  stopRun: (sessionId: string) => void;
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
  pushToast: (text: string) => void;
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
  if (evIdx >= 0) events[evIdx] = ev;
  else events.push(ev);
  msg.toolEvents = events;
  next[idx] = msg;
  return next;
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
      toolEvents: l.toolEvents.length >= m.toolEvents.length ? l.toolEvents : m.toolEvents,
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
  draft: null,
  tempInfo: {},
  currentProjectId: null,
  messages: {},
  hasMore: {},
  queues: {},
  runStatus: {},
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
  // 启动默认进入项目视图
  view: "project",

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
    } catch (e) {
      get().pushToast(String(e));
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
      return {
        runStatus: { ...st.runStatus, [sessionId]: "idle" },
        messages: nextMessages,
        toolOutputs: nextOutputs,
      };
    });
    void ipc.stopRun(sessionId).catch((e) => get().pushToast(String(e)));
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
      },
      currentId: DRAFT_ID,
      readOnly: false,
      messages: { ...get().messages, [DRAFT_ID]: [] },
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
        },
        currentId: DRAFT_ID,
        readOnly: false,
        messages: { ...get().messages, [DRAFT_ID]: [] },
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

  async selectSession(id, readOnly = false) {
    // 先取消息、后原子切换：避免「currentId 已切换、消息未到达」期间消息区整块空白的闪现
    try {
      const msgs = await ipc.getMessages(id, undefined, 200);
      set((st) => ({
        currentId: id,
        readOnly,
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
    } catch (e) {
      get().pushToast(String(e));
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

  pushToast(text) {
    const id = Math.random().toString(36).slice(2);
    set((st) => ({ toasts: [...st.toasts, { id, text }] }));
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

      return {
        messages: { ...st.messages, [m.sessionId]: updatedList },
        sessions: nextSessions,
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
      if (p.status !== "running") {
        if (nextRetry[p.sessionId] && nextRetry[p.sessionId].status === "retrying") {
          nextRetry[p.sessionId] = {
            ...nextRetry[p.sessionId],
            status: "cancelled",
            message: "会话运行结束，自纠已中止",
          };
        }
      }
      let nextTodos = st.sessionTodos;
      // 运行完成时增加双重兜底：若任务清单仍有 in_progress，将其自动置为 done
      if (p.status === "done" && st.sessionTodos[p.sessionId]) {
        const cur = st.sessionTodos[p.sessionId];
        if (cur.some((t) => t.status === "in_progress")) {
          nextTodos = {
            ...st.sessionTodos,
            [p.sessionId]: cur.map((t) => (t.status === "in_progress" ? { ...t, status: "done" } : t)),
          };
        }
      }
      return {
        runStatus: { ...st.runStatus, [p.sessionId]: p.status === "running" ? "running" : "idle" },
        toolRetryStatus: nextRetry,
        sessionTodos: nextTodos,
      };
    });
  },

  onQueueUpdate(p) {
    set((st) => ({ queues: { ...st.queues, [p.sessionId]: p.items ?? [] } }));
  },

  onSessionUpdate(s) {
    if (s.status !== "active") {
      set((st) => ({ sessions: st.sessions.filter((x) => x.id !== s.id) }));
      return;
    }
    set((st) => {
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
      delete messages[deleted];
      delete queues[deleted];
      delete runStatus[deleted];
      delete tempInfo[deleted];
      return {
        messages,
        queues,
        runStatus,
        tempInfo,
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
