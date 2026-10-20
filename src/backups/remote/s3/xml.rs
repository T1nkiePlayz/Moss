//! The few XML bits S3 needs: pulling tag contents out of responses and building the multipart completion body.

pub fn unescape(s: &str) -> String { s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&") }
pub fn escape(s: &str) -> String { s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;") }

/// Contents of every `<name>…</name>` in order (no nesting of the same tag is assumed).
pub fn all<'a>(xml: &'a str, name: &str) -> Vec<&'a str> {
    let (open, close) = (format!("<{name}>"), format!("</{name}>"));
    let mut out = vec![];
    let mut rest = xml;
    while let Some(i) = rest.find(&open) {
        let after = &rest[i + open.len()..];
        let Some(j) = after.find(&close) else { break };
        out.push(&after[..j]);
        rest = &after[j + close.len()..];
    }
    out
}
pub fn first(xml: &str, name: &str) -> Option<String> { all(xml, name).first().map(|s| unescape(s)) }

pub fn complete_body(parts: &[(u32, String)]) -> String {
    let mut s = String::from("<CompleteMultipartUpload>");
    for (n, e) in parts { s.push_str(&format!("<Part><PartNumber>{n}</PartNumber><ETag>{}</ETag></Part>", escape(e))); }
    s.push_str("</CompleteMultipartUpload>");
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn extracts_and_builds() {
        let x = "<R><Contents><Key>a&amp;b</Key><Size>3</Size></Contents><Contents><Key>c</Key><Size>4</Size></Contents><IsTruncated>false</IsTruncated></R>";
        assert_eq!(all(x, "Key"), vec!["a&amp;b", "c"]);
        assert_eq!(first(x, "Key").unwrap(), "a&b");
        assert_eq!(first(x, "Nope"), None);
        assert_eq!(complete_body(&[(1, "\"e1\"".into())]), "<CompleteMultipartUpload><Part><PartNumber>1</PartNumber><ETag>&quot;e1&quot;</ETag></Part></CompleteMultipartUpload>");
    }
}
