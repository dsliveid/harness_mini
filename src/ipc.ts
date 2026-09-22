import { invoke } from "@tauri-apps/api/core";
import type { ActivePlanDetail, ApprovalRule, Attachment, CollaboratorCreateInput, CollaboratorUpdateInput, DataStatus, FileTextContent, GrowthItem, Message, MergeSummary, PlanSummary, Project, ProjectLink, ProjectSopInfo, RunningSession, Session, SessionActiveState, SessionCompaction, SessionCreateInput, SessionModelsUpdateInput, Settings, SkillItem, TempAlloc, TempChanges, TempFileDiff, TempInfo, TokenStatsReport, ToolEvent, ToolInfo, ViewerTabItem, DiffHunk, FileOutlineItem, LongTask, TaskCheckpoint, TaskSubItem } from "./types";

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

  // 项目设置：项目约束 / 执行策略 / 关联项目
  setProjectConstraints: (id: string, constraints: string) =>
    invoke<void>("set_project_constraints", { id, constraints }),
  setProjectPlanMode: (id: string, mode: string) =>
    invoke<void>("set_project_plan_mode", { id, mode }),
  listProjectLinks: (projectId: string) =>
    invoke<ProjectLink[]>("list_project_links", { projectId }),
  addProjectLink: (projectId: string, path: string, description: string) =>
    invoke<ProjectLink>("add_project_link", { projectId, path, description }),
  updateProjectLink: (id: string, path: string, description: string) =>
    invoke<ProjectLink>("update_project_link", { id, path, description }),
  deleteProjectLink: (id: string) => invoke<void>("delete_project_link", { id }),

  listSessions: () => invoke<Session[]>("list_sessions"),
  createSession: (input: SessionCreateInput) =>
    invoke<Session>("create_session", {
      workspacePath: input.workspacePath ?? null,
      projectId: input.projectId ?? null,
      title: input.title ?? null,
      accessMode: input.accessMode ?? null,
      contextTokenLimit: input.contextTokenLimit ?? null,
      temp: input.temp ?? null,
      imageProviderId: input.imageProviderId ?? null,
      imageModelId: input.imageModelId ?? null,
      visionProviderId: input.visionProviderId ?? null,
      visionModelId: input.visionModelId ?? null,
    }),
  forkSessionAtMessage: (sessionId: string, messageId: string, newTitle?: string, includeTarget: boolean = true) =>
    invoke<Session>("fork_session_at_message", {
      sessionId,
      messageId,
      newTitle: newTitle ?? null,
      includeTarget,
    }),
  listArchived: () => invoke<Session[]>("list_archived"),
  renameSession: (id: string, title: string) => invoke<void>("rename_session", { id, title }),
  deleteSession: (id: string) => invoke<void>("delete_session", { id }),
  archiveSession: (id: string) => invoke<void>("archive_session", { id }),
  unarchiveSession: (id: string) => invoke<void>("unarchive_session", { id }),
  setSessionMode: (id: string, mode: string) => invoke<void>("set_session_mode", { id, mode }),
  setSessionContextLimit: (id: string, limit: number | null) =>
    invoke<void>("set_session_context_limit", { id, limit }),
  setSessionWorkspace: (id: string, path: string) =>
    invoke<void>("set_session_workspace", { id, path }),
  setSessionProject: (id: string, projectId: string | null) =>
    invoke<void>("set_session_project", { id, projectId }),

  getMessages: (sessionId: string, beforeSeq?: number, limit?: number) =>
    invoke<Message[]>("get_messages", { sessionId, beforeSeq: beforeSeq ?? null, limit: limit ?? 200 }),

  saveAttachment: (input: {
    sessionId?: string | null;
    name: string;
    mimeType: string;
    base64Data?: string | null;
    sourcePath?: string | null;
  }) =>
    invoke<Attachment>("save_attachment", {
      sessionId: input.sessionId ?? null,
      name: input.name,
      mimeType: input.mimeType,
      base64Data: input.base64Data ?? null,
      sourcePath: input.sourcePath ?? null,
    }),

  sendMessage: (
    sessionId: string | null,
    text: string,
    workspacePath?: string,
    projectId?: string,
    temp?: TempAlloc,
    accessMode?: string,
    contextTokenLimit?: number | null,
    attachments?: Attachment[] | null,
    imageProviderId?: string | null,
    imageModelId?: string | null,
    visionProviderId?: string | null,
    visionModelId?: string | null,
  ) =>
    invoke<{ sessionId: string; messageId: string; queued: boolean; session?: Session | null }>("send_message", {
      sessionId,
      text,
      workspacePath: workspacePath ?? null,
      projectId: projectId ?? null,
      temp: temp ?? null,
      accessMode: accessMode ?? null,
      contextTokenLimit: contextTokenLimit ?? null,
      attachments: attachments ?? null,
      imageProviderId: imageProviderId ?? null,
      imageModelId: imageModelId ?? null,
      visionProviderId: visionProviderId ?? null,
      visionModelId: visionModelId ?? null,
    }),
  listQueued: (sessionId: string) => invoke<Message[]>("list_queued", { sessionId }),
  guideMessage: (sessionId: string, messageId: string) =>
    invoke<void>("guide_message", { sessionId, messageId }),
  deleteQueuedMessage: (sessionId: string, messageId: string) =>
    invoke<void>("delete_queued_message", { sessionId, messageId }),
  stopRun: (sessionId: string) => invoke<void>("stop_run", { sessionId }),
  retryTurn: (sessionId: string) => invoke<void>("retry_turn", { sessionId }),
  continueTurn: (sessionId: string) => invoke<void>("continue_turn", { sessionId }),
  // 协作者与子进程协作
  listCollaborators: (parentSessionId: string) =>
    invoke<Session[]>("list_collaborators", { parentSessionId }),
  listSubprocesses: (parentSessionId: string) =>
    invoke<Session[]>("list_subprocesses", { parentSessionId }),
  createCollaborator: (input: CollaboratorCreateInput) =>
    invoke<Session>("create_collaborator", {
      parentSessionId: input.parentSessionId,
      role: input.role,
      title: input.title ?? `${input.role}协作者`,
      taskPrompt: input.taskPrompt,
      subpath: input.subpath ?? null,
      workspacePath: input.workspacePath ?? null,
      autoReport: input.autoReport ?? true,
      providerId: input.providerId ?? null,
      modelId: input.modelId ?? null,
      dispatchRule: input.dispatchRule ?? null,
      imageProviderId: input.imageProviderId ?? null,
      imageModelId: input.imageModelId ?? null,
      visionProviderId: input.visionProviderId ?? null,
      visionModelId: input.visionModelId ?? null,
    }),
  updateCollaborator: (input: CollaboratorUpdateInput) =>
    invoke<Session>("update_collaborator", {
      collaboratorId: input.collaboratorId,
      title: input.title,
      role: input.role,
      taskPrompt: input.taskPrompt,
      dispatchRule: input.dispatchRule ?? null,
      subpath: input.subpath ?? null,
      workspacePath: input.workspacePath ?? null,
      autoReport: input.autoReport ?? true,
      providerId: input.providerId ?? null,
      modelId: input.modelId ?? null,
      imageProviderId: input.imageProviderId ?? null,
      imageModelId: input.imageModelId ?? null,
      visionProviderId: input.visionProviderId ?? null,
      visionModelId: input.visionModelId ?? null,
    }),
  setSessionModels: (input: SessionModelsUpdateInput) =>
    invoke<Session>("set_session_models", {
      sessionId: input.sessionId,
      providerId: input.providerId ?? null,
      modelId: input.modelId ?? null,
      imageProviderId: input.imageProviderId ?? null,
      imageModelId: input.imageModelId ?? null,
      visionProviderId: input.visionProviderId ?? null,
      visionModelId: input.visionModelId ?? null,
    }),
  setCollaboratorAutoReport: (collaboratorId: string, autoReport: boolean) =>
    invoke<void>("set_collaborator_auto_report", { collaboratorId, autoReport }),
  reportCollaboratorIncrement: (collaboratorId: string) =>
    invoke<string>("report_collaborator_increment", { collaboratorId }),
  listSubagents: (parentSessionId: string) =>
    invoke<Session[]>("list_subagents", { parentSessionId }),
  spawnSubagent: (input: { parentSessionId: string; role: string; taskPrompt: string; title?: string; subpath?: string | null; workspacePath?: string | null }) =>
    invoke<Session>("spawn_subagent", {
      parentSessionId: input.parentSessionId,
      role: input.role,
      title: input.title ?? `${input.role}任务`,
      task: input.taskPrompt,
      subpath: input.subpath ?? null,
      workspacePath: input.workspacePath ?? null,
    }),
  stopSubagent: (subagentId: string) =>
    invoke<void>("stop_subagent", { subagentId }),
  restartSubagent: (subagentId: string) =>
    invoke<void>("restart_subagent", { subagentId }),
  restartAllSubagents: (parentSessionId: string) =>
    invoke<number>("restart_all_subagents", { parentSessionId }),
  deleteSubagent: (subagentId: string) =>
    invoke<void>("delete_subagent", { subagentId }),
  reportSubagentToParent: (subagentId: string) =>
    invoke<string>("report_subagent_to_parent", { subagentId }),
  killCommand: (eventId: string) => invoke<void>("kill_command", { eventId }),
  listRunningSessions: () => invoke<RunningSession[]>("list_running_sessions"),
  editAndResend: (sessionId: string, messageId: string, newText: string, attachments?: Attachment[]) =>
    invoke<void>("edit_and_resend", {
      sessionId,
      messageId,
      newText,
      attachments: attachments && attachments.length > 0 ? attachments : undefined,
    }),

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
  readFileBase64: (path: string) => invoke<string>("read_file_base64", { path }),

  // 任务方案计划中枢 (Plans)
  getActivePlan: (sessionId: string) =>
    invoke<ActivePlanDetail | null>("get_active_plan", { sessionId }),
  listWorkspacePlans: (workspacePath: string, sessionId?: string | null, includeArchived?: boolean) =>
    invoke<PlanSummary[]>("list_workspace_plans", {
      workspacePath,
      sessionId: sessionId ?? null,
      includeArchived: includeArchived ?? false,
    }),
  getPlanDetail: (workspacePath: string, sessionId?: string | null, planId?: string | null) =>
    invoke<ActivePlanDetail | null>("get_plan_detail", {
      workspacePath,
      sessionId: sessionId ?? null,
      planId: planId ?? null,
    }),
  updatePlanStepStatus: (
    workspacePath: string,
    planId: string | null | undefined,
    stepIndex: number,
    status: string,
    sessionId?: string | null
  ) =>
    invoke<void>("update_plan_step_status", {
      workspacePath,
      sessionId: sessionId ?? null,
      planId: planId ?? null,
      stepIndex,
      status,
    }),

  // 文件与变更查看器 (File Viewer)
  openFileViewer: (payload?: ViewerTabItem) =>
    invoke<void>("open_file_viewer", { payload: payload ?? null }),
  getFileViewerInitTab: () =>
    invoke<ViewerTabItem | null>("get_file_viewer_init_tab"),
  readTextFile: (path: string, maxBytes?: number) =>
    invoke<FileTextContent>("read_text_file", { path, maxBytes: maxBytes ?? null }),
  saveTextFile: (path: string, content: string, sessionId?: string | null) =>
    invoke<void>("save_text_file", { path, content, sessionId: sessionId ?? null }),
  getFileDiff: (path: string, oldContent?: string, newContent?: string, workspacePath?: string) =>
    invoke<TempFileDiff>("get_file_diff", {
      path,
      oldContent: oldContent ?? null,
      newContent: newContent ?? null,
      workspacePath: workspacePath ?? null,
    }),
  openInExternalEditor: (path: string, line?: number) =>
    invoke<void>("open_in_external_editor", { path, line: line ?? null }),
  getFileOutline: (path: string) =>
    invoke<FileOutlineItem[]>("get_file_outline", { path }),
  revertFileHunk: (path: string, hunk: DiffHunk) =>
    invoke<void>("revert_file_hunk", { path, hunk }),

  // 长任务（Long-Running Task）
  startLongTask: (sessionId: string, goal: string, maxBudgetTokens?: number | null) =>
    invoke<LongTask>("start_long_task", {
      sessionId,
      goal,
      maxBudgetTokens: maxBudgetTokens ?? null,
    }),
  pauseLongTask: (taskId: string) =>
    invoke<void>("pause_long_task", { taskId }),
  resumeLongTask: (taskId: string) =>
    invoke<LongTask>("resume_long_task", { taskId }),
  cancelLongTask: (taskId: string) =>
    invoke<void>("cancel_long_task", { taskId }),
  getActiveTask: (sessionId: string) =>
    invoke<LongTask | null>("get_active_task", { sessionId }),
  listTaskCheckpoints: (taskId: string) =>
    invoke<TaskCheckpoint[]>("list_task_checkpoints", { taskId }),
  rollbackToCheckpoint: (checkpointId: string) =>
    invoke<LongTask>("rollback_to_checkpoint", { checkpointId }),
  updateTaskSubtasks: (taskId: string, subtasks: TaskSubItem[]) =>
    invoke<LongTask>("update_task_subtasks", { taskId, subtasks }),
};

export type { ActivePlanDetail, PlanSummary, Message, Session, SessionActiveState, Project, ProjectLink, RunningSession, Settings, ToolEvent, DataStatus, TempAlloc, TempInfo, MergeSummary, GrowthItem, SkillItem, ProjectSopInfo, TokenStatsReport, CollaboratorCreateInput, CollaboratorUpdateInput, SessionCreateInput, SessionModelsUpdateInput, FileTextContent, ViewerTabItem, FileOutlineItem, DiffHunk, LongTask, TaskCheckpoint, TaskSubItem };




