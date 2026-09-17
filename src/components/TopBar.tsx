import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { ipc } from "../ipc";
import { currentSession, useStore } from "../store";
import { DRAFT_ID, modelKey, parseModelKey, resolveActiveModel, samePath, type Project } from "../types";
import { askConfirm } from "./PromptModal";

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
} from "./Icons";

type OpenMenu = "ws" | "mode" | "model" | null;

export function TopBar() {
  const session = useStore((s) => currentSession(s));
  const currentId = useStore((s) => s.currentId);
  const draft = useStore((s) => s.draft);
  const settings = useStore((s) => s.settings);
  const projects = useStore((s) => s.projects);
  const runStatus = useStore((s) => s.runStatus);
  const setDraftWorkspace = useStore((s) => s.setDraftWorkspace);
  const newDraft = useStore((s) => s.newDraft);
  const setSettingsLocal = useStore((s) => s.setSettingsLocal);
  const pushToast = useStore((s) => s.pushToast);
  const setDraftAccessMode = useStore((s) => s.setDraftAccessMode);

  const [openMenu, setOpenMenu] = useState<OpenMenu>(null);

  const running = currentId ? runStatus[currentId] === "running" : false;
  const workspacePath = session?.workspacePath ?? draft?.workspacePath ?? "";
  // 临时空间对话（含未落库草稿）：工作区为临时副本，固定不可切换
  const isTempConv = !!session?.isTemp || (currentId === DRAFT_ID && !!draft?.temp);
  // 顶栏模型选择：按厂商分组（optgroup）；激活厂商失效时回落到第一个有模型的厂商
  const groups = settings.providers.filter((p) => (p.models ?? []).length > 0);
  const active = resolveActiveModel(settings);

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
    const picked = await open({ directory: true, multiple: false, title: "选择工作区目录" });
    if (typeof picked !== "string") return;
    setDraftWorkspace(picked, null);
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

  return (
    <div className="relative z-30 h-12 shrink-0 border-b border-edge/60 bg-panel flex items-center gap-2.5 px-3.5 select-none">
      {/* 左侧：会话标题 + 面包屑导航 */}
      <div className="flex items-center gap-2 min-w-0">
        <div className="flex items-center gap-1.5 text-ink min-w-0">
          <MessageSquare size={14} className="text-accent/80 shrink-0" />
          <span
            className="font-medium text-[13px] truncate max-w-[200px]"
            title={session?.title ?? (currentId === DRAFT_ID ? "新对话（未保存）" : "harness_mini")}
          >
            {session?.title ?? (currentId === DRAFT_ID ? "新对话" : "harness_mini")}
          </span>
          {currentId === DRAFT_ID && (
            <span className="shrink-0 px-1.5 py-[1px] rounded text-[10px] bg-accent/15 text-accent font-normal border border-accent/25">
              草稿
            </span>
          )}
        </div>

        <span className="text-inkdim/30 select-none text-[12px]">/</span>

        {/* 工作区（工作区即项目：未保存草稿可选已有项目 / 目录 / 不选择；已保存与临时空间对话只读） */}
        <div className="relative flex items-center gap-1 min-w-0">
          {isSaved || isTempConv ? (
            <div
              className="flex items-center gap-1.5 bg-panel2/50 border border-edge/50 rounded-lg px-2.5 py-1 text-[12px] text-inkdim max-w-[240px] cursor-default select-none shadow-sm"
              title={wsTitle}
            >
              {isTempConv ? (
                <Wind size={13} className="text-amber-400 shrink-0" />
              ) : (
                <Lock size={13} className="text-zinc-400 shrink-0" />
              )}
              <span className="truncate">{wsLabel || "未选择工作区"}</span>
            </div>
          ) : (
            <button
              className={`flex items-center gap-1.5 bg-panel2/60 hover:bg-panel2 border border-edge/60 hover:border-accent/40 rounded-lg px-2.5 py-1 text-[12px] text-inkdim hover:text-ink max-w-[240px] transition-all shadow-sm ${
                openMenu === "ws" ? "border-accent/50 bg-panel2 ring-1 ring-accent/20" : ""
              }`}
              onClick={() => setOpenMenu(openMenu === "ws" ? null : "ws")}
              title={wsTitle}
            >
              <FolderOpen size={13} className="text-accent shrink-0" />
              <span className="truncate">{wsLabel || "未选择工作区（可纯对话）"}</span>
              <ChevronDown size={11} className="opacity-60 shrink-0 ml-0.5" />
            </button>
          )}

          {workspacePath && !isSaved && !isTempConv && (
            <button
              className="w-5 h-5 rounded-md text-inkdim hover:text-ink hover:bg-panel2 flex items-center justify-center transition-colors"
              title="清除工作区（转为纯对话）"
              onClick={() => void clearWorkspace()}
            >
              <X size={12} />
            </button>
          )}

          {/* 工作区下拉弹出层 */}
          {openMenu === "ws" && !isSaved && !isTempConv && (
            <>
              <div className="fixed inset-0 z-40" onMouseDown={() => setOpenMenu(null)} />
              <div className="absolute left-0 top-10 z-50 w-[320px] bg-panel2 border border-edge/80 rounded-xl shadow-2xl py-1.5 max-h-[60vh] overflow-y-auto animate-in fade-in zoom-in-95 duration-100">
                <div className="px-3 py-1 text-[11px] font-medium text-inkdim">选择已有项目</div>
                {sortedProjects.length === 0 && (
                  <div className="px-3 py-2 text-[12px] text-inkdim">暂无项目 · 选择目录后自动创建</div>
                )}
                {sortedProjects.map((p) => {
                  const isActive = samePath(p.path, workspacePath) || (!workspacePath && p.id === boundProjectId);
                  return (
                    <button
                      key={p.id}
                      className="w-full text-left px-3 py-1.5 hover:bg-panel3 transition-colors flex flex-col gap-0.5"
                      onClick={() => void applyProject(p)}
                    >
                      <div className="text-[13px] text-ink truncate flex items-center gap-1.5">
                        {p.pinned && <Pin size={12} className="text-accent fill-accent shrink-0" />}
                        <span className={`truncate ${isActive ? "text-accent font-medium" : ""}`}>{p.name}</span>
                        {isActive && (
                          <span className="text-accent ml-auto text-[11px] flex items-center gap-0.5 shrink-0">
                            <Check size={12} /> 当前
                          </span>
                        )}
                      </div>
                      <div className="text-[11px] text-inkdim truncate">{p.path ?? "未绑定目录"}</div>
                    </button>
                  );
                })}
                <div className="my-1.5 border-t border-edge/50" />
                <button
                  className="w-full text-left px-3 py-1.5 text-[13px] text-ink hover:bg-panel3 flex items-center gap-2 transition-colors"
                  onClick={() => void pickDirectory()}
                >
                  <FolderOpen size={14} className="text-accent shrink-0" />
                  <span className="truncate">选择目录…</span>
                  <span className="text-[11px] text-inkdim ml-auto truncate">自动创建项目</span>
                </button>
                <button
                  className="w-full text-left px-3 py-1.5 text-[13px] text-ink hover:bg-panel3 flex items-center gap-2 transition-colors"
                  onClick={() => void clearWorkspace()}
                >
                  <Ban size={14} className="text-inkdim shrink-0" />
                  <span>不选择工作区（纯对话）</span>
                </button>
              </div>
            </>
          )}
        </div>
      </div>

      {/* 右侧：访问模式 + 模型选择 + 状态指示 */}
      <div className="ml-auto flex items-center gap-2">
        {/* 访问模式选择器（自定义深色下拉） */}
        <div className="relative">
          <button
            className={`flex items-center gap-1.5 bg-panel2/60 hover:bg-panel2 border border-edge/60 hover:border-accent/40 rounded-lg px-2.5 py-1 text-[12px] transition-all shadow-sm ${
              accessMode === "full_access" ? "text-amber-400 font-medium" : "text-ink"
            } ${openMenu === "mode" ? "border-accent/50 bg-panel2 ring-1 ring-accent/20" : ""}`}
            onClick={() => setOpenMenu(openMenu === "mode" ? null : "mode")}
            title={`当前访问模式：${selectModeLabel(accessMode)}`}
          >
            {accessMode === "full_access" ? (
              <Zap size={13} className="text-amber-400 shrink-0" />
            ) : (
              <ShieldCheck size={13} className="text-accent shrink-0" />
            )}
            <span>{selectModeLabel(accessMode)}</span>
            <ChevronDown size={11} className="opacity-60 shrink-0 ml-0.5 text-inkdim" />
          </button>

          {openMenu === "mode" && (
            <>
              <div className="fixed inset-0 z-40" onMouseDown={() => setOpenMenu(null)} />
              <div className="absolute right-0 top-10 z-50 w-[260px] bg-panel2 border border-edge/80 rounded-xl shadow-2xl p-1.5 animate-in fade-in zoom-in-95 duration-100">
                <div className="px-2.5 py-1 text-[11px] font-medium text-inkdim">选择访问模式</div>

                <button
                  className={`w-full text-left p-2 rounded-lg transition-colors flex items-start gap-2.5 ${
                    accessMode === "confirm" ? "bg-panel3 text-ink" : "hover:bg-panel3/70 text-ink/90"
                  }`}
                  onClick={() => void handleSelectMode("confirm")}
                >
                  <div className="p-1 rounded bg-blue-500/15 text-blue-400 mt-0.5 shrink-0">
                    <ShieldCheck size={14} />
                  </div>
                  <div className="flex-1 min-w-0">
                    <div className="text-[12px] font-medium flex items-center justify-between">
                      <span>变更前确认</span>
                      {accessMode === "confirm" && <Check size={13} className="text-accent" />}
                    </div>
                    <div className="text-[11px] text-inkdim leading-tight mt-0.5">
                      命令与写操作需审批，安全可控
                    </div>
                  </div>
                </button>

                <button
                  className={`w-full text-left p-2 rounded-lg transition-colors flex items-start gap-2.5 mt-1 ${
                    accessMode === "full_access" ? "bg-panel3 text-amber-400" : "hover:bg-panel3/70 text-ink/90"
                  }`}
                  onClick={() => void handleSelectMode("full_access")}
                >
                  <div className="p-1 rounded bg-amber-500/15 text-amber-400 mt-0.5 shrink-0">
                    <Zap size={14} />
                  </div>
                  <div className="flex-1 min-w-0">
                    <div className="text-[12px] font-medium flex items-center justify-between">
                      <span className={accessMode === "full_access" ? "text-amber-400 font-semibold" : ""}>
                        完全访问
                      </span>
                      {accessMode === "full_access" && <Check size={13} className="text-amber-400" />}
                    </div>
                    <div className="text-[11px] text-inkdim leading-tight mt-0.5">
                      所有工具自动放行，连续执行不被打断
                    </div>
                  </div>
                </button>
              </div>
            </>
          )}
        </div>

        {/* 模型选择器（自定义深色下拉） */}
        <div className="relative">
          <button
            className={`flex items-center gap-1.5 bg-panel2/60 hover:bg-panel2 border border-edge/60 hover:border-accent/40 rounded-lg px-2.5 py-1 text-[12px] text-ink transition-all shadow-sm max-w-[210px] ${
              openMenu === "model" ? "border-accent/50 bg-panel2 ring-1 ring-accent/20" : ""
            }`}
            onClick={() => setOpenMenu(openMenu === "model" ? null : "model")}
            title={active ? `${active.provider.name} / ${active.model}` : "未配置模型"}
          >
            <Sparkles size={13} className="text-accent shrink-0" />
            <span className="truncate">{active?.model ?? "选择模型"}</span>
            <ChevronDown size={11} className="opacity-60 shrink-0 ml-0.5 text-inkdim" />
          </button>

          {openMenu === "model" && (
            <>
              <div className="fixed inset-0 z-40" onMouseDown={() => setOpenMenu(null)} />
              <div className="absolute right-0 top-10 z-50 w-[280px] bg-panel2 border border-edge/80 rounded-xl shadow-2xl p-1.5 max-h-[65vh] overflow-y-auto animate-in fade-in zoom-in-95 duration-100">
                <div className="px-2.5 py-1 text-[11px] font-medium text-inkdim flex items-center justify-between">
                  <span>选择模型</span>
                  <span className="text-[10px] text-inkdim/70">
                    共 {groups.reduce((acc, g) => acc + g.models.length, 0)} 个
                  </span>
                </div>
                {groups.length === 0 ? (
                  <div className="px-3 py-3 text-[12px] text-inkdim text-center">暂未配置可用模型</div>
                ) : (
                  groups.map((p, idx) => (
                    <div key={p.id} className={idx > 0 ? "mt-2 pt-1.5 border-t border-edge/40" : "mt-1"}>
                      <div className="px-2 py-0.5 text-[10px] font-semibold text-inkdim/80 tracking-wider uppercase">
                        {p.name}
                      </div>
                      <div className="flex flex-col gap-0.5 mt-0.5">
                        {p.models.map((m) => {
                          const isSelected = active?.provider.id === p.id && active?.model === m;
                          return (
                            <button
                              key={m}
                              className={`w-full text-left px-2.5 py-1.5 rounded-lg text-[12px] transition-colors flex items-center justify-between ${
                                isSelected
                                  ? "bg-accent/15 text-accent font-medium"
                                  : "hover:bg-panel3 text-ink/90 hover:text-ink"
                              }`}
                              onClick={() => void handleSelectModel(p.id, m)}
                            >
                              <div className="flex items-center gap-2 truncate min-w-0">
                                <Cpu size={13} className={isSelected ? "text-accent" : "text-inkdim"} />
                                <span className="truncate">{m}</span>
                              </div>
                              {isSelected && <Check size={13} className="text-accent shrink-0 ml-1.5" />}
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

        {/* 状态指示 */}
        <div
          className={`flex items-center gap-1.5 px-2.5 py-1 rounded-full border text-[11px] select-none transition-all ${
            running
              ? "bg-emerald-500/10 border-emerald-500/30 text-emerald-400 shadow-[0_0_12px_rgba(16,185,129,0.15)]"
              : "bg-panel2/50 border-edge/50 text-inkdim"
          }`}
          title={running ? "Agent 正在执行任务中" : "当前空闲"}
        >
          <span
            className={`w-1.5 h-1.5 rounded-full ${
              running ? "bg-emerald-400 animate-pulse shadow-[0_0_6px_rgba(52,211,153,0.8)]" : "bg-zinc-500"
            }`}
          />
          <span className="font-medium">{running ? "运行中" : "空闲"}</span>
        </div>
      </div>
    </div>
  );
}

