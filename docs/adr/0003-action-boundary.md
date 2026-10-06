# 面板只编排三类引擎动作，其余只展示证据

面板代用户执行的引擎命令只有三类：`nw ls`（概览，只读）、`nw up`（装配，写）、`nw doctor` / `nw validate`（只读复核）。三类之外的一切——`bake`、`source register` / `recheck`、`recipe new`、auth、手动编辑配置——面板只展示证据与建议命令，不代跑。

## Consequences

- V1 不含 bake 向导；`source recheck` 的漂移结论以只读方式并入健康屏。
- env 只显示 present 布尔，绝不回显值；用户自加的配置键不算漂移、不代改。
- 「建议动作」= 面板给出可复制的命令文案，执行权留给用户；将来要一键代跑，先立新 ADR。
