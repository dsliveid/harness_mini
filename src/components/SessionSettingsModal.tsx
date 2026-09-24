import { useEffect, useState } from "react";
import { ipc } from "../ipc";
import { useStore } from "../store";
import { ModalClose } from "./ModalActions";
import { askConfirm } from "./PromptModal";
import { KeyRound, Shield, Trash2, Sliders, RotateCcw } from "./Icons";
import { formatTokens, resolveModelContextLimit, MODEL_CONTEXT_PRESETS } from "../types";

type Tab = "mode" | "rules" | "context";

const inputCls = "bg-panel border border-edge rounded-lg px-2 py-1.5 text-[13px] outline-none focus:border-accent";

/** 规则类型的中文名（与后端 ApprovalRule.kind 对应） */
const kindLabel = (kind: string) =>
  kind === "command_prefix" ? "命令前缀" : kind === "path_write" ? "路径写入" : "工具";

/**
 * 会话设置：访问模式 + 本对话的审批规则 + 上下文上限。
 * 这里的改动均即时生效（不走「应用 / 保存」），与顶栏切换共用同一份会话状态。
 */
export function SessionSettingsModal() {
  const sessionId = useStore((s) => s.sessionSettingsId);
  const close = useStore((s) => s.setSessionSettings);
  const sessions = useStore((s) => s.sessions);
  const settings = useStore((s) => s.settings);
  const rules = useStore((s) => (s.sessionSettingsId ? s.sessionRules[s.sessionSettingsId] : undefined));
  const refreshRules = useStore((s) => s.refreshSessionRules);
  const pushToast = useStore((s) => s.pushToast);
  const [tab, setTab] = useState<Tab>("mode");
  const [busy, setBusy] = useState(false);

  const session = sessions.find((s) => s.id === sessionId);

  useEffect(() => {
    if (!sessionId) return;
    setTab("mode");
    void refreshRules(sessionId);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sessionId]);

  if (!sessionId || !session) return null;

  // 生效模式：会话显式设定优先，未设定时回落全局默认值（与后端判定一致）
  const mode = (session.accessMode ?? settings.globalAccessMode) as "confirm" | "full_access";

  const currentPid = session.providerId ?? session.provider_id ?? settings.activeProviderId ?? null;
  const currentMid = session.modelId ?? session.model_id ?? settings.activeModelId ?? settings.activeModel ?? "默认模型";
  const defaultLimit = resolveModelContextLimit(settings, currentPid, currentMid);
  const effectiveLimit = session.contextTokenLimit ?? defaultLimit;
  const isCustomSession = session.contextTokenLimit != null;

  const changeMode = async (next: "confirm" | "full_access") => {
    if (next === "full_access") {
      const confirmed = await askConfirm(
        "完全访问模式将跳过本对话的所有审批（包括命令执行与高危操作），确定开启？",
        "开启",
      );
      if (!confirmed) return;
    }
    setBusy(true);
    try {
      await ipc.setSessionMode(session.id, next);
    } catch (e) {
      pushToast(String(e));
    }
    setBusy(false);
  };

  const removeRule = async (id: string) => {
    try {
      await ipc.deleteSessionRule(session.id, id);
    } catch (e) {
      pushToast(String(e));
    }
  };

  const menuCls = (active: boolean) =>
    `w-full text-left px-3 py-2 rounded-lg text-[13px] ${
      active ? "bg-panel3 text-ink" : "text-inkdim hover:text-ink hover:bg-panel2"
    }`;

  return (
    <div className="fixed inset-0 z-[80] bg-black/50 flex items-center justify-center">
      <div className="bg-panel2 border border-edge rounded-2xl w-[720px] h-[460px] max-w-[92vw] max-h-[86vh] flex flex-col shadow-2xl">
        <div className="h-12 px-5 border-b border-edge/60 font-medium flex items-center gap-2 shrink-0">
          <span>会话设置</span>
          <span className="text-[13px] text-inkdim font-normal truncate">· {session.title}</span>
          <ModalClose onClick={() => close(null)} />
        </div>

        <div className="flex-1 flex min-h-0">
          <aside className="w-[150px] shrink-0 border-r border-edge p-2 flex flex-col gap-1">
            <button className={menuCls(tab === "mode")} onClick={() => setTab("mode")}>
              <span className="flex items-center gap-2">
                <KeyRound size={15} />
                <span>访问模式</span>
              </span>
            </button>
            <button className={menuCls(tab === "rules")} onClick={() => setTab("rules")}>
              <span className="flex items-center gap-2">
                <Shield size={15} />
                <span>审批规则</span>
              </span>
            </button>
            <button className={menuCls(tab === "context")} onClick={() => setTab("context")}>
              <span className="flex items-center gap-2">
                <Sliders size={15} />
                <span>上下文上限</span>
              </span>
            </button>
          </aside>

          <main className="flex-1 min-w-0 min-h-0 overflow-y-auto p-4 text-[13px]">
            {tab === "mode" && (
              <section className="max-w-[480px]">
                <div className="font-medium mb-2">访问模式</div>
                <select
                  className={`${inputCls} w-full`}
                  value={mode}
                  disabled={busy}
                  onChange={(e) => void changeMode(e.target.value as "confirm" | "full_access")}
                >
                  <option value="confirm">变更前确认</option>
                  <option value="full_access">完全访问</option>
                </select>
                <div className="text-[12px] text-inkdim mt-2 leading-relaxed">
                  · 在此设定后固定为当前对话的模式，不受全局默认值影响。
                  <br />· 对话进行中切换同样立即生效：后续工具调用按新模式判定；切到「完全访问」时，已经弹出的审批条会被自动放行。
                </div>
                {mode === "full_access" ? (
                  <div className="mt-3 text-[12px] text-amber-400 leading-relaxed">
                    当前为完全访问：所有工具调用自动放行（含高危命令），仅在工具卡片留痕。
                  </div>
                ) : (
                  <div className="mt-3 text-[12px] text-inkdim leading-relaxed">
                    变更前确认：只读工具自动执行；写入与命令执行需审批，可在审批条选「本会话允许」记下规则减少打断（高危命令仍逐次确认）。
                  </div>
                )}
              </section>
            )}

            {tab === "rules" && (
              <section>
                <div className="flex items-center gap-2 mb-2">
                  <span className="font-medium">审批规则</span>
                  <span className="text-[11px] text-inkdim">
                    仅对当前对话生效；删除后同类调用会重新进入审批
                  </span>
                </div>
                <div className="flex flex-col gap-1">
                  {(rules ?? []).map((r) => (
                    <div
                      key={r.id}
                      className="flex items-center gap-2 bg-panel border border-edge rounded-lg px-3 py-2"
                    >
                      <span className="text-[12px] text-inkdim w-20 shrink-0">{kindLabel(r.kind)}</span>
                      <span className="font-mono text-[12px] flex-1 truncate" title={r.pattern}>
                        {r.pattern}
                      </span>
                      <button
                        className="text-[12px] text-red-400 hover:text-red-300 hover:underline shrink-0 flex items-center gap-1 transition-colors"
                        onClick={() => void removeRule(r.id)}
                      >
                        <Trash2 size={11} />
                        <span>删除</span>
                      </button>
                    </div>
                  ))}
                  {rules && rules.length === 0 && (
                    <div className="text-inkdim">
                      暂无规则。审批时选「本会话允许」会向当前对话添加规则。
                    </div>
                  )}
                  {!rules && <div className="text-inkdim">加载中…</div>}
                </div>
              </section>
            )}

            {tab === "context" && (
              <section className="max-w-[480px] flex flex-col gap-4">
                <div>
                  <div className="font-medium text-[14px]">本次对话上下文上限</div>
                  <div className="text-[12px] text-inkdim mt-0.5 leading-relaxed">
                    可针对当前对话单独调整 Token 上限。超出 75% 时触发智能压缩，超出上限触发原子轮次截断。
                  </div>
                </div>

                {/* 模型信息条 */}
                <div className="flex items-center justify-between px-3 py-2 rounded-xl bg-panel3/50 border border-edge/60 text-[12px]">
                  <div className="flex flex-col min-w-0">
                    <span className="text-[11px] text-inkdim">当前对话使用模型</span>
                    <span className="font-mono font-medium text-ink truncate">{currentMid}</span>
                  </div>
                  <div className="text-[11px] text-inkdim text-right">
                    <div>模型默认: <span className="font-mono text-ink">{formatTokens(defaultLimit)}</span></div>
                    <div className="text-[10px] text-inkdim/70">({defaultLimit.toLocaleString()} tokens)</div>
                  </div>
                </div>

                {/* 快捷规格标签 */}
                <div className="flex flex-col gap-1.5">
                  <label className="text-[12px] font-medium text-ink flex items-center justify-between">
                    <span>快捷规格</span>
                    <span className="text-[11px] text-inkdim font-normal">点击快速应用至本对话</span>
                  </label>
                  <div className="flex flex-wrap items-center gap-1.5">
                    {[
                      { label: "1M (800k)", val: 800_000 },
                      { label: "500k (400k)", val: 400_000 },
                      { label: "200k (180k)", val: 180_000 },
                      { label: "128k (110k)", val: 110_000 },
                      { label: "64k (56k)", val: 56_000 },
                      { label: "32k (28k)", val: 28_000 },
                      { label: "16k (14k)", val: 14_000 },
                      { label: "8k (7k)", val: 7_000 },
                    ].map((chip) => (
                      <button
                        key={chip.val}
                        type="button"
                        className={`text-[11px] px-2 py-0.5 rounded-lg border font-mono transition-all cursor-pointer ${
                          effectiveLimit === chip.val
                            ? "bg-accent/20 border-accent text-accent font-semibold"
                            : "bg-panel border-edge hover:border-accent/40 text-inkdim hover:text-ink"
                        }`}
                        onClick={async () => {
                          try {
                            await ipc.setSessionContextLimit(session.id, chip.val);
                            pushToast(`已设置本次对话上限为 ${chip.label}`);
                          } catch (e) {
                            pushToast(String(e));
                          }
                        }}
                      >
                        {chip.label}
                      </button>
                    ))}
                  </div>
                </div>

                {/* 下拉预设选择 */}
                <div className="flex flex-col gap-1.5">
                  <label className="text-[12px] font-medium text-ink flex items-center justify-between">
                    <span>从主流模型规格列表中选择</span>
                    <span className="text-[11px] text-inkdim font-normal">快速填入推荐阈值</span>
                  </label>
                  <select
                    className={`${inputCls} text-[12px] cursor-pointer`}
                    value=""
                    onChange={async (e) => {
                      const lim = Number(e.target.value);
                      if (lim > 0) {
                        try {
                          await ipc.setSessionContextLimit(session.id, lim);
                          pushToast(`已应用规格上限: ${formatTokens(lim)}`);
                        } catch (err) {
                          pushToast(String(err));
                        }
                      }
                    }}
                  >
                    <option value="" disabled>
                      从主流模型规格列表中选择...
                    </option>
                    {Array.from(new Set(MODEL_CONTEXT_PRESETS.map((p) => p.category))).map((cat) => (
                      <optgroup key={cat} label={cat}>
                        {MODEL_CONTEXT_PRESETS.filter((p) => p.category === cat).map((p) => (
                          <option key={p.id} value={p.recommendedLimit}>
                            {p.name}（{p.desc}）
                          </option>
                        ))}
                      </optgroup>
                    ))}
                  </select>
                </div>

                {/* 手动输入 */}
                <div className="flex flex-col gap-1.5">
                  <label className="text-[12px] font-medium text-ink flex items-center justify-between">
                    <span>手动精确数值（Tokens）</span>
                    <span className="text-[11px] font-mono text-accent font-medium">
                      {formatTokens(effectiveLimit)} ({effectiveLimit.toLocaleString()} tokens)
                    </span>
                  </label>
                  <div className="flex items-center gap-2">
                    <input
                      className={`${inputCls} w-full font-mono`}
                      type="number"
                      min={2000}
                      max={10000000}
                      step={1000}
                      value={effectiveLimit}
                      onChange={async (e) => {
                        const num = Number(e.target.value);
                        if (!num) return;
                        const newLim = Math.max(2000, Math.min(num, 10000000));
                        try {
                          await ipc.setSessionContextLimit(session.id, newLim);
                        } catch (err) {
                          pushToast(String(err));
                        }
                      }}
                    />
                    <span className="text-[12px] text-inkdim shrink-0">tokens</span>
                  </div>
                </div>

                {/* 状态与重置 */}
                <div className="p-3 rounded-xl bg-panel3/30 border border-edge/50 flex items-center justify-between text-[11px]">
                  <div className="text-inkdim">
                    当前状态：
                    {isCustomSession ? (
                      <span className="text-amber-400 font-medium">★ 本对话专属自定义数值</span>
                    ) : (
                      <span className="text-emerald-400">跟随模型默认设定</span>
                    )}
                  </div>
                  {isCustomSession && (
                    <button
                      type="button"
                      className="px-2.5 py-1 rounded-lg bg-panel hover:bg-edge border border-edge text-inkdim hover:text-ink transition-colors flex items-center gap-1 cursor-pointer"
                      onClick={async () => {
                        try {
                          await ipc.setSessionContextLimit(session.id, null);
                          pushToast("已恢复跟随模型默认设定");
                        } catch (e) {
                          pushToast(String(e));
                        }
                      }}
                    >
                      <RotateCcw size={11} />
                      <span>恢复模型默认</span>
                    </button>
                  )}
                </div>
              </section>
            )}
          </main>
        </div>

        <div className="h-12 px-5 border-t border-edge/60 flex items-center justify-end gap-2 shrink-0">
          <button className="px-4 py-1.5 rounded-lg bg-panel3 hover:bg-edge text-ink" onClick={() => close(null)}>
            关闭
          </button>
        </div>
      </div>
    </div>
  );
}
