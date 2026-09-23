/**
 * 安全、轻量、无外部依赖的 YAML Frontmatter 探测与解析模块
 */

export interface ParsedFrontmatter {
  /** 是否包含合法的首部 Frontmatter */
  hasFrontmatter: boolean;
  /** 是否是 Harness 任务计划方案元数据 */
  isPlan: boolean;
  /** 解析后的结构化元数据键值对 */
  meta: Record<string, any>;
  /** 原始提取出的 YAML 纯文本（用于查看源码或调试） */
  rawYaml: string;
  /** 剔除 Frontmatter 后的纯净 Markdown 正文内容 */
  body: string;
}

/**
 * 判定给定的元数据是否符合 Harness 任务方案特征
 */
export function isPlanMeta(meta: Record<string, any>): boolean {
  if (!meta || typeof meta !== "object") return false;
  if (typeof meta.id === "string" && (meta.id.startsWith("plan-") || meta.id.includes("plan"))) {
    return true;
  }
  // 同时具备 session_id 与 status 字段的也是方案
  if (meta.session_id && meta.status) {
    return true;
  }
  return false;
}

/**
 * 清洗与反引号包裹的值（去除外层单/双引号）
 */
function cleanQuotedValue(val: string): string {
  let s = val.trim();
  if (
    (s.startsWith('"') && s.endsWith('"')) ||
    (s.startsWith("'") && s.endsWith("'"))
  ) {
    s = s.slice(1, -1);
    // 处理简单转义字符
    s = s.replace(/\\"/g, '"').replace(/\\'/g, "'").replace(/\\\\/g, "\\");
  }
  return s;
}

/**
 * 尝试转换纯量为基本类型（数字、布尔、去除引号的字符串）
 */
function parseScalarValue(raw: string): any {
  const trimmed = raw.trim();
  if (!trimmed) return "";

  // 1. 去除外层引号的字符串
  if (
    (trimmed.startsWith('"') && trimmed.endsWith('"')) ||
    (trimmed.startsWith("'") && trimmed.endsWith("'"))
  ) {
    return cleanQuotedValue(trimmed);
  }

  // 2. 布尔值
  const lower = trimmed.toLowerCase();
  if (lower === "true") return true;
  if (lower === "false") return false;
  if (lower === "null" || lower === "~") return null;

  // 3. 数字（整数与浮点数）
  if (/^-?\d+(\.\d+)?$/.test(trimmed)) {
    const num = Number(trimmed);
    if (!isNaN(num)) return num;
  }

  // 4. 行内数组 [a, b, c]
  if (trimmed.startsWith("[") && trimmed.endsWith("]")) {
    const inner = trimmed.slice(1, -1).trim();
    if (!inner) return [];
    return inner
      .split(",")
      .map((item) => cleanQuotedValue(item.trim()))
      .filter(Boolean);
  }

  return trimmed;
}

/**
 * 解析 Frontmatter YAML 文本为键值对
 */
export function parseYamlText(yamlStr: string): Record<string, any> {
  const result: Record<string, any> = {};
  if (!yamlStr || !yamlStr.trim()) return result;

  const lines = yamlStr.split(/\r?\n/);
  let currentArrayKey: string | null = null;
  let currentArrayValues: any[] = [];

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const trimmed = line.trim();

    // 跳过空行和纯注释
    if (!trimmed || trimmed.startsWith("#")) {
      continue;
    }

    // 检查是否是当前数组的子项： - item
    if (currentArrayKey && trimmed.startsWith("- ")) {
      const itemVal = trimmed.slice(2).trim();
      currentArrayValues.push(cleanQuotedValue(itemVal));
      continue;
    }

    // 如果不再是当前数组项，保存已累积的数组
    if (currentArrayKey) {
      result[currentArrayKey] = currentArrayValues;
      currentArrayKey = null;
      currentArrayValues = [];
    }

    // 匹配常规 key: value
    const colonIdx = line.indexOf(":");
    if (colonIdx === -1) {
      continue;
    }

    const key = line.slice(0, colonIdx).trim();
    const rawValue = line.slice(colonIdx + 1).trim();

    if (!key) continue;

    // 若值为空，可能是接下来多行数组的起始
    if (!rawValue) {
      currentArrayKey = key;
      currentArrayValues = [];
      continue;
    }

    result[key] = parseScalarValue(rawValue);
  }

  // 结尾残留的数组收尾
  if (currentArrayKey) {
    result[currentArrayKey] = currentArrayValues;
  }

  return result;
}

/**
 * 严格提取文档开头的 YAML Frontmatter 并抽离 Body
 */
export function parseFrontmatter(rawContent: string): ParsedFrontmatter {
  if (!rawContent || typeof rawContent !== "string") {
    return {
      hasFrontmatter: false,
      isPlan: false,
      meta: {},
      rawYaml: "",
      body: "",
    };
  }

  // 过滤可能的 UTF-8 BOM
  const content = rawContent.replace(/^\uFEFF/, "");

  // 严格探测首部定界符
  if (!content.startsWith("---")) {
    return {
      hasFrontmatter: false,
      isPlan: false,
      meta: {},
      rawYaml: "",
      body: content,
    };
  }

  // 匹配前部的 --- ... --- 闭合块
  const fmRegex = /^---\r?\n([\s\S]*?)\r?\n---\r?\n?/;
  const match = content.match(fmRegex);

  if (!match) {
    return {
      hasFrontmatter: false,
      isPlan: false,
      meta: {},
      rawYaml: "",
      body: content,
    };
  }

  const rawYaml = match[1];
  const body = content.slice(match[0].length);
  const meta = parseYamlText(rawYaml);
  const isPlan = isPlanMeta(meta);

  return {
    hasFrontmatter: true,
    isPlan,
    meta,
    rawYaml,
    body,
  };
}

/**
 * 友好格式化时间戳（支持 ISO 8601，转为本地可读格式 YYYY-MM-DD HH:mm:ss）
 */
export function formatTimestamp(val?: string | number | null): string {
  if (!val) return "";
  try {
    const d = new Date(val);
    if (!isNaN(d.getTime())) {
      const year = d.getFullYear();
      const month = String(d.getMonth() + 1).padStart(2, "0");
      const day = String(d.getDate()).padStart(2, "0");
      const hours = String(d.getHours()).padStart(2, "0");
      const minutes = String(d.getMinutes()).padStart(2, "0");
      const seconds = String(d.getSeconds()).padStart(2, "0");
      return `${year}-${month}-${day} ${hours}:${minutes}:${seconds}`;
    }
  } catch {}
  return String(val);
}
