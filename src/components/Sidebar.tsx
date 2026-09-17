import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { ipc } from "../ipc";
import { currentSession, useStore } from "../store";
import { DRAFT_ID, dirName, type Session } from "../types";
import { askConfirm, askPrompt } from "./PromptModal";

import {
  ChevronRight,
  ChevronDown,
  ChevronUp,
  Plus,
  MoreHorizontal,
  Pin,
  Sprout,
  Archive,
  Settings,
  List,
  FolderTree,
  Wind,
  MessageSquare,
} from "./Icons";

function timeLabel(iso?: string | null) {
  if (!iso) return "";
  const d = new Date(iso);
  const now = Date.now();
  const diff = now - d.getTime();
  if (diff < 60_000) return "刚刚";
  if (diff < 3600_000) return `${Math.floor(diff / 60_000)} 分钟前`;
  if (diff < 86400_000) return `${Math.floor(diff / 3600_000)} 小时前`;
  return d.toLocaleDateString("zh-CN");
}

function formatTokens(n?: number): string {
  if (n == null || isNaN(n)) return "0";
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(2)}M`;
  if (n >= 10_000) return `${(n / 1_000).toFixed(1)}k`;
  return n.toLocaleString("zh-CN");
}

function ItemMenu({
  items,
  onClose,
}: {
  items: { label: string; danger?: boolean; onClick: () => void }[];
  onClose: () => void;
}) {
  return (
    <>
      <div
        className="fixed inset-0 z-40"
        onMouseDown={(e) => {
          e.stopPropagation();
          onClose();
        }}
        onClick={(e) => e.stopPropagation()}
      />
      <div
        className="absolute right-1 top-7 z-50 bg-panel2 border border-edge rounded-lg shadow-xl py-1 min-w-[130px] animate-in fade-in zoom-in-95 duration-100"
        onClick={(e) => e.stopPropagation()}
      >
        {items.map((it) => (
          <button
            key={it.label}
            className={`w-full text-left px-3 py-1.5 text-[13px] hover:bg-panel3 transition-colors ${it.danger ? "text-red-400" : "text-ink"}`}
            onClick={(e) => {
              e.stopPropagation();
              onClose();
              it.onClick();
            }}
          >
            {it.label}
          </button>
        ))}
      </div>
    </>
  );
}

/** 展开收起箭头：平滑旋转 90° */
function Chevron({ collapsed }: { collapsed?: boolean }) {
  return (
    <ChevronRight
      size={13}
      className={`transition-transform duration-150 text-inkdim shrink-0 ${collapsed ? "" : "rotate-90"}`}
    />
  );
}

export function Sidebar() {
  const sessions = useStore((s) => s.sessions);
  const projects = useStore((s) => s.projects);
  const currentId = useStore((s) => s.currentId);
  const view = useStore((s) => s.view);
  const runStatus = useStore((s) => s.runStatus);
  const setView = useStore((s) => s.setView);
  const enterProject = useStore((s) => s.enterProject);
  const newDraft = useStore((s) => s.newDraft);
  const newTempDraft = useStore((s) => s.newTempDraft);
  const selectSession = useStore((s) => s.selectSession);
  const draft = useStore((s) => s.draft);
  const setShowArchive = useStore((s) => s.setShowArchive);
  const setShowSettings = useStore((s) => s.setShowSettings);
  const setShowGrowthModal = useStore((s) => s.setShowGrowthModal);
  const setProjectSettings = useStore((s) => s.setProjectSettings);
  const setSessionSettings = useStore((s) => s.setSessionSettings);
  const toggleProjectPinned = useStore((s) => s.toggleProjectPinned);
  const pushToast = useStore((s) => s.pushToast);
  const [menuFor, setMenuFor] = useState<string | null>(null);
  const [collapsed, setCollapsed] = useState<Record<string, boolean>>({});
  // 项目视图顶部大分类（项目 / 对话）的收起状态
  const [groupCollapsed, setGroupCollapsed] = useState<{ projects?: boolean; chats?: boolean }>({});
  // 项目下对话是否展开全部（默认只展示前5条，超出收进“更多”）
  const [expandedMore, setExpandedMore] = useState<Record<string, boolean>>({});

  const createProjectByDialog = async () => {
    const picked = await open({ directory: true, multiple: false, title: "新建项目：选择目录" });
    if (typeof picked !== "string") return;
    await ipc.createProject(dirName(picked), picked);
    setGroupCollapsed((g) => ({ ...g, projects: false }));
  };

  const sorted = [...sessions].sort((a, b) =>
    (b.lastMessageAt ?? b.createdAt).localeCompare(a.lastMessageAt ?? a.createdAt)
  );
  const sortedProjects = [...projects].sort((a, b) => {
    const aPin = Boolean(a.pinned);
    const bPin = Boolean(b.pinned);
    if (aPin !== bPin) return aPin ? -1 : 1;
    const aTime = a.lastActivityAt || a.createdAt || "";
    const bTime = b.lastActivityAt || b.createdAt || "";
    return bTime.localeCompare(aTime);
  });
  // 未归入项目（纯对话）的会话
  const ungrouped = sorted.filter((s) => !s.projectId);

  const sessionMenu = (s: Session) => [
    { label: "会话设置", onClick: () => setSessionSettings(s.id) },
    { label: "重命名", onClick: async () => {
        const t = await askPrompt({ title: "重命名会话", value: s.title });
        if (t && t.trim()) await ipc.renameSession(s.id, t.trim());
      } },
    { label: "归档", onClick: async () => {
        // 临时空间对话在其临时空间未清空时会被后端拒绝，需把原因提示出来
        try {
          await ipc.archiveSession(s.id);
        } catch (e) {
          pushToast(String(e));
        }
      } },
    { label: "删除", danger: true, onClick: async () => {
        if (!(await askConfirm(`确定删除会话「${s.title}」？`))) return;
        try {
          await ipc.deleteSession(s.id);
          if (currentId === s.id) newDraft(s.projectId ?? null); // 落回可发送的空对话（保持项目上下文）
        } catch (e) {
          pushToast(String(e));
        }
      } },
  ];

  const projectMenu = (pid: string, pinned: boolean, hasPath: string | null | undefined) => [
    ...(hasPath
      ? [{ label: "打开目录", onClick: () => ipc.openDir(hasPath).catch((e) => pushToast(String(e))) }]
      : []),
    { label: "项目设置", onClick: () => setProjectSettings(pid) },
    { label: "成长记录", onClick: () => setShowGrowthModal(true, pid) },
    { label: pinned ? "取消固定" : "固定到顶部", onClick: () => void toggleProjectPinned(pid) },
    { label: "移除项目", danger: true, onClick: async () => {
        if (await askConfirm("移除该项目？（项目下的对话将移回顶层，不会被删除）", "移除")) {
          await ipc.removeProject(pid);
        }
      } },
  ];

  const SessionRow = ({ s, indent }: { s: Session; indent: boolean }) => {
    const isRunning = runStatus[s.id] === "running";
    return (
      <div
        className={`group relative flex items-center gap-1.5 rounded-lg px-2 py-1.5 cursor-pointer mr-2 transition-colors ${
          currentId === s.id ? "bg-panel3 text-ink font-medium" : "hover:bg-panel2 text-ink/90"
        } ${indent ? "ml-4" : ""}`}
        onClick={() => selectSession(s.id)}
      >
        {isRunning && (
          <span className="w-2 h-2 rounded-full bg-green-400 animate-pulse shrink-0 shadow-[0_0_8px_rgba(74,222,128,0.6)]" title="Agent 运行中" />
        )}
        <div className="flex-1 min-w-0">
          <div className="truncate text-[13px]" title={s.title}>
            {s.title}
          </div>
          <div className={`flex items-center gap-1.5 text-[11px] ${isRunning ? "text-green-400" : "text-inkdim"}`}>
            <span className="truncate">{isRunning ? "运行中…" : timeLabel(s.lastMessageAt ?? s.createdAt)}</span>
            {s.totalTokens != null && s.totalTokens > 0 && (
              <span
                className="shrink-0 font-mono text-[10px] text-inkdim/75"
                title={`会话累计消耗: ${(Number(s.totalTokens) || 0).toLocaleString()} tokens`}
              >
                · {formatTokens(s.totalTokens)} tokens
              </span>
            )}
            {s.isTemp && (
              <span className="shrink-0 inline-flex items-center gap-0.5 px-1.5 py-[1px] rounded text-[10px] leading-none bg-amber-500/15 text-amber-400 border border-amber-500/30">
                <Wind size={9} />
                <span>临时</span>
              </span>
            )}
          </div>
        </div>
        <button
          className={`opacity-0 group-hover:opacity-100 w-6 h-6 rounded hover:bg-edge flex items-center justify-center text-inkdim hover:text-ink transition-opacity ${
            menuFor === s.id ? "opacity-100" : ""
          }`}
          onClick={(e) => {
            e.stopPropagation();
            setMenuFor(menuFor === s.id ? null : s.id);
          }}
        >
          <MoreHorizontal size={14} />
        </button>
        {menuFor === s.id && <ItemMenu items={sessionMenu(s)} onClose={() => setMenuFor(null)} />}
      </div>
    );
  };

  return (
    <div className="w-[260px] shrink-0 h-full border-r border-edge bg-panel flex flex-col select-none">
      {/* 顶部：视图切换 + 新会话（对齐顶栏基线 h-12） */}
      <div className="h-12 border-b border-edge/60 px-3 flex items-center gap-2 shrink-0">
        <div className="flex bg-panel2 rounded-lg p-0.5 border border-edge">
          <button
            className={`flex items-center gap-1 px-2.5 py-1 rounded-md text-[12px] transition-colors ${
              view === "list" ? "bg-panel3 text-ink shadow-sm" : "text-inkdim hover:text-ink"
            }`}
            onClick={() => setView("list")}
          >
            <List size={13} />
            <span>列表</span>
          </button>
          <button
            className={`flex items-center gap-1 px-2.5 py-1 rounded-md text-[12px] transition-colors ${
              view === "project" ? "bg-panel3 text-ink shadow-sm" : "text-inkdim hover:text-ink"
            }`}
            onClick={() => setView("project")}
          >
            <FolderTree size={13} />
            <span>项目</span>
          </button>
        </div>
        <button
          className="ml-auto w-7 h-7 rounded-lg bg-accent/15 text-accent hover:bg-accent/25 flex items-center justify-center transition-colors"
          title="新建对话"
          onClick={() => {
            // 继承当前会话所属项目的工作区（含临时空间对话的项目上下文）
            const curSession = currentId && currentId !== DRAFT_ID ? sessions.find((s) => s.id === currentId) : null;
            const projectId = curSession?.projectId ?? draft?.projectId ?? null;
            newDraft(projectId);
          }}
        >
          <Plus size={16} strokeWidth={2.2} />
        </button>
      </div>

      {/* 会话树 */}
      <div className="flex-1 overflow-y-auto pl-2 pb-2">
        {view === "list" && (
          <>
            {sorted.map((s) => (
              <SessionRow key={s.id} s={s} indent={false} />
            ))}
            {sorted.length === 0 && <div className="text-inkdim text-[13px] px-2 py-4">暂无会话</div>}
          </>
        )}
        {view === "project" && (
          <>
            {/* 项目大分类：收起/展开项目列表；悬停在尾部显示“新建项目” */}
            <div className="mb-1">
              <div
                className="group relative flex items-center gap-1 rounded-lg px-2 py-1.5 cursor-pointer mr-2 hover:bg-panel2 transition-colors"
                onClick={() => setGroupCollapsed({ ...groupCollapsed, projects: !groupCollapsed.projects })}
              >
                <div className="flex-1 min-w-0 flex items-center gap-1">
                  <span className="text-[13px] font-medium text-inkdim">项目</span>
                  <button
                    className="opacity-0 group-hover:opacity-100 text-inkdim text-[11px] hover:text-ink shrink-0"
                    title={groupCollapsed.projects ? "展开" : "折叠"}
                    onClick={(e) => {
                      e.stopPropagation();
                      setGroupCollapsed({ ...groupCollapsed, projects: !groupCollapsed.projects });
                    }}
                  >
                    <Chevron collapsed={groupCollapsed.projects} />
                  </button>
                </div>
                <button
                  className="opacity-0 group-hover:opacity-100 w-6 h-6 rounded hover:bg-edge flex items-center justify-center text-inkdim hover:text-ink transition-opacity"
                  title="新建项目（选择目录）"
                  onClick={(e) => {
                    e.stopPropagation();
                    void createProjectByDialog();
                  }}
                >
                  <Plus size={14} strokeWidth={2} />
                </button>
              </div>
              {!groupCollapsed.projects && (
                <>
                  {sortedProjects.map((p) => {
                    const children = sorted.filter((s) => s.projectId === p.id);
                    const isCollapsed = collapsed[p.id];
                    const isActive = currentId === DRAFT_ID && draft?.projectId === p.id;
                    const isMoreExpanded = !!expandedMore[p.id];
                    const hasMore = children.length > 5;
                    const displayChildren = hasMore && !isMoreExpanded ? children.slice(0, 5) : children;
                    return (
                      <div key={p.id} className="mb-1">
                        <div
                          className={`group relative flex items-center gap-1 rounded-lg px-2 py-1.5 cursor-pointer mr-2 hover:bg-panel2 transition-colors ${
                            isActive ? "bg-panel3" : ""
                          }`}
                          onClick={() => {
                            // 进入项目：展开并显示该项目下的空对话（可发送，落库到该项目）
                            setCollapsed({ ...collapsed, [p.id]: false });
                            enterProject(p.id);
                          }}
                        >
                          <button
                            className="text-inkdim text-[11px] w-3.5 hover:text-ink flex items-center justify-center"
                            title={isCollapsed ? "展开" : "折叠"}
                            onClick={(e) => {
                              e.stopPropagation();
                              setCollapsed({ ...collapsed, [p.id]: !isCollapsed });
                            }}
                          >
                            <Chevron collapsed={isCollapsed} />
                          </button>
                          <div className="flex-1 min-w-0 flex items-center gap-1">
                            {p.pinned && <Pin size={12} className="text-accent fill-accent shrink-0" />}
                            <span className="truncate text-[13px] font-medium" title={p.path ?? p.name}>
                              {p.name}
                            </span>
                          </div>
                          {p.path && (
                            <button
                              className="opacity-0 group-hover:opacity-100 w-6 h-6 rounded hover:bg-edge flex items-center justify-center text-amber-400 hover:text-amber-300 transition-opacity"
                              title="临时空间（拷贝项目到临时目录开新对话，原目录不受影响）"
                              onClick={(e) => {
                                e.stopPropagation();
                                void newTempDraft(p.id);
                              }}
                            >
                              <Wind size={13} />
                            </button>
                          )}
                          <button
                            className="opacity-0 group-hover:opacity-100 w-6 h-6 rounded hover:bg-edge flex items-center justify-center text-inkdim hover:text-ink transition-opacity"
                            title="新建对话（归入该项目）"
                            onClick={(e) => {
                              e.stopPropagation();
                              newDraft(p.id);
                            }}
                          >
                            <Plus size={14} strokeWidth={2} />
                          </button>
                          <button
                            className={`opacity-0 group-hover:opacity-100 w-6 h-6 rounded hover:bg-edge flex items-center justify-center text-inkdim hover:text-ink transition-opacity ${
                              menuFor === p.id ? "opacity-100" : ""
                            }`}
                            onClick={(e) => {
                              e.stopPropagation();
                              setMenuFor(menuFor === p.id ? null : p.id);
                            }}
                          >
                            <MoreHorizontal size={14} />
                          </button>
                          {menuFor === p.id && <ItemMenu items={projectMenu(p.id, p.pinned, p.path)} onClose={() => setMenuFor(null)} />}
                        </div>
                        {!isCollapsed &&
                          displayChildren.map((s) => <SessionRow key={s.id} s={s} indent={true} />)}
                        {!isCollapsed && hasMore && (
                          !isMoreExpanded ? (
                            <div
                              className="flex items-center gap-1 rounded-lg px-2 py-1 text-[12px] text-inkdim hover:text-ink hover:bg-panel2 cursor-pointer mr-2 ml-4 select-none transition-colors"
                              title={`共 ${children.length} 条对话，点击展开全部`}
                              onClick={() => setExpandedMore((prev) => ({ ...prev, [p.id]: true }))}
                            >
                              <ChevronDown size={12} className="text-inkdim" />
                              <span>更多</span>
                              <span className="text-[11px] text-inkdim">({children.length})</span>
                            </div>
                          ) : (
                            <div
                              className="flex items-center gap-1 rounded-lg px-2 py-1 text-[12px] text-inkdim hover:text-ink hover:bg-panel2 cursor-pointer mr-2 ml-4 select-none transition-colors"
                              title="收起对话"
                              onClick={() => setExpandedMore((prev) => ({ ...prev, [p.id]: false }))}
                            >
                              <ChevronUp size={12} className="text-inkdim" />
                              <span>收起</span>
                            </div>
                          )
                        )}
                        {!isCollapsed && children.length === 0 && (
                          <div className="ml-4 text-[12px] text-inkdim py-1">暂无对话</div>
                        )}
                      </div>
                    );
                  })}
                  {sortedProjects.length === 0 && (
                    <div className="ml-4 text-[12px] text-inkdim py-1">暂无项目 · 悬停“项目”点 + 新建</div>
                  )}
                </>
              )}
            </div>

            {/* 对话分类：收纳未归入项目的纯对话；不存在时不显示 */}
            {ungrouped.length > 0 && (
              <div className="mb-1">
                <div
                  className="group relative flex items-center gap-1 rounded-lg px-2 py-1.5 cursor-pointer mr-2 hover:bg-panel2 transition-colors"
                  onClick={() => setGroupCollapsed({ ...groupCollapsed, chats: !groupCollapsed.chats })}
                >
                  <div className="flex-1 min-w-0 flex items-center gap-1">
                    <span className="text-[13px] font-medium text-inkdim">对话</span>
                    <button
                      className="opacity-0 group-hover:opacity-100 text-inkdim text-[11px] hover:text-ink shrink-0"
                      title={groupCollapsed.chats ? "展开" : "折叠"}
                      onClick={(e) => {
                        e.stopPropagation();
                        setGroupCollapsed({ ...groupCollapsed, chats: !groupCollapsed.chats });
                      }}
                    >
                      <Chevron collapsed={groupCollapsed.chats} />
                    </button>
                  </div>
                  <button
                    className="opacity-0 group-hover:opacity-100 w-6 h-6 rounded hover:bg-edge flex items-center justify-center text-inkdim hover:text-ink transition-opacity"
                    title="新建纯对话"
                    onClick={(e) => {
                      e.stopPropagation();
                      newDraft(null, false);
                    }}
                  >
                    <Plus size={14} strokeWidth={2} />
                  </button>
                </div>
                {!groupCollapsed.chats &&
                  ungrouped.map((s) => <SessionRow key={s.id} s={s} indent={false} />)}
              </div>
            )}
          </>
        )}
      </div>

      {/* 底部：成长 / 统计 / 归档 / 设置 */}
      <div className="border-t border-edge p-2 flex items-center gap-1 bg-panel/80">
        <button
          className="flex-1 text-left px-2 py-1.5 rounded-lg hover:bg-panel2 text-[12px] text-inkdim hover:text-ink flex items-center gap-1.5 transition-colors"
          onClick={() => setShowGrowthModal(true, null)}
          title="查看与管理 Agent 的经验与成长记录"
        >
          <Sprout size={15} className="text-emerald-400 shrink-0" />
          <span>成长</span>
        </button>
        <button
          className="flex-1 text-left px-2 py-1.5 rounded-lg hover:bg-panel2 text-[12px] text-inkdim hover:text-ink flex items-center gap-1.5 transition-colors"
          onClick={() => setShowArchive(true)}
          title="查看已归档的会话记录"
        >
          <Archive size={15} className="shrink-0" />
          <span>归档</span>
        </button>
        <button
          className="flex-1 text-left px-2 py-1.5 rounded-lg hover:bg-panel2 text-[12px] text-inkdim hover:text-ink flex items-center gap-1.5 transition-colors"
          onClick={() => setShowSettings(true)}
          title="打开程序与模型设置"
        >
          <Settings size={15} className="shrink-0" />
          <span>设置</span>
        </button>
      </div>
    </div>
  );
}

export { currentSession, DRAFT_ID };
