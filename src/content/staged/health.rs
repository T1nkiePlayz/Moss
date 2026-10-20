//! Reading a test copy's console log: how many errors and warnings it printed and whether anything in it means
//! the server is broken. Pure functions; the run module feeds them the log text.
/// Substrings that mean the server (or a mod/plugin) is in real trouble, whatever level the line was logged at.
const FATAL: &[&str] = &[
    "OutOfMemoryError", "Exception in thread", "NoClassDefFoundError", "ClassNotFoundException", "NoSuchMethodError", "NoSuchFieldError",
    "UnsupportedClassVersionError", "Encountered an unexpected exception", "Failed to start the minecraft server", "A fatal error has been detected",
    "Crash report saved", "This crash report has been saved", "Error occurred while enabling", "Mod loading has failed", "Mixin apply failed",
    "FAILED TO BIND TO PORT", "Could not load '", "Failed to load plugin", "Unable to find a suitable Java", "Missing or unsupported mandatory dependencies",
    "[moss] the server was killed", "Incompatible mod set",
];

const MAX_SAMPLES: usize = 8;

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Findings {
    pub errors: usize,
    pub warnings: usize,
    /// First few error and fatal lines, trimmed, so the person can see what went wrong without opening the log.
    pub samples: Vec<String>,
    /// The first fatal line, if any.
    pub fatal: Option<String>,
}

/// Removes ANSI colour sequences and carriage returns.
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\x1b' {
            if it.peek() == Some(&'[') { it.next(); for n in it.by_ref() { if n.is_ascii_alphabetic() { break; } } }
        } else if c != '\r' { out.push(c); }
    }
    out
}

fn level(line: &str) -> Option<&'static str> {
    // "[12:00:01 ERROR]: x", "[Server thread/ERROR]: x", "12:00:01 [ERROR] x", "ERROR]" and "WARN]" in the common formats
    for (tag, name) in [("ERROR", "error"), ("FATAL", "error"), ("WARN", "warn")] {
        for pat in [format!("/{tag}]"), format!(" {tag}]"), format!("[{tag}]"), format!("[{tag}:"), format!("/{tag}:")] {
            if line.contains(&pat) { return Some(name); }
        }
    }
    None
}

fn trim(line: &str) -> String { let t = line.trim(); if t.chars().count() > 220 { format!("{}…", t.chars().take(220).collect::<String>()) } else { t.to_string() } }

/// Counts levels and finds fatal patterns. Stack-trace continuation lines ("\tat ...", "Caused by") are not counted again.
pub fn analyze(log: &str) -> Findings {
    let mut f = Findings::default();
    for raw in strip_ansi(log).lines() {
        let line = raw.trim_end();
        if line.starts_with('\t') || line.starts_with("    at ") || line.starts_with("at ") { continue; }
        let fatal = FATAL.iter().any(|p| line.contains(p));
        match level(line) {
            Some("error") => f.errors += 1,
            Some("warn") => f.warnings += 1,
            _ => if fatal { f.errors += 1 },
        }
        if fatal && f.fatal.is_none() { f.fatal = Some(trim(line)); }
        if (fatal || level(line) == Some("error")) && f.samples.len() < MAX_SAMPLES { f.samples.push(trim(line)); }
    }
    f
}

/// Everything the run measured about the copy.
pub struct Outcome<'a> { pub ready_secs: Option<u64>, pub ready_timeout_s: u64, pub crashed: bool, pub exited_early: bool, pub findings: &'a Findings }

/// The verdict: healthy only when it became ready in time, kept running through the watch period and printed nothing fatal.
/// Returns the reasons it is not healthy (empty = healthy).
pub fn verdict(o: &Outcome) -> Vec<String> {
    let mut why = vec![];
    if o.ready_secs.is_none() { why.push(format!("The server did not finish starting within {} seconds.", o.ready_timeout_s)); }
    if o.crashed { why.push("The server crashed.".into()); }
    else if o.exited_early { why.push("The server stopped on its own before the check finished.".into()); }
    if let Some(f) = &o.findings.fatal { why.push(format!("The log contains a fatal error: {f}")); }
    why
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_levels_across_common_log_formats() {
        let log = "[12:00:01 INFO]: Starting\n[12:00:02 WARN]: Can't keep up!\n[12:00:03 ERROR]: Could not pass event\n\
                   [Server thread/WARN]: x\n[Server thread/ERROR]: y\n12:00:04 [ERROR] z\n[12:00:05] [main/FATAL]: boom\n[12:00:06 INFO]: Done (3.1s)!";
        let f = analyze(log);
        assert_eq!((f.errors, f.warnings), (4, 2));
        assert!(f.fatal.is_none(), "plain error lines are not fatal by themselves");
    }

    #[test]
    fn ansi_colours_do_not_hide_levels() {
        let f = analyze("\x1b[33m[12:00:02 WARN]: slow\x1b[0m\n\x1b[31m[12:00:03 ERROR]: bad\x1b[0m\r\n");
        assert_eq!((f.errors, f.warnings), (1, 1));
        assert_eq!(strip_ansi("\x1b[31mred\x1b[0m"), "red");
    }

    #[test]
    fn stack_frames_are_not_counted_and_fatal_patterns_are_found() {
        let log = "[12:00:03 ERROR]: Error occurred while enabling Foo v1 (Is it up to date?)\njava.lang.NoClassDefFoundError: org/x/Y\n\tat a.b.C.d(C.java:1)\n\tat a.b.E.f(E.java:2)\nCaused by: java.lang.ClassNotFoundException: org.x.Y\n";
        let f = analyze(log);
        assert_eq!(f.errors, 3, "the error line plus the two exception lines, not the frames");
        assert!(f.fatal.as_deref().unwrap().contains("Error occurred while enabling"));
        assert_eq!(f.samples.len(), 3);
    }

    #[test]
    fn samples_are_capped_and_trimmed() {
        let log: String = (0..30).map(|i| format!("[12:00:00 ERROR]: {}\n", "x".repeat(300) + &i.to_string())).collect();
        let f = analyze(&log);
        assert_eq!(f.errors, 30);
        assert_eq!(f.samples.len(), MAX_SAMPLES);
        assert!(f.samples[0].chars().count() <= 221);
    }

    #[test]
    fn a_clean_log_is_healthy_and_each_failure_has_a_reason() {
        let clean = Findings::default();
        assert!(verdict(&Outcome { ready_secs: Some(12), ready_timeout_s: 180, crashed: false, exited_early: false, findings: &clean }).is_empty());
        let never = verdict(&Outcome { ready_secs: None, ready_timeout_s: 180, crashed: false, exited_early: false, findings: &clean });
        assert!(never[0].contains("180 seconds"));
        let crashed = verdict(&Outcome { ready_secs: Some(5), ready_timeout_s: 180, crashed: true, exited_early: true, findings: &clean });
        assert_eq!(crashed, vec!["The server crashed.".to_string()]);
        let bad = analyze("java.lang.OutOfMemoryError: Java heap space");
        let v = verdict(&Outcome { ready_secs: Some(5), ready_timeout_s: 180, crashed: false, exited_early: false, findings: &bad });
        assert!(v[0].contains("OutOfMemoryError"));
    }

    #[test]
    fn warnings_alone_never_fail_a_copy() {
        let f = analyze("[12:00:02 WARN]: a\n[12:00:03 WARN]: b\n");
        assert!(verdict(&Outcome { ready_secs: Some(1), ready_timeout_s: 10, crashed: false, exited_early: false, findings: &f }).is_empty());
    }
}
