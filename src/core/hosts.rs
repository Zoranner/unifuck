use anyhow::Context;

const HOSTS_PATH: &str = r"C:\Windows\System32\drivers\etc\hosts";
const HOSTS_BEGIN_MARKER: &str = "# unifuck";
const HOSTS_END_MARKER: &str = "# unifuck-end";

const HOSTS_ENTRIES: &str = "\n\
# unifuck\n\
127.0.0.1 license.unity3d.com\n\
127.0.0.1 core.cloud.unity3d.com\n\
127.0.0.1 api.unity.com\n\
127.0.0.1 unity.com\n\
# unifuck-end\n";

pub fn write_hosts_if_needed() -> anyhow::Result<()> {
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

pub fn clean_hosts() -> anyhow::Result<()> {
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
