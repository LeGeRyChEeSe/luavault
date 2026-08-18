# Changelog

This file is the **single source** of release notes. `scripts/publish-release.ps1`
reads it to build the `history` field of the update manifest; the app then shows,
in its update window, everything that changed **since the installed version** —
not just the latest one.

Two rules to follow when editing this file, because the script depends on them:

- a version heading is written exactly as `## <x.y.z> - <YYYY-MM-DD>` (ASCII dash);
- the lines kept as notes are the ones starting with `- `. Everything else (prose,
  subheadings) is ignored. The app displays them as-is, plain text: no bold, no
  links, one line per bullet.
- a bilingual version can split its bullets under `### fr` and `### en`. The
  publish script then places them separately in `notes_i18n`, with French kept as
  the `notes` fallback for older clients.

## 1.0.0 - 2026-08-15

### fr
- Première publication : bibliothèque `.lua` locale (import par glisser-déposer,
  sélecteur de fichiers, ou adoption automatique depuis `{Steam}\config\lua`).
- Patchs en ligne : téléchargement, sauvegarde du dossier avant application,
  contrôle d'intégrité SHA-256 et désinstallation qui restaure l'état d'origine.
- Bibliothèque : tags, recherche, tri, statistiques, défilement virtuel, sélection
  multiple avec actions groupées.
- Fiche de jeu façon Steam, temps de jeu et dernière session lus localement, flux
  Nouveautés agrégeant les notes de patch de toute la bibliothèque.
- Sauvegardes `.luabak` (cinq instantanés automatiques, export/import, format
  chiffré AES-256-GCM/Argon2id) et intégrité de l'index par signature locale (HMAC).
- Nettoyage granulaire et prévisualisable ; aucun niveau ne touche `steamapps`,
  `userdata` ni le compte Steam.
- Mise à jour automatique de l'application via GitHub Releases, avec manifeste
  signé (Ed25519) et vérification SHA-256 avant toute installation.
- Détection et réparation de Steam et SteamTools, thèmes clair/sombre, traduction
  intégrale français/anglais, raccourcis clavier et navigation accessible.

### en
- First release: local `.lua` library (drag-and-drop import, file picker, or
  automatic adoption from `{Steam}\config\lua`).
- Online fixes: download, folder backup before applying, SHA-256 integrity check,
  and uninstall that restores the original state.
- Library: tags, search, sort, statistics, virtual scroll, multi-selection with
  bulk actions.
- Steam-style game card, playtime and last session read locally, aggregated News
  feed with the whole library's patch notes.
- `.luabak` backups (five automatic rolling snapshots, export/import, encrypted
  AES-256-GCM/Argon2id format) and index integrity via a local HMAC signature.
- Granular, previewable cleanup; no level ever touches `steamapps`, `userdata` or
  the Steam account.
- Automatic app updates via GitHub Releases, with a signed (Ed25519) manifest and
  SHA-256 verification before any install.
- Steam and SteamTools detection/repair, light/dark themes, full French/English
  translation, keyboard shortcuts and accessible navigation.
