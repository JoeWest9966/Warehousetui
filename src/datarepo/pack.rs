//! 组件打包器(spec #95 工单 #112):数据仓写侧「单文件 + sha256」的消费方
//! (ADR-0010 §4 表格里 #112 那一行)。
//!
//! 选中数据仓内一个本地 skill / extension 目录 → 面板自己算**目录校验和**、
//! 生成**组件清单**(`component.yaml`)→ 显示校验和与拟生成清单 → 图形确认 →
//! 经 [`write::DataRepo::commit`] 原子落盘。确认前一个字节都不落
//! ([`plan_pack`] 只算拟落账)。
//!
//! 两件「面板自有实现」的事实底稿:
//! - **sha256**:标准库没有,本模块自实现(FIPS 180-4),与 `sha256sum` 逐字节
//!   一致(测试对拍);不引新依赖。
//! - **校验和算法**:与上游 `internal/manifest.DirHash` 同形 —— 逐文件
//!   「相对路径 + 内容 sha256」一行,按行字典序拼起来再 sha256;排除
//!   `.git/` 与 `node_modules/`(#6 D8:依赖随资产下发但不参与身份)。
//!   于是面板算出的校验和 = 上游 bake / `up` 步 3 后来在同一棵树上算出的值,
//!   不是面板自造的另一套数。清单本身进树(它落盘后就在树上),所以
//!   [`plan_pack`] 算的是**落盘后那棵树**的校验和 —— 由
//!   [`tests::校验和含本清单_与落盘后的目录逐字节一致`] 钉死。
//!
//! 上游依据:`docs/external-sources.md`(本地组件仍住数据仓、bake 消费清单
//! 不生产清单)、数据仓现有组件清单样本
//! (`docs/research/nw-journeys-and-docs.md` 的 component.yaml 形状)。

use std::path::Path;

use crate::datarepo::write::{self, DataRepo, PlannedWrite, RepoRelPath, WriteReceipt};

/// 组件清单的文件名:上游样本 `components/scout/component.yaml`。
pub const MANIFEST_FILE: &str = "component.yaml";

/// 清单里 `apiVersion` 的握手值:样本 `apiVersion: nw/v1`。
pub const MANIFEST_API_VERSION: &str = "nw/v1";

/// 清单里 `kind` 的值:样本 `kind: component`。
pub const MANIFEST_KIND: &str = "component";

// ---- sha256(FIPS 180-4,面板自有实现) ----

/// SHA-256 的 64 个轮常量(FIPS 180-4 §4.2.2)。
const K: [u32; 64] = [
    0x428a_2f98,
    0x7137_4491,
    0xb5c0_fbcf,
    0xe9b5_dba5,
    0x3956_c25b,
    0x59f1_11f1,
    0x923f_82a4,
    0xab1c_5ed5,
    0xd807_aa98,
    0x1283_5b01,
    0x2431_85be,
    0x550c_7dc3,
    0x72be_5d74,
    0x80de_b1fe,
    0x9bdc_06a7,
    0xc19b_f174,
    0xe49b_69c1,
    0xefbe_4786,
    0x0fc1_9dc6,
    0x240c_a1cc,
    0x2de9_2c6f,
    0x4a74_84aa,
    0x5cb0_a9dc,
    0x76f9_88da,
    0x983e_5152,
    0xa831_c66d,
    0xb003_27c8,
    0xbf59_7fc7,
    0xc6e0_0bf3,
    0xd5a7_9147,
    0x06ca_6351,
    0x1429_2967,
    0x27b7_0a85,
    0x2e1b_2138,
    0x4d2c_6dfc,
    0x5338_0d13,
    0x650a_7354,
    0x766a_0abb,
    0x81c2_c92e,
    0x9272_2c85,
    0xa2bf_e8a1,
    0xa81a_664b,
    0xc24b_8b70,
    0xc76c_51a3,
    0xd192_e819,
    0xd699_0624,
    0xf40e_3585,
    0x106a_a070,
    0x19a4_c116,
    0x1e37_6c08,
    0x2748_774c,
    0x34b0_bcb5,
    0x391c_0cb3,
    0x4ed8_aa4a,
    0x5b9c_ca4f,
    0x682e_6ff3,
    0x748f_82ee,
    0x78a5_636f,
    0x84c8_7814,
    0x8cc7_0208,
    0x90be_fffa,
    0xa450_6ceb,
    0xbef9_a3f7,
    0xc671_78f2,
];

/// 一段字节的 sha256 十六进制小写摘要(与 `sha256sum` 的输出同形同值)。
///
/// 自实现而不引 crate:标准库没有 sha256,本仓又没有 `sha2` 一类的依赖
/// (Cargo.toml 零散加密依赖),为一个摘要引一条依赖链不值。
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut h: [u32; 8] = [
        0x6a09_e667,
        0xbb67_ae85,
        0x3c6e_f372,
        0xa54f_f53a,
        0x510e_527f,
        0x9b05_688c,
        0x1f83_d9ab,
        0x5be0_cd19,
    ];
    // 填充:0x80 + 0x00 补到 56 mod 64,再 8 字节大端位长。
    let bit_len = (bytes.len() as u64).wrapping_mul(8);
    let mut msg = Vec::with_capacity(bytes.len() + 72);
    msg.extend_from_slice(bytes);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    for block in msg.chunks_exact(64) {
        let mut w = [0u32; 64];
        for (i, slot) in w.iter_mut().enumerate().take(16) {
            *slot = u32::from_be_bytes([
                block[4 * i],
                block[4 * i + 1],
                block[4 * i + 2],
                block[4 * i + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let mut a = h[0];
        let mut b = h[1];
        let mut c = h[2];
        let mut d = h[3];
        let mut e = h[4];
        let mut f = h[5];
        let mut g = h[6];
        let mut hh = h[7];
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (slot, v) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *slot = slot.wrapping_add(v);
        }
    }

    let mut out = String::with_capacity(64);
    for word in h {
        for byte in word.to_be_bytes() {
            out.push(char::from_digit((byte >> 4) as u32, 16).unwrap_or('0'));
            out.push(char::from_digit((byte & 0x0f) as u32, 16).unwrap_or('0'));
        }
    }
    out
}

// ---- 目录校验和(上游 DirHash 同算法) ----

/// 一棵目录树里的一个文件:相对路径(一律 `/` 分隔)+ 内容 sha256 + 字节数。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PayloadFile {
    pub rel: String,
    pub sha256: String,
    pub bytes: u64,
}

/// 排除表:整棵跳过的目录名。上游 `internal/manifest` 的 WalkTree 同表
/// (#6 D8:vendored 依赖随资产下发,但不参与身份 —— 否则校验和被依赖树的
/// 解析顺序与平台差异绑死)。
const SKIP_DIRS: [&str; 2] = ["node_modules", ".git"];

/// 列出目录树里参与身份的文件(排除 [`SKIP_DIRS`],按相对路径排序)。
///
/// 只读:一个字节都不写。路径知识住在数据仓访问层,屏幕层拿回去的是
/// 相对路径串,自己拼不出绝对路径。
pub fn payload_files(repo: &DataRepo, dir_rel: &str) -> Result<Vec<PayloadFile>, PackError> {
    let rel = RepoRelPath::new(dir_rel).map_err(PackError::BadSource)?;
    let base = repo.root_path();
    let dir = base.join(rel.as_str());
    if !dir.exists() {
        return Err(PackError::SourceMissing(rel.as_str().to_string()));
    }
    if !dir.is_dir() {
        return Err(PackError::SourceNotDir(rel.as_str().to_string()));
    }
    let mut out = Vec::new();
    walk(&dir, &dir, &mut out).map_err(PackError::Walk)?;
    out.sort_by(|a, b| a.rel.cmp(&b.rel));
    if out.is_empty() {
        return Err(PackError::EmptyPayload(rel.as_str().to_string()));
    }
    Ok(out)
}

/// 递归收集(深度优先,顺序由调用方排序保证确定)。
fn walk(base: &Path, dir: &Path, out: &mut Vec<PayloadFile>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("{}: {e}", dir.display()))?;
        let name = entry.file_name().to_string_lossy().to_string();
        let ft = entry
            .file_type()
            .map_err(|e| format!("{}: {e}", entry.path().display()))?;
        if ft.is_dir() {
            // 排除表整棵跳过(根自己不在表内,与上游同)。
            if SKIP_DIRS.contains(&name.as_str()) {
                continue;
            }
            walk(base, &entry.path(), out)?;
            continue;
        }
        let bytes =
            std::fs::read(entry.path()).map_err(|e| format!("{}: {e}", entry.path().display()))?;
        let rel = entry
            .path()
            .strip_prefix(base)
            .map_err(|e| format!("{}: {e}", entry.path().display()))?
            .to_string_lossy()
            .replace('\\', "/");
        out.push(PayloadFile {
            rel,
            sha256: sha256_hex(&bytes),
            bytes: bytes.len() as u64,
        });
    }
    Ok(())
}

/// 目录树的稳定摘要(上游 `manifest.DirHash` 同算法):逐文件一行
/// `"<相对路径> <内容 sha256>"`,按行字典序拼,每行后显式写一个 `\n`
/// (避免相邻行拼接歧义),整体再 sha256。
///
/// `manifest` = 即将落进这棵树的清单(`(相对路径, 内容)`):清单算进摘要,
/// 于是**落盘那一刻起的这棵树**的校验和就是这里算出的值(上游 bake / up
/// 步 3 后来在同一棵树上算,得到同一个数)。树上已有同名清单时替换,不重复。
pub fn dir_digest(files: &[PayloadFile], manifest: Option<(&str, &[u8])>) -> String {
    let mut lines: Vec<String> = files
        .iter()
        .map(|f| format!("{} {}", f.rel, f.sha256))
        .collect();
    if let Some((rel, bytes)) = manifest {
        let line = format!("{rel} {}", sha256_hex(bytes));
        match lines.iter_mut().find(|l| l.starts_with(&format!("{rel} "))) {
            Some(slot) => *slot = line,
            None => lines.push(line),
        }
    }
    lines.sort();
    let mut hasher_input = String::new();
    for l in &lines {
        hasher_input.push_str(l);
        hasher_input.push('\n');
    }
    sha256_hex(hasher_input.as_bytes())
}

// ---- 打包表单 → 拟落账 ----

/// 打包表单(屏幕层只填意图级字段;路径拼接全在接缝里)。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PackDraft {
    /// 数据仓根之下的相对目录(被打包的 skill / extension 目录)。
    pub source: String,
    /// 组件名(落进清单 `name`)。
    pub name: String,
    /// 组件版本(落进清单 `version`;本地组件必填)。
    pub version: String,
}

/// 拟落账:确认闸摊开的形状 —— 校验和 + 拟生成清单全文 + 将写文件 +
/// 体量,以及可直接递去 `commit` 的 [`write::WritePlan`](写侧拟落账)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackPlan {
    pub source: String,
    pub name: String,
    pub version: String,
    /// 目录校验和(含本清单;上游 DirHash 同算法)。
    pub digest: String,
    /// 参与身份的文件数(不含清单)。
    pub files: usize,
    /// 参与身份的字节数(不含清单)。
    pub bytes: u64,
    /// 拟生成的组件清单全文(确认闸原样呈现)。
    pub manifest: String,
    /// 将写文件(数据仓根之下的相对路径)。
    pub target: String,
    /// 写侧拟落账:`commit_pack` 只递它,一个字节都不多写。
    pub write: write::WritePlan,
}

/// 拟打包:读目录树、算校验和、生成清单、过写前校验 —— **一个字节都不落**。
///
/// 面板侧就拒的(形状不合 / 目录不在 / 空目录)留表单可改,不喂给写侧跑一趟。
pub fn plan_pack(repo: &DataRepo, draft: &PackDraft) -> Result<PackPlan, PackError> {
    let source = draft.source.trim();
    if source.is_empty() {
        return Err(PackError::NoSource);
    }
    let name = draft.name.trim();
    check_name(name)?;
    let version = draft.version.trim();
    check_version(version)?;
    let files = payload_files_via(repo, source)?;
    let manifest = component_manifest(name, version);
    let target = format!("{source}/{MANIFEST_FILE}");
    let rel = RepoRelPath::new(&target).map_err(PackError::BadTarget)?;
    let digest = dir_digest(&files, Some((MANIFEST_FILE, manifest.as_bytes())));
    let write = repo
        .plan(vec![PlannedWrite::text(rel, &manifest)])
        .map_err(PackError::WritePlan)?;
    Ok(PackPlan {
        source: source.to_string(),
        name: name.to_string(),
        version: version.to_string(),
        digest,
        files: files.len(),
        bytes: files.iter().map(|f| f.bytes).sum(),
        manifest,
        target: write.writes[0].path.as_str().to_string(),
        write,
    })
}

/// 经写侧句柄列文件(句柄持根,屏幕层不碰路径)。
fn payload_files_via(repo: &DataRepo, dir_rel: &str) -> Result<Vec<PayloadFile>, PackError> {
    payload_files(repo, dir_rel)
}

/// 确认闸放行后才调:把拟落账递去写侧原子落盘(tmp + rename)。
pub fn commit_pack(repo: &DataRepo, plan: &PackPlan) -> Result<WriteReceipt, write::WriteError> {
    repo.commit(&plan.write)
}

/// 组件清单全文(字节确定):schema 以数据仓现有组件为样本钉死 ——
/// `docs/research/nw-journeys-and-docs.md` 的 component.yaml 样本键序
/// `apiVersion/kind/name/version/pi/packages/vendored/depends/params/
/// config/env/requires` 一个不少、一个不多。
///
/// 上游 bake 消费这些文件、不生产它们:清单形状是面板自有实现,但对齐样本。
/// 空集合一律显式空写法(`[]` / `{}`)——样本里这些键都在,不打成「键缺失」。
pub fn component_manifest(name: &str, version: &str) -> String {
    format!(
        "apiVersion: {MANIFEST_API_VERSION}\n\
         kind: {MANIFEST_KIND}\n\
         name: {name}\n\
         version: {version}\n\
         pi: {{ package: \".\" }}\n\
         packages: []\n\
         vendored: false\n\
         depends: []\n\
         params: []\n\
         config: {{}}\n\
         env: []\n\
         requires: []\n"
    )
}

/// 组件名形状:字母 / 数字 / `-` / `_` / `.`,≤64,不以 `.` / `-` 开头
/// (样本 `scout` / `retention` 同形;`/` 一律不容——名字不是路径)。
fn check_name(name: &str) -> Result<(), PackError> {
    let bad = name.is_empty()
        || name.chars().count() > 64
        || name.starts_with('.')
        || name.starts_with('-')
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if bad {
        return Err(PackError::BadName(name.to_string()));
    }
    Ok(())
}

/// 版本形状:`<数字>.<数字>.<数字>` —— 本地组件的 `version` 必填且被上游
/// validate 查(`recipe.version_match`),不收空、不收 `latest` 一类别名。
fn check_version(version: &str) -> Result<(), PackError> {
    let parts: Vec<&str> = version.split('.').collect();
    let ok = parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()));
    if !ok {
        return Err(PackError::BadVersion(version.to_string()));
    }
    Ok(())
}

/// 拟打包阶段的拒件(面板侧就拒,留表单可改;人话供屏上直接摆)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PackError {
    /// 源目录没填。
    NoSource,
    /// 源目录不是合法的数据仓相对路径。
    BadSource(write::RepoRelPathError),
    /// 源目录不在数据仓根之下。
    SourceMissing(String),
    /// 源路径在,但不是目录。
    SourceNotDir(String),
    /// 目录里没有可打包的文件(排除 `.git/` `node_modules/` 之后)。
    EmptyPayload(String),
    /// 组件名形状不合。
    BadName(String),
    /// 版本形状不合。
    BadVersion(String),
    /// 将写路径构造不出(防线加深才够得到)。
    BadTarget(write::RepoRelPathError),
    /// 读目录树失败(权限 / IO)。
    Walk(String),
    /// 写前校验没过(拟落账为空 / 重复目标)。
    WritePlan(write::WritePlanError),
}

impl PackError {
    /// 人话原因(不带数据仓根:根是接缝内部的事,屏上只摆相对路径)。
    pub fn why(&self) -> String {
        match self {
            Self::NoSource => "源目录没填:要打包数据仓里的哪个目录".into(),
            Self::BadSource(e) => format!("源目录不是数据仓根之下的相对路径:{}", e.why()),
            Self::SourceMissing(p) => {
                format!("数据仓里找不到目录 `{p}`:要打包的 skill / extension 目录不在")
            }
            Self::SourceNotDir(p) => format!("`{p}` 不是目录:打包的是一棵目录树"),
            Self::EmptyPayload(p) => {
                format!("`{p}` 里没有可打包的文件(排除 .git/ 与 node_modules/ 之后)")
            }
            Self::BadName(n) => {
                format!("组件名 `{n}` 形状不合:只用字母/数字/-/_/.(≤64,不以 . 或 - 开头)")
            }
            Self::BadVersion(v) => {
                format!("版本 `{v}` 形状不合:形如 1.0.0(本地组件的 version 必填)")
            }
            Self::BadTarget(e) => format!("将写路径构造不出:{}", e.why()),
            Self::Walk(r) => format!("读目录树失败:{r}"),
            Self::WritePlan(e) => format!("拟落账校验没过:{}", e.why()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::process::Command;

    fn fixture(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nwt-pack-{}-{}", std::process::id(), tag));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn repo(root: &Path) -> DataRepo {
        DataRepo::new(root.to_str().unwrap())
    }

    fn draft(source: &str, name: &str, version: &str) -> PackDraft {
        PackDraft {
            source: source.into(),
            name: name.into(),
            version: version.into(),
        }
    }

    // ---- sha256:已知向量 + 与 sha256sum 工具对拍 ----

    #[test]
    fn sha256_已知向量_与fips180_4一致() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"The quick brown fox jumps over the lazy dog"),
            "d7a8fbb307d7809469ca9abcb0082e4f8d5651e46d3cdb762d02d0bf37c9e592"
        );
        // 跨块(1000 字节 = 15 个整块 + 余量)
        assert_eq!(
            sha256_hex(b"x".repeat(1000).as_slice()),
            "44f8354494a5ba03ba1792a8d3e9c534c47a9181980fde7a3f44b06ef2ae7c7f"
        );
        // 填充边界(55 / 56 / 57 字节三条路)都出 64 hex
        for n in [0usize, 1, 55, 56, 57, 64, 119, 120] {
            assert_eq!(sha256_hex(&vec![b'a'; n]).len(), 64, "{n} 字节也出 64 hex");
        }
    }

    #[test]
    fn sha256_与sha256sum工具对拍_字节级一致() {
        // 夹具内容刻意混排:短行、长行、跨块、含空字节(校验和是字节级的事)
        let cases: Vec<Vec<u8>> = vec![
            b"".to_vec(),
            b"abc\n".to_vec(),
            vec![0u8, 1, 2, 255, 254],
            "组件打包器\n".repeat(40).into_bytes(),
        ];
        let dir = fixture("sha-tool");
        for (i, bytes) in cases.iter().enumerate() {
            let path = dir.join(format!("blob-{i}"));
            fs::write(&path, bytes).unwrap();
            let out = Command::new("sha256sum").arg(&path).output();
            let Ok(out) = out else {
                // 平台上没有 sha256sum(多平台目标):已知向量那条已守算法,
                // 这里静默跳过,不把「工具不在」伪装成「对拍通过」。
                eprintln!("sha256sum 不在:跳过工具对拍({i})");
                continue;
            };
            assert!(out.status.success(), "sha256sum 应跑成功");
            let text = String::from_utf8_lossy(&out.stdout);
            let want = text
                .split_whitespace()
                .next()
                .expect("sha256sum 首列是摘要");
            assert_eq!(&sha256_hex(bytes), want, "第 {i} 个用例应与 sha256sum 一致");
        }
        fs::remove_dir_all(&dir).ok();
    }

    // ---- 目录校验和:算法 / 排除表 / 确定性 ----

    #[test]
    fn 目录校验和_逐文件与sha256sum一致_排除git与node_modules_且确定() {
        let root = fixture("digest");
        let payload = root.join("payloads/web-scout");
        fs::create_dir_all(payload.join("references")).unwrap();
        fs::write(payload.join("SKILL.md"), "---\nname: web-scout\n---\n").unwrap();
        fs::write(payload.join("references/triggers.md"), "hi\n").unwrap();
        fs::create_dir_all(payload.join(".git/objects")).unwrap();
        fs::write(payload.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        fs::create_dir_all(payload.join("node_modules/zod")).unwrap();
        fs::write(
            payload.join("node_modules/zod/index.js"),
            "module.exports={}\n",
        )
        .unwrap();

        let files = payload_files(&repo(&root), "payloads/web-scout").unwrap();
        let rels: Vec<&str> = files.iter().map(|f| f.rel.as_str()).collect();
        assert_eq!(
            rels,
            vec!["SKILL.md", "references/triggers.md"],
            ".git/ 与 node_modules/ 整棵排除(上游排除表同)"
        );
        // 逐文件与 sha256sum 对拍
        for f in &files {
            let out = Command::new("sha256sum")
                .arg(payload.join(&f.rel))
                .output()
                .expect("sha256sum 应可跑");
            let want = String::from_utf8_lossy(&out.stdout)
                .split_whitespace()
                .next()
                .unwrap()
                .to_string();
            assert_eq!(f.sha256, want, "`{}` 的 sha256 应与工具一致", f.rel);
        }
        // 确定性:两次算同一个数;内容一改,数就变
        let d1 = dir_digest(&files, None);
        assert_eq!(d1, dir_digest(&files, None), "摘要确定");
        fs::write(payload.join("SKILL.md"), "---\nname: web-scout!\n---\n").unwrap();
        let files2 = payload_files(&repo(&root), "payloads/web-scout").unwrap();
        assert_ne!(d1, dir_digest(&files2, None), "内容变 = 校验和变");
        fs::remove_dir_all(&root).ok();
    }

    // ---- 拟落账:确认前零字节 + 清单字节确定 + 形状对齐样本 ----

    #[test]
    fn 拟落账_零字节_清单字节确定_键序与样本一致() {
        let root = fixture("plan");
        let payload = root.join("payloads/web-scout");
        fs::create_dir_all(&payload).unwrap();
        fs::write(payload.join("SKILL.md"), "---\nname: web-scout\n---\n").unwrap();
        let r = repo(&root);
        let plan = plan_pack(&r, &draft("payloads/web-scout", "web-scout", "1.0.0")).unwrap();
        // 将写文件:清单落在被打包目录里(那正是「这个目录成了组件」)
        assert_eq!(plan.target, "payloads/web-scout/component.yaml");
        // 清单全文:与样本同键序,字节确定
        assert_eq!(
            plan.manifest,
            "apiVersion: nw/v1\n\
             kind: component\n\
             name: web-scout\n\
             version: 1.0.0\n\
             pi: { package: \".\" }\n\
             packages: []\n\
             vendored: false\n\
             depends: []\n\
             params: []\n\
             config: {}\n\
             env: []\n\
             requires: []\n"
        );
        // 样本键一个不少(形状一致性按键名清点)
        for key in [
            "apiVersion",
            "kind",
            "name",
            "version",
            "pi: { package:",
            "packages:",
            "vendored:",
            "depends:",
            "params:",
            "config:",
            "env:",
            "requires:",
        ] {
            assert!(plan.manifest.contains(key), "清单应有样本键 {key}");
        }
        assert_eq!(plan.files, 1);
        assert_eq!(plan.bytes, "---\nname: web-scout\n---\n".len() as u64);
        // 确认前一个字节都不落:根下只有 fixture 自建的 payloads/
        let mut names: Vec<String> = fs::read_dir(&root)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        names.sort();
        assert_eq!(names, vec!["payloads".to_string()]);
        assert!(!payload.join("component.yaml").exists());
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn 确认后落盘_字节确定_重打同一目录得到同一个校验和() {
        let root = fixture("commit");
        let payload = root.join("payloads/web-scout");
        fs::create_dir_all(&payload).unwrap();
        fs::write(payload.join("SKILL.md"), "---\nname: web-scout\n---\n").unwrap();
        let r = repo(&root);
        let plan = plan_pack(&r, &draft("payloads/web-scout", "web-scout", "1.0.0")).unwrap();
        let receipt = commit_pack(&r, &plan).unwrap();
        assert_eq!(receipt.written, vec!["payloads/web-scout/component.yaml"]);
        assert_eq!(
            fs::read(payload.join("component.yaml")).unwrap(),
            plan.write.writes[0].content,
            "fixture 目录进、组件清单出,字节确定"
        );
        // 重打:清单已在树上(被替换),校验和与清单都不漂
        let again = plan_pack(&r, &draft("payloads/web-scout", "web-scout", "1.0.0")).unwrap();
        assert_eq!(again.digest, plan.digest, "同输入同校验和");
        assert_eq!(again.manifest, plan.manifest, "同输入同清单");
        assert_eq!(again.files, 2, "清单自己也在树上(它现在是树的一个文件)");
        commit_pack(&r, &again).unwrap();
        assert_eq!(
            fs::read_to_string(payload.join("component.yaml")).unwrap(),
            plan.manifest,
            "重打逐字节相同(幂等)"
        );
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn 校验和含本清单_与落盘后的目录逐字节一致() {
        let root = fixture("digest-after");
        let payload = root.join("payloads/ribbon");
        fs::create_dir_all(&payload).unwrap();
        fs::write(payload.join("index.ts"), "export default {}\n").unwrap();
        let r = repo(&root);
        let plan = plan_pack(&r, &draft("payloads/ribbon", "ribbon", "0.4.1")).unwrap();
        commit_pack(&r, &plan).unwrap();
        // 落盘后重算(清单已在树上,不再当虚拟条目喂)
        let after = payload_files(&repo(&root), "payloads/ribbon").unwrap();
        assert_eq!(
            dir_digest(&after, None),
            plan.digest,
            "确认时显示的校验和 = 落盘后那棵树的校验和(上游 bake/up 后来算同一个数)"
        );
        fs::remove_dir_all(&root).ok();
    }

    // ---- 面板侧就拒:不喂给写侧跑一趟 ----

    #[test]
    fn 形状不合与目录不在_面板侧拒_一个字节都不落() {
        let root = fixture("reject");
        let payload = root.join("payloads/web-scout");
        fs::create_dir_all(&payload).unwrap();
        fs::write(payload.join("SKILL.md"), "x\n").unwrap();
        let r = repo(&root);
        for (src, name, ver) in [
            ("", "web-scout", "1.0.0"),
            ("payloads/web-scout", "web/scout", "1.0.0"),
            ("payloads/web-scout", ".hidden", "1.0.0"),
            ("payloads/web-scout", "web-scout", "1.0"),
            ("payloads/web-scout", "web-scout", "latest"),
            ("payloads/nope", "web-scout", "1.0.0"),
            ("payloads/web-scout/SKILL.md", "web-scout", "1.0.0"),
        ] {
            let err = plan_pack(&r, &draft(src, name, ver)).unwrap_err();
            assert!(!err.why().is_empty(), "拒件也给人话:{err:?}");
        }
        assert_eq!(
            plan_pack(&r, &draft("", "a", "1.0.0")).unwrap_err(),
            PackError::NoSource
        );
        assert!(!payload.join("component.yaml").exists(), "拒了更不落");
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn 空目录_没有可打包的文件_拒() {
        let root = fixture("empty");
        fs::create_dir_all(root.join("payloads/void")).unwrap();
        let r = repo(&root);
        let err = plan_pack(&r, &draft("payloads/void", "void", "1.0.0")).unwrap_err();
        assert_eq!(err, PackError::EmptyPayload("payloads/void".into()));
        // 只有排除项的目录同样算空
        fs::create_dir_all(root.join("payloads/only-git/.git")).unwrap();
        fs::write(root.join("payloads/only-git/.git/HEAD"), "x\n").unwrap();
        let err2 = plan_pack(&r, &draft("payloads/only-git", "only-git", "1.0.0")).unwrap_err();
        assert_eq!(err2, PackError::EmptyPayload("payloads/only-git".into()));
        fs::remove_dir_all(&root).ok();
    }
}
