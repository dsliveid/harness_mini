import { useState, useRef, useEffect } from "react";
import { X, ZoomIn, ZoomOut, RotateCcw, Copy, Check, Download } from "./Icons";
import { useStore } from "../store";

interface DiagramLightboxModalProps {
  svgHtml: string;
  title?: string;
  code?: string;
  onClose: () => void;
}

export function DiagramLightboxModal({
  svgHtml,
  title = "图表查看器",
  code,
  onClose,
}: DiagramLightboxModalProps) {
  const pushToast = useStore((s) => s.pushToast);
  const [scale, setScale] = useState(1);
  const [pan, setPan] = useState({ x: 0, y: 0 });
  const [isDragging, setIsDragging] = useState(false);
  const [copiedCode, setCopiedCode] = useState(false);

  const dragStartRef = useRef({ x: 0, y: 0 });

  // 1. ESC 键盘快捷键退出
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [onClose]);

  // 2. 鼠标滚轮平滑缩放 (20% ~ 500%)
  const handleWheel = (e: React.WheelEvent) => {
    e.preventDefault();
    const zoomFactor = e.deltaY < 0 ? 1.15 : 0.87;
    setScale((s) => Math.min(Math.max(s * zoomFactor, 0.2), 5));
  };

  // 3. 鼠标拖拽平移
  const handleMouseDown = (e: React.MouseEvent) => {
    if (e.button !== 0) return; // 仅响应鼠标左键拖拽
    setIsDragging(true);
    dragStartRef.current = { x: e.clientX - pan.x, y: e.clientY - pan.y };
  };

  const handleMouseMove = (e: React.MouseEvent) => {
    if (!isDragging) return;
    setPan({
      x: e.clientX - dragStartRef.current.x,
      y: e.clientY - dragStartRef.current.y,
    });
  };

  const handleMouseUp = () => setIsDragging(false);

  // 4. 重置为默认视图
  const handleReset = () => {
    setScale(1);
    setPan({ x: 0, y: 0 });
  };

  // 5. 导出为高清 SVG 矢量文件
  const handleExportSvg = () => {
    try {
      const blob = new Blob([svgHtml], { type: "image/svg+xml;charset=utf-8" });
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = `mermaid_diagram_${Date.now()}.svg`;
      document.body.appendChild(a);
      a.click();
      document.body.removeChild(a);
      URL.revokeObjectURL(url);
      pushToast("已成功导出 SVG 矢量图表", "success");
    } catch {
      pushToast("导出 SVG 失败", "error");
    }
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/85 backdrop-blur-md select-none animate-in fade-in duration-150"
      onMouseMove={handleMouseMove}
      onMouseUp={handleMouseUp}
      onMouseLeave={handleMouseUp}
      onWheel={handleWheel}
    >
      {/* 顶部标题与操作提示 */}
      <div className="absolute top-4 left-6 z-50 flex items-center gap-2 pointer-events-none">
        <span className="w-2.5 h-2.5 rounded-full bg-cyan-400"></span>
        <span className="text-[13px] font-medium text-white/90 drop-shadow-md">{title}</span>
        <span className="text-[11px] text-white/50 font-mono hidden sm:inline">
          （按住左键拖拽平移 · 滚轮缩放 · ESC 退出）
        </span>
      </div>

      {/* 悬浮控制工具栏 */}
      <div
        className="absolute top-4 right-6 z-50 flex items-center gap-1.5 bg-panel/85 border border-edge rounded-xl p-1.5 shadow-2xl backdrop-blur-md"
        onClick={(e) => e.stopPropagation()}
      >
        <button
          onClick={() => setScale((s) => Math.max(s * 0.85, 0.2))}
          className="p-1.5 rounded-lg hover:bg-panel2 text-inkdim hover:text-ink transition-colors cursor-pointer"
          title="缩小 (滚轮下滚)"
        >
          <ZoomOut size={16} />
        </button>
        <span className="text-[11px] font-mono text-ink px-1 min-w-[42px] text-center">
          {Math.round(scale * 100)}%
        </span>
        <button
          onClick={() => setScale((s) => Math.min(s * 1.15, 5))}
          className="p-1.5 rounded-lg hover:bg-panel2 text-inkdim hover:text-ink transition-colors cursor-pointer"
          title="放大 (滚轮上滚)"
        >
          <ZoomIn size={16} />
        </button>
        <button
          onClick={handleReset}
          className="p-1.5 rounded-lg hover:bg-panel2 text-inkdim hover:text-ink transition-colors cursor-pointer"
          title="重置视图与比例"
        >
          <RotateCcw size={15} />
        </button>
        <div className="w-px h-4 bg-edge mx-1" />
        <button
          onClick={handleExportSvg}
          className="p-1.5 rounded-lg hover:bg-panel2 text-inkdim hover:text-ink transition-colors cursor-pointer"
          title="导出为 SVG 矢量图"
        >
          <Download size={15} />
        </button>
        {code && (
          <button
            onClick={() => {
              navigator.clipboard.writeText(code);
              setCopiedCode(true);
              setTimeout(() => setCopiedCode(false), 1500);
            }}
            className="p-1.5 rounded-lg hover:bg-panel2 text-inkdim hover:text-ink transition-colors cursor-pointer"
            title="复制图表源码"
          >
            {copiedCode ? <Check size={15} className="text-emerald-400" /> : <Copy size={15} />}
          </button>
        )}
        <div className="w-px h-4 bg-edge mx-1" />
        <button
          onClick={onClose}
          className="p-1.5 rounded-lg hover:bg-rose-500/20 text-inkdim hover:text-rose-400 transition-colors cursor-pointer"
          title="关闭查看器 (ESC)"
        >
          <X size={16} />
        </button>
      </div>

      {/* 交互视口画布：基于平移与缩放矩阵变换 */}
      <div
        className={`w-full h-full flex items-center justify-center overflow-hidden cursor-grab ${
          isDragging ? "cursor-grabbing" : ""
        }`}
        onMouseDown={handleMouseDown}
      >
        <div
          style={{
            transform: `translate(${pan.x}px, ${pan.y}px) scale(${scale})`,
            transformOrigin: "center center",
            transition: isDragging ? "none" : "transform 0.08s ease-out",
          }}
          className="[&_svg]:max-w-none [&_svg]:max-h-none select-none"
          dangerouslySetInnerHTML={{ __html: svgHtml }}
        />
      </div>
    </div>
  );
}
