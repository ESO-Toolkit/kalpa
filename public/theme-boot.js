/**
 * Flash-free theme bootstrap.
 *
 * Loaded as a render-blocking classic script in <head> (see index.html) so it
 * runs synchronously BEFORE first paint and before the deferred module bundle.
 * It applies the active theme's pre-resolved CSS variables — a plain
 * { "--var": value } map written to localStorage by the theme manager — so this
 * file needs zero application logic.
 *
 * When no mirror exists yet (a genuinely fresh install), it falls back to the
 * factory default's colors baked in below, NOT the authored :root values — the
 * default theme is no longer the :root (ESO Gold) palette, so painting :root
 * first would flash the wrong theme. Colors only: a theme's skin (texture /
 * pattern) is React-rendered after hydration and never affects first paint.
 *
 * Kept dependency-free and in /public so it ships as a same-origin asset that
 * satisfies the strict `script-src 'self'` Content-Security-Policy.
 */
(function () {
  // Resolved color vars for the factory default (Nordic Runestone).
  // KEEP IN SYNC with DEFAULT_THEME in src/lib/theme-presets.ts — guarded by
  // src/lib/__tests__/theme-boot.test.ts.
  var DEFAULT_VARS = {
    "--bg-base": "#16181b",
    "--background": "#191c20",
    "--card": "#23272d",
    "--foreground": "#e7e2d4",
    "--muted-foreground": "#9a9b96",
    "--primary": "#d2a14e",
    "--primary-foreground": "#1a1611",
    "--accent-sky": "#6fa8c4",
    "--border": "#3a3f46",
    "--orb-1": "#d2a14e",
    "--orb-2": "#5d8aa8",
    "--orb-3": "#4a5a52"
  };
  var root = document.documentElement;
  function apply(vars) {
    for (var name in vars) {
      if (Object.prototype.hasOwnProperty.call(vars, name) && typeof vars[name] === "string") {
        root.style.setProperty(name, vars[name]);
      }
    }
  }
  var vars = null;
  try {
    var raw = localStorage.getItem("kalpa.appearance.activeVars");
    if (raw) {
      var parsed = JSON.parse(raw);
      if (parsed && typeof parsed === "object") vars = parsed;
    }
  } catch (e) {
    /* storage unavailable/malformed — fall through to the factory default */
  }
  apply(vars || DEFAULT_VARS);
})();
