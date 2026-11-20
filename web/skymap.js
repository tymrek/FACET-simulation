// All-sky map (Hammer–Aitoff, equatorial J2000), centred on the anti-Sun so
// the night sky sits in the middle. RA increases to the left, as on the sky.
"use strict";

const D2R = Math.PI / 180, R2D = 180 / Math.PI, SQ2 = Math.SQRT2;
const OBLIQ = 23.4393 * D2R;

function vec(ra, dec) {
  const r = ra * D2R, d = dec * D2R;
  return [Math.cos(d) * Math.cos(r), Math.cos(d) * Math.sin(r), Math.sin(d)];
}
function radec(v) {
  let ra = Math.atan2(v[1], v[0]) * R2D; if (ra < 0) ra += 360;
  return [ra, Math.asin(Math.max(-1, Math.min(1, v[2]))) * R2D];
}
function cross(a, b) { return [a[1]*b[2]-a[2]*b[1], a[2]*b[0]-a[0]*b[2], a[0]*b[1]-a[1]*b[0]]; }
function norm(a) { const n = Math.hypot(a[0], a[1], a[2]) || 1; return [a[0]/n, a[1]/n, a[2]/n]; }

class SkyMap {
  constructor(canvas, onClick) {
    this.c = canvas; this.ctx = canvas.getContext("2d");
    this.stars = []; this.status = null; this.centerRa = 0; this.highlight = null;
    this.onClick = onClick;
    canvas.addEventListener("click", (e) => {
      const p = this.unproject(e.offsetX * this.dpr, e.offsetY * this.dpr);
      if (p && this.onClick) this.onClick(p[0], p[1]);
    });
    canvas.addEventListener("mousemove", (e) => {
      const p = this.unproject(e.offsetX * this.dpr, e.offsetY * this.dpr);
      canvas.title = p ? `RA ${p[0].toFixed(2)}°  Dec ${p[1] >= 0 ? "+" : ""}${p[1].toFixed(2)}°` : "";
    });
    window.addEventListener("resize", () => this.draw());
  }
  setStars(s) { this.stars = s; this.draw(); }
  setStatus(s) {
    this.status = s;
    const sun = s.bodies && s.bodies[0];
    if (sun) this.centerRa = (sun.ra + 180) % 360;
    this.draw();
  }
  layout() {
    const dpr = window.devicePixelRatio || 1; this.dpr = dpr;
    const w = this.c.clientWidth * dpr, h = this.c.clientHeight * dpr;
    if (this.c.width !== w || this.c.height !== h) { this.c.width = w; this.c.height = h; }
    const m = 10 * dpr;
    this.s = Math.min((w - 2*m) / (4*SQ2), (h - 2*m) / (2*SQ2));
    this.cx = w / 2; this.cy = h / 2; this.w = w; this.h = h;
  }
  project(ra, dec) {
    let l = (ra - this.centerRa) * D2R;
    l = Math.atan2(Math.sin(l), Math.cos(l));
    const b = dec * D2R, z = Math.sqrt(1 + Math.cos(b) * Math.cos(l / 2));
    const x = 2 * SQ2 * Math.cos(b) * Math.sin(l / 2) / z, y = SQ2 * Math.sin(b) / z;
    return [this.cx - x * this.s, this.cy - y * this.s];
  }
  unproject(px, py) {
    const x = -(px - this.cx) / this.s, y = -(py - this.cy) / this.s;
    if ((x*x) / 8 + (y*y) / 2 > 1) return null;
    const z = Math.sqrt(1 - (x/4)**2 - (y/2)**2);
    const l = 2 * Math.atan2(z * x, 2 * (2*z*z - 1));
    const b = Math.asin(z * y);
    let ra = (l * R2D + this.centerRa) % 360; if (ra < 0) ra += 360;
    return [ra, b * R2D];
  }
  polyline(pts, style, width, dash) {
    const g = this.ctx; g.strokeStyle = style; g.lineWidth = width * this.dpr;
    g.setLineDash(dash ? dash.map(d => d * this.dpr) : []);
    g.beginPath(); let prev = null;
    for (const [ra, dec] of pts) {
      const p = this.project(ra, dec);
      if (prev && Math.abs(p[0] - prev[0]) < this.w / 3) g.lineTo(p[0], p[1]); else g.moveTo(p[0], p[1]);
      prev = p;
    }
    g.stroke(); g.setLineDash([]);
  }
  ring(ra, dec, rDeg, n = 120) {
    const c = vec(ra, dec), p = norm(Math.abs(c[2]) < 0.9 ? cross(c, [0,0,1]) : cross(c, [1,0,0])), q = cross(c, p);
    const r = rDeg * D2R, out = [];
    for (let k = 0; k <= n; k++) {
      const t = k / n * 2 * Math.PI;
      const v = [0,1,2].map(i => c[i]*Math.cos(r) + (p[i]*Math.cos(t) + q[i]*Math.sin(t)) * Math.sin(r));
      out.push(radec(v));
    }
    return out;
  }
  dot(ra, dec, r, fill) {
    const [x, y] = this.project(ra, dec), g = this.ctx;
    g.fillStyle = fill; g.beginPath(); g.arc(x, y, r * this.dpr, 0, 2*Math.PI); g.fill();
    return [x, y];
  }
  label(x, y, text, color, size = 11) {
    const g = this.ctx; g.fillStyle = color; g.font = `${size * this.dpr}px system-ui, sans-serif`;
    g.fillText(text, x + 6 * this.dpr, y - 5 * this.dpr);
  }
  draw() {
    this.layout(); const g = this.ctx, st = this.status;
    g.clearRect(0, 0, this.w, this.h);
    // Sky ellipse
    g.fillStyle = "#03050d"; g.beginPath();
    g.ellipse(this.cx, this.cy, 2*SQ2*this.s, SQ2*this.s, 0, 0, 2*Math.PI); g.fill();
    g.strokeStyle = "#1e2533"; g.lineWidth = this.dpr; g.stroke();
    // Grid
    for (let ra = 0; ra < 360; ra += 30) {
      const pts = []; for (let d = -88; d <= 88; d += 2) pts.push([ra, d]);
      this.polyline(pts, "rgba(90,120,170,0.22)", 1);
    }
    for (let d = -60; d <= 60; d += 30) {
      const pts = []; for (let k = 0; k <= 360; k += 2) pts.push([this.centerRa - 179.99 + k * 0.99997, d]);
      this.polyline(pts, d === 0 ? "rgba(110,160,230,0.5)" : "rgba(90,120,170,0.22)", 1);
    }
    for (let h = 0; h < 24; h += 2) {
      const [x, y] = this.project(h * 15, 0); this.label(x - 10*this.dpr, y + 16*this.dpr, `${h}h`, "rgba(140,160,200,0.6)", 10);
    }
    // Ecliptic
    const ecl = [];
    for (let k = 0; k <= 360; k += 2) {
      const l = k * D2R; ecl.push(radec([Math.cos(l), Math.sin(l) * Math.cos(OBLIQ), Math.sin(l) * Math.sin(OBLIQ)]));
    }
    ecl.sort((a, b) => ((a[0] - this.centerRa + 540) % 360) - ((b[0] - this.centerRa + 540) % 360));
    this.polyline(ecl, "rgba(220,180,70,0.45)", 1.2, [5, 4]);
    // Stars
    for (const [ra, dec, mag, ci] of this.stars) {
      const r = Math.max(0.5, (5.2 - mag) * 0.45);
      const t = Math.max(0, Math.min(1, (ci + 0.3) / 1.9));
      const col = `rgba(${200 + 55*t | 0},${215 - 40*t | 0},${255 - 120*t | 0},${Math.max(0.35, Math.min(1, (5.5 - mag) / 3))})`;
      this.dot(ra, dec, r, col);
    }
    if (!st) return;
    const tel = st.telescope || {};
    const sun = st.bodies[0], moon = st.bodies[1];
    // Keep-out zones
    const zone = (b, r, rgb) => {
      const pts = this.ring(b.ra, b.dec, r);
      this.polyline(pts, `rgba(${rgb},0.8)`, 1.4);
    };
    zone(sun, tel.sun_excl_deg, "255,170,40");
    zone(moon, tel.moon_excl_deg, "180,180,200");
    if (st.mode === "neo") this.polyline(this.ring(sun.ra, sun.dec, Math.max(tel.neo_elong_deg, tel.sun_excl_deg)), "rgba(255,120,60,0.7)", 1.2, [4, 4]);
    // Telescope pointings
    for (const [ra, dec, code] of st.footprints || []) {
      this.dot(ra, dec, code === 2 ? 1.3 : 1.6,
        code === 2 ? "rgba(255,150,40,0.85)" : code ? "rgba(90,255,150,0.75)" : "rgba(120,200,255,0.75)");
    }
    // Bodies
    const colors = ["#ffe178", "#d7d7cd", "#beb4aa", "#fff5d7", "#ff8250", "#ebcda0", "#f0d782", "#96e1eb", "#6e96ff"];
    st.bodies.forEach((b, i) => {
      const [x, y] = this.dot(b.ra, b.dec, i === 0 ? 7 : i === 1 ? 5 : 3, colors[i]);
      this.label(x, y, b.name, colors[i]);
    });
    // Requests
    for (const r of st.requests || []) {
      if (!["pending", "approved", "observing"].includes(r.state)) continue;
      const col = r.state === "observing" ? "#5affa0" : r.state === "approved" ? "#63c8ff" : "#ffd166";
      const [x, y] = this.project(r.ra_deg, r.dec_deg);
      // Programme requests are many: small, unlabelled (ToOs and the hovered one excepted).
      const prog = !!r.programme, hl = this.highlight === r.id, loud = !prog || hl || r.priority >= 9;
      const s = (hl ? 7 : prog && !loud ? 2.5 : 4.5) * this.dpr;
      g.strokeStyle = col; g.lineWidth = (prog && !loud ? 1 : 1.5) * this.dpr;
      g.globalAlpha = prog && !loud ? 0.6 : 1;
      g.beginPath(); g.moveTo(x, y - s); g.lineTo(x + s, y); g.lineTo(x, y + s); g.lineTo(x - s, y); g.closePath(); g.stroke();
      g.globalAlpha = 1;
      if (loud) this.label(x, y, `#${r.id}`, col, 10);
    }
    // Planetary-defence object (once operators know about it)
    const pd = st.defence;
    if (pd && pd.position_known && pd.state !== "impacted") {
      const [x, y] = this.project(pd.now.ra_deg, pd.now.dec_deg);
      g.strokeStyle = "#ff4d4d"; g.lineWidth = 2 * this.dpr; const r = 9 * this.dpr;
      g.beginPath(); g.moveTo(x, y - r); g.lineTo(x + r, y + r * 0.8); g.lineTo(x - r, y + r * 0.8); g.closePath(); g.stroke();
      this.label(x + 12 * this.dpr, y - 10 * this.dpr, `${pd.designation.split(" (")[0]} · IP ${pd.systems[0].ip == null ? "?" : (pd.systems[0].ip * 100).toFixed(pd.systems[0].ip > 0.1 ? 0 : 2) + "%"}`, "#ff4d4d", 11);
    }
    // STARE campaigns
    for (const c of st.campaigns || []) {
      if (!c.active) continue;
      const [x, y] = this.project(c.ra_deg, c.dec_deg);
      g.strokeStyle = c.blocked ? "#ff7a5c" : "#4de8ff"; g.lineWidth = 1.5 * this.dpr;
      g.beginPath(); g.arc(x, y, 12 * this.dpr, 0, 2 * Math.PI); g.stroke();
      g.beginPath(); g.arc(x, y, 16 * this.dpr, 0, 2 * Math.PI); g.stroke();
      this.label(x + 10 * this.dpr, y + 18 * this.dpr, `STARE #${c.id} · ${c.locked_now}/${c.eyes}`, g.strokeStyle, 10);
    }
    // Director's target
    if (st.target && st.mode !== "queue" && st.mode !== "survey" && st.mode !== "neo") {
      const [x, y] = this.project(st.target.ra, st.target.dec);
      g.strokeStyle = st.target.blocked ? "#ff7a5c" : "#5affa0"; g.lineWidth = 1.5 * this.dpr;
      g.beginPath(); g.arc(x, y, 9 * this.dpr, 0, 2*Math.PI); g.stroke();
      this.label(x + 6*this.dpr, y + 14*this.dpr, st.target.name, g.strokeStyle);
    }
  }
}

// Small shared helpers for the two pages.
function el(tag, attrs = {}, ...kids) {
  const e = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (k === "class") e.className = v; else if (k.startsWith("on")) e.addEventListener(k.slice(2), v); else e.setAttribute(k, v);
  }
  for (const k of kids) e.append(k instanceof Node ? k : document.createTextNode(String(k)));
  return e;
}
// Provenance chip for a headline: simulated / modelled / assumed / data,
// with what the figure rests on as a tooltip. Registry comes from /api/status.
let PROV = null;
function basisChip(key) {
  const it = PROV && PROV.items && PROV.items[key];
  if (!it) return "";
  const m = (PROV.bases.find(b => b.basis === it.basis) || {}).meaning || "";
  return el("span", { class: `basis b-${it.basis}`, title: `${m}\n\n${it.rests_on}` }, it.basis);
}
function tileB(label, value, key) {
  return el("div", { class: "tile" }, el("div", { class: "tv" }, value), el("div", { class: "tl" }, label, " ", basisChip(key)));
}
function fmtSpan(s) {
  if (s < 90) return `${Math.round(s)} s`;
  if (s < 5400) return `${Math.round(s / 60)} min`;
  if (s < 2 * 86400) return `${(s / 3600).toFixed(1)} h`;
  return `${(s / 86400).toFixed(1)} days`;
}
function fmtDur(s) {
  if (s < 120) return `${s.toFixed(0)} s`;
  if (s < 7200) return `${(s/60).toFixed(1)} min`;
  if (s < 3*86400) return `${(s/3600).toFixed(1)} h`;
  return `${(s/86400).toFixed(1)} d`;
}
function progressCell(r) {
  const pct = Math.min(100, 100 * r.progress_s / Math.max(1e-9, r.required_s));
  return el("td", {}, el("div", { class: "bar", title: `${pct.toFixed(1)}%` }, el("div", { style: `width:${pct}%` })));
}
function stateCell(r) {
  const s = el("span", { class: `state ${r.state}` }, r.state);
  const td = el("td", {}, s);
  if (r.blocked && (r.state === "approved" || r.state === "observing")) td.append(el("div", { class: "small warn" }, r.blocked));
  else if (r.state === "observing") {
    const slewing = r.assigned_now - r.locked_now;
    td.append(el("div", { class: "small muted" }, `${r.locked_now} locked` + (slewing > 0 ? `, ${slewing} slewing` : "")));
  }
  if (r.response_s !== null && r.response_s !== undefined)
    td.append(el("div", { class: "small muted" }, `first light +${fmtDur(r.response_s)}` + (r.full_lock_s != null ? `, full +${fmtDur(r.full_lock_s)}` : "")));
  else if (r.waiting_s !== null && r.waiting_s !== undefined) td.append(el("div", { class: "small muted" }, `waiting ${fmtDur(r.waiting_s)}`));
  return td;
}

// Pointing-error trace (log scale) for a result report.
function traceCanvas(tr, ag) {
  const c = el("canvas", { class: "trace" });
  requestAnimationFrame(() => {
    const dpr = window.devicePixelRatio || 1, w = c.clientWidth * dpr, h = c.clientHeight * dpr;
    c.width = w; c.height = h; const g = c.getContext("2d");
    const m = { l: 44 * dpr, r: 8 * dpr, t: 8 * dpr, b: 22 * dpr };
    const tmax = tr[tr.length - 1][0], ymin = 0.1, ymax = Math.max(1e4, ...tr.map(p => p[1]));
    const X = t => m.l + (w - m.l - m.r) * t / tmax;
    const Y = e => m.t + (h - m.t - m.b) * (1 - (Math.log10(Math.max(e, ymin)) - Math.log10(ymin)) / (Math.log10(ymax) - Math.log10(ymin)));
    g.font = `${10 * dpr}px system-ui`; g.fillStyle = "#8590a6"; g.strokeStyle = "#1e2533"; g.lineWidth = dpr;
    for (let d = -1; d <= Math.log10(ymax); d++) {
      const y = Y(10 ** d); g.beginPath(); g.moveTo(m.l, y); g.lineTo(w - m.r, y); g.stroke();
      g.fillText(d >= 4 ? `${(10 ** d / 3600).toFixed(0)}°` : `${10 ** d}″`, 2 * dpr, y + 3 * dpr);
    }
    if (ag) {
      const phases = [[0, ag.slew_s, "slew", "rgba(255,150,40,0.10)"], [ag.slew_s, ag.slew_s + ag.settle_s, "settle", "rgba(255,225,80,0.10)"],
                      [ag.slew_s + ag.settle_s, ag.repoint_s, "acquire", "rgba(99,200,255,0.10)"], [ag.repoint_s, tmax, "locked", "rgba(90,255,150,0.07)"]];
      for (const [a, b, name, col] of phases) {
        g.fillStyle = col; g.fillRect(X(a), m.t, X(b) - X(a), h - m.t - m.b);
        if (X(b) - X(a) > 34 * dpr) { g.fillStyle = "#8590a6"; g.fillText(name, X(a) + 3 * dpr, h - 8 * dpr); }
      }
    }
    g.strokeStyle = "#5affa0"; g.lineWidth = 1.2 * dpr; g.beginPath();
    tr.forEach(([t, e], k) => k ? g.lineTo(X(t), Y(e)) : g.moveTo(X(t), Y(e))); g.stroke();
  });
  return c;
}

// ---------------------------------------------------------------- results
// Render an observation result (report + images) into `box`.
// `q` is an optional query string (e.g. "?token=…") for the director port.
class ResultViewer {
  // base: "/api/result" for requests, "/api/campaign" for STARE live frames.
  constructor(box, q = "", base = "/api/result") { this.box = box; this.q = q; this.base = base; this.id = null; this.timer = null; }
  async show(id) {
    this.id = id; clearInterval(this.timer);
    await this.load();
    this.timer = setInterval(() => this.load(), 10000);
    this.box.scrollIntoView({ behavior: "smooth", block: "start" });
  }
  close() { clearInterval(this.timer); this.id = null; this.box.replaceChildren(); this.box.hidden = true; }
  async load() {
    if (this.id == null) return;
    const id = this.id, q = this.q;
    let r;
    try {
      const res = await fetch(`${this.base}/${id}${q}`);
      r = await res.json();
      if (!res.ok) { this.box.hidden = false; this.box.replaceChildren(el("h2", {}, `Request #${id}`), el("p", { class: "muted" }, r.error)); return; }
    } catch { return; }
    if (r.final && this.base === "/api/result") clearInterval(this.timer);
    const bust = `${q ? "&" : "?"}t=${Date.now()}`;
    const kv = (rows) => el("table", { class: "kv" }, ...rows.filter(([, v]) => v !== null && v !== undefined && v !== "")
      .map(([k, v]) => el("tr", {}, el("th", {}, k), el("td", {}, v))));
    const h = r.hardware, x = r.exposure, s = r.sky, m = r.results, im = r.images, op = r.optics;
    const fig = (src, cap) => el("figure", {}, el("img", { src, alt: cap, loading: "lazy" }), el("figcaption", { class: "small muted" }, cap));
    this.box.hidden = false;
    this.box.replaceChildren(
      el("div", { class: "rhead" },
        el("h2", {}, this.base === "/api/campaign" ? `${r.target} · latest frame (${r.utc.slice(11)})` : `Request #${r.id}: ${r.target} ${r.final ? "· final" : "· live stack"}`),
        el("button", { onclick: () => this.close() }, "Close")),
      el("div", { class: "figs" },
        fig(`${this.base}/${id}/full.png${q}${bust}`, `Full field ${Number(h.fov_deg).toFixed(2)}° · ${im.full.arcsec_per_px}″/px (binned ×${im.full.bin}) · ${im.orientation}`),
        fig(`${this.base}/${id}/crop.png${q}${bust}`, `Centre ${im.crop.width_arcmin}′ · ${im.crop.arcsec_per_px}″/px (${im.crop.resampled})`)),
      el("div", { class: "kvs" },
        el("div", {}, el("h3", {}, "Result"), kv([
          ["5σ point-source limit", `V ≈ ${m.lim_mag_5sigma}`],
          ["1σ surface brightness (1″²)", `${m.sb_lim_1sigma_mag_arcsec2} mag/arcsec²`],
          ["Delivered FWHM", `${m.fwhm_arcsec}″ (diffraction ${m.diffraction_fwhm_arcsec}″) · ${m.fwhm_px} px, ${m.sampling}`],
          ["Sources ≥ 5σ in field", `${m.sources_above_5sigma.toLocaleString()} (${m.galaxies_above_5sigma.toLocaleString()} galaxies)`],
          ["Target magnitude", m.target_mag !== null ? `V ${m.target_mag}` : null],
          ["Peak SNR on target", m.target_peak_snr !== null ? m.target_peak_snr.toLocaleString() : null],
          ["Angular diameter", m.angular_diameter_arcsec !== null ? `${m.angular_diameter_arcsec}″` : null],
          ["Phase angle", m.phase_angle_deg !== null ? `${m.phase_angle_deg}°` : null],
          ["Saturn ring tilt (to Earth)", m.saturn_ring_tilt_deg !== null ? `${m.saturn_ring_tilt_deg}°` : null],
        ])),
        el("div", {}, el("h3", {}, "Exposure"), kv([
          ["Integration", `${x.telescope_seconds.toLocaleString()} of ${x.telescope_seconds_required.toLocaleString()} telescope-seconds`],
          ["= one telescope for", `${x.single_scope_equivalent_h} h`],
          ["Light grasp if simultaneous", `${x.equivalent_aperture_cm_if_simultaneous} cm aperture`],
          ["Sub-frames", `${x.frames.toLocaleString()} × ${x.sub_frame_s < 1 ? (x.sub_frame_s * 1000).toFixed(1) + " ms" : x.sub_frame_s + " s"}`],
          ["Sky (zodiacal light)", `${s.zodiacal_mag_arcsec2} mag/arcsec² · elongation ${s.solar_elongation_deg}° · b ${s.galactic_latitude_deg}°`],
          ["Observed", r.utc],
        ])),
        el("div", {}, el("h3", {}, "Pointing"), kv(this.base === "/api/campaign" ? [
          ["Eyes in this frame", `${h.telescopes_requested ?? x.telescope_seconds / x.per_telescope_requested_s}`],
          ["Handovers so far", `${r.pointing.repoints}`],
          ["Agility", r.pointing.agility ? `α ${r.pointing.agility.alpha_dps2}°/s², ω ${r.pointing.agility.omega_dps}°/s` : null],
          ["Effective jitter", `${r.pointing.effective_jitter_arcsec_rms}″ RMS`],
          ["Fine-guided", `${Math.round(r.pointing.fine_guided_fraction * 100)} % of integration`],
        ] : [
          ["Response (approval → first light)", r.pointing.response_s !== null ? fmtDur(r.pointing.response_s) : null],
          ["All requested telescopes locked", r.pointing.full_lock_s !== null ? `after ${fmtDur(r.pointing.full_lock_s)}` : "not reached (fewer could see the target at once)"],
          ["Repoints for this request", `${r.pointing.repoints} (mean slew ${r.pointing.mean_slew_deg}°)`],
          ["Repoint time (mean slew)", r.pointing.agility ? `${r.pointing.agility.repoint_s} s = slew ${r.pointing.agility.slew_s} + settle ${r.pointing.agility.settle_s} + acquire ${r.pointing.agility.acquire_s}` : null],
          ["Agility", r.pointing.agility ? `α ${r.pointing.agility.alpha_dps2}°/s², ω ${r.pointing.agility.omega_dps}°/s` : null],
          ["Effective jitter", `${r.pointing.effective_jitter_arcsec_rms}″ RMS`],
          ["Fine-guided", `${Math.round(r.pointing.fine_guided_fraction * 100)} % of integration`],
        ]), r.pointing.trace && this.base !== "/api/campaign" ? traceCanvas(r.pointing.trace, r.pointing.agility) : "",
          this.base !== "/api/campaign" ? el("div", { class: "small muted" }, "Typical repoint: pointing error vs time") : ""),
        op ? el("div", {}, el("h3", {}, "Optics"), kv([
          ["Strehl ratio", `${op.strehl}`],
          ["FWHM optics / delivered", `${op.fwhm_optics_arcsec}″ / ${op.fwhm_delivered_arcsec}″`],
          ["Encircled energy", `50 % within ${op.ee50_radius_arcsec}″ · 80 % within ${op.ee80_radius_arcsec}″`],
        ]), el("figure", { class: "psf" }, el("img", { src: `${this.base}/${id}/psf.png${q}${bust}`, alt: "PSF" }),
              el("figcaption", { class: "small muted" }, `Effective PSF, log scale, ${op.psf_field_arcsec}″ field`)),
          el("div", { class: "small muted" }, op.model)) : "",
        el("div", {}, el("h3", {}, "Hardware (per telescope)"), kv([
          ["Telescopes requested", h.telescopes_requested],
          ["Aperture", `${h.aperture_cm} cm, ${Math.round(h.obstruction * 100)} % obstruction, f/${h.f_ratio} (${h.focal_mm} mm)`],
          ["Detector", h.sensor],
          ["Pixel scale / FOV", `${Number(h.pixel_scale_arcsec).toFixed(3)}″ · ${Number(h.fov_deg).toFixed(2)}°`],
          ["QE × optics", `${h.qe} × ${h.throughput}`],
          ["Read noise / dark", `${h.read_noise_e} e⁻ · ${h.dark_e_s} e⁻/s`],
          ["Pointing jitter", `${Number(h.jitter_arcsec_rms).toFixed(2)}″ RMS`],
        ]))),
      el("ul", { class: "note" }, ...r.notes.map(n => el("li", {}, n))),
    );
  }
}

function stareCoverage(c) {
  const full = c.eligible_s > 0 ? (100 * c.full_s / c.eligible_s) : null;
  const any = c.eligible_s > 0 ? (100 * c.any_s / c.eligible_s) : null;
  return { full, any };
}
