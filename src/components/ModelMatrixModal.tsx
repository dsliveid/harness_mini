import { useEffect, useState } from "react";
import { ipc } from "../ipc";
import { useStore } from "../store";
import { Cpu, Sparkles, X, Check } from "./Icons";
import { ModelCapabilitySelect } from "./ModelCapabilitySelect";
import {
  MODEL_CAPABILITY_METAS,
  Session,
  resolveActiveImageModel,
  resolveActiveVisionModel,
} from "../types";

export function ModelMatrixModal() {
  const show = useStore((s) => s.showModelMatrixModal);
  const targetSessionId = useStore((s) => s.modelMatrixSessionId);
  const currentId = useStore((s) => s.currentId);
  const draft = useStore((s) => s.draft);
  const close = useStore((s) => s.closeModelMatrixModal);
  const setSessionModels = useStore((s) => s.setSessionModels);
  const setDraftCapabilityModels = useStore((s) => s.setDraftCapabilityModels);
  const settings = useStore((s) => s.settings);
  const setSettingsLocal = useStore((s) => s.setSettingsLocal);
  const pushToast = useStore((s) => s.pushToast);

  const sessions = useStore((s) => s.sessions);
  const collaborators = useStore((s) => s.collaborators);
  const subprocesses = useStore((s) => s.subprocesses);

  // 查找目标会话（主会话 / 协作者 / 子进程）
  let targetSession: Session | null = null;
  if (targetSessionId) {
    targetSession =
      sessions.find((s) => s.id === targetSessionId) ??
      Object.values(collaborators)
        .flat()
        .find((s) => s.id === targetSessionId) ??
      Object.values(subprocesses)
        .flat()
        .find((s) => s.id === targetSessionId) ??
      null;
  }

  // 是否为草稿会话（新对话）配置
  const isDraft = targetSessionId === "draft" || (!targetSession && currentId === "draft");
  const isConfiguringSession = Boolean(targetSession && targetSession.id !== "draft") || isDraft;

  const [imageKey, setImageKey] = useState("");
  const [visionKey, setVisionKey] = useState("");
  const [submitting, setSubmitting] = useState(false);

  // 解析全局实际生效的能力模型，供跟随全局时展示具体模型名称
  const activeImage = resolveActiveImageModel(settings);
  const activeImageDesc = activeImage
    ? `${activeImage.model} (${activeImage.provider.name})`
    : "未配置";
  const activeVision = resolveActiveVisionModel(settings);
  const activeVisionDesc = activeVision
    ? `${activeVision.model} (${activeVision.provider.name})`
    : "回落主对话模型";

  // 当前主对话模型显示名
  const currentChatModelName =
    isConfiguringSession && targetSession
      ? targetSession.modelId || targetSession.model_id || settings.activeModelId || settings.activeModel || "未指定（跟随系统）"
      : settings.activeModelId || settings.activeModel || "未配置";

  // 弹窗打开时初始化表单值
  useEffect(() => {
    if (!show) return;

    if (isDraft && draft) {
      // 从草稿中读取已设置的能力模型
      const ip = draft.imageProviderId || "";
      const im = draft.imageModelId || "";
      setImageKey(ip && im ? `${ip}::${im}` : "");

      const vp = draft.visionProviderId || "";
      const vm = draft.visionModelId || "";
      setVisionKey(vp && vm ? `${vp}::${vm}` : "");
    } else if (isConfiguringSession && targetSession) {
      const ip = targetSession.imageProviderId || targetSession.image_provider_id || "";
      const im = targetSession.imageModelId || targetSession.image_model_id || "";
      setImageKey(ip && im ? `${ip}::${im}` : "");

      const vp = targetSession.visionProviderId || targetSession.vision_provider_id || "";
      const vm = targetSession.visionModelId || targetSession.vision_model_id || "";
      setVisionKey(vp && vm ? `${vp}::${vm}` : "");
    } else {
      // 全局配置模式
      const ip = settings.activeImageProviderId || "";
      const im = settings.activeImageModelId || "";
      setImageKey(ip && im ? `${ip}::${im}` : "");

      const vp = settings.activeVisionProviderId || "";
      const vm = settings.activeVisionModelId || "";
      setVisionKey(vp && vm ? `${vp}::${vm}` : "");
    }
  }, [show, targetSessionId, isDraft, draft, isConfiguringSession, targetSession, settings]);

  if (!show) return null;

  const parseKey = (key: string): [string | null, string | null] => {
    if (!key) return [null, null];
    const parts = key.split("::");
    if (parts.length === 2 && parts[0] && parts[1]) {
      return [parts[0], parts[1]];
    }
    return [null, null];
  };

  const handleSave = async () => {
    if (submitting) return;
    setSubmitting(true);
    try {
      const [imagePid, imageMid] = parseKey(imageKey);
      const [visionPid, visionMid] = parseKey(visionKey);

      if (isDraft) {
        // 当前为未落库的新对话草稿：将能力模型保存到草稿中，首发消息创建会话时随之落库
        setDraftCapabilityModels({
          imageProviderId: imagePid,
          imageModelId: imageMid,
          visionProviderId: visionPid,
          visionModelId: visionMid,
        });
        pushToast("当前对话能力模型配置已保存");
        close();
        return;
      }

      if (isConfiguringSession && targetSession) {
        // 仅更新当前会话专属配置（保留会话现有的主对话模型不变）
        const chatPid = targetSession.providerId ?? targetSession.provider_id ?? null;
        const chatMid = targetSession.modelId ?? targetSession.model_id ?? null;

        const success = await setSessionModels({
          sessionId: targetSession.id,
          providerId: chatPid,
          modelId: chatMid,
          imageProviderId: imagePid,
          imageModelId: imageMid,
          visionProviderId: visionPid,
          visionModelId: visionMid,
        });
        if (!success) {
          setSubmitting(false);
          return;
        }
      } else {
        // 未指定会话时（如无活跃会话状态）更新全局默认
        const nextSettings = {
          ...settings,
          activeImageProviderId: imagePid,
          activeImageModelId: imageMid,
          activeVisionProviderId: visionPid,
          activeVisionModelId: visionMid,
        };
        setSettingsLocal(nextSettings);
        await ipc.setSettings(nextSettings);
        pushToast("全局默认生图与视觉模型已更新");
      }

      close();
    } catch (err) {
      pushToast(`保存模型配置失败: ${err}`);
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/60 backdrop-blur-sm animate-in fade-in duration-150">
      <div
        className="fixed inset-0"
        onClick={close}
        aria-hidden="true"
      />

      <div className="relative w-full max-w-lg bg-panel border border-edge rounded-2xl shadow-2xl overflow-hidden flex flex-col z-10 max-h-[90vh]">
        {/* Header */}
        <div className="px-5 py-4 border-b border-edge flex items-center justify-between bg-panel2/40">
          <div className="flex items-center gap-2.5">
            <div className="p-1.5 rounded-lg bg-accent/15 text-accent">
              <Sparkles size={16} />
            </div>
            <div>
              <div className="text-[14px] font-semibold text-ink flex items-center gap-2">
                <span>能力模型设置 (生图与视觉)</span>
                {isConfiguringSession ? (
                  <span className="text-[11px] font-mono px-2 py-0.5 rounded-full bg-accent/10 border border-accent/20 text-accent truncate max-w-[200px]">
                    当前会话: {isDraft ? "新对话" : targetSession?.title}
                  </span>
                ) : (
                  <span className="text-[11px] font-mono px-2 py-0.5 rounded-full bg-purple-500/10 border border-purple-500/20 text-purple-400">
                    全局默认
                  </span>
                )}
              </div>
              <div className="text-[11.5px] text-inkdim mt-0.5">
                针对生图与视觉感知独立绑定专业模型，避免与主对话模型冲突
              </div>
            </div>
          </div>
          <button
            onClick={close}
            className="text-inkdim hover:text-ink p-1 rounded-lg hover:bg-panel3 transition-colors cursor-pointer"
          >
            <X size={16} />
          </button>
        </div>

        {/* Body */}
        <div className="p-5 space-y-3.5 overflow-y-auto flex-1">
          {/* 当前对话模型提示栏 */}
          <div className="flex items-center justify-between px-3 py-2 rounded-xl bg-panel2/60 border border-edge/60 text-[11.5px]">
            <div className="flex items-center gap-2 text-inkdim truncate min-w-0">
              <Cpu size={13} className="text-accent shrink-0" />
              <span>当前主对话模型：</span>
              <span className="font-mono text-ink font-medium truncate">
                {currentChatModelName}
              </span>
            </div>
            <span className="text-[10.5px] text-inkdim/75 shrink-0 ml-2">（在顶栏主菜单直接切换）</span>
          </div>

          {/* 1. 图像生成槽位 */}
          <div className="p-3.5 rounded-xl border border-pink-500/20 bg-pink-500/5 space-y-2">
            <div className="flex items-center justify-between">
              <div className="flex items-center gap-2">
                <span className="text-[15px]">{MODEL_CAPABILITY_METAS.image_gen.icon}</span>
                <span className="text-[12.5px] font-medium text-ink">
                  {MODEL_CAPABILITY_METAS.image_gen.label}
                </span>
                <span className="text-[10.5px] px-1.5 py-0.5 rounded border border-pink-500/30 text-pink-400 font-mono">
                  image_gen
                </span>
              </div>
              <span className="text-[11px] text-pink-400/80">
                {imageKey ? "已指定专属模型" : `跟随全局: ${activeImageDesc}`}
              </span>
            </div>
            <p className="text-[11px] text-inkdim leading-relaxed">
              {MODEL_CAPABILITY_METAS.image_gen.description}。调用 <code className="font-mono text-pink-400">generate_image</code> 工具时定向调用，杜绝纯文本模型下发导致的 400 Bad Request。
            </p>
            <ModelCapabilitySelect
              capability="image_gen"
              value={imageKey}
              onChange={setImageKey}
              providers={settings.providers}
              settings={settings}
              allowInherit={true}
              inheritLabel={`跟随全局 (${activeImageDesc})`}
            />
          </div>

          {/* 2. 视觉感知槽位 */}
          <div className="p-3.5 rounded-xl border border-purple-500/20 bg-purple-500/5 space-y-2">
            <div className="flex items-center justify-between">
              <div className="flex items-center gap-2">
                <span className="text-[15px]">{MODEL_CAPABILITY_METAS.vision.icon}</span>
                <span className="text-[12.5px] font-medium text-ink">
                  {MODEL_CAPABILITY_METAS.vision.label}
                </span>
                <span className="text-[10.5px] px-1.5 py-0.5 rounded border border-purple-500/30 text-purple-400 font-mono">
                  vision
                </span>
              </div>
              <span className="text-[11px] text-purple-400/80">
                {visionKey ? "已指定专属模型" : `跟随全局: ${activeVisionDesc}`}
              </span>
            </div>
            <p className="text-[11px] text-inkdim leading-relaxed">
              {MODEL_CAPABILITY_METAS.vision.description}。当用户上传截图、UI 设计稿或图片附件时，自动调度多模态视觉模型进行深度语义理解。
            </p>
            <ModelCapabilitySelect
              capability="vision"
              value={visionKey}
              onChange={setVisionKey}
              providers={settings.providers}
              settings={settings}
              allowInherit={true}
              inheritLabel={`跟随全局 (${activeVisionDesc})`}
            />
          </div>
        </div>

        {/* Footer */}
        <div className="px-5 py-3 border-t border-edge bg-panel2/30 flex items-center justify-end gap-2.5">
          <button
            type="button"
            onClick={close}
            className="px-3.5 py-1.5 rounded-lg border border-edge text-inkdim hover:text-ink hover:bg-panel2 text-[12px] transition-colors cursor-pointer"
          >
            取消
          </button>
          <button
            type="button"
            disabled={submitting}
            onClick={handleSave}
            className="px-4 py-1.5 rounded-lg bg-accent hover:bg-blue-500 text-white text-[12px] font-medium transition-colors shadow-sm flex items-center gap-1.5 cursor-pointer disabled:opacity-50"
          >
            <Check size={13} />
            <span>保存生效</span>
          </button>
        </div>
      </div>
    </div>
  );
}
