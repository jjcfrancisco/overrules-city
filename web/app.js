import init, { City } from './pkg/overrules_city.js';

// ─── helpers ────────────────────────────────────────────────────────────────

const $ = (id) => document.getElementById(id);
const esc = (s) => String(s ?? '').replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));
const up = (s) => esc(String(s ?? '').toUpperCase());
const fmtNum = (n) => Math.round(n).toLocaleString('en-US');
const emptyFC = { type: 'FeatureCollection', features: [] };
const line = (coords, props = {}) => ({ type: 'Feature', properties: props, geometry: { type: 'LineString', coordinates: coords } });
const point = (c, props = {}) => ({ type: 'Feature', properties: props, geometry: { type: 'Point', coordinates: c } });
const css = (v) => getComputedStyle(document.documentElement).getPropertyValue(v).trim();

const VEHICLES = ['car', 'van', 'lorry', 'bicycle', 'foot'];
const NOUN = { car: 'CAR', van: 'VAN', lorry: 'LORRY', bicycle: 'BIKE', foot: 'ON FOOT' };
const OUTCOME = { allowed: 'ALLOWED', designated: 'DESIGNATED', denied: 'NO ENTRY', none: 'NO RULE SAYS' };
const START = '2026-10-19T08:00';

let city;
let speed = 5;
let theme = document.documentElement.dataset.theme === 'night' ? 'night' : 'day';
let mode = 'tour';          // tour | explore | about
let agentsBuf = [];
let lastFeed = 0;
let pulses = [];
let feedItems = [];         // newest last, for the attract scene
let road = null;            // the road open in explore: { id, ll, cell }
let claim = '';
let followAgent = null;
let focusFeatures = [];

// ─── the map, recoloured to the page ────────────────────────────────────────

const map = new maplibregl.Map({
  container: 'map',
  style: 'https://tiles.openfreemap.org/styles/positron',
  center: [-73.997, 40.7175],
  zoom: 14.2,
  pitch: 40,
  attributionControl: false,
});
map.addControl(new maplibregl.NavigationControl({ visualizePitch: true }), 'bottom-right');
const mapLoaded = new Promise((r) => map.on('load', r));

const PALETTE = {
  night: { bg: '#0b0b0d', water: '#111317', park: '#0f110f', land: '#0d0d0f', building: '#131315', bline: '#1d1c1b', minor: '#1f1e1c', major: '#2b2a28', motorway: '#3a3836', rail: '#1a1918', label: '#6d6a65', halo: '#0b0b0d' },
  day: { bg: '#f6f4ee', water: '#e4e1d7', park: '#eeece2', land: '#f1efe8', building: '#ebe8df', bline: '#dcd8cc', minor: '#e3dfd4', major: '#d6d2c6', motorway: '#c3bfb2', rail: '#dedad0', label: '#8a8680', halo: '#f6f4ee' },
};
const OWN = new Set(['roads-hit', 'signs-timed', 'signs-unsure', 'signs-closed', 'signs-oneway', 'signs-noturn', 'focus', 'focus-part', 'focus-pts', 'focus-lbl', 'pulses', 'agents']);

function paintBase() {
  const p = PALETTE[theme];
  for (const l of map.getStyle().layers) {
    if (OWN.has(l.id)) continue;
    const id = l.id;
    const set = (k, v) => { try { map.setPaintProperty(id, k, v); } catch { /* not this type */ } };
    const hide = () => map.setLayoutProperty(id, 'visibility', 'none');
    if (l.type === 'background') set('background-color', p.bg);
    else if (l.type === 'fill') {
      if (id === 'water') set('fill-color', p.water);
      else if (id === 'building') { set('fill-color', p.building); set('fill-outline-color', p.bline); }
      else if (id.startsWith('aeroway')) hide();
      else if (id === 'park') set('fill-color', p.park);
      else set('fill-color', p.land);
      set('fill-opacity', 1);
    } else if (l.type === 'line') {
      if (id.startsWith('boundary') || id.startsWith('aeroway')) hide();
      else if (id === 'waterway') set('line-color', p.water);
      else if (id.startsWith('railway')) set('line-color', p.rail);
      else if (id.includes('motorway')) set('line-color', id.includes('casing') ? p.bg : p.motorway);
      else if (id.includes('major')) set('line-color', id.includes('casing') ? p.bg : p.major);
      else set('line-color', p.minor);
    } else if (l.type === 'symbol') {
      if (id.includes('shield') || id === 'airport') hide();
      set('text-color', p.label);
      set('text-halo-color', p.halo);
      set('text-halo-width', 1.2);
      set('icon-opacity', 0);
    } else if (l.type === 'raster') hide();
  }
}

function chevron(color) {
  const s = 24;
  const c = document.createElement('canvas');
  c.width = c.height = s;
  const g = c.getContext('2d');
  g.strokeStyle = color;
  g.lineWidth = 1.5;
  g.beginPath();
  g.moveTo(9, 7); g.lineTo(15, 12); g.lineTo(9, 17);
  g.stroke();
  return g.getImageData(0, 0, s, s);
}

function paintOwn() {
  const ink = css('--z-ink'), mute = css('--z-mute'), accent = css('--z-accent'), cmp = css('--z-cmp');
  const img = chevron(mute);
  if (map.hasImage('chev')) map.updateImage('chev', img); else map.addImage('chev', img, { pixelRatio: 2 });
  map.setPaintProperty('signs-closed', 'line-color', accent);
  map.setPaintProperty('signs-timed', 'line-color', cmp);
  map.setPaintProperty('signs-unsure', 'line-color', cmp);
  map.setPaintProperty('signs-noturn', 'text-color', accent);
  map.setPaintProperty('signs-noturn', 'text-halo-color', PALETTE[theme].bg);
  map.setPaintProperty('focus', 'line-color', accent);
  map.setPaintProperty('focus-part', 'line-color', accent);
  map.setPaintProperty('focus-pts', 'circle-stroke-color', ['case', ['==', ['get', 'kind'], 'flat'], mute, accent]);
  map.setPaintProperty('focus-lbl', 'text-color', ['case', ['==', ['get', 'kind'], 'flat'], mute, accent]);
  map.setPaintProperty('focus-lbl', 'text-halo-color', PALETTE[theme].bg);
  map.setPaintProperty('pulses', 'circle-stroke-color', accent);
  map.setPaintProperty('agents', 'circle-color', ['match', ['get', 'v'], 0, css('--v-car'), 1, css('--v-van'), 2, css('--v-lorry'), 3, css('--v-bicycle'), css('--v-foot')]);
  map.setPaintProperty('agents', 'circle-stroke-color', PALETTE[theme].bg);
  void ink;
}

function setTheme(t) {
  theme = t;
  document.documentElement.dataset.theme = t;
  document.querySelectorAll('.theme button').forEach((b) => b.classList.toggle('active', b.dataset.theme === t));
  paintBase();
  paintOwn();
}

function addLayers(roads) {
  for (const id of ['roads', 'signs', 'agents', 'pulses', 'focus']) map.addSource(id, { type: 'geojson', data: id === 'roads' ? roads : emptyFC });
  const z = (a, b) => ['interpolate', ['linear'], ['zoom'], 13, a, 18, b];
  map.addLayer({ id: 'roads-hit', type: 'line', source: 'roads', paint: { 'line-width': 14, 'line-opacity': 0 } });
  map.addLayer({ id: 'signs-timed', type: 'line', source: 'signs', filter: ['==', ['get', 'timed'], true], paint: { 'line-width': z(1, 3), 'line-opacity': 0.8 } });
  map.addLayer({ id: 'signs-unsure', type: 'line', source: 'signs', filter: ['==', ['get', 'kind'], 'unsure'], paint: { 'line-width': z(1, 3), 'line-dasharray': [0.5, 1.5] } });
  map.addLayer({ id: 'signs-closed', type: 'line', source: 'signs', filter: ['==', ['get', 'kind'], 'closed'], paint: { 'line-width': z(0.8, 1.6), 'line-dasharray': [3, 2] } });
  map.addLayer({
    id: 'signs-oneway', type: 'symbol', source: 'signs', minzoom: 15.2, filter: ['==', ['get', 'kind'], 'oneway'],
    layout: { 'symbol-placement': 'line', 'symbol-spacing': 60, 'icon-image': 'chev', 'icon-size': z(0.6, 1), 'icon-rotation-alignment': 'map', 'icon-allow-overlap': true },
  });
  map.addLayer({ id: 'focus', type: 'line', source: 'focus', filter: ['==', ['get', 'kind'], 'road'], layout: { 'line-cap': 'round' }, paint: { 'line-width': z(2, 4) } });
  map.addLayer({ id: 'focus-part', type: 'line', source: 'focus', filter: ['==', ['get', 'kind'], 'part'], layout: { 'line-cap': 'round' }, paint: { 'line-width': z(4, 9), 'line-opacity': 0.55 } });
  map.addLayer({
    id: 'signs-noturn', type: 'symbol', source: 'signs', minzoom: 15.5, filter: ['==', ['get', 'kind'], 'noturn'],
    layout: { 'text-field': '×', 'text-font': ['Noto Sans Bold'], 'text-size': z(12, 20), 'text-allow-overlap': true },
    paint: { 'text-halo-width': 1.5, 'text-opacity': ['case', ['get', 'sure'], 1, 0.5] },
  });
  map.addLayer({ id: 'pulses', type: 'circle', source: 'pulses', paint: { 'circle-radius': ['get', 'r'], 'circle-color': 'rgba(0,0,0,0)', 'circle-stroke-width': 1, 'circle-stroke-opacity': ['get', 'o'] } });
  map.addLayer({
    id: 'agents', type: 'circle', source: 'agents',
    paint: {
      'circle-radius': ['interpolate', ['linear'], ['zoom'], 13, ['case', ['==', ['get', 'v'], 4], 1, 1.6], 17, ['case', ['==', ['get', 'v'], 4], 2.5, ['==', ['get', 'v'], 2], 5, 4], 19, ['case', ['==', ['get', 'v'], 4], 4, 8]],
      'circle-stroke-width': ['interpolate', ['linear'], ['zoom'], 14, 0, 17, 1],
    },
  });
  map.addLayer({ id: 'focus-pts', type: 'circle', source: 'focus', filter: ['==', ['geometry-type'], 'Point'], paint: { 'circle-radius': 7, 'circle-color': 'rgba(0,0,0,0)', 'circle-stroke-width': 1.5 } });
  map.addLayer({
    id: 'focus-lbl', type: 'symbol', source: 'focus', filter: ['==', ['geometry-type'], 'Point'],
    layout: { 'text-field': ['get', 'label'], 'text-font': ['Noto Sans Bold'], 'text-size': 11, 'text-anchor': ['case', ['==', ['get', 'kind'], 'flat'], 'top', 'bottom'], 'text-radial-offset': 1.1, 'text-letter-spacing': 0.08, 'text-allow-overlap': true },
    paint: { 'text-halo-width': 2 },
  });

  map.on('mouseenter', 'agents', () => (map.getCanvas().style.cursor = 'pointer'));
  map.on('mouseleave', 'agents', () => (map.getCanvas().style.cursor = ''));
  map.on('click', (e) => {
    const box = [[e.point.x - 6, e.point.y - 6], [e.point.x + 6, e.point.y + 6]];
    const a = map.queryRenderedFeatures(box, { layers: ['agents'] })[0];
    if (a) { setMode('explore'); return showAgent(a.properties.i); }
    const r = map.queryRenderedFeatures(e.point, { layers: ['roads-hit'] })[0];
    if (r) { setMode('explore'); showRoad(r.properties.id, [e.lngLat.lng, e.lngLat.lat]); }
  });
  map.on('dragstart', () => { followAgent = null; });
}

// ─── the live city ──────────────────────────────────────────────────────────

let last = performance.now();
let acc = 0;
function loop(now) {
  const dt = Math.min(0.1, (now - last) / 1000);
  last = now;
  if (speed > 0) city.tick(dt * speed);
  agentsBuf = city.agents();
  const feats = new Array(agentsBuf.length / 4);
  for (let i = 0, k = 0; i < agentsBuf.length; i += 4, k++) {
    feats[k] = { type: 'Feature', properties: { i: k, v: agentsBuf[i + 2] }, geometry: { type: 'Point', coordinates: [agentsBuf[i], agentsBuf[i + 1]] } };
  }
  map.getSource('agents').setData({ type: 'FeatureCollection', features: feats });
  if (followAgent !== null) {
    const k = followAgent * 4;
    map.jumpTo({ center: [agentsBuf[k], agentsBuf[k + 1]] });
  }
  pulses = pulses.filter((p) => now - p.t0 < 1800);
  map.getSource('pulses').setData({ type: 'FeatureCollection', features: pulses.map((p) => { const k = (now - p.t0) / 1800; return point(p.c, { r: 3 + 22 * k, o: 1 - k }); }) });
  acc += dt;
  if (acc > 0.25) { slowTick(acc); acc = 0; }
  requestAnimationFrame(loop);
}

let lastRules = 0;
let rateHist = [];
let signsBucket = '';
let rate = 0;
function slowTick(elapsed) {
  const f = JSON.parse(city.frame());
  $('clock-time').textContent = f.time;
  $('clock-day').textContent = f.day.slice(0, 3).toUpperCase();
  rateHist.push((f.rules - lastRules) / elapsed);
  lastRules = f.rules;
  if (rateHist.length > 8) rateHist.shift();
  rate = rateHist.reduce((a, b) => a + b, 0) / rateHist.length;
  $('m-rate').textContent = fmtNum(rate);
  $('m-decisions').textContent = fmtNum(f.decisions);
  $('m-refusals').textContent = fmtNum(f.refusals);
  const bucket = `${f.day}${f.time.slice(0, 3)}${Math.floor(Number(f.time.slice(3)) / 5)}`;
  if (bucket !== signsBucket) { signsBucket = bucket; refreshSigns(); }
  pollFeed();
  if (followAgent !== null && mode === 'explore' && $('left').dataset.agent == followAgent) showAgent(followAgent, false);
}

function refreshSigns() {
  map.getSource('signs').setData(JSON.parse(city.signs()));
  if (mode === 'explore' && road) showRoad(road.id, road.ll, road.cell, false);
}

function pollFeed() {
  const items = JSON.parse(city.feed(lastFeed));
  if (!items.length) return;
  lastFeed = items[items.length - 1].seq;
  feedItems.push(...items);
  feedItems = feedItems.slice(-40);
  const ul = $('feed');
  for (const it of items.slice(-3)) {
    const rule = it.details.find((d) => /rule #/.test(d)) ?? '';
    const li = document.createElement('li');
    li.innerHTML = `<div class="meta">${esc(it.clock.slice(4, 9))} · ${NOUN[VEHICLES[agentKind(it.agent)]] ?? ''} #${it.agent} · <span class="by ${it.by}">${it.by === 'game' ? 'GAME RULE' : 'OVERRULES'}</span></div>
      <div class="what">${esc(it.text.replace(/^skipped /, 'Skipped '))}</div>${rule ? `<div class="rule">${esc(rule)}</div>` : ''}`;
    li.addEventListener('click', () => { setMode('explore'); map.flyTo({ center: [it.lon, it.lat], zoom: 17.5, duration: 900 }); showAgent(it.agent); });
    ul.prepend(li);
    pulses.push({ c: [it.lon, it.lat], t0: performance.now() });
  }
  while (ul.children.length > 30) ul.lastChild.remove();
}

const agentKind = (i) => agentsBuf[i * 4 + 2] ?? 0;

// ─── what overrules said, rule by rule ──────────────────────────────────────

/** One answer from `ask` or `inspect`, as rows: every rule, which decides, and the result. */
function answerHtml(a, opts = {}) {
  const acc = a.access;
  const rows = acc.rules.map((x) => {
    const state = x.decided ? 'decides' : x.overridden ? 'overridden' : x.verdict;
    const label = x.decided ? 'DECIDES' : x.overridden ? 'MATCHED · LATER RULE WINS' : x.verdict === 'match' ? 'MATCHES' : x.verdict === 'undetermined' ? "CAN'T TELL" : "DOESN'T APPLY";
    const why = (opts.why ? x.scopes : x.scopes.filter((c) => c.because.includes('(read "'))).map((c) => {
      const hit = c.because.includes('(read "') || (opts.why && c.verdict !== 'no_match');
      return `<div class="why ${hit ? 'hit' : ''}">${{ match: '✓', no_match: '✗', undetermined: '?' }[c.verdict]} ${esc(c.scope)} ${esc(c.requires)}: ${esc(c.because)}</div>`;
    }).join('');
    return `<div class="r ${state}"><span class="k">RULE #${x.index}</span><span class="t">${esc(x.plain)}</span><span class="v">${label}</span>${why}</div>`;
  });
  const decided = acc.rules.find((x) => x.decided);
  const unsure = !acc.settled;
  const cls = unsure ? 'unsure' : acc.outcome === 'denied' ? 'no' : '';
  const result = unsure ? "CAN'T BE SURE" : OUTCOME[acc.outcome];
  const note = unsure
    ? `Rule ${acc.could_override.map((i) => '#' + i).join(', ')} needs a fact that wasn't given, so overrules won't guess.`
    : acc.outcome === 'none' ? 'No rule matched. That is not the same as allowed: overrules leaves defaults to the app.'
      : decided ? `Rule #${decided.index} of ${acc.rules.length} is the last one that matches.` : '';
  return `<div class="answer">
    <div class="r"><span class="k">ASKING</span><span class="t">${up(a.who.replace(/^\S+\s/, ''))} · ${up(a.when.replace(/(\d\d:\d\d):\d\d$/, '$1'))} · ${up(a.heading)}${opts.claim ? ` · ${up(opts.claim)}` : ''}</span><span class="v"></span></div>
    ${rows.join('')}
    <div class="final"><span class="k">ANSWER</span><span class="big ${cls}">${result}</span><span class="note">${esc(note)}</span></div>
  </div>`;
}

function turnsHtml(a) {
  const t = a.turns;
  return `<div class="answer">
    <div class="r"><span class="k">ASKING</span><span class="t">${up(a.who.replace(/^\S+\s/, ''))} · ${up(a.when.replace(/(\d\d:\d\d):\d\d$/, '$1'))} · ${up(a.heading)}</span><span class="v"></span></div>
    ${t.rules.map((x) => `<div class="r ${x.decided ? 'decides' : x.verdict}"><span class="k">TURN #${x.index}</span><span class="t">${esc(x.plain)}</span><span class="v">${x.decided ? 'IN FORCE' : x.verdict === 'undetermined' ? "CAN'T TELL" : "DOESN'T APPLY"}</span></div>`).join('')}
    <div class="final"><span class="k">ANSWER</span><span class="big no">${t.in_force.length} BANNED</span><span class="note">For banned turns every matching rule applies, not just the last one.</span></div>
  </div>`;
}

// ─── the tour: the city shows itself ────────────────────────────────────────

const ROW = { car: 0, van: 1, lorry: 2, motor: 3, bicycle: 4, foot: 5 };
const GREENWAY = ['f80e2915-6d4d-4b66-9955-44227cffe820', [-74.006626, 40.713599]];
const E14 = ['ae552ea3-6519-4484-8080-e40790cafba2', [-73.990584, 40.734722]];
const FDR = ['069c144d-c1b9-46d6-a24c-3f230e4346f5', [-73.998765, 40.708429]];
const BRIDGE = ['eae80a3f-2bd3-4119-a22c-c8692e77103e', [-73.992926, 40.703083]];
const CANAL = ['d19de29b-115e-494b-8643-38ad470833ce', [-73.996334, 40.716263]];
const RIVINGTON = ['2e39c957-e0a9-44ad-bbd1-4eee3c9411c5', [-73.989053, 40.720289]];

let sceneIdx = 0;
let playing = true;
let token = 0;
class Abort extends Error {}

function ctx(my) {
  return {
    async sleep(ms) {
      let t = 0;
      while (t < ms) {
        await new Promise((r) => setTimeout(r, 100));
        if (my !== token) throw new Abort();
        if (playing && mode === 'tour') { t += 100; setProgress(t / ms); }
      }
    },
    fly(center, zoom, bearing = map.getBearing(), pitch = 45, duration = 2200) {
      map.flyTo({ center, zoom, bearing, pitch, duration, essential: true });
    },
    time(t) { city.set_time(t); signsBucket = ''; },
    speed(s) { setSpeed(s); },
    focus(id, part) {
      const r = JSON.parse(city.road(id));
      const feats = [line(r.coords, { kind: 'road' })];
      if (part) feats.push(line(JSON.parse(city.slice(id, part[0], part[1])), { kind: 'part' }));
      setFocus(feats);
    },
    points(feats) { setFocus([...focusFeatures, ...feats]); },
    show(html) { $('answer').innerHTML = html; },
    ask(r, row, time, heading = 'forward', claimed = '') { return JSON.parse(city.ask(r[0], r[1][0], r[1][1], row, time, heading, claimed)); },
  };
}

const SCENES = [
  {
    kicker: 'THE CITY ASKS', title: ['EVERY JUNCTION.', 'EVERY WAY OUT.'],
    body: () => `Each dot is a car, van, lorry, bike or person crossing Lower Manhattan. At every junction it asks <b>overrules</b> about every way out: may I go there, how fast, is this turn banned? The yellow rings are the answers that said no.`,
    async run(c) {
      c.time(START); c.speed(5);
      setFocus([]);
      c.fly([-73.997, 40.7175], 14.3, -12, 40, 2500);
      c.show(`<div class="answer"><div class="r"><span class="k">ROADS</span><span class="t">11,846 from Overture, 5,217 access rules, 2,392 speed rules, 248 turn rules</span><span class="v"></span></div><div class="r"><span class="k">PEOPLE</span><span class="t">${agentsBuf.length / 4} travellers, each asking at every junction</span><span class="v"></span></div></div>`);
      await c.sleep(3000);
      map.easeTo({ bearing: 18, duration: 11000 });
      await c.sleep(11000);
    },
  },
  {
    kicker: 'LAST MATCH WINS · CITY HALL PARK GREENWAY', title: ['FIVE RULES.', 'THE LAST MATCH DECIDES.'],
    body: () => `The greenway lists five rules, in order. At 3&nbsp;a.m. a pedestrian matches the night closure and is turned away. A cyclist matches a closure too, but a later rule designates the path for bikes, and <b>the last rule that matches decides</b>.`,
    async run(c) {
      c.time('2026-10-25T03:00'); c.speed(1);
      c.focus(GREENWAY[0]);
      c.fly(GREENWAY[1], 17.6, 30, 50);
      c.show(answerHtml(c.ask(GREENWAY, ROW.foot, '2026-10-25T03:00')));
      await c.sleep(7000);
      c.show(answerHtml(c.ask(GREENWAY, ROW.bicycle, '2026-10-25T03:00')));
      await c.sleep(8000);
    },
  },
  {
    kicker: 'TIME · EAST 14TH STREET BUSWAY', title: ['SAME STREET.', 'DIFFERENT HOUR.'],
    body: () => `From 06:00 to 22:00 East 14th Street is closed to motor vehicles so buses can run. Watch a car ask the same question as the clock passes ten.`,
    async run(c) {
      c.time('2026-10-19T21:56'); c.speed(1);
      c.focus(E14[0]);
      c.fly(E14[1], 17.2, -60, 50);
      for (const m of ['21:57', '21:58', '21:59', '22:00', '22:01']) {
        c.show(answerHtml(c.ask(E14, ROW.car, `2026-10-19T${m}`)));
        await c.sleep(m === '21:59' || m === '22:00' ? 3200 : 1800);
      }
      await c.sleep(3500);
    },
  },
  {
    kicker: 'WHO COUNTS AS WHAT · FDR DRIVE', title: ['A CAR IS A MOTOR VEHICLE.', "A LORRY ISN'T A TRUCK."],
    body: () => `FDR Drive is designated for cars and closed to heavy goods vehicles. Overture's travel modes form a tree: car, truck and hgv sit side by side under motor vehicle. Ask as a van (a truck) and neither rule fits. Ask as <b>"some motor vehicle"</b> and overrules can't tell which rule you fall under.`,
    async run(c) {
      c.time(START); c.speed(5);
      c.focus(FDR[0]);
      c.fly(FDR[1], 16.4, 40, 45);
      for (const r of [ROW.car, ROW.van, ROW.lorry, ROW.motor]) {
        c.show(answerHtml(c.ask(FDR, r, '2026-10-19T08:05')));
        await c.sleep(r === ROW.motor ? 7000 : 4200);
      }
    },
  },
  {
    kicker: 'HONEST UNCERTAINTY · CITY HALL PARK GREENWAY', title: ['NO TIME GIVEN?', 'NO GUESS.'],
    body: () => `Leave the time out and the greenway's night closure can neither be applied nor ruled out. Instead of guessing, overrules says it <b>can't be sure</b> and names the rule that needs the missing fact.`,
    async run(c) {
      c.focus(GREENWAY[0]);
      c.fly(GREENWAY[1], 17.6, -20, 50);
      c.show(answerHtml(c.ask(GREENWAY, ROW.foot, '')));
      await c.sleep(8000);
      c.show(answerHtml(c.ask(GREENWAY, ROW.foot, '2026-10-19T08:05')));
      await c.sleep(5000);
    },
  },
  {
    kicker: 'VEHICLE SIZE · BROOKLYN BRIDGE', title: ['OVER 11 FT AND 3 TONS.', 'BOTH MUST HOLD.'],
    body: () => `The bridge turns away vehicles taller than 11 feet <b>and</b> heavier than 3 short tons. A 3.5-tonne van is heavy enough but only 8.9 feet tall, so it is let through. Feet, metres, short tons and tonnes are converted exactly.`,
    async run(c) {
      c.time(START); c.speed(5);
      c.focus(BRIDGE[0]);
      c.fly(BRIDGE[1], 16.2, -50, 50);
      c.show(answerHtml(c.ask(BRIDGE, ROW.van, '2026-10-19T08:05'), { why: true }));
      await c.sleep(8000);
      c.show(answerHtml(c.ask(BRIDGE, ROW.lorry, '2026-10-19T08:05'), { why: true }));
      await c.sleep(7000);
    },
  },
  {
    kicker: 'TURNS · CANAL STREET', title: ['NO TURN.', 'NOT EVEN IN TWO MOVES.'],
    body: () => `Canal Street bans one turn outright, and a two-step manoeuvre through the next junction as well. overrules checks both against the way you are travelling.`,
    async run(c) {
      c.focus(CANAL[0]);
      c.fly(CANAL[1], 17.4, 20, 50);
      c.show(turnsHtml(c.ask(CANAL, ROW.car, '2026-10-19T08:05', 'forward')));
      await c.sleep(9000);
    },
  },
  {
    kicker: 'MESSY DATA · RIVINGTON STREET', title: ['TYPOS IN THE DATA.', 'READ ANYWAY.'],
    body: () => `71 of New York's 99 distinct opening-hours strings are malformed, like <code>Mo-Su12:00-21:00</code> with no space. overrules repairs them on the fly and says so in its answer, here letting a car with local access in on a Saturday afternoon.`,
    async run(c) {
      c.time('2026-10-24T13:00'); c.speed(2);
      c.focus(RIVINGTON[0]);
      c.fly(RIVINGTON[1], 17.4, 60, 50);
      c.show(answerHtml(c.ask(RIVINGTON, ROW.car, '2026-10-24T13:00', 'backward', 'at_destination'), { claim: 'local access' }));
      await c.sleep(10000);
    },
  },
  {
    kicker: 'ON THE GLOBE', title: ['FLAT MAPS LIE.', 'BY ? METRES.'],
    body: () => `Overture measures positions along a road on the real, curved Earth. Treat longitude and latitude as flat x and y, as many tools do, and this rule's boundary lands somewhere else. overrules measures geodesically, as the schema says.`,
    async run(c) {
      const w = JSON.parse(city.worst_planar());
      $('scene-title').innerHTML = `FLAT MAPS LIE.<br><span class="accent">BY ${w.error_m.toFixed(0)} METRES.</span>`;
      c.focus(w.id, [0, w.fraction]);
      c.points([point(w.geodesic, { kind: 'true', label: 'OVERTURE MEANS HERE' }), point(w.planar, { kind: 'flat', label: 'FLAT MATHS PUTS IT HERE' })]);
      c.fly(w.geodesic, 18.6, 0, 30);
      c.show(`<div class="answer">
        <div class="r"><span class="k">ROAD</span><span class="t">${esc(w.name)}, ${Math.round(w.length_m)} m long</span><span class="v"></span></div>
        <div class="r"><span class="k">RULE</span><span class="t">${esc(w.rule)}</span><span class="v"></span></div>
        <div class="r"><span class="k">BOUNDARY</span><span class="t">${(w.fraction * 100).toFixed(1)}% along the road</span><span class="v"></span></div>
        <div class="final"><span class="k">FLAT MATHS</span><span class="big no">${w.error_m.toFixed(1)} M OFF</span><span class="note">The furthest any rule boundary in this area would move. Across all of New York, 159 move by 25 m or more.</span></div>
      </div>`);
      await c.sleep(11000);
    },
  },
  {
    kicker: 'AT SCALE', title: ['RULES BY THE THOUSAND.', 'IN YOUR BROWSER.'],
    body: () => `overrules is a Rust library, compiled here to WebAssembly. There is no server: every answer on this page, for every traveller at every junction, is worked out on your machine while the city moves.`,
    async run(c) {
      c.time(START); c.speed(20);
      setFocus([]);
      c.fly([-73.996, 40.7185], 13.9, 0, 30, 2500);
      for (let i = 0; i < 6; i++) {
        c.show(`<div class="answer"><div class="final"><span class="k">RIGHT NOW</span><span class="big">${fmtNum(rate)}</span><span class="note">rules evaluated per second, at 20× speed</span></div></div>`);
        await c.sleep(2000);
      }
    },
  },
  {
    kicker: 'LIVE · ON ITS OWN', title: ['NOBODY IS STEERING.', 'THE RULES ARE.'],
    body: () => `Here are the latest moves the rules refused, as they happen. Click anything on the map to take over.`,
    async run(c) {
      c.time(START); c.speed(2);
      setFocus([]);
      for (let i = 0; i < 4; i++) {
        const pick = [...feedItems].reverse().find((f) => f.details.some((d) => /during|vehicle|mode|turn rule/i.test(d))) ?? feedItems[feedItems.length - 1];
        if (pick) {
          c.fly([pick.lon, pick.lat], 17.6, map.getBearing() + 40, 50, 2000);
          const rule = pick.details.find((d) => /rule #/.test(d)) ?? '';
          c.show(`<div class="answer"><div class="r"><span class="k">${esc(pick.clock.slice(4, 9))}</span><span class="t">${NOUN[VEHICLES[agentKind(pick.agent)]]} #${pick.agent}: ${esc(pick.text.replace(/^skipped /, 'skipped '))}</span><span class="v ${pick.by === 'game' ? 'by game' : 'by overrules'}">${pick.by === 'game' ? 'GAME RULE' : 'OVERRULES'}</span></div>
            ${rule ? `<div class="r decides"><span class="k">WHY</span><span class="t">${esc(rule)}</span><span class="v"></span></div>` : ''}</div>`);
          feedItems = feedItems.filter((f) => f !== pick);
        }
        await c.sleep(5500);
      }
    },
  },
];

function setFocus(feats) {
  focusFeatures = feats;
  map.getSource('focus').setData({ type: 'FeatureCollection', features: feats });
}

function setProgress(f) { const i = $('progress-bar'); if (i) i.style.width = `${Math.min(100, f * 100)}%`; }

async function runScene(i) {
  const my = ++token;
  sceneIdx = (i + SCENES.length) % SCENES.length;
  const s = SCENES[sceneIdx];
  followAgent = null;
  $('left').dataset.agent = '';
  $('left-body').innerHTML = `
    <div class="kicker"><span class="accent">${String(sceneIdx + 1).padStart(2, '0')}</span><span class="dash">·</span><span>${s.kicker}</span></div>
    <h2 class="scene" id="scene-title">${s.title[0]}<br><span class="accent">${s.title[1]}</span></h2>
    <p>${s.body()}</p>
    <div id="answer"></div>
    <div class="progress"><i id="progress-bar"></i></div>`;
  $('tour-step').textContent = `${String(sceneIdx + 1).padStart(2, '0')} / ${String(SCENES.length).padStart(2, '0')}`;
  history.replaceState(null, '', `#${String(sceneIdx + 1).padStart(2, '0')}`);
  try {
    await s.run(ctx(my));
    if (my === token) runScene(sceneIdx + 1);
  } catch (e) {
    if (!(e instanceof Abort)) throw e;
  }
}

function setPlaying(p) {
  playing = p;
  $('play').textContent = p ? 'PAUSE' : 'PLAY';
}

// ─── explore: click anything ────────────────────────────────────────────────

function setMode(m) {
  if (m === mode && m !== 'tour') return;
  mode = m;
  document.querySelectorAll('.nav [data-mode]').forEach((b) => b.classList.toggle('active', b.dataset.mode === m));
  $('tour-controls').classList.toggle('hidden', m !== 'tour');
  if (m === 'tour') { setPlaying(true); runScene(sceneIdx); return; }
  token++;
  setFocus([]);
  if (m === 'explore') {
    $('left-body').innerHTML = `
      <div class="kicker"><span class="accent">EXPLORE</span><span class="dash">·</span><span>YOUR TURN</span></div>
      <h2 class="scene">CLICK ANY ROAD.<br><span class="accent">CLICK ANYONE.</span></h2>
      <p>Click a road to see every rule on it, which one decides, and what overrules would say for a car, van, lorry, bike or pedestrian at another time. Click a moving dot to see why it turned where it did.</p>
      <p>Change the clock at the top: time windows open and close, and the signs on the map follow.</p>`;
  } else if (m === 'about') about();
}

function showAgent(i, fresh = true) {
  const a = JSON.parse(city.agent(i));
  if (fresh) followAgent = null;
  const d = a.decision;
  $('left').dataset.agent = i;
  $('left-body').innerHTML = `
    <div class="kicker"><span class="accent">${NOUN[a.vehicle]}</span><span class="dash">·</span><span>TRAVELLER #${a.id} · ${a.decisions} JUNCTIONS SO FAR</span></div>
    <h3 class="road">ON ${up(a.road)}</h3>
    <div class="answer">
      <div class="r"><span class="k">HEADING</span><span class="t">${up(a.heading)} along the road as drawn</span><span class="v"></span></div>
      <div class="r"><span class="k">SPEED</span><span class="t">${Math.round(a.speed_kmh)} km/h · limit ${esc(a.limit)}</span><span class="v"></span></div>
    </div>
    <div class="row-actions"><button id="follow-agent">${followAgent === i ? 'FOLLOWING' : 'FOLLOW'}</button><span class="dash">·</span><button id="agent-road">RULES ON THIS ROAD</button></div>
    ${d ? `<div class="kicker">LAST JUNCTION · ${up(d.place)} · ${esc(d.clock.slice(4, 9))}</div>
      <div class="answer">${d.options.map((o) => `
        <div class="r ${o.chosen ? 'decides' : ''}"><span class="k">${up(o.turn)}</span><span class="t">${esc(o.name)}${o.headline ? ` · <span class="mute">${esc(o.headline)}</span>` : ''}</span><span class="v">${o.chosen ? 'TOOK IT' : o.status === 'ok' ? 'OK' : o.status === 'uncertain' ? "CAN'T TELL" : o.status === 'turn_banned' ? 'BANNED TURN' : 'NO ENTRY'}</span></div>`).join('')}
      </div>
      <p style="margin-top:12px">It checked every way out with overrules and picked a legal one at random, preferring straight on.</p>` : '<p>It hasn\'t reached a junction yet.</p>'}`;
  $('follow-agent').onclick = () => { followAgent = i; map.easeTo({ zoom: Math.max(map.getZoom(), 17.5) }); showAgent(i, false); };
  $('agent-road').onclick = () => { const k = i * 4; showRoad(a.segment_id, [agentsBuf[k], agentsBuf[k + 1]]); };
}

function showRoad(id, ll, cell = null, fresh = true) {
  let r, g;
  try {
    r = JSON.parse(city.inspect(id, ll[0], ll[1], cell ? cell[0] : -1, cell ? cell[1] : -1, claim));
    g = JSON.parse(city.what_if(id, ll[0], ll[1], claim));
  } catch { return; }
  if (fresh) followAgent = null;
  road = { id, ll, cell };
  $('left').dataset.agent = '';
  const rd = JSON.parse(city.road(id));
  setFocus([line(rd.coords, { kind: 'road' }), point(r.point, { kind: 'true', label: '' })]);
  const sym = (x) => (!x ? '' : !x.settled ? ['unsure', '?'] : x.outcome === 'denied' ? ['no', 'NO'] : x.outcome === 'none' ? ['none', '–'] : ['yes', 'YES']);
  const cellHtml = (c) => ['forward', 'backward'].map((h) => { const [k, t] = sym(c[h]); return `<span class="${k}">${h === 'forward' ? '→' : '←'}${t}</span>`; }).join(' ');
  const heading = (h) => {
    const view = { ...h, who: r.who, when: r.when, heading: h.heading };
    const game = h.game && h.game.game_rule ? `<p style="margin:8px 0 0"><span class="by game">GAME RULE</span> ${esc(h.game.headline)}. ${esc((h.game.note ?? '').replace(/^Game rule: /, ''))}</p>` : '';
    return `<div class="kicker" style="margin-top:18px">TRAVELLING ${up(h.heading)} ${h.heading === 'forward' ? '→' : '←'}</div>${answerHtml(view)}${game}`;
  };
  $('left-body').innerHTML = `
    <div class="kicker"><span class="accent">ROAD</span><span class="dash">·</span><span>${up(r.class)}${r.subclass ? ' / ' + up(r.subclass) : ''} · ${Math.round(r.length_m)} M · CHECKED ${Math.round(r.at * 100)}% ALONG</span></div>
    <h3 class="road">${up(r.name)}</h3>
    <div class="kicker" style="margin-top:14px">WHAT IF…</div>
    <div class="row-actions">EVERYONE CLAIMING
      <select id="claim">${[['', 'NOTHING'], ['to_deliver', 'DELIVERING'], ['at_destination', 'LOCAL ACCESS'], ['as_customer', 'CUSTOMER'], ['as_private', 'PRIVATE ACCESS'], ['as_permitted', 'A PERMIT']].map(([v, l]) => `<option value="${v}" ${v === claim ? 'selected' : ''}>${l}</option>`).join('')}</select>
    </div>
    <table class="whatif"><tr><th></th>${g.columns.map((c) => `<th>${up(c)}</th>`).join('')}</tr>
      ${g.rows.map((row, ri) => `<tr><th>${up(row.label.replace(/^\S+\s/, ''))}</th>${row.cells.map((c, ci) => `<td data-r="${ri}" data-c="${ci}" class="${cell && cell[0] === ri && cell[1] === ci ? 'sel' : ''}">${cellHtml(c)}</td>`).join('')}</tr>`).join('')}
    </table>
    <p style="font-size:12px">overrules' own answers. → and ← are the two directions along the road as drawn. ? means it can't be sure because a fact is missing. Click a cell for the reasons.</p>
    ${cell ? '<div class="row-actions"><button id="road-you">← BACK TO THE CITY’S OWN VIEW</button></div>' : ''}
    ${r.headings.map(heading).join('')}`;
  $('left-body').querySelectorAll('.whatif td').forEach((td) => td.addEventListener('click', () => showRoad(id, ll, [Number(td.dataset.r), Number(td.dataset.c)], false)));
  $('road-you')?.addEventListener('click', () => showRoad(id, ll, null, false));
  $('claim').addEventListener('change', (e) => { claim = e.target.value; showRoad(id, ll, cell, false); });
}

function about() {
  $('left-body').innerHTML = `
    <div class="kicker"><span class="accent">ABOUT</span><span class="dash">·</span><span>WHAT YOU ARE LOOKING AT</span></div>
    <h2 class="scene">THE DATA. THE LIBRARY.<br><span class="accent">THE CITY.</span></h2>
    <p>Every label on this page says who decided: <span class="by overrules">OVERRULES</span> reading Overture's data, or a <span class="by game">GAME RULE</span> the city adds where the data is silent.</p>
    <div class="about-list">
      <div class="r"><span class="k">OVERTURE DATA</span><span>The rules on each road, in order: who may enter, which way, when, how fast, which turns are banned. Real New York data from the 2026-09-23 release.</span></div>
      <div class="r"><span class="k">OVERRULES</span><span>Reads a road's rules for one traveller (direction, vehicle, size, time, purpose, permits) and answers <b>allowed, no entry, or no rule says</b>, plus the speed limit and banned turns. It says <b>which rule decided</b> and why each condition passed, and says <b>can't be sure</b> instead of guessing when a fact is missing.</span></div>
      <div class="r"><span class="k">GAME RULES</span><span>Things overrules deliberately leaves to the app: footways are for walkers; a one-way that names no mode doesn't stop people on foot; "designated for lorries" doesn't let them go the wrong way; a road that only lets in some people is closed to the rest; 25 mph where no limit is given.</span></div>
      <div class="r"><span class="k">THE CITY</span><span>About 600 travellers on 11,846 roads, each picking a legal way out at random at every junction. Speeds are the limit times a traffic factor.</span></div>
      <div class="r"><span class="k">RUNS ON</span><span>Your browser. overrules is Rust, compiled to WebAssembly; the map is MapLibre with OpenFreeMap tiles.</span></div>
    </div>`;
}

function setSpeed(s) {
  speed = s;
  document.querySelectorAll('#speeds button').forEach((b) => b.classList.toggle('active', Number(b.dataset.speed) === s));
}

// ─── wiring ─────────────────────────────────────────────────────────────────

document.querySelectorAll('#speeds button').forEach((b) => b.addEventListener('click', () => setSpeed(Number(b.dataset.speed))));
document.querySelectorAll('#presets button').forEach((b) => b.addEventListener('click', () => { city.set_time(b.dataset.time); signsBucket = ''; }));
document.querySelectorAll('.theme button').forEach((b) => b.addEventListener('click', () => {
  setTheme(b.dataset.theme);
  try { localStorage.setItem('theme', b.dataset.theme); } catch { /* private mode */ }
}));
document.querySelectorAll('.nav [data-mode]').forEach((b) => b.addEventListener('click', () => setMode(b.dataset.mode)));
$('prev').addEventListener('click', () => runScene(sceneIdx - 1));
$('next').addEventListener('click', () => runScene(sceneIdx + 1));
$('play').addEventListener('click', () => setPlaying(!playing));
$('viewer').addEventListener('change', (e) => { city.set_viewer(e.target.value); refreshSigns(); });
document.addEventListener('keydown', (e) => {
  if (e.target.tagName === 'SELECT') return;
  if (e.key === 'ArrowRight' && mode === 'tour') runScene(sceneIdx + 1);
  if (e.key === 'ArrowLeft' && mode === 'tour') runScene(sceneIdx - 1);
  if (e.key === ' ') { e.preventDefault(); if (mode === 'tour') setPlaying(!playing); else setSpeed(speed ? 0 : 5); }
  if (e.key === 'Escape' && mode !== 'tour') setMode('tour');
});

// ─── boot ───────────────────────────────────────────────────────────────────

/** The play area: gzipped GeoJSON, unpacked here unless the server already did. */
async function loadArea() {
  const bytes = new Uint8Array(await (await fetch('data/area.json.gz')).arrayBuffer());
  if (bytes[0] !== 0x1f || bytes[1] !== 0x8b) return new TextDecoder().decode(bytes);
  const stream = new Blob([bytes]).stream().pipeThrough(new DecompressionStream('gzip'));
  return new Response(stream).text();
}

const [, text] = await Promise.all([init(), loadArea()]);
city = new City(text, START, Math.floor(Math.random() * 2 ** 32));
const raw = JSON.parse(text);
const roads = { type: 'FeatureCollection', features: raw.features.map((f) => ({ type: 'Feature', geometry: f.geometry, properties: { id: f.properties.id } })) };
await mapLoaded;
addLayers(roads);
setTheme(theme);
refreshSigns();
requestAnimationFrame(loop);
// #03 opens the third scene, so scenes can be linked to.
const linked = Number(location.hash.slice(1));
runScene(linked >= 1 && linked <= SCENES.length ? linked - 1 : 0);
window.__city = { map, city, runScene, showRoad, showAgent, get scene() { return sceneIdx; } };
