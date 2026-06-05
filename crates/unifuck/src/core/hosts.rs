use anyhow::Context;

const HOSTS_PATH: &str = r"C:\Windows\System32\drivers\etc\hosts";
const HOSTS_MARKER_PREFIX: &str = "# unifuck";

const HOSTS_ENTRIES: &str = "\n\
# unifuck\n\
127.0.0.1 license.unity3d.com\n\
127.0.0.1 core.cloud.unity3d.com\n\
127.0.0.1 api.unity.com\n\
127.0.0.1 unity.com\n\
127.0.0.1 license.tuanjie.cn\n\
127.0.0.1 core.cloud.tuanjie.cn\n\
127.0.0.1 api.tuanjie.cn\n\
127.0.0.1 tuanjie.cn\n\
# unifuck-end\n";

/// Idempotent: strip any pre-existing unifuck block (even if a previous
/// version wrote it without an end marker), then append the current entries.
/// If the resulting file matches the on-disk content exactly, no write.
pub fn write_hosts_if_needed() -> anyhow::Result<()> {
    let content = std::fs::read_to_string(HOSTS_PATH)
        .with_context(|| format!("read failed: {}", HOSTS_PATH))?;

    let stripped = strip_unifuck_block(&content);
    let desired = format!(
        "{}{}",
        stripped.trim_end_matches(['\n', '\r']),
        HOSTS_ENTRIES
    );

    if desired == content {
        return Ok(());
    }

    let bak = format!("{}.bak", HOSTS_PATH);
    if !std::path::Path::new(&bak).exists() {
        std::fs::write(&bak, &content)
            .with_context(|| format!("hosts backup write failed: {}", bak))?;
    }

    std::fs::write(HOSTS_PATH, desired)
        .with_context(|| format!("write hosts failed: {}", HOSTS_PATH))?;
    Ok(())
}

/// Strip the unifuck block from hosts. Robust to missing end marker.
pub fn clean_hosts() -> anyhow::Result<()> {
    let content = std::fs::read_to_string(HOSTS_PATH)
        .with_context(|| format!("read failed: {}", HOSTS_PATH))?;
    let stripped = strip_unifuck_block(&content);
    if stripped == content {
        return Ok(());
    }
    std::fs::write(HOSTS_PATH, stripped)
        .with_context(|| format!("write hosts failed: {}", HOSTS_PATH))?;
    Ok(())
}

/// Remove the unifuck block (begin marker + 127.0.0.1 entries + end marker).
/// Tolerates malformed blocks left by previous versions: stops at the first
/// line that is neither a `# unifuck*` marker nor a `127.0.0.1 ...` entry.
fn strip_unifuck_block(content: &str) -> String {
    let Some(begin) = content.find(HOSTS_MARKER_PREFIX) else {
        return content.to_string();
    };

    // Swallow exactly one preceding line ending (the leading `\n` that
    // `HOSTS_ENTRIES` writes). Leaves the file's natural trailing newline
    // on whatever line existed before the block.
    let mut start = begin;
    if start > 0 && content.as_bytes()[start - 1] == b'\n' {
        start -= 1;
        if start > 0 && content.as_bytes()[start - 1] == b'\r' {
            start -= 1;
        }
    }

    // Walk forward, consuming any line that is part of our block. The
    // `# unifuck-end` marker is the hard stop; anything after it is the
    // user's, even if it's another `127.0.0.1 ...` entry.
    let bytes = content.as_bytes();
    let mut pos = begin;
    while pos < bytes.len() {
        let line_end = bytes[pos..]
            .iter()
            .position(|&c| c == b'\n')
            .map(|p| pos + p + 1)
            .unwrap_or(bytes.len());
        let line = content[pos..line_end].trim_end_matches(['\n', '\r']);
        let is_end_marker = line.starts_with("# unifuck-end");
        if line.starts_with(HOSTS_MARKER_PREFIX) || line.starts_with("127.0.0.1 ") {
            pos = line_end;
            if is_end_marker {
                break;
            }
        } else {
            break;
        }
    }

    format!("{}{}", &content[..start], &content[pos..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_handles_legacy_block_without_end_marker() {
        let input = "127.0.0.1 localhost\n\n\
            # unifuck\n\
            127.0.0.1 license.unity3d.com\n\
            127.0.0.1 unity.com\n\
            # some other comment\n";
        let out = strip_unifuck_block(input);
        assert_eq!(out, "127.0.0.1 localhost\n# some other comment\n");
    }

    #[test]
    fn strip_handles_full_block_with_end_marker() {
        // Realistic shape: `write_hosts_if_needed` always prepends a leading
        // `\n` to `HOSTS_ENTRIES`, so a written block is always preceded by
        // two newlines (file's natural trailing `\n` + our injected `\n`).
        let input = "127.0.0.1 localhost\n\n\
            # unifuck\n\
            127.0.0.1 license.unity3d.com\n\
            # unifuck-end\n\
            127.0.0.1 something-else\n";
        let out = strip_unifuck_block(input);
        assert_eq!(out, "127.0.0.1 localhost\n127.0.0.1 something-else\n");
    }

    #[test]
    fn strip_is_noop_when_no_marker() {
        let input = "127.0.0.1 localhost\n";
        assert_eq!(strip_unifuck_block(input), input);
    }
}
