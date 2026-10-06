# Warehousetui

`nw`(Nous Warehouse 装配引擎)的交互式驾驶舱:把 `validate` / `up` / `doctor` / `ls` 等命令流包装成「看得见状态、点得出动作、像玩游戏一样顺手」的终端面板。

引擎侧契约冻结——本面板是**纯消费者**:fork `nw` 子进程、解析 stdout 的 JSON envelope、把 stderr 的人话流变成实时日志。面板自己不做装配决策。

## 快速开始

```bash
cargo build --release
./target/release/nwt                      # 自动发现 nw(显式 > 配置 > PATH > $NW_HOME/nw)
./target/release/nwt --nw-bin /path/nw    # 找不到引擎时显式指路
```

终端至少 80×24;真彩色自动探测,SSH 下自动降级 256/16 色。

## 五屏

| 屏 | 干什么 | 键 |
|---|---|---|
| 概览 | 警示带 + 派四态表 + 四域计数卡 + 指令栏 + 最近日志 | `↑/↓` 选派,`r` 刷 ls,`a`/`h`/`l` 跳屏 |
| 装配 | 选锁定集 → 发射仪式 → 七步 + stderr 实时日志 | `↑/↓` 选锁,`Enter` 发射仪式,再 `Enter` 发射 |
| 健康 | doctor 七面卡片 / validate 27 项清单 + 证据五字段 | `r` 复核,`m` 切模式,`↑/↓` 翻阅 |
| 配方 | 五域两栏只读浏览(recipes/components/sources/profiles/locks) | `h/l` 切域,`↑/↓` 选行 |
| 设置 | 引擎发现状态、NW_HOME、主题、配置入口 | `n` 轮换 NW_HOME,`e` 拉起 $EDITOR,`t` 主题 |

通用:`j/k` 切屏 · `Enter` 确认 · `Esc` 返回 · `?` 帮助 · `q` 退出(装配态禁用)。

## 行为纪律

- **降级 ≠ 失败**:required env 缺失、provider 未就绪是 amber 横幅;`skip` 是正常态(pi 懒创建)。
- **证据原样**:errors/warnings 五字段(code/exitCode/message/hint/details)原文呈现,建议动作只给命令不代跑(ADR-0003)。
- **永不回显密钥**:env 只显示 present 布尔;details 可含任意 JSON,但面板不做值猜测。
- **装配幂等**:`up` 可取消、可重入;并发锁(rc 5)明示「同一派已在装配」。

## 开发

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets     # 59 个单测:契约解析 / 发现策略 / 五屏快照 / 装配四路径
cargo build --release
```

CI 三平台(ubuntu/macos/windows)跑齐上面四项。

## 文档

- 领域词汇:[CONTEXT.md](./CONTEXT.md)
- 架构决策:[docs/adr/](./docs/adr/)(0001 子进程契约 / 0002 技术栈 / 0003 动作边界)
- 调研底稿:[docs/research/](./docs/research/)(nw CLI 契约面 / 用户旅程 / 环境验证)
- 工单:[GitHub Issues](https://github.com/JoeWest9966/Warehousetui/issues)(wayfinder 蓝图 #1)
