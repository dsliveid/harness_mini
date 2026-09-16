import { useEffect, useRef } from "react";
import { currentMessages, currentSession, useStore } from "../store";
import { DRAFT_ID } from "../types";
import { FloatingTaskPanel } from "./FloatingTaskPanel";
import { MessageItem } from "./MessageItem";

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
      <div className="flex-1 flex items-center justify-center overflow-y-auto">
        <div className="max-w-[480px] px-6 text-center">
          <div className="text-3xl mb-3">⌨️</div>
          <div className="text-lg font-medium mb-2">{draftProject ? draftProject.name : "harness_mini"}</div>
          {noProvider ? (
            <div className="text-inkdim text-[13px] leading-relaxed">
              尚未配置模型。请先点击左下角 <b className="text-ink">设置</b>，添加一个 OpenAI 兼容厂商（Base URL / API Key / 模型名）。
              <div className="mt-3">
                <button className="px-3 py-1.5 rounded-lg bg-accent hover:bg-blue-500 text-white text-[13px]" onClick={() => setShowSettings(true)}>
                  打开设置
                </button>
              </div>
            </div>
          ) : noWorkspace ? (
            <div className="text-inkdim text-[13px] leading-relaxed">
              未选择工作区，可直接开始<b className="text-ink">纯对话</b>；如需读写文件、执行命令，请先在顶栏选择已有项目或工作区目录（保存对话时自动归入对应项目）。
              <div className="mt-2 text-[12px]">例如：「用通俗的话解释一下 Rust 的所有权机制」</div>
            </div>
          ) : isTempDraft ? (
            <div className="text-inkdim text-[13px] leading-relaxed">
              当前为「{draftProject?.name ?? "项目"}」的<b className="text-ink">临时空间对话</b>。
              发送第一条消息后创建临时空间，并把主项目与关联项目拷贝进去（原目录不受影响，可随时合并或清空）。
              <div className="mt-2 text-[12px]">Agent 可以读写临时副本中的文件、执行命令（需确认）、搜索代码。</div>
            </div>
          ) : (
            <div className="text-inkdim text-[13px] leading-relaxed">
              {draftProject
                ? "当前为项目下的临时对话，发送第一条消息后自动保存到该项目。"
                : "当前为临时对话，发送第一条消息后才会保存。"}
              <div className="mt-2 text-[12px]">Agent 可以读写文件、执行命令（需确认）、搜索代码。</div>
            </div>
          )}
          {!currentId && (
            <div className="mt-4">
              <button className="px-3 py-1.5 rounded-lg bg-accent/15 text-accent hover:bg-accent/25 text-[13px]" onClick={() => newDraft(null)}>
                + 新会话
              </button>
            </div>
          )}
        </div>
      </div>
    );
  }

  const lastUserMsgId = [...msgs].reverse().find((m) => m.role === "user" && !m.queued)?.id;

  return (
    <div className="flex-1 overflow-y-auto relative" ref={boxRef} onScroll={onScroll}>
      <FloatingTaskPanel />
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
        {running && msgs.length === 0 && (
          <div className="text-inkdim text-[13px] flex items-center gap-2">
            <span className="w-2 h-2 rounded-full bg-green-400 animate-pulse" /> Agent 正在思考…
          </div>
        )}
      </div>
    </div>
  );
}
