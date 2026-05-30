use anyhow::Context;
use base64::Engine;
use rsa::pkcs1::DecodeRsaPrivateKey;
use rsa::pkcs1v15::SigningKey;
use rsa::signature::{SignatureEncoding, Signer};
use sha1::{Digest, Sha1};
use std::path::{Path, PathBuf};

// ── 嵌入资源 ──────────────────────────────────────────────────

pub(crate) const PRIV_KEY_PEM: &str = include_str!("../keys/mod/priv.key");
const MOD_CERT_PEM: &[u8] =
    include_bytes!("../keys/mod/Unity.Licensing.EntitlementResolver.Unity.cer");
const ORIG_CERT_PEM: &[u8] =
    include_bytes!("../keys/orig/Unity.Licensing.EntitlementResolver.Unity.cer");
const LICENSE_XML: &str = include_str!("../license.xml");

pub(crate) const HOSTS_PATH: &str = r"C:\Windows\System32\drivers\etc\hosts";
pub(crate) const LICENSE_DIR: &str = r"C:\ProgramData\Unity";
pub(crate) const SHARED_LICENSING_CLIENT_DIR: &str =
    r"C:\Program Files\Unity Hub\UnityLicensingClient_V1";
const LICENSING_DLL_SUBPATH: &str =
    r"Editor\Data\Resources\Licensing\Client\Unity.Licensing.EntitlementResolver.dll";
const EDITOR_EXE_SUBPATH: &str = r"Editor\Unity.exe";
const LICENSE_FILE: &str = "Unity_lic.ulf";

pub(crate) const HOSTS_BEGIN_MARKER: &str = "# unifuck";
pub(crate) const HOSTS_END_MARKER: &str = "# unifuck-end";

pub(crate) const HOSTS_ENTRIES: &str = "\n\
# unifuck\n\
127.0.0.1 license.unity3d.com\n\
127.0.0.1 core.cloud.unity3d.com\n\
127.0.0.1 api.unity.com\n\
127.0.0.1 unity.com\n\
# unifuck-end\n";

// ── 数据结构 ──────────────────────────────────────────────────

#[derive(Clone)]
pub(crate) struct UnityInstall {
    pub version: String,
    pub dll_path: PathBuf,
    pub exe_path: PathBuf,
}

// ── 辅助函数 ──────────────────────────────────────────────────

fn base64_lines(der: &[u8]) -> String {
    let b64 = base64::engine::general_purpose::STANDARD.encode(der);
    let mut out = String::with_capacity(b64.len() + b64.len() / 76 + 1);
    for (i, chunk) in b64.as_bytes().chunks(76).enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(std::str::from_utf8(chunk).unwrap());
    }
    out.push('\n');
    out
}

pub(crate) fn is_patched(dll_path: &Path) -> bool {
    if dll_path.with_extension("dll.bak").exists() {
        return true;
    }
    if let Ok(data) = std::fs::read(dll_path) {
        if let Ok(pem0) = pem::parse(MOD_CERT_PEM) {
            let b64 = base64_lines(pem0.contents());
            if let Some(line) = b64.lines().next() {
                return data.windows(line.len()).any(|w| w == line.as_bytes());
            }
        }
    }
    false
}

// ── 扫描 ──────────────────────────────────────────────────────

fn try_find_install(root: &Path, version: &str) -> Option<UnityInstall> {
    let dll_path = root.join(LICENSING_DLL_SUBPATH);
    if dll_path.exists() {
        Some(UnityInstall {
            version: version.to_string(),
            dll_path,
            exe_path: root.join(EDITOR_EXE_SUBPATH),
        })
    } else {
        None
    }
}

pub(crate) fn scan_installations() -> Vec<UnityInstall> {
    let mut installs = Vec::new();

    if let Ok(dir) = std::fs::read_dir(r"C:\Program Files") {
        for entry in dir.flatten() {
            let path = entry.path();
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            if path.is_dir() && name.starts_with("Unity ") {
                let version = name.strip_prefix("Unity ").unwrap_or(&name);
                if let Some(install) = try_find_install(&path, version) {
                    installs.push(install);
                }
            }
        }
    }

    if let Ok(hub_dir) = std::fs::read_dir(r"C:\Program Files\Unity Hub\Editor") {
        for entry in hub_dir.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let version = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "UNKNOWN".to_string());
                if let Some(install) = try_find_install(&path, &version) {
                    installs.push(install);
                }
            }
        }
    }

    installs.sort_by(|a, b| b.version.cmp(&a.version));
    installs
}

// ── 修补 ──────────────────────────────────────────────────────

/// 在字节数组中搜索 `old` 并原地替换为 `new`。
///
/// `old.len()` 必须等于 `new.len()`，否则替换会破坏后续偏移量。
/// 对于 73 MB 的二进制文件，31 次 76 字节的扫描总计约 2.2 GB 的顺序读，
/// 在现代 CPU 上几十毫秒即可完毕。
fn replace_bytes_in_place(data: &mut [u8], old: &[u8], new: &[u8]) {
    assert_eq!(
        old.len(),
        new.len(),
        "replace_bytes_in_place requires equal-length patterns"
    );
    if old.is_empty() || old == new {
        return;
    }
    let mut start = 0;
    while let Some(rel) = data[start..].windows(old.len()).position(|w| w == old) {
        let pos = start + rel;
        data[pos..pos + old.len()].copy_from_slice(new);
        start = pos + old.len();
    }
}

/// 对二进制数据 (DLL / exe) 执行证书 base64 替换。
///
/// 同时尝试两种匹配形态:
/// - 多行: 按 76 字符一行的 base64 PEM 体 (与 `base64_lines` 输出一致)
/// - 单行: 全部 base64 字符拼接 (无换行, 常见于单体 exe 中的嵌入资源)
///
/// 返回 `true` 表示至少替换了一处, `false` 表示数据中未发现原始证书。
fn apply_cert_replacement(data: &mut [u8]) -> anyhow::Result<bool> {
    let pem_orig = pem::parse(ORIG_CERT_PEM).context("parse original cert failed")?;
    let pem_mod = pem::parse(MOD_CERT_PEM).context("parse modified cert failed")?;
    let der_orig = pem_orig.contents();
    let der_mod = pem_mod.contents();

    if der_orig.len() != der_mod.len() {
        anyhow::bail!(
            "cert DER length mismatch: original {} bytes, modified {} bytes",
            der_orig.len(),
            der_mod.len()
        );
    }

    let b64_orig = base64_lines(der_orig);
    let b64_mod = base64_lines(der_mod);

    // 先验证: 如果任何一行已经被替换过, 说明已经 patched
    for (_lo, lm) in b64_orig.lines().zip(b64_mod.lines()) {
        let mm = lm.as_bytes();
        if !mm.is_empty() && data.windows(mm.len()).any(|w| w == mm) {
            anyhow::bail!("already patched");
        }
    }

    let mut any_replaced = false;

    // 多行形态: 逐行替换 (DLL 及以 76 字符分行存储的 exe)
    for (lo, lm) in b64_orig.lines().zip(b64_mod.lines()) {
        let om = lo.as_bytes();
        let mm = lm.as_bytes();
        if om.is_empty() || om.len() != mm.len() {
            continue;
        }
        if data.windows(om.len()).any(|w| w == om) {
            replace_bytes_in_place(data, om, mm);
            any_replaced = true;
        }
    }

    // 单行形态: 整段 base64 (常见于 Unity 2021 单体 exe 中的嵌入证书)
    if !any_replaced {
        let single_orig: String = b64_orig.lines().collect();
        let single_mod: String = b64_mod.lines().collect();
        let om = single_orig.as_bytes();
        let mm = single_mod.as_bytes();
        if om.len() == mm.len() && !om.is_empty() && data.windows(om.len()).any(|w| w == om) {
            replace_bytes_in_place(data, om, mm);
            any_replaced = true;
        }
    }

    Ok(any_replaced)
}

/// Generic binary file cert replacement with automatic backup.
///
/// `skip_if_absent`: if true, return Ok silently when file doesn't exist (exe, shared client)
/// `min_size`: skip files smaller than this (exe launcher detection, 0 for DLL)
/// `skip_if_backup_exists`: if true, return Ok when .bak already exists (shared client)
fn patch_binary_file(
    path: &Path,
    skip_if_absent: bool,
    min_size: u64,
    skip_if_backup_exists: bool,
) -> anyhow::Result<bool> {
    if skip_if_absent && !path.exists() {
        return Ok(false);
    }

    if min_size > 0 {
        let meta = std::fs::metadata(path)
            .with_context(|| format!("metadata failed: {}", path.display()))?;
        if meta.len() < min_size {
            return Ok(false);
        }
    }

    let bak_path = path.with_extension(
        path.extension()
            .unwrap_or_default()
            .to_str()
            .unwrap_or("")
            .to_string()
            + ".bak",
    );
    if skip_if_backup_exists && bak_path.exists() {
        return Ok(false);
    }

    let mut data =
        std::fs::read(path).with_context(|| format!("read failed: {}", path.display()))?;
    std::fs::write(&bak_path, &data)
        .with_context(|| format!("backup write failed: {}", bak_path.display()))?;

    let replaced = apply_cert_replacement(&mut data)
        .with_context(|| format!("cert replacement failed: {}", path.display()));
    match replaced {
        Ok(true) => {}
        Ok(false) => {
            let _ = std::fs::remove_file(&bak_path);
            return Ok(false);
        }
        Err(e) => return Err(e),
    }

    std::fs::write(path, &data).with_context(|| format!("write failed: {}", path.display()))?;
    Ok(true)
}

pub(crate) fn patch_dll(dll_path: &Path) -> anyhow::Result<()> {
    let patched = patch_binary_file(dll_path, false, 0, false)?;
    if !patched {
        anyhow::bail!("cert not found in DLL: {}", dll_path.display());
    }
    Ok(())
}

// ── 签名 ──────────────────────────────────────────────────────

pub(crate) fn sign_license() -> anyhow::Result<String> {
    let key =
        rsa::RsaPrivateKey::from_pkcs1_pem(PRIV_KEY_PEM).context("parse private key failed")?;
    let signing_key = SigningKey::<Sha1>::new(key);

    let mut xml = LICENSE_XML.to_string();

    let c14n_license = crate::c14n::canonicalize_subtree(&xml, "Terms")?;
    let digest = Sha1::digest(&c14n_license);
    let digest_b64 = base64::engine::general_purpose::STANDARD.encode(digest);
    xml = xml.replace(
        "<DigestValue/>",
        &format!("<DigestValue>{}</DigestValue>", digest_b64),
    );

    let c14n_si = crate::c14n::canonicalize_signed_info(&xml)?;
    let sig = signing_key.sign(&c14n_si);
    let sig_b64 = base64::engine::general_purpose::STANDARD.encode(sig.to_vec());
    xml = xml.replace(
        "<SignatureValue/>",
        &format!("<SignatureValue>{}</SignatureValue>", sig_b64),
    );

    Ok(xml)
}

// ── 部署 ──────────────────────────────────────────────────────

pub(crate) fn deploy_license(xml: &str) -> anyhow::Result<()> {
    let dir = Path::new(LICENSE_DIR);
    std::fs::create_dir_all(dir).with_context(|| format!("mkdir failed: {}", dir.display()))?;

    let path = dir.join(LICENSE_FILE);
    std::fs::write(&path, xml).with_context(|| format!("write failed: {}", path.display()))?;

    Ok(())
}

// ── hosts ─────────────────────────────────────────────────────

pub(crate) fn write_hosts_if_needed() -> anyhow::Result<()> {
    let content = std::fs::read_to_string(HOSTS_PATH)
        .with_context(|| format!("read failed: {}", HOSTS_PATH))?;

    if content.contains(HOSTS_BEGIN_MARKER) {
        return Ok(());
    }

    let bak = format!("{}.bak", HOSTS_PATH);
    std::fs::write(&bak, &content)
        .with_context(|| format!("hosts backup write failed: {}", bak))?;

    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(HOSTS_PATH)
        .context("open hosts failed")?;
    std::io::Write::write_all(&mut file, HOSTS_ENTRIES.as_bytes()).context("write hosts failed")?;

    Ok(())
}

// ── 还原 ──────────────────────────────────────────────────────

fn restore_from_backup(path: &Path) -> anyhow::Result<()> {
    let bak_path = path.with_extension(
        path.extension()
            .unwrap_or_default()
            .to_str()
            .unwrap_or("")
            .to_string()
            + ".bak",
    );
    if !bak_path.exists() {
        return Ok(());
    }
    let data = std::fs::read(&bak_path)
        .with_context(|| format!("read backup failed: {}", bak_path.display()))?;
    std::fs::write(path, &data)
        .with_context(|| format!("restore write failed: {}", path.display()))?;
    std::fs::remove_file(&bak_path)
        .with_context(|| format!("remove backup failed: {}", bak_path.display()))?;
    Ok(())
}

pub(crate) fn unpatch_dll(dll_path: &Path) -> anyhow::Result<()> {
    let bak_path = dll_path.with_extension("dll.bak");
    if !bak_path.exists() {
        anyhow::bail!("no backup: {}", bak_path.display());
    }
    restore_from_backup(dll_path)
}

/// 补丁 Unity Editor 主程序 (Unity.exe) 中内嵌的许可证书。
///
/// 仅对 Unity 2021 等单体二进制版本有效: 这类 exe 约 73 MB, 内含 LicensingLegacy
/// 模块与同一份 X.509 证书; Unity 6000+ 的 exe 只是约 282 KB 的启动器, 不含证书。
///
/// 跳过条件 (返回 Ok 但不生成备份):
/// - exe 不存在
/// - exe 体积小于 1 MB (判定为启动器)
/// - exe 中未发现原始证书 base64 (既不以多行也不以单行形态出现)
pub(crate) fn patch_exe(exe_path: &Path) -> anyhow::Result<()> {
    patch_binary_file(exe_path, true, 1024 * 1024, false)?;
    Ok(())
}

pub(crate) fn unpatch_exe(exe_path: &Path) -> anyhow::Result<()> {
    restore_from_backup(exe_path)
}

pub(crate) fn is_exe_patched(exe_path: &Path) -> bool {
    exe_path.with_extension("exe.bak").exists()
}

// ── Shared Licensing Client (Hub-installed) ──────────────────

/// 补丁 Hub 安装的全局共享 Licensing Client DLL。
///
/// Unity 2021 Editor 通过 IPC 连接到此共享客户端 (位于
/// `C:\Program Files\Unity Hub\UnityLicensingClient_V1\Unity.Licensing.EntitlementResolver.dll`)
/// 进行 ULF 许可证签名验证; 内含与 Editor DLL 相同的原始证书。
/// 若仅补丁 Editor 本地副本而未补丁此共享客户端, 2021 Editor 会报
/// "digital signature is invalid"。
///
/// 跳过条件 (返回 Ok):
/// - Hub 未安装 (目录或 DLL 不存在)
/// - 已存在 `.dll.bak` 备份 (说明已补丁过)
/// - DLL 中未发现原始证书
pub(crate) fn patch_shared_licensing_client() -> anyhow::Result<()> {
    let dll_path =
        Path::new(SHARED_LICENSING_CLIENT_DIR).join("Unity.Licensing.EntitlementResolver.dll");
    patch_binary_file(&dll_path, true, 0, true)?;
    Ok(())
}

/// 还原 Hub 安装的全局共享 Licensing Client DLL 至原始状态。
///
/// 若从未补丁过 (.dll.bak 不存在), 静默成功。
pub(crate) fn unpatch_shared_licensing_client() -> anyhow::Result<()> {
    let dll_path =
        Path::new(SHARED_LICENSING_CLIENT_DIR).join("Unity.Licensing.EntitlementResolver.dll");
    restore_from_backup(&dll_path)
}

/// 检查 Hub 安装的全局共享 Licensing Client DLL 是否处于已补丁状态。
pub(crate) fn is_shared_licensing_client_patched() -> bool {
    let dll_path =
        Path::new(SHARED_LICENSING_CLIENT_DIR).join("Unity.Licensing.EntitlementResolver.dll");
    dll_path.with_extension("dll.bak").exists()
}

pub(crate) fn remove_license() -> anyhow::Result<()> {
    let path = Path::new(LICENSE_DIR).join(LICENSE_FILE);
    if !path.exists() {
        return Ok(());
    }
    std::fs::remove_file(&path)
        .with_context(|| format!("remove license failed: {}", path.display()))?;
    Ok(())
}

pub(crate) fn clean_hosts() -> anyhow::Result<()> {
    let content = std::fs::read_to_string(HOSTS_PATH)
        .with_context(|| format!("read failed: {}", HOSTS_PATH))?;

    let Some(begin_idx) = content.find(HOSTS_BEGIN_MARKER) else {
        return Ok(());
    };

    // 把 begin 之前的 \r/\n 也吃掉 (保持原文件风格干净)
    let mut start = begin_idx;
    while start > 0 {
        let prev = content.as_bytes()[start - 1];
        if prev == b'\n' || prev == b'\r' {
            start -= 1;
        } else {
            break;
        }
    }

    let after_begin = &content[begin_idx..];
    let Some(end_rel) = after_begin.find(HOSTS_END_MARKER) else {
        // 找不到结束标记 → 不动文件, 静默成功避免误删
        return Ok(());
    };
    let mut end = begin_idx + end_rel + HOSTS_END_MARKER.len();
    // 吃掉结束标记后紧跟的换行
    let bytes = content.as_bytes();
    while end < bytes.len() && (bytes[end] == b'\r' || bytes[end] == b'\n') {
        end += 1;
    }

    let mut new_content = String::with_capacity(content.len());
    new_content.push_str(&content[..start]);
    new_content.push_str(&content[end..]);

    std::fs::write(HOSTS_PATH, new_content)
        .with_context(|| format!("write hosts failed: {}", HOSTS_PATH))?;
    Ok(())
}
