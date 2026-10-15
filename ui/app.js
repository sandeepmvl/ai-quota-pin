const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
let data = [], hidden = [], compact = false, last = '', lastC = null;
const $ = (id) => document.getElementById(id);
const esc = (s) => String(s ?? '').replace(/[&<>"]/g, (c) => `&#${c.charCodeAt(0)};`);

function left(ts) {
  if (!ts) return '';
  let s = ts - Date.now() / 1000; if (s <= 0) return 'now';
  const d = Math.floor(s / 86400), h = Math.floor(s % 86400 / 3600), m = Math.floor(s % 3600 / 60);
  return d ? `${d}d${h}h` : h ? `${h}h${m}m` : `${m}m`;
}
function at(ts) {
  const d = new Date(ts * 1000), soon = ts - Date.now() / 1000 < 86400;
  return d.toLocaleString([], soon ? { hour: 'numeric', minute: '2-digit' } : { weekday: 'short', hour: 'numeric' });
}
function render() {
  const html = data.filter((u) => !hidden.includes(u.id)).map((u) => {
    const ws = u.windows.map((w) => {
      const p = Math.round(w.used ?? 0), cls = p >= 90 ? 'crit' : p >= 70 ? 'warn' : '';
      return `<div class="w"><span class="lbl">${esc(w.label)}</span>${w.used == null ? `<span class="det">${esc(w.detail)}</span>` : `<span class="bar ${cls}"><i style="width:${Math.min(p, 100)}%"></i></span><span class="pct">${p}%</span>`}<span class="rs">${w.resets_at ? `↻${left(w.resets_at)} · ${at(w.resets_at)}` : ''}</span></div>`;
    }).join('');
    const sum = u.windows.map((w) => w.used == null ? `${esc(w.label)} ${esc(w.detail)}` : `${esc(w.label)} ${Math.round(w.used)}%`).join(' · ');
    const plan = u.plan ? ` <span class="plan">${esc(u.plan)}${u.renews_at ? ` · renews ${new Date(u.renews_at * 1000).toLocaleDateString([], { month: 'short', day: 'numeric' })}` : ''}</span>` : '';
    const note = u.note ? ` <span class="note">${esc(u.note)}</span>` : '';
    const hint = u.status === 'signin' ? `<div class="hint">${esc(u.hint).replace(/`([^`]+)`/g, '<code>$1</code>')} <button class="x" data-hide="${esc(u.id)}" title="Hide this provider">hide</button></div>` : '';
    return `<div class="p ${u.status === 'na' || u.status === 'signin' ? 'na' : ''}" data-tauri-drag-region><span class="pn">${esc(u.name)}</span>${plan} ${u.windows.length ? `<span class="sum">${sum}</span>` : ''}${u.status === 'na' || u.status === 'signin' || !compact ? note : ''}${hint}${ws}</div>`;
  }).join('') || '<div class="na">No AI tools detected</div>';
  if (html === last && compact === lastC && !document.body.classList.contains('collapsed')) return; // nothing changed: skip DOM + resize
  last = html; lastC = compact; $('rows').innerHTML = html;
  document.body.classList.toggle('compact', compact);
  requestAnimationFrame(fit);
}
function fit() {
  const el = document.body.classList.contains('collapsed') ? $('pin') : $('note');
  const r = el.getBoundingClientRect();
  invoke('fit', { width: Math.ceil(r.width) + 2, height: Math.ceil(r.height) + 2 });
}
function setCollapsed(c) { document.body.classList.toggle('collapsed', c); invoke('set_mode', { collapsed: c }); requestAnimationFrame(fit); }
function setCompact(c) { compact = c; invoke('set_mode', { compact: c }); render(); }

$('hide').onclick = () => setCollapsed(true);
// Collapsed pin: click to expand, drag (>4px) to move the window anywhere.
const win = window.__TAURI__.window.getCurrentWindow();
$('pin').onmousedown = (e) => {
  const x0 = e.screenX, y0 = e.screenY;
  const move = (m) => { if (Math.abs(m.screenX - x0) + Math.abs(m.screenY - y0) > 4) { done(); win.startDragging(); } };
  const up = () => { done(); setCollapsed(false); };
  const done = () => { removeEventListener('mousemove', move); removeEventListener('mouseup', up); };
  addEventListener('mousemove', move); addEventListener('mouseup', up);
};
$('mode').onclick = () => setCompact(!compact);
$('refresh').onclick = () => invoke('refresh');
listen('usage', (e) => { data = e.payload; render(); });
$('rows').onclick = (e) => { const id = e.target.dataset?.hide; if (id) { hidden.push(id); invoke('set_mode', { hide: id }); render(); } };
listen('prefs', (e) => { hidden = e.payload.hidden; render(); });
listen('toggle-compact', () => setCompact(!compact));
setInterval(render, 60_000); // countdowns only; no polling from the UI
invoke('get_state').then(([u, p]) => { data = u; compact = p.compact; hidden = p.hidden || []; document.body.classList.toggle('collapsed', p.collapsed); render(); });
