import React, { Component, type ReactNode } from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { FileViewerApp } from "./file-viewer/FileViewerApp";
import { AlertTriangle } from "./components/Icons";
import "highlight.js/styles/github-dark.css";
import "./index.css";

interface ErrorBoundaryState {
  hasError: boolean;
  error: Error | null;
}

class ErrorBoundary extends Component<{ children: ReactNode }, ErrorBoundaryState> {
  constructor(props: { children: ReactNode }) {
    super(props);
    this.state = { hasError: false, error: null };
  }

  static getDerivedStateFromError(error: Error): ErrorBoundaryState {
    return { hasError: true, error };
  }

  componentDidCatch(error: Error, errorInfo: React.ErrorInfo) {
    console.error("React ErrorBoundary caught error:", error, errorInfo);
  }

  render() {
    if (this.state.hasError) {
      return (
        <div className="h-full w-full bg-[#0d1117] text-[#e6edf3] p-6 flex flex-col items-center justify-center font-sans">
          <div className="max-w-[600px] w-full bg-[#161b22] border border-[#30363d] rounded-2xl p-6 shadow-2xl">
            <div className="flex items-center gap-3 text-red-400 mb-3">
              <AlertTriangle size={22} className="shrink-0 text-red-400" />
              <h2 className="text-base font-semibold">界面渲染异常已拦截</h2>
            </div>
            <p className="text-[13px] text-[#8b949e] mb-4 leading-relaxed">
              程序检测到组件渲染错误，已防止整屏崩溃。您可以尝试点击下方按钮重新加载界面。
            </p>
            <div className="bg-[#0d1117] border border-red-500/30 rounded-xl p-3 text-[12px] font-mono text-red-300 mb-4 overflow-x-auto max-h-[200px]">
              {this.state.error?.name}: {this.state.error?.message}
              {this.state.error?.stack && (
                <div className="text-[11px] text-[#8b949e] mt-2 whitespace-pre-wrap">
                  {this.state.error.stack}
                </div>
              )}
            </div>
            <div className="flex justify-end gap-2">
              <button
                className="px-4 py-1.5 rounded-lg bg-[#21262d] hover:bg-[#30363d] text-[13px] text-[#c9d1d9] transition-colors"
                onClick={() => window.location.reload()}
              >
                刷新页面
              </button>
              <button
                className="px-4 py-1.5 rounded-lg bg-blue-600 hover:bg-blue-500 text-[13px] text-white transition-colors shadow-sm"
                onClick={() => this.setState({ hasError: false, error: null })}
              >
                尝试恢复
              </button>
            </div>
          </div>
        </div>
      );
    }
    return this.props.children;
  }
}

const isFileViewer =
  typeof window !== "undefined" &&
  new URLSearchParams(window.location.search).get("window") === "file_viewer";

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <ErrorBoundary>
      {isFileViewer ? <FileViewerApp /> : <App />}
    </ErrorBoundary>
  </React.StrictMode>
);
