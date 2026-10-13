# 面板只编排三类引擎动作，其余只展示证据

面板代用户执行的引擎命令只有三类：`nw ls`（概览，只读）、`nw up`（装配，写）、`nw doctor` / `nw validate`（只读复核）。三类之外的一切——`bake`、`source register` / `recheck`、`recipe new`、auth、手动编辑配置——面板只展示证据与建议命令，不代跑。

## Consequences

- V1 不含 bake 向导；`source recheck` 的漂移结论以只读方式并入健康屏。
- env 只显示 present 布尔，绝不回显值；用户自加的配置键不算漂移、不代改。
- 「建议动作」= 面板给出可复制的命令文案，执行权留给用户；将来要一键代跑，先立新 ADR。

> **修订（ADR-0006，accepted）**：「三类」已扩为「四类」——`nw source register`（代跑到落账，两段式确认）与 `nw source recheck`（只读，落健康屏）获准；其余写动作仍只展示证据与建议命令。

> **再修订（ADR-0009，accepted）**：新口径为「**六类**」。只读三类不变（`ls` / `doctor` + `validate` / `source recheck`）；写动作三类为 `up`、`source register`、**编目册写动作**（`vps` register + rm / `key` register + rm / `subscription` register + rm / `tag` add + rm，均两段式或带面板图形确认，见 ADR-0009）。`key get` / `subscription get` 一律只给建议命令——上游唯一回显本体的两个出口，面板不接。

> **再修订（spec #96 / 工单 #121，accepted）——「面板不自改」的口径澄清**：本条 ADR 约束的是「面板代跑哪些**引擎**命令」，从不是「面板不许换自己这个二进制」。面板的自升级（设置屏 `U`）**不是**新增第七类引擎动作：它不走 `nw`，落点是面板自己的可执行文件；且它不在 Rust 里另写一套校验，而是把 `install.sh` 的**同一份字节**嵌进二进制（`include_str!`）以 `sh` 跑——下载、sha256 校验、`GH_TOKEN` 口径与脚本出口逐字同源，先校验后落盘、tmp+rename 原子替换、校验不过旧二进制一个字节都不动。所以「面板不自改」的纪律没有被削弱，反而从「只有脚本一道闸」变成「两个出口同一道闸」。纪律成文在 `docs/release.md`「双出口」一节；ADR-0003 的边界（面板不代跑引擎命令）不变。
