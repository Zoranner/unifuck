#[path = "../src/hub/asar.rs"]
mod asar;

use serde_json::json;

#[inline]
fn align4(n: usize) -> usize {
    (n + 3) & !3
}

fn fixture_archive() -> asar::AsarArchive {
    let app_js = b"console.log('old');";
    let package_json = br#"{"version":"1.0.0"}"#;
    let nested = b"nested file";

    let mut data = Vec::new();
    let app_offset = data.len();
    data.extend_from_slice(app_js);
    let package_offset = data.len();
    data.extend_from_slice(package_json);
    let nested_offset = data.len();
    data.extend_from_slice(nested);

    asar::AsarArchive {
        header_json: json!({
            "files": {
                "build": {
                    "files": {
                        "main": {
                            "files": {
                                "app-test.js": {
                                    "size": app_js.len(),
                                    "offset": app_offset.to_string(),
                                    "integrity": {}
                                }
                            }
                        }
                    }
                },
                "package.json": {
                    "size": package_json.len(),
                    "offset": package_offset.to_string(),
                    "integrity": {}
                },
                "nested": {
                    "files": {
                        "fixture.txt": {
                            "size": nested.len(),
                            "offset": nested_offset.to_string(),
                            "integrity": {}
                        }
                    }
                }
            }
        }),
        data,
    }
}

#[test]
fn asar_roundtrip_read_write_uses_self_contained_fixture() {
    let archive = fixture_archive();
    let path = std::env::temp_dir().join(format!(
        "unifuck-asar-integration-{}.asar",
        std::process::id()
    ));

    asar::write_asar(
        &path,
        &[(
            "build/main/app-test.js".to_string(),
            b"console.log('patched');".to_vec(),
        )],
        &archive,
    )
    .unwrap();

    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(u32::from_le_bytes(bytes[0..4].try_into().unwrap()), 4);

    let header_buf_size = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
    let payload_size = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
    let json_len = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
    assert_eq!(payload_size, align4(4 + json_len));
    assert_eq!(header_buf_size, 4 + payload_size);

    let roundtripped = asar::read_asar(&path).unwrap();
    let app_entry = asar::find_entry(&roundtripped.header_json, "build/main/app-test.js").unwrap();
    assert_eq!(
        asar::extract_file(&roundtripped, &app_entry).unwrap(),
        b"console.log('patched');"
    );

    let package_entry = asar::find_entry(&roundtripped.header_json, "package.json").unwrap();
    assert_eq!(
        asar::extract_file(&roundtripped, &package_entry).unwrap(),
        br#"{"version":"1.0.0"}"#
    );

    let nested_entry = asar::find_entry(&roundtripped.header_json, "nested/fixture.txt").unwrap();
    assert_eq!(
        asar::extract_file(&roundtripped, &nested_entry).unwrap(),
        b"nested file"
    );

    std::fs::remove_file(path).ok();
}
