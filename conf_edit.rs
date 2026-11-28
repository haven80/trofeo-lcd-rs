//! Editing a `trofeo.conf` text without disturbing it: the settings editor (`trofeo_config`)
//! changes only the lines of the keys the user touches. Comments, blank lines, the order,
//! the line endings and every key the editor does not know stay exactly as they were.
//!
//! * `set` replaces the value of the last active `key = value` line (keeping its trailing
//!   `# comment`); with no active line it adds one right after the commented sample
//!   (`# key = ...`) of that key when there is one, at the end of the file otherwise.
//! * `unset` comments the active line out (`# key = value`), so the old value is not lost.
//!
//! The syntax is the one of `config.rs`: `key = value`, `#` starts a comment, except in front
//! of a hex digit at the start of a value or after a comma (a `#RRGGBB` color).

/// Where the value of a line ends and its trailing comment begins (byte index into `v`).
/// Same rule as `ConfigFile::parse`.
fn comment_start(v: &str) -> Option<usize> {
    let mut prev_sig: Option<char> = None;
    let mut it = v.char_indices().peekable();
    while let Some((i, c)) = it.next() {
        if c == '#' {
            let color_ctx = prev_sig.is_none() || prev_sig == Some(',');
            if !(color_ctx && it.peek().is_some_and(|(_, n)| n.is_ascii_hexdigit())) {
                return Some(i);
            }
        }
        if !c.is_whitespace() {
            prev_sig = Some(c);
        }
    }
    None
}

/// An active `key = value [# comment]` line: (key in lower case, byte range of the value, whole
/// value text without comment). `None` for blank lines, comments and lines without `=`.
fn active(line: &str) -> Option<(String, usize, usize)> {
    let t = line.trim_start();
    if t.is_empty() || t.starts_with('#') {
        return None;
    }
    let eq = line.find('=')?;
    let key = line[..eq].trim().to_ascii_lowercase();
    if key.is_empty() {
        return None;
    }
    let rest = &line[eq + 1..];
    let lead = rest.len() - rest.trim_start().len();
    let start = eq + 1 + lead;
    let end = match comment_start(&line[start..]) {
        Some(c) => start + line[start..start + c].trim_end().len(),
        None => start + line[start..].trim_end().len(),
    };
    Some((key, start, end.max(start)))
}

/// A commented-out sample line: `# key = value` (any number of `#`/spaces before the key).
fn sample_key(line: &str) -> Option<String> {
    let t = line.trim_start();
    let body = t.strip_prefix('#')?.trim_start_matches(|c| c == '#' || c == ' ');
    let eq = body.find('=')?;
    let key = body[..eq].trim();
    if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    Some(key.to_ascii_lowercase())
}

#[derive(Clone, Debug, PartialEq)]
pub struct ConfDoc {
    lines: Vec<String>,
    crlf: bool,
    bom: bool,
    final_newline: bool,
}

const ADDED_HEADER: &str = "# --- Added by the settings editor ---";

impl Default for ConfDoc {
    fn default() -> Self {
        ConfDoc::parse("")
    }
}

impl ConfDoc {
    pub fn parse(text: &str) -> Self {
        let bom = text.starts_with('\u{feff}');
        let text = text.trim_start_matches('\u{feff}');
        let crlf = text.contains("\r\n");
        let final_newline = text.ends_with('\n');
        let lines: Vec<String> = text.lines().map(str::to_string).collect();
        ConfDoc { lines, crlf, bom, final_newline: final_newline || text.is_empty() }
    }

    pub fn to_text(&self) -> String {
        let eol = if self.crlf { "\r\n" } else { "\n" };
        let mut s = String::new();
        if self.bom {
            s.push('\u{feff}');
        }
        s.push_str(&self.lines.join(eol));
        if self.final_newline && !self.lines.is_empty() {
            s.push_str(eol);
        }
        s
    }

    /// The value of `key` as the program reads it (last active line wins, quotes removed).
    pub fn get(&self, key: &str) -> Option<String> {
        let key = key.to_ascii_lowercase();
        self.lines.iter().rev().find_map(|l| {
            let (k, s, e) = active(l)?;
            (k == key).then(|| l[s..e].trim_matches(|c| c == '"' || c == '\'').to_string())
        })
    }

    pub fn is_set(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    /// Gives `key` the value `value` (see the module docs for where the line goes).
    pub fn set(&mut self, key: &str, value: &str) {
        let key_l = key.to_ascii_lowercase();
        let value = value.trim();
        // the last active line of the key
        if let Some(i) = self.lines.iter().rposition(|l| active(l).is_some_and(|(k, _, _)| k == key_l)) {
            let (_, s, e) = active(&self.lines[i]).expect("checked");
            let l = &self.lines[i];
            let new = format!("{}{}{}", &l[..s], value, &l[e..]);
            self.lines[i] = new;
            return;
        }
        let line = format!("{key} = {value}");
        // right after the commented sample of the key (the last one, if there are several)
        if let Some(i) = self.lines.iter().rposition(|l| sample_key(l).is_some_and(|k| k == key_l)) {
            // skip the comment lines that continue the sample ("#   more text")
            self.lines.insert(i + 1, line);
            return;
        }
        // at the end, under a header
        if !self.lines.iter().any(|l| l.trim() == ADDED_HEADER) {
            if self.lines.last().is_some_and(|l| !l.trim().is_empty()) {
                self.lines.push(String::new());
            }
            self.lines.push(ADDED_HEADER.to_string());
        }
        self.lines.push(line);
        self.final_newline = true;
    }

    /// Back to the default: every active line of the key is commented out.
    pub fn unset(&mut self, key: &str) {
        let key_l = key.to_ascii_lowercase();
        for l in self.lines.iter_mut() {
            if active(l).is_some_and(|(k, _, _)| k == key_l) {
                let indent = l.len() - l.trim_start().len();
                *l = format!("{}# {}", &l[..indent], l.trim_start());
            }
        }
    }

    /// Every active key (lower case), in file order, without repeats.
    pub fn keys(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for l in &self.lines {
            if let Some((k, _, _)) = active(l) {
                if !out.contains(&k) {
                    out.push(k);
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "# Copy this file\n\norientation = landscape   # or portrait\nflip = false\n# margin = 0\nmargin_top = 0\n# effect_palette = neon\n#   more text of the sample\n# effect_speed = 30\ncolor_x = #FF0080, #00FFFF  # two colors\n";

    #[test]
    fn reads_values_like_the_program() {
        let d = ConfDoc::parse(SAMPLE);
        assert_eq!(d.get("orientation").as_deref(), Some("landscape"));
        assert_eq!(d.get("ORIENTATION").as_deref(), Some("landscape"));
        assert_eq!(d.get("flip").as_deref(), Some("false"));
        assert_eq!(d.get("margin"), None);
        assert_eq!(d.get("effect_palette"), None);
        assert_eq!(d.get("color_x").as_deref(), Some("#FF0080, #00FFFF"));
        assert_eq!(d.keys(), ["orientation", "flip", "margin_top", "color_x"]);
        // the same answers as the program's own parser
        let c = crate::config::ConfigFile::parse(SAMPLE).unwrap();
        for k in ["orientation", "flip", "margin_top", "color_x", "margin"] {
            assert_eq!(c.get(k).map(str::to_string), d.get(k), "{k}");
        }
    }

    #[test]
    fn round_trip_is_exact() {
        for t in [SAMPLE, "", "a = 1", "a = 1\n", "# only a comment\n", "a = 1\r\nb = 2\r\n", "\u{feff}a = 1\n"] {
            assert_eq!(ConfDoc::parse(t).to_text(), t, "{t:?}");
        }
    }

    #[test]
    fn set_changes_the_value_and_keeps_the_comment() {
        let mut d = ConfDoc::parse(SAMPLE);
        d.set("orientation", "portrait");
        let t = d.to_text();
        assert!(t.contains("orientation = portrait   # or portrait\n"), "{t}");
        assert_eq!(d.get("orientation").as_deref(), Some("portrait"));
        d.set("color_x", "#112233");
        assert!(d.to_text().contains("color_x = #112233  # two colors"));
        // nothing else moved
        let before: Vec<&str> = SAMPLE.lines().collect();
        let after = d.to_text();
        let after: Vec<&str> = after.lines().collect();
        assert_eq!(before.len(), after.len());
        let changed = before.iter().zip(&after).filter(|(a, b)| a != b).count();
        assert_eq!(changed, 2);
    }

    #[test]
    fn a_new_key_goes_after_its_commented_sample() {
        let mut d = ConfDoc::parse(SAMPLE);
        d.set("effect_palette", "ocean");
        let t = d.to_text();
        let lines: Vec<&str> = t.lines().collect();
        let i = lines.iter().position(|l| *l == "effect_palette = ocean").unwrap();
        assert_eq!(lines[i - 1], "# effect_palette = neon");
        assert_eq!(d.get("effect_palette").as_deref(), Some("ocean"));
        d.set("margin", "10");
        assert!(d.to_text().contains("# margin = 0\nmargin = 10\n"));
    }

    #[test]
    fn an_unknown_key_goes_to_the_end_under_a_header() {
        let mut d = ConfDoc::parse("a = 1\n");
        d.set("brand_new", "yes");
        d.set("other", "2");
        assert_eq!(d.to_text(), format!("a = 1\n\n{ADDED_HEADER}\nbrand_new = yes\nother = 2\n"));
        let mut e = ConfDoc::parse("");
        e.set("a", "1");
        assert_eq!(e.to_text(), format!("{ADDED_HEADER}\na = 1\n"));
    }

    #[test]
    fn unset_comments_the_line_out() {
        let mut d = ConfDoc::parse(SAMPLE);
        d.unset("orientation");
        assert!(d.to_text().contains("# orientation = landscape   # or portrait"));
        assert_eq!(d.get("orientation"), None);
        d.unset("not_there"); // harmless
        // setting it again reuses the commented line's place
        d.set("orientation", "portrait");
        assert!(d.to_text().contains("# orientation = landscape   # or portrait\norientation = portrait\n"));
    }

    #[test]
    fn duplicates_follow_the_last_one() {
        let mut d = ConfDoc::parse("a = 1\nb = 2\na = 3\n");
        assert_eq!(d.get("a").as_deref(), Some("3"));
        d.set("a", "9");
        assert_eq!(d.to_text(), "a = 1\nb = 2\na = 9\n");
        d.unset("a");
        assert_eq!(d.to_text(), "# a = 1\nb = 2\n# a = 9\n");
        assert_eq!(d.get("a"), None);
    }

    #[test]
    fn crlf_and_bom_survive_edits() {
        let mut d = ConfDoc::parse("\u{feff}a = 1\r\n# b = 2\r\n");
        d.set("b", "5");
        d.set("c", "7");
        assert_eq!(d.to_text(), format!("\u{feff}a = 1\r\n# b = 2\r\nb = 5\r\n\r\n{ADDED_HEADER}\r\nc = 7\r\n"));
    }

    #[test]
    fn values_with_hash_colors_and_quotes() {
        let mut d = ConfDoc::parse("c = #00FF00\nq = \"two words\" # quoted\nl = #FF0080,#00FFFF\n");
        assert_eq!(d.get("c").as_deref(), Some("#00FF00"));
        d.set("l", "#111111, #222222");
        assert_eq!(d.get("l").as_deref(), Some("#111111, #222222"));
        let cfg = crate::config::ConfigFile::parse(&d.to_text()).unwrap();
        assert_eq!(cfg.get("l"), Some("#111111, #222222"));
        assert_eq!(cfg.get("c"), Some("#00FF00"));
        assert_eq!(cfg.get("q"), Some("two words"));
        assert_eq!(d.get("q").as_deref(), Some("two words"));
    }

    #[test]
    fn edited_text_is_still_a_valid_config() {
        let mut d = ConfDoc::parse(SAMPLE);
        d.set("effect_palette", "ocean");
        d.set("effect_speed", "55");
        d.set("margin", "12");
        d.unset("flip");
        let c = crate::config::ConfigFile::parse(&d.to_text()).unwrap();
        assert_eq!(c.get("effect_speed"), Some("55"));
        assert_eq!(c.get("flip"), None);
        assert_eq!(c.get("margin_top"), Some("0"));
    }
}
