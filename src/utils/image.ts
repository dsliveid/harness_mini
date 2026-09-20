import { convertFileSrc } from "@tauri-apps/api/core";

export function toAssetUrl(path: string): string {
  if (!path) return "";
  const trimmed = path.trim();
  if (
    trimmed.startsWith("http://") ||
    trimmed.startsWith("https://") ||
    trimmed.startsWith("data:") ||
    trimmed.startsWith("asset:")
  ) {
    return trimmed;
  }
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
