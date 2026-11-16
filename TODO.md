# FACET simulator: to do

## Next up
- **Sunward stray-light model**: make the Sun keep-out slider a real trade.
  Charge scattered sunlight near the keep-out edge (baffle/sunshade
  dependent), so tightening 45° → 35° buys Venus/Mercury/sunward-NEO time
  at the cost of brighter backgrounds and shallower depth.

## Simulation
- All-sky survey mode: actually tile the observable sky and time each pass
  live (the public site currently shows a throughput estimate).
- Real catalogues (fuller star/deep-sky catalogue, real NEO orbits).
- Public engagement metrics (requests, unique users, returns).
- Optics rework (delivered FWHM ~2.7″ is worse than ground seeing).
- Interferometry tier.

## Identity
- Motion logo (B ↔ C gem): port into the app as start-up splash and the
  presentations' title card; favicon from the top view.

## Build and platform
- GitHub CI (build + tests).
- Browser (WASM) build.
- Wayland: test on a real compositor; nicer client-side title bar on GNOME.
