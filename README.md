# Warehousetui

`nw`(Nous Warehouse 装配引擎)的交互式驾驶舱:把 `validate` / `up` / `doctor` / `ls` 等命令流包装成「看得见状态、点得出动作、像玩游戏一样顺手」的终端面板。

引擎侧契约冻结——本面板是**纯消费者**:fork `nw` 子进程、解析 stdout 的 JSON envelope、把 stderr 的人话流变成实时日志。面板自己不做装配决策。

## 安装

```bash
curl -fsSL https://raw.githubusercontent.com/JoeWest9966/Warehousetui/main/install.sh | sh
```

> 本仓是**私有仓**:匿名请求这条 raw URL 会 404。带凭证装:
> `curl -H "Authorization: token $GH_TOKEN" -fsSL https://raw.githubusercontent.com/JoeWest9966/Warehousetui/main/install.sh | sh`
> (install.sh 随后也要带同一个 token 才能读到 release 资产;`export GH_TOKEN=…` 即可,install.sh 透传环境。)

默认装到 `~/.local/bin/nwt`。装完敲 `nwt` 没反应,就是它不在 `PATH`:

```bash
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.bashrc   # 或 ~/.zshrc
```

- 先看计划再装:把脚本下载下来跑 `sh install.sh --dry-run`(只打印装什么、装到哪,不落盘)
- 装到别处:`sh install.sh --prefix DIR`,或 `PREFIX` 环境变量
- 升级:重跑最上面那行,覆盖即升级
- 安装前强制校验 sha256,对不上就取消,不留半个二进制

Windows:去 [releases](https://github.com/JoeWest9966/Warehousetui/releases) 手动下载 `nwt_<版本>_windows_x86_64.zip`。支持级是「编译可运行」([spec #10](https://github.com/JoeWest9966/Warehousetui/issues/10) 钉死的 R3 决定;ADR-0002 只定 Rust/ratatui 技术栈,不涉支持级),不提供 installer;资产与校验细节见 [docs/release.md](./docs/release.md)。

> 预编译二进制随 tag 发布(当前最新 v0.3.0:**linux x86_64 / aarch64 + windows x86_64** 三个包;macOS 暂缺,请源码构建);安装脚本默认取 latest,也可 `--version` 钉版本。命名契约与已知取舍见 [docs/release.md](./docs/release.md)。

## 从源码构建

```bash
cargo build --release
./target/release/nwt                      # 自动发现 nw(显式 > 配置 > PATH > $NW_HOME/nw)
./target/release/nwt --nw-bin /path/nw    # 找不到引擎时显式指路
```

终端至少 66×50;宽 ≥ 80 为常畅态(竖左导航),窄于 80 自动切紧凑态(横向屏 Tab),只改排布不改字号;真彩色自动探测,SSH 下自动降级 256/16 色。发布流程与本地打包复现见 [docs/release.md](./docs/release.md)。

## 六屏

| 屏 | 干什么 | 键 |
|---|---|---|
| 概览 | 警示带(点名到期订阅)+ 派四态表 + 四域计数卡 + 指令栏 + 最近日志 | `↑/↓` 选派,`r` 刷 ls,`a`/`h`/`l`/`v` 跳屏 |
| 装配 | 选锁定集 → 发射仪式 → 七步 + stderr 实时日志;**装配历史**(各派清单存档,只读) | `↑/↓` 选锁,`Enter` 发射仪式,再 `Enter` 发射,`x` 中止在途装配,`h` 翻装配历史(再按退出),历史里 `/` 过滤、`r` 重读快照、`Enter` 看清单原文 |
| 健康 | doctor 七面卡片 / validate 27 项清单 + 证据五字段 | `r` 复核,`m` 切模式,`Enter` 复核选中条,`↑/↓` 翻阅 |
| 配方 | 五域两栏只读浏览(recipes/components/sources/profiles/locks) | `h/l` 切域,`↑/↓` 选行,`c` 取用烘焙建议命令(只读),`/` 过滤五域(本地筛选),`r` 登记外部源(外部源域) |
| 编目 | 四本编目册(机器 / 标签 / 订阅 / 钥匙,`m` 换册):登记 / 看编目 / 删除 / 反查 / 册内过滤 | `m` 换册,`r` 登记,`l` 重读编目,`↑/↓` 选行,`d` 删除(先确认;键位行「d 删除」段可点),`g` 取回本体的建议命令(订阅 / 钥匙册;键位行「g 取回」段可点),`Enter` 反查选中的这条(机器册问什么标签引用了它 / 标签册反查选中的那条),`/` 册内过滤(本地筛选,Esc 清词) |
| 设置 | 引擎发现状态、NW_HOME、数据仓根、主题、配置入口、引擎版本对照 | `n` 轮换 NW_HOME,`i` 手填 NW_HOME,`d` 手填数据仓根,`e` 拉起 $EDITOR,`m` 切换鼠标捕获,`t` 主题,`v` 引擎版本对照(对照 release 仓最新 tag,落后只给升级指路不代跑) |

通用:`j/k` 切屏 · `Enter` 确认 · `Esc` 返回 · `?` 帮助 · `q` 退出(装配态禁用)。

### 鼠标(默认开捕获)

- 点左导航(常畅态)/顶部屏 Tab(紧凑态)切屏;点列表行=选中(与 `↑/↓` 同语义,不触发破坏性动作)。
- 可选列表上滚轮上下移选中;装配 stderr 实时日志区滚轮回看早先输出。
- 发射仪式「Enter 发射 / Esc 返航」两段可点;帮助浮层点任意处关;浮层打开时点击不穿透底层。
- 装配运行中「`x` 中止」段可点 = 进程级真止在途子进程(与按 `x` 等价,只在真有 `up` 在途时在场;和发射前「Esc 返航」是两回事,见下)。
- 编目册正文键位行的「换册 / 过滤 / 反查」三段可点(点它 = 按 `m` / `/` / `Enter`;反查只机器册有,结果是一层薄浮层,`Esc` 或点任意处关闭;过滤段四本册都画得出、点得中(两档都在场,键位行放不下的段按 #67 纪律整段不收);反查段要标签组能力,缺能力时画成禁用态(dim),点下去仍给指路、不发命令)。
- 订阅 / 钥匙册键位行的「g 取回」段可点 = 按 `g`(当前选中那条的建议命令收进日志,执行权留给操作者;面板不代跑 get、不碰剪贴板。只这两本册的非空册体有这一段,空册与登记 / 删除浮层态都没有)。
- 四本编目册键位行的「d 删除」段可点 = 按 `d`(选中行 → ADR-0009 图形确认闸 → 确认才 `rm`;标签册段面是「d 摘/删」,点它进同一条摘引用 / 删标签表单流。空册与登记 / 删除浮层态都没有这一段)。
- 概览警示带点名 expiring / expired 的订阅(三态与剩余天数由引擎 `subscription ls` 回音给,面板不算日期;active 不占带)。点那一条 = 跳到订阅册并选中它——册内过滤把它筛掉时只给一行指路,不硬选中、也不新造禁用态。
- 捕获默认开启的代价:终端原生「拖拽选中 / 右键复制」需按 **Shift**。要还给终端:`--no-mouse` 旗标启动,或设置屏按 `m` 关(写入 `config.mouse_capture`,下次启动生效)。

## 行为纪律

- **降级 ≠ 失败**:required env 缺失、provider 未就绪是 amber 横幅;`skip` 是正常态(pi 懒创建)。
- **证据原样**:errors/warnings 五字段(code/exitCode/message/hint/details)原文呈现,建议动作只给命令不代跑(ADR-0003)。
- **永不回显密钥**:env 只显示 present 布尔;details 可含任意 JSON,但面板不做值猜测。
- **装配幂等**:`up` 可取消、可重入;并发锁(rc 5)明示「同一派已在装配」。取消有两档,话术不许混:**发射前 `Esc` 返航** = 命令还没下发,什么都没发生;**发射后 `x` 中止** = 在途子进程进程级真止,可能停在未收敛中间态——面板不宣布成功也不假装失败,stderr 日志一行不清(停在哪一步看它),重跑同一条装配命令即可收敛,收敛与否以健康屏 `doctor` 复核为准。

## 开发

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets     # 单测:契约解析 / 发现策略 / 六屏快照 / 装配四路径 / 命中区与响应式两档 / 数据仓根三态指路 / 登记两段式 / 80×30 整帧 golden
cargo build --release
```

本仓不跑 CI(GitHub Actions 被账号级计费挡住全部 job,业主裁定不付钱,见 [docs/release.md](./docs/release.md));闸门在本地,两条路任选:

- **自动(推荐)**:`git config core.hooksPath githooks` 启用 `githooks/pre-push`——每次 push 前自动跑齐上面四项,红了就挡住(恢复默认:`git config --unset core.hooksPath`)
- **手动**:改动合入前,上面四项在开发机上跑齐即闸门

已知的**环境性红**(非缺陷)用 `NWT_SKIP_GATE=1 git push ...` 逃逸:discover 测试在装有真 `nw` 的机器上会红。另注意一条只在特定平台暴露的回归历史:建议命令测试的 OS 临时目录长度依赖(macOS/Windows 红)。

发布是同一套闸门的另一面:`scripts/release.sh <版本>` 一条命令跑完 [docs/release.md](./docs/release.md) 的七步(本地闸门 → 打 tag → 三平台交叉构建 → 打包校验 → `gh release create` → 核对),`--dry-run` 先体检。

## 文档

- 领域词汇:[CONTEXT.md](./CONTEXT.md)
- 架构决策:[docs/adr/](./docs/adr/)(0001 子进程契约 / 0002 技术栈 / 0003 动作边界 / 0004 响应式双态 / 0005 鼠标命中区 / 0006 外部源登记两段式 / 0007 面板跟随引擎 / 0008 跟随引擎三层 / 0009 编目册写动作边界与机密纪律 / 0010 数据仓安装根访问层与装配历史)
- 调研底稿:[docs/research/](./docs/research/)(nw CLI 契约面 / 用户旅程 / 环境验证)
- 发布矩阵:[docs/release.md](./docs/release.md)(安装 / 校验 / 打 tag 发布)
- 工单:[GitHub Issues](https://github.com/JoeWest9966/Warehousetui/issues)(wayfinder 蓝图 #1)
