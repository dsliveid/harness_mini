import { useEffect, useRef, useState } from "react";
import { ipc } from "../ipc";
import { useStore } from "../store";
import { Attachment, DRAFT_ID } from "../types";
import { SafeImage } from "./SafeImage";
import { toAssetUrl, isVisionModel, formatFileSize } from "../utils/image";
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
} from "./Icons";

function formatTokens(n?: number | null): string {
  if (n == null || isNaN(n)) return "0";
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(2)}M`;
  if (n >= 10_000) return `${(n / 1_000).toFixed(1)}k`;
  return n.toLocaleString("zh-CN");
}

export function Composer() {
  const currentId = useStore((s) => s.currentId);
  const activeId = currentId ?? DRAFT_ID;
  const text = useStore((s) => s.sessionDrafts[activeId] ?? "");
  const setSessionDraft = useStore((s) => s.setSessionDraft);
  const running = useStore((s) => (s.currentId ? s.runStatus[s.currentId] === "running" : false));
  const readOnly = useStore((s) => s.readOnly);
  const session = useStore((s) => (s.currentId && s.currentId !== DRAFT_ID ? s.sessions.find((x) => x.id === s.currentId) ?? null : null));
  const draft = useStore((s) => s.draft);
  const tempInfo = useStore((s) => s.tempInfo);
  const settings = useStore((s) => s.settings);
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

  const taRef = useRef<HTMLTextAreaElement>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const backupDraftRef = useRef<{ text: string; attachments: Attachment[] } | null>(null);
  const prevEditingIdRef = useRef<string | null>(null);

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

  const placeholder = editingMessage
    ? "正在编辑最后一条消息，Enter 重新发送，Esc 取消编辑…"
    : tempBlocked
    ? "已合并到原项目，清空临时空间后可继续发送消息"
    : readOnly
    ? "已归档会话为只读，取消归档后可继续对话"
    : running
    ? "Agent 运行中…输入消息回车将加入待执行列表"
    : "输入消息，支持 Ctrl+V 粘贴/拖拽文件与图片，Enter 发送…";

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
    session?.modelId || session?.model_id || settings?.activeModelId || settings?.activeModel || "";

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

    setSessionDraft(targetId, "");
    setPendingAttachments([]);
    requestAnimationFrame(resize);
    try {
      const st = useStore.getState();
      const isDraftLike = !currentId || currentId === DRAFT_ID;
      const draft = st.draft;
      const res = await ipc.sendMessage(
        isDraftLike ? null : currentId,
        t,
        isDraftLike ? draft?.workspacePath ?? undefined : undefined,
        isDraftLike ? draft?.projectId ?? st.currentProjectId ?? undefined : undefined,
        isDraftLike ? draft?.temp ?? undefined : undefined,
        isDraftLike ? draft?.accessMode ?? undefined : undefined,
        isDraftLike ? draft?.contextTokenLimit ?? undefined : undefined,
        sendingAttachments.length > 0 ? sendingAttachments : undefined,
        isDraftLike ? draft?.imageProviderId ?? undefined : undefined,
        isDraftLike ? draft?.imageModelId ?? undefined : undefined,
        isDraftLike ? draft?.visionProviderId ?? undefined : undefined,
        isDraftLike ? draft?.visionModelId ?? undefined : undefined
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

  const onKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
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
            <span>正在编辑最后一条消息</span>
            <span className="text-[11px] text-inkdim hidden sm:inline">（其后的消息将被作废并重新运行）</span>
          </div>
          <button
            type="button"
            onClick={cancelEdit}
            className="px-2 py-0.5 rounded-lg hover:bg-accent/20 text-inkdim hover:text-ink flex items-center gap-1.5 transition-colors cursor-pointer text-[11.5px]"
            title="取消编辑 (Esc)"
          >
            <span>取消编辑</span>
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

          <textarea
            ref={taRef}
            style={{ height: 32 }}
            className="flex-1 bg-transparent outline-none resize-none text-[14px] leading-relaxed max-h-[200px] min-h-[32px] py-1 disabled:opacity-50 placeholder:text-inkdim/60"
            rows={1}
            value={text}
            placeholder={placeholder}
            disabled={!canSend}
            onPaste={handlePaste}
            onChange={(e) => {
              setSessionDraft(activeId, e.target.value);
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

        <div className="flex items-center gap-2.5 shrink-0">
          {session ? (
            <button
              type="button"
              className={`inline-flex items-center gap-1.5 px-2 py-0.5 rounded-md text-[11px] transition-all group font-mono cursor-pointer border ${
                (session.totalTokens ?? 0) > 0
                  ? "bg-panel2/80 hover:bg-panel3 border-edge/80 hover:border-amber-400/40 text-ink shadow-xs"
                  : "hover:bg-panel2 text-inkdim hover:text-ink border-transparent hover:border-edge/50"
              }`}
              onClick={() => setShowTokenStatsModal(true)}
              title={`该会话累计消耗: ${(session.totalTokens ?? 0).toLocaleString()} tokens\n输入: ${(session.promptTokens ?? 0).toLocaleString()} · 输出: ${(session.completionTokens ?? 0).toLocaleString()}\n点击打开 Token 消耗统计看板`}
            >
              <Coins
                size={12}
                className={
                  (session.totalTokens ?? 0) > 0
                    ? "text-amber-400 group-hover:scale-110 transition-transform shrink-0"
                    : "text-inkdim group-hover:text-amber-400 transition-colors shrink-0"
                }
              />
              <span className={(session.totalTokens ?? 0) > 0 ? "font-medium text-ink" : "text-inkdim"}>
                {formatTokens(session.totalTokens ?? 0)}
              </span>
              <span className="text-[10px] text-inkdim">tokens</span>
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

