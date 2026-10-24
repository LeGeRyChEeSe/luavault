//! When no upstream answers for a manifest request code, fetch the manifest file itself
//! and put it where Steam looks first: `<Steam>\depotcache\<depot>_<gid>.manifest`.
//!
//! Steam asks for a request code only for a manifest it does not already hold, so a
//! verified file in `depotcache` makes the failing request moot. The log line that says
//! the code was refused carries the AppID, the depot and the manifest id, which is all the
//! providers need.
//!
//! Trust: a provider is a third party handing us a file Steam will read. Nothing is written
//! unless the bytes parse as a Steam manifest **and** its own metadata names the depot and
//! the manifest id that were asked for. A file that is already there is never replaced.
//! Hosts are fixed in this file, redirects are refused, bodies are read under a cap.
//!
//! Keys live in `manifest_keys.json` in the data dir, beside `hmac.key`: never in
//! `config.json`, so a backup export or import cannot carry them, and never logged.
//!
//! Both keys are obtained **by the user, on the provider's own page**, and pasted into the
//! app. Nothing here solves, drives or relays a verification: the app opens the page and
//! waits for the user to come back with a key.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};

/// The pages where the user gets a key by themselves. They live in the binary on purpose:
/// nothing a message, a config or the frontend says can aim the button anywhere else.
pub const MANIFESTHUB_SITE: &str = "https://manifesthub2.filegear-sg.me/";
pub const HUBCAP_SITE: &str = "https://hubcapmanifest.com/";
const MANIFESTHUB_MANIFEST_URL: &str = "https://api.manifesthub2.filegear-sg.me/manifest";
const HUBCAP_MANIFEST_URL: &str = "https://hubcapmanifest.com/api/v1/generate/manifest";

/// A manifest of a very large game is a few MiB; this is generous without being a hole.
const MAX_MANIFEST_BYTES: u64 = 32 * 1024 * 1024;
const MAX_PENDING: usize = 32;
/// A key is renewed this long before it expires, so one never dies mid-fetch.
const KEY_MARGIN_SECS: i64 = 120;
/// A ManifestHub key lasts 24 h from the moment the site hands it out. The user pastes it a
/// little later, so counting from the paste is a touch generous: the worst case is one
/// failed fetch, then the banner comes back.
const KEY_LIFETIME_SECS: i64 = 86_400;

const PAYLOAD_MAGIC: u32 = 0x71F6_17D0;
const METADATA_MAGIC: u32 = 0x1F48_12BE;
const END_MAGIC: u32 = 0x32C4_15AB;

// ------------------------------------------------------------------- what is missing

/// One manifest Steam asked for and could not get a request code for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Missing {
    pub app_id: u32,
    pub depot_id: u32,
    pub gid: u64,
}

fn number_after(line: &str, marker: &str) -> Option<u64> {
    let rest = &line[line.find(marker)? + marker.len()..];
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// `CDepotDownloadMgr::BYldRequestDepotManifest(App: 3883570, Depot: 3883571, Manifest:
/// 3377765570892908199, branch: ): Failed to get manifest request code, 'Access Denied'`
pub fn parse_missing(line: &str) -> Option<Missing> {
    if !line.contains("BYldRequestDepotManifest") || !line.contains("Failed to get manifest request code") {
        return None;
    }
    let app_id = u32::try_from(number_after(line, "App: ")?).ok()?;
    let depot_id = u32::try_from(number_after(line, "Depot: ")?).ok()?;
    let gid = number_after(line, "Manifest: ")?;
    if depot_id == 0 || gid == 0 {
        return None;
    }
    Some(Missing { app_id, depot_id, gid })
}

pub fn depotcache_path(steam: &Path, depot_id: u32, gid: u64) -> PathBuf {
    steam.join("depotcache").join(format!("{depot_id}_{gid}.manifest"))
}

static PENDING: Mutex<Vec<Missing>> = Mutex::new(Vec::new());

/// Remember a manifest to fetch. Steam retries every ~35 s, so the same one comes back
/// again and again: `true` only the first time (the moment worth a notification).
pub fn note_missing(m: Missing) -> bool {
    let mut p = PENDING.lock().unwrap();
    if p.contains(&m) {
        return false;
    }
    if p.len() >= MAX_PENDING {
        p.remove(0);
    }
    p.push(m);
    true
}

pub fn pending() -> Vec<Missing> {
    PENDING.lock().unwrap().clone()
}

pub fn forget(m: &Missing) {
    PENDING.lock().unwrap().retain(|x| x != m);
}

// ------------------------------------------------------------- the manifest file itself

fn read_u32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at.checked_add(4)?)?.try_into().ok()?))
}

fn read_varint(b: &[u8], pos: &mut usize) -> Option<u64> {
    let mut value = 0u64;
    for shift in (0..70).step_by(7) {
        let byte = *b.get(*pos)?;
        *pos += 1;
        if shift < 64 {
            value |= u64::from(byte & 0x7F) << shift;
        }
        if byte & 0x80 == 0 {
            return Some(value);
        }
    }
    None
}

/// `ContentManifestMetadata`: field 1 is the depot id, field 2 the manifest id.
fn metadata_ids(meta: &[u8]) -> Option<(u64, u64)> {
    let (mut depot, mut gid) = (None, None);
    let mut pos = 0;
    while pos < meta.len() {
        let tag = read_varint(meta, &mut pos)?;
        match (tag >> 3, tag & 7) {
            (1, 0) => depot = Some(read_varint(meta, &mut pos)?),
            (2, 0) => gid = Some(read_varint(meta, &mut pos)?),
            (_, 0) => {
                read_varint(meta, &mut pos)?;
            }
            (_, 1) => pos = pos.checked_add(8)?,
            (_, 5) => pos = pos.checked_add(4)?,
            (_, 2) => {
                let len = usize::try_from(read_varint(meta, &mut pos)?).ok()?;
                pos = pos.checked_add(len)?;
            }
            _ => return None,
        }
        if pos > meta.len() {
            return None;
        }
    }
    Some((depot?, gid?))
}

/// Is this a Steam manifest for exactly this depot and this manifest id? Sections are
/// `[magic][length][bytes]`, the first is the payload, the metadata names the ids.
pub fn manifest_matches(bytes: &[u8], depot_id: u32, gid: u64) -> bool {
    if read_u32(bytes, 0) != Some(PAYLOAD_MAGIC) {
        return false;
    }
    let mut pos = 0usize;
    while let Some(magic) = read_u32(bytes, pos) {
        if magic == END_MAGIC {
            return false;
        }
        let Some(len) = read_u32(bytes, pos + 4).map(|l| l as usize) else { return false };
        let start = pos + 8;
        let Some(end) = start.checked_add(len).filter(|e| *e <= bytes.len()) else { return false };
        if magic == METADATA_MAGIC {
            return metadata_ids(&bytes[start..end]) == Some((u64::from(depot_id), gid));
        }
        pos = end;
    }
    false
}

/// Some providers wrap the manifest in a zip holding one file. Anything else is returned as is.
fn unwrap_zip(bytes: Vec<u8>) -> Result<Vec<u8>> {
    if !bytes.starts_with(b"PK\x03\x04") {
        return Ok(bytes);
    }
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).context("archive illisible")?;
    if zip.len() != 1 {
        bail!("l'archive ne contient pas exactement un fichier");
    }
    let file = zip.by_index(0).context("entrée illisible")?;
    let mut out = Vec::new();
    file.take(MAX_MANIFEST_BYTES + 1).read_to_end(&mut out).context("lecture de l'archive")?;
    if out.len() as u64 > MAX_MANIFEST_BYTES {
        bail!("fichier plus grand que le plafond");
    }
    Ok(out)
}

/// Write the manifest next to Steam's own, atomically, and never over an existing one.
pub fn install_manifest(steam: &Path, m: &Missing, bytes: &[u8]) -> Result<bool> {
    if !manifest_matches(bytes, m.depot_id, m.gid) {
        bail!("le fichier reçu n'est pas le manifeste demandé");
    }
    let target = depotcache_path(steam, m.depot_id, m.gid);
    if target.exists() {
        return Ok(false);
    }
    let dir = target.parent().ok_or_else(|| anyhow!("dossier depotcache introuvable"))?;
    std::fs::create_dir_all(dir).context("création de depotcache")?;
    let tmp = target.with_extension("manifest.lv-tmp");
    std::fs::write(&tmp, bytes).context("écriture du manifeste")?;
    if let Err(e) = std::fs::rename(&tmp, &target) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e).context("mise en place du manifeste");
    }
    Ok(true)
}

// ------------------------------------------------------------------------------ keys

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredKey {
    pub key: String,
    /// Unix seconds.
    pub expires_at: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Keys {
    #[serde(default)]
    pub manifesthub: Option<StoredKey>,
    #[serde(default)]
    pub hubcap: Option<String>,
}

impl Keys {
    pub fn manifesthub_valid(&self, now: i64) -> Option<&str> {
        self.manifesthub.as_ref().filter(|k| now + KEY_MARGIN_SECS < k.expires_at).map(|k| k.key.as_str())
    }

    pub fn any_usable(&self, now: i64) -> bool {
        self.hubcap.is_some() || self.manifesthub_valid(now).is_some()
    }
}

fn plausible_key(key: &str) -> bool {
    (8..=128).contains(&key.len()) && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Hubcap keys begin with `smm`; anything else pasted here is a mistake worth refusing.
pub fn check_hubcap_key(key: &str) -> Result<String> {
    let key = key.trim();
    if !key.starts_with("smm") || !plausible_key(key) {
        bail!("ce n'est pas une clé Hubcap (elle commence par « smm »)");
    }
    Ok(key.to_string())
}

fn keys_path(data_dir: &Path) -> PathBuf {
    data_dir.join("manifest_keys.json")
}

/// An unreadable or absent file is "no key", never an error: the way out is to ask again.
pub fn load_keys(data_dir: &Path) -> Keys {
    std::fs::read(keys_path(data_dir)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn save_keys(data_dir: &Path, keys: &Keys) -> Result<()> {
    let path = keys_path(data_dir);
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec(keys)?).context("écriture des clés")?;
    std::fs::rename(&tmp, &path).context("remplacement des clés")?;
    Ok(())
}

/// A ManifestHub key as the site shows it: letters, digits, `-` and `_`. Refusing anything
/// else keeps a pasted sentence (or a page of HTML) out of a request header and a file.
pub fn check_manifesthub_key(key: &str, now: i64) -> Result<StoredKey> {
    let key = key.trim();
    if !plausible_key(key) {
        bail!("ce n'est pas une clé ManifestHub (lettres et chiffres, sans espace)");
    }
    if key.starts_with("smm") {
        bail!("cette clé commence par « smm » : c'est une clé Hubcap");
    }
    Ok(StoredKey { key: key.to_string(), expires_at: now + KEY_LIFETIME_SECS })
}

// ------------------------------------------------------------------------------ http

/// The client for these third-party hosts, in a newtype so `state.http` (openlua's spoofed
/// identity) can never be handed to them by mistake (pitfall 28).
#[derive(Clone)]
pub struct FallbackClient(reqwest::Client);

impl FallbackClient {
    pub fn new() -> Self {
        Self(
            reqwest::Client::builder()
                .use_rustls_tls()
                .no_gzip()
                .no_brotli()
                .no_zstd()
                // The hosts are fixed: a redirect is never a feature here.
                .redirect(reqwest::redirect::Policy::none())
                .timeout(std::time::Duration::from_secs(60))
                .user_agent(concat!("LuaVault/", env!("CARGO_PKG_VERSION")))
                .build()
                .expect("failed to build manifest http client"),
        )
    }
}

impl Default for FallbackClient {
    fn default() -> Self {
        Self::new()
    }
}

/// The wait used when a refusal for rate carries no `Retry-After`.
/// A manifest fetch waits out a short refusal itself, this many times at most, each wait
/// capped: the provider's limiter measured at 4 s, not minutes.
const FETCH_ATTEMPTS: u32 = 3;
const FETCH_WAIT_CAP_SECS: u64 = 15;

/// `Retry-After` in seconds (the HTTP-date form is not used by these services and reads as absent).
fn parse_retry_after(value: Option<&str>) -> Option<u64> {
    value?.trim().parse::<u64>().ok()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Hubcap,
    ManifestHub,
}

async fn fetch_from(client: &FallbackClient, provider: Provider, key: &str, m: &Missing) -> Result<Vec<u8>> {
    let (depot, gid) = (m.depot_id.to_string(), m.gid.to_string());
    let req = match provider {
        Provider::Hubcap => client
            .0
            .get(HUBCAP_MANIFEST_URL)
            .query(&[("depot_id", depot.as_str()), ("manifest_id", gid.as_str())])
            .bearer_auth(key),
        Provider::ManifestHub => client
            .0
            .get(MANIFESTHUB_MANIFEST_URL)
            .query(&[("apikey", key), ("depotid", depot.as_str()), ("manifestid", gid.as_str())]),
    };
    for attempt in 1..=FETCH_ATTEMPTS {
        let resp = req.try_clone().ok_or_else(|| anyhow!("requête non rejouable"))?.send().await.context("service injoignable")?;
        if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            let wait = parse_retry_after(resp.headers().get(reqwest::header::RETRY_AFTER).and_then(|v| v.to_str().ok()))
                .unwrap_or(5)
                .clamp(1, FETCH_WAIT_CAP_SECS);
            if attempt == FETCH_ATTEMPTS {
                bail!("trop de demandes");
            }
            log::debug!("manifestes de secours : {provider:?} limite le débit, nouvel essai dans {wait} s");
            tokio::time::sleep(std::time::Duration::from_secs(wait)).await;
            continue;
        }
        if !resp.status().is_success() {
            bail!("réponse {}", resp.status());
        }
        return unwrap_zip(crate::update::read_capped(resp, MAX_MANIFEST_BYTES).await?);
    }
    bail!("trop de demandes")
}

#[derive(Debug, PartialEq, Eq)]
pub enum Resolved {
    /// The file is in `depotcache` (we wrote it, or it was already there).
    Done,
    /// No provider has a key to try.
    NoKey,
    /// Keys were tried and no provider had a valid file.
    NotFound,
}

/// Try the providers that hold a key, Hubcap first (its key does not expire).
pub async fn resolve(client: &FallbackClient, steam: &Path, keys: &Keys, m: &Missing, now: i64) -> Resolved {
    if depotcache_path(steam, m.depot_id, m.gid).exists() {
        return Resolved::Done;
    }
    let mut tried = false;
    let mut attempts: Vec<(Provider, &str)> = Vec::new();
    if let Some(k) = keys.hubcap.as_deref() {
        attempts.push((Provider::Hubcap, k));
    }
    if let Some(k) = keys.manifesthub_valid(now) {
        attempts.push((Provider::ManifestHub, k));
    }
    for (provider, key) in attempts {
        tried = true;
        match fetch_from(client, provider, key, m).await {
            Ok(bytes) => match install_manifest(steam, m, &bytes) {
                Ok(_) => {
                    log::info!("manifestes de secours : {:?} fourni pour le dépôt {}", provider, m.depot_id);
                    return Resolved::Done;
                }
                Err(e) => log::warn!("manifestes de secours : {:?} refusé pour le dépôt {} ({e})", provider, m.depot_id),
            },
            Err(e) => log::debug!("manifestes de secours : {:?} sans réponse pour le dépôt {} ({e})", provider, m.depot_id),
        }
    }
    if tried {
        Resolved::NotFound
    } else {
        Resolved::NoKey
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REAL_LINE: &str = "[2026-10-24 05:36:08] CDepotDownloadMgr::BYldRequestDepotManifest(App: 3883570, Depot: 3883571, Manifest: 3377765570892908199, branch: ): Failed to get manifest request code, 'Access Denied'";

    fn varint(mut v: u64) -> Vec<u8> {
        let mut out = Vec::new();
        loop {
            let b = (v & 0x7F) as u8;
            v >>= 7;
            if v == 0 {
                out.push(b);
                return out;
            }
            out.push(b | 0x80);
        }
    }

    /// A manifest shaped like Steam's: payload, metadata (depot, gid, then a time and a
    /// bool the parser must step over), signature, end.
    fn manifest(depot: u32, gid: u64) -> Vec<u8> {
        let mut meta = vec![0x08];
        meta.extend(varint(u64::from(depot)));
        meta.push(0x10);
        meta.extend(varint(gid));
        meta.extend([0x18, 0x80, 0x80, 0x80, 0x08, 0x20, 0x01]);
        let mut out = Vec::new();
        for (magic, body) in [(PAYLOAD_MAGIC, vec![1u8, 2, 3]), (METADATA_MAGIC, meta), (0x1B81_B817, vec![])] {
            out.extend(magic.to_le_bytes());
            out.extend((body.len() as u32).to_le_bytes());
            out.extend(body);
        }
        out.extend(END_MAGIC.to_le_bytes());
        out
    }

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("lv-manifest-fallback-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn parses_the_real_log_line() {
        assert_eq!(parse_missing(REAL_LINE), Some(Missing { app_id: 3883570, depot_id: 3883571, gid: 3377765570892908199 }));
    }

    #[test]
    fn ignores_every_other_line() {
        for line in [
            "AppID 3883570 update canceled : Failed downloading 1 manifests (No connection)",
            "CDepotDownloadMgr::BYldRequestDepotManifest(App: 1, Depot: 2, Manifest: 3, branch: ): ok",
            "BYldRequestDepotManifest(App: x, Depot: 2, Manifest: 3): Failed to get manifest request code",
            "BYldRequestDepotManifest(App: 1, Depot: 0, Manifest: 3): Failed to get manifest request code",
            "",
        ] {
            assert_eq!(parse_missing(line), None, "{line}");
        }
    }

    #[test]
    fn a_manifest_is_accepted_only_for_its_own_depot_and_id() {
        let m = manifest(3883571, 3377765570892908199);
        assert!(manifest_matches(&m, 3883571, 3377765570892908199));
        assert!(!manifest_matches(&m, 3883572, 3377765570892908199));
        assert!(!manifest_matches(&m, 3883571, 3377765570892908198));
    }

    #[test]
    fn what_is_not_a_manifest_is_refused() {
        let good = manifest(7, 9);
        assert!(!manifest_matches(b"", 7, 9));
        assert!(!manifest_matches(b"<html>error</html>", 7, 9));
        assert!(!manifest_matches(&good[..good.len() - 30], 7, 9), "truncated");
        let mut no_meta = Vec::new();
        no_meta.extend(PAYLOAD_MAGIC.to_le_bytes());
        no_meta.extend(0u32.to_le_bytes());
        no_meta.extend(END_MAGIC.to_le_bytes());
        assert!(!manifest_matches(&no_meta, 7, 9));
        let mut lying = good.clone();
        lying[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(!manifest_matches(&lying, 7, 9), "a length past the end");
    }

    #[test]
    fn install_writes_once_and_never_replaces() {
        let steam = scratch("install");
        let m = Missing { app_id: 1, depot_id: 7, gid: 9 };
        assert!(install_manifest(&steam, &m, &manifest(7, 9)).unwrap());
        let path = depotcache_path(&steam, 7, 9);
        assert!(path.is_file());
        std::fs::write(&path, b"already here").unwrap();
        assert!(!install_manifest(&steam, &m, &manifest(7, 9)).unwrap());
        assert_eq!(std::fs::read(&path).unwrap(), b"already here");
        assert!(install_manifest(&steam, &Missing { gid: 10, ..m }, &manifest(7, 9)).is_err(), "wrong id writes nothing");
        assert!(!depotcache_path(&steam, 7, 10).exists());
        let leftovers: Vec<_> = std::fs::read_dir(steam.join("depotcache")).unwrap().flatten().collect();
        assert_eq!(leftovers.len(), 1, "no temp file left behind");
        let _ = std::fs::remove_dir_all(&steam);
    }

    #[test]
    fn a_zip_holding_one_manifest_is_unwrapped() {
        use std::io::Write;
        let inner = manifest(7, 9);
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut w = zip::ZipWriter::new(&mut buf);
            w.start_file("7_9.manifest", zip::write::SimpleFileOptions::default()).unwrap();
            w.write_all(&inner).unwrap();
            w.finish().unwrap();
        }
        assert_eq!(unwrap_zip(buf.into_inner()).unwrap(), inner);
        assert_eq!(unwrap_zip(inner.clone()).unwrap(), inner, "a bare manifest passes through");
    }

    #[test]
    fn pending_is_deduplicated_and_bounded() {
        let first = Missing { app_id: 1, depot_id: 1, gid: 1 };
        assert!(note_missing(first));
        assert!(!note_missing(first), "Steam retries every 35 s");
        for i in 2..(MAX_PENDING as u64 + 10) {
            note_missing(Missing { app_id: 1, depot_id: 1, gid: i });
        }
        assert!(pending().len() <= MAX_PENDING);
        forget(&first);
        assert!(!pending().contains(&first));
        PENDING.lock().unwrap().clear();
    }

    #[test]
    fn a_manifesthub_key_is_checked_and_dated_from_the_paste() {
        let now = 1_000_000;
        let k = check_manifesthub_key("  abcDEF12-_x 
", now).unwrap();
        assert_eq!(k, StoredKey { key: "abcDEF12-_x".into(), expires_at: now + KEY_LIFETIME_SECS });
        for bad in ["", "short", "has a space in it", "<script>alert(1)</script>", "smm1234567890abcdef", &"a".repeat(129)] {
            assert!(check_manifesthub_key(bad, now).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn retry_after_is_read_as_seconds_or_not_at_all() {
        assert_eq!(parse_retry_after(Some("4")), Some(4));
        assert_eq!(parse_retry_after(Some(" 30 ")), Some(30));
        assert_eq!(parse_retry_after(Some("Wed, 21 Oct 2026 07:28:00 GMT")), None);
        assert_eq!(parse_retry_after(Some("-1")), None);
        assert_eq!(parse_retry_after(None), None);
    }

    #[test]
    fn a_key_is_valid_until_shortly_before_it_expires() {
        let keys = Keys { manifesthub: Some(StoredKey { key: "abcdefgh".into(), expires_at: 1000 }), hubcap: None };
        assert_eq!(keys.manifesthub_valid(1000 - KEY_MARGIN_SECS - 1), Some("abcdefgh"));
        assert_eq!(keys.manifesthub_valid(1000 - KEY_MARGIN_SECS), None);
        assert!(!keys.any_usable(2000));
        assert!(Keys { hubcap: Some("smmabcdefgh".into()), ..Default::default() }.any_usable(2000));
    }

    #[test]
    fn hubcap_key_must_look_like_one() {
        assert_eq!(check_hubcap_key("  smm1234567890abcdef ").unwrap(), "smm1234567890abcdef");
        for bad in ["", "abc1234567890", "smm", "smm 123456789", "smm12345678\n9"] {
            assert!(check_hubcap_key(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn keys_round_trip_and_a_broken_file_reads_as_none() {
        let dir = scratch("keys");
        assert_eq!(load_keys(&dir), Keys::default());
        let keys = Keys { manifesthub: Some(StoredKey { key: "abcdefgh".into(), expires_at: 5 }), hubcap: Some("smmabcdefgh".into()) };
        save_keys(&dir, &keys).unwrap();
        assert_eq!(load_keys(&dir), keys);
        std::fs::write(keys_path(&dir), b"{not json").unwrap();
        assert_eq!(load_keys(&dir), Keys::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn resolve_without_a_key_says_so_and_with_the_file_present_says_done() {
        let steam = scratch("resolve");
        let client = FallbackClient::new();
        let m = Missing { app_id: 1, depot_id: 7, gid: 9 };
        assert_eq!(resolve(&client, &steam, &Keys::default(), &m, 0).await, Resolved::NoKey);
        install_manifest(&steam, &m, &manifest(7, 9)).unwrap();
        assert_eq!(resolve(&client, &steam, &Keys::default(), &m, 0).await, Resolved::Done);
        let _ = std::fs::remove_dir_all(&steam);
    }
}
