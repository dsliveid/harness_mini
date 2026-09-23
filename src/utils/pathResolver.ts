import { ipc } from "../ipc";
import type { PathInspectResult } from "../types";

export interface ParsedPathInfo {
  raw: string;
  cleanPath: string;
  fileName: string;
  isExplicitDir: boolean;
  highlightLine?: number;
  highlightRange?: { start: number; end?: number };
  isPlan: boolean;
  isAbsolute: boolean;
}

const COMMON_EXTENSIONS = new Set([
  "ts", "tsx", "js", "jsx", "mjs", "cjs",
  "rs", "toml", "json", "jsonc",
  "css", "scss", "less", "html", "htm",
  "md", "markdown", "mdown", "mkdn",
  "py", "pyw", "go", "java", "kt", "kts",
  "c", "cpp", "cc", "cxx", "h", "hpp", "hh",
  "sh", "bash", "zsh", "cmd", "bat", "ps1",
  "sql", "yaml", "yml", "xml", "txt", "log",
  "svg", "png", "jpg", "jpeg", "gif", "webp", "ico",
  "env", "lock", "gitignore", "dockerignore", "editorconfig",
  "vue", "svelte", "astro", "php", "rb", "swift", "dart",
  "proto", "graphql", "gql", "prisma", "lua", "r", "scala",
  "cs", "fs", "ex", "exs", "erl", "wasm", "diff", "patch",
  "properties", "conf", "ini",
]);

const RESERVED_WORDS = new Set([
  "import", "export", "from", "function", "return", "const", "let", "var",
  "class", "interface", "type", "enum", "struct", "impl", "pub", "fn",
  "true", "false", "null", "undefined", "void", "any", "unknown", "never",
  "async", "await", "yield", "switch", "case", "default", "break", "continue",
]);

/**
 * 解析路径文本，提取路径实体、行号与范围锚点
 */
export function parsePathString(input: string): ParsedPathInfo {
  let trimmed = (input || "").trim();
  const raw = trimmed;

  // 剥离 file:/// 或 file:// 前缀
  if (trimmed.startsWith("file:///")) {
    trimmed = trimmed.slice(8);
  } else if (trimmed.startsWith("file://")) {
    trimmed = trimmed.slice(7);
  }

  try {
    trimmed = decodeURIComponent(trimmed);
  } catch {}

  let highlightLine: number | undefined;
  let highlightRange: { start: number; end?: number } | undefined;

  let pathPart = trimmed;

  // 若路径末尾包含已知扩展名且后接中文注解、括号或标点（例如 .vue（移除 或 .ts:42(修改)），剥离多余注解
  const lastSep = Math.max(pathPart.lastIndexOf("/"), pathPart.lastIndexOf("\\"));
  const dirPart = lastSep >= 0 ? pathPart.slice(0, lastSep + 1) : "";
  const filePart = lastSep >= 0 ? pathPart.slice(lastSep + 1) : pathPart;

  const extMatch = filePart.match(/\.([a-zA-Z0-9]{1,8})((?:#L\d+(?:-L?\d+)?|:\d+(?::\d+)?)*)(.*)$/);
  if (extMatch && extMatch[3]) {
    pathPart = `${dirPart}${filePart.slice(0, filePart.length - extMatch[3].length)}`;
  } else {
    // 若没有常规扩展名，但结尾紧贴括号或中文注释（例如 README（移除）或 dir/（移除））
    const commentMatch = filePart.match(/^(.*?)([（(【\[\s、，。！？；：].*)$/);
    if (commentMatch && commentMatch[1]) {
      pathPart = `${dirPart}${commentMatch[1]}`;
    }
  }

  // 1. 匹配 #L12-L24 或 #L12-24 或 #L12
  if (pathPart.includes("#L")) {
    const [p, anchor] = pathPart.split("#L");
    pathPart = p;
    if (anchor) {
      const parts = anchor.includes("-L") ? anchor.split("-L") : anchor.split("-");
      const start = parseInt(parts[0], 10);
      const end = parts[1] ? parseInt(parts[1], 10) : undefined;
      if (!isNaN(start)) {
        if (end && !isNaN(end)) {
          highlightRange = { start, end };
          highlightLine = start;
        } else {
          highlightLine = start;
        }
      }
    }
  } else {
    // 2. 匹配 :12 或 :12:5（需避开 Windows 盘符如 C:）
    const lineColMatch = pathPart.match(/:(\d+)(?::(\d+))?$/);
    if (lineColMatch && lineColMatch.index !== undefined) {
      const prefixBeforeColon = pathPart.slice(0, lineColMatch.index);
      const isDriveColon = /^[a-zA-Z]$/.test(prefixBeforeColon);
      if (!isDriveColon) {
        const start = parseInt(lineColMatch[1], 10);
        if (!isNaN(start)) {
          highlightLine = start;
        }
        pathPart = prefixBeforeColon;
      }
    }
  }

  const isExplicitDir = /[/\\]$/.test(pathPart);
  const normalizedSlashes = pathPart.replace(/\\/g, "/");
  const cleanPath = isExplicitDir ? normalizedSlashes.replace(/\/+$/, "") : normalizedSlashes;

  const fileName = cleanPath.split("/").pop() || cleanPath;
  const isPlan = cleanPath.includes(".harness") && cleanPath.includes("plans");
  const isAbsolute = /^[a-zA-Z]:\//.test(cleanPath) || cleanPath.startsWith("/");

  return {
    raw,
    cleanPath,
    fileName,
    isExplicitDir,
    highlightLine,
    highlightRange,
    isPlan,
    isAbsolute,
  };
}

const COMMON_SLASH_PAIRS = new Set([
  "and/or", "or/and", "true/false", "yes/no", "in/out", "input/output",
  "on/off", "client/server", "read/write", "import/export", "up/down",
  "left/right", "before/after", "success/failure", "pass/fail", "req/res",
  "key/value", "key/val", "get/post", "open/close", "start/stop", "push/pop",
  "encode/decode", "lock/unlock", "load/save", "sync/async",
]);

/**
 * 判断文本是否可能是路径候选（用于行内代码或纯文本的快速筛选）
 */
export function isPotentialPath(text: string): boolean {
  if (!text || typeof text !== "string") return false;
  let s = text.trim();
  if (s.length < 2 || s.length > 320) return false;

  // 包含换行、双引号、尖括号等非法路径字符则直接排除
  if (/[\r\n\t"<>|*?$]/.test(s)) return false;

  // 安全解码 URL 编码（如 %E5%B9%B4%E5%BA%A6 或 %20）
  try {
    s = decodeURIComponent(s);
  } catch {}

  // 排除中文标点符号（中文括号、书名号、中文句逗号、中文冒号等绝不属于合法路径）
  if (/[（）【】《》“”‘’、，。！？；：]/.test(s)) return false;

  // 排除未配对的英文括号与方括号
  if ((s.includes("(") && !s.includes(")")) || (s.includes("[") && !s.includes("]"))) return false;

  // 排除 HTTP / FTP / Mailto 外链
  if (/^https?:\/\//i.test(s) || /^ftp:\/\//i.test(s) || /^mailto:/i.test(s)) return false;

  // 排除常见日期格式（如 2026/09/23、2024-01-01）
  if (/^\d{4}[/\-]\d{1,2}[/\-]\d{1,2}/.test(s)) return false;

  // 排除标准 MIME 类型（如 application/json、text/html）
  if (/^(?:application|text|image|audio|video|multipart|font)\/[a-zA-Z0-9_\-+.]+$/i.test(s)) return false;

  // 排除常见含斜杠的双词并列（如 and/or、true/false）
  if (COMMON_SLASH_PAIRS.has(s.toLowerCase())) return false;

  // 排除 npm scope 包名（如 @tauri-apps/api、@vitejs/plugin-react）
  if (s.startsWith("@") && !s.startsWith("./@")) return false;

  // 排除单个字符除法或极短二元对（如 a/b、x/y）
  if (s.length <= 4 && s.includes("/")) return false;

  // 排除语义化版本号（如 v1.0.0、0.1.0）
  if (/^v?\d+\.\d+\.\d+$/i.test(s)) return false;

  // 若包含文件扩展名，检查扩展名之后是否有非法多余字符（只允许空、行号 :12 或范围 #L12-L24）
  const lastSlashIdx = Math.max(s.lastIndexOf("/"), s.lastIndexOf("\\"));
  const candidateFileName = lastSlashIdx >= 0 ? s.slice(lastSlashIdx + 1) : s;
  const extMatchWithTrail = candidateFileName.match(/\.([a-zA-Z0-9]{1,8})(.*)$/);
  if (extMatchWithTrail) {
    const trailing = extMatchWithTrail[2];
    if (trailing && !/^(?:#L\d+(?:-L?\d+)?|:\d+(?::\d+)?)*$/.test(trailing)) {
      return false;
    }
  }

  // 显式 file://
  if (/^file:\/\/\/?/i.test(s)) return true;

  // 绝对路径（Windows 盘符如 C:/... 或 Unix /...）
  if (/^[a-zA-Z]:[/\\]/.test(s) || (s.startsWith("/") && s.length > 1 && !/\s/.test(s))) return true;

  // 显式相对前缀
  if (s.startsWith("./") || s.startsWith("../") || s.startsWith(".\\") || s.startsWith("..\\")) return true;

  // 检查是否包含有效扩展名
  const extMatch = candidateFileName.match(/\.([a-zA-Z0-9]{1,8})(?::\d+(?::\d+)?)?(?:#L\d+(-L?\d+)?)?$/);
  const hasValidExt = extMatch ? COMMON_EXTENSIONS.has(extMatch[1].toLowerCase()) : false;

  // 中文字符：严防中文斜杠并列词（如 "年度/个人医德总结/科室考评人签字"、"男/女"、"是/否"）
  // 包含中文字符的路径必须拥有合法扩展名、或为绝对路径、或以 ./ ../ 开头
  if (/[\u4e00-\u9fa5]/.test(s)) {
    if (!hasValidExt) {
      return false;
    }
  }

  // 明确以斜杠结尾的代码目录候选（如 src/components/、docs/、.github/）
  if (/[/\\]$/.test(s)) {
    const clean = s.replace(/[/\\]+$/, "");
    if (/^[a-zA-Z0-9_.\-]+(?:[/\\][a-zA-Z0-9_.\-]+)*$/.test(clean)) {
      return true;
    }
  }

  // 包含合法扩展名的文件路径（如 src/App.tsx、views/login.vue、package.json）
  if (hasValidExt) {
    const withoutAnchor = s.replace(/#L\d+(-L?\d+)?$/, "").replace(/:\d+(?::\d+)?$/, "");
    const baseName = withoutAnchor.split(/[/\\]/).pop() || "";
    const baseExtMatch = baseName.match(/^([^.]+)\.([a-zA-Z0-9]{1,8})$/);
    if (baseExtMatch && !RESERVED_WORDS.has(baseExtMatch[1].toLowerCase())) {
      return true;
    }
  }

  // 纯 ASCII 相对目录路径（如 src/components、routes/api），用于行内代码中识别目录
  if (s.includes("/") || s.includes("\\")) {
    if (/\s+[/\\]\s+/.test(s)) return false;
    const withoutAnchor = s.replace(/#L\d+(-L?\d+)?$/, "").replace(/:\d+(?::\d+)?$/, "");
    if (/^[a-zA-Z0-9_.\-]+(?:[/\\][a-zA-Z0-9_.\-]+)+$/.test(withoutAnchor)) {
      return true;
    }
  }

  return false;
}

/**
 * 计算目标物理绝对路径
 */
export function resolveAbsolutePath(
  cleanPath: string,
  workspacePath?: string | null,
  currentDocPath?: string | null
): string {
  if (!cleanPath) return "";
  const normalized = cleanPath.replace(/\\/g, "/");

  // 已经是绝对路径
  if (/^[a-zA-Z]:\//.test(normalized) || normalized.startsWith("/")) {
    return normalized;
  }

  // 若处于特定文档预览中，且为 ./ 或 ../ 开头的相对路径，优先相对于该文档所在目录
  if (currentDocPath && (normalized.startsWith("./") || normalized.startsWith("../"))) {
    const cleanDoc = currentDocPath.replace(/\\/g, "/");
    const docDir = cleanDoc.substring(0, cleanDoc.lastIndexOf("/"));
    if (docDir) {
      const parts = docDir.split("/");
      const relParts = normalized.split("/");
      for (const p of relParts) {
        if (p === "." || !p) continue;
        if (p === "..") {
          parts.pop();
        } else {
          parts.push(p);
        }
      }
      return parts.join("/");
    }
  }

  // 否则相对于当前会话工作区
  if (workspacePath) {
    const cleanWs = workspacePath.trim().replace(/\\/g, "/").replace(/\/+$/, "");
    const cleanRel = normalized.replace(/^\.\//, "").replace(/^\/+/, "");
    if (cleanWs) {
      return `${cleanWs}/${cleanRel}`;
    }
  }

  return normalized;
}

// 探测缓存，避免频繁 IPC
const inspectCache = new Map<string, Promise<PathInspectResult>>();
const inspectSyncCache = new Map<string, PathInspectResult>();

/**
 * 同步获取已缓存的路径属性（如不存在则返回 undefined）
 */
export function getCachedInspectResult(path: string, workspacePath?: string | null): PathInspectResult | undefined {
  const cacheKey = `${workspacePath || ""}::${path}`;
  return inspectSyncCache.get(cacheKey);
}

/**
 * 缓存化探测路径属性（检查存在性、文件/文件夹区分）
 */
export function cachedInspectPath(path: string, workspacePath?: string | null): Promise<PathInspectResult> {
  const cacheKey = `${workspacePath || ""}::${path}`;
  const existing = inspectCache.get(cacheKey);
  if (existing) return existing;

  const promise = ipc.inspectPath(path, workspacePath)
    .then((res) => {
      inspectSyncCache.set(cacheKey, res);
      return res;
    })
    .catch(() => {
      const fallback: PathInspectResult = {
        exists: false,
        isDir: false,
        isFile: false,
        absPath: path,
        fileName: path.replace(/\\/g, "/").split("/").pop() || path,
      };
      inspectSyncCache.set(cacheKey, fallback);
      return fallback;
    });

  inspectCache.set(cacheKey, promise);
  return promise;
}

/**
 * 安全地将 Markdown 纯文本中的文件/目录路径自动包装为链接
 * 避开代码块 (```)、行内代码 (`)、已有链接与图片 ([...](...))、HTML 标签 (<...>)
 */
export function autolinkPlainPaths(content: string): string {
  if (!content || typeof content !== "string") return "";

  // 拆分为 token：代码块、行内代码、已有链接/图片、HTML 标签、以及普通文本
  const tokenRegex = /(```[\s\S]*?```|`[^`\r\n]+`|!?\[[^\]]*\]\([^)]*\)|<[^>]+>)/g;
  const parts: string[] = [];
  let lastIndex = 0;
  let match: RegExpExecArray | null;

  while ((match = tokenRegex.exec(content)) !== null) {
    if (match.index > lastIndex) {
      parts.push(processPlainText(content.slice(lastIndex, match.index)));
    }
    parts.push(match[0]); // 保持原样不处理
    lastIndex = tokenRegex.lastIndex;
  }

  if (lastIndex < content.length) {
    parts.push(processPlainText(content.slice(lastIndex)));
  }

  return parts.join("");
}

function processPlainText(text: string): string {
  if (!text.trim()) return text;

  // 匹配可能是路径的词法单元
  // 1. Windows 绝对路径（如 D:\WorkSpace\Vue\...\app.vue）
  // 2. ./ 或 ../ 显式相对路径（如 ./docs/readme.md、../src/App.tsx）
  // 3. Unix 根绝对路径（如 /var/log/nginx/access.log，前面不紧随字母数字或下划线）
  // 4. 带有合法扩展名的文件路径（如 src/components/Markdown.tsx、views/medicalethics/医德考评登记.vue:35、package.json）
  // 5. 显式以斜杠结尾的代码目录（以字母或点开头，如 src/components/、docs/、.github/），后面紧跟标点、空白或行尾
  const pathRegex = /(?:[a-zA-Z]:[/\\][^\s"<>|*?$'\`()\[\]{}（）【】《》“”‘’、，。！？；：]+(?::\d+(?::\d+)?)?(?:#L\d+(-L?\d+)?)?|(?:\.{1,2}[/\\])[^\s"<>|*?$'\`()\[\]{}（）【】《》“”‘’、，。！？；：]+(?::\d+(?::\d+)?)?(?:#L\d+(-L?\d+)?)?|(?<![a-zA-Z0-9_])\/(?:[a-zA-Z0-9_.\-]+[/\\])+[^\s"<>|*?$'\`()\[\]{}（）【】《》“”‘’、，。！？；：]+(?::\d+(?::\d+)?)?(?:#L\d+(-L?\d+)?)?|(?:[a-zA-Z0-9_.\-\u4e00-\u9fa5]+[/\\])*[a-zA-Z0-9_.\-\u4e00-\u9fa5]+\.[a-zA-Z0-9]{1,8}(?::\d+(?::\d+)?)?(?:#L\d+(-L?\d+)?)?|(?:[a-zA-Z.][a-zA-Z0-9_.\-]*[/\\])+(?=[,\s()（）<>\"'\`、，。！？；：]|$))/g;

  return text.replace(pathRegex, (rawMatch) => {
    let path = rawMatch;
    let suffix = "";

    // 双重防线：若文件名包含已知扩展名且后接多余注释（如 .vue（移除 或 .vue移除），将多余注释剥离到 suffix
    const lastSlash = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
    const dirPart = lastSlash >= 0 ? path.slice(0, lastSlash + 1) : "";
    const filePart = lastSlash >= 0 ? path.slice(lastSlash + 1) : path;

    const extMatch = filePart.match(/\.([a-zA-Z0-9]{1,8})((?:#L\d+(?:-L?\d+)?|:\d+(?::\d+)?)*)(.*)$/);
    if (extMatch && extMatch[3]) {
      const extra = extMatch[3];
      suffix = extra + suffix;
      path = `${dirPart}${filePart.slice(0, filePart.length - extra.length)}`;
    }

    // 剥离末尾的标点符号（中文逗号句号、英文标点、括号、引号等）
    while (path.length > 0) {
      const lastChar = path[path.length - 1];
      if (/[,;!?()\[\]{}<>。，！？；（）】》”’"']/.test(lastChar)) {
        suffix = lastChar + suffix;
        path = path.slice(0, -1);
      } else if (lastChar === "." && !/\.[a-zA-Z0-9]{1,8}$/.test(path)) {
        // 句末英文句号
        suffix = lastChar + suffix;
        path = path.slice(0, -1);
      } else {
        break;
      }
    }

    if (isPotentialPath(path)) {
      return `[${path}](x-path://${path})${suffix}`;
    }
    return rawMatch;
  });
}
