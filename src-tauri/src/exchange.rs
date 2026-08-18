//! Library export (CSV / JSON) and import (AppID list from various sources).
//!
//! All functions are pure — no file I/O, no network.  This makes them trivially
//! testable and allows a single implementation to serve both the backend commands
//! and a future in-app preview panel.

use crate::commands::GameStatus;
use serde::Serialize;
use std::collections::BTreeSet;

// ─────────────────────────────────────────────────────────────────────────────
// Export
// ─────────────────────────────────────────────────────────────────────────────

/// One row per game, columns: app_id, name, stage, tags, added_at, updated_at,
/// fix_installed, hidden.
pub fn to_csv(statuses: &[GameStatus]) -> String {
    let mut out = String::new();
    out.push_str("app_id,name,stage,tags,added_at,updated_at,fix_installed,hidden\r\n");
    for s in statuses {
        let name = escape_csv(&s.name);
        let tags = s.tags.join(";");
        let tags = escape_csv(&tags);
        let fix_installed = if s.fix.health == crate::fixes::FixHealth::Healthy {
            "true"
        } else {
            "false"
        };
        out.push_str(&format!(
            "{},{},{},{},{},{},{},{}\r\n",
            s.app_id, name, s.stage, tags,
            csv_field(&s.added_at),
            csv_field(&s.updated_at),
            fix_installed,
            if s.hidden { "true" } else { "false" },
        ));
    }
    out
}

/// Same data as a JSON array of objects.
pub fn to_json(statuses: &[GameStatus]) -> Result<String, serde_json::Error> {
    let rows: Vec<_> = statuses.iter().map(|s| GameStatusJson {
        app_id: s.app_id.clone(),
        name: s.name.clone(),
        stage: s.stage.to_string(),
        tags: s.tags.clone(),
        added_at: s.added_at.clone(),
        updated_at: s.updated_at.clone(),
        fix_installed: s.fix.health == crate::fixes::FixHealth::Healthy,
        hidden: s.hidden,
    }).collect();
    serde_json::to_string_pretty(&rows)
}

#[derive(Serialize)]
struct GameStatusJson {
    app_id: String,
    name: String,
    stage: String,
    tags: Vec<String>,
    added_at: Option<String>,
    updated_at: Option<String>,
    fix_installed: bool,
    hidden: bool,
}

// ─────────────────────────────────────────────────────────────────────────────
// Import
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct ImportCandidate {
    pub app_id: String,
    /// Name found next to the AppID, when the source provides one.
    pub name: Option<String>,
    /// Already present in the library — nothing to do for this one.
    pub known: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportPreview {
    pub candidates: Vec<ImportCandidate>,
    /// Lines that carried no usable AppID, with the reason, capped at 20.
    pub skipped: Vec<String>,
    /// Total number of lines ignored (not just the first 20).
    pub skipped_total: usize,
    pub total_lines: usize,
}

/// Extract AppIDs from an arbitrary text file: our own CSV export, a Playnite
/// CSV export, a bare list of IDs, or Steam URLs.
pub fn parse_import(text: &str, known_ids: &[String]) -> ImportPreview {
    let known_set: std::collections::HashSet<&str> = known_ids.iter().map(|s| s.as_str()).collect();

    let mut candidates: Vec<ImportCandidate> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    let mut skipped_total: usize = 0;
    let mut seen_ids: BTreeSet<String> = BTreeSet::new();
    let mut total_lines: usize = 0;

    // Detect header on the first non-blank line.
    let first_non_blank = text.lines().find(|l| !l.trim().is_empty());
    let (header_col_appid, header_col_name, use_full_csv) = first_non_blank.map_or((None, None, false), |first_line| {
        let cells = parse_csv_line(first_line);
        // Trim before comparing: spreadsheets happily write "Name, AppId", and a
        // header we fail to recognise is silently treated as data.
        let lower_cells: Vec<String> =
            cells.iter().map(|c| c.trim().to_lowercase()).collect();
        let appid_idx = lower_cells.iter().position(|c| {
            *c == "appid" || *c == "app_id" || *c == "gameid" || *c == "game id"
        });
        let name_idx = lower_cells.iter().position(|c| {
            *c == "name" || *c == "nom" || *c == "title"
        });
        if appid_idx.is_some() {
            (appid_idx, name_idx, true)
        } else {
            (None, None, false)
        }
    });

    if use_full_csv {
        // CSV with header — use full CSV parser to handle multi-line fields.
        // Skip blank lines at the top, then skip the header (first non-blank line).
        let all_rows = parse_csv_text(text);
        // Find the index of the header row (first non-blank row)
        let header_idx = all_rows.iter().position(|r| !r.is_empty() && !r.iter().all(|c| c.trim().is_empty()));
        let data_start = header_idx.map_or(0, |h| h + 1);
        for row in all_rows.iter().skip(data_start) {
            if row.is_empty() || row.iter().all(|c| c.trim().is_empty()) {
                continue; // blank row
            }
            total_lines += 1;
            if let Some(idx) = header_col_appid {
                if idx < row.len() {
                    let raw = row[idx].trim();
                    if let Some(app_id) = extract_appid_from_cell(raw) {
                        let app_id_norm = strip_leading_zeros(&app_id);
                        if seen_ids.insert(app_id_norm.clone()) {
                            let name = header_col_name.and_then(|ni| {
                                if ni < row.len() {
                                    let n = row[ni].trim().to_string();
                                    if !n.is_empty() { Some(n) } else { None }
                                } else { None }
                            });
                            candidates.push(ImportCandidate {
                                app_id: app_id_norm.clone(),
                                name,
                                known: known_set.contains(app_id_norm.as_str()),
                            });
                        }
                    } else {
                        skipped_total += 1;
                        if skipped.len() < 20 {
                            skipped.push(format!("Ligne {} : aucun AppID valide trouvé", total_lines));
                        }
                    }
                } else {
                    // Row has fewer columns than the header — AppID column is absent.
                    skipped_total += 1;
                    if skipped.len() < 20 {
                        skipped.push(format!("Ligne {} : colonne AppID absente", total_lines));
                    }
                }
            }
        }
    } else {
        // No header — line-by-line processing.
        for line in text.lines() {
            if line.trim().is_empty() {
                continue;
            }
            total_lines += 1;
            let cells = parse_csv_line(line);
            let mut plausible: Vec<(usize, String)> = Vec::new();
            for (ci, cell) in cells.iter().enumerate() {
                if let Some(app_id) = extract_appid_from_cell(cell.trim()) {
                    plausible.push((ci, app_id));
                }
            }
            if plausible.is_empty() {
                skipped_total += 1;
                if skipped.len() < 20 {
                    skipped.push(format!("Ligne {} : aucun AppID valide trouvé", total_lines));
                }
                continue;
            }
            // Year filtering (1000–2999):
            // - A 4-digit year is only a false positive when it competes with
            //   another candidate on the same line (e.g. "Jeu,2024,570").
            // - A 4-digit number standing alone on its line (single-cell line)
            //   is treated as a valid AppID.
            let has_year_competition = plausible.iter().any(|(_, id)| {
                id.len() == 4
                    && id.parse::<u16>().ok().is_some_and(|y| (1000..=2999).contains(&y))
            }) && plausible.len() > 1;
            let plausible: Vec<(usize, String)> = if has_year_competition {
                plausible.into_iter().filter(|(_, id)| {
                    !(id.len() == 4
                        && id.parse::<u16>().ok().is_some_and(|y| (1000..=2999).contains(&y)))
                }).collect()
            } else {
                plausible
            };
            if plausible.is_empty() {
                skipped_total += 1;
                if skipped.len() < 20 {
                    skipped.push(format!("Ligne {} : aucun AppID valide trouvé", total_lines));
                }
                continue;
            }
            // When multiple numbers exist and no header distinguishes them,
            // pick the longest digit string (an AppID is 2-7 digits).
            // At equal length, the first occurrence wins — this favours a
            // game ID like "123456" over a year "2024" in a line like
            // "Game,2024,123456" where the longer number is clearly the ID.
            // `Iterator::max_by_key` keeps the LAST maximum, which would silently
            // contradict the rule above. Compare on (len, Reverse(position)) so the
            // first occurrence wins at equal length.
            let (best_ci, raw_app_id) = plausible
                .into_iter()
                .enumerate()
                .max_by_key(|(pos, (_, id))| (id.len(), std::cmp::Reverse(*pos)))
                .map(|(_, entry)| entry)
                .unwrap_or_else(|| (0, cells[0].trim().to_string()));
            let app_id = strip_leading_zeros(&raw_app_id);
            if seen_ids.insert(app_id.clone()) {
                let name_cell = if best_ci > 0 { best_ci - 1 } else { best_ci + 1 };
                let name = if name_cell < cells.len() {
                    let n = cells[name_cell].trim().to_string();
                    if !n.is_empty() && !n.chars().all(|c| c.is_ascii_digit() || c == '.') {
                        Some(n)
                    } else {
                        None
                    }
                } else {
                    None
                };
                let is_known = known_set.contains(app_id.as_str());
                candidates.push(ImportCandidate {
                    app_id,
                    name,
                    known: is_known,
                });
            }
        }
    }

    ImportPreview {
        candidates,
        skipped,
        skipped_total,
        total_lines,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Export
// ─────────────────────────────────────────────────────────────────────────────

/// Serialise the library in the requested format. `Err` on an unknown format.
pub fn render_export(statuses: &[GameStatus], format: &str) -> Result<String, String> {
    match format {
        "csv" => Ok(to_csv(statuses)),
        "json" => to_json(statuses).map_err(|e| e.to_string()),
        _ => Err("format non supporté — choisissez csv ou json".to_string()),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Strip leading zeros from a numeric string: "00440" → "440", "0" → "0".
fn strip_leading_zeros(s: &str) -> String {
    let zero_count = s.chars().take_while(|c| *c == '0').count();
    if zero_count == 0 {
        s.to_string()
    } else if zero_count == s.len() {
        // All zeros → "0"
        "0".to_string()
    } else {
        s[zero_count..].to_string()
    }
}

fn escape_csv(field: &str) -> String {
    if field.contains(',') || field.contains('"') || field.contains('\n') || field.contains('\r') {
        let escaped = field.replace('"', "\"\"");
        format!("\"{}\"", escaped)
    } else {
        field.to_string()
    }
}

fn csv_field(val: &Option<String>) -> String {
    match val {
        Some(s) if !s.is_empty() => escape_csv(s),
        _ => String::new(),
    }
}

/// Parse a single line as CSV (no multi-line field support).
fn parse_csv_line(line: &str) -> Vec<String> {
    let mut cells = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();

    while let Some(c) = chars.next() {
        if in_quotes {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    current.push('"');
                    chars.next();
                } else {
                    in_quotes = false;
                }
            } else {
                current.push(c);
            }
        } else if c == '"' {
            in_quotes = true;
        } else if c == ',' {
            cells.push(current.clone());
            current.clear();
        } else {
            current.push(c);
        }
    }
    cells.push(current);
    cells
}

/// Parse the entire CSV text into rows, supporting quoted fields that span
/// multiple lines (RFC 4180).  Returns one `Vec<String>` per row.
fn parse_csv_text(text: &str) -> Vec<Vec<String>> {
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut chars = text.chars().peekable();

    while let Some(c) = chars.next() {
        if in_quotes {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    current.push('"');
                    chars.next();
                } else {
                    in_quotes = false;
                }
            } else {
                // Inside quotes, a newline is data rather than a record
                // separator (RFC 4180) — which is exactly why it takes the same
                // branch as any other character instead of ending the row.
                current.push(c);
            }
        } else if c == '"' {
            in_quotes = true;
        } else if c == ',' {
            row.push(current.clone());
            current.clear();
        } else if c == '\n' || c == '\r' {
            if c == '\r' && chars.peek() == Some(&'\n') {
                chars.next();
            }
            row.push(current.clone());
            current.clear();
            rows.push(row.clone());
            row.clear();
        } else {
            current.push(c);
        }
    }
    // Last row (may be incomplete if text doesn't end with newline).
    if !current.is_empty() || !row.is_empty() {
        row.push(current);
        rows.push(row);
    }
    rows
}

/// Extract a valid AppID (2-7 digits) from a cell.
///
/// Priority order:
/// 1. `store.steampowered.com/app/<id>` URL
/// 2. `steam://rungameid/<id>` URI
/// 3. Longest digit-run in the cell (first at ties), after rejecting dates,
///    decimals and version strings.
///
/// An AppID is 2–7 digits; single digits are rejected.
fn extract_appid_from_cell(cell: &str) -> Option<String> {
    // 1. Steam store URL
    if let Some(pos) = cell.find("store.steampowered.com/app/") {
        let rest = &cell[pos + "store.steampowered.com/app/".len()..];
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if (2..=7).contains(&digits.len()) {
            return Some(digits);
        }
    }
    // 2. steam://rungameid/<id>
    if let Some(pos) = cell.find("steam://rungameid/") {
        let rest = &cell[pos + 18..];
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if (2..=7).contains(&digits.len()) {
            return Some(digits);
        }
    }
    // 3. Reject dates (YYYY-MM-DD), decimals (19.99 or 19,99), versions (v1.2.3)
    let trimmed = cell.trim();
    let lower = trimmed.to_lowercase();
    // Date pattern: \d{4}-\d{2}-\d{2}
    if looks_like_date(trimmed) {
        return None;
    }
    // Decimal number: digits separated by . or , with no other letters
    if looks_like_decimal(trimmed) {
        return None;
    }
    // Version string: v1.2.3
    if looks_like_version(&lower) {
        return None;
    }
    // 4. Collect all digit runs, keep the longest (first at ties).
    // Year filtering (1000–2999) is handled at the line level in parse_import,
    // not here — a 4-digit number alone on its line is a valid AppID.
    let mut best: Option<String> = None;
    for token in cell.split(|c: char| !c.is_ascii_digit()) {
        if (2..=7).contains(&token.len()) && !token.is_empty() {
            match &best {
                None => best = Some(token.to_string()),
                Some(b) => {
                    if token.len() > b.len() {
                        best = Some(token.to_string());
                    }
                }
            }
        }
    }
    best
}

/// Returns true if the cell looks like a date: YYYY-MM-DD.
fn looks_like_date(s: &str) -> bool {
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != 3 {
        return false;
    }
    parts[0].chars().all(|c| c.is_ascii_digit())
        && parts[0].len() == 4
        && parts[1].chars().all(|c| c.is_ascii_digit())
        && parts[1].len() == 2
        && parts[2].chars().all(|c| c.is_ascii_digit())
        && parts[2].len() == 2
}

/// Returns true if the cell looks like a decimal number: digits.digits or digits,digits.
fn looks_like_decimal(s: &str) -> bool {
    let parts: Vec<&str> = s.split(['.', ',']).collect();
    if parts.len() != 2 {
        return false;
    }
    !parts[0].is_empty()
        && !parts[1].is_empty()
        && parts[0].chars().all(|c| c.is_ascii_digit())
        && parts[1].chars().all(|c| c.is_ascii_digit())
}

/// Returns true if the cell looks like a version string: v1.2.3
fn looks_like_version(s: &str) -> bool {
    let mut chars = s.chars();
    if chars.next() != Some('v') {
        return false;
    }
    let rest: String = chars.collect();
    // Must start with digit, then contain at least one dot-digit
    let mut iter = rest.splitn(2, '.');
    let first = iter.next().unwrap_or("");
    let second = iter.next();
    !first.is_empty()
        && first.chars().all(|c| c.is_ascii_digit())
        && second.is_some()
        && second.unwrap().chars().all(|c| c.is_ascii_digit() || c == '.')
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vdf::GameInstall;

    // A test fixture that mirrors GameStatus field for field; a builder here
    // would obscure what each test actually varies.
    #[allow(clippy::too_many_arguments)]
    fn make_status(
        app_id: &str,
        name: &str,
        stage: &'static str,
        tags: &[&str],
        added: Option<&str>,
        updated: Option<&str>,
        fix_installed: bool,
        hidden: bool,
    ) -> GameStatus {
        GameStatus {
            app_id: app_id.to_string(),
            name: name.to_string(),
            icon: None,
            updated_at: updated.map(|s| s.to_string()),
            added_at: added.map(|s| s.to_string()),
            in_library: true,
            lua_in_steam: true,
            fix_downloaded: false,
            hidden,
            tags: tags.iter().map(|s| s.to_string()).collect(),
            game: GameInstall {
                app_id: app_id.to_string(),
                known_to_steam: true,
                installed: true,
                fully_installed: true,
                ..Default::default()
            },
            playtime_minutes: None,
            last_played: None,
            fix: crate::fixes::FixReport {
                app_id: app_id.to_string(),
                health: if fix_installed {
                    crate::fixes::FixHealth::Healthy
                } else {
                    crate::fixes::FixHealth::NotInstalled
                },
                installed_at: None,
                game_dir: None,
                file_count: 0,
                missing: Vec::new(),
                modified: Vec::new(),
                has_backup: false,
                foreign: Vec::new(),
            },
            stage,
        }
    }

    // ── Export: CSV ──

    #[test]
    fn to_csv_empty_list_gives_header_only() {
        let csv = to_csv(&[]);
        assert!(csv.contains("app_id,name,stage,tags"));
        assert_eq!(csv.lines().count(), 1);
    }

    #[test]
    fn to_csv_escapes_comma_quote_and_newline() {
        let s = make_status(
            "42",
            "Rock, Paper, \"Scissors\"\nEdition",
            "ready",
            &["rpg", "co-op"],
            Some("2024-01-15T10:00:00Z"),
            Some("2024-06-20T14:30:00Z"),
            true, false,
        );
        let csv = to_csv(&[s]);
        assert!(csv.contains("\"Rock, Paper, \"\"Scissors\"\""));
        let preview = parse_import(&csv, &[]);
        assert_eq!(preview.candidates.len(), 1);
        assert_eq!(preview.candidates[0].app_id, "42");
        assert!(preview.candidates[0].name.as_deref().unwrap_or("").contains("Rock"));
        assert!(preview.candidates[0].name.as_deref().unwrap_or("").contains("Scissors"));
    }

    #[test]
    fn to_csv_boolean_and_date_formatting() {
        let s = make_status(
            "12345",
            "Simple Game",
            "ready",
            &[],
            Some("2024-03-01T00:00:00Z"),
            None,
            false, true,
        );
        let csv = to_csv(&[s]);
        assert!(csv.contains(",false,true\r\n"));
        assert!(csv.contains("2024-03-01T00:00:00Z"));
        let lines: Vec<&str> = csv.lines().collect();
        let fields: Vec<&str> = lines[1].split(',').collect();
        assert_eq!(fields[6], "false");
        assert_eq!(fields[7], "true");
    }

    // ── Export: JSON ──

    #[test]
    fn to_json_empty_list() {
        let json = to_json(&[]).unwrap();
        assert_eq!(json, "[]");
    }

    #[test]
    fn to_json_round_trip() {
        let s = make_status("99", "Json Test", "ready", &["test"], None, None, false, false);
        let json = to_json(&[s]).unwrap();
        assert!(json.contains("\"app_id\": \"99\""));
        assert!(json.contains("\"name\": \"Json Test\""));
    }

    // ── Import: Playnite CSV ──

    #[test]
    fn parse_playnite_csv() {
        let text = "Name,Platform,GameId,Release Date\nThe Witcher 3,PC,292030,2015\nCyberpunk 2077,PC,1091500,2020\n";
        let known: Vec<String> = vec!["292030".to_string()];
        let preview = parse_import(text, &known);
        assert_eq!(preview.candidates.len(), 2);
        assert_eq!(preview.candidates[0].app_id, "292030");
        assert!(preview.candidates[0].known);
        assert_eq!(preview.candidates[0].name, Some("The Witcher 3".to_string()));
        assert_eq!(preview.candidates[1].app_id, "1091500");
        assert!(!preview.candidates[1].known);
        assert_eq!(preview.candidates[1].name, Some("Cyberpunk 2077".to_string()));
        assert!(preview.skipped.is_empty());
    }

    // ── Import: bare list ──

    #[test]
    fn parse_bare_list() {
        let text = "440\n220\n730\n";
        let preview = parse_import(text, &[]);
        assert_eq!(preview.candidates.len(), 3);
        assert_eq!(preview.candidates[0].app_id, "440");
        assert_eq!(preview.candidates[1].app_id, "220");
        assert_eq!(preview.candidates[2].app_id, "730");
    }

    // ── Import: Steam URLs ──

    #[test]
    fn parse_steam_urls() {
        let text = "https://store.steampowered.com/app/440_DNA/\nsteam://rungameid/730\nhttps://store.steampowered.com/app/220\n";
        let preview = parse_import(text, &[]);
        assert_eq!(preview.candidates.len(), 3);
        let ids: Vec<&str> = preview.candidates.iter().map(|c| c.app_id.as_str()).collect();
        assert!(ids.contains(&"440"));
        assert!(ids.contains(&"730"));
        assert!(ids.contains(&"220"));
    }

    // ── Import: skipped lines ──

    #[test]
    fn parse_skips_unparseable_lines() {
        let text = "This is just text with no numbers\n440\nAnother random line\n";
        let preview = parse_import(text, &[]);
        assert_eq!(preview.candidates.len(), 1);
        assert_eq!(preview.candidates[0].app_id, "440");
        assert_eq!(preview.skipped.len(), 2);
        assert!(preview.skipped[0].contains("aucun AppID valide"));
    }

    // ── Import: deduplication ──

    #[test]
    fn parse_deduplicates() {
        let text = "440\n730\n440\n730\n";
        let preview = parse_import(text, &[]);
        assert_eq!(preview.candidates.len(), 2);
        assert_eq!(preview.candidates[0].app_id, "440");
        assert_eq!(preview.candidates[1].app_id, "730");
    }

    // ── Import: known flag ──

    #[test]
    fn parse_known_flag() {
        let known = vec!["440".to_string(), "100".to_string()];
        let text = "440\n730\n100\n";
        let preview = parse_import(text, &known);
        assert_eq!(preview.candidates.len(), 3);
        assert!(preview.candidates[0].known);
        assert!(!preview.candidates[1].known);
        assert!(preview.candidates[2].known);
    }

    // ── Import: false positive avoidance ──

    /// Documented deviation from the original spec, decided on purpose.
    ///
    /// Without a header there is no way to tell a year from an AppID: `1250` is
    /// Killing Floor and `2024` is a year, and both are four digits in the same
    /// range. Any rule is wrong somewhere, so we pick the direction that fails
    /// safely: the preview is read-only, so a bogus candidate is visible and
    /// costs the user one glance, whereas a dropped one is a silent loss — the
    /// exact failure this parser was rewritten to eliminate.
    ///
    /// A year therefore survives when it is the only candidate on its line, and
    /// is discarded as soon as a real AppID competes with it (see
    /// `parse_year_in_competition_wins_real_id`).
    #[test]
    fn lone_year_is_kept_as_appid() {
        let text = "Jeu,2024,19.99\n"; // 19.99 is rejected as a decimal
        let preview = parse_import(text, &[]);
        assert_eq!(preview.candidates.len(), 1);
        assert_eq!(preview.candidates[0].app_id, "2024");
    }

    #[test]
    fn extract_appid_half_life_no_candidate() {
        // "Half-Life 2" — the "2" is a single digit → rejected (min 2 digits).
        let text = "Half-Life 2\n";
        let preview = parse_import(text, &[]);
        assert_eq!(preview.candidates.len(), 0);
    }

    #[test]
    fn extract_appid_known_from_url() {
        // Cell is a URL → extract "440", compare against known set → known.
        let known = vec!["440".to_string()];
        let text = "https://store.steampowered.com/app/440/\n";
        let preview = parse_import(text, &known);
        assert_eq!(preview.candidates.len(), 1);
        assert_eq!(preview.candidates[0].app_id, "440");
        assert!(preview.candidates[0].known);
    }

    #[test]
    fn extract_appid_leading_zeros_stripped() {
        // "0000440" → stripped to "440", compared against known → known.
        let known = vec!["440".to_string()];
        let text = "0000440\n";
        let preview = parse_import(text, &known);
        assert_eq!(preview.candidates.len(), 1);
        assert_eq!(preview.candidates[0].app_id, "440");
        assert!(preview.candidates[0].known);
    }

    #[test]
    fn short_row_falls_into_skipped() {
        // Header has 3 columns but this row has only 1 → AppID column absent.
        let text = "Name,AppID,Stage\n440\n";
        let preview = parse_import(text, &[]);
        assert_eq!(preview.candidates.len(), 0);
        assert_eq!(preview.skipped_total, 1);
        assert!(preview.skipped.iter().any(|s| s.contains("colonne AppID absente")));
    }

    #[test]
    fn blank_lines_at_top_dont_shift_header() {
        // Two blank lines then header → header detected correctly.
        let text = "\n\nName,AppID,Stage\n440,440,ready\n";
        let preview = parse_import(text, &[]);
        assert_eq!(preview.candidates.len(), 1);
        assert_eq!(preview.candidates[0].app_id, "440");
    }

    #[test]
    fn skipped_total_counts_all_skipped() {
        // 1000 unparseable lines → skipped_total = 1000, skipped vec = 20.
        // Use only single-digit numbers and words with no digit runs ≥ 2.
        let text = (0..1000)
            .map(|i| format!("garbage-{}-line", i % 10)) // only single-digit runs
            .collect::<Vec<_>>()
            .join("\n") + "\n";
        let preview = parse_import(&text, &[]);
        assert_eq!(preview.skipped_total, 1000);
        assert_eq!(preview.skipped.len(), 20);
        assert_eq!(preview.candidates.len(), 0);
    }

    // ── Export: render_export ──

    #[test]
    fn render_export_csv() {
        let s = make_status("440", "TF2", "ready", &[], None, None, false, false);
        let result = render_export(&[s], "csv");
        assert!(result.is_ok());
        let csv = result.unwrap();
        assert!(csv.contains("app_id,name,stage"));
        assert!(csv.contains("440,TF2,ready"));
    }

    #[test]
    fn render_export_json() {
        let s = make_status("730", "CS2", "ready", &[], None, None, false, false);
        let result = render_export(&[s], "json");
        assert!(result.is_ok());
        let json = result.unwrap();
        assert!(json.contains("\"app_id\": \"730\""));
        assert!(json.contains("\"name\": \"CS2\""));
    }

    #[test]
    fn render_export_unknown_format() {
        let s = make_status("440", "TF2", "ready", &[], None, None, false, false);
        let result = render_export(&[s], "xml");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("non supporté"));
    }

    // ── Edge cases: field with only a quote, only a newline ──

    #[test]
    fn escape_csv_only_quote() {
        // A field containing only a double-quote should be quoted and escaped.
        // RFC 4180: " → "" inside a quoted field → result is four quotes.
        let field = "\"";
        let escaped = escape_csv(field);
        assert_eq!(escaped, "\"\"\"\"");
    }

    #[test]
    fn escape_csv_only_newline() {
        // A field containing only a newline should be quoted.
        let field = "\n";
        let escaped = escape_csv(field);
        assert!(escaped.starts_with('"'));
        assert!(escaped.ends_with('"'));
    }

    // ── Export: no BOM in to_csv (BOM is added only at write time) ──

    #[test]
    fn to_csv_has_no_bom() {
        let s = make_status("440", "Test", "ready", &[], None, None, false, false);
        let csv = to_csv(&[s]);
        // BOM is U+FEFF → bytes EF BB BF in UTF-8.
        // to_csv returns a Rust String (UTF-16 internally), so we check for the character.
        assert!(!csv.starts_with('\u{FEFF}'), "to_csv must not contain a BOM");
    }

    // ── Export: no BOM in to_csv (BOM is added only at write time) ──

    #[test]
    fn parse_own_csv_export() {
        let statuses = vec![
            make_status("440", "Team Fortress 2", "ready", &["fps", "free"], Some("2024-01-01T00:00:00Z"), None, true, false),
            make_status("730", "CS2", "ready", &["fps"], Some("2024-02-01T00:00:00Z"), Some("2024-05-01T00:00:00Z"), false, false),
        ];
        let csv = to_csv(&statuses);
        let preview = parse_import(&csv, &[]);
        assert_eq!(preview.candidates.len(), 2);
        assert_eq!(preview.candidates[0].app_id, "440");
        assert_eq!(preview.candidates[1].app_id, "730");
    }

    // ── Import: blank lines don't count ──

    #[test]
    fn parse_ignores_blank_lines() {
        let text = "440\n\n730\n\n\n220\n";
        let preview = parse_import(text, &[]);
        assert_eq!(preview.candidates.len(), 3);
        assert_eq!(preview.total_lines, 3);
    }

    // ── Import: Steam URL with trailing slash ──

    #[test]
    fn parse_steam_url_trailing_slash() {
        let text = "https://store.steampowered.com/app/440_DNA/\n";
        let preview = parse_import(text, &[]);
        assert_eq!(preview.candidates.len(), 1);
        assert_eq!(preview.candidates[0].app_id, "440");
    }

    // ── B2: steam://rungameid only — no header detection ──

    #[test]
    fn parse_steam_urls_no_header_first_game() {
        // Three steam:// URLs with no header — first line must NOT be mistaken for a header.
        let text = "steam://rungameid/730\nsteam://rungameid/440\nsteam://rungameid/220\n";
        let preview = parse_import(text, &[]);
        assert_eq!(preview.candidates.len(), 3);
        assert_eq!(preview.candidates[0].app_id, "730");
        assert_eq!(preview.candidates[1].app_id, "440");
        assert_eq!(preview.candidates[2].app_id, "220");
        assert_eq!(preview.skipped_total, 0);
    }

    // ── B3: header with leading zeros — known flag and dedup ──

    #[test]
    fn parse_header_leading_zeros_known() {
        // "0000440" with header → known=true because 440 is in the known set.
        let known = vec!["440".to_string()];
        let text = "Name,AppId\nTF2,0000440\n";
        let preview = parse_import(text, &known);
        assert_eq!(preview.candidates.len(), 1);
        assert_eq!(preview.candidates[0].app_id, "440");
        assert!(preview.candidates[0].known);
    }

    #[test]
    fn parse_header_dedup_leading_zeros() {
        // "0000440" then "440" → only one candidate (deduplicated).
        let text = "Name,AppId\nA,0000440\nB,440\n";
        let preview = parse_import(text, &[]);
        assert_eq!(preview.candidates.len(), 1);
        assert_eq!(preview.candidates[0].app_id, "440");
    }

    // ── C11: store.steampowered.com URL length ──

    #[test]
    fn parse_store_url_wins_over_generic_fallback() {
        // Discriminating on purpose: the generic fallback keeps the LONGEST digit
        // run, so it would return 1234567 here. Only a store-URL branch that
        // slices at the real prefix length (27, not 24) yields 440.
        let text = "https://store.steampowered.com/app/440/Team_Fortress_1234567/\n";
        let preview = parse_import(text, &[]);
        assert_eq!(preview.candidates.len(), 1);
        assert_eq!(preview.candidates[0].app_id, "440");
    }

    // ── C12: standalone 4-digit numbers that are valid AppIDs ──

    #[test]
    fn parse_equal_length_keeps_first_occurrence() {
        // Two candidates of the same length on one line: the rule is "first wins".
        // `max_by_key` alone would keep the last one, silently contradicting it.
        let text = "Jeu,123456,654321\n";
        let preview = parse_import(text, &[]);
        assert_eq!(preview.candidates.len(), 1);
        assert_eq!(preview.candidates[0].app_id, "123456");
    }

    #[test]
    fn parse_standalone_1250_is_candidate() {
        // 1250 alone on its line → valid AppID, not a year.
        let text = "1250\n2280\n2600\n";
        let preview = parse_import(text, &[]);
        assert_eq!(preview.candidates.len(), 3);
        assert_eq!(preview.candidates[0].app_id, "1250");
        assert_eq!(preview.candidates[1].app_id, "2280");
        assert_eq!(preview.candidates[2].app_id, "2600");
    }

    #[test]
    fn parse_year_in_competition_wins_real_id() {
        // "Jeu,2024,570" → 570 wins (2024 is a year in competition).
        let text = "Jeu,2024,570\n";
        let preview = parse_import(text, &[]);
        assert_eq!(preview.candidates.len(), 1);
        assert_eq!(preview.candidates[0].app_id, "570");
    }

    // ── Date rejection ──

    #[test]
    fn parse_full_date_rejected() {
        // "2024-01-15" is a full date → no candidate.
        let text = "2024-01-15\n";
        let preview = parse_import(text, &[]);
        assert_eq!(preview.candidates.len(), 0);
        assert_eq!(preview.skipped_total, 1);
    }

    // ── Header exact match ──

    #[test]
    fn parse_header_gameid_is_matched_case_insensitively() {
        // The comparison lowercases each cell, so "GameId" DOES match exactly and
        // the header is detected. Discriminating on purpose: the name column holds
        // a longer digit run than the ID, so bare mode would return 1234567 —
        // reading the declared column is the only way to get 123.
        let text = "Name,GameId\nGame 1234567,123\n";
        let preview = parse_import(text, &[]);
        assert_eq!(preview.candidates.len(), 1);
        assert_eq!(preview.candidates[0].app_id, "123");
        assert_eq!(preview.candidates[0].name.as_deref(), Some("Game 1234567"));
    }

    #[test]
    fn parse_header_exact_match_game_id() {
        // "Game Id" (with space) should match as appid header.
        let text = "Name,Game Id\nTest,123\n";
        let preview = parse_import(text, &[]);
        assert_eq!(preview.candidates.len(), 1);
        assert_eq!(preview.candidates[0].app_id, "123");
        assert_eq!(preview.candidates[0].name, Some("Test".to_string()));
    }
}
