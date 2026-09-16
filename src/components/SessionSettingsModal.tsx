import { useEffect, useState } from "react";
import { ipc } from "../ipc";
import { useStore } from "../store";
import { ModalClose } from "./ModalActions";
import { askConfirm } from "./PromptModal";
import { KeyRound, Shield, Trash2 } from "./Icons";

type Tab = "mode" | "rules";

const inputCls = "bg-panel border border-edge rounded-lg px-2 py-1.5 text-[13px] outline-none focus:border-accent";

/** 规则类型的中文名（与后端 ApprovalRule.kind 对应） */
const kindLabel = (kind: string) =>
  kind === "command_prefix" ? "命令前缀" : kind === "path_write" ? "路径写入" : "工具";

/**
 * 会话设置：访问模式 + 本对话的审批规则。
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
        <div className="px-5 py-4 border-b border-edge font-medium flex items-center gap-2 shrink-0">
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
          </main>
        </div>

        <div className="px-5 py-3.5 border-t border-edge flex justify-end gap-2 shrink-0">
          <button className="px-4 py-1.5 rounded-lg bg-panel3 hover:bg-edge text-ink" onClick={() => close(null)}>
            关闭
          </button>
        </div>
      </div>
    </div>
  );
}
