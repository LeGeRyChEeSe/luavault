//! Which upstream SteamTools asks for depot manifest request codes.
//!
//! Steam refuses a manifest request code for a game the account does not own
//! (`Failed to get manifest request code, 'Access Denied'`); OpenSteamTool hooks that
//! request and fetches the code from an upstream named by `[manifest] url` in
//! `<Steam>\opensteamtool.toml`. When that upstream does not answer, Steam says
//! "No internet connection" and retries by itself every ~35 s, which is why an install
//! fails a few times and then goes through. Measured on 2026-10-19: one upstream
//! answered 502 to most requests while another answered every one in 0.3 s, and the
//! change is read live (no Steam restart).
//!
//! This module only reads and edits that one value, **keeping every other line of the
//! file** (injection, stats, timeouts are the user's). The set of names is closed and
//! lives here: a signed message picks among them, it never supplies a URL.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

/// The upstreams OpenSteamTool documents for `[manifest] url`. `opensteamtool` is its
/// default when the file or the key is absent.
pub const SOURCES: [&str; 3] = ["opensteamtool", "steamrun", "wudrm"];
pub const DEFAULT_SOURCE: &str = "opensteamtool";

pub fn is_known(source: &str) -> bool {
    SOURCES.contains(&source)
}

pub fn config_path(steam_dir: &Path) -> PathBuf {
    steam_dir.join("opensteamtool.toml")
}

/// `[name]` on its own (a comment may follow), the table name.
fn table_name(line: &str) -> Option<&str> {
    let t = line.trim();
    let inner = t.strip_prefix('[')?;
    let end = inner.find(']')?;
    if inner[end + 1..].trim_start().chars().next().is_some_and(|c| c != '#') {
        return None;
    }
    Some(inner[..end].trim())
}

/// `url = "value"` (single or double quotes, a trailing comment allowed).
fn url_value(line: &str) -> Option<String> {
    let t = line.trim_start();
    let rest = t.strip_prefix("url")?;
    if rest.chars().next().is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '-') {
        return None;
    }
    let rest = rest.trim_start().strip_prefix('=')?.trim_start();
    let quote = rest.chars().next().filter(|c| *c == '"' || *c == '\'')?;
    let body = &rest[1..];
    let end = body.find(quote)?;
    Some(body[..end].to_string())
}

/// The configured upstream, `None` when the file has no `[manifest] url`.
pub fn parse_source(text: &str) -> Option<String> {
    let mut in_manifest = false;
    for line in text.lines() {
        if let Some(name) = table_name(line) {
            in_manifest = name == "manifest";
            continue;
        }
        if in_manifest {
            if let Some(v) = url_value(line) {
                return Some(v);
            }
        }
    }
    None
}

/// `text` with `[manifest] url` set to `source`, everything else untouched, the file's
/// own line endings kept (CRLF by default for a new file, like the Windows tooling).
pub fn with_source(text: &str, source: &str) -> String {
    let nl = if text.contains("\r\n") || text.is_empty() { "\r\n" } else { "\n" };
    let new_line = format!("url = \"{source}\"");
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();

    let mut in_manifest = false;
    let mut header_at: Option<usize> = None;
    let mut url_at: Option<usize> = None;
    for (i, line) in lines.iter().enumerate() {
        if let Some(name) = table_name(line) {
            in_manifest = name == "manifest";
            if in_manifest && header_at.is_none() {
                header_at = Some(i);
            }
            continue;
        }
        if in_manifest && url_at.is_none() && url_value(line).is_some() {
            url_at = Some(i);
        }
    }

    match (url_at, header_at) {
        (Some(i), _) => lines[i] = new_line,
        (None, Some(h)) => lines.insert(h + 1, new_line),
        (None, None) => {
            if lines.last().is_some_and(|l| !l.trim().is_empty()) {
                lines.push(String::new());
            }
            lines.push("[manifest]".to_string());
            lines.push(new_line);
        }
    }
    let mut out = lines.join(nl);
    out.push_str(nl);
    out
}

/// What is configured right now (`DEFAULT_SOURCE` when nothing is written down).
pub fn current(steam_dir: &Path) -> String {
    std::fs::read_to_string(config_path(steam_dir))
        .ok()
        .and_then(|t| parse_source(&t))
        .unwrap_or_else(|| DEFAULT_SOURCE.to_string())
}

/// Write the new upstream. The previous file is kept once as `opensteamtool.toml.bak`
/// (never overwritten), and the new content goes through a temp file and a rename so a
/// crash cannot leave a half-written config for SteamTools to read.
pub fn apply(steam_dir: &Path, source: &str) -> Result<()> {
    if !is_known(source) {
        bail!("source de manifestes inconnue : {source}");
    }
    let path = config_path(steam_dir);
    let existing = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e).context("lecture de opensteamtool.toml"),
    };
    let updated = with_source(&existing, source);
    if updated == existing {
        return Ok(());
    }
    if !existing.is_empty() {
        let bak = path.with_extension("toml.bak");
        if !bak.exists() {
            std::fs::write(&bak, &existing).context("sauvegarde de opensteamtool.toml")?;
        }
    }
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, updated.as_bytes()).context("écriture de opensteamtool.toml")?;
    std::fs::rename(&tmp, &path).context("remplacement de opensteamtool.toml")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("steamtools-source-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn the_names_are_the_documented_upstreams() {
        for s in ["opensteamtool", "steamrun", "wudrm"] {
            assert!(is_known(s));
        }
        for s in ["", "WUDRM", "wudrm ", "http://x", "steam.run"] {
            assert!(!is_known(s), "{s:?}");
        }
    }

    #[test]
    fn parse_reads_only_the_manifest_table() {
        assert_eq!(parse_source("[manifest]\r\nurl = \"steamrun\"\r\n").as_deref(), Some("steamrun"));
        assert_eq!(parse_source("[manifest]\nurl='wudrm'  # why\n").as_deref(), Some("wudrm"));
        assert_eq!(parse_source("[manifest]   # t\nurl=\"wudrm\"").as_deref(), Some("wudrm"));
        // A `url` key in another table is not ours: the real file has [stats] and others.
        assert_eq!(parse_source("[stats]\nurl = \"nope\"\n[manifest]\ntimeout_recv_ms = 1\n"), None);
        assert_eq!(parse_source("url = \"top-level\"\n[manifest]\n"), None, "before any table");
        assert_eq!(parse_source("[manifest]\nurls = \"x\"\nurl_x = \"y\"\n"), None, "other key names");
        assert_eq!(parse_source(""), None);
        assert_eq!(parse_source("[manifest]\nurl = \"broken\n"), None, "unterminated string");
    }

    #[test]
    fn with_source_replaces_inserts_or_appends_and_keeps_the_rest() {
        let full = "[log]\r\nlevel = \"info\"\r\n\r\n[manifest]\r\nurl = \"steamrun\"\r\ntimeout_recv_ms = 10000\r\n\r\n[stats]\r\nurl = \"keep-me\"\r\n";
        let out = with_source(full, "wudrm");
        assert_eq!(parse_source(&out).as_deref(), Some("wudrm"));
        assert!(out.contains("level = \"info\"") && out.contains("timeout_recv_ms = 10000"));
        assert!(out.contains("[stats]\r\nurl = \"keep-me\""), "another table's url is untouched");
        assert_eq!(out.matches("steamrun").count(), 0);

        // Section without the key: the key goes right under the header.
        let no_key = with_source("[manifest]\ntimeout_recv_ms = 5\n", "wudrm");
        assert_eq!(no_key, "[manifest]\nurl = \"wudrm\"\ntimeout_recv_ms = 5\n");
        // No section at all: appended after a blank line, other content kept.
        let none = with_source("[log]\nlevel = \"info\"\n", "wudrm");
        assert_eq!(none, "[log]\nlevel = \"info\"\n\n[manifest]\nurl = \"wudrm\"\n");
        // A new file: what the Windows tooling writes (CRLF).
        assert_eq!(with_source("", "wudrm"), "[manifest]\r\nurl = \"wudrm\"\r\n");
    }

    #[test]
    fn with_source_is_idempotent_and_keeps_line_endings() {
        let lf = "[manifest]\nurl = \"steamrun\"\n";
        let once = with_source(lf, "wudrm");
        assert_eq!(once, "[manifest]\nurl = \"wudrm\"\n", "LF stays LF");
        assert_eq!(with_source(&once, "wudrm"), once);
        let crlf = "[manifest]\r\nurl = \"steamrun\"\r\n";
        assert_eq!(with_source(crlf, "wudrm"), "[manifest]\r\nurl = \"wudrm\"\r\n");
    }

    #[test]
    fn apply_backs_up_once_and_leaves_no_temp() {
        let d = scratch("apply");
        std::fs::write(config_path(&d), "[manifest]\r\nurl = \"steamrun\"\r\n[inject]\r\nenabled = false\r\n").unwrap();
        apply(&d, "wudrm").unwrap();
        let text = std::fs::read_to_string(config_path(&d)).unwrap();
        assert_eq!(parse_source(&text).as_deref(), Some("wudrm"));
        assert!(text.contains("[inject]\r\nenabled = false"));
        let bak = std::fs::read_to_string(d.join("opensteamtool.toml.bak")).unwrap();
        assert!(bak.contains("steamrun"), "the previous file is kept");
        assert!(!d.join("opensteamtool.toml.tmp").exists());

        // The first backup is the true original: a second change must not overwrite it.
        apply(&d, "opensteamtool").unwrap();
        assert!(std::fs::read_to_string(d.join("opensteamtool.toml.bak")).unwrap().contains("steamrun"));
        assert_eq!(current(&d), "opensteamtool");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn apply_refuses_an_unknown_name_and_writes_nothing() {
        let d = scratch("refuse");
        assert!(apply(&d, "https://evil.example/").is_err());
        assert!(apply(&d, "").is_err());
        assert!(!config_path(&d).exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn current_defaults_to_the_documented_default() {
        let d = scratch("default");
        assert_eq!(current(&d), DEFAULT_SOURCE, "no file");
        std::fs::write(config_path(&d), "[log]\nlevel = \"info\"\n").unwrap();
        assert_eq!(current(&d), DEFAULT_SOURCE, "no [manifest] url");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn apply_on_an_unchanged_file_touches_nothing() {
        let d = scratch("same");
        std::fs::write(config_path(&d), "[manifest]\r\nurl = \"wudrm\"\r\n").unwrap();
        apply(&d, "wudrm").unwrap();
        assert!(!d.join("opensteamtool.toml.bak").exists(), "no backup for a no-op");
        let _ = std::fs::remove_dir_all(&d);
    }
}
