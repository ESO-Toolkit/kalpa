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
    "--orb-3": "#4a5a52",
    "--structure-rgb": "255 255 255",
    "--scrim-rgb": "0 0 0",
    "--status-success": "#34d399",
    "--status-success-soft": "#6ee7b7",
    "--status-success-muted": "#a7f3d0",
    "--status-success-faint": "#d1fae5",
    "--status-success-strong": "#10b981",
    "--status-warning": "#fbbf24",
    "--status-warning-soft": "#fcd34d",
    "--status-warning-muted": "#fde68a",
    "--status-warning-faint": "#fef3c7",
    "--status-warning-strong": "#f59e0b",
    "--status-danger": "#f87171",
    "--status-danger-soft": "#fca5a5",
    "--status-danger-muted": "#fecaca",
    "--status-danger-faint": "#fee2e2",
    "--status-danger-strong": "#ef4444",
    "--status-library": "#a78bfa",
    "--status-library-strong": "#8b5cf6",
    "--status-info": "#38bdf8",
    "--status-info-soft": "#7dd3fc",
    "--status-info-strong": "#0ea5e9",
    "--status-warning-readable": "#d9a441",
    "--brand-gold-readable": "#c4a44a",
    "--brand-cyan-readable": "#4dc2e6",
    "--addon-disabled": "#6b7280",
    "--status-error": "#f87171",
    "--status-error-strong": "#ef4444",
  };

  var root = document.documentElement;
  var LIGHT_THEME_BACKGROUND_LUMINANCE = 0.45;
  var DARK_STATUS_VARS = {
    "--status-success": "#34d399",
    "--status-success-soft": "#6ee7b7",
    "--status-success-muted": "#a7f3d0",
    "--status-success-faint": "#d1fae5",
    "--status-success-strong": "#10b981",
    "--status-warning": "#fbbf24",
    "--status-warning-soft": "#fcd34d",
    "--status-warning-muted": "#fde68a",
    "--status-warning-faint": "#fef3c7",
    "--status-warning-strong": "#f59e0b",
    "--status-danger": "#f87171",
    "--status-danger-soft": "#fca5a5",
    "--status-danger-muted": "#fecaca",
    "--status-danger-faint": "#fee2e2",
    "--status-danger-strong": "#ef4444",
    "--status-library": "#a78bfa",
    "--status-library-strong": "#8b5cf6",
    "--status-info": "#38bdf8",
    "--status-info-soft": "#7dd3fc",
    "--status-info-strong": "#0ea5e9",
    "--status-warning-readable": "#d9a441",
    "--brand-gold-readable": "#c4a44a",
    "--brand-cyan-readable": "#4dc2e6",
    "--addon-disabled": "#6b7280",
  };
  var LIGHT_STATUS_VARS = {
    "--status-success": "#022c22",
    "--status-success-soft": "#022c22",
    "--status-success-muted": "#022c22",
    "--status-success-faint": "#022c22",
    "--status-success-strong": "#022c22",
    "--status-warning": "#451a03",
    "--status-warning-soft": "#451a03",
    "--status-warning-muted": "#451a03",
    "--status-warning-faint": "#451a03",
    "--status-warning-strong": "#451a03",
    "--status-danger": "#450a0a",
    "--status-danger-soft": "#450a0a",
    "--status-danger-muted": "#450a0a",
    "--status-danger-faint": "#450a0a",
    "--status-danger-strong": "#450a0a",
    "--status-library": "#6d28d9",
    "--status-library-strong": "#6d28d9",
    "--status-info": "#0369a1",
    "--status-info-soft": "#0369a1",
    "--status-info-strong": "#075985",
    "--status-warning-readable": "#451a03",
  };

  function relativeLuminance(hex) {
    var h = String(hex || "")
      .trim()
      .replace(/^#/, "");
    if (h.length === 3)
      h = h.replace(/./g, function (c) {
        return c + c;
      });
    if (h.length !== 6 || /[^0-9a-fA-F]/.test(h)) return 0;
    function channel(i) {
      var s = parseInt(h.slice(i, i + 2), 16) / 255;
      return s <= 0.03928 ? s / 12.92 : Math.pow((s + 0.055) / 1.055, 2.4);
    }
    return 0.2126 * channel(0) + 0.7152 * channel(2) + 0.0722 * channel(4);
  }

  function statusVarsForTheme(themeVars) {
    var isLight = relativeLuminance(themeVars["--background"]) >= LIGHT_THEME_BACKGROUND_LUMINANCE;
    var source = isLight ? LIGHT_STATUS_VARS : DARK_STATUS_VARS;
    var vars = {};
    for (var name in source) vars[name] = source[name];
    vars["--brand-gold-readable"] = isLight
      ? themeVars["--primary"]
      : DARK_STATUS_VARS["--brand-gold-readable"];
    vars["--brand-cyan-readable"] = isLight
      ? themeVars["--accent-sky"]
      : DARK_STATUS_VARS["--brand-cyan-readable"];
    vars["--addon-disabled"] = isLight
      ? themeVars["--muted-foreground"]
      : DARK_STATUS_VARS["--addon-disabled"];
    vars["--status-error"] = source["--status-danger"];
    vars["--status-error-strong"] = source["--status-danger-strong"];
    return vars;
  }

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
    var textured = apply(mirror);
    // The mirror already carries the status vars (themeColorsToVars bakes them
    // in), and it is the authoritative copy. The tables above are only a
    // fallback for mirrors written before those vars were mirrored, so fill in
    // what is missing rather than painting over what is there.
    var fallback = statusVarsForTheme(mirror);
    for (var name in fallback) {
      if (Object.prototype.hasOwnProperty.call(mirror, name)) delete fallback[name];
    }
    apply(fallback);
    if (textured) root.dataset.textured = "true";
  } else {
    apply(DEFAULT_VARS);
  }
})();
