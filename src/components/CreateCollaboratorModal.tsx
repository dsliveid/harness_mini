import { useState } from "react";
import { useStore, currentSession } from "../store";
import { hasModelCapability } from "../types";
import {
  Users,
  X,
  Loader2,
  Sparkles,
  Check,
  Cpu,
  Sliders,
  Layout,
  Server,
  ClipboardList,
  ListTodo,
  Eye,
  Palette,
  FlaskConical,
  Search,
  Layers,
  MessageSquare,
} from "./Icons";
import { ModelCapabilitySelect } from "./ModelCapabilitySelect";

const ROLE_PRESETS = [
  {
    id: "frontend",
    name: "前端开发",
    icon: Layout,
    desc: "专注 UI 界面、组件实现、交互与样式设计",
    defaultPrompt: "负责项目的前端模块开发与样式交互优化，遵循项目现有规范，确保界面流畅与体验统一。",
    defaultDispatchRule: "当涉及 UI 界面设计、页面实现、组件重构、Vue/React 模板与 CSS 交互开发时，必须优先委派本协作者。",
  },
  {
    id: "backend",
    name: "后端开发",
    icon: Server,
    desc: "专注服务端逻辑、API 接口、数据处理与性能",
    defaultPrompt: "负责服务端的业务逻辑与 API 接口开发，确保数据一致性、异常健全与性能稳定。",
    defaultDispatchRule: "当涉及服务端业务逻辑、API 接口、数据库 CRUD、后台架构开发时，必须优先委派本协作者。",
  },
  {
    id: "pm",
    name: "产品经理",
    icon: ClipboardList,
    desc: "专注需求拆解、PRD方案制定与业务边界梳理",
    defaultPrompt: "作为产品经理，负责将用户需求拆解细化为清晰的 PRD 需求文档、交互流程和功能边界，为研发团队提供清晰的产品定义与方案。",
    defaultDispatchRule: "当涉及需求分析梳理、PRD 方案编写、功能边界与业务流程设计时，必须优先委派本协作者。",
  },
  {
    id: "pmo",
    name: "项目经理",
    icon: ListTodo,
    desc: "专注任务拆解(WBS)、里程碑排期与进度追踪",
    defaultPrompt: "作为项目经理 (PMO)，负责拆解任务清单、统筹各协作者里程碑排期、识别关键风险并推动交付闭环。",
    defaultDispatchRule: "当涉及任务拆解(WBS)、里程碑节点排期、进度与风险追踪时，必须优先委派本协作者。",
  },
  {
    id: "vision",
    name: "图像识别",
    icon: Eye,
    desc: "专注多模态图片识别、设计稿解析与视觉分析",
    defaultPrompt: "作为多模态视觉协作者，负责精准识别分析用户提供的图片、设计稿或运行报错截图，提取关键结构并向主进程汇报成果。",
    defaultDispatchRule: "当用户发送图片、截图、设计稿并要求视觉识别分析时，必须优先委派本协作者。",
  },
  {
    id: "image_gen",
    name: "图像生成",
    icon: Palette,
    desc: "专注图片生成、提示词润色与视觉配图制作",
    defaultPrompt: "作为图像生成协作者，负责根据具体场景构思提示词并调用 generate_image 工具生成图片，产出素材并汇报主进程。",
    defaultDispatchRule: "当用户提出画图、生成图片、插图、海报、Logo、图标、配图制作等视觉生成需求时，必须优先委派本协作者。",
  },
  {
    id: "testing",
    name: "测试校验",
    icon: FlaskConical,
    desc: "专注自动化测试、缺陷验证与质量检查",
    defaultPrompt: "编写单元测试与集成验证用例，全面覆盖关键逻辑边界，防范回归缺陷。",
    defaultDispatchRule: "当需要编写自动化测试用例、单元测试、执行回归测试与缺陷验证时，必须优先委派本协作者。",
  },
  {
    id: "review",
    name: "代码审阅",
    icon: Search,
    desc: "专注代码质量审查、Bug 定位与重构优化",
    defaultPrompt: "深入分析代码设计与潜在隐患，提出针对性重构建议并实施精确修复。",
    defaultDispatchRule: "当需要对代码实现进行质量审查、重构优化与架构防劣化时，必须优先委派本协作者。",
  },
  {
    id: "fullstack",
    name: "全栈开发",
    icon: Layers,
    desc: "端到端实现全功能模块与前后端串联",
    defaultPrompt: "端到端打通前后端业务链路，实现高内聚低耦合的完整功能模块。",
    defaultDispatchRule: "当涉及端到端打通前后端完整功能链路开发时，必须优先委派本协作者。",
  },
  {
    id: "custom",
    name: "自定义",
    icon: Users,
    desc: "自由定制专属职责定位与执行模型",
    defaultPrompt: "",
    defaultDispatchRule: "当用户任务属于本协作者专业领域范围时，必须优先委派本协作者处理。",
  },
];

export function CreateCollaboratorModal() {
  const show = useStore((s) => s.showCreateCollaboratorModal);
  const setShow = useStore((s) => s.setShowCreateCollaboratorModal);
  const currentId = useStore((s) => s.currentId);
  const session = useStore((s) => currentSession(s));
  const draft = useStore((s) => s.draft);
  const settings = useStore((s) => s.settings);
  const createCollaborator = useStore((s) => s.createCollaborator);

  const [selectedRole, setSelectedRole] = useState("frontend");
  const [title, setTitle] = useState("前端开发");
  const [taskPrompt, setTaskPrompt] = useState(ROLE_PRESETS[0].defaultPrompt);
  const [dispatchRule, setDispatchRule] = useState(ROLE_PRESETS[0].defaultDispatchRule);
  const [autoReport, setAutoReport] = useState(true);
  const [submitting, setSubmitting] = useState(false);
  const [selectedModelKey, setSelectedModelKey] = useState<string>(""); // format: "providerId::modelId" (chat)
  const [selectedImageModelKey, setSelectedImageModelKey] = useState<string>(""); // format: "providerId::modelId" (image_gen)
  const [selectedVisionModelKey, setSelectedVisionModelKey] = useState<string>(""); // format: "providerId::modelId" (vision)

  const providers = settings?.providers ?? [];

  if (!show || !currentId) return null;

  const handleRoleSelect = (roleId: string) => {
    setSelectedRole(roleId);
    const preset = ROLE_PRESETS.find((r) => r.id === roleId);
    if (preset) {
      setTitle(preset.name);
      if (preset.defaultPrompt) {
        setTaskPrompt(preset.defaultPrompt);
      }
      setDispatchRule(preset.defaultDispatchRule);
    }

    // 智能模型预选：如果是图像生成或视觉识别角色，自动尝试挑选对应的专属模型
    if (roleId === "image_gen") {
      for (const p of providers) {
        const found = p.models.find((m) => hasModelCapability(settings, p.id, m, "image_gen"));
        if (found) {
          setSelectedImageModelKey(`${p.id}::${found}`);
          break;
        }
      }
    } else if (roleId === "vision") {
      for (const p of providers) {
        const found = p.models.find((m) => hasModelCapability(settings, p.id, m, "vision"));
        if (found) {
          setSelectedVisionModelKey(`${p.id}::${found}`);
          break;
        }
      }
    }
  };

  const handleCreate = async () => {
    if (!taskPrompt.trim() || submitting) return;
    setSubmitting(true);
    try {
      let providerId: string | undefined = undefined;
      let modelId: string | undefined = undefined;
      if (selectedModelKey) {
        const parts = selectedModelKey.split("::");
        if (parts.length === 2) {
          providerId = parts[0];
          modelId = parts[1];
        }
      }

      let imageProviderId: string | undefined = undefined;
      let imageModelId: string | undefined = undefined;
      if (selectedImageModelKey) {
        const parts = selectedImageModelKey.split("::");
        if (parts.length === 2) {
          imageProviderId = parts[0];
          imageModelId = parts[1];
        }
      }

      let visionProviderId: string | undefined = undefined;
      let visionModelId: string | undefined = undefined;
      if (selectedVisionModelKey) {
        const parts = selectedVisionModelKey.split("::");
        if (parts.length === 2) {
          visionProviderId = parts[0];
          visionModelId = parts[1];
        }
      }

      await createCollaborator({
        parentSessionId: currentId,
        role: selectedRole,
        title: title.trim() || undefined,
        taskPrompt: taskPrompt.trim(),
        dispatchRule: dispatchRule.trim() || undefined,
        workspacePath: session?.workspacePath ?? draft?.workspacePath ?? undefined,
        autoReport,
        providerId,
        modelId,
        imageProviderId,
        imageModelId,
        visionProviderId,
        visionModelId,
      });
      handleRoleSelect("frontend");
      setAutoReport(true);
      setSelectedModelKey("");
      setSelectedImageModelKey("");
      setSelectedVisionModelKey("");
    } finally {
      setSubmitting(false);
    }
  };

  const activeModelName = settings?.activeModelId || settings?.activeModel || "全局激活模型";


  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 backdrop-blur-xs p-4 animate-in fade-in duration-150">
      <div className="bg-panel border border-edge rounded-2xl w-full max-w-[540px] shadow-2xl flex flex-col max-h-[85vh] overflow-hidden">
        {/* Header */}
        <div className="flex items-center justify-between px-5 py-3.5 border-b border-edge bg-panel2/40">
          <div className="flex items-center gap-2.5">
            <div className="w-8 h-8 rounded-xl bg-accent/15 border border-accent/30 flex items-center justify-center text-accent">
              <Users size={17} />
            </div>
            <div>
              <div className="font-semibold text-ink text-[14px]">添加项目协作者 (Collaborator)</div>
              <div className="text-[11px] text-inkdim">常驻团队角色，主进程自动派发任务，支持指定独立模型</div>
            </div>
          </div>
          <button
            onClick={() => setShow(false)}
            className="w-7 h-7 rounded-lg hover:bg-panel2 flex items-center justify-center text-inkdim hover:text-ink transition-colors cursor-pointer"
          >
            <X size={16} />
          </button>
        </div>

        {/* Content */}
        <div className="flex-1 overflow-y-auto p-4 space-y-3">
          {/* Role selection - Compact pills grid */}
          <div>
            <div className="flex items-center justify-between mb-1.5">
              <label className="text-[12px] font-medium text-ink">选择角色定位</label>
              <span className="text-[11px] text-inkdim">
                {ROLE_PRESETS.find((r) => r.id === selectedRole)?.desc}
              </span>
            </div>
            <div className="grid grid-cols-2 sm:grid-cols-4 gap-1.5">
              {ROLE_PRESETS.map((preset) => {
                const isSelected = selectedRole === preset.id;
                return (
                  <button
                    key={preset.id}
                    type="button"
                    title={preset.desc}
                    onClick={() => handleRoleSelect(preset.id)}
                    className={`flex items-center gap-1.5 px-2.5 py-1.5 rounded-lg border text-left transition-all cursor-pointer ${
                      isSelected
                        ? "bg-accent/15 border-accent text-accent font-medium shadow-xs ring-1 ring-accent/30"
                        : "bg-panel2/40 hover:bg-panel2 border-edge text-inkdim hover:text-ink"
                    }`}
                  >
                    <preset.icon size={14} className="shrink-0 text-accent/80" />
                    <span className="text-[12px] truncate">{preset.name}</span>
                  </button>
                );
              })}
            </div>
          </div>

          {/* Collaborator Title */}
          <div>
            <label className="block text-[12px] font-medium text-ink mb-1">协作者名称</label>
            <input
              type="text"
              value={title}
              onChange={(e) => setTitle(e.target.value)}
              placeholder="例如：视觉还原专家、产品经理"
              className="w-full px-3 py-1.5 rounded-lg bg-panel2/60 border border-edge text-ink text-[12px] focus:outline-none focus:border-accent"
            />
          </div>

          {/* 全能力模型独立配置矩阵 */}
          <div className="bg-panel2/30 border border-edge/80 rounded-xl p-3 space-y-2.5">
            <div className="flex items-center justify-between border-b border-edge/40 pb-2">
              <div className="flex items-center gap-1.5 text-[12px] font-semibold text-ink">
                <Sliders size={13} className="text-accent" />
                <span>各能力执行模型配置</span>
              </div>
              <span className="text-[10.5px] text-inkdim">
                不支持的能力在下拉选项中已置灰禁用
              </span>
            </div>

            <div className="grid grid-cols-1 md:grid-cols-3 gap-2.5">
              {/* 1. 对话思考 */}
              <div className="bg-panel2/60 border border-blue-500/20 rounded-lg p-2.5 flex flex-col justify-between gap-1.5">
                <div>
                  <div className="flex items-center justify-between text-[11.5px] font-medium text-blue-400">
                    <span className="flex items-center gap-1.5">
                      <MessageSquare size={13} />
                      <span>对话思考</span>
                    </span>
                    <span className="text-[10px] font-mono text-inkdim">chat</span>
                  </div>
                  <div className="text-[10.5px] text-inkdim leading-tight mt-0.5 mb-1.5">
                    负责日常规划、逻辑推理与对话
                  </div>
                </div>
                <ModelCapabilitySelect
                  capability="chat"
                  value={selectedModelKey}
                  onChange={setSelectedModelKey}
                  providers={providers}
                  settings={settings}
                  allowInherit={true}
                  inheritLabel={`继承主进程 (${activeModelName})`}
                />
              </div>

              {/* 2. 图像生成 */}
              <div className="bg-panel2/60 border border-pink-500/20 rounded-lg p-2.5 flex flex-col justify-between gap-1.5">
                <div>
                  <div className="flex items-center justify-between text-[11.5px] font-medium text-pink-400">
                    <span className="flex items-center gap-1.5">
                      <Palette size={13} />
                      <span>图像生成</span>
                    </span>
                    <span className="text-[10px] font-mono text-inkdim">image_gen</span>
                  </div>
                  <div className="text-[10.5px] text-inkdim leading-tight mt-0.5 mb-1.5">
                    调用生图工具 generate_image 时执行
                  </div>
                </div>
                <ModelCapabilitySelect
                  capability="image_gen"
                  value={selectedImageModelKey}
                  onChange={setSelectedImageModelKey}
                  providers={providers}
                  settings={settings}
                  allowInherit={true}
                />
              </div>

              {/* 3. 视觉感知 */}
              <div className="bg-panel2/60 border border-purple-500/20 rounded-lg p-2.5 flex flex-col justify-between gap-1.5">
                <div>
                  <div className="flex items-center justify-between text-[11.5px] font-medium text-purple-400">
                    <span className="flex items-center gap-1.5">
                      <Eye size={13} />
                      <span>视觉感知</span>
                    </span>
                    <span className="text-[10px] font-mono text-inkdim">vision</span>
                  </div>
                  <div className="text-[10.5px] text-inkdim leading-tight mt-0.5 mb-1.5">
                    处理图片附件与多模态解析
                  </div>
                </div>
                <ModelCapabilitySelect
                  capability="vision"
                  value={selectedVisionModelKey}
                  onChange={setSelectedVisionModelKey}
                  providers={providers}
                  settings={settings}
                  allowInherit={true}
                />
              </div>
            </div>
          </div>

          {/* 主进程调度触发规则 */}
          <div>
            <label className="block text-[12px] font-medium text-ink mb-1 flex items-center justify-between">
              <span className="flex items-center gap-1">
                <Sparkles size={12} className="text-accent" />
                <span>主进程调度触发规则 (注入主进程)</span>
                <span className="text-rose-400">*</span>
              </span>
              <span className="text-[11px] text-inkdim font-normal">主进程据此意图优先委派</span>
            </label>
            <textarea
              rows={2}
              value={dispatchRule}
              onChange={(e) => setDispatchRule(e.target.value)}
              placeholder="定义主进程在什么场景下必须把任务优先委派给本协作者..."
              className="w-full px-3 py-1.5 rounded-lg bg-panel2/60 border border-edge text-ink text-[12px] focus:outline-none focus:border-accent resize-none leading-relaxed"
            />
          </div>

          {/* Role prompt / description */}
          <div>
            <label className="block text-[12px] font-medium text-ink mb-1 flex items-center justify-between">
              <span>
                专业职责与定位描述 <span className="text-rose-400">*</span>
              </span>
              <span className="text-[11px] text-inkdim font-normal">协作者自身的 System Prompt 职责</span>
            </label>
            <textarea
              rows={2}
              value={taskPrompt}
              onChange={(e) => setTaskPrompt(e.target.value)}
              placeholder="详细描述该协作者的职责定位、技能要求或输出标准..."
              className="w-full px-3 py-1.5 rounded-lg bg-panel2/60 border border-edge text-ink text-[12px] focus:outline-none focus:border-accent resize-none leading-relaxed"
            />
          </div>

          {/* Auto Report Checkbox */}
          <div>
            <label className="flex items-center gap-2 cursor-pointer select-none">
              <input
                type="checkbox"
                checked={autoReport}
                onChange={(e) => setAutoReport(e.target.checked)}
                className="sr-only"
              />
              <div
                className={`w-3.5 h-3.5 rounded border flex items-center justify-center transition-colors ${
                  autoReport ? "bg-accent border-accent text-white" : "border-edge bg-panel2"
                }`}
              >
                {autoReport && <Check size={10} strokeWidth={3} />}
              </div>
              <div className="text-[11.5px] text-ink">
                <span>执行完成自动汇总汇报主任务</span>
                <span className="text-inkdim ml-1 text-[11px]">（完成时自动唤醒主进程衔接后续）</span>
              </div>
            </label>
          </div>
        </div>

        {/* Footer */}
        <div className="px-5 py-3 border-t border-edge bg-panel2/30 flex items-center justify-end gap-2.5">
          <button
            type="button"
            onClick={() => setShow(false)}
            className="px-3.5 py-1.5 rounded-lg border border-edge text-inkdim hover:text-ink hover:bg-panel2 text-[12px] transition-colors cursor-pointer"
          >
            取消
          </button>
          <button
            type="button"
            disabled={!taskPrompt.trim() || submitting}
            onClick={handleCreate}
            className="px-4 py-1.5 rounded-lg bg-accent hover:bg-blue-500 disabled:opacity-50 text-white text-[12px] font-medium transition-colors shadow-sm flex items-center gap-1.5 cursor-pointer"
          >
            {submitting ? (
              <>
                <Loader2 size={13} className="animate-spin" />
                <span>就绪中…</span>
              </>
            ) : (
              <span>创建协作者</span>
            )}
          </button>
        </div>
      </div>
    </div>
  );
}

