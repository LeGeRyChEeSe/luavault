// LuaVault : la pastille « LV » de la barre de titre du client Steam, ses réglages, le bloc de la
// barre « Jouer », les pastilles de la liste des jeux et les gestes du clic droit.
//
// Injecté par l'application (`steam_client.rs`) dans la fenêtre principale du client, et aussi dans
// sa vue web (boutique, communauté) où il ne dessine que le panneau des réglages, sur le mot de
// l'application (`lvOpenSettings`) : cette vue est posée PAR-DESSUS la fenêtre principale, un panneau
// dessiné par la fenêtre y serait caché. Tout part vers l'application par `lvStore`, avec un mot
// d'un ensemble fermé que l'application relit : `open`, `settings`, `panel`, `set:<clé>:<valeur>`,
// `hide`, `lib:<appid>`, `act:<geste>:<appid>`. Rien n'est téléchargé d'ici : « Trouver un
// correctif sur le site » ouvre l'accueil du site, la personne y télécharge elle-même.
//
// Ancrages lus sur le vrai client (2026-11-11/12) : `.avatarHolder` (classe non hachée) dans un
// `.tool-tip-source`, enfant de la rangée des boutons du haut ; le bouton Jouer est le composant React
// aux propriétés exactement `overview` et `strGameActionDetailsOverride` (`bShowingPlayBar` toléré) ;
// une ligne de la liste des jeux est le composant aux propriétés exactement `appid, children,
// className, includeMultiSelect, strCollectionId` ; le menu du clic droit est `.contextMenuItem`.
(function () {
  "use strict";
  if (typeof lvStore !== "function") return;
  var root = document.documentElement;
  if (!root) return;
  // Its own run counter: in a web page this script runs beside the store button's script, which
  // counts its passes in `lvRun`. Sharing it would silence one with the other.
  root.lvWinRun = (root.lvWinRun || 0) + 1;
  var myRun = root.lvWinRun;
  var WEB_HOST = /^([a-z0-9-]+\.)*(steampowered|steamcommunity)\.com$/;
  var IN_STORE = typeof location === "object" && location !== null && WEB_HOST.test(String(location.hostname || ""));
  var FR = String(root.lang || navigator.language || "fr").toLowerCase().indexOf("fr") === 0;
  function words(fr, en) {
    return FR ? fr : en;
  }
  function drop(node) {
    if (node && node.parentNode) node.parentNode.removeChild(node);
  }
  // In the main window everything of ours is this script's; in a web page the store button's nodes
  // must stay, only this script's own (`data-lv-win`) go.
  var stale = document.querySelectorAll(IN_STORE ? "[data-lv-win]" : "[data-lv-node]");
  for (var i = 0; i < stale.length; i++) drop(stale[i]);

  function el(tag, css, text) {
    var n = document.createElement(tag);
    if (css) n.style.cssText = css;
    if (text !== undefined) n.textContent = text;
    return n;
  }
  function mine(n) {
    n.setAttribute("data-lv-node", "1");
    n.setAttribute("data-lv-win", "1");
    return n;
  }
  function stop(e) {
    if (e && typeof e.stopPropagation === "function") e.stopPropagation();
  }
  // The AppID to the clipboard, during the click itself (the only moment a page may write there).
  function copyId(appId) {
    try {
      var area = el("textarea", "position:fixed;left:-9999px;top:0;opacity:0");
      area.value = appId;
      area.setAttribute("readonly", "");
      (document.body || root).appendChild(area);
      area.select();
      document.execCommand("copy");
      drop(area);
    } catch (e) {
      /* the panel's line still names the AppID */
    }
  }

  // The client shows its web view: its address bar, a line of text under the tabs, names one of
  // Valve's sites.
  var WEB_ADDRESS = /^https:\/\/([a-z0-9-]+\.)*(steampowered|steamcommunity)\.com\//;
  function storeShown() {
    var leaves = document.querySelectorAll("div, span");
    for (var i = 0; i < leaves.length; i++) {
      var n = leaves[i];
      if (n.children && n.children.length > 0) continue;
      if (!WEB_ADDRESS.test(String(n.textContent || ""))) continue;
      var r = n.getBoundingClientRect();
      if (r.width > 0 && r.top < 120) return true;
    }
    return false;
  }

  // The row of the top right buttons, found from the account button.
  function row() {
    var n = document.querySelector(".avatarHolder");
    while (n && !(n.classList && n.classList.contains("tool-tip-source"))) n = n.parentNode;
    return n && n.parentNode ? n.parentNode : null;
  }

  function button(text, primary, onClick) {
    var b = el(
      "div",
      "padding:5px 12px;border-radius:2px;font-size:12px;cursor:pointer;color:#fff;" +
        (primary ? "background:linear-gradient(90deg,#06bfff,#2d73ff)" : "background:#3d4450")
    );
    b.setAttribute("role", "button");
    var caption = el("span", "", text);
    caption.setAttribute("data-lv-caption", "1");
    b.appendChild(caption);
    b.addEventListener("click", function (e) {
      stop(e);
      onClick();
    });
    return b;
  }

  // ─────────────────────────────────────────────── les réglages dans Steam
  // Un clic sur la pastille ouvre le panneau : il demande `settings`, l'application répond
  // `lvSettings(vue)`, chaque changement part en `set:<clé>:<valeur>` et la vue revient à jour.
  // « Masquer » y a sa section, à double confirmation.
  var card = null;
  var view = null;
  function closeCard() {
    drop(card);
    card = null;
  }

  function validView(v) {
    if (v === null || typeof v !== "object") return false;
    var flags = ["autoUpdate", "backgroundNotify", "backgroundGuard", "watchDownloads", "libraryBlock", "collection", "experimental", "portable"];
    for (var i = 0; i < flags.length; i++) if (typeof v[flags[i]] !== "boolean") return false;
    return (
      typeof v.version === "string" &&
      (v.locale === "fr" || v.locale === "en") &&
      ["open_steam_tool", "classic", "none"].indexOf(v.steamtools) >= 0 &&
      (v.error === undefined || typeof v.error === "string")
    );
  }

  function section(title) {
    var box = el("div", "margin-top:14px");
    box.appendChild(el("div", "color:#dcdedf;font-size:12px;text-transform:uppercase;letter-spacing:.04em;padding-bottom:6px;border-bottom:1px solid rgba(255,255,255,.08)", title));
    return box;
  }

  function settingRow(label, control) {
    var r = el("div", "display:flex;align-items:center;justify-content:space-between;gap:12px;padding:8px 0");
    r.appendChild(el("div", "color:#acb2b8;font-size:13px;line-height:17px", label));
    r.appendChild(control);
    return r;
  }

  function toggle(key, on, disabled) {
    var t = el(
      "div",
      "flex:none;width:38px;height:20px;border-radius:10px;position:relative;cursor:" + (disabled ? "default" : "pointer") +
        ";opacity:" + (disabled ? ".4" : "1") + ";background:" + (on ? "#1a9fff" : "#4b5361")
    );
    t.setAttribute("role", "switch");
    t.setAttribute("aria-checked", on ? "true" : "false");
    t.setAttribute("data-lv-set", key);
    t.appendChild(el("div", "position:absolute;top:2px;width:16px;height:16px;border-radius:50%;background:#fff;left:" + (on ? "20px" : "2px")));
    t.addEventListener("click", function (e) {
      stop(e);
      if (disabled || t.getAttribute("data-lv-pending")) return;
      t.setAttribute("data-lv-pending", "1");
      t.style.opacity = ".5";
      lvStore("set:" + key + ":" + (on ? "0" : "1"));
    });
    return t;
  }

  function chips(key, values, labels, current) {
    var box = el("div", "display:flex;gap:4px;flex-wrap:wrap;justify-content:flex-end");
    for (var i = 0; i < values.length; i++) {
      (function (value, label) {
        var on = value === current;
        var c = el("div", "padding:3px 9px;border-radius:2px;font-size:12px;cursor:pointer;" + (on ? "background:#1a9fff;color:#fff" : "background:#3d4450;color:#dcdedf"), label);
        c.setAttribute("role", "button");
        c.setAttribute("data-lv-set", key);
        c.setAttribute("data-lv-value", value);
        c.addEventListener("click", function (e) {
          stop(e);
          if (on) return;
          c.style.opacity = ".5";
          lvStore("set:" + key + ":" + value);
        });
        box.appendChild(c);
      })(values[i], labels[i]);
    }
    return box;
  }

  function hideSection() {
    var box = section(words("Masquer LuaVault dans Steam", "Hide LuaVault in Steam"));
    box.appendChild(
      el(
        "div",
        "color:#8b929a;font-size:12px;line-height:16px;margin-top:8px",
        words(
          "Range les jeux de LuaVault hors de Steam et retire les correctifs (fichiers d'origine remis). Les dossiers des jeux restent. Tout se remet en un clic depuis l'onglet « À l'essai » de LuaVault.",
          "Puts LuaVault's games out of Steam and takes the patches off (original files restored). Game folders stay. Put it all back in one click from LuaVault's « Trials » tab."
        )
      )
    );
    var armed = false;
    var hide = button(words("Masquer", "Hide"), false, function () {
      if (!armed) {
        armed = true;
        hide.querySelector("[data-lv-caption]").textContent = words("Confirmer le masquage", "Confirm hiding");
        return;
      }
      closeCard();
      lvStore("hide");
    });
    hide.setAttribute("data-lv-hide", "1");
    hide.style.display = "inline-block";
    hide.style.marginTop = "10px";
    box.appendChild(hide);
    return box;
  }

  // The discreet mode: everything LuaVault draws in Steam goes, LuaVault keeps working. Two clicks,
  // since the way back is in LuaVault's window.
  function discreetSection() {
    var box = section(words("Interface", "Interface"));
    box.appendChild(
      el(
        "div",
        "color:#8b929a;font-size:12px;line-height:16px;margin-top:8px",
        words(
          "Retire de Steam tout ce que LuaVault y affiche (cette pastille, les boutons de la boutique, les blocs et pastilles de la bibliothèque, les cartes). LuaVault continue tout en arrière-plan. Pour les retrouver : onglet « À l'essai » de LuaVault.",
          "Takes out of Steam everything LuaVault shows there (this pill, the store buttons, the library's blocks and dots, the cards). LuaVault keeps doing everything in the background. To get them back: LuaVault's « Trials » tab."
        )
      )
    );
    var armed = false;
    var go = button(words("Rendre discret", "Go discreet"), false, function () {
      if (!armed) {
        armed = true;
        go.querySelector("[data-lv-caption]").textContent = words("Confirmer : tout retirer de Steam", "Confirm: take it all out of Steam");
        return;
      }
      closeCard();
      lvStore("set:discreet:1");
    });
    go.setAttribute("data-lv-discreet", "1");
    go.style.display = "inline-block";
    go.style.marginTop = "10px";
    box.appendChild(go);
    return box;
  }

  function renderSettings(body) {
    while (body.firstChild) body.removeChild(body.firstChild);
    if (!view) {
      body.appendChild(el("div", "color:#8b929a;font-size:12px;margin-top:10px", words("Chargement...", "Loading...")));
      body.appendChild(hideSection());
      return;
    }
    var tool = {
      open_steam_tool: "OpenSteamTool",
      classic: words("SteamTools classique", "classic SteamTools"),
      none: words("SteamTools absent", "no SteamTools"),
    }[view.steamtools];
    body.appendChild(
      el("div", "color:#67707b;font-size:12px;margin-top:2px", "Version " + view.version.slice(0, 30) + (view.experimental ? words(", édition expérimentale", ", experimental edition") : "") + ", " + tool)
    );
    if (view.error) {
      var err = el("div", "color:#c94a4a;font-size:12px;line-height:16px;margin-top:8px", view.error.slice(0, 200));
      err.setAttribute("data-lv-settings-error", "1");
      body.appendChild(err);
    }
    var general = section(words("Général", "General"));
    general.appendChild(settingRow(words("Langue de LuaVault", "LuaVault language"), chips("locale", ["fr", "en"], ["Français", "English"], view.locale)));
    general.appendChild(settingRow(words("Mises à jour automatiques", "Automatic updates"), toggle("auto_update", view.autoUpdate, false)));
    general.appendChild(settingRow(words("Notifications quand l'application est fermée", "Notifications while the application is closed"), toggle("background_notify", view.backgroundNotify, view.portable)));
    general.appendChild(settingRow(words("Service de téléchargement (icône près de l'horloge)", "Download guard (icon next to the clock)"), toggle("background_guard", view.backgroundGuard, false)));
    general.appendChild(settingRow(words("Détection des fichiers dans les Téléchargements", "Detection of files in Downloads"), toggle("watch_downloads", view.watchDownloads, false)));
    body.appendChild(general);
    var lib = section(words("Bibliothèque", "Library"));
    lib.appendChild(settingRow(words("Bloc LuaVault à côté du bouton Jouer", "LuaVault block beside the Play button"), toggle("library_block", view.libraryBlock, false)));
    lib.appendChild(settingRow(words("Collection LuaVault dans la bibliothèque", "LuaVault collection in the library"), toggle("collection", view.collection, false)));
    body.appendChild(lib);
    body.appendChild(discreetSection());
    body.appendChild(hideSection());
  }

  function openCard(anchor) {
    // No anchor: opened in a web page, under the client's own title bar.
    var r = anchor ? anchor.getBoundingClientRect() : { bottom: 8, right: innerWidth - 8 };
    card = mine(el("div"));
    card.setAttribute("data-lv-settings", "1");
    card.style.cssText =
      "position:fixed;z-index:99999;width:420px;max-height:80vh;overflow-y:auto;padding:16px;border-radius:4px;box-sizing:border-box;" +
      "top:" + Math.round(r.bottom + 8) + "px;right:" + Math.max(8, Math.round(innerWidth - r.right)) + "px;" +
      "background:linear-gradient(180deg,#22303f,#171d25);border:1px solid #2a3f5a;box-shadow:0 8px 24px rgba(0,0,0,.6);" +
      "font-family:inherit;-webkit-app-region:no-drag";
    card.addEventListener("click", stop);
    var head = el("div", "display:flex;align-items:center;justify-content:space-between");
    head.appendChild(el("div", "color:#66c0f4;font-size:13px;letter-spacing:.08em;text-transform:uppercase", "LuaVault"));
    var close = el("div", "color:#8b929a;font-size:18px;line-height:14px;cursor:pointer;padding:2px 4px", "×");
    close.setAttribute("role", "button");
    close.setAttribute("data-lv-settings-close", "1");
    close.addEventListener("click", closeCard);
    head.appendChild(close);
    card.appendChild(head);
    var body = el("div");
    body.setAttribute("data-lv-settings-body", "1");
    card.appendChild(body);
    renderSettings(body);
    var foot = el("div", "display:flex;gap:6px;margin-top:16px;padding-top:12px;border-top:1px solid rgba(255,255,255,.08)");
    var openApp = button(words("Ouvrir LuaVault", "Open LuaVault"), true, function () {
      closeCard();
      lvStore("open");
    });
    openApp.setAttribute("data-lv-open-app", "1");
    foot.appendChild(openApp);
    card.appendChild(foot);
    (document.body || root).appendChild(card);
    lvStore("settings");
  }

  if (typeof window === "object" && window !== null) {
    window.lvSettings = function (v) {
      if (root.lvWinRun !== myRun || !validView(v)) return;
      view = v;
      var wasFr = FR;
      FR = v.locale === "fr";
      var body = card && card.querySelector("[data-lv-settings-body]");
      if (body) renderSettings(body);
      // The language changed: this script's own words are written again. The play bar's blocks
      // were rebuilt before this view arrived, still in the old language.
      if (wasFr !== FR) {
        var openCaption = card && card.querySelector("[data-lv-open-app] [data-lv-caption]");
        if (openCaption) openCaption.textContent = words("Ouvrir LuaVault", "Open LuaVault");
        if (typeof window.lvLibReset === "function") window.lvLibReset();
      }
    };
    window.lvOpenSettings = function () {
      if (root.lvWinRun !== myRun || card) return;
      openCard(null);
    };
  }
  document.addEventListener("click", function () {
    if (card) closeCard();
  });
  // In a web page this script draws the panel only.
  if (IN_STORE) return;

  function pill() {
    var box = mine(el("div", "display:flex;align-items:center;margin-right:8px;-webkit-app-region:no-drag"));
    box.setAttribute("data-lv-pill", "1");
    box.className = "no-drag";
    var b = el(
      "div",
      "height:24px;line-height:24px;padding:0 8px;border-radius:2px;box-sizing:border-box;cursor:pointer;" +
        "background:#3d4450;color:#dcdedf;font-size:11px;font-weight:bold;letter-spacing:.06em",
      "LV"
    );
    b.setAttribute("role", "button");
    b.setAttribute("data-lv-pill-open", "1");
    b.setAttribute("title", "LuaVault");
    b.addEventListener("click", function (e) {
      stop(e);
      if (card) return closeCard();
      // A page of Valve's is shown over the window: the panel opens in that page, which is in front.
      if (storeShown()) return lvStore("panel");
      openCard(b);
    });
    box.appendChild(b);
    return box;
  }

  // ─────────────────────────────────────────────── la barre Jouer de la bibliothèque
  // Pour un jeu affiché, la page demande `lib:<appid>` ; l'application répond `lvLibCard({appId,
  // card})`, `card` nul quand le jeu n'est pas à elle (rien n'est alors dessiné).
  var APPID_RULE = /^[1-9][0-9]{0,9}$/;
  function playProps(p) {
    var keys = Object.keys(p).filter(function (k) {
      return k !== "bShowingPlayBar";
    });
    return keys.length === 2 && "strGameActionDetailsOverride" in p && !!p.overview && !!p.overview.appid;
  }
  var TONES = { ok: "#a4d007", warn: "#e5b143", error: "#c94a4a", muted: "#67707b", busy: "#66c0f4" };
  var ACTIONS = { apply: 1, find: 1, repair: 1, unpatch: 1, remove_lua: 1, uninstall: 1, open: 1 };
  var cards = {};
  var asked = {};
  var menu = null;
  var current = null;

  function fiberOf(n) {
    for (var k in n) if (k.indexOf("__reactFiber") === 0) return n[k];
    return null;
  }

  // The play buttons' containers and the game they play. A game page has two: the bar under the
  // artwork, and the sticky header that slides in when the page is scrolled. Each gets the block.
  function playBars() {
    var panel = document.querySelector(".TallRightPanel") || document;
    var divs = panel.querySelectorAll("div");
    var found = [];
    for (var i = 0; i < divs.length; i++) {
      var f = fiberOf(divs[i]);
      for (var up = 0; up < 3 && f; up++, f = f.return) {
        var p = f.memoizedProps;
        if (p && typeof p === "object" && playProps(p)) {
          var host = f;
          while (host && !(host.stateNode && host.stateNode.nodeType === 1)) host = host.child;
          var node = host && host.stateNode;
          if (node && node.parentNode && found.every(function (b) { return b.container !== node; })) {
            found.push({ container: node, appId: String(p.overview.appid) });
          }
        }
      }
    }
    return found;
  }

  function validCard(c) {
    if (c === null || typeof c !== "object" || !APPID_RULE.test(String(c.appId))) return false;
    if (typeof c.line !== "string" || !Object.prototype.hasOwnProperty.call(TONES, c.tone)) return false;
    if (!c.actions || typeof c.actions.length !== "number") return false;
    for (var i = 0; i < c.actions.length; i++) {
      var a = c.actions[i];
      if (!a || !Object.prototype.hasOwnProperty.call(ACTIONS, a.id) || typeof a.label !== "string") return false;
      if (a.confirm !== undefined && typeof a.confirm !== "string") return false;
    }
    return true;
  }

  // One gesture: « find » also copies the AppID (the site has no page per game, its search takes it).
  function send(id, appId) {
    if (id === "find") copyId(appId);
    lvStore("act:" + id + ":" + appId);
  }

  function closeMenu() {
    drop(menu);
    menu = null;
  }

  function menuItem(c, action) {
    var item = el("div", "padding:8px 16px;font-size:13px;color:#dcdedf;cursor:pointer;white-space:nowrap", action.label.slice(0, 80));
    item.setAttribute("role", "menuitem");
    item.setAttribute("data-lv-lib-action", action.id);
    item.addEventListener("mouseenter", function () {
      item.style.background = "#dcdedf";
      item.style.color = "#3d4450";
    });
    item.addEventListener("mouseleave", function () {
      item.style.background = "";
      item.style.color = "#dcdedf";
    });
    var armed = false;
    item.addEventListener("click", function (e) {
      stop(e);
      if (typeof action.confirm === "string" && !armed) {
        armed = true;
        item.textContent = action.confirm.slice(0, 80);
        return;
      }
      closeMenu();
      send(action.id, String(c.appId));
    });
    return item;
  }

  function openMenu(anchor, c) {
    var r = anchor.getBoundingClientRect();
    menu = mine(el("div"));
    menu.setAttribute("data-lv-lib-menu", "1");
    menu.style.cssText =
      "position:fixed;z-index:99999;min-width:260px;padding:4px 0;border-radius:3px;box-sizing:border-box;" +
      "top:" + Math.round(r.bottom + 4) + "px;left:" + Math.round(r.left) + "px;" +
      "background:#3d4450;box-shadow:0 8px 24px rgba(0,0,0,.6);font-family:inherit";
    for (var i = 0; i < c.actions.length; i++) menu.appendChild(menuItem(c, c.actions[i]));
    (document.body || root).appendChild(menu);
  }

  function libBlock(c) {
    var block = mine(el("div", "display:flex;align-items:center;gap:12px;margin:0 8px 0 16px;flex:none"));
    block.setAttribute("data-lv-lib", String(c.appId));
    var text = el("div", "display:flex;flex-direction:column;justify-content:center;min-width:0");
    text.appendChild(el("div", "font-size:12px;text-transform:uppercase;letter-spacing:.04em;color:#dcdedf;white-space:nowrap", "LuaVault"));
    var line = el("div", "display:flex;align-items:center;gap:6px;font-size:12px;color:#8b929a;white-space:nowrap");
    line.setAttribute("data-lv-lib-line", "1");
    line.appendChild(el("span", "width:8px;height:8px;border-radius:50%;flex:none;background:" + TONES[c.tone]));
    line.appendChild(el("span", "", c.line.slice(0, 140)));
    text.appendChild(line);
    block.appendChild(text);
    if (c.actions.length > 0) {
      var manage = el("div", "height:32px;line-height:32px;padding:0 12px;border-radius:2px;cursor:pointer;flex:none;background:#3d4450;color:#dcdedf;font-size:13px", words("Gérer", "Manage") + " ▾");
      manage.setAttribute("role", "button");
      manage.setAttribute("data-lv-lib-open", "1");
      manage.addEventListener("click", function (e) {
        stop(e);
        if (menu) return closeMenu();
        openMenu(manage, c);
      });
      block.appendChild(manage);
    }
    return block;
  }

  function placeLib() {
    var bars = playBars();
    var blocks = document.querySelectorAll("[data-lv-lib]");
    // A block no longer right after a play button of the shown game goes.
    for (var i = 0; i < blocks.length; i++) {
      var prev = blocks[i].previousSibling;
      var keep = false;
      for (var k = 0; k < bars.length; k++) if (bars[k].container === prev && blocks[i].lvCard === cards[bars[k].appId]) keep = true;
      if (!keep) drop(blocks[i]);
    }
    if (bars.length === 0) {
      current = null;
      return;
    }
    var appId = bars[0].appId;
    // A game shown again is asked again: it may have been added, patched or hidden meanwhile.
    if (appId !== current) {
      current = appId;
      delete cards[appId];
      asked[appId] = false;
    }
    if (!Object.prototype.hasOwnProperty.call(cards, appId)) {
      if (!asked[appId]) {
        asked[appId] = true;
        lvStore("lib:" + appId);
      }
      return;
    }
    var c = cards[appId];
    if (!c) return;
    for (var j = 0; j < bars.length; j++) {
      var next = bars[j].container.nextSibling;
      if (next && next.lvCard === c) continue;
      var block = libBlock(c);
      block.lvCard = c;
      bars[j].container.parentNode.insertBefore(block, next);
    }
  }

  if (typeof window === "object" && window !== null) {
    window.lvLibCard = function (payload) {
      if (root.lvWinRun !== myRun || !payload || !APPID_RULE.test(String(payload.appId))) return;
      var id = String(payload.appId);
      asked[id] = false;
      cards[id] = payload.card !== null && validCard(payload.card) && String(payload.card.appId) === id ? payload.card : null;
      closeMenu();
      placeLib();
    };
    // The language or the block's switch changed: every game is asked again.
    window.lvLibReset = function () {
      if (root.lvWinRun !== myRun) return;
      cards = {};
      asked = {};
      current = null;
      closeMenu();
      var blocks = document.querySelectorAll("[data-lv-lib]");
      for (var b = 0; b < blocks.length; b++) drop(blocks[b]);
      placeLib();
    };
  }
  document.addEventListener("click", function () {
    if (menu) closeMenu();
  });

  // ─────────────────────────────── les pastilles de la liste des jeux
  // Un jeu de LuaVault reçoit une pastille de la couleur de son état, l'état en info-bulle. La liste
  // ne garde que ses lignes visibles : les pastilles sont reposées à chaque passage.
  var dots = {};
  var ROW_KEYS = "appid,children,className,includeMultiSelect,strCollectionId";
  function gameRows() {
    var found = [];
    var divs = document.querySelectorAll("div");
    for (var i = 0; i < divs.length; i++) {
      var f = fiberOf(divs[i]);
      // The row's outer div sits two levels under the component; its inner divs are deeper and stay out.
      for (var up = 0; up < 3 && f; up++, f = f.return) {
        var p = f.memoizedProps;
        if (!p || typeof p !== "object" || Object.keys(p).sort().join(",") !== ROW_KEYS) continue;
        if (!APPID_RULE.test(String(p.appid))) continue;
        found.push({ el: divs[i], appId: String(p.appid) });
        break;
      }
    }
    return found;
  }
  function placeDots() {
    var old = document.querySelectorAll("[data-lv-dot]");
    for (var o = 0; o < old.length; o++) {
      if (!dots[old[o].getAttribute("data-lv-dot")]) drop(old[o]);
    }
    if (Object.keys(dots).length === 0) return;
    var rows = gameRows();
    for (var i = 0; i < rows.length; i++) {
      var d = dots[rows[i].appId];
      if (!d) continue;
      var host = rows[i].el;
      var have = null;
      for (var c = 0; c < host.children.length; c++) {
        if (host.children[c].getAttribute && host.children[c].getAttribute("data-lv-dot") !== null) have = host.children[c];
      }
      if (have && have.getAttribute("data-lv-tone") === d.tone) continue;
      drop(have);
      var dot = mine(el("span"));
      dot.setAttribute("data-lv-dot", rows[i].appId);
      dot.setAttribute("data-lv-tone", d.tone);
      dot.setAttribute("title", "LuaVault : " + d.line);
      dot.style.cssText =
        "position:absolute;right:8px;top:50%;width:7px;height:7px;margin-top:-4px;border-radius:50%;background:" + TONES[d.tone] +
        ";box-shadow:0 0 0 2px rgba(0,0,0,.35);pointer-events:auto";
      if (host.style && !host.style.position) host.style.position = "relative";
      host.appendChild(dot);
    }
  }
  if (typeof window === "object" && window !== null) {
    window.lvLibDots = function (map) {
      if (root.lvWinRun !== myRun || !map || typeof map !== "object") return;
      var next = {};
      for (var id in map) {
        if (!Object.prototype.hasOwnProperty.call(map, id) || !APPID_RULE.test(id)) continue;
        var d = map[id];
        if (d && typeof d.line === "string" && Object.prototype.hasOwnProperty.call(TONES, d.tone)) next[id] = { tone: d.tone, line: d.line.slice(0, 120) };
      }
      dots = next;
      placeDots();
    };
  }

  // ─────────────────────────────── le menu du clic droit
  // Après un clic droit sur la ligne d'un jeu de LuaVault, ses gestes sont ajoutés au menu de Steam,
  // à ses classes. Ceux qui retirent quelque chose demandent une confirmation : un menu se ferme au
  // clic, ils restent dans « Gérer » de la barre Jouer.
  var ctxApp = null;
  var ctxTimer = null;
  function rowAppId(node) {
    for (var n = node, depth = 0; n && depth < 8; n = n.parentNode, depth++) {
      var f = fiberOf(n);
      for (var up = 0; up < 3 && f; up++, f = f.return) {
        var p = f.memoizedProps;
        if (p && typeof p === "object" && Object.keys(p).sort().join(",") === ROW_KEYS && APPID_RULE.test(String(p.appid))) return String(p.appid);
      }
    }
    return null;
  }
  // Steam's menu listens for Escape on itself: it holds the focus while open (read in the client,
  // 2026-11-13), and an Escape sent to the body leaves it open.
  function closeSteamMenu() {
    var target = document.activeElement || document.body || root;
    if (typeof KeyboardEvent === "function" && typeof target.dispatchEvent === "function") {
      target.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", code: "Escape", keyCode: 27, bubbles: true }));
    }
  }
  function injectCtx() {
    if (ctxApp === null) return true;
    var c = cards[ctxApp];
    if (c === undefined) return false;
    if (c === null) return true;
    var items = document.querySelectorAll(".contextMenuItem");
    var last = null;
    for (var i = 0; i < items.length; i++) {
      if (items[i].getAttribute("data-lv-ctx") === null) last = items[i];
    }
    if (!last) return false;
    var box = last.parentNode;
    if (box.querySelector("[data-lv-ctx]")) return true;
    var plain = null;
    for (var j = 0; j < items.length; j++) {
      if (items[j].parentNode === box && items[j].children.length === 0) plain = items[j];
    }
    var itemClass = (plain || last).className;
    var steamSep = box.querySelector("[role=separator]");
    var sep = mine(el("div"));
    sep.setAttribute("role", "separator");
    sep.setAttribute("data-lv-ctx", "sep");
    if (steamSep) sep.className = steamSep.className;
    box.appendChild(sep);
    var appId = ctxApp;
    var added = 0;
    for (var a = 0; a < c.actions.length; a++) {
      var act = c.actions[a];
      if (act.confirm) continue;
      var item = mine(el("div", "", "LuaVault : " + act.label));
      item.setAttribute("role", "menuitem");
      item.setAttribute("data-lv-ctx", act.id);
      item.className = itemClass;
      (function (id) {
        item.addEventListener("click", function (e) {
          stop(e);
          send(id, appId);
          closeSteamMenu();
        });
      })(act.id);
      box.appendChild(item);
      added++;
    }
    if (added === 0) drop(sep);
    return true;
  }
  document.addEventListener(
    "contextmenu",
    function (e) {
      if (root.lvWinRun !== myRun || !e || !e.target) return;
      var id = rowAppId(e.target);
      ctxApp = id;
      if (id === null) return;
      if (!Object.prototype.hasOwnProperty.call(cards, id) && !asked[id]) {
        asked[id] = true;
        lvStore("lib:" + id);
      }
      var tries = 0;
      if (ctxTimer) clearInterval(ctxTimer);
      ctxTimer = setInterval(function () {
        tries++;
        if (injectCtx() || tries > 30) {
          clearInterval(ctxTimer);
          ctxTimer = null;
        }
      }, 100);
    },
    true
  );

  // Steam redraws its title bar and its lists: everything is put back within a second.
  function place() {
    if (root.lvWinRun !== myRun) return false;
    placeLib();
    placeDots();
    var r = row();
    if (!r) return true;
    var have = document.querySelector("[data-lv-pill]");
    if (have && have.parentNode === r && r.firstChild === have) return true;
    drop(have);
    r.insertBefore(pill(), r.firstChild);
    return true;
  }

  place();
  var timer = setInterval(function () {
    if (!place()) clearInterval(timer);
  }, 1000);
})();
