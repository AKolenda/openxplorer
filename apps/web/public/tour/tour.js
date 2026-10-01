// SPDX-License-Identifier: AGPL-3.0-only
// The click-through tour of the native app. scenes.js (built from scenes.json
// by tools/prepare-web.cjs) sets window.OPENXPLORER_TOUR: pictures of the real
// app and the rectangles of its controls, which tools/capture-native-tour.py
// read from the app. A control leads to another picture; Back returns.
//
// The page also answers the website's demo controls (interactions.js) over
// postMessage: "status", "scene", "theme", "reset", "play" and "stop", and
// reports its phase: ready, playing, complete, stopped or error.
'use strict';
(() => {
  const tour = window.OPENXPLORER_TOUR;
  const $ = id => document.getElementById(id);
  const picture = $('picture'), frame = $('frame'), layer = $('hotspots');
  const back = $('back'), title = $('title'), caption = $('caption'), hint = $('hint');
  const params = new URLSearchParams(location.search);
  const embedded = params.has('embed') && window.parent !== window;
  const reducedMotion = window.matchMedia('(prefers-reduced-motion: reduce)');
  // The path Play follows: each step clicks the control of one scene that
  // leads to the next.
  const PLAY = [['home', 'documents'], ['documents', 'folder'], ['folder', 'context-menu'],
    ['context-menu', 'versions'], ['versions', 'properties']];
  let theme = params.get('theme') === 'dark' ? 'dark'
    : params.get('theme') === 'light' ? 'light'
      : (window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light');
  let current = null, history = [], timer = 0, playing = false;

  if (!tour || !Array.isArray(tour.scenes)) {
    hint.textContent = 'The tour pictures are missing. Run tools/prepare-web.cjs.';
    report('error', 'The tour could not load.');
    return;
  }
  const scenes = new Map(tour.scenes.map(scene => [scene.id, scene]));

  function report(phase, text) {
    if (embedded) window.parent.postMessage({channel: 'openxplorer-demo', phase, text}, '*');
  }

  function percent(value, total) { return (100 * value / total).toFixed(3) + '%'; }

  function show(id, remember = true) {
    const scene = scenes.get(id);
    if (!scene) return false;
    if (remember && current && current.id !== id) history.push(current.id);
    current = scene;
    document.documentElement.dataset.theme = theme;
    picture.src = scene.images[theme];
    picture.alt = `${scene.title}: ${scene.caption}`;
    title.textContent = scene.title;
    caption.textContent = scene.caption;
    back.disabled = history.length === 0;
    layer.replaceChildren(...scene.hotspots.map(hotspot => button(hotspot)));
    frame.classList.remove('arrived');
    if (!reducedMotion.matches) {
      void frame.offsetWidth; // Restart the hint animation.
      frame.classList.add('arrived');
    }
    return true;
  }

  function button(hotspot) {
    const control = document.createElement('button');
    control.type = 'button';
    control.className = 'hotspot';
    if (hotspot.y + hotspot.height > tour.height * 0.85) control.classList.add('above');
    Object.assign(control.style, {
      left: percent(hotspot.x, tour.width), top: percent(hotspot.y, tour.height),
      width: percent(hotspot.width, tour.width), height: percent(hotspot.height, tour.height),
    });
    control.dataset.goesTo = hotspot.goesTo;
    control.setAttribute('aria-label', hotspot.label);
    const label = document.createElement('span');
    label.textContent = hotspot.label;
    control.append(label);
    control.addEventListener('click', () => { stop(false); follow(hotspot.goesTo); });
    return control;
  }

  function follow(id) {
    if (show(id)) report('ready', `${current.title}: ${current.caption}`);
  }

  function goBack() {
    stop(false);
    const previous = history.pop();
    if (previous) show(previous, false);
    report('ready', `${current.title}: ${current.caption}`);
  }

  function reset() {
    stop(false);
    history = [];
    show(tour.start, false);
    report('ready', 'The real OpenXplorer app with fictional sample files. Click a control in the picture.');
  }

  function stop(announce) {
    clearTimeout(timer);
    if (!playing) return;
    playing = false;
    for (const control of layer.querySelectorAll('.highlighted')) control.classList.remove('highlighted');
    if (announce) report('stopped', 'Tour stopped. Click a control in the picture to go on.');
  }

  function play() {
    stop(false);
    history = [];
    show(PLAY[0][0], false);
    playing = true;
    report('playing', 'Playing the tour…');
    const pause = reducedMotion.matches ? 1600 : 1300;
    let step = 0;
    const next = () => {
      if (!playing) return;
      if (step === PLAY.length) {
        playing = false;
        report('complete', 'Tour complete: every picture is the real app. Click a control to explore.');
        return;
      }
      const [from, to] = PLAY[step];
      if (current.id !== from) show(from);
      const control = [...layer.children].find(child => child.dataset.goesTo === to);
      control?.classList.add('highlighted');
      report('playing', control ? control.getAttribute('aria-label') + '…' : 'Playing the tour…');
      timer = setTimeout(() => { show(to); step += 1; timer = setTimeout(next, pause); }, pause);
    };
    timer = setTimeout(next, 300);
  }

  function setTheme(value) {
    theme = value === 'dark' ? 'dark' : 'light';
    if (current) show(current.id, false);
  }

  back.addEventListener('click', goBack);
  document.addEventListener('keydown', event => {
    if ((event.key === 'Backspace' || (event.altKey && event.key === 'ArrowLeft')) && !back.disabled) {
      event.preventDefault();
      goBack();
    } else if (event.key === 'Escape') {
      stop(true);
    }
  });
  window.addEventListener('message', event => {
    if (!embedded || event.source !== window.parent) return;
    const {channel, command, value} = event.data || {};
    if (channel !== 'openxplorer-demo-command') return;
    if (command === 'status') report(playing ? 'playing' : 'ready', current ? `${current.title}: ${current.caption}` : '');
    else if (command === 'scene') { stop(false); history = []; if (!show(value, false)) show(tour.start, false); report('ready', `${current.title}: ${current.caption}`); }
    else if (command === 'theme') { setTheme(value); if (!playing) report('ready', `${current.title}: ${current.caption}`); }
    else if (command === 'reset') reset();
    else if (command === 'play') play();
    else if (command === 'stop') stop(true);
  });
  show(params.get('scene') && scenes.has(params.get('scene')) ? params.get('scene') : tour.start, false);
})();
