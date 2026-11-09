import { useState, useRef, useEffect, useMemo } from "react";
import { createPortal } from "react-dom";
import { X, ZoomIn, ZoomOut, RotateCcw, Copy, Check, Download, Eye } from "./Icons";
import { useStore } from "../store";

const DEFAULT_ENLARGED_SCALE = 1.45; // 默认以 145% 舒适大图比例打开，无需手动滚轮放大

export function DiagramLightboxModal() {
  const lightboxDiagram = useStore((s) => s.lightboxDiagram);
  const setLightboxDiagram = useStore((s) => s.setLightboxDiagram);
  const pushToast = useStore((s) => s.pushToast);

  const [scale, setScale] = useState(DEFAULT_ENLARGED_SCALE);
  const [pan, setPan] = useState({ x: 0, y: 0 });
  const [isDragging, setIsDragging] = useState(false);
  const [copiedCode, setCopiedCode] = useState(false);

  const dragStartRef = useRef({ x: 0, y: 0 });

  // 每次打开新图表时，重置为默认放大状态 (145%) 并居中
  useEffect(() => {
    if (lightboxDiagram) {
      setScale(DEFAULT_ENLARGED_SCALE);
      setPan({ x: 0, y: 0 });
      setCopiedCode(false);
    }
  }, [lightboxDiagram]);

  // 1. ESC 键盘快捷键退出
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape" && lightboxDiagram) {
        setLightboxDiagram(null);
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [lightboxDiagram, setLightboxDiagram]);

  // 2. 鼠标滚轮平滑缩放 (20% ~ 600%)
  const handleWheel = (e: React.WheelEvent) => {
    e.preventDefault();
    const zoomFactor = e.deltaY < 0 ? 1.15 : 0.87;
    setScale((s) => {
      const next = s * zoomFactor;
      return Math.min(Math.max(next, 0.2), 6.0);
    });
  };

  // 3. 鼠标左键拖拽平移
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

  // 4. 双击快捷切换：在【145% 放大】与【100% 原始/适应】间切换
  const handleDoubleClick = (e: React.MouseEvent) => {
    e.stopPropagation();
    if (Math.abs(scale - DEFAULT_ENLARGED_SCALE) < 0.1) {
      setScale(1.0);
      setPan({ x: 0, y: 0 });
    } else {
      setScale(DEFAULT_ENLARGED_SCALE);
      setPan({ x: 0, y: 0 });
    }
  };

  // 5. 导出为高清 SVG 矢量文件
  const handleExportSvg = () => {
    if (!lightboxDiagram?.svgHtml) return;
    try {
      const blob = new Blob([lightboxDiagram.svgHtml], { type: "image/svg+xml;charset=utf-8" });
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

  // 6. 清理 SVG 内部自带的 max-width 压制，确保脱离 800px 限制实现真正全屏矢量放大
  const processedSvgHtml = useMemo(() => {
    if (!lightboxDiagram?.svgHtml) return "";
    return lightboxDiagram.svgHtml.replace(
      /<svg([^>]*?)style="([^"]*?)"/i,
      (_match, pre, style) => {
        const cleanStyle = style.replace(/max-width:[^;]+;?/gi, "").trim();
        return `<svg${pre}style="${cleanStyle}"`;
      }
    );
  }, [lightboxDiagram?.svgHtml]);

  if (!lightboxDiagram) return null;

  const title = lightboxDiagram.title || "Mermaid 架构流程图";
  const code = lightboxDiagram.code;

  // 使用 React Portal 传送门强保证挂载在 document.body 根部，绝不受聊天对话框或任何父级 CSS 包含块限制
  return createPortal(
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/85 backdrop-blur-md select-none animate-in fade-in duration-150"
      onMouseMove={handleMouseMove}
      onMouseUp={handleMouseUp}
      onMouseLeave={handleMouseUp}
      onWheel={handleWheel}
      onClick={() => setLightboxDiagram(null)}
    >
      {/* 顶部标题与使用说明 */}
      <div
        className="absolute top-4 left-6 z-50 flex items-center gap-2 pointer-events-none"
        onClick={(e) => e.stopPropagation()}
      >
        <span className="w-2.5 h-2.5 rounded-full bg-cyan-400"></span>
        <span className="text-[13px] font-medium text-white/95 drop-shadow-md">{title}</span>
        <span className="text-[11px] text-white/55 font-mono hidden md:inline">
          （滚轮缩放 · 左键按住拖拽 · 双击复位 · ESC 退出）
        </span>
      </div>

      {/* 悬浮控制工具栏 */}
      <div
        className="absolute top-4 right-6 z-50 flex items-center gap-1.5 bg-panel/90 border border-edge rounded-xl p-1.5 shadow-2xl backdrop-blur-md"
        onClick={(e) => e.stopPropagation()}
      >
        {/* 缩放微调 */}
        <button
          onClick={() => setScale((s) => Math.max(s * 0.85, 0.2))}
          className="p-1.5 rounded-lg hover:bg-panel2 text-inkdim hover:text-ink transition-colors cursor-pointer"
          title="缩小 (滚轮下滚)"
        >
          <ZoomOut size={16} />
        </button>
        <span className="text-[11px] font-mono text-ink px-1 min-w-[42px] text-center font-medium">
          {Math.round(scale * 100)}%
        </span>
        <button
          onClick={() => setScale((s) => Math.min(s * 1.15, 6.0))}
          className="p-1.5 rounded-lg hover:bg-panel2 text-inkdim hover:text-ink transition-colors cursor-pointer"
          title="放大 (滚轮上滚)"
        >
          <ZoomIn size={16} />
        </button>

        <div className="w-px h-4 bg-edge mx-0.5" />

        {/* 常用挡位快捷切换 */}
        <button
          onClick={() => {
            setScale(1.0);
            setPan({ x: 0, y: 0 });
          }}
          className={`px-2 py-1 rounded-lg text-[11px] font-mono transition-colors cursor-pointer ${
            Math.abs(scale - 1.0) < 0.05
              ? "bg-accent/20 text-accent font-semibold"
              : "hover:bg-panel2 text-inkdim hover:text-ink"
          }`}
          title="切换至 100% 原始尺寸"
        >
          100%
        </button>
        <button
          onClick={() => {
            setScale(DEFAULT_ENLARGED_SCALE);
            setPan({ x: 0, y: 0 });
          }}
          className={`px-2 py-1 rounded-lg text-[11px] font-mono transition-colors cursor-pointer ${
            Math.abs(scale - DEFAULT_ENLARGED_SCALE) < 0.05
              ? "bg-cyan-500/20 text-cyan-400 font-semibold"
              : "hover:bg-panel2 text-inkdim hover:text-ink"
          }`}
          title="切换至 145% 默认清晰放大状态"
        >
          放大
        </button>

        <button
          onClick={() => {
            setScale(DEFAULT_ENLARGED_SCALE);
            setPan({ x: 0, y: 0 });
          }}
          className="p-1.5 rounded-lg hover:bg-panel2 text-inkdim hover:text-ink transition-colors cursor-pointer"
          title="重置视图"
        >
          <RotateCcw size={15} />
        </button>

        <div className="w-px h-4 bg-edge mx-0.5" />

        {/* 导出与代码复制 */}
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

        <div className="w-px h-4 bg-edge mx-0.5" />

        {/* 关闭按钮 */}
        <button
          onClick={() => setLightboxDiagram(null)}
          className="p-1.5 rounded-lg hover:bg-rose-500/20 text-inkdim hover:text-rose-400 transition-colors cursor-pointer"
          title="关闭图表弹窗 (ESC)"
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
        onDoubleClick={handleDoubleClick}
        onClick={(e) => e.stopPropagation()}
      >
        <div
          style={{
            transform: `translate(${pan.x}px, ${pan.y}px) scale(${scale})`,
            transformOrigin: "center center",
            transition: isDragging ? "none" : "transform 0.08s ease-out",
          }}
          className="[&_svg]:!max-w-none [&_svg]:!w-auto [&_svg]:!h-auto [&_svg]:drop-shadow-2xl select-none"
          dangerouslySetInnerHTML={{ __html: processedSvgHtml }}
        />
      </div>
    </div>,
    document.body
  );
}
