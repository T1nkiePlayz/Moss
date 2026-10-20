//! Secret-handling primitives: OS-RNG random strings, constant-time comparison and SHA-256 hex digests.
use rand::{TryRng, rngs::SysRng};
use sha2::{Digest, Sha256};

const ALPHABET: &[u8; 62] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";

/// A uniformly random alphanumeric (URL-safe) string from the operating system RNG.
/// Uses rejection sampling so no character is more likely than another.
pub fn random_string(len: usize) -> String {
    let mut out = String::with_capacity(len);
    let mut buf = [0u8; 64];
    while out.len() < len {
        SysRng.try_fill_bytes(&mut buf).expect("operating system random number generator failed");
        for b in buf {
            // 248 = 62 * 4: discard the top of the byte range to avoid modulo bias
            if b < 248 && out.len() < len { out.push(ALPHABET[(b % 62) as usize] as char); }
        }
    }
    out
}

/// Constant-time equality (the length is not secret here; contents are never compared early).
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() { return false; }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

pub fn sha256_hex(s: &str) -> String { hex::encode(Sha256::digest(s.as_bytes())) }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn random_strings_have_length_and_alphabet() {
        let a = random_string(40);
        assert_eq!(a.len(), 40);
        assert!(a.bytes().all(|b| ALPHABET.contains(&b)));
        assert_ne!(a, random_string(40));
        assert_eq!(random_string(0), "");
    }
    #[test]
    fn constant_time_compare() {
        assert!(ct_eq(b"abc", b"abc"));
        assert!(!ct_eq(b"abc", b"abd"));
        assert!(!ct_eq(b"abc", b"abcd"));
        assert!(ct_eq(b"", b""));
    }
    #[test]
    fn sha256_known_vector() {
        assert_eq!(sha256_hex("abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }
}
