#!/usr/bin/env bash
# Asig 一键安装脚本(零成本分发)。
#
# 不签名 / 不公证也能用:Gatekeeper 只拦带 `com.apple.quarantine` 隔离属性的 app。
#   - 预编译 zip(浏览器下载带 quarantine)→ 下完后 `xattr -cr` 去掉隔离属性。
#   - 源码构建(本地 rustc 生成)→ 产物天然无 quarantine,最干净。
#
# 用法:
#   curl -fsSL https://raw.githubusercontent.com/kokifish/Asig/main/scripts/install.sh | bash
#   bash scripts/install.sh                       # 安装/更新到最新版
#   bash scripts/install.sh --app-dir ~/Applications
#   bash scripts/install.sh --uninstall
#   bash scripts/install.sh -h
#
# 选项:
#   --app-dir <path>   安装目录(默认 /Applications,不可写则 ~/Applications)
#   --force            跳过覆盖确认
#   --uninstall        卸载已安装的 Asig
#   -h, --help         显示帮助
#
# 预编译产物命名约定:GitHub Release asset 固定名 `Asig.zip`(内含 Asig.app)。
# 未发布 Release 时自动回退到源码构建。

set -euo pipefail

readonly APP_NAME="Asig"
readonly REPO="kokifish/Asig"
readonly BRANCH="main"
readonly BIN_NAME="agent-light"
# GitHub 短链:直接指向 latest release 的 Asig.zip(404 时回退源码)。
readonly PKG_URL="https://github.com/${REPO}/releases/latest/download/Asig.zip"
readonly SRC_URL="https://github.com/${REPO}/archive/refs/heads/${BRANCH}.tar.gz"

APP_DIR=""
FORCE=0
UNINSTALL=0

# ---- 输出 helpers(tty 才上色,避免管道里混入 ANSI) ----
if [[ -t 1 ]]; then
    C_BOLD=$'\033[1m'; C_BLUE=$'\033[34m'; C_GREEN=$'\033[32m'
    C_YELLOW=$'\033[33m'; C_RED=$'\033[31m'; C_RESET=$'\033[0m'
else
    C_BOLD=""; C_BLUE=""; C_GREEN=""; C_YELLOW=""; C_RED=""; C_RESET=""
fi
info() { printf '%s==> %s%s\n' "${C_BOLD}${C_BLUE}" "$*" "${C_RESET}"; }
ok()   { printf '%s✓ %s%s\n' "${C_GREEN}" "$*" "${C_RESET}"; }
warn() { printf '%s⚠ %s%s\n' "${C_YELLOW}" "$*" "${C_RESET}" >&2; }
die()  { printf '%s✗ %s%s\n' "${C_RED}" "$*" "${C_RESET}" >&2; exit 1; }

usage() {
    sed -n '3,/^set -euo pipefail$/p' "$0" | sed 's/^# \{0,1\}//' | sed '/^set -euo pipefail$/d'
    exit 0
}

# ---- 参数解析 ----
while [[ $# -gt 0 ]]; do
    case "$1" in
        --app-dir)    APP_DIR="${2:-}"; [[ -n "$APP_DIR" ]] || die "--app-dir 需要一个路径参数"; shift 2;;
        --force)      FORCE=1; shift;;
        --uninstall)  UNINSTALL=1; shift;;
        -h|--help)    usage;;
        --*)          die "未知选项: $1(用 -h 查看帮助)";;
        *)            die "未知参数: $1(用 -h 查看帮助)";;
    esac
done

# ---- 前置检查 ----
[[ "$(uname -s)" == "Darwin" ]] || die "Asig 仅支持 macOS(当前: $(uname -s))"
command -v curl  >/dev/null 2>&1 || die "缺少 curl"
command -v unzip >/dev/null 2>&1 || die "缺少 unzip"

# ---- 工具 ----
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" 2>/dev/null && pwd)"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/asig-install.XXXXXX")"
cleanup() { rm -rf "$WORK"; }
trap cleanup EXIT

# cargo 在非交互 shell 可能不在 PATH(CLAUDE.md 坑②),补 source ~/.cargo/env。
ensure_cargo() {
    command -v cargo >/dev/null 2>&1 && return 0
    # shellcheck disable=SC1091
    [[ -f "$HOME/.cargo/env" ]] && . "$HOME/.cargo/env"
    command -v cargo >/dev/null 2>&1
}

# 优雅退出运行中的 Asig,避免覆盖正在执行的 binary 失败 / 留幽灵进程。
quit_running() {
    osascript -e 'tell application "Asig" to quit' >/dev/null 2>&1 || true
    pkill -f "/${BIN_NAME}" >/dev/null 2>&1 || true
}

# ---- 安装目录决策 ----
resolve_app_dir() {
    if [[ -n "$APP_DIR" ]]; then
        mkdir -p "$APP_DIR" || die "无法创建安装目录: $APP_DIR"
        return
    fi
    if [[ -w /Applications ]]; then
        APP_DIR="/Applications"
    else
        APP_DIR="$HOME/Applications"
        mkdir -p "$APP_DIR"
        warn "无 /Applications 写权限,改装到 ${APP_DIR}"
    fi
}

# ---- 卸载 ----
do_uninstall() {
    resolve_app_dir
    local target="${APP_DIR}/${APP_NAME}.app"
    if [[ ! -d "$target" ]]; then
        # 也查 ~/Applications,避免用户换了安装位置残留。
        local alt="$HOME/Applications/${APP_NAME}.app"
        if [[ -d "$alt" ]]; then target="$alt"; else
            info "未找到 ${APP_NAME}.app(已卸载?)"
            return
        fi
    fi
    quit_running
    rm -rf "$target"
    ok "已卸载: $target"
    info "用户配置保留: ~/Library/Application Support/Asig/settings.json(需手动删)"
}

# ---- 定位本地 Asig repo(脚本就在 repo 里跑时复用源码,省下载 + 复用 target 缓存) ----
local_repo_root() {
    local cand="${SCRIPT_DIR}/.."
    local cargo="$cand/Cargo.toml" make="$SCRIPT_DIR/make-app.sh"
    [[ -f "$make" && -f "$cargo" ]] || return 1
    grep -q 'crates/core' "$cargo" 2>/dev/null || return 1   # 确认是 Asig 而非同名目录
    (cd "$cand" && pwd) || return 1
}

# ---- 路径 A:下载预编译 zip ----
try_prefab() {
    local out="$WORK/Asig.zip"
    info "尝试下载预编译包(无 Release 时自动回退源码构建)…"
    if curl -fsL --retry 2 -o "$out" "$PKG_URL"; then
        ok "下载完成: Asig.zip"
        (cd "$WORK" && unzip -qq -o Asig.zip) || die "解压失败"
        BUILT_APP="$(find "$WORK" -maxdepth 2 -name "${APP_NAME}.app" -type d | head -1)"
        [[ -n "$BUILT_APP" && -d "$BUILT_APP" ]] || die "包内未找到 ${APP_NAME}.app"
        return 0
    fi
    warn "预编译包不可用(尚未发布 Release?)→ 回退源码构建"
    return 1
}

# ---- 路径 B:源码构建 ----
build_from_source() {
    ensure_cargo || die "构建需要 cargo(未找到)。装 Rust: https://rustup.rs"
    local srcroot
    if srcroot="$(local_repo_root)"; then
        info "检测到本地 Asig repo,复用源码: $srcroot"
    else
        info "下载源码(${BRANCH})…"
        local tarball="$WORK/src.tar.gz"
        curl -fsL --retry 2 -o "$tarball" "$SRC_URL" || die "源码下载失败: $SRC_URL"
        tar -xzf "$tarball" -C "$WORK" || die "源码解压失败"
        srcroot="$(find "$WORK" -maxdepth 1 -type d -name "${APP_NAME}-*" | head -1)"
        [[ -n "$srcroot" ]] || die "未找到解压后的源码目录"
    fi
    info "构建 release(首次较慢,LTO;已有 target 缓存会快很多)…"
    (cd "$srcroot" && bash scripts/make-app.sh) >/dev/null || die "构建失败(make-app.sh)"
    BUILT_APP="${srcroot}/build/${APP_NAME}.app"
    [[ -d "$BUILT_APP" ]] || die "构建产物缺失: $BUILT_APP"
    ok "构建完成"
}

# ---- 主流程 ----
main() {
    if [[ "$UNINSTALL" -eq 1 ]]; then do_uninstall; exit 0; fi

    BUILT_APP=""
    if ! try_prefab; then
        build_from_source
    fi

    # 去隔离属性:预编译 zip 解压出来的 app 带 quarantine;源码构建产物无(此处幂等无害)。
    if xattr -cr "$BUILT_APP" 2>/dev/null; then
        ok "已清除 quarantine 隔离属性(免 Gatekeeper 拦截)"
    fi

    resolve_app_dir
    local target="${APP_DIR}/${APP_NAME}.app"

    if [[ -d "$target" ]] && [[ "$FORCE" -ne 1 ]]; then
        if [[ -t 0 ]]; then
            printf '%s已存在 %s,覆盖? [y/N] %s' "${C_YELLOW}" "$target" "${C_RESET}"
            read -r ans
            [[ "$ans" =~ ^[Yy]$ ]] || die "已取消(用 --force 跳过确认)"
        else
            info "已存在旧版,直接覆盖(管道执行;用 --force 显式确认)"
        fi
    fi

    quit_running
    rm -rf "$target"
    cp -R "$BUILT_APP" "$target"
    ok "已安装: $target"

    cat <<EOF

${C_GREEN}${C_BOLD}完成${C_RESET}。启动:
  open -a ${APP_NAME}        或      open '${target}'

开机自启动:系统设置 → 通用 → 登录项 → 把 Asig 拖进列表
卸载:       bash install.sh --uninstall

${C_BOLD}状态${C_RESET}:菜单栏出现彩色灯;屏幕上方出现药丸浮窗。零配置,自动发现已装的 agent 会话。
EOF
}

main "$@"
