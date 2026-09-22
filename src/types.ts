export interface Provider {
  id: string;
  name: string; // 厂商名称
  baseUrl: string;
  models: string[]; // 该厂商下配置的模型列表
  apiKey: string;
}

export interface ApprovalRule {
  id: string;
  /** 所属对话：规则仅在该对话内参与判定，随对话删除一并清理 */
  sessionId: string;
  kind: string; // command_prefix | path_write | tool
  pattern: string;
  createdAt: string;
}

export interface TodoItem {
  content: string;
  status: "pending" | "in_progress" | "done" | string;
}

export interface PlanMeta {
  id: string;
  title: string;
  status: "drafting" | "in_progress" | "completed" | "suspended" | "archived" | string;
  created_at: string;
  updated_at: string;
  version: number;
  session_id: string;
}

export interface PlanStep {
  index: number;
  content: string;
  status: "pending" | "in_progress" | "done" | string;
}

export interface PlanSummary {
  id: string;
  title: string;
  status: string;
  version: number;
  filename: string;
  created_at: string;
  updated_at: string;
  session_id: string;
  total_steps: number;
  completed_steps: number;
  is_active: boolean;
}

export interface ActivePlanDetail {
  meta: PlanMeta;
  filename: string;
  body: string;
  steps: PlanStep[];
}

export interface ToolInfo {
  name: string;
  description: string;
  risk: "read" | "write" | "execute";
  isTemp: boolean;
}

export interface Settings {
  providers: Provider[];
  activeProviderId?: string | null;
  activeModelId?: string | null; // 全局激活的模型，属于 activeProviderId 指向的厂商
  activeModel?: string | null; // 兼容后端 activeModel 字段
  globalAccessMode: "confirm" | "full_access";
  maxSteps: number;
  commandTimeoutSecs: number;
  contextTokenLimit: number;
  modelContextLimits?: Record<string, number>;
  modelCapabilities?: Record<string, string[]>;
  activeImageProviderId?: string | null;
  activeImageModelId?: string | null;
  activeVisionProviderId?: string | null;
  activeVisionModelId?: string | null;
  lastWorkspacePath?: string | null;
  disabledTools?: string[];
  disabledSops?: string[];
}

export interface AgentSopInfo {
  id: string;
  name: string;
  category: "thinking" | "memory" | "orchestration" | "quality" | "workflow";
  categoryLabel: string;
  description: string;
  disableEffect: string;
}

export const AGENT_SOPS: AgentSopInfo[] = [
  {
    id: "plan_first",
    name: "方案先行与计划中枢规范",
    category: "thinking",
    categoryLabel: "思考范式",
    description: "面对大改动任务（3个以上文件）或多轮需求，在 .harness/plans/ 自动建立结构化计划 MD 文档；需求变更时优先更新计划，后续严格根据计划分步执行，防止多轮需求失真与遗忘。",
    disableEffect: "禁用后：Agent 接到任务后可直接编写代码，不强制在 .harness/plans/ 建立结构化方案文档。",
  },
  {
    id: "memory_distill",
    name: "边读边记与认知沉淀规范",
    category: "memory",
    categoryLabel: "知识资产",
    description: "内置工作区长期认知记忆（.harness/memory/）；常见技术栈提问优先秒级召回；深入探索或阅读多个文件后强制调用 record_memory 沉淀技术大盘与主题碎记；对话完成后台自动萃取 digests。",
    disableEffect: "禁用后：移除强制调用 record_memory 沉淀约束，且会话结束后不触发后台自动提炼。",
  },
  {
    id: "subagent_orchestration",
    name: "子进程协作与架构编排规范",
    category: "orchestration",
    categoryLabel: "协同体系",
    description: "面对复杂需求、多模块并发开发（如前后端分离、多业务模块并行）时，总架构师按标准流程（规划拆解 -> 派生子 Agent -> 等待汇聚 -> 全局验收）采用子进程协同提升效率与模块隔离度。",
    disableEffect: "禁用后：停用多子进程协作编排指引，Agent 倾向于在单主进程内串行处理全部任务。",
  },
  {
    id: "safe_code_edit",
    name: "代码克制与精确修改规范",
    category: "quality",
    categoryLabel: "工程质量",
    description: "修改文件前必须先用 read_file 读取相关内容，用 edit_file 做基于精确原文的最小化修改；新文件才用 write_file；动手前探索代码结构，修改后尽量用 run_command 运行构建或测试验证。",
    disableEffect: "禁用后：移除代码最小化精确修改与构建验证的强指引约束。",
  },
  {
    id: "surgical_code_reading",
    name: "精益代码研读规范",
    category: "quality",
    categoryLabel: "工程质量",
    description: "探索业务与代码时遵循漏斗式渐进探索（目录/清单 ➔ file_outline 骨架 ➔ grep 线索 ➔ 局部靶向精读）；严禁对大型文件进行多轮无休止的分页切片（offset_line）式逐行通读；修改前只精读最小必要上下文，严禁凭空盲猜。",
    disableEffect: "禁用后：允许 Agent 自由阅读大段源码，解除对连续分页通读与精益读取的强约束。",
  },
  {
    id: "todo_lifecycle",
    name: "任务清单全生命周期规范",
    category: "workflow",
    categoryLabel: "任务规划",
    description: "接到多步任务时，先用 todo 工具列出计划，并随进展更新各项状态；在执行完最后一步、给出最终回复前，务必调用 todo 工具将已完成任务的状态更新为 done，切勿遗留 in_progress 状态。",
    disableEffect: "禁用后：允许 Agent 自由多步推进，不强制调用 todo 工具维护状态清单。",
  },
];

export type PlanMode = "standard" | "always_plan" | "always_proceed";

export interface Project {
  id: string;
  name: string;
  /** 项目绑定的目录（工作区即项目）；早期手动创建的项目可能为空 */
  path?: string | null;
  pinned: boolean;
  createdAt: string;
  lastActivityAt?: string | null;
  /** 项目约束（Markdown：规范 / 注意事项等）；空 = 未设置。该项目下每个新对话注入 system prompt */
  constraints?: string | null;
  sopVerifyCmd?: string | null;
  sopEnabled?: boolean;
  /** 任务规划与执行策略模式："standard" | "always_plan" | "always_proceed" */
  planMode?: PlanMode;
}

/** 关联项目：对另一目录的引用 + 说明；对方项目实体存在且已设约束时，其约束一并注入 */
export interface ProjectLink {
  id: string;
  projectId: string;
  path: string;
  description: string;
  createdAt: string;
}

export interface Attachment {
  id: string;
  name: string;
  mime_type: string;
  size: number;
  path: string;
  is_image: boolean;
}

export interface Session {
  id: string;
  title: string;
  workspacePath: string;
  accessMode?: "confirm" | "full_access" | null;
  projectId?: string | null;
  status: string; // active | archived
  lastMessageAt?: string | null;
  createdAt: string;
  updatedAt: string;
  /** 临时空间对话：工作区为项目/关联项目的临时副本 */
  isTemp: boolean;
  /** 临时空间来源：主项目原始目录 */
  sourceWorkspace?: string | null;
  mergedSeq?: number | null; // 最近一次合并的消息序号边界，之前的消息不可编辑
  mergedPending: boolean; // 已合并且临时空间未清空：禁止继续发送
  totalTokens?: number;
  promptTokens?: number;
  completionTokens?: number;
  /** 父会话 ID（子 Agent 进程非空） */
  parentSessionId?: string | null;
  /** 会话类型：main | subagent */
  sessionType?: "main" | "subagent" | string;
  /** 子 Agent 角色标签（前端开发 / 后端开发 / 测试 等） */
  subagentRole?: string | null;
  /** 子 Agent 初始分配的任务描述 */
  subagentTask?: string | null;
  /** 会话专属上下文 Token 上限（覆盖全局及模型推荐值） */
  contextTokenLimit?: number | null;
  /** 增量汇报水位线游标：上次已汇报的消息 ID（仅协作者使用） */
  lastReportedMsgId?: string | null;
  /** 协作者执行完成后是否自动汇报主会话 */
  autoReport?: boolean | null;
  /** 触发派生该子任务的工具事件 ID */
  triggerToolEventId?: string | null;
  /** 专属模型厂商 ID */
  providerId?: string | null;
  provider_id?: string | null;
  /** 专属模型 ID */
  modelId?: string | null;
  model_id?: string | null;
  /** 主进程调度触发规则 */
  dispatchRule?: string | null;
  dispatch_rule?: string | null;
  /** 专属生图模型厂商 ID */
  imageProviderId?: string | null;
  image_provider_id?: string | null;
  /** 专属生图模型 ID */
  imageModelId?: string | null;
  image_model_id?: string | null;
  /** 专属视觉模型厂商 ID */
  visionProviderId?: string | null;
  vision_provider_id?: string | null;
  /** 专属视觉模型 ID */
  visionModelId?: string | null;
  vision_model_id?: string | null;
  /** 分支来源会话 ID */
  forkedFromSessionId?: string | null;
  forked_from_session_id?: string | null;
  /** 分支来源消息 ID */
  forkedFromMessageId?: string | null;
  forked_from_message_id?: string | null;
}

export interface SubagentCreateInput {
  role: string;
  title: string;
  task: string;
  subpath?: string;
  workspacePath?: string;
}

export interface CollaboratorCreateInput {
  parentSessionId: string;
  role: string;
  title?: string;
  taskPrompt: string;
  dispatchRule?: string | null;
  subpath?: string;
  workspacePath?: string;
  autoReport?: boolean;
  providerId?: string | null;
  modelId?: string | null;
  imageProviderId?: string | null;
  imageModelId?: string | null;
  visionProviderId?: string | null;
  visionModelId?: string | null;
}

export interface CollaboratorUpdateInput {
  collaboratorId: string;
  title: string;
  role: string;
  taskPrompt: string;
  dispatchRule?: string | null;
  subpath?: string | null;
  workspacePath?: string | null;
  autoReport?: boolean;
  providerId?: string | null;
  modelId?: string | null;
  imageProviderId?: string | null;
  imageModelId?: string | null;
  visionProviderId?: string | null;
  visionModelId?: string | null;
}

export interface SessionModelsUpdateInput {
  sessionId: string;
  providerId?: string | null;
  modelId?: string | null;
  imageProviderId?: string | null;
  imageModelId?: string | null;
  visionProviderId?: string | null;
  visionModelId?: string | null;
}

export interface SessionCreateInput {
  workspacePath?: string;
  projectId?: string;
  title?: string;
  accessMode?: string;
  contextTokenLimit?: number | null;
  temp?: TempAlloc;
  imageProviderId?: string | null;
  imageModelId?: string | null;
  visionProviderId?: string | null;
  visionModelId?: string | null;
}

/** 临时空间中被拷贝的单个项目条目（主项目 key="main"，关联项目 key="link:<id>"） */
export interface TempProjectEntry {
  key: string;
  name: string;
  source: string;
  temp: string;
  description: string;
  baseline?: string | null;
}

/** alloc_temp_code 返回的临时空间计划（草稿持有，首发落库时原样回传） */
export interface TempAlloc {
  code: string;
  root: string;
  mainTemp: string;
  sourceWorkspace: string;
  projects: TempProjectEntry[];
}

/** 临时空间运行时状态（后端 get_temp_info / temp:update 事件） */
export interface TempInfo {
  isTemp: boolean;
  exists: boolean;
  hasChanges: boolean;
  changedCount: number;
  merged: boolean;
  mergedPending: boolean;
  mergedSeq?: number | null;
  tempRoot?: string | null;
  sourceWorkspace?: string | null;
}

/** 合并结果汇总 */
export interface MergeSummary {
  projects: { name: string; source: string; applied: number; aiMerged: number; skipped: string[] }[];
  totalApplied: number;
  totalAiMerged: number;
  totalSkipped: number;
}

// ---------- 临时空间变更列表 / 文件 diff ----------

/** 变更文件条目（弹窗左侧列表项） */
export interface TempChangeFile {
  path: string;
  name: string;
  change: "added" | "modified" | "deleted";
  added: number;
  removed: number;
  binary: boolean;
  tooLarge: boolean;
}

/** 按项目分组的变更（弹窗左侧分组头） */
export interface TempChangeProject {
  key: string;
  name: string;
  source: string;
  temp: string;
  files: TempChangeFile[];
}

export interface TempChanges {
  totalFiles: number;
  projects: TempChangeProject[];
}

/** diff 行：tag = same | del | add；oldNo / newNo 为 1 起始行号（该侧无对应行时为 null） */
export interface DiffLine {
  tag: "same" | "del" | "add";
  oldNo?: number | null;
  newNo?: number | null;
  text: string;
}

/** diff 分块（±3 行上下文），统一视图与并排对比共用 */
export interface DiffHunk {
  oldStart: number;
  oldLines: number;
  newStart: number;
  newLines: number;
  lines: DiffLine[];
}

/** 单文件 diff（弹窗右侧） */
export interface TempFileDiff {
  projectKey: string;
  projectName: string;
  path: string;
  name: string;
  change: "added" | "modified" | "deleted";
  binary: boolean;
  tooLarge: boolean;
  added: number;
  removed: number;
  truncated: boolean;
  hunks: DiffHunk[];
}

export interface ToolEvent {
  id: string;
  messageId: string;
  toolName: string;
  toolCallId?: string | null;
  params: any;
  resultText?: string | null;
  status: string; // pending_approval | running | success | failed | denied | timeout
  approvalScope?: string | null;
  createdAt: string;
  subprocessId?: string | null;
}

export interface Message {
  id: string;
  sessionId: string;
  runId?: string | null;
  seq: number;
  role: string; // user | assistant | tool | system
  content?: string | null;
  /** 模型思考过程（reasoning_content），仅用于展示与存档，不进入上下文组装 */
  reasoning?: string | null;
  toolCalls?: any[] | null;
  toolCallId?: string | null;
  queued: boolean;
  attachments?: Attachment[] | null;
  usage?: any;
  createdAt: string;
  toolEvents: ToolEvent[];
  durationMs?: number;
  turnDurationMs?: number;
  promptTokens?: number;
  completionTokens?: number;
  totalTokens?: number;
}

export interface QueuedItem {
  id: string;
  content: string;
  createdAt: string;
}

export interface TurnMetrics {
  isTurnEnd: boolean;
  turnDurationMs: number | null;
  turnTokens: number;
  turnPromptTokens?: number;
  turnCompletionTokens?: number;
  turnStepCount: number;
  stepIndex?: number;
  turnStartTime: number;
  isCurrentRunningTurn: boolean;
}

/**
 * 按轮次分组计算对话消息的耗时和 Token 指标。
 * 一轮对话以 user 消息发起，以该 user 后的最后一条 assistant 消息结束。
 */
export function computeTurnMetrics(messages: Message[], running: boolean): Map<string, TurnMetrics> {
  const map = new Map<string, TurnMetrics>();

  let currentTurnUser: Message | null = null;
  let currentTurnAssistants: Message[] = [];

  const flushTurn = (isLastTurn: boolean) => {
    if (currentTurnAssistants.length === 0) return;

    const rawUserTime = currentTurnUser?.createdAt ? new Date(currentTurnUser.createdAt).getTime() : NaN;
    const rawAsstTime = currentTurnAssistants[0]?.createdAt ? new Date(currentTurnAssistants[0].createdAt).getTime() : NaN;
    const turnStartTime = !isNaN(rawUserTime) ? rawUserTime : (!isNaN(rawAsstTime) ? rawAsstTime : Date.now());

    const totalSteps = currentTurnAssistants.length;
    const isCurrentRunning = isLastTurn && running;

    // 累计整个轮次的 token
    let turnTokens = 0;
    let turnPromptTokens = 0;
    let turnCompletionTokens = 0;
    for (const a of currentTurnAssistants) {
      const tt = a.totalTokens ?? (a.usage?.totalTokens || (a.usage?.inputEst || 0) + (a.usage?.outputEst || 0)) ?? 0;
      const pt = a.promptTokens ?? (a.usage?.promptTokens || a.usage?.inputEst) ?? 0;
      const ct = a.completionTokens ?? (a.usage?.completionTokens || a.usage?.outputEst) ?? 0;
      turnTokens += Number(tt) || 0;
      turnPromptTokens += Number(pt) || 0;
      turnCompletionTokens += Number(ct) || 0;
    }

    // 遍历本轮所有 assistant 消息
    for (let i = 0; i < totalSteps; i++) {
      const a = currentTurnAssistants[i];
      const isTurnEnd = i === totalSteps - 1;

      let turnDurationMs: number | null = null;
      if (isTurnEnd) {
        if (isCurrentRunning) {
          turnDurationMs = null;
        } else if (a.turnDurationMs != null && a.turnDurationMs > 0) {
          turnDurationMs = a.turnDurationMs;
        } else {
          // 历史数据兜底
          const aTime = new Date(a.createdAt).getTime();
          const durationFallback = aTime >= turnStartTime ? aTime - turnStartTime + (a.durationMs ?? 0) : (a.durationMs ?? 0);
          const sumSteps = currentTurnAssistants.reduce((sum, item) => sum + (item.durationMs ?? 0), 0);
          turnDurationMs = Math.max(durationFallback, sumSteps) || (a.durationMs ?? null);
        }
      }

      map.set(a.id, {
        isTurnEnd,
        turnDurationMs,
        turnTokens,
        turnPromptTokens,
        turnCompletionTokens,
        turnStepCount: totalSteps,
        stepIndex: i + 1,
        turnStartTime,
        isCurrentRunningTurn: isCurrentRunning && isTurnEnd,
      });
    }
  };

  for (const m of messages) {
    if (m.role === "tool" || m.queued) continue;
    if (m.role === "user") {
      flushTurn(false);
      currentTurnUser = m;
      currentTurnAssistants = [];
    } else if (m.role === "assistant") {
      currentTurnAssistants.push(m);
    }
  }

  // 最后一轮
  flushTurn(true);

  return map;
}

export interface ApprovalReq {
  eventId: string;
  sessionId: string;
  toolName: string;
  params: any;
  risk: string; // write | execute | path
  preview: string;
  forceOnce: boolean;
}

/** 上下文自动压缩挂起请求（等待用户确认/补充） */
export interface CompactionReq {
  eventId: string;
  sessionId: string;
  startSeq: number;
  endSeq: number;
  startPreview: string;
  endPreview: string;
  messageCount: number;
  tokensBefore: number;
  summary: string;
  /** 超时阻塞等待秒数（默认 30s） */
  timeoutSeconds?: number;
  /** 请求创建本地时间戳（毫秒） */
  createdAt?: number;
  /** 是否已超时无操作并自动应用（此时保持展示卡片供查看，但不提供继续下一步按钮） */
  timedOut?: boolean;
}

/** 会话历史已压缩记录 */
export interface SessionCompaction {
  id: string;
  sessionId: string;
  startSeq: number;
  endSeq: number;
  summaryMarkdown: string;
  tokensBefore: number;
  createdAt: string;
}

/** 模型上下文预设规格 */
export interface ModelContextPreset {
  id: string;
  name: string;
  category: string;
  windowTokens: number;
  recommendedLimit: number;
  desc: string;
}

/** 上下文硬截断提醒（原子轮次丢弃通知） */
export interface TruncationNotice {
  id: string;
  sessionId: string;
  droppedTurns: number;
  droppedMessages: number;
  droppedTokens: number;
  tokenLimit: number;
  estTokensBefore: number;
  estTokensAfter: number;
  firstPreview: string;
  createdAt: string;
}

/** 程序数据目录状态（后端 get_data_status） */
export interface DataStatus {
  /** 数据目录不可写，等待用户在界面中选择 */
  pending: boolean;
  /** 当前生效的数据目录（pending 时为 null） */
  dataDir: string | null;
  /** 默认存放位置（程序同级 data\） */
  defaultDir: string;
  defaultWritable: boolean;
  /** 是否使用了自定义目录 */
  isCustom: boolean;
  /** pending 时不可写目录的提示信息 */
  unwritablePath?: string | null;
}

export const DRAFT_ID = "draft";

/** 解析当前生效的厂商 + 模型：激活项失效时回落到第一个有模型的厂商 */
export function resolveActiveModel(settings: Settings): { provider: Provider; model: string } | null {
  const withModels = settings.providers.filter((p) => (p.models ?? []).length > 0);
  const provider = withModels.find((p) => p.id === settings.activeProviderId) ?? withModels[0];
  if (!provider) return null;
  const active = settings.activeModelId ?? settings.activeModel;
  const model =
    active && provider.models.includes(active)
      ? active
      : provider.models[0];
  return { provider, model };
}

/** 顶栏下拉项的复合值：厂商 id 与模型名拼合（厂商 id 为 base36，不含冒号） */
export const modelKey = (providerId: string, model: string) => `${providerId}:${model}`;

export function parseModelKey(key: string): { providerId: string; model: string } | null {
  const i = key.indexOf(":");
  if (i <= 0) return null;
  return { providerId: key.slice(0, i), model: key.slice(i + 1) };
}

export interface ModelContextPreset {
  id: string;
  name: string;
  category: string;
  windowTokens: number;
  recommendedLimit: number;
  desc: string;
}

export const MODEL_CONTEXT_PRESETS: ModelContextPreset[] = [
  {
    id: "deepseek",
    name: "DeepSeek V3 / R1",
    category: "主流云端",
    windowTokens: 64_000,
    recommendedLimit: 56_000,
    desc: "窗口 64K · 推荐安全上限 56,000",
  },
  {
    id: "claude-3-5",
    name: "Claude 3.5 Sonnet / Haiku",
    category: "主流云端",
    windowTokens: 200_000,
    recommendedLimit: 180_000,
    desc: "窗口 200K · 推荐安全上限 180,000",
  },
  {
    id: "gpt-4o",
    name: "GPT-4o / GPT-4o-mini",
    category: "主流云端",
    windowTokens: 128_000,
    recommendedLimit: 110_000,
    desc: "窗口 128K · 推荐安全上限 110,000",
  },
  {
    id: "qwen-2-5",
    name: "通义千问 Qwen 2.5 / Plus",
    category: "国内厂商",
    windowTokens: 128_000,
    recommendedLimit: 110_000,
    desc: "窗口 128K · 推荐安全上限 110,000",
  },
  {
    id: "glm-4",
    name: "智谱 GLM-4 / Plus",
    category: "国内厂商",
    windowTokens: 128_000,
    recommendedLimit: 110_000,
    desc: "窗口 128K · 推荐安全上限 110,000",
  },
  {
    id: "kimi-moonshot",
    name: "Kimi / Moonshot",
    category: "国内厂商",
    windowTokens: 200_000,
    recommendedLimit: 180_000,
    desc: "窗口 200K · 推荐安全上限 180,000",
  },
  {
    id: "gemini-2",
    name: "Gemini 1.5 / 2.0",
    category: "超长上下文",
    windowTokens: 1_000_000,
    recommendedLimit: 200_000,
    desc: "窗口 1M+ · 推荐安全上限 200,000+",
  },
  {
    id: "ollama-32k",
    name: "本地 32K (Ollama / Mistral)",
    category: "本地模型",
    windowTokens: 32_000,
    recommendedLimit: 28_000,
    desc: "窗口 32K · 推荐安全上限 28,000",
  },
  {
    id: "ollama-8k",
    name: "本地 8K (Llama 3 8B 默认)",
    category: "本地模型",
    windowTokens: 8_192,
    recommendedLimit: 7_000,
    desc: "窗口 8K · 推荐安全上限 7,000",
  },
];

/** 根据模型名称启发式推断预设上下文上限 */
export function inferModelContextLimit(model: string): number {
  const lower = model.toLowerCase();
  if (lower.includes("deepseek")) return 56_000;
  if (lower.includes("claude")) return 180_000;
  if (lower.includes("gpt-4o") || lower.includes("gpt-4.5") || lower.includes("o1") || lower.includes("o3")) return 110_000;
  if (lower.includes("qwen") || lower.includes("千问")) return 110_000;
  if (lower.includes("glm")) return 110_000;
  if (lower.includes("kimi") || lower.includes("moonshot")) return 180_000;
  if (lower.includes("gemini")) return 200_000;
  if (lower.includes("8k")) return 7_000;
  if (lower.includes("16k")) return 14_000;
  if (lower.includes("32k")) return 28_000;
  if (lower.includes("64k")) return 56_000;
  if (lower.includes("128k")) return 110_000;
  if (lower.includes("200k")) return 180_000;
  if (lower.includes("1m")) return 200_000;
  return 64_000;
}

/**
 * 解析具体模型的生效上下文上限：
 * 1. 优先根据 `providerId:model` 查找精准配置
 * 2. 其次根据 `model` 查找全局模型配置
 * 3. 再次根据模型名关键字推断预设规格
 * 4. 最后回落至全局保底值 settings.contextTokenLimit
 */
export function resolveModelContextLimit(
  settings: Settings,
  providerId?: string | null,
  model?: string | null
): number {
  if (!model) return settings.contextTokenLimit || 64_000;
  const limits = settings.modelContextLimits || {};
  if (providerId && limits[`${providerId}:${model}`] != null) {
    return limits[`${providerId}:${model}`];
  }
  if (limits[model] != null) {
    return limits[model];
  }
  const inferred = inferModelContextLimit(model);
  if (inferred !== 64_000) {
    return inferred;
  }
  return settings.contextTokenLimit || 64_000;
}

/** 根据模型名称启发式推断默认能力（chat 对话、image_gen 生图、vision 视觉） */
export function inferDefaultModelCapabilities(model: string): string[] {
  const lower = model.toLowerCase();
  if (lower.includes("embedding") || lower.includes("rerank")) {
    return [];
  }
  const isImg =
    lower.includes("seedream") ||
    lower.includes("seedance") ||
    lower.includes("seed-edit") ||
    lower.includes("cogview") ||
    lower.includes("dall-e") ||
    lower.includes("dalle") ||
    lower.includes("flux") ||
    lower.includes("sdxl") ||
    lower.includes("stable-diffusion") ||
    lower.includes("wanx") ||
    lower.includes("kolors") ||
    lower.includes("t2i") ||
    lower.includes("imagen") ||
    lower.includes("image-gen") ||
    lower.includes("image_gen");
  if (isImg) {
    return ["image_gen"];
  }
  const isVis =
    lower.includes("vl") ||
    lower.includes("vision") ||
    lower.includes("4v") ||
    lower.includes("omni") ||
    lower.includes("gpt-4o") ||
    lower.includes("claude-3") ||
    lower.includes("gemini");
  if (isVis) {
    return ["chat", "vision"];
  }
  return ["chat"];
}

/**
 * 获取指定模型的能力列表：
 * 1. 优先根据 `providerId:model` 查找用户显式配置
 * 2. 其次根据 `model` 查找全局模型配置
 * 3. 缺省时通过模型名进行平滑推断
 */
export function resolveModelCapabilities(
  settings: Settings,
  providerId?: string | null,
  model?: string | null
): string[] {
  if (!model) return ["chat"];
  const caps = settings.modelCapabilities || {};
  if (providerId && caps[`${providerId}:${model}`] != null) {
    return caps[`${providerId}:${model}`];
  }
  if (caps[model] != null) {
    return caps[model];
  }
  return inferDefaultModelCapabilities(model);
}

/** 检查指定模型是否具备某项特定能力 */
export function hasModelCapability(
  settings: Settings,
  providerId: string | null | undefined,
  model: string | null | undefined,
  cap: string
): boolean {
  const list = resolveModelCapabilities(settings, providerId, model);
  return list.includes(cap);
}

export type ModelCapability = "chat" | "image_gen" | "vision";

export interface ModelCapabilityMeta {
  id: ModelCapability;
  label: string;
  icon: string;
  description: string;
  badgeClass: string;
}

export const MODEL_CAPABILITY_METAS: Record<ModelCapability, ModelCapabilityMeta> = {
  chat: {
    id: "chat",
    label: "对话思考",
    icon: "💬",
    description: "驱动日常对话、任务规划、代码编写与执行逻辑推演",
    badgeClass: "bg-blue-500/15 text-blue-400 border-blue-500/30",
  },
  image_gen: {
    id: "image_gen",
    label: "图像生成",
    icon: "🎨",
    description: "驱动生图工具 (generate_image)，支持海报/图标/概念图生成",
    badgeClass: "bg-pink-500/15 text-pink-400 border-pink-500/30",
  },
  vision: {
    id: "vision",
    label: "视觉感知",
    icon: "👁️",
    description: "驱动图片多模态理解与视觉识别，支持分析用户截图与设计稿",
    badgeClass: "bg-purple-500/15 text-purple-400 border-purple-500/30",
  },
};

/** 解析当前全局生效的生图厂商 + 模型 */
export function resolveActiveImageModel(settings: Settings): { provider: Provider; model: string } | null {
  if (settings.activeImageProviderId && settings.activeImageModelId) {
    const p = settings.providers.find((item) => item.id === settings.activeImageProviderId);
    if (p && p.models.includes(settings.activeImageModelId)) {
      return { provider: p, model: settings.activeImageModelId };
    }
  }
  for (const p of settings.providers) {
    for (const m of p.models) {
      if (hasModelCapability(settings, p.id, m, "image_gen")) {
        return { provider: p, model: m };
      }
    }
  }
  return null;
}

/** 解析当前全局生效的视觉厂商 + 模型 */
export function resolveActiveVisionModel(settings: Settings): { provider: Provider; model: string } | null {
  if (settings.activeVisionProviderId && settings.activeVisionModelId) {
    const p = settings.providers.find((item) => item.id === settings.activeVisionProviderId);
    if (p && p.models.includes(settings.activeVisionModelId)) {
      return { provider: p, model: settings.activeVisionModelId };
    }
  }
  for (const p of settings.providers) {
    for (const m of p.models) {
      if (hasModelCapability(settings, p.id, m, "vision")) {
        return { provider: p, model: m };
      }
    }
  }
  return resolveActiveModel(settings);
}

/** 预设角色的默认调度触发规则（主进程 System Prompt 注入使用） */
export function defaultDispatchRuleForRole(role: string): string {
  switch (role) {
    case "image_gen":
      return "当用户提出画图、生成图片、插图、海报、Logo、图标、配图制作等视觉生成需求时，必须优先委派本协作者。";
    case "frontend":
      return "当涉及 UI 界面设计、页面实现、组件重构、Vue/React 模板与 CSS 交互开发时，必须优先委派本协作者。";
    case "backend":
      return "当涉及服务端业务逻辑、API 接口、数据库 CRUD、后台架构开发时，必须优先委派本协作者。";
    case "pm":
      return "当涉及需求分析梳理、PRD 方案编写、功能边界与业务流程设计时，必须优先委派本协作者。";
    case "pmo":
      return "当涉及任务拆解(WBS)、里程碑节点排期、进度与风险追踪时，必须优先委派本协作者。";
    case "vision":
      return "当用户发送图片、截图、设计稿并要求视觉识别分析时，必须优先委派本协作者。";
    case "testing":
      return "当需要编写自动化测试用例、单元测试、执行回归测试与缺陷验证时，必须优先委派本协作者。";
    case "review":
      return "当需要对代码实现进行质量审查、重构优化与架构防劣化时，必须优先委派本协作者。";
    case "fullstack":
      return "当涉及端到端打通前后端完整功能链路开发时，必须优先委派本协作者。";
    default:
      return "当用户任务属于本协作者专业领域范围时，必须优先委派本协作者处理。";
  }
}


/** 将 token 数量格式化为易读的文本（如 56k, 180k, 1.2m） */
export function formatTokens(tokens: number): string {
  if (tokens >= 1_000_000) {
    const m = tokens / 1_000_000;
    return m % 1 === 0 ? `${m}M` : `${m.toFixed(1)}M`;
  }
  if (tokens >= 1_000) {
    const k = tokens / 1_000;
    return k % 1 === 0 ? `${k}k` : `${k.toFixed(1)}k`;
  }
  return String(tokens);
}

const IS_WIN = typeof navigator !== "undefined" && /win/i.test(navigator.platform);

/** 路径比较键：去掉尾部分隔符；Windows 下统一分隔符并忽略大小写（与后端 path_key 一致） */
function pathKey(p: string): string {
  const t = p.replace(/[\\/]+$/, "");
  return IS_WIN ? t.replace(/\\/g, "/").toLowerCase() : t;
}

/** 判断两个目录路径是否指向同一目录 */
export function samePath(a?: string | null, b?: string | null): boolean {
  if (!a || !b) return false;
  return pathKey(a) === pathKey(b);
}

/** 取路径最后一段非空目录名（后端自动建项目时的命名规则） */
export function dirName(p: string): string {
  const t = p.replace(/[\\/]+$/, "");
  const i = Math.max(t.lastIndexOf("/"), t.lastIndexOf("\\"));
  return (i >= 0 ? t.slice(i + 1) : t) || t;
}

/** 运行中的会话（全局查询结果项）：界面刷新后恢复运行状态用 */
export interface RunningSession {
  sessionId: string;
  runId: string;
}

export interface GrowthItem {
  id: string;
  projectId?: string | null;
  sessionId?: string | null;
  sessionTitle?: string | null;
  messageId?: string | null;
  runId?: string | null;
  triggerType: "user_rejection" | "self_healed" | "user_taught" | "manual" | string;
  triggerContext: string;
  reflectionThought: string;
  category: "command_rule" | "code_style" | "build_test" | "pitfall" | "workflow" | string;
  title: string;
  ruleContent: string;
  status: "proposed" | "accepted" | "rejected" | "disabled" | string;
  appliedCount: number;
  createdAt: string;
  updatedAt: string;
}

export interface SkillItem {
  name: string;
  description: string;
  scriptType: "bat" | "ps1" | "sh" | "py" | "js" | string;
  path: string;
  content: string;
  updatedAt: string;
}

export interface ProjectSopInfo {
  projectId: string;
  projectName: string;
  sopVerifyCmd: string;
  sopEnabled: boolean;
  detectedStack: string;
  detectedDefaultCmd: string;
}

export interface SopStatusEvent {
  sessionId: string;
  status: "checking" | "passed" | "failed" | "error";
  command: string;
  output?: string;
}

export interface ToolRetryGuidanceEvent {
  sessionId: string;
  toolName: string;
  attempt?: number;
  maxRetries?: number;
  status?: "retrying" | "success" | "failed";
  error?: string;
  message?: string;
}

export interface ToolRetryStatus {
  toolName: string;
  attempt: number;
  maxRetries: number;
  status: "retrying" | "success" | "failed" | "cancelled";
  error?: string;
  message?: string;
  dismissed?: boolean;
}

// ---------- Token 消耗统计 ----------

export interface TokenStatsSummary {
  totalPromptTokens: number;
  totalCompletionTokens: number;
  totalTokens: number;
  todayPromptTokens: number;
  todayCompletionTokens: number;
  todayTokens: number;
  totalSessions: number;
  totalMessages: number;
}

export interface ProjectTokenStats {
  projectId?: string | null;
  projectName: string;
  projectPath?: string | null;
  promptTokens: number;
  completionTokens: number;
  totalTokens: number;
  sessionCount: number;
  messageCount: number;
  lastUsedAt?: string | null;
}

export interface DailyTokenStats {
  date: string; // YYYY-MM-DD
  promptTokens: number;
  completionTokens: number;
  totalTokens: number;
  messageCount: number;
}

export interface SessionTokenStats {
  sessionId: string;
  title: string;
  projectId?: string | null;
  projectName?: string | null;
  promptTokens: number;
  completionTokens: number;
  totalTokens: number;
  messageCount: number;
  lastMessageAt?: string | null;
}

export interface TokenStatsReport {
  summary: TokenStatsSummary;
  byProject: ProjectTokenStats[];
  byTime: DailyTokenStats[];
  bySession: SessionTokenStats[];
}

export interface SessionActiveState {
  sessionId: string;
  isRunning: boolean;
  activeRunId: string | null;
  currentMessageId: string | null;
  streamingContent: string;
  streamingReasoning: string;
  activeToolEvents: ToolEvent[];
  pendingApproval: ApprovalReq | null;
  pendingCompaction: CompactionReq | null;
}

// ---------- 文件与变更查看器类型 ----------

export interface FileTextContent {
  path: string;
  name: string;
  content: string;
  sizeBytes: number;
  linesCount: number;
  isBinary: boolean;
  isTruncated: boolean;
  language: string;
}

export type ViewerTabType = "file" | "plan" | "diff" | "image";

export interface BaseViewerTab {
  id: string;
  type: ViewerTabType;
  title: string;
  subtitle?: string;
  workspacePath?: string;
  pinned?: boolean;
}

export interface FileViewerTab extends BaseViewerTab {
  type: "file";
  path: string;
  highlightLine?: number;
  highlightRange?: { start: number; end?: number };
  sessionId?: string;
}

export interface ImageViewerTab extends BaseViewerTab {
  type: "image";
  path: string;
}

export interface PlanViewerTab extends BaseViewerTab {
  type: "plan";
  planId?: string;
  sessionId?: string;
  filename?: string;
}

export interface DiffViewerTab extends BaseViewerTab {
  type: "diff";
  diffSource: "temp" | "git" | "custom";
  path: string;
  projectKey?: string;
  sessionId?: string;
  oldContent?: string;
  newContent?: string;
  defaultMode?: "unified" | "split";
}

export type ViewerTabItem = FileViewerTab | PlanViewerTab | DiffViewerTab | ImageViewerTab;

export interface FileOutlineItem {
  line: number;
  symbol: string;
  kind: string;
}

// ==================== 长任务（Long-Running Task）类型定义 ====================

export type LongTaskStatus =
  | "planning"
  | "running"
  | "paused"
  | "waiting_approval"
  | "completed"
  | "failed"
  | "cancelled"
  | string;

export interface TaskSubItem {
  id: string;
  index: number;
  title: string;
  description?: string | null;
  status: "pending" | "in_progress" | "verifying" | "completed" | "failed" | "skipped" | string;
  summary?: string | null;
  error?: string | null;
  verifyCommand?: string | null;
  verifyOutput?: string | null;
}

export interface LongTask {
  id: string;
  sessionId: string;
  workspacePath: string;
  goal: string;
  status: LongTaskStatus;
  currentSubtaskIndex: number;
  subtasks: TaskSubItem[];
  maxBudgetTokens?: number | null;
  totalTokensUsed: number;
  currentStep: number;
  maxSteps: number;
  createdAt: string;
  updatedAt: string;
}

export interface TaskCheckpoint {
  id: string;
  taskId: string;
  stepNumber: number;
  subtaskId?: string | null;
  status: string;
  summary: string;
  workingMemory: string;
  gitCommitHash?: string | null;
  createdAt: string;
}





