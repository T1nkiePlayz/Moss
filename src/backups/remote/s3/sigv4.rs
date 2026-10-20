//! AWS Signature Version 4 (header-based), hand written over `sha2`. Verified against the published AWS examples.
use sha2::{Digest, Sha256};

pub fn sha256_hex(b: &[u8]) -> String { hex::encode(Sha256::digest(b)) }

pub fn hmac(key: &[u8], msg: &[u8]) -> [u8; 32] {
    let mut k = [0u8; 64];
    if key.len() > 64 { k[..32].copy_from_slice(&Sha256::digest(key)); } else { k[..key.len()].copy_from_slice(key); }
    let (mut i, mut o) = (k, k);
    for b in i.iter_mut() { *b ^= 0x36; }
    for b in o.iter_mut() { *b ^= 0x5c; }
    let inner = Sha256::new().chain_update(i).chain_update(msg).finalize();
    Sha256::new().chain_update(o).chain_update(inner).finalize().into()
}

/// RFC 3986 percent-encoding as SigV4 wants it: unreserved characters stay, everything else is %XX; `/` only when `slash` is set.
pub fn uri_encode(s: &str, slash: bool) -> String {
    let mut o = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') || (b == b'/' && !slash) { o.push(b as char); } else { o.push_str(&format!("%{b:02X}")); }
    }
    o
}

pub struct Signed { pub authorization: String, pub signature: String }

/// `uri` is the already-encoded absolute path. `query` holds raw pairs (they are encoded and sorted here).
/// `headers` are (name, value) pairs; names are lowercased and values trimmed. `host` must be among them.
#[allow(clippy::too_many_arguments)]
pub fn sign(method: &str, uri: &str, query: &[(String, String)], headers: &[(String, String)], payload_hash: &str, access: &str, secret: &str, region: &str, amz_date: &str) -> Signed {
    let mut q: Vec<(String, String)> = query.iter().map(|(k, v)| (uri_encode(k, true), uri_encode(v, true))).collect();
    q.sort();
    let cq = q.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&");
    let mut h: Vec<(String, String)> = headers.iter().map(|(k, v)| (k.to_ascii_lowercase(), v.split_whitespace().collect::<Vec<_>>().join(" "))).collect();
    h.sort();
    let ch: String = h.iter().map(|(k, v)| format!("{k}:{v}\n")).collect();
    let signed = h.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>().join(";");
    let canonical = format!("{method}\n{uri}\n{cq}\n{ch}\n{signed}\n{payload_hash}");
    let date = &amz_date[..8];
    let scope = format!("{date}/{region}/s3/aws4_request");
    let sts = format!("AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}", sha256_hex(canonical.as_bytes()));
    let k = hmac(&hmac(&hmac(&hmac(format!("AWS4{secret}").as_bytes(), date.as_bytes()), region.as_bytes()), b"s3"), b"aws4_request");
    let signature = hex::encode(hmac(&k, sts.as_bytes()));
    Signed { authorization: format!("AWS4-HMAC-SHA256 Credential={access}/{scope}, SignedHeaders={signed}, Signature={signature}"), signature }
}

#[cfg(test)]
mod tests {
    use super::*;
    const AK: &str = "AKIAIOSFODNN7EXAMPLE";
    const SK: &str = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";
    const EMPTY: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    fn h(v: &[(&str, &str)]) -> Vec<(String, String)> { v.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect() }

    #[test]
    fn hmac_rfc_4231_case_2() {
        assert_eq!(hex::encode(hmac(b"Jefe", b"what do ya want for nothing?")), "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843");
        assert_eq!(hex::encode(hmac(&[0xaa; 131], b"Test Using Larger Than Block-Size Key - Hash Key First")), "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54");
    }

    #[test]
    fn aws_get_object_with_range() {
        let s = sign("GET", "/test.txt", &[], &h(&[("Host", "examplebucket.s3.amazonaws.com"), ("Range", "bytes=0-9"), ("x-amz-content-sha256", EMPTY), ("x-amz-date", "20130524T000000Z")]), EMPTY, AK, SK, "us-east-1", "20130524T000000Z");
        assert_eq!(s.signature, "f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41");
        assert!(s.authorization.contains("SignedHeaders=host;range;x-amz-content-sha256;x-amz-date"));
    }

    #[test]
    fn aws_put_object_with_special_key_and_storage_class() {
        let body = sha256_hex(b"Welcome to Amazon S3.");
        assert_eq!(body, "44ce7dd67c959e0d3524ffac1771dfbba87d2b6b4b4e99e42034a8b803f8b072");
        let uri = format!("/{}", uri_encode("test$file.text", false));
        assert_eq!(uri, "/test%24file.text");
        let s = sign("PUT", &uri, &[], &h(&[("Date", "Fri, 24 May 2013 00:00:00 GMT"), ("Host", "examplebucket.s3.amazonaws.com"), ("x-amz-date", "20130524T000000Z"), ("x-amz-storage-class", "REDUCED_REDUNDANCY"), ("x-amz-content-sha256", &body)]), &body, AK, SK, "us-east-1", "20130524T000000Z");
        assert_eq!(s.signature, "98ad721746da40c64f1a55b78f14c238d841ea1380cd77a1b5971af0ece108bd");
    }

    #[test]
    fn aws_get_bucket_lifecycle_subresource() {
        let s = sign("GET", "/", &h(&[("lifecycle", "")]), &h(&[("Host", "examplebucket.s3.amazonaws.com"), ("x-amz-date", "20130524T000000Z"), ("x-amz-content-sha256", EMPTY)]), EMPTY, AK, SK, "us-east-1", "20130524T000000Z");
        assert_eq!(s.signature, "fea454ca298b7da1c68078a5d1bdbfbbe0d65c699e0f91ac7a200a0136783543");
    }

    #[test]
    fn aws_list_objects_with_sorted_query() {
        let s = sign("GET", "/", &h(&[("prefix", "J"), ("max-keys", "2")]), &h(&[("Host", "examplebucket.s3.amazonaws.com"), ("x-amz-date", "20130524T000000Z"), ("x-amz-content-sha256", EMPTY)]), EMPTY, AK, SK, "us-east-1", "20130524T000000Z");
        assert_eq!(s.signature, "34b48302e7b5fa45bde8084f4b7868a86f0a534bc59db6670ed5711ef69dc6f7");
    }

    #[test]
    fn encoding_rules() {
        assert_eq!(uri_encode("a b/c+d~e", false), "a%20b/c%2Bd~e");
        assert_eq!(uri_encode("a/b", true), "a%2Fb");
        assert_eq!(uri_encode("é", false), "%C3%A9");
    }
}
