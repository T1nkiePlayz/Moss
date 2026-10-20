//! Splits the raw log stream into lines in linear time and caps their length. A game (or a misbehaving plugin) can
//! print one multi-megabyte line; without a cap the ring buffer, the broadcast channel and every browser would hold it.
use std::mem;

/// Longest line kept, in bytes. Anything beyond is dropped and replaced by a marker.
pub const MAX_LINE: usize = 8 * 1024;

#[derive(Default)]
pub struct LineSplitter {
    buf: Vec<u8>,
    /// Bytes of the current line already thrown away because it exceeded `MAX_LINE`.
    dropped: usize,
}

fn text(b: &[u8]) -> String { String::from_utf8_lossy(b).trim_end_matches('\r').to_string() }

impl LineSplitter {
    fn finish_line(&mut self, out: &mut impl FnMut(String)) {
        let mut line = text(&mem::take(&mut self.buf));
        if self.dropped > 0 {
            line.push_str(&format!(" … [line cut by Moss: {} more bytes]", self.dropped));
            self.dropped = 0;
        }
        out(line);
    }

    fn take(&mut self, part: &[u8]) {
        let room = MAX_LINE.saturating_sub(self.buf.len());
        let keep = part.len().min(room);
        self.buf.extend_from_slice(&part[..keep]);
        self.dropped += part.len() - keep;
    }

    /// Feed raw bytes; `out` receives each completed line (without its newline). Memory held is at most `MAX_LINE`.
    pub fn feed(&mut self, mut bytes: &[u8], mut out: impl FnMut(String)) {
        while let Some(i) = bytes.iter().position(|&b| b == b'\n') {
            self.take(&bytes[..i]);
            self.finish_line(&mut out);
            bytes = &bytes[i + 1..];
        }
        self.take(bytes);
    }

    /// The log ended mid-line (the process exited): emit what is pending.
    pub fn flush(&mut self, mut out: impl FnMut(String)) { if !self.buf.is_empty() || self.dropped > 0 { self.finish_line(&mut out); } }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(chunks: &[&[u8]]) -> Vec<String> {
        let (mut s, mut v) = (LineSplitter::default(), vec![]);
        for c in chunks { s.feed(c, |l| v.push(l)); }
        s.flush(|l| v.push(l));
        v
    }

    #[test]
    fn splits_on_newlines_across_chunk_boundaries() {
        assert_eq!(run(&[b"one\ntw", b"o\r\nthree"]), ["one", "two", "three"]);
        assert_eq!(run(&[b"\n\n"]), ["", ""]);
        assert!(run(&[b""]).is_empty());
    }

    #[test]
    fn multibyte_characters_split_across_chunks_survive() {
        let b = "gr\u{fc}n\n".as_bytes();
        assert_eq!(run(&[&b[..3], &b[3..]]), ["gr\u{fc}n"]);
    }

    #[test]
    fn a_huge_line_is_cut_and_memory_stays_bounded() {
        let big = vec![b'x'; 5 << 20];
        let (mut s, mut v) = (LineSplitter::default(), vec![]);
        for c in big.chunks(64 * 1024) { s.feed(c, |l| v.push(l)); assert!(s.buf.len() <= MAX_LINE); }
        s.feed(b"\nafter\n", |l| v.push(l));
        assert_eq!(v.len(), 2);
        assert!(v[0].len() < MAX_LINE + 64 && v[0].contains("line cut by Moss") && v[0].starts_with("xxxx"));
        assert_eq!(v[1], "after");
    }
}
