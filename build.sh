#!/usr/bin/env bash
#
# harness_mini 简易打包脚本：构建前端 + 编译 Rust + 生成 NSIS 安装包
# 用法：./build.sh [--debug]

set -euo pipefail

# 无论从哪里调用，都先切到脚本所在目录（= 项目根）
cd "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# --debug：构建 debug 版（编译快，不优化；产物在 target/debug）
BUILD_PROFILE="release"
if [ "${1:-}" = "--debug" ]; then
  BUILD_PROFILE="debug"
fi

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

# ---- 3. 打包：tauri build 会先构建前端（vite build），再编译 Rust 并产出安装包 ----
echo "==> 开始打包（profile: $BUILD_PROFILE）..."
if [ "$BUILD_PROFILE" = "debug" ]; then
  npm run tauri -- build --debug
else
  npm run tauri -- build
fi

# ---- 4. 列出产物 ----
TARGET_DIR="src-tauri/target/$BUILD_PROFILE"
echo
echo "==> 构建完成，产物："
[ -f "$TARGET_DIR/harness-mini.exe" ] && echo "  可执行文件  $TARGET_DIR/harness-mini.exe"
for installer in "$TARGET_DIR"/bundle/nsis/*.exe; do
  [ -f "$installer" ] && echo "  安装包      $installer"
done