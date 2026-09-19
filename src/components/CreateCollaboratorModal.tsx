import { useState, useEffect } from "react";
import { useStore, currentSession } from "../store";
import { Users, X, Loader2, Sparkles, Folder, Check } from "./Icons";

const ROLE_PRESETS = [
  {
    id: "frontend",
    name: "前端开发",
    icon: "🎨",
    desc: "专注 UI 界面、组件实现、交互与样式设计",
    defaultPrompt: "负责项目的前端模块开发与样式交互优化，遵循项目现有规范，确保界面流畅与体验统一。",
  },
  {
    id: "backend",
    name: "后端开发",
    icon: "⚙️",
    desc: "专注服务端逻辑、API 接口、数据处理与性能",
    defaultPrompt: "负责服务端的业务逻辑与 API 接口开发，确保数据一致性、异常健全与性能稳定。",
  },
  {
    id: "testing",
    name: "测试与校验",
    icon: "🧪",
    desc: "专注自动化测试、缺陷验证与质量检查",
    defaultPrompt: "编写单元测试与集成验证用例，全面覆盖关键逻辑边界，防范回归缺陷。",
  },
  {
    id: "review",
    name: "代码审阅与排查",
    icon: "🔍",
    desc: "专注代码质量审查、Bug 定位与重构优化",
    defaultPrompt: "深入分析代码设计与潜在隐患，提出针对性重构建议并实施精确修复。",
  },
  {
    id: "fullstack",
    name: "全栈开发",
    icon: "⚡",
    desc: "端到端实现全功能模块与接口串联",
    defaultPrompt: "端到端打通前后端业务链路，实现高内聚低耦合的完整功能模块。",
  },
  {
    id: "custom",
    name: "自定义角色",
    icon: "🤝",
    desc: "自由定制团队协作者的专属职责与定位",
    defaultPrompt: "",
  },
];

export function CreateCollaboratorModal() {
  const show = useStore((s) => s.showCreateCollaboratorModal);
  const setShow = useStore((s) => s.setShowCreateCollaboratorModal);
  const currentId = useStore((s) => s.currentId);
  const session = useStore((s) => currentSession(s));
  const draft = useStore((s) => s.draft);
  const createCollaborator = useStore((s) => s.createCollaborator);

  const [selectedRole, setSelectedRole] = useState("frontend");
  const [title, setTitle] = useState("前端开发协作者");
  const [taskPrompt, setTaskPrompt] = useState(ROLE_PRESETS[0].defaultPrompt);
  const [workspacePath, setWorkspacePath] = useState(session?.workspacePath ?? draft?.workspacePath ?? "");
  const [subpath, setSubpath] = useState("");
  const [autoReport, setAutoReport] = useState(true);
  const [submitting, setSubmitting] = useState(false);

  useEffect(() => {
    if (show) {
      setWorkspacePath(session?.workspacePath ?? draft?.workspacePath ?? "");
    }
  }, [show, session?.workspacePath, draft?.workspacePath]);

  if (!show || !currentId) return null;

  const handleRoleSelect = (roleId: string) => {
    setSelectedRole(roleId);
    const preset = ROLE_PRESETS.find((r) => r.id === roleId);
    if (preset) {
      setTitle(`${preset.name}协作者`);
      if (preset.defaultPrompt) {
        setTaskPrompt(preset.defaultPrompt);
      }
    }
  };

  const handleCreate = async () => {
    if (!taskPrompt.trim() || submitting) return;
    setSubmitting(true);
    try {
      await createCollaborator({
        parentSessionId: currentId,
        role: selectedRole,
        title: title.trim() || undefined,
        taskPrompt: taskPrompt.trim(),
        subpath: subpath.trim() || undefined,
        workspacePath: workspacePath.trim() || undefined,
        autoReport,
      });
      handleRoleSelect("frontend");
      setSubpath("");
      setAutoReport(true);
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 backdrop-blur-xs p-4 animate-in fade-in duration-150">
      <div className="bg-panel border border-edge rounded-2xl w-full max-w-[560px] shadow-2xl flex flex-col max-h-[90vh] overflow-hidden">
        {/* Header */}
        <div className="flex items-center justify-between px-5 py-4 border-b border-edge bg-panel2/40">
          <div className="flex items-center gap-2.5">
            <div className="w-8 h-8 rounded-xl bg-accent/15 border border-accent/30 flex items-center justify-center text-accent">
              <Users size={17} />
            </div>
            <div>
              <div className="font-semibold text-ink text-[15px]">添加项目协作者 (Collaborator)</div>
              <div className="text-[12px] text-inkdim">常驻团队角色，主进程将自动委派任务，也可手动交互协作</div>
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
        <div className="flex-1 overflow-y-auto p-5 space-y-4">
          {/* Role selection */}
          <div>
            <label className="block text-[13px] font-medium text-ink mb-2">选择协作者角色定位</label>
            <div className="grid grid-cols-2 sm:grid-cols-3 gap-2">
              {ROLE_PRESETS.map((preset) => {
                const isSelected = selectedRole === preset.id;
                return (
                  <button
                    key={preset.id}
                    type="button"
                    onClick={() => handleRoleSelect(preset.id)}
                    className={`flex flex-col items-start p-2.5 rounded-xl border text-left transition-all cursor-pointer ${
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

          {/* Collaborator Title / Name */}
          <div>
            <label className="block text-[13px] font-medium text-ink mb-1.5">协作者名称 / 称谓</label>
            <input
              type="text"
              value={title}
              onChange={(e) => setTitle(e.target.value)}
              placeholder="例如：前端架构专家、订单系统后端协作者"
              className="w-full px-3 py-2 rounded-xl bg-panel2/60 border border-edge text-ink text-[13px] focus:outline-none focus:border-accent"
            />
          </div>

          {/* Workspace Path & Subpath */}
          <div className="grid grid-cols-1 sm:grid-cols-2 gap-2.5">
            <div>
              <label className="block text-[13px] font-medium text-ink mb-1.5 flex items-center justify-between">
                <span className="flex items-center gap-1">
                  <Folder size={13} className="text-inkdim" />
                  <span>工作区根目录</span>
                </span>
                <span className="text-[11px] text-inkdim font-normal">执行基准根</span>
              </label>
              <input
                type="text"
                value={workspacePath}
                onChange={(e) => setWorkspacePath(e.target.value)}
                placeholder={session?.workspacePath || draft?.workspacePath || "继承当前主项目根目录"}
                className="w-full px-3 py-2 rounded-xl bg-panel2/60 border border-edge text-ink text-[12px] font-mono focus:outline-none focus:border-accent"
                title="协作者物理执行根目录，默认继承当前项目根目录"
              />
            </div>
            <div>
              <label className="block text-[13px] font-medium text-ink mb-1.5 flex items-center justify-between">
                <span>专注子目录</span>
                <span className="text-[11px] text-inkdim font-normal">可选范围指引</span>
              </label>
              <input
                type="text"
                value={subpath}
                onChange={(e) => setSubpath(e.target.value)}
                placeholder="例如：src/components 或 api"
                className="w-full px-3 py-2 rounded-xl bg-panel2/60 border border-edge text-ink text-[12px] font-mono focus:outline-none focus:border-accent"
                title="引导协作者优先专注该子目录开展开发"
              />
            </div>
          </div>

          {/* Role prompt / description */}
          <div>
            <label className="block text-[13px] font-medium text-ink mb-1.5">
              专业职责与定位描述 <span className="text-rose-400">*</span>
            </label>
            <textarea
              rows={3}
              value={taskPrompt}
              onChange={(e) => setTaskPrompt(e.target.value)}
              placeholder="详细描述该协作者的职责定位、技能范围或执行标准..."
              className="w-full px-3 py-2 rounded-xl bg-panel2/60 border border-edge text-ink text-[13px] focus:outline-none focus:border-accent resize-none"
            />
            <div className="flex items-center gap-1 text-[11px] text-inkdim mt-1">
              <Sparkles size={12} className="text-accent shrink-0" />
              <span>主进程在规划相关任务时将自动感知其职责并在空闲时优先派发。</span>
            </div>
          </div>

          {/* Auto Report Checkbox */}
          <div className="pt-1">
            <label className="flex items-center gap-2.5 cursor-pointer select-none">
              <input
                type="checkbox"
                checked={autoReport}
                onChange={(e) => setAutoReport(e.target.checked)}
                className="sr-only"
              />
              <div
                className={`w-4 h-4 rounded border flex items-center justify-center transition-colors ${
                  autoReport ? "bg-accent border-accent text-white" : "border-edge bg-panel2"
                }`}
              >
                {autoReport && <Check size={11} strokeWidth={3} />}
              </div>
              <div className="text-[12px] text-ink">
                <span>执行完成自动汇报给主任务</span>
                <span className="text-inkdim ml-1.5 text-[11px]">（每轮任务执行完毕后，自动提取增量结论并唤醒主进程）</span>
              </div>
            </label>
          </div>
        </div>

        {/* Footer */}
        <div className="px-5 py-3.5 border-t border-edge bg-panel2/30 flex items-center justify-end gap-2.5">
          <button
            type="button"
            onClick={() => setShow(false)}
            className="px-4 py-2 rounded-xl border border-edge text-inkdim hover:text-ink hover:bg-panel2 text-[13px] transition-colors cursor-pointer"
          >
            取消
          </button>
          <button
            type="button"
            disabled={!taskPrompt.trim() || submitting}
            onClick={handleCreate}
            className="px-4 py-2 rounded-xl bg-accent hover:bg-blue-500 disabled:opacity-50 text-white text-[13px] font-medium transition-colors shadow-sm flex items-center gap-1.5 cursor-pointer"
          >
            {submitting ? (
              <>
                <Loader2 size={14} className="animate-spin" />
                <span>创建就绪中…</span>
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
