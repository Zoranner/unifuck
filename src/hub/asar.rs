use anyhow::Context;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::Path;

pub(crate) struct AsarArchive {
    pub header_json: Value,
    pub data: Vec<u8>,
}

pub(crate) fn read_asar(path: &Path) -> anyhow::Result<AsarArchive> {
    let bytes = std::fs::read(path).with_context(|| format!("read asar: {}", path.display()))?;

    anyhow::ensure!(bytes.len() >= 16, "asar file too small");

    let _outer_size = u32::from_le_bytes(bytes[0..4].try_into()?) as usize;
    let header_payload_size = u32::from_le_bytes(bytes[4..8].try_into()?) as usize;
    let _inner_type = u32::from_le_bytes(bytes[8..12].try_into()?);
    let json_len = u32::from_le_bytes(bytes[12..16].try_into()?) as usize;

    anyhow::ensure!(
        16 + json_len <= bytes.len(),
        "asar header exceeds file size"
    );

    let json_str =
        std::str::from_utf8(&bytes[16..16 + json_len]).context("asar header is not valid UTF-8")?;
    let raw_header: Value = serde_json::from_str(json_str).context("parse asar JSON header")?;

    let header_total = 8 + header_payload_size;
    let data = if header_total < bytes.len() {
        bytes[header_total..].to_vec()
    } else {
        Vec::new()
    };

    // Normalise: some asar versions wrap the tree under a "files" key at root level.
    let header_json = if raw_header.get("files").is_some()
        && raw_header.get("files").map(|f| f.is_object()) == Some(true)
        && raw_header.get("size").is_none()
        && raw_header.get("offset").is_none()
    {
        raw_header
    } else {
        json!({ "files": raw_header })
    };

    Ok(AsarArchive { header_json, data })
}

pub(crate) fn write_asar(
    path: &Path,
    modified: &[(String, Vec<u8>)],
    original: &AsarArchive,
) -> anyhow::Result<()> {
    let entries = walk_asar(&original.header_json);
    let mod_set: std::collections::HashMap<&str, &[u8]> = modified
        .iter()
        .map(|(p, d)| (p.as_str(), d.as_slice()))
        .collect();

    let mut new_header = original.header_json.clone();
    let mut new_data = Vec::new();
    let mut offset: u64 = 0;

    for (entry_path, _) in &entries {
        let content = match mod_set.get(entry_path.as_str()) {
            Some(data) => data.to_vec(),
            None => match find_entry(&original.header_json, entry_path) {
                Some(e) => extract_file(original, &e).unwrap_or_default(),
                None => Vec::new(),
            },
        };

        let size = content.len() as u64;
        new_data.extend_from_slice(&content);

        let segments: Vec<&str> = entry_path.split('/').collect();
        let mut pointer = String::new();
        for seg in &segments {
            pointer.push_str("/files/");
            pointer.push_str(seg);
        }

        if let Some(obj) = new_header
            .pointer_mut(&pointer)
            .and_then(|v| v.as_object_mut())
        {
            obj.insert("offset".into(), json!(offset.to_string()));
            obj.insert("size".into(), json!(size));
            // Only update integrity for modified files to avoid header bloat
            if mod_set.contains_key(entry_path.as_str()) {
                let integrity = compute_integrity(&content);
                obj.insert("integrity".into(), integrity);
            }
        }

        offset += size;
    }

    let json_str = serde_json::to_string(&new_header)?;
    let json_bytes = json_str.as_bytes();
    let json_len = json_bytes.len();
    let inner_payload_size = align4(4 + json_len) as u32;
    let outer_payload_size = 4 + inner_payload_size;

    let mut header_buf = Vec::with_capacity(inner_payload_size as usize);
    header_buf.extend_from_slice(&inner_payload_size.to_le_bytes());
    header_buf.extend_from_slice(&(json_len as u32).to_le_bytes());
    header_buf.extend_from_slice(json_bytes);
    header_buf.resize(outer_payload_size as usize, 0);

    let mut output = Vec::with_capacity(8 + header_buf.len() + new_data.len());
    output.extend_from_slice(&4u32.to_le_bytes());
    output.extend_from_slice(&outer_payload_size.to_le_bytes());
    output.extend_from_slice(&header_buf);
    output.extend_from_slice(&new_data);

    std::fs::write(path, &output).with_context(|| format!("write asar: {}", path.display()))?;
    Ok(())
}

#[inline]
fn align4(n: usize) -> usize {
    (n + 3) & !3
}

fn walk_asar(header: &Value) -> Vec<(String, Value)> {
    let mut result = Vec::new();
    walk_recursive(header, "", &mut result);
    result
}

fn walk_recursive(node: &Value, path: &str, result: &mut Vec<(String, Value)>) {
    let Some(files) = node.get("files").and_then(|f| f.as_object()) else {
        if node.get("size").is_some() && node.get("offset").is_some() && !path.is_empty() {
            result.push((path.to_string(), node.clone()));
        }
        return;
    };
    for (name, child) in files {
        let child_path = if path.is_empty() {
            name.clone()
        } else {
            format!("{}/{}", path, name)
        };
        walk_recursive(child, &child_path, result);
    }
}

pub(crate) fn find_entry(header: &Value, path: &str) -> Option<Value> {
    let segments: Vec<&str> = path.split('/').collect();
    let mut current = header;
    for seg in &segments {
        current = current.get("files")?.get(seg)?;
    }
    Some(current.clone())
}

pub(crate) fn extract_file(asar: &AsarArchive, entry: &Value) -> Option<Vec<u8>> {
    let size = entry.get("size")?.as_u64()? as usize;
    let offset = entry.get("offset")?.as_str()?.parse::<usize>().ok()?;
    if offset.saturating_add(size) > asar.data.len() {
        return None;
    }
    Some(asar.data[offset..offset + size].to_vec())
}

fn sha256_hex(data: &[u8]) -> String {
    let hash = Sha256::digest(data);
    hex_encode(&hash)
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

fn compute_integrity(data: &[u8]) -> Value {
    const BLOCK_SIZE: usize = 4 * 1024 * 1024;
    let hash = sha256_hex(data);
    let blocks: Vec<Value> = if data.is_empty() {
        vec![json!(sha256_hex(&[]))]
    } else {
        data.chunks(BLOCK_SIZE)
            .map(|chunk| json!(sha256_hex(chunk)))
            .collect()
    };
    json!({
        "algorithm": "SHA256",
        "hash": hash,
        "blockSize": BLOCK_SIZE,
        "blocks": blocks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_archive() -> AsarArchive {
        let main_js = b"old main";
        let package_json = br#"{"version":"1.0.0"}"#;
        let mut data = Vec::new();
        data.extend_from_slice(main_js);
        data.extend_from_slice(package_json);

        AsarArchive {
            header_json: json!({
                "files": {
                    "build": {
                        "files": {
                            "main": {
                                "files": {
                                    "app-test.js": {
                                        "size": main_js.len(),
                                        "offset": "0",
                                        "integrity": {}
                                    }
                                }
                            }
                        }
                    },
                    "package.json": {
                        "size": package_json.len(),
                        "offset": main_js.len().to_string(),
                        "integrity": {}
                    }
                }
            }),
            data,
        }
    }

    #[test]
    fn write_asar_aligns_header_and_roundtrips_modified_files() {
        let archive = fixture_archive();
        let path = std::env::temp_dir().join(format!(
            "unifuck-asar-roundtrip-{}.asar",
            std::process::id()
        ));

        write_asar(
            &path,
            &[(
                "build/main/app-test.js".to_string(),
                b"patched main".to_vec(),
            )],
            &archive,
        )
        .unwrap();

        let bytes = std::fs::read(&path).unwrap();
        let header_payload_size = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
        let json_len = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
        assert_eq!(header_payload_size, align4(4 + json_len));

        let roundtripped = read_asar(&path).unwrap();
        let entry = find_entry(&roundtripped.header_json, "build/main/app-test.js").unwrap();
        assert_eq!(
            extract_file(&roundtripped, &entry).unwrap(),
            b"patched main"
        );

        let entry = find_entry(&roundtripped.header_json, "package.json").unwrap();
        assert_eq!(
            extract_file(&roundtripped, &entry).unwrap(),
            br#"{"version":"1.0.0"}"#
        );

        std::fs::remove_file(path).ok();
    }
}
