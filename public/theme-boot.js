/**
 * Flash-free theme bootstrap.
 *
 * Loaded as a render-blocking classic script in <head> (see index.html) so it
 * runs synchronously BEFORE first paint and before the deferred module bundle.
 * It applies the active theme's pre-resolved CSS variables — a plain
 * { "--var": value } map written to localStorage by the theme manager.
 *
 * Two behaviours keep first paint correct:
 *  - Fresh install (no mirror yet): paint the factory default's colors baked in
 *    below, NOT the authored :root (ESO Gold) — the default is no longer :root.
 *  - Pending forced migration (mirror exists but this install hasn't been moved
 *    through the current forced-default version): ignore the soon-to-be-replaced
 *    mirror and paint the factory default, so the migration launch doesn't first
 *    paint the user's old theme.
 *
 * Skin note: a theme's texture/pattern IMAGE is applied later by hydration (the
 * SVG data-URIs are too large for this render-blocking script, and the texture
 * layer is React-rendered anyway). For a returning user the mirror carries the
 * skin vars, so we set `data-textured` from them to keep the glass tokens right.
 * For a fresh install / pending migration there is no skin yet, so we paint only
 * colors and let hydration apply the skin and `data-textured` together — marking
 * "textured" before the texture exists would briefly mis-tint the glass.
 *
 * Kept dependency-free and in /public so it ships as a same-origin asset that
 * satisfies the strict `script-src 'self'` Content-Security-Policy.
 */
(function () {
  var ACTIVE_VARS_KEY = "kalpa.appearance.activeVars";
  var FORCED_KEY = "kalpa.appearance.forcedDefaultVersion";
  // KEEP IN SYNC with FORCED_DEFAULT_VERSION in src/lib/theme-manager.ts —
  // guarded by src/lib/__tests__/theme-boot.test.ts.
  var FORCED_VERSION = 1;

  // Resolved color vars for the factory default (Nordic Runestone). KEEP IN SYNC
  // with DEFAULT_THEME in src/lib/theme-presets.ts — guarded by the same test.
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
    var textured = false;
    for (var name in vars) {
      if (Object.prototype.hasOwnProperty.call(vars, name) && typeof vars[name] === "string") {
        root.style.setProperty(name, vars[name]);
        if ((name === "--app-texture" || name === "--app-pattern") && vars[name] !== "none") {
          textured = true;
        }
      }
    }
    return textured;
  }

  var mirror = null;
  var applied = 0;
  try {
    var raw = localStorage.getItem(ACTIVE_VARS_KEY);
    if (raw) {
      var parsed = JSON.parse(raw);
      if (parsed && typeof parsed === "object") mirror = parsed;
    }
    applied = parseInt(localStorage.getItem(FORCED_KEY) || "0", 10) || 0;
  } catch (e) {
    /* storage unavailable/malformed — fall through to the factory default */
  }

  // Trust the per-user mirror only once this install has been through the current
  // forced migration; otherwise paint the factory default (colors only — the skin
  // and its data-textured flag arrive together at hydration).
  if (mirror && applied >= FORCED_VERSION) {
    if (apply(mirror)) root.dataset.textured = "true";
  } else {
    apply(DEFAULT_VARS);
  }
})();
