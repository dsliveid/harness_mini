import { useEffect, useState } from "react";
import { ipc } from "../ipc";
import { useStore } from "../store";
import type { Provider, Settings } from "../types";
import { DataDirSection } from "./DataDirSection";
import { ModalActions, ModalClose } from "./ModalActions";

type Tab = "models" | "datadir";

export function SettingsModal() {
  const show = useStore((s) => s.showSettings);
  const setShow = useStore((s) => s.setShowSettings);
  const storeSettings = useStore((s) => s.settings);
  const setSettingsLocal = useStore((s) => s.setSettingsLocal);
  const pushToast = useStore((s) => s.pushToast);
  const [tab, setTab] = useState<Tab>("models");
  const [local, setLocal] = useState<Settings>(storeSettings);
  const [testing, setTesting] = useState<Record<string, string>>({});
  const [saving, setSaving] = useState(false);
  const [visibleKeys, setVisibleKeys] = useState<Record<string, boolean>>({});
  const [copiedId, setCopiedId] = useState<string | null>(null);
  const [newModel, setNewModel] = useState<Record<string, string>>({});

  useEffect(() => {
    if (show) {
      setTab("models");
      setLocal(structuredClone(storeSettings));
      setTesting({});
      setVisibleKeys({});
      setCopiedId(null);
      setNewModel({});
      // 打开时从后端拉取最新设置，避免草稿基于启动时的旧缓存回写
      ipc
        .getSettings()
        .then((fresh) => {
          setSettingsLocal(fresh);
          setLocal(structuredClone(fresh));
        })
        .catch((e) => pushToast(String(e)));
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [show]);

  if (!show) return null;

  // 是否有待保存的改动（决定「应用」可用性）
  const dirty = JSON.stringify(local) !== JSON.stringify(storeSettings);

  const updateProvider = (id: string, patch: Partial<Provider>) => {
    setLocal((cur) => ({ ...cur, providers: cur.providers.map((p) => (p.id === id ? { ...p, ...patch } : p)) }));
  };

  const addProvider = () => {
    const p: Provider = {
      id: Math.random().toString(36).slice(2),
      name: "新厂商",
      baseUrl: "https://open.bigmodel.cn/api/paas/v4",
      models: ["glm-4.6"],
      apiKey: "",
    };
    setLocal((cur) => ({
      ...cur,
      providers: [...cur.providers, p],
      activeProviderId: cur.activeProviderId ?? p.id,
      activeModelId: cur.activeProviderId ? cur.activeModelId : p.models[0] ?? null,
    }));
  };

  // 选择默认厂商：默认使用该厂商的第一个模型
  const selectProvider = (id: string) => {
    setLocal((cur) => ({
      ...cur,
      activeProviderId: id,
      activeModelId: cur.providers.find((x) => x.id === id)?.models[0] ?? null,
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
    updateProvider(providerId, { models: [...p.models, name] });
  };

  const removeModel = (providerId: string, model: string) => {
    const p = local.providers.find((x) => x.id === providerId);
    if (!p) return;
    const models = p.models.filter((m) => m !== model);
    updateProvider(providerId, { models });
    // 删除的是激活模型时，回落到该厂商剩余的第一个模型
    setLocal((cur) => ({
      ...cur,
      activeModelId: cur.activeProviderId === providerId && cur.activeModelId === model ? models[0] ?? null : cur.activeModelId,
    }));
  };

  const removeProvider = (id: string) => {
    setLocal((cur) => {
      const providers = cur.providers.filter((x) => x.id !== id);
      if (cur.activeProviderId !== id) return { ...cur, providers };
      return {
        ...cur,
        providers,
        activeProviderId: providers[0]?.id ?? null,
        activeModelId: providers[0]?.models[0] ?? null,
      };
    });
  };

  // 应用：只保存数据，不关闭窗口
  const apply = async (): Promise<boolean> => {
    setSaving(true);
    try {
      await ipc.setSettings(local);
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
          <aside className="w-[150px] shrink-0 border-r border-edge p-2 flex flex-col gap-1">
            <button className={menuCls(tab === "models")} onClick={() => setTab("models")}>
              🤖 模型设置
            </button>
            <button className={menuCls(tab === "datadir")} onClick={() => setTab("datadir")}>
              📂 数据目录
            </button>
          </aside>
          <main className="flex-1 min-w-0 min-h-0 overflow-y-auto p-4 text-[13px]">
            {tab === "models" && (
              <div className="flex flex-col gap-6">
                {/* 模型厂商 */}
                <section>
                  <div className="flex items-center mb-2">
                    <div className="font-medium">模型厂商（OpenAI 兼容）</div>
                    <button className="ml-auto text-accent hover:underline" onClick={addProvider}>
                      + 添加厂商
                    </button>
                  </div>
                  <div className="flex flex-col gap-3">
                    {local.providers.map((p) => (
                      <div key={p.id} className="border border-edge rounded-xl p-3 bg-panel">
                        <div className="grid grid-cols-2 gap-2 mb-2">
                          <label className="flex items-center gap-2">
                            <span className="text-inkdim shrink-0 w-14">厂商名称</span>
                            <input className={inputCls} value={p.name} onChange={(e) => updateProvider(p.id, { name: e.target.value })} />
                          </label>
                          <label className="flex items-center gap-2">
                            <span className="text-inkdim shrink-0 w-14">Base URL</span>
                            <input className={inputCls} value={p.baseUrl} onChange={(e) => updateProvider(p.id, { baseUrl: e.target.value })} />
                          </label>
                        </div>
                        <div className="flex items-center gap-2 mb-2">
                          <span className="text-inkdim shrink-0 w-14">API Key</span>
                          <input
                            className={`${inputCls} flex-1 min-w-0`}
                            type={visibleKeys[p.id] ? "text" : "password"}
                            value={p.apiKey}
                            placeholder="sk-…"
                            onChange={(e) => updateProvider(p.id, { apiKey: e.target.value })}
                          />
                          <button
                            className="shrink-0 w-7 h-7 rounded-lg bg-panel3 hover:bg-edge flex items-center justify-center text-[13px] text-inkdim hover:text-ink"
                            title={visibleKeys[p.id] ? "隐藏" : "明文显示"}
                            onClick={() => setVisibleKeys((v) => ({ ...v, [p.id]: !v[p.id] }))}
                          >
                            {visibleKeys[p.id] ? "🙈" : "👁"}
                          </button>
                          <button
                            className={`shrink-0 w-7 h-7 rounded-lg bg-panel3 hover:bg-edge flex items-center justify-center text-[13px] ${
                              copiedId === p.id ? "text-green-400" : "text-inkdim hover:text-ink"
                            } ${p.apiKey ? "" : "opacity-40 cursor-not-allowed"}`}
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
                            {copiedId === p.id ? "✓" : "📋"}
                          </button>
                        </div>
                        {/* 该厂商下的模型列表（可配置多个） */}
                        <div className="mb-2">
                          <div className="text-inkdim mb-1.5">模型（同一厂商可配置多个）</div>
                          <div className="flex flex-col gap-1 mb-1.5">
                            {p.models.map((m) => (
                              <div key={m} className="flex items-center gap-2 bg-panel2 border border-edge rounded-lg px-2.5 py-1.5">
                                <span className="font-mono text-[12px] flex-1 truncate">{m}</span>
                                {local.activeProviderId === p.id && local.activeModelId === m && (
                                  <span className="text-[11px] text-green-400 shrink-0">当前使用</span>
                                )}
                                <button
                                  className="text-[12px] text-red-400 hover:underline shrink-0"
                                  onClick={() => removeModel(p.id, m)}
                                >
                                  删除
                                </button>
                              </div>
                            ))}
                            {p.models.length === 0 && <div className="text-[12px] text-amber-400">请至少添加一个模型</div>}
                          </div>
                          <div className="flex gap-2">
                            <input
                              className={inputCls}
                              placeholder="输入模型名，如 glm-4.6"
                              value={newModel[p.id] ?? ""}
                              onChange={(e) => setNewModel((m) => ({ ...m, [p.id]: e.target.value }))}
                              onKeyDown={(e) => {
                                if (e.key === "Enter") addModel(p.id);
                              }}
                            />
                            <button
                              className="shrink-0 text-[12px] px-3 rounded-lg bg-panel3 hover:bg-edge text-inkdim hover:text-ink"
                              onClick={() => addModel(p.id)}
                            >
                              + 添加模型
                            </button>
                          </div>
                        </div>
                        <div className="flex items-center gap-2">
                          <label className="flex items-center gap-1.5 text-inkdim">
                            <input
                              type="radio"
                              checked={local.activeProviderId === p.id}
                              onChange={() => selectProvider(p.id)}
                            />
                            使用此厂商
                          </label>
                          <button
                            className="ml-auto text-[12px] px-2 py-1 rounded-lg bg-panel3 hover:bg-edge text-inkdim"
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
                          <button className="text-[12px] px-2 py-1 rounded-lg bg-panel3 hover:bg-edge text-red-400" onClick={() => removeProvider(p.id)}>
                            删除
                          </button>
                        </div>
                        {testing[p.id] && (
                          <div className={`mt-2 text-[12px] ${testing[p.id].startsWith("连接成功") ? "text-green-400" : "text-red-400"}`}>
                            {testing[p.id]}
                          </div>
                        )}
                      </div>
                    ))}
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
