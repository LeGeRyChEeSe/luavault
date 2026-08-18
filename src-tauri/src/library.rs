//! Library management: index.json, .lua files, fix archives, and Steam sync.
//!
//! `load_index` is cached with a [`FileStamp`] so that the next read after a
//! save always returns the freshly written data without re-parsing the file.
//! The 2-second settle window (see [`crate::cache::StampedCache`]) intentionally
//! suppresses the index cache during rapid `load_index` → `save_index` sequences
//! (bulk installs), trading a few extra disk reads for correctness — the real
//! gain comes from `MANIFEST_CACHE` in `vdf.rs`.
//!
//! HMAC integrity: every writer calls the HMAC core before modifying anything.
//! On integrity failure the writer returns `Err` without touching index, sidecar,
//! `.lua`, fix, or deletion targets.

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use crate::cache::{FileStamp, StampedCache};
use crate::config;
use crate::hmac;
use crate::i18n_log;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LibraryEntry {
    pub app_id: String,
    pub name: String,
    #[serde(default)]
    pub icon: Option<String>,
    pub file_name: String,
    pub added_at: String,
    pub updated_at: String,
    /// The online-fix archive has been downloaded into the library.
    #[serde(default)]
    pub has_fix: bool,
    /// Hidden from the library view. Nothing is deleted — the entry simply
    /// stops showing up until the user reveals it again in the settings.
    #[serde(default)]
    pub hidden: bool,
    /// User-defined tags for categorising games.
    #[serde(default)]
    pub tags: Vec<String>,
}

fn index_path(lib: &Path) -> PathBuf {
    lib.join("index.json")
}

// Cache for load_index — keyed by the index.json path, validated by file stamp.
static INDEX_CACHE: LazyLock<StampedCache<PathBuf, Vec<LibraryEntry>>> =
    LazyLock::new(StampedCache::new);

/// Clear the library index cache. Used by tests to avoid cross-test pollution.
pub fn clear_index_cache() {
    INDEX_CACHE.clear();
    // The settle window is process-wide state too: a test that shortens it
    // must not leak that into the tests that follow.
    #[cfg(test)]
    INDEX_CACHE.reset_settle();
}

/// Process-wide serialisation for every test that touches `INDEX_CACHE` —
/// the cache is a static and Rust runs tests on parallel threads. Library,
/// discover and wipe tests all take this lock.
#[cfg(test)]
static CACHE_TEST_GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
pub(crate) fn cache_test_lock() -> std::sync::MutexGuard<'static, ()> {
    CACHE_TEST_GUARD
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
fn set_index_settle(n: u64) {
    INDEX_CACHE.set_settle_nanos(n);
}

pub fn lua_file_name(app_id: &str) -> String {
    format!("{app_id}.lua")
}

/// Read-only view of a .lua: which AppIDs it declares, and when the server
/// generated it. We never write to this file — the depot keys inside it can
/// only come from LuaVault, so editing it locally could only break it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LuaContents {
    /// Every AppID passed to `addappid` — main application, DLCs and content
    /// depots alike.
    pub app_ids: Vec<String>,
    /// AppIDs declared in the DLC sections ("-- DLCs with Content",
    /// "-- DLCs without Dedicated Depots"). Content depots and the main
    /// application are excluded: a .lua always declares its depots
    /// (Windows/macOS/Linux), so counting them as DLCs would show the
    /// section for games that have none. A file without recognised section
    /// headers yields an empty list — when the file doesn't say, we don't
    /// assert.
    pub dlc_app_ids: Vec<String>,
    /// Server generation date from the header comment ("2026-01-01 05:55",
    /// UTC), when the file carries one.
    pub generated_at: Option<String>,
}

/// Which section of the file the current line sits in. The server generates
/// headed sections; anything before (or without) a header counts as the main
/// application, never as DLC.
#[derive(Clone, Copy)]
enum LuaSection {
    /// "-- Content Depots" / "-- Shared Depots": depot declarations, not DLCs.
    Depots,
    /// "-- DLCs with Content" / "-- DLCs without Dedicated Depots".
    Dlcs,
    /// An unrecognised header: we assert nothing — its AppIDs feed the
    /// global list but never `dlc_app_ids`.
    Other,
}

/// The count the server writes in a DLC section header:
/// `-- DLCs with Content (3)` → `Some(3)`. Absent or unparseable → `None`,
/// which means "take every AppID in the section" (over-count, never lose).
fn parse_section_count(header: &str) -> Option<usize> {
    let open = header.rfind('(')?;
    let rest = &header[open + 1..];
    let close = rest.find(')')?;
    rest[..close].trim().parse().ok()
}

/// Section a full-line comment opens. Unrecognised headers yield `Other`:
/// an unknown section is never treated as DLC (D5 — `-- Shared Depots`
/// placed after a DLC block used to inherit the DLC section).
fn section_after_header(comment: &str) -> (LuaSection, Option<usize>) {
    // Zero-width characters are stripped before matching: the obfuscation
    // that hits titles could hit section headers too.
    let clean: String = comment.chars().filter(|c| !is_zero_width(*c)).collect();
    let clean = clean.trim();
    if clean.starts_with("-- Content Depots") || clean.starts_with("-- Shared Depots") {
        (LuaSection::Depots, None)
    } else if clean.starts_with("-- DLCs with Content")
        || clean.starts_with("-- DLCs without Dedicated Depots")
    {
        (LuaSection::Dlcs, parse_section_count(clean))
    } else {
        (LuaSection::Other, None)
    }
}

/// Zero-width code points the server embeds in the header line (title
/// obfuscation). They carry no meaning for us — strip them wherever they
/// would pollute an extracted value.
fn is_zero_width(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}' | '\u{200B}' | '\u{200C}' | '\u{200D}' | '\u{2060}' | '\u{FEFF}'
    )
}

/// The AppIDs one line of .lua declares. Linear even on a delimiter-free
/// line: the scan resumes after each extracted token and never re-walks it
/// (the old `take_while`-from-each-occurrence version went quadratic on a
/// corrupted single-line file — seconds at a few hundred KB).
///
/// A single left-to-right pass tracks strings (with `\"` / `\'` escapes),
/// Lua long strings (`[[ … ]]`), and line comments (`-- …`) in the correct
/// order — truncating at `--` before tracking strings made `"a--b"` end the
/// line early, and ignoring escapes made `"a\"b"` swallow the rest.
/// `.` and `:` before `addappid(` are identifier boundaries (`t.addappid(5)`
/// is a method call on `t`, not a declaration).
fn line_app_ids(line: &str, out: &mut Vec<String>) {
    const CALL: &str = "addappid(";
    let bytes = line.as_bytes();
    let mut quote: Option<u8> = None;
    let mut in_long_string = false;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];

        // Inside a long string [[ … ]]: only ]] ends it.
        if in_long_string {
            if b == b']' && bytes.get(i + 1) == Some(&b']') {
                in_long_string = false;
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }

        // Inside a regular string: honour backslash escapes.
        if let Some(q) = quote {
            if b == b'\\' {
                i += 2; // skip the escaped character
                continue;
            }
            if b == q {
                quote = None;
            }
            i += 1;
            continue;
        }

        // Outside any string.
        // Line comment: the rest is dead code.
        if b == b'-' && bytes.get(i + 1) == Some(&b'-') {
            break;
        }
        // Long string opening.
        if b == b'[' && bytes.get(i + 1) == Some(&b'[') {
            in_long_string = true;
            i += 2;
            continue;
        }
        if b == b'"' || b == b'\'' {
            quote = Some(b);
            i += 1;
            continue;
        }

        // `bytes[i] == b'a'` is ASCII, so `i` and `i + CALL.len()` sit on
        // char boundaries; UTF-8 continuation bytes never match an ASCII
        // byte, which is what makes the byte scan safe.
        if b == b'a' && line[i..].starts_with(CALL) {
            // A multi-byte character before the call leaves a continuation
            // byte in `bytes[i - 1]`, which fails both checks: a zero-width
            // code point is a valid boundary, an identifier character isn't.
            // `.` and `:` are boundaries too: `t.addappid(5)` is a method
            // call, not a free declaration.
            let boundary = i == 0
                || !(bytes[i - 1].is_ascii_alphanumeric()
                    || bytes[i - 1] == b'_'
                    || bytes[i - 1] == b'.'
                    || bytes[i - 1] == b':');
            if boundary {
                let start = i + CALL.len();
                let mut end = start;
                while end < bytes.len() && !matches!(bytes[end], b',' | b')') {
                    end += 1;
                }
                // `end` stopped on an ASCII delimiter or the line's end:
                // both slice points are char boundaries.
                let token: String = line[start..end]
                    .chars()
                    .filter(|c| !is_zero_width(*c))
                    .collect();
                let token = token.trim();
                if !token.is_empty() && token.chars().all(|c| c.is_ascii_digit()) {
                    out.push(token.to_string());
                }
                i = end; // resume after the token — one walk per line
                continue;
            }
        }
        i += 1;
    }
}

/// Parse a .lua's declarations without ever returning an error: a file we
/// can't make sense of is still a working .lua, so the view simply asserts
/// nothing (empty lists, no date). Commented-out calls don't count, and the
/// header line's zero-width characters are survived, not assumed away.
pub fn parse_lua(text: &str) -> LuaContents {
    // Header: "-- AppID … | Generated on 2026-01-01 05:55 UTC | LuaVault".
    // Bound the search to the line carrying the marker BEFORE splitting: a
    // header without " UTC" would hand the rest of the file to `split`.
    const MARKER: &str = "Generated on ";
    let generated_at = text
        .lines()
        .find_map(|line| line.find(MARKER).map(|start| &line[start + MARKER.len()..]))
        .map(|rest| rest.split(" UTC").next().unwrap_or(""))
        .map(|rest| rest.split('|').next().unwrap_or(""))
        .map(|raw| raw.chars().filter(|c| !is_zero_width(*c)).collect::<String>())
        .map(|date| date.trim().to_string())
        .filter(|date| !date.is_empty());

    let mut app_ids = Vec::new();
    let mut dlc_app_ids = Vec::new();
    let mut section = LuaSection::Other;
    // DLCs the header's count still allows in the current section (D4).
    // `None` = no count parsed → take everything (over-count, never lose).
    let mut dlc_cap: Option<usize> = None;
    let mut dlc_taken: usize = 0;
    // Multi-line block comment `--[[ … ]]` (D6).
    let mut in_block_comment = false;

    for line in text.lines() {
        // Inside a block comment: look for the closing ]].
        if in_block_comment {
            if let Some(pos) = line.find("]]") {
                in_block_comment = false;
                // Process the remainder after ]] on the same line.
                let rest = &line[pos + 2..];
                let before = app_ids.len();
                line_app_ids(rest, &mut app_ids);
                if matches!(section, LuaSection::Dlcs) {
                    let new_ids = &app_ids[before..];
                    let allowed = dlc_cap.map_or(new_ids.len(), |c| c.saturating_sub(dlc_taken));
                    let take = new_ids.len().min(allowed);
                    dlc_app_ids.extend(new_ids[..take].iter().cloned());
                    dlc_taken += take;
                }
            }
            continue;
        }

        let trimmed = line.trim();
        if trimmed.starts_with("--") {
            // Block comment opening: --[[ … (may close on the same line).
            if let Some(rest) = trimmed.strip_prefix("--[[") {
                if !rest.contains("]]") {
                    in_block_comment = true;
                }
                continue;
            }
            let (new_section, count) = section_after_header(trimmed);
            section = new_section;
            dlc_cap = count;
            dlc_taken = 0;
            continue;
        }
        let before = app_ids.len();
        line_app_ids(line, &mut app_ids);
        if matches!(section, LuaSection::Dlcs) {
            let new_ids = &app_ids[before..];
            let allowed = dlc_cap.map_or(new_ids.len(), |c| c.saturating_sub(dlc_taken));
            let take = new_ids.len().min(allowed);
            dlc_app_ids.extend(new_ids[..take].iter().cloned());
            dlc_taken += take;
        }
    }

    LuaContents {
        app_ids,
        dlc_app_ids,
        generated_at,
    }
}

/// Read the library's .lua for `app_id` and parse it. A missing, locked or
/// binary file yields an empty view, never an error — see [`parse_lua`].
///
/// This read-only convenience is exercised by the module's disk-level tests;
/// no production command exposes parsed Lua contents at present.
#[cfg_attr(not(test), allow(dead_code))]
pub fn read_lua_contents(lib: &Path, app_id: &str) -> LuaContents {
    match std::fs::read_to_string(lib.join(lua_file_name(app_id))) {
        Ok(text) => parse_lua(&text),
        Err(_) => LuaContents::default(),
    }
}

// ================================================================ Strict load with data_dir injection

/// Strict load with HMAC verification and injected `data_dir`.
///
/// Contract:
/// - index absent → `Ok([])`, no key, no sidecar, no cache entry;
/// - key absent + sidecar absent + JSON array valid → migration: create the
///   key and sign the exact bytes — not one byte of the JSON is rewritten —
///   then parse/cache/return. JSON invalid → error, **no key created**;
/// - key absent + sidecar present → error + cache invalidation by path;
/// - key present + sidecar absent → error + cache invalidation by path;
/// - key file malformed → error;
/// - sidecar malformed (magic, length, case, separator) → error, distinct
///   from a wrong tag;
/// - HMAC mismatch → error + cache invalidation by path;
/// - the two-file verification precedes **any** cache hit; no failure ever
///   enters the cache.
///
/// Cache is keyed by `(index_path, FileStamp)` — two distinct files can share
/// a stamp but never a path, so the path is the primary key.
pub fn load_index_with_data_dir(lib: &Path, data_dir: &Path) -> Result<Vec<LibraryEntry>> {
    // Held for the whole function, migration included: the read path writes
    // too (it signs a legitimate unsigned index), so releasing the guard
    // mid-way would reopen the very window this closes.
    let _guard = hmac::INDEX_GUARD
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    let idx = hmac::index_path(lib);

    // Index absent → empty, no sidecar/key required.
    if !idx.exists() {
        return Ok(Vec::new());
    }

    let key_exists = hmac::has_key(data_dir);
    let sidecar_exists = hmac::has_sidecar(&idx);

    match (key_exists, sidecar_exists) {
        (false, false) => {
            // Migration. Parse BEFORE creating anything: invalid JSON must
            // not leave a key behind.
            let raw = std::fs::read(&idx).context("read index.json")?;
            let entries: Vec<LibraryEntry> =
                serde_json::from_slice(&raw).context("parse index.json as JSON array")?;
            let key = hmac::load_or_create_key(data_dir).context("create HMAC key")?;
            // Sign the exact bytes — the JSON itself is never rewritten.
            let tag = hmac::sign_bytes(&key, &raw);
            hmac::write_sidecar(&idx, &tag).context("write index.json sidecar")?;
            if let Some(stamp) = FileStamp::from_path(&idx) {
                INDEX_CACHE.put(idx.clone(), stamp, entries.clone());
            }
            Ok(entries)
        }
        (false, true) => {
            INDEX_CACHE.invalidate(&idx);
            Err(anyhow!(
                "sidecar index.json.hmac présente mais clé HMAC absente ({})",
                data_dir.display()
            ))
        }
        (true, false) => {
            INDEX_CACHE.invalidate(&idx);
            // A malformed key file is an error too — check it first.
            hmac::load_or_create_key(data_dir).context("load HMAC key")?;
            Err(anyhow!(
                "clé HMAC présente mais sidecar index.json.hmac absente : intégrité de index.json invérifiable"
            ))
        }
        (true, true) => {
            let key = hmac::load_or_create_key(data_dir).context("load HMAC key")?;
            let raw = std::fs::read(&idx).context("read index.json")?;
            let stored = match hmac::read_sidecar(&idx) {
                Ok(Some(tag)) => tag,
                Ok(None) => {
                    INDEX_CACHE.invalidate(&idx);
                    return Err(anyhow!("sidecar index.json.hmac vide"));
                }
                Err(e) => {
                    INDEX_CACHE.invalidate(&idx);
                    return Err(e).context("sidecar index.json.hmac mal formée");
                }
            };
            if !hmac::verify_tag(&key, &raw, &stored) {
                INDEX_CACHE.invalidate(&idx);
                return Err(anyhow!(
                    "HMAC de index.json invalide : le fichier a été modifié hors de l'application"
                ));
            }

            // Integrity verified BEFORE any cache hit.
            let stamp = FileStamp::from_path(&idx);
            if let Some(ref s) = stamp {
                if let Some(cached) = INDEX_CACHE.get(&idx, s) {
                    return Ok(cached);
                }
            }

            let entries: Vec<LibraryEntry> =
                serde_json::from_slice(&raw).context("parse index.json as JSON array")?;
            if let Some(s) = stamp {
                INDEX_CACHE.put(idx.clone(), s, entries.clone());
            }
            Ok(entries)
        }
    }
}

// ================================================================ load_index (best-effort wrapper)

/// Best-effort entry point for production callers: the strict HMAC path,
/// and on any integrity or parse failure the library is treated as empty.
/// Raw JSON is never parsed after an error — a damaged or tampered index
/// is ignored wholesale, never trusted.
pub fn load_index(lib: &Path) -> Vec<LibraryEntry> {
    load_index_best_effort_with_data_dir(lib, &config::data_dir())
}

/// Injectable best-effort wrapper around the strict load. This is the exact
/// code path production uses (`load_index` only supplies `config::data_dir()`),
/// exposed so tests can pin the "never parse raw JSON after an integrity
/// error" rule without touching the real application folder.
pub fn load_index_best_effort_with_data_dir(lib: &Path, data_dir: &Path) -> Vec<LibraryEntry> {
    match load_index_with_data_dir(lib, data_dir) {
        Ok(entries) => entries,
        Err(e) => {
            log::warn!("{}", i18n_log::i18n_log(format!("index de la bibliothèque ignoré: {e:#}"), "logs.library.index-ignored", &[("error", serde_json::json!(format!("{e:#}")))]));
            Vec::new()
        }
    }
}

// ================================================================ save_index (HMAC-aware)

/// Save entries to `lib/index.json` and sign them.
///
/// 1. Serialises entries to JSON.
/// 2. Writes JSON atomically via the HMAC core.
/// 3. Signs the written bytes.
/// 4. Updates cache only after both succeed.
/// 5. Invalidates cache on any error.
pub fn save_index_with_data_dir(lib: &Path, data_dir: &Path, entries: &[LibraryEntry]) -> Result<()> {
    let raw = serde_json::to_vec_pretty(entries).context("serialize index")?;
    hmac::save_index_with_data_dir(lib, data_dir, &raw)
        .context("save index.json")?;
    // Update cache only after both succeed.
    let stamp = FileStamp::from_path(&index_path(lib));
    if let Some(s) = stamp {
        INDEX_CACHE.put(index_path(lib).clone(), s, entries.to_vec());
    }
    Ok(())
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

// ================================================================ Writers with data_dir injection

/// Write the .lua bytes into the library and update the index, with an
/// injected `data_dir` for test isolation.
pub fn upsert_with_data_dir(
    lib: &Path,
    data_dir: &Path,
    app_id: &str,
    name: &str,
    icon: Option<&str>,
    lua_bytes: &[u8],
) -> Result<LibraryEntry> {
    // Strict load — fails on HMAC mismatch, preventing any write.
    let entries = load_index_with_data_dir(lib, data_dir)
        .context("integrity check before upsert")?;

    upsert_verified_index_with_data_dir(lib, data_dir, entries, app_id, name, icon, lua_bytes)
}

/// Write a `.lua` file and update an index that the caller has just loaded
/// through [`load_index_with_data_dir`].
///
/// This is deliberately crate-private: callers must not manufacture the
/// entries, because the preceding strict load is what enforces the index HMAC
/// before a writer changes either file.
pub(crate) fn upsert_verified_index_with_data_dir(
    lib: &Path,
    data_dir: &Path,
    mut entries: Vec<LibraryEntry>,
    app_id: &str,
    name: &str,
    icon: Option<&str>,
    lua_bytes: &[u8],
) -> Result<LibraryEntry> {
    std::fs::create_dir_all(lib).context("create library dir")?;
    let file_name = lua_file_name(app_id);
    std::fs::write(lib.join(&file_name), lua_bytes).context("write .lua file")?;

    let now = now_rfc3339();
    let entry = match entries.iter_mut().find(|e| e.app_id == app_id) {
        Some(existing) => {
            existing.name = name.to_string();
            if icon.is_some() {
                existing.icon = icon.map(str::to_string);
            }
            existing.updated_at = now;
            existing.clone()
        }
        None => {
            let entry = LibraryEntry {
                app_id: app_id.to_string(),
                name: name.to_string(),
                icon: icon.map(str::to_string),
                file_name: file_name.clone(),
                added_at: now.clone(),
                updated_at: now,
                has_fix: false,
                hidden: false,
                tags: Vec::new(),
            };
            entries.push(entry.clone());
            entry
        }
    };
    save_index_with_data_dir(lib, data_dir, &entries)?;
    Ok(entry)
}

/// Record a fix archive in a test library while preserving the HMAC invariant.
///
/// This writer is only exercised by integrity tests: online-fix downloads no
/// longer have a production caller after their removal from the public UI.
#[cfg_attr(not(test), allow(dead_code))]
pub fn mark_fix_with_data_dir(lib: &Path, data_dir: &Path, app_id: &str, fix_bytes: &[u8]) -> Result<PathBuf> {
    // Strict load — fails on HMAC mismatch.
    let mut entries = load_index_with_data_dir(lib, data_dir)
        .context("integrity check before mark_fix")?;

    let fixes = lib.join("fixes");
    std::fs::create_dir_all(&fixes).context("create fixes dir")?;
    let out = fixes.join(format!("{app_id}_online_fix.rar"));
    std::fs::write(&out, fix_bytes).context("write fix archive")?;

    if let Some(entry) = entries.iter_mut().find(|e| e.app_id == app_id) {
        entry.has_fix = true;
        entry.updated_at = now_rfc3339();
        save_index_with_data_dir(lib, data_dir, &entries)?;
    }
    Ok(out)
}



/// Hide or reveal a game in the library view. The entry and its files are
/// untouched — only the `hidden` flag flips.
pub fn set_hidden(lib: &Path, app_id: &str, hidden: bool) -> Result<()> {
    set_hidden_with_data_dir(lib, &config::data_dir(), app_id, hidden)
}

/// `set_hidden` with injected `data_dir` (for tests).
pub fn set_hidden_with_data_dir(lib: &Path, data_dir: &Path, app_id: &str, hidden: bool) -> Result<()> {
    // Strict load — fails on HMAC mismatch.
    let mut entries = load_index_with_data_dir(lib, data_dir)
        .context("integrity check before set_hidden")?;

    let Some(entry) = entries.iter_mut().find(|e| e.app_id == app_id) else {
        return Ok(());
    };
    entry.hidden = hidden;
    save_index_with_data_dir(lib, data_dir, &entries)
}

/// Correct the name and (optionally) icon of an existing entry without
/// touching its `.lua` file. An unknown AppID is a quiet no-op, just like
/// `set_hidden`: a concurrent removal is not an error.
pub fn set_display(lib: &Path, app_id: &str, name: &str, icon: Option<&str>) -> Result<()> {
    set_display_with_data_dir(lib, &config::data_dir(), app_id, name, icon)
}

/// `set_display` with injected `data_dir` (for tests).
pub fn set_display_with_data_dir(
    lib: &Path,
    data_dir: &Path,
    app_id: &str,
    name: &str,
    icon: Option<&str>,
) -> Result<()> {
    let mut entries = load_index_with_data_dir(lib, data_dir)
        .context("integrity check before set_display")?;

    let Some(entry) = entries.iter_mut().find(|e| e.app_id == app_id) else {
        return Ok(());
    };
    entry.name = name.to_string();
    // `None` means Steam did not supply a header image. It must never erase
    // an icon already known locally.
    if let Some(icon) = icon {
        entry.icon = Some(icon.to_string());
    }
    entry.updated_at = now_rfc3339();
    save_index_with_data_dir(lib, data_dir, &entries)
}

/// Normalise une liste de tags fournie par l'utilisateur : trim, suppression des
/// vides, limitation de la longueur et du nombre, normalisation des espaces internes,
/// et déduplication insensible à la casse en conservant la première orthographe.
///
/// La déduplication s'effectue **après** la troncature, sur la valeur tronquée.
pub fn normalize_tags(raw: &[String]) -> Vec<String> {
    /// Maximum number of tags per game.
    const MAX_TAGS: usize = 8;
    /// Maximum length per tag in characters.
    const MAX_LEN: usize = 24;

    let mut seen_lower: Vec<String> = Vec::with_capacity(MAX_TAGS);
    let mut out: Vec<String> = Vec::with_capacity(MAX_TAGS);

    for raw_tag in raw {
        let trimmed = raw_tag.trim();
        if trimmed.is_empty() {
            continue;
        }
        // Normalise les espaces internes : un seul espace entre mots.
        let normalized: String = trimmed.split_whitespace().collect::<Vec<_>>().join(" ");
        // Tronque sur un caractère (pas un octet), en conservant la casse.
        let capped: String = normalized.chars().take(MAX_LEN).collect::<String>().trim_end().to_string();
        if capped.is_empty() {
            continue;
        }
        // Déduplication insensible à la casse, sur la valeur **tronquée**.
        let key = capped.to_lowercase();
        if seen_lower.contains(&key) {
            continue;
        }
        seen_lower.push(key);
        out.push(capped);
        if out.len() >= MAX_TAGS {
            break;
        }
    }
    out
}

/// Set (replace) the tags for a library entry. Normalises before saving.
/// If the AppID is unknown, returns `Ok(())` without doing anything.
pub fn set_tags(lib: &Path, app_id: &str, tags: &[String]) -> Result<()> {
    set_tags_with_data_dir(lib, &config::data_dir(), app_id, tags)
}

/// `set_tags` with injected `data_dir` (for tests).
pub fn set_tags_with_data_dir(lib: &Path, data_dir: &Path, app_id: &str, tags: &[String]) -> Result<()> {
    // Strict load — fails on HMAC mismatch.
    let mut entries = load_index_with_data_dir(lib, data_dir)
        .context("integrity check before set_tags")?;

    let normalised = normalize_tags(tags);
    let Some(entry) = entries.iter_mut().find(|e| e.app_id == app_id) else {
        return Ok(());
    };
    entry.tags = normalised;
    save_index_with_data_dir(lib, data_dir, &entries)
}



/// Remove a game from the library (index entry, .lua, fix archive).
pub fn remove(lib: &Path, app_id: &str) -> Result<()> {
    remove_with_data_dir(lib, &config::data_dir(), app_id)
}

/// `remove` with injected `data_dir` (for tests).
pub fn remove_with_data_dir(lib: &Path, data_dir: &Path, app_id: &str) -> Result<()> {
    // Strict load — fails on HMAC mismatch.
    let mut entries = load_index_with_data_dir(lib, data_dir)
        .context("integrity check before remove")?;

    entries.retain(|e| e.app_id != app_id);
    save_index_with_data_dir(lib, data_dir, &entries)?;
    let _ = std::fs::remove_file(lib.join(lua_file_name(app_id)));
    let _ = std::fs::remove_file(lib.join("fixes").join(format!("{app_id}_online_fix.rar")));
    Ok(())
}

/// Is the `.lua` currently present in `{Steam}\config\lua`?
pub fn is_in_steam(app_id: &str, steam: &Path) -> bool {
    crate::detect::lua_dir(steam)
        .join(lua_file_name(app_id))
        .is_file()
}

/// Remove the `.lua` from Steam, leaving the library copy intact.
/// Returns `false` when there was nothing to remove.
pub fn remove_from_steam(app_id: &str, steam: &Path) -> Result<bool> {
    let path = crate::detect::lua_dir(steam).join(lua_file_name(app_id));
    if !path.is_file() {
        return Ok(false);
    }
    std::fs::remove_file(&path)
        .with_context(|| format!("suppression de {}", path.display()))?;
    Ok(true)
}

/// Copy a library .lua into {Steam}\config\lua, creating the folder if needed.
pub fn copy_to_steam(lib: &Path, app_id: &str, steam: &Path) -> Result<PathBuf> {
    let src = lib.join(lua_file_name(app_id));
    if !src.exists() {
        return Err(anyhow!("fichier .lua introuvable dans la bibliothèque"));
    }
    let dst_dir = crate::detect::lua_dir(steam);
    std::fs::create_dir_all(&dst_dir).context("create Steam config\\lua")?;
    let dst = dst_dir.join(lua_file_name(app_id));
    std::fs::copy(&src, &dst).context("copy .lua to Steam")?;
    Ok(dst)
}

/// Copy every library .lua into {Steam}\config\lua. Returns the copied count.
///
/// On HMAC failure, returns the error without copying anything.
pub fn sync_all(lib: &Path, steam: &Path) -> Result<u32> {
    sync_all_with_data_dir(lib, &config::data_dir(), steam)
}

/// `sync_all` with injected `data_dir` (for tests).
pub fn sync_all_with_data_dir(lib: &Path, data_dir: &Path, steam: &Path) -> Result<u32> {
    let entries = load_index_with_data_dir(lib, data_dir)
        .context("integrity check before sync_all")?;
    let mut count = 0u32;
    for entry in &entries {
        if copy_to_steam(lib, &entry.app_id, steam).is_ok() {
            count += 1;
        }
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Every cache-sensitive test takes the process-wide lock (shared with
    // the discover and wipe tests, since `INDEX_CACHE` is a static and
    // Rust runs tests on parallel threads).
    fn cache_lock() -> std::sync::MutexGuard<'static, ()> {
        cache_test_lock()
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("ast_test_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn make_index(lib: &Path, content: &str) {
        let idx = index_path(lib);
        std::fs::create_dir_all(lib).unwrap();
        std::fs::write(&idx, content).unwrap();
    }

    // ------------------------------------------------------- load_index_with_data_dir

    #[test]
    fn load_index_with_data_dir_absent_index() {
        let _lock = cache_lock();
        clear_index_cache();
        let lib = scratch("lid_absent");
        let data = scratch("lid_data_absent");
        let entries = load_index_with_data_dir(&lib, &data).unwrap();
        assert!(entries.is_empty());
        assert!(!hmac::has_key(&data));
        assert!(!hmac::has_sidecar(&hmac::index_path(&lib)));
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn load_index_with_data_dir_migration_creates_key() {
        let _lock = cache_lock();
        clear_index_cache();
        let data = scratch("lid_mig");
        let lib = data.join("lib");
        make_index(&lib, r#"[]"#);
        let entries = load_index_with_data_dir(&lib, &data).unwrap();
        assert!(entries.is_empty());
        assert!(hmac::has_key(&data));
        assert!(hmac::has_sidecar(&hmac::index_path(&lib)));
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn load_index_with_data_dir_hmac_mismatch_errors() {
        let _lock = cache_lock();
        clear_index_cache();
        let data = scratch("lid_hmac");
        let lib = data.join("lib");
        make_index(&lib, r#"[{"app_id":"1","name":"T","file_name":"1.lua","added_at":"2024-01-01T00:00:00Z","updated_at":"2024-01-01T00:00:00Z"}]"#);
        // First load signs it.
        load_index_with_data_dir(&lib, &data).unwrap();
        // Tamper with different-length content so the stamp changes
        // (same-length writes within the same 15.6 ms tick produce
        // an identical FileStamp and would hit the cache before HMAC).
        make_index(&lib, r#"[{"app_id":"2","name":"X","file_name":"2.lua","added_at":"2024-01-01T00:00:00Z","updated_at":"2024-01-01T00:00:00Z","extra":"field"}]"#);
        assert!(load_index_with_data_dir(&lib, &data).is_err());
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn load_index_with_data_dir_sidecar_without_key_errors() {
        let _lock = cache_lock();
        clear_index_cache();
        let data = scratch("lid_orphan");
        let lib = data.join("lib");
        make_index(&lib, r#"[]"#);
        // Create sidecar without key.
        let key = [42u8; 32];
        let tag = hmac::sign_bytes(&key, b"[]");
        hmac::write_sidecar(&hmac::index_path(&lib), &tag).unwrap();
        assert!(load_index_with_data_dir(&lib, &data).is_err());
        let _ = std::fs::remove_dir_all(&data);
    }

    // ------------------------------------------------------- Writers with data_dir

    #[test]
    fn upsert_with_data_dir_roundtrip() {
        let _lock = cache_lock();
        clear_index_cache();
        let data = scratch("uprt");
        let lib = data.join("lib");
        let entry = upsert_with_data_dir(&lib, &data, "264710", "Subnautica", Some("https://x/icon.jpg"), b"-- lua v1")
            .expect("upsert");
        assert_eq!(entry.file_name, "264710.lua");
        assert!(lib.join("264710.lua").exists());
        let entries = load_index_with_data_dir(&lib, &data).unwrap();
        assert_eq!(entries.len(), 1);
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn upsert_with_data_dir_refuses_bad_hmac() {
        let _lock = cache_lock();
        clear_index_cache();
        let data = scratch("uprt_bad");
        let lib = data.join("lib");
        make_index(&lib, r#"[{"app_id":"1","name":"T","file_name":"1.lua","added_at":"2024-01-01T00:00:00Z","updated_at":"2024-01-01T00:00:00Z"}]"#);
        let key = hmac::load_or_create_key(&data).unwrap();
        hmac::sign_index(&hmac::index_path(&lib), &key).unwrap();
        // Tamper.
        make_index(&lib, r#"[{"app_id":"99","name":"Evil","file_name":"99.lua","added_at":"2024-01-01T00:00:00Z","updated_at":"2024-01-01T00:00:00Z"}]"#);
        let idx = hmac::index_path(&lib);
        let original = std::fs::read(&idx).unwrap();
        let result = upsert_with_data_dir(&lib, &data, "2", "Evil2", None, b"lua");
        assert!(result.is_err(), "upsert must refuse tampered index");
        let after = std::fs::read(&idx).unwrap();
        assert_eq!(original, after, "index must be unchanged after failed upsert");
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn mark_fix_with_data_dir_refuses_bad_hmac() {
        let _lock = cache_lock();
        clear_index_cache();
        let data = scratch("mfx_bad");
        let lib = data.join("lib");
        make_index(&lib, r#"[{"app_id":"1","name":"T","file_name":"1.lua","added_at":"2024-01-01T00:00:00Z","updated_at":"2024-01-01T00:00:00Z"}]"#);
        let key = hmac::load_or_create_key(&data).unwrap();
        hmac::sign_index(&hmac::index_path(&lib), &key).unwrap();
        make_index(&lib, r#"[{"app_id":"99","name":"Evil","file_name":"99.lua","added_at":"2024-01-01T00:00:00Z","updated_at":"2024-01-01T00:00:00Z"}]"#);
        assert!(mark_fix_with_data_dir(&lib, &data, "1", b"fix").is_err());
        let _ = std::fs::remove_dir_all(&data);
    }



    #[test]
    fn set_hidden_with_data_dir_refuses_bad_hmac() {
        let _lock = cache_lock();
        clear_index_cache();
        let data = scratch("sh_bad");
        let lib = data.join("lib");
        make_index(&lib, r#"[{"app_id":"1","name":"T","file_name":"1.lua","added_at":"2024-01-01T00:00:00Z","updated_at":"2024-01-01T00:00:00Z"}]"#);
        let key = hmac::load_or_create_key(&data).unwrap();
        hmac::sign_index(&hmac::index_path(&lib), &key).unwrap();
        make_index(&lib, r#"[{"app_id":"99","name":"Evil","file_name":"99.lua","added_at":"2024-01-01T00:00:00Z","updated_at":"2024-01-01T00:00:00Z"}]"#);
        assert!(set_hidden_with_data_dir(&lib, &data, "1", true).is_err());
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn set_display_with_data_dir_refuses_bad_hmac() {
        let _lock = cache_lock();
        clear_index_cache();
        let data = scratch("sd_bad");
        let lib = data.join("lib");
        make_index(&lib, r#"[{"app_id":"1","name":"T","file_name":"1.lua","added_at":"2024-01-01T00:00:00Z","updated_at":"2024-01-01T00:00:00Z"}]"#);
        let key = hmac::load_or_create_key(&data).unwrap();
        hmac::sign_index(&hmac::index_path(&lib), &key).unwrap();
        make_index(&lib, r#"[{"app_id":"99","name":"Evil","file_name":"99.lua","added_at":"2024-01-01T00:00:00Z","updated_at":"2024-01-01T00:00:00Z"}]"#);
        let idx = hmac::index_path(&lib);
        let original = std::fs::read(&idx).unwrap();

        assert!(set_display_with_data_dir(&lib, &data, "1", "Legit", None).is_err());
        assert_eq!(std::fs::read(&idx).unwrap(), original);

        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn set_tags_with_data_dir_refuses_bad_hmac() {
        let _lock = cache_lock();
        clear_index_cache();
        let data = scratch("st_bad");
        let lib = data.join("lib");
        make_index(&lib, r#"[{"app_id":"1","name":"T","file_name":"1.lua","added_at":"2024-01-01T00:00:00Z","updated_at":"2024-01-01T00:00:00Z"}]"#);
        let key = hmac::load_or_create_key(&data).unwrap();
        hmac::sign_index(&hmac::index_path(&lib), &key).unwrap();
        make_index(&lib, r#"[{"app_id":"99","name":"Evil","file_name":"99.lua","added_at":"2024-01-01T00:00:00Z","updated_at":"2024-01-01T00:00:00Z"}]"#);
        assert!(set_tags_with_data_dir(&lib, &data, "1", &["tag1".into()]).is_err());
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn remove_with_data_dir_refuses_bad_hmac() {
        let _lock = cache_lock();
        clear_index_cache();
        let data = scratch("rm_bad");
        let lib = data.join("lib");
        make_index(&lib, r#"[{"app_id":"1","name":"T","file_name":"1.lua","added_at":"2024-01-01T00:00:00Z","updated_at":"2024-01-01T00:00:00Z"}]"#);
        let key = hmac::load_or_create_key(&data).unwrap();
        hmac::sign_index(&hmac::index_path(&lib), &key).unwrap();
        make_index(&lib, r#"[{"app_id":"99","name":"Evil","file_name":"99.lua","added_at":"2024-01-01T00:00:00Z","updated_at":"2024-01-01T00:00:00Z"}]"#);
        assert!(remove_with_data_dir(&lib, &data, "1").is_err());
        let _ = std::fs::remove_dir_all(&data);
    }

    // ------------------------------------------------------- sync_all_with_data_dir

    #[test]
    fn sync_all_with_data_dir_refuses_bad_hmac() {
        let _lock = cache_lock();
        clear_index_cache();
        let data = scratch("sa_bad");
        let lib = data.join("lib");
        let steam = data.join("steam");
        std::fs::create_dir_all(&steam).unwrap();
        make_index(&lib, r#"[{"app_id":"1","name":"T","file_name":"1.lua","added_at":"2024-01-01T00:00:00Z","updated_at":"2024-01-01T00:00:00Z"}]"#);
        let key = hmac::load_or_create_key(&data).unwrap();
        hmac::sign_index(&hmac::index_path(&lib), &key).unwrap();
        make_index(&lib, r#"[{"app_id":"99","name":"Evil","file_name":"99.lua","added_at":"2024-01-01T00:00:00Z","updated_at":"2024-01-01T00:00:00Z"}]"#);
        assert!(sync_all_with_data_dir(&lib, &data, &steam).is_err());
        let _ = std::fs::remove_dir_all(&data);
    }

    // ------------------------------------------------------- Full roundtrip (existing test)

    #[test]
    fn upsert_list_copy_sync_remove_roundtrip() {
        let _lock = cache_lock();
        clear_index_cache();
        let root = scratch("roundtrip");
        let data = root.join("data");
        let lib = root.join("lib");
        let steam = root.join("steam");
        std::fs::create_dir_all(&steam).unwrap();

        let entry = upsert_with_data_dir(&lib, &data, "264710", "Subnautica", Some("https://x/icon.jpg"), b"-- lua v1")
            .expect("upsert");
        assert_eq!(entry.file_name, "264710.lua");
        assert!(lib.join("264710.lua").exists());
        assert_eq!(load_index_with_data_dir(&lib, &data).unwrap().len(), 1);

        // Re-upsert updates in place instead of duplicating.
        upsert_with_data_dir(&lib, &data, "264710", "Subnautica", None, b"-- lua v2").expect("re-upsert");
        assert_eq!(load_index_with_data_dir(&lib, &data).unwrap().len(), 1);

        let dst = copy_to_steam(&lib, "264710", &steam).expect("copy");
        assert_eq!(dst, steam.join("config").join("lua").join("264710.lua"));
        assert_eq!(std::fs::read_to_string(&dst).unwrap(), "-- lua v2");

        assert_eq!(sync_all_with_data_dir(&lib, &data, &steam).unwrap(), 1);

        let fix_path = mark_fix_with_data_dir(&lib, &data, "264710", b"rar-bytes").expect("mark_fix");
        assert!(fix_path.exists());
        assert!(load_index_with_data_dir(&lib, &data).unwrap()[0].has_fix);



        assert!(is_in_steam("264710", &steam));
        assert!(remove_from_steam("264710", &steam).unwrap());
        assert!(!is_in_steam("264710", &steam));
        // Removing twice is a no-op, not an error.
        assert!(!remove_from_steam("264710", &steam).unwrap());
        // The library copy survives a Steam-side removal.
        assert!(lib.join("264710.lua").exists());

        remove_with_data_dir(&lib, &data, "264710").expect("remove");
        assert!(load_index_with_data_dir(&lib, &data).unwrap().is_empty());
        assert!(!lib.join("264710.lua").exists());
        assert!(!fix_path.exists());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn copy_to_steam_fails_when_missing() {
        let _lock = cache_lock();
        clear_index_cache();
        let lib = scratch("lib_missing");
        let steam = scratch("steam_missing");
        std::fs::create_dir_all(&steam).unwrap();
        assert!(copy_to_steam(&lib, "999", &steam).is_err());
        let _ = std::fs::remove_dir_all(&lib);
        let _ = std::fs::remove_dir_all(&steam);
    }

    #[test]
    fn hide_and_reveal_keeps_everything() {
        let _lock = cache_lock();
        clear_index_cache();
        let root = scratch("hide");
        let data = root.join("data");
        let lib = root.join("lib");
        upsert_with_data_dir(&lib, &data, "264710", "Subnautica", None, b"-- lua").expect("upsert");
        assert!(!load_index_with_data_dir(&lib, &data).unwrap()[0].hidden);

        set_hidden_with_data_dir(&lib, &data, "264710", true).expect("hide");
        let entry = &load_index_with_data_dir(&lib, &data).unwrap()[0];
        assert!(entry.hidden);
        // Hiding never touches the files themselves.
        assert!(lib.join("264710.lua").exists());

        set_hidden_with_data_dir(&lib, &data, "264710", false).expect("reveal");
        assert!(!load_index_with_data_dir(&lib, &data).unwrap()[0].hidden);
        assert!(lib.join("264710.lua").exists());

        // Hiding an unknown game is a quiet no-op.
        assert!(set_hidden_with_data_dir(&lib, &data, "999", true).is_ok());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn set_display_with_data_dir_updates_name_and_icon() {
        let _lock = cache_lock();
        clear_index_cache();
        let root = scratch("set_display");
        let data = root.join("data");
        let lib = root.join("lib");
        let lua = b"-- original lua";
        upsert_with_data_dir(&lib, &data, "264710", "AppID 264710", Some("https://old/icon.jpg"), lua)
            .expect("upsert");

        set_display_with_data_dir(
            &lib,
            &data,
            "264710",
            "Subnautica",
            Some("https://new/icon.jpg"),
        )
        .expect("set display");

        let entry = &load_index_with_data_dir(&lib, &data).unwrap()[0];
        assert_eq!(entry.name, "Subnautica");
        assert_eq!(entry.icon.as_deref(), Some("https://new/icon.jpg"));
        assert_eq!(std::fs::read(lib.join("264710.lua")).unwrap(), lua);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn set_display_with_data_dir_keeps_icon_when_none() {
        let _lock = cache_lock();
        clear_index_cache();
        let root = scratch("set_display_no_icon");
        let data = root.join("data");
        let lib = root.join("lib");
        upsert_with_data_dir(&lib, &data, "264710", "Old name", Some("https://known/icon.jpg"), b"-- lua")
            .expect("upsert");

        set_display_with_data_dir(&lib, &data, "264710", "Subnautica", None)
            .expect("set display");

        let entry = &load_index_with_data_dir(&lib, &data).unwrap()[0];
        assert_eq!(entry.name, "Subnautica");
        assert_eq!(entry.icon.as_deref(), Some("https://known/icon.jpg"));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn set_display_with_data_dir_unknown_app_id_is_noop() {
        let _lock = cache_lock();
        clear_index_cache();
        let root = scratch("set_display_unknown");
        let data = root.join("data");
        let lib = root.join("lib");
        upsert_with_data_dir(&lib, &data, "264710", "Subnautica", Some("https://icon.jpg"), b"-- lua")
            .expect("upsert");
        let idx = hmac::index_path(&lib);
        let original = std::fs::read(&idx).unwrap();

        set_display_with_data_dir(&lib, &data, "999", "Unknown", Some("https://other/icon.jpg"))
            .expect("unknown app is a no-op");

        assert_eq!(std::fs::read(&idx).unwrap(), original);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Freshness and integrity: a rewrite of index.json is never masked by
    /// the cache. Re-signed with the same key the strict load sees the new
    /// entries (a); rewritten without a signature it fails (b).
    #[test]
    fn load_index_sees_external_modifications() {
        let _lock = cache_lock();
        clear_index_cache();
        let root = scratch("fresh");
        let data = root.join("data");
        let lib = root.join("lib");

        // Upsert creates and signs index.json with one entry.
        upsert_with_data_dir(&lib, &data, "264710", "Subnautica", None, b"-- lua").expect("upsert");
        let entries = load_index_with_data_dir(&lib, &data).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].app_id, "264710");

        let idx = hmac::index_path(&lib);

        // (a) Rewritten then re-signed with the same key: the strict load
        // sees the new entries — the cache must not mask them.
        std::fs::write(
            &idx,
            r#"[{"app_id":"999","name":"Other","file_name":"999.lua","added_at":"2024-01-01T00:00:00Z","updated_at":"2024-01-01T00:00:00Z"}]"#,
        )
        .unwrap();
        let key = hmac::load_or_create_key(&data).unwrap();
        hmac::sign_index(&idx, &key).unwrap();
        let entries = load_index_with_data_dir(&lib, &data).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].app_id, "999");
        assert_eq!(entries[0].name, "Other");

        // (b) Rewritten without signing (same size — the stamp alone could
        // match the cache): the strict load fails on integrity.
        std::fs::write(
            &idx,
            r#"[{"app_id":"888","name":"Fake1","file_name":"888.lua","added_at":"2024-01-01T00:00:00Z","updated_at":"2024-01-01T00:00:00Z"}]"#,
        )
        .unwrap();
        assert!(load_index_with_data_dir(&lib, &data).is_err());

        let _ = std::fs::remove_dir_all(&root);
    }

    /// Regression: a parse failure on the strict path must never enter the
    /// cache. The index is loaded, then replaced by malformed JSON under a
    /// valid signature — the strict load errors, the cache gains nothing,
    /// and a signed valid rewrite restores the entries.
    #[test]
    fn load_index_does_not_cache_read_failures() {
        let _lock = cache_lock();
        clear_index_cache();
        let root = scratch("nocache_fail");
        let data = root.join("data");
        let lib = root.join("lib");
        std::fs::create_dir_all(&lib).unwrap();

        // Write a valid index with one entry.
        let valid = r#"[{"app_id":"264710","name":"Subnautica","file_name":"264710.lua","added_at":"2024-01-01T00:00:00Z","updated_at":"2024-01-01T00:00:00Z"}]"#;
        std::fs::write(index_path(&lib), valid).unwrap();

        let entries = load_index_with_data_dir(&lib, &data).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].app_id, "264710");

        // Malformed JSON under a valid signature: the strict path fails at
        // parse, and the failure never enters the cache.
        let key = hmac::load_or_create_key(&data).unwrap();
        std::fs::write(index_path(&lib), "{bad json").unwrap();
        hmac::sign_index(&index_path(&lib), &key).unwrap();
        assert!(load_index_with_data_dir(&lib, &data).is_err());
        assert!(
            INDEX_CACHE.len() <= 1,
            "a parse failure must not enter the cache"
        );

        // Rewrite the valid JSON and sign it — the entries come back.
        std::fs::write(index_path(&lib), valid).unwrap();
        hmac::sign_index(&index_path(&lib), &key).unwrap();
        let entries = load_index_with_data_dir(&lib, &data).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].app_id, "264710");
        assert_eq!(entries[0].name, "Subnautica");

        let _ = std::fs::remove_dir_all(&root);
    }

    /// Prove that the cache branch of the strict load is exercised and
    /// returns identical data to the disk branch. The settle window is
    /// neutralised for the test; `clear_index_cache` (taken by every
    /// cache-sensitive test) restores the default 2 s afterwards.
    #[test]
    fn load_index_cache_hit_returns_same_data() {
        let _lock = cache_lock();
        clear_index_cache();
        set_index_settle(0);
        let root = scratch("cache_hit");
        let data = root.join("data");
        let lib = root.join("lib");
        std::fs::create_dir_all(&lib).unwrap();

        std::fs::write(
            index_path(&lib),
            r#"[{"app_id":"264710","name":"Subnautica","file_name":"264710.lua","added_at":"2024-01-01T00:00:00Z","updated_at":"2024-01-01T00:00:00Z","has_fix":true,"hidden":false}]"#,
        )
        .unwrap();

        // Reset the hit counter right before the two calls — no other test
        // can interfere because we hold the cache lock.
        INDEX_CACHE.reset_hits();

        // First call — migration + disk read.
        let first = load_index_with_data_dir(&lib, &data).unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].app_id, "264710");
        assert_eq!(first[0].name, "Subnautica");
        assert!(first[0].has_fix);
        assert_eq!(INDEX_CACHE.hits(), 0, "the first load is a disk read");

        // Second call — integrity verified, then served from cache.
        let second = load_index_with_data_dir(&lib, &data).unwrap();
        assert_eq!(
            INDEX_CACHE.hits(),
            1,
            "second load must hit the cache"
        );

        // Every field must match exactly.
        assert_eq!(first, second);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn normalize_tags_trims_and_drops_empty() {
        let input = vec![
            "  coop  ".to_string(),
            "".to_string(),
            "   ".to_string(),
            "solo".to_string(),
        ];
        let result = normalize_tags(&input);
        assert_eq!(result, vec!["coop", "solo"]);
    }

    #[test]
    fn normalize_tags_dedup_case_insensitive() {
        let input = vec![
            "Coop".to_string(),
            "coop".to_string(),
            "  COOP  ".to_string(),
        ];
        let result = normalize_tags(&input);
        // Keeps first spelling.
        assert_eq!(result, vec!["Coop"]);
    }

    #[test]
    fn normalize_tags_cap_at_eight() {
        let input: Vec<String> = (0..12)
            .map(|i| format!("tag{}", i))
            .collect();
        let result = normalize_tags(&input);
        assert_eq!(result.len(), 8);
        assert_eq!(result[7], "tag7");
    }

    #[test]
    fn normalize_tags_truncates_on_char_boundary() {
        // "café" — é is multi-byte. Truncating to 24 chars on a string slice
        // would panic; using chars().take(24) is safe.
        let long_tag = "a".repeat(20) + "é";
        assert_eq!(long_tag.chars().count(), 21);
        let input = vec![long_tag.clone()];
        let result = normalize_tags(&input);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].chars().count(), 21); // under the 24-char cap

        // Exactly 25 chars — must truncate to 24.
        let too_long = "a".repeat(24) + "x";
        assert_eq!(too_long.chars().count(), 25);
        let input2 = vec![too_long];
        let result2 = normalize_tags(&input2);
        assert_eq!(result2[0].chars().count(), 24);
    }

    /// Truncation at 24 chars with a trailing emoji: the result must be
    /// exactly 24 **characters** (not bytes). If you replace `chars().take(24)`
    /// with `&s[..24.min(s.len())]` the test will panic on a multi-byte char.
    #[test]
    fn normalize_tags_truncates_emoji_boundary() {
        // 25 chars: 21 'a' + é (2 bytes) + 3 emoji (12 bytes) = 21+2+12 = 35 bytes.
        // Byte-slicing at 24 would land inside the é character → panic.
        let input = vec!["a".repeat(21) + "é\u{1F600}\u{1F601}\u{1F602}"];
        let result = normalize_tags(&input);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].chars().count(), 24);
        // The 24th char (index 23) must be the 4th emoji (😂), not a UTF-8 fragment.
        // chars().take(24) → 21 'a' + é + 😀 + 😁 = 24 chars → nth(23) = 😁
        assert_eq!(result[0].chars().nth(23), Some('\u{1F601}'));
    }

    /// Two tags of 30 chars identical on the first 24 → one tag in output.
    /// This is the truncation-before-dedup bug (point 1).
    #[test]
    fn normalize_tags_truncates_before_dedup() {
        let input = vec![
            "aaaaaaaaaaaaaaaaaaaaaaaabbbb".to_string(), // 28 chars
            "aaaaaaaaaaaaaaaaaaaaaaaacccc".to_string(), // 28 chars
        ];
        let result = normalize_tags(&input);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].chars().count(), 24);
    }

    /// Internal-space normalisation: "  co op  ", "co  op", "co op", " co op"
    /// → a single tag "co op".
    #[test]
    fn normalize_tags_normalizes_internal_spaces() {
        let input = vec![
            "  co op  ".to_string(),
            "co  op".to_string(),
            "co op".to_string(),
            " co op".to_string(),
        ];
        let result = normalize_tags(&input);
        assert_eq!(result, vec!["co op"]);

        // Troncature à 24 chars : 23 'a' + espace + 'b' → le trim_end supprime
        // l'espace au 24e caractère, le 'b' est tronqué par chars().take(24).
        let input2 = vec!["a".repeat(23) + " b"];
        let result2 = normalize_tags(&input2);
        assert_eq!(result2, vec!["a".repeat(23)]);
    }

    /// `set_tags` with a non-normalised list (duplicates, wrong case, spaces,
    /// too-long tag, more than 8) must round-trip through `load_index` as the
    /// normalised version.
    #[test]
    fn set_tags_roundtrip_normalises() {
        let _lock = cache_lock();
        clear_index_cache();
        let root = scratch("tags");
        let data = root.join("data");
        let lib = root.join("lib");
        upsert_with_data_dir(&lib, &data, "264710", "Subnautica", None, b"-- lua").expect("upsert");

        // Feed a deliberately messy list.
        let messy = vec![
            "  Coop  ".into(),
            "coop".into(),           // dup, different case
            "co  op".into(),         // double space
            "co  op".into(),         // dup after normalisation
            "tag0".into(),
            "tag1".into(),
            "tag2".into(),
            "tag3".into(),
            "tag4".into(),
            "tag5".into(),
            "tag6".into(),
            "tag7".into(),
            "tag8".into(),           // #9 → dropped
            "this-is-a-very-long-tag-that-exceeds".into(), // > 24 → truncated
        ];
        set_tags_with_data_dir(&lib, &data, "264710", &messy).expect("set_tags");

        let entries = load_index_with_data_dir(&lib, &data).unwrap();
        let tags = &entries[0].tags;
        // Expected: Coop, co op, tag0..tag5 = 8 items (MAX_TAGS).
        // "coop" is a dup of "Coop" (case-insensitive), "co  op" normalises
        // to "co op" (dup after normalisation), tag8 and the long tag are
        // beyond the 8-tag cap.
        assert_eq!(tags.len(), 8);
        assert_eq!(tags[0], "Coop");
        assert_eq!(tags[1], "co op");
        assert_eq!(tags[2], "tag0");
        assert_eq!(tags[7], "tag5");

        // Unknown app_id is a quiet no-op.
        set_tags_with_data_dir(&lib, &data, "99999", &["foo".into()]).expect("unknown no-op");
        assert_eq!(
            load_index_with_data_dir(&lib, &data).unwrap()[0].tags,
            vec!["Coop", "co op", "tag0", "tag1", "tag2", "tag3", "tag4", "tag5"]
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// A parse failure during the migration (no key, no sidecar yet) errors
    /// on the strict path and must not populate the cache.
    #[test]
    fn load_index_failure_does_not_fill_empty_cache() {
        let _lock = cache_lock();
        clear_index_cache();
        let root = scratch("nocache_fail2");
        let data = root.join("data");
        let lib = root.join("lib");
        std::fs::create_dir_all(&lib).unwrap();

        // Write malformed JSON — no prior cache entry.
        std::fs::write(index_path(&lib), "{bad json").unwrap();

        assert!(load_index_with_data_dir(&lib, &data).is_err());

        // The cache must still be empty — the failure was not stored.
        assert_eq!(
            INDEX_CACHE.len(),
            0,
            "a parse failure must not be cached"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    // ------------------------------------------------------- Missing contract tests

    /// The migration signs the bytes it finds — it never rewrites a single
    /// byte of the JSON (a re-serialisation would destroy formatting no
    /// serialiser produces, like the irregular spacing below).
    #[test]
    fn migration_does_not_rewrite_json_bytes() {
        let _lock = cache_lock();
        clear_index_cache();
        let root = scratch("mig_bytes");
        let data = root.join("data");
        let lib = root.join("lib");
        make_index(
            &lib,
            "[{\"app_id\":\"7\",\"name\":\"Seven\",   \"file_name\":\"7.lua\",\"added_at\":\"2024-01-01T00:00:00Z\",\"updated_at\":\"2024-01-01T00:00:00Z\"}]",
        );
        let idx = hmac::index_path(&lib);
        let before = std::fs::read(&idx).unwrap();

        let entries = load_index_with_data_dir(&lib, &data).unwrap();
        assert_eq!(entries.len(), 1);
        assert!(hmac::has_key(&data));
        assert!(hmac::has_sidecar(&idx));
        assert_eq!(
            std::fs::read(&idx).unwrap(),
            before,
            "migration must not rewrite a single byte of the JSON"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// Invalid JSON during the migration: an error, and no key created.
    #[test]
    fn migration_invalid_json_creates_no_key() {
        let _lock = cache_lock();
        clear_index_cache();
        let root = scratch("mig_badjson");
        let data = root.join("data");
        let lib = root.join("lib");
        make_index(&lib, "{bad json");

        assert!(load_index_with_data_dir(&lib, &data).is_err());
        assert!(!hmac::has_key(&data), "invalid JSON must not create a key");
        assert!(!hmac::has_sidecar(&hmac::index_path(&lib)));

        let _ = std::fs::remove_dir_all(&root);
    }

    /// Key present but sidecar absent: the index cannot be verified.
    #[test]
    fn key_present_sidecar_absent_errors() {
        let _lock = cache_lock();
        clear_index_cache();
        let root = scratch("key_no_sidecar");
        let data = root.join("data");
        let lib = root.join("lib");
        make_index(&lib, r#"[]"#);
        hmac::load_or_create_key(&data).unwrap();

        assert!(load_index_with_data_dir(&lib, &data).is_err());

        let _ = std::fs::remove_dir_all(&root);
    }

    /// The best-effort wrapper never parses raw JSON after an integrity
    /// error: a tampered index yields an empty library, not its contents.
    /// Replacing the `Vec::new()` in `load_index_best_effort_with_data_dir`
    /// with a raw-parse fallback turns this red — the rule that keeps a
    /// damaged index from being trusted again.
    #[test]
    fn best_effort_load_never_parses_raw_json_after_integrity_error() {
        let _lock = cache_lock();
        clear_index_cache();
        let root = scratch("best_effort");
        let data = root.join("data");
        let lib = root.join("lib");

        upsert_with_data_dir(&lib, &data, "1", "Legit", None, b"-- lua").expect("upsert");

        // Tamper without re-signing (different size: a fresh stamp, so the
        // cache cannot interfere either).
        make_index(&lib, r#"[{"app_id":"99","name":"Evil","file_name":"99.lua","added_at":"2024-01-01T00:00:00Z","updated_at":"2024-01-01T00:00:00Z"}]"#);

        let entries = load_index_best_effort_with_data_dir(&lib, &data);
        assert!(
            entries.is_empty(),
            "a tampered index must be ignored wholesale, never parsed: {entries:?}"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// The integrity check precedes any cache hit: with the settle window
    /// neutralised the cache below could serve the entry on stamp alone —
    /// a deleted sidecar must still be refused.
    #[test]
    fn sidecar_deleted_after_cache_is_refused() {
        let _lock = cache_lock();
        clear_index_cache();
        set_index_settle(0);
        let root = scratch("sidecar_deleted");
        let data = root.join("data");
        let lib = root.join("lib");
        make_index(&lib, r#"[{"app_id":"1","name":"T","file_name":"1.lua","added_at":"2024-01-01T00:00:00Z","updated_at":"2024-01-01T00:00:00Z"}]"#);

        load_index_with_data_dir(&lib, &data).unwrap();
        assert_eq!(INDEX_CACHE.len(), 1);

        hmac::remove_sidecar(&hmac::index_path(&lib)).unwrap();
        assert!(load_index_with_data_dir(&lib, &data).is_err());

        let _ = std::fs::remove_dir_all(&root);
    }

    /// Same guarantee with a corrupted sidecar: the cache entry is
    /// irrelevant, and the error says "malformed", not "wrong tag".
    #[test]
    fn sidecar_malformed_after_cache_is_refused() {
        let _lock = cache_lock();
        clear_index_cache();
        set_index_settle(0);
        let root = scratch("sidecar_bad");
        let data = root.join("data");
        let lib = root.join("lib");
        make_index(&lib, r#"[{"app_id":"1","name":"T","file_name":"1.lua","added_at":"2024-01-01T00:00:00Z","updated_at":"2024-01-01T00:00:00Z"}]"#);

        load_index_with_data_dir(&lib, &data).unwrap();
        assert_eq!(INDEX_CACHE.len(), 1);

        std::fs::write(hmac::sidecar_path(&hmac::index_path(&lib)), "garbage").unwrap();
        let err = load_index_with_data_dir(&lib, &data).unwrap_err();
        assert!(err.to_string().contains("mal formée"), "got: {err}");

        let _ = std::fs::remove_dir_all(&root);
    }

    /// A malformed sidecar and a well-formed sidecar carrying a wrong tag
    /// are different failures — the first says "unreadable signature", the
    /// second says "the index was modified".
    #[test]
    fn sidecar_malformed_is_distinct_from_wrong_tag() {
        let _lock = cache_lock();
        clear_index_cache();
        let root = scratch("sidecar_distinct");
        let data = root.join("data");
        let lib = root.join("lib");
        make_index(&lib, r#"[{"app_id":"1","name":"T","file_name":"1.lua","added_at":"2024-01-01T00:00:00Z","updated_at":"2024-01-01T00:00:00Z"}]"#);
        load_index_with_data_dir(&lib, &data).unwrap();

        let idx = hmac::index_path(&lib);
        let raw = std::fs::read(&idx).unwrap();

        // Well-formed sidecar, wrong tag → HMAC error.
        let forged = hmac::sign_bytes(&[0u8; 32], &raw);
        hmac::write_sidecar(&idx, &forged).unwrap();
        let err = load_index_with_data_dir(&lib, &data).unwrap_err();
        assert!(err.to_string().contains("HMAC"), "got: {err}");

        // Lowercase hex → malformed sidecar, a distinct error.
        let hex: String = forged.iter().map(|b| format!("{:02x}", b)).collect();
        std::fs::write(hmac::sidecar_path(&idx), format!("LV-HMAC-v1:{hex}")).unwrap();
        let err = load_index_with_data_dir(&lib, &data).unwrap_err();
        assert!(err.to_string().contains("mal formée"), "got: {err}");

        let _ = std::fs::remove_dir_all(&root);
    }

    // ------------------------------------------------------- parse_lua (LOT-10)

    /// The exact file the server generates: header with zero-width characters
    /// (title obfuscation), main application, one DLC with content, one token.
    #[test]
    fn parse_lua_reads_the_real_server_format() {
        let text = "-- AppID 1593030\u{200B} | Generated on 2026-01-01 05:55 UTC | LuaVault\n\
-- Main Application\n\
addappid(1593030, 1, \"340ae2fcb31a92507c1e0a9139f7cac3ab50f45a8c701f6d9f06b1793aca9981\")\n\
-- DLCs with Content (1)\n\
addappid(2321930, 1, \"02757f56a474735a503289ae2b52ffb6aeda1c41eb2eec9b5e04bd596eca88d3\")\n\
addtoken(2321930, \"11175953412085981858\")\n";
        let parsed = parse_lua(text);
        // addtoken is not addappid — exactly two AppIDs, main first.
        assert_eq!(
            parsed.app_ids,
            vec!["1593030".to_string(), "2321930".to_string()]
        );
        // The main application is not a DLC; the one declared under
        // "-- DLCs with Content" is.
        assert_eq!(parsed.dlc_app_ids, vec!["2321930".to_string()]);
        assert_eq!(parsed.generated_at.as_deref(), Some("2026-01-01 05:55"));
    }

    #[test]
    fn parse_lua_accepts_a_bare_addappid() {
        let parsed = parse_lua("addappid(123)\n");
        assert_eq!(parsed.app_ids, vec!["123".to_string()]);
        assert_eq!(parsed.generated_at, None);
    }

    #[test]
    fn parse_lua_survives_a_missing_header() {
        let parsed = parse_lua("addappid(1, 1, \"abc\")\naddappid(2, 1, \"def\")\n");
        assert_eq!(parsed.app_ids, vec!["1".to_string(), "2".to_string()]);
        assert_eq!(parsed.generated_at, None);
    }

    #[test]
    fn parse_lua_returns_empty_on_empty_input() {
        assert_eq!(parse_lua(""), LuaContents::default());
    }

    #[test]
    fn parse_lua_returns_empty_on_binary_garbage() {
        // Valid UTF-8 but binary-like: NULs and control characters everywhere,
        // and a broken call that must not yield an AppID.
        let garbage = "\0\x01\x02\0\x7f\n\0addappid(\naddappid(12ab)\n";
        assert_eq!(parse_lua(garbage), LuaContents::default());
    }

    #[test]
    fn parse_lua_ignores_commented_calls() {
        let text = "-- addappid(999)\naddappid(123)\n   -- addappid(888)\n";
        let parsed = parse_lua(text);
        assert_eq!(parsed.app_ids, vec!["123".to_string()]);
    }

    #[test]
    fn parse_lua_strips_zero_width_chars_from_the_date() {
        let text = "-- Generated on 2026\u{200B}-01-01 05\u{200C}:55 UTC | LuaVault\naddappid(7)\n";
        let parsed = parse_lua(text);
        assert_eq!(parsed.generated_at.as_deref(), Some("2026-01-01 05:55"));
    }

    /// M2 — a header without " UTC" must not hand the rest of the file to
    /// the frontend: the date is bounded to its own line before splitting.
    #[test]
    fn parse_lua_bounds_the_date_to_its_line() {
        let parsed = parse_lua("-- Generated on 2026-01-01 05:55\naddappid(7)\n");
        assert_eq!(parsed.generated_at.as_deref(), Some("2026-01-01 05:55"));
        assert_eq!(parsed.app_ids, vec!["7".to_string()]);
    }

    // M3 — one test per measured false positive / false negative.

    #[test]
    fn parse_lua_rejects_a_call_glued_to_an_identifier() {
        // `myaddappid` is another function: the character before the call
        // must not be an identifier character.
        let parsed = parse_lua("local x = myaddappid(4242)\n");
        assert!(parsed.app_ids.is_empty());
    }

    #[test]
    fn parse_lua_rejects_a_call_inside_a_string() {
        let parsed = parse_lua("print(\"addappid(5)\")\n");
        assert!(parsed.app_ids.is_empty());
    }

    #[test]
    fn parse_lua_trims_the_token() {
        let parsed = parse_lua("addappid( 123 )\n");
        assert_eq!(parsed.app_ids, vec!["123".to_string()]);
    }

    #[test]
    fn parse_lua_stops_at_an_inline_comment() {
        let parsed = parse_lua("addappid(1) -- addappid(999)\n");
        assert_eq!(parsed.app_ids, vec!["1".to_string()]);
    }

    /// H1 — a .lua always declares its content depots (Windows, macOS,
    /// Linux); they are not DLCs. Only the two DLC sections feed
    /// `dlc_app_ids`, so a depot-only game has nothing to show offline.
    #[test]
    fn parse_lua_separates_dlcs_from_content_depots() {
        let text = "-- AppID 3949040 | Generated on 2026-07-29 20:30 UTC | LuaVault\n\
-- Main Application\n\
addappid(3949040)\n\
-- Content Depots\n\
addappid(3949041, 1, \"aaa\") -- RV There Yet? - Windows\n\
addappid(3949042, 1, \"bbb\") -- RV There Yet? - Linux\n\
-- DLCs with Content (1)\n\
addappid(4000000, 1, \"ccc\")\n\
-- DLCs without Dedicated Depots (1)\n\
addappid(4000001)\n";
        let parsed = parse_lua(text);
        assert_eq!(
            parsed.app_ids,
            vec!["3949040", "3949041", "3949042", "4000000", "4000001"]
        );
        assert_eq!(
            parsed.dlc_app_ids,
            vec!["4000000".to_string(), "4000001".to_string()]
        );
    }

    #[test]
    fn parse_lua_without_headers_asserts_no_dlc() {
        // No section headers: every call still feeds `app_ids`, but nothing
        // can be claimed to be a DLC.
        let parsed = parse_lua("addappid(1)\naddappid(2, 1, \"k\")\n");
        assert_eq!(parsed.app_ids, vec!["1".to_string(), "2".to_string()]);
        assert!(parsed.dlc_app_ids.is_empty());
    }

    /// Disk-level guarantees of the read-only view: a missing file and a
    /// binary file both yield an empty structure, never an error or a panic.
    #[test]
    fn read_lua_contents_never_errors() {
        let lib = scratch("lua_contents");
        std::fs::create_dir_all(&lib).unwrap();

        // Missing file → empty view.
        assert_eq!(read_lua_contents(&lib, "42"), LuaContents::default());

        // Binary file (invalid UTF-8) → read_to_string fails → empty view.
        std::fs::write(lib.join("42.lua"), [0u8, 1, 2, 255, 254, 0, 128]).unwrap();
        assert_eq!(read_lua_contents(&lib, "42"), LuaContents::default());

        // A real file round-trips.
        std::fs::write(lib.join("43.lua"), "addappid(43)\n").unwrap();
        assert_eq!(
            read_lua_contents(&lib, "43").app_ids,
            vec!["43".to_string()]
        );

        let _ = std::fs::remove_dir_all(&lib);
    }

    // ── D4: the header count caps the DLC list ──────────────────

    /// `2416450.lua`: `-- DLCs with Content (1)` declares the DLC AND its
    /// dedicated depot. The count (1) must keep only the first AppID.
    #[test]
    fn parse_lua_header_count_excludes_dedicated_depots() {
        let text = "-- Main Application\n\
addappid(2416450)\n\
-- DLCs with Content (1)\n\
addappid(3879490, 1, \"aaa\")\n\
addappid(3879491, 1, \"bbb\")\n";
        let parsed = parse_lua(text);
        assert_eq!(
            parsed.app_ids,
            vec!["2416450", "3879490", "3879491"]
        );
        // Count says 1 → only the first call is a DLC; the second is its depot.
        assert_eq!(parsed.dlc_app_ids, vec!["3879490".to_string()]);
    }

    /// No count in the header → take everything (over-count, never lose).
    #[test]
    fn parse_lua_without_count_takes_all_in_section() {
        let text = "-- DLCs with Content\n\
addappid(100)\n\
addappid(101)\n\
addappid(102)\n";
        let parsed = parse_lua(text);
        assert_eq!(
            parsed.dlc_app_ids,
            vec!["100".to_string(), "101".to_string(), "102".to_string()]
        );
    }

    // ── D5: Shared Depots and unknown sections ──────────────────

    /// `-- Shared Depots` after a DLC section must end the DLC section.
    #[test]
    fn parse_lua_shared_depots_ends_dlc_section() {
        let text = "-- DLCs with Content (1)\n\
addappid(100)\n\
-- Shared Depots\n\
addappid(101)\n\
addappid(102)\n";
        let parsed = parse_lua(text);
        assert_eq!(
            parsed.app_ids,
            vec!["100", "101", "102"]
        );
        // 101 and 102 are shared depots, not DLCs.
        assert_eq!(parsed.dlc_app_ids, vec!["100".to_string()]);
    }

    /// An unrecognised section header is never DLC (D5 — "par défaut, on
    /// n'affirme rien").
    #[test]
    fn parse_lua_unknown_section_is_not_dlc() {
        let text = "-- DLCs with Content (1)\n\
addappid(100)\n\
-- Some Future Section\n\
addappid(200)\n";
        let parsed = parse_lua(text);
        assert_eq!(parsed.app_ids, vec!["100", "200"]);
        assert_eq!(parsed.dlc_app_ids, vec!["100".to_string()]);
    }

    // ── D6: parser hardening, one test per measured case ────────

    #[test]
    fn parse_lua_escaped_quotes_inside_string() {
        // print("dis \"addappid(5)\" ok") → the call is inside the string.
        let parsed = parse_lua("print(\"dis \\\"addappid(5)\\\" ok\")\n");
        assert!(parsed.app_ids.is_empty());
    }

    #[test]
    fn parse_lua_escaped_quote_does_not_end_string() {
        // addappid(1, 1, "a\"b") addappid(2) → both calls are real.
        let parsed = parse_lua("addappid(1, 1, \"a\\\"b\") addappid(2)\n");
        assert_eq!(parsed.app_ids, vec!["1".to_string(), "2".to_string()]);
    }

    #[test]
    fn parse_lua_double_dash_inside_string() {
        // addappid(1, 1, "a--b") addappid(2) → "--" inside the string
        // must not truncate the line.
        let parsed = parse_lua("addappid(1, 1, \"a--b\") addappid(2)\n");
        assert_eq!(parsed.app_ids, vec!["1".to_string(), "2".to_string()]);
    }

    #[test]
    fn parse_lua_long_string_brackets() {
        // local s = [[ addappid(999) ]] → long string, not a call.
        let parsed = parse_lua("local s = [[ addappid(999) ]]\n");
        assert!(parsed.app_ids.is_empty());
    }

    #[test]
    fn parse_lua_block_comment_multiline() {
        // --[[\naddappid(999)\n]] → block comment spanning three lines.
        let parsed = parse_lua("--[[\naddappid(999)\n]]\n");
        assert!(parsed.app_ids.is_empty());
    }

    #[test]
    fn parse_lua_dot_and_colon_are_boundaries() {
        // t.addappid(5) and t:addappid(5) are method calls, not declarations.
        let parsed = parse_lua("t.addappid(5)\n");
        assert!(parsed.app_ids.is_empty());
        let parsed = parse_lua("t:addappid(5)\n");
        assert!(parsed.app_ids.is_empty());
    }

    // ── Concurrent read/write integrity (LOT-21-fix04) ──────────

    /// Readers never see a broken index when `load_index_with_data_dir`
    /// and `save_index_with_data_dir` are called concurrently by multiple
    /// threads.
    ///
    /// This test calls the **real** functions (not a hand-rolled
    /// re-implementation) and counts every `Err` from a reader in a
    /// shared `AtomicUsize`.  With the `INDEX_GUARD` in place the counter
    /// stays at zero; removing the guard makes readers pick up partially
    /// written files and the assertion fails.
    #[test]
    fn concurrent_readers_never_see_a_broken_index() {
        let _lock = cache_lock();
        clear_index_cache();
        set_index_settle(0);

        let root = scratch("conc_readers");
        let data = root.join("data");
        let lib = root.join("lib");
        std::fs::create_dir_all(&lib).unwrap();

        // Seed a valid index with one entry.
        let entry = LibraryEntry {
            app_id: "1".to_string(),
            name: "Game1".to_string(),
            icon: None,
            file_name: "1.lua".to_string(),
            added_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
            has_fix: false,
            hidden: false,
            tags: Vec::new(),
        };
        save_index_with_data_dir(&lib, &data, &[entry]).unwrap();

        let iterations: usize = 500;
        let n_readers: usize = 4;
        let n_writers: usize = 4;
        let total_threads = n_readers + n_writers;

        let erreurs = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut handles = Vec::with_capacity(total_threads);

        // Readers: each calls `load_index_with_data_dir` `iterations` times.
        for _ in 0..n_readers {
            let lib = lib.clone();
            let data = data.clone();
            let errs = erreurs.clone();
            handles.push(std::thread::spawn(move || {
                for _ in 0..iterations {
                    if load_index_with_data_dir(&lib, &data).is_err() {
                        errs.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    }
                }
            }));
        }

        // Writers: each rotates the index content.
        for w in 0..n_writers {
            let lib = lib.clone();
            let data = data.clone();
            handles.push(std::thread::spawn(move || {
                for i in 0..iterations {
                    let entry = LibraryEntry {
                        app_id: format!("app_w{w}_{i}"),
                        name: format!("Game_w{w}_{i}"),
                        icon: None,
                        file_name: format!("{w}_{i}.lua"),
                        added_at: "2024-01-01T00:00:00Z".to_string(),
                        updated_at: "2024-01-01T00:00:00Z".to_string(),
                        has_fix: false,
                        hidden: false,
                        tags: Vec::new(),
                    };
                    let _ = save_index_with_data_dir(&lib, &data, &[entry]);
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        let errs = erreurs.load(std::sync::atomic::Ordering::SeqCst);
        assert_eq!(
            errs, 0,
            "readers saw {errs} broken index reads during concurrent writes"
        );

        let _ = std::fs::remove_dir_all(&root);
    }
}
