import { convertFileSrc } from "@tauri-apps/api/core";

export function toAssetUrl(path: string): string {
  if (!path) return "";
  let trimmed = path.trim();
  if (
    trimmed.startsWith("http://") ||
    trimmed.startsWith("https://") ||
    trimmed.startsWith("data:") ||
    trimmed.startsWith("asset:")
  ) {
    return trimmed;
  }
  // 剥离 file:/// 或 file:// 前缀
  if (trimmed.startsWith("file:///")) {
    trimmed = trimmed.slice(8);
  } else if (trimmed.startsWith("file://")) {
    trimmed = trimmed.slice(7);
  }
  // URL 百分号解码（如将 %20 解码为空格）
  try {
    trimmed = decodeURIComponent(trimmed);
  } catch {}
  // Windows 反斜杠规范化为正斜杠，避免 URL 路径解析错误
  trimmed = trimmed.replace(/\\/g, "/");
  return convertFileSrc(trimmed);
}

export function isVisionModel(modelName?: string | null): boolean {
  if (!modelName) return false;
  return /vl|vision|4v|omni|gpt-4o|claude-3|gemini|glm-4v|qwen-vl|qwen2.5-vl/i.test(modelName);
}

export function formatFileSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}
