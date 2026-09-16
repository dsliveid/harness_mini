import { useState } from "react";
import { ipc } from "../ipc";
import { useStore } from "../store";
import { DataDirSection } from "./DataDirSection";
import { AlertTriangle } from "./Icons";

/**
 * 程序数据目录不可写时的启动拦截：
 * 「确认」进入数据目录设置（选择其他存放目录），「取消」退出程序。
 */
export function DataDirGate() {
  const status = useStore((s) => s.dataStatus);
  const [choosing, setChoosing] = useState(false);

  if (!status?.pending) return null;

  return (
    <div className="fixed inset-0 z-[100] bg-black/70 flex items-center justify-center p-4">
      <div className="bg-panel2 border border-edge rounded-2xl w-[620px] max-w-[92vw] max-h-[88vh] overflow-y-auto p-6 shadow-2xl text-[13px]">
        <div className="font-medium text-[16px] mb-2 flex items-center gap-2 text-ink">
          <AlertTriangle size={18} className="text-amber-400 shrink-0" />
          <span>程序数据目录无法写入</span>
        </div>
        <div className="text-inkdim leading-relaxed break-all">
          目录 <span className="font-mono text-ink">{status.unwritablePath ?? status.defaultDir}</span>{" "}
          无法创建或写入，会话记录与配置无法保存到程序同级目录。请选择其他程序数据存放目录，或将程序迁移到可写位置。
        </div>
        {choosing ? (
          <>
            <div className="mt-4 border-t border-edge pt-4">
              <DataDirSection />
            </div>
            <div className="flex justify-end mt-4">
              <button
                className="text-[12px] text-inkdim hover:text-ink"
                onClick={() => void ipc.exitApp()}
              >
                退出程序
              </button>
            </div>
          </>
        ) : (
          <div className="flex justify-end gap-2 mt-5">
            <button
              className="px-4 py-1.5 rounded-lg text-inkdim hover:bg-panel3"
              onClick={() => void ipc.exitApp()}
            >
              取消
            </button>
            <button
              className="px-4 py-1.5 rounded-lg bg-accent hover:bg-blue-500 text-white"
              onClick={() => setChoosing(true)}
            >
              确认
            </button>
          </div>
        )}
      </div>
    </div>
  );
}
