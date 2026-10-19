//! typed 命令构造。面板只发三类动作(ADR-0003)+ 版本探测 + 只读 `ls`。

/// 五域总览。数据仓根**显式携带**——签名强制带 repo,
/// 类型层面杜绝「不带 --repo 让引擎拿当前目录兜底」(spec #24,票 #27)。
/// 缺根时调用方不应构造命令(下发前有门控,见 `App::spawn_engine`)。
pub fn ls(repo: &str) -> Vec<String> {
    vec!["ls".to_string(), "--repo".into(), repo.to_string()]
}

/// 装配。`lock` 可以是 `.nwlock` 或装配命令文件。
pub fn up(lock: &str, nw_home: Option<&str>) -> Vec<String> {
    let mut a = vec!["up".to_string(), "--lock".into(), lock.to_string()];
    if let Some(h) = nw_home {
        a.push("--nw-home".into());
        a.push(h.into());
    }
    a
}

/// 健康体检:按派名(profile)。
pub fn doctor_profile(profile: &str, nw_home: Option<&str>) -> Vec<String> {
    let mut a = vec![
        "doctor".to_string(),
        "--profile".into(),
        profile.to_string(),
    ];
    if let Some(h) = nw_home {
        a.push("--nw-home".into());
        a.push(h.into());
    }
    a
}

/// 健康体检:按锁定集。
pub fn doctor_lock(lock: &str, nw_home: Option<&str>) -> Vec<String> {
    let mut a = vec!["doctor".to_string(), "--lock".into(), lock.to_string()];
    if let Some(h) = nw_home {
        a.push("--nw-home".into());
        a.push(h.into());
    }
    a
}

/// 校验:配方侧(spec #89 工单 #92 的单侧两态之一)。
///
/// 槽位纪律与 [`ls`] / [`recipe_bake`] 同源:数据仓根**显式携带**,签名里
/// 没有「不带 `--repo`、让引擎拿当前目录兜底」的写法。`quiet` 是留位槽位
/// (见 [`validate_argv`]),缺省不落。
pub fn validate_recipe(recipe: &str, repo: &str, quiet: bool) -> Vec<String> {
    validate_argv(&[("--recipe", recipe)], repo, quiet)
}

/// 校验:锁定集侧(spec #89 工单 #92 的单侧两态之二)。
///
/// `--repo` 在这一态尤其不是可选项:锁定集放在引擎推导不出的目录布局时,
/// 配方与组件清单全靠它才发现(spec #89 工单 #92;上游语义:仅 `--lock`
/// 单侧时 `--repo` 用于发现配方与组件清单)。`quiet` 留位槽位同
/// [`validate_recipe`](fn.validate_recipe),缺省不落。
pub fn validate_lock(lock: &str, repo: &str, quiet: bool) -> Vec<String> {
    validate_argv(&[("--lock", lock)], repo, quiet)
}

/// 校验:两侧同给(spec #89 工单 #98 的第三态)。
///
/// 一次跑全 27 项里的**跨文件检查项**(1–19):单侧 `--lock` 在命令路径推导
/// 不出数据仓根时发现不全配方与组件清单,配方与锁定集同给就不靠发现兜底
/// (spec #89 用户故事 2)。`--repo` 恒落,纪律同两个单侧态——不假设引擎拿
/// 当前目录兜底。`quiet` 留位槽位同两个单侧态,缺省不落。
///
/// 旗标顺序冻结:`validate` → `--recipe` → `--lock` → `--repo` → (`--quiet`)。
pub fn validate_both(recipe: &str, lock: &str, repo: &str, quiet: bool) -> Vec<String> {
    validate_argv(&[("--recipe", recipe), ("--lock", lock)], repo, quiet)
}

/// validate argv 的**唯一拼装处**(spec #89 工单 #92):两侧槽位按序落,
/// `--repo` 恒落,`quiet` 留位**缺省不落**。
///
/// 三态(单侧配方 / 单侧锁定集 / 双侧同给)都从本函数出,**不另立第二条
/// 拼装路径**:旗标一处真相,与 `ls` / `up` 同层(工单 #98 在两侧槽位上多
/// 传一组即得双侧同给,见 [`validate_both`])。
///
/// `quiet` 是**留位槽位**:上游 `--quiet` 今天是压 stderr 进度行的 no-op
/// (进度阶段没启用,警告永不被压),语义以上游激活为准;面板不摆 UI 入口,
/// 只在 argv 构造函数的能力范围里留出这一枚——给了才落,落点固定在
/// `--repo` 之后(argv 末尾),屏级断言不必猜位置。
fn validate_argv(sides: &[(&str, &str)], repo: &str, quiet: bool) -> Vec<String> {
    let mut a = vec!["validate".to_string()];
    for (flag, value) in sides {
        a.push((*flag).to_string());
        a.push((*value).to_string());
    }
    a.push("--repo".to_string());
    a.push(repo.to_string());
    if quiet {
        a.push("--quiet".to_string());
    }
    a
}

/// 版本探测。注意:`nw --version` 的裸版本号走 **stderr**。
pub fn version() -> Vec<String> {
    vec!["--version".to_string()]
}

/// 外部源登记·预审段(spec #25 ticket #30):扫描 + 验收 5(+1) 条,
/// 拟落账记录进 JSON,**文件不落**——铁律是不带 `--yes`(确认闸),
/// 本函数连 `--yes` 的位置都没有,从类型层面杜绝预审误落账。
/// `--root` 指数据仓根(与 `ls --repo` 同义的本地仓根槽位)。
pub fn source_register_preaudit(repo: &str, kind: &str, root: &str) -> Vec<String> {
    vec![
        "source".into(),
        "register".into(),
        "--repo".into(),
        repo.to_string(),
        "--kind".into(),
        kind.to_string(),
        "--root".into(),
        root.to_string(),
    ]
}

/// 外部源登记·落账段(spec #25 ticket #31):与预审同参,多一枚 `--yes`
/// 确认闸——引擎才把 `catalog.json` 写进数据仓根。**只能由图形确认后的
/// `App::dispatch_register_commit` 调用**;预审段用的
/// [`source_register_preaudit`] 连 `--yes` 的位置都没有,两段从构造函数
/// 形状上分开,误下发靠类型杜绝。
pub fn source_register_commit(repo: &str, kind: &str, root: &str) -> Vec<String> {
    let mut a = source_register_preaudit(repo, kind, root);
    a.push("--yes".into());
    a
}

/// vps 登记·预审段(工单 #53,ADR-0009 §1 获准两段式代跑):
/// 五道闸(`register.name` / `register.ips` / `register.port` /
/// `register.logins` / `register.secret_guard`)全跑一遍,拟落账记录进
/// JSON,**文件不落**——铁律是不带 `--yes`(确认闸),本函数连 `--yes`
/// 的位置都没有,从类型层面杜绝预审误落账(同
/// [`source_register_preaudit`])。
///
/// 槽位纪律:**只认 `--root <数据仓根>`**。`--repo` 是 `source register`
/// 的外部源仓库链接槽,喂给 vps 组 = 未知 flag → rc 3(工单 #51 §3)。
/// 形状(上游 `cli.go` `runVpsRegister` / `vps/register.go`):
/// `nw vps register <名> --ip <label>=<addr> … --login <user>=<ref> … [--port N] [--note <话>] [--root <目录>]`。
/// 多组 `--ip` / `--login` 可重复,顺序即入参数组顺序(引擎自己排序落盘)。
pub fn vps_register_preaudit(
    name: &str,
    ips: &[(String, String)],
    port: Option<u16>,
    logins: &[(String, String)],
    note: Option<&str>,
    root: &str,
) -> Vec<String> {
    let mut a = vec!["vps".into(), "register".into(), name.to_string()];
    for (label, addr) in ips {
        a.push("--ip".into());
        a.push(format!("{label}={addr}"));
    }
    if let Some(p) = port {
        a.push("--port".into());
        a.push(p.to_string());
    }
    for (user, key_ref) in logins {
        a.push("--login".into());
        a.push(format!("{user}={key_ref}"));
    }
    if let Some(n) = note.filter(|n| !n.is_empty()) {
        a.push("--note".into());
        a.push(n.to_string());
    }
    a.push("--root".into());
    a.push(root.to_string());
    a
}

/// vps 登记·落账段(工单 #53):与预审同参,多一枚 `--yes` 确认闸——
/// 引擎才把 `vps.json` 写进数据仓根。**只能由图形确认后的落账转换
/// 调用**;预审段用的 [`vps_register_preaudit`] 连 `--yes` 的位置都没有,
/// 两段从构造函数形状上分开,误下发靠类型杜绝。落账 = 预审 + `--yes`,
/// 不另改槽位。
pub fn vps_register_commit(
    name: &str,
    ips: &[(String, String)],
    port: Option<u16>,
    logins: &[(String, String)],
    note: Option<&str>,
    root: &str,
) -> Vec<String> {
    let mut a = vps_register_preaudit(name, ips, port, logins, note, root);
    a.push("--yes".into());
    a
}

/// vps 编目册**只读**投影(工单 #53,ADR-0009 §1 准许代跑):列数据仓根
/// 下 `vps.json` 里的机器。编目不存在 = 空数组 rc 0(合法空态)。
/// 只认 `--root <数据仓根>`——**不喂 `--repo`**(那是 source register
/// 的外部源链接槽,喂进来 = 未知 flag → rc 3)。
pub fn vps_ls(root: &str) -> Vec<String> {
    vec!["vps".into(), "ls".into(), "--root".into(), root.to_string()]
}

/// vps 删一台机器(工单 #53,ADR-0009 §1:代跑,但面板加一道图形确认)。
/// 上游**没有 `--yes`**(点名即删),所以本函数也不产生确认旗标——面板
/// 的图形确认是面板侧的编排,不改引擎语义(下发的 argv 与在 shell 里
/// 敲的逐字相同)。删不存在的名 → rc 3 且文件一个字节都不动。
pub fn vps_rm(name: &str, root: &str) -> Vec<String> {
    vec![
        "vps".into(),
        "rm".into(),
        name.to_string(),
        "--root".into(),
        root.to_string(),
    ]
}

// ---- tag 组(工单 #54,ADR-0009 §1:add 代跑无确认闸 / ls 只读 / rm 代跑带面板确认) ----

/// `--ref <kind>/<条目>` 的 kind **已知集合一处定义**(上游 `tag/tag.go`
/// 的 knownRefKinds):新 kind 随上游加,面板不靠存在性间接放行。
pub const TAG_REF_KINDS: [&str; 3] = ["skill", "extension", "vps"];

/// 拆一条 `<kind>/<条目>` 引用:两侧都非空、kind 在已知集合内才认。
/// 面板侧就拒、不喂给引擎猜(引擎第③道闸也拒,但那是引擎的事);
/// 坏行由调用方留证据、留驻可改。
pub fn parse_tag_ref(row: &str) -> Option<(String, String)> {
    let (kind, id) = row.trim().split_once('/')?;
    let (kind, id) = (kind.trim(), id.trim());
    if kind.is_empty() || id.is_empty() || !TAG_REF_KINDS.contains(&kind) {
        return None;
    }
    Some((kind.to_string(), id.to_string()))
}

/// 挂标签(工单 #54,ADR-0009 §1:代跑,**无确认闸**)。形状(上游
/// `cli.go` `runTagAdd` / `tag/add.go`):
/// `nw tag add <标签> --ref <kind>/<条目> … [--root <目录>]`。
///
/// 上游**没有 `--yes`**:追加合并、幂等,没有「覆盖」可确认。所以本函数
/// **不产生确认旗标**——面板也不自己造一道(ADR-0009 §2:那会改引擎语义,
/// 多一枚未知 flag 直接 rc 3)。幂等由面板复用同一份 argv 保证:同一条
/// 命令重跑,`tags.json` 一个字节不变。
///
/// 槽位纪律同 vps 组:只认 `--root <数据仓根>`,**不喂 `--repo`**。
pub fn tag_add(name: &str, refs: &[(String, String)], root: &str) -> Vec<String> {
    let mut a = vec!["tag".into(), "add".into(), name.to_string()];
    for (kind, id) in refs {
        a.push("--ref".into());
        a.push(format!("{kind}/{id}"));
    }
    a.push("--root".into());
    a.push(root.to_string());
    a
}

/// 反查(工单 #54 的**主价值**,只读):不带参列全部标签及各引用,带参
/// 只列那一条。形状:`nw tag ls [<标签>] [--root <目录>]`。
/// 编目不存在 = 空数组 rc 0(合法空态);带参查不到 = **rc 3**(问错了名字,
/// 不是空结果——两种语义面板分开呈现,不都画成空列表)。
pub fn tag_ls(root: &str) -> Vec<String> {
    vec!["tag".into(), "ls".into(), "--root".into(), root.to_string()]
}

/// 反查一条标签(同 [`tag_ls`],多一个位置参数)。
pub fn tag_ls_one(name: &str, root: &str) -> Vec<String> {
    vec![
        "tag".into(),
        "ls".into(),
        name.to_string(),
        "--root".into(),
        root.to_string(),
    ]
}

/// 摘引用 / 删标签(工单 #54,ADR-0009 §1:代跑,面板加一道图形确认)。
/// 形状:`nw tag rm <标签> [--ref <kind>/<条目> …] [--root <目录>]`。
/// 带 `--ref` 只摘那几条引用(标签还在,action `ref_removed`);不带删整个
/// 标签。上游没有 `--yes`,本函数也不产生确认旗标。删不存在的标签 →
/// rc 3 且 `tags.json` 一个字节都不动。
pub fn tag_rm(name: &str, refs: &[(String, String)], root: &str) -> Vec<String> {
    let mut a = vec!["tag".into(), "rm".into(), name.to_string()];
    for (kind, id) in refs {
        a.push("--ref".into());
        a.push(format!("{kind}/{id}"));
    }
    a.push("--root".into());
    a.push(root.to_string());
    a
}

// ---- subscription 组(工单 #56,ADR-0009 §1:register 两段式经 stdin / ls 只读 /
// ---- rm 代跑带面板确认;get 一律只给建议命令) ----

/// 订阅登记·预审段(工单 #56):跑四道闸、拟落账,**文件不落**——铁律是
/// 不带 `--yes`,本函数连 `--yes` 的位置都没有(与
/// [`vps_register_preaudit`] 同构,误下发靠类型杜绝)。
///
/// 形状(上游 `cli.go` `runSubscriptionRegister`):
/// `nw subscription register --name <名> --client <应用> --token-stdin [--expires <YYYY-MM-DD>] [--root <目录>]`。
///
/// **argv 面没有任何明文链接形参**:链接本体只从 stdin 进(`--token-stdin`
/// 是「声明从 stdin 进」的开关,不是载荷;误粘的 argv 会进 ps / history)。
/// 所以这个函数**不收本体参数**——本体由调用方经
/// [`crate::engine::CommandRunner::spawn_with_stdin`] 喂进去,两者在类型
/// 层面就是两条路。`expires` 空 = 永不过期(上游闸③认空串),此时**不产生
/// `--expires` 旗标**。
///
/// 槽位纪律同 vps / tag 组:只认 `--root <数据仓根>`,**不喂 `--repo`**。
pub fn subscription_register_preaudit(
    name: &str,
    client: &str,
    expires: Option<&str>,
    root: &str,
) -> Vec<String> {
    let mut a = vec![
        "subscription".into(),
        "register".into(),
        "--name".into(),
        name.to_string(),
        "--client".into(),
        client.to_string(),
        "--token-stdin".into(),
    ];
    if let Some(e) = expires.filter(|e| !e.trim().is_empty()) {
        a.push("--expires".into());
        a.push(e.to_string());
    }
    a.push("--root".into());
    a.push(root.to_string());
    a
}

/// 订阅登记·落账段(工单 #56):与预审同参,多一枚 `--yes` 确认闸——引擎才把
/// `subscriptions.json` 写进数据仓根。**只能由图形确认后的落账转换调用**;
/// 落账 = 预审 + `--yes`,不另改槽位。链接本体同样只经 stdin 喂入
/// (喂入是一次性的:预审与落账各喂一次,见 ADR-0009 §3)。
pub fn subscription_register_commit(
    name: &str,
    client: &str,
    expires: Option<&str>,
    root: &str,
) -> Vec<String> {
    let mut a = subscription_register_preaudit(name, client, expires, root);
    a.push("--yes".into());
    a
}

/// 订阅编目册**只读**投影(工单 #56,ADR-0009 §1 准许代跑):列数据仓根下
/// `subscriptions.json` 里的订阅。编目不存在 = 空数组 rc 0(合法空态)。
/// 投影**没有链接槽、不印指纹**;`status` / `daysLeft` 由引擎在读取时按
/// UTC 现算。只认 `--root <数据仓根>`——**不喂 `--repo`**。
pub fn subscription_ls(root: &str) -> Vec<String> {
    vec![
        "subscription".into(),
        "ls".into(),
        "--root".into(),
        root.to_string(),
    ]
}

/// 删一条订阅(工单 #56,ADR-0009 §1:代跑,面板加一道图形确认)。形状:
/// `nw subscription rm <名> [--root <目录>]`。上游**没有 `--yes`**(点名
/// 即删),本函数也不产生确认旗标——面板的图形确认是面板侧的编排,不改
/// 引擎语义。删不存在的名 → rc 3 且文件一个字节都不动。
pub fn subscription_rm(name: &str, root: &str) -> Vec<String> {
    vec![
        "subscription".into(),
        "rm".into(),
        name.to_string(),
        "--root".into(),
        root.to_string(),
    ]
}

/// 算出的**建议命令**:argv(下发形态,面板**不**下发)与给人看的展示串。
/// bake(#39)/ recipe new(#58)与本条 subscription get 建议命令共用这一个
/// 形状——argv 与展示串同源同层,`App` 没有任何路径会下发它。
///
/// 与 bake / recipe new 的唯一差别:展示串多一段管道(`| jq -r .data.url`)——
/// 上游不开 `--raw`,要裸链接自己拼管道(ADR-0009 §1:上游的裁法本身就是
/// 「别让这玩意有个方便的出口」,面板跟着)。argv 仍是一条完整可执行的命令
/// (不含管道段),旗标一处真相。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Suggest {
    /// 完整 argv(含二进制名之后的所有元素)。
    pub argv: Vec<String>,
    /// 操作者可见的命令文案(`nw ` 起头,可直接复制)。
    pub display: String,
}

/// 取回订阅链接本体的**建议命令**(ADR-0009 §1 裁定:面板**不代跑**
/// `subscription get`——`data.url` 是上游唯一回显本体的出口,屏显铺一条
/// 活订阅链接的扩散面(终端滚动历史 / 截图 / 快照帧)面板没有任何机制兜住)。
///
/// 纯函数:无 IO、不读环境、不碰能力账——本模块只算命令,永远不把它下发。
/// 形状:`nw subscription get --name <名> [--root <目录>] | jq -r .data.url`
/// (上游 usage 原话的形态;`--root` 照面板既有纪律显式给,不让引擎拿当前
/// 目录兜底)。
pub fn subscription_get_suggest(name: &str, root: &str) -> Suggest {
    let argv = vec![
        "subscription".into(),
        "get".into(),
        "--name".into(),
        name.to_string(),
        "--root".into(),
        root.to_string(),
    ];
    Suggest {
        display: format!("nw {} | jq -r .data.url", argv.join(" ")),
        argv,
    }
}

// ---- key 组(工单 #57,ADR-0009 §1:register 两段式经 stdin / ls 只读投影 /
// ---- get 是唯一回显处只给建议命令 / rm 代跑带面板确认) ----

/// 钥匙登记·预审段(工单 #57):跑三道闸、拟落账,**文件不落**——铁律是
/// 不带 `--yes`,本函数连 `--yes` 的位置都没有(与 subscription 同构)。
///
/// 形状(上游 `cli.go` `runKeyRegister` / `key/register.go`):
/// `nw key register --name <名> --token-stdin [--host <主机>] [--scope <域>]... [--rotated-at <YYYY-MM-DD>] [--root <目录>]`。
///
/// **argv 面没有任何明文形参**:钥匙本体只从 stdin 进(`--token-stdin`
/// 是「声明从 stdin 进」的开关,不是载荷;误粘的 argv 会进 ps / history)。
/// 所以这个函数**不收本体参数**——本体由调用方经
/// [`crate::engine::CommandRunner::spawn_with_stdin`] 喂进去,两者在类型
/// 层面就是两条路。
///
/// `--scope` / `--rotated-at` **只登记、不设闸**(ADR-0009 §3:多宽、何时
/// 轮换归所有者自裁,面板不代管、不提醒、不评审);但形状错照样由面板侧就拒
/// (空 scope / 非 YYYY-MM-DD,不喂给引擎猜——引擎也在闸前拒,只是不占闸行)。
/// `host` 空 = 缺省 github.com(上游 CLI flag 缺省,此时**不产生 `--host`**;
/// 显式 `--host ""` 会被第②道闸拒,所以空串面板也不下发)。
/// `rotated_at` 空 = 缺省登记日 UTC(上游口径),此时**不产生 `--rotated-at`**。
///
/// 槽位纪律同 vps / tag / subscription 组:只认 `--root <数据仓根>`,
/// **不喂 `--repo`**。
pub fn key_register_preaudit(
    name: &str,
    host: &str,
    scopes: &[String],
    rotated_at: Option<&str>,
    root: &str,
) -> Vec<String> {
    let mut a = vec![
        "key".into(),
        "register".into(),
        "--name".into(),
        name.to_string(),
        "--token-stdin".into(),
    ];
    let host = host.trim();
    if !host.is_empty() {
        a.push("--host".into());
        a.push(host.to_string());
    }
    for s in scopes.iter().map(|s| s.trim()).filter(|s| !s.is_empty()) {
        a.push("--scope".into());
        a.push(s.to_string());
    }
    if let Some(r) = rotated_at.map(str::trim).filter(|r| !r.is_empty()) {
        a.push("--rotated-at".into());
        a.push(r.to_string());
    }
    a.push("--root".into());
    a.push(root.to_string());
    a
}

/// 钥匙登记·落账段(工单 #57):与预审同参,多一枚 `--yes` 确认闸——引擎才把
/// `keys.json` 写进数据仓根。**只能由图形确认后的落账转换调用**;落账 = 预审
/// 加一枚 `--yes`,不另改槽位。钥匙本体同样只经 stdin 喂入(喂入是一次性的:
/// 预审与落账各喂一次,见 ADR-0009 §3)。
pub fn key_register_commit(
    name: &str,
    host: &str,
    scopes: &[String],
    rotated_at: Option<&str>,
    root: &str,
) -> Vec<String> {
    let mut a = key_register_preaudit(name, host, scopes, rotated_at, root);
    a.push("--yes".into());
    a
}

/// 钥匙编目册**只读**投影(工单 #57,ADR-0009 §1 准许代跑):列数据仓根下
/// `keys.json` 里的钥匙。编目不存在 = 空数组 rc 0(合法空态)。投影**没有
/// secret 槽、不印指纹**。只认 `--root <数据仓根>`——**不喂 `--repo`**。
pub fn key_ls(root: &str) -> Vec<String> {
    vec!["key".into(), "ls".into(), "--root".into(), root.to_string()]
}

/// 删一把钥匙(工单 #57,ADR-0009 §1:代跑,面板加一道图形确认)。形状:
/// `nw key rm <名> [--root <目录>]`。上游**没有 `--yes`**(点名即删),本函数
/// 也不产生确认旗标——面板的图形确认是面板侧的编排,不改引擎语义。删不存在的
/// 名 → rc 3 且文件一个字节都不动。
pub fn key_rm(name: &str, root: &str) -> Vec<String> {
    vec![
        "key".into(),
        "rm".into(),
        name.to_string(),
        "--root".into(),
        root.to_string(),
    ]
}

/// 取回钥匙本体的**建议命令**(ADR-0009 §1 裁定:面板**不代跑** `key get`——
/// `data.key` 是上游唯一回显本体的出口,屏显铺一把活钥匙的扩散面(终端滚动
/// 历史 / 截图 / 快照帧)面板没有任何机制兜住)。
///
/// 纯函数:无 IO、不读环境、不碰能力账——本模块只算命令,永远不把它下发。
/// 形状:`nw key get --name <名> [--root <目录>] | jq -r .data.key`(上游
/// usage 原话的形态;`--root` 照面板既有纪律显式给,不让引擎拿当前目录兜底;
/// 上游不开 `--raw`,要裸串自己拼管道)。
pub fn key_get_suggest(name: &str, root: &str) -> Suggest {
    let argv = vec![
        "key".into(),
        "get".into(),
        "--name".into(),
        name.to_string(),
        "--root".into(),
        root.to_string(),
    ];
    Suggest {
        display: format!("nw {} | jq -r .data.key", argv.join(" ")),
        argv,
    }
}

/// 三条前提缺哪一条:两种配方域建议命令(bake / new)共用这一份判定,
/// 措辞各归各自的 gate enum——判定只有一处,说法不互相冒充。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Precondition {
    NoDataRoot,
    NotARecipe,
    NoCapability,
}

/// 前提判定(纯函数,无 IO、不读环境、不碰能力账):顺序即 bake 既有的
/// 信任顺序——先看选中行是不是配方,再看数据仓根,最后看引擎能力。
/// 任缺一条返回缺的那一条,**绝不返回半条命令**。
fn precondition<'a>(
    recipe: Option<&'a str>,
    data_root: Option<&'a str>,
    capable: bool,
) -> Result<(&'a str, &'a str), Precondition> {
    let Some(name) = recipe else {
        return Err(Precondition::NotARecipe);
    };
    let Some(root) = data_root else {
        return Err(Precondition::NoDataRoot);
    };
    if !capable {
        return Err(Precondition::NoCapability);
    }
    Ok((name, root))
}

/// 配方域**建议命令**(spec #33 / 工单 #39,ADR-0003):选中一个配方时,
/// 面板给出可复制的 `nw recipe bake …` 命令文案,**执行权留给操作者**——
/// 本模块只算命令,永远不把它下发(`App` 没有任何路径会调它)。
///
/// 旗标与 [`recipe_bake`] 的拼装只有这一处真相:与 `ls` / `up` / `doctor`
/// 同一层构造,面板不内置第二份「命令长什么样」。
///
/// 三种缺前提各有可区分的指路,**绝不渲染半条命令**:没设数据仓根(spec #24:
/// 不猜、不拿当前目录兜底)、选中行不是配方、引擎没有 `recipe bake` 能力
/// (#29 能力探测门的结论,不是本地常量表)。
///
/// 缺前提的三种可区分原因(与 [`crate::app::DataRootCause`] 同形:
/// 标题 + 说法,两两不同、互不冒充)。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BakeGate {
    /// 数据仓根未设——五域读不到数据仓,命令指哪儿都不对。
    NoDataRoot,
    /// 当前没有选中的配方(配方域为空,或选中行不是配方)。
    NotARecipe,
    /// 引擎没有 `recipe bake` 能力(缺的是引擎能力,不是面板坏了)。
    NoCapability,
}

impl BakeGate {
    /// 指路标题:三态各说各话。
    pub fn title(self) -> &'static str {
        match self {
            BakeGate::NoDataRoot => "数据仓根未设",
            BakeGate::NotARecipe => "没有选中的配方",
            BakeGate::NoCapability => "引擎缺 recipe bake 能力",
        }
    }

    /// 指路人话:每态一条,带各自的事实与下一步。
    pub fn advice(self) -> String {
        match self {
            BakeGate::NoDataRoot => {
                "五域读不到数据仓,bake 命令指不到本地仓。去设置屏按 d 录入数据仓根;面板不拿当前目录兜底。"
                    .into()
            }
            BakeGate::NotARecipe => {
                "配方域为空或未选中配方——先在配方域选中一条(↑/↓ 或鼠标点行)。".into()
            }
            BakeGate::NoCapability => {
                "缺的是引擎能力,不是面板坏了;升级所配 nw 后建议命令自动给出。".into()
            }
        }
    }

    /// 与 [`Precondition`] 的映射:判定只有一处(见 [`precondition`]),
    /// 说法仍是 bake 自己那一套。
    fn of(p: Precondition) -> Self {
        match p {
            Precondition::NoDataRoot => BakeGate::NoDataRoot,
            Precondition::NotARecipe => BakeGate::NotARecipe,
            Precondition::NoCapability => BakeGate::NoCapability,
        }
    }
}

/// 算出的建议命令:argv(下发形态,面板**不**下发)与给人看的展示串。
/// bake(#39)与 recipe new(#58)两条建议命令共用这一个形状——argv 与
/// 展示串同源,`display` 永远是 `"nw " + argv.join(" ")`。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BakeSuggest {
    /// 完整 argv(含二进制名之后的所有元素),与 [`recipe_bake`] / [`recipe_new`] 同源。
    pub argv: Vec<String>,
    /// 操作者可见的命令文案(`nw ` 起头,可直接复制)。
    pub display: String,
}

/// 位置参数 `<配方>[@版本]`:配方带版本时才拼 `@版本` 段(#39 验收第 3 条)。
/// 版本缺失时**不猜**——只给配方名,让引擎按配方自己的版本烤。
fn recipe_arg(recipe: &str, version: Option<&str>) -> String {
    match version {
        Some(v) if !v.is_empty() => format!("{recipe}@{v}"),
        _ => recipe.to_string(),
    }
}

/// 建议命令的 `--out` 默认值:`<配方>.sh`(操作者可改)。
/// 省了 `--out` 时引擎自己落 `<配方目录>/../baked/nw-assemble.sh`——
/// 那句话由呈现层照引擎原话说,不在这里替引擎承诺。
pub fn bake_out_default(recipe: &str) -> String {
    format!("{recipe}.sh")
}

/// 烘焙的两个可选槽位旗标名(spec #89 工单 #93):argv 拼法一处真相,
/// 设置屏的指路也照它说,不另写一串。形状实测自引擎
/// `nw recipe bake --help` 的用法行(所配 0.1.0)。
pub const ASSETS_DIR_FLAG: &str = "--assets-dir";
pub const BUNDLE_REPO_FLAG: &str = "--bundle-repo";

/// 烘焙:把配方烤成锁定集 + 一条 POSIX sh 装配命令。
///
/// `version` 非空时位置参数带 `@版本`(与配方版本不符由引擎拒,rc 3)。
/// 形状实测自引擎 `nw recipe bake --help` 的用法行(所配 0.1.0):
/// `nw recipe bake <配方>[@版本] --out <命令文件> [--assets-dir <目录>] [--repo <目录>]`。
///
/// #93 加的两个**可选增强槽位**:`assets_dir` 与 `bundle_repo` 非空才落,
/// 空 = 让引擎自己算(资产目录缺省按 release 预设算 sha256;取货仓缺省从数据仓
/// git origin 推导——面板**不复制**这个推导,见 spec #89 决策)。落点都钉在
/// `--repo` 之后:两个可选槽位共用同一条尾巴顺序,旗标先后对引擎的旗标解析无影响。
pub fn recipe_bake(
    recipe: &str,
    version: Option<&str>,
    out: &str,
    repo: &str,
    assets_dir: Option<&str>,
    bundle_repo: Option<&str>,
) -> Vec<String> {
    let mut a = vec![
        "recipe".into(),
        "bake".into(),
        recipe_arg(recipe, version),
        "--out".into(),
        out.to_string(),
        "--repo".into(),
        repo.to_string(),
    ];
    if let Some(d) = assets_dir.map(str::trim).filter(|d| !d.is_empty()) {
        a.push(ASSETS_DIR_FLAG.into());
        a.push(d.to_string());
    }
    if let Some(b) = bundle_repo.map(str::trim).filter(|b| !b.is_empty()) {
        a.push(BUNDLE_REPO_FLAG.into());
        a.push(b.to_string());
    }
    a
}

/// 由「选中配方 + 数据仓根 + 出参默认值 + 引擎能力」算出建议命令。
///
/// 纯函数:无 IO、不读环境、不碰能力账——`bake_capable` 由调用方从
/// 能力探测门(#29)取来。任一前提不成立就走对应的 [`BakeGate`],
/// **绝不返回半条命令**。
///
/// `assets_dir` / `bundle_repo`(#93)是**可选增强槽位**,由调用方从设置屏的
/// 一等设置取来(未设传 `None`)。它们不参与前提判定:填了照旧过三道闸,
/// 缺了也照旧给得出命令;进了 argv 的落点与 [`recipe_bake`] 一致(都在
/// `--repo` 之后)。展示串永远由 argv 拼出(`"nw " + argv.join(" ")`),
/// 复制去 shell 敲与面板算出来的是同一条命令。
pub fn recipe_bake_suggest(
    recipe: Option<(&str, Option<&str>)>,
    data_root: Option<&str>,
    bake_capable: bool,
    assets_dir: Option<&str>,
    bundle_repo: Option<&str>,
) -> Result<BakeSuggest, BakeGate> {
    let (name, repo) =
        precondition(recipe.map(|(n, _)| n), data_root, bake_capable).map_err(BakeGate::of)?;
    let version = recipe.and_then(|(_, v)| v);
    let out = bake_out_default(name);
    let argv = recipe_bake(name, version, &out, repo, assets_dir, bundle_repo);
    Ok(BakeSuggest {
        display: format!("nw {}", argv.join(" ")),
        argv,
    })
}

/// 配方域**新建骨架**建议命令(spec #33 / 工单 #58,ADR-0003 + ADR-0006):
/// 选中一个配方时,面板在 bake 之外一并给出可复制的 `nw recipe new …`
/// 命令文案,**执行权留给操作者**——与 bake 同源同层(同一个纯函数层产出
/// argv 与展示串),本模块只算命令,永远不把它下发(`App` 没有任何路径会调它)。
///
/// 三种缺前提各有可区分的指路,**绝不渲染半条命令**:没设数据仓根、
/// 选中行不是配方、引擎没有 `recipe new` 能力(#29 能力探测门的结论)。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RecipeNewGate {
    /// 数据仓根未设——骨架要写进数据仓,`--out-dir` 指哪儿都不对。
    NoDataRoot,
    /// 当前没有选中的配方(配方域为空,或选中行不是配方):骨架名跟着它起。
    NotARecipe,
    /// 引擎没有 `recipe new` 能力(缺的是引擎能力,不是面板坏了)。
    NoCapability,
}

impl RecipeNewGate {
    /// 指路标题:三态各说各话。
    pub fn title(self) -> &'static str {
        match self {
            RecipeNewGate::NoDataRoot => "数据仓根未设",
            RecipeNewGate::NotARecipe => "没有选中的配方",
            RecipeNewGate::NoCapability => "引擎缺 recipe new 能力",
        }
    }

    /// 指路人话:每态一条,带各自的事实与下一步;与 [`BakeGate`] 同形
    /// 不同文(标题可能同题,说法绝不互相冒充)。
    pub fn advice(self) -> String {
        match self {
            RecipeNewGate::NoDataRoot => {
 "五域读不到数据仓,recipe new 的 --out-dir 指不到本地仓。去设置屏按 d 录入数据仓根;面板不拿当前目录兜底。"
                    .into()
            }
            RecipeNewGate::NotARecipe => {
                "配方域为空或未选中配方——先在配方域选中一条(↑/↓ 或鼠标点行),骨架名跟着它起。".into()
            }
            RecipeNewGate::NoCapability => {
                "缺的是引擎能力,不是面板坏了;升级所配 nw 后 recipe new 的建议命令自动给出。"
                    .into()
            }
        }
    }

    fn of(p: Precondition) -> Self {
        match p {
            Precondition::NoDataRoot => RecipeNewGate::NoDataRoot,
            Precondition::NotARecipe => RecipeNewGate::NotARecipe,
            Precondition::NoCapability => RecipeNewGate::NoCapability,
        }
    }
}

/// 新建配方骨架:写 `<out-dir>/recipes/<名>/recipe.yaml`,不碰 git。
///
/// `profile` 非空时才带 `--profile`——上游缺省 "" = 按配方名兜底
/// (#8 布局),面板不替引擎猜一个派名。`--out-dir` 与 bake 的 `--repo`
/// 同一纪律:显式指数据仓根,不让引擎拿当前目录兜底。
/// 形状实测自引擎 usage 行(所配 0.1.0):
/// `nw recipe new <名> [--profile <名>] [--out-dir <目录>]`。
pub fn recipe_new(name: &str, profile: Option<&str>, out_dir: &str) -> Vec<String> {
    let mut a = vec!["recipe".into(), "new".into(), name.to_string()];
    if let Some(p) = profile.filter(|p| !p.is_empty()) {
        a.push("--profile".into());
        a.push(p.to_string());
    }
    a.push("--out-dir".into());
    a.push(out_dir.to_string());
    a
}

/// 由「选中配方 + 数据仓根 + 引擎能力」算出新建骨架的建议命令。
///
/// 纯函数:无 IO、不读环境、不碰能力账——`new_capable` 由调用方从
/// 能力探测门(#29)取来。任一前提不成立就走对应的 [`RecipeNewGate`],
/// **绝不返回半条命令**。
pub fn recipe_new_suggest(
    recipe: Option<&str>,
    data_root: Option<&str>,
    new_capable: bool,
) -> Result<BakeSuggest, RecipeNewGate> {
    let (name, out_dir) =
        precondition(recipe, data_root, new_capable).map_err(RecipeNewGate::of)?;
    // 派名面板不知道,也不猜:不填 --profile,由引擎按配方名兜底(#8 布局)
    let argv = recipe_new(name, None, out_dir);
    Ok(BakeSuggest {
        display: format!("nw {}", argv.join(" ")),
        argv,
    })
}

/// 外部源漂移复核(spec #34 / 工单 #40):**只读**——只报告漂移、永不写账。
/// `name` 非空 = 只查这一条记录;None = 全查已登记源。
/// `--root` 指数据仓根(与 register 同槽位;别与 ls 的 `--repo` 混)。
/// 铁律:本命令不产生、也不允许产生任何写旗标——两段式登记的写入口只有
/// `source_register_commit` 一个,那里才有 `--yes`。
pub fn source_recheck(name: Option<&str>, root: &str) -> Vec<String> {
    let mut a = vec!["source".into(), "recheck".into()];
    if let Some(n) = name {
        a.push("--name".into());
        a.push(n.to_string());
    }
    a.push("--root".into());
    a.push(root.to_string());
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn up_带_nw_home() {
        assert_eq!(
            up("x.nwlock", Some("/data")),
            vec!["up", "--lock", "x.nwlock", "--nw-home", "/data"]
        );
    }

    #[test]
    fn doctor_二选一_flag_不混用() {
        assert_eq!(
            doctor_profile("desk", None),
            vec!["doctor", "--profile", "desk"]
        );
        assert_eq!(
            doctor_lock("x.nwlock", None),
            vec!["doctor", "--lock", "x.nwlock"]
        );
    }

    #[test]
    fn ls_必带_repo_不给引擎当前目录兜底() {
        assert_eq!(ls("/data/repo"), vec!["ls", "--repo", "/data/repo"]);
        // 签名里没有 Option:想不带 --repo 都写不出来
        let a = ls(".");
        assert_eq!(a[1], "--repo");
    }

    #[test]
    fn recheck_只读_按名限定与全查都不带任何写旗标() {
        let all = source_recheck(None, "/data/root");
        assert_eq!(
            all,
            vec!["source", "recheck", "--root", "/data/root"],
            "全查:不带 --name"
        );
        let one = source_recheck(Some("web-search"), "/data/root");
        assert_eq!(one[2], "--name");
        assert_eq!(one[3], "web-search");
        assert_eq!(one[4], "--root");
        for a in [&all, &one] {
            assert!(
                !a.iter().any(|s| s == "--yes"),
                "只读铁律:recheck 的 argv 里不许出现 --yes(写入口只有 register 一个)"
            );
        }
    }

    #[test]
    fn 登记预审_永不带_yes_根槽位指数据仓根() {
        let a = source_register_preaudit("owner/repo", "skill", "/data/root");
        assert_eq!(
            a,
            vec![
                "source",
                "register",
                "--repo",
                "owner/repo",
                "--kind",
                "skill",
                "--root",
                "/data/root"
            ]
        );
        assert!(
            !a.iter().any(|s| s == "--yes"),
            "预审段铁律:argv 里不许出现 --yes(文件不落)"
        );
    }

    #[test]
    fn vps_登记预审_永不带_yes_多组ip与login按序落槽() {
        // 形状实测自上游 usage:`nw vps register <名> --ip <label>=<addr> …
        // --login <user>=<ref> … [--port N] [--note <话>] [--root <目录>]`
        let a = vps_register_preaudit(
            "gate",
            &[
                ("home".to_string(), "10.0.0.1".to_string()),
                ("office".to_string(), "10.0.0.2".to_string()),
            ],
            Some(22),
            &[("root".to_string(), "$DEPLOY_KEY".to_string())],
            Some("门口那台"),
            "/data/root",
        );
        assert_eq!(
            a,
            vec![
                "vps",
                "register",
                "gate",
                "--ip",
                "home=10.0.0.1",
                "--ip",
                "office=10.0.0.2",
                "--port",
                "22",
                "--login",
                "root=$DEPLOY_KEY",
                "--note",
                "门口那台",
                "--root",
                "/data/root"
            ]
        );
        assert!(
            !a.iter().any(|s| s == "--yes"),
            "预审段铁律:argv 里不许出现 --yes(文件不落)"
        );
        assert!(
            !a.iter().any(|s| s == "--repo"),
            "vps 组只认 --root;--repo 是 source register 的外部源链接槽,喂进来 = 未知 flag"
        );
        // 空备注等于没给(上游 note 是唯一 omitempty 槽)
        let no_note = vps_register_preaudit("gate", &[], None, &[], Some(""), "/data/root");
        assert!(!no_note.iter().any(|s| s == "--note"), "空备注不下发");
    }

    #[test]
    fn vps_登记落账_等于预审加一枚_yes_不另改槽位() {
        let ips = vec![("home".to_string(), "10.0.0.1".to_string())];
        let logins = vec![("root".to_string(), "$DEPLOY_KEY".to_string())];
        let p = vps_register_preaudit("gate", &ips, Some(22), &logins, None, "/data/root");
        let c = vps_register_commit("gate", &ips, Some(22), &logins, None, "/data/root");
        assert_eq!(c, [p, vec!["--yes".into()]].concat(), "落账 = 预审 + --yes");
        assert_eq!(
            c.iter().filter(|s| *s == "--yes").count(),
            1,
            "确认闸恰好一枚"
        );
    }

    #[test]
    fn vps_ls_与_rm_只认_root_且rm无确认旗标() {
        assert_eq!(
            vps_ls("/data/root"),
            vec!["vps", "ls", "--root", "/data/root"]
        );
        assert_eq!(
            vps_rm("gate", "/data/root"),
            vec!["vps", "rm", "gate", "--root", "/data/root"]
        );
        for a in [vps_ls("/r"), vps_rm("gate", "/r")] {
            assert!(!a.iter().any(|s| s == "--repo"), "四组只认 --root");
            assert!(
                !a.iter().any(|s| s == "--yes"),
                "上游 rm 没有确认闸,面板不自己造一枚下发"
            );
        }
    }

    // ---- 工单 #54:tag 组(add 无确认闸 / ls 反查 / rm 两态) ----

    #[test]
    fn tag_ref的kind已知集合一处定义_坏行面板侧就拒() {
        assert_eq!(TAG_REF_KINDS, ["skill", "extension", "vps"]);
        // 两侧都非空 + kind 在集合内才认
        assert_eq!(
            parse_tag_ref("skill/deep-research"),
            Some(("skill".to_string(), "deep-research".to_string()))
        );
        assert_eq!(
            parse_tag_ref(" vps / gate "),
            Some(("vps".to_string(), "gate".to_string())),
            "两侧空白容忍(面板侧 Trim,不喂给引擎猜)"
        );
        for bad in [
            "skill/",   // 空条目
            "/gate",    // 空 kind
            "recipe/x", // kind 不在已知集合
            "skill",    // 没有分隔
        ] {
            assert_eq!(parse_tag_ref(bad), None, "坏行 `{bad}` 应被面板侧拒下");
        }
        // 上游口径(`tag/add.go` 第③道闸):**第一个斜杠切 kind**,skill 的 id
        // 自带斜杠(`<仓库>/<条目>`)——只认第一刀,不是坏行。
        assert_eq!(
            parse_tag_ref("skill/o/r/deep-research"),
            Some(("skill".to_string(), "o/r/deep-research".to_string())),
            "skill 的 id = <仓库>/<条目>,自带斜杠"
        );
    }

    #[test]
    fn tag_add_无确认闸_同一份argv可重跑_幂等由面板复用保证() {
        let refs = vec![
            ("skill".to_string(), "deep-research".to_string()),
            ("vps".to_string(), "gate".to_string()),
        ];
        let a = tag_add("pi", &refs, "/data/root");
        assert_eq!(
            a,
            vec![
                "tag",
                "add",
                "pi",
                "--ref",
                "skill/deep-research",
                "--ref",
                "vps/gate",
                "--root",
                "/data/root"
            ]
        );
        // 铁律:上游没有 --yes,面板**不自己造一枚**(多一枚未知 flag 直接 rc 3)
        assert!(!a.iter().any(|s| s == "--yes"), "tag add 没有确认闸");
        assert!(!a.iter().any(|s| s == "--repo"), "tag 组只认 --root");
        // 幂等:面板复用同一份 argv,重跑字节不变
        assert_eq!(tag_add("pi", &refs, "/data/root"), a);
        // 无引用也合法(标签可以先建后挂——追加合并)
        assert_eq!(
            tag_add("pi", &[], "/r"),
            vec!["tag", "add", "pi", "--root", "/r"]
        );
    }

    #[test]
    fn tag_ls两态_无参列全部_带参反查一条() {
        assert_eq!(
            tag_ls("/data/root"),
            vec!["tag", "ls", "--root", "/data/root"]
        );
        assert_eq!(
            tag_ls_one("pi", "/data/root"),
            vec!["tag", "ls", "pi", "--root", "/data/root"]
        );
        // 反查是只读:两个形态都不带任何写旗标
        for a in [tag_ls("/r"), tag_ls_one("pi", "/r")] {
            assert!(!a.iter().any(|s| s == "--yes" || s == "--ref"));
        }
    }

    #[test]
    fn tag_rm两态_带ref只摘引用_不带删整个标签() {
        let refs = vec![("extension".to_string(), "theme-pack".to_string())];
        assert_eq!(
            tag_rm("pi", &refs, "/data/root"),
            vec![
                "tag",
                "rm",
                "pi",
                "--ref",
                "extension/theme-pack",
                "--root",
                "/data/root"
            ]
        );
        // 不带 --ref = 删整个标签(引用行为空即这一态)
        assert_eq!(
            tag_rm("pi", &[], "/data/root"),
            vec!["tag", "rm", "pi", "--root", "/data/root"]
        );
        // 上游 rm 没有 --yes:面板的图形确认是面板侧编排,不下发旗标
        for a in [tag_rm("pi", &refs, "/r"), tag_rm("pi", &[], "/r")] {
            assert!(!a.iter().any(|s| s == "--yes"));
        }
    }

    #[test]
    fn 建议命令_带版本与出参默认值_旗标只有一份真相() {
        // 与 ls/up/doctor 同一处拼装:形状变了只改 recipe_bake 一处
        let a = recipe_bake(
            "research-desk",
            Some("1.0.0"),
            "research-desk.sh",
            "/data/repo",
            None,
            None,
        );
        assert_eq!(
            a,
            vec![
                "recipe",
                "bake",
                "research-desk@1.0.0",
                "--out",
                "research-desk.sh",
                "--repo",
                "/data/repo"
            ]
        );
        // 版本缺失:不猜,只给配方名
        assert_eq!(
            recipe_bake("desk", None, "desk.sh", "/data/repo", None, None)[2],
            "desk"
        );
        assert_eq!(bake_out_default("desk"), "desk.sh");
    }

    #[test]
    fn bake_两槽位都缺_与现状逐字节一致() {
        // 缺省态的 argv 就是 #93 之前那一条:七个元素,一个字节都不多。
        // 期望值是 #93 之前实测的原文(不是拿新实现反推的),所以这条测试
        // 钉的是「可选槽位的加入没有改动日常烘焙的命令」。
        let none = recipe_bake(
            "research-desk",
            Some("1.0.0"),
            "research-desk.sh",
            "/data/repo",
            None,
            None,
        );
        assert_eq!(
            none,
            vec![
                "recipe",
                "bake",
                "research-desk@1.0.0",
                "--out",
                "research-desk.sh",
                "--repo",
                "/data/repo"
            ]
        );
        assert_eq!(
            none.join(" "),
            "recipe bake research-desk@1.0.0 --out research-desk.sh --repo /data/repo",
            "空格拼接后的串也是原文:一个字节都不多"
        );
        // 空串 / 纯空白 / 没给,三种写法同一条 argv(可选槽位的「空」只有一种语义)
        for (a, b) in [
            (Some(""), None),
            (Some("   "), None),
            (None, Some("")),
            (None, Some("  ")),
            (Some(""), Some("")),
        ] {
            assert_eq!(
                recipe_bake(
                    "research-desk",
                    Some("1.0.0"),
                    "research-desk.sh",
                    "/data/repo",
                    a,
                    b
                ),
                none,
                "空槽位({a:?}, {b:?})不得给命令添一个字节"
            );
        }
    }

    #[test]
    fn bake_资产目录_空不落_非空落() {
        // 空值不落槽:让引擎按 release 预设算 sha256(面板不替引擎猜一份资产)
        let none = recipe_bake("desk", None, "desk.sh", "/data/repo", None, None);
        assert!(
            !none.iter().any(|s| s == "--assets-dir"),
            "没给就不落:{none:?}"
        );
        let blank = recipe_bake("desk", None, "desk.sh", "/data/repo", Some("  "), None);
        assert!(
            !blank.iter().any(|s| s == "--assets-dir"),
            "纯空白等同没给:{blank:?}"
        );
        // 非空才落,落点钉在 --repo 之后
        let with = recipe_bake(
            "desk",
            None,
            "desk.sh",
            "/data/repo",
            Some("/data/assets"),
            None,
        );
        assert_eq!(
            with,
            vec![
                "recipe",
                "bake",
                "desk",
                "--out",
                "desk.sh",
                "--repo",
                "/data/repo",
                "--assets-dir",
                "/data/assets",
            ]
        );
    }

    #[test]
    fn bake_取货仓_空不落_非空落() {
        // 空 = 让引擎从数据仓 git origin 自己推导取货仓(面板不复制这个推导)
        let none = recipe_bake("desk", None, "desk.sh", "/data/repo", None, None);
        assert!(
            !none.iter().any(|s| s == "--bundle-repo"),
            "没给就不落:{none:?}"
        );
        let blank = recipe_bake("desk", None, "desk.sh", "/data/repo", None, Some("   "));
        assert!(
            !blank.iter().any(|s| s == "--bundle-repo"),
            "纯空白等同没给:{blank:?}"
        );
        // 非空才落,落点在 --repo 之后(与资产目录同一条尾巴,顺序固定)
        let with = recipe_bake(
            "desk",
            None,
            "desk.sh",
            "/data/repo",
            None,
            Some("git@github.com:me/bundles.git"),
        );
        assert_eq!(
            with,
            vec![
                "recipe",
                "bake",
                "desk",
                "--out",
                "desk.sh",
                "--repo",
                "/data/repo",
                "--bundle-repo",
                "git@github.com:me/bundles.git",
            ]
        );
    }

    #[test]
    fn 建议命令_三前提齐备才算得出_且可复制() {
        // 两个可选槽位都缺:展示串就是 #93 之前那一条(逐字节一致,见
        // `bake_两槽位都缺_与现状逐字节一致`)——期望值是历史原文,不是反推的。
        let ok = recipe_bake_suggest(
            Some(("research-desk", Some("1.0.0"))),
            Some("/data/repo"),
            true,
            None,
            None,
        )
        .expect("三前提齐备必须给得出命令");
        assert_eq!(
            ok.display,
            "nw recipe bake research-desk@1.0.0 --out research-desk.sh --repo /data/repo"
        );
        assert!(
            ok.argv.iter().all(|a| !a.contains("--yes")),
            "建议命令永不带确认闸"
        );
        // 展示串永远由 argv 拼出:复制去 shell 敲与面板算出来的是同一条命令
        assert_eq!(ok.display, format!("nw {}", ok.argv.join(" ")));
        // 缺省态一个字节都不多:七个元素、两个可选旗标都不在场
        assert_eq!(ok.argv.len(), 7, "缺省态 argv 元素数不变:{:?}", ok.argv);
        assert!(!ok.display.contains("--assets-dir"));
        assert!(!ok.display.contains("--bundle-repo"));
    }

    #[test]
    fn 建议命令_两槽位非空_进argv与展示串_落点仍在repo之后() {
        // 资产目录 + 取货仓都设了:两条旗标按固定顺序接在 --repo 之后,
        // 且展示串仍由 argv 拼出(同源同层,没有第二处「命令长什么样」)。
        let ok = recipe_bake_suggest(
            Some(("desk", None)),
            Some("/data/repo"),
            true,
            Some("/data/assets"),
            Some("git@github.com:me/bundles.git"),
        )
        .expect("三前提齐备必须给得出命令(可选槽位不是前提)");
        assert_eq!(
            ok.argv,
            vec![
                "recipe",
                "bake",
                "desk",
                "--out",
                "desk.sh",
                "--repo",
                "/data/repo",
                "--assets-dir",
                "/data/assets",
                "--bundle-repo",
                "git@github.com:me/bundles.git",
            ]
        );
        assert_eq!(
            ok.display,
            "nw recipe bake desk --out desk.sh --repo /data/repo --assets-dir /data/assets \
             --bundle-repo git@github.com:me/bundles.git"
        );
        assert_eq!(ok.display, format!("nw {}", ok.argv.join(" ")));
        // 只设一个也成立:另一个照样不落(可选槽位各自独立)
        let only_assets = recipe_bake_suggest(
            Some(("desk", None)),
            Some("/data/repo"),
            true,
            Some("/data/assets"),
            None,
        )
        .expect("只设资产目录也算三前提齐备");
        assert!(!only_assets.display.contains("--bundle-repo"));
        assert!(only_assets.display.ends_with("--assets-dir /data/assets"));
    }

    #[test]
    fn 建议命令_三种缺前提各有可区分指路_绝不给半条命令() {
        // 选中行不是配方
        assert_eq!(
            recipe_bake_suggest(None, Some("/data/repo"), true, None, None),
            Err(BakeGate::NotARecipe)
        );
        // 未设数据仓根(不拿当前目录兜底)
        assert_eq!(
            recipe_bake_suggest(Some(("desk", None)), None, true, None, None),
            Err(BakeGate::NoDataRoot)
        );
        // 引擎没有 recipe bake 能力
        assert_eq!(
            recipe_bake_suggest(Some(("desk", None)), Some("/data/repo"), false, None, None),
            Err(BakeGate::NoCapability)
        );
        // #93:两个可选槽位都设了也不改变前提闸的结论——资产与取货仓是可选
        // 增强,不是前提。数据仓根未设时照旧只给指路,一个新字节都不落。
        for (a, b) in [
            (Some("/data/assets"), Some("git@github.com:me/bundles.git")),
            (Some("/data/assets"), None),
        ] {
            assert_eq!(
                recipe_bake_suggest(Some(("desk", None)), None, true, a, b),
                Err(BakeGate::NoDataRoot),
                "可选槽位填了也不放行半条命令"
            );
        }
        // 三种指路两两不同,互不冒充
        let gates = [
            BakeGate::NoDataRoot,
            BakeGate::NotARecipe,
            BakeGate::NoCapability,
        ];
        for (i, a) in gates.iter().enumerate() {
            for b in &gates[i + 1..] {
                assert_ne!(a.title(), b.title(), "标题两两不同");
                assert_ne!(a.advice(), b.advice(), "说法两两不同");
            }
        }
        assert!(BakeGate::NoCapability.advice().contains("不是面板坏了"));
        assert!(BakeGate::NoDataRoot.advice().contains("不拿当前目录兜底"));
    }

    #[test]
    fn 建议命令_指路人话不带连续空格_换行残留不入屏() {
        // NoDataRoot 的说法曾是一行拼接残留,中段混进 17 个空格,
        // 操作者在屏上会看到怪异空档——指路人话是一句正常句子。
        for gate in [
            BakeGate::NoDataRoot,
            BakeGate::NotARecipe,
            BakeGate::NoCapability,
        ] {
            let advice = gate.advice();
            assert!(
                !advice.contains("  "),
                "{} 的说法中段不许有一长串空格:{advice:?}",
                gate.title()
            );
        }
    }

    #[test]
    fn 建议命令_recipe_new_位置参数恰好一个_旗标与数据仓根同源() {
        // 形状实测自引擎 usage 行(所配 0.1.0):
        // nw recipe new <名> [--profile <名>] [--out-dir <目录>]
        assert_eq!(
            recipe_new("research-desk", None, "/data/repo"),
            vec!["recipe", "new", "research-desk", "--out-dir", "/data/repo"]
        );
        // 派名非空才给 --profile:上游缺省 = 按配方名兜底(#8 布局),面板不替引擎猜
        assert_eq!(
            recipe_new("desk", Some("desk"), "/data/repo"),
            vec![
                "recipe",
                "new",
                "desk",
                "--profile",
                "desk",
                "--out-dir",
                "/data/repo"
            ]
        );
        assert_eq!(
            recipe_new("desk", Some(""), "/data/repo"),
            recipe_new("desk", None, "/data/repo"),
            "空派名等于没给"
        );
    }

    #[test]
    fn 建议命令_recipe_new_三前提齐备_argv与展示串同源且无写旗标() {
        let ok = recipe_new_suggest(Some("research-desk"), Some("/data/repo"), true)
            .expect("三前提齐备必须给得出命令");
        assert_eq!(
            ok.display,
            "nw recipe new research-desk --out-dir /data/repo"
        );
        assert_eq!(
            ok.display,
            format!("nw {}", ok.argv.join(" ")),
            "展示串由 argv 拼出"
        );
        for a in &ok.argv {
            assert!(
                !a.contains("--yes") && !a.contains("--json"),
                "建议命令永不带写旗标/no-op 旗标:{a}"
            );
        }
    }

    #[test]
    fn 建议命令_recipe_new_三种缺前提各有可区分指路_不给半条命令() {
        assert_eq!(
            recipe_new_suggest(None, Some("/data/repo"), true),
            Err(RecipeNewGate::NotARecipe)
        );
        assert_eq!(
            recipe_new_suggest(Some("desk"), None, true),
            Err(RecipeNewGate::NoDataRoot)
        );
        assert_eq!(
            recipe_new_suggest(Some("desk"), Some("/data/repo"), false),
            Err(RecipeNewGate::NoCapability)
        );
        // 三种指路两两不同,互不冒充;且与 bake 的同名指路不同文(不冒充)
        let gates = [
            RecipeNewGate::NoDataRoot,
            RecipeNewGate::NotARecipe,
            RecipeNewGate::NoCapability,
        ];
        for (i, a) in gates.iter().enumerate() {
            for b in &gates[i + 1..] {
                assert_ne!(a.title(), b.title(), "标题两两不同");
                assert_ne!(a.advice(), b.advice(), "说法两两不同");
            }
        }
        assert_ne!(
            RecipeNewGate::NoDataRoot.advice(),
            BakeGate::NoDataRoot.advice(),
            "同为未设根,new 的说法要点名 --out-dir,不拿 bake 的话冒充"
        );
        assert_ne!(
            RecipeNewGate::NoCapability.advice(),
            BakeGate::NoCapability.advice(),
            "缺能力要说是缺哪一个"
        );
        assert!(RecipeNewGate::NoCapability.advice().contains("recipe new"));
        assert!(RecipeNewGate::NoDataRoot.advice().contains("--out-dir"));
        assert!(RecipeNewGate::NoDataRoot
            .advice()
            .contains("不拿当前目录兜底"));
    }

    #[test]
    fn 建议命令_recipe_new_指路人话不带连续空格_换行残留不入屏() {
        for gate in [
            RecipeNewGate::NoDataRoot,
            RecipeNewGate::NotARecipe,
            RecipeNewGate::NoCapability,
        ] {
            let advice = gate.advice();
            assert!(
                !advice.contains("  "),
                "{} 的说法中段不许有一长串空格:{advice:?}",
                gate.title()
            );
        }
    }

    #[test]
    fn 登记落账_带_yes_带_root_不带_category() {
        let a = source_register_commit("owner/repo", "skill", "/data/root");
        assert_eq!(
            a,
            vec![
                "source",
                "register",
                "--repo",
                "owner/repo",
                "--kind",
                "skill",
                "--root",
                "/data/root",
                "--yes"
            ]
        );
        assert!(
            a.iter().any(|s| s == "--yes"),
            "落账段铁律:确认闸必须在 argv 里(否则文件不落)"
        );
        assert!(
            !a.iter().any(|s| s == "--category"),
            "类别输入不给:--category 是 0.1.0 就有的 source register 可重复旗标,\
             不是引擎能力缺口;要不要给类别输入是产品范围问题(ADR-0006),\
             理由不能写成「引擎没这个能力」"
        );
        // 预审与落账只差确认闸一枚棋:同根、同链、不同后果
        let p = source_register_preaudit("owner/repo", "skill", "/data/root");
        let mut pc = p.clone();
        pc.push("--yes".into());
        assert_eq!(a, pc, "落账 = 预审 + --yes,不另改槽位");
    }

    // ---- 工单 #56:subscription 组(register 两段式经 stdin / ls / rm / get 只给建议) ----

    #[test]
    fn 订阅登记_argv面无链接本体_只经_stdin_声明与_root_进() {
        let p =
            subscription_register_preaudit("airport", "Mihomo", Some("2026-03-01"), "/data/root");
        assert_eq!(
            p,
            vec![
                "subscription",
                "register",
                "--name",
                "airport",
                "--client",
                "Mihomo",
                "--token-stdin",
                "--expires",
                "2026-03-01",
                "--root",
                "/data/root",
            ]
        );
        // 链接本体一个字节都不在 argv 里:上游只认 stdin(--token-stdin 是开关
        // 不是载荷;误粘的 argv 会进 ps / history)
        assert!(
            !p.iter().any(|s| s.contains("https://")),
            "argv 面没有任何明文链接形参:{p:?}"
        );
        assert!(
            !p.iter().any(|s| s == "--token" || s == "--password"),
            "上游 usage 禁 --token / --password 二字,面板不造这种旗标"
        );
        assert!(
            !p.iter().any(|s| s == "--yes"),
            "预审段连 --yes 的位置都没有"
        );
        assert!(
            !p.iter().any(|s| s == "--repo"),
            "subscription 组只认 --root"
        );
        // 到期日可省 = 永不过期:空串不产生 --expires 旗标(上游闸③认空串)
        let forever = subscription_register_preaudit("airport", "Mihomo", Some(""), "/data/root");
        assert!(
            !forever.iter().any(|s| s == "--expires"),
            "没给到期日不下发 --expires:{forever:?}"
        );
        assert_eq!(
            forever,
            subscription_register_preaudit("airport", "Mihomo", None, "/data/root"),
            "空串与没给是同一种语义(永不过期)"
        );
        // 落账 = 预审 + 恰好一枚 --yes,不另改槽位
        let c = subscription_register_commit("airport", "Mihomo", Some("2026-03-01"), "/data/root");
        assert_eq!(c, [p.clone(), vec!["--yes".into()]].concat());
        assert_eq!(c.iter().filter(|s| *s == "--yes").count(), 1);
    }

    #[test]
    fn 订阅_ls_只读_rm_只认_root_且无确认旗标() {
        assert_eq!(
            subscription_ls("/data/root"),
            vec!["subscription", "ls", "--root", "/data/root"]
        );
        assert_eq!(
            subscription_rm("airport", "/data/root"),
            vec!["subscription", "rm", "airport", "--root", "/data/root"]
        );
        for a in [subscription_ls("/r"), subscription_rm("airport", "/r")] {
            assert!(!a.iter().any(|s| s == "--repo"), "四组只认 --root");
            assert!(
                !a.iter().any(|s| s == "--yes"),
                "上游 rm 没有确认闸,面板不自己造一枚下发"
            );
        }
    }

    #[test]
    fn 订阅_get_只给建议命令_带管道且永不带写旗标() {
        let s = subscription_get_suggest("airport", "/data/root");
        // 展示串照上游 usage 的形态:要裸链接自己拼管道(上游不开 --raw)
        assert_eq!(
            s.display,
            "nw subscription get --name airport --root /data/root | jq -r .data.url"
        );
        // argv 是完整可执行的一条(不含管道段),旗标一处真相
        assert_eq!(s.argv[..3], ["subscription", "get", "--name"]);
        assert!(!s.display.contains("--yes"), "get 是只读出口,永不带写旗标");
        assert!(!s.display.contains("--raw"), "上游不开 --raw,面板也不造");
        assert!(
            s.argv.contains(&"--root".to_string()),
            "root 槽照面板既有纪律显式给,不让引擎拿当前目录兜底"
        );
        assert_eq!(
            s,
            subscription_get_suggest("airport", "/data/root"),
            "纯函数:同名同根永远同一条命令(可测、可复制)"
        );
    }

    // ---- 工单 #57:key 组(register 两段式经 stdin / ls / rm / get 只给建议) ----

    #[test]
    fn 钥匙登记_argv面无本体_只经_stdin_声明与_root_进() {
        let p = key_register_preaudit("deploy", "github.com", &[], None, "/data/root");
        assert_eq!(
            p,
            vec![
                "key",
                "register",
                "--name",
                "deploy",
                "--token-stdin",
                "--host",
                "github.com",
                "--root",
                "/data/root",
            ]
        );
        // 本体一个字节都不在 argv 里(上游只认 stdin;误粘的 argv 会进 ps / history)
        assert!(
            !p.iter().any(|s| s == "--token" || s == "--password"),
            "上游 usage 禁 --token / --password 二字,面板不造:{p:?}"
        );
        assert!(
            !p.iter().any(|s| s == "--yes"),
            "预审段连 --yes 的位置都没有"
        );
        assert!(!p.iter().any(|s| s == "--repo"), "key 组只认 --root");
        // host 空 = 缺省 github.com:不产生 --host 旗标(显式 --host "" 会被第②道闸拒)
        let no_host = key_register_preaudit("deploy", "", &[], None, "/data/root");
        assert!(
            !no_host.iter().any(|s| s == "--host"),
            "host 空不下发 --host:{no_host:?}"
        );
        // 作用域:只登记不设闸,逐条 --scope 落槽;空 scope 面板侧就滤掉不喂给引擎
        let scopes = vec!["repo".to_string(), "read:org".to_string(), "  ".to_string()];
        let with_scopes = key_register_preaudit("deploy", "github.com", &scopes, None, "/r");
        assert_eq!(
            with_scopes.iter().filter(|s| *s == "--scope").count(),
            2,
            "空 scope 被滤掉:{with_scopes:?}"
        );
        assert!(with_scopes.contains(&"repo".to_string()));
        assert!(with_scopes.contains(&"read:org".to_string()));
        // 轮换日:空 = 缺省登记日 UTC,不产生 --rotated-at 旗标
        let no_rot = key_register_preaudit("deploy", "github.com", &[], Some(""), "/r");
        assert!(
            !no_rot.iter().any(|s| s == "--rotated-at"),
            "轮换日空不下发 --rotated-at:{no_rot:?}"
        );
        assert_eq!(
            no_rot,
            key_register_preaudit("deploy", "github.com", &[], None, "/r"),
            "空串与没给同义(缺省登记日)"
        );
        let rot = key_register_preaudit("deploy", "github.com", &[], Some("2026-03-01"), "/r");
        assert!(
            rot.contains(&"--rotated-at".to_string()) && rot.contains(&"2026-03-01".to_string())
        );
        // 落账 = 预审 + 恰好一枚 --yes,不另改槽位
        let c = key_register_commit("deploy", "github.com", &[], None, "/data/root");
        assert_eq!(c, [p.clone(), vec!["--yes".into()]].concat());
        assert_eq!(c.iter().filter(|s| *s == "--yes").count(), 1);
    }

    #[test]
    fn 钥匙_ls_只读_rm_只认_root_且无确认旗标() {
        assert_eq!(
            key_ls("/data/root"),
            vec!["key", "ls", "--root", "/data/root"]
        );
        assert_eq!(
            key_rm("deploy", "/data/root"),
            vec!["key", "rm", "deploy", "--root", "/data/root"]
        );
        for a in [key_ls("/r"), key_rm("deploy", "/r")] {
            assert!(!a.iter().any(|s| s == "--repo"), "四组只认 --root");
            assert!(
                !a.iter().any(|s| s == "--yes"),
                "上游 rm 没有确认闸,面板不自己造一枚下发"
            );
        }
    }

    // ---- 工单 #92:validate argv 族补全(单侧两态显式带 --repo + quiet 留位) ----

    #[test]
    fn validate_单侧配方_显式带repo_不让引擎拿当前目录兜底() {
        // 与 ls --repo / bake --repo 同源同纪律:数据仓根恒落,签名里没有
        // 「不带 repo」的写法(与 ls 同一强制手法,见 ls_必带_repo_不给引擎当前目录兜底)
        assert_eq!(
            validate_recipe("recipes/research-desk/recipe.yaml", "/data/repo", false),
            vec![
                "validate",
                "--recipe",
                "recipes/research-desk/recipe.yaml",
                "--repo",
                "/data/repo"
            ]
        );
    }

    #[test]
    fn validate_单侧锁定集_显式带repo_跨文件项靠它发现配方与组件清单() {
        // 单侧 --lock 时上游用 --repo 发现配方与组件清单(spec #89 Further
        // Notes):少了它,1–19 项跨文件检查在命令路径上跑不全
        assert_eq!(
            validate_lock("baked/nw-assemble.sh", "/data/repo", false),
            vec![
                "validate",
                "--lock",
                "baked/nw-assemble.sh",
                "--repo",
                "/data/repo"
            ]
        );
        // 两态都不让 --repo 缺席:兜底当前目录是既有纪律明令排除的那条路
        for a in [
            validate_recipe("r.yaml", "/r", false),
            validate_lock("x.nwlock", "/r", false),
        ] {
            assert!(a.contains(&"--repo".to_string()), "{a:?} 必须带 --repo");
        }
    }

    #[test]
    fn validate_双侧同给_配方与锁定集一次跑全_旗标顺序稳定可断言() {
        // spec #89 工单 #98 的第三态:`--recipe` + `--lock` 同给,27 项里的
        // 跨文件项(1–19)一次跑全,不靠单侧模式的发现兜底。旗标顺序冻结:
        // validate → --recipe → --lock → --repo(quiet 留位仍在最末)。
        assert_eq!(
            validate_both(
                "recipes/research-desk/recipe.yaml",
                "/root/.nous-warehouse/locks/9f8e.nwlock",
                "/data/repo",
                false
            ),
            vec![
                "validate",
                "--recipe",
                "recipes/research-desk/recipe.yaml",
                "--lock",
                "/root/.nous-warehouse/locks/9f8e.nwlock",
                "--repo",
                "/data/repo"
            ]
        );
        // 顺序不是「恰好这七个元素」而已:--recipe 必在 --lock 之前、
        // --repo 必在两侧之后(上游 usage 的形状,可断言所以不变)
        let a = validate_both("r.yaml", "x.nwlock", "/r", false);
        let i = |f: &str| a.iter().position(|s| s == f).expect("旗标在场");
        assert!(i("--recipe") < i("--lock"), "配方侧在前:{a:?}");
        assert!(i("--lock") < i("--repo"), "锁定集侧在数据仓根之前:{a:?}");
        assert_eq!(a.len(), 7, "双侧七元素,quiet 留位缺省不落:{a:?}");
        // quiet 留位槽位(#92)在双侧同给上同一纪律:缺省不落、给了才落,
        // 落点仍在 --repo 之后(argv 末位)
        assert!(!a.iter().any(|s| s == "--quiet"), "{a:?} 缺省不落 quiet");
        let q = validate_both("r.yaml", "x.nwlock", "/r", true);
        assert_eq!(q.last().map(String::as_str), Some("--quiet"), "{q:?}");
        assert_eq!(q.len(), 8, "双侧给了 quiet 才多一枚:{q:?}");
    }

    #[test]
    fn validate_quiet_缺省不落_给了才落_落点稳定在_repo_之后() {
        // 缺省:一枚 --quiet 都不落(与加这个留位槽之前逐字节同形)
        for a in [
            validate_recipe("r.yaml", "/data/repo", false),
            validate_lock("x.nwlock", "/data/repo", false),
        ] {
            assert!(!a.iter().any(|s| s == "--quiet"), "{a:?} 缺省不落 quiet");
        }
        // 显式给:恰好一枚,且落在 argv 末尾(--repo 之后)——位置稳定可断言
        for a in [
            validate_recipe("r.yaml", "/data/repo", true),
            validate_lock("x.nwlock", "/data/repo", true),
        ] {
            assert_eq!(a.last().map(String::as_str), Some("--quiet"), "{a:?}");
            assert_eq!(
                a.iter().filter(|s| *s == "--quiet").count(),
                1,
                "quiet 恰好一枚:{a:?}"
            );
            let i = a.iter().position(|s| s == "--repo").expect("带了 repo");
            assert_eq!(a[i + 1], "/data/repo", "--repo 的值紧随其后");
            assert_eq!(a[i + 2], "--quiet", "quiet 落在 --repo 之后");
        }
        // quiet 只此一枚:给了不多带别的旗标,长度恰好多一个元素
        assert_eq!(
            validate_recipe("r.yaml", "/data/repo", true).len(),
            validate_recipe("r.yaml", "/data/repo", false).len() + 1
        );
    }

    #[test]
    fn 钥匙_get_只给建议命令_带管道且永不带写旗标() {
        let s = key_get_suggest("deploy", "/data/root");
        // 展示串照上游 usage 的形态:要裸钥匙自己拼管道(上游不开 --raw)
        assert_eq!(
            s.display,
            "nw key get --name deploy --root /data/root | jq -r .data.key"
        );
        assert_eq!(s.argv[..3], ["key", "get", "--name"]);
        assert!(!s.display.contains("--yes"), "get 是只读出口,永不带写旗标");
        assert!(!s.display.contains("--raw"), "上游不开 --raw,面板也不造");
        assert!(
            s.argv.contains(&"--root".to_string()),
            "root 槽照面板既有纪律显式给"
        );
        assert_eq!(
            s,
            key_get_suggest("deploy", "/data/root"),
            "纯函数:同名同根永远同一条命令"
        );
    }
}
