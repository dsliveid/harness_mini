import { SubprocessBranchTree } from "./SubprocessBranchTree";
import type { ToolEvent } from "../types";

/**
 * 兼容性组件：将单子进程卡片重定向为横向树形展示组件
 */
export function SubprocessCard({ ev, subprocessId }: { ev?: ToolEvent; subprocessId?: string }) {
  if (subprocessId) {
    return <SubprocessBranchTree singleSubId={subprocessId} />;
  }
  if (ev) {
    return <SubprocessBranchTree event={ev} />;
  }
  return null;
}
