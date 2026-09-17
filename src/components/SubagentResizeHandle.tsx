import { useState, useEffect, useCallback } from "react";
import { useStore } from "../store";

export function SubagentResizeHandle() {
  const setSubagentPanelWidth = useStore((s) => s.setSubagentPanelWidth);
  const [isDragging, setIsDragging] = useState(false);

  const handleMouseDown = useCallback((e: React.MouseEvent) => {
    e.preventDefault();
    setIsDragging(true);
  }, []);

  const handleDoubleClick = useCallback(() => {
    setSubagentPanelWidth(480);
  }, [setSubagentPanelWidth]);

  useEffect(() => {
    if (!isDragging) return;

    const handleMouseMove = (e: MouseEvent) => {
      const newWidth = window.innerWidth - e.clientX;
      setSubagentPanelWidth(newWidth);
    };

    const handleMouseUp = () => {
      setIsDragging(false);
    };

    window.addEventListener("mousemove", handleMouseMove);
    window.addEventListener("mouseup", handleMouseUp);
    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";

    return () => {
      window.removeEventListener("mousemove", handleMouseMove);
      window.removeEventListener("mouseup", handleMouseUp);
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
    };
  }, [isDragging, setSubagentPanelWidth]);

  return (
    <div
      onMouseDown={handleMouseDown}
      onDoubleClick={handleDoubleClick}
      title="拖动调整子 Agent 面板宽度，双击恢复默认宽度"
      className={`relative w-2 cursor-col-resize shrink-0 transition-colors z-20 flex items-center justify-center select-none group ${
        isDragging ? "bg-accent" : "hover:bg-accent/40 bg-edge/40"
      }`}
    >
      <div
        className={`w-0.5 h-8 rounded-full transition-colors ${
          isDragging ? "bg-white" : "bg-edge group-hover:bg-accent"
        }`}
      />
    </div>
  );
}
