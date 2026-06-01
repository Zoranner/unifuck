use anyhow::Context;
use base64::Engine;
use rsa::pkcs1::DecodeRsaPrivateKey;
use rsa::pkcs1v15::SigningKey;
use rsa::signature::{SignatureEncoding, Signer};
use sha1::{Digest, Sha1};
use std::path::Path;

const PRIV_KEY_PEM: &str = include_str!("../../keys/mod/priv.key");
const LICENSE_XML: &str = include_str!("../../license.xml");
const LICENSE_DIR: &str = r"C:\ProgramData\Unity";
const LICENSE_FILE: &str = "Unity_lic.ulf";

pub fn sign_license() -> anyhow::Result<String> {
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

pub fn deploy_license(xml: &str) -> anyhow::Result<()> {
    let dir = Path::new(LICENSE_DIR);
    std::fs::create_dir_all(dir).with_context(|| format!("mkdir failed: {}", dir.display()))?;

    let path = dir.join(LICENSE_FILE);
    std::fs::write(&path, xml).with_context(|| format!("write failed: {}", path.display()))?;

    Ok(())
}

pub fn remove_license() -> anyhow::Result<()> {
    let path = Path::new(LICENSE_DIR).join(LICENSE_FILE);
    if !path.exists() {
        return Ok(());
    }
    std::fs::remove_file(&path)
        .with_context(|| format!("remove license failed: {}", path.display()))?;
    Ok(())
}
