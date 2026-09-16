import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { ipc } from "../ipc";
import { useStore } from "../store";
import { dirName, samePath, type Project, type ProjectLink } from "../types";
import { askConfirm } from "./PromptModal";
import { Markdown } from "./Markdown";
import { ModalActions, ModalClose } from "./ModalActions";
import { FileCode, Link2, Plus, Edit3, Trash2, Folder } from "./Icons";

type Tab = "constraints" | "links";

const inputCls = "bg-panel border border-edge rounded-lg px-2 py-1.5 text-[13px] outline-none focus:border-accent w-full";

/** 与后端 estimate_tokens 同口径的粗估：非 ASCII 按字计（×0.7）、ASCII 4 字符 1 token */
function estimateTokens(s: string): number {
  let cjk = 0;
  let other = 0;
  for (const ch of s) {
    if (ch.charCodeAt(0) <= 0x7f) other++;
    else cjk++;
  }
  return Math.floor(cjk * 0.7) + Math.floor(other / 4);
}

// ---------- 项目约束页（草稿由外壳统一保存） ----------

function ConstraintsPane({ value, onChange }: { value: string; onChange: (v: string) => void }) {
  const [mode, setMode] = useState<"edit" | "preview">(value.trim() ? "preview" : "edit");

  const toggleCls = (active: boolean) =>
    `px-3 py-1.5 rounded-lg text-[12px] ${active ? "bg-panel3 text-ink" : "text-inkdim hover:text-ink"}`;

  return (
    <div className="h-full min-h-0 flex flex-col">
      <div className="flex items-center gap-2 mb-2 shrink-0">
        <div className="font-medium">项目约束</div>
        <span className="text-[11px] text-inkdim truncate">
          该项目下每个新对话自动阅读；对话进行中修改，下一条消息生效
        </span>
        <div className="ml-auto flex gap-1 shrink-0">
          <button className={toggleCls(mode === "edit")} onClick={() => setMode("edit")}>
            编辑
          </button>
          <button className={toggleCls(mode === "preview")} onClick={() => setMode("preview")}>
            预览
          </button>
        </div>
      </div>

      {mode === "edit" ? (
        <>
          <textarea
            className="flex-1 min-h-0 bg-panel border border-edge rounded-xl p-3 text-[13px] font-mono leading-relaxed outline-none focus:border-accent resize-none"
            value={value}
            placeholder="使用 Markdown 撰写本项目的规范、注意事项、编码约定等…"
            onChange={(e) => onChange(e.target.value)}
          />
          <div className="shrink-0 mt-1.5 text-[11px] text-inkdim text-right">
            约 {estimateTokens(value)} tokens · {value.length} 字符（约束随 system prompt 常驻上下文，建议精炼）
          </div>
        </>
      ) : value.trim() ? (
        <div className="flex-1 min-h-0 overflow-y-auto bg-panel border border-edge rounded-xl p-3">
          <Markdown content={value} />
        </div>
      ) : (
        <div className="flex-1 flex items-center justify-center text-inkdim text-[13px]">
          尚未设置项目约束，切换到「编辑」撰写本项目的规范与注意事项
        </div>
      )}
    </div>
  );
}

// ---------- 关联项目页（行操作仅改草稿，「应用 / 保存」时统一提交） ----------

function LinksPane({
  project,
  links,
  setLinks,
  onOpenConstraints,
}: {
  project: Project;
  links: ProjectLink[];
  setLinks: (links: ProjectLink[]) => void;
  onOpenConstraints: () => void;
}) {
  const projects = useStore((s) => s.projects);
  const pushToast = useStore((s) => s.pushToast);
  // 行内表单：id 缺省 = 新增，否则为编辑该条
  const [form, setForm] = useState<{ id?: string; path: string; description: string } | null>(null);

  const startAdd = async () => {
    const picked = await open({ directory: true, multiple: false, title: "添加关联项目：选择目录" });
    if (typeof picked !== "string") return;
    if (project.path && samePath(project.path, picked)) return pushToast("不能关联项目自身的目录");
    if (links.some((l) => samePath(l.path, picked))) return pushToast("该目录已关联");
    setForm({ path: picked, description: "" });
  };

  const repick = async () => {
    if (!form) return;
    const picked = await open({ directory: true, multiple: false, title: "选择关联目录" });
    if (typeof picked !== "string") return;
    setForm({ ...form, path: picked });
  };

  // 将行内表单提交进草稿（新行用临时 id 标记，落库由「应用 / 保存」完成）
  const commitForm = () => {
    if (!form) return;
    if (form.id) {
      setLinks(links.map((l) => (l.id === form.id ? { ...l, path: form.path, description: form.description } : l)));
    } else {
      setLinks([...links, { id: `new-${Date.now()}`, projectId: project.id, path: form.path, description: form.description, createdAt: "" }]);
    }
    setForm(null);
  };

  const remove = async (l: ProjectLink) => {
    if (!(await askConfirm(`移除对「${dirName(l.path)}」的关联？点击「应用」或「保存」后生效。`, "移除"))) return;
    setLinks(links.filter((x) => x.id !== l.id));
  };

  // 关联目录若对应本工具中的项目且已设约束 → 注入时会带上其自身约束
  const linkedHasConstraints = (path: string) =>
    projects.some((p) => samePath(p.path, path) && (p.constraints ?? "").trim().length > 0);

  return (
    <div className="h-full min-h-0 flex flex-col">
      <div className="flex items-center gap-2 mb-2 shrink-0">
        <div className="font-medium">关联项目</div>
        <span className="text-[11px] text-inkdim truncate">
          新对话会一并注入关联说明；与本项目约束冲突时，以关联项目为准
        </span>
        <button
          className="ml-auto shrink-0 px-3 py-1.5 rounded-lg text-[12px] bg-accent/15 text-accent hover:bg-accent/25 flex items-center gap-1.5 transition-colors"
          onClick={() => void startAdd()}
        >
          <Plus size={13} />
          <span>添加关联项目</span>
        </button>
      </div>

      <div className="flex-1 min-h-0 overflow-y-auto flex flex-col gap-2 pr-1">
        {/* 添加 / 编辑表单 */}
        {form && (
          <div className="border border-accent/40 rounded-xl p-3 bg-panel flex flex-col gap-2 shrink-0">
            <div className="flex items-center gap-2 text-[12px]">
              <span className="text-inkdim shrink-0">关联目录</span>
              <span className="font-mono flex-1 truncate" title={form.path}>
                {form.path}
              </span>
              <button className="text-[11px] text-inkdim hover:text-ink shrink-0" onClick={() => void repick()}>
                重选目录
              </button>
            </div>
            <textarea
              className={`${inputCls} resize-none`}
              rows={4}
              placeholder="说明该关联项目：用途、依赖关系、目录约定、注意事项等（Markdown）…"
              value={form.description}
              onChange={(e) => setForm({ ...form, description: e.target.value })}
            />
            <div className="flex justify-end gap-2">
              <button className="px-3 py-1.5 rounded-lg text-[12px] text-inkdim hover:bg-panel3" onClick={() => setForm(null)}>
                取消
              </button>
              <button
                className="px-3 py-1.5 rounded-lg text-[12px] bg-accent hover:bg-blue-500 text-white"
                onClick={commitForm}
              >
                确定
              </button>
            </div>
          </div>
        )}

        {/* 当前项目：列表首行，只读 */}
        <div className="border border-edge rounded-xl p-3 bg-panel/60">
          <div className="flex items-center gap-2">
            <span className="text-[13px] font-medium truncate">{project.name}</span>
            <span className="text-[11px] text-inkdim font-mono truncate flex-1" title={project.path ?? undefined}>
              {project.path ?? "未绑定目录"}
            </span>
            <span className="text-[11px] text-inkdim shrink-0">当前项目 · 只读</span>
            {(project.constraints ?? "").trim() ? (
              <button
                className="text-[11px] text-accent hover:underline shrink-0"
                title="查看项目约束"
                onClick={onOpenConstraints}
              >
                已设约束
              </button>
            ) : null}
          </div>
        </div>

        {/* 关联项目条目（草稿） */}
        {links.map((l) => (
          <div key={l.id} className="border border-edge rounded-xl p-3 bg-panel">
            <div className="flex items-center gap-2">
              <Folder size={14} className="text-inkdim shrink-0" />
              <span className="text-[13px] font-medium shrink-0">{dirName(l.path)}</span>
              <span className="text-[11px] text-inkdim font-mono truncate flex-1" title={l.path}>
                {l.path}
              </span>
              {linkedHasConstraints(l.path) && (
                <span className="text-[11px] text-accent shrink-0" title="该目录对应的项目已设置约束，注入时一并带上">
                  已注入该项目约束
                </span>
              )}
              <button
                className="text-[12px] text-inkdim hover:text-ink shrink-0 flex items-center gap-1 transition-colors"
                onClick={() => setForm({ id: l.id, path: l.path, description: l.description })}
              >
                <Edit3 size={11} />
                <span>编辑</span>
              </button>
              <button className="text-[12px] text-red-400 hover:underline shrink-0 flex items-center gap-1 transition-colors" onClick={() => void remove(l)}>
                <Trash2 size={11} />
                <span>删除</span>
              </button>
            </div>
            {l.description.trim() && (
              <div className="text-[12px] text-inkdim mt-1.5 whitespace-pre-wrap break-words max-h-20 overflow-y-auto">
                {l.description}
              </div>
            )}
          </div>
        ))}

        {links.length === 0 && !form && (
          <div className="flex-1 flex items-center justify-center text-inkdim text-[13px]">
            暂无关联项目。点击「添加关联项目」选择目录并撰写说明
          </div>
        )}
      </div>
    </div>
  );
}

// ---------- 弹窗外壳：左侧菜单 + 右侧内容 + 统一操作条 ----------

export function ProjectSettingsModal() {
  const projectSettingsId = useStore((s) => s.projectSettingsId);
  const close = useStore((s) => s.setProjectSettings);
  const projects = useStore((s) => s.projects);
  const refreshProjects = useStore((s) => s.refreshProjects);
  const pushToast = useStore((s) => s.pushToast);
  const [tab, setTab] = useState<Tab>("constraints");
  // 草稿：约束文本 + 关联项目列表；打开时装载，「应用 / 保存」时统一落库
  const [constraints, setConstraints] = useState("");
  const [links, setLinks] = useState<ProjectLink[]>([]);
  const [origLinks, setOrigLinks] = useState<ProjectLink[]>([]);
  const [saving, setSaving] = useState(false);

  const project = projects.find((p) => p.id === projectSettingsId);

  // 打开（或切换项目）时重置页面与草稿
  useEffect(() => {
    if (!projectSettingsId) return;
    setTab("constraints");
    setConstraints(projects.find((x) => x.id === projectSettingsId)?.constraints ?? "");
    let cancelled = false;
    ipc
      .listProjectLinks(projectSettingsId)
      .then((ls) => {
        if (cancelled) return;
        setLinks(ls);
        setOrigLinks(ls);
      })
      .catch((e) => pushToast(String(e)));
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [projectSettingsId]);

  if (!projectSettingsId) return null;
  if (!project) return null; // 项目可能刚被移除

  const linksEqual =
    links.length === origLinks.length &&
    links.every((l, i) => l.id === origLinks[i].id && l.path === origLinks[i].path && l.description === origLinks[i].description);
  const dirty = constraints !== (project.constraints ?? "") || !linksEqual;

  // 应用：把草稿统一写入后端（约束 + 关联项目增/改/删），不关闭窗口
  const apply = async (): Promise<boolean> => {
    setSaving(true);
    let ok = true;
    try {
      if (constraints !== (project.constraints ?? "")) {
        await ipc.setProjectConstraints(projectSettingsId, constraints);
      }
      for (const l of links) {
        const orig = origLinks.find((o) => o.id === l.id);
        if (!orig) await ipc.addProjectLink(projectSettingsId, l.path, l.description);
        else if (orig.path !== l.path || orig.description !== l.description)
          await ipc.updateProjectLink(l.id, l.path, l.description);
      }
      for (const o of origLinks) {
        if (!links.some((l) => l.id === o.id)) await ipc.deleteProjectLink(o.id);
      }
    } catch (e) {
      pushToast(String(e));
      ok = false;
    }
    // 无论成败都以落库结果为准刷新草稿（避免部分成功后重复提交）
    try {
      const fresh = await ipc.listProjectLinks(projectSettingsId);
      setLinks(fresh);
      setOrigLinks(fresh);
    } catch {
      /* 忽略刷新失败，下次打开会重新装载 */
    }
    await refreshProjects();
    setSaving(false);
    return ok;
  };

  // 保存：保存数据并关闭窗口；无改动时直接关闭
  const save = async () => {
    if (dirty && !(await apply())) return;
    close(null);
  };

  const menuCls = (active: boolean) =>
    `w-full text-left px-3 py-2 rounded-lg text-[13px] ${
      active ? "bg-panel3 text-ink" : "text-inkdim hover:text-ink hover:bg-panel2"
    }`;

  return (
    <div className="fixed inset-0 z-[80] bg-black/50 flex items-center justify-center">
      <div className="bg-panel2 border border-edge rounded-2xl w-[860px] h-[560px] max-w-[92vw] max-h-[86vh] flex flex-col shadow-2xl">
        <div className="px-5 py-4 border-b border-edge font-medium flex items-center gap-2 shrink-0">
          <span>项目设置</span>
          <span className="text-[13px] text-inkdim font-normal truncate">
            · {project.name}
            {project.path ? `（${project.path}）` : "（未绑定目录）"}
          </span>
          <ModalClose onClick={() => close(null)} />
        </div>
        <div className="flex-1 flex min-h-0">
          <aside className="w-[150px] shrink-0 border-r border-edge p-2 flex flex-col gap-1">
            <button className={menuCls(tab === "constraints")} onClick={() => setTab("constraints")}>
              <span className="flex items-center gap-2">
                <FileCode size={15} />
                <span>项目约束</span>
              </span>
            </button>
            <button className={menuCls(tab === "links")} onClick={() => setTab("links")}>
              <span className="flex items-center gap-2">
                <Link2 size={15} />
                <span>关联项目</span>
              </span>
            </button>
          </aside>
          <main className="flex-1 min-w-0 min-h-0 p-4">
            {tab === "constraints" ? (
              <ConstraintsPane value={constraints} onChange={setConstraints} />
            ) : (
              <LinksPane project={project} links={links} setLinks={setLinks} onOpenConstraints={() => setTab("constraints")} />
            )}
          </main>
        </div>
        <ModalActions
          onCancel={() => close(null)}
          onApply={() => void apply()}
          onSave={() => void save()}
          canApply={dirty}
          busy={saving}
        />
      </div>
    </div>
  );
}
