import { useState, useRef, useEffect, useMemo, useCallback } from "react";
import { createPortal } from "react-dom";
import { X, ZoomIn, ZoomOut, RotateCcw, Copy, Check, Download, Maximize2 } from "./Icons";
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

  const dragStartRef = useRef({ x: 0, y: 0, startClientX: 0, startClientY: 0 });
  const hasDraggedRef = useRef(false);

  // 1. 标准化解析 Mermaid SVG：提取真实矢量自然宽高 (viewBox)，消除 0x0 塌陷与行内限制
  const { processedSvgHtml, diagramDim, fitScale } = useMemo(() => {
    const rawSvg = lightboxDiagram?.svgHtml || "";
    if (!rawSvg) {
      return {
        processedSvgHtml: "",
        diagramDim: { width: 800, height: 500 },
        fitScale: 1.0,
      };
    }

    // 优先从 viewBox 提取真实坐标系尺寸 (viewBox="minX minY width height")
    const viewBoxMatch = rawSvg.match(
      /viewBox=["']\s*([0-9.-]+)\s+([0-9.-]+)\s+([0-9.-]+)\s+([0-9.-]+)\s*["']/i
    );
    let naturalWidth = 0;
    let naturalHeight = 0;

    if (viewBoxMatch) {
      naturalWidth = Math.abs(parseFloat(viewBoxMatch[3]));
      naturalHeight = Math.abs(parseFloat(viewBoxMatch[4]));
    }

    // 备用方案：从 style="max-width: ...px" 提取宽度
    if (!naturalWidth || isNaN(naturalWidth)) {
      const maxWidthMatch = rawSvg.match(/max-width:\s*([0-9.]+)px/i);
      if (maxWidthMatch) {
        naturalWidth = parseFloat(maxWidthMatch[1]);
        naturalHeight = naturalWidth * 0.6;
      }
    }

    // 物理安全兜底，杜绝 0 尺寸
    if (!naturalWidth || naturalWidth < 20 || isNaN(naturalWidth)) naturalWidth = 800;
    if (!naturalHeight || naturalHeight < 20 || isNaN(naturalHeight)) naturalHeight = 500;

    const roundedWidth = Math.round(naturalWidth);
    const roundedHeight = Math.round(naturalHeight);

    // 标准化清洗 <svg> 根节点属性：剔除限制性的 max-width 与固定尺寸，赋予 100% 容器自适应能力
    const cleanedSvg = rawSvg.replace(/<svg\b([^>]*)>/i, (_match, attrs) => {
      const cleanAttrs = attrs
        .replace(/\bwidth=["'][^"']*["']/gi, "")
        .replace(/\bheight=["'][^"']*["']/gi, "")
        .replace(/\bstyle=["'][^"']*["']/gi, "")
        .trim();

      return `<svg ${cleanAttrs} width="100%" height="100%" style="display:block;max-width:none;max-height:none;overflow:visible;">`;
    });

    // 计算适应当前屏幕的比例 (留出外围工具栏与边距)
    const availW = Math.max(window.innerWidth * 0.82, 320);
    const availH = Math.max(window.innerHeight * 0.78, 240);
    const calculatedFit = Math.min(availW / roundedWidth, availH / roundedHeight, 2.5);

    return {
      processedSvgHtml: cleanedSvg,
      diagramDim: {
        width: roundedWidth,
        height: roundedHeight,
      },
      fitScale: Math.round(calculatedFit * 100) / 100,
    };
  }, [lightboxDiagram?.svgHtml]);

  // 2. 每次打开新图表时，重置为默认放大状态 (145%) 并居中
  useEffect(() => {
    if (lightboxDiagram) {
      setScale(DEFAULT_ENLARGED_SCALE);
      setPan({ x: 0, y: 0 });
      setCopiedCode(false);
      hasDraggedRef.current = false;
    }
  }, [lightboxDiagram]);

  // 3. ESC 键盘快捷键退出
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape" && lightboxDiagram) {
        setLightboxDiagram(null);
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [lightboxDiagram, setLightboxDiagram]);

  // 4. 鼠标滚轮平滑缩放 (15% ~ 600%)
  const handleWheel = useCallback((e: React.WheelEvent) => {
    e.preventDefault();
    e.stopPropagation();
    const zoomFactor = e.deltaY < 0 ? 1.12 : 0.89;
    setScale((s) => {
      const next = s * zoomFactor;
      return Math.min(Math.max(next, 0.15), 6.0);
    });
  }, []);

  // 5. 鼠标左键拖拽平移
  const handleMouseDown = (e: React.MouseEvent) => {
    if (e.button !== 0) return; // 仅响应鼠标左键拖拽
    setIsDragging(true);
    hasDraggedRef.current = false;
    dragStartRef.current = {
      x: e.clientX - pan.x,
      y: e.clientY - pan.y,
      startClientX: e.clientX,
      startClientY: e.clientY,
    };
  };

  const handleMouseMove = (e: React.MouseEvent) => {
    if (!isDragging) return;
    const dx = e.clientX - dragStartRef.current.startClientX;
    const dy = e.clientY - dragStartRef.current.startClientY;
    if (Math.hypot(dx, dy) > 5) {
      hasDraggedRef.current = true;
    }
    setPan({
      x: e.clientX - dragStartRef.current.x,
      y: e.clientY - dragStartRef.current.y,
    });
  };

  const handleMouseUp = () => setIsDragging(false);

  // 点击背景遮罩关闭（拖拽释放时不误触关闭）
  const handleBackdropClick = () => {
    if (hasDraggedRef.current) return;
    setLightboxDiagram(null);
  };

  // 6. 双击快捷切换：在【145% 默认放大】与【整屏适应 (fitScale) 或 100%】间无缝切换
  const handleDoubleClick = (e: React.MouseEvent) => {
    e.stopPropagation();
    if (Math.abs(scale - DEFAULT_ENLARGED_SCALE) < 0.1) {
      setScale(fitScale || 1.0);
      setPan({ x: 0, y: 0 });
    } else {
      setScale(DEFAULT_ENLARGED_SCALE);
      setPan({ x: 0, y: 0 });
    }
  };

  // 7. 导出为高清 SVG 矢量文件
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

  if (!lightboxDiagram) return null;

  const title = lightboxDiagram.title || "Mermaid 架构流程图";
  const code = lightboxDiagram.code;

  // 使用 React Portal 传送门挂载在 document.body 根部，完全脱离任何祖先包含块
  return createPortal(
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/85 backdrop-blur-md select-none animate-in fade-in duration-150 overflow-hidden"
      onMouseMove={handleMouseMove}
      onMouseUp={handleMouseUp}
      onMouseLeave={handleMouseUp}
      onWheel={handleWheel}
      onClick={handleBackdropClick}
    >
      {/* 顶部标题与交互说明 */}
      <div
        className="absolute top-4 left-6 z-50 flex items-center gap-2 pointer-events-none"
        onClick={(e) => e.stopPropagation()}
      >
        <span className="w-2.5 h-2.5 rounded-full bg-cyan-400"></span>
        <span className="text-[13px] font-medium text-white/95 drop-shadow-md">{title}</span>
        <span className="text-[11px] text-white/55 font-mono hidden md:inline">
          （滚轮缩放 · 鼠标拖拽 · 双击复位 · ESC 退出）
        </span>
      </div>

      {/* 悬浮控制工具栏 */}
      <div
        className="absolute top-4 right-6 z-50 flex items-center gap-1.5 bg-panel/90 border border-edge rounded-xl p-1.5 shadow-2xl backdrop-blur-md"
        onClick={(e) => e.stopPropagation()}
      >
        {/* 缩放微调 */}
        <button
          onClick={() => setScale((s) => Math.max(s * 0.88, 0.15))}
          className="p-1.5 rounded-lg hover:bg-panel2 text-inkdim hover:text-ink transition-colors cursor-pointer"
          title="缩小 (滚轮下滚)"
        >
          <ZoomOut size={16} />
        </button>
        <span className="text-[11px] font-mono text-ink px-1 min-w-[42px] text-center font-medium">
          {Math.round(scale * 100)}%
        </span>
        <button
          onClick={() => setScale((s) => Math.min(s * 1.12, 6.0))}
          className="p-1.5 rounded-lg hover:bg-panel2 text-inkdim hover:text-ink transition-colors cursor-pointer"
          title="放大 (滚轮上滚)"
        >
          <ZoomIn size={16} />
        </button>

        <div className="w-px h-4 bg-edge mx-0.5" />

        {/* 挡位 1：100% 原始尺寸 */}
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
          title="切换至 100% 原始矢量尺寸"
        >
          100%
        </button>

        {/* 挡位 2：整屏适应 */}
        <button
          onClick={() => {
            setScale(fitScale);
            setPan({ x: 0, y: 0 });
          }}
          className={`px-2 py-1 rounded-lg text-[11px] font-mono transition-colors cursor-pointer ${
            Math.abs(scale - fitScale) < 0.05
              ? "bg-emerald-500/20 text-emerald-400 font-semibold"
              : "hover:bg-panel2 text-inkdim hover:text-ink"
          }`}
          title={`自适应视口大小 (${Math.round(fitScale * 100)}%)`}
        >
          适应
        </button>

        {/* 挡位 3：145% 默认放大状态 */}
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

        {/* 重置 */}
        <button
          onClick={() => {
            setScale(DEFAULT_ENLARGED_SCALE);
            setPan({ x: 0, y: 0 });
          }}
          className="p-1.5 rounded-lg hover:bg-panel2 text-inkdim hover:text-ink transition-colors cursor-pointer"
          title="重置视图居中"
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

      {/* 视口交互画布区 */}
      <div
        className={`w-full h-full flex items-center justify-center ${
          isDragging ? "cursor-grabbing" : "cursor-grab"
        }`}
        onMouseDown={handleMouseDown}
        onDoubleClick={handleDoubleClick}
      >
        {/* 经由矩阵平移与缩放的图表卡片 */}
        <div
          style={{
            transform: `translate3d(${pan.x}px, ${pan.y}px, 0) scale(${scale})`,
            transformOrigin: "center center",
            transition: isDragging ? "none" : "transform 0.08s cubic-bezier(0.16, 1, 0.3, 1)",
          }}
          onClick={(e) => e.stopPropagation()}
          className="select-none flex items-center justify-center p-6 rounded-2xl bg-panel2/95 border border-edge/80 shadow-2xl backdrop-blur-md"
        >
          {/* 具备真实物理尺寸的 SVG 容器，绝不塌陷为 0x0 */}
          <div
            style={{
              width: `${diagramDim.width}px`,
              height: `${diagramDim.height}px`,
            }}
            className="flex items-center justify-center [&_svg]:w-full [&_svg]:h-full [&_svg]:max-w-none [&_svg]:max-h-none pointer-events-auto"
            dangerouslySetInnerHTML={{ __html: processedSvgHtml }}
          />
        </div>
      </div>
    </div>,
    document.body
  );
}
