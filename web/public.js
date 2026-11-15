"use strict";
const $ = (id) => document.getElementById(id);
const mineKey = "facet-my-requests";
const mine = () => { try { return JSON.parse(localStorage.getItem(mineKey) || "[]"); } catch { return []; } };
let last = null;

const map = new SkyMap($("sky"), (ra, dec) => {
  $("target").value = ""; $("ra").value = ra.toFixed(3); $("dec").value = dec.toFixed(3); estimate();
});
fetch("/api/stars").then(r => r.json()).then(s => map.setStars(s)).catch(() => {});

function stat(k, v) { return el("div", { class: "stat" }, el("div", { class: "k" }, k), el("div", { class: "v" }, v)); }

function estimate() {
  if (!last) return;
  const t = last.telescope, n = Math.max(1, +$("tel").value || 1), e = Math.max(1, +$("exp").value || 1);
  const lim = t.base_lim_mag + 1.25 * Math.log10(n) + 1.25 * Math.log10(e / t.base_exp_s);
  $("estimate").textContent =
    `Stacking ${n} × ${t.aperture_cm} cm telescopes for ${fmtDur(e)} each ≈ the light grasp of a ` +
    `${(t.aperture_cm * Math.sqrt(n) / 100).toFixed(2)} m aperture; estimated 5σ depth V ≈ ${lim.toFixed(1)} ` +
    `(soft estimate). Field of view ${t.fov_deg}° per telescope.`;
}

function render(s) {
  last = s;
  $("clock").textContent = `${s.utc} · ${s.playing ? (s.rate === 1 ? "real time" : `${s.rate}× speed`) : "paused"}`;
  map.setStatus(s);
  const st = $("stats"); st.replaceChildren(
    stat("Satellites", s.shell.n.toLocaleString()),
    stat("Orbit", `${s.shell.alt_km.toFixed(0)} km · ${s.shell.inc_deg}°`),
    stat("Telescopes busy", `${s.stats.active.toLocaleString()}`),
    stat("In sunlight", `${s.stats.lit.toLocaleString()}`),
    stat("Sky reachable now", `${s.stats.sky_reachable}%`),
    stat("Mode", s.mode_label),
  );
  const q = $("queue"); q.replaceChildren();
  const open = s.requests.filter(r => ["pending", "approved", "observing"].includes(r.state));
  const recent = s.requests.filter(r => r.state === "done").slice(0, 10);
  for (const r of [...open, ...recent].slice(0, 60)) {
    q.append(el("tr", { onmouseenter: () => { map.highlight = r.id; map.draw(); } },
      el("td", {}, r.id), el("td", {}, r.submitter), el("td", {}, r.target_name),
      el("td", { class: "num" }, r.telescopes), el("td", { class: "num" }, fmtDur(r.exposure_s)),
      stateCell(r), progressCell(r)));
  }
  if (!q.children.length) q.append(el("tr", {}, el("td", { class: "muted small", colspan: 7 }, "The queue is empty. Be the first!")));
  const ids = mine(), m = $("mine"); m.replaceChildren();
  for (const r of s.requests.filter(r => ids.includes(r.id))) {
    m.append(el("tr", {}, el("td", {}, `#${r.id} ${r.target_name}`), stateCell(r), progressCell(r)));
  }
  if (!m.children.length) m.append(el("tr", {}, el("td", { class: "muted small" }, "None yet from this browser.")));
  estimate();
}

async function poll() {
  try { const r = await fetch("/api/status"); if (r.ok) render(await r.json()); }
  catch { $("clock").textContent = "simulator offline"; }
}
setInterval(poll, 1000); poll();
["tel", "exp"].forEach(id => $(id).addEventListener("input", estimate));

$("form").addEventListener("submit", async (ev) => {
  ev.preventDefault();
  const body = { submitter: $("who").value, telescopes: +$("tel").value, exposure_s: +$("exp").value, note: $("note").value };
  if ($("target").value.trim()) body.target = $("target").value.trim();
  else if ($("ra").value !== "" && $("dec").value !== "") { body.ra_deg = +$("ra").value; body.dec_deg = +$("dec").value; }
  const msg = $("msg");
  try {
    const r = await fetch("/api/request", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) });
    const j = await r.json();
    if (!r.ok) { msg.className = "msg small warn"; msg.textContent = j.error || "rejected"; return; }
    const ids = mine(); ids.push(j.id); localStorage.setItem(mineKey, JSON.stringify(ids.slice(-50)));
    msg.className = "msg small"; msg.textContent = `Request #${j.id} submitted (${j.state}).`;
    poll();
  } catch { msg.className = "msg small warn"; msg.textContent = "Could not reach the simulator."; }
});
