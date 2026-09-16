import { useState } from "react";
import { useStore } from "../store";
import type { GrowthItem } from "../types";
import { Markdown } from "./Markdown";
import { Sprout, Check, Brain, ChevronRight, Edit3, X } from "./Icons";

const CATEGORY_NAMES: Record<string, string> = {
  command_rule: "命令规范",
  code_style: "代码规范",
  build_test: "构建测试",
  pitfall: "避坑防雷",
  workflow: "工作流",
};

export function GrowthCard({ item }: { item: GrowthItem }) {
  const acceptGrowth = useStore((s) => s.acceptGrowth);
  const rejectGrowth = useStore((s) => s.rejectGrowth);
  const updateGrowthRule = useStore((s) => s.updateGrowthRule);
  const [openThought, setOpenThought] = useState(false);
  const [editing, setEditing] = useState(false);
  const [title, setTitle] = useState(item.title);
  const [ruleContent, setRuleContent] = useState(item.ruleContent);
  const [category, setCategory] = useState(item.category);

  const isProposed = item.status === "proposed";
  const isAccepted = item.status === "accepted";
  const isRejected = item.status === "rejected";

  const handleSaveEdit = async () => {
    await updateGrowthRule(item.id, title, ruleContent, category);
    setEditing(false);
  };

  return (
    <div
      className={`rounded-2xl border p-4 transition-all duration-200 ${
        isAccepted
          ? "bg-emerald-950/20 border-emerald-500/40 text-ink shadow-sm"
          : isRejected
            ? "bg-zinc-900/40 border-edge opacity-60 text-inkdim"
            : "bg-panel2 border-emerald-500/40 shadow-md"
      }`}
    >
      {/* 头部标题与分类 */}
      <div className="flex items-center justify-between gap-2 mb-2">
        <div className="flex items-center gap-2">
          <Sprout size={16} className="text-emerald-400 shrink-0" />
          <span className="font-medium text-[13px] text-ink">
            Agent 成长提案 · {CATEGORY_NAMES[item.category] ?? item.category}
          </span>
          <span className="text-[11px] text-inkdim px-1.5 py-0.5 rounded-md bg-panel3 border border-edge/60">
            {item.title}
          </span>
        </div>
        <div className="flex items-center gap-1">
          {isProposed && (
            <span className="text-[11px] px-2 py-0.5 rounded-full bg-amber-500/15 text-amber-400 font-medium border border-amber-500/30">
              待审阅
            </span>
          )}
          {isAccepted && (
            <span className="text-[11px] px-2 py-0.5 rounded-full bg-emerald-500/15 text-emerald-400 font-medium flex items-center gap-1 border border-emerald-500/30">
              <Check size={11} strokeWidth={2.5} /> 已固化生效
            </span>
          )}
          {isRejected && (
            <span className="text-[11px] px-2 py-0.5 rounded-full bg-zinc-500/20 text-zinc-400 border border-zinc-500/30">
              已忽略
            </span>
          )}
        </div>
      </div>

      {/* 触发来源 */}
      {item.triggerContext && (
        <div className="text-[12px] text-inkdim bg-panel/60 rounded-lg px-2.5 py-1.5 mb-2 font-mono whitespace-pre-wrap border border-edge/40">
          <span className="text-ink font-sans">触发背景：</span>
          {item.triggerContext}
        </div>
      )}

      {/* 反思推演过程（折叠） */}
      {item.reflectionThought && (
        <div className="mb-2">
          <button
            className="text-[12px] text-inkdim hover:text-ink flex items-center gap-1.5 py-1 select-none transition-colors"
            onClick={() => setOpenThought(!openThought)}
          >
            <ChevronRight size={13} className={`transition-transform duration-150 text-inkdim shrink-0 ${openThought ? "rotate-90" : ""}`} />
            <Brain size={13} className="text-purple-400 shrink-0" />
            <span>查看 AI 反思推导过程</span>
          </button>
          {openThought && (
            <div className="text-[12px] text-inkdim bg-panel3/40 rounded-lg p-2.5 mt-1 border border-edge leading-relaxed whitespace-pre-wrap">
              {item.reflectionThought}
            </div>
          )}
        </div>
      )}

      {/* 提炼的规则正文 */}
      <div className="my-2 bg-panel rounded-xl p-3 border border-edge/80">
        <div className="text-[11px] font-medium text-inkdim uppercase tracking-wider mb-1.5">
          提炼规则（将注入项目上下文指导后续编码）：
        </div>
        {editing ? (
          <div className="flex flex-col gap-2">
            <input
              className="bg-panel2 border border-edge rounded px-2.5 py-1.5 text-[13px] text-ink outline-none focus:border-accent"
              value={title}
              onChange={(e) => setTitle(e.target.value)}
              placeholder="简要标题"
            />
            <textarea
              className="bg-panel2 border border-edge rounded p-2.5 text-[13px] font-mono text-ink outline-none resize-y min-h-[70px] focus:border-accent"
              value={ruleContent}
              onChange={(e) => setRuleContent(e.target.value)}
              placeholder="规则正文 (Markdown)"
            />
            <div className="flex justify-end gap-2 mt-1">
              <button
                className="px-2.5 py-1 rounded text-[12px] text-inkdim hover:bg-panel3 transition-colors"
                onClick={() => setEditing(false)}
              >
                取消
              </button>
              <button
                className="px-2.5 py-1 rounded text-[12px] bg-accent hover:bg-blue-500 text-white transition-colors shadow-sm"
                onClick={handleSaveEdit}
              >
                保存修改
              </button>
            </div>
          </div>
        ) : (
          <div className="text-[13px] leading-relaxed text-ink">
            <Markdown content={item.ruleContent} />
          </div>
        )}
      </div>

      {/* 操作栏 */}
      {isProposed && !editing && (
        <div className="flex items-center gap-2 mt-3 pt-2 border-t border-edge/60">
          <button
            className="px-3 py-1.5 rounded-lg bg-emerald-600/90 hover:bg-emerald-500 text-white text-[12px] font-medium flex items-center gap-1.5 shadow-sm transition-colors"
            onClick={() => acceptGrowth(item.id)}
          >
            <Check size={13} strokeWidth={2.2} />
            <span>采纳并固化到项目</span>
          </button>
          <button
            className="px-3 py-1.5 rounded-lg bg-panel3 hover:bg-edge text-ink text-[12px] flex items-center gap-1.5 transition-colors"
            onClick={() => setEditing(true)}
          >
            <Edit3 size={13} />
            <span>编辑修改</span>
          </button>
          <button
            className="px-3 py-1.5 rounded-lg bg-panel3 hover:bg-edge text-inkdim hover:text-red-400 text-[12px] flex items-center gap-1.5 transition-colors"
            onClick={() => rejectGrowth(item.id)}
          >
            <X size={13} />
            <span>忽略</span>
          </button>
        </div>
      )}

      {isAccepted && (
        <div className="text-[11px] text-emerald-400/90 mt-2 flex items-center gap-1.5">
          <Check size={12} strokeWidth={2.5} />
          <span>已固化为项目长效经验，下一轮对话自动装配进系统提示词。</span>
        </div>
      )}
    </div>
  );
}
