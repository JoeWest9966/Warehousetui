# 以子进程消费 nw，JSON envelope 是唯一契约

Warehousetui 通过 fork `nw` 二进制、解析其 stdout 的 JSON envelope 来驱动装配；面板不复制、不内嵌任何引擎逻辑。

## Considered Options

- **cgo 链入 Go 代码**：排除。面板要跨三平台单二进制发布，不值得为此引入 Go 工具链与 cgo 链接。
- **Rust 复刻引擎**：排除。契约已在 Nous-Warehouse #9 字段级冻结，复刻必然漂移；且地图工单 #1 明文「CLI 是唯一接口，未来面板/TUI/Agent 直接调它」，不得要求改 nw。

## Consequences

- 面板与 nw 可各自演进：消费侧按 `schema` 字段做兼容分支（只加可选字段不破，改语义升 v2）。
- 面板必须自带子命令的帮助文案——`nw --help` 在引擎侧是 rc 3 的错误路径，不是帮助面。
- `up` 无实时进度、有 profile 级并发锁（rc 5），面板须把「装配态」做成全局互斥态。
- `PI_CODING_AGENT_DIR` 等环境变量需要透传给子进程（doctor 探 provider 必需）。
