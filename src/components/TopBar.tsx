import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { ipc } from "../ipc";
import { currentSession, useStore } from "../store";
import { DRAFT_ID, formatTokens, modelKey, parseModelKey, resolveActiveModel, resolveModelContextLimit, samePath, type Project } from "../types";
import { askConfirm } from "./PromptModal";
import { ModelContextModal } from "./ModelContextModal";

import {
  Wind,
  Lock,
  FolderOpen,
  ChevronDown,
  X,
  Pin,
  Check,
  Ban,
  MessageSquare,
  ShieldCheck,
  Zap,
  Sparkles,
  Cpu,
  Sliders,
  GitBranch,
} from "./Icons";

type OpenMenu = "ws" | "mode" | "model" | null;

export function TopBar() {
  const session = useStore((s) => currentSession(s));
  const currentId = useStore((s) => s.currentId);
  const selectSession = useStore((s) => s.selectSession);
  const parentSession = useStore((s) =>
    session?.forkedFromSessionId ? s.sessions.find((sess) => sess.id === session.forkedFromSessionId) ?? null : null
  );
  const isForked = Boolean(session?.forkedFromSessionId);
  const isParentDeleted = isForked && !parentSession;
  const draft = useStore((s) => s.draft);
  const settings = useStore((s) => s.settings);
  const projects = useStore((s) => s.projects);
  const runStatus = useStore((s) => s.runStatus);
  const setDraftWorkspace = useStore((s) => s.setDraftWorkspace);
  const newDraft = useStore((s) => s.newDraft);
  const setSettingsLocal = useStore((s) => s.setSettingsLocal);
  const pushToast = useStore((s) => s.pushToast);
  const setDraftAccessMode = useStore((s) => s.setDraftAccessMode);
  const setDraftContextTokenLimit = useStore((s) => s.setDraftContextTokenLimit);
  const openModelMatrixModal = useStore((s) => s.openModelMatrixModal);

  const [openMenu, setOpenMenu] = useState<OpenMenu>(null);
  const [openSessionContextModal, setOpenSessionContextModal] = useState(false);

  const running = currentId ? runStatus[currentId] === "running" : false;
  const workspacePath = session?.workspacePath ?? draft?.workspacePath ?? "";
  // 临时空间对话（含未落库草稿）：工作区为临时副本，固定不可切换
  const isTempConv = !!session?.isTemp || (currentId === DRAFT_ID && !!draft?.temp);
  // 顶栏模型选择：按厂商分组（optgroup）；激活厂商失效时回落到第一个有模型的厂商
  const groups = settings.providers.filter((p) => (p.models ?? []).length > 0);
  const active = resolveActiveModel(settings);

  // 上下文上限三级解析：会话专属设置 -> 模型独立配置/推断 -> 全局保底值
  const sessionLimitOverride = session
    ? session.contextTokenLimit ?? null
    : (currentId === DRAFT_ID ? draft?.contextTokenLimit ?? null : null);
  const isSessionOverridden = sessionLimitOverride != null;
  const modelDefaultLimit = active ? resolveModelContextLimit(settings, active.provider.id, active.model) : (settings.contextTokenLimit || 64_000);
  const effectiveLimit = sessionLimitOverride ?? modelDefaultLimit;
  const activeLimit = effectiveLimit;

  // 访问模式为会话级：已保存对话取自身取值，未保存草稿取草稿上的取值（新建时已从
  // “上一条对话”继承）。不再有“跟随全局”状态，全局值仅作为新建对话的默认值。
  const accessMode: "confirm" | "full_access" = session
    ? ((session.accessMode ?? settings.globalAccessMode) as "confirm" | "full_access")
    : draft?.accessMode ?? settings.globalAccessMode;

  // 已保存的对话工作空间只读：工作区在首次发送转正时固定，仅未保存草稿可选择/切换；
  // 临时空间对话的工作区为临时副本，同样只读
  const isSaved = !!session;

  // 展示用：工作区对应的项目（工作区即项目），或仅绑定了无目录项目
  const boundProject = projects.find((p) => samePath(p.path, workspacePath));
  const boundProjectId = session?.projectId ?? draft?.projectId ?? null;
  const boundProjectById = projects.find((p) => p.id === boundProjectId);
  const pathlessProject = !workspacePath ? projects.find((p) => p.id === boundProjectId && !p.path) : undefined;
  const wsLabel = isTempConv
    ? `${boundProjectById?.name ?? "项目"}（临时空间）`
    : workspacePath
    ? boundProject?.name ?? workspacePath
    : pathlessProject
    ? `${pathlessProject.name}（未绑定目录）`
    : "";
  const wsTitle = isTempConv
    ? `临时空间对话：工作区为「${boundProjectById?.name ?? "项目"}」及其关联项目的临时副本\n原目录 ${draft?.temp?.sourceWorkspace ?? session?.sourceWorkspace ?? ""} 不会被修改`
    : isSaved
    ? `${workspacePath ? (boundProject ? `${boundProject.name}\n${boundProject.path}` : workspacePath) : "未绑定工作区"}\n对话已保存，工作空间为只读，不允许切换`
    : workspacePath
    ? boundProject
      ? `${boundProject.name}\n${boundProject.path}`
      : workspacePath
    : pathlessProject
    ? "该项目未绑定目录，仅用于对话分组"
    : "点击选择工作区：已有项目 / 目录，或不选择（纯对话）";

  const sortedProjects = [...projects].sort((a, b) => {
    const aPin = Boolean(a.pinned);
    const bPin = Boolean(b.pinned);
    if (aPin !== bPin) return aPin ? -1 : 1;
    const aTime = a.lastActivityAt || a.createdAt || "";
    const bTime = b.lastActivityAt || b.createdAt || "";
    return bTime.localeCompare(aTime);
  });

  const applyProject = (p: Project) => {
    setOpenMenu(null);
    if (currentId !== DRAFT_ID) return;
    // 有目录的项目同时绑定工作区；无目录项目仅设置归属
    setDraftWorkspace(p.path ?? null, p.id);
  };

  const pickDirectory = async () => {
    setOpenMenu(null);
    if (currentId !== DRAFT_ID) return;
    try {
      const picked = await open({ directory: true, multiple: false, title: "选择工作区目录" });
      if (typeof picked !== "string") return;
      setDraftWorkspace(picked, null);
    } catch (err) {
      pushToast(`打开目录选择失败: ${String(err)}`);
    }
  };

  const clearWorkspace = () => {
    setOpenMenu(null);
    if (currentId !== DRAFT_ID) return;
    if (draft?.temp) {
      // 临时空间草稿：丢弃整个临时空间计划，回到该项目的普通空对话
      newDraft(draft.projectId);
      return;
    }
    setDraftWorkspace(null, null);
  };

  const selectModeLabel = (m: "confirm" | "full_access") =>
    m === "full_access" ? "完全访问" : "变更前确认";

  const handleSelectMode = async (mode: "confirm" | "full_access") => {
    setOpenMenu(null);
    if (mode === accessMode) return;
    if (mode === "full_access") {
      const confirmed = await askConfirm(
        "完全访问模式将跳过所有审批（包括命令执行与高危操作），确定开启？",
        "开启"
      );
      if (!confirmed) return;
    }
    if (session) await ipc.setSessionMode(session.id, mode);
    else setDraftAccessMode(mode);
  };

  const handleSelectModel = async (providerId: string, model: string) => {
    setOpenMenu(null);
    const next = {
      ...settings,
      activeProviderId: providerId,
      activeModelId: model,
      activeModel: model,
    };
    setSettingsLocal(next);
    try {
      await ipc.setSettings(next);
    } catch (err) {
      pushToast(String(err));
    }
  };

  const handleSaveSessionLimit = async (newLimit: number | null) => {
    if (session) {
      try {
        await ipc.setSessionContextLimit(session.id, newLimit);
      } catch (err) {
        pushToast(String(err));
      }
    } else if (currentId === DRAFT_ID) {
      setDraftContextTokenLimit(newLimit);
    }
  };

  const handleParentJump = () => {
    if (!session?.forkedFromSessionId) return;
    if (isParentDeleted) {
      pushToast("来源原对话已被删除，无法跳转");
      return;
    }
    void selectSession(session.forkedFromSessionId);
  };

  return (
    <div className="relative z-30 h-12 shrink-0 border-b border-edge/60 bg-panel flex items-center justify-between gap-2 px-3.5 select-none">
      {/* 左侧：会话标题 + 工作区 + 分支血缘指示 面包屑导航 */}
      <div className="flex items-center gap-1.5 min-w-0 shrink">
        {/* 会话标题 */}
        <div className="flex items-center gap-1.5 text-ink min-w-0 shrink h-8">
          <MessageSquare size={13} className="text-accent/80 shrink-0" />
          <span
            className="font-medium text-[12.5px] truncate max-w-[85px] sm:max-w-[130px]"
            title={session?.title ?? (currentId === DRAFT_ID ? "新对话（未保存）" : "harness_mini")}
          >
            {session?.title ?? (currentId === DRAFT_ID ? "新对话" : "harness_mini")}
          </span>
          {currentId === DRAFT_ID && (
            <span className="shrink-0 px-1.5 py-0.5 rounded text-[10px] bg-accent/15 text-accent font-normal border border-accent/25">
              草稿
            </span>
          )}
        </div>

        <span className="text-inkdim/30 select-none text-[11px] shrink-0">/</span>

        {/* 工作区（工作区即项目：未保存草稿可选已有项目 / 目录 / 不选择；已保存与临时空间对话只读） */}
        <div className="relative flex items-center gap-1 min-w-0 shrink">
          {isSaved || isTempConv ? (
            <div
              className="flex items-center gap-1.5 h-8 bg-panel2/50 hover:bg-panel2/80 border border-edge/50 hover:border-edge rounded-lg px-2.5 text-[12px] text-inkdim max-w-[90px] sm:max-w-[140px] cursor-pointer select-none shrink min-w-0 transition-colors"
              title={wsTitle}
              onClick={() => {
                if (isTempConv) {
                  pushToast("临时空间对话的工作区为项目临时副本，固定不可切换");
                } else {
                  pushToast("当前对话已保存，工作空间为只读；如需切换工作区请新建对话");
                }
              }}
            >
              {isTempConv ? (
                <Wind size={12} className="text-amber-400 shrink-0" />
              ) : (
                <Lock size={12} className="text-zinc-400 shrink-0" />
              )}
              <span className="truncate min-w-0 text-left">{wsLabel || "无工作区"}</span>
            </div>
          ) : (
            <button
              className={`flex items-center gap-1.5 h-8 bg-panel2/60 hover:bg-panel2 border border-edge/60 hover:border-accent/40 rounded-lg px-2.5 text-[12px] text-inkdim hover:text-ink max-w-[90px] sm:max-w-[140px] transition-all shrink min-w-0 cursor-pointer ${
                openMenu === "ws" ? "border-accent/50 bg-panel2 ring-1 ring-accent/20" : ""
              }`}
              onClick={() => setOpenMenu(openMenu === "ws" ? null : "ws")}
              title={wsTitle}
            >
              <FolderOpen size={12} className="text-accent shrink-0" />
              <span className="truncate min-w-0 text-left">{wsLabel || "选择工作区"}</span>
              <ChevronDown size={10} className="opacity-50 shrink-0 ml-0.5" />
            </button>
          )}

          {workspacePath && !isSaved && !isTempConv && (
            <button
              className="w-5 h-5 shrink-0 rounded text-inkdim hover:text-ink hover:bg-panel2 flex items-center justify-center transition-colors cursor-pointer"
              title="清除工作区（转为纯对话）"
              onClick={() => void clearWorkspace()}
            >
              <X size={11} />
            </button>
          )}

          {/* 工作区下拉弹出层：精简收窄至 220px，单行展示 */}
          {openMenu === "ws" && !isSaved && !isTempConv && (
            <>
              <div className="fixed inset-0 z-40" onMouseDown={() => setOpenMenu(null)} />
              <div className="absolute left-0 top-10 z-50 w-[220px] bg-panel2 border border-edge/80 rounded-xl shadow-2xl p-1.5 max-h-[60vh] overflow-y-auto animate-in fade-in zoom-in-95 duration-100">
                <div className="px-2 py-1 text-[10.5px] font-medium text-inkdim">选择已有项目</div>
                {sortedProjects.length === 0 && (
                  <div className="px-2 py-1.5 text-[11.5px] text-inkdim">暂无项目 · 可选下方目录</div>
                )}
                <div className="flex flex-col gap-0.5">
                  {sortedProjects.map((p) => {
                    const isActive = samePath(p.path, workspacePath) || (!workspacePath && p.id === boundProjectId);
                    return (
                      <button
                        key={p.id}
                        className={`w-full text-left px-2.5 py-1.5 rounded-lg text-[12px] hover:bg-panel3 transition-colors flex items-center gap-1.5 cursor-pointer ${
                          isActive ? "bg-accent/15 text-accent font-medium" : "text-ink/90 hover:text-ink"
                        }`}
                        onClick={() => void applyProject(p)}
                        title={`${p.name}\n${p.path ?? "未绑定目录"}`}
                      >
                        {p.pinned ? (
                          <Pin size={12} className="text-accent fill-accent shrink-0" />
                        ) : (
                          <FolderOpen size={12} className="text-inkdim shrink-0" />
                        )}
                        <span className="truncate flex-1 min-w-0">{p.name}</span>
                        {isActive && <Check size={12} className="text-accent shrink-0 ml-1" />}
                      </button>
                    );
                  })}
                </div>
                <div className="my-1 border-t border-edge/50" />
                <div className="flex flex-col gap-0.5">
                  <button
                    className="w-full text-left px-2.5 py-1.5 text-[12px] text-ink hover:bg-panel3 rounded-lg flex items-center gap-2 transition-colors cursor-pointer"
                    onClick={() => void pickDirectory()}
                    title="从本地文件系统选择目录并自动创建项目"
                  >
                    <FolderOpen size={13} className="text-accent shrink-0" />
                    <span className="truncate">选择新目录…</span>
                  </button>
                  <button
                    className="w-full text-left px-2.5 py-1.5 text-[12px] text-inkdim hover:text-ink hover:bg-panel3 rounded-lg flex items-center gap-2 transition-colors cursor-pointer"
                    onClick={() => void clearWorkspace()}
                    title="不绑定目录，进行纯文本对话"
                  >
                    <Ban size={13} className="shrink-0" />
                    <span className="truncate">不绑定工作区</span>
                  </button>
                </div>
              </div>
            </>
          )}
        </div>

        {/* 分支血缘指示标签：紧凑化收起 */}
        {isForked && (
          <>
            <span className="text-inkdim/30 select-none text-[11px] shrink-0">/</span>
            <button
              type="button"
              onClick={handleParentJump}
              className={`flex items-center gap-1 h-8 px-2 rounded-lg border text-[11.5px] transition-all select-none max-w-[75px] sm:max-w-[110px] shrink min-w-0 ${
                isParentDeleted
                  ? "bg-panel2/40 border-edge/60 text-inkdim/60 cursor-not-allowed opacity-75"
                  : "bg-accent/10 border-accent/25 text-accent hover:bg-accent/20 cursor-pointer"
              }`}
              title={
                isParentDeleted
                  ? "来源原对话已被删除，无法跳转"
                  : `分支自：${parentSession?.title || "原对话"}\n点击跳转回原对话`
              }
            >
              <GitBranch size={12} className={`shrink-0 ${isParentDeleted ? "text-inkdim/60" : "text-accent"}`} />
              <span className="truncate min-w-0 text-left">
                {isParentDeleted ? "已删除" : parentSession?.title || "原对话"}
              </span>
            </button>
          </>
        )}
      </div>

      {/* 右侧：访问模式 + 模型选择 + 状态指示 */}
      <div className="flex items-center gap-1.5 shrink-0 ml-auto select-none">
        {/* 访问模式选择器（统一 h-8 高度，下拉收窄至 170px） */}
        <div className="relative shrink-0">
          <button
            className={`flex items-center gap-1.5 h-8 bg-panel2/60 hover:bg-panel2 border border-edge/60 hover:border-accent/40 rounded-lg px-2.5 text-[12px] shrink-0 transition-all cursor-pointer ${
              accessMode === "full_access" ? "text-amber-400 font-medium" : "text-ink"
            } ${openMenu === "mode" ? "border-accent/50 bg-panel2 ring-1 ring-accent/20" : ""}`}
            onClick={() => setOpenMenu(openMenu === "mode" ? null : "mode")}
            title={accessMode === "full_access" ? "访问模式：完全访问（自动放行）" : "访问模式：变更前确认（操作需审批）"}
          >
            {accessMode === "full_access" ? (
              <Zap size={13} className="text-amber-400 shrink-0" />
            ) : (
              <ShieldCheck size={13} className="text-accent shrink-0" />
            )}
            <span>{accessMode === "full_access" ? "完全" : "确认"}</span>
            <ChevronDown size={10} className="opacity-50 shrink-0 ml-0.5 text-inkdim" />
          </button>

          {openMenu === "mode" && (
            <>
              <div className="fixed inset-0 z-40" onMouseDown={() => setOpenMenu(null)} />
              <div className="absolute right-0 top-10 z-50 w-[170px] bg-panel2 border border-edge/80 rounded-xl shadow-2xl p-1 animate-in fade-in zoom-in-95 duration-100">
                <div className="px-2 py-1 text-[10.5px] font-medium text-inkdim">选择访问模式</div>

                <div className="flex flex-col gap-0.5">
                  <button
                    className={`w-full text-left px-2.5 py-1.5 rounded-lg text-[12px] transition-colors flex items-center justify-between cursor-pointer ${
                      accessMode === "confirm" ? "bg-panel3 text-ink font-medium" : "hover:bg-panel3/70 text-inkdim hover:text-ink"
                    }`}
                    onClick={() => void handleSelectMode("confirm")}
                    title="变更前确认：只读工具自动执行，写入与命令执行需逐项审批"
                  >
                    <div className="flex items-center gap-2 min-w-0 truncate">
                      <ShieldCheck size={13} className="text-accent shrink-0" />
                      <span className="truncate">变更前确认</span>
                    </div>
                    {accessMode === "confirm" && <Check size={13} className="text-accent shrink-0 ml-1" />}
                  </button>

                  <button
                    className={`w-full text-left px-2.5 py-1.5 rounded-lg text-[12px] transition-colors flex items-center justify-between cursor-pointer ${
                      accessMode === "full_access" ? "bg-panel3 text-amber-400 font-medium" : "hover:bg-panel3/70 text-inkdim hover:text-ink"
                    }`}
                    onClick={() => void handleSelectMode("full_access")}
                    title="完全访问：所有工具自动放行，连续执行不被打断"
                  >
                    <div className="flex items-center gap-2 min-w-0 truncate">
                      <Zap size={13} className="text-amber-400 shrink-0" />
                      <span className="truncate">完全访问</span>
                    </div>
                    {accessMode === "full_access" && <Check size={13} className="text-amber-400 shrink-0 ml-1" />}
                  </button>
                </div>
              </div>
            </>
          )}
        </div>

        {/* 模型切换器（统一 h-8 高度，下拉收窄至 230px） */}
        <div className="relative shrink-0">
          <button
            className={`flex items-center gap-1.5 h-8 bg-panel2/60 hover:bg-panel2 border border-edge/60 hover:border-accent/40 rounded-lg px-2.5 text-[12px] text-ink shrink-0 transition-all cursor-pointer max-w-[120px] sm:max-w-[160px] ${
              openMenu === "model" ? "border-accent/50 bg-panel2 ring-1 ring-accent/20" : ""
            }`}
            onClick={() => setOpenMenu(openMenu === "model" ? null : "model")}
            title={
              active
                ? `${active.provider.name} / ${active.model}\n有效上下文上限: ${effectiveLimit.toLocaleString()} tokens (~${formatTokens(effectiveLimit)})${
                    isSessionOverridden ? " (★ 仅当前对话生效)" : " (跟随模型默认)"
                  }`
                : "未配置模型"
            }
          >
            <Sparkles size={13} className="text-accent shrink-0" />
            <span className="truncate min-w-0 flex-1 text-left">{active?.model ?? "选择模型"}</span>
            {isSessionOverridden && (
              <span
                className="w-1.5 h-1.5 rounded-full bg-amber-400 shrink-0"
                title="已自定义本次对话上下文上限"
              />
            )}
            <ChevronDown size={10} className="opacity-50 shrink-0 ml-0.5 text-inkdim" />
          </button>

          {openMenu === "model" && (
            <>
              <div className="fixed inset-0 z-40" onMouseDown={() => setOpenMenu(null)} />
              <div className="absolute right-0 top-10 z-50 w-[230px] bg-panel2 border border-edge/80 rounded-xl shadow-2xl p-1.5 max-h-[65vh] overflow-y-auto animate-in fade-in zoom-in-95 duration-100">
                <div className="p-0.5 mb-1 border-b border-edge/50 flex items-center gap-1">
                  <button
                    className="flex-1 flex items-center justify-between px-2 py-1 rounded-md bg-accent/10 hover:bg-accent/20 border border-accent/25 text-accent text-[11px] font-medium transition-colors cursor-pointer"
                    onClick={() => {
                      setOpenMenu(null);
                      openModelMatrixModal(currentId);
                    }}
                    title="配置主模型、图像模型、视觉模型及上下文上限"
                  >
                    <span className="flex items-center gap-1">
                      <Sliders size={11} />
                      <span>能力模型</span>
                    </span>
                    <span className="text-[10px] opacity-80">&rarr;</span>
                  </button>
                  {active && (
                    <button
                      className={`px-1.5 py-1 rounded-md border text-[10.5px] font-mono transition-colors cursor-pointer shrink-0 ${
                        isSessionOverridden
                          ? "text-amber-400 bg-amber-500/15 border-amber-500/35 hover:bg-amber-500/25 font-semibold"
                          : "text-inkdim hover:text-ink bg-panel3 hover:bg-panel border-edge"
                      }`}
                      title={`本次有效上限: ${effectiveLimit.toLocaleString()} tokens (~${formatTokens(effectiveLimit)})${
                        isSessionOverridden ? "\n（★ 本次对话专属自定义）" : "\n（跟随模型默认配置）"
                      }\n点击调整本次对话专属上限`}
                      onClick={() => {
                        setOpenMenu(null);
                        setOpenSessionContextModal(true);
                      }}
                    >
                      <span>{formatTokens(effectiveLimit)}</span>
                    </button>
                  )}
                </div>
                <div className="px-2 py-0.5 text-[10.5px] font-medium text-inkdim flex items-center justify-between">
                  <span>选择对话模型</span>
                  <span className="text-[9.5px] text-inkdim/70">
                    共 {groups.reduce((acc, g) => acc + g.models.length, 0)} 个
                  </span>
                </div>
                {groups.length === 0 ? (
                  <div className="px-3 py-3 text-[11.5px] text-inkdim text-center">暂未配置可用模型</div>
                ) : (
                  groups.map((p, idx) => (
                    <div key={p.id} className={idx > 0 ? "mt-1.5 pt-1 border-t border-edge/40" : "mt-0.5"}>
                      <div className="px-2 py-0.5 text-[9.5px] font-semibold text-inkdim/80 tracking-wider uppercase">
                        {p.name}
                      </div>
                      <div className="flex flex-col gap-0.5 mt-0.5">
                        {p.models.map((m) => {
                          const isSelected = active?.provider.id === p.id && active?.model === m;
                          const mLimit = resolveModelContextLimit(settings, p.id, m);
                          return (
                            <button
                              key={m}
                              className={`w-full text-left px-2 py-1 rounded-md text-[11.5px] transition-colors flex items-center justify-between gap-1.5 cursor-pointer ${
                                isSelected
                                  ? "bg-accent/15 text-accent font-medium"
                                  : "hover:bg-panel3 text-ink/90 hover:text-ink"
                              }`}
                              onClick={() => void handleSelectModel(p.id, m)}
                              title={`${p.name} · ${m}\n上下文上限: ${mLimit.toLocaleString()} tokens`}
                            >
                              <div className="flex items-center gap-1.5 truncate min-w-0 flex-1">
                                <Cpu size={12} className={isSelected ? "text-accent" : "text-inkdim"} />
                                <span className="truncate">{m}</span>
                              </div>
                              <div className="flex items-center gap-1 shrink-0">
                                <span className="text-[9.5px] font-mono text-inkdim bg-panel3 px-1 py-0.2 rounded border border-edge/60">
                                  {formatTokens(mLimit)}
                                </span>
                                {isSelected && <Check size={12} className="text-accent shrink-0" />}
                              </div>
                            </button>
                          );
                        })}
                      </div>
                    </div>
                  ))
                )}
              </div>
            </>
          )}
        </div>

        {/* 状态指示（统一 h-8 规格） */}
        <div
          className={`flex items-center gap-1.5 h-8 px-2.5 rounded-lg text-[12px] shrink-0 select-none transition-all ${
            running
              ? "bg-emerald-500/10 text-emerald-400 font-medium border border-emerald-500/20"
              : "text-inkdim/60 hover:text-inkdim"
          }`}
          title={running ? "Agent 正在执行任务中" : "当前空闲待命"}
        >
          <span
            className={`w-1.5 h-1.5 rounded-full shrink-0 ${
              running ? "bg-emerald-400 animate-pulse shadow-[0_0_6px_rgba(52,211,153,0.8)]" : "bg-inkdim/40"
            }`}
          />
          <span>{running ? "运行中" : "空闲"}</span>
        </div>
      </div>

      {/* 调整本次对话上下文上限小弹窗（仅当前对话生效） */}
      {openSessionContextModal && active && (
        <ModelContextModal
          open={openSessionContextModal}
          providerId={active.provider.id}
          providerName={active.provider.name}
          modelName={active.model}
          currentLimit={effectiveLimit}
          isSessionScope={true}
          onSave={handleSaveSessionLimit}
          onClose={() => setOpenSessionContextModal(false)}
        />
      )}
    </div>
  );
}

