# Warehousetui

`nw`（Nous Warehouse 装配引擎）的交互式驾驶舱：把 `validate` / `up` / `doctor` / `ls` 等命令流包装成「看得见状态、点得出动作、像玩游戏一样顺手」的终端面板。

引擎侧契约冻结（子进程 + JSON envelope），本面板是纯消费者，不做装配决策。技术栈：Rust + ratatui + crossterm + tokio。

领域词汇见 [CONTEXT.md](./CONTEXT.md)；架构决策见 [docs/adr/](./docs/adr/)。
