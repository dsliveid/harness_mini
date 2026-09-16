import { useEffect, useRef } from "react";
import { currentMessages, currentSession, useStore } from "../store";
import { DRAFT_ID } from "../types";
import { FloatingTaskPanel } from "./FloatingTaskPanel";
import { GrowthCard } from "./GrowthCard";
import { MessageItem } from "./MessageItem";
import { Bot, Plus, Sprout, ShieldCheck, AlertTriangle, Loader2, RefreshCw } from "./Icons";

export function ChatView() {
  const currentId = useStore((s) => s.currentId);
  const msgs = useStore((s) => currentMessages(s));
  const session = useStore((s) => currentSession(s));
  const running = useStore((s) => (s.currentId ? s.runStatus[s.currentId] === "running" : false));
  const readOnly = useStore((s) => s.readOnly);
  const hasMore = useStore((s) => (s.currentId ? s.hasMore[s.currentId] ?? false : false));
  const loadEarlier = useStore((s) => s.loadEarlier);
  const newDraft = useStore((s) => s.newDraft);
  const setShowSettings = useStore((s) => s.setShowSettings);
  const providers = useStore((s) => s.settings.providers);
  const projects = useStore((s) => s.projects);
  const draft = useStore((s) => s.draft);
  const proposals = useStore((s) => (s.currentId ? s.activeProposals[s.currentId] ?? [] : []));
  const currentGrowthStatus = useStore((s) => (s.currentId ? s.growthStatus[s.currentId] : null));
  const currentSopStatus = useStore((s) => (s.currentId ? s.sopStatus[s.currentId] : null));
  const currentToolRetry = useStore((s) => (s.currentId ? s.toolRetryStatus[s.currentId] : null));
  // 临时空间对话：合并点（含）之前的消息永久不可编辑重发
  const mergedBoundary = session?.mergedSeq ?? null;

  const boxRef = useRef<HTMLDivElement>(null);
  const stick = useRef(true);

  useEffect(() => {
    const el = boxRef.current;
    if (el && stick.current) {
      el.scrollTop = el.scrollHeight;
    }
  }, [msgs, running]);

  const onScroll = () => {
    const el = boxRef.current;
    if (!el) return;
    stick.current = el.scrollHeight - el.scrollTop - el.clientHeight < 100;
  };

  if (!currentId || (currentId === DRAFT_ID && msgs.length === 0)) {
    const noProvider = providers.every((p) => !(p.models ?? []).length);
    const noWorkspace = currentId === DRAFT_ID && !draft?.workspacePath;
    const isTempDraft = currentId === DRAFT_ID && !!draft?.temp;
    const draftProject = draft?.projectId ? projects.find((p) => p.id === draft.projectId) : null;
    return (
      <div className="flex-1 flex items-center justify-center overflow-y-auto select-none">
        <div className="max-w-[480px] px-6 text-center animate-in fade-in zoom-in-95 duration-200">
          <div className="w-14 h-14 rounded-2xl bg-panel2 border border-edge flex items-center justify-center mx-auto mb-4 shadow-lg text-accent">
            <Bot size={28} strokeWidth={1.8} />
          </div>
          <div className="text-lg font-semibold mb-2 text-ink">{draftProject ? draftProject.name : "harness_mini"}</div>
          {noProvider ? (
            <div className="text-inkdim text-[13px] leading-relaxed">
              尚未配置模型。请先点击左下角 <b className="text-ink">设置</b>，添加一个 OpenAI 兼容厂商（Base URL / API Key / 模型名）。
              <div className="mt-3">
                <button className="px-3.5 py-1.5 rounded-lg bg-accent hover:bg-blue-500 text-white text-[13px] transition-colors shadow-sm" onClick={() => setShowSettings(true)}>
                  打开设置
                </button>
              </div>
            </div>
          ) : noWorkspace ? (
            <div className="text-inkdim text-[13px] leading-relaxed">
              未选择工作区，可直接开始<b className="text-ink">纯对话</b>；如需读写文件、执行命令，请先在顶栏选择已有项目或工作区目录（保存对话时自动归入对应项目）。
              <div className="mt-2 text-[12px] opacity-80">例如：「用通俗的话解释一下 Rust 的所有权机制」</div>
            </div>
          ) : isTempDraft ? (
            <div className="text-inkdim text-[13px] leading-relaxed">
              当前为「{draftProject?.name ?? "项目"}」的<b className="text-ink">临时空间对话</b>。
              发送第一条消息后创建临时空间，并把主项目与关联项目拷贝进去（原目录不受影响，可随时合并或清空）。
              <div className="mt-2 text-[12px] opacity-80">Agent 可以读写临时副本中的文件、执行命令（需确认）、搜索代码。</div>
            </div>
          ) : (
            <div className="text-inkdim text-[13px] leading-relaxed">
              {draftProject
                ? "当前为项目下的临时对话，发送第一条消息后自动保存到该项目。"
                : "当前为临时对话，发送第一条消息后才会保存。"}
              <div className="mt-2 text-[12px] opacity-80">Agent 可以读写文件、执行命令（需确认）、搜索代码。</div>
            </div>
          )}
          {!currentId && (
            <div className="mt-4">
              <button className="inline-flex items-center gap-1.5 px-3.5 py-1.5 rounded-lg bg-accent/15 text-accent hover:bg-accent/25 text-[13px] transition-colors font-medium" onClick={() => newDraft(null)}>
                <Plus size={15} strokeWidth={2} />
                <span>新建会话</span>
              </button>
            </div>
          )}
        </div>
      </div>
    );
  }

  const lastUserMsgId = [...msgs].reverse().find((m) => m.role === "user" && !m.queued)?.id;

  return (
    <div className="flex-1 min-h-0 relative flex flex-col">
      <FloatingTaskPanel />
      <div className="flex-1 overflow-y-auto" ref={boxRef} onScroll={onScroll}>
        <div className="max-w-[820px] mx-auto px-4 py-6 flex flex-col gap-5">
          {hasMore && (
            <button className="self-center text-[12px] text-inkdim hover:text-ink px-3 py-1 rounded-lg hover:bg-panel2" onClick={() => void loadEarlier(currentId)}>
              加载更早的消息
            </button>
          )}
          {msgs
            .filter((m) => m.role !== "tool")
            .map((m) => (
              <MessageItem
                key={m.id}
                msg={m}
                isLastUser={m.id === lastUserMsgId}
                streaming={running && m.role === "assistant" && m.id === msgs[msgs.length - 1]?.id}
                readOnly={readOnly}
                running={running}
                editBlocked={mergedBoundary != null && m.seq <= mergedBoundary}
              />
            ))}
          {/* 当前会话待审阅的成长提案卡片 */}
          {proposals.map((item) => (
            <GrowthCard key={item.id} item={item} />
          ))}
          {/* 经验反思提炼进行中指示 */}
          {currentGrowthStatus?.status === "analyzing" && (
            <div className="rounded-xl border border-emerald-500/30 bg-emerald-500/10 p-3 flex items-center gap-2.5 text-[13px] text-emerald-400 shadow-sm animate-in fade-in duration-200">
              <Sprout size={16} className="shrink-0 animate-pulse text-emerald-400" />
              <span className="font-medium">{currentGrowthStatus.message || "AI 正在反思并提炼经验规则..."}</span>
            </div>
          )}
          {/* 交付前 SOP 自检状态指示 */}
          {currentSopStatus && running && currentSopStatus.status === "checking" && (
            <div className="rounded-xl border border-blue-500/30 bg-blue-500/10 p-3 flex items-center justify-between text-[13px] text-blue-400 shadow-sm">
              <div className="flex items-center gap-2.5">
                <ShieldCheck size={16} className="shrink-0 text-blue-400 animate-pulse" />
                <span className="font-medium">交付前 SOP 自检中: <code className="bg-panel2 px-1.5 py-0.5 rounded text-[12px] font-mono border border-blue-500/20">{currentSopStatus.command}</code></span>
              </div>
              <span className="text-[11px] text-blue-300/80 hidden sm:inline">检测到代码已修改，正在执行编译/测试...</span>
            </div>
          )}
          {currentSopStatus && currentSopStatus.status === "failed" && running && (
            <div className="rounded-xl border border-amber-500/30 bg-amber-500/10 p-3 flex items-center justify-between text-[13px] text-amber-400 shadow-sm">
              <div className="flex items-center gap-2.5">
                <AlertTriangle size={16} className="shrink-0 text-amber-400" />
                <span className="font-medium">交付自检未通过 (<code className="bg-panel2 px-1.5 py-0.5 rounded text-[12px] font-mono border border-amber-500/20">{currentSopStatus.command}</code>)，Agent 正在自动排查并自愈修复...</span>
              </div>
            </div>
          )}
          {currentSopStatus && currentSopStatus.status === "passed" && !running && (
            <div className="rounded-xl border border-emerald-500/30 bg-emerald-500/10 px-3 py-2 flex items-center gap-2 text-[12px] text-emerald-400 shadow-sm">
              <ShieldCheck size={15} className="shrink-0 text-emerald-400" />
              <span>交付前 SOP 自检已通过 (<code className="bg-panel2 px-1.5 py-0.5 rounded text-[12px] font-mono text-emerald-300 border border-emerald-500/20">{currentSopStatus.command}</code>)</span>
            </div>
          )}
          {/* 工具报错自动自纠错状态指示 */}
          {currentToolRetry && running && currentToolRetry.active && (
            <div className="rounded-xl border border-purple-500/30 bg-purple-500/10 p-3 flex items-center justify-between text-[13px] text-purple-300 shadow-sm animate-in fade-in duration-200">
              <div className="flex items-center gap-2.5">
                <RefreshCw size={15} className="shrink-0 text-purple-400 animate-spin" />
                <span className="font-medium">
                  工具 <code className="bg-panel2 px-1.5 py-0.5 rounded text-[12px] font-mono border border-purple-500/20 text-purple-200">{currentToolRetry.toolName}</code> 执行遇阻，Agent 正在内部自纠修正重试 ({currentToolRetry.attempt}/{currentToolRetry.maxRetries})...
                </span>
              </div>
              <span className="text-[11px] text-purple-300/70 hidden sm:inline">自动自愈中</span>
            </div>
          )}
          {running && msgs.length === 0 && (
            <div className="text-inkdim text-[13px] flex items-center gap-2 py-2">
              <Loader2 size={15} className="animate-spin text-accent" />
              <span>Agent 正在思考…</span>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
