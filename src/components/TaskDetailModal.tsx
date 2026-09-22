import { useEffect, useState } from "react";
import { useStore } from "../store";
import { DRAFT_ID, TaskSubItem } from "../types";
import {
  Target,
  X,
  CheckCircle2,
  Loader2,
  Circle,
  AlertCircle,
  Clock,
  Sparkles,
  GitBranch,
  RotateCcw,
  Pencil,
  Trash2,
  Plus,
  Check,
  Terminal,
  ChevronDown,
  ChevronRight,
} from "./Icons";
import { Markdown } from "./Markdown";

export function TaskDetailModal() {
  const show = useStore((s) => s.showTaskDetailModal);
  const setShow = useStore((s) => s.setShowTaskDetailModal);
  const currentId = useStore((s) => s.currentId);
  const activeTasks = useStore((s) => s.activeTasks);
  const task = currentId && currentId !== DRAFT_ID ? activeTasks[currentId] ?? null : null;
  const taskCheckpoints = useStore((s) => s.taskCheckpoints);
  const checkpoints = task ? taskCheckpoints[task.id] ?? [] : [];
  const fetchTaskCheckpoints = useStore((s) => s.fetchTaskCheckpoints);
  const rollbackToCheckpoint = useStore((s) => s.rollbackToCheckpoint);
  const updateTaskSubtasks = useStore((s) => s.updateTaskSubtasks);

  // 本地交互编辑状态
  const [editingSubtaskId, setEditingSubtaskId] = useState<string | null>(null);
  const [editTitle, setEditTitle] = useState("");
  const [editDesc, setEditDesc] = useState("");
  const [editVerifyCmd, setEditVerifyCmd] = useState("");

  const [showAddSubtask, setShowAddSubtask] = useState(false);
  const [newTitle, setNewTitle] = useState("");
  const [newDesc, setNewDesc] = useState("");
  const [newVerifyCmd, setNewVerifyCmd] = useState("");

  const [expandedVerifyLogs, setExpandedVerifyLogs] = useState<Record<string, boolean>>({});
  const [isRollingBack, setIsRollingBack] = useState(false);

  useEffect(() => {
    if (show && task) {
      void fetchTaskCheckpoints(task.id);
    }
  }, [show, task?.id]);

  if (!show || !task) {
    return null;
  }

  const subtasks = task.subtasks || [];
  const completedCount = subtasks.filter((s) => s.status === "completed").length;
  const totalCount = subtasks.length;
  const pct = totalCount > 0 ? Math.round((completedCount / totalCount) * 100) : 0;

  const handleRollback = async (checkpointId: string, stepNumber: number) => {
    const ok = window.confirm(
      `确定要将项目代码和任务进度回退至 Step #${stepNumber} 快照吗？\n该快照之后的代码修改与执行进度将被重置。`
    );
    if (!ok) return;
    setIsRollingBack(true);
    await rollbackToCheckpoint(checkpointId);
    setIsRollingBack(false);
  };

  const startEditSubtask = (sub: TaskSubItem) => {
    setEditingSubtaskId(sub.id);
    setEditTitle(sub.title);
    setEditDesc(sub.description || "");
    setEditVerifyCmd(sub.verifyCommand || "");
  };

  const saveEditSubtask = async (subtaskId: string) => {
    if (!task) return;
    const updated = task.subtasks.map((s) => {
      if (s.id === subtaskId) {
        return {
          ...s,
          title: editTitle.trim() || s.title,
          description: editDesc.trim() || null,
          verifyCommand: editVerifyCmd.trim() || null,
        };
      }
      return s;
    });
    await updateTaskSubtasks(task.id, updated);
    setEditingSubtaskId(null);
  };

  const handleDeleteSubtask = async (subtaskId: string) => {
    if (!task) return;
    const ok = window.confirm("确定删除该阶段子任务吗？");
    if (!ok) return;
    const updated = task.subtasks
      .filter((s) => s.id !== subtaskId)
      .map((s, i) => ({ ...s, index: i + 1 }));
    await updateTaskSubtasks(task.id, updated);
  };

  const handleAddSubtask = async () => {
    if (!task || !newTitle.trim()) return;
    const newItem: TaskSubItem = {
      id: `subtask-${Date.now()}`,
      index: task.subtasks.length + 1,
      title: newTitle.trim(),
      description: newDesc.trim() || null,
      verifyCommand: newVerifyCmd.trim() || null,
      status: "pending",
      summary: null,
      error: null,
      verifyOutput: null,
    };
    const updated = [...task.subtasks, newItem];
    await updateTaskSubtasks(task.id, updated);
    setNewTitle("");
    setNewDesc("");
    setNewVerifyCmd("");
    setShowAddSubtask(false);
  };

  const toggleVerifyLog = (subtaskId: string) => {
    setExpandedVerifyLogs((prev) => ({
      ...prev,
      [subtaskId]: !prev[subtaskId],
    }));
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 backdrop-blur-sm select-none p-4">
      <div className="bg-panel2 border border-edge rounded-2xl shadow-2xl w-full max-w-2xl max-h-[88vh] flex flex-col overflow-hidden animate-in fade-in zoom-in-95 duration-200">
        {/* Header */}
        <div className="px-5 py-4 border-b border-edge flex items-center justify-between shrink-0">
          <div className="flex items-center gap-2.5">
            <div className="p-1.5 rounded-lg bg-purple-500/10 text-purple-400 border border-purple-500/20">
              <Target size={16} />
            </div>
            <div>
              <h2 className="font-semibold text-ink text-[14px]">长任务路线图与检查点</h2>
              <p className="text-[11px] text-inkdim">查看与调整长任务目标分解、推进进度及阶段快照</p>
            </div>
          </div>
          <button
            onClick={() => setShow(false)}
            className="p-1.5 rounded-lg hover:bg-panel text-inkdim hover:text-ink transition-colors cursor-pointer"
          >
            <X size={16} />
          </button>
        </div>

        {/* Content Body */}
        <div className="flex-1 overflow-y-auto p-5 space-y-5">
          {/* 总目标卡片 */}
          <div className="p-3.5 rounded-xl bg-panel border border-edge flex flex-col gap-2">
            <div className="flex items-center justify-between">
              <span className="text-[11px] font-medium text-inkdim uppercase tracking-wider">总任务目标</span>
              <span className="text-[11px] font-mono text-inkdim">
                进度: {completedCount}/{totalCount} ({pct}%)
              </span>
            </div>
            <div className="text-[13px] font-medium text-ink leading-relaxed">
              {task.goal}
            </div>
            <div className="h-1.5 bg-panel3 rounded-full overflow-hidden mt-1">
              <div
                className="h-full bg-gradient-to-r from-purple-500 to-indigo-400 rounded-full transition-all duration-300"
                style={{ width: `${pct}%` }}
              />
            </div>
          </div>

          {/* 子任务阶段树 */}
          <div className="space-y-2.5">
            <div className="flex items-center justify-between">
              <h3 className="text-[12px] font-semibold text-ink flex items-center gap-1.5">
                <span>阶段子任务分解</span>
                <span className="text-[11px] text-inkdim font-normal">({subtasks.length} 个阶段)</span>
              </h3>

              {!showAddSubtask && (
                <button
                  onClick={() => setShowAddSubtask(true)}
                  className="flex items-center gap-1 px-2 py-0.5 rounded text-[11px] text-purple-400 hover:text-purple-300 hover:bg-purple-500/10 border border-purple-500/20 transition-colors cursor-pointer"
                >
                  <Plus size={12} />
                  <span>添加子任务</span>
                </button>
              )}
            </div>

            {subtasks.length === 0 ? (
              <div className="p-4 rounded-xl bg-panel border border-edge text-center text-inkdim text-[12px] flex items-center justify-center gap-2">
                <Sparkles size={14} className="text-amber-400 animate-spin" />
                正在调用大模型拆解任务清单中…
              </div>
            ) : (
              <div className="space-y-2">
                {subtasks.map((sub, idx) => {
                  const isCurrent = idx === task.currentSubtaskIndex && task.status === "running";
                  const isEditing = editingSubtaskId === sub.id;
                  const canEdit = sub.status === "pending" || task.status === "paused" || task.status === "failed";
                  const hasLogs = !!sub.verifyOutput;
                  const logsOpen = !!expandedVerifyLogs[sub.id];

                  return (
                    <div
                      key={sub.id}
                      className={`p-3.5 rounded-xl border transition-all ${
                        isCurrent
                          ? "bg-purple-500/5 border-purple-500/40 ring-1 ring-purple-500/20"
                          : sub.status === "completed"
                          ? "bg-panel border-edge/80"
                          : "bg-panel/50 border-edge/50"
                      }`}
                    >
                      <div className="flex items-start gap-2.5">
                        <div className="mt-0.5 shrink-0">
                          {sub.status === "completed" ? (
                            <CheckCircle2 size={16} className="text-emerald-400" />
                          ) : isCurrent ? (
                            <Loader2 size={16} className="text-purple-400 animate-spin" />
                          ) : sub.status === "failed" ? (
                            <AlertCircle size={16} className="text-red-400" />
                          ) : (
                            <Circle size={16} className="text-inkdim/50" />
                          )}
                        </div>

                        <div className="flex-1 min-w-0 space-y-1.5">
                          {isEditing ? (
                            /* 内联编辑表单 */
                            <div className="space-y-2 bg-panel2 p-3 rounded-lg border border-purple-500/30">
                              <input
                                type="text"
                                value={editTitle}
                                onChange={(e) => setEditTitle(e.target.value)}
                                placeholder="阶段标题"
                                className="w-full px-2.5 py-1 text-[12px] rounded bg-panel border border-edge text-ink outline-none focus:border-purple-500"
                              />
                              <textarea
                                value={editDesc}
                                onChange={(e) => setEditDesc(e.target.value)}
                                placeholder="阶段详细要求或描述（选填）"
                                rows={2}
                                className="w-full px-2.5 py-1 text-[11px] rounded bg-panel border border-edge text-ink outline-none focus:border-purple-500 resize-none"
                              />
                              <div className="flex items-center gap-1.5">
                                <Terminal size={12} className="text-inkdim shrink-0" />
                                <input
                                  type="text"
                                  value={editVerifyCmd}
                                  onChange={(e) => setEditVerifyCmd(e.target.value)}
                                  placeholder="自动化质量门禁验证命令，例如 cargo check 或 npm test（选填）"
                                  className="flex-1 px-2.5 py-1 text-[11px] font-mono rounded bg-panel border border-edge text-ink outline-none focus:border-purple-500"
                                />
                              </div>
                              <div className="flex items-center justify-end gap-2 pt-1">
                                <button
                                  onClick={() => setEditingSubtaskId(null)}
                                  className="px-2.5 py-0.5 rounded text-[11px] text-inkdim hover:text-ink cursor-pointer"
                                >
                                  取消
                                </button>
                                <button
                                  onClick={() => saveEditSubtask(sub.id)}
                                  className="flex items-center gap-1 px-2.5 py-0.5 rounded bg-purple-600 hover:bg-purple-500 text-white text-[11px] font-medium transition-colors cursor-pointer"
                                >
                                  <Check size={12} />
                                  <span>保存</span>
                                </button>
                              </div>
                            </div>
                          ) : (
                            /* 常规展示 */
                            <>
                              <div className="flex items-center justify-between gap-2">
                                <span className="text-[13px] font-medium text-ink truncate">
                                  {sub.index}. {sub.title}
                                </span>
                                <div className="flex items-center gap-1.5 shrink-0">
                                  <span
                                    className={`px-1.5 py-0.5 rounded text-[10px] font-medium ${
                                      sub.status === "completed"
                                        ? "bg-emerald-500/15 text-emerald-400 border border-emerald-500/30"
                                        : isCurrent
                                        ? "bg-purple-500/15 text-purple-400 border border-purple-500/30"
                                        : sub.status === "failed"
                                        ? "bg-red-500/15 text-red-400 border border-red-500/30"
                                        : "bg-panel2 text-inkdim border border-edge"
                                    }`}
                                  >
                                    {sub.status === "completed"
                                      ? "已完成"
                                      : isCurrent
                                      ? "进行中"
                                      : sub.status === "failed"
                                      ? "异常"
                                      : "等待中"}
                                  </span>

                                  {canEdit && (
                                    <div className="flex items-center gap-1 ml-1">
                                      <button
                                        onClick={() => startEditSubtask(sub)}
                                        title="编辑阶段内容"
                                        className="p-1 rounded hover:bg-panel2 text-inkdim hover:text-ink transition-colors cursor-pointer"
                                      >
                                        <Pencil size={12} />
                                      </button>
                                      <button
                                        onClick={() => handleDeleteSubtask(sub.id)}
                                        title="删除阶段"
                                        className="p-1 rounded hover:bg-red-500/20 text-inkdim hover:text-red-400 transition-colors cursor-pointer"
                                      >
                                        <Trash2 size={12} />
                                      </button>
                                    </div>
                                  )}
                                </div>
                              </div>

                              {sub.description && (
                                <p className="text-[11px] text-inkdim leading-relaxed">
                                  {sub.description}
                                </p>
                              )}

                              {/* 自动化质量门禁标记与日志 */}
                              {sub.verifyCommand && (
                                <div className="pt-1">
                                  <div className="inline-flex items-center gap-1.5 px-2 py-0.5 rounded bg-panel2 border border-edge/80 text-[10px] font-mono text-inkdim">
                                    <Terminal size={11} className="text-purple-400" />
                                    <span>门禁: {sub.verifyCommand}</span>
                                    {hasLogs && (
                                      <button
                                        onClick={() => toggleVerifyLog(sub.id)}
                                        className="ml-1 text-purple-400 hover:text-purple-300 flex items-center cursor-pointer"
                                      >
                                        {logsOpen ? <ChevronDown size={11} /> : <ChevronRight size={11} />}
                                        <span className="ml-0.5">日志</span>
                                      </button>
                                    )}
                                  </div>

                                  {hasLogs && logsOpen && (
                                    <div className="mt-1.5 p-2 rounded bg-black/40 border border-edge font-mono text-[10px] text-inkdim max-h-36 overflow-y-auto whitespace-pre-wrap">
                                      {sub.verifyOutput}
                                    </div>
                                  )}
                                </div>
                              )}

                              {sub.summary && (
                                <div className="mt-2 pt-2 border-t border-edge/60 text-[11px] text-ink/80 bg-panel2/40 p-2 rounded-lg">
                                  <span className="font-medium text-inkdim block mb-0.5">验收结论：</span>
                                  <div className="line-clamp-3">
                                    <Markdown content={sub.summary} />
                                  </div>
                                </div>
                              )}
                            </>
                          )}
                        </div>
                      </div>
                    </div>
                  );
                })}

                {/* 添加新子任务面板 */}
                {showAddSubtask && (
                  <div className="p-3.5 rounded-xl border border-dashed border-purple-500/40 bg-purple-500/5 space-y-2">
                    <div className="text-[11px] font-semibold text-purple-400">添加自定义推进阶段</div>
                    <input
                      type="text"
                      value={newTitle}
                      onChange={(e) => setNewTitle(e.target.value)}
                      placeholder="阶段标题（例如：编写前端组件并联调）"
                      className="w-full px-2.5 py-1 text-[12px] rounded bg-panel border border-edge text-ink outline-none focus:border-purple-500"
                    />
                    <textarea
                      value={newDesc}
                      onChange={(e) => setNewDesc(e.target.value)}
                      placeholder="阶段详细目标与要求（选填）"
                      rows={2}
                      className="w-full px-2.5 py-1 text-[11px] rounded bg-panel border border-edge text-ink outline-none focus:border-purple-500 resize-none"
                    />
                    <div className="flex items-center gap-1.5">
                      <Terminal size={12} className="text-inkdim shrink-0" />
                      <input
                        type="text"
                        value={newVerifyCmd}
                        onChange={(e) => setNewVerifyCmd(e.target.value)}
                        placeholder="自动化质量门禁验证命令，例如 cargo test（选填）"
                        className="flex-1 px-2.5 py-1 text-[11px] font-mono rounded bg-panel border border-edge text-ink outline-none focus:border-purple-500"
                      />
                    </div>
                    <div className="flex items-center justify-end gap-2 pt-1">
                      <button
                        onClick={() => setShowAddSubtask(false)}
                        className="px-2.5 py-1 rounded text-[11px] text-inkdim hover:text-ink cursor-pointer"
                      >
                        取消
                      </button>
                      <button
                        onClick={handleAddSubtask}
                        disabled={!newTitle.trim()}
                        className="flex items-center gap-1 px-3 py-1 rounded bg-purple-600 hover:bg-purple-500 disabled:opacity-50 text-white text-[11px] font-medium transition-colors cursor-pointer"
                      >
                        <Plus size={12} />
                        <span>确认追加</span>
                      </button>
                    </div>
                  </div>
                )}
              </div>
            )}
          </div>

          {/* 阶段检查点快照与时光机 (Checkpoints & Rollback) */}
          {checkpoints.length > 0 && (
            <div className="space-y-2.5 pt-2">
              <h3 className="text-[12px] font-semibold text-ink flex items-center gap-1.5">
                <GitBranch size={13} className="text-purple-400" />
                <span>持久化快照与时光机回退 (Checkpoints)</span>
                <span className="text-[11px] text-inkdim font-normal">({checkpoints.length} 条)</span>
              </h3>

              <div className="space-y-1.5 max-h-[180px] overflow-y-auto pr-1">
                {checkpoints.map((cp) => (
                  <div
                    key={cp.id}
                    className="p-2.5 rounded-lg bg-panel border border-edge text-[11px] flex items-center justify-between gap-3"
                  >
                    <div className="flex items-center gap-2 truncate min-w-0">
                      <span className="font-mono text-purple-400 font-semibold shrink-0">
                        Step #{cp.stepNumber}
                      </span>
                      <span className="text-ink truncate font-medium">
                        {cp.summary}
                      </span>
                      {cp.gitCommitHash && (
                        <span className="px-1.5 py-0.2 rounded bg-purple-500/10 text-purple-400 border border-purple-500/20 font-mono text-[9px] shrink-0">
                          git:{cp.gitCommitHash.slice(0, 7)}
                        </span>
                      )}
                    </div>
                    <div className="flex items-center gap-2 shrink-0">
                      <span className="text-inkdim text-[10px] font-mono">
                        {cp.createdAt ? new Date(cp.createdAt).toLocaleTimeString() : ""}
                      </span>
                      <button
                        onClick={() => handleRollback(cp.id, cp.stepNumber)}
                        disabled={isRollingBack}
                        title="回退工作区代码与任务进度至该检查点"
                        className="flex items-center gap-1 px-2 py-0.5 rounded bg-panel2 hover:bg-amber-500/10 text-inkdim hover:text-amber-400 border border-edge hover:border-amber-500/30 transition-colors cursor-pointer text-[10px]"
                      >
                        <RotateCcw size={10} />
                        <span>回退</span>
                      </button>
                    </div>
                  </div>
                ))}
              </div>
            </div>
          )}
        </div>

        {/* Footer */}
        <div className="px-5 py-3 border-t border-edge bg-panel flex items-center justify-between shrink-0 text-[11px] text-inkdim">
          <span>总消耗 Tokens: {task.totalTokensUsed}</span>
          <button
            onClick={() => setShow(false)}
            className="px-3.5 py-1 rounded-lg bg-panel2 hover:bg-panel3 text-ink border border-edge transition-colors cursor-pointer"
          >
            关闭
          </button>
        </div>
      </div>
    </div>
  );
}
