# 外部源登记进面板：register 两段式代跑，凭据只从环境来

面板新增第四类引擎动作：`nw source register`，且**代跑到落账**（含 `--yes`）。执行路径固定两段——**预审**（不带 `--yes`：扫描 + 验收 + 拟落账记录进 stdout，文件不落）→ **图形确认** → **落账**（带 `--yes`，写 `<数据仓根>/catalog.json`）。为此面板新增「数据仓根」这一状态（设置屏输入，与 NW_HOME 并列，持久化进 config）。`nw source recheck`（只读）一并纳入准许，落点是健康屏的漂移结论——这正是 ADR-0003 Consequences 里「`source recheck` 的漂移结论以只读方式并入健康屏」的兑现。

Status: accepted

> **决策记录**：操作者已拍板走两段式（预审 → 图形确认 → 落账），并批准 spec #25 / #33 / #34 / #35 全量；本文「Considered Options」里排除的三条（只给建议命令 / 一键到底 / 凭据收进面板配置）为其明确否决。落地票：#30 预审段、#31 落账段；汇总见 #41。

## Considered Options

- **只给建议命令（ADR-0003 对 bake 等的既定形态）**：排除。本工具的存在理由就是「用面板做这些操作、不碰命令行终端」；把旗标记忆成本推回给人等于没做。
- **一键到底（填完 repo + kind 直接 `--yes`）**：排除。引擎源码明言「落一条没验过的记录 = 闸门自己开后门」（`internal/source/acceptance.go`）；没有信息量的确认只是把手滑当天意——编目是信任闸唯一一道。
- **把凭据收进面板配置**：排除。上游纪律：凭据只从 `GH_TOKEN` / `GITHUB_TOKEN` 来、绝不落盘落编目；面板多存一份就是第二真相源。

## Consequences

- ADR-0003 的「三类」扩为「四类」，新增仅 register / recheck 这一类；`bake` / `recipe new` / `category` / `vps` / 手改配置维持「只展示证据与建议命令，不代跑」。
- `src/engine/commands.rs` 增 `source_register(repo, kind, name?, yes)`、`source_recheck(name?)`；一律走既有 `App::spawn_engine` 单缝与 `CommandRunner`（Real / Missing / Fake 可测）。
- envelope 增补 `nw/source-register.v1` / `nw/source-recheck.v1`；验收项 `data.acceptance[]` 的 **id 冻结、措辞不承诺**，面板按 id 渲染、复用结果四态（pass / warn / fail / skip；「无 `--yes`」= skip 是设计内正常态，不是错误）。
- 登记流程状态机：输入 → 预审 → 确认 → 落账 → 结果；复用发射仪式已有的「浮层 + 布尔 + Enter/Esc 两段命中」，并与 ADR-0005 的命中区同源，鼠标随之可用。
- 写动作互斥并入既有 `assembling`：登记与装配不同时进行。
- 验收有 fail、或退出码非 0 → **不得进入落账段**；`--yes` 只可能由用户在图形确认后触发。
- 凭据：面板只继承进程环境，不存、不写、不回显、不进日志；公开源零凭据可用。
- 退出码不发明新码：register / recheck 的码表以引擎为准，未登记码走既有 `ExitCode::from_number` 兜底。
- 已知限制：所装 `nw 0.1.0` 无 `nw category`，面板不提供类别输入（记录落「未分类」）；未设「数据仓根」时登记入口不可用并给指路，不猜默认值。
