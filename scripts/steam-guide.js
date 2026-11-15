// LuaVault : le guide interactif dessiné dans le client Steam.
//
// Injecté par l'application (`steam_guide.rs`, via `steam_client.rs`) dans la fenêtre principale
// du client ET dans sa vue web (magasin, communauté), après leurs propres scripts. Jamais livré à un
// navigateur. Ce script ne décide rien : le moteur est dans la fenêtre de LuaVault
// (`src/lib/steam-guide/`), qui envoie une étape à la fois (`lvGuide(vue)`), déjà traduite.
// Chaque vue dessine sa part : l'anneau et le voile dans la vue de la cible (`where`), la bulle dans
// la sienne (`bubble`), car la vue web est posée PAR-DESSUS la fenêtre principale.
//
// Ce qu'il renvoie, par la seule fonction qu'on lui donne (`lvStore`), en mots fermés que
// l'application relit (`GuideCall::parse`) : `guide:next|back|quit|start`, `guide:choose:<id>`,
// `guide:hit:<cible>` (la cible a été cliquée), `guide:lost:<cible>` (introuvable depuis 3 s), et
// `guide:place:<lieu>` quand le lieu change (le moteur suit la personne, il ne la pousse pas).
//
// Les textes sont posés avec `textContent`, jamais en HTML. Les cibles sont presque toutes nos
// propres nœuds (`data-lv-*`) ; trois ancres viennent du vrai client, relevées le 2026-11-14 :
// les onglets (props `name` « #Menu_Store » / « #Menu_Library » de leur composant React) et le
// champ de recherche du magasin (`input[name=term]`, classe hachée).
(function () {
  "use strict";
  if (typeof lvStore !== "function") return;
  var root = document.documentElement;
  root.lvGuideRun = (root.lvGuideRun || 0) + 1;
  var myRun = root.lvGuideRun;
  var host = String((typeof location === "object" && location && location.hostname) || "");
  var KIND = host === "steamloopback.host" ? "window" : /^([a-z0-9-]+\.)*(steampowered|steamcommunity)\.com$/.test(host) ? "web" : null;
  if (KIND === null) return;
  var APPID_RULE = /^[1-9][0-9]{0,9}$/;
  var CHOICE_RULE = /^[a-z][a-z0-9-]{0,23}$/;

  function drop(node) {
    if (node && node.parentNode) node.parentNode.removeChild(node);
  }
  var stale = document.querySelectorAll("[data-lv-guide]");
  for (var s = 0; s < stale.length; s++) drop(stale[s]);

  function el(tag, css, text) {
    var n = document.createElement(tag);
    if (css) n.style.cssText = css;
    if (text !== undefined) n.textContent = text;
    return n;
  }
  function mark(n) {
    n.setAttribute("data-lv-node", "1");
    n.setAttribute("data-lv-guide", "1");
    return n;
  }
  function stop(e) {
    if (e && typeof e.stopPropagation === "function") e.stopPropagation();
    if (e && typeof e.preventDefault === "function") e.preventDefault();
  }
  function moves() {
    try {
      return !(typeof matchMedia === "function" && matchMedia("(prefers-reduced-motion: reduce)").matches);
    } catch (_) {
      return false;
    }
  }

  // ─────────────────────────────────────────────── les cibles
  function fiberOf(n) {
    for (var k in n) if (k.indexOf("__reactFiber") === 0) return n[k];
    return null;
  }
  // The tab of the main window whose component is named so (`#Menu_Store`), its outer div.
  function tab(name) {
    var divs = document.querySelectorAll("div");
    for (var i = 0; i < divs.length; i++) {
      var f = fiberOf(divs[i]);
      for (var up = 0; up < 4 && f; up++, f = f.return) {
        var p = f.memoizedProps;
        if (p && typeof p === "object" && p.name === name && "rootRoute" in p) return divs[i];
      }
    }
    return null;
  }
  // Of several candidates, the one a person actually sees: a game's page has two play bars (under
  // the artwork, and the sticky header that only shows once the page is scrolled), and the first
  // in the tree is the hidden one (seen on the real client, 2026-11-15). The test: what the screen
  // shows at the candidate's centre, our own layer aside, is the candidate or inside it.
  function ours(n) {
    for (var p = n; p; p = p.parentNode) if (p.getAttribute && p.getAttribute("data-lv-guide") !== null) return true;
    return false;
  }
  function seen(n) {
    var r = n.getBoundingClientRect();
    if (!r || r.width <= 0 || r.height <= 0) return false;
    if (typeof document.elementsFromPoint !== "function") return true;
    var hits = document.elementsFromPoint(r.left + r.width / 2, r.top + r.height / 2) || [];
    for (var i = 0; i < hits.length; i++) {
      if (ours(hits[i])) continue;
      return hits[i] === n || (typeof n.contains === "function" && n.contains(hits[i]));
    }
    return false;
  }
  function visible(list) {
    var first = null;
    for (var i = 0; i < list.length; i++) {
      if (!list[i]) continue;
      if (seen(list[i])) return list[i];
      if (first === null) first = list[i];
    }
    return first;
  }
  function byAttr(attr, appId) {
    var all = document.querySelectorAll("[" + attr + "]");
    var mine = [];
    for (var i = 0; i < all.length; i++) {
      if (!appId || all[i].getAttribute(attr) === appId) mine.push(all[i]);
    }
    return visible(mine);
  }
  var TARGETS = {
    pill: function () { return document.querySelector("[data-lv-pill-open]"); },
    "tab-store": function () { return tab("#Menu_Store"); },
    "tab-library": function () { return tab("#Menu_Library"); },
    settings: function () { return document.querySelector("[data-lv-settings]"); },
    "store-search": function () { return document.querySelector("input[name=term]"); },
    "store-find": function (id) { return byAttr("data-lv-find", id); },
    "store-owned": function (id) { return byAttr("data-lv-owned", id); },
    "store-panel": function () { return document.querySelector("[data-lv-panel]"); },
    "store-job": function (id) { return byAttr("data-lv-job", id); },
    "store-install": function (id) { return byAttr("data-lv-job-install", id); },
    "play-button": function () {
      return typeof window === "object" && window && typeof window.lvPlayButtons === "function" ? visible(window.lvPlayButtons()) : null;
    },
    "play-block": function (id) { return byAttr("data-lv-lib", id); },
    "play-manage": function (id) {
      var block = byAttr("data-lv-lib", id);
      return block ? block.querySelector("[data-lv-lib-open]") : null;
    },
    "play-menu": function () { return document.querySelector("[data-lv-lib-menu]"); },
    "list-dot": function (id) { return byAttr("data-lv-dot", id); },
  };
  // Targets that open something UNDER themselves (the store's search suggestions): the bubble goes
  // beside them, never below (seen on the real store, 2026-11-15: below, it hid the suggestions).
  var BESIDE = { "store-search": true };
  function shown(n) {
    if (!n) return null;
    var r = n.getBoundingClientRect();
    return r && r.width > 0 && r.height > 0 ? r : null;
  }

  // ─────────────────────────────────────────────── l'étape reçue
  function segments(list) {
    if (!list || typeof list.length !== "number") return false;
    for (var i = 0; i < list.length; i++) if (!list[i] || typeof list[i].t !== "string") return false;
    return true;
  }
  function valid(v) {
    if (v === null || typeof v !== "object") return false;
    if (typeof v.run !== "number" || ["window", "web", "any"].indexOf(v.where) < 0 || ["window", "web"].indexOf(v.bubble) < 0) return false;
    if (v.target !== undefined && v.target !== null && !Object.prototype.hasOwnProperty.call(TARGETS, v.target)) return false;
    if (v.appId !== undefined && v.appId !== null && !APPID_RULE.test(String(v.appId))) return false;
    if (!segments(v.info) || !segments(v.action) || !v.choices || typeof v.choices.length !== "number") return false;
    for (var i = 0; i < v.choices.length; i++) {
      var c = v.choices[i];
      if (!c || !CHOICE_RULE.test(String(c.id)) || typeof c.label !== "string") return false;
    }
    return typeof v.quit === "string";
  }

  var view = null;
  var layer = null;
  var parts = null;
  var lastRun = -1;
  var lostSent = false;
  var missingSince = 0;
  var lastPlace = null;
  var pending = false;

  function send(word) {
    if (root.lvGuideRun !== myRun) return;
    lvStore("guide:" + word);
  }

  function now() {
    return Date.now();
  }

  // The view this script draws the target in: its own, or `any` (the settings panel, drawn by
  // whichever view the client shows) when the target is found here.
  function targetHere() {
    if (!view || !view.target) return false;
    return view.where === KIND || view.where === "any";
  }

  function text(list, css, uiCss) {
    var p = el("div", css);
    for (var i = 0; i < list.length; i++) {
      var seg = list[i];
      var span = el("span", seg.ui ? uiCss : "", seg.t.slice(0, 400));
      if (seg.ui) span.setAttribute("data-lv-guide-ui", "1");
      p.appendChild(span);
    }
    return p;
  }

  function button(label, primary, word, attr) {
    var b = el(
      "div",
      "padding:6px 14px;border-radius:2px;font-size:13px;cursor:pointer;user-select:none;color:#fff;" +
        (primary ? "background:linear-gradient(90deg,#06bfff,#2d73ff)" : "background:#3d4450;color:#dcdedf"),
      label.slice(0, 60),
    );
    b.setAttribute("role", "button");
    b.setAttribute(attr, word);
    if (primary) b.setAttribute("data-lv-guide-primary", "1");
    b.addEventListener("click", function (e) {
      stop(e);
      if (pending) return;
      pending = true;
      b.style.opacity = ".6";
      send(word);
    });
    if (primary && moves() && typeof b.animate === "function") {
      b.animate(
        [{ boxShadow: "0 0 0 0 rgba(26,159,255,.55)" }, { boxShadow: "0 0 0 6px rgba(26,159,255,0)" }],
        { duration: 1400, iterations: Infinity },
      );
    }
    return b;
  }

  function bubble() {
    var b = mark(el(
      "div",
      "position:fixed;width:340px;max-width:calc(100vw - 32px);box-sizing:border-box;padding:14px 16px;border-radius:4px;" +
        "background:linear-gradient(180deg,#22303f,#171d25);border:1px solid #2a3f5a;box-shadow:0 8px 24px rgba(0,0,0,.6);" +
        "pointer-events:auto;-webkit-app-region:no-drag;font-family:'Motiva Sans',Arial,sans-serif;left:-9999px;top:-9999px",
    ));
    b.setAttribute("data-lv-guide-bubble", "1");
    b.setAttribute("role", "dialog");
    b.addEventListener("click", function (e) {
      if (e && typeof e.stopPropagation === "function") e.stopPropagation();
    });
    var head = el("div", "display:flex;align-items:baseline;justify-content:space-between;gap:8px");
    head.appendChild(el("div", "color:#66c0f4;font-size:11px;letter-spacing:.08em;text-transform:uppercase", (view.title || "LuaVault").slice(0, 60)));
    if (view.progress) head.appendChild(el("div", "color:#67707b;font-size:11px;white-space:nowrap", view.progress.slice(0, 60)));
    b.appendChild(head);
    var ui = "color:#fff;background:rgba(255,255,255,.1);border-radius:2px;padding:0 4px";
    if (view.info.length > 0) {
      var info = text(view.info, "color:#acb2b8;font-size:13px;line-height:18px;margin-top:8px", ui);
      info.setAttribute("data-lv-guide-info", "1");
      b.appendChild(info);
    }
    if (view.action.length > 0) {
      var action = text(view.action, "color:#66c0f4;font-size:13px;line-height:18px;font-weight:bold;margin-top:8px", ui);
      action.setAttribute("data-lv-guide-action", "1");
      action.insertBefore(el("span", "margin-right:6px", "›"), action.firstChild);
      b.appendChild(action);
    }
    if (view.choices.length > 0) {
      var row = el("div", "display:flex;flex-wrap:wrap;gap:6px;margin-top:12px");
      for (var i = 0; i < view.choices.length; i++) {
        var c = view.choices[i];
        row.appendChild(button(c.label, c.primary === true, "choose:" + c.id, "data-lv-guide-choice"));
      }
      b.appendChild(row);
    }
    var foot = el("div", "display:flex;align-items:center;gap:6px;margin-top:12px;padding-top:10px;border-top:1px solid rgba(255,255,255,.08)");
    var quit = el("div", "color:#8b929a;font-size:12px;cursor:pointer;margin-right:auto", view.quit.slice(0, 60));
    quit.setAttribute("role", "button");
    quit.setAttribute("data-lv-guide-quit", "1");
    quit.addEventListener("click", function (e) {
      stop(e);
      send("quit");
    });
    foot.appendChild(quit);
    if (view.back) foot.appendChild(button(view.back, false, "back", "data-lv-guide-back"));
    if (view.next) foot.appendChild(button(view.next, view.choices.length === 0, "next", "data-lv-guide-next"));
    b.appendChild(foot);
    return b;
  }

  function build() {
    drop(layer);
    layer = null;
    parts = null;
    if (!view) return;
    layer = mark(el("div", "position:fixed;left:0;top:0;width:0;height:0;z-index:2147483600;pointer-events:none"));
    parts = { veil: [], ring: null, bubble: null };
    var wantsVeil = view.veil && (targetHere() || (!view.target && view.bubble === KIND));
    if (wantsVeil) {
      for (var i = 0; i < 4; i++) {
        var v = mark(el("div", "position:fixed;background:rgba(0,0,0,.6);pointer-events:auto;-webkit-app-region:no-drag;left:0;top:0;width:0;height:0"));
        v.setAttribute("data-lv-guide-veil", String(i));
        v.addEventListener("click", stop);
        v.addEventListener("mousedown", stop);
        parts.veil.push(v);
        layer.appendChild(v);
      }
    }
    if (targetHere()) {
      var ring = mark(el("div", "position:fixed;border:2px solid #1a9fff;border-radius:4px;box-shadow:0 0 0 3px rgba(26,159,255,.25),0 0 18px rgba(26,159,255,.6);pointer-events:none;display:none"));
      ring.setAttribute("data-lv-guide-ring", view.target);
      if (moves() && typeof ring.animate === "function") {
        ring.animate([{ opacity: 1 }, { opacity: 0.45 }, { opacity: 1 }], { duration: 1600, iterations: Infinity });
      }
      parts.ring = ring;
      layer.appendChild(ring);
    }
    if (view.bubble === KIND) {
      parts.bubble = bubble();
      layer.appendChild(parts.bubble);
    }
    (document.body || root).appendChild(layer);
    place();
  }

  function box(n, x, y, w, h) {
    n.style.left = Math.round(x) + "px";
    n.style.top = Math.round(y) + "px";
    n.style.width = Math.max(0, Math.round(w)) + "px";
    n.style.height = Math.max(0, Math.round(h)) + "px";
  }

  // What the person opens wins over the guide (the rule of the window's guide): the « Gérer » menu,
  // Steam's right-click menu, the pill's panel. Under the veil they opened and could not be used
  // (seen on the real client, 2026-11-15). The guide steps aside while one is open.
  var OPENED = { "[data-lv-lib-menu]": "play-menu", ".contextMenuItem": null, "[data-lv-settings]": "settings" };
  function yielded() {
    for (var sel in OPENED) {
      if (!Object.prototype.hasOwnProperty.call(OPENED, sel) || OPENED[sel] === view.target) continue;
      var n = document.querySelector(sel);
      if (n && shown(n)) return true;
    }
    return false;
  }

  // Position everything against the target as it stands now (Steam moves things around).
  function place() {
    if (!view || !parts) return;
    var aside = yielded();
    layer.style.display = aside ? "none" : "";
    if (aside) return;
    var W = innerWidth, H = innerHeight;
    var node = targetHere() ? TARGETS[view.target](view.appId || null) : null;
    var r = shown(node);
    // Beside a field, beside its whole form: the store's magnifier sits right of the field.
    var side = r && BESIDE[view.target] && node.form ? shown(node.form) || r : r;
    if (targetHere() && view.where !== "any") {
      if (r) {
        missingSince = 0;
      } else if (missingSince === 0) {
        missingSince = now();
      } else if (!lostSent && now() - missingSince > 3000) {
        lostSent = true;
        send("lost:" + view.target);
      }
    }
    var pad = 6;
    var hole = r ? { x: r.left - pad, y: r.top - pad, w: r.width + 2 * pad, h: r.height + 2 * pad } : null;
    if (parts.veil.length === 4) {
      if (hole) {
        box(parts.veil[0], 0, 0, W, hole.y);
        box(parts.veil[1], 0, hole.y + hole.h, W, H - hole.y - hole.h);
        box(parts.veil[2], 0, hole.y, hole.x, hole.h);
        box(parts.veil[3], hole.x + hole.w, hole.y, W - hole.x - hole.w, hole.h);
      } else if (!view.target) {
        box(parts.veil[0], 0, 0, W, H);
        for (var i = 1; i < 4; i++) box(parts.veil[i], 0, 0, 0, 0);
      } else {
        // The target is not drawn yet: never a veil that would stop the page with nothing to click.
        for (var j = 0; j < 4; j++) box(parts.veil[j], 0, 0, 0, 0);
      }
    }
    if (parts.ring) {
      if (hole) {
        parts.ring.style.display = "block";
        box(parts.ring, hole.x, hole.y, hole.w, hole.h);
      } else {
        parts.ring.style.display = "none";
      }
    }
    var b = parts.bubble;
    if (!b) return;
    var bw = Math.min(340, W - 32);
    var bh = b.getBoundingClientRect().height || 180;
    var x, y;
    if (view.corner) {
      x = 16;
      y = H - bh - 16;
    } else if (hole && BESIDE[view.target] && side.left + side.width + pad + 12 + bw <= W - 16) {
      x = side.left + side.width + pad + 12;
      y = Math.max(8, hole.y);
    } else if (hole && BESIDE[view.target] && side.left - pad - 12 - bw >= 16) {
      x = side.left - pad - 12 - bw;
      y = Math.max(8, hole.y);
    } else if (hole) {
      x = Math.min(Math.max(16, hole.x + hole.w / 2 - bw / 2), W - bw - 16);
      y = hole.y + hole.h + 12 + bh <= H - 8 ? hole.y + hole.h + 12 : Math.max(8, hole.y - bh - 12);
    } else if (view.target) {
      // The target is in the other view (the title bar, behind this one): the top right corner.
      x = W - bw - 16;
      y = 12;
    } else {
      x = (W - bw) / 2;
      y = Math.max(16, (H - bh) / 2);
    }
    b.style.left = Math.round(x) + "px";
    b.style.top = Math.round(y) + "px";
  }

  // ─────────────────────────────────────────────── le lieu
  function placeNow() {
    if (KIND === "window") {
      return typeof window === "object" && window && typeof window.lvWindowPlace === "function" ? String(window.lvWindowPlace()) : "window";
    }
    if (document.visibilityState && document.visibilityState !== "visible") return null;
    if (host !== "store.steampowered.com") return "web";
    var path = String(location.pathname || "");
    var app = /^\/app\/([1-9][0-9]{0,9})(\/|$)/.exec(path);
    if (app) {
      var flags = "";
      if (document.querySelector(".game_area_already_owned")) flags += "o";
      if (typeof lvStoreKnown === "object" && lvStoreKnown !== null && Object.prototype.hasOwnProperty.call(lvStoreKnown, app[1])) flags += "k";
      return "store-app:" + app[1] + (flags ? ":" + flags : "");
    }
    if (/^\/(bundle|sub)\//.test(path)) return "store-bundle";
    return "store";
  }
  var placeTicks = 0;
  function report() {
    if (!view) return;
    // Reading the main window's place walks its whole tree: once a second is plenty.
    if (placeTicks++ % 4 !== 0) return;
    var p = placeNow();
    if (p === null || p === lastPlace) return;
    lastPlace = p;
    send("place:" + p);
  }

  // ─────────────────────────────────────────────── le geste
  document.addEventListener(
    "click",
    function (e) {
      if (root.lvGuideRun !== myRun || !view || !view.gesture || !targetHere() || !e || !e.target) return;
      var t = TARGETS[view.target](view.appId || null);
      if (t && (t === e.target || (typeof t.contains === "function" && t.contains(e.target)))) send("hit:" + view.target);
    },
    true,
  );

  if (typeof window === "object" && window !== null) {
    window.lvGuide = function (v) {
      if (root.lvGuideRun !== myRun) return;
      view = valid(v) ? v : null;
      if (!view || view.run !== lastRun) {
        lastRun = view ? view.run : -1;
        lostSent = false;
        missingSince = 0;
        lastPlace = null;
        placeTicks = 0;
      }
      pending = false;
      build();
      report();
    };
  }
  if (typeof lvGuideView !== "undefined" && lvGuideView !== null && typeof window === "object" && window && window.lvGuide) {
    window.lvGuide(lvGuideView);
  }
  var timer = setInterval(function () {
    if (root.lvGuideRun !== myRun) return clearInterval(timer);
    place();
    report();
  }, 250);
})();
