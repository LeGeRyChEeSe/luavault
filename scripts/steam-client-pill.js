// LuaVault : la pastille « LV » de la barre de titre du client Steam.
//
// Injectée par l'application (`steam_client.rs`) dans la fenêtre principale du client, et nulle
// part ailleurs. Elle se place en tête de la rangée de boutons en haut à droite (annonces,
// notifications, compte), sur tous les onglets de Steam. Un clic ouvre la fenêtre de LuaVault :
// « open » est le seul mot que cette fenêtre envoie à l'application.
//
// Ancrage lu sur le vrai client : `.avatarHolder` (nom de classe non haché) est dans un
// `.tool-tip-source`, enfant de la rangée ; les autres classes de la barre sont hachées et changent
// d'une version à l'autre de Steam.
(function () {
  "use strict";
  if (typeof lvStore !== "function") return;
  var root = document.documentElement;
  if (!root) return;
  // Its own run counter: a later pass (the application gives the script again at every page load)
  // silences the earlier one.
  root.lvWinRun = (root.lvWinRun || 0) + 1;
  var myRun = root.lvWinRun;
  var FR = String(root.lang || navigator.language || "fr").toLowerCase().indexOf("fr") === 0;

  function drop(node) {
    if (node && node.parentNode) node.parentNode.removeChild(node);
  }
  var stale = document.querySelectorAll("[data-lv-node]");
  for (var i = 0; i < stale.length; i++) drop(stale[i]);

  // The row of the top right buttons, found from the account button.
  function row() {
    var n = document.querySelector(".avatarHolder");
    while (n && !(n.classList && n.classList.contains("tool-tip-source"))) n = n.parentNode;
    return n && n.parentNode ? n.parentNode : null;
  }

  function pill() {
    var box = document.createElement("div");
    box.setAttribute("data-lv-node", "1");
    box.setAttribute("data-lv-pill", "1");
    box.className = "no-drag";
    box.style.cssText = "display:flex;align-items:center;margin-right:8px;-webkit-app-region:no-drag";
    var b = document.createElement("div");
    b.setAttribute("role", "button");
    b.setAttribute("data-lv-pill-open", "1");
    b.setAttribute("title", FR ? "Ouvrir LuaVault" : "Open LuaVault");
    b.textContent = "LV";
    b.style.cssText =
      "height:24px;line-height:24px;padding:0 8px;border-radius:2px;box-sizing:border-box;cursor:pointer;" +
      "background:#3d4450;color:#dcdedf;font-size:11px;font-weight:bold;letter-spacing:.06em";
    b.addEventListener("click", function (e) {
      if (e && typeof e.stopPropagation === "function") e.stopPropagation();
      lvStore("open");
    });
    box.appendChild(b);
    return box;
  }

  // Steam redraws its title bar: the pill is put back within a second, at the head of the row.
  function place() {
    if (root.lvWinRun !== myRun) return false;
    var r = row();
    if (!r) return true;
    var mine = document.querySelector("[data-lv-pill]");
    if (mine && mine.parentNode === r && r.firstChild === mine) return true;
    drop(mine);
    r.insertBefore(pill(), r.firstChild);
    return true;
  }

  place();
  var timer = setInterval(function () {
    if (!place()) clearInterval(timer);
  }, 1000);
})();
