import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { ipc } from "../ipc";
import { currentSession, useStore } from "../store";
import { DRAFT_ID, modelKey, parseModelKey, resolveActiveModel, samePath, type Project } from "../types";
import { askConfirm } from "./PromptModal";

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
    if (a.pinned !== b.pinned) return a.pinned ? -1 : 1;
    return (b.lastActivityAt ?? b.createdAt).localeCompare(a.lastActivityAt ?? a.createdAt);
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
    <div className="h-12 shrink-0 border-b border-edge bg-panel flex items-center gap-3 px-4">
      {/* 会话标题 */}
      <div className="font-medium text-[14px] max-w-[200px] truncate">
        {session?.title ?? (currentId === DRAFT_ID ? "新对话（未保存）" : "harness_mini")}
      </div>

      {/* 工作区（工作区即项目：未保存草稿可选已有项目 / 目录 / 不选择；已保存与临时空间对话只读） */}
      <div className="relative flex items-center gap-1">
        {isSaved || isTempConv ? (
          <div
            className="flex items-center gap-1.5 bg-panel2 border border-edge rounded-lg px-2.5 py-1.5 text-[12px] text-inkdim max-w-[280px] cursor-default"
            title={wsTitle}
          >
            <span>{isTempConv ? "🌪" : "🔒"}</span>
            <span className="truncate">{wsLabel || "未选择工作区"}</span>
          </div>
        ) : (
          <button
            className="flex items-center gap-1.5 bg-panel2 border border-edge rounded-lg px-2.5 py-1.5 text-[12px] text-inkdim hover:text-ink max-w-[280px]"
            onClick={() => setWsMenu(!wsMenu)}
            title={wsTitle}
          >
            <span>📁</span>
            <span className="truncate">{wsLabel || "未选择工作区（可纯对话）"}</span>
            <span className="text-[10px] opacity-70">▾</span>
          </button>
        )}
        {workspacePath && !isSaved && !isTempConv && (
          <button
            className="w-5 h-5 rounded text-[11px] text-inkdim hover:text-ink hover:bg-panel2 flex items-center justify-center"
            title="清除工作区（转为纯对话）"
            onClick={() => void clearWorkspace()}
          >
            ✕
          </button>
        )}
        {/* 打开工作区目录：已移至侧栏项目下拉「打开目录」；临时空间对话用输入框左侧的「临时目录」按钮 */}
        {wsMenu && !isSaved && !isTempConv && (
          <>
            <div className="fixed inset-0 z-40" onMouseDown={() => setWsMenu(false)} />
            <div className="absolute left-0 top-10 z-50 w-[320px] bg-panel2 border border-edge rounded-lg shadow-xl py-1 max-h-[60vh] overflow-y-auto">
              <div className="px-3 py-1.5 text-[11px] text-inkdim">选择已有项目</div>
              {sortedProjects.length === 0 && (
                <div className="px-3 py-1.5 text-[12px] text-inkdim">暂无项目 · 选择目录后自动创建</div>
              )}
              {sortedProjects.map((p) => {
                const isActive = samePath(p.path, workspacePath) || (!workspacePath && p.id === boundProjectId);
                return (
                  <button
                    key={p.id}
                    className="w-full text-left px-3 py-1.5 hover:bg-panel3"
                    onClick={() => void applyProject(p)}
                  >
                    <div className="text-[13px] text-ink truncate">
                      {p.pinned && <span className="text-accent mr-1">📌</span>}
                      <span className={isActive ? "text-accent" : ""}>{p.name}</span>
                      {isActive && <span className="text-accent ml-1.5 text-[11px]">✓ 当前</span>}
                    </div>
                    <div className="text-[11px] text-inkdim truncate">{p.path ?? "未绑定目录"}</div>
                  </button>
                );
              })}
              <div className="my-1 border-t border-edge" />
              <button
                className="w-full text-left px-3 py-1.5 text-[13px] text-ink hover:bg-panel3"
                onClick={() => void pickDirectory()}
              >
                📂 选择目录…
                <span className="text-[11px] text-inkdim ml-1.5">保存对话时将自动创建对应项目</span>
              </button>
              <button
                className="w-full text-left px-3 py-1.5 text-[13px] text-ink hover:bg-panel3"
                onClick={() => void clearWorkspace()}
              >
                🚫 不选择工作区（纯对话）
              </button>
            </div>
          </>
        )}
      </div>

      <div className="ml-auto flex items-center gap-2">
        {/* 访问模式 */}
        <select
          className={`bg-panel2 border border-edge rounded-lg px-2 py-1.5 text-[12px] outline-none cursor-pointer ${
            accessMode === "full_access" ? "text-amber-400" : "text-ink"
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
            // 访问模式仅作用于当前对话：已保存对话落库（即时生效），未保存草稿先记在草稿上，
            // 首次发送时随会话一起落库（不影响其他对话）
            if (session) await ipc.setSessionMode(session.id, v);
            else setDraftAccessMode(v);
          }}
          title="访问模式"
        >
          <option value="confirm">{selectModeLabel("confirm")}</option>
          <option value="full_access">{selectModeLabel("full_access")}</option>
        </select>

        {/* 模型（按厂商分组） */}
        <select
          className="bg-panel2 border border-edge rounded-lg px-2 py-1.5 text-[12px] outline-none cursor-pointer max-w-[220px]"
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

        {/* 状态指示 */}
        <span
          className={`w-2.5 h-2.5 rounded-full ${running ? "bg-green-400 animate-pulse" : "bg-edge"}`}
          title={running ? "Agent 运行中" : "空闲"}
        />
      </div>
    </div>
  );
}
