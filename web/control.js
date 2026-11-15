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
const PHASE_COL = ["#e9e2d0", "#5f73b4", "#ff9628", "#ffe150", "#5affa0", "#ff4b4b"];
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
    // satellites
    const r = (fleet.sats.length > 3000 ? 1.2 : fleet.sats.length > 500 ? 1.7 : 2.4) * this.dpr;
    for (const code of [0, 1, 3, 2, 4, 5]) {
      g.fillStyle = PHASE_COL[code];
      for (const [lo, la, c] of fleet.sats) if (c === code) { const [x, y] = this.xy(lo, la); g.fillRect(x - r, y - r, 2 * r, 2 * r); }
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
const MODES = [["survey", "Survey"], ["stack", "Stack"], ["mosaic", "Mosaic"], ["neo", "NEO sweep"], ["queue", "Public queue"]];
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
  const lamp = s.hold ? "red" : s.phase.startsWith("locked") ? "green" : s.phase === "idle" ? "grey" : "amber";
  $("inspector").className = "";
  $("inspector").replaceChildren(
    el("div", { class: "insphead" }, el("span", { class: `lamp ${lamp}` }), el("b", {}, s.name), el("span", { class: "muted small" }, ` plane ${s.plane} · slot ${s.slot}`)),
    kv([
      ["State", s.phase],
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
    el("div", { class: "btns" }, s.hold
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

function kvRows(tbl, rows) {
  tbl.replaceChildren(...rows.map(([k, v]) => el("tr", {}, el("th", {}, k), el("td", {}, v))));
}

function render(s) {
  last = s;
  const [d, t] = s.utc.split(" ");
  $("utc").textContent = `${t} UTC`;
  const doy = Math.floor((Date.parse(d + "T00:00:00Z") - Date.parse(d.slice(0, 4) + "-01-01T00:00:00Z")) / 86400000) + 1;
  $("doy").textContent = `${d} · DOY ${String(doy).padStart(3, "0")}`;
  $("modelabel").textContent = s.mode_label;
  sky.setStatus(s);

  // System state pill
  const hold = fleet ? fleet.sats.filter(x => x[2] === 5).length : 0;
  const pill = $("sysstate");
  const caution = hold > 0 || s.shell.links_blocked > 0;
  pill.textContent = caution ? "CAUTION" : "NOMINAL"; pill.className = `pill ${caution ? "amber" : "green"}`;

  // Fleet
  const st = s.stats, n = s.shell.n, idle = Math.max(0, n - st.locked - st.slewing - st.settling - hold);
  $("f_service").textContent = (n - hold).toLocaleString();
  $("f_hold").textContent = hold.toLocaleString();
  $("f_locked").textContent = st.locked.toLocaleString();
  const parts = [["integrating", st.locked, "#5affa0"], ["slewing", st.slewing, "#ff9628"], ["settling", st.settling, "#ffe150"],
                 ["idle", idle, "#3a4558"], ["safe hold", hold, "#ff4b4b"]];
  $("phasebar").replaceChildren(...parts.filter(p => p[1] > 0).map(([k, v, c]) =>
    el("div", { style: `flex:${v};background:${c}`, title: `${k}: ${v}` })));
  $("phaselegend").replaceChildren(...parts.map(([k, v, c]) => el("span", {}, el("i", { style: `background:${c}` }), `${k} ${v}`)));
  kvRows($("fleetkv"), [
    ["Shell", `${n.toLocaleString()} sats · ${s.shell.planes} planes · ${s.shell.alt_km.toFixed(0)} km · ${s.shell.inc_deg}°`],
    ["Orbital period", `${s.shell.period_min} min`],
    ["Sunlit / eclipse", `${st.lit} / ${st.eclipsed}`],
    ["Optical mesh", `${(s.shell.links - s.shell.links_blocked).toLocaleString()} links up` + (s.shell.links_blocked ? ` · ${s.shell.links_blocked} through atmosphere` : "")],
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
    $("shareval").textContent = s.mode === "queue" ? "100 % (public queue mode)" : `${Math.round(s.public_share * 100)} %`;
  }
  const tl = $("targetline");
  if (s.target && ["stack", "mosaic"].includes(s.mode)) {
    tl.replaceChildren(el("b", {}, s.target.name), " · ",
      s.target.blocked ? el("span", { class: "warn" }, s.target.blocked)
        : `${s.target.locked} locked of ${s.target.visible} with line of sight` + (s.mode === "stack" && s.target.locked ? ` · depth V ≈ ${s.stats.stack_lim_mag}` : ""));
  } else tl.textContent = s.mode === "queue" ? "All telescopes serve the public request queue." : s.mode === "survey" ? "Zenith-stare survey." : s.mode === "neo" ? "Near-Sun asteroid sweep." : "";

  // Alerts
  const alerts = [];
  if (hold) alerts.push(["warn", `${hold} satellite${hold > 1 ? "s" : ""} in safe hold`]);
  if (s.shell.links_blocked) alerts.push(["warn", `${s.shell.links_blocked} laser links pass through the atmosphere`]);
  if (s.target && s.target.blocked && ["stack", "mosaic"].includes(s.mode)) alerts.push(["warn", `Director target ${s.target.name}: ${s.target.blocked}`]);
  const open = s.requests.filter(r => ["pending", "approved", "observing"].includes(r.state));
  const pending = open.filter(r => r.state === "pending").length;
  if (pending) alerts.push(["info", `${pending} public request${pending > 1 ? "s" : ""} awaiting approval`]);
  const stuck = open.filter(r => r.waiting_s > 600);
  if (stuck.length) alerts.push(["warn", `${stuck.length} approved request${stuck.length > 1 ? "s" : ""} waiting > 10 min for telescopes`]);
  if (s.mode !== "queue" && s.public_share === 0 && open.length) alerts.push(["warn", "Public time share is 0 %: the queue is not being served"]);
  $("alerts").replaceChildren(...(alerts.length ? alerts.map(([l, m]) => el("li", { class: l }, m)) : [el("li", { class: "muted" }, "none")]));

  // Queue (don't rebuild while the director is typing in it)
  const done = s.requests.filter(r => r.state === "done").length;
  $("qsummary").textContent = `${open.length} open · ${pending} pending · ${done} done`;
  if (!$("queue").contains(document.activeElement)) {
    const q = $("queue"); q.replaceChildren();
    for (const r of s.requests.slice(0, 150)) q.append(reqRow(r));
    if (!q.children.length) q.append(el("tr", {}, el("td", { class: "muted small", colspan: 9 }, "No requests.")));
  }

  renderStares(s);
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
