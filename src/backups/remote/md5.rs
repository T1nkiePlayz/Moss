//! MD5, needed only to check S3 ETags (which are MD5 digests; not used for any security decision).

const S: [u32; 64] = [7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20,
    4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21];

pub struct Md5 { st: [u32; 4], buf: Vec<u8>, len: u64, k: [u32; 64] }

impl Default for Md5 { fn default() -> Self { Self::new() } }

impl Md5 {
    pub fn new() -> Self {
        let mut k = [0u32; 64];
        for (i, v) in k.iter_mut().enumerate() { *v = ((i as f64 + 1.0).sin().abs() * 4294967296.0) as u32; }
        Self { st: [0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476], buf: vec![], len: 0, k }
    }
    fn block(&mut self, b: &[u8]) {
        let m: Vec<u32> = b.chunks(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
        let [mut a, mut bb, mut c, mut d] = self.st;
        #[allow(clippy::needless_range_loop)]
        for i in 0..64 {
            let (f, g) = match i / 16 { 0 => ((bb & c) | (!bb & d), i), 1 => ((d & bb) | (!d & c), (5 * i + 1) % 16), 2 => (bb ^ c ^ d, (3 * i + 5) % 16), _ => (c ^ (bb | !d), (7 * i) % 16) };
            let t = d; d = c; c = bb;
            bb = bb.wrapping_add(a.wrapping_add(f).wrapping_add(self.k[i]).wrapping_add(m[g]).rotate_left(S[i]));
            a = t;
        }
        for (s, v) in self.st.iter_mut().zip([a, bb, c, d]) { *s = s.wrapping_add(v); }
    }
    pub fn update(&mut self, data: &[u8]) {
        self.len += data.len() as u64;
        self.buf.extend_from_slice(data);
        let n = self.buf.len() / 64 * 64;
        let full: Vec<u8> = self.buf.drain(..n).collect();
        for b in full.chunks(64) { self.block(b); }
    }
    pub fn finish(mut self) -> [u8; 16] {
        let bits = self.len.wrapping_mul(8);
        let mut pad = vec![0x80u8];
        while (self.buf.len() + pad.len()) % 64 != 56 { pad.push(0); }
        pad.extend_from_slice(&bits.to_le_bytes());
        let l = self.len;
        self.update(&pad);
        self.len = l;
        let mut out = [0u8; 16];
        for (i, v) in self.st.iter().enumerate() { out[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes()); }
        out
    }
}

#[cfg(test)]
pub fn md5(data: &[u8]) -> [u8; 16] { let mut m = Md5::new(); m.update(data); m.finish() }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rfc_1321_vectors() {
        for (i, o) in [("", "d41d8cd98f00b204e9800998ecf8427e"), ("a", "0cc175b9c0f1b6a831c399e269772661"), ("abc", "900150983cd24fb0d6963f7d28e17f72"),
            ("message digest", "f96b697d7cb7938d525a2f31aaf161d0"), ("12345678901234567890123456789012345678901234567890123456789012345678901234567890", "57edf4a22be3c955ac49da2e2107b67a")] {
            assert_eq!(hex::encode(md5(i.as_bytes())), o);
        }
        let mut m = Md5::new();
        for c in "abcdefghijklmnopqrstuvwxyz".as_bytes().chunks(5) { m.update(c); }
        assert_eq!(hex::encode(m.finish()), "c3fcd3d76192e4007dfb496cca67e13b");
    }
}
