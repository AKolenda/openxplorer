// SPDX-License-Identifier: AGPL-3.0-only
// Modified 2026-09-06; original notices: licenses/Winspace-MIT.txt.
/* Runs before the stylesheet and first paint. No network or filesystem access.
   Native preferences are injected at document START by the host. */
(() => {
  'use strict';
  const native = window.__OPENXPLORER_NATIVE__ === true;
  let theme = native ? (window.__OPENXPLORER_BOOT__?.theme || 'system') : 'dark';
  if (!native) {
    try {
      const saved = JSON.parse(localStorage.getItem('winspace-preview-v3') || localStorage.getItem('winspace-preview-v2') || '{}');
      if (['light', 'dark', 'system'].includes(saved.preferences?.theme)) theme = saved.preferences.theme;
    } catch (_) { /* file:// or opaque browser origins may disallow storage */ }
  }
  const systemDark = native ? !!window.__OPENXPLORER_BOOT__?.systemDark :
    !!window.matchMedia?.('(prefers-color-scheme: dark)').matches;
  const dark = theme === 'dark' || (theme === 'system' && systemDark);
  document.documentElement.dataset.theme = dark ? 'dark' : 'light';
  window.__OPENXPLORER_FIRST_THEME__ = theme;
})();
