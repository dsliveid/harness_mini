import { memo } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import rehypeHighlight from "rehype-highlight";
import { SafeImage } from "./SafeImage";
import { useStore } from "../store";

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
