import { useEffect, useState } from "react";
import { ipc } from "../ipc";
import { useStore } from "../store";
import type { Provider, Settings, ToolInfo, AgentSopInfo } from "../types";
import { formatTokens, resolveModelContextLimit, resolveModelCapabilities, hasModelCapability, AGENT_SOPS } from "../types";
import { DataDirSection } from "./DataDirSection";
import { ModalActions, ModalClose } from "./ModalActions";
import { ModelContextModal } from "./ModelContextModal";
import { ModelCapabilitySelect } from "./ModelCapabilitySelect";

const MODEL_CAP_OPTIONS = [
  { id: "chat", label: "对话", icon: "💬" },
  { id: "image_gen", label: "生图", icon: "🎨" },
  { id: "vision", label: "视觉", icon: "👁️" },
] as const;

import {
  Cpu,
  Wrench,
  ShieldCheck,
  Database,
  Plus,
  Eye,
  EyeOff,
  Copy,
  Check,
  Terminal,
  Wind,
  Zap,
  Sparkles,
  BarChart2,
  Sliders,
} from "./Icons";

type Tab = "models" | "tools" | "sop" | "datadir";

const TOOL_TITLES: Record<string, string> = {
  read_file: "读取文件",
  list_dir: "列出目录",
  glob: "查找文件",
  grep: "搜索内容",
  write_file: "写入文件",
  edit_file: "编辑文件",
  run_command: "执行命令",
  todo: "任务清单",
  list_skills: "查询技能库",
  save_skill: "固化项目技能",
  run_skill: "执行项目技能",
  temp_status: "空间状态",
  temp_changes: "变更列表",
  temp_diff: "变更对比",
  temp_snapshot: "保存备份快照",
  temp_restore: "恢复状态",
  temp_merge: "合并回原目录",
};

function renderRiskBadge(risk: string) {
  switch (risk) {
    case "read":
      return (
        <span className="text-[11px] px-1.5 py-0.5 rounded bg-blue-500/10 text-blue-400 border border-blue-500/20">
          只读
        </span>
      );
    case "write":
      return (
        <span className="text-[11px] px-1.5 py-0.5 rounded bg-amber-500/10 text-amber-400 border border-amber-500/20">
          写入
        </span>
      );
    case "execute":
      return (
        <span className="text-[11px] px-1.5 py-0.5 rounded bg-red-500/10 text-red-400 border border-red-500/20">
          执行
        </span>
      );
    default:
      return (
        <span className="text-[11px] px-1.5 py-0.5 rounded bg-panel3 text-inkdim border border-edge">
          {risk}
        </span>
      );
  }
}

export function SettingsModal() {
  const show = useStore((s) => s.showSettings);
  const setShow = useStore((s) => s.setShowSettings);
  const storeSettings = useStore((s) => s.settings);
  const setSettingsLocal = useStore((s) => s.setSettingsLocal);
  const pushToast = useStore((s) => s.pushToast);
  const setShowTokenStatsModal = useStore((s) => s.setShowTokenStatsModal);
  const [tab, setTab] = useState<Tab>("models");
  const [local, setLocal] = useState<Settings>(storeSettings);
  const [toolList, setToolList] = useState<ToolInfo[]>([]);
  const [testing, setTesting] = useState<Record<string, string>>({});
  const [saving, setSaving] = useState(false);
  const [visibleKeys, setVisibleKeys] = useState<Record<string, boolean>>({});
  const [copiedId, setCopiedId] = useState<string | null>(null);
  const [newModel, setNewModel] = useState<Record<string, string>>({});
  const [expandedIds, setExpandedIds] = useState<Record<string, boolean>>({});
  const [editingContextModel, setEditingContextModel] = useState<{
    providerId: string;
    providerName?: string;
    modelName: string;
    currentLimit: number;
  } | null>(null);

  useEffect(() => {
    if (show) {
      setTab("models");
      setLocal(structuredClone(storeSettings));
      setTesting({});
      setVisibleKeys({});
      setCopiedId(null);
      setNewModel({});
      setExpandedIds({});
      setEditingContextModel(null);
      // 打开时从后端拉取最新设置与工具列表，避免草稿基于启动时的旧缓存回写
      ipc
        .getSettings()
        .then((fresh) => {
          setSettingsLocal(fresh);
          setLocal(structuredClone(fresh));
        })
        .catch((e) => pushToast(String(e)));

      ipc
        .listTools()
        .then(setToolList)
        .catch((e) => pushToast(String(e)));
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [show]);

  if (!show) return null;

  // 是否有待保存的改动（决定「应用」可用性）
  const dirty = JSON.stringify(local) !== JSON.stringify(storeSettings);

  const toggleExpand = (id: string) => {
    setExpandedIds((cur) => ({ ...cur, [id]: !cur[id] }));
  };

  const updateProvider = (id: string, patch: Partial<Provider>) => {
    setLocal((cur) => ({ ...cur, providers: cur.providers.map((p) => (p.id === id ? { ...p, ...patch } : p)) }));
  };

  const handleSaveModelContext = (newLimit: number | null) => {
    if (!editingContextModel) return;
    const { providerId, modelName } = editingContextModel;
    const key = `${providerId}:${modelName}`;
    setLocal((cur) => {
      const nextLimits = { ...(cur.modelContextLimits || {}) };
      if (newLimit === null) {
        delete nextLimits[key];
        delete nextLimits[modelName];
      } else {
        nextLimits[key] = newLimit;
        nextLimits[modelName] = newLimit;
      }
      return {
        ...cur,
        modelContextLimits: nextLimits,
      };
    });
  };

  const addProvider = () => {
    const p: Provider = {
      id: Math.random().toString(36).slice(2),
      name: "新厂商",
      baseUrl: "https://open.bigmodel.cn/api/paas/v4",
      models: [],
      apiKey: "",
    };
    setLocal((cur) => ({
      ...cur,
      providers: [...cur.providers, p],
      activeProviderId: cur.activeProviderId ?? p.id,
      activeModelId: cur.activeProviderId ? cur.activeModelId : (p.models[0] ?? null),
    }));
    setExpandedIds((cur) => ({ ...cur, [p.id]: true }));
  };

  // 选择默认厂商：默认使用该厂商的第一个模型
  const selectProvider = (id: string) => {
    const defaultModel = local.providers.find((x) => x.id === id)?.models[0] ?? null;
    setLocal((cur) => ({
      ...cur,
      activeProviderId: id,
      activeModelId: defaultModel,
      activeModel: defaultModel,
    }));
  };

  const addModel = (providerId: string) => {
    const name = (newModel[providerId] ?? "").trim();
    if (!name) return;
    const p = local.providers.find((x) => x.id === providerId);
    if (!p) return;
    if (p.models.includes(name)) {
      pushToast("该模型已存在");
      return;
    }
    setNewModel((m) => ({ ...m, [providerId]: "" }));
    setLocal((cur) => {
      const providers = cur.providers.map((item) =>
        item.id === providerId ? { ...item, models: [...item.models, name] } : item
      );
      const isFirst = cur.activeProviderId === providerId && !(cur.activeModelId ?? cur.activeModel);
      const activeM = isFirst ? name : (cur.activeModelId ?? cur.activeModel ?? null);
      return {
        ...cur,
        providers,
        activeModelId: activeM,
        activeModel: activeM,
      };
    });
  };

  const removeModel = (providerId: string, model: string) => {
    const p = local.providers.find((x) => x.id === providerId);
    if (!p) return;
    const models = p.models.filter((m) => m !== model);
    updateProvider(providerId, { models });
    // 删除的是激活模型时，回落到该厂商剩余的第一个模型
    setLocal((cur) => {
      const activeCurrent = cur.activeModelId ?? cur.activeModel;
      const isCurrent = cur.activeProviderId === providerId && activeCurrent === model;
      const activeM = isCurrent ? models[0] ?? null : (cur.activeModelId ?? cur.activeModel ?? null);
      return {
        ...cur,
        activeModelId: activeM,
        activeModel: activeM,
      };
    });
  };

  const removeProvider = (id: string) => {
    setLocal((cur) => {
      const providers = cur.providers.filter((x) => x.id !== id);
      if (cur.activeProviderId !== id) return { ...cur, providers };
      const defaultModel = providers[0]?.models[0] ?? null;
      return {
        ...cur,
        providers,
        activeProviderId: providers[0]?.id ?? null,
        activeModelId: defaultModel,
        activeModel: defaultModel,
      };
    });
    setExpandedIds((cur) => {
      const next = { ...cur };
      delete next[id];
      return next;
    });
  };

  // 应用：只保存数据，不关闭窗口
  const apply = async (): Promise<boolean> => {
    setSaving(true);
    try {
      const activeM = local.activeModelId ?? local.activeModel ?? null;
      const toSave = { ...local, activeModelId: activeM, activeModel: activeM };
      await ipc.setSettings(toSave);
      const fresh = await ipc.getSettings();
      setSettingsLocal(fresh);
      setLocal(fresh);
      return true;
    } catch (e) {
      pushToast(String(e));
      return false;
    } finally {
      setSaving(false);
    }
  };

  // 保存：保存数据并关闭窗口；无改动时直接关闭
  const save = async () => {
    if (dirty && !(await apply())) return;
    setShow(false);
  };

  const isToolEnabled = (name: string) => !(local.disabledTools ?? []).includes(name);

  const toggleTool = (name: string) => {
    const current = local.disabledTools ?? [];
    const next = current.includes(name)
      ? current.filter((t) => t !== name)
      : [...current, name];
    setLocal((cur) => ({ ...cur, disabledTools: next }));
  };

  const enableAllTools = () => {
    setLocal((cur) => ({ ...cur, disabledTools: [] }));
  };

  const isSkillTool = (name: string) => ["list_skills", "save_skill", "run_skill"].includes(name);

  const coreTools = toolList.filter((t) => !t.isTemp && !isSkillTool(t.name));
  const evolutionTools = toolList.filter((t) => isSkillTool(t.name));
  const tempTools = toolList.filter((t) => t.isTemp);
  const disabledCount = (local.disabledTools ?? []).length;

  const renderCategoryBadge = (name: string, isTemp: boolean) => {
    if (isTemp) {
      return (
        <span className="text-[11px] px-1.5 py-0.5 rounded bg-amber-500/15 text-amber-400 border border-amber-500/30 font-medium">
          临时空间
        </span>
      );
    }
    if (isSkillTool(name)) {
      return (
        <span className="text-[11px] px-1.5 py-0.5 rounded bg-purple-500/15 text-purple-400 border border-purple-500/30 font-medium">
          自演化引擎
        </span>
      );
    }
    return (
      <span className="text-[11px] px-1.5 py-0.5 rounded bg-blue-500/15 text-blue-400 border border-blue-500/30 font-medium">
        系统内置
      </span>
    );
  };

  const renderToolItem = (t: ToolInfo) => {
    const enabled = isToolEnabled(t.name);
    const title = TOOL_TITLES[t.name];
    return (
      <div
        key={t.name}
        className={`border rounded-xl p-3 flex items-start justify-between gap-3 transition-colors ${
          enabled ? "border-edge bg-panel" : "border-edge/50 bg-panel/40 opacity-75"
        }`}
      >
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2 flex-wrap">
            <code className="text-[13px] font-mono font-medium text-ink bg-panel3 px-1.5 py-0.5 rounded border border-edge/60">
              {t.name}
            </code>
            {title && <span className="text-[12px] text-ink font-medium">{title}</span>}
            {renderRiskBadge(t.risk)}
            {renderCategoryBadge(t.name, t.isTemp)}
          </div>
          <div className="text-[12px] text-inkdim mt-1.5 leading-relaxed">{t.description}</div>
        </div>

        <div className="flex items-center gap-2 shrink-0 pt-0.5">
          <span className={`text-[12px] select-none whitespace-nowrap ${enabled ? "text-green-400 font-medium" : "text-inkdim"}`}>
            {enabled ? "启用" : "禁用"}
          </span>
          <button
            type="button"
            role="switch"
            aria-checked={enabled}
            title={enabled ? `点击禁用 ${t.name}` : `点击启用 ${t.name}`}
            onClick={() => toggleTool(t.name)}
            className={`relative inline-flex h-5 w-9 shrink-0 cursor-pointer rounded-full border border-transparent transition-colors duration-200 ease-in-out focus:outline-none ${
              enabled ? "bg-accent" : "bg-panel3 border-edge"
            }`}
          >
            <span
              className={`pointer-events-none inline-block h-4 w-4 transform rounded-full bg-white shadow-sm ring-0 transition duration-200 ease-in-out ${
                enabled ? "translate-x-4" : "translate-x-0"
              }`}
            />
          </button>
        </div>
      </div>
    );
  };

  const isSopEnabled = (id: string) => !(local.disabledSops ?? []).includes(id);

  const toggleSop = (id: string) => {
    const current = local.disabledSops ?? [];
    const next = current.includes(id)
      ? current.filter((s) => s !== id)
      : [...current, id];
    setLocal((cur) => ({ ...cur, disabledSops: next }));
  };

  const enableAllSops = () => {
    setLocal((cur) => ({ ...cur, disabledSops: [] }));
  };

  const disabledSopCount = (local.disabledSops ?? []).length;

  const renderSopCategoryBadge = (category: AgentSopInfo["category"], label: string) => {
    switch (category) {
      case "thinking":
        return (
          <span className="text-[11px] px-1.5 py-0.5 rounded bg-cyan-500/15 text-cyan-400 border border-cyan-500/30 font-medium">
            {label}
          </span>
        );
      case "memory":
        return (
          <span className="text-[11px] px-1.5 py-0.5 rounded bg-emerald-500/15 text-emerald-400 border border-emerald-500/30 font-medium">
            {label}
          </span>
        );
      case "orchestration":
        return (
          <span className="text-[11px] px-1.5 py-0.5 rounded bg-purple-500/15 text-purple-400 border border-purple-500/30 font-medium">
            {label}
          </span>
        );
      case "quality":
        return (
          <span className="text-[11px] px-1.5 py-0.5 rounded bg-blue-500/15 text-blue-400 border border-blue-500/30 font-medium">
            {label}
          </span>
        );
      case "workflow":
        return (
          <span className="text-[11px] px-1.5 py-0.5 rounded bg-amber-500/15 text-amber-400 border border-amber-500/30 font-medium">
            {label}
          </span>
        );
      default:
        return (
          <span className="text-[11px] px-1.5 py-0.5 rounded bg-panel3 text-inkdim border border-edge font-medium">
            {label}
          </span>
        );
    }
  };

  const renderSopItem = (s: AgentSopInfo) => {
    const enabled = isSopEnabled(s.id);
    return (
      <div
        key={s.id}
        className={`border rounded-xl p-3 flex items-start justify-between gap-3 transition-colors ${
          enabled ? "border-edge bg-panel" : "border-edge/50 bg-panel/40 opacity-75"
        }`}
      >
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2 flex-wrap">
            <code className="text-[13px] font-mono font-medium text-ink bg-panel3 px-1.5 py-0.5 rounded border border-edge/60">
              {s.id}
            </code>
            <span className="text-[12px] text-ink font-medium">{s.name}</span>
            {renderSopCategoryBadge(s.category, s.categoryLabel)}
          </div>
          <div className="text-[12px] text-inkdim mt-1.5 leading-relaxed">{s.description}</div>
          <div className="text-[11px] text-inkdim/60 mt-1.5 flex items-center gap-1 font-mono">
            <span>↳</span>
            <span>{s.disableEffect}</span>
          </div>
        </div>

        <div className="flex items-center gap-2 shrink-0 pt-0.5">
          <span className={`text-[12px] select-none whitespace-nowrap ${enabled ? "text-green-400 font-medium" : "text-inkdim"}`}>
            {enabled ? "启用" : "禁用"}
          </span>
          <button
            type="button"
            role="switch"
            aria-checked={enabled}
            title={enabled ? `点击禁用 ${s.name}` : `点击启用 ${s.name}`}
            onClick={() => toggleSop(s.id)}
            className={`relative inline-flex h-5 w-9 shrink-0 cursor-pointer rounded-full border border-transparent transition-colors duration-200 ease-in-out focus:outline-none ${
              enabled ? "bg-accent" : "bg-panel3 border-edge"
            }`}
          >
            <span
              className={`pointer-events-none inline-block h-4 w-4 transform rounded-full bg-white shadow-sm ring-0 transition duration-200 ease-in-out ${
                enabled ? "translate-x-4" : "translate-x-0"
              }`}
            />
          </button>
        </div>
      </div>
    );
  };

  const inputCls = "bg-panel border border-edge rounded-lg px-2 py-1.5 text-[13px] outline-none focus:border-accent w-full";

  const menuCls = (active: boolean) =>
    `w-full text-left px-3 py-2 rounded-lg text-[13px] ${
      active ? "bg-panel3 text-ink" : "text-inkdim hover:text-ink hover:bg-panel2"
    }`;

  return (
    <div className="fixed inset-0 z-[80] bg-black/50 flex items-center justify-center">
      <div className="bg-panel2 border border-edge rounded-2xl w-[860px] h-[560px] max-w-[92vw] max-h-[86vh] flex flex-col shadow-2xl">
        <div className="h-12 px-5 border-b border-edge/60 font-medium flex items-center gap-2 shrink-0">
          <span>程序设置</span>
          <ModalClose onClick={() => setShow(false)} />
        </div>
        <div className="flex-1 flex min-h-0">
          <aside className="w-[155px] shrink-0 border-r border-edge p-2 flex flex-col gap-1">
            <button className={menuCls(tab === "models")} onClick={() => setTab("models")}>
              <span className="flex items-center gap-2">
                <Cpu size={15} />
                <span>模型设置</span>
              </span>
            </button>
            <button className={menuCls(tab === "tools")} onClick={() => setTab("tools")}>
              <span className="flex items-center gap-2">
                <Wrench size={15} />
                <span>Agent 工具</span>
              </span>
            </button>
            <button className={menuCls(tab === "sop")} onClick={() => setTab("sop")}>
              <span className="flex items-center gap-2">
                <ShieldCheck size={15} />
                <span>Agent SOP</span>
              </span>
            </button>
            <button className={menuCls(tab === "datadir")} onClick={() => setTab("datadir")}>
              <span className="flex items-center gap-2">
                <Database size={15} />
                <span>数据目录</span>
              </span>
            </button>
            <div className="border-t border-edge/40 my-1" />
            <button
              className="w-full text-left px-3 py-2 rounded-lg text-[13px] text-inkdim hover:text-ink hover:bg-panel2"
              onClick={() => { setShow(false); setShowTokenStatsModal(true); }}
            >
              <span className="flex items-center gap-2">
                <BarChart2 size={15} className="text-amber-400" />
                <span>Token 统计</span>
              </span>
            </button>
          </aside>
          <main className="flex-1 min-w-0 min-h-0 overflow-y-auto p-4 text-[13px]">
            {tab === "models" && (
              <div className="flex flex-col gap-6">
                {/* 模型厂商 */}
                <section>
                  <div className="flex items-center mb-2">
                    <div className="font-medium">模型厂商（OpenAI 兼容）</div>
                    <button className="ml-auto text-accent hover:underline flex items-center gap-1 text-[12px]" onClick={addProvider}>
                      <Plus size={13} />
                      <span>添加厂商</span>
                    </button>
                  </div>
                  <div className="flex flex-col gap-2.5">
                    {local.providers.map((p) => {
                      const isExpanded = !!expandedIds[p.id];
                      return (
                        <div key={p.id} className="border border-edge rounded-xl bg-panel overflow-hidden transition-colors">
                          {/* 厂商条目头部（默认折叠，紧凑展示） */}
                          <div
                            className="p-3 flex items-center gap-2.5 cursor-pointer hover:bg-panel2/50 select-none transition-colors"
                            onClick={() => toggleExpand(p.id)}
                          >
                            <label
                              className="flex items-center gap-1.5 cursor-pointer shrink-0"
                              title={local.activeProviderId === p.id ? "当前激活厂商" : "设为激活厂商"}
                              onClick={(e) => e.stopPropagation()}
                            >
                              <input
                                type="radio"
                                name="activeProvider"
                                checked={local.activeProviderId === p.id}
                                onChange={() => selectProvider(p.id)}
                              />
                            </label>

                            <div className="flex items-center gap-2 min-w-0 flex-1">
                              <span className="font-medium text-ink truncate max-w-[150px]">{p.name || "未命名厂商"}</span>
                              {local.activeProviderId === p.id && (
                                <span className="text-[11px] px-1.5 py-0.5 rounded bg-green-500/10 text-green-400 border border-green-500/20 shrink-0">
                                  使用中
                                </span>
                              )}
                              {!isExpanded && (
                                <span className="text-[12px] text-inkdim truncate ml-1 font-mono flex-1 min-w-0" title={p.models.join(", ")}>
                                  {p.models.length > 0 ? (
                                    `模型: ${p.models.join(", ")}`
                                  ) : (
                                    <span className="text-amber-400/80 font-sans">未配置模型</span>
                                  )}
                                </span>
                              )}
                            </div>

                            <div className="flex items-center gap-1.5 shrink-0 ml-auto" onClick={(e) => e.stopPropagation()}>
                              <button
                                className="text-[12px] px-2.5 py-1 rounded-lg bg-panel3 hover:bg-edge text-inkdim hover:text-ink"
                                onClick={() => toggleExpand(p.id)}
                              >
                                {isExpanded ? "收起" : "编辑"}
                              </button>
                              <button
                                className="text-[12px] px-2.5 py-1 rounded-lg bg-panel3 hover:bg-edge text-red-400"
                                onClick={() => removeProvider(p.id)}
                              >
                                删除
                              </button>
                            </div>
                          </div>

                          {/* 展开内容 */}
                          {isExpanded && (
                            <div className="px-3 pb-3 pt-2.5 border-t border-edge/60 flex flex-col gap-3">
                              <div className="grid grid-cols-2 gap-2">
                                <label className="flex items-center gap-2">
                                  <span className="text-inkdim shrink-0 w-14">厂商名称</span>
                                  <input className={inputCls} value={p.name} onChange={(e) => updateProvider(p.id, { name: e.target.value })} />
                                </label>
                                <label className="flex items-center gap-2">
                                  <span className="text-inkdim shrink-0 w-14">Base URL</span>
                                  <input
                                    className={inputCls}
                                    value={p.baseUrl}
                                    placeholder="https://..."
                                    onChange={(e) => updateProvider(p.id, { baseUrl: e.target.value })}
                                  />
                                </label>
                              </div>
                              <div className="flex items-center gap-2">
                                <span className="text-inkdim shrink-0 w-14">API Key</span>
                                <input
                                  className={`${inputCls} flex-1 min-w-0`}
                                  type={visibleKeys[p.id] ? "text" : "password"}
                                  value={p.apiKey}
                                  placeholder="sk-…"
                                  onChange={(e) => updateProvider(p.id, { apiKey: e.target.value })}
                                />
                                <button
                                  className="shrink-0 w-7 h-7 rounded-lg bg-panel3 hover:bg-edge flex items-center justify-center text-inkdim hover:text-ink transition-colors"
                                  title={visibleKeys[p.id] ? "隐藏" : "明文显示"}
                                  onClick={() => setVisibleKeys((v) => ({ ...v, [p.id]: !v[p.id] }))}
                                >
                                  {visibleKeys[p.id] ? <EyeOff size={14} /> : <Eye size={14} />}
                                </button>
                                <button
                                  className={`shrink-0 w-7 h-7 rounded-lg bg-panel3 hover:bg-edge flex items-center justify-center ${
                                    copiedId === p.id ? "text-emerald-400" : "text-inkdim hover:text-ink"
                                  } ${p.apiKey ? "" : "opacity-40 cursor-not-allowed"} transition-colors`}
                                  title="复制 API Key"
                                  disabled={!p.apiKey}
                                  onClick={() => {
                                    if (!p.apiKey) return;
                                    navigator.clipboard.writeText(p.apiKey).then(
                                      () => {
                                        setCopiedId(p.id);
                                        setTimeout(() => setCopiedId((cur) => (cur === p.id ? null : cur)), 1500);
                                      },
                                      () => pushToast("复制失败")
                                    );
                                  }}
                                >
                                  {copiedId === p.id ? <Check size={14} /> : <Copy size={14} />}
                                </button>
                              </div>
                              {/* 该厂商下的模型列表（可配置多个） */}
                              <div>
                                <div className="text-inkdim mb-1.5">模型（同一厂商可配置多个）</div>
                                <div className="flex flex-col gap-1 mb-1.5">
                                  {p.models.map((m) => {
                                    const isCurrent =
                                      local.activeProviderId === p.id &&
                                      (local.activeModelId === m || local.activeModel === m);
                                    const limit = resolveModelContextLimit(local, p.id, m);
                                    const isCustom =
                                      local.modelContextLimits?.[`${p.id}:${m}`] != null ||
                                      local.modelContextLimits?.[m] != null;
                                    const caps = resolveModelCapabilities(local, p.id, m);
                                    return (
                                      <div key={m} className="flex items-center gap-2 bg-panel2 border border-edge rounded-lg px-2.5 py-1.5">
                                        <span className="font-mono text-[12px] flex-1 truncate">{m}</span>

                                        {/* 模型能力标签微调 */}
                                        <div className="flex items-center gap-1 shrink-0" title="模型能力标记（点击切换）">
                                          {MODEL_CAP_OPTIONS.map((cap) => {
                                            const active = caps.includes(cap.id);
                                            return (
                                              <button
                                                key={cap.id}
                                                type="button"
                                                onClick={() => {
                                                  const nextCaps = active
                                                    ? caps.filter((c) => c !== cap.id)
                                                    : [...caps, cap.id];
                                                  setLocal((cur) => ({
                                                    ...cur,
                                                    modelCapabilities: {
                                                      ...(cur.modelCapabilities || {}),
                                                      [`${p.id}:${m}`]: nextCaps,
                                                    },
                                                  }));
                                                }}
                                                title={active ? `点击取消【${cap.label}】能力` : `点击赋予【${cap.label}】能力`}
                                                className={`text-[10px] px-1.5 py-0.5 rounded transition-all cursor-pointer flex items-center gap-0.5 border ${
                                                  active
                                                    ? cap.id === "chat"
                                                      ? "bg-blue-500/15 text-blue-400 border-blue-500/30 font-medium"
                                                      : cap.id === "image_gen"
                                                      ? "bg-purple-500/15 text-purple-400 border-purple-500/30 font-medium"
                                                      : "bg-emerald-500/15 text-emerald-400 border-emerald-500/30 font-medium"
                                                    : "bg-panel3/40 text-inkdim/40 border-edge/50 hover:text-inkdim hover:border-edge"
                                                }`}
                                              >
                                                <span>{cap.icon}</span>
                                                <span>{cap.label}</span>
                                              </button>
                                            );
                                          })}
                                        </div>

                                        <button
                                          type="button"
                                          className={`text-[11px] font-mono px-2 py-0.5 rounded-md border flex items-center gap-1 transition-all shrink-0 ${
                                            isCustom
                                              ? "bg-accent/15 border-accent/40 text-accent hover:bg-accent/25"
                                              : "bg-panel3/80 border-edge/80 text-inkdim hover:text-ink hover:border-accent/40 hover:bg-panel3"
                                          }`}
                                          onClick={() =>
                                            setEditingContextModel({
                                              providerId: p.id,
                                              providerName: p.name,
                                              modelName: m,
                                              currentLimit: limit,
                                            })
                                          }
                                          title={`点击配置 ${m} 的上下文上限（当前: ${limit.toLocaleString()} tokens）`}
                                        >
                                          <Sliders size={11} className="opacity-70" />
                                          <span>{formatTokens(limit)}</span>
                                        </button>
                                        {isCurrent ? (
                                          <span className="text-[11px] text-green-400 shrink-0">当前使用</span>
                                        ) : (
                                          <button
                                            className="text-[12px] text-inkdim hover:text-accent shrink-0"
                                            onClick={() =>
                                              setLocal((cur) => ({
                                                ...cur,
                                                activeProviderId: p.id,
                                                activeModelId: m,
                                                activeModel: m,
                                              }))
                                            }
                                          >
                                            设为使用
                                          </button>
                                        )}
                                        <button
                                          className="text-[12px] text-red-400 hover:underline shrink-0"
                                          onClick={() => removeModel(p.id, m)}
                                        >
                                          删除
                                        </button>
                                      </div>
                                    );

                                  })}
                                  {p.models.length === 0 && <div className="text-[12px] text-amber-400">请至少添加一个模型</div>}
                                </div>
                                <div className="flex gap-2">
                                  <input
                                    className={inputCls}
                                    placeholder="输入模型名，如 deepseek-chat"
                                    value={newModel[p.id] ?? ""}
                                    onChange={(e) => setNewModel((m) => ({ ...m, [p.id]: e.target.value }))}
                                    onKeyDown={(e) => {
                                      if (e.key === "Enter") addModel(p.id);
                                    }}
                                  />
                                  <button
                                    className="shrink-0 text-[12px] px-3 rounded-lg bg-panel3 hover:bg-edge text-inkdim hover:text-ink flex items-center gap-1.5 transition-colors"
                                    onClick={() => addModel(p.id)}
                                  >
                                    <Plus size={13} />
                                    <span>添加模型</span>
                                  </button>
                                </div>
                              </div>
                              <div className="flex items-center gap-2 pt-1">
                                <label className="flex items-center gap-1.5 text-inkdim text-[12px] cursor-pointer">
                                  <input
                                    type="radio"
                                    checked={local.activeProviderId === p.id}
                                    onChange={() => selectProvider(p.id)}
                                  />
                                  使用此厂商
                                </label>
                                <button
                                  className="ml-auto text-[12px] px-2.5 py-1 rounded-lg bg-panel3 hover:bg-edge text-inkdim"
                                  onClick={async () => {
                                    setTesting((t) => ({ ...t, [p.id]: "测试中…" }));
                                    try {
                                      const msg = await ipc.testProvider(p);
                                      setTesting((t) => ({ ...t, [p.id]: msg }));
                                    } catch (e) {
                                      setTesting((t) => ({ ...t, [p.id]: String(e) }));
                                    }
                                  }}
                                >
                                  测试连接
                                </button>
                                <button
                                  className="text-[12px] px-2.5 py-1 rounded-lg bg-panel3 hover:bg-edge text-inkdim hover:text-ink"
                                  onClick={() => toggleExpand(p.id)}
                                >
                                  收起
                                </button>
                              </div>
                              {testing[p.id] && (
                                <div className={`text-[12px] ${testing[p.id].startsWith("连接成功") ? "text-green-400" : "text-red-400"}`}>
                                  {testing[p.id]}
                                </div>
                              )}
                            </div>
                          )}
                        </div>
                      );
                    })}
                    {local.providers.length === 0 && <div className="text-inkdim">尚未添加厂商。支持任意 OpenAI 兼容接口。</div>}
                  </div>
                </section>

                {/* 默认能力模型配置 */}
                <section>
                  <div className="font-medium mb-2 flex items-center justify-between">
                    <div className="flex items-center gap-2">
                      <span>全局默认能力模型</span>
                      <span className="text-[11px] font-normal text-inkdim">
                        （当未在对话或协作者中单独指定时使用；若留空则主进程默认不直接提供生图工具，引导调度协作者）
                      </span>
                    </div>
                  </div>
                  <div className="bg-panel border border-edge rounded-xl p-3 flex flex-col gap-2.5">
                    <div className="flex items-center gap-3">
                      <span className="text-inkdim text-[12px] shrink-0 w-24">执行生图模型</span>
                      <div className="flex-1">
                        <ModelCapabilitySelect
                          capability="image_gen"
                          value={
                            local.activeImageProviderId && local.activeImageModelId
                              ? `${local.activeImageProviderId}::${local.activeImageModelId}`
                              : ""
                          }
                          onChange={(val) => {
                            if (!val) {
                              setLocal((cur) => ({
                                ...cur,
                                activeImageProviderId: null,
                                activeImageModelId: null,
                              }));
                            } else {
                              const [pid, mid] = val.split("::");
                              setLocal((cur) => ({
                                ...cur,
                                activeImageProviderId: pid,
                                activeImageModelId: mid,
                              }));
                            }
                          }}
                          providers={local.providers}
                          settings={local}
                          allowInherit={true}
                          inheritLabel="未配置（推荐：由专门的图像生成协作者承接生图，避免主进程模型冲突）"
                        />
                      </div>
                    </div>
                    <div className="flex items-center gap-3">
                      <span className="text-inkdim text-[12px] shrink-0 w-24">视觉感知模型</span>
                      <div className="flex-1">
                        <ModelCapabilitySelect
                          capability="vision"
                          value={
                            local.activeVisionProviderId && local.activeVisionModelId
                              ? `${local.activeVisionProviderId}::${local.activeVisionModelId}`
                              : ""
                          }
                          onChange={(val) => {
                            if (!val) {
                              setLocal((cur) => ({
                                ...cur,
                                activeVisionProviderId: null,
                                activeVisionModelId: null,
                              }));
                            } else {
                              const [pid, mid] = val.split("::");
                              setLocal((cur) => ({
                                ...cur,
                                activeVisionProviderId: pid,
                                activeVisionModelId: mid,
                              }));
                            }
                          }}
                          providers={local.providers}
                          settings={local}
                          allowInherit={true}
                          inheritLabel="未配置（跟随当前对话模型或回落多模态模型）"
                        />
                      </div>
                    </div>
                    <div className="text-[11px] text-inkdim flex items-center gap-1.5 leading-relaxed bg-panel3/40 rounded-lg px-2.5 py-1.5 border border-edge/40">
                      <Sparkles size={13} className="text-accent shrink-0" />
                      <span>
                        建议：将纯生图模型（如 <code className="font-mono text-accent">doubao-seedream-5.0-lite</code>）标记为 <span className="text-purple-400 font-medium">🎨 生图</span> 能力，并在「图像生成协作者」中单独绑定。这样对话思考与生图物理隔离，杜绝 400 Bad Request。
                      </span>
                    </div>
                  </div>
                </section>

                {/* 运行参数 */}
                <section>
                  <div className="font-medium mb-2">运行参数</div>
                  <div className="grid grid-cols-2 sm:grid-cols-4 gap-3">
                    <label className="flex flex-col gap-1">
                      <span className="text-inkdim">默认访问模式</span>
                      <select
                        className={inputCls}
                        value={local.globalAccessMode}
                        onChange={(e) => setLocal({ ...local, globalAccessMode: e.target.value as Settings["globalAccessMode"] })}
                      >
                        <option value="confirm">变更前确认</option>
                        <option value="full_access">完全访问</option>
                      </select>
                    </label>
                    <label className="flex flex-col gap-1">
                      <span className="text-inkdim">最大步数</span>
                      <input
                        className={inputCls}
                        type="number"
                        min={1}
                        max={100}
                        value={local.maxSteps}
                        onChange={(e) => setLocal({ ...local, maxSteps: Number(e.target.value) || 30 })}
                      />
                    </label>
                    <label className="flex flex-col gap-1">
                      <span className="text-inkdim">命令超时（秒）</span>
                      <input
                        className={inputCls}
                        type="number"
                        min={5}
                        max={1800}
                        value={local.commandTimeoutSecs}
                        onChange={(e) => setLocal({ ...local, commandTimeoutSecs: Number(e.target.value) || 120 })}
                      />
                    </label>
                    <label className="flex flex-col gap-1">
                      <span className="text-inkdim" title="当模型未单独配置且无法自动推断时使用的全局保底上限">
                        未识别模型保底上限
                      </span>
                      <input
                        className={inputCls}
                        type="number"
                        min={2000}
                        max={2000000}
                        step={1000}
                        value={local.contextTokenLimit}
                        onChange={(e) => setLocal({ ...local, contextTokenLimit: Number(e.target.value) || 64000 })}
                        title="当模型未单独配置且无法自动推断时使用的全局保底上限"
                      />
                    </label>
                  </div>
                  <div className="mt-3 text-[11px] text-inkdim bg-panel3/40 border border-edge/50 rounded-xl px-3.5 py-2.5 flex items-center justify-between">
                    <div className="flex items-center gap-2">
                      <Sparkles size={13} className="text-accent shrink-0" />
                      <span>
                        各模型的上下文上限已按模型维度独立配置：点击上方模型列表中模型右侧的标签（如 <span className="font-mono text-accent font-medium">{formatTokens(local.contextTokenLimit || 64000)}</span>）即可弹出配置对话框进行微调。
                      </span>
                    </div>
                  </div>
                </section>
              </div>
            )}

            {tab === "tools" && (
              <div className="flex flex-col gap-5">
                <section>
                  <div className="flex items-start justify-between gap-4 mb-3">
                    <div className="flex-1 min-w-0">
                      <div className="font-medium text-[14px]">Agent 工具管理</div>
                      <div className="text-[12px] text-inkdim mt-0.5 leading-relaxed max-w-[520px]">
                        配置 Agent 可调用的工具。禁用后模型将无法获取和调用该工具。
                      </div>
                    </div>
                    <div className="flex items-center gap-2.5 shrink-0 pt-0.5">
                      <span className="text-[12px] text-inkdim font-medium whitespace-nowrap bg-panel3 px-2 py-0.5 rounded-md border border-edge/60">
                        已启用 {toolList.length - disabledCount} / {toolList.length}
                      </span>
                      {disabledCount > 0 && (
                        <button
                          className="text-[12px] text-accent hover:underline whitespace-nowrap"
                          onClick={enableAllTools}
                        >
                          全部启用
                        </button>
                      )}
                    </div>
                  </div>

                  {/* 架构与自演化说明卡片 */}
                  <div className="p-3 rounded-xl border border-edge/80 bg-panel3/40 flex items-start gap-3 mt-1 mb-2 text-[12px] leading-relaxed">
                    <Sparkles size={16} className="text-accent shrink-0 mt-0.5" />
                    <div className="flex flex-col gap-1">
                      <div className="font-medium text-ink">工具体系与自成长架构说明</div>
                      <div className="text-inkdim leading-relaxed">
                        <span className="text-blue-400 font-medium">1. 系统内置工具</span>：平台底座原生提供的文件读写、代码搜索与命令行执行能力；<br />
                        <span className="text-purple-400 font-medium">2. 自演化引擎工具</span>：Agent 发现多步任务规律并将其固化为复用技能的内置元工具；<br />
                        <span className="text-emerald-400 font-medium">3. 项目自成长技能</span>：Agent 或用户自动沉淀的项目定制技能脚本，存储于项目的 <code className="bg-panel2 px-1 rounded text-accent font-mono">.harness/skills/</code> 目录。可在顶栏 <b className="text-ink">「🌱 成长中心」</b> 的「⚡ 技能工具库」中查看与管理。
                      </div>
                    </div>
                  </div>

                  {toolList.length === 0 ? (
                    <div className="text-inkdim py-8 text-center">正在加载工具列表…</div>
                  ) : (
                    <>
                      {/* 系统内置基础工具 */}
                      <div className="mt-4">
                        <div className="text-[12px] font-medium text-inkdim mb-2 flex items-center justify-between">
                          <div className="flex items-center gap-1.5">
                            <Terminal size={14} className="text-blue-400" />
                            <span className="text-ink">系统内置基础工具</span>
                            <span className="text-[11px] text-inkdim/60">
                              ({coreTools.filter((t) => isToolEnabled(t.name)).length}/{coreTools.length})
                            </span>
                          </div>
                          <span className="text-[11px] text-inkdim/60">文件读写 · 代码检索 · 终端交互 · 任务管理</span>
                        </div>
                        <div className="flex flex-col gap-2">
                          {coreTools.map(renderToolItem)}
                        </div>
                      </div>

                      {/* 自演化技能引擎工具 */}
                      {evolutionTools.length > 0 && (
                        <div className="mt-5">
                          <div className="text-[12px] font-medium text-inkdim mb-2 flex items-center justify-between">
                            <div className="flex items-center gap-1.5">
                              <Zap size={14} className="text-purple-400" />
                              <span className="text-ink">自演化技能引擎工具</span>
                              <span className="text-[11px] text-inkdim/60">
                                ({evolutionTools.filter((t) => isToolEnabled(t.name)).length}/{evolutionTools.length})
                              </span>
                            </div>
                            <span className="text-[11px] text-purple-400/80">用于 Agent 编写并沉淀项目自演化脚本</span>
                          </div>
                          <div className="flex flex-col gap-2">
                            {evolutionTools.map(renderToolItem)}
                          </div>
                        </div>
                      )}

                      {/* 临时空间工具 */}
                      {tempTools.length > 0 && (
                        <div className="mt-5">
                          <div className="text-[12px] font-medium text-inkdim mb-2 flex items-center justify-between">
                            <div className="flex items-center gap-1.5">
                              <Wind size={14} className="text-amber-400" />
                              <span className="text-ink">临时空间专用工具</span>
                              <span className="text-[11px] text-inkdim/60">
                                ({tempTools.filter((t) => isToolEnabled(t.name)).length}/{tempTools.length})
                              </span>
                            </div>
                            <span className="text-[11px] text-amber-400/80">仅在临时空间对话中下发</span>
                          </div>
                          <div className="flex flex-col gap-2">
                            {tempTools.map(renderToolItem)}
                          </div>
                        </div>
                      )}
                    </>
                  )}
                </section>
              </div>
            )}

            {tab === "sop" && (
              <div className="flex flex-col gap-5">
                <section>
                  <div className="flex items-start justify-between gap-4 mb-3">
                    <div className="flex-1 min-w-0">
                      <div className="font-medium text-[14px]">Agent SOP 规范管理</div>
                      <div className="text-[12px] text-inkdim mt-0.5 leading-relaxed max-w-[520px]">
                        配置 Agent 在执行任务时遵循的标准作业程序规范（SOP）。禁用后模型将免除对应的硬性提示词约束与相关自动化行为。
                      </div>
                    </div>
                    <div className="flex items-center gap-2.5 shrink-0 pt-0.5">
                      <span className="text-[12px] text-inkdim font-medium whitespace-nowrap bg-panel3 px-2 py-0.5 rounded-md border border-edge/60">
                        已启用 {AGENT_SOPS.length - disabledSopCount} / {AGENT_SOPS.length}
                      </span>
                      {disabledSopCount > 0 && (
                        <button
                          className="text-[12px] text-accent hover:underline whitespace-nowrap"
                          onClick={enableAllSops}
                        >
                          全部启用
                        </button>
                      )}
                    </div>
                  </div>

                  {/* SOP 体系说明卡片 */}
                  <div className="p-3 rounded-xl border border-edge/80 bg-panel3/40 flex items-start gap-3 mt-1 mb-2 text-[12px] leading-relaxed">
                    <ShieldCheck size={16} className="text-accent shrink-0 mt-0.5" />
                    <div className="flex flex-col gap-1">
                      <div className="font-medium text-ink">标准作业程序 (SOP) 与自动化规范说明</div>
                      <div className="text-inkdim leading-relaxed">
                        SOP 是 Agent 保持谨慎、工程规范与高质量交付的底层约束。关闭某项规范后，Agent 将在该维度解除硬性规则束缚（例如：关闭方案先行后可直接动手编码；关闭边读边记后将不强制记录并跳过任务完成后的后台自动提炼）。
                      </div>
                    </div>
                  </div>

                  <div className="flex flex-col gap-2 mt-4">
                    {AGENT_SOPS.map(renderSopItem)}
                  </div>
                </section>
              </div>
            )}

            {tab === "datadir" && (
              /* 数据目录（独立生效，不随「应用 / 保存」提交） */
              <section>
                <div className="font-medium mb-2">数据目录</div>
                <DataDirSection />
              </section>
            )}
          </main>
        </div>
        <ModalActions
          onCancel={() => setShow(false)}
          onApply={() => void apply()}
          onSave={() => void save()}
          canApply={dirty}
          busy={saving}
        />
      </div>

      {editingContextModel && (
        <ModelContextModal
          open={true}
          providerId={editingContextModel.providerId}
          providerName={editingContextModel.providerName}
          modelName={editingContextModel.modelName}
          currentLimit={editingContextModel.currentLimit}
          onSave={handleSaveModelContext}
          onClose={() => setEditingContextModel(null)}
        />
      )}
    </div>
  );
}
