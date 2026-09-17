import { invoke } from "@tauri-apps/api/core";
import type { ApprovalRule, DataStatus, GrowthItem, Message, MergeSummary, Project, ProjectLink, ProjectSopInfo, RunningSession, Session, SessionActiveState, SessionCompaction, Settings, SkillItem, TempAlloc, TempChanges, TempFileDiff, TempInfo, TokenStatsReport, ToolEvent, ToolInfo } from "./types";

export const ipc = {
  getSettings: () => invoke<Settings>("get_settings"),
  setSettings: (settings: Settings) => invoke<void>("set_settings", { settings }),
  listTools: () => invoke<ToolInfo[]>("list_tools"),
  testProvider: (provider: any) => invoke<string>("test_provider", { provider }),

  // 数据目录
  getDataStatus: () => invoke<DataStatus>("get_data_status"),
  setDataDir: (path: string) => invoke<void>("set_data_dir", { path }),
  resetDataDir: () => invoke<void>("reset_data_dir"),
  exitApp: () => invoke<void>("exit_app"),

  listProjects: () => invoke<Project[]>("list_projects"),
  createProject: (name: string, path?: string) =>
    invoke<Project>("create_project", { name, path: path ?? null }),
  removeProject: (id: string) => invoke<void>("remove_project", { id }),
  setProjectPinned: (id: string, pinned: boolean) =>
    invoke<void>("set_project_pinned", { id, pinned }),

  // 项目设置：项目约束 / 关联项目
  setProjectConstraints: (id: string, constraints: string) =>
    invoke<void>("set_project_constraints", { id, constraints }),
  listProjectLinks: (projectId: string) =>
    invoke<ProjectLink[]>("list_project_links", { projectId }),
  addProjectLink: (projectId: string, path: string, description: string) =>
    invoke<ProjectLink>("add_project_link", { projectId, path, description }),
  updateProjectLink: (id: string, path: string, description: string) =>
    invoke<ProjectLink>("update_project_link", { id, path, description }),
  deleteProjectLink: (id: string) => invoke<void>("delete_project_link", { id }),

  listSessions: () => invoke<Session[]>("list_sessions"),
  listArchived: () => invoke<Session[]>("list_archived"),
  renameSession: (id: string, title: string) => invoke<void>("rename_session", { id, title }),
  deleteSession: (id: string) => invoke<void>("delete_session", { id }),
  archiveSession: (id: string) => invoke<void>("archive_session", { id }),
  unarchiveSession: (id: string) => invoke<void>("unarchive_session", { id }),
  setSessionMode: (id: string, mode: string) => invoke<void>("set_session_mode", { id, mode }),
  setSessionWorkspace: (id: string, path: string) =>
    invoke<void>("set_session_workspace", { id, path }),
  setSessionProject: (id: string, projectId: string | null) =>
    invoke<void>("set_session_project", { id, projectId }),

  getMessages: (sessionId: string, beforeSeq?: number, limit?: number) =>
    invoke<Message[]>("get_messages", { sessionId, beforeSeq: beforeSeq ?? null, limit: limit ?? 200 }),

  sendMessage: (
    sessionId: string | null,
    text: string,
    workspacePath?: string,
    projectId?: string,
    temp?: TempAlloc,
    accessMode?: string,
  ) =>
    invoke<{ sessionId: string; messageId: string; queued: boolean; session?: Session | null }>("send_message", {
      sessionId,
      text,
      workspacePath: workspacePath ?? null,
      projectId: projectId ?? null,
      temp: temp ?? null,
      accessMode: accessMode ?? null,
    }),
  listQueued: (sessionId: string) => invoke<Message[]>("list_queued", { sessionId }),
  guideMessage: (sessionId: string, messageId: string) =>
    invoke<void>("guide_message", { sessionId, messageId }),
  deleteQueuedMessage: (sessionId: string, messageId: string) =>
    invoke<void>("delete_queued_message", { sessionId, messageId }),
  stopRun: (sessionId: string) => invoke<void>("stop_run", { sessionId }),
  // 子 Agent 进程协作
  listSubagents: (parentSessionId: string) =>
    invoke<Session[]>("list_subagents", { parentSessionId }),
  spawnSubagent: (input: { parentSessionId: string; role: string; taskPrompt: string; title?: string; subpath?: string | null }) =>
    invoke<Session>("spawn_subagent", {
      parentSessionId: input.parentSessionId,
      role: input.role,
      title: input.title ?? `${input.role}任务`,
      task: input.taskPrompt,
      subpath: input.subpath ?? null,
    }),
  stopSubagent: (subagentId: string) =>
    invoke<void>("stop_subagent", { subagentId }),
  restartSubagent: (subagentId: string) =>
    invoke<void>("restart_subagent", { subagentId }),
  restartAllSubagents: (parentSessionId: string) =>
    invoke<number>("restart_all_subagents", { parentSessionId }),
  deleteSubagent: (subagentId: string) =>
    invoke<void>("delete_subagent", { subagentId }),
  killCommand: (eventId: string) => invoke<void>("kill_command", { eventId }),
  listRunningSessions: () => invoke<RunningSession[]>("list_running_sessions"),
  editAndResend: (sessionId: string, messageId: string, newText: string) =>
    invoke<void>("edit_and_resend", { sessionId, messageId, newText }),

  respondApproval: (eventId: string, decision: string, reason?: string) =>
    invoke<void>("respond_approval", { eventId, decision, reason: reason ?? null }),

  respondCompaction: (eventId: string, approved: boolean, finalSummary: string) =>
    invoke<void>("respond_compaction", { eventId, approved, finalSummary }),

  listSessionCompactions: (sessionId: string) =>
    invoke<SessionCompaction[]>("list_session_compactions", { sessionId }),

  // 审批规则（会话级：仅对所属对话生效）
  listSessionRules: (sessionId: string) =>
    invoke<ApprovalRule[]>("list_session_rules", { sessionId }),
  deleteSessionRule: (sessionId: string, id: string) =>
    invoke<ApprovalRule[]>("delete_session_rule", { sessionId, id }),
  getSessionTodos: (sessionId: string) => invoke<any>("get_session_todos", { sessionId }),

  // 临时空间
  allocTempCode: (projectId: string) => invoke<TempAlloc>("alloc_temp_code", { projectId }),
  getTempInfo: (sessionId: string) => invoke<TempInfo>("get_temp_info", { sessionId }),
  listTempChanges: (sessionId: string) => invoke<TempChanges>("list_temp_changes", { sessionId }),
  getTempChangeDiff: (sessionId: string, projectKey: string, path: string) =>
    invoke<TempFileDiff>("get_temp_change_diff", { sessionId, projectKey, path }),
  mergeTempSpace: (sessionId: string) => invoke<MergeSummary>("merge_temp_space", { sessionId }),
  clearTempSpace: (sessionId: string) => invoke<void>("clear_temp_space", { sessionId }),

  // 打开目录（系统文件管理器）
  openDir: (path: string) => invoke<void>("open_dir", { path }),

  // 成长演进 (Growth)
  listGrowths: (projectId?: string | null, status?: string | null) =>
    invoke<GrowthItem[]>("list_growths", { projectId: projectId ?? null, status: status ?? null }),
  updateGrowthStatus: (id: string, status: string) =>
    invoke<void>("update_growth_status", { id, status }),
  updateGrowthRule: (id: string, title: string, ruleContent: string, category: string) =>
    invoke<void>("update_growth_rule", { id, title, ruleContent, category }),
  deleteGrowth: (id: string) => invoke<void>("delete_growth", { id }),
  triggerGrowthReflection: (sessionId: string, userInstruction?: string) =>
    invoke<void>("trigger_growth_reflection", { sessionId, userInstruction: userInstruction ?? null }),

  // 技能库 (Skills)
  listProjectSkills: (workspacePath: string) =>
    invoke<SkillItem[]>("list_project_skills", { workspacePath }),
  saveProjectSkill: (workspacePath: string, name: string, description: string, scriptType: string, content: string) =>
    invoke<SkillItem>("save_project_skill", { workspacePath, name, description, scriptType, content }),
  deleteProjectSkill: (workspacePath: string, skillName: string) =>
    invoke<void>("delete_project_skill", { workspacePath, skillName }),

  // 交付自检 SOP
  getProjectSop: (workspacePath: string, projectId?: string | null) =>
    invoke<ProjectSopInfo>("get_project_sop", { workspacePath, projectId: projectId ?? null }),
  setProjectSop: (projectId: string, verifyCmd: string | null, enabled: boolean) =>
    invoke<void>("set_project_sop", { projectId, verifyCmd, enabled }),
  runWorkspaceSop: (workspacePath: string, cmd: string) =>
    invoke<string>("run_workspace_sop", { workspacePath, cmd }),
  getTokenStats: (projectId?: string | null, days?: number | null) =>
    invoke<TokenStatsReport>("get_token_stats", { projectId: projectId ?? null, days: days ?? null }),
  getSessionActiveState: (sessionId: string) =>
    invoke<SessionActiveState>("get_session_active_state", { sessionId }),
};

export type { Message, Session, SessionActiveState, Project, ProjectLink, RunningSession, Settings, ToolEvent, DataStatus, TempAlloc, TempInfo, MergeSummary, GrowthItem, SkillItem, ProjectSopInfo, TokenStatsReport };

