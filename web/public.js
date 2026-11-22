"use strict";
const $ = (id) => document.getElementById(id);
const mineKey = "facet-my-requests";
const mine = () => { try { return JSON.parse(localStorage.getItem(mineKey) || "[]"); } catch { return []; } };
let last = null;
const viewer = new ResultViewer($("result"));
const streamViewer = new ResultViewer($("result"), "", "/api/campaign");
const mosaicViewer = new ResultViewer($("result"), "", "/api/mosaic");
const closeAll = () => { streamViewer.close(); viewer.close(); mosaicViewer.close(); };
const viewBtn = (r) => r.has_result ? el("button", { class: "view", onclick: () => { closeAll(); viewer.show(r.id); } }, r.state === "done" ? "Result" : "Live stack") : "";

// One row for a whole mosaic in "Your requests": overall progress, the
// stitched overview once every tile has data, and each tile's own result.
function mosaicRow(tiles) {
  const m = tiles[0].mosaic, req = tiles.reduce((a, r) => a + r.required_s, 0), got = tiles.reduce((a, r) => a + Math.min(r.progress_s, r.required_s), 0);
  const done = tiles.filter(r => r.state === "done").length, started = tiles.filter(r => r.has_result).length;
  const open = tiles.some(r => ["pending", "approved", "observing"].includes(r.state));
  const state = done === tiles.length ? "done" : open ? (tiles.some(r => r.state === "observing") ? "observing" : tiles[0].state) : tiles[0].state;
  const pct = Math.min(100, 100 * got / Math.max(1e-9, req));
  const sorted = [...tiles].sort((a, b) => a.mosaic.index - b.mosaic.index);
  return el("tr", {},
    el("td", {}, `#${m.group} ${m.object}`, el("div", { class: "small muted" }, `mosaic: ${tiles.length} tiles (${m.cols} × ${m.rows} grid), ${tiles[0].telescopes} telescopes each`)),
    el("td", {}, el("span", { class: `state ${state}` }, state), el("div", { class: "small muted" }, `${done}/${tiles.length} tiles done`)),
    el("td", {}, el("div", { class: "bar", title: `${pct.toFixed(1)}%` }, el("div", { style: `width:${pct}%` }))),
    el("td", {},
      started === tiles.length ? el("button", { class: "view", onclick: () => { closeAll(); mosaicViewer.show(m.group); } }, done === tiles.length ? "Mosaic" : "Live mosaic") : "",
      el("div", { class: "small" }, ...sorted.filter(r => r.has_result).map(r =>
        el("a", { href: "#", style: "margin-right:6px", onclick: (ev) => { ev.preventDefault(); closeAll(); viewer.show(r.id); } }, `tile ${r.mosaic.index + 1}`)))));
}

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
  $("estimate").replaceChildren(basisChip("depth"), " ",
    `Stacking ${n} × ${t.aperture_cm} cm telescopes for ${fmtDur(e)} each ≈ the light grasp of a ` +
    `${(t.aperture_cm * Math.sqrt(n) / 100).toFixed(2)} m aperture; estimated 5σ depth V ≈ ${lim.toFixed(1)} ` +
    `(dark sky, noise-limited; the result page computes it exactly). Field of view ${Number(t.fov_deg).toFixed(2)}° per telescope.`);
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
  const t = (l, v) => tileB(l, v, "defence");
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
    tileB("of the time FACET is observing", `${Math.round(100 * g.facet_duty)} %`, "duty"),
    tileB("for a telescope on the ground", `${Math.round(100 * g.ground_duty)} %`, "ground_equiv"),
    tileB("ground telescopes to match one FACET telescope", `≈ ${g.equivalent_ground_telescopes.toFixed(0)}`, "ground_equiv"));
}

function renderAllSky(s) {
  const a = s.allsky, c = $("allsky_card"); c.hidden = !a; if (!a) return;
  const n = (x) => Number(x).toLocaleString(undefined, { maximumFractionDigits: 0 });
  const t = (l, v) => el("div", { class: "tile" }, el("div", { class: "tv" }, v), el("div", { class: "tl" }, l));
  const [p60, p300, p3600] = a.passes;
  $("as_time").replaceChildren(fmtSpan(p60.seconds), " ", basisChip("allsky_estimate"));
  $("as_head").textContent = `for FACET's ${n(a.eyes)} telescopes to photograph every part of the sky that isn't hidden by the Sun or Moon right now: ${n(a.available_deg2)} square degrees, ${n(a.fields)} overlapping fields, one minute each, down to magnitude ${p60.depth_v.toFixed(1)}.`;
  $("as_tiles").replaceChildren(
    tileB("complete passes of the sky per day", n(a.per_day_60s), "allsky_estimate"),
    tileB(`to go deeper: 5 minutes per field (to mag ${p300.depth_v.toFixed(1)})`, fmtSpan(p300.seconds), "allsky_estimate"),
    tileB(`an hour per field (to mag ${p3600.depth_v.toFixed(1)})`, fmtSpan(p3600.seconds), "allsky_estimate"),
    tileB("of raw images per one-minute pass", `${a.tb_per_pass_60s.toFixed(1)} TB`, "allsky_data"));
  $("as_stages").replaceChildren(...a.stages.map(r => el("tr", r.n === s.shell.n ? { class: "me" } : {},
    el("td", {}, `${n(r.n)}${r.n === s.shell.n ? " (now)" : ""}`),
    el("td", { class: "num" }, fmtSpan(r.s60)),
    el("td", { class: "num" }, fmtSpan(r.s300)),
    el("td", { class: "num" }, fmtSpan(r.s3600)))));
  const sv = s.survey, m = $("as_meas");
  const last = sv && sv.history && sv.history.length ? sv.history[sv.history.length - 1] : null;
  m.hidden = !last;
  if (last) m.replaceChildren(basisChip("allsky_measured"), " ", `Measured in this simulation, not estimated: the fleet's last complete pass of the sky took ${fmtSpan(last.seconds)} (95 % of it in ${fmtSpan(last.t95)}), with ${Math.round(last.eyes_mean).toLocaleString()} telescopes working on average${sv.overhead_per_tile_s != null ? ` and ${Math.round(sv.overhead_per_tile_s)} s of slewing, settling and losses per field` : ""}.`);
  $("as_note").textContent = `How the estimate is worked out: fields × (exposure + ${a.overhead_s.toFixed(0)} s to slew and settle) ÷ (telescopes × the share of time they actually spend observing, ${Math.round(100 * a.duty)} %${a.duty_measured ? " as measured in this run, after slews, the South Atlantic Anomaly and Earthshine" : ", assumed until this run has measured it"}). It assumes the whole fleet is given to the job; with a typical ${Math.round(100 * a.survey_share)} % of telescopes surveying and the rest doing requested science, a one-minute pass takes ${fmtSpan(a.survey_share_60s)}. The part of the sky behind the Sun comes into view as the Earth orbits: the entire celestial sphere is covered within about ${Math.round(a.whole_sphere_days)} days. The Earth blocks each telescope's view but never the fleet's: every patch of sky is above the horizon for many satellites at once.`;
}

function renderPublic(s) {
  const p = s.public, c = $("pub_card"); c.hidden = !p || !p.on || !p.days_measured; if (c.hidden) return;
  const f = (x) => Math.round(x).toLocaleString();
  const t = (l, v) => el("div", { class: "tile" }, el("div", { class: "tv" }, v), el("div", { class: "tl" }, l));
  $("pub_tiles").replaceChildren(
    tileB("images for the public a year", p.images_per_year >= 1e6 ? `${(p.images_per_year / 1e6).toFixed(1)} million` : f(p.images_per_year), "public_service"),
    tileB("typical wait from request to image", p.median_wait_s == null ? "–" : fmtSpan(p.median_wait_s), "public_service"),
    tileB("classroom sessions a day", f(p.classroom_sessions_per_day), "public_service"),
    tileB("citizen-science measurements a day", f(p.citizen_measurements_per_day), "public_service"));
  $("pub_segs").replaceChildren(...p.segments.map(g => el("tr", {},
    el("td", {}, g.segment), el("td", { class: "num" }, f(g.submitted_per_day)), el("td", { class: "num" }, f(g.completed_per_day)))));
  $("pub_stages").replaceChildren(...p.stages.map(r => el("tr", r.n === s.shell.n ? { class: "me" } : {},
    el("td", {}, `${r.n.toLocaleString()}${r.n === s.shell.n ? " (now)" : ""}`), el("td", { class: "num" }, f(r.users_served)))));
  const live = p.live.submitted ? ` Separately, real visitors to this site have sent ${p.live.submitted} request${p.live.submitted === 1 ? "" : "s"} (${p.live.completed} delivered).` : "";
  $("pub_note").textContent = `The users are modelled: ${f(p.population)} people a month (an assumption), split between casual visitors, amateur astronomers, classrooms and citizen scientists with assumed habits. Their requests are real to the simulation: each is scheduled and observed by the fleet in the ${Math.round(100 * p.public_share)} % of telescope time set aside for the public, so the waits and deliveries are measured, not assumed. The public time is ${Math.round(100 * p.pool_utilisation)} % used; the last table scales that to each build-out stage at the same mix.${live}`;
}

function renderNeos(s) {
  const n = s.neos, c = $("neo_card"); c.hidden = !n; if (!n) return;
  const f = (x) => Number(x).toLocaleString();
  const t = (l, v) => el("div", { class: "tile" }, el("div", { class: "tv" }, v), el("div", { class: "tl" }, l));
  $("neo_tiles").replaceChildren(
    tileB("known near-Earth asteroids", f(n.known), "neo_catalogue"),
    tileB("potentially hazardous", f(n.pha), "neo_catalogue"),
    tileB(`observable by one FACET telescope right now (brighter than mag ${n.lim_1tel_60s.toFixed(1)} in one minute)`, f(n.visible_1tel), "neo_visible"),
    tileB(`observable with 16 telescopes stacked (to mag ${n.lim_16tel.toFixed(1)})`, f(n.visible_16tel), "neo_visible"));
  const size = (h) => Math.round(1329e3 / Math.sqrt(0.14) * Math.pow(10, -h / 5));
  $("neo_ca").replaceChildren(...n.approaches.slice(0, 8).map(a => el("tr", {},
    el("td", {}, a.name + (a.pha ? " (hazardous)" : "")),
    el("td", {}, a.utc.slice(0, 16)),
    el("td", { class: "num" }, `${a.dist_ld.toFixed(1)} lunar distances`),
    el("td", { class: "num" }, `mag ${a.peak_v.toFixed(1)}`),
    el("td", { class: "num" }, `~${size(a.h)} m`))));
  $("neo_note").textContent = `Orbits: the IAU Minor Planet Center's list of known near-Earth asteroids. Positions are computed in this simulation from each orbit without the planets' pull, so close-approach dates and distances are approximate (typically within hours and 10–20 %); sizes assume a typical reflectivity. ${n.bright_but_in_keepout.toLocaleString()} more asteroids are bright enough right now but too close to the Sun or Moon to observe.`;
}

let legendDone = false;
function renderLegend() {
  if (legendDone || !PROV) return; legendDone = true;
  $("basis_legend").replaceChildren(...PROV.bases.map(b => el("div", {}, basisChip0(b.basis), " ", b.meaning)));
}
function basisChip0(basis) { return el("span", { class: `basis b-${basis}` }, basis); }

function renderScience(s) {
  const sc = s.science; if (!sc) return;
  const d = sc.per_day, n = (x) => Number(x).toLocaleString(undefined, { maximumFractionDigits: 0 });
  const tile = (l, v) => el("div", { class: "tile" }, el("div", { class: "tv" }, v), el("div", { class: "tl" }, l));
  $("sci_since").textContent = `· ${fmtDur(sc.elapsed_s)} simulated`;
  $("sci_tiles").replaceChildren(
    tileB("observations completed per day", n(d.completed), "science"),
    tileB("finished in time to be useful", sc.on_time_pct === null ? "–" : `${sc.on_time_pct.toFixed(1)} %`, "science"),
    tileB("of the time working telescopes are observing", `${(100 * (sc.efficiency.integrating_pct - (sc.efficiency.earthshine_pct || 0)) / Math.max(1, 100 - sc.efficiency.hold_pct)).toFixed(0)} %`, "duty"),
    tileB("research teams served", `${sc.investigators_served}`, "science"));
  $("sci_cases").replaceChildren(...sc.cases.map(c => {
    const fin = c.completed + c.expired + (c.failed || 0);
    return el("tr", {}, el("td", {}, c.label, el("div", { class: "small muted" }, c.requirement)), el("td", { class: "num" }, n(c.completed)),
      el("td", { class: "num" }, fin ? `${(100 * c.on_time / fin).toFixed(0)} %` : "–"),
      el("td", { class: "num" }, c.median_response_s === null ? "–" : fmtDur(c.median_response_s)),
      el("td", { class: "num" }, c.median_precision_ppm != null ? `${Math.round(c.median_precision_ppm)} ppm per hour` : c.median_depth_mag === null ? "–" : `mag ${c.median_depth_mag.toFixed(1)}`));
  }));
}

function render(s) {
  last = s;
  if (s.provenance) { PROV = s.provenance; renderLegend(); }
  if (s.build) $("build").textContent = `build ${s.build}`;
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
  renderAllSky(s);
  renderNeos(s);
  renderPublic(s);
  const q = $("queue"); q.replaceChildren();
  const open = s.requests.filter(r => ["pending", "approved", "observing"].includes(r.state));
  const recent = s.requests.filter(r => r.state === "done").slice(0, 10);
  for (const r of [...open, ...recent].slice(0, 60)) {
    q.append(el("tr", { onmouseenter: () => { map.highlight = r.id; map.draw(); } },
      el("td", {}, r.id), el("td", {}, r.submitter), el("td", {}, r.target_name, r.moving ? el("div", { class: "small muted" }, trackNote(r.moving)) : ""),
      el("td", { class: "num" }, r.telescopes), el("td", { class: "num" }, fmtDur(r.exposure_s)),
      stateCell(r), progressCell(r), el("td", {}, viewBtn(r))));
  }
  if (!q.children.length) q.append(el("tr", {}, el("td", { class: "muted small", colspan: 8 }, "The queue is empty. Be the first!")));
  const ids = mine(), m = $("mine"); m.replaceChildren();
  const groups = new Map();
  for (const r of s.requests.filter(r => ids.includes(r.id))) {
    if (r.mosaic) {
      if (!groups.has(r.mosaic.group)) { groups.set(r.mosaic.group, []); m.append(el("tr", { "data-group": r.mosaic.group })); }
      groups.get(r.mosaic.group).push(r);
    } else m.append(el("tr", {}, el("td", {}, `#${r.id} ${r.target_name}`, r.moving ? el("div", { class: "small muted" }, trackNote(r.moving)) : ""), stateCell(r), progressCell(r), el("td", {}, viewBtn(r))));
  }
  for (const [g, tiles] of groups) m.querySelector(`tr[data-group="${g}"]`).replaceWith(mosaicRow(tiles));
  if (!m.children.length) m.append(el("tr", {}, el("td", { class: "muted small" }, "None yet from this browser.")));
  const streams = (s.campaigns || []).filter(c => c.active);
  $("streams").replaceChildren(...(streams.length ? streams.map(c => {
    const { any } = stareCoverage(c);
    return el("div", { class: "stare" },
      el("div", { class: "insphead" }, el("span", { class: `lamp ${c.paused || c.blocked ? "amber" : c.locked_now ? "green" : "grey"}` }),
        el("b", {}, c.target_name), el("span", { class: "muted small" }, c.paused ? " · paused" : ` · ${c.locked_now} telescopes watching · new frame every ${fmtDur(c.frame_s)}`)),
      el("div", { class: "small muted" }, c.paused ? `Paused: ${c.target_name} is too close to the ${String(c.blocked).startsWith("Sun") ? "Sun" : "Moon"} to observe safely. The stream resumes automatically.`
        : c.blocked ? c.blocked : any !== null ? `on target ${any.toFixed(2)} % of the time since ${c.started_utc.slice(0, 16)} UTC` : "acquiring…"),
      el("div", { class: "btns" }, c.has_frame ? el("button", { class: "view", onclick: () => { closeAll(); streamViewer.show(c.id); } }, "Watch") : ""));
  }) : [el("div", { class: "muted small" }, "No live streams right now.")]));
  estimate();
}

async function poll() {
  // Ask for this browser's own requests by id, so they stay listed (with
  // their results) after they finish, however busy the queue is.
  const ids = mine();
  try { const r = await fetch("/api/status" + (ids.length ? `?mine=${ids.join(",")}` : "")); if (r.ok) render(await r.json()); }
  catch { $("clock").textContent = "simulator offline"; }
}
setInterval(poll, 1000); poll();
["tel", "exp"].forEach(id => $(id).addEventListener("input", estimate));

$("form").addEventListener("submit", async (ev) => {
  ev.preventDefault();
  const body = { submitter: $("who").value, telescopes: +$("tel").value, exposure_s: +$("exp").value, note: $("note").value };
  if ($("mosaic").checked) body.mosaic = true;
  if ($("target").value.trim()) body.target = $("target").value.trim();
  else if ($("ra").value !== "" && $("dec").value !== "") { body.ra_deg = +$("ra").value; body.dec_deg = +$("dec").value; }
  const msg = $("msg");
  try {
    const r = await fetch("/api/request", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) });
    const j = await r.json();
    if (!r.ok) { msg.className = "msg small warn"; msg.textContent = j.error || "rejected"; return; }
    const ids = mine(); ids.push(...(j.ids || [j.id])); localStorage.setItem(mineKey, JSON.stringify(ids.slice(-100)));
    msg.className = "msg small";
    msg.textContent = j.mosaic
      ? `Mosaic #${j.id} submitted (${j.state}): ${j.mosaic.n} tiles on a ${j.mosaic.cols} × ${j.mosaic.rows} grid, ${j.mosaic.telescopes_per_tile} telescopes per tile.`
      : `Request #${j.id} submitted (${j.state}).`;
    poll();
  } catch { msg.className = "msg small warn"; msg.textContent = "Could not reach the simulator."; }
});
// Shareable link: /?result=<id>
{ const rid = new URLSearchParams(location.search).get("result"); if (rid) viewer.show(+rid); }
// Shareable mosaic link: /?mosaic=<group id>
{ const mid = new URLSearchParams(location.search).get("mosaic"); if (mid) mosaicViewer.show(+mid); }
// Shareable live-stream link: /?stream=<campaign id>
{ const sid = new URLSearchParams(location.search).get("stream"); if (sid) streamViewer.show(+sid); }
