# FACET simulator

An Earth-centred planetarium with the FACET telescope shell running inside it.
Scale the constellation from 2 to 10,000 satellites live and see what it can
point at, stack, mosaic and sweep.

Rust + egui/eframe (OpenGL via glow). One native binary, data embedded.

## Build & run

```sh
cargo run --release
```

Needs Rust ≥ 1.79 (the windowing stack pulls `quick-xml`, which needs 1.79).
The distro Cargo 1.75 on *crash* is too old. Install a user-local toolchain
with rustup. It lives in `~/.rustup` and `~/.cargo` and leaves `/usr/bin/cargo` alone:

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh   # then open a new shell
cargo run --release
```

System packages (Debian/Ubuntu): `libx11-dev libxcursor-dev libxrandr-dev libxi-dev libgl1-mesa-dev libxkbcommon-dev`.

## What's in it

**Sky view (planetarium).** Geocentric J2000 sky in stereographic projection:
8,920 stars to V 6.5 (HYG v4.0) with B−V colours, Sun, Moon and planets, an RA/Dec
grid, the ecliptic, and Sun/Moon keep-out zones. Each telescope's FOV footprint is
drawn on the sky. Drag to pan, scroll to zoom, click to set the target.

**Orbit view.** A 3D Earth with a day/night terminator and coastlines, the shell,
laser links and pointing rays. Satellites are coloured green (observing),
cream (idle, sunlit) or blue (idle, eclipsed). Links that would pass through the
atmosphere show red. Drag to rotate, scroll to zoom.

**Constellation.** Walker delta or star pattern with satellite count T (2–10,000,
plus funding-stage presets), planes P (auto ≈ √T), phasing F, altitude,
inclination (with a sun-synchronous lock) and RAAN. Orbits are circular,
propagated analytically with secular J2. Laser links use a +grid topology:
fore/aft in-plane, plus the nearest-phase satellite in the next plane.

**Operations modes.**
- *Survey*: every telescope stares at its zenith.
- *Target: stack*: every telescope with a clear line of sight points at the
  target. The panel shows collecting area, equivalent aperture, stacked depth,
  and the longest projected baseline with its theoretical λ/B.
- *Target: mosaic*: the visible telescopes tile a grid of FOVs around the target.
- *NEO sweep*: a ring of pointings at a chosen solar elongation, for near-Sun
  asteroid search.

**Constraints.** Sun exclusion, Moon exclusion, the Earth + atmosphere keep-out
cone plus a limb margin, and Earth-shadow (eclipse) state.

**Export TLEs.** Writes the shell at the current sim time as a `.tle` file.
Checked against SGP4: within 0.3° after 12 h.

## Use it with Stellarium

1. Click **Export TLEs** to write `facet_<N>_sats.tle`.
2. In Stellarium, open Configuration → Plugins → Satellites → configure → Sources,
   add `file:///full/path/to/facet_<N>_sats.tle`, then click *Update*.

Stellarium then shows FACET over your real horizon, so you can see what an
observer in Maine sees.

## Command line

```
facet-sim --n 1000 --mode neo --view sky --look-sun --fov 160
facet-sim --n 10000 --mode stack --target Jupiter --look --coverage
facet-sim --n 50 --sso --mode survey --view orbit --zoom 20000
```

Flags: `--n --planes --alt --inc --sso --star --aperture --rate --paused
--mode survey|stack|mosaic|neo --view orbit|sky|split --target <planet or star>
--look --look-sun --fov <deg> --zoom <km> --coverage`.
`FACET_BENCH=1` prints the frame time.

## Accuracy and assumptions

Every performance number is an **unchecked soft estimate**:

- Stacked depth = (assumed single-telescope limit) + 1.25·log₁₀N + 1.25·log₁₀(t/t₀).
  This is background-limited incoherent stacking with perfect registration. The
  single-telescope limit is a slider, not a model. Replace it with a real SNR
  model before quoting any number.
- Equivalent aperture = D·√N (it collects as much light as that aperture, but
  resolution does not improve).
- λ/B is the theoretical resolution if the apertures were phased. Getting there
  needs metrology to nanometres over thousands of km, so treat it as a ceiling,
  not a capability.
- Revisit = accessible sky ÷ (active telescopes × FOV²) × (exposure + overhead).
  It assumes perfect tiling. The overhead is a slider.
- Ephemeris: Schlyter low-precision theory, precessed to J2000. Checked against
  ERFA (astropy) for 2026-11-14: Sun and planets within 0.015°, Moon within 0.01°.
- Orbits: circular, secular J2 only. No drag, station-keeping or collision avoidance.

## Layout

| file | contents |
|---|---|
| `src/astro.rs` | time, GMST, Sun/Moon/planets |
| `src/constellation.rs` | Walker shell, J2 propagation, link topology |
| `src/obs.rs` | constraints, pointing modes, metrics, sky grid |
| `src/views.rs` | orbit and sky rendering, picking |
| `src/app.rs` | state, clock, control panel |
| `src/tle.rs` | TLE export |
| `assets/` | stars (HYG v4.0, CC BY-SA 4.0), coastlines (Natural Earth, public domain) |

`cargo test` runs 16 checks: ephemeris vs ERFA, orbital period, SSO inclination,
nodal regression, Earth-cone geometry, stacking maths and TLE format.

## Next steps

- Request-load simulation: a queue of public observation requests, telescope
  assignment and slews, plus throughput and latency.
- Live planet streams: telescopes per planet over a day, and handover.
- Aperture trade study: sweep D and FOV against cost per satellite.
- Ground observer view: a topocentric sky from any site, to show what FACET does
  to ground astronomy.
- Optical-link budget and real link scheduling.
