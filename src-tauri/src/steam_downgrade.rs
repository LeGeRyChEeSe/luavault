//! Steam pinned to a known-good client build, in one gesture.
//!
//! Nothing is embedded: no binary, and no compiled-in pin either. The date and the
//! hash come from a signed MOTD action, so the author can aim the button at the next
//! broken build — or withdraw it once the tool works with current Steam — without a
//! release, and the UI offers it only while such a message is live.
//!
//! Why it exists: a Steam client update (2026-10-13) broke SteamTools, and the only
//! fix people had was a manual downgrade. The procedure is documented by the
//! community (`-overridepackageurl`), but done by hand it is a dozen steps, and the
//! obvious shortcut — pointing Steam at the Wayback Machine — does not work: Steam
//! appends `?t=<epoch>` to its requests, and Wayback answers a URL it never archived
//! with its *nearest* capture, which was the broken build's manifest. Measured on
//! 2026-10-17: the first attempt "succeeded" and changed nothing.
//!
//! What does work, and what this module does:
//! 1. fetch the one manifest (`steam_client_win64`, 7 KiB) of an archived capture;
//! 2. **verify its SHA-256 against a pin** — the manifest decides which binaries
//!    Steam installs into its own folder, so an unauthenticated one is arbitrary
//!    code. The pin comes from a signed MOTD action;
//! 3. serve it on `127.0.0.1` and answer 404 to everything else: Steam then falls
//!    back to its own CDN for the packages, which stores them by hash and still has
//!    the old ones;
//! 4. run `steam.exe -forcesteamupdate -forcepackagedownload -overridepackageurl
//!    http://127.0.0.1:<port> -exitsteam`, check the installed version is the
//!    manifest's, then write `steam.cfg` so Steam does not update itself again.
//!
//! Steam is closed and relaunched, so every entry point is behind a confirmation
//! in the UI; nothing here is reachable without one.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use log::{debug, info, warn};
use serde::Serialize;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use crate::update::{self, UpdateClient};

const MANIFEST_NAME: &str = "steam_client_win64";
/// The real one is 7 379 bytes; a manifest is a list of packages, never more.
const MAX_MANIFEST_SIZE: u64 = 64 * 1024;

/// The two keys that stop Steam from updating itself (`steam.cfg`, read by the
/// bootstrapper). They are the only lines this module ever writes or removes.
const PIN_KEYS: [(&str, &str); 2] = [
    ("BootStrapperInhibitAll", "enable"),
    ("BootStrapperForceSelfUpdate", "disable"),
];

const STEAM_UPDATE_TIMEOUT: Duration = Duration::from_secs(600);
const STEAM_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(90);

// -------------------------------------------------------------------- the pin

/// Which archived manifest to install, and the hash it must have.
#[derive(Debug, Clone, PartialEq)]
pub struct Pin {
    /// 14 digits, `YYYYMMDDhhmmss`, a Wayback capture of the manifest.
    pub snapshot: String,
    /// 64 lowercase hex characters.
    pub sha256: String,
}

impl Pin {
    /// Build a pin from untrusted text (a MOTD action). Both fields are checked
    /// for shape here because the snapshot lands in a URL: only digits are
    /// accepted, so it cannot carry a path, a host or a query.
    pub fn parse(snapshot: &str, sha256: &str) -> Option<Self> {
        let snapshot = snapshot.trim();
        let sha256 = sha256.trim().to_ascii_lowercase();
        let digits = snapshot.len() == 14 && snapshot.bytes().all(|b| b.is_ascii_digit());
        let hex = sha256.len() == 64 && sha256.bytes().all(|b| b.is_ascii_hexdigit());
        (digits && hex).then(|| Self { snapshot: snapshot.to_string(), sha256 })
    }

    /// The raw (`id_`) capture of the manifest. The host is fixed: a signed
    /// message may choose a date, never a server.
    pub fn manifest_url(&self) -> String {
        format!(
            "https://web.archive.org/web/{}id_/https://media.steampowered.com/client/{MANIFEST_NAME}",
            self.snapshot
        )
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// `true` only when `bytes` hash to the pin. Compared on the hex text, which is
/// not a secret: this is an integrity check, not a MAC.
pub fn matches_pin(bytes: &[u8], pin: &Pin) -> bool {
    sha256_hex(bytes) == pin.sha256
}

/// The `"version"` value of a Steam package manifest.
pub fn manifest_version(text: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let mut parts = line.split('"').filter(|s| !s.trim().is_empty());
        (parts.next()? == "version").then(|| parts.next().map(str::to_string))?
    })
}

// -------------------------------------------------------------------- steam.cfg

/// `steam.cfg` with the pin lines present. Other lines stay as they were; a key
/// already there with another value is corrected, not duplicated.
pub fn with_pin(existing: &str) -> String {
    let mut out: Vec<String> = existing
        .lines()
        .filter(|l| !is_pin_key(l))
        .map(str::to_string)
        .collect();
    out.extend(PIN_KEYS.iter().map(|(k, v)| format!("{k}={v}")));
    out.join("\r\n") + "\r\n"
}

/// `steam.cfg` without the pin lines, or `None` when nothing else is left (the
/// file is then deleted: an empty one is noise in Steam's folder).
pub fn without_pin(existing: &str) -> Option<String> {
    let kept: Vec<&str> = existing.lines().filter(|l| !is_pin_key(l)).collect();
    if kept.iter().all(|l| l.trim().is_empty()) {
        None
    } else {
        Some(kept.join("\r\n") + "\r\n")
    }
}

fn is_pin_key(line: &str) -> bool {
    let key = line.split('=').next().unwrap_or("").trim();
    PIN_KEYS.iter().any(|(k, _)| k.eq_ignore_ascii_case(key))
}

fn cfg_path(steam_dir: &Path) -> PathBuf {
    steam_dir.join("steam.cfg")
}

/// Whether Steam is currently told not to update itself.
pub fn is_pinned(steam_dir: &Path) -> bool {
    let text = std::fs::read_to_string(cfg_path(steam_dir)).unwrap_or_default();
    text.lines().any(|l| {
        let mut kv = l.splitn(2, '=');
        let k = kv.next().unwrap_or("").trim();
        let v = kv.next().unwrap_or("").trim();
        k.eq_ignore_ascii_case("BootStrapperInhibitAll") && v.eq_ignore_ascii_case("enable")
    })
}

fn write_pin(steam_dir: &Path) -> Result<()> {
    let path = cfg_path(steam_dir);
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    std::fs::write(&path, with_pin(&existing)).context("écriture de steam.cfg")
}

fn remove_pin(steam_dir: &Path) -> Result<()> {
    let path = cfg_path(steam_dir);
    let Ok(existing) = std::fs::read_to_string(&path) else {
        return Ok(());
    };
    match without_pin(&existing) {
        Some(rest) => std::fs::write(&path, rest).context("écriture de steam.cfg"),
        None => std::fs::remove_file(&path).context("suppression de steam.cfg"),
    }
}

/// The installed client's package version, from Steam's own manifest.
pub fn installed_version(steam_dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(steam_dir.join("package").join("steam_client_win64.manifest")).ok()?;
    manifest_version(&text)
}

// -------------------------------------------------------------------- fetching

/// Fetch the archived manifest and refuse it unless it hashes to the pin.
/// Wayback answers 503 now and then, so three tries.
pub async fn fetch_manifest(http: &UpdateClient, pin: &Pin) -> Result<Vec<u8>> {
    fetch_manifest_from(http, &pin.manifest_url(), pin).await
}

/// Inflate a gzip body, refusing anything that grows past `limit`: the input is
/// not authenticated yet, so a few KiB must not become gigabytes (pitfall 29).
fn gunzip_capped(bytes: &[u8], limit: u64) -> Result<Vec<u8>> {
    use std::io::Read;
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(bytes)
        .take(limit + 1)
        .read_to_end(&mut out)
        .context("manifeste gzip illisible")?;
    if out.len() as u64 > limit {
        bail!("manifeste décompressé au-delà du plafond de {limit} octets");
    }
    Ok(out)
}

async fn fetch_manifest_from(http: &UpdateClient, url: &str, pin: &Pin) -> Result<Vec<u8>> {
    let mut last = anyhow!("aucune tentative");
    for attempt in 1..=3u64 {
        match http.inner().get(url).send().await {
            Ok(resp) if resp.status().is_success() => {
                let raw = update::read_capped(resp, MAX_MANIFEST_SIZE).await?;
                // Wayback may hand the capture back gzipped whatever we ask
                // (`Content-Encoding: gzip` even to `Accept-Encoding: identity`),
                // and the client deliberately decodes nothing. The pin is checked on
                // what comes out, and what comes out is capped.
                let bytes = if raw.starts_with(&[0x1f, 0x8b]) { gunzip_capped(&raw, MAX_MANIFEST_SIZE)? } else { raw };
                if !matches_pin(&bytes, pin) {
                    // Not retried: the archive will not change its mind.
                    bail!("le manifeste téléchargé ne correspond pas à l'empreinte attendue : refusé");
                }
                return Ok(bytes);
            }
            Ok(resp) => last = anyhow!("réponse {}", resp.status()),
            Err(e) => last = anyhow!("{e}"),
        }
        debug!("steam_downgrade: manifeste, tentative {attempt} : {last}");
        tokio::time::sleep(Duration::from_secs(2 * attempt)).await;
    }
    Err(last.context("manifeste Steam archivé indisponible"))
}

// -------------------------------------------------------------------- local server

/// Serve `manifest` on `127.0.0.1:<random port>`, and nothing else: any other path
/// is a 404, which is what sends Steam to its own CDN for the packages. The query
/// string Steam appends is ignored. Returns the port and the task to abort.
pub async fn serve_manifest(manifest: Vec<u8>) -> Result<(u16, tokio::task::JoinHandle<()>)> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.context("serveur local")?;
    let port = listener.local_addr()?.port();
    let handle = tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else { break };
            let body = manifest.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 4096];
                let mut len = 0;
                // Read up to the end of the headers, never more than the buffer.
                while len < buf.len() {
                    match sock.read(&mut buf[len..]).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => len += n,
                    }
                    if buf[..len].windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
                let head = String::from_utf8_lossy(&buf[..len]);
                let path = head.split_whitespace().nth(1).unwrap_or("");
                let path = path.split('?').next().unwrap_or("");
                let reply = if path == format!("/{MANIFEST_NAME}") {
                    let mut r = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    )
                    .into_bytes();
                    r.extend_from_slice(&body);
                    r
                } else {
                    b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
                };
                let _ = sock.write_all(&reply).await;
                let _ = sock.shutdown().await;
            });
        }
    });
    Ok((port, handle))
}

// -------------------------------------------------------------------- steam processes

#[cfg(windows)]
fn hidden(cmd: &mut std::process::Command) -> &mut std::process::Command {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW)
}

#[cfg(not(windows))]
fn hidden(cmd: &mut std::process::Command) -> &mut std::process::Command {
    cmd
}

fn steam_running() -> bool {
    let out = hidden(
        std::process::Command::new("tasklist").args(["/FI", "IMAGENAME eq steam.exe", "/NH"]),
    )
    .output();
    out.map(|o| String::from_utf8_lossy(&o.stdout).to_ascii_lowercase().contains("steam.exe"))
        .unwrap_or(false)
}

/// Ask Steam to quit and wait until it has. `Ok(())` when it was not running.
fn shutdown_steam(exe: &Path) -> Result<()> {
    if !steam_running() {
        return Ok(());
    }
    let _ = hidden(std::process::Command::new(exe).arg("-shutdown")).spawn().map(|mut c| c.wait());
    let start = Instant::now();
    while steam_running() {
        if start.elapsed() > STEAM_SHUTDOWN_TIMEOUT {
            bail!("Steam ne s'est pas fermé : fermez-le puis recommencez");
        }
        std::thread::sleep(Duration::from_secs(2));
    }
    // The webhelpers outlive steam.exe by a moment and hold files in package\.
    std::thread::sleep(Duration::from_secs(3));
    Ok(())
}

/// Run the bootstrapper against the local server and wait for it to exit.
fn run_override(exe: &Path, port: u16) -> Result<()> {
    let url = format!("http://127.0.0.1:{port}");
    let mut child = hidden(std::process::Command::new(exe).args([
        "-forcesteamupdate",
        "-forcepackagedownload",
        "-overridepackageurl",
        &url,
        "-exitsteam",
    ]))
    .spawn()
    .context("lancement de Steam")?;
    let start = Instant::now();
    loop {
        if child.try_wait()?.is_some() {
            return Ok(());
        }
        if start.elapsed() > STEAM_UPDATE_TIMEOUT {
            let _ = child.kill();
            bail!("la mise à niveau de Steam n'a pas abouti dans le temps imparti");
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}

fn steam_exe(steam_dir: &Path) -> Result<PathBuf> {
    let exe = steam_dir.join("steam.exe");
    if exe.is_file() {
        Ok(exe)
    } else {
        bail!("steam.exe introuvable dans le dossier Steam")
    }
}

// -------------------------------------------------------------------- entry points

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Status {
    /// Steam is told not to update itself (`steam.cfg`).
    pub pinned: bool,
    /// The client's package version, when readable.
    pub installed_version: Option<String>,
}

pub fn status(steam_dir: &Path) -> Status {
    Status {
        pinned: is_pinned(steam_dir),
        installed_version: installed_version(steam_dir),
    }
}

/// Downgrade Steam to the pinned build, block its self-update, restart it.
/// Returns the version now installed.
pub async fn apply(http: &UpdateClient, steam_dir: &Path, pin: &Pin) -> Result<String> {
    let exe = steam_exe(steam_dir)?;
    let manifest = fetch_manifest(http, pin).await?;
    let wanted = manifest_version(&String::from_utf8_lossy(&manifest))
        .ok_or_else(|| anyhow!("manifeste sans numéro de version"))?;
    info!("steam_downgrade: cible {wanted} (capture {})", pin.snapshot);

    let (port, server) = serve_manifest(manifest).await?;
    let dir = steam_dir.to_path_buf();
    let exe2 = exe.clone();
    // Closing Steam and waiting for the bootstrapper takes tens of seconds.
    let ran = tauri::async_runtime::spawn_blocking(move || -> Result<()> {
        shutdown_steam(&exe2)?;
        // A previous pin would make the bootstrapper ignore the forced update.
        remove_pin(&dir)?;
        run_override(&exe2, port)
    })
    .await;
    server.abort();
    ran.map_err(|e| anyhow!("{e}"))??;

    let now = installed_version(steam_dir);
    if now.as_deref() != Some(wanted.as_str()) {
        warn!("steam_downgrade: version installée {now:?}, attendue {wanted}");
        bail!(
            "Steam n'est pas revenu à la version attendue (installée : {}). Relancez Steam normalement : rien n'est bloqué.",
            now.as_deref().unwrap_or("inconnue")
        );
    }
    write_pin(steam_dir)?;
    let exe3 = exe;
    tauri::async_runtime::spawn_blocking(move || crate::install::restart_steam(&exe3))
        .await
        .map_err(|e| anyhow!("{e}"))??;
    info!("steam_downgrade: terminé, version {wanted}, mises à jour bloquées");
    Ok(wanted)
}

/// Let Steam update itself again: remove the pin and relaunch it, which is when
/// the bootstrapper looks for an update.
pub async fn undo(steam_dir: &Path) -> Result<()> {
    let exe = steam_exe(steam_dir)?;
    remove_pin(steam_dir)?;
    tauri::async_runtime::spawn_blocking(move || crate::install::restart_steam(&exe))
        .await
        .map_err(|e| anyhow!("{e}"))??;
    info!("steam_downgrade: mises à jour de Steam rétablies");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::update::tests::{http_ok, spawn_raw_server};

    /// 2026-10-06 capture, verified working on 2026-10-17 (test data, not shipped).
    const SNAPSHOT: &str = "20261006145653";
    const SHA: &str = "40ec2f2aac99fb65c62b7b60db77e8e1575e0c8b59cfeba80294eeb1c963731c";

    fn known_good() -> Pin {
        Pin { snapshot: SNAPSHOT.into(), sha256: SHA.into() }
    }

    const REAL_MANIFEST_HEAD: &str =
        "\"win64\"\n{\n\t\"version\"\t\t\"1788652215\"\n\t\"ostype\"\t\t\"win10\"\n\t\"tenfoot_images_all\"\n\t{\n\t\t\"file\"\t\t\"tenfoot_images_all.zip.86419c7a\"\n";

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("lv-steamdown-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    // ── pin ──

    #[test]
    fn the_reference_pin_is_well_formed() {
        let p = known_good();
        assert_eq!(Pin::parse(&p.snapshot, &p.sha256), Some(p));
    }

    #[test]
    fn pin_parse_refuses_anything_that_could_reach_the_url() {
        let ok = SHA;
        assert!(Pin::parse("20261006145653", ok).is_some());
        for bad in ["", "2026100614565", "202610061456530", "2026100614565x", "../../evil/x1234", "20261006145653/", "2026 006145653"] {
            assert!(Pin::parse(bad, ok).is_none(), "{bad:?}");
        }
        for bad in ["", "abc", &ok[..63], &"g".repeat(64)] {
            assert!(Pin::parse("20261006145653", bad).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn pin_parse_normalises_hash_case() {
        let up = SHA.to_uppercase();
        assert_eq!(Pin::parse(SNAPSHOT, &up).unwrap().sha256, SHA);
    }

    #[test]
    fn manifest_url_has_a_fixed_host_and_the_snapshot_only() {
        let u = known_good().manifest_url();
        assert_eq!(
            u,
            "https://web.archive.org/web/20261006145653id_/https://media.steampowered.com/client/steam_client_win64"
        );
    }

    #[test]
    fn matches_pin_compares_the_real_hash() {
        let body = b"hello";
        let pin = Pin { snapshot: "20261006145653".into(), sha256: sha256_hex(body) };
        assert!(matches_pin(body, &pin));
        assert!(!matches_pin(b"hellp", &pin));
        // The compiled-in pin is not the hash of an empty body or of garbage.
        assert!(!matches_pin(b"", &known_good()));
    }

    // ── manifest version ──

    #[test]
    fn manifest_version_reads_the_real_shape() {
        assert_eq!(manifest_version(REAL_MANIFEST_HEAD).as_deref(), Some("1788652215"));
    }

    #[test]
    fn manifest_version_is_none_without_a_version_line() {
        assert_eq!(manifest_version("\"win64\"\n{\n}\n"), None);
        assert_eq!(manifest_version(""), None);
        // A package called "version…" must not be read as the version.
        assert_eq!(manifest_version("\"versions\"\n{\n\t\"file\"\t\"x\"\n}"), None);
    }

    // ── steam.cfg ──

    #[test]
    fn with_pin_adds_both_lines_and_keeps_the_rest() {
        let out = with_pin("SomethingElse=1\r\n");
        assert!(out.contains("SomethingElse=1"));
        assert!(out.contains("BootStrapperInhibitAll=enable"));
        assert!(out.contains("BootStrapperForceSelfUpdate=disable"));
    }

    #[test]
    fn with_pin_is_idempotent_and_corrects_a_wrong_value() {
        let once = with_pin("bootstrapperinhibitall=disable\r\n");
        assert_eq!(once.matches("InhibitAll").count() + once.matches("inhibitall").count(), 1);
        assert!(once.contains("BootStrapperInhibitAll=enable"));
        assert_eq!(with_pin(&once), once);
    }

    #[test]
    fn without_pin_removes_only_our_keys() {
        let cfg = with_pin("Other=2\r\n");
        let rest = without_pin(&cfg).expect("another line remains");
        assert!(rest.contains("Other=2"));
        assert!(!rest.contains("BootStrapper"));
    }

    #[test]
    fn without_pin_deletes_the_file_when_nothing_is_left() {
        assert_eq!(without_pin(&with_pin("")), None);
        assert_eq!(without_pin(""), None);
    }

    #[test]
    fn pin_roundtrip_on_disk() {
        let d = tmp("cfg");
        assert!(!is_pinned(&d));
        write_pin(&d).unwrap();
        assert!(is_pinned(&d));
        remove_pin(&d).unwrap();
        assert!(!is_pinned(&d));
        assert!(!cfg_path(&d).exists(), "an empty steam.cfg must not be left behind");
        remove_pin(&d).unwrap(); // absent file: not an error
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn is_pinned_needs_the_enable_value() {
        let d = tmp("cfg2");
        std::fs::write(cfg_path(&d), "BootStrapperInhibitAll=disable\r\n").unwrap();
        assert!(!is_pinned(&d));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn installed_version_reads_steams_own_manifest() {
        let d = tmp("ver");
        assert_eq!(installed_version(&d), None);
        std::fs::create_dir_all(d.join("package")).unwrap();
        std::fs::write(d.join("package").join("steam_client_win64.manifest"), REAL_MANIFEST_HEAD).unwrap();
        assert_eq!(installed_version(&d).as_deref(), Some("1788652215"));
        let _ = std::fs::remove_dir_all(&d);
    }

    // ── fetch ──

    #[tokio::test]
    async fn fetch_accepts_the_pinned_bytes_and_refuses_others() {
        let body = REAL_MANIFEST_HEAD.as_bytes();
        let pin = Pin { snapshot: "20261006145653".into(), sha256: sha256_hex(body) };
        let mut routes = std::collections::HashMap::new();
        routes.insert("/m".to_string(), http_ok(body));
        let base = spawn_raw_server(routes).await;
        let http = UpdateClient::wrap(reqwest::Client::new());
        let got = fetch_manifest_from(&http, &format!("{base}/m"), &pin).await.expect("pinned");
        assert_eq!(got, body);

        let other = Pin { sha256: sha256_hex(b"autre chose"), ..pin };
        let err = fetch_manifest_from(&http, &format!("{base}/m"), &other).await.unwrap_err();
        assert!(err.to_string().contains("empreinte"), "{err}");
    }

    fn gzip(data: &[u8]) -> Vec<u8> {
        use std::io::Write;
        let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        e.write_all(data).unwrap();
        e.finish().unwrap()
    }

    #[tokio::test]
    async fn fetch_inflates_a_gzipped_capture_before_checking_the_pin() {
        let body = REAL_MANIFEST_HEAD.as_bytes();
        let pin = Pin { snapshot: SNAPSHOT.into(), sha256: sha256_hex(body) };
        let mut routes = std::collections::HashMap::new();
        routes.insert("/m".to_string(), http_ok(&gzip(body)));
        let base = spawn_raw_server(routes).await;
        let http = UpdateClient::wrap(reqwest::Client::new());
        assert_eq!(fetch_manifest_from(&http, &format!("{base}/m"), &pin).await.unwrap(), body);
    }

    #[test]
    fn gunzip_refuses_a_bomb() {
        let bomb = gzip(&vec![0u8; 1_000_000]);
        assert!(bomb.len() < 4096, "the fixture must be small on the wire");
        assert!(gunzip_capped(&bomb, 64 * 1024).is_err());
        assert!(gunzip_capped(&gzip(b"ok"), 64 * 1024).is_ok());
        assert!(gunzip_capped(&[0x1f, 0x8b, 0, 1, 2], 64 * 1024).is_err(), "truncated stream");
    }

    // ── local server ──

    async fn get(port: u16, path: &str) -> (String, Vec<u8>) {
        let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        s.write_all(format!("GET {path} HTTP/1.1\r\nHost: x\r\n\r\n").as_bytes()).await.unwrap();
        let mut out = Vec::new();
        s.read_to_end(&mut out).await.unwrap();
        let split = out.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
        (String::from_utf8_lossy(&out[..split]).to_string(), out[split..].to_vec())
    }

    #[tokio::test]
    async fn server_serves_the_manifest_with_or_without_steams_query() {
        let (port, h) = serve_manifest(b"MANIFEST".to_vec()).await.unwrap();
        for path in ["/steam_client_win64", "/steam_client_win64?t=1792245664"] {
            let (head, body) = get(port, path).await;
            assert!(head.starts_with("HTTP/1.1 200"), "{path}: {head}");
            assert_eq!(body, b"MANIFEST");
        }
        h.abort();
    }

    #[tokio::test]
    async fn server_answers_404_to_everything_else() {
        let (port, h) = serve_manifest(b"MANIFEST".to_vec()).await.unwrap();
        for path in ["/", "/bins_win64.zip.vz.abc_1", "/steam_client_win64/x", "/../steam_client_win64x", "/steam_client_win32"] {
            let (head, body) = get(port, path).await;
            assert!(head.starts_with("HTTP/1.1 404"), "{path}: {head}");
            assert!(body.is_empty());
        }
        h.abort();
    }

    #[tokio::test]
    async fn server_listens_on_loopback_only() {
        let (port, h) = serve_manifest(Vec::new()).await.unwrap();
        // The listener's own address: not 0.0.0.0, or the LAN could read it.
        let probe = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        assert!(probe.peer_addr().unwrap().ip().is_loopback());
        h.abort();
    }

    #[test]
    fn status_reports_an_untouched_folder() {
        let d = tmp("status");
        let s = status(&d);
        assert!(!s.pinned);
        assert_eq!(s.installed_version, None);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// The only check that notices Wayback losing the capture, or the pin and
    /// the archive drifting apart. `cargo test live_ -- --ignored`.
    #[tokio::test]
    #[ignore]
    async fn live_known_good_manifest_matches_its_pin() {
        let http = UpdateClient::new();
        let pin = known_good();
        let bytes = fetch_manifest(&http, &pin).await.expect("archived and pinned");
        assert_eq!(manifest_version(&String::from_utf8_lossy(&bytes)).as_deref(), Some("1788652215"));
    }
}
