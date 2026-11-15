"use strict";
const $ = (id) => document.getElementById(id);
const mineKey = "facet-my-requests";
const mine = () => { try { return JSON.parse(localStorage.getItem(mineKey) || "[]"); } catch { return []; } };
let last = null;
const viewer = new ResultViewer($("result"));
const streamViewer = new ResultViewer($("result"), "", "/api/campaign");
const viewBtn = (r) => r.has_result ? el("button", { class: "view", onclick: () => { streamViewer.close(); viewer.show(r.id); } }, r.state === "done" ? "Result" : "Live stack") : "";

const map = new SkyMap($("sky"), (ra, dec) => {
  $("target").value = ""; $("ra").value = ra.toFixed(3); $("dec").value = dec.toFixed(3); estimate();
});
fetch("/api/stars").then(r => r.json()).then(s => map.setStars(s)).catch(() => {});

function stat(k, v) { return el("div", { class: "stat" }, el("div", { class: "k" }, k), el("div", { class: "v" }, v)); }

function estimate() {
  if (!last) return;
  const t = last.telescope, n = Math.max(1, +$("tel").value || 1), e = Math.max(1, +$("exp").value || 1);
  // Background-limited scaling from the server's physical 1-telescope, 60 s limit.
  const lim = t.lim_1tel_60s + 1.25 * Math.log10(n * e / 60);
  $("estimate").textContent =
    `Stacking ${n} × ${t.aperture_cm} cm telescopes for ${fmtDur(e)} each ≈ the light grasp of a ` +
    `${(t.aperture_cm * Math.sqrt(n) / 100).toFixed(2)} m aperture; estimated 5σ depth V ≈ ${lim.toFixed(1)} ` +
    `(dark sky, soft estimate; the result page computes it exactly). Field of view ${t.fov_deg}° per telescope.`;
}

function renderDefence(s) {
  const d = s.defence, c = $("pd_card"); c.hidden = !d; if (!d) return;
  const [a, b] = d.systems, known = d.position_known || d.state === "impacted";
  $("pd_name").textContent = known ? d.designation : "Nothing detected yet";
  $("pd_count").textContent = d.state === "impacted" ? "impact has occurred" : known ? `predicted impact in ${fmtDur(d.t_to_impact_s)}` : "";
  const ip = (p) => p == null ? "–" : p >= 0.9999 ? "certain" : p >= 0.001 ? `${(p * 100).toFixed(1)} %` : "< 0.1 %";
  $("pd_text").textContent = `A practice scenario: a ${d.diameter_m.toFixed(0)} m asteroid on a collision course, approaching ` +
    (d.radiant_elong_deg < 60 ? "out of the daytime sky, where telescopes on the ground cannot look. " : "through the night sky. ") +
    "FACET searches for it, and once found, dozens of telescopes track it to pin down where it will strike. " +
    "Below: what FACET adds, compared with ground-based telescopes alone.";
  const t = (l, v) => el("div", { class: "tile" }, el("div", { class: "tv" }, v), el("div", { class: "tl" }, l));
  $("pd_tiles").replaceChildren(
    t("warning time with FACET", a.warning_s == null ? "–" : fmtDur(a.warning_s)),
    t("warning time, ground only", b.warning_s == null ? (d.state === "impacted" ? "none" : "–") : fmtDur(b.warning_s)),
    t("impact probability (FACET + ground)", ip(a.ip)),
    t("impact probability (ground only)", ip(b.ip)));
}

function renderWhy(s) {
  const g = s.ground, c = $("why_card"); c.hidden = !g; if (!g) return;
  const t = (l, v) => el("div", { class: "tile" }, el("div", { class: "tv" }, v), el("div", { class: "tl" }, l));
  $("why_text").textContent = `Compared with the same number of identical telescopes at ${g.sites} of the world's good observatory sites: on the ground, telescopes only work at night, in clear weather, through the air.`;
  $("why_tiles").replaceChildren(
    t("of the time FACET is observing", `${Math.round(100 * g.facet_duty)} %`),
    t("for a telescope on the ground", `${Math.round(100 * g.ground_duty)} %`),
    t("ground telescopes to match one FACET telescope", `≈ ${g.equivalent_ground_telescopes.toFixed(0)}`));
}

function renderScience(s) {
  const sc = s.science; if (!sc) return;
  const d = sc.per_day, n = (x) => Number(x).toLocaleString(undefined, { maximumFractionDigits: 0 });
  const tile = (l, v) => el("div", { class: "tile" }, el("div", { class: "tv" }, v), el("div", { class: "tl" }, l));
  $("sci_since").textContent = `· ${fmtDur(sc.elapsed_s)} simulated`;
  $("sci_tiles").replaceChildren(
    tile("observations completed per day", n(d.completed)),
    tile("finished in time to be useful", sc.on_time_pct === null ? "–" : `${sc.on_time_pct.toFixed(1)} %`),
    tile("of the time working telescopes are observing", `${(100 * sc.efficiency.integrating_pct / Math.max(1, 100 - sc.efficiency.hold_pct)).toFixed(0)} %`),
    tile("research teams served", `${sc.investigators_served}`));
  $("sci_cases").replaceChildren(...sc.cases.map(c => {
    const fin = c.completed + c.expired;
    return el("tr", {}, el("td", {}, c.label), el("td", { class: "num" }, n(c.completed)),
      el("td", { class: "num" }, fin ? `${(100 * c.on_time / fin).toFixed(0)} %` : "–"),
      el("td", { class: "num" }, c.median_response_s === null ? "–" : fmtDur(c.median_response_s)),
      el("td", { class: "num" }, c.median_depth_mag === null ? "–" : `mag ${c.median_depth_mag.toFixed(1)}`));
  }));
}

function render(s) {
  last = s;
  $("clock").textContent = `${s.utc} · ${s.playing ? (s.rate === 1 ? "real time" : `${s.rate}× speed`) : "paused"}`;
  map.setStatus(s);
  const st = $("stats"); st.replaceChildren(
    stat("Satellites", s.shell.n.toLocaleString()),
    stat("Orbit", `${s.shell.alt_km.toFixed(0)} km · ${s.shell.inc_deg}°`),
    stat("Locked on target", `${s.stats.locked.toLocaleString()}`),
    stat("Slewing / settling", `${s.stats.slewing} / ${s.stats.settling}`),
    stat("In sunlight", `${s.stats.lit.toLocaleString()}`),
    stat("Sky reachable now", `${s.stats.sky_reachable}%`),
    stat("Telescopes reserved for the public", `${Math.round(s.public_share * 100)} %`),
    stat("Mode", s.mode_label),
  );
  renderScience(s);
  renderDefence(s);
  renderWhy(s);
  const q = $("queue"); q.replaceChildren();
  const open = s.requests.filter(r => ["pending", "approved", "observing"].includes(r.state));
  const recent = s.requests.filter(r => r.state === "done").slice(0, 10);
  for (const r of [...open, ...recent].slice(0, 60)) {
    q.append(el("tr", { onmouseenter: () => { map.highlight = r.id; map.draw(); } },
      el("td", {}, r.id), el("td", {}, r.submitter), el("td", {}, r.target_name),
      el("td", { class: "num" }, r.telescopes), el("td", { class: "num" }, fmtDur(r.exposure_s)),
      stateCell(r), progressCell(r), el("td", {}, viewBtn(r))));
  }
  if (!q.children.length) q.append(el("tr", {}, el("td", { class: "muted small", colspan: 8 }, "The queue is empty. Be the first!")));
  const ids = mine(), m = $("mine"); m.replaceChildren();
  for (const r of s.requests.filter(r => ids.includes(r.id))) {
    m.append(el("tr", {}, el("td", {}, `#${r.id} ${r.target_name}`), stateCell(r), progressCell(r), el("td", {}, viewBtn(r))));
  }
  if (!m.children.length) m.append(el("tr", {}, el("td", { class: "muted small" }, "None yet from this browser.")));
  const streams = (s.campaigns || []).filter(c => c.active);
  $("streams").replaceChildren(...(streams.length ? streams.map(c => {
    const { any } = stareCoverage(c);
    return el("div", { class: "stare" },
      el("div", { class: "insphead" }, el("span", { class: `lamp ${c.paused || c.blocked ? "amber" : c.locked_now ? "green" : "grey"}` }),
        el("b", {}, c.target_name), el("span", { class: "muted small" }, c.paused ? " · paused" : ` · ${c.locked_now} telescopes watching · new frame every ${fmtDur(c.frame_s)}`)),
      el("div", { class: "small muted" }, c.paused ? `Paused: ${c.target_name} is too close to the ${String(c.blocked).startsWith("Sun") ? "Sun" : "Moon"} to observe safely. The stream resumes automatically.`
        : c.blocked ? c.blocked : any !== null ? `on target ${any.toFixed(2)} % of the time since ${c.started_utc.slice(0, 16)} UTC` : "acquiring…"),
      el("div", { class: "btns" }, c.has_frame ? el("button", { class: "view", onclick: () => { viewer.close(); streamViewer.show(c.id); } }, "Watch") : ""));
  }) : [el("div", { class: "muted small" }, "No live streams right now.")]));
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
// Shareable link: /?result=<id>
{ const rid = new URLSearchParams(location.search).get("result"); if (rid) viewer.show(+rid); }
// Shareable live-stream link: /?stream=<campaign id>
{ const sid = new URLSearchParams(location.search).get("stream"); if (sid) streamViewer.show(+sid); }
