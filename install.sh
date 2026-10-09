#!/bin/sh
# nwt(Nous Warehouse 驾驶舱)安装脚本 —— spec #10 / 工单 #12
#
# 一行安装:
#   curl -fsSL https://raw.githubusercontent.com/JoeWest9966/Warehousetui/main/install.sh | sh
#
# 行为契约(与 docs/release.md「资产命名」表严格同源,改一处必须同步另一处):
#   资产名   nwt_<版本>_<os>_<arch>.tar.gz
#   os       linux(macos 自 v0.2.0 起暂缺预编译,见 detect_os)
#   arch     x86_64 | aarch64
#   校验文件 checksums.txt(sha256sum 双列格式)
#   <版本>   = release tag 去掉 v 前缀(如 v0.1.0 -> 0.1.0)
#
# 本脚本只覆盖 unix。Windows 用户请手动下载 release 里的 zip
# (Windows 为「编译运行级」支持,支持级在 spec 工单 #10 钉死,
#  ADR-0002 只定技术栈、不涉支持级)。
#
# POSIX sh 约束:不用 local / 数组 / [[ ]] / function;函数间用固定前缀的
# 全局变量传值(见 SYS_OS / SYS_ARCH / VER)。

set -u

REPO="JoeWest9966/Warehousetui"
BIN_NAME="nwt"
# 默认指向 GitHub 正式源;沙盒实测/内部镜像可用同名环境变量覆盖
DOWNLOAD_BASE="${NWT_DOWNLOAD_BASE:-https://github.com/$REPO/releases/download}"
API_BASE="${NWT_API_BASE:-https://api.github.com}"

# ---------- 凭证(私有仓必读)----------
# 本仓是私有仓:release 资产的 **web 下载面对 token 也返回 404**,匿名更不必说。
# 唯一可用的下载面是带 token 的 API 资产面,所以有 token 时全部下载走 API。
# GH_TOKEN / GITHUB_TOKEN 任选其一;都没有 = 匿名(公开仓够用,私有仓会明说)。
GH_TOK="${GH_TOKEN:-${GITHUB_TOKEN:-}}"

# gh_curl:统一的 curl 出口。有 token 就带 Authorization 头,没有就裸跑。
# 注意:GitHub 的 API **要求头里冒号后有空格**(「Authorization: token X」),
# 无空格会被当成未认证(私有仓 404);而带空格的值没法用「变量 + 词分裂」
# 拼进 curl 参数(会裂成三段),所以用这个函数分叉,不引 eval / 数组。
gh_curl() {
  if [ -n "$GH_TOK" ]; then
    curl -H "Authorization: token $GH_TOK" "$@"
  else
    curl "$@"
  fi
}

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

# ---------- 参数后校验:空值守卫 ----------
# --prefix="" / --prefix= 传给空串时,PREFIX 为空会让后面的相对路径补全
# 拼成 "$PWD/",二进制被装进当前目录。空值一律非零退出、说人话。
[ -n "$PREFIX" ] || die "--prefix 不能是空值(不传就用默认 \$HOME/.local/bin,或用 PREFIX 环境变量)" 2
[ -n "$VERSION" ] || die "--version 不能是空值(不传默认 latest)" 2

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
    # v0.2.0 起 macOS 预编译暂缺(无 Apple 设备,产不出两个 mac 包):
    # 明说,别让用户撞 404。见 docs/release.md「支持级与已知取舍」。
    Darwin) die "macOS 预编译包暂缺(v0.2.0 起):本机无 Apple 设备,无法产出。请源码构建:cargo build --release(只需 Rust 工具链)。Windows 同理请手动下载 zip。" ;;
    *) die "不支持的系统: $SYS_KERNEL(本脚本只支持 linux;Windows 请手动下载 zip)" ;;
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

# latest 版本号:两条路,都取不到才死
#   1) GitHub API 的 /releases/latest 取 tag_name —— 有速率限制
#   (未认证 60 次/小时),被限流或仓库暂无 release 时会失败;
#   2) 兜底:对 https://github.com/$REPO/releases/latest 发 HEAD,读 302 的
#      Location 头(/releases/tag/<tag> 或 /releases/download/<tag>/...),
#      从里面抠 tag —— 走 HTML 重定向,不吃 API 配额。
# 全程不引 jq;curl -I 不跟随重定向,只读响应头。
resolve_latest() {
  LATEST_TAG=$(gh_curl -fsSL --retry 2 "$API_BASE/repos/$REPO/releases/latest" 2>/dev/null \
    | grep -o '"tag_name"[[:space:]]*:[[:space:]]*"[^"]*"' | head -n 1 \
    | sed 's/.*"\([^"]*\)"$/\1/')

  if [ -z "$LATEST_TAG" ]; then
    info "GitHub API 没给出 latest 版本号(限流或无 release?),改用 releases/latest 重定向兜底"
    LATEST_TAG=$(curl -fsSI --retry 2 "https://github.com/$REPO/releases/latest" 2>/dev/null \
      | tr -d '\r' \
      | sed -n 's/^[Ll]ocation:[[:space:]]*//p' | head -n 1 \
      | sed 's#.*/releases/##; s#^tag/##; s#^download/##; s#/.*##')
  fi

  [ -n "$LATEST_TAG" ] || die "取不到 latest 版本号(API 与重定向两条路都失败了:网络不通、仓库没有 release,或私有仓没有凭证 —— 私有仓请先 export GH_TOKEN=… 或 gh auth login)"
}

# ---------- 下载与校验 ----------
# asset_id <资产名>:从 releases/tags 的 JSON 里抠该资产对象的 id。
# GitHub REST 的 JSON 是美化格式(每字段一行),asset 对象里 id 在 name 之前;
# tr ',' '\n' 把紧凑格式也归一成每字段一行,awk 跨行记住最后一个 id、
# 命中name 时输出 —— 不引 jq,awk 本就是依赖预检项。
asset_id() {
  [ -n "$1" ] || die "内部错误:asset_id 缺参数"
  gh_curl -fsSL --retry 2 "$API_BASE/repos/$REPO/releases/tags/v$VER" 2>/dev/null \
    | tr ',' '\n' \
    | awk -v want="$1" '
        /"id"[[:space:]]*:/ { if (match($0, /[0-9]+/)) id = substr($0, RSTART, RLENGTH) }
        /"name"[[:space:]]*:/ && index($0, "\"" want "\"") { print id; exit }
      ' \
    | grep -o '[0-9]*$' | head -n 1
}

# fetch <url> <落盘路径>:失败即中止(调用方 trap 负责清理)。
# 有 token(私有仓)时走 API 资产面;匿名(公开仓)走 web 下载面。
fetch() {
  [ -n "$1" ] && [ -n "$2" ] || die "内部错误:fetch 参数不全"
  name=${1##*/}
  if [ -n "$GH_TOK" ]; then
    aid=$(asset_id "$name")
    [ -n "$aid" ] || die "release v$VER 里找不到资产 $name(版本对不上,或资产还没上传完)"
    gh_curl -H "Accept: application/octet-stream" -fsSL --retry 2 \
      -o "$2" "$API_BASE/repos/$REPO/releases/assets/$aid" \
      || die "下载失败(API 资产面): $name(网络问题或被限流?)"
    return 0
  fi
  curl -fsSL --retry 2 -o "$2" "$1" \
    || die "下载失败: $1(网络问题、版本不存在,或被限流?私有仓请设 GH_TOKEN/GITHUB_TOKEN)"
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
  if [ -n "$GH_TOK" ]; then
    info "  凭证     用 GH_TOKEN/GITHUB_TOKEN 走 API 资产面(私有仓唯一可用面)"
  else
    info "  凭证     匿名(私有仓会失败;export GH_TOKEN=… 后重试)"
  fi
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
