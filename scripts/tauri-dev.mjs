#!/usr/bin/env node
/**
 * tauri-dev.mjs —— `npm run tauri` 的包装器：让 Tauri 开发模式支持多实例端口顺延。
 *
 * 背景：vite 在端口被占时可自动顺延（strictPort: false，端口 +1 重试），但
 * tauri.conf.json 的 build.devUrl 是静态配置，CLI 没有任何机制得知 vite 的
 * 实际端口——第二实例会照旧加载第一实例的 dev server（脏跑）。
 *
 * 方案：vite 主导顺延（原生能力原样保留），本包装器只做观察者：
 *   1. 先启动 vite，从其就绪输出 `Local: http://localhost:<port>/` 解析实际端口；
 *   2. 再以 `tauri dev --config <内联JSON>` 启动，把 devUrl 覆盖为实际端口，
 *      并置空 beforeDevCommand（vite 已由本包装器拉起，避免被 CLI 重复启动）。
 * 包装器自身不做端口探测、不干预 vite 的顺延策略。
 *
 * 非 dev 子命令（build / icon ...）原样透传给 tauri CLI，行为不变。
 */

import { spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
// 依赖包的 bin 均为纯 JS 文件，直接用 node 执行，无需 shell（Windows 下 .cmd 才需要）
const VITE_JS = path.join(ROOT, "node_modules", "vite", "bin", "vite.js");
const TAURI_JS = path.join(ROOT, "node_modules", "@tauri-apps", "cli", "tauri.js");

for (const f of [VITE_JS, TAURI_JS]) {
  if (!fs.existsSync(f)) {
    console.error(`[tauri-dev] 未找到 ${f}，请先在项目根目录执行 npm install`);
    process.exit(1);
  }
}

/** 就绪输出形如 "➜  Local:   http://localhost:5601/"（vite 各大版本格式稳定） */
const PORT_RE = /Local:\s+http:\/\/(?:localhost|127\.0\.0\.1):(\d+)\//;
/** vite 打出就绪行的最长等待（正常为毫秒级，超时视为异常卡死） */
const READY_TIMEOUT_MS = 120_000;

/** 非 dev 子命令：原样透传，等待退出并保留退出码 */
function passthrough(args) {
  const child = spawn(process.execPath, [TAURI_JS, ...args], {
    cwd: ROOT,
    stdio: "inherit",
  });
  child.on("exit", (code) => process.exit(code ?? 1));
}

/**
 * dev 子命令：起 vite → 解析实际端口 → 覆盖 devUrl 启动 tauri。
 * @param {string[]} rest dev 之后的附加参数（如 --release），原样转交 tauri CLI
 */
function runDev(rest) {
  // stdout 管道收集用于解析；stderr 直接透传；收到的 stdout 同时回写终端，输出保持透明
  const vite = spawn(process.execPath, [VITE_JS], {
    cwd: ROOT,
    stdio: ["inherit", "pipe", "inherit"],
  });

  let buf = "";
  let port = null;
  let tauriStarted = false;

  const watchdog = setTimeout(() => {
    if (!port) {
      console.error("[tauri-dev] 等待 vite 就绪超时（120s），退出。");
      vite.kill();
      process.exit(1);
    }
  }, READY_TIMEOUT_MS);

  vite.on("exit", (code) => {
    clearTimeout(watchdog);
    // 已成功把接力棒交给 tauri 后，vite 的退出由 tauri 的退出回调统一收尾
    if (!port) {
      console.error(`[tauri-dev] vite 未能启动（exit=${code ?? "signal"}）。`);
      process.exit(code ?? 1);
    }
  });

  vite.stdout.on("data", (d) => {
    process.stdout.write(d); // 全量透传：端口被占顺延的提示行对用户可见
    if (port) return;
    buf += d.toString();
    const m = buf.replace(/\u001b\[[0-9;]*m/g, "").match(PORT_RE); // 先剥离 ANSI 颜色码再匹配
    if (!m) return;
    port = Number(m[1]);
    buf = "";
    launchTauri(port, rest);
  });

  /** vite 就绪后启动 tauri：覆盖 devUrl 为实际端口，并置空 beforeDevCommand */
  function launchTauri(p) {
    const overlay = JSON.stringify({
      build: { beforeDevCommand: "", devUrl: `http://localhost:${p}` },
    });
    console.log(`[tauri-dev] vite 就绪于端口 ${p} → tauri dev 将加载 http://localhost:${p}`);
    const tauri = spawn(
      process.execPath,
      [TAURI_JS, "dev", "--config", overlay, ...rest],
      { cwd: ROOT, stdio: "inherit" },
    );
    tauriStarted = true;
    // 生命周期换主：vite 原本由 tauri CLI 托管，现在由本包装器托管——
    // tauri 退出（正常结束 / Ctrl+C / 崩溃）时收掉 vite，避免孤儿进程
    tauri.on("exit", (code) => {
      clearTimeout(watchdog);
      vite.kill();
      process.exit(code ?? 0);
    });
  }

  // 兜底：包装器自身被强杀时也收掉 vite，不留孤儿（Ctrl+C 场景子进程同组自然收到信号）
  process.on("exit", () => {
    if (tauriStarted || port) vite.kill();
  });
}

const args = process.argv.slice(2);
if (args[0] === "dev") {
  runDev(args.slice(1));
} else {
  passthrough(args);
}
