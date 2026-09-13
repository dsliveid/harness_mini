export interface Provider {
  id: string;
  name: string; // 厂商名称
  baseUrl: string;
  models: string[]; // 该厂商下配置的模型列表
  apiKey: string;
}

export interface ApprovalRule {
  id: string;
  kind: string; // command_prefix | path_write | tool
  pattern: string;
  scope: string;
  createdAt: string;
}

export interface Settings {
  providers: Provider[];
  activeProviderId?: string | null;
  activeModelId?: string | null; // 全局激活的模型，属于 activeProviderId 指向的厂商
  globalAccessMode: "confirm" | "full_access";
  maxSteps: number;
  commandTimeoutSecs: number;
  contextTokenLimit: number;
  lastWorkspacePath?: string | null;
  approvalRules: ApprovalRule[];
}

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
}

/** 关联项目：对另一目录的引用 + 说明；对方项目实体存在且已设约束时，其约束一并注入 */
export interface ProjectLink {
  id: string;
  projectId: string;
  path: string;
  description: string;
  createdAt: string;
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
}

export interface Message {
  id: string;
  sessionId: string;
  runId?: string | null;
  seq: number;
  role: string; // user | assistant | tool | system
  content?: string | null;
  toolCalls?: any[] | null;
  toolCallId?: string | null;
  queued: boolean;
  usage?: any;
  createdAt: string;
  toolEvents: ToolEvent[];
}

export interface QueuedItem {
  id: string;
  content: string;
  createdAt: string;
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
  const model =
    settings.activeModelId && provider.models.includes(settings.activeModelId)
      ? settings.activeModelId
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
