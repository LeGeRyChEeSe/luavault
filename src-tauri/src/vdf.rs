//! Minimal Valve KeyValues (VDF/ACF) reader — just enough to answer
//! "is this AppID actually installed, and where?", and (LOT-13) "how long
//! was this game played, locally, with no network?".
//!
//! Three caching layers protect against repeated disk reads:
//!
//! 1. `steamapps_dirs` is cached with a 10-second TTL — library folders rarely change.
//! 2. `appmanifest_*.acf` parse results are cached with a [`FileStamp`] so that
//!    when Steam rewrites a manifest, the cache automatically invalidates.
//! 3. `localconfig.vdf` playtime parses are cached the same way — Steam
//!    rewrites the file on exit, which invalidates the entry by itself.

use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use crate::cache::{FileStamp, StampedCache, TtlCache};

/// StateFlags bit set by Steam once a game is fully installed.
const STATE_FULLY_INSTALLED: u64 = 4;

#[derive(Debug, Clone, Serialize, Default)]
pub struct GameInstall {
    pub app_id: String,
    /// An appmanifest_*.acf was found for this AppID.
    pub known_to_steam: bool,
    /// The `common\<installdir>` folder exists on disk.
    pub installed: bool,
    /// StateFlags reports a complete install (not mid-download / update-required).
    pub fully_installed: bool,
    pub install_dir: Option<String>,
    pub steam_name: Option<String>,
    pub state_flags: u64,
    pub size_on_disk: u64,
}

// ================================================================ Caches

/// Cached result of `steamapps_dirs` keyed by the Steam directory path.
/// TTL of 10 seconds — new libraries are rare events.
static STEAMAPPS_DIRS_CACHE: LazyLock<TtlCache<PathBuf, Vec<PathBuf>>> =
    LazyLock::new(|| TtlCache::new(std::time::Duration::from_secs(10)));

/// Cached parse results for `appmanifest_<id>.acf` files.
/// Key = full path to the manifest; value = key-value pairs.
static MANIFEST_CACHE: LazyLock<StampedCache<PathBuf, HashMap<String, String>>> =
    LazyLock::new(StampedCache::new);

/// Clear all caches. Used by tests to avoid cross-test pollution, and by
/// `set_steam_dir` / a Steam restart — Steam rewrites its VDF files there,
/// `localconfig.vdf` (playtime) included.
pub fn clear_caches() {
    STEAMAPPS_DIRS_CACHE.clear();
    MANIFEST_CACHE.clear();
    PLAYTIME_CACHE.clear();
    LOGINUSERS_CACHE.clear();
}

#[cfg(test)]
fn set_manifest_settle(n: u64) {
    MANIFEST_CACHE.set_settle_nanos(n);
}

// ================================================================ Parsing

/// Flatten a KeyValues document into `key -> value` pairs.
///
/// Nesting is ignored on purpose: the keys we care about (`path`, `installdir`,
/// `StateFlags`, …) are unambiguous inside the files we read. Keys are lowercased
/// because Valve is inconsistent about casing across client versions.
fn parse_pairs(raw: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut chars = raw.chars().peekable();
    let mut tokens: Vec<String> = Vec::new();

    while let Some(c) = chars.next() {
        match c {
            '"' => {
                let mut token = String::new();
                while let Some(c) = chars.next() {
                    match c {
                        '\\' => {
                            if let Some(escaped) = chars.next() {
                                token.push(match escaped {
                                    'n' => '\n',
                                    't' => '\t',
                                    other => other,
                                });
                            }
                        }
                        '"' => break,
                        other => token.push(other),
                    }
                }
                tokens.push(token);
                if tokens.len() == 2 {
                    // A quoted pair on the same logical line = key/value.
                    let value = tokens.pop().unwrap();
                    let key = tokens.pop().unwrap();
                    out.push((key.to_lowercase(), value));
                }
            }
            '{' | '}' => tokens.clear(),
            '/' if chars.peek() == Some(&'/') => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

fn pairs_map(raw: &str) -> HashMap<String, String> {
    parse_pairs(raw).into_iter().collect()
}

// ======================================== Playtime — localconfig.vdf (LOT-13)
//
// Steam records per-game playtime locally, in
// `{Steam}\userdata\<AccountID>\config\localconfig.vdf`:
//
// "UserLocalConfigStore" { "Software" { "Valve" { "Steam" { "apps" {
//     "<AppID>" { "LastPlayed" "<unix s>"  "Playtime" "<minutes>" ... }
// } } } } }
//
// Two traps shape this code:
// - `Playtime` appears once per AppID. Flattening the document the way
//   `parse_pairs` does throws away the association between a game and its
//   time (the last value would win, and every game would show somebody
//   else's hours). The reader here is nesting-aware.
// - `cloud` / `autocloud` sub-blocks carry their own timestamps
//   (`lastlaunch`, `lastexit`): only the DIRECT pairs of each AppID block
//   are read, never what sits below it.
//
// `userdata` is read-only, absolutely: this code reads, never writes.

/// SteamID64 base: `AccountID32 = SteamID64 - STEAMID64_BASE`. The
/// `userdata` folder is named after the 32-bit AccountID.
const STEAMID64_BASE: u64 = 76_561_197_960_265_728;

/// One game's playtime as Steam records it locally.
///
/// `None` means "Steam writes no such key" — never a zero. An app entry can
/// carry `LastPlayed` without `Playtime`, or neither (e.g. only a `cloud`
/// block): all of it is legitimate and must not be displayed as a measured
/// zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct PlaytimeRecord {
    /// `Playtime` key, in minutes.
    pub minutes: Option<u64>,
    /// `LastPlayed` key, in Unix seconds.
    pub last_played: Option<u64>,
}

/// Cached parse of `localconfig.vdf`, keyed by file path (one per account).
static PLAYTIME_CACHE: LazyLock<StampedCache<PathBuf, HashMap<String, PlaytimeRecord>>> =
    LazyLock::new(StampedCache::new);

/// Cached resolution of the signed-in account's `userdata` folder, keyed by
/// the `loginusers.vdf` path — Steam rewrites it at every login.
static LOGINUSERS_CACHE: LazyLock<StampedCache<PathBuf, PathBuf>> =
    LazyLock::new(StampedCache::new);

#[cfg(test)]
fn set_playtime_settle(n: u64) {
    PLAYTIME_CACHE.set_settle_nanos(n);
}

#[cfg(test)]
fn set_loginusers_settle(n: u64) {
    LOGINUSERS_CACHE.set_settle_nanos(n);
}

#[derive(Debug, Clone, PartialEq)]
enum KvToken {
    Str(String),
    Open,
    Close,
}

/// Tokenize a KeyValues document while keeping its structure: quoted strings
/// plus the braces delimiting blocks. Same escape and `//` comment handling
/// as `parse_pairs`.
fn tokenize_kv(raw: &str) -> Vec<KvToken> {
    let mut tokens = Vec::new();
    let mut chars = raw.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '"' => {
                let mut token = String::new();
                while let Some(c) = chars.next() {
                    match c {
                        '\\' => {
                            if let Some(escaped) = chars.next() {
                                token.push(match escaped {
                                    'n' => '\n',
                                    't' => '\t',
                                    other => other,
                                });
                            }
                        }
                        '"' => break,
                        other => token.push(other),
                    }
                }
                tokens.push(KvToken::Str(token));
            }
            '{' => tokens.push(KvToken::Open),
            '}' => tokens.push(KvToken::Close),
            '/' if chars.peek() == Some(&'/') => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    tokens
}

#[derive(Debug, Clone, PartialEq)]
enum KvNode {
    Value(String),
    Block(Vec<(String, KvNode)>),
}

/// Parse a whole document into a tree of `key → value / block` entries.
fn parse_kv_tree(raw: &str) -> Vec<(String, KvNode)> {
    let tokens = tokenize_kv(raw);
    let mut pos = 0usize;
    parse_entries(&tokens, &mut pos)
}

/// Parse `key value` / `key { ... }` entries until the matching `Close`
/// (or the end of input at the top level).
fn parse_entries(tokens: &[KvToken], pos: &mut usize) -> Vec<(String, KvNode)> {
    let mut entries = Vec::new();
    while let Some(token) = tokens.get(*pos) {
        match token {
            KvToken::Str(key) => {
                let key = key.clone();
                *pos += 1;
                match tokens.get(*pos) {
                    Some(KvToken::Open) => {
                        *pos += 1;
                        let block = parse_entries(tokens, pos);
                        entries.push((key, KvNode::Block(block)));
                    }
                    Some(KvToken::Str(value)) => {
                        *pos += 1;
                        entries.push((key, KvNode::Value(value.clone())));
                    }
                    // A key with neither value nor block — skip it.
                    _ => {}
                }
            }
            KvToken::Close => {
                *pos += 1;
                break;
            }
            KvToken::Open => {
                // A block without a key — malformed, skip its contents.
                *pos += 1;
                parse_entries(tokens, pos);
            }
        }
    }
    entries
}

/// The block child registered under `key` (case-insensitive), if any.
fn child_block<'a>(
    entries: &'a [(String, KvNode)],
    key: &str,
) -> Option<&'a [(String, KvNode)]> {
    entries.iter().find_map(|(k, node)| match node {
        KvNode::Block(block) if k.eq_ignore_ascii_case(key) => Some(block.as_slice()),
        _ => None,
    })
}

/// Nesting-aware read of a `localconfig.vdf` document: every direct child of
/// the `apps` block becomes an AppID → record pair, reading only the child's
/// OWN pairs — `cloud` and `autocloud` sub-blocks carry timestamps
/// (`lastlaunch`, `lastexit`) that must never be confused with `LastPlayed`.
pub fn parse_playtimes(raw: &str) -> HashMap<String, PlaytimeRecord> {
    let tree = parse_kv_tree(raw);
    // The document root is usually a lone `UserLocalConfigStore` block;
    // accept both that and a bare tree.
    let root = child_block(&tree, "userlocalconfigstore").unwrap_or(&tree);
    let Some(apps) = child_block(root, "software")
        .and_then(|software| child_block(software, "valve"))
        .and_then(|valve| child_block(valve, "steam"))
        .and_then(|steam| child_block(steam, "apps"))
    else {
        return HashMap::new();
    };

    let mut out = HashMap::new();
    for (app_id, node) in apps {
        let KvNode::Block(entries) = node else {
            continue;
        };
        let mut record = PlaytimeRecord {
            minutes: None,
            last_played: None,
        };
        for (key, value) in entries {
            // Direct string pairs only: sub-blocks (cloud, autocloud, ...)
            // are deliberately not descended into.
            let KvNode::Value(value) = value else {
                continue;
            };
            match key.to_lowercase().as_str() {
                "playtime" => record.minutes = value.parse().ok(),
                "lastplayed" => record.last_played = value.parse().ok(),
                _ => {}
            }
        }
        out.insert(app_id.clone(), record);
    }
    out
}

/// The SteamID64 of the most recently used account in a `loginusers.vdf`
/// document. Prefers the entry flagged `MostRecent`; ties (or no flag at
/// all) fall back to the highest `Timestamp`, then the highest SteamID64 so
/// the choice never flips between two reads of the same file.
pub fn parse_most_recent_login(raw: &str) -> Option<u64> {
    let tree = parse_kv_tree(raw);
    // The file is named loginusers.vdf but Steam writes `"users"` as the root
    // key — measured on a real install. Accepting only "loginusers" made this
    // return None for every real account while every fixture passed.
    let users = child_block(&tree, "users")
        .or_else(|| child_block(&tree, "loginusers"))
        .unwrap_or(&tree);
    let mut best: Option<(bool, u64, u64)> = None; // (most_recent, timestamp, steamid64)
    for (key, node) in users {
        let KvNode::Block(fields) = node else {
            continue;
        };
        // Pitfall 27: one malformed entry must not abort the whole scan.
        let Ok(steamid64) = key.parse::<u64>() else {
            continue;
        };
        let mut most_recent = false;
        let mut timestamp = 0u64;
        for (field, value) in fields {
            let KvNode::Value(value) = value else {
                continue;
            };
            match field.to_lowercase().as_str() {
                "mostrecent" => most_recent = value == "1",
                "timestamp" => timestamp = value.parse().unwrap_or(0),
                _ => {}
            }
        }
        let candidate = (most_recent, timestamp, steamid64);
        let better = match best {
            Some(current) => candidate > current,
            None => true,
        };
        if better {
            best = Some(candidate);
        }
    }
    best.map(|(_, _, steamid64)| steamid64)
}

/// The 32-bit AccountID that names the `userdata` folder of a SteamID64.
pub fn account_id_from_steamid64(steamid64: u64) -> Option<u64> {
    steamid64.checked_sub(STEAMID64_BASE)
}

/// The `userdata` folder of the signed-in account, resolved from
/// `loginusers.vdf`. Fallback when the file is absent or yields nothing
/// usable: a lone `userdata` folder is unambiguous — take it; several are
/// not — show nothing rather than another account's playtime.
fn userdata_dir(steam: &Path) -> Option<PathBuf> {
    if let Some(dir) = loginusers_account_dir(steam) {
        return Some(dir);
    }
    let root = steam.join("userdata");
    let Ok(entries) = std::fs::read_dir(&root) else {
        return None;
    };
    let dirs: Vec<PathBuf> = entries
        .flatten()
        .filter(|entry| entry.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|entry| entry.path())
        .collect();
    match dirs.len() {
        1 => dirs.into_iter().next(),
        _ => None,
    }
}

fn loginusers_account_dir(steam: &Path) -> Option<PathBuf> {
    let loginusers = steam.join("config").join("loginusers.vdf");
    let stamp = FileStamp::from_path(&loginusers)?;
    if let Some(cached) = LOGINUSERS_CACHE.get(&loginusers, &stamp) {
        return Some(cached);
    }
    let raw = std::fs::read_to_string(&loginusers).ok()?;
    let dir = parse_most_recent_login(&raw)
        .and_then(account_id_from_steamid64)
        .map(|account_id| steam.join("userdata").join(account_id.to_string()))
        .filter(|dir| dir.is_dir())?;
    LOGINUSERS_CACHE.put(loginusers, stamp, dir.clone());
    Some(dir)
}

/// Every AppID's playtime for the signed-in account, cached on the
/// `localconfig.vdf` stamp — Steam rewrites the file periodically and on
/// exit, which invalidates the entry by itself. A read failure is never
/// cached, and neither is an empty result obtained by error.
fn playtimes_map(steam: &Path) -> Option<HashMap<String, PlaytimeRecord>> {
    let file = userdata_dir(steam)?.join("config").join("localconfig.vdf");
    let stamp = FileStamp::from_path(&file)?;
    if let Some(cached) = PLAYTIME_CACHE.get(&file, &stamp) {
        return Some(cached);
    }
    let raw = std::fs::read_to_string(&file).ok()?;
    let map = parse_playtimes(&raw);
    PLAYTIME_CACHE.put(file, stamp, map.clone());
    Some(map)
}

/// "For this AppID, how many minutes were played, and when was the last
/// session?" — `None` when nothing can be known (no Steam, unreadable file,
/// ambiguous account, or the AppID absent from the account's data). Pure
/// local file reads: no network, ever.
pub fn playtime_for(steam: &Path, app_id: &str) -> Option<PlaytimeRecord> {
    playtimes_map(steam).and_then(|map| map.get(app_id).copied())
}

// ================================================================ Public API

/// Every `steamapps` folder Steam knows about (main install + extra libraries).
///
/// Results are cached with a 10-second TTL keyed by the Steam directory path.
pub fn steamapps_dirs(steam: &Path) -> Vec<PathBuf> {
    let steam_buf = steam.to_path_buf();
    if let Some(cached) = STEAMAPPS_DIRS_CACHE.get(&steam_buf) {
        return cached;
    }

    let mut dirs = vec![steam_buf.join("steamapps")];

    for candidate in [
        steam_buf.join("steamapps").join("libraryfolders.vdf"),
        steam_buf.join("config").join("libraryfolders.vdf"),
    ] {
        let Ok(raw) = std::fs::read_to_string(&candidate) else {
            continue;
        };
        for (key, value) in parse_pairs(&raw) {
            if key == "path" {
                let dir = PathBuf::from(value).join("steamapps");
                if dir.is_dir() && !dirs.contains(&dir) {
                    dirs.push(dir);
                }
            }
        }
    }
    dirs.retain(|d| d.is_dir());

    // Only cache non-empty results — an empty vector can happen when Steam
    // is not yet installed or the library folder has not been created.
    // Caching it would block all games in `needs_steam_install` for 10 s.
    if !dirs.is_empty() {
        STEAMAPPS_DIRS_CACHE.put(steam_buf.clone(), dirs.clone());
    }
    dirs
}

/// Canonical form of an existing directory: real casing, backslashes, and no
/// `\\?\` prefix. Steam (and the registry) sometimes hand us a path in lowercase
/// and/or with forward slashes; canonicalizing makes `e:/applications/steam` and
/// `E:\Applications\Steam` the same folder, so we never exclude it twice.
fn clean_dir(path: &Path) -> PathBuf {
    match std::fs::canonicalize(path) {
        Ok(canonical) => {
            let as_string = canonical.display().to_string();
            PathBuf::from(as_string.strip_prefix(r"\\?\").unwrap_or(&as_string))
        }
        Err(_) => path.to_path_buf(),
    }
}

/// Every `steamapps\common` folder that exists across the Steam libraries —
/// i.e. wherever games are actually installed. Excluding these from the
/// antivirus covers every current and future game in one rule. Paths are
/// canonicalized and de-duplicated case-insensitively.
pub fn common_dirs(steam: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for dir in steamapps_dirs(steam) {
        let common = clean_dir(&dir.join("common"));
        if !common.is_dir() {
            continue;
        }
        let key = common.display().to_string().to_lowercase();
        if !out
            .iter()
            .any(|existing| existing.display().to_string().to_lowercase() == key)
        {
            out.push(common);
        }
    }
    out
}

/// Resolve where an AppID is installed, across every Steam library folder.
///
/// Results are cached per manifest file path using a [`FileStamp`].
/// If Steam rewrites the manifest, the cache automatically invalidates
/// because the file's `mtime` or size will change.
pub fn locate_game(steam: &Path, app_id: &str) -> GameInstall {
    let mut result = GameInstall {
        app_id: app_id.to_string(),
        ..Default::default()
    };

    for steamapps in steamapps_dirs(steam) {
        let manifest = steamapps.join(format!("appmanifest_{app_id}.acf"));
        if !manifest.is_file() {
            continue;
        }

        let stamp = FileStamp::from_path(&manifest);
        // Try the cache first.
        if let Some(cached_map) = stamp.as_ref().and_then(|s| MANIFEST_CACHE.get(&manifest, s)) {
            let map = &cached_map;
            result.known_to_steam = true;
            result.steam_name = map.get("name").cloned();
            result.state_flags = map
                .get("stateflags")
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(0);
            result.size_on_disk = map
                .get("sizeondisk")
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(0);

            if let Some(install_dir) = map.get("installdir") {
                let path = steamapps.join("common").join(install_dir);
                if path.is_dir() {
                    result.installed = true;
                    result.fully_installed = result.state_flags & STATE_FULLY_INSTALLED != 0;
                    result.install_dir = Some(path.display().to_string());
                    return result;
                }
            }
            continue;
        }

        // Cache miss — read and parse the file.
        let Ok(raw) = std::fs::read_to_string(&manifest) else {
            continue;
        };
        let map = pairs_map(&raw);

        // Populate the cache with the current stamp.
        if let Some(s) = stamp.clone() {
            MANIFEST_CACHE.put(manifest.clone(), s, map.clone());
        }

        result.known_to_steam = true;
        result.steam_name = map.get("name").cloned();
        result.state_flags = map
            .get("stateflags")
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0);
        result.size_on_disk = map
            .get("sizeondisk")
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0);

        if let Some(install_dir) = map.get("installdir") {
            let path = steamapps.join("common").join(install_dir);
            if path.is_dir() {
                result.installed = true;
                result.fully_installed = result.state_flags & STATE_FULLY_INSTALLED != 0;
                result.install_dir = Some(path.display().to_string());
                return result;
            }
        }
    }
    result
}

pub fn game_dir(steam: &Path, app_id: &str) -> Option<PathBuf> {
    locate_game(steam, app_id).install_dir.map(PathBuf::from)
}

// ================================================================ Tests

#[cfg(test)]
mod tests {
    use super::*;

    // Serialise all tests touching `MANIFEST_CACHE` so counters and cache
    // state are not disturbed by parallel tests. `into_inner()` survives panics.
    static CACHE_GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn cache_lock() -> std::sync::MutexGuard<'static, ()> {
        CACHE_GUARD.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ast_vdf_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn parses_pairs_and_ignores_nesting() {
        let raw = r#"
        "libraryfolders"
        {
            "0"
            {
                "path"      "C:\\Program Files (x86)\\Steam"
                "label"     ""
                "apps" { "220" "1234" }
            }
            // a comment
            "1" { "path" "D:\\SteamLibrary" }
        }"#;
        let paths: Vec<_> = parse_pairs(raw)
            .into_iter()
            .filter(|(k, _)| k == "path")
            .map(|(_, v)| v)
            .collect();
        assert_eq!(paths, vec![r"C:\Program Files (x86)\Steam", r"D:\SteamLibrary"]);
    }

    #[test]
    fn locates_installed_game_in_secondary_library() {
        let _lock = cache_lock();
        clear_caches();
        let steam = scratch("steam");
        let extra = scratch("extra");
        std::fs::create_dir_all(steam.join("steamapps")).unwrap();
        std::fs::create_dir_all(extra.join("steamapps").join("common").join("Subnautica")).unwrap();
        std::fs::write(
            steam.join("steamapps").join("libraryfolders.vdf"),
            format!(
                "\"libraryfolders\"\n{{\n\"0\" {{ \"path\" \"{}\" }}\n}}",
                extra.display().to_string().replace('\\', "\\\\")
            ),
        )
        .unwrap();
        std::fs::write(
            extra.join("steamapps").join("appmanifest_264710.acf"),
            "\"AppState\" { \"appid\" \"264710\" \"name\" \"Subnautica\" \"StateFlags\" \"4\" \"installdir\" \"Subnautica\" \"SizeOnDisk\" \"1234\" }",
        )
        .unwrap();

        let found = locate_game(&steam, "264710");
        assert!(found.known_to_steam);
        assert!(found.installed);
        assert!(found.fully_installed);
        assert_eq!(found.steam_name.as_deref(), Some("Subnautica"));
        assert_eq!(found.size_on_disk, 1234);
        assert!(found.install_dir.unwrap().ends_with("Subnautica"));

        let missing = locate_game(&steam, "999999");
        assert!(!missing.known_to_steam);
        assert!(!missing.installed);

        let _ = std::fs::remove_dir_all(&steam);
        let _ = std::fs::remove_dir_all(&extra);
    }

    #[test]
    fn downloading_game_is_known_but_not_fully_installed() {
        let _lock = cache_lock();
        clear_caches();
        let steam = scratch("dl");
        let common = steam.join("steamapps").join("common").join("HalfInstalled");
        std::fs::create_dir_all(&common).unwrap();
        std::fs::write(
            steam.join("steamapps").join("appmanifest_42.acf"),
            "\"AppState\" { \"StateFlags\" \"1026\" \"installdir\" \"HalfInstalled\" }",
        )
        .unwrap();

        let found = locate_game(&steam, "42");
        assert!(found.installed);
        assert!(!found.fully_installed);

        let _ = std::fs::remove_dir_all(&steam);
    }

    #[test]
    fn deleted_manifest_is_reported_as_unknown() {
        let _lock = cache_lock();
        clear_caches();
        let steam = scratch("cache_hit");
        let extra = scratch("extra_cache");
        std::fs::create_dir_all(steam.join("steamapps")).unwrap();
        std::fs::create_dir_all(extra.join("steamapps").join("common").join("Subnautica")).unwrap();
        std::fs::write(
            steam.join("steamapps").join("libraryfolders.vdf"),
            format!(
                "\"libraryfolders\"\n{{\n\"0\" {{ \"path\" \"{}\" }}\n}}",
                extra.display().to_string().replace('\\', "\\\\")
            ),
        )
        .unwrap();
        std::fs::write(
            extra.join("steamapps").join("appmanifest_264710.acf"),
            "\"AppState\" { \"appid\" \"264710\" \"name\" \"Subnautica\" \"StateFlags\" \"4\" \"installdir\" \"Subnautica\" \"SizeOnDisk\" \"9999\" }",
        )
        .unwrap();

        let first = locate_game(&steam, "264710");
        assert!(first.known_to_steam);
        assert_eq!(first.size_on_disk, 9999);

        // Delete the manifest file — the stamp will no longer match,
        // so the cache must NOT serve the stale value.
        std::fs::remove_file(
            extra.join("steamapps").join("appmanifest_264710.acf"),
        )
        .unwrap();

        let second = locate_game(&steam, "264710");
        assert!(!second.known_to_steam);
        assert!(!second.installed);

        let _ = std::fs::remove_dir_all(&steam);
        let _ = std::fs::remove_dir_all(&extra);
    }

    /// Freshness regression test: after rewriting the manifest with different
    /// StateFlags and a different file size, `locate_game` must see the change.
    #[test]
    fn cache_invalidates_when_manifest_is_rewritten() {
        let _lock = cache_lock();
        clear_caches();
        let steam = scratch("freshness");
        let common = steam.join("steamapps").join("common").join("Subnautica");
        std::fs::create_dir_all(&common).unwrap();

        // Initial manifest: fully installed (StateFlags=4), 100 bytes.
        std::fs::write(
            steam.join("steamapps").join("appmanifest_264710.acf"),
            "\"AppState\" { \"appid\" \"264710\" \"name\" \"Subnautica\" \"StateFlags\" \"4\" \"installdir\" \"Subnautica\" \"SizeOnDisk\" \"100\" }",
        )
        .unwrap();

        let first = locate_game(&steam, "264710");
        assert!(first.fully_installed);
        assert_eq!(first.size_on_disk, 100);

        // Rewrite manifest: not fully installed (StateFlags=1), larger payload.
        std::fs::write(
            steam.join("steamapps").join("appmanifest_264710.acf"),
            "\"AppState\" { \"appid\" \"264710\" \"name\" \"Subnautica\" \"StateFlags\" \"1\" \"installdir\" \"Subnautica\" \"SizeOnDisk\" \"200\" \"buildid\" \"123456\" }",
        )
        .unwrap();

        let second = locate_game(&steam, "264710");
        assert!(!second.fully_installed);
        assert_eq!(second.size_on_disk, 200);

        let _ = std::fs::remove_dir_all(&steam);
    }

    /// Prove that the cache branch of `locate_game` is exercised and returns
    /// identical data to the disk branch.
    #[test]
    fn locate_game_cache_hit_returns_same_data() {
        let _lock = cache_lock();
        clear_caches();
        set_manifest_settle(0); // neutralise la fenetre de garde
        let steam = scratch("cache_hit_eq");
        let extra = scratch("extra_cache_eq");
        std::fs::create_dir_all(steam.join("steamapps")).unwrap();
        std::fs::create_dir_all(
            extra
                .join("steamapps")
                .join("common")
                .join("Subnautica"),
        )
        .unwrap();
        std::fs::write(
            steam.join("steamapps").join("libraryfolders.vdf"),
            format!(
                "\"libraryfolders\"\n{{\n\"0\" {{ \"path\" \"{}\" }}\n}}",
                extra.display().to_string().replace('\\', "\\\\")
            ),
        )
        .unwrap();
        std::fs::write(
            extra.join("steamapps").join("appmanifest_264710.acf"),
            "\"AppState\" { \"appid\" \"264710\" \"name\" \"Subnautica\" \"StateFlags\" \"4\" \"installdir\" \"Subnautica\" \"SizeOnDisk\" \"5000\" }",
        )
        .unwrap();

        // Reset the hit counter right before the two calls — no other test
        // can interfere because we hold `hit_lock`.
        MANIFEST_CACHE.reset_hits();

        // First call — disk read.
        let first = locate_game(&steam, "264710");
        assert!(first.known_to_steam);
        assert!(first.installed);
        assert!(first.fully_installed);
        assert_eq!(first.steam_name.as_deref(), Some("Subnautica"));
        assert_eq!(first.size_on_disk, 5000);
        assert!(first.install_dir.is_some());

        // Second call — must come from cache.
        let second = locate_game(&steam, "264710");
        assert_eq!(
            MANIFEST_CACHE.hits(),
            1,
            "second locate_game must hit the cache"
        );

        // Every field must match exactly.
        assert_eq!(first.app_id, second.app_id);
        assert_eq!(first.known_to_steam, second.known_to_steam);
        assert_eq!(first.installed, second.installed);
        assert_eq!(first.fully_installed, second.fully_installed);
        assert_eq!(first.install_dir, second.install_dir);
        assert_eq!(first.steam_name, second.steam_name);
        assert_eq!(first.state_flags, second.state_flags);
        assert_eq!(first.size_on_disk, second.size_on_disk);

        let _ = std::fs::remove_dir_all(&steam);
        let _ = std::fs::remove_dir_all(&extra);
    }

    // ============================================ LOT-13 — playtime tests

    /// The brief's excerpt, verbatim: one full record, one `LastPlayed`
    /// without `Playtime`, one entry carrying nothing but a `cloud` block.
    const LOCALCONFIG_EXCERPT: &str = r#"
"UserLocalConfigStore"
{
    "Software" { "Valve" { "Steam" { "apps"
    {
        "1172620"
        {
            "LastPlayed"    "1712725190"
            "Playtime"      "217"
        }
        "2879840"
        {
            "LastPlayed"    "1764627784"
            "cloud" { "last_sync_state" "synchronized" }
        }
        "241100"
        {
            "cloud" { "last_sync_state" "synchronized" }
        }
    } } } }
}
"#;

    /// `Playtime` appears under EACH AppID: flattening the document would
    /// attribute one game's time to another (or keep only the last value).
    /// Every AppID must keep exactly its own pairs.
    #[test]
    fn playtimes_scopes_each_appid_to_its_own_pairs() {
        let map = parse_playtimes(LOCALCONFIG_EXCERPT);
        assert_eq!(map.len(), 3, "trois jeux dans l'extrait, pas plus");

        let full = map.get("1172620").expect("1172620 doit exister");
        assert_eq!(full.minutes, Some(217));
        assert_eq!(full.last_played, Some(1712725190));

        // `LastPlayed` without `Playtime` — legitimate, not a zero.
        let no_minutes = map.get("2879840").expect("2879840 doit exister");
        assert_eq!(no_minutes.minutes, None);
        assert_eq!(no_minutes.last_played, Some(1764627784));

        // Neither key, just a `cloud` block — never played is legitimate.
        let cloud_only = map.get("241100").expect("241100 doit exister");
        assert_eq!(cloud_only.minutes, None);
        assert_eq!(cloud_only.last_played, None);
    }

    /// `cloud`/`autocloud` sub-blocks carry their own timestamps
    /// (`lastlaunch`, `lastexit`) — and nothing below an AppID block may
    /// leak into its record, not even a decoy with the same key name.
    const CLOUD_DECOYS: &str = r#"
"UserLocalConfigStore"
{
    "Software" { "Valve" { "Steam" { "apps"
    {
        "555"
        {
            "Playtime" "90"
            "autocloud"
            {
                "lastlaunch" "1999999999"
                "lastexit" "2000000000"
                "LastPlayed" "1888888888"
            }
        }
        "666"
        {
            "cloud"
            {
                "last_sync_state" "synchronized"
                "Playtime" "4242"
            }
        }
    } } } }
}
"#;

    #[test]
    fn playtimes_never_descends_into_cloud_subblocks() {
        let map = parse_playtimes(CLOUD_DECOYS);

        let with_autocloud = map.get("555").expect("555 doit exister");
        assert_eq!(with_autocloud.minutes, Some(90));
        assert_eq!(
            with_autocloud.last_played, None,
            "lastlaunch/lastexit/LastPlayed sous autocloud ne doivent pas remonter"
        );

        let with_cloud = map.get("666").expect("666 doit exister");
        assert_eq!(
            with_cloud.minutes, None,
            "un Playtime sous cloud n'est pas le Playtime du jeu"
        );
        assert_eq!(with_cloud.last_played, None);
    }

    /// Copied from the REAL file (author's machine, account 397533232,
    /// app 480): an AppID block carries siblings that merely LOOK like
    /// `Playtime` — `Playtime2wks`, `PlaytimeDisconnected` — plus
    /// `BadgeData`, a `cloud` sub-block and an `autocloud/lastexit`.
    /// Measured there: 14 apps carry `Playtime2wks`, 4 carry
    /// `PlaytimeDisconnected`. Steam writes these keys in no fixed order
    /// (at least four orderings observed in that one file), so the last
    /// two are swapped relative to app 480: with a `starts_with("playtime")`
    /// match (last key wins), this fixture then reads the 2-weeks value.
    /// The exact-equality match is what keeps this honest.
    const REAL_LOCALCONFIG_BLOCK: &str = r#"
"UserLocalConfigStore"
{
    "Software" { "Valve" { "Steam" { "apps"
    {
        "480"
        {
            "LastPlayed"            "1784474703"
            "Playtime"              "39489"
            "cloud"
            {
                "last_sync_state"               "synchronized"
            }
            "BadgeData"             "020000000809"
            "autocloud"
            {
                "lastexit"              "1784474703"
            }
            "PlaytimeDisconnected"          "1"
            "Playtime2wks"          "55"
        }
    } } } }
}
"#;

    /// The value read must be EXACTLY `Playtime`'s — not the two-weeks
    /// time (55), not the disconnected counter (1), both of which start
    /// with "playtime" once lowercased.
    #[test]
    fn playtimes_reads_exactly_playtime_among_its_siblings() {
        let map = parse_playtimes(REAL_LOCALCONFIG_BLOCK);
        let record = map.get("480").expect("480 doit exister");
        assert_eq!(
            record.minutes, Some(39489),
            "seule la clé Playtime donne le temps de jeu — ni Playtime2wks (55), ni PlaytimeDisconnected (1)"
        );
        assert_eq!(record.last_played, Some(1784474703));
    }

    #[test]
    fn playtimes_on_empty_or_unrelated_document() {
        assert!(parse_playtimes("").is_empty());
        assert!(parse_playtimes("\"UserLocalConfigStore\" { }").is_empty());
        // An apps block without the full ancestry is not ours to read.
        assert!(parse_playtimes("\"apps\" { \"42\" { \"Playtime\" \"9\" } }").is_empty());
    }

    /// Root key `"users"` — the spelling Steam actually writes (a fixture
    /// built on the invented root `"loginusers"` once kept every test green
    /// while the parser returned `None` on every real machine).
    ///
    /// Shaped to discriminate: the account flagged `MostRecent` carries
    /// the LOWEST `Timestamp` AND the LOWEST SteamID64, so the test cannot
    /// pass by falling back to either tiebreak — the flag itself must win.
    /// The real `loginusers.vdf` on this machine has no `MostRecent` at
    /// all, so this flag is exercised here, or nowhere.
    ///
    /// The first entry is deliberately NOT a SteamID64, and placed FIRST:
    /// one malformed entry must be skipped (pitfall 27) — a parser that
    /// aborts the whole scan on it never reaches the valid accounts.
    const LOGINUSERS_EXCERPT: &str = r#"
"users"
{
    "not_an_account"
    {
        "SomethingElse"    "1"
    }
    "76561197960265839"
    {
        "AccountName"      "actif"
        "PersonaName"      "Compte Actif"
        "RememberPassword" "1"
        "MostRecent"       "1"
        "Timestamp"        "1700000000"
    }
    "76561197960265950"
    {
        "AccountName"      "ancien"
        "PersonaName"      "Ancien Compte"
        "RememberPassword" "1"
        "MostRecent"       "0"
        "Timestamp"        "1712725190"
    }
}
"#;

    #[test]
    fn loginusers_prefers_the_most_recent_account() {
        assert_eq!(
            parse_most_recent_login(LOGINUSERS_EXCERPT),
            Some(76561197960265839),
            "MostRecent=1 gagne même avec le plus petit Timestamp et le plus petit SteamID64, \
             et l'entrée malformée placée avant est sautée, pas fatale"
        );
    }

    /// Also pins the tolerance on the root key: this fixture uses the
    /// `"LoginUsers"` spelling, the real files use `"users"` (see
    /// LOGINUSERS_EXCERPT). Both must resolve.
    #[test]
    fn loginusers_falls_back_to_highest_timestamp() {
        let raw = r#"
"LoginUsers"
{
    "76561197960265839" { "Timestamp" "1700000000" }
    "76561197960265950" { "Timestamp" "1712725190" }
}
"#;
        assert_eq!(parse_most_recent_login(raw), Some(76561197960265950));
        assert_eq!(parse_most_recent_login("\"LoginUsers\" { }"), None);
        assert_eq!(parse_most_recent_login(""), None);
    }

    #[test]
    fn account_id_conversion_matches_known_values() {
        // Verified on the author's machine: the conversion lands exactly on
        // an existing userdata folder.
        assert_eq!(
            account_id_from_steamid64(76561197960265728 + 12345678),
            Some(12345678)
        );
        assert_eq!(account_id_from_steamid64(STEAMID64_BASE), Some(0));
        // Below the base there is no AccountID32 — refuse, don't wrap.
        assert_eq!(account_id_from_steamid64(STEAMID64_BASE - 1), None);
    }

    // ------------------------------------------------------------------
    // Disk-level fixtures: a scratch Steam tree per test.

    fn write_localconfig(steam: &Path, account_folder: &str, content: &str) {
        let dir = steam
            .join("userdata")
            .join(account_folder)
            .join("config");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("localconfig.vdf"), content).unwrap();
    }

    fn apps_with(apps_body: &str) -> String {
        format!(
            "\"UserLocalConfigStore\"\n{{\n\"Software\" {{ \"Valve\" {{ \"Steam\" {{ \"apps\"\n{{\n{apps_body}\n}} }} }} }}\n}}\n"
        )
    }

    /// Writes the root key Steam actually writes — `"users"`, not
    /// `"loginusers"`. A fixture shaped after the implementation instead of
    /// after a real file is how this parser shipped returning `None` for
    /// every real account while every test stayed green.
    fn write_loginusers(steam: &Path, users: &[(u64, u8, u64)]) {
        let mut raw = String::from("\"users\"\n{\n");
        for (steamid64, most_recent, timestamp) in users {
            raw.push_str(&format!(
                "\"{steamid64}\"\n{{\n\"AccountName\"\t\t\"user\"\n\"MostRecent\"\t\t\"{most_recent}\"\n\"Timestamp\"\t\t\"{timestamp}\"\n}}\n"
            ));
        }
        raw.push('}');
        std::fs::create_dir_all(steam.join("config")).unwrap();
        std::fs::write(steam.join("config").join("loginusers.vdf"), raw).unwrap();
    }

    /// The signed-in account is neither the first `userdata` folder in
    /// alphabetical order nor necessarily the biggest file: `loginusers.vdf`
    /// decides, via the SteamID64 → AccountID32 conversion.
    #[test]
    fn playtime_for_reads_the_signed_in_account_not_the_first_folder() {
        let _lock = cache_lock();
        clear_caches();
        let steam = scratch("pt_account");

        // AccountID32 folders: 111 (ancien) and 222 (actif).
        write_localconfig(
            &steam,
            "111",
            &apps_with("\"42\" { \"Playtime\" \"999\" \"LastPlayed\" \"1500000000\" }"),
        );
        write_localconfig(
            &steam,
            "222",
            &apps_with("\"42\" { \"Playtime\" \"42\" \"LastPlayed\" \"1712725190\" }"),
        );
        // Most recent → STEAMID64_BASE + 222. The flag is made to CONTRADICT
        // both tiebreaks (the other account has the higher Timestamp and the
        // higher SteamID64): only a parser that truly prefers `MostRecent`
        // lands on folder 222.
        write_loginusers(
            &steam,
            &[
                (STEAMID64_BASE + 111, 0, 1712725190),
                (STEAMID64_BASE + 222, 1, 1500000000),
            ],
        );

        let record = playtime_for(&steam, "42").expect("le compte actif a des données");
        assert_eq!(
            record.minutes,
            Some(42),
            "le temps lu doit être celui du compte connecté (dossier 222), pas du premier dossier trouvé"
        );
        assert_eq!(record.last_played, Some(1712725190));

        let _ = std::fs::remove_dir_all(&steam);
    }

    #[test]
    fn playtime_for_fallback_single_folder_only() {
        let _lock = cache_lock();
        clear_caches();
        let steam = scratch("pt_fallback_one");
        // No loginusers.vdf at all — one userdata folder is unambiguous.
        write_localconfig(&steam, "999", &apps_with("\"42\" { \"Playtime\" \"7\" }"));
        let record = playtime_for(&steam, "42").expect("dossier unique → repli");
        assert_eq!(record.minutes, Some(7));
        let _ = std::fs::remove_dir_all(&steam);

        clear_caches();
        let steam = scratch("pt_fallback_many");
        // No loginusers.vdf, two folders: don't guess, show nothing.
        write_localconfig(&steam, "111", &apps_with("\"42\" { \"Playtime\" \"1\" }"));
        write_localconfig(&steam, "222", &apps_with("\"42\" { \"Playtime\" \"2\" }"));
        assert_eq!(
            playtime_for(&steam, "42"),
            None,
            "plusieurs dossiers sans loginusers.vdf → on ne devine pas"
        );
        let _ = std::fs::remove_dir_all(&steam);

        clear_caches();
        let steam = scratch("pt_fallback_stale");
        // loginusers.vdf exists but points at a folder that doesn't exist,
        // and two folders remain: still don't guess.
        write_localconfig(&steam, "111", &apps_with("\"42\" { \"Playtime\" \"1\" }"));
        write_localconfig(&steam, "222", &apps_with("\"42\" { \"Playtime\" \"2\" }"));
        write_loginusers(&steam, &[(STEAMID64_BASE + 333, 1, 1712725190)]);
        assert_eq!(playtime_for(&steam, "42"), None);
        let _ = std::fs::remove_dir_all(&steam);
    }

    /// Prove the cache branch is exercised (pitfall 25): the settle window
    /// is neutralised, and the hit counter must reach exactly 1 on the
    /// second call — a test without the cache would fail here, not pass.
    #[test]
    fn playtime_cache_hit_returns_same_data() {
        let _lock = cache_lock();
        clear_caches();
        set_playtime_settle(0);
        set_loginusers_settle(0);
        let steam = scratch("pt_cache_hit");
        write_localconfig(
            &steam,
            "999",
            &apps_with("\"42\" { \"Playtime\" \"217\" \"LastPlayed\" \"1712725190\" }"),
        );

        let first = playtime_for(&steam, "42").expect("lecture disque");
        let second = playtime_for(&steam, "42").expect("lecture en cache");
        assert_eq!(first, second);
        assert_eq!(
            PLAYTIME_CACHE.hits(),
            1,
            "la seconde lecture doit toucher le cache"
        );

        let _ = std::fs::remove_dir_all(&steam);
        // Restore BOTH settle windows: they are process-global, and a test
        // that leaves one at zero changes the behaviour of every test that
        // runs after it in the same process.
        set_playtime_settle(DEFAULT_SETTLE_NANOS_FOR_TEST);
        set_loginusers_settle(DEFAULT_SETTLE_NANOS_FOR_TEST);
    }

    const DEFAULT_SETTLE_NANOS_FOR_TEST: u64 = 2_000_000_000;

    #[test]
    fn playtime_read_failure_is_never_cached() {
        let _lock = cache_lock();
        clear_caches();
        set_playtime_settle(0);
        let steam = scratch("pt_nocache_fail");
        // userdata exists but localconfig.vdf does not: FileStamp itself
        // returns None, so nothing can enter the cache.
        std::fs::create_dir_all(steam.join("userdata").join("999").join("config")).unwrap();

        assert!(playtime_for(&steam, "42").is_none());
        assert!(playtime_for(&steam, "42").is_none());
        assert_eq!(
            PLAYTIME_CACHE.len(),
            0,
            "un fichier absent ne doit jamais entrer en cache"
        );

        let _ = std::fs::remove_dir_all(&steam);
        set_playtime_settle(DEFAULT_SETTLE_NANOS_FOR_TEST);
    }

    /// The real untested case: the file EXISTS but the read fails — Steam
    /// holds it locked mid-rewrite, ACLs, disk. A DIRECTORY wearing the
    /// name is the simplest portable way to make `read_to_string` fail:
    /// `FileStamp::from_path` succeeds on it (metadata reads directories),
    /// so execution reaches the read guard this test is about. Caching
    /// that failure would show "temps inconnu" for the WHOLE library until
    /// Steam happens to rewrite the file.
    #[test]
    fn playtime_unreadable_file_is_never_cached() {
        let _lock = cache_lock();
        clear_caches();
        set_playtime_settle(0);
        let steam = scratch("pt_nocache_unreadable");
        std::fs::create_dir_all(
            steam
                .join("userdata")
                .join("999")
                .join("config")
                .join("localconfig.vdf"),
        )
        .unwrap();

        assert!(playtime_for(&steam, "42").is_none());
        assert!(playtime_for(&steam, "42").is_none());
        assert_eq!(
            PLAYTIME_CACHE.len(),
            0,
            "un fichier existant mais illisible ne doit jamais entrer en cache"
        );

        let _ = std::fs::remove_dir_all(&steam);
        set_playtime_settle(DEFAULT_SETTLE_NANOS_FOR_TEST);
    }

    /// Freshness regression: Steam rewrites localconfig.vdf on exit — the
    /// stamp changes and the cached parse must not survive it.
    #[test]
    fn playtime_cache_invalidates_when_rewritten() {
        let _lock = cache_lock();
        clear_caches();
        set_playtime_settle(0);
        let steam = scratch("pt_rewrite");
        write_localconfig(&steam, "999", &apps_with("\"42\" { \"Playtime\" \"10\" }"));
        assert_eq!(playtime_for(&steam, "42").unwrap().minutes, Some(10));

        // Rewrite with a different value AND a different size.
        write_localconfig(
            &steam,
            "999",
            &apps_with("\"42\" { \"Playtime\" \"999\" \"LastPlayed\" \"1712725190\" }"),
        );
        assert_eq!(playtime_for(&steam, "42").unwrap().minutes, Some(999));

        let _ = std::fs::remove_dir_all(&steam);
        set_playtime_settle(DEFAULT_SETTLE_NANOS_FOR_TEST);
    }

    /// The one thing fixtures cannot prove: that the real path
    /// `userdata\<AccountID>\config\localconfig.vdf` and the SteamID64 →
    /// AccountID conversion actually land on the signed-in account's file.
    ///
    /// Ignored by default — it depends on the machine having Steam installed
    /// with at least one account that has played something.
    #[test]
    #[ignore = "reads the Steam install of this machine"]
    fn live_playtime_from_the_signed_in_account() {
        let steam = crate::detect::detect_steam_path().expect("Steam introuvable sur cette machine");
        let dir = userdata_dir(&steam).expect("compte connecté non résolu depuis loginusers.vdf");
        let file = dir.join("config").join("localconfig.vdf");
        assert!(file.is_file(), "localconfig.vdf attendu à {}", file.display());

        let map = playtimes_map(&steam).expect("lecture de localconfig.vdf");
        let played: Vec<_> = map
            .iter()
            .filter_map(|(id, r)| r.minutes.filter(|m| *m > 0).map(|m| (id.clone(), m)))
            .collect();
        println!(
            "compte {} — {} entrées, {} avec un temps de jeu",
            dir.file_name().unwrap_or_default().to_string_lossy(),
            map.len(),
            played.len()
        );
        let mut top = played.clone();
        top.sort_by_key(|b| std::cmp::Reverse(b.1));
        for (id, minutes) in top.iter().take(3) {
            println!("  {id}: {} h {:02}", minutes / 60, minutes % 60);
        }

        // A real account that has launched games has playtimes; an AppID key
        // is numeric, and minutes must not be absurd (10 years of wall clock).
        assert!(!played.is_empty(), "aucun temps de jeu lu — le chemin ou le parseur est faux");
        for (id, minutes) in &played {
            assert!(id.chars().all(|c| c.is_ascii_digit()), "AppID non numérique: {id}");
            assert!(*minutes < 5_256_000, "temps de jeu absurde pour {id}: {minutes} min");
        }
    }
}
