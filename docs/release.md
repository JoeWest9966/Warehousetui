# 发布矩阵

nwt 的发布面:打 `v*` tag → CI 五组合构建 → GitHub Release(附 sha256 校验和)→ 用户一行 `curl` 安装。本文是**发布者文档**;用户侧安装说明在 [README 安装段](../README.md#安装)。

> 状态:本文件随首个发布基建 PR 落地;首个 tag(`v0.1.0`)尚未发布,发布后回填底部「发布记录」。

## 发布流程

1. 确认 main 绿:CI 三平台跑齐 `cargo fmt --check` / `cargo clippy --all-targets -- -D warnings` / `cargo test --all-targets` / `cargo build --release`。
2. 确认版本号:`Cargo.toml` 的 `version` 与要打的 tag 一致(首版 `0.1.0`)。tag 与 Cargo 版本的一致性靠这一步人工核对,不引入构建期注入。
3. 打 tag 并推:

   ```bash
   git tag v0.1.0
   git push origin v0.1.0
   ```

4. `.github/workflows/release.yml` 自动触发(只在 `v*` tag 上跑,普通 push/PR 不触发):五组合(linux / macos × x86_64 / aarch64 + windows x86_64)各构建 release 二进制 → 按「资产命名」打包 → 生成 `checksums.txt` → 创建 GitHub Release 并附上全部资产。
5. 人工核对:release 页资产与下表一一对应;在「发布记录」登记一行(日期 / tag / CI run 链接)。

## 资产命名(三处同源的契约)

| 平台 | 资产 |
|---|---|
| linux x86_64 | `nwt_<版本>_linux_x86_64.tar.gz`(runner `ubuntu-latest`,原生) |
| linux aarch64 | `nwt_<版本>_linux_aarch64.tar.gz`(runner `ubuntu-24.04-arm`,原生) |
| macos x86_64 | `nwt_<版本>_macos_x86_64.tar.gz`(runner `macos-latest` 上**交叉编译** `x86_64-apple-darwin`) |
| macos aarch64 | `nwt_<版本>_macos_aarch64.tar.gz`(runner `macos-latest`,原生 Apple Silicon) |
| windows x86_64 | `nwt_<版本>_windows_x86_64.zip`(runner `windows-latest`,唯一 `.zip`) |

> runner 说明:macOS 13(Intel)公共 runner 已不可靠——v0.1.0 首发彩排时排队 47 分钟未获分配,故 macos x86_64 改在 `macos-latest`(Apple Silicon)上交叉编译;产物仍是真 x86_64 二进制,命名契约不变。
| 校验和 | `checksums.txt` |

- `<版本>` = tag 去掉 `v` 前缀(`v0.1.0` → `0.1.0`)。
- unix 包内二进制名 `nwt`;windows 包内 `nwt.exe`。
- **只出 windows x86_64**:Windows 是「编译可运行」级支持([spec #10](https://github.com/JoeWest9966/Warehousetui/issues/10) 钉死的 R3 决定;ADR-0002 只定技术栈、不涉支持级),不为它出全矩阵。
- `checksums.txt` 用 `sha256sum` 双列格式,unix 上可直接 `sha256sum -c` 验。

**契约同步要求**:release.yml 的产出名、install.sh 的请求名、`checksums.txt` 的条目名,三处同源。改命名必须三处一起改,并更新本表——这是 install.sh 能装对版本的唯一保证。

## 用户侧校验

手动下载的用户,装前自校验:

```bash
curl -fsSLO https://github.com/JoeWest9966/Warehousetui/releases/download/v0.1.0/checksums.txt
sha256sum -c checksums.txt        # 每一项都要 OK
```

install.sh 装前做同一件事;对不上就非零退出、不留半个二进制。

## 本地复现(不推 tag 也能验打包与校验)

Linux / macOS:

```bash
cargo build --release
os=$(uname -s | tr 'A-Z' 'a-z' | sed 's/darwin/macos/')
arch=$(uname -m | sed 's/arm64/aarch64/')          # macOS Apple Silicon 归一成 aarch64
tar -C target/release -czf "nwt_0.1.0_${os}_${arch}.tar.gz" nwt
sha256sum nwt_*.tar.gz > checksums.txt
```

Windows(在 Windows 上):

```powershell
cargo build --release
Compress-Archive -Path target\release\nwt.exe -DestinationPath nwt_0.1.0_windows_x86_64.zip
Get-FileHash -Algorithm SHA256 nwt_0.1.0_windows_x86_64.zip
```

`Get-FileHash` 的输出要手工整理成 `sha256sum` 双列格式(小写 hash、两空格、文件名)再合入 `checksums.txt`。五组合的完整矩阵以 CI 为准,本地只做单平台冒烟。

## 支持级与已知取舍

- Windows:编译运行级([spec #10](https://github.com/JoeWest9966/Warehousetui/issues/10) 的 R3 决定),无 installer、无自动安装;手动 zip。
- 无代码签名/公证(macOS notarization、Windows Authenticode):V1 不做,靠 sha256 校验兜底。
- 无自更新命令(`nwt self-update`):保持 ADR-0003 精神,面板不自改。
- macos x86_64 走交叉编译:不占 Intel runner(见上),代价是构建机与目标机不同架构,二进制未经同架构实跑验证——安装脚本按 checksums 保证完整性,功能风险由 CI 测试面兜底。
- **仓库私有时匿名安装必然 404**:release 资产与 `raw.githubusercontent.com` 的 install.sh 对匿名请求都返回 404(私有仓库行为)。`curl | sh` 一行装的前提是**仓库公开**;保持私有则需给 install.sh 加 token 认证通道(未实现)。
- **glibc 基线 2.39**:linux 包在 ubuntu-latest(24.04)构建,老发行版(< glibc 2.39)运行会报 `GLIBC_2.39 not found`。需要更宽兼容时可改 ubuntu-22.04 构建(未实施,彩排实测确认)。
- 包管理器分发(Homebrew / Scoop / AUR):后续票据。

## 发布记录

| 日期 | tag | CI run | 备注 |
|---|---|---|---|
| 2026-10-06 | v0.1.0 | [37443153354](https://github.com/JoeWest9966/Warehousetui/actions/runs/37443153354) | 首次彩排:5 组合全绿;修复两条彩排问题(macOS x86_64 改交叉编译、runs-on 重复定义)后正式成发布 |
| 待填 | v0.1.0 | — | 首个正式发布 |
| 2026-10-09 | v0.2.0 | [37880214287](https://github.com/JoeWest9966/Warehousetui/actions/runs/37880214287) | 真机首跑实证( docs/real-first-run.md)后打 tag;发布构建因账号级计费一度无法启动(jobs 无 runner),计费恢复后 `gh run rerun 37880214287`(全部 job,含被依赖跳过的 release job)即成发布,回填本行 |
