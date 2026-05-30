use anyhow::Context;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::Path;

fn main() -> anyhow::Result<()> {
    let original = r"C:\Program Files\Unity Hub\resources\app.asar.bak";
    let patched = r"C:\Program Files\Unity Hub\resources\app.asar";

    println!("=== Original asar ===");
    let orig = read_asar(Path::new(original))?;
    println!("Header size: {}", orig.header_size);
    println!("Data size: {}", orig.data.len());
    println!("Header JSON keys: {:?}", orig.header_json.as_object().map(|o| o.keys().collect::<Vec<_>>()));

    println!("\n=== Patched asar ===");
    let patched = read_asar(Path::new(patched))?;
    println!("Header size: {}", patched.header_size);
    println!("Data size: {}", patched.data.len());
    println!("Header JSON keys: {:?}", patched.header_json.as_object().map(|o| o.keys().collect::<Vec<_>>()));

    // Compare a sample file entry
    let test_path = "build/main/services/licenseService/licenseService.js";
    println!("\n=== Comparing {} ===", test_path);

    if let Some(orig_entry) = find_entry(&orig.header_json, test_path) {
        println!("Original entry: {:?}", orig_entry);
        if let Some(data) = extract_file(&orig, &orig_entry) {
            println!("Original file size: {}", data.len());
            println!("Original first 100 bytes: {:?}", String::from_utf8_lossy(&data[..100.min(data.len())]));
        }
    }

    if let Some(patch_entry) = find_entry(&patched.header_json, test_path) {
        println!("\nPatched entry: {:?}", patch_entry);
        if let Some(data) = extract_file(&patched, &patch_entry) {
            println!("Patched file size: {}", data.len());
            println!("Patched first 100 bytes: {:?}", String::from_utf8_lossy(&data[..100.min(data.len())]));
        }
    }

    Ok(())
}

struct AsarArchive {
    header_json: Value,
    header_size: usize,
    data: Vec<u8>,
}

fn read_asar(path: &Path) -> anyhow::Result<AsarArchive> {
    let bytes = std::fs::read(path).with_context(|| format!("read asar: {}", path.display()))?;

    anyhow::ensure!(bytes.len() >= 16, "asar file too small");

    let outer_size = u32::from_le_bytes(bytes[0..4].try_into()?) as usize;
    let header_payload_size = u32::from_le_bytes(bytes[4..8].try_into()?) as usize;
    let inner_type = u32::from_le_bytes(bytes[8..12].try_into()?) as usize;
    let json_len = u32::from_le_bytes(bytes[12..16].try_into()?) as usize;

    println!("  outer_size: {}", outer_size);
    println!("  header_payload_size: {}", header_payload_size);
    println!("  inner_type: {}", inner_type);
    println!("  json_len: {}", json_len);

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

    let header_json = if raw_header.get("files").is_some()
        && raw_header.get("files").map(|f| f.is_object()) == Some(true)
        && raw_header.get("size").is_none()
        && raw_header.get("offset").is_none()
    {
        raw_header
    } else {
        json!({ "files": raw_header })
    };

    Ok(AsarArchive {
        header_json,
        header_size: header_total,
        data,
    })
}

fn find_entry(header: &Value, path: &str) -> Option<Value> {
    let segments: Vec<&str> = path.split('/').collect();
    let mut current = header;
    for seg in &segments {
        current = current.get("files")?.get(seg)?;
    }
    Some(current.clone())
}

fn extract_file(asar: &AsarArchive, entry: &Value) -> Option<Vec<u8>> {
    let size = entry.get("size")?.as_u64()? as usize;
    let offset = entry.get("offset")?.as_str()?.parse::<usize>().ok()?;
    if offset.saturating_add(size) > asar.data.len() {
        println!("  offset {} + size {} > data.len() {}", offset, size, asar.data.len());
        return None;
    }
    Some(asar.data[offset..offset + size].to_vec())
}
