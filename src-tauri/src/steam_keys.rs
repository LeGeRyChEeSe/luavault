//! The classic SteamTools and the depot keys (Windows Sandbox, Steam 1792618285, 2026-11-11).
//!
//! The classic SteamTools hands Steam the keys of a `.lua` only when the file is put in
//! `config\stplug-in` (or rewritten there) **while Steam runs**: a file already there when Steam
//! starts gives no key, and a game that is not downloaded yet fails with « Missing decryption key ».
//! Steam keeps the depot keys it has in `config\config.vdf` (`InstallConfigStore` > `Software` >
//! `Valve` > `Steam` > `depots` > `<depot>` > `DecryptionKey`) and reads them at every start.
//!
//! Two gestures follow, both measured on the bench:
//! - Steam closed: the depot keys of every `.lua` in `stplug-in` are written into that cache
//!   (added only, never replaced, the file copied aside first, temp + rename). Steam then starts and
//!   downloads with the app not running, and keeps the entries when it closes.
//! - Steam running: a `.lua` of a game not installed yet whose keys are missing from the cache is
//!   removed and written again, which makes the classic SteamTools hand them over.
//!
//! The watcher (`spawn_watcher`) does both on Steam's transitions. Nothing here runs under
//! OpenSteamTool, which reads `config\lua` live with its own keys.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use log::{info, warn};

/// Steam's own file, relative to its folder.
pub const CONFIG_VDF: &str = "config/config.vdf";
/// The copy kept beside it before each write.
pub const BACKUP_NAME: &str = "config.vdf.lv-bak";
/// How often the watcher looks at Steam.
pub const POLL_SECS: u64 = 10;
/// After Steam appears, how long the watcher waits before rewriting files: the classic SteamTools
/// must be loaded and Steam past its account loading (≈ 60 s on the bench).
pub const SETTLE_SECS: u64 = 60;

fn is_hex_key(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

fn is_id(s: &str) -> bool {
    !s.is_empty() && s.len() <= 10 && s.bytes().all(|b| b.is_ascii_digit()) && !s.starts_with('0')
}

/// Pure. The `(depot, key)` pairs of a `.lua`: every `addappid(<id>, <0|1>, "<64 hex>")` line but
/// the main game's (its key is not a depot key, and a `.lua` written by the app no longer carries it).
pub fn depot_keys(text: &str, app_id: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let Some(rest) = line.trim_start().strip_prefix("addappid(") else { continue };
        let Some(close) = rest.find(')') else { continue };
        let args: Vec<&str> = rest[..close].split(',').map(str::trim).collect();
        if args.len() < 3 || args[0] == app_id || !is_id(args[0]) {
            continue;
        }
        let key = args[2].trim_matches('"');
        if args[2].starts_with('"') && args[2].ends_with('"') && is_hex_key(key) {
            out.push((args[0].to_string(), key.to_string()));
        }
    }
    out
}

fn indent_of(line: &str) -> &str {
    &line[..line.len() - line.trim_start_matches('\t').len()]
}

/// The `"depots"` block: (index of its `{` line, index of its `}` line, its indentation).
/// Exactly one block at the expected place, else `None`.
fn depots_block(lines: &[&str]) -> Option<(usize, usize, String)> {
    let found: Vec<usize> = lines.iter().enumerate().filter(|(_, l)| l.trim() == "\"depots\"").map(|(i, _)| i).collect();
    let [at] = found.as_slice() else { return None };
    let indent = indent_of(lines[*at]).to_string();
    if lines.get(at + 1).map(|l| l.trim_end_matches('\r')) != Some(&format!("{indent}{{")) {
        return None;
    }
    let close = (at + 2..lines.len()).find(|&i| lines[i].trim_end_matches('\r') == format!("{indent}}}"))?;
    Some((at + 1, close, indent))
}

/// Pure. The depots whose key Steam keeps in this `config.vdf`.
pub fn cached_depots(vdf: &str) -> BTreeSet<String> {
    let lines: Vec<&str> = vdf.split('\n').collect();
    let Some((open, close, indent)) = depots_block(&lines) else { return BTreeSet::new() };
    let entry = format!("{indent}\t");
    let mut out = BTreeSet::new();
    let mut i = open + 1;
    while i < close {
        let line = lines[i].trim_end_matches('\r');
        if indent_of(line) == entry {
            let name = line.trim().trim_matches('"');
            let body_has_key = (i + 1..close)
                .take_while(|&j| lines[j].trim_end_matches('\r') != format!("{entry}}}"))
                .any(|j| lines[j].trim().starts_with("\"DecryptionKey\""));
            if is_id(name) && body_has_key {
                out.insert(name.to_string());
            }
        }
        i += 1;
    }
    out
}

/// Pure. `config.vdf` with the keys Steam does not hold yet added to its `depots` block (created in
/// `InstallConfigStore` > `Software` > `Valve` > `Steam` when absent). A key Steam already holds is
/// never replaced. `None` when there is nothing to add or the file is not shaped as expected: then
/// nothing is written.
pub fn with_keys(vdf: &str, keys: &[(String, String)]) -> Option<String> {
    let nl = if vdf.contains("\r\n") { "\r\n" } else { "\n" };
    let have = cached_depots(vdf);
    let mut seen = BTreeSet::new();
    let wanted: Vec<&(String, String)> = keys
        .iter()
        .filter(|(id, key)| is_id(id) && is_hex_key(key) && !have.contains(id) && seen.insert(id.clone()))
        .collect();
    if wanted.is_empty() {
        return None;
    }
    let lines: Vec<&str> = vdf.split('\n').collect();
    let entries = |indent: &str| -> String {
        wanted
            .iter()
            .map(|(id, key)| {
                format!("{indent}\t\"{id}\"{nl}{indent}\t{{{nl}{indent}\t\t\"DecryptionKey\"\t\t\"{key}\"{nl}{indent}\t}}{nl}")
            })
            .collect()
    };
    let mut out = String::with_capacity(vdf.len() + wanted.len() * 128);
    if let Some((open, _, indent)) = depots_block(&lines) {
        for (i, line) in lines.iter().enumerate() {
            out.push_str(line);
            if i + 1 < lines.len() {
                out.push('\n');
            }
            if i == open {
                out.push_str(&entries(&indent));
            }
        }
        return Some(out);
    }
    // No block yet (and only then: an ambiguous or broken one is left alone): Steam's own
    // section, by its exact path and indentation.
    if lines.iter().any(|l| l.trim() == "\"depots\"") {
        return None;
    }
    let path = ["\"InstallConfigStore\"", "\"Software\"", "\"Valve\"", "\"Steam\""];
    let mut depth = 0usize;
    let mut open = None;
    for (i, line) in lines.iter().enumerate() {
        let l = line.trim_end_matches('\r');
        if depth < path.len()
            && l.eq_ignore_ascii_case(&format!("{}{}", "\t".repeat(depth), path[depth]))
            && lines.get(i + 1).map(|n| n.trim_end_matches('\r')) == Some(&format!("{}{{", "\t".repeat(depth)))
        {
            depth += 1;
            if depth == path.len() {
                open = Some(i + 1);
                break;
            }
        }
    }
    let open = open?;
    let indent = "\t".repeat(path.len());
    for (i, line) in lines.iter().enumerate() {
        out.push_str(line);
        if i + 1 < lines.len() {
            out.push('\n');
        }
        if i == open {
            out.push_str(&format!("{indent}\"depots\"{nl}{indent}{{{nl}{}{indent}}}{nl}", entries(&indent)));
        }
    }
    Some(out)
}

/// The AppID-named `.lua` files of a folder, with their text.
fn lua_files(dir: &Path) -> Vec<(String, PathBuf, String)> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out = Vec::new();
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        let Some(app_id) = path.file_name().and_then(|n| n.to_str()).and_then(|n| n.strip_suffix(".lua")).filter(|s| is_id(s)) else {
            continue;
        };
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        out.push((app_id.to_string(), path, text));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn classic(steam: &Path) -> bool {
    crate::detect::flavor(steam) == crate::detect::Flavor::Classic
}

/// Steam closed (the watcher's word: Steam would write its own copy over ours when closing),
/// classic SteamTools: the depot keys of every `.lua` in `stplug-in` go into Steam's cache. Returns
/// the depots added; does nothing when `config.vdf` is absent or not shaped as expected.
fn write_cache(steam: &Path) -> std::io::Result<Vec<String>> {
    let vdf_path = steam.join(CONFIG_VDF);
    let Ok(vdf) = std::fs::read_to_string(&vdf_path) else { return Ok(Vec::new()) };
    let dir = crate::detect::lua_dir_for(steam, crate::detect::Flavor::Classic);
    let keys: Vec<(String, String)> = lua_files(&dir).iter().flat_map(|(id, _, text)| depot_keys(text, id)).collect();
    let before = cached_depots(&vdf);
    let Some(updated) = with_keys(&vdf, &keys) else { return Ok(Vec::new()) };
    let added: Vec<String> = cached_depots(&updated).difference(&before).cloned().collect();
    std::fs::copy(&vdf_path, vdf_path.with_file_name(BACKUP_NAME))?;
    let tmp = vdf_path.with_file_name("config.vdf.lv-tmp");
    std::fs::write(&tmp, updated.as_bytes())?;
    std::fs::rename(&tmp, &vdf_path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })?;
    Ok(added)
}

/// Pure. Whether a `.lua` must be written again while Steam runs: a game Steam has not fully
/// installed, with a depot key Steam does not hold.
pub fn needs_rearm(keys: &[(String, String)], cached: &BTreeSet<String>, fully_installed: bool) -> bool {
    !fully_installed && keys.iter().any(|(id, _)| !cached.contains(id))
}

/// Steam running, classic SteamTools: every `.lua` that `needs_rearm` is removed and written again,
/// so the classic SteamTools hands its keys over. Returns the AppIDs rewritten.
pub fn rearm(steam: &Path) -> Vec<String> {
    if !classic(steam) {
        return Vec::new();
    }
    let cached = std::fs::read_to_string(steam.join(CONFIG_VDF)).map(|v| cached_depots(&v)).unwrap_or_default();
    let dir = crate::detect::lua_dir_for(steam, crate::detect::Flavor::Classic);
    let mut done = Vec::new();
    for (app_id, path, text) in lua_files(&dir) {
        let keys = depot_keys(&text, &app_id);
        let installed = crate::vdf::locate_game(steam, &app_id).fully_installed;
        if !needs_rearm(&keys, &cached, installed) {
            continue;
        }
        // Removed, then written again: a rewrite in place was not measured.
        if std::fs::remove_file(&path).is_err() {
            continue;
        }
        std::thread::sleep(Duration::from_secs(3));
        match std::fs::write(&path, text.as_bytes()) {
            Ok(()) => done.push(app_id),
            Err(e) => warn!("SteamTools classique : {app_id}.lua non réécrit ({e})"),
        }
    }
    done
}

/// Best-effort: is `steam.exe` in the process list?
pub fn steam_running() -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        std::process::Command::new("tasklist.exe")
            .args(["/FI", "IMAGENAME eq steam.exe", "/NH"])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).lines().any(|l| l.to_lowercase().contains("steam.exe")))
            .unwrap_or(false)
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// Pure. What the watcher does on this tick, from Steam's state now and at the last tick
/// (`None` = first tick).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Move {
    /// Steam closed (or found closed at start): fill the cache.
    FillCache,
    /// Steam started (or found running at start): rewrite what still lacks its keys, once settled.
    Rearm,
    Nothing,
}

pub fn next_move(was_running: Option<bool>, running: bool) -> Move {
    match (was_running, running) {
        (Some(was), now) if was == now => Move::Nothing,
        (_, false) => Move::FillCache,
        (_, true) => Move::Rearm,
    }
}

/// The watcher every instance runs: on each of Steam's transitions, the gesture of the moment.
pub fn spawn_watcher(steam_of: impl Fn() -> Option<PathBuf> + Send + 'static) {
    std::thread::spawn(move || {
        let mut was = None;
        loop {
            let running = steam_running();
            if let Some(steam) = steam_of().filter(|s| classic(s)) {
                // The pin of the old Steam was OpenSteamTool's workaround: lifted once the classic
                // SteamTools is in place, at every tick (a quick restart of Steam must find it gone),
                // and Steam updates at its next start.
                if let Err(e) = crate::steam_downgrade::release_after_port(&steam) {
                    warn!("SteamTools classique : steam.cfg non modifié ({e})");
                }
                match next_move(was, running) {
                    Move::FillCache => match write_cache(&steam) {
                        Ok(added) if !added.is_empty() => info!("SteamTools classique : {} clé(s) de dépôt données à Steam ({})", added.len(), added.join(", ")),
                        Ok(_) => {}
                        Err(e) => warn!("SteamTools classique : cache des clés de Steam non écrit ({e})"),
                    },
                    Move::Rearm => {
                        std::thread::sleep(Duration::from_secs(SETTLE_SECS));
                        if steam_running() {
                            let done = rearm(&steam);
                            if !done.is_empty() {
                                info!("SteamTools classique : fichiers réécrits pour donner leurs clés à Steam ({})", done.join(", "));
                            }
                        }
                    }
                    Move::Nothing => {}
                }
            }
            was = Some(running);
            std::thread::sleep(Duration::from_secs(POLL_SECS));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const KEY_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const KEY_C: &str = "0123456789ABCDEF0123456789abcdef0123456789abcdef0123456789abcdef";

    /// The shape of the real file (Windows Sandbox and the author's Steam, 2026-11-11): LF line
    /// ends, tabs, `depots` four tabs deep under `Steam`. Keys replaced, other sections shortened.
    fn real_vdf(depots: &str) -> String {
        format!(
            "\"InstallConfigStore\"\n{{\n\t\"Software\"\n\t{{\n\t\t\"Valve\"\n\t\t{{\n\t\t\t\"Steam\"\n\t\t\t{{\n\t\t\t\t\"ShaderCacheManager\"\n\t\t\t\t{{\n\t\t\t\t\t\"HasCurrentBucket\"\t\t\"1\"\n\t\t\t\t}}\n{depots}\t\t\t\t\"RecentDownloadRate\"\t\t\"3607748\"\n\t\t\t}}\n\t\t}}\n\t}}\n\t\"streaming\"\n\t{{\n\t\t\"ClientID\"\t\t\"-1\"\n\t}}\n}}\n"
        )
    }

    fn block(entries: &[(&str, &str)]) -> String {
        let mut s = "\t\t\t\t\"depots\"\n\t\t\t\t{\n".to_string();
        for (id, key) in entries {
            s += &format!("\t\t\t\t\t\"{id}\"\n\t\t\t\t\t{{\n\t\t\t\t\t\t\"DecryptionKey\"\t\t\"{key}\"\n\t\t\t\t\t}}\n");
        }
        s + "\t\t\t\t}\n"
    }

    #[test]
    fn steam_keys_reads_the_depot_lines_of_a_real_lua() {
        // Terra Nil as served, reduced by the app (main line without key), keys replaced.
        let lua = format!(
            "-- Terra Nil\n-- AppID 1593030 | Generated on 2026-01-01 05:55 UTC | example\n\n-- Main Application\naddappid(1593030)\n\n-- Content Depots (3)\naddappid(1593031, 1, \"{KEY_A}\") -- Terra Nil - Windows\naddappid(1593032, 1, \"{KEY_B}\") -- Terra Nil - macOS\n\n-- DLCs with Content (1)\naddappid(2321930, 1, \"{KEY_C}\") -- Terra Nil Digital Artbook\naddtoken(2321930, \"11175953412085981858\")\naddappid(3879490)\n"
        );
        let keys = depot_keys(&lua, "1593030");
        assert_eq!(keys.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(), ["1593031", "1593032", "2321930"]);
        assert_eq!(keys[2].1, KEY_C);
        // The main game's key is never a depot key, even when a .lua still carries it.
        assert!(depot_keys(&format!("addappid(1593030, 1, \"{KEY_A}\")\n"), "1593030").is_empty());
        for bad in ["addappid(1, 1, \"short\")", "addappid(0123, 1, \"{KEY_A}\")", "addappid(12, 1, abc)", "-- addappid"] {
            assert!(depot_keys(&bad.replace("{KEY_A}", KEY_A), "1").is_empty(), "{bad}");
        }
    }

    #[test]
    fn steam_keys_adds_only_what_steam_lacks_and_never_replaces() {
        let vdf = real_vdf(&block(&[("2881651", KEY_A)]));
        assert_eq!(cached_depots(&vdf).into_iter().collect::<Vec<_>>(), ["2881651"]);
        let keys = vec![("2881651".to_string(), KEY_B.to_string()), ("1593031".to_string(), KEY_C.to_string())];
        let out = with_keys(&vdf, &keys).expect("one key to add");
        assert_eq!(cached_depots(&out).into_iter().collect::<Vec<_>>(), ["1593031", "2881651"]);
        assert!(out.contains(KEY_A) && !out.contains(KEY_B), "Steam's own key stays, ours is not written over it");
        assert!(out.contains(&format!("\t\t\t\t\t\"1593031\"\n\t\t\t\t\t{{\n\t\t\t\t\t\t\"DecryptionKey\"\t\t\"{KEY_C}\"\n\t\t\t\t\t}}\n")));
        // Everything else is byte for byte what it was.
        assert_eq!(out.replace(&format!("\t\t\t\t\t\"1593031\"\n\t\t\t\t\t{{\n\t\t\t\t\t\t\"DecryptionKey\"\t\t\"{KEY_C}\"\n\t\t\t\t\t}}\n"), ""), vdf);
        assert_eq!(with_keys(&out, &keys), None, "nothing left to add");
    }

    #[test]
    fn steam_keys_creates_the_block_in_steams_section_when_absent() {
        let vdf = real_vdf("");
        assert!(cached_depots(&vdf).is_empty());
        let out = with_keys(&vdf, &[("1593031".to_string(), KEY_A.to_string())]).expect("block created");
        assert_eq!(cached_depots(&out).into_iter().collect::<Vec<_>>(), ["1593031"]);
        assert!(out.contains("\t\t\t{\n\t\t\t\t\"depots\"\n\t\t\t\t{\n\t\t\t\t\t\"1593031\""), "right under Steam's opening brace");
    }

    #[test]
    fn steam_keys_writes_nothing_into_a_file_it_does_not_recognise() {
        let keys = [("1593031".to_string(), KEY_A.to_string())];
        assert_eq!(with_keys("", &keys), None);
        assert_eq!(with_keys("\"InstallConfigStore\"\n{\n}\n", &keys), None);
        let two = real_vdf(&format!("{}{}", block(&[]), block(&[])));
        assert_eq!(with_keys(&two, &keys), None, "two depots blocks: which one Steam reads is not ours to guess");
        let broken = real_vdf("\t\t\t\t\"depots\"\n");
        assert_eq!(with_keys(&broken, &keys), None);
        assert_eq!(with_keys(&real_vdf(""), &[("1".to_string(), "nothex".to_string())]), None);
    }

    #[test]
    fn steam_keys_keeps_windows_line_ends() {
        let vdf = real_vdf(&block(&[])).replace('\n', "\r\n");
        let out = with_keys(&vdf, &[("1593031".to_string(), KEY_A.to_string())]).unwrap();
        assert!(!out.replace("\r\n", "").contains('\n'), "every line end stays CRLF");
        assert_eq!(cached_depots(&out).len(), 1);
    }

    #[test]
    fn steam_keys_rewrites_only_a_game_not_installed_with_a_key_missing() {
        let keys = vec![("1593031".to_string(), KEY_A.to_string()), ("1593032".to_string(), KEY_B.to_string())];
        let cached: BTreeSet<String> = ["1593031".to_string()].into();
        assert!(needs_rearm(&keys, &cached, false));
        assert!(!needs_rearm(&keys, &cached, true), "an installed game keeps its keys in Steam: never touched");
        let all: BTreeSet<String> = ["1593031".to_string(), "1593032".to_string()].into();
        assert!(!needs_rearm(&keys, &all, false));
        assert!(!needs_rearm(&[], &BTreeSet::new(), false), "a file with no key has nothing to hand over");
    }

    #[test]
    fn steam_keys_acts_on_transitions_only() {
        assert_eq!(next_move(None, false), Move::FillCache);
        assert_eq!(next_move(None, true), Move::Rearm);
        assert_eq!(next_move(Some(true), false), Move::FillCache);
        assert_eq!(next_move(Some(false), true), Move::Rearm);
        assert_eq!(next_move(Some(false), false), Move::Nothing);
        assert_eq!(next_move(Some(true), true), Move::Nothing);
    }

    #[test]
    fn steam_keys_fills_the_cache_of_a_classic_steam_on_disk() {
        let steam = std::env::temp_dir().join(format!("lv-steam-keys-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&steam);
        std::fs::create_dir_all(steam.join("config/stplug-in")).unwrap();
        for f in ["xinput1_4.dll", "dwmapi.dll", "steam.exe"] {
            std::fs::write(steam.join(f), b"x").unwrap();
        }
        let vdf = real_vdf(&block(&[("2881651", KEY_A)]));
        std::fs::write(steam.join(CONFIG_VDF), &vdf).unwrap();
        std::fs::write(steam.join("config/stplug-in/1593030.lua"), format!("addappid(1593030)\naddappid(1593031, 1, \"{KEY_B}\")\n")).unwrap();
        std::fs::write(steam.join("config/stplug-in/notes.lua"), format!("addappid(9, 1, \"{KEY_C}\")\n")).unwrap();
        assert_eq!(write_cache(&steam).unwrap(), ["1593031"]);
        assert_eq!(std::fs::read_to_string(steam.join("config").join(BACKUP_NAME)).unwrap(), vdf, "Steam's file copied aside first");
        let now = std::fs::read_to_string(steam.join(CONFIG_VDF)).unwrap();
        assert_eq!(cached_depots(&now).into_iter().collect::<Vec<_>>(), ["1593031", "2881651"], "only AppID-named files count");
        assert!(write_cache(&steam).unwrap().is_empty(), "second pass: nothing to add, nothing written");
        let _ = std::fs::remove_dir_all(&steam);
    }
}
