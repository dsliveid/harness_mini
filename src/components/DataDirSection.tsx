import { useEffect, useState } from "react";
import { ask, open } from "@tauri-apps/plugin-dialog";
import { ipc } from "../ipc";
import { useStore } from "../store";
import type { DataStatus } from "../types";

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
    <div className="flex flex-col gap-2 text-[13px]">
      <label className="flex items-center gap-2 min-w-0">
        <span className="text-inkdim shrink-0 w-24">当前数据目录</span>
        <span className="font-mono text-[12px] flex-1 truncate" title={status.dataDir ?? ""}>
          {status.dataDir ?? "（未就绪：等待选择数据目录）"}
          {status.isCustom && <span className="text-inkdim">（已改用自定义目录）</span>}
        </span>
      </label>
      {!status.defaultWritable && (
        <div className="text-amber-400 text-[12px]">
          默认位置不可写：程序所在目录无写入权限，请选择其他数据目录，或迁移程序到可写位置。
        </div>
      )}
      <div className="flex gap-2 mt-1">
        <button
          className="px-3 py-1.5 rounded-lg bg-panel3 hover:bg-edge text-ink hover:text-ink disabled:opacity-50"
          disabled={busy}
          onClick={() => void choose()}
        >
          选择数据目录…
        </button>
        {status.isCustom && (
          <button
            className="px-3 py-1.5 rounded-lg bg-panel3 hover:bg-edge text-inkdim hover:text-ink disabled:opacity-50"
            title={`恢复到默认存放位置：${status.defaultDir}`}
            disabled={busy}
            onClick={() => void reset()}
          >
            恢复默认目录
          </button>
        )}
      </div>
      <div className="text-inkdim text-[12px]">
        更改数据目录会自动迁移现有数据（含 API Key），切换后立即生效。
      </div>
    </div>
  );
}
