import { useEffect, useRef, useState, useId } from "react";
import mermaid from "mermaid";
import { Copy, Check, Code, Eye } from "./Icons";

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

interface MermaidBlockProps {
  code: string;
}

export function MermaidBlock({ code }: MermaidBlockProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const [svgHtml, setSvgHtml] = useState<string>("");
  const [error, setError] = useState<string | null>(null);
  const [showCode, setShowCode] = useState(false);
  const [copied, setCopied] = useState(false);
  const rawId = useId();
  const safeId = `mermaid_${rawId.replace(/[^a-zA-Z0-9_-]/g, "_")}`;

  useEffect(() => {
    let isMounted = true;
    const cleanCode = code.trim();
    if (!cleanCode) {
      setSvgHtml("");
      setError(null);
      return;
    }

    ensureMermaidInitialized();

    const renderDiagram = async () => {
      try {
        const renderId = `${safeId}_${Date.now()}`;
        // Validate diagram syntax first
        await mermaid.parse(cleanCode);
        const { svg } = await mermaid.render(renderId, cleanCode);
        if (isMounted) {
          setSvgHtml(svg);
          setError(null);
        }
      } catch (err: any) {
        if (isMounted) {
          setError(err?.message || "图表语法解析中或存在格式错误");
        }
      }
    };

    renderDiagram();

    return () => {
      isMounted = false;
    };
  }, [code, safeId]);

  const handleCopyCode = () => {
    navigator.clipboard.writeText(code);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };

  return (
    <div className="my-3 rounded-xl border border-edge/80 bg-panel2/60 overflow-hidden shadow-sm">
      <div className="flex items-center justify-between px-3 py-1.5 border-b border-edge/60 bg-panel3/50 text-[11px] text-inkdim select-none">
        <div className="flex items-center gap-1.5 font-medium">
          <span className="w-2 h-2 rounded-full bg-cyan-400"></span>
          <span>Mermaid 图表</span>
        </div>
        <div className="flex items-center gap-1">
          <button
            type="button"
            onClick={() => setShowCode(!showCode)}
            className="p-1 rounded hover:bg-panel hover:text-ink text-inkdim transition-colors"
            title={showCode ? "查看渲染图表" : "查看 Mermaid 源码"}
          >
            {showCode ? <Eye size={13} /> : <Code size={13} />}
          </button>
          <button
            type="button"
            onClick={handleCopyCode}
            className="p-1 rounded hover:bg-panel hover:text-ink text-inkdim transition-colors"
            title="复制代码"
          >
            {copied ? <Check size={13} className="text-emerald-400" /> : <Copy size={13} />}
          </button>
        </div>
      </div>

      {showCode ? (
        <pre className="p-3 text-[12px] font-mono text-ink/90 overflow-x-auto bg-panel/40">
          <code>{code}</code>
        </pre>
      ) : error ? (
        <div className="p-3 flex flex-col gap-2">
          <div className="text-[11px] text-amber-400/90 font-mono bg-amber-500/10 border border-amber-500/20 px-2.5 py-1.5 rounded-lg">
            图表渲染中或语法解析错误：{error}
          </div>
          <pre className="p-2.5 text-[12px] font-mono text-ink/80 overflow-x-auto bg-panel/30 rounded-lg">
            <code>{code}</code>
          </pre>
        </div>
      ) : (
        <div
          ref={containerRef}
          className="p-4 overflow-x-auto flex justify-center items-center bg-panel/20 [&_svg]:max-w-full [&_svg]:h-auto"
          dangerouslySetInnerHTML={{ __html: svgHtml }}
        />
      )}
    </div>
  );
}
