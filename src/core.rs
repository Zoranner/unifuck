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

pub(crate) fn scan_installations() -> Vec<UnityInstall> {
    let mut installs = Vec::new();

    if let Ok(dir) = std::fs::read_dir(r"C:\Program Files") {
        for entry in dir.flatten() {
            let path = entry.path();
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            if !path.is_dir() || !name.starts_with("Unity ") {
                continue;
            }
            let dll = path.join(
                r"Editor\Data\Resources\Licensing\Client\Unity.Licensing.EntitlementResolver.dll",
            );
            if dll.exists() {
                let version = name.strip_prefix("Unity ").unwrap_or(&name).to_string();
                installs.push(UnityInstall {
                    version,
                    dll_path: dll,
                });
            }
        }
    }

    if let Ok(hub_dir) = std::fs::read_dir(r"C:\Program Files\Unity Hub\Editor") {
        for entry in hub_dir.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let dll = path.join(
                r"Editor\Data\Resources\Licensing\Client\Unity.Licensing.EntitlementResolver.dll",
            );
            if dll.exists() {
                let version = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "UNKNOWN".to_string());
                installs.push(UnityInstall {
                    version,
                    dll_path: dll,
                });
            }
        }
    }

    installs.sort_by(|a, b| b.version.cmp(&a.version));
    installs
}

// ── 修补 ──────────────────────────────────────────────────────

pub(crate) fn patch_dll(dll_path: &Path) -> anyhow::Result<()> {
    let mut resolver =
        std::fs::read(dll_path).with_context(|| format!("read failed: {}", dll_path.display()))?;

    let bak_path = dll_path.with_extension("dll.bak");
    std::fs::write(&bak_path, &resolver)
        .with_context(|| format!("backup write failed: {}", bak_path.display()))?;

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

    for (line_orig, line_mod) in b64_orig.lines().zip(b64_mod.lines()) {
        let lo = line_orig.as_bytes();
        let lm = line_mod.as_bytes();

        if resolver.windows(lm.len()).any(|w| w == lm) {
            anyhow::bail!("DLL already patched");
        }

        if !resolver.windows(lo.len()).any(|w| w == lo) {
            anyhow::bail!("cert line not found in DLL: {}", line_orig);
        }

        if let Some(pos) = resolver.windows(lo.len()).position(|w| w == lo) {
            resolver[pos..pos + lo.len()].copy_from_slice(lm);
        }
    }

    std::fs::write(dll_path, &resolver)
        .with_context(|| format!("write failed: {}", dll_path.display()))?;

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

pub(crate) fn unpatch_dll(dll_path: &Path) -> anyhow::Result<()> {
    let bak_path = dll_path.with_extension("dll.bak");
    if !bak_path.exists() {
        anyhow::bail!("no backup: {}", bak_path.display());
    }
    let data = std::fs::read(&bak_path)
        .with_context(|| format!("read backup failed: {}", bak_path.display()))?;
    std::fs::write(dll_path, &data)
        .with_context(|| format!("restore write failed: {}", dll_path.display()))?;
    std::fs::remove_file(&bak_path)
        .with_context(|| format!("remove backup failed: {}", bak_path.display()))?;
    Ok(())
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
