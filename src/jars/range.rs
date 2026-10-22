//! Best-effort version comparison and range evaluation: Fabric/Quilt semver ranges (`>=1.2 <2`, `^1.2`, `~1.2`, `1.20.x`,
//! `a || b`) and Forge/NeoForge Maven ranges (`[1.0,2.0)`, `(,3]`, `[1.5]`). Anything that cannot be parsed yields `None`
//! ("unknown"), which callers treat as satisfied: a guess must never block a server.
use std::cmp::Ordering;

#[derive(Debug, Clone, PartialEq)]
pub struct Ver { pub nums: Vec<u64>, pub pre: Option<String> }

/// `1.20.1`, `v2.0`, `1.0.0-beta.1`, `1.2+build5` (build metadata is ignored). `None` when it does not start with a number.
pub fn parse_ver(s: &str) -> Option<Ver> {
    let s = s.trim().trim_start_matches(['v', 'V']);
    let s = s.split('+').next()?;
    let (core, pre) = match s.split_once('-') { Some((c, p)) => (c, Some(p.to_string())), None => (s, None) };
    let mut nums = vec![];
    for part in core.split('.') {
        let digits: String = part.chars().take_while(char::is_ascii_digit).collect();
        if digits.is_empty() { return if nums.is_empty() { None } else { Some(Ver { nums, pre }) }; }
        nums.push(digits.parse().ok()?);
        if digits.len() != part.len() { break; }
    }
    (!nums.is_empty()).then_some(Ver { nums, pre })
}

pub fn cmp(a: &Ver, b: &Ver) -> Ordering {
    for i in 0..a.nums.len().max(b.nums.len()) {
        let o = a.nums.get(i).unwrap_or(&0).cmp(b.nums.get(i).unwrap_or(&0));
        if o != Ordering::Equal { return o; }
    }
    match (&a.pre, &b.pre) { (None, None) => Ordering::Equal, (None, Some(_)) => Ordering::Greater, (Some(_), None) => Ordering::Less, (Some(x), Some(y)) => x.cmp(y) }
}

/// Orders two version strings; unparsable ones fall back to plain string order.
pub fn cmp_str(a: &str, b: &str) -> Ordering {
    match (parse_ver(a), parse_ver(b)) { (Some(x), Some(y)) => cmp(&x, &y), _ => a.cmp(b) }
}

/// Does `version` satisfy `range`? `None` when either cannot be understood.
pub fn satisfies(range: &str, version: &str) -> Option<bool> {
    let range = range.trim();
    if range.is_empty() || range == "*" { return Some(true); }
    let v = parse_ver(version)?;
    if range.starts_with(['[', '(']) { maven(range, &v) } else { semver(range, &v) }
}

fn maven(range: &str, v: &Ver) -> Option<bool> {
    let mut rest = range.trim();
    let mut unknown = true;
    while !rest.is_empty() {
        let open = rest.chars().next()?;
        let end = rest.find([']', ')'])?;
        let (lo_inc, hi_inc) = (open == '[', rest.as_bytes()[end] == b']');
        let body = &rest[1..end];
        rest = rest[end + 1..].trim_start_matches([',', ' ']);
        let ok = match body.split_once(',') {
            None => { let e = parse_ver(body)?; cmp(v, &e) == Ordering::Equal }
            Some((lo, hi)) => {
                let lo_ok = match lo.trim() { "" => true, l => { let o = cmp(v, &parse_ver(l)?); o == Ordering::Greater || (lo_inc && o == Ordering::Equal) } };
                let hi_ok = match hi.trim() { "" => true, h => { let o = cmp(v, &parse_ver(h)?); o == Ordering::Less || (hi_inc && o == Ordering::Equal) } };
                lo_ok && hi_ok
            }
        };
        unknown = false;
        if ok { return Some(true); }
    }
    if unknown { None } else { Some(false) }
}

fn semver(range: &str, v: &Ver) -> Option<bool> {
    let (mut any_known, mut any_unknown) = (false, false);
    for alt in range.split("||") {
        match all_of(alt, v) { Some(true) => return Some(true), Some(false) => any_known = true, None => any_unknown = true }
    }
    (any_known && !any_unknown).then_some(false)
}

fn all_of(alt: &str, v: &Ver) -> Option<bool> {
    let mut toks: Vec<String> = vec![];
    for t in alt.split_whitespace() {
        match toks.last_mut() { Some(l) if l.chars().all(|c| "<>=^~".contains(c)) => l.push_str(t), _ => toks.push(t.to_string()) }
    }
    if toks.is_empty() { return Some(true); }
    let mut result = true;
    for t in toks { result &= comparator(&t, v)?; }
    Some(result)
}

fn ver(s: &str) -> Option<Ver> { parse_ver(s) }

fn bump(nums: &[u64], at: usize) -> Ver {
    let mut n: Vec<u64> = nums.iter().take(at + 1).copied().collect();
    n.resize(at + 1, 0);
    n[at] += 1;
    Ver { nums: n, pre: None }
}

fn comparator(t: &str, v: &Ver) -> Option<bool> {
    let split = t.find(|c: char| !"<>=^~".contains(c)).unwrap_or(t.len());
    let (op, rest) = t.split_at(split);
    if rest == "*" || rest.eq_ignore_ascii_case("x") { return Some(true); }
    // x-ranges: 1.20.x, 1.x, 1.20.*
    let parts: Vec<&str> = rest.split('.').collect();
    if let Some(w) = parts.iter().position(|p| matches!(*p, "x" | "X" | "*")) {
        let base = ver(&parts[..w].join("."))?;
        let hi = bump(&base.nums, w - 1);
        return Some(cmp(v, &base) != Ordering::Less && cmp(v, &hi) == Ordering::Less);
    }
    let b = ver(rest)?;
    let o = cmp(v, &b);
    Some(match op {
        ">=" => o != Ordering::Less,
        ">" => o == Ordering::Greater,
        "<=" => o != Ordering::Greater,
        "<" => o == Ordering::Less,
        "" | "=" | "==" => o == Ordering::Equal,
        "~" => o != Ordering::Less && cmp(v, &bump(&b.nums, if b.nums.len() > 1 { 1 } else { 0 })) == Ordering::Less,
        "^" => {
            let at = b.nums.iter().position(|n| *n != 0).unwrap_or(b.nums.len().saturating_sub(1));
            o != Ordering::Less && cmp(v, &bump(&b.nums, at)) == Ordering::Less
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::satisfies;

    #[test]
    fn or_range_with_unparsable_alternative_is_unknown_when_nothing_matches() {
        assert_eq!(satisfies(">=2.0 || foo", "1.0"), None);
        assert_eq!(satisfies(">=2.0 || foo", "3.0"), Some(true));
        assert_eq!(satisfies(">=2.0 || <1.0", "1.5"), Some(false));
    }
}
