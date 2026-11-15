"use strict";
const $ = (id) => document.getElementById(id);
const token = new URLSearchParams(location.search).get("token") || "";
const H = { "X-Facet-Token": token };
let last = null, formsLoaded = false;

function flash(text, bad) { const m = $("msg"); m.className = "msg small" + (bad ? " warn" : ""); m.textContent = text; }

async function cmd(body) {
  try {
    const r = await fetch("/api/cmd", { method: "POST", headers: { ...H, "Content-Type": "application/json" }, body: JSON.stringify(body) });
    const j = await r.json();
    if (!r.ok) flash(j.error || "command failed", true); else flash("ok");
    poll();
  } catch { flash("simulator offline", true); }
}

const map = new SkyMap($("sky"), (ra, dec) => cmd({ cmd: "target", ra_deg: ra, dec_deg: dec }));
fetch("/api/stars", { headers: H }).then(r => r.ok ? r.json() : Promise.reject()).then(s => map.setStars(s))
  .catch(() => flash("Not authorised. Open the console with the ?token=… URL the simulator printed.", true));

// Log-scale satellite slider: 2 … 10 000
const nFromSlider = (v) => Math.round(2 * Math.pow(5000, v / 1000));
const sliderFromN = (n) => Math.round(1000 * Math.log(n / 2) / Math.log(5000));
$("n").addEventListener("input", () => $("nval").textContent = nFromSlider(+$("n").value).toLocaleString());
$("n").addEventListener("change", () => cmd({ cmd: "shell", n: nFromSlider(+$("n").value) }));
for (const n of [2, 10, 50, 100, 500, 1000, 5000, 10000])
  $("stages").append(el("button", { onclick: () => cmd({ cmd: "shell", n }) }, n.toLocaleString()));

for (const [label, rate] of [["Pause", 0], ["1×", 1], ["60×", 60], ["600×", 600], ["1 h/s", 3600], ["1 d/s", 86400]])
  $("timebtns").append(el("button", { "data-rate": rate, onclick: () => cmd(rate === 0 ? { cmd: "time", playing: false } : { cmd: "time", rate, playing: true }) }, label));
$("timebtns").append(el("button", { onclick: () => cmd({ cmd: "time", now: true }) }, "Now"));

const modes = [["survey", "Survey"], ["stack", "Stack"], ["mosaic", "Mosaic"], ["neo", "NEO sweep"], ["queue", "Queue"]];
for (const [k, label] of modes) $("modebtns").append(el("button", { "data-mode": k, onclick: () => cmd({ cmd: "mode", mode: k }) }, label));

$("share").addEventListener("input", () => $("shareval").textContent = `${$("share").value} %`);
$("share").addEventListener("change", () => cmd({ cmd: "public_share", share: +$("share").value / 100 }));
$("settarget").addEventListener("click", () => cmd({ cmd: "target", name: $("target").value }));
$("auto").addEventListener("change", () => cmd({ cmd: "auto_approve", on: $("auto").checked }));
$("applyshell").addEventListener("click", () => {
  const b = { cmd: "shell", alt_km: +$("alt").value, planes: +$("planes").value, phasing: +$("phasing").value, sso: $("sso").checked };
  if (!$("sso").checked) b.inc_deg = +$("inc").value;
  cmd(b);
});
const telKeys = ["aperture_cm", "fov_deg", "sun_excl_deg", "moon_excl_deg", "limb_margin_deg", "neo_elong_deg", "jitter_arcsec", "read_noise_e", "sub_max_s",
                 "inertia_kgm2", "wheel_torque_nm", "rate_limit_dps", "settle_tau_s", "acquire_s", "coarse_jitter_arcsec", "guide_mag"];
const viewer = new ResultViewer($("result"), `?token=${encodeURIComponent(token)}`);
$("applytel").addEventListener("click", () => {
  const b = { cmd: "telescope" }; for (const k of telKeys) b[k] = +$(k).value; cmd(b);
});

function stat(k, v) { return el("div", { class: "stat" }, el("div", { class: "k" }, k), el("div", { class: "v" }, v)); }

function reqRow(r) {
  const prio = el("input", { type: "number", value: r.priority, min: -100, max: 100, style: "width:58px" });
  const tel = el("input", { type: "number", value: r.telescopes, min: 1, max: 10000, style: "width:70px" });
  const act = (action) => cmd({ cmd: "request", id: r.id, action, priority: +prio.value, telescopes: +tel.value });
  const btns = el("td", {});
  if (r.state === "pending") btns.append(el("button", { class: "primary", onclick: () => act("approve") }, "Approve"), " ",
                                         el("button", { class: "danger", onclick: () => act("reject") }, "Reject"));
  else if (["approved", "observing"].includes(r.state)) btns.append(el("button", { onclick: () => act("update") }, "Update"), " ",
                                         el("button", { class: "danger", onclick: () => act("cancel") }, "Cancel"));
  else btns.append(el("button", { onclick: () => act("requeue") }, "Re-queue"));
  if (r.has_result) btns.append(" ", el("button", { class: "view", onclick: () => viewer.show(r.id) }, r.state === "done" ? "Result" : "Live"));
  const who = el("td", {}, r.submitter); who.title = `${r.ip} · submitted ${r.submitted_utc}${r.note ? "\n" + r.note : ""}`;
  return el("tr", { onmouseenter: () => { map.highlight = r.id; map.draw(); } },
    el("td", {}, r.id), who, el("td", {}, r.target_name, r.note ? el("div", { class: "small muted" }, r.note) : ""),
    el("td", { class: "num" }, tel), el("td", { class: "num" }, fmtDur(r.exposure_s)),
    el("td", { class: "num" }, prio), stateCell(r), progressCell(r), btns);
}

function render(s) {
  last = s;
  $("clock").textContent = `${s.utc} · ${s.playing ? `${s.rate}×` : "paused"}`;
  $("modelabel").textContent = s.mode_label;
  map.setStatus(s);
  document.querySelectorAll("#modebtns button").forEach(b => b.classList.toggle("sel", b.dataset.mode === s.mode));
  document.querySelectorAll("#timebtns button[data-rate]").forEach(b =>
    b.classList.toggle("sel", (+b.dataset.rate === 0 && !s.playing) || (s.playing && +b.dataset.rate === s.rate)));
  $("auto").checked = s.auto_approve;
  if (document.activeElement !== $("share")) {
    $("share").value = Math.round(s.public_share * 100); $("share").disabled = s.mode === "queue";
    $("shareval").textContent = `${Math.round(s.public_share * 100)} %`;
  }
  if (document.activeElement !== $("n")) { $("n").value = sliderFromN(s.shell.n); $("nval").textContent = s.shell.n.toLocaleString(); }
  if (!formsLoaded) {
    $("alt").value = s.shell.alt_km; $("inc").value = s.shell.inc_deg; $("planes").value = s.shell.planes_setting;
    $("phasing").value = s.shell.phasing; $("sso").checked = s.shell.sso;
    for (const k of telKeys) $(k).value = s.telescope[k];
    formsLoaded = true;
  }
  const t = s.telescope;
  $("agilityinfo").textContent = `α ${t.alpha_dps2}°/s², ω ${t.omega_dps}°/s · repoint 10°: ${t.repoint_10_s} s, 90°: ${t.repoint_90_s} s (slew + settle + guide lock)`;
  $("shellinfo").textContent = `${s.shell.planes} planes · period ${s.shell.period_min} min · node drift ${s.shell.node_drift_deg_day}°/day · ` +
    `${s.shell.links} laser links (${s.shell.links_blocked} through atmosphere)`;
  const open = s.requests.filter(r => ["pending", "approved", "observing"].includes(r.state));
  const busyQ = open.reduce((a, r) => a + r.assigned_now, 0);
  $("stats").replaceChildren(
    stat("Satellites", s.shell.n.toLocaleString()),
    stat("Locked / slewing / settling", `${s.stats.locked} / ${s.stats.slewing} / ${s.stats.settling}`),
    stat("Sunlit / eclipsed", `${s.stats.lit} / ${s.stats.eclipsed}`),
    stat("Sky reachable", `${s.stats.sky_reachable}%`),
    stat("Open requests", `${open.length} (${open.filter(r => r.state === "pending").length} pending)`),
    stat("On queue now", `${busyQ} telescopes`),
    ...(["stack", "mosaic"].includes(s.mode) && s.target ? [
      stat("Target", `${s.target.name}: ${s.target.blocked || `${s.target.locked} locked of ${s.target.visible}`}`),
      stat("Stacked depth (est.)", s.target.visible ? `V ≈ ${s.stats.stack_lim_mag}` : "—"),
    ] : []),
  );
  // Don't rebuild the table while the director is typing in it.
  if ($("queue").contains(document.activeElement)) return;
  const q = $("queue"); q.replaceChildren();
  for (const r of s.requests.slice(0, 120)) q.append(reqRow(r));
  if (!q.children.length) q.append(el("tr", {}, el("td", { class: "muted small", colspan: 9 }, "No requests yet.")));
}

async function poll() {
  try {
    const r = await fetch("/api/status", { headers: H });
    if (r.status === 401) { flash("Not authorised: wrong or missing token.", true); return; }
    if (r.ok) render(await r.json());
  } catch { $("clock").textContent = "simulator offline"; }
}
setInterval(poll, 1000); poll();
{ const rid = new URLSearchParams(location.search).get("result"); if (rid) viewer.show(+rid); }
