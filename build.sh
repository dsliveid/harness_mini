#!/usr/bin/env bash
#
# harness_mini 打包脚本
#
# 默认产出 release 版可执行文件与 NSIS 安装包，结束时列出产物路径与体积。
# 开发启动用 run.sh。
#
# Windows 提示：Git Bash 若不在 PATH，命令行或双击运行同目录的 build.cmd 即可。

set -euo pipefail

# 无论从哪里调用，都切到脚本所在目录（= 项目根）
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT_DIR"

DEBUG_BUILD=0
NO_BUNDLE=0
CLEAN=0
SKIP_INSTALL=0
DRY_RUN=0

usage() {
  cat <<'EOF'
用法：./build.sh [选项]

打包 harness_mini（先构建前端，再编译 Rust，最后生成 NSIS 安装包）。

选项：
  --debug          构建 debug 版（不做优化，编译更快；产物在 target/debug）
  --no-bundle      只编译可执行文件，不生成安装包（快速验证 / 供其他流程复用）
  --clean          构建前执行 cargo clean（全量重编译，最慢但最干净）
  --skip-install   跳过前端依赖检查与安装
  --dry-run        只做环境检查并打印将要执行的命令，不真正构建
  -h, --help       显示本帮助

产物：
  src-tauri/target/release/harness-mini.exe          可执行文件
  src-tauri/target/release/bundle/nsis/*.exe         NSIS 安装包
  （加 --debug 时对应 target/debug 目录）

提示：
  release 版数据存放在 exe 同级的 data\ 目录（含全部会话与 API Key），
  把 exe 与 data\ 一起拷贝即可整体迁移，无需安装。
EOF
}

while [ $# -gt 0 ]; do
  case "$1" in
    --debug)        DEBUG_BUILD=1 ;;
    --no-bundle)    NO_BUNDLE=1 ;;
    --clean)        CLEAN=1 ;;
    --skip-install) SKIP_INSTALL=1 ;;
    --dry-run)      DRY_RUN=1 ;;
    -h|--help)      usage; exit 0 ;;
    *) printf '未知选项：%s\n\n' "$1" >&2; usage >&2; exit 2 ;;
  esac
  shift
done

# 非终端（如被重定向到文件）时关闭颜色，避免日志里出现乱码
if [ -t 1 ]; then
  C_BOLD=$'\033[1m'; C_DIM=$'\033[2m'; C_CYAN=$'\033[36m'
  C_YELLOW=$'\033[33m'; C_RED=$'\033[31m'; C_OFF=$'\033[0m'
else
  C_BOLD=''; C_DIM=''; C_CYAN=''; C_YELLOW=''; C_RED=''; C_OFF=''
fi

step() { printf '\n%s==> %s%s\n' "$C_BOLD" "$*" "$C_OFF"; }
info() { printf '  %s%s%s\n' "$C_DIM" "$*" "$C_OFF"; }
warn() { printf '%s警告：%s%s\n' "$C_YELLOW" "$*" "$C_OFF" >&2; }
die()  { printf '%s错误：%s%s\n' "$C_RED" "$*" "$C_OFF" >&2; exit 1; }

# dry-run 时只打印命令，便于确认脚本将要做什么
run_cmd() {
  if [ "$DRY_RUN" = "1" ]; then
    printf '  [dry-run] %s\n' "$*"
  else
    "$@"
  fi
}

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || die "未找到 $1。$2"
}

human_size() {
  if command -v du >/dev/null 2>&1; then
    du -h "$1" | cut -f1
  else
    ls -l "$1" | awk '{printf "%.1f MB", $5 / 1048576}'
  fi
}

check_env() {
  step "检查环境"
  require_cmd node  "请安装 Node.js 18 或更高版本：https://nodejs.org"
  require_cmd npm   "npm 通常随 Node.js 一起安装，请检查 Node.js 安装是否完整。"
  require_cmd cargo "请安装 Rust 工具链：https://rustup.rs"
  require_cmd rustc "请安装 Rust 工具链：https://rustup.rs"

  local node_major
  node_major="$(node -p 'process.versions.node.split(".")[0]')"
  if [ "$node_major" -lt 18 ]; then
    die "Node.js 版本过低（需要 18+），当前为 $(node --version)。"
  fi

  info "项目目录  $ROOT_DIR"
  info "node      $(node --version)"
  info "npm       $(npm --version)"
  info "rustc     $(rustc --version | awk '{print $2}')"
}

# 前端依赖是否需要重新安装：目录缺失、或 package.json / package-lock.json 有更新
needs_install() {
  if [ ! -d node_modules ]; then return 0; fi
  if [ ! -f node_modules/.package-lock.json ]; then return 0; fi
  if [ package.json -nt node_modules/.package-lock.json ]; then return 0; fi
  if [ package-lock.json -nt node_modules/.package-lock.json ]; then return 0; fi
  return 1
}

install_deps() {
  if needs_install; then
    step "安装前端依赖（npm install）"
    run_cmd npm install
  else
    step "前端依赖"
    info "已是最新，跳过安装"
  fi
}

report_artifacts() {
  local target_dir="$1" with_bundle="$2" f found=0
  step "构建产物"
  for f in "$target_dir"/*.exe; do
    if [ -f "$f" ]; then
      printf '  可执行文件  %s  (%s)\n' "${f#"$ROOT_DIR"/}" "$(human_size "$f")"
      found=1
    fi
  done
  if [ "$with_bundle" = "1" ]; then
    for f in "$target_dir"/bundle/nsis/*.exe; do
      if [ -f "$f" ]; then
        printf '  安装包      %s  (%s)\n' "${f#"$ROOT_DIR"/}" "$(human_size "$f")"
        found=1
      fi
    done
  fi
  if [ "$found" = "0" ]; then
    warn "未找到预期产物，请检查上面的构建输出。"
  fi
}

SECONDS=0

step "harness_mini 打包"
check_env
if [ "$SKIP_INSTALL" = "0" ]; then
  install_deps
fi

if [ "$DEBUG_BUILD" = "1" ]; then
  TARGET_DIR="src-tauri/target/debug"
  info "构建类型  debug（不优化，体积大、运行慢，仅用于验证）"
else
  TARGET_DIR="src-tauri/target/release"
  info "构建类型  release（strip + LTO，编译较慢）"
fi
if [ "$NO_BUNDLE" = "1" ]; then
  info "安装包    跳过（--no-bundle）"
else
  info "安装包    NSIS（仅当前用户安装，无需管理员权限）"
fi

if [ "$CLEAN" = "1" ]; then
  step "清理编译缓存（cargo clean）"
  warn "全量重编译会显著增加耗时。"
  ( cd src-tauri && run_cmd cargo clean )
fi

step "构建前端并编译 Rust（tauri build）"
info "前端产物：dist/    Rust 产物：$TARGET_DIR/"
tauri_args=(build)
if [ "$DEBUG_BUILD" = "1" ]; then tauri_args+=(--debug); fi
if [ "$NO_BUNDLE" = "1" ]; then tauri_args+=(--no-bundle); fi
run_cmd npm run tauri -- "${tauri_args[@]}"

if [ "$DRY_RUN" = "1" ]; then
  printf '\n%s[dry-run] 已跳过实际构建，未列出产物。%s\n' "$C_DIM" "$C_OFF"
  exit 0
fi

report_artifacts "$TARGET_DIR" "$([ "$NO_BUNDLE" = "1" ] && echo 0 || echo 1)"

step "完成"
info "耗时 $((SECONDS / 60)) 分 $((SECONDS % 60)) 秒"
if [ "$NO_BUNDLE" = "0" ]; then
  info "分发：安装包可直接发给用户；或把 exe 与同级 data\\ 一起拷贝做绿色版"
fi
