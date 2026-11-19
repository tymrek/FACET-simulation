"use strict";
// FACET operations console. Talks only to the ops API: programme, targets,
// public share, request book and satellite safe-hold. It cannot change the
// simulated fleet, orbits, hardware or clock.

const $ = (id) => document.getElementById(id);
const token = new URLSearchParams(location.search).get("token") || "";
const H = { "X-Facet-Token": token };
const Q = `?token=${encodeURIComponent(token)}`;
let last = null, fleet = null, selected = null, coast = [];

function flash(text, bad) { const m = $("msg"); m.className = "msg small" + (bad ? " warn" : ""); m.textContent = text; }

async function cmd(body) {
  try {
    const r = await fetch("/api/cmd", { method: "POST", headers: { ...H, "Content-Type": "application/json" }, body: JSON.stringify(body) });
    const j = await r.json();
    if (!r.ok) flash(j.error || "command failed", true); else flash("");
    poll();
  } catch { flash("lost contact with the operations server", true); }
}

// ---------------------------------------------------------------- ground map
const PHASE_COL = ["#e9e2d0", "#5f73b4", "#ff9628", "#ffe150", "#5affa0", "#ff4b4b", "#7a2b2b"];
class GroundMap {
  constructor(canvas, onPick) {
    this.c = canvas; this.g = canvas.getContext("2d"); this.onPick = onPick;
    canvas.addEventListener("click", (e) => {
      if (!fleet) return;
      const [lon, lat] = this.inv(e.offsetX * this.dpr, e.offsetY * this.dpr);
      let best = -1, bd = 1e9;
      fleet.sats.forEach(([lo, la], i) => {
        let dl = Math.abs(lo - lon); if (dl > 180) dl = 360 - dl;
        const d = Math.hypot(dl * Math.cos(lat * Math.PI / 180), la - lat);
        if (d < bd) { bd = d; best = i; }
      });
      if (best >= 0 && bd < 6) this.onPick(best);
    });
    window.addEventListener("resize", () => this.draw());
  }
  layout() {
    const dpr = window.devicePixelRatio || 1; this.dpr = dpr;
    const w = this.c.clientWidth * dpr, h = this.c.clientHeight * dpr;
    if (this.c.width !== w || this.c.height !== h) { this.c.width = w; this.c.height = h; }
    this.w = w; this.h = h;
  }
  xy(lon, lat) { return [(lon + 180) / 360 * this.w, (90 - lat) / 180 * this.h]; }
  inv(x, y) { return [x / this.w * 360 - 180, 90 - y / this.h * 180]; }
  draw() {
    this.layout(); const g = this.g, { w, h } = this;
    g.fillStyle = "#061020"; g.fillRect(0, 0, w, h);
    // graticule
    g.strokeStyle = "rgba(90,120,170,0.18)"; g.lineWidth = this.dpr;
    for (let lon = -180; lon <= 180; lon += 30) { const [x] = this.xy(lon, 0); g.beginPath(); g.moveTo(x, 0); g.lineTo(x, h); g.stroke(); }
    for (let lat = -60; lat <= 60; lat += 30) { const [, y] = this.xy(0, lat); g.beginPath(); g.moveTo(0, y); g.lineTo(w, y); g.stroke(); }
    // coastlines
    g.strokeStyle = "rgba(150,200,160,0.65)"; g.lineWidth = 1 * this.dpr;
    for (const line of coast) {
      g.beginPath(); let prev = null;
      for (const [lo, la] of line) {
        const p = this.xy(lo, la);
        if (prev && Math.abs(p[0] - prev[0]) < w / 2) g.lineTo(p[0], p[1]); else g.moveTo(p[0], p[1]);
        prev = p;
      }
      g.stroke();
    }
    if (!fleet) return;
    // night side
    const [sl, sb] = fleet.subsolar, D = Math.PI / 180;
    const tb = Math.tan(sb * D) || 1e-6;
    g.fillStyle = "rgba(0,0,12,0.55)"; g.beginPath();
    const edge = sb > 0 ? -90 : 90;
    g.moveTo(...this.xy(-180, edge));
    for (let lon = -180; lon <= 180; lon += 2) {
      const lat = Math.atan(-Math.cos((lon - sl) * D) / tb) / D;
      g.lineTo(...this.xy(lon, lat));
    }
    g.lineTo(...this.xy(180, edge)); g.closePath(); g.fill();
    // subsolar point
    const [sx, sy] = this.xy(sl, sb);
    g.fillStyle = "#ffd25a"; g.beginPath(); g.arc(sx, sy, 5 * this.dpr, 0, 7); g.fill();
    // other operators and debris (thinned)
    if (fleet.traffic && fleet.traffic.length) {
      const TC = ["rgba(140,200,255,0.30)", "rgba(200,140,255,0.75)", "rgba(170,170,170,0.55)"];
      for (const [lo, la, p] of fleet.traffic) { const [x, y] = this.xy(lo, la); g.fillStyle = TC[p] || TC[2]; g.fillRect(x - this.dpr, y - this.dpr, 2 * this.dpr, 2 * this.dpr); }
    }
    // satellites
    const r = (fleet.sats.length > 3000 ? 1.2 : fleet.sats.length > 500 ? 1.7 : 2.4) * this.dpr;
    for (const code of [0, 1, 3, 2, 4, 5, 6]) {
      g.fillStyle = PHASE_COL[code];
      for (const [lo, la, c] of fleet.sats) if (c === code) { const [x, y] = this.xy(lo, la); g.fillRect(x - r, y - r, 2 * r, 2 * r); }
    }
    // Planetary defence: risk corridor and (after impact) the impact point
    const pd = last && last.defence;
    if (pd) {
      g.fillStyle = "rgba(255,70,70,0.55)";
      for (const [lo, la] of pd.corridor || []) { const [x, y] = this.xy(lo, la); g.beginPath(); g.arc(x, y, 2.2 * this.dpr, 0, 7); g.fill(); }
      if (pd.true_impact) {
        const [x, y] = this.xy(pd.true_impact[0], pd.true_impact[1]);
        g.strokeStyle = "#fff"; g.lineWidth = 2 * this.dpr; const r = 7 * this.dpr;
        g.beginPath(); g.moveTo(x - r, y - r); g.lineTo(x + r, y + r); g.moveTo(x + r, y - r); g.lineTo(x - r, y + r); g.stroke();
        g.fillStyle = "#fff"; g.font = `${11 * this.dpr}px system-ui`; g.fillText("impact", x + 9 * this.dpr, y - 6 * this.dpr);
      } else if ((pd.corridor || []).length) {
        const [lo, la] = pd.corridor[0]; const [x, y] = this.xy(lo, la);
        g.fillStyle = "#ff7a7a"; g.font = `${11 * this.dpr}px system-ui`; g.fillText("risk corridor", x + 8 * this.dpr, y - 8 * this.dpr);
      }
    }
    if (selected !== null && fleet.sats[selected]) {
      const [x, y] = this.xy(fleet.sats[selected][0], fleet.sats[selected][1]);
      g.strokeStyle = "#ffffff"; g.lineWidth = 1.5 * this.dpr;
      g.beginPath(); g.arc(x, y, 8 * this.dpr, 0, 7); g.stroke();
      g.fillStyle = "#fff"; g.font = `${11 * this.dpr}px system-ui`; g.fillText(`FACET-${String(selected + 1).padStart(5, "0")}`, x + 10 * this.dpr, y - 8 * this.dpr);
    }
  }
}

const ground = new GroundMap($("ground"), (i) => inspect(i));
let shiftDown = false;
window.addEventListener("keydown", e => { if (e.key === "Shift") shiftDown = true; });
window.addEventListener("keyup", e => { if (e.key === "Shift") shiftDown = false; });
const sky = new SkyMap($("sky"), (ra, dec) => {
  if (shiftDown) { $("st_target").value = ""; $("st_target").dataset.ra = ra; $("st_target").dataset.dec = dec;
                   $("st_target").placeholder = `RA ${ra.toFixed(2)}° Dec ${dec.toFixed(2)}° (from sky)`; }
  else cmd({ cmd: "target", ra_deg: ra, dec_deg: dec });
});
const viewer = new ResultViewer($("result"), Q);
const stareViewer = new ResultViewer($("result"), Q, "/api/campaign");

fetch("/api/stars", { headers: H }).then(r => r.ok ? r.json() : Promise.reject()).then(s => sky.setStars(s))
  .catch(() => flash("Not authorised. Open the console with the ?token=… URL the simulator printed.", true));
fetch("/api/coast", { headers: H }).then(r => r.ok ? r.json() : []).then(c => { coast = c; ground.draw(); }).catch(() => {});

// ---------------------------------------------------------------- programme
const MODES = [["survey", "Survey"], ["stack", "Stack"], ["mosaic", "Mosaic"], ["neo", "NEO sweep"], ["queue", "Science ops"]];
for (const [k, label] of MODES) $("modebtns").append(el("button", { "data-mode": k, onclick: () => cmd({ cmd: "mode", mode: k }) }, label));
$("settarget").addEventListener("click", () => cmd({ cmd: "target", name: $("target").value }));
$("target").addEventListener("keydown", (e) => { if (e.key === "Enter") cmd({ cmd: "target", name: $("target").value }); });
$("auto").addEventListener("change", () => cmd({ cmd: "auto_approve", on: $("auto").checked }));
$("share").addEventListener("input", () => $("shareval").textContent = `${$("share").value} %`);
$("share").addEventListener("change", () => cmd({ cmd: "public_share", share: +$("share").value / 100 }));

// ---------------------------------------------------------------- STARE
$("st_start").addEventListener("click", () => {
  const b = { cmd: "stare_start", eyes: +$("st_eyes").value, frame_s: +$("st_frame").value };
  if ($("st_dur").value) b.duration_h = +$("st_dur").value;
  const t = $("st_target").value.trim();
  if (t) b.target = t;
  else if ($("st_target").dataset.ra) { b.ra_deg = +$("st_target").dataset.ra; b.dec_deg = +$("st_target").dataset.dec; }
  else { flash("Give a STARE target (name, or Shift-click the sky).", true); return; }
  cmd(b);
});
$("st_all").addEventListener("click", () => cmd({ cmd: "stare_activate_all" }));
function renderStares(s) {
  const box = $("stares");
  if (box.contains(document.activeElement)) return;
  const all = s.campaigns || [];
  const act = all.filter(c => c.active), ready = all.filter(c => c.armed && !c.active);
  const ended = all.filter(c => !c.active && !c.armed).slice(-3).reverse();
  $("st_all").disabled = ready.length === 0;
  const row = (c) => {
    const { full, any } = stareCoverage(c);
    const eyes = el("input", { type: "number", value: c.eyes, min: 1, max: 5000, class: "tiny" });
    if (c.armed && !c.active) {
      return el("div", { class: "stare ready" },
        el("div", { class: "insphead" }, el("span", { class: "lamp grey" }), el("b", {}, c.target_name),
          el("span", { class: "muted small" }, c.blocked ? ` · ready · in ${c.blocked.toLowerCase()} (would start paused)` : " · ready")),
        el("div", { class: "btns" },
          el("button", { class: "primary", onclick: () => cmd({ cmd: "stare_activate", id: c.id }) }, "Start"),
          eyes, el("button", { onclick: () => cmd({ cmd: "stare_eyes", id: c.id, eyes: +eyes.value }) }, "Set eyes")));
    }
    const lamp = !c.active ? "grey" : c.paused ? "amber" : c.locked_now >= c.eyes ? "green" : c.locked_now > 0 ? "amber" : "red";
    const hours = (c.eligible_s + c.excluded_s) / 3600;
    return el("div", { class: "stare" },
      el("div", { class: "insphead" }, el("span", { class: `lamp ${lamp}` }), el("b", {}, `#${c.id} ${c.target_name}`),
        el("span", { class: "muted small" }, c.active ? (c.paused ? " · PAUSED" : ` · ${c.locked_now}/${c.eyes} locked`) : " · ended")),
      el("div", { class: "small" },
        c.paused ? el("div", { class: "warn" }, `Target inside the ${String(c.blocked).toLowerCase()} — eyes released, resumes automatically`) : "",
        !c.paused && c.blocked ? el("span", { class: "warn" }, c.blocked + " · ") : "",
        full !== null ? `full strength ${full.toFixed(2)} % · ≥1 eye ${any.toFixed(2)} % · ` : (c.paused ? "" : "acquiring… · "),
        `gaps ${c.gaps}${c.max_gap_s > 0 ? ` (max ${fmtDur(c.max_gap_s)})` : ""} · handovers ${c.handovers}` +
        (hours > 0.05 ? ` (${(c.handovers / hours).toFixed(1)}/h)` : "") + ` · ${fmtDur(c.integ_s)} integrated` +
        (c.excluded_s > 0 ? ` · ${fmtDur(c.excluded_s)} paused` : "")),
      el("div", { class: "btns" },
        c.has_frame ? el("button", { class: "view", onclick: () => { viewer.close(); stareViewer.show(c.id); } }, "Live frame") : "",
        c.active ? eyes : "", c.active ? el("button", { onclick: () => cmd({ cmd: "stare_eyes", id: c.id, eyes: +eyes.value }) }, "Set eyes") : "",
        c.active ? el("button", { class: "danger", onclick: () => cmd({ cmd: "stare_stop", id: c.id }) }, "Stop") : ""));
  };
  box.replaceChildren(...act.map(row), ...(ready.length ? [el("div", { class: "small muted", style: "margin-top:4px" }, "Ready")] : []), ...ready.map(row),
                      ...(ended.length ? [el("div", { class: "small muted", style: "margin-top:4px" }, "Ended")] : []), ...ended.map(row));
  if (!box.children.length) box.append(el("div", { class: "muted small" }, "No campaigns."));
}

// ---------------------------------------------------------------- inspector
function parseSat(q) { const m = String(q).match(/(\d+)/); return m ? +m[1] - 1 : null; }
$("satgo").addEventListener("click", () => { const i = parseSat($("satq").value); if (i !== null) inspect(i); });
$("satq").addEventListener("keydown", (e) => { if (e.key === "Enter") $("satgo").click(); });

async function inspect(i) {
  selected = i; ground.draw();
  await loadInspector();
}
async function loadInspector() {
  if (selected === null) return;
  try {
    const r = await fetch(`/api/sat/${selected}`, { headers: H });
    const s = await r.json();
    if (s.pending) { $("inspector").textContent = "requesting telemetry…"; setTimeout(loadInspector, 400); return; }
    if (s.error) { $("inspector").textContent = s.error; return; }
    renderInspector(s);
  } catch {}
}
function renderInspector(s) {
  const kv = (rows) => el("table", { class: "kv" }, ...rows.filter(([, v]) => v !== null && v !== undefined)
    .map(([k, v]) => el("tr", {}, el("th", {}, k), el("td", {}, v))));
  const lamp = s.failed || s.hold ? "red" : s.phase.startsWith("locked") ? "green" : s.phase === "idle" ? "grey" : "amber";
  $("inspector").className = "";
  $("inspector").replaceChildren(
    el("div", { class: "insphead" }, el("span", { class: `lamp ${lamp}` }), el("b", {}, s.name), el("span", { class: "muted small" }, ` plane ${s.plane} · slot ${s.slot}`)),
    kv([
      ["State", s.failed ? "FAILED (no telemetry)" : s.phase],
      ["Task", s.task],
      ["Position", `${s.lat >= 0 ? s.lat + "° N" : -s.lat + "° S"}, ${s.lon >= 0 ? s.lon + "° E" : -s.lon + "° W"} · ${s.alt_km} km · ${s.speed_km_s} km/s`],
      ["Illumination", s.sunlit ? "sunlit" : "eclipse"],
      ["Boresight", `RA ${s.boresight.ra}° Dec ${s.boresight.dec >= 0 ? "+" : ""}${s.boresight.dec}°`],
      ["Commanded", s.target ? `RA ${s.target.ra}° Dec ${s.target.dec >= 0 ? "+" : ""}${s.target.dec}°` : null],
      ["Pointing error", s.target ? `${s.pointing_error_arcsec}″` : null],
      ["Guidance", s.guidance],
      ["Jitter", s.jitter_arcsec !== null ? `${s.jitter_arcsec}″ RMS` : null],
      ["Laser links", `${s.links_up} up` + (s.links_blocked ? ` · ${s.links_blocked} blocked by atmosphere` : "")],
    ]),
    el("div", { class: "btns" }, s.failed ? el("span", { class: "muted small" }, "Failed: awaiting replacement.") : s.hold
      ? el("button", { class: "primary", onclick: () => cmd({ cmd: "safe_hold", sat: s.index, on: false }) }, "Return to service")
      : el("button", { class: "danger", onclick: () => cmd({ cmd: "safe_hold", sat: s.index, on: true }) }, "Command safe hold")),
  );
}

// ---------------------------------------------------------------- tables & panels
function reqRow(r) {
  const prio = el("input", { type: "number", value: r.priority, min: -100, max: 100, class: "tiny" });
  const tel = el("input", { type: "number", value: r.telescopes, min: 1, max: 10000, class: "tiny" });
  const act = (action) => cmd({ cmd: "request", id: r.id, action, priority: +prio.value, telescopes: +tel.value });
  const btns = el("td", { class: "acts" });
  if (r.state === "pending") btns.append(el("button", { class: "primary", onclick: () => act("approve") }, "Approve"),
                                         el("button", { class: "danger", onclick: () => act("reject") }, "Reject"));
  else if (["approved", "observing"].includes(r.state)) btns.append(el("button", { onclick: () => act("update") }, "Update"),
                                         el("button", { class: "danger", onclick: () => act("cancel") }, "Cancel"));
  else btns.append(el("button", { onclick: () => act("requeue") }, "Re-queue"));
  if (r.has_result) btns.append(el("button", { class: "view", onclick: () => { stareViewer.close(); viewer.show(r.id); } }, r.state === "done" ? "Result" : "Live"));
  const who = el("td", {}, r.submitter); who.title = `${r.ip} · submitted ${r.submitted_utc}${r.note ? "\n" + r.note : ""}`;
  return el("tr", { onmouseenter: () => { sky.highlight = r.id; sky.draw(); } },
    el("td", {}, r.id), who, el("td", {}, r.target_name, r.note ? el("div", { class: "small muted" }, r.note) : ""),
    el("td", { class: "num" }, tel), el("td", { class: "num" }, fmtDur(r.exposure_s)),
    el("td", { class: "num" }, prio), stateCell(r), progressCell(r), btns);
}

// ---------------------------------------------------------------- planetary defence
function fmtSigned(x, d = 2) { return x === null || x === undefined ? "–" : (x >= 0 ? "+" : "") + x.toFixed(d); }
function fmtIp(p) {
  if (p === null || p === undefined) return "–";
  if (p >= 0.9999) return "> 99.99 %";
  if (p >= 0.001) return `${(p * 100).toFixed(p >= 0.1 ? 1 : 2)} %`;
  if (p <= 0) return "0";
  return `1 in ${Math.round(1 / p).toLocaleString()}`;
}
function renderDefence(s) {
  const d = s.defence, sec = $("pd_sec");
  sec.hidden = !d; if (!d) return;
  const [a, b] = d.systems;
  $("pd_state").textContent = d.state === "impacted" ? "IMPACT" : d.state === "tracking" ? "TRACKING" : "UNDISCOVERED";
  $("pd_state").className = "pdbadge " + d.state;
  $("pd_name").textContent = d.position_known || d.state === "impacted" ? d.designation : "Undiscovered object (exercise truth hidden from operators)";
  $("pd_count").textContent = d.state === "impacted" ? `impact ${d.impact_utc.slice(5, 16)} UTC` : `impact in ${fmtDur(d.t_to_impact_s)}`;
  $("pd_sub").textContent = `${d.scenario} · ${d.diameter_m.toFixed(0)} m, H = ${d.h_mag.toFixed(1)}, ${d.energy_mt.toFixed(1)} Mt · v∞ ${d.v_inf_kms} km/s · radiant ${d.radiant_elong_deg.toFixed(0)}° from the Sun · simulated exercise`;
  const row = (k, f) => el("tr", {}, el("th", {}, k), el("td", {}, f(a)), el("td", {}, f(b)));
  $("pd_cmp").replaceChildren(
    row("Discovered", v => v.warning_s == null ? "not yet" : `${fmtDur(v.warning_s)} before impact`),
    row("Astrometric observations", v => v.n_obs ? v.n_obs.toLocaleString() : "–"),
    row("Observed arc", v => v.arc_h ? fmtDur(v.arc_h * 3600) : "–"),
    row("Impact probability", v => fmtIp(v.ip)),
    row("Palermo scale", v => fmtSigned(v.palermo)),
    row("b-plane 1σ", v => v.sigma_b_km == null ? "–" : `${v.sigma_b_km < 10 ? v.sigma_b_km.toFixed(1) : Math.round(v.sigma_b_km).toLocaleString()} km`),
    row("Nominal miss distance", v => v.nominal_miss_km == null ? "–" : `${Math.round(v.nominal_miss_km).toLocaleString()} km`),
    row("IP ≥ 1 %", v => v.ip_1pct_s == null ? "–" : `${fmtDur(v.ip_1pct_s)} before`),
    row("IP ≥ 99 % (certain)", v => v.certain_s == null ? "–" : `${fmtDur(v.certain_s)} before`),
  );
  const n = d.now, known = d.position_known;
  const kv = (k, v) => el("tr", {}, el("th", {}, k), el("td", {}, v));
  $("pd_now").replaceChildren(
    kv("V magnitude", `${n.v_mag.toFixed(1)}`),
    kv("Distance", `${Math.round(n.dist_km).toLocaleString()} km (${(n.dist_km / 384400).toFixed(1)} LD)`),
    kv("Solar elongation", `${n.elong_deg.toFixed(0)}°` + (n.blocked ? ` · ${n.blocked}` : "")),
    kv("Phase angle", `${n.phase_deg.toFixed(0)}°`),
    kv("Sky motion", `${n.rate_arcsec_s.toFixed(2)}″/s`),
    kv("FACET search limit (trailed)", `V ${n.facet_search_lim.toFixed(1)}` + (n.detectable_facet ? " · detectable" : "")),
    kv("FACET follow-up limit", `V ${n.facet_follow_lim.toFixed(1)}`),
    kv("Ground", n.observable_ground ? "observable at night" : (n.elong_deg < d.ground_model.min_elong_deg ? "in the daytime sky" : "too faint")),
    kv("Searching telescopes", `${n.search_eyes}` + (n.sky_revisit_h ? ` · open sky every ${n.sky_revisit_h.toFixed(1)} h` : "")),
    known ? "" : kv("", el("span", { class: "muted small" }, "Operators do not know this object yet; these are exercise truth values.")),
  );
  drawIpChart($("pd_chart"), d);
}
function drawIpChart(c, d) {
  const dpr = window.devicePixelRatio || 1, w = c.clientWidth * dpr, h = c.clientHeight * dpr;
  if (c.width !== w || c.height !== h) { c.width = w; c.height = h; }
  const g = c.getContext("2d"); g.clearRect(0, 0, w, h);
  const L = 44 * dpr, R = 10 * dpr, T = 10 * dpr, B = 22 * dpr;
  // Zoom the time axis to the decision window: from just before the first
  // discovery until both systems are certain (or now, while tracking).
  const total = Math.max(1, d.history.length ? d.history[0][0] : 1), tnow = d.t_to_impact_s / 3600;
  const warn = d.systems.map(v => v.warning_s).filter(v => v != null).map(v => v / 3600);
  const cert = d.systems.map(v => v.certain_s);
  let thi = warn.length ? Math.min(total, Math.max(...warn) * 1.08) : total;
  let tlo = Math.max(0, tnow);
  if (warn.length && cert.every((c, i) => c != null || d.systems[i].warning_s == null)) {
    const cs = cert.filter(c => c != null).map(c => c / 3600);
    if (cs.length) tlo = Math.max(tlo, Math.min(...cs) - (thi - Math.min(...cs)) * 0.35);
  }
  if (thi - tlo < 0.05) tlo = Math.max(0, thi - 0.05);
  const x = t => L + (thi - t) / (thi - tlo) * (w - L - R);
  const y = p => T + (1 - (Math.log10(Math.max(p, 1e-6)) + 6) / 6) * (h - T - B);
  g.strokeStyle = "rgba(120,140,180,0.25)"; g.fillStyle = "#8a94a8"; g.font = `${10 * dpr}px system-ui`; g.lineWidth = dpr;
  for (let e = -6; e <= 0; e++) { const yy = y(10 ** e); g.beginPath(); g.moveTo(L, yy); g.lineTo(w - R, yy); g.stroke(); g.fillText(e === 0 ? "1" : `1e${e}`, 4 * dpr, yy + 3 * dpr); }
  for (let i = 0; i <= 4; i++) {
    const t = thi - (thi - tlo) * i / 4, X = x(t);
    g.beginPath(); g.moveTo(X, T); g.lineTo(X, h - B); g.stroke();
    g.fillText(`T−${fmtDur(t * 3600)}`, Math.min(X - 14 * dpr, w - 60 * dpr), h - 6 * dpr);
  }
  g.save(); g.beginPath(); g.rect(L, 0, w - L - R, h); g.clip();
  if (tnow > tlo && d.state !== "impacted") { g.strokeStyle = "rgba(255,120,120,0.6)"; const X = x(tnow); g.beginPath(); g.moveTo(X, T); g.lineTo(X, h - B); g.stroke(); }
  for (const [k, sys] of [[1, d.systems[0]], [2, d.systems[1]]]) {
    if (sys.warning_s != null) { const X = x(sys.warning_s / 3600); g.fillStyle = k === 1 ? "#4fa3ff" : "#ffb44a"; g.beginPath(); g.arc(X, h - B - 4 * dpr, 4 * dpr, 0, 7); g.fill(); }
  }
  for (const [k, col] of [[2, "#ffb44a"], [1, "#4fa3ff"]]) {
    g.strokeStyle = col; g.lineWidth = 2 * dpr; g.beginPath(); let on = false;
    for (const p of d.history) { if (p[k] < 0) { on = false; continue; } const X = x(p[0]), Y = y(p[k]); if (on) g.lineTo(X, Y); else g.moveTo(X, Y); on = true; }
    g.stroke();
  }
  g.restore();
  g.font = `${10 * dpr}px system-ui`;
  g.fillStyle = "#4fa3ff"; g.fillText("FACET + ground", L + 6 * dpr, T + 12 * dpr);
  g.fillStyle = "#ffb44a"; g.fillText("Ground only", L + 110 * dpr, T + 12 * dpr);
}

// ---------------------------------------------------------------- space environment
function fmtYears(y) {
  if (y == null) return "–";
  if (y >= 299) return "> 300 yr";
  if (y >= 20) return `${y.toFixed(0)} yr`;
  if (y >= 1) return `${y.toFixed(1)} yr`;
  return `${Math.round(y * 365.25)} days`;
}
function renderEnvironment(s) {
  const e = s.environment; if (!e) return;
  const pol = { pause: "detectors safed inside", flag: "exposing through it (flagged)", off: "ignored" }[e.policy];
  $("en_head").textContent = `· ${s.shell.alt_km} km, ${s.shell.inc_deg}° shell`;
  $("en_saa").replaceChildren(
    tile("in the anomaly now", `${e.in_saa_now.toLocaleString()}`, `${pol} · threshold ${e.threshold} p/cm²/s (>10 MeV)`, "saa"),
    tile("orbit average", e.orbit_saa_pct == null ? "–" : `${e.orbit_saa_pct.toFixed(1)} %`, "share of each satellite's time inside", "saa"),
    tile("fleet time lost", `${e.saa_lost_pct.toFixed(1)} %`, "exposure time lost to safed detectors this run", "saa"),
    tile("false transients", `${Math.round(e.candidates_per_day).toLocaleString()}/day`, `single-frame particle hits that look star-like; ${e.coincidences_per_day.toFixed(0)}/day pass a 2-telescope check, ${e.coincidences3_per_day.toExponential(1)}/day a 3-telescope check`, "saa"),
    tile("proton fluence", `${e.fluence_per_sat_yr.toExponential(1)}`, "p/cm² per satellite per year (detector damage budget)", "saa"),
  );
  const L = e.lifetime || {};
  $("en_life").replaceChildren(
    tile("Earthshine loss", e.earthshine_on ? `${e.earthshine_lost_pct.toFixed(1)} %` : "off", `of exposure time (dark-sky equivalent) · ${e.earthshine_now} telescopes badly affected now`, "earthshine"),
    tile("sunlight stray-light loss", `${(e.sunlight_lost_pct || 0).toFixed(1)} %`, `of exposure time (dark-sky equivalent) · ${e.sunlight_now || 0} telescopes badly affected now`, "stray_loss"),
    tile("passive lifetime", fmtYears(L.mean_yr), `mean sun · ${fmtYears(L.solar_max_yr)} at solar max, ${fmtYears(L.solar_min_yr)} at solar min`, "lifetime"),
    tile("following the cycle", fmtYears(L.cycle_yr), L.cycle_yr != null && L.cycle_yr <= 5 ? "meets a 5-year disposal rule passively" : `needs a ${e.disposal_dv_ms == null ? "–" : e.disposal_dv_ms.toFixed(0)} m/s disposal burn for a 5-year rule`, "lifetime"),
    tile("ballistic coefficient", `${e.ballistic_m2_kg.toFixed(4)}`, "C_D·A/m, m²/kg", "lifetime"),
  );
  const ko = s.shell.sun_excl_deg;
  $("en_keep").replaceChildren(...(e.keepout || []).map(r => el("tr", Math.abs(r.keepout_deg - ko) < 0.5 ? { class: "me" } : {},
    el("td", {}, `${r.keepout_deg.toFixed(0)}°${Math.abs(r.keepout_deg - ko) < 0.5 ? " (now)" : ""}`),
    el("td", { class: "num" }, `${(100 * r.venus_frac).toFixed(0)} %`),
    el("td", { class: "num" }, `${(100 * r.mercury_frac).toFixed(0)} %`),
    el("td", { class: "num" }, `${Math.round(r.open_deg2).toLocaleString()} deg²`),
    el("td", { class: "num" }, `−${r.edge_depth_loss.toFixed(1)} mag`),
    el("td", { class: "num" }, `−${r.edge_stray_loss.toFixed(2)} mag`))));
  $("en_trade").replaceChildren(...e.trade.map(r => el("tr", {},
    el("td", {}, `${r.alt_km.toFixed(0)}`),
    el("td", { class: "num" }, `${(100 * r.saa_frac).toFixed(1)} %`),
    el("td", { class: "num" }, fmtYears(r.life_mean_yr)),
    el("td", { class: "num" }, `${fmtYears(r.life_min_yr)} – ${fmtYears(r.life_max_yr)}`),
    el("td", { class: "num" }, `${r.disposal_dv.toFixed(0)} m/s`))));
}

// ---------------------------------------------------------------- resilience & ground comparison
function renderResilience(s) {
  const r = s.resilience, g = s.ground; if (!r) return;
  const last = r.series.length ? r.series[r.series.length - 1] : null;
  $("rs_tiles").replaceChildren(
    tile("fleet alive", `${(100 * r.alive_frac).toFixed(1)} %`, `${r.failed} failed now · ${r.total_failed} failures, ${r.total_replaced} replaced so far`, "resilience"),
    tile("science output, last hour", last ? `${Math.round(100 * last[2])} %` : "–", "integration per satellite-hour, relative to the healthy fleet", "resilience"),
    tile("observations, last hour", last ? `${Math.round(100 * last[3])} %` : "–", "completed requests, relative to the healthy fleet", "resilience"),
    tile("random failures", `${r.rate_pct_yr} %/yr`, "", "resilience"),
    tile("replacement", r.replace_days > 0 ? `after ${r.replace_days} d` : "none", "", "resilience"),
  );
  drawResilience($("rs_chart"), r);
  if (g) {
    const pc = x => `${(100 * x).toFixed(0)} %`;
    const row = (k, a, b, note) => el("tr", { title: note || "" }, el("th", {}, k), el("td", {}, a), el("td", {}, b));
    $("gc_table").replaceChildren(
      row("Time on the sky (duty cycle)", pc(g.facet_duty), pc(g.ground_duty), "night × weather × uptime on the ground"),
      row("Sky observable right now", pc(g.facet_sky_now), pc(g.ground_sky_now), "at least one telescope can point there"),
      row("Queue targets observable within 24 h", pc(g.facet_within_24h), pc(g.ground_within_24h), `${g.targets} open requests`),
      row("Queue targets observable right now", pc(g.facet_now), pc(g.ground_now), "ground: above 30° at a site that is dark and clear now (expected over weather)"),
      row("FACET response (slew + settle + lock)", g.facet_median_response_s == null ? "–" : fmtDur(g.facet_median_response_s), "–", "median measured in the simulation"),
      row("5σ depth, one telescope, 60 s", `V ${g.facet_depth_60s.toFixed(1)}`, `V ${g.ground_depth_60s.toFixed(1)}`, "same 20 cm aperture"),
    );
    $("gc_head").textContent = `FACET (${s.shell.n.toLocaleString()} telescopes) vs ${g.telescopes.toLocaleString()} identical telescopes at ${g.sites} ground sites`;
    $("gc_equiv").textContent = `One FACET telescope delivers about as much as ${g.equivalent_ground_telescopes.toFixed(1)} identical ground telescopes at equal depth.`;
    $("gc_notes").textContent = g.notes.join(" ");
  }
}
function drawResilience(c, r) {
  const dpr = window.devicePixelRatio || 1, w = c.clientWidth * dpr, h = c.clientHeight * dpr;
  if (c.width !== w || c.height !== h) { c.width = w; c.height = h; }
  const g = c.getContext("2d"); g.clearRect(0, 0, w, h);
  const L = 36 * dpr, R = 8 * dpr, T = 8 * dpr, B = 18 * dpr, S = r.series;
  g.strokeStyle = "rgba(120,140,180,0.25)"; g.fillStyle = "#8a94a8"; g.font = `${10 * dpr}px system-ui`; g.lineWidth = dpr;
  const y = v => T + (1 - Math.min(Math.max(v, 0), 1.2) / 1.2) * (h - T - B);
  for (const v of [0, 0.5, 1]) { g.beginPath(); g.moveTo(L, y(v)); g.lineTo(w - R, y(v)); g.stroke(); g.fillText(`${v * 100}%`, 2 * dpr, y(v) + 3 * dpr); }
  if (S.length < 2) { g.fillText("Collecting hourly data…", L + 8 * dpr, T + 14 * dpr); return; }
  const t0 = S[0][0], t1 = S[S.length - 1][0], x = t => L + (t - t0) / Math.max(t1 - t0, 1e-6) * (w - L - R);
  for (const [k, col] of [[1, "#ffffff"], [2, "#4fa3ff"], [3, "#5fe0a0"]]) {
    g.strokeStyle = col; g.lineWidth = (k === 1 ? 2 : 1.5) * dpr; g.beginPath();
    S.forEach((p, i) => { const X = x(p[0]), Y = y(p[k]); if (i) g.lineTo(X, Y); else g.moveTo(X, Y); });
    g.stroke();
  }
  g.fillStyle = "#fff"; g.fillText("fleet alive", L + 6 * dpr, h - 4 * dpr);
  g.fillStyle = "#4fa3ff"; g.fillText("science output", L + 80 * dpr, h - 4 * dpr);
  g.fillStyle = "#5fe0a0"; g.fillText("observations", L + 180 * dpr, h - 4 * dpr);
  g.fillStyle = "#8a94a8"; g.fillText(`last ${fmtDur((t1 - t0) * 86400)}`, w - 90 * dpr, h - 4 * dpr);
}

// ---------------------------------------------------------------- space traffic
function fmtPc(p) { return p >= 1e-12 ? p.toExponential(1) : "< 1e-12"; }
function renderTraffic(s) {
  const t = s.traffic; if (!t) return;
  const d = t.data, st = t.streaks, c = t.conjunctions, pd = c.per_day;
  $("tr_counts").textContent = `Starlink ${t.counts.starlink.toLocaleString()} at ${t.config.starlink_alt_km} km · OneWeb ${t.counts.oneweb} at 1,200 km · ${t.counts.other.toLocaleString()} other objects within ±${t.config.other_band_km} km of FACET (assumed)`;
  const pct = x => `${(x * 100).toFixed(x < 0.01 ? 2 : 0)} %`;
  $("tr_data").replaceChildren(
    tile("raw data / day", `${d.raw_tb_day.toFixed(1)} TB`, "16-bit full frames from the science ledger", "downlink"),
    tile("downlinked / day", `${d.demand_tb_day.toFixed(1)} TB`, d.product_label, "downlink"),
    tile("relay capacity", t.config.relay ? `${fmtNum(d.relay_cap_tb_day)} TB/day` : "off", `Starlink optical relay (assumed service): ${t.config.relay_gbps} Gb/s per FACET terminal, ${pct(d.relay_linked_frac)} of satellites linked, mean range ${Math.round(d.relay_mean_range_km)} km`, "downlink"),
    tile("ground-station capacity", `${fmtNum(d.dtg_cap_tb_day)} TB/day`, `${t.config.stations} stations × ${t.config.antennas_per_station} antennas × ${t.config.station_gbps} Gb/s × 70 % (assumed)`, "downlink"),
    tile("alert latency via relay", d.relay_latency_ms == null ? "–" : `${Math.round(d.relay_latency_ms)} ms`, "FACET → Starlink → gateway → operations (model)", "downlink"),
    tile("wait for a ground pass", fmtDur(d.dtg_median_wait_s), `median; a satellite is in view of a station ${pct(d.dtg_in_view_frac)} of the time`, "downlink"),
    tile("capacity margin", `${d.margin.toFixed(d.margin < 10 ? 1 : 0)}×`, "(relay + ground) / downlinked volume", "downlink"),
    tile("on-board storage fills", d.storage_fill_days == null ? "never" : fmtDur(d.storage_fill_days * 86400), `${t.config.storage_tb} TB per satellite (assumed)`, "downlink"),
  );
  $("tr_products").replaceChildren(...d.products.map(p => el("tr", { class: p.selected ? "sel" : "" },
    el("td", {}, p.label), el("td", { class: "num" }, p.tb_day.toFixed(1)),
    el("td", { class: "num" + (p.relay_util > 1 ? " warn" : "") }, p.relay_util == null ? "–" : pct(p.relay_util)),
    el("td", { class: "num" + (p.dtg_util > 1 ? " warn" : "") }, pct(p.dtg_util)))));
  const pk = st.per_1000_frames;
  $("tr_streaks").replaceChildren(
    tile("frames that would be streaked", `${st.streaked_pct.toFixed(2)} %`, `sampled ${fmtNum(st.frames)} exposures`, "streaks"),
    tile("avoidance", st.avoidance ? "ON" : "OFF", "Exposures are gated around predicted crossings", "streaks"),
    tile("integration lost to gating", `${st.avoidance_cost_pct.toFixed(2)} %`, "", "streaks"),
    tile("Starlink / 1000 frames", pk.starlink.toFixed(1), "below FACET: only when pointing low, toward the limb", "streaks"),
    tile("OneWeb / 1000 frames", pk.oneweb.toFixed(1), "above FACET: crosses upward-looking fields", "streaks"),
    tile("FACET / 1000 frames", pk.facet.toFixed(1), "FACET's own neighbours", "streaks"),
  );
  $("tr_streak_note").textContent = st.avoidance
    ? "Crossings are predicted from published ephemerides; the shutter closes or the sub-frame is dropped for the few seconds a satellite is in the field."
    : "Avoidance is off: streaked frames are delivered and must be masked downstream.";
  $("tr_conj").replaceChildren(
    tile("close approaches < 5 km / day", fmtNum(pd.approaches_5km, 1), "", "conjunctions"),
    tile("notices / day", fmtNum(pd.notices, 1), "miss < 1 km or Pc ≥ 1e-6", "conjunctions"),
    tile("high risk / day", fmtNum(pd.high_risk, 2), "Pc ≥ 1e-5", "conjunctions"),
    tile("manoeuvres / day", fmtNum(pd.manoeuvres, 2), `Pc ≥ ${t.config.pc_manoeuvre.toExponential(0)}`, "conjunctions"),
    tile("Δv per satellite per year", `${c.dv_per_sat_year_ms.toFixed(2)} m/s`, "", "conjunctions"),
    tile("closest approach", c.stats.closest_km == null ? "–" : `${Math.round(c.stats.closest_km * 1000)} m`, "", "conjunctions"),
  );
  const nowJd = s.jd;
  $("tr_notices").replaceChildren(...c.upcoming.map(n => el("tr", { class: `lvl-${n.level}` },
    el("td", {}, `T−${fmtDur((n.tca_jd - nowJd) * 86400)}`),
    el("td", {}, `FACET-${String(n.facet + 1).padStart(5, "0")}`), el("td", {}, n.object),
    el("td", { class: "num" }, `${Math.round(n.miss_km * 1000)} m`), el("td", { class: "num" }, fmtPc(n.pc)),
    el("td", { class: "small" }, n.action + (n.dv_ms ? ` (Δv ${(n.dv_ms * 1000).toFixed(0)} mm/s)` : "")))));
  if (!c.upcoming.length) $("tr_notices").append(el("tr", {}, el("td", { class: "muted small", colspan: 6 }, "No notices in the screening window.")));
}

// ---------------------------------------------------------------- science return
const USE = [["programme", "Science programme", "#4fa3ff"], ["public", "Public requests", "#5fe0a0"], ["stare", "STARE", "#c58bff"],
             ["director", "Director target", "#ffd166"], ["survey", "Zenith survey", "#7c8aa5"], ["neo_sweep", "NEO sweep", "#ff9f6e"]];
function fmtNum(x, d = 0) { return x === null || x === undefined ? "–" : Number(x).toLocaleString(undefined, { maximumFractionDigits: d, minimumFractionDigits: d }); }
function tile(label, value, hint, key) { return el("div", { class: "tile", title: hint || "" }, el("div", { class: "tv" }, value), el("div", { class: "tl" }, label, key ? " " : "", key ? basisChip(key) : "")); }
function renderScience(s) {
  const sc = s.science; if (!sc) return;
  const e = sc.efficiency, d = sc.per_day;
  $("sci_since").textContent = `since ${sc.since_utc.slice(0, 16)} UTC · ${fmtDur(sc.elapsed_s)} simulated` + (s.programme ? ` · demand ${s.programme.load.toFixed(2)}× capacity · ${s.programme.investigators} teams` : "");
  $("sci_tiles").replaceChildren(
    tile("requests completed / day", fmtNum(d.completed), "Programme requests finished per simulated day", "science"),
    tile("on time", sc.on_time_pct === null ? "–" : `${sc.on_time_pct.toFixed(1)} %`, "Completed before the science case's deadline, of all completed + expired", "science"),
    tile("open-shutter efficiency", `${e.integrating_pct.toFixed(1)} %`, "Satellite time spent integrating (any use)", "duty"),
    tile("pointed science", `${e.pointed_science_pct.toFixed(1)} %`, "Satellite time integrating on requests, STARE and director targets", "duty"),
    tile("telescope-hours / day", fmtNum(d.tel_h), "Integration delivered per day, all uses", "science"),
    tile("étendue × time / day", `${fmtNum(d.etendue_m2deg2h)} m²·deg²·h`, "Σ collecting area × field of view × integration time: the standard survey-speed figure of merit", "science"),
    tile("raw data / day", `${d.tb.toFixed(1)} TB`, "16-bit full frames before compression: sizes the downlink", "allsky_data"),
    tile("teams served", `${sc.investigators_served} / ${sc.investigators}`, "Investigator teams with at least one completed request", "science"),
  );
  const tot = 100;
  const segs = USE.map(([k, , c]) => [sc.use_h[k] * 3600 / Math.max(1, sc.elapsed_s * (s.shell.n || 1)) * 100, c]);
  segs.push([e.overhead_pct, "#3a4256"], [e.saa_pct || 0, "#ff5ac8"], [e.idle_pct + e.hold_pct, "#1a2130"]);
  $("sci_use").replaceChildren(...segs.map(([p, c]) => el("div", { style: `width:${Math.max(0, p / tot * 100)}%;background:${c}` })));
  $("sci_legend").replaceChildren(...USE.map(([k, l, c]) => el("span", {}, el("i", { style: `background:${c}` }), `${l} ${fmtNum(sc.use_h[k])} h`)),
    el("span", {}, el("i", { style: "background:#3a4256" }), `slew/settle/acquire ${e.overhead_pct.toFixed(1)} %`),
    el("span", {}, el("i", { style: "background:#ff5ac8" }), `South Atlantic Anomaly ${(e.saa_pct || 0).toFixed(1)} %`),
    el("span", {}, el("i", { style: "background:#1a2130" }), `idle/hold ${(e.idle_pct + e.hold_pct).toFixed(1)} %`));
  $("sci_cases").replaceChildren(...sc.cases.map(c => {
    const fin = c.completed + c.expired;
    return el("tr", {}, el("td", {}, c.label), el("td", { class: "num" }, fmtDur(c.deadline_h * 3600)),
      el("td", { class: "num" }, fmtNum(c.submitted)), el("td", { class: "num" }, fmtNum(c.open)),
      el("td", { class: "num" }, fmtNum(c.completed)), el("td", { class: "num" + (c.expired ? " warn" : "") }, fmtNum(c.expired)),
      el("td", { class: "num" }, fin ? `${(100 * c.on_time / fin).toFixed(1)} %` : "–"),
      el("td", { class: "num" }, c.median_response_s === null ? "–" : fmtDur(c.median_response_s)),
      el("td", { class: "num" }, c.median_depth_mag === null ? "–" : `V ${c.median_depth_mag.toFixed(1)}`),
      el("td", { class: "num" }, fmtNum(c.tel_s / 3600)));
  }));
  $("sci_top").replaceChildren(...(sc.top.length ? sc.top.map(t => el("tr", {}, el("td", {}, t.name, el("div", { class: "small muted" }, t.case)),
    el("td", { class: "num" }, `${fmtNum(t.tel_h)} tel.-h`), el("td", { class: "num" }, `${t.done} done`))) : [el("tr", {}, el("td", { class: "muted small" }, "Nothing completed yet."))]));
}
$("qfilter").addEventListener("change", () => last && render(last));

function kvRows(tbl, rows) {
  tbl.replaceChildren(...rows.map(([k, v]) => el("tr", {}, el("th", {}, k), el("td", {}, v))));
}

function render(s) {
  last = s;
  if (s.provenance) PROV = s.provenance;
  const [d, t] = s.utc.split(" ");
  $("utc").textContent = `${t} UTC`;
  const doy = Math.floor((Date.parse(d + "T00:00:00Z") - Date.parse(d.slice(0, 4) + "-01-01T00:00:00Z")) / 86400000) + 1;
  $("doy").textContent = `${d} · DOY ${String(doy).padStart(3, "0")}`;
  $("modelabel").textContent = s.mode_label;
  sky.setStatus(s);
  if (s.defence) ground.draw();

  // System state pill
  const hold = fleet ? fleet.sats.filter(x => x[2] === 5).length : 0;
  const failed = s.resilience ? s.resilience.failed : 0;
  const pill = $("sysstate");
  const caution = hold > 0 || failed > 0 || s.shell.links_blocked > 0;
  pill.textContent = caution ? "CAUTION" : "NOMINAL"; pill.className = `pill ${caution ? "amber" : "green"}`;

  // Fleet
  const st = s.stats, n = s.shell.n, idle = Math.max(0, n - st.locked - st.slewing - st.settling - hold - failed);
  $("f_service").textContent = (n - hold - failed).toLocaleString();
  $("f_hold").textContent = hold.toLocaleString();
  $("f_failed").textContent = failed.toLocaleString();
  $("f_locked").textContent = st.locked.toLocaleString();
  const parts = [["integrating", st.locked, "#5affa0"], ["slewing", st.slewing, "#ff9628"], ["settling", st.settling, "#ffe150"],
                 ["idle", idle, "#3a4558"], ["safe hold", hold, "#ff4b4b"], ["failed", failed, "#7a2b2b"]];
  $("phasebar").replaceChildren(...parts.filter(p => p[1] > 0).map(([k, v, c]) =>
    el("div", { style: `flex:${v};background:${c}`, title: `${k}: ${v}` })));
  $("phaselegend").replaceChildren(...parts.map(([k, v, c]) => el("span", {}, el("i", { style: `background:${c}` }), `${k} ${v}`)));
  kvRows($("fleetkv"), [
    ["Shell", `${n.toLocaleString()} sats · ${s.shell.planes} planes · ${s.shell.alt_km.toFixed(0)} km · ${s.shell.inc_deg}°`],
    ["Orbital period", `${s.shell.period_min} min`],
    ["Sunlit / eclipse", `${st.lit} / ${st.eclipsed}`],
    ["Optical mesh", `${(s.shell.links - s.shell.links_blocked - (s.shell.links_down || 0)).toLocaleString()} links up` + (s.shell.links_blocked ? ` · ${s.shell.links_blocked} through atmosphere` : "") + (s.shell.links_down ? ` · ${s.shell.links_down} down (failed satellites)` : "")],
    ["Sky reachable now", `${st.sky_reachable} %`],
  ]);
  const t2 = s.telescope;
  kvRows($("payloadkv"), [
    ["Telescope", `${t2.aperture_cm} cm aperture · ${t2.fov_deg}° field`],
    ["Optics", `${t2.wfe_nm} nm RMS wavefront · Strehl ${t2.strehl} · ${t2.spider_vanes ? t2.spider_vanes + "-vane spider" : "unobstructed spider-free"}`],
    ["Pointing", `${t2.jitter_arcsec}″ RMS fine-guided · ${t2.coarse_jitter_arcsec}″ star tracker`],
    ["Agility", `${t2.omega_dps}°/s · repoint 10° ${t2.repoint_10_s} s, 90° ${t2.repoint_90_s} s`],
    ["Keep-out", `Sun ${t2.sun_excl_deg}° · Moon ${t2.moon_excl_deg}° · limb +${t2.limb_margin_deg}°`],
    ["1-telescope depth", `V ≈ ${t2.lim_1tel_60s} in 60 s`],
  ]);

  // Programme
  document.querySelectorAll("#modebtns button").forEach(b => b.classList.toggle("sel", b.dataset.mode === s.mode));
  $("auto").checked = s.auto_approve;
  if (document.activeElement !== $("share")) {
    $("share").value = Math.round(s.public_share * 100); $("share").disabled = s.mode === "queue";
    $("shareval").textContent = `${Math.round(s.public_share * 100)} %` + (s.programme ? ` · NEO search reserve ${Math.round(s.programme.search_share * 100)} %` : "");
  }
  const tl = $("targetline");
  if (s.target && ["stack", "mosaic"].includes(s.mode)) {
    tl.replaceChildren(el("b", {}, s.target.name), " · ",
      s.target.blocked ? el("span", { class: "warn" }, s.target.blocked)
        : `${s.target.locked} locked of ${s.target.visible} with line of sight` + (s.mode === "stack" && s.target.locked ? ` · depth V ≈ ${s.stats.stack_lim_mag}` : ""));
  } else tl.textContent = s.mode === "queue" ? "Science operations: public share first, then the science programme; the NEO search reserve and spare telescopes survey." : s.mode === "survey" ? "Zenith-stare survey." : s.mode === "neo" ? "Near-Sun asteroid sweep." : "";

  // Alerts
  const alerts = [];
  if (hold) alerts.push(["warn", `${hold} satellite${hold > 1 ? "s" : ""} in safe hold`]);
  if (failed) alerts.push(["bad", `${failed} satellite${failed > 1 ? "s" : ""} failed (${(100 * failed / n).toFixed(1)} % of the fleet)`]);
  if (s.shell.links_blocked) alerts.push(["warn", `${s.shell.links_blocked} laser links pass through the atmosphere`]);
  if (s.target && s.target.blocked && ["stack", "mosaic"].includes(s.mode)) alerts.push(["warn", `Director target ${s.target.name}: ${s.target.blocked}`]);
  const open = s.requests.filter(r => ["pending", "approved", "observing"].includes(r.state));
  const pending = open.filter(r => r.state === "pending").length;
  if (pending) alerts.push(["info", `${pending} public request${pending > 1 ? "s" : ""} awaiting approval`]);
  const stuck = open.filter(r => r.waiting_s > 600 && !r.programme);
  if (stuck.length) alerts.push(["warn", `${stuck.length} approved public request${stuck.length > 1 ? "s" : ""} waiting > 10 min for telescopes`]);
  if (s.programme && s.programme.on && !s.programme.active) alerts.push(["info", "Science programme paused: fleet dedicated to the director's sweep"]);
  if (s.mode !== "queue" && s.public_share === 0 && open.length) alerts.push(["warn", "Public time share is 0 %: the queue is not being served"]);
  $("alerts").replaceChildren(...(alerts.length ? alerts.map(([l, m]) => el("li", { class: l }, m)) : [el("li", { class: "muted" }, "none")]));

  // Queue (don't rebuild while the director is typing in it)
  const done = s.requests.filter(r => r.state === "done").length;
  const progOpen = (s.science && s.science.cases || []).reduce((a, c) => a + (c.open || 0), 0);
  $("qsummary").textContent = `${open.length - open.filter(r => r.programme).length} public open · ${pending} pending · ${progOpen} programme open · ${done} done (shown)`;
  if (!$("queue").contains(document.activeElement)) {
    const q = $("queue"); q.replaceChildren();
    const f = $("qfilter").value;
    const rows = s.requests.filter(r => f === "all" || (f === "programme") === !!r.programme);
    for (const r of rows.slice(0, 150)) q.append(reqRow(r));
    if (!q.children.length) q.append(el("tr", {}, el("td", { class: "muted small", colspan: 9 }, "No requests.")));
  }

  renderStares(s);
  renderScience(s);
  renderDefence(s);
  renderTraffic(s);
  renderResilience(s);
  renderEnvironment(s);
  for (const c of (s.campaigns || []).filter(c => c.active)) {
    if (c.paused) alerts.push(["info", `STARE ${c.target_name} paused: ${String(c.blocked).toLowerCase()}`]);
    else if (c.eligible_s > 0 && c.locked_now === 0 && !c.blocked) alerts.push(["warn", `STARE #${c.id} on ${c.target_name}: no eyes locked`]);
  }
  $("alerts").replaceChildren(...(alerts.length ? alerts.map(([l, m]) => el("li", { class: l }, m)) : [el("li", { class: "muted" }, "none")]));

  // Event log
  $("log").replaceChildren(...(s.events || []).map(e =>
    el("li", { class: e.level }, el("span", { class: "t" }, e.utc.slice(11, 19)), e.msg)));
}

async function poll() {
  try {
    const r = await fetch("/api/status", { headers: H });
    if (r.status === 401) { $("sysstate").textContent = "NO AUTH"; $("sysstate").className = "pill red"; return; }
    if (r.ok) render(await r.json());
  } catch { $("sysstate").textContent = "NO CONTACT"; $("sysstate").className = "pill red"; }
}
async function pollFleet() {
  try { const r = await fetch("/api/fleet", { headers: H }); if (r.ok) { fleet = await r.json(); ground.draw(); } } catch {}
  $("gt_note").textContent = fleet && fleet.sats ? `· ${fleet.sats.length.toLocaleString()} satellites` : "";
}
setInterval(poll, 1000); poll();
setInterval(pollFleet, 1500); pollFleet();
setInterval(loadInspector, 1500);
{ const rid = new URLSearchParams(location.search).get("result"); if (rid) viewer.show(+rid); }
