# FACET simulator: to do

## Done since 0.5.0
- Sunward stray-light model and Sun keep-out trade table.
- All-sky survey mode: the fleet tiles the sky; passes measured live.
- Real star, deep-sky and variable-star catalogues; programme on real targets.
- Known near-Earth asteroids (MPC orbits): sky view, visibility, close approaches.
- Public engagement model and metrics.
- Intensity interferometry clusters and the propellant budget.
- Optics rework: design inputs (aperture, f-ratio, design, sensor, pointing presets), derived field, image-size budget.

## Adversarial review (Nov 2026): fixes in code
- [x] Removed the sub-second lucky-imaging jitter cap.
- [x] Provenance tags on every headline (simulated / modelled / assumed / data).
- [x] STARE handover prediction includes J2 node drift; tested against the propagator.
- [x] Science-specific scheduling: event windows (transits, occultations)
      with coverage and gap limits, NEO visit cadence, no partial credit;
      handover-aware staffing of open windows.
- [x] Fleet optical variation: per-telescope aberrations; PSF and depth reported as a spread.
- [x] Photometric precision separate from detection depth, with an assumed systematic floor.
- [x] SAA-aware handover: telescopes about to enter the anomaly are treated as
      about to lose the target.
- [x] Requests credit each telescope by its own sensitivity (fleet optical variation).

## Validation work (engineering, not code)
- Stray-light ray trace and baffle test.
- Structural/thermal pointing and wavefront error budget (STOP analysis).
- Optical inter-satellite and downlink link budgets.
- Settling from attitude dynamics (reaction wheels, flexible modes), not a timer.
- External evidence of demand: letters of interest, user surveys.

## Simulation
- Interferometry follow-ups: limb-darkened and oblate star models; closure
  of the third-order correlation for phase; cluster size optimisation per star.
- [x] Twin-telescope satellites (1–5 % of the fleet, boom 0.5–3 m) for large stars.
- [x] Interferometry photon flux at the observing wavelength from B−V.
- [ ] Twin bus in the cost model and drag/propellant (wider, heavier spacecraft).
- [ ] Twin observing queue: several large stars, not one fixed target.
- [ ] On a rigid boom, amplitude (Michelson) interferometry becomes realistic:
      revisit the parked ladder's first rung with the twin bus.

## Identity
- Motion logo (B ↔ C gem): port into the app as start-up splash and the
  presentations' title card; favicon from the top view.

## Usability
- [x] Tabbed controls, headline strip, saved scenarios, target search (desktop).
- [x] Director console: headline strip, tabbed lower sections.
- [x] All-sky survey tail in Science operations (kept progress, search radius, tail fallback).
- [x] Extended deep-sky objects in result images (src/deepsky.rs): OpenNGC galaxies,
      globular and open clusters, planetary nebulae, nebulae and SNRs drawn as models
      scaled to catalogue size, shape, PA and magnitude; internal structure procedural.
- [x] Public mosaics (src/mosaic.rs): tiles a catalogue object larger than the field on an
      RA/Dec grid with 10 % overlap, shares the telescopes between tiles, approves/rejects
      as a group, and serves a stitched overview at /api/mosaic/{group}.
- [x] All-sky survey: day-side satellites (zenith in the Sun/Moon keep-out) now take
      tiles (passes 10–25 % faster); per-phase and per-|Dec|-band pass statistics;
      optional polar duty (measured neutral at 1,000, slightly worse at 10,000).
- [ ] All-sky at 10,000: the last 5 % (~1–2 min) sits at |Dec| < 30°, not the caps;
      find out which tiles (Sun keep-out edge?) and why they come late.
- [ ] Mosaic follow-ups: roll the grid to the object's major axis (fewer tiles for
      inclined galaxies); overview at full depth per tile instead of the shallowest.
- [ ] Galaxy models from catalogue axis ratios: near edge-on galaxies with big halos
      (M104, M82) come out as inclined spirals; OpenNGC "S?" types are drawn as spirals.
- [ ] Moving targets: track NEOs and planets as they move when chosen from search.

## Build and platform
- [x] GitHub CI (build + tests): .github/workflows/ci.yml.
- Wayland: test on a real compositor; nicer client-side title bar on GNOME.

## Last
- Browser (WASM) build.

## Parked (out of scope for the current proposal)
- Amplitude (Michelson) interferometry. Needs dedicated spacecraft, not the
  shell. Ladder discussed: single-spacecraft boom interferometer (≤ ~20 m,
  3 collectors) → spinning tethered formation (100 m – 1 km, fibre-fed) →
  free-flying formation (> 1 km); 3 collectors + combiner minimum for
  closure-phase imaging, ~6 + 1 for a science array; one per plane of two
  planes ~90° apart for complementary uv coverage.
