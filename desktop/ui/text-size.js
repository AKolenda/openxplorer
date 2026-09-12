// SPDX-License-Identifier: AGPL-3.0-only
/* Text scaling shared by the native UI and preview. Does not zoom the browser
   or change desktop DPI. Row metrics follow text size, including virtualization. */
((root) => {
  'use strict';
  const levels = Object.freeze([80, 90, 100, 110, 125, 150, 175, 200]);
  const normalize = value => levels.includes(value) ? value : 100;
  const step = (value, direction) => levels[Math.max(0, Math.min(levels.length - 1, levels.indexOf(normalize(value)) + direction))];
  function action(e) {
    if (!(e.ctrlKey || e.metaKey) || e.altKey || e.isComposing || e.keyCode === 229 || e.getModifierState?.('AltGraph')) return null;
    if (['+', '=', 'Add'].includes(e.key) || e.code === 'NumpadAdd') return 'increase';
    if (['-', '_', 'Subtract'].includes(e.key) || e.code === 'NumpadSubtract') return 'decrease';
    if (e.key === '0' || e.code === 'Numpad0') return 'reset';
    return null;
  }
  function metrics(value) {
    const scale = normalize(value) / 100;
    return {scale, detailRow: Math.max(38, Math.ceil(24 * scale + 14)),
      gridRow: 130 + Math.max(0, Math.ceil((scale - 1) * 46)),
      gridWidth: Math.max(135, Math.ceil(90 * scale + 45))};
  }
  const api = {levels, normalize, step, action, metrics};
  root.OpenXplorerTextSize = api;
  if (typeof module !== 'undefined' && module.exports) module.exports = api;
})(typeof window === 'undefined' ? globalThis : window);
