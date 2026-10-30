import { useEffect, useMemo, useRef, useState } from "react";
import { ipc } from "../ipc";
import { useStore } from "../store";
import { Attachment, DRAFT_ID } from "../types";
import { SafeImage } from "./SafeImage";
import { toAssetUrl, isVisionModel, formatFileSize } from "../utils/image";
import { ContextUsageGauge } from "./ContextUsageGauge";
import {
  ArrowUp,
  Square,
  Coins,
  Paperclip,
  X,
  File,
  Loader2,
  AlertTriangle,
  Bot,
  Settings as SettingsIcon,
  Pencil,
  Target,
  Zap,
  Sparkles,
} from "./Icons";

function formatTokens(n?: number | null): string {
  if (n == null || isNaN(n)) return "0";
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(2)}M`;
  if (n >= 10_000) return `${(n / 1_000).toFixed(1)}k`;
  return n.toLocaleString("zh-CN");
}

// 斜杠指令统一注册表：菜单项与标签态配置合一。
// 选中后在输入框首部呈现可删除的标签 Badge，提交时按 buildMessage 生成最终消息（原生融入当前会话）。
interface SlashCommand {
  cmd: string;
  label: string;
  desc: string;
  placeholder: string;
  /** 是否必须填写正文（为空时不发送，保持标签态并提示） */
  requireBody: boolean;
  emptyHint: string;
  buildMessage: (body: string) => string;
  badgeClass: string;
  iconClass: string;
  closeClass: string;
  badgeTitle: string;
  Icon: typeof Target;
}

const BEST_REVIEW_REQUIREMENT =
  "评审要求：分析在不考虑成本的情况下，当前方案是否为最优。我不希望采用堆叠式的修改，我希望每次改动后能够让程序结构更清晰，功能更稳定。";

const JEV_BADGE = {
  badgeClass: "bg-amber-500/15 border-amber-500/35 text-amber-300",
  iconClass: "text-amber-400",
  closeClass: "hover:bg-amber-500/25 text-amber-400 hover:text-amber-100",
  Icon: Zap,
};

const SLASH_COMMANDS: SlashCommand[] = [
  {
    cmd: "/goal",
    label: "/goal <总任务目标>",
    desc: "发起长任务：自动拆解多阶段路线图并在独立顶栏中自主推进",
    placeholder: "输入长任务总目标，Enter 发送启动推进，Backspace 删除标签取消…",
    requireBody: true,
    emptyHint: "长任务指令已激活，请输入长任务总目标",
    buildMessage: (body) =>
      `【长任务目标模式】${body}\n\n请分析并拆解该长任务目标，制定明确的阶段方案（优先调用 create_plan / todo 规划），并立即按步骤推进落实。`,
    badgeClass: "bg-purple-500/15 border-purple-500/35 text-purple-300",
    iconClass: "text-purple-400",
    closeClass: "hover:bg-purple-500/25 text-purple-400 hover:text-purple-100",
    badgeTitle: "删除标签以取消长任务 (Backspace)",
    Icon: Target,
  },
  {
    cmd: "/best",
    label: "/best <需求或方案>",
    desc: "方案最优性评审：在不考虑成本下评估当前需求/方案是否最优",
    placeholder: "输入待评审的需求或方案，Enter 发送评审，Backspace 删除标签取消…",
    requireBody: false,
    emptyHint: "",
    buildMessage: (body) =>
      body
        ? `【方案最优性评审模式】\n\n请对以下需求/方案进行最优性评审：\n${body}\n\n${BEST_REVIEW_REQUIREMENT}`
        : `【方案最优性评审模式】\n\n请针对本会话最近的需求或你已提出的方案进行最优性评审。\n\n${BEST_REVIEW_REQUIREMENT}`,
    badgeClass: "bg-sky-500/15 border-sky-500/35 text-sky-300",
    iconClass: "text-sky-400",
    closeClass: "hover:bg-sky-500/25 text-sky-400 hover:text-sky-100",
    badgeTitle: "删除标签以取消方案评审 (Backspace)",
    Icon: Sparkles,
  },
  {
    cmd: "/compact",
    label: "/compact [可选聚焦重点]",
    desc: "提炼并压缩当前上下文历史：生成结构化技术备忘录并替换早期对话，释放 Token 空间",
    placeholder: "可选输入需重点保留的技术细节或约束，Enter 触发压缩，Backspace 删除标签取消…",
    requireBody: false,
    emptyHint: "",
    buildMessage: (body) =>
      body
        ? `请调用 compact_context 工具压缩当前上下文历史，重点关注：${body}`
        : "请调用 compact_context 工具压缩当前上下文历史，提炼技术备忘录并释放 Token 空间",
    badgeClass: "bg-emerald-500/15 border-emerald-500/35 text-emerald-300",
    iconClass: "text-emerald-400",
    closeClass: "hover:bg-emerald-500/25 text-emerald-400 hover:text-emerald-100",
    badgeTitle: "删除标签以取消压缩指令 (Backspace)",
    Icon: Sparkles,
  },
  {
    cmd: "/jev:决策",
    label: "/jev:决策 <意图或方案>",
    desc: "场景化意图理解与方案决策：自动分析意图并智能调度决策工具（评分/判定/选择）",
    placeholder: "输入待评估的意图或方案，Enter 自动选择决策工具执行决策，Backspace 删除标签取消…",
    requireBody: true,
    emptyHint: "/jev:决策 指令已激活，请输入待评估的意图或方案",
    buildMessage: (body) => `/jev:决策 ${body}`,
    ...JEV_BADGE,
    badgeTitle: "删除标签以取消决策 (Backspace)",
  },
  {
    cmd: "/jev:判断",
    label: "/jev:判断 <判定命题或问题>",
    desc: "调用决策模型进行二元真伪判定与置信度打分",
    placeholder: "输入待判定的命题或问题，Enter 执行判定，Backspace 删除标签取消…",
    requireBody: true,
    emptyHint: "/jev:判断 指令已激活，请输入待判定的命题或问题",
    buildMessage: (body) => `/jev:判断 ${body}`,
    ...JEV_BADGE,
    badgeTitle: "删除标签以取消判断决策 (Backspace)",
  },
  {
    cmd: "/jev:选择",
    label: "/jev:选择 <决策问题> 选项: A, B, C",
    desc: "调用决策模型在多个选项中挑选最优解",
    placeholder: "输入决策问题与选项（选项: A, B, C），Enter 执行选择，Backspace 删除标签取消…",
    requireBody: true,
    emptyHint: "/jev:选择 指令已激活，请输入决策问题与备选项",
    buildMessage: (body) => `/jev:选择 ${body}`,
    ...JEV_BADGE,
    badgeTitle: "删除标签以取消选择决策 (Backspace)",
  },
  {
    cmd: "/jev:评分",
    label: "/jev:评分 <待评估内容>",
    desc: "调用决策模型给出 0-100% 量化评分与详细依据",
    placeholder: "输入待评估内容，Enter 执行评分，Backspace 删除标签取消…",
    requireBody: true,
    emptyHint: "/jev:评分 指令已激活，请输入待评估内容",
    buildMessage: (body) => `/jev:评分 ${body}`,
    ...JEV_BADGE,
    badgeTitle: "删除标签以取消评分决策 (Backspace)",
  },
];

// 预编译指令令牌匹配正则（按令牌长度降序，避免前缀误配）
const SLASH_TOKEN_ALT = SLASH_COMMANDS.map((c) => c.cmd.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"))
  .sort((a, b) => b.length - a.length)
  .join("|");
const SLASH_TOKEN_RE = new RegExp(`(?:^|[^\\w\\/:])(${SLASH_TOKEN_ALT})($|\\s+)`, "i");
const SLASH_TOKEN_ONLY_RE = new RegExp(`(${SLASH_TOKEN_ALT})($|\\s+)`, "i");
const SLASH_AUTO_TAG_RE = new RegExp(`(?:^|[^\\w\\/:])(${SLASH_TOKEN_ALT})\\s+`, "i");

function findSlashCommand(cmd: string): SlashCommand | null {
  const lower = cmd.toLowerCase();
  return SLASH_COMMANDS.find((c) => c.cmd.toLowerCase() === lower) ?? null;
}

// 从文本中剥离行首的斜杠指令令牌，返回命中的指令配置与剩余正文
function parseSlashCommand(text: string): { cfg: SlashCommand | null; body: string } {
  const normalized = text.replace(/^\/jev:\s+决策/i, "/jev:决策");
  const m = normalized.match(SLASH_TOKEN_RE);
  if (!m) return { cfg: null, body: normalized.trim() };
  const body = normalized
    .replace(SLASH_TOKEN_RE, (match) => {
      const prefix = match.replace(SLASH_TOKEN_ONLY_RE, "");
      return prefix ? prefix.trimEnd() + " " : "";
    })
    .trim();
  return { cfg: findSlashCommand(m[1]), body };
}

export function Composer() {
  const currentId = useStore((s) => s.currentId);
  const activeId = currentId ?? DRAFT_ID;
  const text = useStore((s) => s.sessionDrafts[activeId] ?? "");
  const setSessionDraft = useStore((s) => s.setSessionDraft);
  const running = useStore((s) => (s.currentId ? s.runStatus[s.currentId] === "running" : false));
  const readOnly = useStore((s) => s.readOnly);
  const session = useStore((s) => (s.currentId && s.currentId !== DRAFT_ID ? s.sessions.find((x) => x.id === s.currentId) ?? null : null));
  const msgs = useStore((s) => (s.currentId ? s.messages[s.currentId] ?? [] : []));
  const tokenStats = useMemo(() => {
    let total = 0;
    let prompt = 0;
    let completion = 0;
    let cached = 0;
    for (const msg of msgs) {
      const tt = msg.totalTokens ?? (msg.usage?.totalTokens || (msg.usage?.inputEst || 0) + (msg.usage?.outputEst || 0)) ?? 0;
      const pt = msg.promptTokens ?? (msg.usage?.promptTokens || msg.usage?.inputEst) ?? 0;
      const ct = msg.completionTokens ?? (msg.usage?.completionTokens || msg.usage?.outputEst) ?? 0;
      const ckt = msg.cachedTokens ?? (msg.usage?.cachedTokens || msg.usage?.promptCacheHitTokens || msg.usage?.cacheReadInputTokens) ?? 0;
      total += Number(tt) || 0;
      prompt += Number(pt) || 0;
      completion += Number(ct) || 0;
      cached += Number(ckt) || 0;
    }
    const finalTotal = Math.max(total, Number(session?.totalTokens) || 0);
    const finalPrompt = Math.max(prompt, Number(session?.promptTokens) || 0);
    const finalCached = Math.max(cached, Number(session?.cachedTokens) || 0);
    const finalCompletion = Math.max(completion, Number(session?.completionTokens) || 0);
    const cacheHitRate = finalPrompt > 0 ? Math.round((finalCached / finalPrompt) * 1000) / 10 : (session?.cacheHitRate ?? 0);
    return {
      totalTokens: finalTotal,
      promptTokens: finalPrompt,
      completionTokens: finalCompletion,
      cachedTokens: finalCached,
      cacheHitRate,
    };
  }, [msgs, session?.totalTokens, session?.promptTokens, session?.completionTokens, session?.cachedTokens, session?.cacheHitRate]);
  const draft = useStore((s) => s.draft);
  const tempInfo = useStore((s) => s.tempInfo);
  const fallbackModel = useStore((s) => s.settings.activeModelId || s.settings.activeModel || "");
  const pushToast = useStore((s) => s.pushToast);
  const setShowTokenStatsModal = useStore((s) => s.setShowTokenStatsModal);
  const setShowCreateCollaboratorModal = useStore((s) => s.setShowCreateCollaboratorModal);
  const setLightboxImage = useStore((s) => s.setLightboxImage);
  const editingMessage = useStore((s) => s.editingMessage);
  const setEditingMessage = useStore((s) => s.setEditingMessage);

  const [pendingAttachments, setPendingAttachments] = useState<Attachment[]>([]);
  const [uploading, setUploading] = useState(false);
  const [isDragging, setIsDragging] = useState(false);
  const [visionWarningOpen, setVisionWarningOpen] = useState(false);
  const [slashIndex, setSlashIndex] = useState(0);
  const [activeCmd, setActiveCmd] = useState<string | null>(null);
  const [cursorPos, setCursorPos] = useState<number | null>(null);
  const [dismissedQuery, setDismissedQuery] = useState<string | null>(null);

  const taRef = useRef<HTMLTextAreaElement>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const backupDraftRef = useRef<{ text: string; attachments: Attachment[] } | null>(null);
  const prevEditingIdRef = useRef<string | null>(null);

  // 切换会话时重置标签态指令
  useEffect(() => {
    setActiveCmd(null);
  }, [activeId]);

  // 同步编辑目标到输入框与附件
  useEffect(() => {
    const curEditingId = editingMessage?.messageId ?? null;
    if (curEditingId !== prevEditingIdRef.current) {
      if (editingMessage) {
        // 进入编辑模式：备份进入编辑前的草稿与临时附件
        backupDraftRef.current = {
          text: useStore.getState().sessionDrafts[activeId] ?? "",
          attachments: pendingAttachments,
        };
        setSessionDraft(activeId, editingMessage.text);
        setPendingAttachments(editingMessage.attachments ?? []);
        requestAnimationFrame(() => {
          resize();
          if (taRef.current) {
            taRef.current.focus();
            const len = taRef.current.value.length;
            taRef.current.setSelectionRange(len, len);
          }
        });
      } else {
        // 退出编辑模式且存在草稿备份时恢复
        if (backupDraftRef.current) {
          setSessionDraft(activeId, backupDraftRef.current.text);
          setPendingAttachments(backupDraftRef.current.attachments);
          backupDraftRef.current = null;
          requestAnimationFrame(resize);
        }
      }
      prevEditingIdRef.current = curEditingId;
    }
  }, [editingMessage, activeId]);

  const cancelEdit = () => {
    if (!editingMessage) return;
    const backup = backupDraftRef.current;
    backupDraftRef.current = null;
    setEditingMessage(null);
    if (backup) {
      setSessionDraft(activeId, backup.text);
      setPendingAttachments(backup.attachments);
    }
    requestAnimationFrame(resize);
  };

  // 临时空间对话：已合并且临时空间未清空期间禁止发送（清空后可继续）
  const tempBlocked = !!(
    session?.mergedPending &&
    session.isTemp &&
    (tempInfo[session.id]?.exists ?? false)
  );

  const hasAttachments = pendingAttachments.length > 0;
  const canSend = !readOnly && !tempBlocked && !uploading && (!editingMessage || !running);
  const hasContent = !!text.trim() || hasAttachments;

  const isTagCommand = !!activeCmd || SLASH_TOKEN_RE.test(text.trim());

  const slashCtx = useMemo(() => {
    if (activeCmd) return null;
    const pos = cursorPos ?? (taRef.current ? taRef.current.selectionStart : text.length);
    const safePos = Math.max(0, Math.min(pos, text.length));
    const before = text.slice(0, safePos);
    const m = before.match(/(?:^|[^\w\/:])\/([a-zA-Z0-9_:\u4e00-\u9fa5-]*)$/);
    if (!m) return null;
    const queryBefore = m[1];
    const slashStartIndex = before.length - queryBefore.length - 1;
    const after = text.slice(safePos);
    const afterMatch = after.match(/^[a-zA-Z0-9_:\u4e00-\u9fa5-]*/);
    const queryAfter = afterMatch ? afterMatch[0] : "";
    const fullQuery = (queryBefore + queryAfter).toLowerCase();
    const slashEndIndex = safePos + queryAfter.length;
    return {
      startIndex: slashStartIndex,
      endIndex: slashEndIndex,
      query: fullQuery,
    };
  }, [activeCmd, text, cursorPos]);

  const filteredCommands = useMemo(() => {
    if (!slashCtx) return [];
    return SLASH_COMMANDS.filter((c) =>
      c.cmd.toLowerCase().startsWith(`/${slashCtx.query}`)
    );
  }, [slashCtx]);

  useEffect(() => {
    if (dismissedQuery !== null && slashCtx?.query !== dismissedQuery) {
      setDismissedQuery(null);
    }
    setSlashIndex(0);
  }, [slashCtx?.query]);

  const showSlashMenu =
    !activeCmd &&
    !!slashCtx &&
    (dismissedQuery === null || slashCtx.query !== dismissedQuery) &&
    filteredCommands.length > 0;

  const activeCommandCfg = activeCmd ? findSlashCommand(activeCmd) : null;
  const ActiveTagIcon = activeCommandCfg?.Icon ?? null;

  const placeholder = editingMessage
    ? "正在编辑最后一条消息，Enter 重新发送，Esc 取消编辑…"
    : tempBlocked
    ? "已合并到原项目，清空临时空间后可继续发送消息"
    : readOnly
    ? "已归档会话为只读，取消归档后可继续对话"
    : activeCommandCfg
    ? activeCommandCfg.placeholder
    : isTagCommand
    ? "斜杠标签指令：输入内容，回车将注入对应提示词并推进执行…"
    : running
    ? "Agent 运行中…输入消息回车将加入待执行列表"
    : "输入消息，/goal 发起长任务、/best 评审方案，Enter 发送…";

  const resize = () => {
    const ta = taRef.current;
    if (!ta) return;
    ta.style.height = "auto";
    ta.style.height = `${Math.max(32, Math.min(ta.scrollHeight, 200))}px`;
  };

  useEffect(() => {
    resize();
  }, [activeId, text]);

  const insertNewline = () => {
    const ta = taRef.current;
    if (!ta) return;
    const start = ta.selectionStart;
    const end = ta.selectionEnd;
    const next = text.slice(0, start) + "\n" + text.slice(end);
    setSessionDraft(activeId, next);
    requestAnimationFrame(() => {
      ta.selectionStart = ta.selectionEnd = start + 1;
      resize();
    });
  };

  const uploadFile = async (file: File): Promise<Attachment | null> => {
    return new Promise((resolve) => {
      const reader = new FileReader();
      reader.onload = async () => {
        try {
          const base64Data = reader.result as string;
          const att = await ipc.saveAttachment({
            sessionId: editingMessage ? editingMessage.sessionId : currentId,
            name: file.name,
            mimeType: file.type || "application/octet-stream",
            base64Data,
          });
          resolve(att);
        } catch (e) {
          pushToast(`保存附件失败: ${e}`);
          resolve(null);
        }
      };
      reader.onerror = () => {
        pushToast("读取文件失败");
        resolve(null);
      };
      reader.readAsDataURL(file);
    });
  };

  const handleFiles = async (files: FileList | File[]) => {
    if (!files || files.length === 0) return;
    setUploading(true);
    try {
      const newAtts: Attachment[] = [];
      for (let i = 0; i < files.length; i++) {
        const file = files[i];
        const att = await uploadFile(file);
        if (att) newAtts.push(att);
      }
      if (newAtts.length > 0) {
        setPendingAttachments((prev) => [...prev, ...newAtts]);
      }
    } finally {
      setUploading(false);
    }
  };

  const handlePaste = async (e: React.ClipboardEvent) => {
    const items = e.clipboardData?.items;
    if (!items) return;
    const filesToUpload: File[] = [];
    for (let i = 0; i < items.length; i++) {
      const item = items[i];
      if (item.kind === "file") {
        const file = item.getAsFile();
        if (file) filesToUpload.push(file);
      }
    }
    if (filesToUpload.length > 0) {
      e.preventDefault();
      await handleFiles(filesToUpload);
    }
  };

  const handleDragOver = (e: React.DragEvent) => {
    e.preventDefault();
    if (!isDragging) setIsDragging(true);
  };

  const handleDragLeave = (e: React.DragEvent) => {
    e.preventDefault();
    if (e.currentTarget.contains(e.relatedTarget as Node)) return;
    setIsDragging(false);
  };

  const handleDrop = async (e: React.DragEvent) => {
    e.preventDefault();
    setIsDragging(false);
    if (e.dataTransfer?.files?.length > 0) {
      await handleFiles(e.dataTransfer.files);
    }
  };

  const removeAttachment = (id: string) => {
    setPendingAttachments((prev) => prev.filter((a) => a.id !== id));
  };

  const activeModel =
    session?.modelId || session?.model_id || draft?.modelId || fallbackModel;

  const checkVisionAndSend = async (bypassVisionCheck = false) => {
    const t = text.trim();
    if (!hasContent || !canSend) return;

    const hasImages = pendingAttachments.some((a) => a.is_image);
    if (hasImages && !bypassVisionCheck && !isVisionModel(activeModel)) {
      setVisionWarningOpen(true);
      return;
    }

    setVisionWarningOpen(false);
    await doSend();
  };

  const doSend = async () => {
    const t = text.trim();
    if (!hasContent || !canSend) return;
    const targetId = activeId;
    const sendingAttachments = [...pendingAttachments];

    if (editingMessage) {
      const curEditing = editingMessage;
      setSessionDraft(targetId, "");
      setPendingAttachments([]);
      requestAnimationFrame(resize);
      try {
        await ipc.editAndResend(
          curEditing.sessionId,
          curEditing.messageId,
          t,
          sendingAttachments.length > 0 ? sendingAttachments : undefined
        );
        const backup = backupDraftRef.current;
        backupDraftRef.current = null;
        setEditingMessage(null);
        if (backup && (backup.text.trim() || backup.attachments.length > 0)) {
          setSessionDraft(targetId, backup.text);
          setPendingAttachments(backup.attachments);
        }
      } catch (e) {
        pushToast(String(e));
        setSessionDraft(targetId, t);
        setPendingAttachments(sendingAttachments);
      }
      return;
    }

    const trimmed = t.trim();

    // 解析标签态斜杠指令：优先当前激活的标签，其次解析消息首部的斜杠指令（原生融入当前会话）
    const parsed = parseSlashCommand(trimmed);
    const activeCfg = activeCmd ? findSlashCommand(activeCmd) : null;
    const cfg = activeCfg ?? parsed.cfg;
    const body = activeCfg ? trimmed : parsed.body;
    let messageToSend = t;
    if (cfg?.cmd === "/goal") {
      const goalBody = (body || "").trim();
      if (!goalBody) {
        setActiveCmd(cfg.cmd);
        setSessionDraft(targetId, "");
        pushToast(cfg.emptyHint, "info");
        requestAnimationFrame(() => {
          if (taRef.current) {
            taRef.current.focus();
          }
        });
        return;
      }
      setActiveCmd(null);
      setSessionDraft(targetId, "");
      setPendingAttachments([]);
      requestAnimationFrame(resize);

      try {
        const st = useStore.getState();
        const isDraftLike = !currentId || currentId === DRAFT_ID;
        const draft = st.draft;
        let targetSessionId = currentId;

        if (isDraftLike || !targetSessionId) {
          const s = await ipc.createSession({
            workspacePath: draft?.workspacePath ?? undefined,
            projectId: draft?.projectId ?? st.currentProjectId ?? undefined,
            title: `长任务: ${goalBody.slice(0, 20)}`,
            accessMode: draft?.accessMode ?? undefined,
            contextTokenLimit: draft?.contextTokenLimit ?? undefined,
            temp: draft?.temp ?? undefined,
            providerId: draft?.providerId ?? undefined,
            modelId: draft?.modelId ?? undefined,
            imageProviderId: draft?.imageProviderId ?? undefined,
            imageModelId: draft?.imageModelId ?? undefined,
            visionProviderId: draft?.visionProviderId ?? undefined,
            visionModelId: draft?.visionModelId ?? undefined,
            decisionProviderId: draft?.decisionProviderId ?? undefined,
            decisionModelId: draft?.decisionModelId ?? undefined,
            reasoningEffort: draft?.reasoningEffort ?? undefined,
          });
          targetSessionId = s.id;
          st.onSessionUpdate(s);
          await st.selectSession(s.id);
        }

        // 启动长任务核心引擎，并在主界面激活 TaskBar
        await st.startLongTask(targetSessionId, goalBody);
      } catch (e) {
        pushToast(`启动长任务失败: ${e}`, "error");
        setSessionDraft(targetId, t);
        setPendingAttachments(sendingAttachments);
      }
      return;
    }

    if (cfg) {
      // 需要正文的指令在留空时不发送，保持标签态并提示用户输入
      if (cfg.requireBody && !body) {
        setActiveCmd(cfg.cmd);
        setSessionDraft(targetId, "");
        pushToast(cfg.emptyHint, "info");
        requestAnimationFrame(() => {
          if (taRef.current) {
            taRef.current.focus();
          }
        });
        return;
      }
      setActiveCmd(null);
      messageToSend = cfg.buildMessage(body);
    }

    setSessionDraft(targetId, "");
    setPendingAttachments([]);
    requestAnimationFrame(resize);

    // 发起新一轮对话时，即刻重置当前会话的看板状态为 null，实现无缝切轮与旧看板平滑隐去
    if (targetId && targetId !== DRAFT_ID) {
      useStore.setState((st) => ({
        currentBoards: {
          ...st.currentBoards,
          [targetId]: null,
        },
      }));
    }

    try {
      const st = useStore.getState();
      const isDraftLike = !currentId || currentId === DRAFT_ID;
      const draft = st.draft;
      const res = await ipc.sendMessage(
        isDraftLike ? null : currentId,
        messageToSend,
        isDraftLike ? draft?.workspacePath ?? undefined : undefined,
        isDraftLike ? draft?.projectId ?? st.currentProjectId ?? undefined : undefined,
        isDraftLike ? draft?.temp ?? undefined : undefined,
        isDraftLike ? draft?.accessMode ?? undefined : undefined,
        isDraftLike ? draft?.contextTokenLimit ?? undefined : undefined,
        sendingAttachments.length > 0 ? sendingAttachments : undefined,
        isDraftLike ? draft?.providerId ?? undefined : undefined,
        isDraftLike ? draft?.modelId ?? undefined : undefined,
        isDraftLike ? draft?.imageProviderId ?? undefined : undefined,
        isDraftLike ? draft?.imageModelId ?? undefined : undefined,
        isDraftLike ? draft?.visionProviderId ?? undefined : undefined,
        isDraftLike ? draft?.visionModelId ?? undefined : undefined,
        isDraftLike ? draft?.decisionProviderId ?? undefined : undefined,
        isDraftLike ? draft?.decisionModelId ?? undefined : undefined,
        isDraftLike ? draft?.reasoningEffort ?? undefined : undefined
      );
      const st2 = useStore.getState();
      if (res.session) st2.onSessionUpdate(res.session);
      if (!res.queued && res.sessionId && st2.runStatus[res.sessionId] === undefined) {
        st2.onRunStatus({ sessionId: res.sessionId, status: "running" });
      }
      if (isDraftLike && res.sessionId) {
        await st2.selectSession(res.sessionId);
      }
    } catch (e) {
      pushToast(String(e));
      setSessionDraft(targetId, t);
      setPendingAttachments(sendingAttachments);
    }
  };

  const applySlashCommand = (cmd: string) => {
    const cfg = findSlashCommand(cmd);
    if (!cfg) return;
    // 已选指令保留输入框中其余文本作为正文，标签态仅呈现指令本身
    let remaining = "";
    if (slashCtx) {
      const beforeSlash = text.slice(0, slashCtx.startIndex);
      const afterCommand = text.slice(slashCtx.endIndex);
      remaining = (beforeSlash.trimEnd() + " " + afterCommand.trimStart()).trim();
    } else {
      remaining = parseSlashCommand(text).body;
    }

    setActiveCmd(cfg.cmd);
    setSessionDraft(activeId, remaining);
    requestAnimationFrame(() => {
      if (taRef.current) {
        taRef.current.focus();
        const len = remaining.length;
        taRef.current.setSelectionRange(len, len);
        setCursorPos(len);
        resize();
      }
    });
  };

  const onKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (showSlashMenu && filteredCommands.length > 0) {
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setSlashIndex((i) => (i + 1) % filteredCommands.length);
        return;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        setSlashIndex((i) => (i - 1 + filteredCommands.length) % filteredCommands.length);
        return;
      }
      if (e.key === "Tab" || (e.key === "Enter" && !e.shiftKey && !e.altKey)) {
        e.preventDefault();
        const safeIdx = Math.min(slashIndex, filteredCommands.length - 1);
        applySlashCommand(filteredCommands[safeIdx]?.cmd || "/goal");
        return;
      }
      if (e.key === "Escape") {
        e.preventDefault();
        setDismissedQuery(slashCtx?.query ?? "");
        return;
      }
    }

    // 当标签激活且输入框光标在首位时，按 Backspace 删除标签并取消该指令
    if (e.key === "Backspace" && activeCmd) {
      const ta = taRef.current;
      if (ta && ta.selectionStart === 0 && ta.selectionEnd === 0) {
        e.preventDefault();
        setActiveCmd(null);
        return;
      }
    }

    if (e.key === "Escape" && editingMessage) {
      e.preventDefault();
      cancelEdit();
      return;
    }
    if (e.key !== "Enter") return;
    if ((e.nativeEvent as unknown as { isComposing?: boolean }).isComposing) return;
    if (e.altKey || e.shiftKey) {
      e.preventDefault();
      insertNewline();
      return;
    }
    e.preventDefault();
    void checkVisionAndSend();
  };

  const effectiveWorkspace = session?.workspacePath ?? draft?.workspacePath ?? "";

  return (
    <div className="p-3">
      {/* Hidden file input */}
      <input
        type="file"
        multiple
        ref={fileInputRef}
        className="hidden"
        onChange={(e) => {
          if (e.target.files) {
            void handleFiles(e.target.files);
            e.target.value = "";
          }
        }}
      />

      {/* Editing Mode Banner */}
      {editingMessage && (
        <div className="mb-2 px-3 py-1.5 rounded-xl bg-accent/10 border border-accent/30 flex items-center justify-between text-xs animate-in fade-in slide-in-from-bottom-1 duration-150 shadow-xs">
          <div className="flex items-center gap-2 text-accent font-medium">
            <Pencil size={13} className="shrink-0 animate-pulse" />
            <span>已撤回对话到此处（修改并重新发送将覆盖后续旧历史）</span>
            <span className="text-[11px] text-inkdim hidden sm:inline">（如需恢复全部最新历史，可点击上方气泡前的「重新应用」）</span>
          </div>
          <button
            type="button"
            onClick={cancelEdit}
            className="px-2 py-0.5 rounded-lg hover:bg-accent/20 text-inkdim hover:text-ink flex items-center gap-1.5 transition-colors cursor-pointer text-[11.5px]"
            title="退出编辑模式 (Esc)"
          >
            <span>退出编辑</span>
            <kbd className="text-[10px] font-mono bg-panel2/80 px-1 py-0.5 rounded border border-edge">Esc</kbd>
          </button>
        </div>
      )}

      {/* Main Composer Box */}
      <div
        onDragOver={handleDragOver}
        onDragLeave={handleDragLeave}
        onDrop={handleDrop}
        className={`relative flex flex-col bg-panel2/90 border rounded-2xl shadow-sm focus-within:border-accent/60 focus-within:ring-1 focus-within:ring-accent/20 transition-all ${
          isDragging
            ? "border-accent ring-2 ring-accent/30 bg-accent/5"
            : editingMessage
            ? "border-accent/70 ring-1 ring-accent/20"
            : "border-edge"
        }`}
      >
        {/* Drag Overlay */}
        {isDragging && (
          <div className="absolute inset-0 z-20 flex items-center justify-center bg-panel/90 rounded-2xl backdrop-blur-xs border-2 border-dashed border-accent pointer-events-none">
            <div className="flex items-center gap-2 text-accent font-medium text-sm">
              <Paperclip size={18} />
              <span>释放文件或图片以添加为对话附件</span>
            </div>
          </div>
        )}

        {/* Pending Attachments Strip */}
        {hasAttachments && (
          <div className="flex items-center gap-2 px-3 pt-2.5 pb-1 overflow-x-auto border-b border-edge/40">
            {pendingAttachments.map((att) => (
              <div
                key={att.id}
                className="group relative flex items-center gap-2 px-2 py-1.5 rounded-xl bg-panel border border-edge/80 text-xs shrink-0 max-w-[220px] shadow-2xs hover:border-accent/50 transition-all"
              >
                {att.is_image ? (
                  <SafeImage
                    src={att.path}
                    alt={att.name}
                    onClick={() => setLightboxImage({ src: att.path, title: att.name })}
                    className="w-7 h-7 rounded-md object-cover bg-panel2 shrink-0 cursor-zoom-in hover:opacity-90"
                    title="点击放大查看"
                  />
                ) : (
                  <div className="w-7 h-7 rounded-md bg-accent/10 border border-accent/20 flex items-center justify-center text-accent shrink-0">
                    <File size={14} />
                  </div>
                )}
                <div className="min-w-0 flex-1">
                  <div className="text-[11.5px] font-medium text-ink truncate" title={att.name}>
                    {att.name}
                  </div>
                  <div className="text-[10px] text-inkdim">{formatFileSize(att.size)}</div>
                </div>
                <button
                  type="button"
                  onClick={() => removeAttachment(att.id)}
                  className="w-4 h-4 rounded-full hover:bg-rose-500/20 hover:text-rose-400 flex items-center justify-center text-inkdim transition-colors cursor-pointer shrink-0"
                  title="移除附件"
                >
                  <X size={11} />
                </button>
              </div>
            ))}
            {uploading && (
              <div className="flex items-center gap-1.5 px-2 py-1 text-xs text-inkdim">
                <Loader2 size={13} className="animate-spin text-accent" />
                <span>正在上传...</span>
              </div>
            )}
          </div>
        )}

        {/* Slash Command Suggestions Popover */}
        {showSlashMenu && filteredCommands.length > 0 && (
          <div className="mx-3 mb-2 p-1.5 rounded-xl bg-panel2/95 border border-edge backdrop-blur shadow-xl animate-in fade-in slide-in-from-bottom-2 duration-150">
            <div className="px-2 py-1 text-[11px] font-semibold text-inkdim border-b border-edge/60 flex items-center justify-between">
              <span>快捷指令 (Enter / Tab 选择)</span>
              <span className="font-mono text-[10px]">↑↓ 切换</span>
            </div>
            <div className="mt-1 space-y-1">
              {filteredCommands.map((item, idx) => {
                const isSelected = idx === slashIndex;
                return (
                  <div
                    key={item.cmd}
                    onMouseDown={(e) => {
                      e.preventDefault();
                      applySlashCommand(item.cmd);
                    }}
                    onClick={() => applySlashCommand(item.cmd)}
                    onMouseEnter={() => setSlashIndex(idx)}
                    className={`px-2.5 py-1.5 rounded-lg flex items-center justify-between gap-3 text-[12px] cursor-pointer transition-colors ${
                      isSelected
                        ? "bg-purple-500/15 text-purple-300 border border-purple-500/30"
                        : "hover:bg-panel text-ink"
                    }`}
                  >
                    <div className="flex items-center gap-2">
                      <div className={`p-1 rounded bg-panel2 border border-edge/60 ${item.iconClass}`}>
                        <item.Icon size={13} />
                      </div>
                      <span className="font-mono font-medium">{item.label}</span>
                    </div>
                    <span className="text-[11px] text-inkdim truncate max-w-[240px]">
                      {item.desc}
                    </span>
                  </div>
                );
              })}
            </div>
          </div>
        )}

        {/* Text Input Row */}
        <div className="flex items-end gap-2 px-3 py-2">
          {/* Attach Button */}
          <button
            type="button"
            disabled={!canSend || uploading}
            onClick={() => fileInputRef.current?.click()}
            className="shrink-0 w-8 h-8 rounded-xl hover:bg-panel border border-transparent hover:border-edge text-inkdim hover:text-accent flex items-center justify-center transition-colors cursor-pointer disabled:opacity-30 disabled:cursor-not-allowed mb-0.5"
            title="添加图片或文件附件（也支持直接粘贴或拖拽）"
          >
            <Paperclip size={16} />
          </button>

          {/* Slash Command Tag Badge */}
          {activeCommandCfg && ActiveTagIcon && (
            <div className={`shrink-0 flex items-center gap-1.5 px-2.5 py-1 rounded-xl border text-[12px] font-medium animate-in fade-in zoom-in-95 duration-150 select-none mb-0.5 shadow-xs ${activeCommandCfg.badgeClass}`}>
              <ActiveTagIcon size={13} className={`shrink-0 animate-pulse ${activeCommandCfg.iconClass}`} />
              <span className="font-mono">{activeCommandCfg.cmd}</span>
              <button
                type="button"
                onClick={() => {
                  setActiveCmd(null);
                  if (taRef.current) taRef.current.focus();
                }}
                className={`p-0.5 rounded-md transition-colors cursor-pointer ${activeCommandCfg.closeClass}`}
                title={activeCommandCfg.badgeTitle}
              >
                <X size={11} />
              </button>
            </div>
          )}

          <textarea
            ref={taRef}
            style={{ height: 32 }}
            className="flex-1 bg-transparent outline-none resize-none text-[14px] leading-relaxed max-h-[200px] min-h-[32px] py-1 disabled:opacity-50 placeholder:text-inkdim/60"
            rows={1}
            value={text}
            placeholder={placeholder}
            disabled={!canSend}
            onPaste={handlePaste}
            onClick={(e) => setCursorPos(e.currentTarget.selectionStart)}
            onKeyUp={(e) => setCursorPos(e.currentTarget.selectionStart)}
            onSelect={(e) => setCursorPos(e.currentTarget.selectionStart)}
            onChange={(e) => {
              const val = e.target.value;
              const pos = e.target.selectionStart;
              setCursorPos(pos);

              // 用户键入或粘贴斜杠指令加空格时，自动转为对应标签
              const autoMatch = val.match(SLASH_AUTO_TAG_RE);
              const autoCfg = autoMatch ? findSlashCommand(autoMatch[1]) : null;
              if (!activeCmd && autoCfg) {
                const remaining = val
                  .replace(SLASH_AUTO_TAG_RE, (match) => {
                    const prefix = match.replace(SLASH_TOKEN_ONLY_RE, "");
                    return prefix ? prefix.trimEnd() + " " : "";
                  })
                  .trim();
                setActiveCmd(autoCfg.cmd);
                setSessionDraft(activeId, remaining);
                requestAnimationFrame(() => {
                  if (taRef.current) {
                    const len = remaining.length;
                    taRef.current.setSelectionRange(len, len);
                    setCursorPos(len);
                    resize();
                  }
                });
                return;
              }
              setSessionDraft(activeId, val);
            }}
            onKeyDown={onKeyDown}
          />

          {running ? (
            <button
              className="shrink-0 w-8 h-8 rounded-xl bg-red-600/90 hover:bg-red-500 text-white flex items-center justify-center transition-colors shadow-sm mb-0.5"
              title="停止生成"
              onClick={() => currentId && useStore.getState().stopRun(currentId)}
            >
              <Square size={13} className="fill-current" />
            </button>
          ) : (
            <button
              className="shrink-0 w-8 h-8 rounded-xl bg-accent hover:bg-blue-500 text-white flex items-center justify-center disabled:opacity-30 disabled:cursor-not-allowed transition-all shadow-sm mb-0.5"
              title={editingMessage ? "重新发送（Enter）" : "发送（Enter）"}
              disabled={!canSend || !hasContent}
              onClick={() => void checkVisionAndSend()}
            >
              <ArrowUp size={16} strokeWidth={2.4} />
            </button>
          )}
        </div>
      </div>

      {/* Vision Model Mismatch Warning Modal */}
      {visionWarningOpen && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-xs p-4 animate-in fade-in duration-150">
          <div className="bg-panel border border-edge rounded-2xl w-full max-w-[440px] shadow-2xl p-5 space-y-4">
            <div className="flex items-start gap-3">
              <div className="w-10 h-10 rounded-xl bg-amber-500/15 border border-amber-500/30 flex items-center justify-center text-amber-400 shrink-0">
                <AlertTriangle size={20} />
              </div>
              <div className="space-y-1">
                <div className="font-semibold text-ink text-[15px]">当前模型可能不支持图像识别</div>
                <div className="text-[12.5px] text-inkdim leading-relaxed">
                  您上传了图片附件，但当前使用的模型{" "}
                  <code className="px-1.5 py-0.5 rounded bg-panel2 border border-edge text-ink text-[11px] font-mono">
                    {activeModel || "纯文本模型"}
                  </code>{" "}
                  未标识具备多模态视觉 (Vision) 解析能力。直接发送可能导致接口报错或无法读取图片。
                </div>
              </div>
            </div>

            <div className="p-3 rounded-xl bg-panel2/50 border border-edge/60 space-y-1.5 text-xs text-inkdim">
              <div className="font-medium text-ink flex items-center gap-1.5">
                <Bot size={13} className="text-accent" />
                <span>建议解决方案：</span>
              </div>
              <div>1. 为主进程添加专属【图像识别协作者】（指定 Vision 模型）</div>
              <div>2. 或在模型设置中将主进程切换为具备多模态能力的模型</div>
            </div>

            <div className="flex items-center justify-end gap-2 pt-1">
              <button
                type="button"
                onClick={() => setVisionWarningOpen(false)}
                className="px-3 py-1.5 rounded-lg border border-edge text-inkdim hover:text-ink text-xs transition-colors cursor-pointer"
              >
                取消
              </button>
              <button
                type="button"
                onClick={() => {
                  setVisionWarningOpen(false);
                  setShowCreateCollaboratorModal(true);
                }}
                className="px-3 py-1.5 rounded-lg border border-accent/40 bg-accent/10 hover:bg-accent/20 text-accent text-xs font-medium transition-colors cursor-pointer"
              >
                添加图像协作者
              </button>
              <button
                type="button"
                onClick={() => void checkVisionAndSend(true)}
                className="px-3.5 py-1.5 rounded-lg bg-accent hover:bg-blue-500 text-white text-xs font-medium transition-colors shadow-sm cursor-pointer"
              >
                仍要发送
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Footer bar */}
      <div className="h-6 flex items-center justify-between text-[11px] text-inkdim mt-2 px-1 select-none gap-3">
        <div className="flex items-center gap-2 min-w-0 flex-1">
          <span className="truncate max-w-[320px] md:max-w-[440px]" title={effectiveWorkspace || undefined}>
            {effectiveWorkspace || "未绑定工作区 · 可直接对话（文件/命令工具不可用）"}
          </span>
        </div>

        <div className="flex items-center gap-2 shrink-0">
          {/* 上下文长度消耗圆圈进度指示器 */}
          <ContextUsageGauge session={session} />

          {session ? (
            <button
              type="button"
              className={`inline-flex items-center gap-1.5 px-2 py-0.5 rounded-md text-[11px] transition-all group font-mono cursor-pointer border ${
                tokenStats.totalTokens > 0
                  ? "bg-panel2/80 hover:bg-panel3 border-edge/80 hover:border-amber-400/40 text-ink shadow-xs"
                  : "hover:bg-panel2 text-inkdim hover:text-ink border-transparent hover:border-edge/50"
              }`}
              onClick={() => setShowTokenStatsModal(true)}
              title={`该会话累计消耗: ${tokenStats.totalTokens.toLocaleString()} tokens\n输入: ${tokenStats.promptTokens.toLocaleString()}${tokenStats.cachedTokens > 0 ? ` (缓存命中: ${tokenStats.cachedTokens.toLocaleString()} · ${tokenStats.cacheHitRate.toFixed(1)}%)` : ""} · 输出: ${tokenStats.completionTokens.toLocaleString()}\n点击打开 Token 消耗统计看板`}
            >
              <Coins
                size={12}
                className={
                  tokenStats.totalTokens > 0
                    ? "text-amber-400 group-hover:scale-110 transition-transform shrink-0"
                    : "text-inkdim group-hover:text-amber-400 transition-colors shrink-0"
                }
              />
              <span className={tokenStats.totalTokens > 0 ? "font-medium text-ink" : "text-inkdim"}>
                {formatTokens(tokenStats.totalTokens)}
              </span>
              <span className="text-[10px] text-inkdim">tokens</span>
              {tokenStats.cacheHitRate > 0 && (
                <span className="px-1 py-0.2 rounded text-[9.5px] bg-cyan-500/10 text-cyan-400 font-medium border border-cyan-500/20 inline-flex items-center gap-0.5">
                  <Zap size={9} className="shrink-0" />
                  <span>{tokenStats.cacheHitRate.toFixed(1)}%</span>
                </span>
              )}
            </button>
          ) : (
            <button
              type="button"
              className="inline-flex items-center gap-1 px-1.5 py-0.5 rounded-md hover:bg-panel2 text-[11px] text-inkdim hover:text-ink transition-colors font-mono cursor-pointer border border-transparent hover:border-edge/50"
              onClick={() => setShowTokenStatsModal(true)}
              title="点击打开 Token 消耗统计看板"
            >
              <Coins size={12} className="text-inkdim group-hover:text-amber-400 transition-colors shrink-0" />
              <span>Token 统计</span>
            </button>
          )}

          <span className="hidden sm:inline-block opacity-65">
            <kbd className="px-1 py-0.5 rounded bg-panel3 border border-edge text-[10px]">Enter</kbd> 发送 ·{" "}
            <kbd className="px-1 py-0.5 rounded bg-panel3 border border-edge text-[10px]">Shift+Enter</kbd> 换行
          </span>
        </div>
      </div>
    </div>
  );
}

