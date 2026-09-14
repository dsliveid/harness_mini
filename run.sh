#!/usr/bin/env bash
#
# harness_mini 开发启动脚本
#
# 日常开发用这个：启动 Tauri 开发模式（前端 Vite 热更新 + Rust 改动自动重编译）。
# 打包发布用 build.sh。
#
# Windows 提示：Git Bash 若不在 PATH，命令行或双击运行同目录的 run.cmd 即可。

set -euo pipefail

# 无论从哪里调用，都切到脚本所在目录（= 项目根）
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT_DIR"

SKIP_INSTALL=0
DRY_RUN=0

usage() {
  cat <<'EOF'
用法：./run.sh [选项]

启动 Tauri 开发模式。

选项：
  --skip-install   跳过前端依赖检查与安装
  --dry-run        只做环境检查并打印将要执行的命令，不真正启动
  -h, --help       显示本帮助

说明：
  - 开发模式数据存放在项目根 .dev-data/（避开 tauri dev 的文件监视，且不受
    cargo clean 影响），与 release 版的数据目录相互独立
  - 首次运行需要编译全部 Rust 依赖，耗时较长，属正常现象
  - Windows 上 Git Bash 不在 PATH 时，请使用同目录的 run.cmd
EOF
}

while [ $# -gt 0 ]; do
  case "$1" in
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

# 端口是否已被监听：用 bash 的 /dev/tcp 直接尝试连接；
# 不支持该特性的 bash 会直接失败，因此不会误报。
port_in_use() {
  (exec 3<>"/dev/tcp/127.0.0.1/$1") >/dev/null 2>&1
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

step "harness_mini 开发启动"
check_env
if [ "$SKIP_INSTALL" = "0" ]; then
  install_deps
fi

step "启动 Tauri 开发模式"
# 开发地址统一取自 tauri.conf.json（单一事实源），避免脚本里再硬编码一份而随配置漂移
dev_url="$(node -p 'require("./src-tauri/tauri.conf.json").build.devUrl' 2>/dev/null || true)"
dev_port="${dev_url##*:}"
if [ -n "$dev_url" ] && [ "$dev_port" != "$dev_url" ]; then
  info "前端地址  $dev_url（strictPort：端口被占用会直接启动失败）"
  if port_in_use "$dev_port"; then
    warn "端口 $dev_port 已被占用，tauri dev 会启动失败。请先关闭占用它的进程，或同时修改"
    warn "vite.config.ts 的 server.port 与 src-tauri/tauri.conf.json 的 build.devUrl。"
  fi
else
  info "前端地址  见 src-tauri/tauri.conf.json 的 build.devUrl（strictPort）"
fi
info "数据目录：.dev-data/  停止：在终端按 Ctrl+C"
info "首次编译 Rust 依赖较慢，请耐心等待；编译完成后会自动打开应用窗口"
run_cmd npm run tauri dev
