# Rust + ratatui + crossterm + tokio 技术栈

终端外观与交互用 ratatui 0.30（渲染）+ crossterm（终端后端），子进程用 tokio 驱动，UI 状态集中管理。

## Considered Options

- **跟随 nw 用 Go 写面板**（bubbletea 等）：排除。本项目立项要求即 ratatui；Rust 侧实测可用（工具链 1.99、ratatui 0.30.2 直连 crates.io 无阻碍）。
- **同步阻塞跑子进程**：排除。`up` / `doctor` 有 stderr 人话流，阻塞会把 UI 卡死。

## Consequences

- 锁 ratatui 小版本并提交 `Cargo.lock`（0.30 生态新，API 有变更面：widgets 不在 prelude，workspace 已拆分子 crate）。
- 终端降级自己做：目标场景含 SSH，`xterm-256color` 下 RGB 颜色需降级到 256 色。
- 测试策略：进程执行抽象为可注入的 runner，UI 用 TestBackend 快照测试，端到端用 fixture JSON 回放，无需真 `nw`。
