//! Content classification. Built-in heuristics for now; user rules plug in here later.
use regex::Regex;
use std::sync::OnceLock;

pub const TEXT: &str = "text";
pub const CODE: &str = "code";
pub const IMAGE: &str = "image";
pub const FILE: &str = "file";
pub const LINK: &str = "link";

fn url_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^(https?|ftp)://\S+$").unwrap())
}

fn kw_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"\b(fn|let|const|var|def|class|import|from|return|function|public|private|struct|impl|interface|SELECT|INSERT|UPDATE|#include|package|async|await)\b",
        )
        .unwrap()
    })
}

pub fn classify_text(s: &str) -> &'static str {
    let t = s.trim();
    if url_re().is_match(t) {
        return LINK;
    }
    if looks_like_code(t) {
        return CODE;
    }
    TEXT
}

fn looks_like_code(t: &str) -> bool {
    let lines: Vec<&str> = t.lines().collect();
    if t.len() < 8 {
        return false;
    }
    let sym = t.chars().filter(|c| matches!(c, '{' | '}' | ';' | '(' | ')' | '=' | '<' | '>' | '[' | ']')).count();
    let density = sym as f32 / t.chars().count() as f32;
    let kws = kw_re().find_iter(t).count();
    let indented = lines.iter().filter(|l| l.starts_with("    ") || l.starts_with('\t')).count();
    let mut score = 0;
    if density > 0.04 {
        score += 1;
    }
    if kws >= 1 {
        score += 1;
    }
    if kws >= 3 {
        score += 1;
    }
    if lines.len() > 1 && indented * 3 >= lines.len() {
        score += 1;
    }
    if t.ends_with(';') || t.ends_with('{') || t.ends_with('}') {
        score += 1;
    }
    score >= 3 || (lines.len() == 1 && score >= 2 && kws >= 1 && density > 0.04)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link() {
        assert_eq!(classify_text("https://example.com/a?b=1"), LINK);
    }

    #[test]
    fn code() {
        assert_eq!(classify_text("fn main() {\n    let x = 1;\n    println!(\"{}\", x);\n}"), CODE);
        assert_eq!(classify_text("const a = (b) => { return b; };"), CODE);
    }

    #[test]
    fn plain() {
        assert_eq!(classify_text("Meeting moved to 3pm tomorrow, bring the notes."), TEXT);
    }
}
