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

## 1.1.3 - 2026-10-19

### fr
- Le message du jour peut maintenant proposer de changer la source que SteamTools interroge pour télécharger les jeux. C'est ce qui corrige l'erreur « Pas de connexion Internet » qui apparaît à l'installation d'un jeu, puis disparaît au bout de plusieurs essais. Un clic, rien n'est redémarré, et le bouton indique s'il est déjà appliqué.

### en
- The message of the day can now offer to change the source SteamTools asks when downloading games. It is what fixes the "No internet connection" error that shows up when installing a game and then goes away after several tries. One click, nothing restarts, and the button says whether it is already applied.

## 1.1.2 - 2026-10-19

### fr
- Les boutons du message du jour disent maintenant s'ils ont déjà servi. Quand le retour à la version précédente de Steam est déjà en place, le message l'affiche au lieu de proposer le bouton une seconde fois. Un bouton qui n'est plus en place se représente tout seul, et un bouton dont l'effet ne peut pas être vérifié garde une petite option « Refaire ».

### en
- The buttons in the message of the day now tell you whether they have already been used. When the return to the previous Steam version is already in place, the message says so instead of offering the button a second time. A button whose effect is gone comes back on its own, and one whose effect cannot be checked keeps a small "Do it again" option.

## 1.1.1 - 2026-10-18

### fr
- Steam : le correctif en un clic. Quand une mise à jour de Steam casse SteamTools, le message du jour peut maintenant proposer des boutons, dont le retour à une version de Steam qui fonctionne, vérifiée avant d'être appliquée. L'onglet Steam & Outils garde le bouton qui annule ce retour, même sans message.
- Quand le correctif est nécessaire sur votre PC, l'application vous le rappelle : un bandeau reste affiché et le message se rouvre au démarrage. Rien n'est jamais imposé, et rien n'est bloqué : chaque action demande votre confirmation.
- Notifications Windows, si vous les activez : le message du jour et les nouvelles versions de l'application vous parviennent même quand elle est fermée, avec des boutons qui agissent sans ouvrir la fenêtre. Vérification à l'ouverture de session puis toutes les 6 heures, désactivable dans Réglages. Rien n'est lancé pendant qu'un jeu tourne.
- Correctif : les jeux ajoutés à Steam à la main ne bloquent plus l'affichage du temps de jeu.

### en
- Steam: the one-click fix. When a Steam update breaks SteamTools, the message of the day can now offer buttons, including a return to a working Steam version, checked before it is applied. The Steam & Tools tab keeps the button that undoes it, even without a message.
- When the fix is needed on your PC, the app reminds you: a banner stays up and the message reopens at startup. Nothing is ever forced and nothing is blocked: every action asks for your confirmation.
- Windows notifications, if you turn them on: the message of the day and new app versions reach you even when the app is closed, with buttons that act without opening the window. Checked at logon then every 6 hours, can be turned off in Settings. Nothing runs while a game is running.
- Fix: games added to Steam by hand no longer prevent playtime from showing.

## 1.1.0 - 2026-09-16

### fr
- Message du jour : l'auteur peut publier une notice signée (panne, avertissement, information) affichée au lancement pendant une durée choisie, sans mise à jour de l'application.
- La notice est traduite (français / anglais), mise en forme (gras, listes, liens, couleurs) et peut être écartée définitivement ; un bouton « Message du jour » la réaffiche à la demande.

### en
- Message of the day: the author can publish a signed notice (outage, warning, information) shown at launch for a chosen duration, with no application update required.
- The notice is translated (French / English), formatted (bold, lists, links, colours) and can be dismissed for good; a "Message of the day" button brings it back on demand.

## 1.0.2 - 2026-08-19

### fr
- IMPORTANT — si ce bouton "Installer" échoue encore avec « HTTP 404 »,
  c'est ce bug lui-même : téléchargez et réinstallez manuellement la
  dernière version depuis la page des Releases GitHub, une seule fois.
  Les mises à jour automatiques suivantes fonctionneront normalement.
- Correctif critique : la mise à jour depuis l'application échouait toujours
  avec « serveur de mise à jour : HTTP 404 » — l'URL de téléchargement de
  l'artefact ajoutait un segment `/download/<version>/` qui n'existe pas sur
  GitHub Releases. Le téléchargement pointe maintenant vers la bonne adresse.

### en
- IMPORTANT — if this "Install" button still fails with "HTTP 404", that's
  this exact bug: manually download and reinstall the latest version from
  the GitHub Releases page once. Automatic updates will work normally
  after that.
- Critical fix: updating from within the app always failed with "update
  server: HTTP 404" — the artefact download URL added a `/download/<version>/`
  segment that doesn't exist on GitHub Releases. Downloads now point at the
  correct address.

## 1.0.1 - 2026-08-19

### fr
- Correctifs : le bouton « Ouvrir le dossier » des Réglages qui échouait
  silencieusement, une indication obsolète pointant vers un onglet retiré de
  l'interface, une carte de jeu qui continuait à proposer une étape déjà
  terminée, et un point de terminaison de commande externe cassé.
- Crédits mis à jour ; retrait de la dernière mention de vérification
  Cloudflare et d'une entrée SteamDB obsolète.
- Ajout d'un badge Discord dans le README, pointant vers le serveur dédié.

### en
- Fixes: a Settings "Open folder" button that failed silently, a stale hint
  pointing at a removed UI tab, a game card that kept offering an
  already-completed step, and a broken external command endpoint.
- Updated credits; removed the last Cloudflare verification mention and a
  stale SteamDB entry.
- Added a Discord badge to the README, linking to the dedicated server.

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
