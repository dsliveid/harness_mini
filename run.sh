#!/usr/bin/env bash
#
# harness_mini 简易启动脚本：启动 Tauri 开发模式
# 用法：./run.sh        （在项目根目录执行，或从任意目录调用均可）

set -euo pipefail

# 无论从哪里调用，都先切到脚本所在目录（= 项目根）
cd "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# ---- 1. 基础环境检查 ----
for cmd in node npm cargo; do
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "错误：未找到 $cmd，请先安装。" >&2
    case "$cmd" in
      node|npm)  echo "  Node.js: https://nodejs.org" >&2 ;;
      cargo)     echo "  Rust:    https://rustup.rs" >&2 ;;
    esac
    exit 1
  fi
done

# ---- 2. 前端依赖：node_modules 缺失时自动 npm install ----
if [ ! -d node_modules ]; then
  echo "==> 安装前端依赖（npm install）..."
  npm install
fi

# ---- 3. 启动 Tauri 开发模式（Vite 热更新 + Rust 自动重编译）----
echo "==> 启动 Tauri 开发模式（停止：Ctrl+C）..."
npm run tauri dev