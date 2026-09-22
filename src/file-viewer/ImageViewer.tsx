import React, { useState, useEffect, useRef } from "react";
import { ipc } from "../ipc";
import type { ImageViewerTab, FileViewerTab } from "./types";
import {
  ZoomIn,
  ZoomOut,
  Maximize2,
  RotateCcw,
  Copy,
  Check,
  FolderOpen,
  Image as ImageIcon,
  AlertCircle,
} from "../components/Icons";

export function ImageViewer({ tab }: { tab: ImageViewerTab | FileViewerTab }) {
  const [dataUrl, setDataUrl] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [zoom, setZoom] = useState<number>(1);
  const [naturalSize, setNaturalSize] = useState<{ width: number; height: number } | null>(null);
  const [copied, setCopied] = useState(false);
  const [fitMode, setFitMode] = useState<"fit" | "actual" | "custom">("fit");

  const [pan, setPan] = useState({ x: 0, y: 0 });
  const [isDragging, setIsDragging] = useState(false);
  const dragStartRef = useRef({ x: 0, y: 0, panX: 0, panY: 0 });
  const containerRef = useRef<HTMLDivElement>(null);

  const loadImage = async () => {
    setLoading(true);
    setError(null);
    try {
      const res = await ipc.readFileBase64(tab.path);
      setDataUrl(res);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    void loadImage();
    setZoom(1);
    setPan({ x: 0, y: 0 });
    setFitMode("fit");
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tab.path, tab.reloadNonce]);

  const handleImageLoaded = (e: React.SyntheticEvent<HTMLImageElement>) => {
    const img = e.currentTarget;
    setNaturalSize({ width: img.naturalWidth, height: img.naturalHeight });
  };

  const handleZoomIn = () => {
    setZoom((z) => Math.min(5, Math.round((z + 0.25) * 100) / 100));
    setFitMode("custom");
  };

  const handleZoomOut = () => {
    setZoom((z) => Math.max(0.1, Math.round((z - 0.25) * 100) / 100));
    setFitMode("custom");
  };

  const handleActualSize = () => {
    setZoom(1);
    setPan({ x: 0, y: 0 });
    setFitMode("actual");
  };

  const handleFitWindow = () => {
    setZoom(1);
    setPan({ x: 0, y: 0 });
    setFitMode("fit");
  };

  const handleWheel = (e: React.WheelEvent) => {
    e.preventDefault();
    const delta = e.deltaY < 0 ? 0.15 : -0.15;
    setZoom((z) => {
      const newZoom = Math.min(5, Math.max(0.1, Math.round((z + delta) * 100) / 100));
      return newZoom;
    });
    setFitMode("custom");
  };

  const handleMouseDown = (e: React.MouseEvent) => {
    if (e.button !== 0) return;
    setIsDragging(true);
    dragStartRef.current = {
      x: e.clientX,
      y: e.clientY,
      panX: pan.x,
      panY: pan.y,
    };
  };

  const handleMouseMove = (e: React.MouseEvent) => {
    if (!isDragging) return;
    const dx = e.clientX - dragStartRef.current.x;
    const dy = e.clientY - dragStartRef.current.y;
    setPan({
      x: dragStartRef.current.panX + dx,
      y: dragStartRef.current.panY + dy,
    });
  };

  const handleMouseUp = () => {
    setIsDragging(false);
  };

  const handleCopyPath = () => {
    navigator.clipboard.writeText(tab.path).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    });
  };

  const ext = tab.title.split(".").pop()?.toUpperCase() || "IMAGE";

  return (
    <div className="flex-1 flex flex-col min-h-0 bg-panel text-ink overflow-hidden select-none">
      {/* 顶部工具栏 */}
      <div className="h-10 px-4 border-b border-edge/60 bg-panel2/60 flex items-center justify-between shrink-0 text-[12px]">
        <div className="flex items-center gap-2.5 min-w-0">
          <ImageIcon size={15} className="text-purple-400 shrink-0" />
          <span className="font-semibold text-ink truncate">{tab.title}</span>
          <span className="text-[10px] px-1.5 py-0.5 rounded bg-purple-500/15 text-purple-300 border border-purple-500/30 font-mono">
            {ext}
          </span>
          {naturalSize && (
            <span className="text-inkdim font-mono text-[11px] shrink-0">
              {naturalSize.width} × {naturalSize.height} px
            </span>
          )}
          <span className="text-inkdim font-mono text-[11px] truncate max-w-[280px]" title={tab.path}>
            {tab.path}
          </span>
        </div>

        <div className="flex items-center gap-1.5 shrink-0">
          {/* 缩放控制器 */}
          <div className="flex items-center bg-panel3/80 rounded-lg border border-edge px-1 py-0.5 text-[11.5px] font-mono gap-1">
            <button
              type="button"
              className="p-1 rounded hover:bg-panel hover:text-ink text-inkdim transition-colors cursor-pointer"
              title="缩小 (Mouse wheel down)"
              onClick={handleZoomOut}
            >
              <ZoomOut size={13} />
            </button>
            <span className="w-12 text-center text-ink select-none font-medium">
              {Math.round(zoom * 100)}%
            </span>
            <button
              type="button"
              className="p-1 rounded hover:bg-panel hover:text-ink text-inkdim transition-colors cursor-pointer"
              title="放大 (Mouse wheel up)"
              onClick={handleZoomIn}
            >
              <ZoomIn size={13} />
            </button>
          </div>

          <button
            type="button"
            className={`px-2 py-1 rounded-lg text-[11px] font-mono border transition-colors ${
              fitMode === "fit"
                ? "bg-panel border-edge text-ink shadow-2xs font-semibold"
                : "border-transparent text-inkdim hover:text-ink hover:bg-panel3"
            }`}
            title="适应窗口"
            onClick={handleFitWindow}
          >
            <Maximize2 size={13} className="inline mr-1 -mt-0.5" />
            适应
          </button>

          <button
            type="button"
            className={`px-2 py-1 rounded-lg text-[11px] font-mono border transition-colors ${
              fitMode === "actual"
                ? "bg-panel border-edge text-ink shadow-2xs font-semibold"
                : "border-transparent text-inkdim hover:text-ink hover:bg-panel3"
            }`}
            title="原始尺寸 (100%)"
            onClick={handleActualSize}
          >
            1:1
          </button>

          <button
            type="button"
            className="p-1.5 rounded-lg text-inkdim hover:text-ink hover:bg-panel3 transition-colors"
            title="刷新"
            onClick={loadImage}
          >
            <RotateCcw size={14} />
          </button>

          <button
            type="button"
            className="flex items-center gap-1 px-2 py-1 rounded-lg text-inkdim hover:text-ink hover:bg-panel3 transition-colors text-[11px]"
            title="复制文件路径"
            onClick={handleCopyPath}
          >
            {copied ? <Check size={13} className="text-green-400" /> : <Copy size={13} />}
            <span>{copied ? "已复制" : "复制路径"}</span>
          </button>

          <button
            type="button"
            className="p-1.5 rounded-lg text-inkdim hover:text-ink hover:bg-panel3 transition-colors"
            title="在文件资源管理器中打开"
            onClick={() => ipc.openDir(tab.path)}
          >
            <FolderOpen size={14} />
          </button>
        </div>
      </div>

      {/* 图片主视口画布（棋盘透明格背景） */}
      <div
        ref={containerRef}
        className="flex-1 overflow-hidden relative flex items-center justify-center bg-[#18181b] cursor-grab active:cursor-grabbing"
        style={{
          backgroundImage:
            "linear-gradient(45deg, #202023 25%, transparent 25%), linear-gradient(-45deg, #202023 25%, transparent 25%), linear-gradient(45deg, transparent 75%, #202023 75%), linear-gradient(-45deg, transparent 75%, #202023 75%)",
          backgroundSize: "20px 20px",
          backgroundPosition: "0 0, 0 10px, 10px -10px, -10px 0px",
        }}
        onWheel={handleWheel}
        onMouseDown={handleMouseDown}
        onMouseMove={handleMouseMove}
        onMouseUp={handleMouseUp}
        onMouseLeave={handleMouseUp}
      >
        {loading && (
          <div className="flex items-center justify-center h-48 text-inkdim text-[13px] bg-panel/80 px-4 py-2 rounded-xl backdrop-blur-xs">
            加载图片资源中…
          </div>
        )}

        {error && (
          <div className="p-6 flex flex-col items-center justify-center text-red-400 gap-2 bg-panel/90 rounded-2xl border border-edge">
            <AlertCircle size={24} />
            <div className="text-[13px] font-medium">{error}</div>
            <button
              className="mt-2 px-3 py-1 bg-panel3 rounded-lg text-ink text-[12px] hover:bg-edge"
              onClick={loadImage}
            >
              重新加载
            </button>
          </div>
        )}

        {dataUrl && !loading && !error && (
          <div
            className="transition-transform ease-out duration-75 select-none pointer-events-none"
            style={{
              transform: `translate(${pan.x}px, ${pan.y}px) scale(${zoom})`,
              transformOrigin: "center center",
            }}
          >
            <img
              src={dataUrl}
              alt={tab.title}
              onLoad={handleImageLoaded}
              className={`rounded shadow-2xl max-w-none ${
                fitMode === "fit" && zoom === 1
                  ? "max-h-[calc(100vh-140px)] max-w-[calc(100vw-80px)] object-contain"
                  : ""
              }`}
              draggable={false}
            />
          </div>
        )}
      </div>
    </div>
  );
}
