#!/usr/bin/env bash
#
# harness_mini 安装包打包脚本（构建前端 + 编译 Rust + 生成可安装文件）
# 用法：./build-install.sh [--debug]
#
# 如只需快速编译可执行文件（不生成安装包），请使用 ./build.sh

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

# ---- 3. 打包：tauri build 先构建前端（vite build），再编译 Rust 并产出可安装文件 ----
echo "==> 开始打包为可安装文件（profile: $BUILD_PROFILE）..."
npm run tauri -- build "${EXTRA_ARGS[@]}"

# ---- 4. 列出产物 ----
TARGET_DIR="src-tauri/target/$BUILD_PROFILE"
echo
EXE_PATH=""
if [ -f "$TARGET_DIR/harness-mini.exe" ]; then
  EXE_PATH="$TARGET_DIR/harness-mini.exe"
elif [ -f "$TARGET_DIR/harness-mini" ]; then
  EXE_PATH="$TARGET_DIR/harness-mini"
fi

if [ -n "$EXE_PATH" ]; then
  size="$(ls -lh "$EXE_PATH" 2>/dev/null | awk '{print $5}')"
  echo "  可执行文件  $EXE_PATH  ($size)"
fi

FOUND_INSTALLER=0
for installer in "$TARGET_DIR"/bundle/nsis/*.exe "$TARGET_DIR"/bundle/msi/*.msi "$TARGET_DIR"/bundle/dmg/*.dmg "$TARGET_DIR"/bundle/deb/*.deb "$TARGET_DIR"/bundle/appimage/*.AppImage; do
  if [ -f "$installer" ]; then
    size="$(ls -lh "$installer" 2>/dev/null | awk '{print $5}')"
    echo "  安装包      $installer  ($size)"
    FOUND_INSTALLER=1
  fi
done

if [ "$FOUND_INSTALLER" = "0" ]; then
  echo "  提示：未在 $TARGET_DIR/bundle 下找到安装包产物，请检查上方打包输出。"
fi
