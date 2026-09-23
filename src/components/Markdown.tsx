import { memo, useMemo, Component, type ReactNode } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import rehypeHighlight from "rehype-highlight";
import { SafeImage } from "./SafeImage";
import { PathLink } from "./PathLink";
import { ExternalLink } from "./Icons";
import { isPotentialPath, autolinkPlainPaths } from "../utils/pathResolver";
import { parseFrontmatter } from "../utils/frontmatter";
import { FrontmatterCard } from "./FrontmatterCard";
import { useStore } from "../store";
import { ipc } from "../ipc";

export function resolveMarkdownImagePath(src: string, workspacePath?: string | null): string {
  if (!src || typeof src !== "string") return "";
  let resolved = src.trim();
  try {
    resolved = decodeURIComponent(resolved);
  } catch {}
  resolved = resolved.replace(/\\/g, "/");

  const isAbsoluteOrProtocol =
    resolved.startsWith("http://") ||
    resolved.startsWith("https://") ||
    resolved.startsWith("data:") ||
    resolved.startsWith("asset:") ||
    resolved.startsWith("file:") ||
    /^[a-zA-Z]:\//.test(resolved) ||
    resolved.startsWith("/");

  if (!isAbsoluteOrProtocol && workspacePath) {
    const cleanWorkspace = workspacePath.trim().replace(/\\/g, "/").replace(/\/+$/, "");
    const cleanRel = resolved.replace(/^\.\//, "").replace(/^\/+/, "");
    if (cleanWorkspace) {
      resolved = `${cleanWorkspace}/${cleanRel}`;
    }
  }

  return resolved;
}

function safeUrlTransform(url: string): string {
  const trimmed = url.trim().toLowerCase();
  if (trimmed.startsWith("javascript:") || trimmed.startsWith("vbscript:")) {
    return "";
  }
  return url;
}

interface MarkdownErrorBoundaryProps {
  fallbackText: string;
  children: ReactNode;
}

interface MarkdownErrorBoundaryState {
  hasError: boolean;
}

class MarkdownErrorBoundary extends Component<MarkdownErrorBoundaryProps, MarkdownErrorBoundaryState> {
  state: MarkdownErrorBoundaryState = { hasError: false };

  static getDerivedStateFromError() {
    return { hasError: true };
  }

  componentDidCatch(error: Error, errorInfo: any) {
    console.warn("Markdown rendering error caught by local boundary, falling back to raw text:", error, errorInfo);
  }

  componentDidUpdate(prevProps: MarkdownErrorBoundaryProps) {
    if (this.state.hasError && prevProps.fallbackText !== this.props.fallbackText) {
      this.setState({ hasError: false });
    }
  }

  render() {
    if (this.state.hasError) {
      return (
        <div className="p-3 my-1.5 rounded-lg border border-edge/60 bg-panel3/40 text-[12.5px] font-mono text-ink/90 whitespace-pre-wrap select-text">
          {this.props.fallbackText}
        </div>
      );
    }
    return this.props.children;
  }
}

export const Markdown = memo(function Markdown({
  content,
  workspacePath,
  currentDocPath,
  enableAutolink = true,
  showFrontmatter = true,
  onOpenPlanViewer,
}: {
  content: string;
  workspacePath?: string | null;
  currentDocPath?: string | null;
  enableAutolink?: boolean;
  showFrontmatter?: boolean;
  onOpenPlanViewer?: () => void;
}) {
  const setLightboxImage = useStore((s) => s.setLightboxImage);
  const storeWorkspacePath = useStore(
    (s) =>
      s.sessions.find((x) => x.id === s.currentId)?.workspacePath ||
      s.draft?.workspacePath ||
      s.settings.lastWorkspacePath ||
      ""
  );
  const effectiveWorkspace = (workspacePath || storeWorkspacePath || "").trim();

  // 提取 YAML Frontmatter 与纯净 Body
  const { hasFrontmatter, isPlan, meta, rawYaml, body } = useMemo(() => {
    return parseFrontmatter(content || "");
  }, [content]);

  // 纯文本显式路径自动链接化（对剔除 Frontmatter 后的 body 处理）
  const processedContent = useMemo(() => {
    if (!enableAutolink || !body) return body || "";
    return autolinkPlainPaths(body);
  }, [body, enableAutolink]);

  return (
    <div className="md-content">
      {hasFrontmatter && showFrontmatter && (
        <FrontmatterCard
          meta={meta}
          rawYaml={rawYaml}
          isPlan={isPlan}
          workspacePath={effectiveWorkspace}
          currentDocPath={currentDocPath}
          onOpenPlanViewer={onOpenPlanViewer}
        />
      )}
      <MarkdownErrorBoundary fallbackText={content}>
        <ReactMarkdown
          urlTransform={safeUrlTransform}
        remarkPlugins={[remarkGfm]}
        rehypePlugins={[rehypeHighlight]}
        components={{
          a: ({ href, children, ...props }) => {
            const trimmedHref = (href || "").trim();
            const isAutoPath = trimmedHref.startsWith("x-path://");
            const rawActualPath = isAutoPath ? trimmedHref.slice(9) : trimmedHref;
            let actualPath = rawActualPath;
            try {
              actualPath = decodeURIComponent(rawActualPath);
            } catch {}

            // 若是由 autolinkPlainPaths 自动识别的路径候选
            if (isAutoPath) {
              if (isPotentialPath(actualPath)) {
                return (
                  <PathLink
                    rawPath={actualPath}
                    workspacePath={effectiveWorkspace}
                    currentDocPath={currentDocPath}
                    isAutoLinked={true}
                    isExplicitLink={false}
                  >
                    {children}
                  </PathLink>
                );
              }
              // 如果不是潜在合法路径，直接降级为纯文本，绝不能变成外链
              return <span>{children}</span>;
            }

            // 作者显式书写的链接 [text](url)
            const isExternalWeb =
              actualPath.startsWith("http://") ||
              actualPath.startsWith("https://") ||
              actualPath.startsWith("mailto:");

            if (isExternalWeb) {
              return (
                <a
                  href={href}
                  target="_blank"
                  rel="noreferrer"
                  className="inline-flex items-center gap-0.5 text-sky-400 hover:text-sky-300 underline underline-offset-2 decoration-sky-400/40 hover:decoration-sky-300 transition-colors select-text group/ext"
                  {...props}
                >
                  <span>{children}</span>
                  <ExternalLink size={11} className="inline ml-0.5 opacity-65 group-hover/ext:opacity-100 transition-opacity shrink-0 align-baseline" />
                </a>
              );
            }

            // 页内锚点跳转（如 #heading-1）
            if (trimmedHref.startsWith("#")) {
              return (
                <a
                  href={trimmedHref}
                  className="text-sky-400 hover:text-sky-300 underline underline-offset-2 transition-colors select-text"
                  {...props}
                >
                  {children}
                </a>
              );
            }

            // 本地文件、目录或文档相对引用（如 [指南](./guide.md) 或 [源码](src/App.tsx)）
            if (actualPath.startsWith("file://") || isPotentialPath(actualPath)) {
              return (
                <PathLink
                  rawPath={actualPath}
                  workspacePath={effectiveWorkspace}
                  currentDocPath={currentDocPath}
                  isAutoLinked={false}
                  isExplicitLink={true}
                >
                  {children}
                </PathLink>
              );
            }

            // 链接地址：纯外部超链接，独立的天空蓝配色与外部跳转 SVG 小图标
            return (
              <a
                href={href}
                target="_blank"
                rel="noreferrer"
                className="inline-flex items-center gap-0.5 text-sky-400 hover:text-sky-300 underline underline-offset-2 decoration-sky-400/40 hover:decoration-sky-300 transition-colors select-text group/ext"
                {...props}
              >
                <span>{children}</span>
                <ExternalLink size={11} className="inline ml-0.5 opacity-65 group-hover/ext:opacity-100 transition-opacity shrink-0 align-baseline" />
              </a>
            );
          },
          code: ({ className, children, ...props }) => {
            const match = /language-(\w+)/.exec(className || "");
            const text = String(children).replace(/\n$/, "");
            const isMultiLine = text.includes("\n");

            // 单行行内代码：若符合文件或目录路径特征，转换为交互式 PathLink
            if (!match && !isMultiLine && isPotentialPath(text)) {
              return (
                <PathLink
                  rawPath={text}
                  workspacePath={effectiveWorkspace}
                  currentDocPath={currentDocPath}
                  isInlineCode={true}
                >
                  {text}
                </PathLink>
              );
            }

            // 文字标签：中性深灰底色 + 浅白灰字，无链接交互，无下划线，作为纯代码/配置/术语标签
            return (
              <code
                className={`bg-panel3/90 text-zinc-300 border border-zinc-700/60 rounded-md px-1.5 py-0.5 font-mono text-[12px] inline-block my-0.5 select-text ${className || ""}`}
                {...props}
              >
                {children}
              </code>
            );
          },
          h1: ({ node, children, ...props }) => {
            const line = node?.position?.start?.line;
            return (
              <h1 id={line ? `md-line-${line}` : undefined} {...props}>
                {children}
              </h1>
            );
          },
          h2: ({ node, children, ...props }) => {
            const line = node?.position?.start?.line;
            return (
              <h2 id={line ? `md-line-${line}` : undefined} {...props}>
                {children}
              </h2>
            );
          },
          h3: ({ node, children, ...props }) => {
            const line = node?.position?.start?.line;
            return (
              <h3 id={line ? `md-line-${line}` : undefined} {...props}>
                {children}
              </h3>
            );
          },
          h4: ({ node, children, ...props }) => {
            const line = node?.position?.start?.line;
            return (
              <h4 id={line ? `md-line-${line}` : undefined} {...props}>
                {children}
              </h4>
            );
          },
          h5: ({ node, children, ...props }) => {
            const line = node?.position?.start?.line;
            return (
              <h5 id={line ? `md-line-${line}` : undefined} {...props}>
                {children}
              </h5>
            );
          },
          h6: ({ node, children, ...props }) => {
            const line = node?.position?.start?.line;
            return (
              <h6 id={line ? `md-line-${line}` : undefined} {...props}>
                {children}
              </h6>
            );
          },
          table: ({ children, ...props }) => (
            <div className="overflow-x-auto my-3 max-w-full">
              <table {...props}>{children}</table>
            </div>
          ),
          img: ({ src, alt, ...props }) => {
            const finalSrc = resolveMarkdownImagePath(src || "", effectiveWorkspace);
            return (
              <SafeImage
                src={finalSrc}
                alt={alt || "图片"}
                onClick={() => {
                  if (typeof window !== "undefined" && window.location.search.includes("window=file_viewer")) {
                    ipc.openFileViewer({
                      id: `image:${finalSrc}`,
                      type: "image",
                      title: alt || finalSrc.split("/").pop() || "图片",
                      path: finalSrc,
                      workspacePath: effectiveWorkspace,
                    });
                  } else {
                    setLightboxImage({ src: finalSrc, title: alt });
                  }
                }}
                className="max-h-80 rounded-xl my-2 border border-edge object-contain cursor-zoom-in hover:opacity-95 transition-opacity shadow-sm"
                loading="lazy"
                {...props}
              />
            );
          },
        }}
      >
        {processedContent}
        </ReactMarkdown>
      </MarkdownErrorBoundary>
    </div>
  );
});
