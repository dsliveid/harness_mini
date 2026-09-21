import React, { useState, useEffect, useRef, memo } from "react";
import { toAssetUrl } from "../utils/image";
import { ipc } from "../ipc";

export interface SafeImageProps extends React.ImgHTMLAttributes<HTMLImageElement> {
  src?: string;
  fallbackIcon?: React.ReactNode;
}

/**
 * SafeImage: 具备 Tauri Asset 协议原生加载与 IPC Base64 降级保障的安全图片组件。
 * 1. 优先使用 Tauri convertFileSrc (asset://) 本地协议流式渲染，快速且零内存负担；
 * 2. 若因 WebView 拦截、权限策略或路径特殊转义发生加载失败 (onError)，自动调用后端 read_file_base64 兜底加载；
 * 3. 彻底避免由于协议不通或反斜杠转义导致的本地图片裂图问题。
 */
export const SafeImage = memo(function SafeImage({
  src,
  alt,
  className,
  fallbackIcon,
  onError,
  ...rest
}: SafeImageProps) {
  const [currentSrc, setCurrentSrc] = useState<string>(() => (src ? toAssetUrl(src) : ""));
  const [hasError, setHasError] = useState(false);
  const attemptedBase64Ref = useRef<string | null>(null);

  useEffect(() => {
    setCurrentSrc(src ? toAssetUrl(src) : "");
    setHasError(false);
    attemptedBase64Ref.current = null;
  }, [src]);

  const handleError = async (e: React.SyntheticEvent<HTMLImageElement, Event>) => {
    // 若尚未针对该源路径尝试过 base64 兜底读取，且不是网络图片/已是 data 链接
    const raw = src?.trim();
    if (raw && !raw.startsWith("data:") && !raw.startsWith("http://") && !raw.startsWith("https://")) {
      if (attemptedBase64Ref.current !== raw) {
        attemptedBase64Ref.current = raw;
        try {
          const b64 = await ipc.readFileBase64(raw);
          if (b64) {
            setCurrentSrc(b64);
            return;
          }
        } catch (err) {
          console.warn("SafeImage base64 fallback failed for:", raw, err);
        }
      }
    }

    setHasError(true);
    if (onError) {
      onError(e);
    }
  };

  if (hasError && fallbackIcon) {
    return <div className={className}>{fallbackIcon}</div>;
  }

  return (
    <img
      src={currentSrc}
      alt={alt}
      className={className}
      onError={handleError}
      {...rest}
    />
  );
});
