use anyhow::Context;
use base64::Engine;
use rsa::pkcs1::DecodeRsaPrivateKey;
use rsa::pkcs1v15::SigningKey;
use rsa::signature::{SignatureEncoding, Signer};
use sha1::{Digest, Sha1};
use std::path::Path;

use crate::vendor::Vendor;

const LICENSE_XML: &str = include_str!("../../license.xml");

pub fn sign_license(vendor: Vendor) -> anyhow::Result<String> {
    let key = rsa::RsaPrivateKey::from_pkcs1_pem(vendor.priv_key_pem())
        .with_context(|| format!("parse {} private key failed", vendor.display_name()))?;
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

pub fn deploy_license(vendor: Vendor, xml: &str) -> anyhow::Result<()> {
    let dir = Path::new(vendor.license_dir());
    std::fs::create_dir_all(dir).with_context(|| format!("mkdir failed: {}", dir.display()))?;

    let path = dir.join(vendor.license_filename());
    std::fs::write(&path, xml).with_context(|| format!("write failed: {}", path.display()))?;

    Ok(())
}

pub fn remove_license(vendor: Vendor) -> anyhow::Result<()> {
    let path = Path::new(vendor.license_dir()).join(vendor.license_filename());
    if !path.exists() {
        return Ok(());
    }
    std::fs::remove_file(&path)
        .with_context(|| format!("remove license failed: {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Smoke test: each vendor's embedded priv.key must be PKCS#1-parseable
    /// and produce a signed license XML containing populated DigestValue and
    /// SignatureValue elements. Catches OpenSSL PKCS#8 default leaking into
    /// keys/<vendor>/mod/priv.key.
    #[test]
    fn sign_license_succeeds_for_every_vendor() {
        for vendor in Vendor::all().iter().copied() {
            let xml = sign_license(vendor)
                .unwrap_or_else(|e| panic!("sign_license({:?}) failed: {}", vendor, e));
            assert!(
                xml.contains("<DigestValue>")
                    && !xml.contains("<DigestValue/>")
                    && xml.contains("<SignatureValue>")
                    && !xml.contains("<SignatureValue/>"),
                "{:?} signed XML missing digest or signature population",
                vendor
            );
        }
    }
}
