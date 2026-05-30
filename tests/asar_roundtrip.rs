use std::path::Path;

#[inline]
fn align4(n: usize) -> usize { (n + 3) & !3 }

#[test]
fn asar_roundtrip_read_write() {
    let bak_path = r"C:\Program Files\Unity Hub\resources\app.asar.bak";
    if !Path::new(bak_path).exists() {
        eprintln!("SKIP: no asar.bak found");
        return;
    }
    let bytes = std::fs::read(bak_path).unwrap();
    
    // Read
    let header_buf_size = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
    let payload_size = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
    let json_len = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
    
    // Verify format
    assert_eq!(u32::from_le_bytes(bytes[0..4].try_into().unwrap()), 4, "size pickle payload");
    assert_eq!(payload_size, align4(4 + json_len), "payload_size = align4(4 + json_len)");
    
    let json_str = std::str::from_utf8(&bytes[16..16+json_len]).unwrap();
    let header: serde_json::Value = serde_json::from_str(json_str).unwrap();
    let data = bytes[8 + header_buf_size..].to_vec();
    
    // Write back (unmodified)
    let new_json = serde_json::to_string(&header).unwrap();
    let new_json_bytes = new_json.as_bytes();
    let new_json_len = new_json_bytes.len();
    let new_payload_size = align4(4 + new_json_len) as u32;
    
    let mut header_buf = Vec::new();
    header_buf.extend_from_slice(&new_payload_size.to_le_bytes());
    header_buf.extend_from_slice(&(new_json_len as u32).to_le_bytes());
    header_buf.extend_from_slice(new_json_bytes);
    header_buf.resize(4 + new_payload_size as usize, 0);
    
    let new_header_buf_size = header_buf.len() as u32;
    let mut output = Vec::new();
    output.extend_from_slice(&4u32.to_le_bytes());
    output.extend_from_slice(&new_header_buf_size.to_le_bytes());
    output.extend_from_slice(&header_buf);
    output.extend_from_slice(&data);
    
    let out_path = r"C:\Users\Zoranner\AppData\Local\Temp\test_roundtrip.asar";
    std::fs::write(out_path, &output).unwrap();
    
    // Verify npx asar can parse it
    let result = std::process::Command::new("npx")
        .args(["asar", "list", out_path])
        .output()
        .unwrap();
    
    assert!(result.status.success(), "npx asar list failed: {}", String::from_utf8_lossy(&result.stderr));
    
    let file_count = String::from_utf8_lossy(&result.stdout).lines().count();
    assert!(file_count > 100, "expected many files, got {}", file_count);
    eprintln!("OK: roundtrip produced {} files", file_count);
}
