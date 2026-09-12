// SPDX-License-Identifier: AGPL-3.0-only
// Modified 2026-09-06; original notices: licenses/Winspace-MIT.txt.
'use strict';
/* Prefix selection over an already-loaded, display-ordered list.
 * No DOM, I/O, search-index access, or application activation in this module.
 * Browser: OpenXplorerTypeSelect. Node: require() for deterministic unit tests.
 */
(function (root, factory) {
  const api = factory();
  if (typeof module === 'object' && module.exports) module.exports = api;
  else root.OpenXplorerTypeSelect = api;
})(globalThis, function () {
  const TIMEOUT_MS = 1000;
  const MAX_PREFIX = 256;
  const fold = value => String(value ?? '').normalize('NFC').toLocaleLowerCase();
  const isCharacter = key => typeof key === 'string' &&
    Array.from(key).length === 1 && !/[\u0000-\u001f\u007f-\u009f]/u.test(key);

  function isTypingKey(event) {
    return !event.defaultPrevented && !event.isComposing && event.keyCode !== 229 &&
      !event.ctrlKey && !event.metaKey && !event.altKey && isCharacter(event.key);
  }

  /** Search by filename prefix in the supplied display order, with wraparound. */
  function findPrefix(entries, prefix, current = -1, includeCurrent = false) {
    if (!prefix || !entries.length) return -1;
    const valid = Number.isInteger(current) && current >= 0 && current < entries.length;
    const start = valid ? current + (includeCurrent ? 0 : 1) : 0;
    const wanted = fold(prefix);
    for (let offset = 0; offset < entries.length; offset++) {
      const index = (start + offset) % entries.length;
      if (fold(entries[index].name).startsWith(wanted)) return index;
    }
    return -1;
  }

  class Controller {
    constructor(timeoutMs = TIMEOUT_MS) {
      if (!Number.isFinite(timeoutMs) || timeoutMs <= 0) throw new TypeError('Invalid timeout');
      this.timeoutMs = timeoutMs;
      this.reset();
    }
    reset() { this.text = ''; this.lastAt = -Infinity; }
    active(now) { return !!this.text && now - this.lastAt >= 0 && now - this.lastAt < this.timeoutMs; }
    push(key, entries, current, now) {
      if (!isCharacter(key) || !Number.isFinite(now)) return null;
      const extending = this.active(now);
      // Repeating a single letter cycles through matches, rather than seeking "sss".
      const cycling = extending && fold(this.text) === fold(key);
      this.text = (!extending || cycling) ? key : Array.from(this.text + key).slice(0, MAX_PREFIX).join('');
      this.lastAt = now;
      const index = findPrefix(entries, this.text, current, extending && !cycling);
      return {text: this.text, index, cycling};
    }
    backspace(entries, current, now) {
      if (!this.active(now)) { this.reset(); return null; }
      this.text = Array.from(this.text).slice(0, -1).join('');
      this.lastAt = now;
      return {text: this.text, index: findPrefix(entries, this.text, current, true), cycling: false};
    }
  }
  return Object.freeze({Controller, TIMEOUT_MS, MAX_PREFIX, fold, findPrefix, isTypingKey});
});
