/**
 * Flash-free theme bootstrap.
 *
 * Loaded as a render-blocking classic script in <head> (see index.html) so it
 * runs synchronously BEFORE first paint and before the deferred module bundle.
 * It applies the active theme's pre-resolved CSS variables — a plain
 * { "--var": value } map written to localStorage by the theme manager — so this
 * file needs zero application logic. The default theme stores nothing here and
 * falls back to the authored :root values in index.css.
 *
 * Kept dependency-free and in /public so it ships as a same-origin asset that
 * satisfies the strict `script-src 'self'` Content-Security-Policy.
 */
(function () {
  try {
    var raw = localStorage.getItem("kalpa.appearance.activeVars");
    if (!raw) return;
    var vars = JSON.parse(raw);
    if (!vars || typeof vars !== "object") return;
    var root = document.documentElement;
    for (var name in vars) {
      if (Object.prototype.hasOwnProperty.call(vars, name) && typeof vars[name] === "string") {
        root.style.setProperty(name, vars[name]);
      }
    }
  } catch (e) {
    /* malformed/unavailable storage — fall back to the default dark theme */
  }
})();
