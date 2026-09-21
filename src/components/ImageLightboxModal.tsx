import { useState, useEffect } from "react";
import { useStore } from "../store";
import { SafeImage } from "./SafeImage";
import { X, ZoomIn, ZoomOut, RotateCcw, Copy, Check } from "./Icons";

export function ImageLightboxModal() {
  const lightboxImage = useStore((s) => s.lightboxImage);
  const setLightboxImage = useStore((s) => s.setLightboxImage);
  const pushToast = useStore((s) => s.pushToast);

  const [scale, setScale] = useState(1);
  const [copied, setCopied] = useState(false);
  const [copiedTitle, setCopiedTitle] = useState(false);

  useEffect(() => {
    setScale(1);
    setCopied(false);
    setCopiedTitle(false);
  }, [lightboxImage]);

  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        setLightboxImage(null);
      }
    };
    if (lightboxImage) {
      window.addEventListener("keydown", handleKeyDown);
      return () => window.removeEventListener("keydown", handleKeyDown);
    }
  }, [lightboxImage, setLightboxImage]);

  if (!lightboxImage) return null;

  const handleCopyPath = async () => {
    try {
      await navigator.clipboard.writeText(lightboxImage.src);
      setCopied(true);
      pushToast("已复制图片路径到剪贴板");
      setTimeout(() => setCopied(false), 2000);
    } catch {
      pushToast("复制失败");
    }
  };

  const handleCopyTitle = async (e: React.MouseEvent) => {
    e.stopPropagation();
    const text = lightboxImage.title || lightboxImage.alt;
    if (!text) return;
    try {
      await navigator.clipboard.writeText(text);
      setCopiedTitle(true);
      pushToast("已复制提示词到剪贴板");
      setTimeout(() => setCopiedTitle(false), 2000);
    } catch {
      pushToast("复制失败");
    }
  };

  const zoomIn = () => setScale((s) => Math.min(s + 0.25, 4));
  const zoomOut = () => setScale((s) => Math.max(s - 0.25, 0.5));
  const resetZoom = () => setScale(1);

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/80 backdrop-blur-md p-4 animate-in fade-in duration-150 select-none"
      onClick={() => setLightboxImage(null)}
    >
      {/* Floating Toolbar */}
      <div
        className="absolute top-4 right-4 z-50 flex items-center gap-2 bg-panel/80 border border-edge rounded-xl p-1.5 shadow-xl backdrop-blur-md"
        onClick={(e) => e.stopPropagation()}
      >
        <button
          onClick={zoomOut}
          title="缩小"
          className="p-1.5 rounded-lg hover:bg-panel2 text-inkdim hover:text-ink transition-colors cursor-pointer"
        >
          <ZoomOut size={16} />
        </button>
        <span className="text-[11px] font-mono text-inkdim px-1 min-w-[38px] text-center">
          {Math.round(scale * 100)}%
        </span>
        <button
          onClick={zoomIn}
          title="放大"
          className="p-1.5 rounded-lg hover:bg-panel2 text-inkdim hover:text-ink transition-colors cursor-pointer"
        >
          <ZoomIn size={16} />
        </button>
        <button
          onClick={resetZoom}
          title="重置缩放"
          className="p-1.5 rounded-lg hover:bg-panel2 text-inkdim hover:text-ink transition-colors cursor-pointer"
        >
          <RotateCcw size={15} />
        </button>
        <div className="w-px h-4 bg-edge mx-0.5" />
        <button
          onClick={handleCopyPath}
          title="复制本地路径"
          className="p-1.5 rounded-lg hover:bg-panel2 text-inkdim hover:text-ink transition-colors cursor-pointer flex items-center gap-1"
        >
          {copied ? <Check size={16} className="text-emerald-400" /> : <Copy size={16} />}
        </button>
        <button
          onClick={() => setLightboxImage(null)}
          title="关闭 (Esc)"
          className="p-1.5 rounded-lg hover:bg-panel2 text-inkdim hover:text-ink transition-colors cursor-pointer"
        >
          <X size={17} />
        </button>
      </div>

      {/* Title / Description */}
      {(lightboxImage.title || lightboxImage.alt) && (
        <div
          className="absolute bottom-4 left-1/2 -translate-x-1/2 z-50 max-w-[85vw] bg-panel/90 border border-edge rounded-xl px-3.5 py-1.5 shadow-xl backdrop-blur-md text-[13px] text-ink flex items-center gap-2 select-text"
          onClick={(e) => e.stopPropagation()}
        >
          <span className="truncate max-w-[70vw] font-mono" title={lightboxImage.title || lightboxImage.alt}>
            🎨 {lightboxImage.title || lightboxImage.alt}
          </span>
          <button
            type="button"
            onClick={handleCopyTitle}
            title="复制提示词"
            className="p-1 rounded-lg hover:bg-panel2 text-inkdim hover:text-ink transition-colors cursor-pointer shrink-0"
          >
            {copiedTitle ? <Check size={14} className="text-emerald-400" /> : <Copy size={14} />}
          </button>
        </div>
      )}

      {/* Image Container */}
      <div
        className="max-w-[90vw] max-h-[85vh] overflow-hidden flex items-center justify-center"
        onClick={(e) => e.stopPropagation()}
      >
        <SafeImage
          src={lightboxImage.src}
          alt={lightboxImage.alt || "图片预览"}
          style={{ transform: `scale(${scale})` }}
          className="max-w-[85vw] max-h-[80vh] object-contain rounded-lg shadow-2xl transition-transform duration-100 ease-out cursor-zoom-in"
          onClick={scale < 2 ? zoomIn : resetZoom}
        />
      </div>
    </div>
  );
}
