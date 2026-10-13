# 发布矩阵

nwt 的发布面:**本地构建 → 打 `v*` tag → `gh release create` 手动建 Release(附 sha256 校验和)**。本仓**不跑 GitHub Actions**(2026-10-09 起:账号级计费挡住全部 job,且发布矩阵里的 macOS 组合需要 Apple 设备,Action 公有 runner 也随计费一起不可用;业主裁定不走 Action、不付钱,发布改全手动)。本文是**发布者文档**;用户侧安装说明在 [README 安装段](../README.md#安装)。

> 状态:v0.1.0 由 Action 发布(历史记录见底部);**v0.2.0、v0.3.0 均为手动发布**,流程即下文。

**下面七步已由 [`scripts/release.sh`](../scripts/release.sh) 一条命令跑完**(工单 #84):`scripts/release.sh 0.4.0` 顺序走 check → gate → tag → build → pack → publish → verify,`--dry-run` 只体检、`--from <step>` 失败续跑、`--skip-gate` 跳过闸门。脚本**只执行本文定死的契约,不新造契约**;本文仍是事实源,两边对不上以本文为准、并去改脚本。

## 发布流程(手动)

1. **本地闸门**(开发机上跑齐四项,替代原 CI 三平台):

   ```bash
   cargo fmt --check
   cargo clippy --all-targets -- -D warnings
   cargo test --all-targets
   cargo build --release
   ```

   注意:跨平台只有一台 Linux 开发机,macOS/Windows 专属回归靠历史教训背书(见 README「开发」节列的两条)。
2. **确认版本号**:`Cargo.toml` 的 `version` 与要打的 tag 一致。tag 与 Cargo 版本的一致性靠这一步人工核对,不引入构建期注入。(版本显示另有顶栏/splash 两处硬编码,bump 时三处一起改。)
3. **打 tag 并推**:

   ```bash
   git tag v0.2.0
   git push origin v0.2.0
   ```

4. **本地构建三平台**(交叉编译细节见「本地复现」节;v0.2.0 起 macOS 缺席,见「支持级与已知取舍」):

   ```bash
   rustup target add aarch64-unknown-linux-gnu x86_64-pc-windows-gnu
   # 链接器:apt install gcc-aarch64-linux-gnu mingw-w64
   cargo build --release                                                    # linux x86_64(原生)
   CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc \
     cargo build --release --target aarch64-unknown-linux-gnu               # linux aarch64
   CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc \
     RUSTFLAGS="-C target-feature=+crt-static" \
     cargo build --release --target x86_64-pc-windows-gnu                   # windows x86_64
   ```

5. **打包 + 校验和**(命名契约见下表;包内只有二进制本体):

   ```bash
   mkdir -p stage && cp target/release/nwt stage/nwt
   tar -C stage -czf nwt_0.2.0_linux_x86_64.tar.gz nwt
   cp target/aarch64-unknown-linux-gnu/release/nwt stage/nwt
   tar -C stage -czf nwt_0.2.0_linux_aarch64.tar.gz nwt
   cp target/x86_64-pc-windows-gnu/release/nwt.exe stage/nwt.exe
   (cd stage && zip -q ../nwt_0.2.0_windows_x86_64.zip nwt.exe)
   for a in nwt_0.2.0_*.tar.gz nwt_0.2.0_*.zip; do sha256sum "$a" > "$a.sha256"; done
   cat nwt_0.2.0_*.sha256 > checksums.txt && sha256sum -c checksums.txt
   ```

6. **建 Release 并附资产**(tag 已推,直接挂上去):

   ```bash
   gh release create v0.2.0 --title v0.2.0 --generate-notes \
     nwt_0.2.0_linux_aarch64.tar.gz nwt_0.2.0_linux_x86_64.tar.gz \
     nwt_0.2.0_windows_x86_64.zip checksums.txt
   ```

   `--generate-notes` 之后按需 `gh release edit v0.2.0 --notes-file` 补平台状态说明(macOS 缺席必须写,别让用户撞 404)。
7. **核对 + 记账**:`gh release view v0.2.0` 对资产表;带凭证下载全部资产跑 `sha256sum -c`;在「发布记录」登记一行。

脚本对应关系:`scripts/release.sh <版本>` 的 check(第 1 步前置 + 版本三处一致 + tag 未占用 + 工具链)/ gate(第 1 步)/ tag(第 3 步)/ build(第 4 步)/ pack(第 5 步)/ publish(第 6 步)/ verify(第 7 步)。publish 前有一条铁闸:**HEAD 必须等于 origin/main 的当前 tip**——`gh release create` 在 tag 不存在时把 tag 建在远端 main tip 上,不等就说明「标签源码 ≠ 二进制源码」,脚本直接拒发。

## 本地闸门:pre-push 钩子(CI 空位的日常补位)

原 CI 的活日常由 `githooks/pre-push` 在 push 前干(启用一次,恢复默认见注释):

```bash
git config core.hooksPath githooks      # 启用
# git config --unset core.hooksPath    # 恢复 .git/hooks 默认
```

钩子跑齐 README「开发」节那四项(fmt / clippy / test / build),红了就挡住 push。已知的**环境性红**(非缺陷)用 `NWT_SKIP_GATE=1 git push ...` 逃逸:discover 测试在装有真 `nw` 的机器上会红。发布时的 `scripts/release.sh` gate 步跑同一套,两边不许分成两套口径。

## 自托管 runner(可选,未接线)

**自托管 runner 不烧 Actions 分钟、也不受 spending limit 管**——账号级计费挡的是 GitHub 托管 runner,不是这个。想在 GitHub 上也看到绿勾可以挂一台(开发机上常驻一个进程):repo → Settings → Actions → Runners → New self-hosted runner,按给出的两条命令装并跑;然后从历史恢复工作流、只留 Linux 一条:

```bash
git show 1b9eeeb:.github/workflows/ci.yml > .github/workflows/ci.yml
# 再把 runs-on 改成 [self-hosted, linux],矩阵只留 linux x86_64
```

**未接线的原因**:业主裁定不走 Action;且没注册 runner 时这些 job 不会秒失败、而是**一直排队**,比没有更吵。真要挂,先把 CI 观感需求和「常驻一个进程」的代价摆到桌面上再定。

## 资产命名(四处同源的契约)

| 平台 | 资产 | v0.2.0 状态 |
|---|---|---|
| linux x86_64 | `nwt_<版本>_linux_x86_64.tar.gz` | ✅ 原生构建 |
| linux aarch64 | `nwt_<版本>_linux_aarch64.tar.gz` | ✅ 交叉构建(gcc-aarch64) |
| macos x86_64 | `nwt_<版本>_macos_x86_64.tar.gz` | ⛔ 暂缺(无 Apple 设备) |
| macos aarch64 | `nwt_<版本>_macos_aarch64.tar.gz` | ⛔ 暂缺(无 Apple 设备) |
| windows x86_64 | `nwt_<版本>_windows_x86_64.zip`(唯一 `.zip`) | ✅ 交叉构建(gnu 目标 + 静态 CRT;v0.1.0 是 msvc,功能等价) |
| 校验和 | `checksums.txt` | ✅ |

- `<版本>` = tag 去掉 `v` 前缀(`v0.2.0` → `0.2.0`)。
- unix 包内二进制名 `nwt`;windows 包内 `nwt.exe`。
- **只出 windows x86_64**:Windows 是「编译可运行」级支持([spec #10](https://github.com/JoeWest9966/Warehousetui/issues/10) 钉死的 R3 决定;ADR-0002 只定技术栈、不涉支持级),不为它出全矩阵。
- `checksums.txt` 用 `sha256sum` 双列格式,unix 上可直接 `sha256sum -c` 验。

**契约同步要求**:本文「发布流程」的打包命令、install.sh 的请求名、`checksums.txt` 的条目名、`scripts/release.sh` 的 `PKG_*` 常量,四处同源。改命名必须四处一起改,并更新上表——这是 install.sh 能装对版本的唯一保证。

## 用户侧校验

手动下载的用户,装前自校验:

```bash
curl -fsSLO https://github.com/JoeWest9966/Warehousetui/releases/download/v0.2.0/checksums.txt
sha256sum -c checksums.txt        # 每一项都要 OK
```

install.sh 装前做同一件事;对不上就非零退出、不留半个二进制。手动下载等价于 `gh release download v0.2.0`(公开仓免凭证)。

## 本地复现(交叉构建细节)

linux aarch64 / windows x86_64 的交叉编译前提(全部免费):

```bash
rustup target add aarch64-unknown-linux-gnu x86_64-pc-windows-gnu
apt install gcc-aarch64-linux-gnu mingw-w64 zip
```

- linux aarch64:`CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc`;可用 `qemu-aarch64-static -L /usr/aarch64-linux-gnu <bin>` 冒烟(能启动即过,TUI 会报无 TTY 的错,预期)。
- windows x86_64:`CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc` + `RUSTFLAGS="-C target-feature=+crt-static"`(静态 CRT,免 libgcc DLL 依赖);产物是 PE32+ console,本仓对 Windows 的支持级就是「编译可运行」。
- macOS 两个目标**无法**在 Linux 上产出(需 Apple SDK/Mac);`cargo-zigbuild` 等实验路径不采用(链接 Apple 系统库不可靠,不为发布冒险)。

## 支持级与已知取舍

- Windows:编译运行级(spec #10 的 R3 决定),无 installer、无自动安装;手动 zip。**v0.2.0 起产物是 gnu 目标**(交叉工具链所限),与 v0.1.0 的 msvc 产物功能等价、无外部 DLL 依赖。
- **macOS 预编译暂缺(v0.2.0 起)**:无 Apple 设备,两个 mac 包产不出来;install.sh 对 Darwin 直接给出「请源码构建」的明路(不撞 404)。等有 Mac 再补,命名契约不变。
- 无代码签名/公证(macOS notarization、Windows Authenticode):V1 不做,靠 sha256 校验兜底。
- 无自更新命令(`nwt self-update`):保持 ADR-0003 精神,面板不自改。
- **公开仓匿名安装即用**:`curl | sh` 一行装开箱即用;未认证 API 有 60 次/小时限流,install.sh 带 `GH_TOKEN`/`GITHUB_TOKEN` 时自动改走 API 资产面(既避限流、也兼容私有仓场景)。
- **glibc 基线 2.41(v0.2.0 linux 包)**:v0.1.0 的 linux 包构建于 ubuntu-24.04(glibc 2.39);v0.2.0 起在本机(Debian 13)构建,基线升到 2.41,老发行版运行会报 `GLIBC_2.41 not found`。需要更宽兼容时用更老的构建机重打这两个 linux 包(命名不变)。
- 包管理器分发(Homebrew / Scoop / AUR):后续票据。
- **三平台回归已回 CI**:公库公开后 GitHub Actions 免费额度恢复,ci.yml 在 ubuntu / macos / windows 三平台跑 fmt/clippy/test/build;曾被 macOS/Windows 专属问题咬过两次(建议命令测试的 OS 临时目录长度依赖;见 README「开发」节),CI 回归后这类回归会被当场抓住。

## 发布记录

| 日期 | tag | 发布方式 | 备注 |
|---|---|---|---|
| 2026-10-06 | v0.1.0 | Action(彩排 run [37443153354](https://github.com/JoeWest9966/Warehousetui/actions/runs/37443153354)) | 5 组合全绿;修复两条彩排问题(macOS x86_64 改交叉编译、runs-on 重复定义)后正式成发布 |
| 2026-10-09 | v0.2.0 | **手动**(`gh release create`,[release 页](https://github.com/JoeWest9966/Warehousetui/releases/tag/v0.2.0)) | 真机首跑实证(docs/real-first-run.md)后发布;Action 因账号级计费全部无法启动(run [37880214287](https://github.com/JoeWest9966/Warehousetui/actions/runs/37880214287) 五 job 无 runner),改为本地交叉构建 3 平台手动发布;macOS 暂缺、windows 换 gnu 目标、glibc 基线 2.41,均记于「支持级与已知取舍」 |
| 2026-10-12 | v0.3.0 | **手动**(`gh release create`,[release 页](https://github.com/JoeWest9966/Warehousetui/releases/tag/v0.3.0)) | 第 8/9/10 轮(#64→#74→#79 并入 main,62 提交 / 22 票)合并后发布;发布证据裁 A(C-first,#77):fixture + 本地四项闸门(test 421 passed)为凭,真机首跑另立里程碑;三平台本地交叉构建(linux x86_64 / aarch64 + windows x86_64 gnu 静态 CRT),macOS 仍暂缺,glibc 基线 2.41(同 Debian 13 构建机);公开仓,匿名安装即用。 |
