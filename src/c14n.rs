use anyhow::Context;

/// Escape special XML characters in text content per C14N rules.
fn escape_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\r' => out.push_str("&#xD;"),
            _ => out.push(c),
        }
    }
    out
}

/// Escape special characters in XML attribute values per C14N rules.
fn escape_attr(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '"' => out.push_str("&quot;"),
            '\t' => out.push_str("&#x9;"),
            '\n' => out.push_str("&#xA;"),
            '\r' => out.push_str("&#xD;"),
            _ => out.push(c),
        }
    }
    out
}

/// Recursively canonicalize an XML element subtree per C14N with comments.
fn canonicalize_node(node: roxmltree::Node, parent_ns: &str, output: &mut Vec<u8>) {
    match node.node_type() {
        roxmltree::NodeType::Element => {
            let ns = node.tag_name().namespace().unwrap_or("");
            let local = node.tag_name().name();
            let ns_differs = ns != parent_ns;

            output.push(b'<');
            output.extend_from_slice(local.as_bytes());

            // Emit namespace declaration on root element of the subtree if needed
            if ns_differs && !ns.is_empty() {
                output.extend_from_slice(b" xmlns=\"");
                output.extend_from_slice(ns.as_bytes());
                output.push(b'"');
            }

            // Collect and sort attributes: namespace URI then local name
            let mut attrs: Vec<_> = node.attributes().collect();
            attrs.sort_by(|a, b| {
                let na = a.namespace().unwrap_or("");
                let nb = b.namespace().unwrap_or("");
                na.cmp(nb).then_with(|| a.name().cmp(b.name()))
            });

            // Emit non-namespace attributes
            for attr in &attrs {
                let an = attr.namespace().unwrap_or("");
                if an.is_empty() {
                    output.push(b' ');
                    output.extend_from_slice(attr.name().as_bytes());
                    output.extend_from_slice(b"=\"");
                    output.extend_from_slice(escape_attr(attr.value()).as_bytes());
                    output.push(b'"');
                }
            }

            // Emit namespaced attributes
            for attr in &attrs {
                let an = attr.namespace().unwrap_or("");
                if !an.is_empty() {
                    output.push(b' ');
                    output.extend_from_slice(attr.name().as_bytes());
                    output.extend_from_slice(b"=\"");
                    output.extend_from_slice(escape_attr(attr.value()).as_bytes());
                    output.push(b'"');
                }
            }

            output.push(b'>');

            // Children
            for child in node.children() {
                canonicalize_node(child, ns, output);
            }

            output.extend_from_slice(b"</");
            output.extend_from_slice(local.as_bytes());
            output.push(b'>');
        }
        roxmltree::NodeType::Text => {
            let text = node.text().unwrap_or("");
            // C14N: normalize line endings \r\n → \n, isolated \r → \n
            let normalized: String = text
                .chars()
                .map(|c| match c {
                    '\r' => {
                        // Handled by escape_text as &#xD; but first normalize
                        '\n'
                    }
                    other => other,
                })
                .collect();
            output.extend_from_slice(escape_text(&normalized).as_bytes());
        }
        roxmltree::NodeType::Comment => {
            let text = node.text().unwrap_or("");
            output.extend_from_slice(b"<!--");
            output.extend_from_slice(text.as_bytes());
            output.extend_from_slice(b"-->");
        }
        _ => {}
    }
}

/// Canonicalize the XML subtree rooted at the element with `target_id`.
///
/// Returns the canonical octets (C14N with comments).
pub fn canonicalize_subtree(xml: &str, target_id: &str) -> anyhow::Result<Vec<u8>> {
    let doc = roxmltree::Document::parse(xml).context("Failed to parse XML")?;

    let target = doc
        .descendants()
        .find(|n| n.is_element() && n.attribute("id") == Some(target_id))
        .with_context(|| format!("Element with id='{}' not found", target_id))?;

    let mut output = Vec::new();
    canonicalize_node(target, "", &mut output);
    Ok(output)
}

/// Canonicalize the SignedInfo element (no id attribute, find by tag name).
pub fn canonicalize_signed_info(xml: &str) -> anyhow::Result<Vec<u8>> {
    let doc = roxmltree::Document::parse(xml).context("Failed to parse XML")?;

    let target = doc
        .descendants()
        .find(|n| n.is_element() && n.has_tag_name("SignedInfo"))
        .context("SignedInfo element not found")?;

    let mut output = Vec::new();
    canonicalize_node(target, "", &mut output);
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escape_text() {
        assert_eq!(escape_text("a & b"), "a &amp; b");
        assert_eq!(escape_text("a < b"), "a &lt; b");
        assert_eq!(escape_text("a > b"), "a &gt; b");
    }

    #[test]
    fn test_canonicalize_empty_element() {
        let xml = r#"<root>
    <License id="Terms">
        <NoHardwareCheck Value="true"/>
        <Foo bar="baz"/>
    </License>
</root>"#;
        let result = canonicalize_subtree(xml, "Terms").unwrap();
        let result_str = String::from_utf8_lossy(&result);
        // Empty elements expand to start+end tags
        assert!(result_str.contains("<NoHardwareCheck Value=\"true\"></NoHardwareCheck>"));
        assert!(result_str.contains("<Foo bar=\"baz\"></Foo>"));
    }

    #[test]
    fn test_attribute_sorting() {
        let xml = r#"<root>
    <License id="Terms">
        <Binding Key="1" Value="foo"/>
    </License>
</root>"#;
        let result = canonicalize_subtree(xml, "Terms").unwrap();
        let result_str = String::from_utf8_lossy(&result);
        // "Key" < "Value" alphabetically
        assert!(
            result_str.contains("<Binding Key=\"1\" Value=\"foo\">"),
            "Expected Key before Value, got: {}",
            result_str
        );
    }

    #[test]
    fn test_signed_info_has_xmlns() {
        let xml = r##"<root>
    <Signature xmlns="http://www.w3.org/2000/09/xmldsig#">
        <SignedInfo>
            <CanonicalizationMethod Algorithm="http://www.w3.org/TR/2001/REC-xml-c14n-20010315#WithComments"/>
            <SignatureMethod Algorithm="http://www.w3.org/2000/09/xmldsig#rsa-sha1"/>
            <Reference URI="#Terms">
                <DigestMethod Algorithm="http://www.w3.org/2000/09/xmldsig#sha1"/>
                <DigestValue>dGVzdA==</DigestValue>
            </Reference>
        </SignedInfo>
        <SignatureValue>dGVzdA==</SignatureValue>
    </Signature>
</root>"##;
        let result = canonicalize_signed_info(xml).unwrap();
        let result_str = String::from_utf8_lossy(&result);
        assert!(
            result_str.contains(r#"xmlns="http://www.w3.org/2000/09/xmldsig#""#),
            "SignedInfo should have xmlns declaration, got: {}",
            result_str
        );
    }
}
