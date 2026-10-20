//! Request signing: `X-Moss-Signature: sha256=<hex>` is HMAC-SHA256 over `"<timestamp>.<body>"` with the endpoint secret.
use sha2::{Digest, Sha256};

pub fn hmac_sha256(key: &[u8], msg: &[u8]) -> [u8; 32] {
    let mut k = [0u8; 64];
    if key.len() > 64 { k[..32].copy_from_slice(&Sha256::digest(key)); } else { k[..key.len()].copy_from_slice(key); }
    let mut inner = Sha256::new();
    inner.update(k.map(|b| b ^ 0x36));
    inner.update(msg);
    let mut outer = Sha256::new();
    outer.update(k.map(|b| b ^ 0x5c));
    outer.update(inner.finalize());
    outer.finalize().into()
}

pub fn signature(secret: &str, timestamp: i64, body: &str) -> String {
    format!("sha256={}", hex::encode(hmac_sha256(secret.as_bytes(), format!("{timestamp}.{body}").as_bytes())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_rfc_4231_vectors() {
        assert_eq!(hex::encode(hmac_sha256(&[0x0b; 20], b"Hi There")), "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7");
        assert_eq!(hex::encode(hmac_sha256(b"Jefe", b"what do ya want for nothing?")), "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843");
        // a key longer than the block size is hashed first (test case 6)
        assert_eq!(hex::encode(hmac_sha256(&[0xaa; 131], b"Test Using Larger Than Block-Size Key - Hash Key First")), "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54");
    }

    #[test]
    fn signature_covers_timestamp_and_body() {
        let a = signature("s", 1, "{}");
        assert!(a.starts_with("sha256=") && a.len() == 71);
        assert_ne!(a, signature("s", 2, "{}"));
        assert_ne!(a, signature("s", 1, "{ }"));
        assert_ne!(a, signature("t", 1, "{}"));
    }
}
