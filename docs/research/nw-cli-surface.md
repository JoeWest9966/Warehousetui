# nw CLI 对外契约面（给 Warehousetui 的事实底稿）

调研对象：`/opt/Nous-Warehouse`（Go 实现，headless CLI，binary `./nw`，envelope.Version = **0.1.0**）。
调研方式：读源码（`cmd/`、`internal/cli`、`internal/envelope`、`internal/{up,doctor,ls,bake,source,validate,manifest,exec}`）+ 只读实跑（`--help`、validate/doctor/ls 样例）。**未运行任何写操作（未跑 up/bake/source register）**， Nous-Warehouse 仓库 git 保持 clean。

---

## 0. 一句话总纲

stdout **永远只有一份 JSON envelope**（六字段），人话/进度/警告/`--version` 全走 stderr；`--json` 是 accepted no-op（不开人类可读模式）；退出码是机器面。设计文档冻结于问题 #9《08 · CLI 契约》（D1/D2/D4/D5）。契约纪律：**已发布字段名一个字符不许改，破坏性变更必须升 `.v2`；消息措辞不承诺，`id`/`surface` 才是稳定面**。

## 1. 子命令 × flag 全表（权威来自 `--help` 与 `internal/cli/cli.go`）

| 子命令 | 位置参数 | flags（默认值） | 必填约束 | schema |
|---|---|---|---|---|
| `nw --version` / `version` / `-version` | — | — | 裸版本号落 **stderr**，stdout 空，rc 0 | — |
| `--help` / `-h` / `help` | — | — | usage 落 stderr，**rc 0** | — |
| `validate` | — | `--recipe <yaml>`("")、`--lock <cmd或.nwlock>`("")、`--repo <dir>`("")、`--quiet`(false)、`--json`(no-op) | `--recipe`/`--lock` **至少一个**，否则 rc 3「拒绝猜要校验哪侧」 | `nw/validate.v1` |
| `recipe new` | `<名>`（恰好 1 个） | `--profile`("")、`--out-dir`(".",骨架写 `<out-dir>/recipes/<名>/recipe.yaml`)、`--json` | 位置参数**必须恰好一个** | `nw/recipe-new.v1` |
| `recipe bake` | `<配方>[@版本]`（恰好 1 个） | `--out <命令文件>`("",缺省 `nw-assemble.sh` 落在 `<配方目录>/../baked/`)、`--assets-dir`("")、`--repo`("")、`--json` | 位置参数恰好一个；`@` 之后是要烤的版本，与配方版本不符 → rc 3 | `nw/bake.v1` |
| `up` | — | `--lock <file>`("")、`--nw-home <dir>`("")、`--json` | `--lock` 必给，「没有锁就没有目标态」 | `nw/up.v1` |
| `doctor` | — | `--lock`("")、`--profile`("")、`--nw-home`("")、`--json` | `--lock` 与 `--profile` **至少一个** | `nw/doctor.v1` |
| `ls` | `[recipes|components|sources|profiles|locks]`（至多 1 个域，不给 = `all`） | `--repo`("")、`--nw-home`("")、`--json` | 位置参数至多一个；未知域 → rc 3 | `nw/ls.v1` |
| `source register` | —（**不收位置参数**，给了 → rc 3） | `--repo <链接>`("")、`--kind skill|extension`("")、`--name`("")、`--category`（可重复，多级类别）、`--yes`(false 确认闸)、`--root`(".","本地仓库根")、`--json` | `--repo` 是**外部源仓库链接**（owner/repo｜https｜git@…，非 GitHub 拒）；`--root` 是本地仓库根（编目落 `<root>/catalog.json`）。注意与 validate/ls/bake 的 `--repo` **槽位含义相反** | `nw/source-register.v1` |
| `source recheck` | —（不收位置参数） | `--name`("",缺省全查)、`--root`(".")、`--json` | 只读报告漂移、永不写账；有漂移 → rc 3 | `nw/source-recheck.v1` |

解析特性（对 TUI 很重要）：

- 底层是 Go `flag`，但 `recipe new/bake`、`ls`、`source` 用 `parseInterleeded` 循环，所以 **flag 可以写在位置参数前后任意顺序**。
- `--json` 在**每个**子命令都存在且是 no-op：TUI 永远能拿到 JSON，不需要开flag。
- **`--help` 对子命令不是成功路径**：Go flag 碰到 help requested 报错 → rc 3 + 一份 `INPUT_INVALID` envelope。TUI 要自己带帮助文案/键位说明，别指望问 `nw x --help`。
- 无子命令（`nw` 裸跑）→ usage 落 stderr，**rc 3**；未知子命令 → rc 3 + usage。

## 2. 退出码表（`internal/envelope`，冻结码表 #9 D2）

字符串码 ↔ 数字码一一对应，未登记码按最保守 3 处理，绝不静默当成功。

| 数字码 | 字符串码 | 谁会返回 |
|---|---|---|
| 0 | `OK` | 全部成功（validate 有 warn 仍 0；up 缺 required env 降级仍 0） |
| 2 | `ENV_MISSING` | `$NW_HOME`/`$HOME` 不可解析（ls/up/doctor 步 0） |
| 3 | `INPUT_INVALID` | 参数错、输入不存在/不可解析、validate 有 fail 项、bake 校验失败、source recheck 有漂移 |
| 3 | `VERIFY_FAILED` | 资产 sha256 不符、锁定集 schema 不认（与 INPUT_INVALID 共用数字码 3，D2 有意为之） |
| 4 | `APPLY_FAILED` | 装配某步执行失败（chezmoi/铺组件等） |
| 5 | `LOCK_CONFLICT` | 同 profile 已有 up 在跑（profile 级并发锁） |
| 10 | `DEGRADED` | doctor：required env 缺失 / provider 未就绪（降级不阻断） |
| 11 | `DRIFT` | doctor：声明物被改或与锁定集不符 |
| 12 | `MANIFEST_BROKEN` | doctor：清单缺失或损坏（`ErrNotFound`/`ErrSchema`「需升级 nw」） |

子命令级口径：validate {0,3} · bake {0,2,3,4} · up {0,2,3,4,5} · doctor {0,10,11,12}，**doctor 取最重一档（12>11>10>0）**。

## 3. Envelope 形状（唯一 stdout 面，缩进两格 JSON + 末尾换行）

```
{
  "schema": "nw/<cmd>.v1",   // validate / up / doctor / bake / recipe-new / ls / source-register / source-recheck
  "nw": "0.1.0",             // CLI 自身版本（可 -ldflags 覆盖）
  "ok":   true,              // 铁律：ok 严格 == (exitCode == 0)
  "errors":   [ Error... ],  // 恒为数组（nil 会序列化成 null，被纪律禁止）
  "warnings": [ Error... ],
  "data": { ... }            // 各子命令形状见下；失败时 data 可能是 {} 或带证据
}
Error = { code, exitCode, message, hint?(omitempty), details?(map: 只放名字/布尔/路径/哈希，永不密钥值) }
```

各子命令 `data` 形状（字段名即契约，措辞不承诺）：

- **validate**：`{ target: "recipe"|"lock"|"recipe+lock"|"lock(+repo)", results[]:{id,status,message,path?}, summary:{total,pass,warn,fail,skip} }`
  - `status` 四态词 `pass/fail/warn/skip`（skip = 对当前 target 不适用，如无锁定集时跨文件项全 skip）。
  - `results[]` 顺序固定 = D6 表 1..27（`Recipe.Report` 注释），面板可稳定渲染。
  - 每个 fail 项在 `errors[]` 落一条 `INPUT_INVALID/3`，`details.id` = 检查项 id。
  - **warn 契约缺口**：`#9` 冻结 errors 五字段但没定义 warnings 的 code，「不发明新码」→ warn 只由 `data.results[].status=="warn"` + stderr 承载。**TUI 别在 `warnings[]` 里找 warn。**
- **up**：`{ profile, profileDir, runtimeKey, components[], skeletonFiles[], configKeys[], envPresent:int, envMissing[], convergeHash, lockId, upgraded:bool, installedAt }`
  - 降级成功（缺 required env）仍 rc 0，信号是 `data.envMissing[]` 非空。
- **doctor**：`{ results[]:{surface,status,message,details?}, exit:0|10|11|12, converged:bool, manifest?:{schema,lockId,profile,recipe:{name,version},version,runtimeKey,installedAt,convergeHash,engine} }`
  - 七个检查面（names are contract）：`manifest`/`consist`/`converge`/`config`/`env`/`runtime`/`provider`；四态词同 validate。env 面 required 且缺失 → 降级不 fail；runtime 面只报 pi 懒创建产物，缺失报 skip 不 fail；provider 面跑 `pi auth check --json`，**nw 自映射 `status`（`ready`/`not_ready`），不采信 pi 退出码**（pi not_ready 时自己也 rc 1）。
- **ls**：`{ domain:"all"|..., recipes[4], components[], sources[], profiles[], locks[] }`，五域恒存在、域空 = 空数组。
  - RecipeRow `{name,version,profile,path}`；ComponentRow `{name,version,path,depends[{name,version}],vendored,usedBy[]}`；SourceRow `{name,kind,category[],repo,entries[]}`；ProfileRow `{name,lockId,convergeHash,installedAt,recipe:{name,version},runtimeKey,state,degraded[]}`；LockRow `{lockId,recipe:{name,version},bakedAt,assets:int}`。
  - ProfileRow `state` 四态与 doctor 数字码一一对应：`ok`(0)/`degraded`(10)/`drift`(11)/`broken`(12) —— **ls 是 doctor 的廉价预览**（不重算收敛指纹）。
- **bake**：`{ lockId, recipe:{name,version}, profile, command:{file,sha256,bytes,shell}, invoke:{unix:{run,shell?},windows:{run,shell?}}, lock:{…内嵌锁定集…}, assets[]:{platform,tag,file,sha256,bytes}, bundle:{file,sha256,bytes,dir,files[]}, validate:{pass,warn,fail,skip,total} }`
  - 内嵌锁定集 `nw/lock.v1`：`{schema,lockId,bakedAt,recipe:{name,version},profile,runtime:{node,pi,chezmoi},components[]:{name,version?,landing:{path,kind},packages[],sha256},env[]:{name,required,usedBy[]},assets[]:{tag,file,sha256}}`。
  - `lockId` **由内容决定、不含 bakedAt** → 同输入重烤同锁；同 lock 连跑两次 up 的 manifest 逐字节一致（幂等铁律）。
- **recipe new**：`{ name, profile(缺省=名), path }`
- **source register**：成功 `{repo,kind,name,category[],commit,entries[],acceptance[]:{id,status,message,path?},catalogPath,written:bool,updated:bool,action}`；失败（rc 3 等）**data 仍带全套扫描/验收证据** + `errors[]`。验收 id 冻结 6 条：`register.repo_structure/frontmatter/name_unique/metadata_ignored/confirmed(+extension_manifest 仅 extension)`。
- **source recheck**：`{catalogPath,checked[]:{name,kind,repo,commit,entries},drift[]:{source,kind,entry,path,detail?}}`，OK=false + 一条 errors，rc 3。

## 4. 环境变量与宿主事实

| 变量 | 语义 |
|---|---|
| `NW_HOME` | 安装根；缺省 `$HOME/.nous-warehouse`；`--nw-home` flag 覆盖（up/doctor/ls 同序解析）。布局：`$NW_HOME/profiles/<profile>/`（该根部写清单 `.nw-manifest`）、`runtimes/<runtimeKey>/{bin,<树>}/`、`state/{chezmoi.toml,chezmoistate.boltdb,cache}`、`pruned/<ts>/`（旧组件退场区，不 rm） |
| `HOME` | `NW_HOME` 缺省时解析根；空则 `os/user` 回填，再空 → `ENV_MISSING`(2) |
| `GH_TOKEN` → `GITHUB_TOKEN` 兜底 | 外部源 bake 取回凭据（`github.com/JoeWest9966/nw/internal/source` TokenFromEnv）；**绝不落编目、绝不进锁定集** |
| `PI_CODING_AGENT_DIR` | doctor 探 provider 就绪时把 profile 目录指给 pi 的唯一手段（无等价 CLI flag）——TUI 调 doctor 时子进程环境要透传 |

## 5. 进度与实时性（对 TUI 体验最致命的一条）

**`nw up` 七步执行期间没有任何增量 stdout/stderr 进度行。** 引擎内部把 chezmoi 调用 stderr 截断 400 字节收进 details，跑完只在 stderr 打**一行**汇总 + 一份完整 JSON 上 stdout。结论：TUI 想要「游戏化进度条/步骤仪式」，要么

1. 接受「转圈 + 完成后一步展开结果」的粗粒度（最忠实契约）；要么
2. 自己用只读面（`ls` 的老清单 vs 新锁、`doctor` 的七面）在 up 跑的时候**并行探活**做合成进度；要么
3. 改 nw 加流式进度（**破坏 #9 冻结 stdout 面，禁区**——只能在 stderr 加，但那也动了 CLI 契约面，需开票）。

`--quiet` 目前是保留位（进度阶段没启用，警告永不被压）。

## 6. 幂等/复现事实（TUI 文案可直接引用）

- 同 `lockId` 连跑两次 up，manifest **逐字节一致**（`installedAt` 只在 lockId 变化时刷新；`env` 只记 `present` 布尔，永不记值）。
- 装配命令**非逐字节确定**：命令内嵌 `bakedAt`，所以同输入不同时刻重烤 diff 出来就一处时间戳；幂等靠 lockId 不靠命令字节（#15 家长裁案 B）。
- bundle 归档（tar.gz）字节可复现（成员排序、mtime/uid/gid=0）。
- 幂等的唯一提交点 = manifest 永远最后写（步 5 原子写）；上一轮没写完 = doctor 报 12。
- 引擎**不联网**：资产由 bake 下发的 bundle 提供；chezmoi/node/pi 全走可注入 `exec.Runner`（OSRunner，argv 逐元素不过 shell）。

## 7. 关键实测样本（截断）

### 7.1 `./nw validate --recipe docs/samples/recipe/recipe.yaml --repo .`（rc 0）

```json
{
  "schema": "nw/validate.v1", "nw": "0.1.0", "ok": true, "errors": [], "warnings": [],
  "data": {
    "target": "recipe",
    "results": [
      {"id":"lock.literal","status":"skip","message":"本次 target 未提供锁定集(lock.literal 属跨文件检查)"},
      {"id":"recipe.schema","status":"pass","message":"配方是 nw/v1 配方"},
      {"id":"recipe.runtime_pins","status":"pass","message":"node/pi/chezmoi 三个 pin 齐全"},
      {"id":"command.no_bashism","status":"skip","message":"…"}
    ],
    "summary": {"total": 27, "pass": 12, "warn": 0, "fail": 0, "skip": 15}
  }
}
```
stderr 同步人话：`nw validate: 12/27 pass, 15 skip`（fail/warn 项逐行列）。

### 7.2 `./nw doctor --profile no-such-profile-xyz`（rc 12，只读）

```json
{
  "schema": "nw/doctor.v1", "nw": "0.1.0", "ok": false,
  "errors": [{"code":"MANIFEST_BROKEN","exitCode":12,
              "message":"manifest: 清单不存在:该 profile 尚未完成装配或被清过",
              "details":{"profileDir":"/root/.nous-warehouse/profiles/no-such-profile-xyz"}}],
  "warnings": [],
  "data": {"results":[{"surface":"manifest","status":"fail","message":"…","details":{"profileDir":"…"}}],
           "exit": 12, "converged": false}
}
```
（注意 errors[].message 带 `surface: ` 前缀；`data.manifest` 缺省时整个键 omitempty。）

### 7.3 `./nw ls --repo . --nw-home /tmp/nw-home-probe-none`（rc 0）

```json
{"domain":"all",
 "recipes":[{"name":"research-desk","version":"1.0.0","profile":"research-desk","path":"recipes/research-desk/recipe.yaml"}],
 "components":[{"name":"scout","version":"1.0.0","path":"components/scout","depends":[{"name":"retention","version":"0.4.1"}],"vendored":true,"usedBy":["research-desk"]}],
 "sources":[], "profiles":[], "locks":[]}
```

### 7.4 `./nw validate --help`（rc 3）——「问帮助=错误路径」实证

stdout 一份 `INPUT_INVALID/3` envelope（`message:"参数解析失败:flag: help requested"`, hint「跑 nw validate --help 看用法」），用法表在 stderr。

## 8. 给 TUI 包装者最重要的 10 个事实

1. **契约面只有三样**：stdout 六字段 envelope、stderr 人话、rc 数字码。TUI 消费 JSON + rc，不要把 stderr 当数据面（但可以当「家长话」给人看）。
2. **`ok == (rc == 0)` 恒等**：成功与否只看这一个布尔，不必解析一堆字段；但**真像要看 `code` 字符串码**（3 同时是 INPUT_INVALID 和 VERIFY_FAILED，文案要分「写错了」与「被篡改了」）。
3. **`--json` 永远 no-op，JSON 永远是唯一 stdout**：包装层无条件 `--json` 可开可不开，行为不变；**永远别给人类可读模式开关的幻想**。
4. **问 `--help` 会拿到 rc 3 + 错误 envelope**：TUI 的帮助/探索文案必须内置（本文件第 1 节即权威表）。
5. **`--repo` 槽位在不同子命令含义相反**：validate/ls/bake 的 `--repo` = 本地仓库根；source register 的 `--repo` = 外部源仓库链接（本地根用 `--root`）。UI 表单必须按子命令分开建模。
6. **doctor 的数字码（0/10/11/12）与 ls `profiles[].state` 四态一一对应**：装备一个「概况页」只需一次只读 `ls`，不必扫全 NW_HOME、不重算收敛指纹；要证据再点开 doctor。
7. **up 没有任何实时进度**，且会持 profile 级并发锁（重入 = `LOCK_CONFLICT` rc 5）：UI 上「装配中」必须是**全局互斥态**，再次触发应给「忙」而不是排队转圈。
8. **validate 的结果顺序固定、四态词全仓通用**（pass/fail/warn/skip；`id`/`surface` 冻结）：一个结果列表组件可复用给 validate/doctor/source register 三处，只换 id↔surface 列名。
9. **失败也带证据**：doctor 失败带全部 results + manifest 摘要；source register 失败带扫描清单与 6 条验收；errors.details 只含路径/名字/布尔/哈希，**永不含密钥值** —— UI 可以放心原样展示 details。
10. **默认 `NW_HOME=$HOME/.nous-warehouse`，TUI 应当把它当一等公民**：默认值展示、`--nw-home` 覆盖、以及「NW_HOME 一键搬走全部家当」的故事线（profiles/runtimes/state 全在树下），是「设置」面板的自然内容；同时 `HOME` 不可解析 = rc 2 要能翻译成家长话。
