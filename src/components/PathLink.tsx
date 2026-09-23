import React, { useState, useEffect, useMemo } from "react";
import { ipc } from "../ipc";
import { useStore } from "../store";
import {
  FileCode,
  Folder,
  FileText,
} from "./Icons";
import {
  parsePathString,
  resolveAbsolutePath,
  cachedInspectPath,
  getCachedInspectResult,
} from "../utils/pathResolver";
import type { PathInspectResult } from "../types";

export interface PathLinkProps {
  rawPath: string;
  workspacePath?: string | null;
  currentDocPath?: string | null;
  children?: React.ReactNode;
  className?: string;
  isInlineCode?: boolean;
  isAutoLinked?: boolean;
  isExplicitLink?: boolean;
}

// 根据类型配置视觉主题颜色（纯函数推导，脱离 Hook 调度体系）
function getPathTheme(isPlan: boolean, isDirectory: boolean) {
  if (isPlan) {
    return {
      pill: "bg-emerald-500/10 text-emerald-300 border-emerald-500/30 hover:bg-emerald-500/20 hover:border-emerald-500/50 hover:text-emerald-200",
      badge: "bg-emerald-500/20 text-emerald-200 border-emerald-500/30",
      icon: <FileText size={12} className="text-emerald-400 shrink-0" />,
    };
  }
  if (isDirectory) {
    return {
      pill: "bg-amber-500/10 text-amber-300 border-amber-500/30 hover:bg-amber-500/20 hover:border-amber-500/50 hover:text-amber-200",
      badge: "bg-amber-500/20 text-amber-200 border-amber-500/30",
      icon: <Folder size={12} className="text-amber-400 shrink-0" />,
    };
  }
  // 默认文件（代码文件、配置文件等）采用清晰的青绿色 (Teal)
  return {
    pill: "bg-teal-500/10 text-teal-300 border-teal-500/30 hover:bg-teal-500/20 hover:border-teal-500/50 hover:text-teal-200",
    badge: "bg-teal-500/20 text-teal-200 border-teal-500/30",
    icon: <FileCode size={12} className="text-teal-400 shrink-0" />,
  };
}

export const PathLink: React.FC<PathLinkProps> = ({
  rawPath,
  workspacePath,
  currentDocPath,
  children,
  className = "",
  isInlineCode = false,
  isAutoLinked = false,
  isExplicitLink = false,
}) => {
  const pushToast = useStore((s) => s.pushToast);
  const storeWorkspacePath = useStore(
    (s) =>
      s.sessions.find((x) => x.id === s.currentId)?.workspacePath ||
      s.draft?.workspacePath ||
      s.settings.lastWorkspacePath ||
      ""
  );

  const effectiveWorkspace = (workspacePath || storeWorkspacePath || "").trim();
  const parsed = useMemo(() => parsePathString(rawPath), [rawPath]);

  const targetPathToInspect = useMemo(() => {
    if (currentDocPath && (parsed.cleanPath.startsWith("./") || parsed.cleanPath.startsWith("../"))) {
      return resolveAbsolutePath(parsed.cleanPath, effectiveWorkspace, currentDocPath);
    }
    return parsed.cleanPath;
  }, [parsed.cleanPath, effectiveWorkspace, currentDocPath]);

  // 同步初始化：优先从内存命中，消除初次渲染的闪烁
  const [inspectInfo, setInspectInfo] = useState<PathInspectResult | null>(() => {
    return getCachedInspectResult(targetPathToInspect, effectiveWorkspace) || null;
  });

  // 异步探测目标真实性
  useEffect(() => {
    let canceled = false;
    cachedInspectPath(targetPathToInspect, effectiveWorkspace).then((res) => {
      if (!canceled && res) {
        setInspectInfo(res);
      }
    });
    return () => {
      canceled = true;
    };
  }, [targetPathToInspect, effectiveWorkspace]);

  // 1. 尚未完成文件探测时的平滑降级（在确认真实存在前保持纯文本或原生样式，绝不提前渲染胶囊）
  if (inspectInfo === null) {
    if (isAutoLinked) {
      // 纯文本探测出来的候选词，在未确认真实存在前保持纯文本
      return <span>{children || parsed.raw}</span>;
    }
    if (isInlineCode) {
      // 行内代码在未确认真实存在前保持文字标签样式
      return (
        <code className={`bg-panel3/90 text-zinc-300 border border-zinc-700/60 rounded-md px-1.5 py-0.5 font-mono text-[12px] inline-block my-0.5 select-text ${className}`}>
          {children || parsed.raw}
        </code>
      );
    }
    // 显式链接在探测中展示纯文本，待确认后再渲染
    return (
      <span className="inline-flex items-center text-zinc-300 select-text">
        {children || parsed.cleanPath}
      </span>
    );
  }

  // 2. 经后端真实文件系统检验后：目标路径不存在
  if (inspectInfo !== null && !inspectInfo.exists) {
    if (isAutoLinked) {
      // 纯文本误判词（如并列名词、and/or 等），直接还原为纯文本，绝不生成无效链接
      return <span>{children || parsed.raw}</span>;
    }
    if (isInlineCode) {
      // 命令行、普通代码或不存在的代码路径，还原为文字标签
      return (
        <code className={`bg-panel3/90 text-zinc-300 border border-zinc-700/60 rounded-md px-1.5 py-0.5 font-mono text-[12px] inline-block my-0.5 select-text ${className}`}>
          {children || parsed.raw}
        </code>
      );
    }
    if (isExplicitLink) {
      // 作者明确写了 [name](path) 语法，但文件确实不存在：展示失效提示
      return (
        <a
          href={`#path:${parsed.cleanPath}`}
          onClick={(e) => {
            e.preventDefault();
            pushToast(`未在工作区找到目标: ${parsed.cleanPath}`, "warning");
          }}
          className="inline-flex items-center gap-1 font-mono text-[12px] text-zinc-400 hover:text-zinc-200 line-through decoration-dotted select-none px-1.5 py-0.5 rounded border border-dashed border-zinc-700/60 bg-zinc-800/30"
          title={`目标路径未找到: ${parsed.cleanPath}`}
        >
          <span>{children || parsed.cleanPath}</span>
          <span className="text-[10px] text-amber-400 font-sans">(未找到)</span>
        </a>
      );
    }
    return <span>{children || parsed.raw}</span>;
  }

  // 3. 目标路径确认为真实存在！渲染交互式路径胶囊（仅响应点击，无悬浮浮窗）
  const finalAbsPath =
    inspectInfo?.absPath ||
    resolveAbsolutePath(parsed.cleanPath, effectiveWorkspace, currentDocPath);

  const isDirectory = inspectInfo ? inspectInfo.isDir : parsed.isExplicitDir;

  const handleClick = (e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();

    if (isDirectory) {
      // 打开系统文件管理器
      ipc
        .openDir(finalAbsPath, effectiveWorkspace)
        .then(() => {
          pushToast(`📂 已打开目录: ${parsed.fileName || parsed.cleanPath}`);
        })
        .catch((err) => {
          pushToast(`打开目录失败: ${err}`, "error");
        });
      return;
    }

    if (parsed.isPlan) {
      // 打开任务方案预览
      ipc.openFileViewer({
        id: `plan:${parsed.fileName}`,
        type: "plan",
        title: parsed.fileName,
        planId: parsed.fileName.replace(/\.md$/i, ""),
        workspacePath: effectiveWorkspace,
      });
      return;
    }

    // 打开内置文件查看器
    ipc.openFileViewer({
      id: `file:${finalAbsPath}`,
      type: "file",
      title: parsed.fileName,
      subtitle: parsed.cleanPath,
      path: finalAbsPath,
      highlightLine: parsed.highlightLine,
      highlightRange: parsed.highlightRange,
      workspacePath: effectiveWorkspace,
    });
  };

  const theme = getPathTheme(parsed.isPlan, isDirectory);

  const actionTitle = isDirectory
    ? `在文件管理器中打开目录: ${finalAbsPath}`
    : `在文件查看器中打开: ${finalAbsPath}${parsed.highlightLine ? `:${parsed.highlightLine}` : ""}`;

  return (
    <a
      href={`#path:${parsed.cleanPath}`}
      onClick={handleClick}
      className={`inline-flex items-center gap-1.5 font-mono text-[12px] cursor-pointer select-none transition-all rounded-md px-1.5 py-0.5 border shadow-2xs my-0.5 ${theme.pill} ${className}`}
      title={actionTitle}
    >
      {theme.icon}
      <span className="truncate max-w-[280px] sm:max-w-[400px]">
        {children || parsed.cleanPath}
      </span>
      {parsed.highlightLine && (
        <span className={`text-[10px] px-1 py-0.2 rounded font-semibold shrink-0 border ${theme.badge}`}>
          :{parsed.highlightLine}
        </span>
      )}
    </a>
  );
};
