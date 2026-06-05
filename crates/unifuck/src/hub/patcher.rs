/// Replace the body of a JS function identified by `signature`.
///
/// Handles:
/// - `async` prefix (e.g. `async methodName(...)`)
/// - Arrow functions (e.g. `methodName = (...) => {}`)
/// - Class methods (e.g. `methodName(...) {}`)
///
/// Skips comments and string literals while searching and counting braces.
pub(crate) fn replace_fn_body(source: &str, signature: &str, new_body: &str) -> Option<String> {
    let src = source.as_bytes();
    let pat = signature.as_bytes();
    let mut search_start = 0;

    loop {
        let start = source[search_start..].find(signature)? + search_start;
        let after_pat = start + pat.len();

        // Reject if preceded by an identifier character (partial match).
        if start > 0 {
            let prev = src[start - 1];
            if prev.is_ascii_alphanumeric() || prev == b'_' || prev == b'$' {
                search_start = after_pat;
                continue;
            }
        }

        // Skip past ')' to find the opening brace.
        let mut pos = after_pat;
        let mut paren_depth: i32 = 1;
        while pos < src.len() && paren_depth > 0 {
            match src[pos] {
                b'(' => paren_depth += 1,
                b')' => paren_depth -= 1,
                _ => {}
            }
            if paren_depth > 0 {
                pos += 1;
            }
        }
        if paren_depth != 0 {
            search_start = after_pat;
            continue;
        }
        pos += 1; // move past ')'

        // Skip whitespace / `async` / `=>` until '{'.
        while pos < src.len() {
            let b = src[pos];
            if b == b' ' || b == b'\t' || b == b'\n' || b == b'\r' {
                pos += 1;
            } else if src[pos..].starts_with(b"=>") {
                pos += 2;
            } else if b == b'{' {
                break;
            } else {
                // Unexpected character — not a function definition, retry.
                break;
            }
        }
        if pos >= src.len() || src[pos] != b'{' {
            search_start = after_pat;
            continue;
        }

        // Match braces to find function body end, skipping comments and strings.
        let brace_start = pos;
        let mut depth: i32 = 0;
        let mut end = pos;
        while pos < src.len() {
            match src[pos] {
                // Line comment
                b'/' if pos + 1 < src.len() && src[pos + 1] == b'/' => {
                    pos += 2;
                    while pos < src.len() && src[pos] != b'\n' {
                        pos += 1;
                    }
                }
                // Block comment
                b'/' if pos + 1 < src.len() && src[pos + 1] == b'*' => {
                    pos += 2;
                    while pos + 1 < src.len() && !(src[pos] == b'*' && src[pos + 1] == b'/') {
                        pos += 1;
                    }
                    if pos + 1 < src.len() {
                        pos += 2;
                    }
                }
                // String literals
                b'"' | b'\'' | b'`' => {
                    let quote = src[pos];
                    pos += 1;
                    while pos < src.len() && src[pos] != quote {
                        if src[pos] == b'\\' && pos + 1 < src.len() {
                            pos += 2;
                        } else {
                            pos += 1;
                        }
                    }
                    if pos < src.len() {
                        pos += 1;
                    }
                }
                b'{' => {
                    depth += 1;
                    pos += 1;
                }
                b'}' => {
                    depth -= 1;
                    pos += 1;
                    if depth == 0 {
                        end = pos;
                        break;
                    }
                }
                _ => {
                    pos += 1;
                }
            }
        }

        if end <= brace_start {
            search_start = after_pat;
            continue;
        }

        let mut result = String::with_capacity(source.len());
        result.push_str(&source[..brace_start]);
        result.push_str(new_body);
        result.push_str(&source[end..]);
        return Some(result);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_replace_simple_method() {
        let source = r#"
class Foo {
    bar() {
        return 1;
    }
}
"#;
        let result = replace_fn_body(source, "bar(", "{ return 2; }");
        assert!(result.is_some());
        assert!(result.unwrap().contains("return 2;"));
    }

    #[test]
    fn test_replace_async_method() {
        let source = r#"
class Foo {
    async bar() {
        await something();
        return 1;
    }
}
"#;
        let result = replace_fn_body(source, "bar(", "{ return 2; }");
        assert!(result.is_some());
    }

    #[test]
    fn test_skip_string_with_braces() {
        let source = r#"
function test() {
    const s = "not { a } brace";
    return 1;
}
"#;
        let result = replace_fn_body(source, "test(", "{ return 2; }");
        assert!(result.is_some());
        assert!(result.unwrap().contains("return 2;"));
    }
}
