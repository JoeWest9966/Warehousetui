# 真机首跑报告(V1 发布实证 · 裁定 A)

**日期**:2026-10-09 · **执行机**:本机(Linux x86_64)· **引擎**:`nw` 0.1.0(`/usr/local/bin/nw`,release `nw/v0.1.0` 资产同源)· **数据仓**:`/opt/Data-001`(私有仓 `JoeWest9966/Data-001`,main @ README 落地约定修复后)· **面板**:`nwt` release 构建,源码 = Warehousetui main @ `fa81903`(PR #44 / #45 / #46 合并后)。

**结论:发布实证成立。** 面板在**真锁定集**(引擎从真数据仓烤出、与 2026-10-05 引擎侧首跑发布物逐位同源)上完成了一次**真装配**:`nw up` rc 0、`.nw-manifest` 逐字节收敛;五屏全部走过真数据(真 ls / 真 doctor / 真 validate / 真 recheck / 真 bake 建议命令)。首跑暴露 4 个真机缺口(G1–G4、G5),全部在本次范围内修掉(PR #44 / #45 / #46 + Data-001 PR #5)。v0.2.0 以本报告 + 三平台 CI 绿为发布证据。

## 1 · 四相过程

```
相 A 作者侧   真数据仓 + 真资产 → nw recipe bake(复现发布物 lockId)
相 B 目标侧   装配命令真跑(资产 + bundle 真下载校验,up + doctor)
相 C 面板侧   nwt 在 pty 里真人操作 12 幕(本目录 frames/)
相 D 结论     缺口处置 + 已知未知 + 复跑入口
```

## 2 · 相 A:作者侧真 bake —— 复现性实证

```bash
gh release download nw/v0.1.0 --repo JoeWest9966/Nous-Warehouse \
  --pattern '{node,pi,chezmoi,nw-linux-amd64}' --dir <资产目录>
nw recipe bake recipes/research-desk/recipe.yaml --out assembly-command.sh \
  --assets-dir <资产目录> --bundle-repo JoeWest9966/Nous-Warehouse --repo /opt/Data-001
```

- 产出:**lockId `research-desk-1.0.0-360ae68`** —— 与 2026-10-05 引擎侧首跑发布物**逐位一致**(锁定集由内容决定,不吃 bakedAt;内嵌锁定集除 `bakedAt` 外逐字节相同,见 `envelopes/bake.json`)。
- validate 前置:12/27 pass · 0 fail · 15 skip(rc 0)。
- **附带发现(数据仓侧,已修)**:直接 bake 曾被拒 —— `component.landing` 1 项 fail。根因:数据仓迁入私有仓时丢了 README 里的**落地约定书面出处**(`./nw/components/<名>`);引擎侧首跑报告当初要求补这一节。数据仓自己的 PR 门禁抓不到(只验配方侧,锁定集侧检查全 skip)。修复 = Data-001 PR #5(README 补「已转正内容与落地约定」节),合并后 bake 放行。
- **附带发现(复现边界)**:bundle **内容**可复现(两份 bundle 解包后 `diff -r` 逐字节相同、文件清单相同),但 bundle **归档字节**不可位复现 —— tar 成员带 mtime,换一次 checkout 哈希就变(发布物 `c922e3d0…` vs 本次 `2a8b9a86…`,内容同)。故可发布的锁定集以「命令 + 同名 bundle release」为原子单位;复跑用已发布的那一对(见 §7),重烤只用于验证内容等价。

## 3 · 相 B:目标侧装配命令真跑

```bash
NW_HOME=/opt/nwt-realrun/nw-home sh assembly-command.sh   # 用已发布的那一对(§7)
```

- 64s,真下载 4 资产(node 121MB / pi 25.8MB / chezmoi / nw-linux-amd64,sha256 逐个校验)+ bundle 89.4MB(sha 校验后解包进 `locks/<lockId>/`)。
- `nw up --lock $NW_HOME/locks/research-desk-1.0.0-360ae68.nwlock` → **rc 0**,降级警告:`required env 缺失 TAVILY_API_KEY`(本机无任何模型 API key,不 Fabricate 凭据)。
- `nw doctor --lock …` → **rc 10(降级)**:`env` FAIL(缺 TAVILY_API_KEY)+ `provider` FAIL(`stepfun-sb` `credentials_not_configured`)。降级档呈现即预期(证据五字段原样进面板,见 §4 健康屏)。
- 落地状态:`profiles/research-desk/`(manifest + 组件 + 配置)、`runtimes/node-22.19.0+pi-1.0.2/`、`locks/<lockId>.nwlock` + bundle 解包目录、`bin/` 四资产。manifest `installedAt 2026-10-09T03:00:07Z`,`convergeHash 3cf6a584…`。

## 4 · 相 C:面板真机首跑(pty 驱动,12 幕全留帧)

驱动方式:`docs/real-first-run/drive_panel.py`(Python pty,110×30,TERM=xterm-256color;NW_HOME 与 XDG_CONFIG_HOME 均指向隔离目录;CWD 无 `catalog.json`)。逐幕摘要(全文见 `frames/`):

| 帧 | 屏 | 真数据实录 |
|---|---|---|
| 01 | 概览 | 数据仓根未设指路(不拿当前目录兜底);日志 `ls 未下发:数据仓根未设` |
| 02–04 | 设置 | 手填数据仓根 `/opt/Data-001` → 真 ls 回音;引擎发现:PATH `/usr/local/bin/nw`,**版本 nw 0.1.0**,能力 ✓登记 ✓复核 ✓烘焙 ?类别(未决)✓vps |
| 05 | 配方 | 五域真计数:配方 1 / 组件 2 / 外部源 0 / 派 1 / 锁定集 1;`research-desk 1.0.0` 详情;**建议命令** `nw recipe bake research-desk@1.0.0 --out research-desk.sh --repo /opt/Data-001`(执行权留给操作者) |
| 07 | 健康·doctor | **rc 10 降级**七面:manifest ● / consist ●● / converge ●(指纹与清单一致)/ config ●(9 键)/ env ✗(TAVILY_API_KEY)/ runtime ◐(派目录里 pi 尚未起过,预期)/ provider ✗;证据五字段原样:`[DEGRADED rc10] env {"missing":["TAVILY_API_KEY"]}`、`provider {"reason":"credentials_not_configured"}` |
| 08 | 健康·validate | **27 项:过 12 警 0 败 0 略 15**(真配方文件路径下发,见 G4) |
| 09 | 健康·recheck | 0 已登记外部源,只读零联网(CWD 无 catalog.json;#29 残留复核通过) |
| 10 | 装配 | 选择器列出真锁定集 `research-desk-1.0.0-360ae68` |
| 11 | 发射仪式 | 锁定集 / 配方 / NW_HOME 摊开,Enter 发射 |
| 12 | 装配结果 | **`nw up --lock /opt/nwt-realrun/nw-home/locks/research-desk-1.0.0-360ae68.nwlock` → rc 0,派已出炉** |

**幂等实证**:面板发射的这次真 up 之后,`.nw-manifest` 与面板操作前**逐字节一致**(mtime 更新、`installedAt` 保持 `03:00:07Z` 不变 —— 引擎「连跑两次逐字节一致」的收敛语义,经面板走通)。

## 5 · 首跑暴露的缺口与处置

| # | 缺口 | 症状(真机实测) | 处置 |
|---|---|---|---|
| G1 | 数据仓 README 丢落地约定书面出处 | `nw recipe bake` 拒烤(`component.landing` fail);数据仓 PR 门禁只验配方侧,抓不到 | Data-001 PR #5(README 补节),已合并 |
| G2 | 装配发射下发裸 lockId | `nw up --lock research-desk-1.0.0-360ae68` → rc 3 `INPUT_INVALID:读锁定集 …: no such file or directory`;引擎 `--lock` 只收载体文件 | PR #44:发射时按 `ls`/交锁段同一落点还原 `$NW_HOME/locks/<lockId>.nwlock` |
| G4 | 健康屏 validate 档传派名 | `nw validate --recipe research-desk` → rc 3 `INPUT_INVALID:读配方 …`;引擎 `--recipe` 只收配方路径 | PR #45:按「派名 = 配方名」(#5 命名决策)取 ls 行的 path 拼数据仓根下配方文件;缺前提不入队 |
| G5 | spawn 路径 stderr 尾巴空 | 顶栏/设置屏引擎版本永远「nw 未知」(`nw --version` 裸版本号在 stderr) | PR #46:streamer 收拢 stderr 进 Finished 尾巴(200 行截顶) |

四个缺口同一病因:**fixture 回放不校验 argv 语义**(FakeRunner 按规则回放,224 绿全过)—— 正是 #43 立票时预判「真机首跑会暴露新事实」的第一批实证。缺口修复均带回归测试;三平台 CI 因账号级计费一度无法启动(见 §6),本地全量验证(fmt / clippy 0 警告 / 226 passed;唯一 2 条失败为本机 PATH 有真 `nw` 的 discover 环境测试,未改动分支上同样失败)。

## 6 · 已知未知(记录在案,不阻塞 v0.2.0)

- **本机无任何模型 API key**:doctor 走降级档(rc 10)是预期呈现,不是缺陷;凭据绝不进配方/锁定集/日志的纪律在本次全程得到负向验证(没有密钥被触碰)。
- **三平台 CI 被账号级计费挡住**:首跑修复期间 GitHub Actions job 无法启动(注解:「recent account payments have failed or your spending limit needs to be increased」)。合并以本地全量验证 + 透明记账进行;**发布 tag 前须确认 CI 可跑并复跑 main**。
- **config 的 NW_HOME 启动不恢复**:进程环境有 NW_HOME 时面板显示与引擎一致;仅持久化配置里有、环境没有时,面板以 env→HOME 缺省开局(与引擎同源,不漂移),但上次手填值要再按 n 轮换或 i 手填。记录,不单独开票。
- **bundle 归档字节不位复现**(tar 成员 mtime):内容可复现;发布以「命令 + bundle release」为原子单位(§2)。

## 7 · 复跑入口(下一个人照做即复现)

```bash
# 相 A(作者侧):真资产 + 真数据仓 → 复现 lockId research-desk-1.0.0-360ae68
gh release download nw/v0.1.0 --repo JoeWest9966/Nous-Warehouse \
  --pattern '{node,pi,chezmoi,nw-linux-amd64}' --dir /tmp/assets-dir
nw recipe bake recipes/research-desk/recipe.yaml --out /tmp/assembly-command.sh \
  --assets-dir /tmp/assets-dir --bundle-repo JoeWest9966/Nous-Warehouse --repo /opt/Data-001

# 相 B(目标侧):用已发布的原子对(2026-10-05 首跑发布物,与今烤内容同源)
gh release download bake/research-desk-1.0.0-360ae68 \
  --repo JoeWest9966/Nous-Warehouse -p assembly-command.sh -p research-desk-1.0.0.nwbundle --dir /tmp/t
NW_HOME=/tmp/nwhome sh /tmp/t/assembly-command.sh        # rc 0(降级);doctor rc 10(无 key)

# 相 C(面板侧):pty 驱动 12 幕(脚本即本报告的证据驱动器)
python3 docs/real-first-run/drive_panel.py               # 帧落 docs/real-first-run/frames/
```
