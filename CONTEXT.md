# Warehousetui

Warehousetui 是 Nous Warehouse 装配引擎（`nw`）的交互式驾驶舱：终端操作者通过它与引擎对话，面板自己不做任何装配决策。

## Language

### 引擎侧术语（沿用 Nous Warehouse 的词汇，本面板不另造词）

**nw / 装配引擎（engine）**：
Nous Warehouse 的命令行二进制，装配的唯一执行者；本面板只调用它，不复制它的逻辑。
_Avoid_: 后端、核心、服务

**配方（recipe）**：
声明一个派需要哪些组件、什么版本、什么模型配置的文件（`recipe.yaml`），仓库里的一等公民。
_Avoid_: 模板（chezmoi 侧「模板」指骨架 `.tmpl`，撞名）、清单（清单另有其物）

**组件（component）**：
一个有名字、有版本的可复用单元——skill / 扩展 / 模型配置 / agent 模板。
_Avoid_: 插件、模块

**锁定集（locked set / lock）**：
装配开始前就已经确定的那一组引用——哪些组件、什么版本、什么校验和；目标机拿到它就不再有任何选择。`lockId` 由内容决定，不含烘焙时刻。
_Avoid_: 依赖锁（太窄）、快照

**烘焙（bake）**：
作者侧把配方解析成锁定集、连同资产打成一条装配命令的过程（`nw recipe bake`）。
_Avoid_: 构建、打包

**装配（assemble / up）**：
目标机上把锁定集落地成一个派的过程；七步、幂等、可重入。
_Avoid_: 安装（太泛）、部署（会让人以为有服务端）

**装配命令（assembly command）**：
bake 产出的一条 POSIX sh 命令，在目标机上跑完即完成装配。
_Avoid_: 脚本、installer

**派（Pi）/ profile**：
装配出的成品——一个配置好的 pi 运行时实例；一个 NW_HOME 下可并存多个 profile。
_Avoid_: 实例、环境（env 在领域里另指环境变量）、配置

**清单（manifest）**：
装配结果的事实来源（`.nw-manifest`）：记录这个派是谁、何时、以哪个锁定集合成；装配过程中唯一个写入提交点，永远最后写。
_Avoid_: 缓存、状态文件

**收敛（converge）**：
同一目标态被装配多少次都停在同一个结果上；`convergeHash` 是它的指纹。
_Avoid_: 一致（太泛）

**NW_HOME**：
安装根，默认 `$HOME/.nous-warehouse`，布局 `profiles/runtimes/state/pruned`，「一键搬走全部家当」。
_Avoid_: 家目录（与 HOME 混淆）、安装目录

**数据仓根（data repo root）**：
配方 / 组件 / 编目的家，`--repo` 指向的目录；`catalog.json` 落在它的根部。面板的一等设置，与 NW_HOME 并列持久化；未设不猜、不给空列表，未设与不可用都给指路。
_Avoid_: 项目根、workspace

**作者侧 / 目标侧**：
生成装配命令的一侧 / 执行装配命令的一侧；本面板双视角并存，但 V1 的交互打磨优先目标侧。
_Avoid_: 客户端 / 服务端（我们没有服务端）

**外部源（external source）/ 编目（catalog）/ 条目（entry）**：
源在作者自己的 GitHub 仓、本仓库只存一条编目记录；一条记录可含多个条目。
_Avoid_: 市场、注册表

**预审（dry preview）**：
`nw source register` 不带 `--yes` 的那一段——扫描 + 跑验收 + 把拟落账记录送进 stdout，**文件不落、rc 0**；引擎设计内的只读态，不是错误态。
_Avoid_: 试运行、空跑

**确认闸（confirmation gate）**：
引擎侧 `--yes` 的语义：无它 = 拟落账不落盘。面板的图形确认必须承载同等信息量（摊开验收结论与拟落账记录），否则等于把闸门交给手滑。
_Avoid_: 二次确认、确认弹窗（那是交互形态，这是语义）

**envelope**：
`nw` stdout 的唯一输出面——`{schema, nw, ok, errors, warnings, data}`；铁律 `ok ≡ (退出码 == 0)`。
_Avoid_: 响应、输出

### 面板侧术语

**驾驶舱（cockpit）**：
本面板的角色——呈现引擎状态、编排引擎动作；不做装配决策、不重造引擎逻辑。
_Avoid_: 客户端（暗示另一套契约面）、GUI

**代跑（run on behalf）**：
面板自己把引擎命令跑完。准许清单（ADR-0009 新口径，**六类**）：只读三类 `ls` / `doctor` + `validate` / `source recheck`；写动作三类 `up` / `source register` / **编目册写动作**（`vps` / `key` / `subscription` 的 register 两段式 + rm，`tag` 的 add + rm）。`key get` / `subscription get` 一律只给建议命令——上游唯一回显本体的两个出口，面板不接。
_Avoid_: 自动执行、一键执行

**建议命令（suggested command）**：
面板给出可复制的命令文案、执行权留给用户——`bake` 等未获准代跑者的既定形态（ADR-0003）。
_Avoid_: 命令提示、待办

**跟随引擎（upstream-first）**：
本面板不拥有任何领域决策：CLI 契约、领域词汇、「准许代跑哪些命令」的边界,一律以上游 Nous Warehouse 仓为唯一真相源;上游改,面板跟,不自创、不背离、不复刻引擎逻辑。
_Avoid_: 兼容、适配（太被动;适配暗示以面板为主位）

**契约守卫（schema guard）**：
面板只认自己认识 `data` 形状的 envelope schema；不认识时合成「需升级 nw」五字段证据（带收到的 schema 原文与引擎版本），`data` 不解码、不按旧形状降级解析。这是面板自己的契约知识,与引擎能力探测是两回事。
**认识形状 ≠ 准许代跑**：契约守卫答「我听不听得懂」，能力探测门答「它会不会做」，两层分工不被混淆（ADR-0009）。
_Avoid_: 版本校验、兼容层

**能力探测门（capability gate）**：
面板问引擎「你有什么」：一次无害调用,答案来自引擎自己的回答;缺的能力入口禁用并指路,面板绝不自己造替代品、也绝不替引擎回答。
**入口清单由面板定义,可用性由引擎决定**：清单（外部源登记 / 外部源复核 / 配方烘焙 / 配方新建 / vps / key / 订阅 / 标签）是面板对操作者的承诺,每加一项就多一次无害调用、多一行设置屏,所以新增入口必须在调研票 #51 的差异表里点名过,不许谁想加就加;每一项的探针还只能指向上游真实存在的命令面——`--category` 是 `nw source register` 的旗标、不是子命令,所以「类别」不配占一个能力项（#63）。
_Avoid_: 特性开关、能力表（不建本地常量表）

**屏幕（screen）**：
用户停留的界面。原 V1 五屏（概览、装配、健康、配方、设置）；#53 起加**编目**屏成六屏——四组编目命令（vps / key / subscription / tag）的驾驶舱落点，第一本是 vps，后三本只换数据源与登记规格，不换屏与骨架（`screens::catalog`）。
_Avoid_: 页面、视图（与代码里的 view 混淆）

**HUD**：
常驻屏幕底部的键位提示栏，游戏式操作的主入口。
_Avoid_: 状态栏、工具栏

**发射仪式（launch ritual）**：
装配执行前的确认交互——面板先展示即将发生的变更，用户按键确认后才真正调用引擎。
_Avoid_: 确认弹窗（太窄）

**装配态（assembling）**：
全局互斥状态；任一装配进行中，面板不再发起新的装配（引擎侧同样是 profile 级并发锁）。
_Avoid_: 忙、loading

**结果四态（pass / warn / fail / skip）**：
引擎检查项的通用状态词，一个列表组件贯穿 `validate` / `doctor` / `source` 三处。
_Avoid_: 成功 / 失败（与 `ok` 铁律混淆）

**降级（degraded）/ 漂移（drift）/ 破损（broken）**：
doctor 的 10 / 11 / 12 三档结果态——装好了但没完全好（required env 缺失或 provider 未就绪）/ 声明物被改动 / 清单缺失或损坏。
_Avoid_: 警告、错误（与 envelope 的 warn / fail 是两层，别混）

**证据（evidence）**：
`errors[]` / `warnings[]` 条目的五字段内容——`code` / `exitCode` / `message` / `hint` / `details`；面板原样呈现，不猜测、不改写、不回显密钥。
_Avoid_: 报错信息、日志

**指路屏（signpost）**：
找不到 `nw` 二进制时亮相的引导屏：说清面板需要什么、去哪装、怎么指，而不是报错了事。
_Avoid_: 错误页

**ls 五域**：
`nw ls` 的五个恒存在域——recipes / components / sources / profiles / locks；侧边导航与概览屏的骨架。
_Avoid_: 分类、清单（与 manifest 撞名）

### 编目册（catalog books）

**编目（catalog）**：
数据仓对一类**条目**的记录，一个 JSON 文件。上游现有五本，都住在数据仓根部：`catalog.json`（外部源，skills[] + extensions[]）/ `vps.json`（machines[]）/ `keys.json`（entries[]）/ `subscriptions.json`（entries[]）/ `tags.json`（tags[]）。机器写、确定性排序、永不手改。
_Avoid_: 注册表、市场

**条目（entry）**：
编目里的一个单元。外部条目住 `catalog.json`、vps 条目住 `vps.json`（kind `vps`）、key 条目 kind `github-key`、订阅条目 kind `subscription-url`；条目可打标签。
_Avoid_: 记录、行

**登记（register）**：
把一条条目写进编目（`register` 的动词）；写入口唯一，要改就重跑 register 覆盖写。注意与上游另一个词**登记包**（写 `settings.json` 的 `packages`）不是一回事。
_Avoid_: 注册、录入

**投影（projection）**：
`ls` 给出的可读视图，刻意不是编目形状本身（去掉本体槽）。key 的投影没有 secret 槽、也不印指纹；subscription 的投影没有 url 槽。
_Avoid_: 视图、摘要

**闸（gate）**：
register 的逐条校验，任一不过 = 一个字节都不落。vps 五道 / key 三道 / subscription 四道 / tag 四道，`data.gates[]` 恒 N 行、id 是稳定面、三态 pass/fail/skip（复用「结果四态」词）。
_Avoid_: 校验项、检查

**反查（reverse lookup）**：
按标签列出条目，只走 `nw tag ls [<标签>]`；不开 `nw ls` 第六域、不掺过滤。
_Avoid_: 搜索、检索

**密钥引用（key ref）**：
vps 登录里指向机密的 **opaque 串**（`$ENV` / `!command` / ssh-agent / 密钥路径），nw 存原样、不解析、不连接；**指针不是密钥**，所以 `nw vps` 没有密码参数面。
_Avoid_: 密钥、凭据引用

**到期标记（expiry marking）**：
订阅的 `status` / `daysLeft` 三态 active / expiring / expired，**由上游 `status.go` 读取时算好、随回音下发，永不写进文件**；面板只做状态映射，本地零日期运算（ADR-0009 §3）。7 天阈值是上游的、不配置化，只标记不治理（不改退出码、不提醒、不靠 ANSI 颜色）。
_Avoid_: 过期、续期

### 尺寸与呈现

**响应式（responsive）**：
面板随终端尺寸变化而改变自身外观的呈现方式，铁律是只改布局与排布、永不缩放字号；最小支持到 66×50。
_Avoid_: 缩放、自适应字号、弹性布局（Constraint 的 Min/Percentage 只是实现手段，不是这个术语本身）

**断点（breakpoint）**：
常畅态与紧凑态相互切换的那个终端宽度阈值。
_Avoid_: 阈值（太泛）、media query（Web 借词）

**常畅态（comfy）**：
宽度达到断点时的呈现态——竖左导航轨、内层栏展开，即面板既有的完整外观。
_Avoid_: 宽态、桌面态

**紧凑态（compact）**：
宽度低于断点、但仍不小于 66×50 时的呈现态——左导航折成横向屏 Tab、内层栏收起、顶栏与 HUD 精简，以更稠的排布在更小终端里装下同样的内容。
_Avoid_: 窄态、精简模式、小屏模式
