// LuaVault : le bouton « Trouver sur le site » de la boutique, dans le client Steam.
//
// Injecté par l'application (`steam_client.rs`) dans la vue web du client, jamais livré à un
// navigateur. LuaVault ne télécharge aucun fichier : un clic copie l'AppID, puis l'application
// ouvre l'accueil du site dans le navigateur, où la personne télécharge le .lua et le correctif.
// La détection des Téléchargements de LuaVault les reprend. Le panneau en bas à droite suit chaque
// jeu demandé ; ses textes viennent de l'application et sont posés comme du texte.
//
// Avant ce script, l'application déclare `lvStoreKnown` ({appid: "installed" | "downloading" |
// "absent"} : les jeux dont le .lua est déjà là où Steam le lit) et `lvStoreJobs` (les lignes du
// panneau). Elle n'envoie que trois mots : `find:<appid>`, `dismiss:<appid>`, `open`.
//
// Les ancres sont celles de la vraie page, relevées dans le client Steam :
//   1. la ligne d'achat du jeu lui-même (`.game_purchase_action`) du PREMIER bloc
//      `.game_area_purchase_game_wrapper` qui n'est pas un bundle (`dynamic_bundle_description`),
//      un bloc de licence gratuite (formulaire vers `/freelicense/`) étant passé ;
//   2. `#queueActionsCtn` (liste de souhaits / suivre / ignorer), présent même sans bloc d'achat ;
//   3. `#game_area_purchase`, en dernier secours.
// Les lignes de recherche sont `a.search_result_row[data-ds-appid]`.
(function () {
  "use strict";
  if (typeof lvStore !== "function") return;
  var root = document.documentElement;
  // Posé avant que la page ait un document : rien à habiller, l'application le repose plus tard.
  if (!root) return;
  root.lvRun = (root.lvRun || 0) + 1;
  var myRun = root.lvRun;

  var APPID_RULE = /^[1-9]\d{0,9}$/;
  var FR = String(root.lang || navigator.language || "fr").toLowerCase().indexOf("fr") === 0;
  function words(fr, en) {
    return FR ? fr : en;
  }
  function drop(node) {
    if (node && node.parentNode && typeof node.parentNode.removeChild === "function") node.parentNode.removeChild(node);
  }
  // Everything this script puts in a page carries `data-lv-node` on its outermost element, so a later
  // pass takes all of it back.
  if (myRun > 1) {
    var stale = document.querySelectorAll("[data-lv-node]");
    for (var s = 0; s < stale.length; s++) drop(stale[s]);
  }

  var KNOWN = typeof lvStoreKnown === "object" && lvStoreKnown !== null ? lvStoreKnown : {};
  function knownState(appId) {
    var state = KNOWN[appId];
    return state === "installed" || state === "downloading" || state === "absent" ? state : null;
  }
  function libraryHref(appId) {
    return "steam://nav/games/details/" + appId;
  }
  function installHref(appId) {
    return "steam://install/" + appId;
  }
  function hasClass(node, name) {
    return (" " + String(node.className || "") + " ").indexOf(" " + name + " ") >= 0;
  }

  // The AppID to the clipboard, during the click itself (the only moment a page may write there).
  function copyId(appId) {
    try {
      var area = document.createElement("textarea");
      area.value = appId;
      area.setAttribute("readonly", "");
      area.style.cssText = "position:fixed;left:-9999px;top:0;opacity:0";
      (document.body || root).appendChild(area);
      area.select();
      var ok = document.execCommand("copy");
      drop(area);
      return !!ok;
    } catch (e) {
      return false;
    }
  }

  // A magnifier: « chercher ». A picture, so the button is told from Steam's at a glance.
  var ICON =
    "data:image/svg+xml," +
    encodeURIComponent(
      '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#ffffff" stroke-width="2.6" stroke-linecap="round" stroke-linejoin="round">' +
        '<circle cx="10.5" cy="10.5" r="6.5"/><path d="M15.5 15.5 21 21"/></svg>'
    );

  // After a click the button says what happened for a few seconds and takes no second click.
  var BUSY_MS = 4000;
  function ask(button, caption, appId) {
    if (button.lvBusy) return;
    button.lvBusy = true;
    var before = caption.textContent;
    caption.textContent = copyId(appId) ? words("AppID copié, site ouvert", "AppID copied, site opened") : words("Site ouvert", "Site opened");
    button.style.opacity = "0.75";
    lvStore("find:" + appId);
    setTimeout(function () {
      button.lvBusy = false;
      caption.textContent = before;
      button.style.opacity = "";
    }, BUSY_MS);
  }

  function findButton(appId, text) {
    var button = document.createElement("a");
    button.className = "btn_blue_steamui btn_medium";
    button.setAttribute("role", "button");
    button.setAttribute("data-lv-find", appId);
    button.setAttribute("data-lv-page", appId);
    button.style.cursor = "pointer";
    button.style.verticalAlign = "top";
    var content = document.createElement("span");
    content.style.cssText = "display:inline-flex;align-items:center;gap:7px";
    var icon = document.createElement("img");
    icon.src = ICON;
    icon.alt = "";
    icon.style.cssText = "width:16px;height:16px";
    content.appendChild(icon);
    var caption = document.createElement("span");
    caption.setAttribute("data-lv-caption", "1");
    caption.textContent = text;
    content.appendChild(caption);
    button.appendChild(content);
    // The handler closes over `appId`, this function's own parameter, never a loop variable.
    button.addEventListener("click", function (e) {
      if (e && typeof e.preventDefault === "function") e.preventDefault();
      ask(button, caption, appId);
    });
    return button;
  }

  function steamLink(href, text, green, appId) {
    var link = document.createElement("a");
    link.className = (green ? "btn_green_steamui" : "btn_blue_steamui") + " btn_medium";
    link.setAttribute("href", href);
    link.setAttribute("data-lv-owned", appId);
    link.style.verticalAlign = "top";
    var caption = document.createElement("span");
    caption.textContent = text;
    link.appendChild(caption);
    return link;
  }

  function ownedButtons(appId, state) {
    var group = document.createElement("span");
    group.setAttribute("data-lv-page", appId);
    group.style.cssText = "display:inline-flex;align-items:center;gap:2px;vertical-align:top";
    if (state === "absent") group.appendChild(steamLink(installHref(appId), words("Installer", "Install"), true, appId));
    group.appendChild(steamLink(libraryHref(appId), words("Dans la bibliothèque", "In your library"), false, appId));
    return group;
  }

  // The frame around LuaVault's buttons: they sit beside Steam's own and must not read as Steam's.
  function frame(box, inRow) {
    box.style.position = "relative";
    box.style.display = "inline-block";
    box.style.background = "#0e1720";
    box.style.border = "2px solid rgba(102,192,244,.75)";
    box.style.borderRadius = "4px";
    box.style.padding = "5px 4px 4px";
    box.style.height = "auto";
    if (inRow) box.style.marginBottom = "-4px";
    var name = document.createElement("span");
    name.textContent = "LuaVault";
    name.setAttribute("data-lv-frame-name", "1");
    name.style.cssText =
      "position:absolute;left:50%;top:-6px;transform:translateX(-50%);z-index:2;background:#0e1720;padding:0 6px;" +
      "border-radius:3px;color:#66c0f4;font-size:9px;line-height:10px;letter-spacing:.14em;text-transform:uppercase;" +
      "white-space:nowrap;pointer-events:none";
    box.insertBefore(name, box.firstChild);
  }

  function freeLicense(wrapper) {
    var form = wrapper.querySelector("form");
    return !!form && String(form.getAttribute("action") || "").indexOf("/freelicense/") >= 0;
  }

  function purchaseRow() {
    var area = document.getElementById("game_area_purchase");
    if (!area) return null;
    var kids = area.children;
    var free = null;
    for (var i = 0; i < kids.length; i++) {
      if (!hasClass(kids[i], "game_area_purchase_game_wrapper")) continue;
      if (hasClass(kids[i], "dynamic_bundle_description")) break;
      if (freeLicense(kids[i])) {
        if (!free) free = kids[i];
        continue;
      }
      return kids[i].querySelector(".game_purchase_action");
    }
    return free ? free.querySelector(".game_purchase_action") : null;
  }

  // A DLC's page names its base game: a DLC has no file of its own, the base game's opens it.
  function baseGameOfDlc() {
    var bubble = document.querySelector(".game_area_dlc_bubble");
    if (!bubble) return null;
    var links = bubble.querySelectorAll("[href]");
    for (var i = 0; i < links.length; i++) {
      var m = String(links[i].getAttribute("href") || "").match(/^https:\/\/store\.steampowered\.com\/app\/(\d{1,10})(?:\/|\?|$)/);
      if (m && APPID_RULE.test(m[1])) return m[1];
    }
    return null;
  }

  var PAGE_ANCHORS = ["queueActionsCtn", "game_area_purchase"];

  function decorateGamePage(pageAppId) {
    // ONE button for the game this page is about; the guard is the button itself.
    if (document.querySelectorAll("[data-lv-page]").length > 0) return;
    var base = baseGameOfDlc();
    var appId = base || pageAppId;
    var owned = knownState(appId);
    var button =
      owned !== null
        ? ownedButtons(appId, owned)
        : findButton(appId, base ? words("Trouver le jeu de base sur le site", "Find the base game on the site") : words("Trouver sur le site", "Find on the site"));
    var row = purchaseRow();
    if (row) {
      var wrap = document.createElement("div");
      wrap.className = "game_purchase_action_bg";
      wrap.setAttribute("data-lv-node", "1");
      var holder = document.createElement("div");
      holder.className = "btn_addtocart";
      holder.appendChild(button);
      wrap.appendChild(holder);
      frame(wrap, true);
      row.appendChild(wrap);
      return;
    }
    for (var i = 0; i < PAGE_ANCHORS.length; i++) {
      var host = document.getElementById(PAGE_ANCHORS[i]);
      if (!host) continue;
      var box = document.createElement("div");
      box.className = "queue_control_button";
      box.setAttribute("data-lv-node", "1");
      box.appendChild(button);
      frame(box, false);
      var before = null;
      var kids = host.children;
      for (var j = 0; j < kids.length; j++) {
        if (hasClass(kids[j], "expand_to_fill")) {
          before = kids[j];
          break;
        }
      }
      if (before === null && i > 0 && kids.length > 0) before = kids[0];
      host.insertBefore(box, before);
      return;
    }
  }

  // The row is itself a link to the game's page: Steam's own address is opened through a link made
  // for the click and dropped right after.
  function go(href) {
    var link = document.createElement("a");
    link.setAttribute("href", href);
    var host = document.body || root;
    host.appendChild(link);
    link.click();
    drop(link);
  }

  function rowButton(appId, text, onClick) {
    var b = document.createElement("span");
    b.setAttribute("role", "button");
    b.setAttribute("data-lv-node", "1");
    b.setAttribute("data-lv-row", appId);
    b.textContent = text;
    b.style.cssText =
      "position:absolute;right:8px;bottom:4px;line-height:20px;font-size:11px;padding:0 9px;border-radius:2px;cursor:pointer;z-index:2;color:#fff;" +
      "background:rgba(103,193,245,.25);outline:1px solid rgba(102,192,244,.6);outline-offset:2px";
    b.addEventListener("click", function (e) {
      e.preventDefault();
      e.stopPropagation();
      onClick(b);
    });
    return b;
  }

  function decorateLists() {
    var cards = document.querySelectorAll("a.search_result_row[data-ds-appid]");
    for (var i = 0; i < cards.length; i++) {
      var card = cards[i];
      var mine = false;
      for (var k = 0; k < card.children.length; k++) if (card.children[k].getAttribute("data-lv-row") !== null) mine = true;
      if (mine) continue;
      var appId = (card.getAttribute("data-ds-appid") || "").trim();
      if (!APPID_RULE.test(appId)) continue;
      (function (id) {
        var button =
          knownState(id) !== null
            ? rowButton(id, words("Dans la bibliothèque", "In your library"), function () {
                go(libraryHref(id));
              })
            : rowButton(id, words("Trouver", "Find"), function (b) {
                if (b.lvBusy) return;
                b.lvBusy = true;
                copyId(id);
                lvStore("find:" + id);
                setTimeout(function () {
                  b.lvBusy = false;
                }, BUSY_MS);
              });
        if (card.style && !card.style.position) card.style.position = "relative";
        card.appendChild(button);
      })(appId);
    }
  }

  function appIdFromPage() {
    var m = location.pathname.match(/^\/app\/(\d{1,10})(?:\/|$)/);
    return m && APPID_RULE.test(m[1]) ? m[1] : null;
  }

  // ───────────────────────────────────────────────────────────── le panneau LuaVault
  var JOB_TONES = { busy: "#66c0f4", ok: "#a4d007", warn: "#e5b143", error: "#c94a4a", muted: "#67707b" };
  var jobs = [];

  function validJob(job) {
    return (
      job !== null &&
      typeof job === "object" &&
      APPID_RULE.test(String(job.appId)) &&
      typeof job.name === "string" &&
      typeof job.line === "string" &&
      Object.prototype.hasOwnProperty.call(JOB_TONES, job.tone)
    );
  }

  function smallLink(text, green, onClick) {
    var link = document.createElement("a");
    link.className = (green ? "btn_green_steamui" : "btn_blue_steamui") + " btn_small";
    link.setAttribute("role", "button");
    link.style.cursor = "pointer";
    var caption = document.createElement("span");
    caption.textContent = text;
    link.appendChild(caption);
    link.addEventListener("click", function (e) {
      if (e && typeof e.preventDefault === "function") e.preventDefault();
      onClick();
    });
    return link;
  }

  function jobRow(job) {
    var appId = String(job.appId);
    var row = document.createElement("div");
    row.setAttribute("data-lv-job", appId);
    row.style.cssText = "display:flex;gap:10px;padding:10px 12px;border-top:1px solid rgba(255,255,255,.06)";
    var dot = document.createElement("div");
    dot.style.cssText =
      "flex:none;width:10px;height:10px;margin-top:4px;border-radius:50%;box-sizing:border-box;" +
      (job.tone === "busy" ? "border:2px solid " + JOB_TONES.busy + ";border-right-color:transparent" : "background:" + JOB_TONES[job.tone]);
    if (job.tone === "busy" && typeof dot.animate === "function" && !(typeof matchMedia === "function" && matchMedia("(prefers-reduced-motion: reduce)").matches)) {
      dot.animate([{ transform: "rotate(0deg)" }, { transform: "rotate(360deg)" }], { duration: 900, iterations: Infinity });
    }
    row.appendChild(dot);
    var body = document.createElement("div");
    body.style.cssText = "flex:1;min-width:0";
    var name = document.createElement("div");
    name.textContent = job.name.slice(0, 80);
    name.style.cssText = "color:#fff;font-size:13px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis";
    body.appendChild(name);
    var line = document.createElement("div");
    line.setAttribute("data-lv-job-line", "1");
    line.textContent = job.line.slice(0, 320);
    line.style.cssText = "color:#acb2b8;font-size:12px;line-height:16px;margin-top:2px";
    body.appendChild(line);
    if (job.install === true || job.library === true || job.site === true) {
      var actions = document.createElement("div");
      actions.style.cssText = "display:flex;gap:6px;margin-top:8px;flex-wrap:wrap";
      if (job.install === true) actions.appendChild(smallLink(words("Installer", "Install"), true, function () { go(installHref(appId)); }));
      if (job.library === true) actions.appendChild(smallLink(words("Bibliothèque", "Library"), false, function () { go(libraryHref(appId)); }));
      if (job.site === true)
        actions.appendChild(
          smallLink(words("Rouvrir le site", "Open the site again"), false, function () {
            copyId(appId);
            lvStore("find:" + appId);
          })
        );
      body.appendChild(actions);
    }
    row.appendChild(body);
    var close = document.createElement("span");
    close.setAttribute("role", "button");
    close.setAttribute("data-lv-job-close", appId);
    close.textContent = "×";
    close.style.cssText = "flex:none;color:#67707b;font-size:16px;line-height:14px;cursor:pointer";
    close.addEventListener("click", function () {
      forget(appId);
      lvStore("dismiss:" + appId);
    });
    row.appendChild(close);
    return row;
  }

  function renderPanel() {
    var old = document.querySelectorAll("[data-lv-panel]");
    for (var i = 0; i < old.length; i++) drop(old[i]);
    if (jobs.length === 0 || location.hostname !== "store.steampowered.com") return;
    var panel = document.createElement("div");
    panel.setAttribute("data-lv-panel", "1");
    panel.setAttribute("data-lv-node", "1");
    panel.style.cssText =
      "position:fixed;right:16px;bottom:16px;width:360px;z-index:9999;border-radius:4px;overflow:hidden;" +
      "background:linear-gradient(180deg,#22303f,#171d25);border:1px solid #2a3f5a;box-shadow:0 8px 24px rgba(0,0,0,.6);font-family:inherit";
    var head = document.createElement("div");
    head.textContent = "LuaVault";
    head.style.cssText = "padding:8px 12px;color:#66c0f4;font-size:11px;letter-spacing:.08em;text-transform:uppercase";
    panel.appendChild(head);
    for (var j = jobs.length - 1; j >= 0; j--) panel.appendChild(jobRow(jobs[j]));
    (document.body || root).appendChild(panel);
  }

  function remember(job) {
    var kept = [];
    for (var i = 0; i < jobs.length; i++) if (String(jobs[i].appId) !== String(job.appId)) kept.push(jobs[i]);
    kept.push(job);
    jobs = kept.slice(-8);
  }

  function forget(appId) {
    var kept = [];
    for (var i = 0; i < jobs.length; i++) if (String(jobs[i].appId) !== appId) kept.push(jobs[i]);
    jobs = kept;
    renderPanel();
  }

  var given = typeof lvStoreJobs === "object" && lvStoreJobs !== null && typeof lvStoreJobs.length === "number" ? lvStoreJobs : [];
  for (var g = 0; g < given.length; g++) if (validJob(given[g])) remember(given[g]);
  if (typeof window === "object" && window !== null) {
    // Only the latest pass answers.
    window.lvStoreJob = function (job) {
      if (root.lvRun !== myRun || !validJob(job)) return;
      remember(job);
      renderPanel();
    };
  }

  function run() {
    if (root.lvRun !== myRun) return;
    // The view moves from one of Valve's sites to another: nothing outside the store.
    if (location.hostname !== "store.steampowered.com") return;
    var pageAppId = appIdFromPage();
    if (pageAppId) {
      decorateGamePage(pageAppId);
      return;
    }
    if (location.pathname.indexOf("/search") === 0) decorateLists();
  }

  run();
  renderPanel();
  var mo = new MutationObserver(function () {
    if (root.lvRun !== myRun) return mo.disconnect();
    run();
  });
  mo.observe(root, { childList: true, subtree: true });
})();
