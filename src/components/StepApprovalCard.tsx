import React, { useState } from "react";
import { Clock, Plus, Play, XCircle, CheckCircle2, Loader2 } from "lucide-react";
import { ApprovalReq } from "../types";
import { ipc } from "../ipc";
import { useStore } from "../store";

interface StepApprovalCardProps {
  request: ApprovalReq;
}

export const StepApprovalCard: React.FC<StepApprovalCardProps> = ({ request }) => {
  const approvalDone = useStore((s) => s.approvalDone);
  const pushToast = useStore((s) => s.pushToast);

  const currentSteps = request.stepData?.currentSteps ?? request.params?.current_steps ?? 0;
  const maxSteps = request.stepData?.maxSteps ?? request.params?.max_steps ?? 0;
  const sessionRole = request.stepData?.sessionRole ?? request.params?.session_role;

  const [extraSteps, setExtraSteps] = useState<number>(20);
  const [submitting, setSubmitting] = useState<boolean>(false);

  const handleApprove = async (stepsToAdd?: number) => {
    const finalSteps = stepsToAdd ?? extraSteps;
    if (finalSteps <= 0) {
      pushToast("追加步数必须大于 0", "warning");
      return;
    }
    setSubmitting(true);
    try {
      await ipc.respondApproval(request.eventId, "allow_step_extension", String(finalSteps));
      approvalDone(request.eventId);
      pushToast(`已批准追加 ${finalSteps} 步，正在原地热续跑...`, "success");
    } catch (err: any) {
      pushToast(`审批提交失败: ${err?.message || err}`, "error");
    } finally {
      setSubmitting(false);
    }
  };

  const handleDeny = async () => {
    setSubmitting(true);
    try {
      await ipc.respondApproval(request.eventId, "deny", "用户拒绝追加步数");
      approvalDone(request.eventId);
      pushToast("已拒绝追加步数，任务以当前进度结案", "info");
    } catch (err: any) {
      pushToast(`操作失败: ${err?.message || err}`, "error");
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div className="rounded-xl border border-amber-500/40 bg-gradient-to-r from-amber-500/15 via-panel2/90 to-amber-500/10 p-4 shadow-lg backdrop-blur-sm animate-in fade-in slide-in-from-bottom-2 duration-200">
      <div className="flex flex-col md:flex-row items-start md:items-center justify-between gap-3">
        {/* 左侧说明区 */}
        <div className="flex items-start gap-3">
          <div className="p-2 rounded-lg bg-amber-500/20 text-amber-300 shrink-0 mt-0.5 md:mt-0">
            <Clock size={20} className="animate-pulse" />
          </div>
          <div>
            <div className="flex items-center gap-2 flex-wrap">
              <span className="font-semibold text-[13px] text-amber-200">
                ⏱️ 步数上限审核申请
              </span>
              {sessionRole && (
                <span className="px-1.5 py-0.5 rounded text-[11px] font-medium bg-amber-500/20 text-amber-300 border border-amber-500/30">
                  {sessionRole}
                </span>
              )}
              <span className="text-[12px] text-inkdim">
                已达上限：
                <span className="font-mono text-amber-300 font-medium">
                  {currentSteps} / {maxSteps}
                </span>{" "}
                步
              </span>
            </div>
            <p className="text-[12px] text-inkdim mt-1 leading-relaxed">
              任务已达到当前配置的步数上限，协程已在原地安全挂起。您可以追加步数原地热续跑，或以当前进展结案收尾。
            </p>
          </div>
        </div>

        {/* 右侧交互与决策区 */}
        <div className="flex items-center gap-2.5 shrink-0 self-end md:self-auto flex-wrap">
          {/* 快捷增量按钮 */}
          <div className="flex items-center gap-1.5 bg-panel1/60 p-1 rounded-lg border border-border/50">
            <button
              disabled={submitting}
              onClick={() => handleApprove(20)}
              className="px-2.5 py-1 text-[12px] rounded-md font-medium text-ink hover:text-white bg-panel2/80 hover:bg-emerald-600/30 hover:border-emerald-500/40 border border-transparent transition-all"
              title="一键追加 20 步并立即继续"
            >
              +20 步
            </button>
            <button
              disabled={submitting}
              onClick={() => handleApprove(50)}
              className="px-2.5 py-1 text-[12px] rounded-md font-medium text-ink hover:text-white bg-panel2/80 hover:bg-emerald-600/30 hover:border-emerald-500/40 border border-transparent transition-all"
              title="一键追加 50 步并立即继续"
            >
              +50 步
            </button>
            <div className="flex items-center gap-1 pl-1 border-l border-border/40">
              <span className="text-[11px] text-inkdim">+</span>
              <input
                type="number"
                min={1}
                max={200}
                value={extraSteps}
                onChange={(e) => setExtraSteps(Math.max(1, parseInt(e.target.value) || 1))}
                className="w-12 px-1 py-0.5 text-[12px] font-mono text-center bg-input border border-border/60 rounded focus:border-accent outline-none text-ink"
                title="自定义追加的步数"
              />
            </div>
          </div>

          {/* 拒绝结案 */}
          <button
            disabled={submitting}
            onClick={handleDeny}
            className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-lg bg-panel2/80 hover:bg-red-500/20 text-inkdim hover:text-red-300 border border-border hover:border-red-500/30 text-[12px] font-medium transition-all"
            title="拒绝增加步数，任务以当前进度结案收尾"
          >
            <XCircle size={14} />
            <span>拒绝并结案</span>
          </button>

          {/* 批准热续跑 */}
          <button
            disabled={submitting}
            onClick={() => handleApprove()}
            className="inline-flex items-center gap-1.5 px-3.5 py-1.5 rounded-lg bg-emerald-600 hover:bg-emerald-500 text-white font-medium text-[12px] transition-all shadow-md hover:shadow-emerald-500/20 active:scale-95 disabled:opacity-50"
            title={`追加 ${extraSteps} 步并继续执行`}
          >
            {submitting ? (
              <Loader2 size={14} className="animate-spin" />
            ) : (
              <Play size={14} className="fill-current" />
            )}
            <span>批准并续跑 (+{extraSteps})</span>
          </button>
        </div>
      </div>
    </div>
  );
};
