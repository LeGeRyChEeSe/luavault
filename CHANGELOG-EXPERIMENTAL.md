# Experimental changelog

The source of the **experimental channel**'s notes: `scripts/publish-release.ps1 -Channel
experimental` reads it for the `notes` and `history` of `manifest-experimental.json`, exactly as
`CHANGELOG.md` for the stable. Same rules, same guards (a missing version or an entry without a
bullet fails the publication):

- a heading is written `## <x.y.z-exp.N> - <YYYY-MM-DD>` (ASCII dash, pre-release suffix);
- only the `- ` lines are kept, as plain text, one line per bullet;
- `### fr` and `### en` split the bullets per language.

An experimental version describes what it adds over the stable. A stable that takes a feature
over describes it again in `CHANGELOG.md`.

## Unreleased

## 1.3.3-exp.1 - 2026-11-14

### fr
- LuaVault dans Steam, activé dès l'installation : Steam redémarre tout seul une fois (après sa mise à jour si besoin, jamais pendant un jeu) pour l'afficher.
- Un bouton « Trouver sur le site » sur les pages de jeu de la boutique Steam : il ouvre le site et copie l'AppID, puis LuaVault reprend les fichiers téléchargés et les place.
- À côté du bouton Jouer, l'état du jeu et de ses correctifs, avec leurs gestes ; une pastille dans la liste des jeux, les gestes au clic droit, et une collection « LuaVault » dans la bibliothèque.
- Une pastille « LV » en haut de Steam : les réglages de LuaVault, le mode discret et « Masquer LuaVault dans Steam ».
- Une carte dans Steam à chaque .lua, correctif ou contournement DRM détecté dans vos Téléchargements, et les correctifs remis tout seuls après une mise à jour d'un jeu.
- Un message d'accueil présente ces nouveautés, à quoi elles servent et où les trouver. Il ne s'affiche pas sur la version stable.
- SteamTools s'installe tout seul quand il manque ou que l'ancien est encore là : une seule autorisation Windows à accepter, aucun bouton à cliquer dans l'installeur.
- Une mise à jour automatique est annoncée : une notification avant l'installation, et un message dans la fenêtre si l'application est ouverte, au lieu de la fermer sans rien dire.

### en
- LuaVault in Steam, on as soon as it is installed: Steam restarts by itself once (after its update if needed, never during a game) to show it.
- A « Find on the site » button on the game pages of the Steam store: it opens the site and copies the AppID, then LuaVault picks up the downloaded files and puts them in place.
- Next to the Play button, the game's state and its patches, with their gestures; a dot in the game list, the gestures on right click, and a « LuaVault » collection in the library.
- An « LV » pill at the top of Steam: LuaVault's settings, the discreet mode and « Hide LuaVault in Steam ».
- A card in Steam for every .lua, patch or DRM bypass detected in your Downloads, and patches put back by themselves after a game update.
- A welcome message presents these features, what they are for and where to find them. It does not show on the stable version.
- SteamTools installs itself when it is missing or the old one is still there: a single Windows permission to accept, no button to click in the installer.
- An automatic update is announced: a notification before the installation, and a message in the window if the app is open, instead of closing it without a word.
