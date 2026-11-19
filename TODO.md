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
- [ ] SAA-aware handover: don't recruit telescopes about to enter the anomaly
      (the remaining ~5 % of transit failures).
- [ ] Requests use per-telescope hardware (currently the fleet spread is
      reported but stacks use the nominal telescope).

## Validation work (engineering, not code)
- Stray-light ray trace and baffle test.
- Structural/thermal pointing and wavefront error budget (STOP analysis).
- Optical inter-satellite and downlink link budgets.
- Settling from attitude dynamics (reaction wheels, flexible modes), not a timer.
- External evidence of demand: letters of interest, user surveys.

## Simulation
- Interferometry follow-ups: limb-darkened and oblate star models; closure
  of the third-order correlation for phase; cluster size optimisation per star.

## Identity
- Motion logo (B ↔ C gem): port into the app as start-up splash and the
  presentations' title card; favicon from the top view.

## Usability
- The controls column is very long: regroup into tabs (Fleet, Optics,
  Operations, Science, Environment, Programme).

## Build and platform
- GitHub CI (build + tests).
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
