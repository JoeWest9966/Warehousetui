# nw 用户旅程与项目现状（Warehousetui TUI 事实调研）

> 只读调研（未修改 Nous-Warehouse 与 Warehousetui 任何文件；`.scratch` 与 `docs/samples` 零改动）。
> 事实来源：仓库代码与文档一手阅读 + `./nw --help` / `./nw ls` / `./nw validate` / `./nw doctor` 只读实测 + `gh issue list`。

---

## 1 · 项目现状速览（对 TUI 最关键的一条）

- `nw` = Nous Warehouse 的 **headless CLI**（Go，仅 stdlib + yaml.v3），当前 release **v0.1.0**（`nw --version` → `0.1.0`，走 stderr）。
- **TUI 在 nw 的地图里明确 Out of scope，但契约为其预留**——地图 Notes 硬约束原文：
  > 「CLI 是唯一接口,未来面板/TUI/Agent(nous-satellite 等)直接调它 —— JSON in/out 是契约。」
  → **Warehousetui 就是这句话里的「未来面板」**：不是绕过 CLI，而是 nw CLI 的官方图形层。这是本项目存在的许可证。
- 工单现状（2026-10-06 实测）：Nous-Warehouse #2–#27 **全部 CLOSED**，唯地图 [#1](https://github.com/JoeWest9966/Nous-Warehouse/issues/1) OPEN；Warehousetui 仓库 2026-10-06 已建（`gh repo view` 确认存在），**issue 列表为空**（尚无任何工单）。
- 能力闭环已全部落地：validate 27 项 / up 七步幂等 / doctor 七面 / bake / 外部源 register-recheck-bake / 真机首跑 rc 0（报告 `docs/real-first-run.md`，11 条真实世界的坑）。

## 2 · docs/ 索引与线索文档

| 文档 | 行数 | 是什么 | TUI 价值 |
|---|---|---|---|
| `docs/real-first-run.md` | 92 | #27 真机首跑报告：干净容器装配 rc 0、幂等、doctor 0、11 条坑 | 目标侧旅程的权威脚本 + 错误形态样本 |
| `docs/external-sources.md` | 321 | 外部源机制全设计（编目/取回锁定/凭据/检查面/复查） | 外部源旅程 + validate 27 项 id 语义 |
| `docs/samples/README.md` | 218 | 配方/组件 schema 样本 + check.cjs 14 项不变式 | 配方与组件字段的权威 schema 说明 |
| `docs/review/2026-10-02-acceptance.md` | 41 | 验收复核报告（ Standards/Spec 双轴） | 已知技术债（warnings[] code 未定义等） |
| `.scratch/nous-warehouse/map.md` | 48 | 历史地图（已迁移 GitHub #1） | 全局决策速览 |
| `.scratch/nous-warehouse/grilling/05–08` | 280/151/154/278 | 装配引擎/命令机制/跨平台/CLI 契约的**最终决议** | CLI 契约字段级冻结的来源 |
| `.scratch/nous-warehouse/research/01–03` | — | pi 运行时 / packages / chezmoi 内嵌可行性 | 目标机前提的事实依据 |

## 3 · 样例资产（真实摘录）

### 3.1 配方 `recipes/research-desk/recipe.yaml`（= 目标机 profile 名）

```yaml
apiVersion: nw/v1
kind: recipe
name: research-desk            # = profile 名 → ~/.nous-warehouse/profiles/research-desk
version: 1.0.0
profile: research-desk         # 一台两派才覆盖
runtime: { node: "22.19.0", pi: "1.0.2", chezmoi: "2.73.0" }   # bake 时译成具体 release 资产
components:
  - component: scout
    version: 1.0.0
  - component: retention
    version: 0.4.1
    params: { dir: notes }     # 配方只给值,旋钮由组件定义
models:
  defaults: { defaultProvider: stepfun-sb, defaultModel: step-5-preview, defaultThinkingLevel: medium, theme: dark }
  providers:
    - id: stepfun-sb
      baseUrl: https://api.stepfun.com/step_plan/v1
      api: openai-completions
      apiKey: $STEPFUN_API_KEY       # ← 只有变量名,值永远在目标机环境
env:
  - { name: TAVILY_API_KEY, required: true, usedBy: [components/scout] }
```

**铁律：配方/组件里没有任何密钥值，只有 `$ENV` 名。**

### 3.2 组件 `components/scout/component.yaml`（组件 = 本地 pi package，同住 monorepo）

```yaml
apiVersion: nw/v1
kind: component
name: scout
version: 1.0.0
pi: { package: "." }                 # 包根 = 本目录
packages: []                         # 第三方 pi 包一律 npm:/git: 带 pin,bake 时 vendored
vendored: true                       # 声明依赖由 bake 打进资产(目标机零 npm)
depends: [{ component: retention, version: 0.4.1 }]   # 引擎据此拓扑排序
params: [{ name: dir, type: string, required: false, default: notes }]
config:                              # 装配时键级 merge 进 profile 的 settings/models/mcp
  settings: { collapseChangelog: true, defaultTools: [read, bash, edit, write] }
env: [{ name: TAVILY_API_KEY, required: true }]
requires: [node>=22.19, git]         # 违反 = doctor 报红,不是装配失败
```

### 3.3 锁定集 + 装配命令 `docs/samples/baked/assembly-command.sh`（139 行 POSIX sh）

```sh
NW_LOCK=$(cat <<'LOCK'
{
  "schema": "nw/lock.v1",
  "bakedAt": "2026-10-01T12:00:00Z",
  "recipe": { "name": "research-desk", "version": "1.0.0" },
  "profile": "research-desk",
  "runtime": { "node": "22.19.0", "pi": "0.99.2", "chezmoi": "2.73.0" },
  "components": [
    { "name": "retention", "version": "0.4.1",
      "landing": { "path": "<agent-dir>/nw/components/retention", "kind": "pi-local-package" },
      "sha256": "b1c0…" }
  ],
  "env": [ { "name": "TAVILY_API_KEY", "required": true, "usedBy": ["components/scout"] } ],
  "assets": [ … nw-<os>-<arch> + node/chezmoi/pi,每条带 sha256 … ]
}
LOCK
)
```

命令 = **引导器四段**（引导下载校验 → 交锁 `$NW_HOME/locks/<lockId>.nwlock` → 交棒 `nw up --lock` + `nw doctor`（码透传）→ 收尾 hints）；编排全归引擎。bundle = `tar.gz`（组件整树含 node_modules + skeleton + recipe.yaml），字节可复现，`bakedAt` 使命令非逐字节确定（家长裁案 B，#15）。

### 3.4 bundle 布局（`recipes/research-desk/baked/research-desk-1.0.0/`）

```
bin/{nw-linux-amd64, node, chezmoi, pi}   # 运行期资产
components/{retention, scout}/…           # 组件整树(node_modules 随资产下发)
recipe.yaml                               # 坑5 修复:配方打进 bundle 根
```

## 4 · CLI 面速览（`./nw --help` 实测；字段级细节归 cli-surface 调研）

```
nw validate (--recipe <f> | --lock <sh>) [--repo <dir>]      # 27 项冻结检查, rc 0/2/3
nw recipe new <名> [--profile <名>] [--out-dir <目录>]        # 脚手架
nw recipe bake <配方>[@版本] --out <命令文件> [--assets-dir <目录>] [--repo <目录>]   # rc 0/2/3/4
nw up --lock <file> [--nw-home <dir>]                        # 七步幂等, rc 0/2/3/4/5
nw doctor (--lock <file> | --profile <name>) [--nw-home <dir>]   # 只读七面, rc 0/10/11/12
nw ls [recipes|components|sources|profiles|locks] [--repo <dir>] [--nw-home <dir>]  # 五域
nw source register --repo <链接> --kind skill|extension [--name <名>] [--category <类>]… [--yes] [--root <dir>]
nw source recheck [--name <名>] [--root <dir>]               # 只报告,永不写账
```

**输出面硬约束（#9 D1/D2）**：stdout = 唯一一份 JSON envelope（`--json` 是 no-op，不提供人类可读 stdout）；人话/进度/警告/`--version` 全走 stderr。实测 stderr 人话形态（doctor）：

```
nw doctor: rc=12
  FAIL  manifest  清单不存在:该 profile 尚未完成装配或被清过
```

**envelope 实测**（ls / validate / doctor 同构）：

```json
{ "schema": "nw/doctor.v1", "nw": "0.1.0", "ok": false,
  "errors": [ { "code": "MANIFEST_BROKEN", "exitCode": 12, "message": "…", "details": { "profileDir": "…" } } ],
  "warnings": [], "data": { "results": [ { "surface": "manifest", "status": "fail", "message": "…", "details": { … } } ], "exit": 12, "converged": false } }
```

（契约错误五字段 = `code`/`exitCode`/`message`/`hint`/`details`；`hint` 可缺。）

## 5 · 用户旅程（8 个，标注命令与易错点）

### J1 作者侧 · 从零发布一个派（完整发布环）
1. `nw recipe new research-desk` —— 脚手架本身就是合法配方（首次 validate 即 0）。
2. 编辑 `recipes/research-desk/recipe.yaml`（runtime 三个 pin、components、models、env）。
3. `nw validate --recipe recipes/research-desk/recipe.yaml --repo .` —— 27 项，fail → rc 3。
4. `nw recipe bake recipes/research-desk/recipe.yaml --out assembly-command.sh --assets-dir <资产目录> --repo .` —— 先自己跑 validate，不过拒烤；产出 lockId + bundle + 命令 + `invoke.<platform>`。
5. `gh release create nw/v0.1.0 nw-linux-amd64 node pi chezmoi`（引擎与运行期资产，每平台一份）。
6. `gh release create bake/<lockId> <bundle> assembly-command.sh`（bundle 89.4MB / node 121MB 现实）。
- **易错点**：`packages` 声明了却没 vendored → `missingVendored` 拒烤；占位第三方包（npm:playwright 假源 / example 仓 404）首跑拒烤，转正须清空 `packages: []`；`bakedAt` 进命令 → 同输入不同时刻命令字节不同（幂等靠不含它的 lockId）；跨仓 release 上传需显式 `GH_TOKEN`（Actions 自带 token 只读当前仓）。

### J2 作者侧 · 收编一个外部源（skill/extension 在自己 GitHub 仓）
1. `nw source register --repo owner/repo --kind skill --category 工具 --category cli`
2. 不带 `--yes`：只扫描 + 跑验收 5 条 + 拟落账记录进 stdout JSON，**文件不落**（headless 弹不了交互，确认闸的唯一合法形态）。
3. `nw source register … --yes` → 写 `<repo-root>/catalog.json`（`nw/catalog.v1`，skills[]/extensions[] 两数组）。
4. 配方引用：`components: - component: owner/repo/条名` + `source: external`（禁 `version`、禁 `params`）。
5. 日常对账：`nw source recheck` —— 只报四种漂移（added/removed/renamed/path_changed），rc 3 报红，永不写账。
- **易错点**：外部源**没有版本层**（用「配方钉 version」心智模型必错）；bake 现拉默认分支 HEAD，上游动 → 重烤 lockId 变；类别只能选自词表（`nw category new` 增补），单层禁 `/`；凭据只从 `GH_TOKEN`/`GITHUB_TOKEN` 来、绝不落 catalog/lock/bundle；点名查无此源 → bake fail-closed rc 3。

### J3 作者侧 · 组件升级闭环
1. 改 `components/scout/` 本体 → bump `version` → `nw validate --recipe … --repo .`。
2. 重新 `nw recipe bake` → 新 lockId（内容决定）→ 发布新 `bake/<lockId>` release。
3. 目标机**重跑同一条装配命令**（或下载新的）→ `nw up` 收敛到新锁；旧组件整目录移入 `nw/pruned/<ts>/`（只移不删，`--allow-prune` 才真删）。
- **易错点**：source 路径不带版本目录（升级不堆目录）；`installedAt` 只在 lockId 变化时刷新（保幂等逐字节一致）。

### J4 目标侧 · 新机器装配（真机首跑实证路径）
```bash
gh release download bake/research-desk-1.0.0-360ae68 --repo JoeWest9966/Nous-Warehouse -p assembly-command.sh
NW_HOME=/root/.nous-warehouse TAVILY_API_KEY=… STEPFUN_API_KEY=… bash assembly-command.sh   # rc 0
$NW_HOME/profiles/research-desk/bin/pi --version        # 1.0.2
bash assembly-command.sh                                # 第二次 rc 0,.nw-manifest 逐字节一致
```
- **易错点**：**有 gh ≠ 有私库读权限**（gh 对无权限私库谎报 `release not found` rc 1；坏 token 报 401——两种可能并列给排查提示，不猜）；`HOME`+`NW_HOME` 双缺 → rc 2；required env 缺失 = up rc 0（降级成功）+ doctor rc 10；老 glibc/kernel（Tier 3）未实测，只报错+指引自备 node。

### J5 目标侧 · 日常健康巡检
`nw doctor --profile research-desk --nw-home $NW_HOME` —— 只读七面（manifest / consist / converge / config / env / runtime / provider），rc **0/10/11/12 取最重**（12>11>10>0）。
- **易错点**：provider 面由 nw 自己映射 `pi auth check --json` 的 `status`（`ready`/`not_ready`），**不采信 pi 的 rc**（not_ready 时 pi 也 rc 1）；探 provider 必须带 `--provider <defaultProvider>` 且经 env 传 `PI_CODING_AGENT_DIR`；`chezmoi --version` 带尾逗号曾致假漂移；运行期产物（auth.json/sessions/missions/git/models-store.json/bin）刚装完一个都没有 = 正常态，报 skip 不报 fail；用户自加的 settings 键不算漂移。

### J6 目标侧 · 排障「漂移」（rc 11）
现象：声明物被改 / 与锁定集不符。动作：读 doctor `data.results[].details`（面 + 第一段文件名 + 其后 JSON 键路径）→ 定位被改的键/文件 → **重跑装配命令**即幂等收敛 → 对比 `.nw-manifest` 的 `convergeHash`。
- **易错点**：`ls` 的 `converged` 态是廉价判据（必要不充分，实测 ls converged 而 doctor 实时探针报 10）；漂移修复 = 重跑，不是手改。

### J7 任意侧 · 一眼看全景
`nw ls [--repo <仓库>] [--nw-home $NW_HOME]` —— 五域：`recipes` / `components`（含 depends、vendored、usedBy）/ `sources` / `profiles`（含 `state` 四态：converged/degraded/drift/missing，与 doctor 码一一对应）/ `locks`。
- **易错点**：域不存在 = 空数组（不是错误）；坏 YAML → rc 3 且 `details.path` 点名；只有 `$NW_HOME` 不可解析才 rc 2；ls 零写（测试断言跑完 NW_HOME 逐文件 sha256 零改动）。

### J8 CI / 批量化
`ci/smoke.sh`（九格平台矩阵 + 连跑两次逐字节一致）、`ci/negative.sh`（N1–N6）、`ci/drift-check.sh`（真联网，nightly）、`nw source recheck` 挂 CI。
- **易错点**： Actions 自带 `GITHUB_TOKEN` 只读当前仓，跨仓必须显式 `GH_TOKEN`；上游真相靠 drift-check 已知集合 + nightly 报红覆盖。

## 6 · 对 TUI 信息架构的事实含义（供设计轮消费）

1. **导航天然五页**：`nw ls` 五域（派 / 组件 / 外部源 / 配方 / 锁）就是侧边栏的五个一级页面。
2. **健康总览天然七块**：doctor 七面可直接映射为一块总览面板的分区；退出码 0/10/11/12 = 四种结果态（ok / degraded / drift / broken）。
3. **装配 = 有进度仪式感的七步向导**：up 的 D1 七步（前置/引导/骨架/组件/合配置/写清单/自检）就是向导的步骤条；stderr 人话进度实时进日志窗格。
4. **validate 27 项 = 可扫的检查清单视图**：results[] 每项 `{id,status,message,path}`，status ∈ pass/warn/fail/skip，分色 + 可展开 details。
5. **环境变量只显示 present 布尔 + required**（manifest 只记 `present`、永不记值）—— TUI 必须延续这条安全纪律，绝不回显 secret。
6. **错误展示直接用五字段**（code/exitCode/message/hint/details），TUI 不发明自己的错误语言。
7. **契约稳定性**：envelope 顶层 `schema`（`nw/<cmd>.v1`）+ `nw` 版本号；只加可选字段不破、改语义升 v2。TUI 应按 `schema` 做兼容分支，而非钉死字段。
8. **`--json` 是 no-op**：TUI 生成的调用串不必带 `--json`，带了也合法。
9. **硬约束背书**：nw 地图明文「CLI 是唯一接口,未来面板/TUI Agent 直接调它」→ Warehousetui 的存在合法且被期待；但 Out-of-scope 也意味着 **TUI 不得要求改 nw**（nw 契约已字段级冻结），一切差异在 TUI 侧消化。

## 7 · 已知未决 / 风险（记录在案，不阻塞 TUI 设计）

- `warnings[]` 元素 `code` 未定义（warn 暂走 results.status + stderr）—— TUI 展示 warnings 时要兼容「无 code」形态。
- `up --json` 真实字段 `data.convergeHash` vs 文档 `data.manifest.convergeHash`（文档 conform 已修，代码为准）。
- ls `drift` 态判据廉价（mtime 疑探），与 doctor 结论可能不一致——TUI 若做「一键跑 doctor」要说明 ls 是预览。
- 供应链签名（超越 sha256）未做；公开镜像下载通道未开。
- 老 glibc/kernel（Tier 3）路径未实测。
- Warehousetui 仓库 issue 列表为空：尚未建立任何工单/地图。
