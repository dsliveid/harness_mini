import { useEffect, useState } from "react";
import { ipc } from "../ipc";
import { useStore } from "../store";
import type { Provider, Settings, ToolInfo } from "../types";
import { DataDirSection } from "./DataDirSection";
import { ModalActions, ModalClose } from "./ModalActions";
import {
  Cpu,
  Wrench,
  Database,
  Plus,
  Eye,
  EyeOff,
  Copy,
  Check,
  Terminal,
  Wind,
} from "./Icons";

type Tab = "models" | "tools" | "datadir";

const TOOL_TITLES: Record<string, string> = {
  read_file: "读取文件",
  list_dir: "列出目录",
  glob: "查找文件",
  grep: "搜索内容",
  write_file: "写入文件",
  edit_file: "编辑文件",
  run_command: "执行命令",
  todo: "任务清单",
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
  const [tab, setTab] = useState<Tab>("models");
  const [local, setLocal] = useState<Settings>(storeSettings);
  const [toolList, setToolList] = useState<ToolInfo[]>([]);
  const [testing, setTesting] = useState<Record<string, string>>({});
  const [saving, setSaving] = useState(false);
  const [visibleKeys, setVisibleKeys] = useState<Record<string, boolean>>({});
  const [copiedId, setCopiedId] = useState<string | null>(null);
  const [newModel, setNewModel] = useState<Record<string, string>>({});
  const [expandedIds, setExpandedIds] = useState<Record<string, boolean>>({});

  useEffect(() => {
    if (show) {
      setTab("models");
      setLocal(structuredClone(storeSettings));
      setTesting({});
      setVisibleKeys({});
      setCopiedId(null);
      setNewModel({});
      setExpandedIds({});
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

  const baseTools = toolList.filter((t) => !t.isTemp);
  const tempTools = toolList.filter((t) => t.isTemp);
  const disabledCount = (local.disabledTools ?? []).length;

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
            {t.isTemp && (
              <span className="text-[11px] px-1.5 py-0.5 rounded bg-purple-500/10 text-purple-400 border border-purple-500/20">
                临时空间
              </span>
            )}
          </div>
          <div className="text-[12px] text-inkdim mt-1.5 leading-relaxed">{t.description}</div>
        </div>

        <div className="flex items-center gap-2 shrink-0 pt-0.5">
          <span className={`text-[12px] select-none ${enabled ? "text-green-400 font-medium" : "text-inkdim"}`}>
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

  const inputCls = "bg-panel border border-edge rounded-lg px-2 py-1.5 text-[13px] outline-none focus:border-accent w-full";

  const menuCls = (active: boolean) =>
    `w-full text-left px-3 py-2 rounded-lg text-[13px] ${
      active ? "bg-panel3 text-ink" : "text-inkdim hover:text-ink hover:bg-panel2"
    }`;

  return (
    <div className="fixed inset-0 z-[80] bg-black/50 flex items-center justify-center">
      <div className="bg-panel2 border border-edge rounded-2xl w-[860px] h-[560px] max-w-[92vw] max-h-[86vh] flex flex-col shadow-2xl">
        <div className="px-5 py-4 border-b border-edge font-medium flex items-center gap-2 shrink-0">
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
            <button className={menuCls(tab === "datadir")} onClick={() => setTab("datadir")}>
              <span className="flex items-center gap-2">
                <Database size={15} />
                <span>数据目录</span>
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
                                    return (
                                      <div key={m} className="flex items-center gap-2 bg-panel2 border border-edge rounded-lg px-2.5 py-1.5">
                                        <span className="font-mono text-[12px] flex-1 truncate">{m}</span>
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

                {/* 运行参数 */}
                <section>
                  <div className="font-medium mb-2">运行参数</div>
                  <div className="grid grid-cols-3 gap-3">
                    <label className="flex flex-col gap-1">
                      <span className="text-inkdim">新对话默认访问模式</span>
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
                  </div>
                  <label className="flex flex-col gap-1 mt-3">
                    <span className="text-inkdim">上下文 token 上限（估算，超出自动丢弃最早消息）</span>
                    <input
                      className={inputCls}
                      type="number"
                      min={4000}
                      max={200000}
                      value={local.contextTokenLimit}
                      onChange={(e) => setLocal({ ...local, contextTokenLimit: Number(e.target.value) || 28000 })}
                    />
                  </label>
                </section>
              </div>
            )}

            {tab === "tools" && (
              <div className="flex flex-col gap-5">
                <section>
                  <div className="flex items-center justify-between mb-3">
                    <div>
                      <div className="font-medium text-[14px]">Agent 工具管理</div>
                      <div className="text-[12px] text-inkdim mt-0.5">
                        配置 Agent 可调用的工具。禁用后模型将无法获取和调用该工具。
                      </div>
                    </div>
                    <div className="flex items-center gap-3">
                      <span className="text-[12px] text-inkdim">
                        已启用 {toolList.length - disabledCount} / {toolList.length}
                      </span>
                      {disabledCount > 0 && (
                        <button
                          className="text-[12px] text-accent hover:underline"
                          onClick={enableAllTools}
                        >
                          全部启用
                        </button>
                      )}
                    </div>
                  </div>

                  {toolList.length === 0 ? (
                    <div className="text-inkdim py-8 text-center">正在加载工具列表…</div>
                  ) : (
                    <>
                      {/* 基础工具 */}
                      <div className="mt-4">
                        <div className="text-[12px] font-medium text-inkdim mb-2 flex items-center gap-1.5">
                          <Terminal size={14} className="text-blue-400" />
                          <span>基础工具</span>
                          <span className="text-[11px] text-inkdim/60">
                            ({baseTools.filter((t) => isToolEnabled(t.name)).length}/{baseTools.length})
                          </span>
                        </div>
                        <div className="flex flex-col gap-2">
                          {baseTools.map(renderToolItem)}
                        </div>
                      </div>

                      {/* 临时空间工具 */}
                      {tempTools.length > 0 && (
                        <div className="mt-5">
                          <div className="text-[12px] font-medium text-inkdim mb-2 flex items-center gap-1.5">
                            <Wind size={14} className="text-purple-400" />
                            <span>临时空间专用工具</span>
                            <span className="text-[11px] text-inkdim/60">
                              ({tempTools.filter((t) => isToolEnabled(t.name)).length}/{tempTools.length})
                            </span>
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
    </div>
  );
}
