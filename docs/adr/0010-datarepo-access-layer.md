# 数据仓/安装根访问层(`datarepo`)与装配历史

上游既有事实:`$NW_HOME` 的布局是 `profiles/<派>/.nw-manifest`(清单,引擎侧**永远最后写**的唯一提交点)、`locks/`、`pruned/<时间戳>/`(旧组件退场区,不 rm)、`runtimes/`、`state/`;数据仓根另在 `--repo` 指的目录,住五本编目 JSON(`catalog.json` / `vps.json` / `keys.json` / `subscriptions.json` / `tags.json`)。spec #94 起面板要**不调引擎、直接读这些文件**(装配历史 / 回收站 / 派总览 / 日志存档),而在本 ADR 之前,全仓唯一拼 `$NW_HOME` 路径的地方只有 `App::lock_carrier` 一行(`app.rs` 的 `format!("{}/locks/{lock_id}.nwlock", expand_tilde(nw_home))`)。

本 ADR 裁两件事:① 这些读文件的动作收进哪一个模块、契约长什么样;② 装配历史这个新视图挂在哪、边界在哪。

Status: accepted

> **决策记录**:spec #94(驾驶舱增强)的第一张消费票 = 工单 #101。上游依据:`docs/research/nw-cli-surface.md` §4 的 `NW_HOME` 布局表、`internal/manifest` 的原子提交点、`docs/real-first-run/envelopes/doctor-profile.json` 里 `data.manifest` 的真样本(schema `nw/manifest.v1`)。

## Considered Options

- **各屏自己拼路径**(概览拼 `profiles/`、装配屏拼 `pruned/`、设置屏拼 `logs/`…):排除。路径知识散落 N 处,上游一改布局要改 N 处——而「跟随引擎」纪律(ADR-0007)要求只改一处;更糟的是每一处都会各自决定「读不到时显示什么」,空列表 / 报错 / 指路三种话术必然漂。规格故事 27(屏幕层永不直接拼 NW_HOME 路径)正是这条的反面。
- **塞进 `engine` 层**:排除。`engine` 的定义是「调 `nw` 子进程的唯一接缝」(`engine/mod.rs` 头注),读文件不是调子进程;混在一起,"面板碰文件系统"就失去一个可指名的边界,ADR-0009 的机密纪律也少一道可检查的闸。且 `engine` 层今天的布局知识只有 `<nw_home>/nw` 一处(发现引擎二进制),把整套 `NW_HOME` 布局搬进去是给它换定义。
- **新立 `datarepo` 模块,与 `engine` 并列**:采纳。第二条访问层,唯一拥有 NW_HOME / 数据仓文件路径知识,对外只暴露意图级方法。这一接缝同时服务本 spec(读侧)与作者侧 spec #95(写侧;工单 #118 收窄后只剩配方编辑器)——写侧落地时,**写动作的方法加在同一个模块、同一篇 ADR 的修订里**,不另起一个"数据仓写层"。

## Decisions

### 1. 模块边界:两条访问层,分工不混

- `engine`:调 `nw` 子进程的唯一接缝(既有,不变)。
- `datarepo`:不调引擎、直接读文件的唯一接缝(本 ADR 立)。**屏幕层与 app 层不得绕过它碰文件系统**——包括 `fs::read_dir` / `fs::read_to_string` / 路径拼接。

既有唯一一处 app 层拼 `$NW_HOME` 的地方(`App::lock_carrier`,锁定集载体落点)已迁移为 `datarepo::lock_carrier` 的转口,行为逐字节不变(`app.rs` 既有两条载体落点测试原样绿)。**这是本次迁移的全部存量**:新增的读文件动作从第一天就走接缝,不欠新债。

`~` 展开器全仓仍只有一份(`engine::discover::expand_tilde`);`datarepo` 复用它,不写第二个。`datarepo` 因此**依赖** `engine`(借展开器与 `ManifestBrief` / `Evidence` 两个既有类型),而 `engine` 不认识 `datarepo`——依赖单向,不成环。

### 2. 意图级方法清单(接缝契约)

对外只暴露"要什么",不暴露"去哪个目录取哪个文件"。方法名即意图;目录与文件名是模块私事。

| 意图级方法 | 形状 | 状态 |
|---|---|---|
| `lock_carrier(nw_home, lock_id)` | `-> String` | **#101 已落地**(存量迁移) |
| `home_cause(nw_home)` | `-> Option<HomeCause>` | **#101 已落地** |
| `list_profiles(nw_home)` | `-> Vec<String>`(确定性字典序) | **#101 已落地** |
| `manifest_path(nw_home, profile)` | `-> PathBuf` | **#101 已落地** |
| `read_manifest(nw_home, profile)` | `-> ManifestStatus`(四分类) | **#101 已落地** |
| 回收站枚举(`pruned/<ts>/` 下的旧组件) | 待 #102 钉形状 | **后续票** |
| 装配日志落盘与回看(`logs/<lockId>/`) | 待 #103 钉形状(唯一写例外,见 §6) | **后续票** |
| 派枚举 + doctor 档位摘要的数据面 | 待 #107 钉形状 | **后续票**(派枚举已在此) |
| 引擎版本对照的数据面 | 待 #108 钉形状 | **后续票** |
| 数据仓引用图(配方 × 组件目录) | 待 #116 钉形状 | **后续票** |

**后续票不在这里留 stub**:`#![allow(dead_code)]` 是施工期放开,不是留占位函数的理由。契约以本表为准,各票落地时往表里填行、往 ADR 修订记录里记一笔。

### 3. 分类,不是 `Err`

`read_manifest` 返回四种分类,**没有第五种**,也绝不返回 `Err`:

| 分类 | 判据 | 屏上落点 |
|---|---|---|
| `Ok(ManifestArchive)` | 文件在、JSON 合法、`schema` 认得 | 全字段明细 + 落点 + 原文 |
| `Missing` | 文件不在(`NotFound`) | 「还没装(清单缺失)」,warn |
| `Broken { reason }` | 读不出 / 不是合法 JSON / 缺 `schema` 字段 / 认得 schema 但字段形状不对 | 「清单破损」+ 原因,fail |
| `UnknownSchema { guard }` | JSON 合法、`schema` 不认得 | 契约守卫五字段证据,warn |

**为什么是分类而不是错误**:一条坏记录只该坏它自己那一行,不该炸掉整屏(规格故事 4)。屏幕层把四种分类映射成四种呈现,`datarepo` 不决定颜色、不决定措辞——接缝答"读到了什么",不答"该画多红"。

`ManifestArchive` 的字段形状沿用引擎侧**已有的** `ManifestBrief`(`engine/envelope.rs`,doctor 的 `data.manifest` 同一份契约),不另立一套字段名;外加 `path`(只读呈现)与 `raw`(回车看原文)。不派生 `PartialEq`/`Eq`:里面嵌的引擎解析视图天生不可比,测试按字段断言。

### 4. 清单 schema 契约守卫:同款话术,同一处工厂

`.nw-manifest` 也是上游契约面,面板只认 `nw/manifest.v1`。收到不认识的 schema 时合成与 `Envelope::apply_schema_guard` **同款**的五字段证据(`code = SCHEMA_UNKNOWN` / `exitCode` / `message` / `hint` / `details`),措辞工厂收在 `engine::envelope::schema_guard_evidence` 一处(参数 `subject` 区分 envelope / 清单),不复制第二份文案。

两处刻意的差别:

- **`exit_code` 留空**(`None`)。读文件没有引擎退出码可报,编一个 rc 冒充引擎说的是造假——envelope 那条有真 rc 是因为 rc 就在手边。
- **缺 `schema` 字段走 `Broken`,不走守卫**。空 schema 拼不进"面板不认识 schema X"这句话;它是"这份文件不是一份清单",不是"这是一份新版的清单"。

不猜、不按旧形状降级解析:与引擎侧对未知 envelope 的裁法(`EngineOutput::evidence()` 对 guard 只回 guard)同一立场。屏上明写"不按旧形状降级解析"。

### 5. 安装根三态指路:没有"空列表"

派枚举在安装根不在时天然返回空,而"空"在屏上与"一个派都还没装"长得一样——操作者会以为面板读了、只是没数据(规格故事 28:不出现拼错路径读空目录的假象)。所以:

- `datarepo::home_cause` 单独回答"安装根可用吗"(与 `App::data_root_cause` 并列的**第二条**三态门,判定只用一次 `fs::metadata`,不遍历)。
- `HomeCause::{Missing, Unreadable{reason}}` 各有各的话术,共用渲染件 `screens::home_signpost`(与 `data_root_signpost` 同口径、同文件)。
- 不可用时历史视图整块换成指路,**连列表都不画**;命中区随之不产(`layout::assemble_hits` 同一判据)。

数据仓根的三态(`DataRootCause`)与安装根的两态不合并:一个管"五域读不读得到",一个管"清单存档读不读得到",混成一个枚举会让两种不可用互相冒充。

### 6. 只读铁律,以及 #103 的唯一写例外

本视图**只读**:没有任何写动作、没有任何引擎调用(读盘只经 `datarepo`)。面板永不写 `.nw-manifest`——上游把它当唯一提交点、永远最后写,面板插一脚就是第二个真相源。

`NW_HOME` 下唯一的写例外是 **#103 的装配日志落盘**(`logs/<lockId>/`):那是 stderr 的旁路副本,不是状态。#103 落地时在本 ADR 修订记录里钉死路径规则与原子性,并把"作者侧 spec #95 永不写 NW_HOME"的禁写口径与它对齐——**例外只有这一处,且必须记账**。

### 7. 装配历史是装配屏的次级视图,不是新屏

- 挂在装配屏(`screens::assemble`),不新开顶级屏:装配屏已有"选择器 / 发射仪式 / 运行中 / 结果"四态纵深,历史是第五态(`app.as_history` 旗)。屏枚举 `Screen::ALL` 一个字节不动,左导航与紧凑 Tab 不动。
- **"历史"的诚实形状**:上游每个派只保留**最近一次**收敛的清单(原子提交点,`installedAt` 只在 lockId 变化时刷新),没有历史目录。所以本视图列的是"各派最近一次收敛的存档",按派分组;上游若哪天立了存档目录,`datarepo` 的 `read_manifest` 是唯一要改的地方,屏幕层不动。屏上不写"历次收敛"这种上游没给的话。
- 两档几何:常畅 55/45(派清单 + 选中派明细),紧凑上下两块。字段按 `clip` 截断、**从不折行**——折行会多占一行,命中区整体错位(ADR-0005 渲染与命中共用一份几何)。
- 键位:`h` 进出、`↑/↓` 选派、`Enter` 看清单原文、`/` 本地过滤、`r` 重读快照、`Esc` 两级(先清词再退视图)。装配在途时整屏归运行态,次级视图的键位一律无分支;`up` 一下发即清场(`note_engine_dispatch`)。
- 原文浮层是**顶层浮层**(与帮助 / 发射仪式同级),`Enter`/`Esc`/`q` 或点任意处关闭;看原文不写任何选中态(规格故事 3 是"看",不是"挑")。
- **#102 回收站 / #103 存档日志的挂点**:明细末尾一行 dim 说明"随后续票落地",**不画假动作段**——点不到一个当前不存在的动作,比点得到更糟(#67 纪律)。

### 8. 后续票怎么接着长

- 加一个读文件意图 = 在 `datarepo` 加一个意图级方法 + 在 §2 表里填一行 + 写它自己的测试(fixture 目录树,断言分类返回,不测遍历顺序)。
- 加一个次级视图 = `App` 上加一个旗/枚举(照 `HealthMode` / `as_history`)+ `render_*` 里 match 分画 + `key_*` 里 match 分派 + `layout::*_hits` 按同一判据产命中区 + 帮助对照表与 README 同步(双向测试守着)。
- 安装根指路共用 `screens::home_signpost`,不各写一份。

## Consequences

- **`datarepo` 是全仓第二个"拥有路径知识"的模块**,也是唯一拥有 NW_HOME / 数据仓布局的那一个。`grep -rn "profiles\|pruned\|nwlock" src/` 今天只该在 `datarepo`(与 `engine::discover` 的 `nw` 二进制一处)里命中——这条可 grep,是接缝纪律的验收尺。
- **`App::lock_carrier` 从"拼路径的人"变成"转口的人"**:app 层再没有一处 `$NW_HOME` 拼接。存量行为逐字节不变(两条既有测试原样绿),这次迁移买的是"以后新增读文件动作时,屏层想拼也无处可拼"。
- **契约守卫的话术只有一份**:`engine::envelope::schema_guard_evidence` 一个工厂,envelope 与清单两个调用点。将来第三种契约面(如 `nw/lock.v1` 文件)照样走它,`subject` 参数区分。
- **`engine::mod` 新导出一个 `manifest_schema_guard`**:它是给 `datarepo` 用的,但住在契约模块里——证据五字段的形状归契约模块所有,消费方不自己捏 `Evidence`。
- **`App` 新增六个槽**(`as_history` / `as_history_rows` / `as_history_sel` / `as_history_filter` / `as_history_filter_on` / `as_history_raw`)与两条三态门方法(`home_cause` / `load_history` / `history_visible` / `history_total`),`App::assemble` 的初始化清单同步补行。
- **`ui::Prep` 多一个 `HistoryView` 布场口**:帮助对照表要能布出"装配历史在场"这个上下文,否则 `/` 与 `r` 两个真实分支在帮助↔实现双向测试里无法被如实记账(与既有的 `running` 旗同级,不是新浮层)。
- **新增键位全部三处对齐**:实现 + `ui::HELP_ROWS` + `README.md` 六屏表(双向测试守着,漏一处就红)。
- **本 ADR 服务两张 spec**:读侧(#94,本票)与写侧(#95)。写侧落地时不新立"数据仓写层",在本 ADR 填 §2 的表、记修订,并把"永不写 NW_HOME"的禁写口径与 #103 那一条写例外明确并列。
