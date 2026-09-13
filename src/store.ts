import { create } from "zustand";
import { ipc } from "./ipc";
import { DRAFT_ID, type ApprovalReq, type DataStatus, type Message, type Project, type QueuedItem, type Session, type Settings, type TempAlloc, type TempInfo, type ToolEvent } from "./types";

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
  draft: { projectId: string | null; workspacePath: string | null; temp?: TempAlloc | null } | null;
  /** 临时空间运行时状态（临时目录是否存在 / 是否有变更 / 合并状态），按会话 id 缓存 */
  tempInfo: Record<string, TempInfo>;
  currentProjectId: string | null; // 当前进入的项目（项目视图下发送空对话时落库到该项目）
  messages: Record<string, Message[]>;
  hasMore: Record<string, boolean>;
  queues: Record<string, QueuedItem[]>;
  runStatus: Record<string, "idle" | "running">;
  approvals: Record<string, ApprovalReq>;
  toolOutputs: Record<string, string>;
  toasts: Toast[];
  readOnly: boolean;
  showSettings: boolean;
  showArchive: boolean;
  /** 临时空间变更列表弹窗（仅临时空间对话可用） */
  showChanges: boolean;
  /** 临时空间清空进行中：对话区显示「删除中」遮罩，期间屏蔽交互 */
  tempClearing: boolean;
  /** 项目设置弹窗当前打开的项目 id；null = 关闭 */
  projectSettingsId: string | null;
  /** 程序数据目录状态；pending=true 时启动拦截对话框等待用户选择 */
  dataStatus: DataStatus | null;
  view: "list" | "project";

  bootstrap: () => Promise<void>;
  refreshSessions: () => Promise<void>;
  refreshProjects: () => Promise<void>;
  setView: (v: "list" | "project") => void;
  enterProject: (projectId: string) => void;
  newDraft: (projectId?: string | null) => void;
  /** 临时空间对话：向项目要一个临时空间计划（仅地址不建目录），以草稿形式打开 */
  newTempDraft: (projectId: string) => Promise<void>;
  setDraftWorkspace: (p: string | null, projectId?: string | null) => void;
  selectSession: (id: string, readOnly?: boolean) => Promise<void>;
  loadEarlier: (id: string) => Promise<void>;
  reloadMessages: (id: string) => Promise<void>;
  setCurrent: (id: string | null) => void;
  setShowSettings: (v: boolean) => void;
  setShowArchive: (v: boolean) => void;
  setShowChanges: (v: boolean) => void;
  setTempClearing: (v: boolean) => void;
  setProjectSettings: (id: string | null) => void;
  setSettingsLocal: (s: Settings) => void;
  pushToast: (text: string) => void;
  dismissToast: (id: string) => void;

  // 事件处理
  onMessageDelta: (p: any) => void;
  onMessageFinal: (m: Message) => void;
  onToolUpdate: (p: any) => void;
  onToolOutput: (p: any) => void;
  onApprovalRequest: (r: ApprovalReq) => void;
  approvalDone: (eventId: string) => void;
  onRunStatus: (p: any) => void;
  onQueueUpdate: (p: any) => void;
  onSessionUpdate: (s: Session) => void;
  onSessionsChanged: (p?: { deleted?: string; archived?: string; unarchived?: string; created?: string }) => Promise<void>;
  onTempUpdate: (p: { sessionId: string; info: TempInfo }) => void;
  onProjectsChanged: () => Promise<void>;
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

function upsertToolEvent(list: Message[], ev: ToolEvent): Message[] {
  const idx = list.findIndex((m) => m.id === ev.id);
  if (idx < 0) return list;
  const next = list.slice();
  const msg = { ...next[idx] };
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
    globalAccessMode: "confirm",
    maxSteps: 30,
    commandTimeoutSecs: 120,
    contextTokenLimit: 28000,
    lastWorkspacePath: null,
    approvalRules: [],
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
  toolOutputs: {},
  toasts: [],
  readOnly: false,
  showSettings: false,
  showArchive: false,
  showChanges: false,
  tempClearing: false,
  projectSettingsId: null,
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
      const [settings, sessions, projects] = await Promise.all([
        ipc.getSettings(),
        ipc.listSessions(),
        ipc.listProjects(),
      ]);
      set({ settings, sessions, projects, ready: true });
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

  setView(v) {
    set({ view: v, ...(v === "list" ? { currentProjectId: null } : {}) });
  },

  // 进入项目：右侧切换为该项目下的空对话，发送首条消息后自动落库到该项目
  enterProject(projectId) {
    set({ currentProjectId: projectId });
    get().newDraft(projectId);
  },

  newDraft(projectId) {
    // 工作区即项目：从项目新建空对话时，草稿直接绑定该项目的目录；
    // 其余新对话默认为纯对话（未选择工作区），需要时在顶栏选择项目/目录
    const proj = projectId ? get().projects.find((p) => p.id === projectId) : null;
    localStorage.removeItem(LAST_SESSION_KEY); // 草稿不可恢复，清除记住的会话
    set({
      draft: {
        projectId: projectId ?? null,
        workspacePath: proj?.path ?? null,
        temp: null,
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
      set({
        draft: {
          projectId,
          workspacePath: temp.mainTemp,
          temp,
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
  setShowChanges(v) {
    set({ showChanges: v });
  },
  setTempClearing(v) {
    set({ tempClearing: v });
  },
  setProjectSettings(id) {
    set({ projectSettingsId: id });
  },
  setSettingsLocal(s) {
    set({ settings: s });
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

  onMessageFinal(m) {
    if (m.queued) return; // 待执行列表消息由 queue:update 呈现
    set((st) => {
      const list = st.messages[m.sessionId] ?? [];
      return { messages: { ...st.messages, [m.sessionId]: upsertMessage(list, m) } };
    });
  },

  onToolUpdate(p) {
    const ev: ToolEvent = p.event;
    set((st) => {
      const list = st.messages[p.sessionId] ?? [];
      return { messages: { ...st.messages, [p.sessionId]: upsertToolEvent(list, ev) } };
    });
    if (ev.status !== "running" && ev.status !== "pending_approval") {
      set((st) => {
        const outs = { ...st.toolOutputs };
        delete outs[ev.id];
        return { toolOutputs: outs };
      });
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

  onRunStatus(p) {
    set((st) => ({
      runStatus: { ...st.runStatus, [p.sessionId]: p.status === "running" ? "running" : "idle" },
    }));
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
