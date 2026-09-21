import { useState } from "react";
import { useStore, currentSession } from "../store";
import { Layers, X, Loader2, Sparkles } from "./Icons";

const ROLE_PRESETS = [
  {
    id: "frontend",
    name: "前端开发",
    icon: "🎨",
    desc: "专注 UI 界面、组件实现、交互与样式设计",
    defaultPrompt: "负责当前需求的前端模块开发，请遵循现有前端规范与组件风格，确保交互流畅与无报错。",
  },
  {
    id: "backend",
    name: "后端开发",
    icon: "⚙️",
    desc: "专注服务端逻辑、API 接口、数据处理与性能",
    defaultPrompt: "负责当前需求的后端逻辑或 API 模块开发，确保数据正确性、异常处理健全及接口规范统一。",
  },
  {
    id: "testing",
    name: "测试与校验",
    icon: "🧪",
    desc: "专注自动化测试、缺陷验证与质量检查",
    defaultPrompt: "负责编写测试用例并执行验证，全面排查潜在边界情况与回归缺陷，确保代码稳定可靠。",
  },
  {
    id: "review",
    name: "代码审阅与排查",
    icon: "🔍",
    desc: "专注代码质量审查、Bug 定位与重构优化",
    defaultPrompt: "对目标模块进行深入代码审查或 Bug 排查，提出明确的优化或修复建议并实施验证。",
  },
  {
    id: "fullstack",
    name: "全栈开发",
    icon: "⚡",
    desc: "端到端实现全功能模块与接口串联",
    defaultPrompt: "负责端到端完整功能模块开发，贯通前后端交互逻辑并确保最终可用。",
  },
  {
    id: "custom",
    name: "自定义",
    icon: "🤖",
    desc: "自由定制目标角色与专属工作任务",
    defaultPrompt: "",
  },
];

export function CreateSubagentModal() {
  const show = useStore((s) => s.showCreateSubagentModal);
  const setShow = useStore((s) => s.setShowCreateSubagentModal);
  const currentId = useStore((s) => s.currentId);
  const session = useStore((s) => currentSession(s));
  const createSubagent = useStore((s) => s.createSubagent);

  const [selectedRole, setSelectedRole] = useState("frontend");
  const [title, setTitle] = useState("前端开发任务");
  const [taskPrompt, setTaskPrompt] = useState(ROLE_PRESETS[0].defaultPrompt);
  const [submitting, setSubmitting] = useState(false);

  if (!show || !currentId || !session) return null;

  const handleRoleSelect = (roleId: string) => {
    setSelectedRole(roleId);
    const preset = ROLE_PRESETS.find((r) => r.id === roleId);
    if (preset) {
      setTitle(`${preset.name}任务`);
      if (preset.defaultPrompt) {
        setTaskPrompt(preset.defaultPrompt);
      }
    }
  };

  const handleCreate = async () => {
    if (!taskPrompt.trim() || submitting) return;
    setSubmitting(true);
    try {
      await createSubagent({
        parentSessionId: currentId,
        role: selectedRole,
        title: title.trim() || undefined,
        taskPrompt: taskPrompt.trim(),
      });
      handleRoleSelect("frontend");
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 backdrop-blur-xs p-4 animate-in fade-in duration-150">
      <div className="bg-panel border border-edge rounded-2xl w-full max-w-[560px] shadow-2xl flex flex-col max-h-[90vh] overflow-hidden">
        {/* Header */}
        <div className="flex items-center justify-between px-5 py-4 border-b border-edge bg-panel2/40">
          <div className="flex items-center gap-2">
            <div className="w-8 h-8 rounded-xl bg-accent/15 border border-accent/30 flex items-center justify-center text-accent">
              <Layers size={17} />
            </div>
            <div>
              <div className="font-semibold text-ink text-[15px]">创建子 Agent 协作进程</div>
              <div className="text-[12px] text-inkdim">在独立子进程中并行执行开发、排查或测试任务</div>
            </div>
          </div>
          <button
            onClick={() => setShow(false)}
            className="w-7 h-7 rounded-lg hover:bg-panel2 flex items-center justify-center text-inkdim hover:text-ink transition-colors"
          >
            <X size={16} />
          </button>
        </div>

        {/* Content */}
        <div className="flex-1 overflow-y-auto p-5 space-y-4">
          {/* Role selection */}
          <div>
            <label className="block text-[13px] font-medium text-ink mb-2">选择子 Agent 角色定位</label>
            <div className="grid grid-cols-2 sm:grid-cols-3 gap-2">
              {ROLE_PRESETS.map((preset) => {
                const isSelected = selectedRole === preset.id;
                return (
                  <button
                    key={preset.id}
                    type="button"
                    onClick={() => handleRoleSelect(preset.id)}
                    className={`flex flex-col items-start p-2.5 rounded-xl border text-left transition-all ${
                      isSelected
                        ? "bg-accent/10 border-accent text-ink shadow-xs ring-1 ring-accent/30"
                        : "bg-panel2/50 hover:bg-panel2 border-edge text-inkdim hover:text-ink"
                    }`}
                  >
                    <div className="flex items-center gap-1.5 font-medium text-[13px]">
                      <span className="text-base">{preset.icon}</span>
                      <span>{preset.name}</span>
                    </div>
                    <div className="text-[11px] text-inkdim mt-1 line-clamp-2 leading-tight">
                      {preset.desc}
                    </div>
                  </button>
                );
              })}
            </div>
          </div>

          {/* Subagent Name/Title */}
          <div>
            <label className="block text-[13px] font-medium text-ink mb-1.5">子任务标题 / 名称</label>
            <input
              type="text"
              value={title}
              onChange={(e) => setTitle(e.target.value)}
              placeholder="例如：开发前端登录卡片组件、重构数据导出接口"
              className="w-full px-3 py-2 rounded-xl bg-panel2/60 border border-edge text-ink text-[13px] focus:outline-none focus:border-accent"
            />
          </div>

          {/* Task prompt */}
          <div>
            <label className="block text-[13px] font-medium text-ink mb-1.5">
              任务目标与详细要求 <span className="text-rose-400">*</span>
            </label>
            <textarea
              rows={5}
              value={taskPrompt}
              onChange={(e) => setTaskPrompt(e.target.value)}
              placeholder="详细描述需要该子 Agent 完成的目标与具体要求。工作区物理根目录将与主项目保持严格一致；如需聚焦特定子目录（如 src/components），直接在此描述中指明即可..."
              className="w-full px-3 py-2.5 rounded-xl bg-panel2/60 border border-edge text-ink text-[13px] focus:outline-none focus:border-accent resize-none"
            />
            <div className="flex items-center gap-1 text-[11px] text-inkdim mt-1">
              <Sparkles size={12} className="text-accent shrink-0" />
              <span>工作区与主会话严格保持一致；子 Agent 将获得独立上下文空间与工具执行能力。</span>
            </div>
          </div>
        </div>

        {/* Footer */}
        <div className="px-5 py-3.5 border-t border-edge bg-panel2/30 flex items-center justify-end gap-2.5">
          <button
            type="button"
            onClick={() => setShow(false)}
            className="px-4 py-2 rounded-xl border border-edge text-inkdim hover:text-ink hover:bg-panel2 text-[13px] transition-colors"
          >
            取消
          </button>
          <button
            type="button"
            disabled={!taskPrompt.trim() || submitting}
            onClick={handleCreate}
            className="px-4 py-2 rounded-xl bg-accent hover:bg-blue-500 disabled:opacity-50 text-white text-[13px] font-medium transition-colors shadow-sm flex items-center gap-1.5"
          >
            {submitting ? (
              <>
                <Loader2 size={14} className="animate-spin" />
                <span>创建启动中…</span>
              </>
            ) : (
              <span>创建并启动</span>
            )}
          </button>
        </div>
      </div>
    </div>
  );
}
