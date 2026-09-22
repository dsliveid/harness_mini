import { memo } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import rehypeHighlight from "rehype-highlight";
import { SafeImage } from "./SafeImage";
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

export const Markdown = memo(function Markdown({
  content,
  workspacePath,
}: {
  content: string;
  workspacePath?: string | null;
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

  return (
    <div className="md-content">
      <ReactMarkdown
        urlTransform={safeUrlTransform}
        remarkPlugins={[remarkGfm]}
        rehypePlugins={[rehypeHighlight]}
        components={{
          a: ({ href, children, ...props }) => {
            const trimmedHref = (href || "").trim();
            const isFileLink =
              trimmedHref &&
              (trimmedHref.startsWith("file://") ||
                trimmedHref.includes(".harness/plans/") ||
                (/\.[a-zA-Z0-9_-]+(#L\d+(-L?\d+)?)?$/.test(trimmedHref) &&
                  !trimmedHref.startsWith("http://") &&
                  !trimmedHref.startsWith("https://")));

            if (isFileLink) {
              const rawClean = trimmedHref.replace(/^file:\/\/\/?/, "");
              const [filePathPart, anchor] = rawClean.split("#L");
              let highlightLine: number | undefined;
              let highlightRange: { start: number; end?: number } | undefined;
              if (anchor) {
                const parts = anchor.includes("-L") ? anchor.split("-L") : anchor.split("-");
                const start = parseInt(parts[0], 10);
                const end = parts[1] ? parseInt(parts[1], 10) : undefined;
                if (!isNaN(start)) {
                  if (end && !isNaN(end)) {
                    highlightRange = { start, end };
                  } else {
                    highlightLine = start;
                  }
                }
              }

              const cleanPath = filePathPart.replace(/\\/g, "/");
              const fileName = cleanPath.split("/").pop() || "文件";
              const isPlan = cleanPath.includes(".harness") && cleanPath.includes("plans");

              return (
                <a
                  href={trimmedHref}
                  onClick={(e) => {
                    e.preventDefault();
                    if (isPlan) {
                      ipc.openFileViewer({
                        id: `plan:${fileName}`,
                        type: "plan",
                        title: fileName,
                        planId: fileName.replace(/\.md$/i, ""),
                        workspacePath: effectiveWorkspace,
                      });
                    } else {
                      // 若为相对路径，拼上当前工作区
                      const absPath =
                        /^[a-zA-Z]:\//.test(cleanPath) || cleanPath.startsWith("/")
                          ? cleanPath
                          : effectiveWorkspace
                          ? `${effectiveWorkspace.replace(/\\/g, "/")}/${cleanPath.replace(/^\.\//, "")}`
                          : cleanPath;

                      ipc.openFileViewer({
                        id: `file:${absPath}`,
                        type: "file",
                        title: fileName,
                        subtitle: cleanPath,
                        path: absPath,
                        highlightLine,
                        highlightRange,
                        workspacePath: effectiveWorkspace,
                      });
                    }
                  }}
                  className="inline-flex items-center gap-1 text-accent hover:underline cursor-pointer font-mono text-[12px] bg-accent/10 px-1.5 py-0.5 rounded border border-accent/20 my-0.5 select-none"
                  title={`在文件查看器中打开: ${cleanPath}`}
                >
                  <span className="text-[11px]">{isPlan ? "📋" : "📄"}</span>
                  <span>{children}</span>
                </a>
              );
            }

            return (
              <a href={href} target="_blank" rel="noreferrer" {...props}>
                {children}
              </a>
            );
          },
          img: ({ src, alt, ...props }) => {
            const finalSrc = resolveMarkdownImagePath(src || "", effectiveWorkspace);
            return (
              <SafeImage
                src={finalSrc}
                alt={alt || "图片"}
                onClick={() => setLightboxImage({ src: finalSrc, title: alt })}
                className="max-h-80 rounded-xl my-2 border border-edge object-contain cursor-zoom-in hover:opacity-95 transition-opacity shadow-sm"
                loading="lazy"
                {...props}
              />
            );
          },
        }}
      >
        {content}
      </ReactMarkdown>
    </div>
  );
});
