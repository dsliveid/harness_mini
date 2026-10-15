#!/usr/bin/env bash
#
# harness_mini 编译可执行文件脚本（只构建可执行文件，不生成安装包）
# 用法：./build.sh [--debug]
#
# 如需打包为可安装文件（NSIS 安装包），请使用 ./build-install.sh

set -euo pipefail

# 无论从哪里调用，都先切到脚本所在目录（= 项目根）
cd "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

BUILD_PROFILE="release"
EXTRA_ARGS=()

for arg in "$@"; do
  case "$arg" in
    --debug)
      BUILD_PROFILE="debug"
      EXTRA_ARGS+=("--debug")
      ;;
    *)
      EXTRA_ARGS+=("$arg")
      ;;
  esac
done

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

# ---- 3. 编译：tauri build --no-bundle 构建前端与 Rust 可执行文件，跳过安装包生成 ----
echo "==> 开始编译可执行文件（profile: $BUILD_PROFILE, --no-bundle）..."
npm run tauri -- build --no-bundle "${EXTRA_ARGS[@]}"

# ---- 4. 列出产物 ----
TARGET_DIR="src-tauri/target/$BUILD_PROFILE"
echo
echo "==> 构建完成，产物："
FOUND=0
EXE_PATH=""
if [ -f "$TARGET_DIR/harness-mini.exe" ]; then
  EXE_PATH="$TARGET_DIR/harness-mini.exe"
elif [ -f "$TARGET_DIR/harness-mini" ]; then
  EXE_PATH="$TARGET_DIR/harness-mini"
fi

if [ -n "$EXE_PATH" ]; then
  size="$(ls -lh "$EXE_PATH" 2>/dev/null | awk '{print $5}')"
  echo "  可执行文件  $EXE_PATH  ($size)"
  FOUND=1
fi

if [ "$FOUND" = "0" ]; then
  echo "  提示：未在 $TARGET_DIR 下找到可执行文件，请检查上方编译输出。"
else
  echo
  echo "  说明：如需生成安装包（.exe 安装程序），请运行 ./build-install.sh"
fi