# FACET simulator: to do

## Done since 0.5.0
- Sunward stray-light model and Sun keep-out trade table.
- All-sky survey mode: the fleet tiles the sky; passes measured live.
- Real star, deep-sky and variable-star catalogues; programme on real targets.
- Known near-Earth asteroids (MPC orbits): sky view, visibility, close approaches.
- Public engagement model and metrics.
- Optics rework: design inputs (aperture, f-ratio, design, sensor, pointing presets), derived field, image-size budget.

## Simulation
- Intensity interferometry on the shell (photon time-tagging over the laser
  links): baseline/uv geometry from real orbits, HBT SNR per pair, bright-star
  diameters; two planes ~90° apart for complementary coverage, half their
  satellites carrying timing detectors.

## Identity
- Motion logo (B ↔ C gem): port into the app as start-up splash and the
  presentations' title card; favicon from the top view.

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
