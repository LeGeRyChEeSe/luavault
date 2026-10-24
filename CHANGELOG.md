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

## 1.1.6 - 2026-10-24

### fr
- Quand aucun serveur ne donne à Steam le code d'un fichier de jeu, l'application peut maintenant aller chercher ce fichier elle-même et le poser là où Steam le cherche en premier. Steam réessaie ensuite tout seul. Cela demande une clé gratuite que vous obtenez vous-même sur le site d'un fournisseur : l'application n'automatise jamais la vérification Cloudflare.
- Une notification vous prévient. Un clic dessus ouvre la page où obtenir la clé dans votre navigateur et amène sur l'onglet Steam & Outils, où il suffit de la coller. Une bannière reste en haut de tous les onglets tant qu'un fichier manque, et le menu de l'icône près de l'horloge propose la même chose : le mode « Ne pas déranger » de Windows cache les notifications, pas ces deux-là.
- Deux fournisseurs sont possibles : une clé de 24 heures, ou une clé sans date limite qui demande une connexion Discord pour être créée. Les clés restent sur l'ordinateur et n'entrent jamais dans une sauvegarde.
- Le fichier récupéré est vérifié avant d'être posé (il doit être celui du jeu demandé) et un fichier déjà présent n'est jamais remplacé.

### en
- When no server gives Steam the code for a game file, the app can now fetch that file itself and put it where Steam looks first. Steam then retries on its own. This needs a free key that you get yourself on a provider's site: the app never automates the Cloudflare check.
- A notification tells you. One click opens the page where the key is issued in your browser and takes you to the Steam & Tools tab, where you just paste it. A banner stays at the top of every tab while a file is missing, and the menu of the icon next to the clock offers the same: Windows' Do Not Disturb hides notifications, not those two.
- Two providers are supported: a 24-hour key, or a key with no expiry that needs a Discord sign-in to create. Keys stay on the computer and never enter a backup.
- The fetched file is checked before it is put in place (it must be the one for the requested game) and a file that is already there is never replaced.

## 1.1.5 - 2026-10-21

### fr
- L'application voit arriver vos fichiers .lua, patchs en ligne et bypass DRM téléchargés, d'où qu'ils viennent, tant qu'elle tourne (fenêtre ouverte ou réduite près de l'horloge). Fenêtre au premier plan : une fenêtre vous propose de les ajouter. Sinon : une notification liste les jeux détectés et se met à jour à chaque nouveau fichier ; un clic dessus rouvre l'application. Rien n'est ajouté sans votre accord.
- Le bouton « Fichiers détectés » de la bibliothèque liste aussi les anciens fichiers de vos dossiers, pour les ajouter plus tard en un clic en choisissant lesquels. Un fichier déjà disponible dans l'application n'est pas reproposé.
- Chaque ligne affiche le nom du jeu et le type de fichier (fichier .lua, patch en ligne, bypass DRM). Les archives sont ouvertes avec votre mot de passe par défaut avant d'être proposées, et un AppID inconnu de Steam est signalé.
- Les réglages permettent de choisir les dossiers surveillés, et distinguent clairement le suivi des téléchargements Steam de la détection de vos fichiers.

### en
- The app sees your downloaded .lua files, online patches and DRM bypasses arrive, whatever their source, as long as it runs (window open or minimised next to the clock). Window in front: a dialog offers to add them. Otherwise a notification lists the detected games and updates with each new file; clicking it brings the app back. Nothing is added without your consent.
- The library's "Detected files" button also lists older files in your folders, so you can add them later in one click and pick which ones. A file already available in the app is not offered again.
- Each row shows the game's name and the file's type (.lua file, online patch, DRM bypass). Archives are opened with your default password before being offered, and an AppID Steam does not know is flagged.
- Settings let you choose the watched folders, and clearly tell the Steam download tracking apart from the detection of your files.

## 1.1.4 - 2026-10-19

### fr
- L'application suit maintenant les téléchargements en arrière-plan. Quand un téléchargement échoue avec « Pas de connexion Internet », elle le voit, vous prévient par une notification et change de serveur toute seule, sans rien redémarrer. Quand aucun serveur ne répond, elle laisse des pauses de plus en plus longues entre deux séries d'essais.
- Une icône près de l'horloge montre l'état des serveurs de téléchargement (en ligne, dégradé, hors ligne) et le prochain essai. Fermer la fenêtre garde l'application active, « Quitter » dans le menu de l'icône l'arrête. Elle démarre avec Windows, et un réglage permet de la couper.

### en
- The app now watches downloads in the background. When a download fails with "No internet connection", it sees it, tells you with a notification and switches servers by itself, with nothing to restart. When no server answers, it leaves longer and longer pauses between rounds of attempts.
- An icon next to the clock shows the state of the download servers (online, degraded, offline) and the next attempt. Closing the window keeps the app running, "Quit" in the icon's menu stops it. It starts with Windows, and a setting turns it off.

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
