# 环境调研：Rust + ratatui 可行性（2026-10-06）

调研人：子代理（只读调研，唯一写入为本报告；/tmp 下一次性脚手架实验）。

## 结论

**可行。** ratatui 0.30.2 + crossterm 0.29.0 在当前环境完成 `cargo check` + `cargo build` + pty 实跑，
无系统级依赖缺口；网络可拉取新依赖（实测 tokio 1.53.2 `cargo fetch` 成功）。

## 1. 工具链

| 项 | 值 |
|---|---|
| rustc / cargo | 1.99.0（stable-x86_64-unknown-linux-gnu，rustup 默认） |
| rustup | 在 `~/.cargo/bin/rustup`，toolchain 装于 `~/.rustup` |
| PATH 坑 | **`~/.cargo/bin` 不在非交互 shell 的 PATH 里**（`bash -lc` 也没有）。所有脚本/CI 必须先 `export PATH="$HOME/.cargo/bin:$PATH"` 或 `source ~/.cargo/env` |
| 组件 | cargo / cargo-clippy / cargo-fmt / rustfmt / rust-analyzer / rust-gdb 全套 |
| 已装 target | 仅 `x86_64-unknown-linux-gnu`（跨平台编译需 `rustup target add`） |

## 2. 硬件与磁盘

- CPU 4 核，内存 7.3 GiB（可用约 4.2 GiB）——dev build ratatui 约 7.6s，无压力。
- 磁盘：根文件系统 40G 总量 / **19G 可用**。cargo target 目录注意放 /tmp 或工程内即可。

## 3. 网络与 crates.io

- `index.crates.io`（sparse index）：**200 可达**。
- `static.crates.io` 裸域名 HEAD 返回 403（S3 默认行为，**不是封锁**）；实测具体 `.crate` 对象 GET 返回 **200**。
- **新依赖可拉取**：干净目录里 `cargo fetch` 成功下载 tokio 1.53.2 及其依赖树。
- 无 `~/.cargo/config.toml`（未配镜像），无 `CARGO_NET_OFFLINE` → 默认 crates.io，直连即可。
- 附带好处：`~/.cargo/registry` 缓存里已有 ratatui 0.30.2 全线的 crate（此前某次探针留下的），冷启动工程也能秒过。

## 4. 版本组合（实测解析结果）

| crate | 版本 | 备注 |
|---|---|---|
| ratatui | **0.30.2** | crates.io 最新稳定；MSRV 1.88.0 < 本机 1.99.0 |
| crossterm | **0.29.0** | ratatui 0.30 同代；0.28 亦可（feature `crossterm_0_28` 切换） |
| ratatui-core | 0.1.2 | 0.30 起 workspace 拆分产物 |
| ratatui-widgets | 0.3.2 | 同上 |
| ratatui-crossterm | 0.1.2 | CrosstermBackend；按 feature 选 crossterm 0_28/0_29 |
| ratatui-macros | 0.7.2 | 默认 feature 含 |
| tokio | 1.53.2（fetch 验证） | 异步跑 nw 子进程可选方案 |

保守备选：ratatui 0.29.0（MSRV 1.74，单体 crate，API 与网上教程/0.28 一致）。

## 5. ratatui 0.30 API 形态（与 0.29 及以前不同，写代码必读）

- **workspace 拆分**：umbrella `ratatui` crate re-export 全部常见路径，应用只依赖 `ratatui` 即可。
- 初始化：`ratatui::run(|terminal| ...)` 一行搞定（raw mode + alternate screen + panic hook + 退出恢复）。
- **widgets 不在 prelude**：`use ratatui::prelude::*;` 只带 `Widget/StatefulWidget/BlockExt`；
  `Block`/`Paragraph` 等要显式 `use ratatui::widgets::{Block, Paragraph};`（探针实测踩到并修复）。
- prelude 有：`Frame, Terminal, CrosstermBackend, Backend, TestBackend, buffer, layout, style{Color,Style,Stylize}, text{Line,Span}, symbols`。
- 事件：`ratatui::crossterm::event` re-export 可直接用，无需另引 crossterm crate（但显式 `crossterm = "0.29"` 更稳）。
- feature：默认 = all-widgets + crossterm + layout-cache + macros + underline-color；serde、palette、scrolling-regions 按需开。

## 6. 终端能力

- `TERM=xterm-256color`，`COLORTERM=truecolor` → **真彩色可用**。
- 建议：代码里做 `Color::Rgb` + 降级到 256 色的能力探测（服务端/SSH 场景常见 256-only）。

## 7. 实验证据（/tmp/ratatui-probe）

- `cargo init` + `ratatui="0.30" crossterm="0.29"`，`cargo check` 通过。
- `cargo build` dev profile 7.57s 全绿（含 crossterm/ratatui-widgets/crossterm/macros 编译）。
- pty 实跑：`timeout 10 script -qec "printf 'q' | ./target/debug/probe" /dev/null` → 输出 alternate-screen 进出/光标隐藏恢复的转义序列，**rc=0**，按键后干净退出。
- 无系统依赖：不需要 pkg-config / openssl / X11；无头环境完全 OK。

## 8. 坑与风险

1. PATH 坑（见上）——所有构建/CI 脚本显式加 `~/.cargo/bin`。
2. ratatui 0.30.x 生态较新（0.30.2 / macros 0.7.2）：建议 Cargo.toml 锁定小版本并提交 `Cargo.lock`；网络资料多针对 0.29 及以前，照抄会中 widgets-prelude 这类小坑。
3. 单 target 已装；若 V1 要出 macOS/Windows 二进制需另加 target 或用 CI。
4. /tmp 探针目录（/tmp/ratatui-probe、/tmp/fetch-probe）为一次性脚手架，可删。
