import { useEffect, useState, useMemo } from "react";
import { useStore } from "../store";
import { ipc } from "../ipc";
import type { GrowthItem, SkillItem, ProjectSopInfo } from "../types";
import { Markdown } from "./Markdown";
import { askConfirm } from "./PromptModal";
import {
  Sprout,
  Sparkles,
  Zap,
  ShieldCheck,
  Folder,
  FolderOpen,
  Globe,
  Check,
  Edit3,
  Trash2,
  Brain,
  Link2,
  Plus,
  Play,
  Lightbulb,
  Target,
  ChevronRight,
  ChevronDown,
  X,
} from "./Icons";

const CATEGORY_NAMES: Record<string, string> = {
  command_rule: "命令规范",
  code_style: "代码规范",
  build_test: "构建测试",
  pitfall: "避坑防雷",
  workflow: "工作流",
};

const CATEGORY_COLORS: Record<string, string> = {
  command_rule: "bg-blue-500/15 text-blue-400 border-blue-500/30",
  code_style: "bg-purple-500/15 text-purple-400 border-purple-500/30",
  build_test: "bg-amber-500/15 text-amber-400 border-amber-500/30",
  pitfall: "bg-red-500/15 text-red-400 border-red-500/30",
  workflow: "bg-teal-500/15 text-teal-400 border-teal-500/30",
};

export function GrowthModal() {
  const show = useStore((s) => s.showGrowthModal);
  const setShow = useStore((s) => s.setShowGrowthModal);
  const initialProjectId = useStore((s) => s.growthModalProjectId);
  const projects = useStore((s) => s.projects);
  const growths = useStore((s) => s.growths);
  const loadGrowths = useStore((s) => s.loadGrowths);
  const toggleGrowth = useStore((s) => s.toggleGrowth);
  const acceptGrowth = useStore((s) => s.acceptGrowth);
  const updateGrowthRule = useStore((s) => s.updateGrowthRule);
  const deleteGrowth = useStore((s) => s.deleteGrowth);
  const selectSession = useStore((s) => s.selectSession);
  const triggerManualGrowth = useStore((s) => s.triggerManualGrowth);
  const currentId = useStore((s) => s.currentId);
  const pushToast = useStore((s) => s.pushToast);

  // 顶层三阶选项卡
  const [activeTab, setActiveTab] = useState<"reflexion" | "skills" | "sop">("reflexion");

  // 阶梯 1：经验反思状态
  const [selectedProject, setSelectedProject] = useState<string>("all");
  const [selectedCategory, setSelectedCategory] = useState<string>("all");
  const [selectedStatus, setSelectedStatus] = useState<string>("active");
  const [searchQuery, setSearchQuery] = useState("");
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editTitle, setEditTitle] = useState("");
  const [editContent, setEditContent] = useState("");
  const [editCategory, setEditCategory] = useState("");
  const [expandedThoughts, setExpandedThoughts] = useState<Record<string, boolean>>({});

  // 阶梯 2：项目技能状态
  const [skillsProject, setSkillsProject] = useState<string>("");
  const [skills, setSkills] = useState<SkillItem[]>([]);
  const [skillsLoading, setSkillsLoading] = useState(false);
  const [showSkillEditor, setShowSkillEditor] = useState(false);
  const [editingSkillName, setEditingSkillName] = useState<string | null>(null);
  const [skillFormName, setSkillFormName] = useState("");
  const [skillFormDesc, setSkillFormDesc] = useState("");
  const [skillFormType, setSkillFormType] = useState<string>("bat");
  const [skillFormContent, setSkillFormContent] = useState("");

  // 阶梯 3：交付 SOP 状态
  const [sopProject, setSopProject] = useState<string>("");
  const [sopInfo, setSopInfo] = useState<ProjectSopInfo | null>(null);
  const [sopLoading, setSopLoading] = useState(false);
  const [sopVerifyCmd, setSopVerifyCmd] = useState("");
  const [sopEnabled, setSopEnabled] = useState(true);
  const [testRunning, setTestRunning] = useState(false);
  const [testOutput, setTestOutput] = useState<string | null>(null);
  const [testSuccess, setTestSuccess] = useState<boolean | null>(null);

  // 初始化设置
  useEffect(() => {
    if (show) {
      if (initialProjectId) {
        setSelectedProject(initialProjectId);
        setSkillsProject(initialProjectId);
        setSopProject(initialProjectId);
      } else {
        setSelectedProject("all");
        const firstWithPath = projects.find((p) => p.path)?.id || projects[0]?.id || "";
        setSkillsProject(firstWithPath);
        setSopProject(firstWithPath);
      }
      void loadGrowths();
    }
  }, [show, initialProjectId, loadGrowths, projects]);

  // 加载当前项目的技能列表
  const loadCurrentSkills = async (projId: string) => {
    const proj = projects.find((p) => p.id === projId);
    if (!proj?.path) {
      setSkills([]);
      return;
    }
    setSkillsLoading(true);
    try {
      const list = await ipc.listProjectSkills(proj.path);
      setSkills(list);
    } catch (e) {
      pushToast(`加载技能列表失败: ${e}`);
    } finally {
      setSkillsLoading(false);
    }
  };

  useEffect(() => {
    if (show && activeTab === "skills" && skillsProject) {
      void loadCurrentSkills(skillsProject);
    }
  }, [show, activeTab, skillsProject]);

  // 加载当前项目的 SOP 配置
  const loadCurrentSop = async (projId: string) => {
    const proj = projects.find((p) => p.id === projId);
    if (!proj?.path) {
      setSopInfo(null);
      return;
    }
    setSopLoading(true);
    setTestOutput(null);
    setTestSuccess(null);
    try {
      const info = await ipc.getProjectSop(proj.path, proj.id);
      setSopInfo(info);
      setSopVerifyCmd(info.sopVerifyCmd);
      setSopEnabled(info.sopEnabled);
    } catch (e) {
      pushToast(`加载 SOP 配置失败: ${e}`);
    } finally {
      setSopLoading(false);
    }
  };

  useEffect(() => {
    if (show && activeTab === "sop" && sopProject) {
      void loadCurrentSop(sopProject);
    }
  }, [show, activeTab, sopProject]);

  const filteredGrowths = useMemo(() => {
    return growths.filter((g) => {
      if (selectedProject !== "all") {
        if (selectedProject === "global") {
          if (g.projectId) return false;
        } else if (g.projectId !== selectedProject) {
          return false;
        }
      }
      if (selectedCategory !== "all" && g.category !== selectedCategory) {
        return false;
      }
      if (selectedStatus === "active" && g.status !== "accepted") {
        return false;
      }
      if (selectedStatus === "disabled" && g.status !== "disabled") {
        return false;
      }
      if (selectedStatus === "proposed" && g.status !== "proposed") {
        return false;
      }
      if (searchQuery.trim()) {
        const q = searchQuery.toLowerCase();
        const matchTitle = g.title.toLowerCase().includes(q);
        const matchContent = g.ruleContent.toLowerCase().includes(q);
        const matchThought = g.reflectionThought.toLowerCase().includes(q);
        if (!matchTitle && !matchContent && !matchThought) return false;
      }
      return true;
    });
  }, [growths, selectedProject, selectedCategory, selectedStatus, searchQuery]);

  const stats = useMemo(() => {
    const total = growths.length;
    const accepted = growths.filter((g) => g.status === "accepted").length;
    const proposed = growths.filter((g) => g.status === "proposed").length;
    const disabled = growths.filter((g) => g.status === "disabled").length;
    return { total, accepted, proposed, disabled };
  }, [growths]);

  if (!show) return null;

  // 经验编辑保存
  const startEdit = (g: GrowthItem) => {
    setEditingId(g.id);
    setEditTitle(g.title);
    setEditContent(g.ruleContent);
    setEditCategory(g.category);
  };

  const saveEdit = async (id: string) => {
    await updateGrowthRule(id, editTitle, editContent, editCategory);
    setEditingId(null);
  };

  const toggleThought = (id: string) => {
    setExpandedThoughts((prev) => ({ ...prev, [id]: !prev[id] }));
  };

  const handleJumpToSession = (sessionId: string) => {
    setShow(false);
    void selectSession(sessionId);
  };

  // 技能创建/编辑
  const openNewSkill = () => {
    setEditingSkillName(null);
    setSkillFormName("");
    setSkillFormDesc("");
    setSkillFormType("bat");
    setSkillFormContent("@echo off\necho [Skill] 正在执行任务...\n");
    setShowSkillEditor(true);
  };

  const openEditSkill = (s: SkillItem) => {
    setEditingSkillName(s.name);
    setSkillFormName(s.name);
    setSkillFormDesc(s.description);
    setSkillFormType(s.scriptType);
    setSkillFormContent(s.content);
    setShowSkillEditor(true);
  };

  const handleSaveSkill = async () => {
    const proj = projects.find((p) => p.id === skillsProject);
    if (!proj?.path) {
      pushToast("当前未选择有效工作区项目");
      return;
    }
    if (!skillFormName.trim()) {
      pushToast("请输入技能名称");
      return;
    }
    try {
      await ipc.saveProjectSkill(
        proj.path,
        skillFormName.trim(),
        skillFormDesc.trim(),
        skillFormType,
        skillFormContent
      );
      pushToast(`技能【${skillFormName}】保存成功`);
      setShowSkillEditor(false);
      await loadCurrentSkills(skillsProject);
    } catch (e) {
      pushToast(`保存技能失败: ${e}`);
    }
  };

  const handleDeleteSkill = async (name: string) => {
    const proj = projects.find((p) => p.id === skillsProject);
    if (!proj?.path) return;
    if (await askConfirm(`确定删除技能「${name}」？此操作将删除对应文件。`)) {
      try {
        await ipc.deleteProjectSkill(proj.path, name);
        pushToast(`技能【${name}】已删除`);
        await loadCurrentSkills(skillsProject);
      } catch (e) {
        pushToast(`删除技能失败: ${e}`);
      }
    }
  };

  // SOP 保存与即时测试
  const handleSaveSop = async () => {
    const proj = projects.find((p) => p.id === sopProject);
    if (!proj) return;
    try {
      await ipc.setProjectSop(proj.id, sopVerifyCmd.trim() || null, sopEnabled);
      pushToast("交付 SOP 配置已更新");
      await loadCurrentSop(sopProject);
    } catch (e) {
      pushToast(`保存失败: ${e}`);
    }
  };

  const handleTestRunSop = async () => {
    const proj = projects.find((p) => p.id === sopProject);
    if (!proj?.path) return;
    const cmdToRun = sopVerifyCmd.trim() || sopInfo?.detectedDefaultCmd || "";
    if (!cmdToRun) {
      pushToast("未设置自检命令");
      return;
    }
    setTestRunning(true);
    setTestOutput(null);
    setTestSuccess(null);
    try {
      const out = await ipc.runWorkspaceSop(proj.path, cmdToRun);
      setTestOutput(out);
      setTestSuccess(true);
    } catch (e) {
      setTestOutput(String(e));
      setTestSuccess(false);
    } finally {
      setTestRunning(false);
    }
  };

  const activeProjectForSkills = projects.find((p) => p.id === skillsProject);
  const activeProjectForSop = projects.find((p) => p.id === sopProject);

  return (
    <div
      className="fixed inset-0 z-[80] bg-black/60 flex items-center justify-center p-4"
      onMouseDown={() => setShow(false)}
    >
      <div
        className="bg-panel2 border border-edge rounded-2xl w-[860px] max-h-[88vh] flex flex-col shadow-2xl overflow-hidden"
        onMouseDown={(e) => e.stopPropagation()}
      >
        {/* 顶部标题栏 */}
        <div className="px-6 py-4 border-b border-edge flex items-center justify-between shrink-0 bg-panel">
          <div className="flex items-center gap-3">
            <div className="w-9 h-9 rounded-xl bg-emerald-500/15 border border-emerald-500/30 flex items-center justify-center text-emerald-400 shrink-0">
              <Sprout size={18} />
            </div>
            <div>
              <div className="font-medium text-[16px] text-ink flex items-center gap-2">
                <span>Agent 自我成长中心</span>
                <span className="text-[11px] px-2 py-0.5 rounded-full bg-panel3 text-inkdim border border-edge font-normal">
                  三阶演进体系
                </span>
              </div>
              <div className="text-[12px] text-inkdim mt-0.5">
                经验记忆反思 · 工具能力自扩充 · 行为范式交付自检
              </div>
            </div>
          </div>
          <div className="flex items-center gap-2">
            {currentId && currentId !== "draft" && (
              <button
                className="px-2.5 py-1.5 text-[12px] rounded-lg bg-panel3 hover:bg-edge text-inkdim hover:text-ink flex items-center gap-1.5 transition-colors"
                onClick={() => {
                  triggerManualGrowth(currentId);
                }}
                title="对当前会话进行经验复盘并提炼新规则"
              >
                <Sparkles size={13} className="text-amber-400" />
                <span>复盘当前会话</span>
              </button>
            )}
            <button
              className="w-8 h-8 rounded-lg hover:bg-panel3 flex items-center justify-center text-inkdim hover:text-ink transition-colors"
              onClick={() => setShow(false)}
            >
              <X size={16} />
            </button>
          </div>
        </div>

        {/* 阶梯三选项卡导航 */}
        <div className="px-6 border-b border-edge bg-panel flex items-center gap-4 shrink-0">
          <button
            className={`py-3 px-1 text-[13px] font-medium border-b-2 transition-all flex items-center gap-2 ${
              activeTab === "reflexion"
                ? "border-accent text-accent"
                : "border-transparent text-inkdim hover:text-ink"
            }`}
            onClick={() => setActiveTab("reflexion")}
          >
            <Sprout size={15} />
            <span>经验规则库 (Reflexion)</span>
            <span className="text-[11px] px-1.5 py-0.5 rounded-full bg-panel3 text-inkdim">
              {stats.accepted}
            </span>
          </button>
          <button
            className={`py-3 px-1 text-[13px] font-medium border-b-2 transition-all flex items-center gap-2 ${
              activeTab === "skills"
                ? "border-accent text-accent"
                : "border-transparent text-inkdim hover:text-ink"
            }`}
            onClick={() => setActiveTab("skills")}
          >
            <Zap size={15} />
            <span>技能工具库 (Dynamic Skills)</span>
            <span className="text-[11px] px-1.5 py-0.5 rounded-full bg-panel3 text-inkdim">
              {skills.length}
            </span>
          </button>
          <button
            className={`py-3 px-1 text-[13px] font-medium border-b-2 transition-all flex items-center gap-2 ${
              activeTab === "sop"
                ? "border-accent text-accent"
                : "border-transparent text-inkdim hover:text-ink"
            }`}
            onClick={() => setActiveTab("sop")}
          >
            <ShieldCheck size={15} />
            <span>交付自检 SOP (Pre-flight Check)</span>
            {sopInfo?.sopEnabled && (
              <span className="w-2 h-2 rounded-full bg-emerald-400" title="已开启自检" />
            )}
          </button>
        </div>

        {/* ============================================================ */}
        {/* Tab 1: 经验规则库 (Reflexion)                                 */}
        {/* ============================================================ */}
        {activeTab === "reflexion" && (
          <>
            {/* 筛选与搜索控制栏 */}
            <div className="px-6 py-3 border-b border-edge/80 bg-panel flex flex-col gap-2.5 shrink-0">
              <div className="flex items-center gap-3">
                <div className="flex items-center gap-1.5 shrink-0">
                  <span className="text-[12px] text-inkdim">项目:</span>
                  <select
                    className="bg-panel2 border border-edge rounded-lg px-2.5 py-1 text-[12px] text-ink outline-none"
                    value={selectedProject}
                    onChange={(e) => setSelectedProject(e.target.value)}
                  >
                    <option value="all">全部项目</option>
                    <option value="global">全局通用经验</option>
                    {projects.map((p) => (
                      <option key={p.id} value={p.id}>
                        {p.name}
                      </option>
                    ))}
                  </select>
                </div>

                <div className="flex items-center gap-1 bg-panel2 p-0.5 rounded-lg border border-edge text-[12px]">
                  <button
                    className={`px-2.5 py-0.5 rounded-md transition-all ${
                      selectedStatus === "active"
                        ? "bg-accent text-white font-medium shadow-sm"
                        : "text-inkdim hover:text-ink"
                    }`}
                    onClick={() => setSelectedStatus("active")}
                  >
                    生效中 ({stats.accepted})
                  </button>
                  <button
                    className={`px-2.5 py-0.5 rounded-md transition-all ${
                      selectedStatus === "proposed"
                        ? "bg-amber-500/20 text-amber-300 font-medium"
                        : "text-inkdim hover:text-ink"
                    }`}
                    onClick={() => setSelectedStatus("proposed")}
                  >
                    待审阅 ({stats.proposed})
                  </button>
                  <button
                    className={`px-2.5 py-0.5 rounded-md transition-all ${
                      selectedStatus === "disabled"
                        ? "bg-panel3 text-ink font-medium"
                        : "text-inkdim hover:text-ink"
                    }`}
                    onClick={() => setSelectedStatus("disabled")}
                  >
                    已停用 ({stats.disabled})
                  </button>
                  <button
                    className={`px-2.5 py-0.5 rounded-md transition-all ${
                      selectedStatus === "all"
                        ? "bg-panel3 text-ink font-medium"
                        : "text-inkdim hover:text-ink"
                    }`}
                    onClick={() => setSelectedStatus("all")}
                  >
                    全部 ({stats.total})
                  </button>
                </div>

                <div className="flex-1 min-w-0">
                  <input
                    className="w-full bg-panel2 border border-edge rounded-lg px-3 py-1 text-[12px] text-ink outline-none placeholder:text-inkdim/60 focus:border-accent"
                    placeholder="搜索标题、规则内容或反思过程..."
                    value={searchQuery}
                    onChange={(e) => setSearchQuery(e.target.value)}
                  />
                </div>
              </div>

              <div className="flex items-center gap-1.5 overflow-x-auto text-[12px]">
                <span className="text-inkdim text-[11px] shrink-0">分类:</span>
                <button
                  className={`px-2 py-0.5 rounded transition-all shrink-0 ${
                    selectedCategory === "all"
                      ? "bg-panel3 text-ink font-medium"
                      : "text-inkdim hover:text-ink"
                  }`}
                  onClick={() => setSelectedCategory("all")}
                >
                  全部
                </button>
                {Object.entries(CATEGORY_NAMES).map(([cat, name]) => (
                  <button
                    key={cat}
                    className={`px-2 py-0.5 rounded transition-all shrink-0 ${
                      selectedCategory === cat
                        ? "bg-panel3 text-ink font-medium"
                        : "text-inkdim hover:text-ink"
                    }`}
                    onClick={() => setSelectedCategory(cat)}
                  >
                    {name}
                  </button>
                ))}
              </div>
            </div>

            {/* 规则列表滚动区 */}
            <div className="flex-1 overflow-y-auto px-6 py-4 flex flex-col gap-3">
              {filteredGrowths.map((g) => {
                const isEditing = editingId === g.id;
                const isAccepted = g.status === "accepted";
                const isProposed = g.status === "proposed";
                const isThoughtOpen = !!expandedThoughts[g.id];
                const project = projects.find((p) => p.id === g.projectId);

                return (
                  <div
                    key={g.id}
                    className={`p-4 rounded-xl border transition-all ${
                      isAccepted
                        ? "bg-panel border-edge hover:border-edge/80"
                        : isProposed
                        ? "bg-amber-500/5 border-amber-500/30"
                        : "bg-panel/40 border-edge/40 opacity-70"
                    }`}
                  >
                    <div className="flex items-center justify-between gap-3">
                      <div className="flex items-center gap-2 flex-wrap">
                        <span
                          className={`text-[11px] px-2 py-0.5 rounded-full border font-medium ${
                            CATEGORY_COLORS[g.category] || "bg-panel3 text-inkdim border-edge"
                          }`}
                        >
                          {CATEGORY_NAMES[g.category] || g.category}
                        </span>
                        <span className="font-medium text-[14px] text-ink">{g.title}</span>
                        {project ? (
                          <span className="inline-flex items-center gap-1 text-[11px] px-1.5 py-0.5 rounded bg-panel3 text-inkdim">
                            <Folder size={11} className="opacity-70" />
                            <span>{project.name}</span>
                          </span>
                        ) : (
                          <span className="inline-flex items-center gap-1 text-[11px] px-1.5 py-0.5 rounded bg-panel3 text-inkdim">
                            <Globe size={11} className="opacity-70" />
                            <span>全局通用</span>
                          </span>
                        )}
                        {g.appliedCount > 0 && (
                          <span className="text-[11px] text-emerald-400 bg-emerald-500/10 px-1.5 py-0.5 rounded">
                            已生效 {g.appliedCount} 次
                          </span>
                        )}
                      </div>

                      <div className="flex items-center gap-2 shrink-0">
                        {isProposed ? (
                          <button
                            className="px-2.5 py-1 rounded bg-emerald-600 hover:bg-emerald-500 text-white text-[12px] font-medium flex items-center gap-1 shadow-sm transition-colors"
                            onClick={() => acceptGrowth(g.id)}
                          >
                            <Check size={13} />
                            <span>采纳生效</span>
                          </button>
                        ) : (
                          <label className="flex items-center gap-1.5 cursor-pointer select-none">
                            <span className="text-[11px] text-inkdim">
                              {isAccepted ? "生效中" : "已停用"}
                            </span>
                            <input
                              type="checkbox"
                              checked={isAccepted}
                              onChange={(e) => toggleGrowth(g.id, e.target.checked)}
                              className="accent-accent cursor-pointer"
                            />
                          </label>
                        )}
                      </div>
                    </div>

                    <div className="my-2 bg-panel2/70 border border-edge/60 rounded-lg p-3">
                      {isEditing ? (
                        <div className="flex flex-col gap-2">
                          <div className="flex gap-2">
                            <input
                              className="flex-1 bg-panel border border-edge rounded px-2.5 py-1 text-[13px] text-ink outline-none focus:border-accent"
                              value={editTitle}
                              onChange={(e) => setEditTitle(e.target.value)}
                              placeholder="经验标题"
                            />
                            <select
                              className="bg-panel border border-edge rounded px-2.5 py-1 text-[12px] text-ink outline-none"
                              value={editCategory}
                              onChange={(e) => setEditCategory(e.target.value)}
                            >
                              {Object.entries(CATEGORY_NAMES).map(([k, name]) => (
                                <option key={k} value={k}>
                                  {name}
                                </option>
                              ))}
                            </select>
                          </div>
                          <textarea
                            className="w-full bg-panel border border-edge rounded p-2.5 text-[13px] font-mono text-ink outline-none resize-y min-h-[70px] focus:border-accent"
                            value={editContent}
                            onChange={(e) => setEditContent(e.target.value)}
                            placeholder="规则正文 (Markdown)"
                          />
                          <div className="flex justify-end gap-2 mt-1">
                            <button
                              className="px-2.5 py-1 rounded text-[12px] text-inkdim hover:bg-panel3"
                              onClick={() => setEditingId(null)}
                            >
                              取消
                            </button>
                            <button
                              className="px-3 py-1 rounded text-[12px] bg-accent hover:bg-blue-500 text-white"
                              onClick={() => saveEdit(g.id)}
                            >
                              保存修改
                            </button>
                          </div>
                        </div>
                      ) : (
                        <div className="text-[13px] leading-relaxed text-ink">
                          <Markdown content={g.ruleContent} />
                        </div>
                      )}
                    </div>

                    <div className="text-[12px] flex flex-col gap-1 mt-2 text-inkdim">
                      <div className="flex items-center justify-between">
                        <button
                          className="hover:text-ink flex items-center gap-1.5 text-[11px] transition-colors"
                          onClick={() => toggleThought(g.id)}
                        >
                          {isThoughtOpen ? <ChevronDown size={13} /> : <ChevronRight size={13} />}
                          <Brain size={13} className="text-purple-400" />
                          <span>查看反思推导与触发来源</span>
                        </button>
                        <div className="flex items-center gap-3">
                          {g.sessionId && (
                            <button
                              className="hover:text-accent text-[11px] flex items-center gap-1 underline underline-offset-2 transition-colors"
                              onClick={() => handleJumpToSession(g.sessionId!)}
                              title="跳转到触发该经验沉淀的原始会话"
                            >
                              <Link2 size={12} className="opacity-70" />
                              <span>来源会话:</span>
                              <span className="max-w-[120px] truncate">
                                {g.sessionTitle || "查看对话"}
                              </span>
                            </button>
                          )}
                          {!isEditing && (
                            <>
                              <button
                                className="hover:text-ink text-[11px] flex items-center gap-1 transition-colors"
                                onClick={() => startEdit(g)}
                              >
                                <Edit3 size={11} />
                                <span>编辑</span>
                              </button>
                              <button
                                className="hover:text-red-400 text-[11px] flex items-center gap-1 transition-colors"
                                onClick={async () => {
                                  if (
                                    await askConfirm(
                                      `确定删除规则「${g.title}」？删除后无法恢复。`
                                    )
                                  ) {
                                    await deleteGrowth(g.id);
                                  }
                                }}
                              >
                                <Trash2 size={11} />
                                <span>删除</span>
                              </button>
                            </>
                          )}
                        </div>
                      </div>

                      {isThoughtOpen && (
                        <div className="bg-panel2/80 rounded-lg p-2.5 mt-1 border border-edge text-[11px] leading-relaxed flex flex-col gap-1.5">
                          {g.triggerContext && (
                            <div>
                              <span className="text-ink font-medium">触发背景：</span>
                              <span className="font-mono whitespace-pre-wrap">{g.triggerContext}</span>
                            </div>
                          )}
                          {g.reflectionThought && (
                            <div>
                              <span className="text-ink font-medium">AI 推演反思：</span>
                              <span>{g.reflectionThought}</span>
                            </div>
                          )}
                          <div className="text-inkdim/60 text-[10px]">
                            沉淀时间: {new Date(g.createdAt).toLocaleString("zh-CN")}
                          </div>
                        </div>
                      )}
                    </div>
                  </div>
                );
              })}

              {filteredGrowths.length === 0 && (
                <div className="flex-1 flex flex-col items-center justify-center py-16 text-center text-inkdim">
                  <div className="w-12 h-12 rounded-2xl bg-emerald-500/10 border border-emerald-500/20 flex items-center justify-center text-emerald-400 mb-3">
                    <Sprout size={24} />
                  </div>
                  <div className="text-[14px] font-medium text-ink mb-1">暂无匹配的成长经验</div>
                  <div className="text-[12px] max-w-[360px] leading-relaxed">
                    当 Agent 在日常编码中遭遇审批拒绝纠偏、踩坑自愈或用户教导时，将自动在此沉淀经验规则。
                  </div>
                </div>
              )}
            </div>
          </>
        )}

        {/* ============================================================ */}
        {/* Tab 2: 技能工具库 (Dynamic Skills)                             */}
        {/* ============================================================ */}
        {activeTab === "skills" && (
          <div className="flex-1 overflow-y-auto px-6 py-4 flex flex-col gap-4">
            {/* 项目选择与操作栏 */}
            <div className="flex items-center justify-between bg-panel p-3 rounded-xl border border-edge">
              <div className="flex items-center gap-2">
                <span className="text-[13px] text-inkdim">目标项目:</span>
                <select
                  className="bg-panel2 border border-edge rounded-lg px-3 py-1 text-[13px] text-ink outline-none"
                  value={skillsProject}
                  onChange={(e) => setSkillsProject(e.target.value)}
                >
                  {projects
                    .filter((p) => !!p.path)
                    .map((p) => (
                      <option key={p.id} value={p.id}>
                        {p.name} ({p.path})
                      </option>
                    ))}
                </select>
                {activeProjectForSkills?.path && (
                  <button
                    className="text-[12px] text-accent hover:underline ml-1 flex items-center gap-1 transition-colors"
                    onClick={() => ipc.openDir(`${activeProjectForSkills.path}/.harness/skills`)}
                    title="在系统资源管理器中打开技能目录"
                  >
                    <FolderOpen size={13} />
                    <span>打开 .harness/skills</span>
                  </button>
                )}
              </div>
              <button
                className="px-3.5 py-1.5 rounded-lg bg-accent text-white text-[12px] font-medium hover:bg-blue-500 flex items-center gap-1.5 shadow-sm transition-colors"
                onClick={openNewSkill}
              >
                <Plus size={14} />
                <span>创建新技能</span>
              </button>
            </div>

            {/* 技能说明卡片 */}
            <div className="p-3.5 bg-panel3/50 rounded-xl border border-edge/60 text-[12px] text-inkdim leading-relaxed flex items-start gap-2.5">
              <Lightbulb size={16} className="text-amber-400 shrink-0 mt-0.5" />
              <div>
                <b className="text-ink">技能自扩充机制</b>：Agent
                在执行复杂复合流程或重复性任务时，可将常用操作封装为技能并固化在项目工作区的{" "}
                <code className="bg-panel2 px-1 py-0.5 rounded text-accent">.harness/skills/</code>{" "}
                目录下。技能包含说明文档与脚本，天然随 Git 版本库管理。后续 Agent 可通过{" "}
                <code className="bg-panel2 px-1 py-0.5 rounded">list_skills</code> 查询并使用{" "}
                <code className="bg-panel2 px-1 py-0.5 rounded">run_skill</code> 随时调用。
              </div>
            </div>

            {/* 技能列表 */}
            {skillsLoading ? (
              <div className="py-16 text-center text-inkdim text-[13px]">正在读取技能目录...</div>
            ) : skills.length === 0 ? (
              <div className="py-16 text-center text-inkdim flex flex-col items-center">
                <div className="w-12 h-12 rounded-2xl bg-amber-500/10 border border-amber-500/20 flex items-center justify-center text-amber-400 mb-3">
                  <Zap size={24} />
                </div>
                <div className="font-medium text-ink mb-1">当前项目尚未定义技能</div>
                <div className="text-[12px] max-w-md text-inkdim/80 mb-4">
                  你可以通过右上角「创建新技能」录入脚本，或者让 Agent
                  在对话中自动生成并封装复合工具！
                </div>
                <button
                  className="px-3 py-1.5 rounded-lg bg-accent text-white text-[12px] hover:bg-blue-500 flex items-center gap-1.5 transition-colors shadow-sm"
                  onClick={openNewSkill}
                >
                  <Plus size={13} />
                  <span>录入首个技能</span>
                </button>
              </div>
            ) : (
              <div className="flex flex-col gap-3">
                {skills.map((s) => (
                  <div
                    key={s.name}
                    className="p-4 rounded-xl border border-edge bg-panel flex flex-col gap-2.5 hover:border-edge/80 transition-all"
                  >
                    <div className="flex items-center justify-between">
                      <div className="flex items-center gap-2">
                        <span className="font-semibold text-ink text-[14px]">{s.name}</span>
                        <span className="px-2 py-0.5 rounded text-[11px] font-mono bg-panel3 text-inkdim border border-edge">
                          {s.scriptType}
                        </span>
                        <span className="text-[11px] text-inkdim/70 font-mono">{s.path}</span>
                      </div>
                      <div className="flex items-center gap-2">
                        <button
                          className="px-2.5 py-1 text-[11px] rounded bg-panel2 hover:bg-panel3 text-inkdim hover:text-ink flex items-center gap-1 transition-colors"
                          onClick={() => openEditSkill(s)}
                        >
                          <Edit3 size={11} />
                          <span>编辑</span>
                        </button>
                        <button
                          className="px-2.5 py-1 text-[11px] rounded bg-panel2 hover:bg-red-500/20 text-red-400 hover:text-red-300 flex items-center gap-1 transition-colors"
                          onClick={() => handleDeleteSkill(s.name)}
                        >
                          <Trash2 size={11} />
                          <span>删除</span>
                        </button>
                      </div>
                    </div>
                    <p className="text-[13px] text-ink leading-relaxed">{s.description}</p>
                    {s.content && (
                      <div className="bg-panel2 rounded-lg p-2.5 border border-edge font-mono text-[12px] text-inkdim overflow-x-auto max-h-36">
                        <pre>{s.content}</pre>
                      </div>
                    )}
                  </div>
                ))}
              </div>
            )}
          </div>
        )}

        {/* ============================================================ */}
        {/* Tab 3: 交付自检 SOP (Pre-flight Check)                          */}
        {/* ============================================================ */}
        {activeTab === "sop" && (
          <div className="flex-1 overflow-y-auto px-6 py-4 flex flex-col gap-4">
            {/* 项目选择栏 */}
            <div className="flex items-center justify-between bg-panel p-3 rounded-xl border border-edge">
              <div className="flex items-center gap-2">
                <span className="text-[13px] text-inkdim">目标项目:</span>
                <select
                  className="bg-panel2 border border-edge rounded-lg px-3 py-1 text-[13px] text-ink outline-none"
                  value={sopProject}
                  onChange={(e) => setSopProject(e.target.value)}
                >
                  {projects
                    .filter((p) => !!p.path)
                    .map((p) => (
                      <option key={p.id} value={p.id}>
                        {p.name} ({p.path})
                      </option>
                    ))}
                </select>
              </div>
              <div className="text-[12px] text-inkdim">
                技术栈自适应探测 · 交付前守卫验证 · 自愈闭环
              </div>
            </div>

            {sopLoading ? (
              <div className="py-16 text-center text-inkdim text-[13px]">正在探测项目技术栈与 SOP 配置...</div>
            ) : !activeProjectForSop?.path ? (
              <div className="py-16 text-center text-inkdim text-[13px]">请先选择已绑定本地目录的项目。</div>
            ) : (
              <div className="flex flex-col gap-4">
                {/* 状态与开关卡片 */}
                <div className="p-4 rounded-xl border border-edge bg-panel flex flex-col gap-3">
                  <div className="flex items-center justify-between">
                    <div>
                      <div className="text-[14px] font-medium text-ink flex items-center gap-2">
                        <span>代码修改后强制交付自检</span>
                        {sopEnabled ? (
                          <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-[11px] bg-emerald-500/15 text-emerald-400 border border-emerald-500/30">
                            <Check size={12} />
                            <span>已开启守卫</span>
                          </span>
                        ) : (
                          <span className="px-2 py-0.5 rounded-full text-[11px] bg-panel3 text-inkdim border border-edge">
                            已停用
                          </span>
                        )}
                      </div>
                      <p className="text-[12px] text-inkdim mt-1 max-w-xl leading-relaxed">
                        当 Agent 在会话中修改了工程代码后，向你交付回复前将自动执行构建/测试验证；若编译失败或报错，将自动拦截并进入自愈重试，直到代码通过验证。
                      </p>
                    </div>
                    <label className="relative inline-flex items-center cursor-pointer">
                      <input
                        type="checkbox"
                        className="sr-only peer"
                        checked={sopEnabled}
                        onChange={(e) => setSopEnabled(e.target.checked)}
                      />
                      <div className="w-11 h-6 bg-panel3 peer-focus:outline-none rounded-full peer peer-checked:after:translate-x-full peer-checked:after:border-white after:content-[''] after:absolute after:top-[2px] after:left-[2px] after:bg-white after:border-gray-300 after:border after:rounded-full after:h-5 after:w-5 after:transition-all peer-checked:bg-accent"></div>
                    </label>
                  </div>

                  {/* 自动探测信息栏 */}
                  <div className="flex items-center gap-4 pt-3 border-t border-edge text-[12px]">
                    <div className="flex items-center gap-1.5 text-inkdim">
                      <Target size={13} className="text-blue-400" />
                      <span>自动识别技术栈:</span>
                      <span className="text-ink font-medium">
                        {sopInfo?.detectedStack || "未知技术栈"}
                      </span>
                    </div>
                    {sopInfo?.detectedDefaultCmd && (
                      <div className="flex items-center gap-1.5 text-inkdim">
                        <Lightbulb size={13} className="text-amber-400" />
                        <span>推荐默认自检命令:</span>
                        <code className="bg-panel2 px-1.5 py-0.5 rounded text-accent font-mono">
                          {sopInfo.detectedDefaultCmd}
                        </code>
                      </div>
                    )}
                  </div>
                </div>

                {/* 命令配置表单 */}
                <div className="p-4 rounded-xl border border-edge bg-panel flex flex-col gap-3">
                  <div className="flex items-center justify-between">
                    <label className="text-[13px] font-medium text-ink">
                      自检执行命令 (Shell Command)
                    </label>
                    {sopInfo?.detectedDefaultCmd && (
                      <button
                        className="text-[12px] text-accent hover:underline"
                        onClick={() => setSopVerifyCmd(sopInfo.detectedDefaultCmd)}
                      >
                        使用推荐命令 ({sopInfo.detectedDefaultCmd})
                      </button>
                    )}
                  </div>
                  <div className="flex items-center gap-2">
                    <input
                      className="flex-1 bg-panel2 border border-edge rounded-lg px-3 py-1.5 text-[13px] text-ink font-mono outline-none focus:border-accent"
                      placeholder={sopInfo?.detectedDefaultCmd || "例如: cargo check 或 npm test"}
                      value={sopVerifyCmd}
                      onChange={(e) => setSopVerifyCmd(e.target.value)}
                    />
                    <button
                      className="px-4 py-1.5 rounded-lg bg-accent text-white text-[12px] font-medium hover:bg-blue-500 shrink-0 shadow-sm transition-colors"
                      onClick={handleSaveSop}
                    >
                      保存配置
                    </button>
                  </div>
                  <p className="text-[11px] text-inkdim leading-relaxed">
                    可配置编译检查命令（如 <code>cargo check</code>）、测试命令（如 <code>npm test</code>、<code>pytest</code>）或代码格式与静态检查脚本。
                  </p>
                </div>

                {/* 即时测试卡片 */}
                <div className="p-4 rounded-xl border border-edge bg-panel flex flex-col gap-3">
                  <div className="flex items-center justify-between">
                    <div>
                      <div className="text-[13px] font-medium text-ink">即时验证执行</div>
                      <div className="text-[11px] text-inkdim mt-0.5">
                        在当前工作区实际运行该自检命令，确认环境配置与预期输出
                      </div>
                    </div>
                    <button
                      className="px-3.5 py-1.5 rounded-lg bg-panel3 hover:bg-edge text-ink text-[12px] flex items-center gap-1.5 disabled:opacity-50 shadow-sm transition-colors"
                      disabled={testRunning || (!sopVerifyCmd.trim() && !sopInfo?.detectedDefaultCmd)}
                      onClick={handleTestRunSop}
                    >
                      {testRunning ? (
                        <>
                          <span className="w-2 h-2 rounded-full bg-accent animate-ping" />
                          <span>正在执行自检...</span>
                        </>
                      ) : (
                        <>
                          <Play size={12} fill="currentColor" className="text-emerald-400" />
                          <span>立即测试自检命令</span>
                        </>
                      )}
                    </button>
                  </div>

                  {testOutput !== null && (
                    <div
                      className={`p-3 rounded-lg border text-[12px] font-mono whitespace-pre-wrap max-h-56 overflow-y-auto ${
                        testSuccess
                          ? "bg-emerald-500/10 border-emerald-500/30 text-emerald-300"
                          : "bg-red-500/10 border-red-500/30 text-red-300"
                      }`}
                    >
                      {testOutput}
                    </div>
                  )}
                </div>
              </div>
            )}
          </div>
        )}

        {/* 底部关闭按钮 */}
        <div className="px-6 py-3 border-t border-edge flex justify-between items-center bg-panel shrink-0">
          <div className="text-[11px] text-inkdim">
            所有成长沉淀均本地持久化存储，透明可见且随时可停用。
          </div>
          <button
            className="px-4 py-1.5 rounded-lg text-inkdim hover:bg-panel3 text-[13px] transition-colors"
            onClick={() => setShow(false)}
          >
            关闭
          </button>
        </div>
      </div>

      {/* 技能编辑/创建浮层 */}
      {showSkillEditor && (
        <div
          className="fixed inset-0 z-[90] bg-black/70 flex items-center justify-center p-4"
          onMouseDown={() => setShowSkillEditor(false)}
        >
          <div
            className="bg-panel2 border border-edge rounded-2xl w-[600px] max-h-[85vh] flex flex-col shadow-2xl p-5 gap-3.5"
            onMouseDown={(e) => e.stopPropagation()}
          >
            <div className="flex items-center justify-between border-b border-edge pb-3">
              <span className="font-semibold text-ink text-[15px]">
                {editingSkillName ? `编辑技能: ${editingSkillName}` : "创建新项目技能"}
              </span>
              <button
                className="w-7 h-7 rounded-lg hover:bg-panel3 flex items-center justify-center text-inkdim hover:text-ink transition-colors"
                onClick={() => setShowSkillEditor(false)}
              >
                <X size={16} />
              </button>
            </div>

            <div className="flex flex-col gap-1">
              <label className="text-[12px] text-inkdim">技能标识符 (英文、连字符或下划线):</label>
              <input
                className="bg-panel border border-edge rounded-lg px-3 py-1.5 text-[13px] text-ink outline-none focus:border-accent font-mono disabled:opacity-50"
                placeholder="例如: build-check, export-data"
                value={skillFormName}
                disabled={!!editingSkillName}
                onChange={(e) => setSkillFormName(e.target.value)}
              />
            </div>

            <div className="flex flex-col gap-1">
              <label className="text-[12px] text-inkdim">技能说明 (说明用途与触发时机):</label>
              <input
                className="bg-panel border border-edge rounded-lg px-3 py-1.5 text-[13px] text-ink outline-none focus:border-accent"
                placeholder="例如: 快速执行工程代码静态分析与打包测试"
                value={skillFormDesc}
                onChange={(e) => setSkillFormDesc(e.target.value)}
              />
            </div>

            <div className="flex flex-col gap-1">
              <label className="text-[12px] text-inkdim">脚本类型:</label>
              <select
                className="bg-panel border border-edge rounded-lg px-3 py-1.5 text-[13px] text-ink outline-none"
                value={skillFormType}
                onChange={(e) => setSkillFormType(e.target.value)}
              >
                <option value="bat">Windows 批处理 (.bat)</option>
                <option value="ps1">PowerShell (.ps1)</option>
                <option value="sh">Shell 脚本 (.sh)</option>
                <option value="py">Python 脚本 (.py)</option>
                <option value="js">Node.js 脚本 (.js)</option>
              </select>
            </div>

            <div className="flex flex-col gap-1">
              <label className="text-[12px] text-inkdim">脚本源码:</label>
              <textarea
                className="bg-panel border border-edge rounded-lg p-3 text-[12px] font-mono text-ink outline-none focus:border-accent min-h-[140px] resize-y"
                value={skillFormContent}
                onChange={(e) => setSkillFormContent(e.target.value)}
                placeholder="编写执行脚本..."
              />
            </div>

            <div className="flex justify-end gap-2 pt-2 border-t border-edge">
              <button
                className="px-3.5 py-1.5 rounded-lg text-inkdim hover:bg-panel3 text-[12px]"
                onClick={() => setShowSkillEditor(false)}
              >
                取消
              </button>
              <button
                className="px-4 py-1.5 rounded-lg bg-accent text-white text-[12px] font-medium hover:bg-blue-500"
                onClick={handleSaveSkill}
              >
                保存技能
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
