import { useEffect, useState } from "react";
import { ask, open } from "@tauri-apps/plugin-dialog";
import { ipc } from "../ipc";
import { useStore } from "../store";
import type { DataStatus } from "../types";

import { Database, FolderOpen, RotateCcw } from "./Icons";

/**
 * 数据目录设置区块：设置弹窗与启动拦截对话框共用。
 * 显示当前生效目录（支持选择自定义目录 / 恢复默认），切换时确认并自动迁移数据，
 * 完成后整页刷新即生效（后端热切换数据库连接，进程不重启）。
 */
export function DataDirSection() {
  const pushToast = useStore((s) => s.pushToast);
  const [status, setStatus] = useState<DataStatus | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    ipc
      .getDataStatus()
      .then(setStatus)
      .catch((e) => pushToast(String(e)));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 确认 → 执行（迁移 + 热切换）→ 整页刷新加载新目录数据
  const confirmAndRun = async (tipTarget: string, run: () => Promise<void>) => {
    const ok = await ask(
      `将把程序数据（含 API Key）自动迁移到：\n${tipTarget}\n\n迁移完成后立即生效，无需重启程序；原目录数据将保留作为备份。`,
      { title: "切换数据目录", kind: "info", okLabel: "确认切换", cancelLabel: "取消" }
    );
    if (!ok) return;
    setBusy(true);
    try {
      await run();
      window.location.reload();
    } catch (e) {
      pushToast(String(e));
      setBusy(false);
    }
  };

  const choose = async () => {
    if (busy || !status) return;
    const sel = await open({ directory: true, multiple: false, title: "选择程序数据存放目录" });
    if (typeof sel !== "string" || !sel) return;
    await confirmAndRun(sel, () => ipc.setDataDir(sel));
  };

  const reset = async () => {
    if (busy || !status) return;
    await confirmAndRun(status.defaultDir, () => ipc.resetDataDir());
  };

  if (!status) return null;

  return (
    <div className="flex flex-col gap-2.5 text-[13px]">
      <label className="flex items-center gap-2 min-w-0 bg-panel border border-edge rounded-lg p-2.5">
        <Database size={15} className="text-inkdim shrink-0" />
        <span className="text-inkdim shrink-0">当前数据目录:</span>
        <span className="font-mono text-[12px] flex-1 truncate text-ink" title={status.dataDir ?? ""}>
          {status.dataDir ?? "（未就绪：等待选择数据目录）"}
          {status.isCustom && <span className="text-inkdim ml-1">（已改用自定义目录）</span>}
        </span>
      </label>
      {!status.defaultWritable && (
        <div className="text-amber-400 text-[12px]">
          默认位置不可写：程序所在目录无写入权限，请选择其他数据目录，或迁移程序到可写位置。
        </div>
      )}
      <div className="flex gap-2 mt-1">
        <button
          className="px-3 py-1.5 rounded-lg bg-panel3 hover:bg-edge text-ink hover:text-ink disabled:opacity-50 flex items-center gap-1.5 text-[12px] transition-colors"
          disabled={busy}
          onClick={() => void choose()}
        >
          <FolderOpen size={13} />
          <span>选择数据目录…</span>
        </button>
        {status.isCustom && (
          <button
            className="px-3 py-1.5 rounded-lg bg-panel3 hover:bg-edge text-inkdim hover:text-ink disabled:opacity-50 flex items-center gap-1.5 text-[12px] transition-colors"
            title={`恢复到默认存放位置：${status.defaultDir}`}
            disabled={busy}
            onClick={() => void reset()}
          >
            <RotateCcw size={13} />
            <span>恢复默认目录</span>
          </button>
        )}
      </div>
      <div className="text-inkdim text-[12px]">
        更改数据目录会自动迁移现有数据（含 API Key），切换后立即生效。
      </div>
      <StorageModeSection />
    </div>
  );
}

/**
 * 存储模式开关（三库分离回滚通路）：
 * hybrid（默认）：消息/工具/任务按会话分库，无分库数据回落主库；
 * single：读写一律走主库（旧单库模式，用于排查问题时一键回滚）；
 * split：同 hybrid 路由，日志库（按月轮转）作为审计读取的权威源。
 * 切换需显式确认；有任务运行时后端会拒绝。
 */
function StorageModeSection() {
  const pushToast = useStore((s) => s.pushToast);
  const [mode, setMode] = useState<string>("hybrid");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    ipc
      .getStorageMode()
      .then(setMode)
      .catch(() => {});
  }, []);

  const change = async (next: string) => {
    if (busy || next === mode) return;
    const label: Record<string, string> = {
      hybrid: "分库模式（默认）：会话数据独立分库、日志按月轮转",
      single: "单库模式：读写全部走主库 harness_mini.db（回滚通路）",
      split: "完全分库：同分库模式，且日志库作为审计读取权威源",
    };
    const ok = await ask(`确认切换存储模式为「${label[next] ?? next}」？\n\n切换立即生效，可随时切回。`, {
      title: "切换存储模式",
      kind: "info",
      okLabel: "确认切换",
      cancelLabel: "取消",
    });
    if (!ok) return;
    setBusy(true);
    try {
      await ipc.setStorageMode(next);
      setMode(next);
      pushToast("存储模式已切换");
    } catch (e) {
      pushToast(String(e));
    } finally {
      setBusy(false);
    }
  };

  const options: { key: string; label: string }[] = [
    { key: "single", label: "单库" },
    { key: "hybrid", label: "分库（推荐）" },
    { key: "split", label: "完全分库" },
  ];

  return (
    <div className="flex items-center gap-2 mt-1 text-[12px]">
      <span className="text-inkdim shrink-0">存储模式:</span>
      <div className="flex gap-1.5">
        {options.map((o) => (
          <button
            key={o.key}
            disabled={busy}
            onClick={() => void change(o.key)}
            className={`px-2.5 py-1 rounded-lg border text-[12px] transition-colors disabled:opacity-50 ${
              mode === o.key
                ? "bg-panel3 border-edge text-ink"
                : "bg-panel border-edge text-inkdim hover:text-ink"
            }`}
            title={`切换为${o.label}`}
          >
            {o.label}
          </button>
        ))}
      </div>
    </div>
  );
}
