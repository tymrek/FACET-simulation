# Changelog

## Unreleased

All-sky survey: one faster tiling pattern, and a fairer end of pass.

- SAA first: the sky overhead the South Atlantic Anomaly (reachable only
  from outside it, looking sideways) was 80-90 % of the last 5 % of tiles
  at 10,000 telescopes. Every free telescope that can see an undone tile of
  that sky now takes the cheapest, at any slew.
- Sunward first: tiles within 35 deg outside the Sun keep-out are preferred
  wherever one is in reach.
- Tail stacking: once 2,000 or fewer tiles remain, idle telescopes join the
  tile they can bring forward most; exposures are co-added.
- Pass completion: a tile hidden by the Sun or Moon keep-out at any check
  during the pass no longer holds the pass open (each tile the moving Moon
  uncovered at the end needed a full exposure with the fleet idle). Near
  the end the check runs every 5 s, not every minute.
- Polar duty removed (neutral at 1,000, slower at 10,000); `--survey-polar`
  is ignored with a warning.
- Measured (60x, dedicated fleet), pass / 95 %: 10,000 telescopes 8.7 / 6.5
  -> 8.0 / 6.7 min; 1,000 telescopes 69.3 / 64.0 -> 67.6 / 63.5 min.
- Gap-free tiling: survey tiles, public mosaics and NEO recovery searches
  were laid out for a square frame of the same area, but the sensor frame
  is 3:2 (1.29 x 0.86 deg): rows 1.0 deg apart under a 0.86 deg frame would
  have left 11-14 % of the sky in gaps. All three now tile the real frame,
  long side east-west. The survey overlaps >= 5 % on each axis (seams
  >= ~310 x 450 px), adds tiles near the poles until each ring's edges are
  covered (41,596 tiles, was 41,172), and a test checks 60,000 sky points.
  Mosaics keep 10 % overlap (M31 is now 8 tiles on 2 x 4, was 6 on 2 x 3).
  Pass times with the new tiles: 68.4 / 64.4 min (1,000), 8.1 / 6.8 min
  (10,000).
- Pass records add tail_in_saa, tail_in_sun, SAA-sky tile completion times
  and stack_joins; status adds saa_sky_tiles and saa_sky_left.

## 0.7.2 (2026-11-23)

Queue allocator stays fast as the science-programme backlog fills. At
10,000 satellites and 600x the backlog reaches its 5,000-request cap within
minutes of wall time, about 1,000 of them inside open event windows. On one
thread the allocator grew from 36 to 150 ms per call and the whole tick
from ~330 to ~450 ms; it now stays flat at ~200-220 ms.

- Event-window (continuous) requests evaluate the loss-of-target time
  lazily, in slew-cost order, stopping once no remaining candidate can beat
  the best found. The penalty only adds cost, so the choice is unchanged.
- Background programme requests outside a window look for extra
  telescopes every fourth allocation, staggered by request id; telescopes
  they already hold are kept every step.
- Cheaper bookkeeping: taken telescopes are swept from the pool once 256
  have piled up, and the protected set is built in one pass.
- Scheduling results unchanged: over 25 simulated hours at 10,000
  satellites, transits 19,872 of 19,878 and occultations 3,118 of 3,118
  met (before: 21,182 of 21,186 and 3,319 of 3,319); at 1,000, transits
  1,794 of 1,794 and occultations 279 of 280.

## 0.7.1 (2026-11-23)

Faster simulation steps at large fleets and high time warp (found while
profiling the browser build). With 10,000 satellites at 60x on one thread
the mean step fell from 43.7 to 31.3 ms and the ~90 ms stalls are gone.

- SAA look-ahead (used for handover): refreshed round-robin so each
  satellite is recomputed every 15 s of simulated time without stalling a
  step, and evaluated against a cached 1-degree map of the above-threshold
  region (predicted entry times change by at most ~15 s).
- STARE recruiting and continuous event-window scans screen with the
  closed-form set time and refine only the leading candidates, or values
  near a threshold, with the drift-corrected prediction.
- The Earth's heliocentric position is computed once per instant instead
  of twice per tracked asteroid per step.
- Scheduling results unchanged: over 25 simulated hours at 1,000
  satellites, 100 % of transits and 99.7 % of occultations met (0.7.0 in the
  same run: 100 % and 100 %).

## 0.7.0 (2026-11-23)

Includes 0.6.1 (passively collision-safe interferometry clusters).

- Moving targets: known near-Earth asteroids are tracked as they move, by
  name in public requests, STARE campaigns, desktop search, and the science
  programme's and simulated visitors' NEO follow-ups. Ephemeris (direction,
  distance, V, rate, position angle) refreshed every scheduling step.
- Each telescope aims from its own orbital position, correcting the
  topocentric parallax of asteroids, planets and the Moon.
- NEO recovery exercise: a synthetic new-discovery alert (a real catalogued
  NEO as truth, short-arc errors assumed) whose growing uncertainty region is
  tiled with drifting fields at top priority; detection, cancellation of the
  remaining search, tracked follow-up, and alert-to-recovery statistics.
  Director console tab "NEO recovery"; `recovery_start` / `recovery_auto`
  commands; `--recovery-auto`. Measured: median ~3 min from alert to
  recovery at 6–48 h since discovery; 92–100 % recovered.
- Result images of a tracked asteroid are registered on it: a point source
  with trailed stars; new Tracking table (rate, PA, distance, parallax, star
  trails) and the target's V and peak SNR. Request lists show the tracking
  rate.

## 0.6.1 (2026-11-21)

- Interferometry clusters are passively collision-safe: deputies fly with
  parallel relative eccentricity/inclination vectors (e/i vector separation,
  as TerraSAR-X/TanDEM-X), so radial and cross-track offsets never vanish
  together and along-track drift cannot bring two satellites together.
  Previously the cross-track phase was 90° off, safe only while along-track
  positions were held. Defaults: every pair ≥ 96 m apart radially and
  cross-track.
- New passive-safety readout (desktop panel and `interferometry.safety` in
  /api/status): minimum separation per pair and how a deputy that can no
  longer thrust drifts clear (~3 days, passing others at ≥ 90 m).
- Bellatrix diameter fits are unchanged by the new layout (0.69 ± 0.09 mas
  after ~23 h on target, true 0.70).

## 0.6.0 (2026-11)

The release that goes with version 0.12 of the FACET proposal site.

### Sky and catalogues
- Real catalogues: HYG v4.1 stars to V 10 (synthetic below), OpenNGC deep-sky
  objects with Hubble types, and variable stars; the science programme runs on
  real targets.
- Known near-Earth asteroids from the MPC orbit file: sky view, visibility and
  close approaches.

### Observing and scheduling
- All-sky survey mode: the fleet tiles the sky outside the keep-outs and each
  pass is measured. Day-side satellites (zenith inside the Sun or Moon
  keep-out) take tiles elsewhere in their sky. Measured with defaults: 67–73
  min per pass with 1,000 telescopes, 8.1–9.2 min with 10,000. Pass records
  give time per tile by attitude phase and completion times by declination
  band. Optional polar duty (`--survey-polar`).
- The survey tail in Science operations no longer stalls (part-done tiles
  keep their integration; full search radius; tail fallback).
- Science-specific scheduling: event windows (transits, occultations) with
  coverage and gap limits, NEO visit cadence, no partial credit, and
  handover-aware staffing of open windows (transits 100 %, occultations 99 %
  met in a 24 h run, against 61 % and 81 % without handover).
- STARE handover prediction includes J2 nodal drift; SAA-aware handover
  treats telescopes about to enter the anomaly as about to lose the target.
- Sunward stray light and the Sun keep-out trade table.

### Optics and imaging
- Optics rework: design inputs (aperture, f-ratio, design, sensor, pointing
  presets), derived field and image-size budget. The default is f/8:
  0.485″ pixels, 1.29° × 0.86° field.
- Fleet optical variation: per-telescope aberrations and throughput; PSF and
  depth reported as a spread; each telescope credited by its own sensitivity.
- Photometric precision reported separately from detection depth, with an
  assumed systematic floor.
- No sub-second lucky-imaging credit: the full pointing jitter is applied.
- Result images draw catalogue deep-sky objects (galaxies by Hubble type,
  globular and open clusters, planetary nebulae, nebulae, supernova
  remnants) scaled to catalogue size, shape, orientation and magnitude.
  Internal structure is procedural and the result notes say so.

### Interferometry
- Intensity interferometry clusters and a propellant budget.
- Twin-telescope satellites (1–5 % of the fleet, 0.5–3 m boom) measure large
  stars (Betelgeuse, Antares, …); photon flux at the observing wavelength
  from B−V. The cost model prices twins as ordinary satellites, and says so.

### Public site and director console
- Public engagement model: simulated visitors go through the real queue.
- Mosaics: a request can tile a catalogue object larger than one field
  (M31: 6 tiles), with the telescopes shared between tiles, one approval for
  the whole mosaic, and a stitched overview.
- Finished requests no longer vanish from "Your requests", and their results
  are kept.
- Provenance tags (simulated / modelled / assumed / data) on every headline
  number, on both web pages and in the desktop app.
- Build stamp on both web pages and in the Display tab.
- Director console: headline strip and tabbed sections.

### Desktop app
- Tabbed controls, headline strip, saved scenarios (`--scenario`; built-in
  100-satellite pilot, 1k defaults, 1k conservative, 10k optimistic) and
  target search (bodies, stars, deep-sky objects, NEOs, RA/Dec).

### Fixes
- Start-up panic when interferometry arrays were read before the first
  measurement.
- Stack label counted telescopes that were not stacking.

## 0.5.0

First public release.
