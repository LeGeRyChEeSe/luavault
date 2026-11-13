// LuaVault : les annonces dans Steam.
//
// Injecté par l'application (`steam_client.rs`) à la suite du script de la fenêtre principale et de
// celui de la boutique. Il ne pose rien tant qu'il n'a rien à dire : il définit `lvNotice`, que
// l'application appelle quand elle détecte un `.lua`, un correctif ou un contournement DRM, ou quand
// un correctif en attente s'applique. La carte reprend le style des notifications de Steam (image du
// jeu, un titre, une ligne), en bas à droite, et part seule après 10 s. Les textes viennent de
// l'application et sont posés comme du texte, jamais comme du HTML.
//
// Les cartes ne portent pas `data-lv-node` : les autres scripts retirent tout ce qui le porte
// quand l'application les redonne (un jeu arrivé, une page rechargée), et une annonce doit survivre
// à ce passage.
(function () {
  "use strict";
  var root = document.documentElement;
  if (!root) return;
  root.lvNoticeRun = (root.lvNoticeRun || 0) + 1;
  var myRun = root.lvNoticeRun;
  var FR = String(root.lang || navigator.language || "fr").toLowerCase().indexOf("fr") === 0;
  var TONES = { ok: "#a4d007", warn: "#e5b143", error: "#c94a4a", info: "#66c0f4" };
  var SHOWN_MS = 10000;

  function el(tag, css, text) {
    var n = document.createElement(tag);
    if (css) n.style.cssText = css;
    if (text !== undefined) n.textContent = text;
    return n;
  }
  function drop(node) {
    if (node && node.parentNode) node.parentNode.removeChild(node);
  }

  function valid(n) {
    return (
      n !== null &&
      typeof n === "object" &&
      typeof n.title === "string" &&
      typeof n.line === "string" &&
      Object.prototype.hasOwnProperty.call(TONES, n.tone) &&
      (n.appId === null || n.appId === undefined || /^[1-9][0-9]{0,9}$/.test(String(n.appId)))
    );
  }

  // Above the store's panel when there is one, in the bottom right corner otherwise.
  function stack() {
    var s = document.querySelector("[data-lv-notices]");
    if (!s) {
      s = el("div", "position:fixed;right:16px;bottom:16px;z-index:2147483001;display:flex;flex-direction:column;gap:8px;align-items:flex-end;pointer-events:none");
      s.setAttribute("data-lv-notices", "1");
      (document.body || root).appendChild(s);
    }
    var panel = document.querySelector("[data-lv-panel]");
    // In the main window, above Steam's bottom bar (« Ajouter un jeu », downloads), 40 px high.
    var bottom = location.hostname === "steamloopback.host" ? 56 : 16;
    if (panel && typeof panel.getBoundingClientRect === "function") {
      var r = panel.getBoundingClientRect();
      if (r.height > 0) bottom = Math.max(bottom, Math.round(window.innerHeight - r.top + 8));
    }
    s.style.bottom = bottom + "px";
    return s;
  }

  function show(n) {
    if (root.lvNoticeRun !== myRun || !valid(n)) return;
    var card = el(
      "div",
      "pointer-events:auto;display:flex;width:340px;background:linear-gradient(135deg,#2a3a4d,#1b2534);box-shadow:0 6px 18px rgba(0,0,0,.55);" +
        "border-left:3px solid " + TONES[n.tone] + ";border-radius:3px;overflow:hidden;font-family:'Motiva Sans',Arial,sans-serif"
    );
    card.setAttribute("data-lv-notice", n.tone);
    if (n.appId) {
      var img = el("img", "width:96px;height:auto;align-self:center;flex-shrink:0;margin-left:8px;border-radius:2px");
      img.alt = "";
      img.onerror = function () {
        drop(img);
      };
      img.src = "https://shared.cloudflare.steamstatic.com/store_item_assets/steam/apps/" + String(n.appId) + "/header.jpg";
      card.appendChild(img);
    }
    var body = el("div", "flex:1;min-width:0;padding:8px 10px");
    body.appendChild(el("div", "color:#66c0f4;font-size:9px;letter-spacing:.14em;text-transform:uppercase", "LuaVault"));
    body.appendChild(el("div", "color:#ffffff;font-size:13px;font-weight:600;margin-top:2px", n.title.slice(0, 80)));
    body.appendChild(el("div", "color:#acb2b8;font-size:12px;margin-top:2px;line-height:1.35", n.line.slice(0, 240)));
    card.appendChild(body);
    var close = el("div", "color:#8f98a0;cursor:pointer;padding:6px 8px;font-size:15px;line-height:15px", "×");
    close.setAttribute("role", "button");
    close.setAttribute("title", FR ? "Fermer" : "Close");
    close.setAttribute("data-lv-notice-close", "1");
    card.appendChild(close);
    var timer = setTimeout(function () {
      drop(card);
    }, SHOWN_MS);
    close.addEventListener("click", function (e) {
      if (e && typeof e.stopPropagation === "function") e.stopPropagation();
      clearTimeout(timer);
      drop(card);
    });
    if (n.open === true && typeof lvStore === "function") {
      card.style.cursor = "pointer";
      card.setAttribute("title", FR ? "Ouvrir LuaVault" : "Open LuaVault");
      card.addEventListener("click", function () {
        clearTimeout(timer);
        drop(card);
        lvStore("open");
      });
    }
    stack().appendChild(card);
  }

  if (typeof window === "object" && window !== null) window.lvNotice = show;
})();
