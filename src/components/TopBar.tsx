import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { ipc } from "../ipc";
import { currentSession, useStore } from "../store";
import { DRAFT_ID, modelKey, parseModelKey, resolveActiveModel, samePath, type Project } from "../types";
import { askConfirm } from "./PromptModal";

import {
  Wind,
  Lock,
  Folder,
  FolderOpen,
  ChevronDown,
  X,
  Pin,
  Check,
  Ban,
} from "./Icons";

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
  const [wsMenu, setWsMenu] = useState(false);

  const running = currentId ? runStatus[currentId] === "running" : false;
  const workspacePath = session?.workspacePath ?? draft?.workspacePath ?? "";
  // 临时空间对话（含未落库草稿）：工作区为临时副本，固定不可切换
  const isTempConv = !!session?.isTemp || (currentId === DRAFT_ID && !!draft?.temp);
  // 顶栏模型选择：按厂商分组（optgroup）；激活厂商失效时回落到第一个有模型的厂商
  const groups = settings.providers.filter((p) => (p.models ?? []).length > 0);
  const active = resolveActiveModel(settings);
  const selectValue = active ? modelKey(active.provider.id, active.model) : "";

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
    setWsMenu(false);
    if (currentId !== DRAFT_ID) return;
    // 有目录的项目同时绑定工作区；无目录项目仅设置归属
    setDraftWorkspace(p.path ?? null, p.id);
  };

  const pickDirectory = async () => {
    setWsMenu(false);
    if (currentId !== DRAFT_ID) return;
    const picked = await open({ directory: true, multiple: false, title: "选择工作区目录" });
    if (typeof picked !== "string") return;
    setDraftWorkspace(picked, null);
  };

  const clearWorkspace = () => {
    setWsMenu(false);
    if (currentId !== DRAFT_ID) return;
    if (draft?.temp) {
      // 临时空间草稿：丢弃整个临时空间计划，回到该项目的普通空对话
      newDraft(draft.projectId);
      return;
    }
    setDraftWorkspace(null, null);
  };

  const setDraftAccessMode = useStore((s) => s.setDraftAccessMode);

  const selectModeLabel = (m: "confirm" | "full_access") =>
    m === "full_access" ? "完全访问" : "变更前确认";

  return (
    <div className="h-12 shrink-0 border-b border-edge bg-panel flex items-center gap-3 px-4 select-none">
      {/* 会话标题 */}
      <div className="font-semibold text-[13px] max-w-[220px] truncate text-ink">
        {session?.title ?? (currentId === DRAFT_ID ? "新对话（未保存）" : "harness_mini")}
      </div>

      {/* 工作区（工作区即项目：未保存草稿可选已有项目 / 目录 / 不选择；已保存与临时空间对话只读） */}
      <div className="relative flex items-center gap-1">
        {isSaved || isTempConv ? (
          <div
            className="flex items-center gap-1.5 bg-panel2/80 border border-edge rounded-lg px-2.5 py-1.5 text-[12px] text-inkdim max-w-[280px] cursor-default shadow-sm"
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
            className="flex items-center gap-1.5 bg-panel2 border border-edge hover:border-accent/40 rounded-lg px-2.5 py-1.5 text-[12px] text-inkdim hover:text-ink max-w-[280px] transition-colors shadow-sm"
            onClick={() => setWsMenu(!wsMenu)}
            title={wsTitle}
          >
            <FolderOpen size={13} className="text-accent shrink-0" />
            <span className="truncate">{wsLabel || "未选择工作区（可纯对话）"}</span>
            <ChevronDown size={11} className="opacity-70 shrink-0 ml-0.5" />
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
        {/* 打开工作区目录：已移至侧栏项目下拉「打开目录」；临时空间对话用输入框左侧的「临时目录」按钮 */}
        {wsMenu && !isSaved && !isTempConv && (
          <>
            <div className="fixed inset-0 z-40" onMouseDown={() => setWsMenu(false)} />
            <div className="absolute left-0 top-10 z-50 w-[320px] bg-panel2 border border-edge rounded-xl shadow-2xl py-1.5 max-h-[60vh] overflow-y-auto animate-in fade-in zoom-in-95 duration-100">
              <div className="px-3 py-1.5 text-[11px] font-medium text-inkdim">选择已有项目</div>
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
              <div className="my-1.5 border-t border-edge" />
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

      <div className="ml-auto flex items-center gap-2">
        {/* 访问模式 */}
        <div className="relative flex items-center">
          <select
            className={`appearance-none bg-panel2 border border-edge hover:border-accent/40 rounded-lg pl-2.5 pr-7 py-1.5 text-[12px] outline-none cursor-pointer transition-colors ${
              accessMode === "full_access" ? "text-amber-400 font-medium" : "text-ink"
            }`}
            value={accessMode}
            onChange={async (e) => {
              const v = e.target.value as "confirm" | "full_access";
              if (v === "full_access") {
                const confirmed = await askConfirm(
                  "完全访问模式将跳过所有审批（包括命令执行与高危操作），确定开启？",
                  "开启"
                );
                if (!confirmed) return;
              }
              if (session) await ipc.setSessionMode(session.id, v);
              else setDraftAccessMode(v);
            }}
            title="访问模式"
          >
            <option value="confirm">{selectModeLabel("confirm")}</option>
            <option value="full_access">{selectModeLabel("full_access")}</option>
          </select>
          <ChevronDown size={12} className="absolute right-2 pointer-events-none opacity-60 text-inkdim" />
        </div>

        {/* 模型（按厂商分组） */}
        <div className="relative flex items-center">
          <select
            className="appearance-none bg-panel2 border border-edge hover:border-accent/40 rounded-lg pl-2.5 pr-7 py-1.5 text-[12px] text-ink outline-none cursor-pointer max-w-[220px] transition-colors"
            value={selectValue}
            onChange={async (e) => {
              const parsed = parseModelKey(e.target.value);
              if (!parsed) return;
              const next = {
                ...settings,
                activeProviderId: parsed.providerId,
                activeModelId: parsed.model,
                activeModel: parsed.model,
              };
              setSettingsLocal(next);
              try {
                await ipc.setSettings(next);
              } catch (err) {
                pushToast(String(err));
              }
            }}
            title="模型（按厂商分组）"
          >
            {groups.length === 0 && <option value="">未配置模型</option>}
            {groups.map((p) => (
              <optgroup key={p.id} label={p.name}>
                {p.models.map((m) => (
                  <option key={m} value={modelKey(p.id, m)}>
                    {m}
                  </option>
                ))}
              </optgroup>
            ))}
          </select>
          <ChevronDown size={12} className="absolute right-2 pointer-events-none opacity-60 text-inkdim" />
        </div>

        {/* 状态指示 */}
        <div
          className={`flex items-center gap-1.5 px-2.5 py-1 rounded-full border text-[11px] select-none transition-colors ${
            running
              ? "bg-green-500/10 border-green-500/30 text-green-400"
              : "bg-panel2 border-edge text-inkdim"
          }`}
          title={running ? "Agent 正在执行任务中" : "当前空闲"}
        >
          <span
            className={`w-1.5 h-1.5 rounded-full ${
              running ? "bg-green-400 animate-pulse shadow-[0_0_8px_rgba(74,222,128,0.8)]" : "bg-zinc-500"
            }`}
          />
          <span>{running ? "运行中" : "空闲"}</span>
        </div>
      </div>
    </div>
  );
}
