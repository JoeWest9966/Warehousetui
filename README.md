# Warehousetui

`nw`(Nous Warehouse 装配引擎)的交互式驾驶舱:把 `validate` / `up` / `doctor` / `ls` 等命令流包装成「看得见状态、点得出动作、像玩游戏一样顺手」的终端面板。

引擎侧契约冻结——本面板是**纯消费者**:fork `nw` 子进程、解析 stdout 的 JSON envelope、把 stderr 的人话流变成实时日志。面板自己不做装配决策。

## 安装

```bash
curl -fsSL https://raw.githubusercontent.com/JoeWest9966/Warehousetui/main/install.sh | sh
```

默认装到 `~/.local/bin/nwt`。装完敲 `nwt` 没反应,就是它不在 `PATH`:

```bash
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.bashrc   # 或 ~/.zshrc
```

- 先看计划再装:把脚本下载下来跑 `sh install.sh --dry-run`(只打印装什么、装到哪,不落盘)
- 装到别处:`sh install.sh --prefix DIR`,或 `PREFIX` 环境变量
- 升级:重跑最上面那行,覆盖即升级
- 安装前强制校验 sha256,对不上就取消,不留半个二进制

Windows:去 [releases](https://github.com/JoeWest9966/Warehousetui/releases) 手动下载 `nwt_<版本>_windows_x86_64.zip`。支持级是「编译可运行」([spec #10](https://github.com/JoeWest9966/Warehousetui/issues/10) 钉死的 R3 决定;ADR-0002 只定 Rust/ratatui 技术栈,不涉支持级),不提供 installer;资产与校验细节见 [docs/release.md](./docs/release.md)。

> 预编译二进制随首个 tag(v0.1.0)发布后才可用;在那之前请用下面的源码构建。

## 从源码构建

```bash
cargo build --release
./target/release/nwt                      # 自动发现 nw(显式 > 配置 > PATH > $NW_HOME/nw)
./target/release/nwt --nw-bin /path/nw    # 找不到引擎时显式指路
```

终端至少 66×50;宽 ≥ 80 为常畅态(竖左导航),窄于 80 自动切紧凑态(横向屏 Tab),只改排布不改字号;真彩色自动探测,SSH 下自动降级 256/16 色。发布流程与本地打包复现见 [docs/release.md](./docs/release.md)。

## 五屏

| 屏 | 干什么 | 键 |
|---|---|---|
| 概览 | 警示带 + 派四态表 + 四域计数卡 + 指令栏 + 最近日志 | `↑/↓` 选派,`r` 刷 ls,`a`/`h`/`l` 跳屏 |
| 装配 | 选锁定集 → 发射仪式 → 七步 + stderr 实时日志 | `↑/↓` 选锁,`Enter` 发射仪式,再 `Enter` 发射 |
| 健康 | doctor 七面卡片 / validate 27 项清单 + 证据五字段 | `r` 复核,`m` 切模式,`↑/↓` 翻阅 |
| 配方 | 五域两栏只读浏览(recipes/components/sources/profiles/locks) | `h/l` 切域,`↑/↓` 选行 |
| 设置 | 引擎发现状态、NW_HOME、数据仓根、主题、配置入口 | `n` 轮换 NW_HOME,`d` 手填数据仓根,`e` 拉起 $EDITOR,`t` 主题 |

通用:`j/k` 切屏 · `Enter` 确认 · `Esc` 返回 · `?` 帮助 · `q` 退出(装配态禁用)。

### 鼠标(默认开捕获)

- 点左导航(常畅态)/顶部屏 Tab(紧凑态)切屏;点列表行=选中(与 `↑/↓` 同语义,不触发破坏性动作)。
- 可选列表上滚轮上下移选中;装配 stderr 实时日志区滚轮回看早先输出。
- 发射仪式「Enter 发射 / Esc 返航」两段可点;帮助浮层点任意处关;浮层打开时点击不穿透底层。
- 捕获默认开启的代价:终端原生「拖拽选中 / 右键复制」需按 **Shift**。要还给终端:`--no-mouse` 旗标启动,或设置屏按 `m` 关(写入 `config.mouse_capture`,下次启动生效)。

## 行为纪律

- **降级 ≠ 失败**:required env 缺失、provider 未就绪是 amber 横幅;`skip` 是正常态(pi 懒创建)。
- **证据原样**:errors/warnings 五字段(code/exitCode/message/hint/details)原文呈现,建议动作只给命令不代跑(ADR-0003)。
- **永不回显密钥**:env 只显示 present 布尔;details 可含任意 JSON,但面板不做值猜测。
- **装配幂等**:`up` 可取消、可重入;并发锁(rc 5)明示「同一派已在装配」。

## 开发

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets     # 133 个单测:契约解析 / 发现策略 / 五屏快照 / 装配四路径 / 命中区与响应式两档 / 数据仓根三态指路 / 80×30 整帧 golden
cargo build --release
```

CI 三平台(ubuntu/macos/windows)跑齐上面四项。

## 文档

- 领域词汇:[CONTEXT.md](./CONTEXT.md)
- 架构决策:[docs/adr/](./docs/adr/)(0001 子进程契约 / 0002 技术栈 / 0003 动作边界)
- 调研底稿:[docs/research/](./docs/research/)(nw CLI 契约面 / 用户旅程 / 环境验证)
- 发布矩阵:[docs/release.md](./docs/release.md)(安装 / 校验 / 打 tag 发布)
- 工单:[GitHub Issues](https://github.com/JoeWest9966/Warehousetui/issues)(wayfinder 蓝图 #1)
