#!/bin/sh
# nwt(Nous Warehouse 驾驶舱)安装脚本 —— spec #10 / 工单 #12
#
# 一行安装:
#   curl -fsSL https://raw.githubusercontent.com/JoeWest9966/Warehousetui/main/install.sh | sh
#
# 行为契约(与 .github/workflows/release.yml 的命名严格同源,改一处必须同步另一处):
#   资产名   nwt_<版本>_<os>_<arch>.tar.gz
#   os       linux | macos
#   arch     x86_64 | aarch64
#   校验文件 checksums.txt(sha256sum 双列格式)
#   <版本>   = release tag 去掉 v 前缀(如 v0.1.0 -> 0.1.0)
#
# 本脚本只覆盖 unix。Windows 用户请手动下载 release 里的 zip
# (Windows 为「编译运行级」支持,见 docs/adr/0002-rust-ratatui-stack.md)。
#
# POSIX sh 约束:不用 local / 数组 / [[ ]] / function;函数间用固定前缀的
# 全局变量传值(见 SYS_OS / SYS_ARCH / VER)。

set -u

REPO="JoeWest9966/Warehousetui"
BIN_NAME="nwt"
# 默认指向 GitHub 正式源;沙盒实测/内部镜像可用同名环境变量覆盖
DOWNLOAD_BASE="${NWT_DOWNLOAD_BASE:-https://github.com/$REPO/releases/download}"
API_BASE="${NWT_API_BASE:-https://api.github.com}"

# ---------- 输出帮手:统一人话化 ----------
info() { printf 'nwt-install: %s\n' "$1"; }
err() { printf 'nwt-install: 错误: %s\n' "$1" >&2; }
# die <人话> [退出码];任何失败都走这里,保证非零退出且不留半装文件
die() { err "$1"; exit "${2:-1}"; }

usage() {
  cat <<'USAGE'
nwt 安装脚本(POSIX sh)

用法:
  install.sh [--dry-run] [--prefix DIR] [--version VER]

选项:
  --dry-run      只打印安装计划(版本/资产/目标路径),不做任何改动
  --prefix DIR   安装目录,默认 $HOME/.local/bin(也可用 PREFIX 环境变量)
  --version VER  指定版本,如 0.1.0 或 v0.1.0;默认 latest
  -h, --help     显示本帮助

说明:
  * 自动探测系统与架构(linux/macos × x86_64/aarch64),下载对应资产
  * 落盘前按 release 附带的 checksums.txt 校验 sha256,不过则不安装
  * 安装成功打印路径;若目标目录不在 PATH,给出可复制的 export 提示
USAGE
}

# ---------- 参数解析 ----------
DRY_RUN=0
PREFIX="${PREFIX:-$HOME/.local/bin}"
VERSION="latest"

while [ $# -gt 0 ]; do
  case "$1" in
    --dry-run) DRY_RUN=1 ;;
    --prefix)
      [ $# -ge 2 ] || die "--prefix 缺少目录参数" 2
      PREFIX="$2"; shift ;;
    --prefix=*) PREFIX="${1#--prefix=}" ;;
    --version)
      [ $# -ge 2 ] || die "--version 缺少版本参数" 2
      VERSION="$2"; shift ;;
    --version=*) VERSION="${1#--version=}" ;;
    -h|--help) usage; exit 0 ;;
    *) die "不认识的参数: $1(用 --help 看用法)" 2 ;;
  esac
  shift
done

# ---------- 依赖预检:缺什么说什么人话 ----------
check_deps() {
  command -v curl >/dev/null 2>&1 || die "找不到 curl,请先安装 curl"
  command -v tar >/dev/null 2>&1 || die "找不到 tar,请先安装 tar"
  command -v mktemp >/dev/null 2>&1 || die "找不到 mktemp"
  command -v awk >/dev/null 2>&1 || die "找不到 awk"
  if ! command -v sha256sum >/dev/null 2>&1 && ! command -v shasum >/dev/null 2>&1; then
    die "找不到 sha256sum / shasum,无法做落盘前校验,拒绝继续安装"
  fi
}

# ---------- 平台探测:结果写全局,避开命令子壳里 die 失效的坑 ----------
detect_os() {
  SYS_KERNEL=$(uname -s 2>/dev/null) || die "uname -s 失败"
  case "$SYS_KERNEL" in
    Linux) SYS_OS=linux ;;
    Darwin) SYS_OS=macos ;;
    *) die "不支持的系统: $SYS_KERNEL(本脚本只支持 linux/macos;Windows 请手动下载 zip)" ;;
  esac
}

detect_arch() {
  SYS_MACHINE=$(uname -m 2>/dev/null) || die "uname -m 失败"
  case "$SYS_MACHINE" in
    x86_64|amd64) SYS_ARCH=x86_64 ;;
    aarch64|arm64) SYS_ARCH=aarch64 ;;
    *) die "不支持的架构: $SYS_MACHINE(目前只发布 x86_64 / aarch64 包)" ;;
  esac
}

# latest 版本号:GitHub API 取 tag_name,不引 jq 依赖(纯 curl+grep+sed)
resolve_latest() {
  LATEST_TAG=$(curl -fsSL --retry 2 "$API_BASE/repos/$REPO/releases/latest" 2>/dev/null \
    | grep -o '"tag_name"[[:space:]]*:[[:space:]]*"[^"]*"' | head -n 1 \
    | sed 's/.*"\([^"]*\)"$/\1/')
  [ -n "$LATEST_TAG" ] || die "取不到 latest 版本号($API_BASE 无响应或仓库还没有 release)"
}

# ---------- 下载与校验 ----------
# fetch <url> <落盘路径>:失败即中止(调用方 trap 负责清理)
fetch() {
  [ -n "$1" ] && [ -n "$2" ] || die "内部错误:fetch 参数不全"
  curl -fsSL --retry 2 -o "$2" "$1" \
    || die "下载失败: $1(网络问题、版本不存在,或被限流?)"
}

# sha256_of <文件>:两种常见实现择一,输出纯哈希
sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

# verify_checksum:从 checksums.txt 抠出本资产那行比对;
# 不一致 = 拒绝安装,且此时目标路径尚未被碰过(先校验后安装)
verify_checksum() {
  EXPECTED=$(awk -v n="$ASSET_NAME" '$2 == n { print $1 }' "$WORK/checksums.txt")
  [ -n "$EXPECTED" ] || die "checksums.txt 里找不到 $ASSET_NAME 这一行,拒绝安装"
  ACTUAL=$(sha256_of "$WORK/$ASSET_NAME")
  [ "$EXPECTED" = "$ACTUAL" ] \
    || die "sha256 校验失败(期望 $EXPECTED,实得 $ACTUAL)——资产可能被篡改或下载不完整,已中止,未安装任何文件"
}

# ---------- 主流程 ----------
check_deps
detect_os
detect_arch

# 版本: latest 走 API;显式给的兼容带 v / 不带 v 两种写法
if [ "$VERSION" = "latest" ]; then
  resolve_latest
  VER="${LATEST_TAG#v}"
else
  VER="${VERSION#v}"
fi
[ -n "$VER" ] || die "版本号为空"

ASSET_NAME="nwt_${VER}_${SYS_OS}_${SYS_ARCH}.tar.gz"
ASSET_URL="$DOWNLOAD_BASE/v${VER}/$ASSET_NAME"
CK_URL="$DOWNLOAD_BASE/v${VER}/checksums.txt"

# 目标目录统一成绝对路径(相对路径按当前目录解释),去掉结尾斜杠
case "$PREFIX" in
  /*) ;;
  *) PREFIX="$PWD/$PREFIX" ;;
esac
while [ "${PREFIX%/}" != "$PREFIX" ] && [ "$PREFIX" != "/" ]; do
  PREFIX="${PREFIX%/}"
done

if [ "$DRY_RUN" = "1" ]; then
  info "安装计划(--dry-run,未做任何改动):"
  info "  版本     v$VER"
  info "  平台     $SYS_OS/$SYS_ARCH"
  info "  资产     $ASSET_URL"
  info "  校验     $CK_URL"
  info "  装到     $PREFIX/$BIN_NAME"
  exit 0
fi

WORK=$(mktemp -d 2>/dev/null) || die "创建临时目录失败"
trap 'rm -rf "$WORK"' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM HUP

info "安装 nwt v$VER ($SYS_OS/$SYS_ARCH) 到 $PREFIX"
fetch "$ASSET_URL" "$WORK/$ASSET_NAME"
fetch "$CK_URL" "$WORK/checksums.txt"
verify_checksum
info "sha256 校验通过($ACTUAL)"

tar -xzf "$WORK/$ASSET_NAME" -C "$WORK" || die "解包失败: $ASSET_NAME"
[ -f "$WORK/$BIN_NAME" ] || die "资产里找不到 $BIN_NAME,拒绝安装"

mkdir -p "$PREFIX" || die "建不了目标目录: $PREFIX(可用 --prefix 换个位置)"
cp "$WORK/$BIN_NAME" "$PREFIX/$BIN_NAME" || die "拷贝到 $PREFIX 失败(权限不够?可用 --prefix 换个位置)"
chmod 0755 "$PREFIX/$BIN_NAME" || die "设可执行权限失败: $PREFIX/$BIN_NAME"

info "完成: $PREFIX/$BIN_NAME (v$VER)"
case ":$PATH:" in
  *":$PREFIX:"*) ;;
  *) info "提示: $PREFIX 不在 PATH 里,把它加进 shell 配置即可全局使用:"
     info "  export PATH=\"$PREFIX:\$PATH\"" ;;
esac
exit 0
