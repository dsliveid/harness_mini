import { memo } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import rehypeHighlight from "rehype-highlight";
import { toAssetUrl } from "../utils/image";
import { useStore } from "../store";

export const Markdown = memo(function Markdown({ content }: { content: string }) {
  const setLightboxImage = useStore((s) => s.setLightboxImage);

  return (
    <div className="md-content">
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        rehypePlugins={[rehypeHighlight]}
        components={{
          img: ({ src, alt, ...props }) => {
            const assetSrc = toAssetUrl(src || "");
            return (
              <img
                src={assetSrc}
                alt={alt || "图片"}
                onClick={() => setLightboxImage({ src: src || "", title: alt })}
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
