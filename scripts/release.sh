#!/bin/sh
# nwt 发布脚本 —— 工单 #84(docs/release.md 七步的一条命令化)
#
# 用法:
#   scripts/release.sh 0.4.0                 # 全流程(tag / publish 前停下确认)
#   scripts/release.sh 0.4.0 --dry-run       # 只读:只体检,不 tag、不构建、不发布
#   scripts/release.sh 0.4.0 --from build    # 从某步续跑(check/gate/tag/build/pack/publish/verify)
#   scripts/release.sh 0.4.0 --skip-gate     # 闸门刚跑过就跳过(重跑打包/发布用)
#   scripts/release.sh 0.4.0 --yes           # 免逐步确认(脚本化场景)
#
# 契约边界:本脚本只执行 docs/release.md 已定死的契约,不新造契约。
# 资产命名 / 包内二进制名 / checksums.txt 格式,与 docs/release.md 资产表、
# install.sh 的请求名、checksums.txt 条目名四处同源 —— 改命名必须四处一起
# 改,本文件的 PKG_* 常量就是第四处。
#
# POSIX sh 约束(同 install.sh):不用 local / 数组 / [[ ]] / function;
# 函数间用固定前缀的全局变量传值(见 VER / TAG / MAJMIN / DIST)。

set -u

# ---------- 入参 ----------
VER=""            # 去掉 v 前缀的版本(0.4.0)
TAG=""            # 带 v 前缀的 tag(v0.4.0)
MAJMIN=""         # 主.次(0.4),版本显示只用这两位
DIST=""           # 产物目录 dist/<tag>
FROM="check"      # 从哪一步开始
DRY_RUN=0
SKIP_GATE=0
YES=0

STEPS="check gate tag build pack publish verify"

usage() {
  cat <<'USAGE'
nwt 发布脚本(把 docs/release.md 的七步串成一条命令)

用法:
  release.sh <版本> [--dry-run] [--from <step>] [--skip-gate] [--yes]

  <版本>   0.4.0 或 v0.4.0 都认;必须与 Cargo.toml、顶栏/splash 三处一致

选项:
  --dry-run      只读体检:版本/工作区/工具链/gh 认证全查,不 tag、不构建、不发布
  --from <step>  从某一步续跑(之前的步骤视为已跑过);step ∈ check gate tag build pack publish verify
  --skip-gate    gate 步(四项本地闸门)刚跑过就跳过;发布前请至少完整跑过一次
  --yes          免逐步确认(注意:tag 推送与 gh release create 也不再问;publish 前仍会校验 HEAD == origin/main tip)
  -h, --help     本页

步骤就是 docs/release.md「发布流程」的七步:
  check → gate → tag → build → pack → publish → verify
USAGE
}

die() { printf 'release: 错误: %s\n' "$1" >&2; exit "${2:-1}"; }
info() { printf 'release: %s\n' "$1"; }
ok() { printf 'release: ✔ %s\n' "$1"; }
warn() { printf 'release: 注意: %s\n' "$1" >&2; }

# confirm <动作人话>;--yes / --dry-run 时不问
confirm() {
  if [ "$YES" = "1" ]; then
    info "[--yes] $1"
    return 0
  fi
  if [ "$DRY_RUN" = "1" ]; then
    info "[dry-run] 将$1"
    return 0
  fi
  printf '\n即将%s。回车继续, Ctrl-C 取消: ' "$1" >&2
  read -r _ans || die "没有读到回答,取消"
}

# run <人话> <命令...>;任何非零退出都带着人话死
run() {
  _desc="$1"; shift
  info "$_desc"
  "$@" || die "失败:$_desc(可修好后用 --from 续跑,不必从头再来)"
}

# ---------- 入参解析 ----------
while [ $# -gt 0 ]; do
  case "$1" in
    --dry-run) DRY_RUN=1 ;;
    --skip-gate) SKIP_GATE=1 ;;
    --yes) YES=1 ;;
    --from)
      [ $# -ge 2 ] || die "--from 缺一个 step 名" 2
      FROM="$2"; shift ;;
    -h|--help) usage; exit 0 ;;
    -*) usage >&2; die "不认识的旗标:$1" 2 ;;
    *)
      [ -z "$VER" ] || die "版本只收一个,却给了两个:$VER 和 $1" 2
      VER="$1" ;;
  esac
  shift
done

[ -n "$VER" ] || { usage >&2; die "缺版本号" 2; }
VER="${VER#v}"
printf '%s' "$VER" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$' \
  || die "版本号要是 主.次.补 的数字形态(如 0.4.0),收到:$VER" 2
TAG="v$VER"
MAJMIN=$(printf '%s' "$VER" | sed 's/^\([0-9]*\.[0-9]*\)\..*$/\1/')
DIST="dist/$TAG"

# 资产名四处同源(docs/release.md「契约同步要求」):本文资产表 / install.sh 的
# 请求名 / checksums.txt 条目名 / 本文件的这三个常量。改命名四处一起改。
PKG_X64="nwt_${VER}_linux_x86_64.tar.gz"
PKG_ARM64="nwt_${VER}_linux_aarch64.tar.gz"
PKG_WIN="nwt_${VER}_windows_x86_64.zip"
SUMS="checksums.txt"

# step_at <step>:FROM 之前的步骤直接跳过(FROM 之前的步骤视为已跑过)
step_at() {
  _target="$1"; _seen=0
  for _s in $STEPS; do
    [ "$_s" = "$FROM" ] && _seen=1
    [ "$_s" = "$_target" ] && { [ "$_seen" = "1" ] && return 0 || return 1; }
  done
  return 1
}

# FROM 必须是真步骤名:否则一步都不跑却报「发布完成」,那是骗人
case " $STEPS " in
  *" $FROM "*) ;;
  *) die "--from 只认:$STEPS(收到:$FROM)" 2 ;;
esac

info "版本 $TAG(资产版本号 $VER,版本显示 $MAJMIN)  产物目录 $DIST"
[ "$DRY_RUN" = "1" ] && info "dry-run 模式:只体检,不落任何东西"

# ---------- 版本三处一致性(只读、便宜,任何入口都先跑) ----------
# 为什么它不长在 check 步里:--from 续跑会整段跳过 check,而版本号漂移
# (Cargo.toml / 顶栏 / splash 三处没一起改)恰恰是续跑时最容易带上的伤 ——
# 拿着 0.4.0 的旗号打 0.3.0 的包,发布面一切自检都会「绿」。
check_versions() {
  # 1) Cargo.toml 的 version(只认 [package] 段,不碰 [dependencies] 里的同名键)
  _cargo_ver=$(sed -n '/^\[package\]/,/^\[/{s/^version = "\(.*\)"/\1/p}' Cargo.toml | head -1)
  [ "$_cargo_ver" = "$VER" ] \
    || die "Cargo.toml 的 version 是 $_cargo_ver,与要发的 $VER 不一致 —— 先 bump 三处(Cargo.toml / 顶栏 / splash)再发"

  # 2) 版本显示两处硬编码(src/ui/mod.rs 顶栏与 splash;golden 帧里的同名串
  #    由 gate 的 cargo test 兜底,对不上会红)
  _disp=$(grep -c "nwt v$MAJMIN · " src/ui/mod.rs || true)
  [ "$_disp" -ge 2 ] \
    || die "src/ui/mod.rs 里「nwt v$MAJMIN · 」只有 $_disp 处(顶栏 + splash 该是 2 处)—— 版本显示漏改,gate 的 golden 也会红"
  ok "版本三处一致:Cargo.toml / 顶栏 / splash 都是 $VER($MAJMIN)"
}

# ---------- check:只读体检 ----------
step_check() {
  info "── check:版本三处一致 / 工作区 / tag 占用 / gh 认证 / 交叉工具链"
  check_versions

  # 3) 工作区干净(发布产物必须可复现;顺带挡住「带着半成品发版」)
  if [ -n "$(git status --porcelain)" ]; then
    git status --short >&2
    die "工作区有未提交改动 —— 先提交或 stash,发布要从干净树出发"
  fi
  ok "工作区干净"

  # 4) 分支提醒(不拦:允许从集成分支打 tag,但要知道自己在哪)
  _br=$(git rev-parse --abbrev-ref HEAD)
  [ "$_br" = "main" ] || warn "当前在 $_br,不是 main —— 确认这个 tag 就该从这条线出"

  # 5) tag 未占用(本地 + 远端都查)
  if git rev-parse -q --verify "refs/tags/$TAG" >/dev/null; then
    die "本地已有 tag $TAG —— 版本号往前跳,别覆盖旧 tag"
  fi
  if git ls-remote --exit-code --tags origin "refs/tags/$TAG" >/dev/null 2>&1; then
    die "远端已有 tag $TAG —— 已推过的 tag 不重推(覆盖 = 改写历史)"
  fi
  if gh release view "$TAG" >/dev/null 2>&1; then
    die "Release $TAG 已存在 —— 补资产用 gh release upload,别重建"
  fi
  ok "tag $TAG 与 Release 都还没占用"

  # 6) gh 认证(私有仓建 Release / 带凭证下载都要)
  gh auth status >/dev/null 2>&1 || die "gh 未认证 —— 先 gh auth login(私有仓的一切 GitHub 操作都要凭证)"
  ok "gh 已认证"

  # 7) 交叉工具链(缺啥给啥命令,不让人猜)
  _miss=""
  command -v tar >/dev/null || _miss="$_miss tar"
  command -v zip >/dev/null || _miss="$_miss zip"
  command -v sha256sum >/dev/null || _miss="$_miss sha256sum"
  command -v aarch64-linux-gnu-gcc >/dev/null || _miss="$_miss aarch64-linux-gnu-gcc"
  command -v x86_64-w64-mingw32-gcc >/dev/null || _miss="$_miss x86_64-w64-mingw32-gcc"
  [ -z "$_miss" ] || die "缺交叉工具链:$_miss —— 装齐再跑(sudo apt install gcc-aarch64-linux-gnu mingw-w64 zip)"
  _targets=$(rustup target list --installed 2>/dev/null || true)
  for _t in aarch64-unknown-linux-gnu x86_64-pc-windows-gnu; do
    printf '%s\n' "$_targets" | grep -q "^$_t$" \
      || die "缺 rustup target $_t —— rustup target add aarch64-unknown-linux-gnu x86_64-pc-windows-gnu"
  done
  ok "交叉工具链与 target 齐备(linux x86_64 原生 / linux aarch64 / windows x86_64)"
  info "macOS 两个包仍缺席(无 Apple 设备);本轮发布按支持级只出三平台"
}

# ---------- gate:四项本地闸门(替代原 CI 三平台) ----------
step_gate() {
  if [ "$SKIP_GATE" = "1" ]; then
    warn "按 --skip-gate 跳过闸门 —— 只该在刚跑齐过、且改的与代码无关时用"
    return 0
  fi
  info "── gate:fmt / clippy / test / build(原 CI 三平台的活,现在一台 Linux 机干)"
  run "cargo fmt --check" cargo fmt --check
  run "cargo clippy --all-targets -- -D warnings" cargo clippy --all-targets -- -D warnings
  run "cargo test --all-targets" cargo test --all-targets
  run "cargo build --release(linux x86_64 原生,打包直接用)" cargo build --release
  ok "闸门全绿"
}

# ---------- tag:打 tag 并推 ----------
step_tag() {
  info "── tag:$TAG"
  confirm "打轻量 tag $TAG 并推 origin(推错了可 git push origin :refs/tags/$TAG 删)"
  git tag "$TAG" || die "打 tag 失败"
  git push origin "$TAG" || die "推 tag 失败(本地 tag 已打,可重推或删:git tag -d $TAG)"
  ok "tag $TAG 已推"
}

# ---------- build:三平台交叉构建 ----------
step_build() {
  info "── build:三平台(linux x86_64 原生 / aarch64 交叉 / windows gnu 交叉)"
  run "linux x86_64(原生)" cargo build --release
  run "linux aarch64(交叉,gcc-aarch64 链接)" \
    env CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc \
    cargo build --release --target aarch64-unknown-linux-gnu
  # 静态 CRT:免 libgcc DLL 依赖;RUSTFLAGS 只在这一条命令上生效,不外溢
  run "windows x86_64(交叉,gnu 目标 + 静态 CRT)" \
    env CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc \
    RUSTFLAGS="-C target-feature=+crt-static" \
    cargo build --release --target x86_64-pc-windows-gnu
  ok "三个二进制就位"
}

# ---------- pack:打包 + 校验和 + 自检 ----------
step_pack() {
  info "── pack:tar/zip + sha256 + 自检"
  _stage="$DIST/stage"
  mkdir -p "$_stage" || die "建不出 $_stage"
  [ -f target/release/nwt ] || die "缺 target/release/nwt —— 先跑 build 步"
  [ -f target/aarch64-unknown-linux-gnu/release/nwt ] || die "缺 aarch64 产物 —— 先跑 build 步"
  [ -f target/x86_64-pc-windows-gnu/release/nwt.exe ] || die "缺 windows 产物 —— 先跑 build 步"

  run "linux x86_64 包" cp target/release/nwt "$_stage/nwt"
  run "linux x86_64 tar.gz" tar -C "$_stage" -czf "$DIST/$PKG_X64" nwt
  run "linux aarch64 包" cp target/aarch64-unknown-linux-gnu/release/nwt "$_stage/nwt"
  run "linux aarch64 tar.gz" tar -C "$_stage" -czf "$DIST/$PKG_ARM64" nwt
  run "windows x86_64 包" cp target/x86_64-pc-windows-gnu/release/nwt.exe "$_stage/nwt.exe"
  ( cd "$_stage" && zip -q "../$PKG_WIN" nwt.exe ) \
    || die "打 windows zip 失败"
  ok "三个包齐(命名契约见 docs/release.md 资产表)"

  # 逐包 .sha256,再汇成 checksums.txt;checksums.txt 自己也要能验
  for _a in "$PKG_X64" "$PKG_ARM64" "$PKG_WIN"; do
    [ -f "$DIST/$_a" ] || die "打包后找不到 $_a"
    ( cd "$DIST" && sha256sum "$_a" > "$_a.sha256" ) || die "算 $_a 校验和失败"
  done
  cat "$DIST"/nwt_"$VER"_*.sha256 > "$DIST/$SUMS" || die "汇 $SUMS 失败"
  ( cd "$DIST" && sha256sum -c checksums.txt ) || die "checksums.txt 自检不过 —— 包或校验和有错,别发布"
  ok "checksums.txt 自检全 OK(sha256sum -c 每一项都过)"
}

# ---------- publish:建 Release 并附资产 ----------
step_publish() {
  info "── publish:gh release create + 平台状态说明"

  # 发布面铁闸:Release 的 tag 必须指向「本地打包的这棵树」。
  # gh release create 在 tag 不存在时,默认把 tag 建在 origin/main 的 tip 上 ——
  # 所以只有 HEAD == origin/main tip 时,标签源码才等于二进制源码。
  # --from 续跑会整段跳过 check 步,这条闸就必须长在 publish 自己身上:
  # 否则在临时 worktree / 特性分支上一条 --yes 就能把没进 main 的东西发成
  # Release(#84 开发时实测踩过,已删档重来)。
  _head=$(git rev-parse HEAD)
  _remote_main=$(git ls-remote origin refs/heads/main | awk '{print $1}')
  [ -n "$_remote_main" ] || die "读不到 origin/main —— 发布面要以远端 main 为准绳,先修网络/remote"
  if [ "$_remote_main" != "$_head" ]; then
    die "HEAD($_head)不是 origin/main 的当前 tip($_remote_main)—— Release 的 tag 会指向远端 tip,不是你本地这棵树;把版本 bump 提交推上去(git push origin main)、两边一致再发"
  fi
  ok "HEAD 就是 origin/main tip($_head),标签源码 = 二进制源码"

  for _a in "$PKG_X64" "$PKG_ARM64" "$PKG_WIN" "$SUMS"; do
    [ -f "$DIST/$_a" ] || die "缺 $_a —— 先跑 pack 步"
  done

  # glibc 基线按本机实测填,不写死(老发行版用户要能看见自己会不会撞)
  _glibc=$(ldd --version 2>/dev/null | sed -n '1s/.* \([0-9][0-9]*\.[0-9][0-9]*\)$/\1/p')
  _notes="$DIST/notes-append.md"
  {
    printf '\n## 平台状态\n\n'
    printf -- '- linux x86_64:原生构建\n'
    printf -- '- linux aarch64:交叉构建(gcc-aarch64-linux-gnu)\n'
    printf -- '- windows x86_64:交叉构建(gnu 目标 + 静态 CRT,无外部 DLL 依赖)\n'
    printf -- '- macOS(x86_64 / aarch64):暂缺预编译 —— 无 Apple 设备,请源码构建\n'
    [ -n "$_glibc" ] && printf -- '- glibc 基线 %s;老发行版报 GLIBC_%s not found 时请源码构建\n' "$_glibc" "$_glibc"
    printf -- '- 校验:下载 checksums.txt 后 sha256sum -c checksums.txt\n'
  } > "$_notes"

  confirm "建 Release $TAG(tag 指向 origin/main tip $(printf '%s' "$_head" | cut -c1-7))并附四个资产 —— 建完难撤,资产可补可换,Release 本身要 gh release delete"
  run "gh release create(附资产 + checksums)" \
    gh release create "$TAG" --title "$TAG" --generate-notes \
    "$DIST/$PKG_X64" \
    "$DIST/$PKG_ARM64" \
    "$DIST/$PKG_WIN" \
    "$DIST/$SUMS"
  # --generate-notes 只吐提交笔记,平台状态得靠人补;这里自动接在后面
  _body="$DIST/notes-full.md"
  gh release view "$TAG" --json body --jq .body > "$_body" 2>/dev/null || : > "$_body"
  cat "$_notes" >> "$_body"
  run "gh release edit(补平台状态说明)" gh release edit "$TAG" --notes-file "$_body"
  ok "Release $TAG 已建,平台状态已写进 notes"
}

# ---------- verify:核对资产表 + 带凭证下载复验 ----------
step_verify() {
  info "── verify:资产表核对 + 带凭证下载全量复验"
  run "gh release view(看资产表)" gh release view "$TAG"
  _vdir="$DIST/verify"
  rm -rf "$_vdir"
  mkdir -p "$_vdir" || die "建不出 $_vdir"
  run "gh release download(带凭证全量拉回)" gh release download "$TAG" --dir "$_vdir" --clobber
  ( cd "$_vdir" && sha256sum -c checksums.txt ) || die "发布面复验不过 —— 资产与 checksums.txt 对不上,查包是不是传错了"
  ok "发布面复验全 OK:下载回来的每一项都与 checksums.txt 一致"
  printf '\n'
  info "最后一步人肉:把下面这行贴进 docs/release.md「发布记录」表并提交"
  printf '  | %s | %s | 手动(scripts/release.sh,工单 #84) | <备注> |\n' "$(date +%F)" "$TAG"
}

# ---------- 主流程 ----------
# 无论从哪一步进(--from 续跑也一样),版本三处一致性都先过一遍:只读、便宜,
# 而版本号漂移恰恰是续跑时最容易带上的伤(见 check_versions 的注释)。
check_versions

for _s in $STEPS; do
  step_at "$_s" || continue
  case "$_s" in
    check)   step_check ;;
    gate)    step_gate ;;
    tag)     step_tag ;;
    build)   step_build ;;
    pack)    step_pack ;;
    publish) step_publish ;;
    verify)  step_verify ;;
  esac
  [ "$DRY_RUN" = "1" ] && { info "dry-run 到此为止(check 之后的步骤都没执行)"; exit 0; }
done

info "发布完成:$TAG(资产与 checksums.txt 见 $DIST;发布记录那行别忘了)"
