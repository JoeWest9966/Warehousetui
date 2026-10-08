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

**数据仓根（data root）**：
数据仓目录——配方 / 组件 / 编目的落点，`nw ls --repo` 的显式指向；面板的一等设置，与 NW_HOME 并列持久化；未设不猜、不拿启动时的当前目录兜底，未设与不可用都给指路。
_Avoid_: 当前目录（被移除的兜底）、仓库根（太泛）、仓库路径

**作者侧 / 目标侧**：
生成装配命令的一侧 / 执行装配命令的一侧；本面板双视角并存，但 V1 的交互打磨优先目标侧。
_Avoid_: 客户端 / 服务端（我们没有服务端）

**外部源（external source）/ 编目（catalog）/ 条目（entry）**：
源在作者自己的 GitHub 仓、本仓库只存一条编目记录；一条记录可含多个条目。
_Avoid_: 市场、注册表

**envelope**：
`nw` stdout 的唯一输出面——`{schema, nw, ok, errors, warnings, data}`；铁律 `ok ≡ (退出码 == 0)`。
_Avoid_: 响应、输出

### 面板侧术语

**驾驶舱（cockpit）**：
本面板的角色——呈现引擎状态、编排引擎动作；不做装配决策、不重造引擎逻辑。
_Avoid_: 客户端（暗示另一套契约面）、GUI

**屏幕（screen）**：
用户停留的界面；V1 五屏：概览、装配、健康、配方、设置。
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
