import { useEffect, useState } from "react";
import { ipc } from "../ipc";
import { useStore } from "../store";
import type { McpServerConfig, McpToolDefinition } from "../types";
import {
  Server,
  Plus,
  Trash2,
  RefreshCw,
  Play,
  Check,
  AlertCircle,
  Terminal,
  Loader2,
  Wrench,
  Sparkles,
  ChevronDown,
  ChevronRight,
  Sliders,
} from "./Icons";

export function McpSection() {
  const [servers, setServers] = useState<McpServerConfig[]>([]);
  const [loading, setLoading] = useState(false);
  const [testingId, setTestingId] = useState<string | null>(null);
  const [testResults, setTestResults] = useState<Record<string, { success: boolean; tools?: McpToolDefinition[]; error?: string }>>({});

  // 新增 / 编辑表单状态
  const [showForm, setShowForm] = useState(false);
  const [editingServer, setEditingServer] = useState<McpServerConfig | null>(null);
  const [name, setName] = useState("");
  const [command, setCommand] = useState("");
  const [argsStr, setArgsStr] = useState("");
  const [envStr, setEnvStr] = useState("");
  const [enabled, setEnabled] = useState(true);
  const [formTesting, setFormTesting] = useState(false);
  const [formTestResult, setFormTestResult] = useState<{ success: boolean; tools?: McpToolDefinition[]; error?: string } | null>(null);

  const pushToast = useStore((s) => s.pushToast);

  const loadServers = async () => {
    setLoading(true);
    try {
      const list = await ipc.listMcpServers();
      setServers(list);
    } catch (e: any) {
      pushToast(`加载 MCP 服务列表失败: ${e}`, "error");
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    void loadServers();
  }, []);

  const openAddForm = () => {
    setEditingServer(null);
    setName("");
    setCommand("");
    setArgsStr("");
    setEnvStr("");
    setEnabled(true);
    setFormTestResult(null);
    setShowForm(true);
  };

  const openEditForm = (srv: McpServerConfig) => {
    setEditingServer(srv);
    setName(srv.name);
    setCommand(srv.command);
    setArgsStr(srv.args.join(" "));
    setEnvStr(
      Object.entries(srv.env || {})
        .map(([k, v]) => `${k}=${v}`)
        .join("\n")
    );
    setEnabled(srv.enabled);
    setFormTestResult(null);
    setShowForm(true);
  };

  const parseArgs = (raw: string): string[] => {
    return raw
      .trim()
      .split(/\s+/)
      .filter((s) => s.length > 0);
  };

  const parseEnv = (raw: string): Record<string, string> => {
    const res: Record<string, string> = {};
    for (const line of raw.split("\n")) {
      const trimmed = line.trim();
      if (!trimmed || trimmed.startsWith("#")) continue;
      const eqIdx = trimmed.indexOf("=");
      if (eqIdx > 0) {
        const k = trimmed.slice(0, eqIdx).trim();
        const v = trimmed.slice(eqIdx + 1).trim();
        if (k) res[k] = v;
      }
    }
    return res;
  };

  // 测试单项服务连接
  const handleTestServer = async (srv: McpServerConfig) => {
    setTestingId(srv.id);
    try {
      const tools = await ipc.testMcpServer(srv.command, srv.args, srv.env || {});
      setTestResults((prev) => ({
        ...prev,
        [srv.id]: { success: true, tools },
      }));
      pushToast(`服务「${srv.name}」连接成功，检测到 ${tools.length} 个工具`, "success");
    } catch (err: any) {
      setTestResults((prev) => ({
        ...prev,
        [srv.id]: { success: false, error: String(err) },
      }));
      pushToast(`服务「${srv.name}」测试失败: ${err}`, "error");
    } finally {
      setTestingId(null);
    }
  };

  // 表单中的测试连接
  const handleTestForm = async () => {
    if (!command.trim()) {
      pushToast("请先填写启动命令", "warning");
      return;
    }
    setFormTesting(true);
    setFormTestResult(null);
    try {
      const tools = await ipc.testMcpServer(command.trim(), parseArgs(argsStr), parseEnv(envStr));
      setFormTestResult({ success: true, tools });
      pushToast(`连接成功，获取到 ${tools.length} 个工具`, "success");
    } catch (err: any) {
      setFormTestResult({ success: false, error: String(err) });
      pushToast(`测试失败: ${err}`, "error");
    } finally {
      setFormTesting(false);
    }
  };

  // 保存服务
  const handleSaveForm = async () => {
    if (!name.trim()) {
      pushToast("请填写服务名称", "warning");
      return;
    }
    if (!command.trim()) {
      pushToast("请填写启动命令", "warning");
      return;
    }

    const cfg: McpServerConfig = {
      id: editingServer ? editingServer.id : `mcp-${Date.now()}`,
      name: name.trim(),
      command: command.trim(),
      args: parseArgs(argsStr),
      env: parseEnv(envStr),
      enabled,
      created_at: editingServer ? editingServer.created_at : new Date().toISOString(),
      updated_at: new Date().toISOString(),
    };

    try {
      const tools = await ipc.saveMcpServer(cfg);
      pushToast(`已成功保存 MCP 服务「${cfg.name}」，已加载 ${tools.length} 个工具`, "success");
      setShowForm(false);
      void loadServers();
    } catch (err: any) {
      pushToast(`保存 MCP 服务失败: ${err}`, "error");
    }
  };

  // 删除服务
  const handleDeleteServer = async (id: string, srvName: string) => {
    if (!window.confirm(`确认删除 MCP 服务「${srvName}」吗？`)) return;
    try {
      await ipc.deleteMcpServer(id);
      pushToast(`已删除服务「${srvName}」`, "success");
      void loadServers();
    } catch (err: any) {
      pushToast(`删除失败: ${err}`, "error");
    }
  };

  // 批量重载
  const handleReloadAll = async () => {
    setLoading(true);
    try {
      await ipc.reloadMcpServers();
      pushToast("已重新载入所有启用的 MCP 服务并刷新工具注册表", "success");
      void loadServers();
    } catch (err: any) {
      pushToast(`重载失败: ${err}`, "error");
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="flex flex-col gap-5">
      {/* 头部标题与操作 */}
      <section>
        <div className="flex items-start justify-between gap-4 mb-3">
          <div className="flex-1 min-w-0">
            <div className="font-medium text-[14px] flex items-center gap-2">
              <Server size={16} className="text-purple-400" />
              <span>Model Context Protocol (MCP) 服务集成</span>
            </div>
            <div className="text-[12px] text-inkdim mt-0.5 leading-relaxed max-w-[620px]">
              MCP 是开放的工具标准协议，支持无缝接入成千上万款开源与第三方 MCP 服务器（如 Filesystem, GitHub, Postgres, Fetch, Memory 等）。免 Docker 容器，基于原生 stdio 标准进程管道即插即用。
            </div>
          </div>

          <div className="flex items-center gap-2 shrink-0 pt-0.5">
            <button
              onClick={() => void handleReloadAll()}
              disabled={loading}
              className="px-2.5 py-1.5 rounded-lg border border-edge hover:bg-panel3 text-[12px] text-inkdim hover:text-ink flex items-center gap-1.5 transition-colors cursor-pointer"
              title="重新加载所有启用的 MCP 服务进程"
            >
              <RefreshCw size={13} className={loading ? "animate-spin text-accent" : ""} />
              <span>重新加载</span>
            </button>
            <button
              onClick={openAddForm}
              className="px-3 py-1.5 rounded-lg bg-accent text-white hover:bg-accent/90 text-[12px] font-medium flex items-center gap-1.5 transition-colors cursor-pointer shadow-xs"
            >
              <Plus size={14} />
              <span>添加 MCP 服务</span>
            </button>
          </div>
        </div>

        {/* 提示条 */}
        <div className="p-3 rounded-xl border border-edge/80 bg-panel3/40 flex items-start gap-3 mt-1 mb-4 text-[12px] leading-relaxed">
          <Sparkles size={16} className="text-purple-400 shrink-0 mt-0.5" />
          <div className="flex flex-col gap-1">
            <div className="font-medium text-ink">开箱即用示例</div>
            <div className="text-inkdim leading-relaxed">
              命令: <code className="bg-panel2 px-1.5 py-0.5 rounded text-accent font-mono">npx</code> 参数: <code className="bg-panel2 px-1.5 py-0.5 rounded text-accent font-mono">-y @modelcontextprotocol/server-everything</code><br />
              支持任何支持 stdio 的 Node、Python (uvx/pipx)、Go 或本地编译二进制服务。
            </div>
          </div>
        </div>

        {/* 表单弹窗 / 展开区 */}
        {showForm && (
          <div className="p-4 rounded-xl border border-accent/40 bg-panel2/80 mb-5 flex flex-col gap-4 animate-in fade-in duration-150">
            <div className="flex items-center justify-between border-b border-edge/60 pb-2.5">
              <span className="font-medium text-[13px] text-ink">
                {editingServer ? `编辑 MCP 服务: ${editingServer.name}` : "添加新的 MCP 服务"}
              </span>
              <button
                onClick={() => setShowForm(false)}
                className="text-inkdim hover:text-ink text-[12px] px-2 py-0.5 rounded hover:bg-panel3"
              >
                取消
              </button>
            </div>

            <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
              <label className="flex flex-col gap-1">
                <span className="text-inkdim text-[12px]">服务名称</span>
                <input
                  type="text"
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  placeholder="例如: Everything MCP Server"
                  className="bg-panel border border-edge rounded-lg px-2.5 py-1.5 text-[12px] text-ink outline-none focus:border-accent"
                />
              </label>

              <label className="flex flex-col gap-1">
                <span className="text-inkdim text-[12px]">可执行命令 (Command)</span>
                <input
                  type="text"
                  value={command}
                  onChange={(e) => setCommand(e.target.value)}
                  placeholder="例如: npx, uvx, python, 或绝对路径"
                  className="bg-panel border border-edge rounded-lg px-2.5 py-1.5 text-[12px] font-mono text-ink outline-none focus:border-accent"
                />
              </label>
            </div>

            <label className="flex flex-col gap-1">
              <span className="text-inkdim text-[12px]">命令行参数 (Args，空格分隔)</span>
              <input
                type="text"
                value={argsStr}
                onChange={(e) => setArgsStr(e.target.value)}
                placeholder="例如: -y @modelcontextprotocol/server-everything"
                className="bg-panel border border-edge rounded-lg px-2.5 py-1.5 text-[12px] font-mono text-ink outline-none focus:border-accent"
              />
            </label>

            <label className="flex flex-col gap-1">
              <span className="text-inkdim text-[12px]">环境变量 (Environment Variables，每行 KEY=VALUE)</span>
              <textarea
                value={envStr}
                onChange={(e) => setEnvStr(e.target.value)}
                placeholder="API_KEY=xyz123&#10;DEBUG=1"
                rows={3}
                className="bg-panel border border-edge rounded-lg px-2.5 py-1.5 text-[12px] font-mono text-ink outline-none focus:border-accent resize-none"
              />
            </label>

            <div className="flex items-center justify-between pt-1">
              <label className="flex items-center gap-2 cursor-pointer select-none text-[12px] text-ink">
                <input
                  type="checkbox"
                  checked={enabled}
                  onChange={(e) => setEnabled(e.target.checked)}
                  className="rounded border-edge text-accent focus:ring-accent"
                />
                <span>保存后立即启用并注册工具</span>
              </label>

              <div className="flex items-center gap-2">
                <button
                  type="button"
                  onClick={() => void handleTestForm()}
                  disabled={formTesting}
                  className="px-3 py-1.5 rounded-lg border border-edge hover:bg-panel3 text-[12px] text-inkdim hover:text-ink flex items-center gap-1.5 cursor-pointer"
                >
                  {formTesting ? <Loader2 size={13} className="animate-spin text-accent" /> : <Play size={13} />}
                  <span>测试连接</span>
                </button>
                <button
                  type="button"
                  onClick={() => void handleSaveForm()}
                  className="px-4 py-1.5 rounded-lg bg-accent text-white text-[12px] font-medium hover:bg-accent/90 transition-colors cursor-pointer shadow-xs"
                >
                  保存配置
                </button>
              </div>
            </div>

            {/* 测试结果回显 */}
            {formTestResult && (
              <div
                className={`p-3 rounded-lg border text-[12px] leading-relaxed font-mono ${
                  formTestResult.success
                    ? "bg-emerald-500/10 border-emerald-500/30 text-emerald-300"
                    : "bg-red-500/10 border-red-500/30 text-red-300"
                }`}
              >
                {formTestResult.success ? (
                  <div>
                    <div className="font-semibold flex items-center gap-1.5 text-emerald-400 mb-1">
                      <Check size={14} />
                      <span>连接成功！发现以下可调用的 MCP 工具 ({formTestResult.tools?.length || 0} 个)：</span>
                    </div>
                    <ul className="list-disc list-inside space-y-0.5 ml-1">
                      {(formTestResult.tools || []).map((t) => (
                        <li key={t.name}>
                          <span className="font-bold text-ink">{t.name}</span>
                          {t.description && <span className="text-inkdim font-sans ml-1.5">- {t.description}</span>}
                        </li>
                      ))}
                    </ul>
                  </div>
                ) : (
                  <div className="flex items-start gap-1.5 text-red-400">
                    <AlertCircle size={14} className="shrink-0 mt-0.5" />
                    <div className="whitespace-pre-wrap break-all">{formTestResult.error}</div>
                  </div>
                )}
              </div>
            )}
          </div>
        )}

        {/* 服务列表 */}
        {servers.length === 0 ? (
          <div className="p-8 border border-dashed border-edge rounded-xl text-center text-inkdim text-[13px]">
            暂无已配置的 MCP 服务。点击上方「添加 MCP 服务」接入第一个 MCP 工具库。
          </div>
        ) : (
          <div className="flex flex-col gap-3">
            {servers.map((srv) => {
              const test = testResults[srv.id];
              return (
                <div
                  key={srv.id}
                  className={`border rounded-xl p-3.5 transition-colors flex flex-col gap-2.5 ${
                    srv.enabled ? "border-edge bg-panel" : "border-edge/50 bg-panel/40 opacity-75"
                  }`}
                >
                  <div className="flex items-start justify-between gap-3">
                    <div className="flex-1 min-w-0">
                      <div className="flex items-center gap-2 flex-wrap">
                        <span className="font-medium text-[13px] text-ink">{srv.name}</span>
                        <span
                          className={`text-[11px] px-2 py-0.5 rounded-full font-medium ${
                            srv.enabled
                              ? "bg-emerald-500/15 text-emerald-400 border border-emerald-500/30"
                              : "bg-panel3 text-inkdim border border-edge"
                          }`}
                        >
                          {srv.enabled ? "已启用" : "已停用"}
                        </span>
                      </div>
                      <div className="flex items-center gap-1.5 font-mono text-[11.5px] text-inkdim mt-1.5 truncate">
                        <Terminal size={12} className="shrink-0 text-accent" />
                        <span className="text-ink font-semibold">{srv.command}</span>
                        <span>{srv.args.join(" ")}</span>
                      </div>
                    </div>

                    <div className="flex items-center gap-1.5 shrink-0 pt-0.5">
                      <button
                        onClick={() => void handleTestServer(srv)}
                        disabled={testingId === srv.id}
                        className="px-2.5 py-1 rounded-lg border border-edge hover:bg-panel3 text-[11.5px] text-inkdim hover:text-ink flex items-center gap-1 transition-colors cursor-pointer"
                        title="测试该 MCP 服务的运行与工具清单"
                      >
                        {testingId === srv.id ? (
                          <Loader2 size={12} className="animate-spin text-accent" />
                        ) : (
                          <Play size={12} />
                        )}
                        <span>测试</span>
                      </button>
                      <button
                        onClick={() => openEditForm(srv)}
                        className="px-2.5 py-1 rounded-lg border border-edge hover:bg-panel3 text-[11.5px] text-inkdim hover:text-ink transition-colors cursor-pointer"
                      >
                        编辑
                      </button>
                      <button
                        onClick={() => void handleDeleteServer(srv.id, srv.name)}
                        className="px-2.5 py-1 rounded-lg border border-red-500/30 hover:bg-red-500/10 text-[11.5px] text-red-400 transition-colors cursor-pointer"
                      >
                        删除
                      </button>
                    </div>
                  </div>

                  {/* 测试结果回显 */}
                  {test && (
                    <div
                      className={`p-2.5 rounded-lg border text-[11.5px] leading-relaxed font-mono ${
                        test.success
                          ? "bg-emerald-500/10 border-emerald-500/30 text-emerald-300"
                          : "bg-red-500/10 border-red-500/30 text-red-300"
                      }`}
                    >
                      {test.success ? (
                        <div>
                          <span className="font-semibold text-emerald-400">
                            已在线识别 {test.tools?.length || 0} 个工具：
                          </span>{" "}
                          {(test.tools || []).map((t) => t.name).join(", ")}
                        </div>
                      ) : (
                        <div className="text-red-400">{test.error}</div>
                      )}
                    </div>
                  )}
                </div>
              );
            })}
          </div>
        )}
      </section>
    </div>
  );
}
