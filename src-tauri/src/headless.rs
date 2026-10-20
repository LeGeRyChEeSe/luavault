//! The two ways Windows starts this binary without a window.
//!
//! `--notify-check` — at logon and every few hours (the `Run` key and the scheduled
//! task `notify::register` wrote): fetch the signed message and the signed update
//! manifest, raise a toast for what is new, exit. `--motd-action <url>` — a toast was
//! clicked: the custom URL is parsed strictly, its single-use nonce consumed, then
//! either a message's action is looked up in the freshly verified message and run
//! (`motd_run::execute`), or the update is downloaded, verified and its installer
//! opened. The outcome is reported by a second toast. Neither opens a window,
//! neither touches the webview.
//!
//! Both run before Tauri starts (`run` in `lib.rs`): a headless start must not
//! initialise a window, a log plugin or the single large `AppState`.

use std::io::Write;

use chrono::Utc;

use crate::config::{self, AppConfig};
use crate::notify::{self, Activation};
use crate::update::{self, UpdateClient};
use crate::{detect, motd, motd_run};

/// Appended to `notify.log` in the data dir, trimmed at 64 KiB: a headless start
/// has no window and no log plugin, and "the toast never came" needs a trace.
fn log_line(msg: &str) {
    let path = config::data_dir().join("notify.log");
    if std::fs::metadata(&path).map(|m| m.len() > 64 * 1024).unwrap_or(false) {
        let _ = std::fs::remove_file(&path);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "{} {msg}", Utc::now().format("%Y-%m-%dT%H:%M:%SZ"));
    }
}

/// `Some(exit code)` when this start was a headless one, `None` for the app.
pub fn run_if_requested() -> Option<i32> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str)? {
        "--notify-check" => Some(tauri::async_runtime::block_on(notify_check())),
        "--motd-action" => {
            let Some(url) = args.get(1) else {
                log_line("--motd-action sans adresse");
                return Some(2);
            };
            Some(tauri::async_runtime::block_on(motd_action(url)))
        }
        _ => None,
    }
}

fn lang(cfg: &AppConfig) -> &'static str {
    notify::pick_language(cfg.locale.as_deref(), notify::system_locale().as_deref())
}

/// French or English, by the notification's language.
fn say<'a>(lang: &str, fr: &'a str, en: &'a str) -> &'a str {
    if lang == "fr" { fr } else { en }
}

/// The offer this binary would get from `base`, as the window's own check
/// computes it (`commands::check_update`).
async fn current_offer(http: &UpdateClient, _cfg: &AppConfig, base: &str) -> Option<update::UpdateAvailable> {
    let manifest = update::fetch_verified_manifest(http, base).await?;
    update::evaluate_manifest(manifest, env!("CARGO_PKG_VERSION"), config::is_portable())
}

async fn notify_check() -> i32 {
    // Said first, whatever follows: this line is what proves Windows started the
    // executable (the scheduled task, the logon entry) — see the live registration test.
    log_line("vérification lancée");
    let cfg = AppConfig::load().unwrap_or_default();
    // The registrations outlive a "no" only if something went wrong: honour the
    // choice anyway.
    if cfg.background_notify != Some(true) {
        log_line("vérification ignorée : notifications en arrière-plan non activées");
        return 0;
    }
    let http = UpdateClient::new();
    // Two addresses, not one: the notice and the update manifest are not always served
    // from the same place (an edition may keep its notice on a release of its own, and
    // its manifest on another). Asking the notice's base for the manifest finds nothing,
    // and no update toast ever comes; each toast then remembers ITS base for the click.
    let motd_base = motd::base_url();
    let update_base = update::base_url();
    let lang = lang(&cfg);
    let now = Utc::now().timestamp();
    let mut state = notify::load_state();
    state.prune(now);

    // (what to log, the toast) — the nonces are issued while building them.
    let mut toasts: Vec<(String, String)> = Vec::new();

    if let Some(message) = motd::fetch_verified_motd(&http, &motd_base)
        .await
        .and_then(|doc| motd::evaluate(doc, Utc::now()))
    {
        // A message that offers the Steam fix goes to the people who need it, not to
        // someone without SteamTools or already fixed (`motd::relevant_for_toast`).
        let facts = motd::SteamFacts::read(cfg.steam_dir.clone().or_else(detect::detect_steam_path).as_deref());
        if notify::should_notify(&message.id, cfg.motd_dismissed_id.as_deref(), state.last_notified_id.as_deref())
            && motd::relevant_for_toast(&message, &facts)
        {
            let xml = notify::build_toast(&message, lang, &mut state, &motd_base, now);
            toasts.push((format!("message {}", message.id), xml));
        }
    }

    if let Some(offer) = current_offer(&http, &cfg, &update_base).await {
        if notify::wants_update_toast(&offer)
            && notify::should_notify_update(
                &offer.version,
                cfg.update_notified_version.as_deref(),
                state.last_update_notified.as_deref(),
            )
        {
            let xml = notify::build_update_toast(&offer.version, env!("CARGO_PKG_VERSION"), lang, &mut state, &update_base, now);
            toasts.push((format!("mise à jour {}", offer.version), xml));
        }
    }

    if toasts.is_empty() {
        log_line("vérification terminée : rien de nouveau");
        return 0;
    }
    // Persist the nonces BEFORE showing: a button whose nonce was never saved
    // would be a dead button.
    if let Err(e) = notify::save_state(&state) {
        log_line(&format!("état non inscriptible : {e}"));
        return 1;
    }
    let mut code = 0;
    for (what, xml) in &toasts {
        match notify::show_toast(xml) {
            Ok(()) => log_line(&format!("notification affichée pour {what}")),
            Err(e) => {
                log_line(&format!("notification impossible pour {what} : {e}"));
                code = 1;
            }
        }
    }
    // `Show` returns before the platform has taken the toast over.
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
    code
}

async fn motd_action(raw: &str) -> i32 {
    let Some(activation) = notify::parse_activation(raw) else {
        log_line("adresse d'activation refusée");
        return 0;
    };
    let now = Utc::now().timestamp();
    let mut state = notify::load_state();
    let nonce = match &activation {
        Activation::Open { nonce } | Activation::Run { nonce } => nonce.clone(),
    };
    let Some(pending) = state.consume(&nonce, now) else {
        // Forged, replayed or expired: nothing happens, and nothing says why.
        log_line("jeton inconnu, déjà utilisé ou périmé : ignoré");
        return 0;
    };
    let _ = notify::save_state(&state);
    let what = match (&pending.update_version, &pending.action_id) {
        (Some(v), _) => format!("bouton mise à jour {v}"),
        (None, Some(a)) => format!("bouton {a}"),
        (None, None) => "corps du toast".to_string(),
    };
    log_line(&format!("clic reçu : {}, {what}", pending.message_id));

    if pending.downloads {
        // « Files detected »: nothing to fetch or run. Leave a note for the running app and,
        // when none runs, open it. The files wait in the window's own list.
        if let Err(e) = crate::downloads::request_wake(now) {
            log_line(&format!("note de réveil non écrite : {e}"));
        }
        open_app();
        return 0;
    }

    let cfg = AppConfig::load().unwrap_or_default();
    let lang = lang(&cfg);
    // Ask where the toast's message came from: this process was started by Windows
    // with its own environment, which may not be the one `--notify-check` ran in.
    let base = pending.base.clone().unwrap_or_else(motd::base_url);

    if let Some(version) = pending.update_version.as_deref() {
        update_click(version, &base, &cfg, lang).await;
        return 0;
    }

    let Some(action_id) = pending.action_id.as_deref() else {
        open_app();
        return 0;
    };

    let http = UpdateClient::new();
    let resolved = match motd::fetch_verified_motd(&http, &base).await {
        Some(doc) => motd::evaluate(doc, Utc::now())
            .filter(|m| m.id == pending.message_id)
            .and_then(|m| motd::find_action(&m, action_id, Utc::now())),
        None => None,
    };
    let Some(resolved) = resolved else {
        log_line(&format!("message {} introuvable, expiré ou signature refusée ({base})", pending.message_id));
        report(
            lang,
            false,
            say(
                lang,
                "Ce message n'est plus actif : ouvrez l'application pour voir l'état actuel.",
                "This message is no longer active: open the app to see the current state.",
            ),
        );
        return 0;
    };

    let steam_dir = cfg.steam_dir.clone().or_else(detect::detect_steam_path);
    match motd_run::execute(&http, steam_dir, resolved).await {
        Ok(text) => {
            log_line(&format!("action {action_id} effectuée"));
            // The window reads this ledger to say "already done" instead of offering the button again.
            let mut done = cfg.clone();
            done.motd_actions_done =
                motd::record_run(&done.motd_actions_done, &pending.message_id, action_id, Utc::now().timestamp());
            if let Err(e) = done.save() {
                log_line(&format!("registre des boutons non écrit : {e}"));
            }
            if !text.is_empty() {
                report(lang, true, &text);
            }
        }
        Err(e) => {
            log_line(&format!("action {action_id} refusée ou échouée : {e}"));
            report(lang, false, &e);
        }
    }
    0
}

/// The « update » button: the window's own path, without the window. Read the
/// signed manifest again (the version announced must still be the offered one),
/// download, verify size and SHA-256, re-hash against the pair
/// just recorded, then open the installer.
async fn update_click(version: &str, base: &str, cfg: &AppConfig, lang: &str) {
    let http = UpdateClient::new();
    let Some(offer) = current_offer(&http, cfg, base)
        .await
        .filter(|o| o.version == version && notify::wants_update_toast(o))
    else {
        log_line(&format!("mise à jour {version} : l'offre a changé ou n'est plus accessible ({base})"));
        report(
            lang,
            false,
            say(
                lang,
                "La version proposée a changé : ouvrez l'application pour voir la mise à jour actuelle.",
                "The offered version has changed: open the app to see the current update.",
            ),
        );
        return;
    };
    let artifact = offer.artifacts[0].clone();

    report(
        lang,
        true,
        say(lang, "Téléchargement de la mise à jour…", "Downloading the update…"),
    );
    let path = match update::download_and_verify_from(
        &http,
        base,
        &offer.version,
        &artifact.file,
        &artifact.sha256,
        artifact.size,
    )
    .await
    {
        Ok(p) => p,
        Err(e) => {
            log_line(&format!("mise à jour : téléchargement refusé ou échoué : {e}"));
            report(lang, false, &e.to_string());
            return;
        }
    };

    let verified = Some((path.clone(), artifact.sha256.clone()));
    let target = match tauri::async_runtime::spawn_blocking(move || update::validate_install_path(&path, &verified)).await {
        Ok(Ok(t)) => t,
        Ok(Err(e)) => {
            log_line(&format!("mise à jour : fichier refusé : {e}"));
            report(lang, false, &e);
            return;
        }
        Err(e) => {
            log_line(&format!("mise à jour : vérification interrompue : {e}"));
            return;
        }
    };

    // Remember the version we update FROM, before the installer starts, so the next
    // launch can say whether it worked (`commands::install_update` does the same).
    let mut cfg = AppConfig::load().unwrap_or_default();
    cfg.update_from_version = Some(env!("CARGO_PKG_VERSION").to_string());
    let _ = cfg.save();

    if !target.to_string_lossy().to_ascii_lowercase().ends_with(".exe") {
        log_line("mise à jour : ce n'est pas un installeur, rien n'est lancé");
        report(lang, false, say(lang, "Fichier inattendu : mise à jour annulée.", "Unexpected file: update cancelled."));
        return;
    }
    match tauri_plugin_opener::open_path(target.to_string_lossy().as_ref(), None::<&str>) {
        Ok(()) => log_line(&format!("mise à jour {version} : installeur ouvert")),
        Err(e) => {
            log_line(&format!("mise à jour : ouverture de l'installeur impossible : {e}"));
            report(lang, false, &format!("ouverture de l'installeur : {e}"));
        }
    }
}

/// A toast that says how it went, then a moment for the platform to take it.
fn report(lang: &str, ok: bool, text: &str) {
    let title = match (ok, lang == "fr") {
        (true, true) => "Action effectuée",
        (true, false) => "Done",
        (false, true) => "Action impossible",
        (false, false) => "Could not do it",
    };
    let xml = notify::toast_xml(title, &notify::plain_text(text, 220), "", &[]);
    if let Err(e) = notify::show_toast(&xml) {
        log_line(&format!("notification de résultat impossible : {e}"));
    }
    std::thread::sleep(std::time::Duration::from_millis(1500));
}

/// Open the app, unless one is already running (this start counts as one).
fn open_app() {
    let Ok(exe) = std::env::current_exe() else { return };
    let image = exe.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    if instances_running(&image) > 1 {
        return;
    }
    let _ = std::process::Command::new(exe).spawn();
}

#[cfg(windows)]
fn instances_running(image: &str) -> usize {
    use std::os::windows::process::CommandExt;
    let out = std::process::Command::new("tasklist")
        .args(["/FI", &format!("IMAGENAME eq {image}"), "/NH"])
        .creation_flags(0x0800_0000)
        .output();
    out.map(|o| {
        String::from_utf8_lossy(&o.stdout)
            .lines()
            .filter(|l| l.to_ascii_lowercase().contains(&image.to_ascii_lowercase()))
            .count()
    })
    .unwrap_or(0)
}

#[cfg(not(windows))]
fn instances_running(_image: &str) -> usize {
    0
}
