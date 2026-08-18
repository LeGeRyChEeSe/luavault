//! Read-only proxy over Steam's public store & news endpoints.
//!
//! Everything here is cosmetic — it feeds the big game card (artwork, pitch,
//! latest patch notes). A failure never blocks an action, it just leaves the
//! card sparser, so every field is optional and every call is best-effort.

use anyhow::{anyhow, Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

const APPDETAILS: &str = "https://store.steampowered.com/api/appdetails";
const NEWS: &str = "https://api.steampowered.com/ISteamNews/GetNewsForApp/v2/";

#[derive(Debug, Clone, Serialize, Default)]
pub struct Changelog {
    pub title: String,
    /// Unix seconds, as Steam reports it.
    pub date: i64,
    /// Plain text — Steam mixes BBCode and HTML in this field.
    pub body: String,
    pub url: String,
    /// The post is explicitly tagged `patchnotes` rather than being general news.
    pub is_patch_notes: bool,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct SteamDetails {
    pub app_id: String,
    pub name: String,
    pub short_description: String,
    pub header_image: Option<String>,
    pub background: Option<String>,
    pub capsule: Option<String>,
    pub developers: Vec<String>,
    pub publishers: Vec<String>,
    pub genres: Vec<String>,
    pub release_date: Option<String>,
    pub coming_soon: bool,
    pub metacritic: Option<u32>,
    pub price: Option<String>,
    pub website: Option<String>,
    pub screenshots: Vec<Shot>,
    pub changelog: Option<Changelog>,
}

/// One screenshot in both sizes: the strip shows `thumbnail`, the viewer
/// opens `full`. Kept as a pair so the card never has to guess a full-size
/// URL by rewriting the thumbnail's — Steam's naming is its own business.
#[derive(Debug, Clone, Serialize, Default)]
pub struct Shot {
    pub thumbnail: String,
    pub full: String,
}

// ------------------------------------------------------------- appdetails

#[derive(Debug, Deserialize)]
struct AppDetailsEnvelope {
    #[serde(default)]
    success: bool,
    #[serde(default)]
    data: Option<AppData>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
struct AppData {
    name: String,
    short_description: String,
    header_image: Option<String>,
    background_raw: Option<String>,
    background: Option<String>,
    capsule_imagev5: Option<String>,
    website: Option<String>,
    developers: Vec<String>,
    publishers: Vec<String>,
    genres: Vec<NamedItem>,
    release_date: Option<ReleaseDate>,
    metacritic: Option<Metacritic>,
    price_overview: Option<PriceOverview>,
    is_free: bool,
    screenshots: Vec<Screenshot>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
struct NamedItem {
    description: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
struct ReleaseDate {
    coming_soon: bool,
    date: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
struct Metacritic {
    score: u32,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
struct PriceOverview {
    final_formatted: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
struct Screenshot {
    path_thumbnail: String,
    /// The same shot at full size. The card shows the thumbnail (~116×65);
    /// blowing that one up would be a smear, so the viewer needs this one.
    /// Same response, no extra request.
    path_full: String,
}

// -------------------------------------------------------------- news feed

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct NewsEnvelope {
    appnews: NewsBody,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct NewsBody {
    newsitems: Vec<NewsItem>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct NewsItem {
    title: String,
    url: String,
    contents: String,
    date: i64,
    tags: Vec<String>,
}

/// Turn Steam's BBCode/HTML soup into readable plain text.
///
/// The result is rendered as text in the webview, never as markup — the store
/// feed is third-party content and must not reach the DOM as HTML.
pub fn to_plain_text(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            // HTML tags: <br> and </p> become line breaks, the rest vanish.
            '<' => {
                let mut tag = String::new();
                for c in chars.by_ref() {
                    if c == '>' {
                        break;
                    }
                    tag.push(c);
                }
                let lower = tag.to_lowercase();
                if lower.starts_with("br")
                    || lower.starts_with("/p")
                    || lower.starts_with("/div")
                    || lower.starts_with("li")
                    || lower.starts_with("/h")
                {
                    out.push('\n');
                }
                if lower.starts_with("li") {
                    out.push_str("• ");
                }
            }
            // BBCode: [b]…[/b], [list], [*]… — drop the markers, keep bullets.
            '[' => {
                let mut tag = String::new();
                for c in chars.by_ref() {
                    if c == ']' {
                        break;
                    }
                    tag.push(c);
                }
                let lower = tag.to_lowercase();
                if lower == "*" {
                    out.push_str("\n• ");
                } else if lower.starts_with("/list") || lower.starts_with("/h") {
                    out.push('\n');
                }
            }
            other => out.push(other),
        }
    }

    // Collapse the runs of blank lines the stripping leaves behind.
    let mut cleaned = String::with_capacity(out.len());
    let mut blank_run = 0;
    for line in out.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            blank_run += 1;
            if blank_run > 1 {
                continue;
            }
        } else {
            blank_run = 0;
        }
        cleaned.push_str(trimmed);
        cleaned.push('\n');
    }
    cleaned.trim().to_string()
}

fn decode_entities(text: &str) -> String {
    text.replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

async fn fetch_details(http: &Client, app_id: &str, lang: &str) -> Result<AppData> {
    let url = format!("{APPDETAILS}?appids={app_id}&l={lang}");
    let resp = http
        .get(&url)
        // Steam's store API rejects the LuaVault Origin/Referer defaults.
        .header(reqwest::header::ORIGIN, "https://store.steampowered.com")
        .header(reqwest::header::REFERER, "https://store.steampowered.com/")
        .send()
        .await
        .context("steam appdetails: erreur réseau")?;
    if !resp.status().is_success() {
        return Err(anyhow!("steam appdetails: HTTP {}", resp.status()));
    }
    let body: HashMap<String, AppDetailsEnvelope> = resp
        .json()
        .await
        .context("steam appdetails: JSON invalide")?;
    let envelope = body
        .get(app_id)
        .ok_or_else(|| anyhow!("steam appdetails: AppID absent de la réponse"))?;
    if !envelope.success {
        return Err(anyhow!("steam appdetails: fiche indisponible pour cet AppID"));
    }
    envelope
        .data
        .clone()
        .ok_or_else(|| anyhow!("steam appdetails: fiche vide"))
}

/// Keep at most `max` characters — never cutting a multi-byte character in
/// two — and append an ellipsis when the text was shortened.
fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let truncated: String = text.chars().take(max).collect();
    format!("{truncated}…")
}

/// A feed-sized excerpt: at most `max_chars` characters *in total*, ellipsis
/// included. Cut on characters, never bytes — a byte cut can land inside a
/// multi-byte accent and produce invalid text.
pub fn excerpt(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let cut: String = text.chars().take(max_chars.saturating_sub(1)).collect();
    format!("{cut}…")
}

/// The `limit` most recent posts for an app, newest first.
///
/// An HTTP failure, an invalid payload or a network cut is an `Err`; a valid
/// response without a single post is `Ok(vec![])` — a game that never
/// published is a real, cacheable state, not an error. Callers that must
/// keep the two apart (the aggregated feed) rely on this `Result`.
pub async fn changelogs(http: &Client, app_id: &str, limit: usize) -> Result<Vec<Changelog>> {
    let url = format!(
        "{NEWS}?appid={app_id}&count={limit}&maxlength=0&feeds=steam_community_announcements"
    );
    let resp = http
        .get(&url)
        .send()
        .await
        .context("steam news: erreur réseau")?;
    if !resp.status().is_success() {
        return Err(anyhow!("steam news: HTTP {}", resp.status()));
    }
    let envelope: NewsEnvelope = resp.json().await.context("steam news: JSON invalide")?;
    let mut items: Vec<Changelog> = envelope
        .appnews
        .newsitems
        .iter()
        .map(|item| {
            let body = to_plain_text(&decode_entities(&item.contents));
            Changelog {
                title: decode_entities(&item.title),
                date: item.date,
                // Long posts are read in the browser, not in a 500px-wide card.
                body: truncate_chars(&body, 4000),
                url: item.url.clone(),
                is_patch_notes: item.tags.iter().any(|t| t == "patchnotes"),
            }
        })
        .collect();
    // Steam already answers newest first, but the contract must not depend on it.
    items.sort_by_key(|b| std::cmp::Reverse(b.date));
    Ok(items)
}

/// Latest post for the app, preferring one tagged `patchnotes`.
///
/// The details card treats the changelog as a bonus: any failure degrades to
/// "no changelog", hence the `Option` — the aggregated feed is the caller
/// that keeps the error/empty-success distinction (see [`changelogs`]).
async fn fetch_changelog(http: &Client, app_id: &str) -> Option<Changelog> {
    let mut items = changelogs(http, app_id, 12).await.ok()?;
    if items.is_empty() {
        return None;
    }
    let idx = items.iter().position(|c| c.is_patch_notes).unwrap_or(0);
    Some(items.remove(idx))
}

/// Full card payload. The store fiche is required; the changelog is a bonus.
pub async fn details(http: &Client, app_id: &str, lang: &str) -> Result<SteamDetails> {
    let (data, changelog) = tokio::join!(
        fetch_details(http, app_id, lang),
        fetch_changelog(http, app_id)
    );
    let data = data?;
    let release = data.release_date.as_ref();
    Ok(SteamDetails {
        app_id: app_id.to_string(),
        name: data.name,
        short_description: to_plain_text(&decode_entities(&data.short_description)),
        header_image: data.header_image,
        background: data.background_raw.or(data.background),
        capsule: data.capsule_imagev5,
        developers: data.developers,
        publishers: data.publishers,
        genres: data.genres.into_iter().map(|g| g.description).collect(),
        release_date: release.map(|r| r.date.clone()).filter(|d| !d.is_empty()),
        coming_soon: release.map(|r| r.coming_soon).unwrap_or(false),
        metacritic: data.metacritic.map(|m| m.score),
        price: if data.is_free {
            Some("Gratuit".to_string())
        } else {
            data.price_overview.map(|p| p.final_formatted)
        },
        website: data.website.filter(|w| !w.is_empty()),
        screenshots: data
            .screenshots
            .into_iter()
            .filter(|s| !s.path_thumbnail.is_empty())
            .map(|s| Shot {
                // A shot Steam gives us without a full size still belongs in
                // the strip; the viewer then opens the thumbnail rather than
                // nothing at all.
                full: if s.path_full.is_empty() {
                    s.path_thumbnail.clone()
                } else {
                    s.path_full
                },
                thumbnail: s.path_thumbnail,
            })
            .take(8)
            .collect(),
        changelog,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_html_and_bbcode_into_readable_text() {
        let raw = "<h2>Fixes</h2><ul><li>Crash on boot</li><li>Audio glitch</li></ul>\
                   <br>See <a href=\"https://x\">here</a>.";
        let text = to_plain_text(raw);
        assert!(text.contains("• Crash on boot"));
        assert!(text.contains("• Audio glitch"));
        assert!(!text.contains('<'));
        // Blank-line runs are collapsed, so the card never shows a gaping hole.
        assert!(!text.contains("\n\n\n"));
    }

    #[test]
    fn strips_bbcode_lists() {
        let text = to_plain_text("[b]Notes[/b][list][*]One[*]Two[/list]");
        assert!(text.contains("Notes"));
        assert!(text.contains("• One"));
        assert!(text.contains("• Two"));
        assert!(!text.contains('['));
    }

    #[test]
    fn decodes_the_entities_steam_double_escapes() {
        assert_eq!(decode_entities("R&amp;D &quot;x&quot;"), "R&D \"x\"");
    }

    #[test]
    fn excerpt_cuts_on_chars_never_bytes() {
        // 500 accented characters = 1000 bytes. A byte-oriented cut near the
        // budget lands inside a character: either it panics, or it yields
        // fewer than 400 characters — both failures this test catches.
        let heavy: String = "é".repeat(500);
        let cut = excerpt(&heavy, 400);
        assert_eq!(cut.chars().count(), 400, "le budget est en caractères, pas en octets");
        assert!(cut.ends_with('…'), "un texte coupé porte l'ellipse");
        assert!(cut.starts_with('é'), "la coupe n'écrase pas le premier caractère");
    }

    #[test]
    fn excerpt_keeps_short_text_whole() {
        assert_eq!(excerpt("Bonjour", 400), "Bonjour");
        let exact: String = "a".repeat(400);
        assert_eq!(excerpt(&exact, 400), exact, "le budget exact n'est pas tronqué");
        let over: String = "a".repeat(401);
        assert_eq!(excerpt(&over, 400).chars().count(), 400);
    }

    #[test]
    fn truncate_chars_cuts_on_chars_never_bytes() {
        // Accented sibling of excerpt_cuts_on_chars_never_bytes, at the feed's
        // real 4000 budget: 4500 accented characters = 9000 bytes. A
        // byte-oriented cut at 4000 lands inside a character — it panics, or
        // yields fewer than 4000 kept characters. Both fail here.
        let heavy: String = "é".repeat(4500);
        let cut = truncate_chars(&heavy, 4000);
        assert_eq!(
            cut.chars().count(),
            4001,
            "4000 caractères gardés plus l'ellipse — le budget est en caractères, pas en octets"
        );
        assert!(cut.ends_with('…'), "un texte coupé porte l'ellipse");
        assert!(cut.starts_with('é'), "la coupe n'écrase pas le premier caractère");
        // Exactly at the budget: untouched, no ellipsis.
        let exact: String = "é".repeat(4000);
        assert_eq!(truncate_chars(&exact, 4000), exact);
    }

    #[test]
    #[ignore = "hits the live Steam store API"]
    fn live_details_for_subnautica() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let http = reqwest::Client::new();
            for lang in ["french", "english"] {
                let details = details(&http, "264710", lang).await.expect("details");
                assert_eq!(details.name, "Subnautica");
                assert!(details.header_image.is_some());
                assert!(!details.short_description.is_empty());

                // The viewer opens `full`, so prove Steam really sends it rather
                // than assuming: a fixture cannot show that the field exists in
                // the live payload, and a silent fallback to the thumbnail would
                // give a smeared image nobody notices until they click.
                assert!(!details.screenshots.is_empty(), "des captures sont attendues ({lang})");
                for shot in &details.screenshots {
                    assert!(!shot.thumbnail.is_empty(), "vignette vide ({lang})");
                    assert!(!shot.full.is_empty(), "taille réelle vide ({lang})");
                }
                let distinct = details
                    .screenshots
                    .iter()
                    .filter(|s| s.full != s.thumbnail)
                    .count();
                assert!(
                    distinct > 0,
                    "aucune capture n'a d'URL pleine taille distincte de sa vignette ({lang}) — \
                     path_full n'est pas remonté"
                );
                println!(
                    "{lang}: {} capture(s), {distinct} avec une pleine taille distincte\n  vignette: {}\n  pleine : {}",
                    details.screenshots.len(),
                    details.screenshots[0].thumbnail,
                    details.screenshots[0].full
                );
            }
        });
    }
}
