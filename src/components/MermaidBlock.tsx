import { useEffect, useRef, useState, useId, memo } from "react";
import mermaid from "mermaid";
import { Copy, Check, Code, Eye, Maximize2, Loader2 } from "./Icons";
import { useStore } from "../store";

let initialized = false;
function ensureMermaidInitialized() {
  if (!initialized) {
    mermaid.initialize({
      startOnLoad: false,
      theme: "dark",
      themeVariables: {
        darkMode: true,
        background: "transparent",
        primaryColor: "#1e293b",
        primaryTextColor: "#f1f5f9",
        primaryBorderColor: "#334155",
        lineColor: "#64748b",
        secondaryColor: "#0f172a",
        tertiaryColor: "#1e293b",
      },
      securityLevel: "loose",
      fontFamily: "ui-sans-serif, system-ui, sans-serif",
    });
    initialized = true;
  }
}

// 模块级极速渲染缓存 Map<cleanCode, svgString>
const MERMAID_RENDER_CACHE = new Map<string, string>();
const MAX_CACHE_ENTRIES = 120;

function setSvgCache(key: string, svg: string) {
  if (MERMAID_RENDER_CACHE.size >= MAX_CACHE_ENTRIES) {
    const firstKey = MERMAID_RENDER_CACHE.keys().next().value;
    if (firstKey) MERMAID_RENDER_CACHE.delete(firstKey);
  }
  MERMAID_RENDER_CACHE.set(key, svg);
}

interface MermaidBlockProps {
  code: string;
}

export const MermaidBlock = memo(function MermaidBlock({ code }: MermaidBlockProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const rawId = useId();
  const safeId = `mermaid_${rawId.replace(/[^a-zA-Z0-9_-]/g, "_")}`;

  const setLightboxDiagram = useStore((s) => s.setLightboxDiagram);

  const cleanCode = code.trim();
  const initialCache = cleanCode ? MERMAID_RENDER_CACHE.get(cleanCode) || "" : "";

  // 状态机：svgHtml 初始优先从全局内存命中，绝不空置
  const [svgHtml, setSvgHtml] = useState<string>(initialCache);
  const [isCompiling, setIsCompiling] = useState<boolean>(!initialCache && !!cleanCode);
  const [showCode, setShowCode] = useState(false);
  const [copied, setCopied] = useState(false);

  // 记录最近一次编译成功的合法 SVG，作为双缓冲防护底板
  const lastValidSvgRef = useRef<string>(initialCache);
  const debounceTimerRef = useRef<any>(null);

  useEffect(() => {
    if (!cleanCode) {
      setSvgHtml("");
      setIsCompiling(false);
      return;
    }

    // 1. 同步命中缓存：0 延迟原地复用，绝不重绘
    const cached = MERMAID_RENDER_CACHE.get(cleanCode);
    if (cached) {
      setSvgHtml(cached);
      lastValidSvgRef.current = cached;
      setIsCompiling(false);
      return;
    }

    // 2. 流式输入防抖：避免每一个字符都触发昂贵的 mermaid.render
    if (debounceTimerRef.current) {
      clearTimeout(debounceTimerRef.current);
    }

    setIsCompiling(true);

    debounceTimerRef.current = setTimeout(async () => {
      try {
        ensureMermaidInitialized();
        // 预检语法合法性
        await mermaid.parse(cleanCode);

        const renderId = `${safeId}_${Date.now()}`;
        const { svg } = await mermaid.render(renderId, cleanCode);

        // 编译成功：写入全局缓存并原子性更新视图
        setSvgCache(cleanCode, svg);
        lastValidSvgRef.current = svg;
        setSvgHtml(svg);
        setIsCompiling(false);
      } catch (err) {
        // 语法未完整或流式未闭合：保持上一次合法图表或展示静默等待，绝不跳黄色报错卡片打乱高度！
        setIsCompiling(false);
      }
    }, 280); // 280ms 优雅防抖：兼顾流式结束响应速度与编译性能

    return () => {
      if (debounceTimerRef.current) {
        clearTimeout(debounceTimerRef.current);
      }
    };
  }, [cleanCode, safeId]);

  const handleCopyCode = () => {
    navigator.clipboard.writeText(code);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };

  const hasSvg = Boolean(svgHtml || lastValidSvgRef.current);
  const activeSvg = svgHtml || lastValidSvgRef.current;

  const handleOpenLightbox = () => {
    if (hasSvg && activeSvg) {
      setLightboxDiagram({
        svgHtml: activeSvg,
        title: "Mermaid 架构流程图",
        code: cleanCode,
      });
    }
  };

  return (
    <div className="my-3 rounded-xl border border-edge/80 bg-panel2/60 overflow-hidden shadow-sm transition-all">
      {/* 顶栏操作条 */}
      <div className="flex items-center justify-between px-3 py-1.5 border-b border-edge/60 bg-panel3/50 text-[11px] text-inkdim select-none">
        <div className="flex items-center gap-1.5 font-medium">
          <span className="w-2 h-2 rounded-full bg-cyan-400"></span>
          <span>Mermaid 图表</span>
          {isCompiling && (
            <span className="inline-flex items-center gap-1 text-[10px] text-cyan-400/80 font-normal ml-1 animate-pulse">
              <Loader2 size={10} className="animate-spin" />
              <span>构图中…</span>
            </span>
          )}
        </div>
        <div className="flex items-center gap-1">
          {hasSvg && (
            <button
              type="button"
              onClick={handleOpenLightbox}
              className="p-1 rounded hover:bg-panel hover:text-ink text-inkdim transition-colors cursor-pointer"
              title="打开全屏弹窗大图 (默认放大 145%，支持滚轮缩放与平移)"
            >
              <Maximize2 size={13} />
            </button>
          )}
          <button
            type="button"
            onClick={() => setShowCode(!showCode)}
            className="p-1 rounded hover:bg-panel hover:text-ink text-inkdim transition-colors cursor-pointer"
            title={showCode ? "查看渲染图表" : "查看 Mermaid 源码"}
          >
            {showCode ? <Eye size={13} /> : <Code size={13} />}
          </button>
          <button
            type="button"
            onClick={handleCopyCode}
            className="p-1 rounded hover:bg-panel hover:text-ink text-inkdim transition-colors cursor-pointer"
            title="复制图表源码"
          >
            {copied ? <Check size={13} className="text-emerald-400" /> : <Copy size={13} />}
          </button>
        </div>
      </div>

      {/* 主展示区 */}
      {showCode ? (
        <pre className="p-3 text-[12px] font-mono text-ink/90 overflow-x-auto bg-panel/40 select-text">
          <code>{code}</code>
        </pre>
      ) : hasSvg ? (
        <div
          ref={containerRef}
          onClick={handleOpenLightbox}
          className="p-4 overflow-x-auto flex justify-center items-center bg-panel/20 cursor-zoom-in group hover:bg-panel/30 transition-colors [&_svg]:max-w-full [&_svg]:h-auto min-h-[120px]"
          dangerouslySetInnerHTML={{ __html: activeSvg }}
          title="点击打开全屏弹窗大图 (默认以 145% 放大状态呈现，支持滚轮缩放与平移)"
        />
      ) : (
        /* 首步尚未生成合法 SVG 时：以紧凑代码块静默占位，高度恒定，绝不突变报错卡片 */
        <div className="p-3 bg-panel/30 min-h-[90px] flex flex-col justify-center">
          <div className="text-[11px] text-inkdim/70 mb-1.5 flex items-center gap-1.5">
            <Loader2 size={12} className="animate-spin text-cyan-400" />
            <span>AI 正在绘制架构图表…</span>
          </div>
          <pre className="text-[11.5px] font-mono text-ink/60 truncate opacity-70">
            <code>{cleanCode || "正在准备图表定义..."}</code>
          </pre>
        </div>
      )}
    </div>
  );
});
