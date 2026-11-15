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

**Unified view.** The orbit scene placed inside the planetarium: the full sky
(stars, grid, ecliptic, Sun/Moon/planets, keep-out zones, FOV footprints,
stacking-depth map, labels) is drawn as the celestial sphere behind Earth
and the shell. Drag to orbit, scroll to change distance, shift+scroll to
change the camera field of view, click the sky to set a target. *Look at
target* / *Look at Sun* swing the camera so that direction sits just past
Earth's limb.

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

## Web access

The simulator serves two ports while it runs:

| port | who | what |
|---|---|---|
| **8780** operations console | observatory director, token-protected | an in-world ops centre (see below) |
| **8781** public access node | anyone | live all-sky map (Hammer–Aitoff, centred on the anti-Sun) showing telescope pointings, keep-outs, planets and the queue; status; observation-request form (click the map to fill in RA/Dec); "your requests" tracker |

**Simulator vs. operations.** The desktop window is the *simulator*. It alone
controls the simulated universe: fleet size, orbits, hardware, agility, keep-out
geometry and the clock. The web consoles are *part of the simulation*: they show
what FACET's real operations staff and public users would have. The ops API
cannot change the fleet, orbits, hardware or clock (`cargo test` checks that
such commands are rejected).

The **operations console** (port 8780) is a mock-up of the hoped-for
control surface. It has:
- **Fleet** status: in service, safe hold and integrating counts, a phase bar,
  sunlit/eclipse split and the optical mesh;
- **Alerts**;
- a read-only payload and constraints summary;
- a **ground-track map**: every satellite by state, day/night terminator, subsolar point;
- the **sky map**;
- **Programme** control: survey, stack, mosaic, NEO sweep or the public queue;
  director's target; public time share; auto-approval;
- a **satellite inspector** (click any satellite): state, task, position,
  illumination, boresight vs commanded pointing, pointing error, guidance mode,
  jitter, laser links, and a **safe hold / return to service** command;
- the **request queue** with approve / reject / priority / telescopes / results;
- a timestamped **event log**: submissions, approvals, first light, full
  strength, completions, programme changes, safe holds, fleet changes.

When the app starts it prints the console URL with its token and shows it in the
*Web access* panel. The token is random each run unless you set `--token`.

By default both ports listen on **127.0.0.1 only**. To reach them from other machines:

```sh
facet-sim --control-addr 127.0.0.1:8780 --public-addr 0.0.0.0:8781
```

Keep the director port on localhost, or behind an SSH tunnel or a TLS reverse
proxy (nginx, caddy). It's plain HTTP, so the token is sent unencrypted.
To disable ports: `--no-control`, `--no-public`, `--no-web`.

**The public queue runs in every mode.** In Queue mode every telescope serves
it. In the other modes the director's programme runs and the queue may take
over up to the **public time share** of the shell: default 25 %, adjustable
on the director page, in the app panel, or with `--public-share 0.4`. Approved
requests are scheduled by priority (ties go to the oldest). Each takes up to its requested number of telescopes from
the satellites that have a clear line of sight. It then accumulates telescope-seconds
until it reaches telescopes × exposure. Requests inside the Sun or Moon keep-out
wait, and show why. The allocator is a deliberately simple greedy baseline for
comparing smarter schedulers against.

Public-port limits: 5 open requests per IP, 500 open requests in total,
8 KB request bodies, and text fields length-capped and stripped of control
characters. Submitter IP addresses are visible only on the director port.

API (JSON): `GET /api/status`, `GET /api/stars` on both ports;
`POST /api/request` on the public port. On the ops port, with header
`X-Facet-Token` or `?token=`: `GET /api/fleet` (sub-satellite points and states),
`GET /api/sat/<index>` (inspector record), `GET /api/coast`, and
`POST /api/cmd`. Example ops commands:

```json
{"cmd":"mode","mode":"queue"}
{"cmd":"target","name":"Jupiter"}
{"cmd":"safe_hold","sat":41,"on":true}
{"cmd":"request","id":7,"action":"approve","priority":5,"telescopes":200}
{"cmd":"auto_approve","on":true}
{"cmd":"public_share","share":0.25}
```

## Simulated observation results

Every request that has been observed gets a **simulated data product**: a
stacked, noisy, sky-subtracted image plus an observation report. They're built
from the hardware snapshot taken when the request started observing, and the
telescope-seconds actually integrated. *Result* / *Live stack* buttons appear on
both pages. While a request is still observing, the stack deepens in 5 % steps.
Shareable link: `http://host:8781/?result=<id>`.

API: `GET /api/result/<id>` (JSON report), `/api/result/<id>/full.png` (whole
field, binned), `/api/result/<id>/crop.png` (centre at native scale; planets
are framed and resampled up to 3× finer than a pixel so the delivered detail is visible).

What's modelled (`src/imaging.rs`). Every input is a labelled assumption:

- **Optics:** aperture D with 33 % central obstruction; FWHM = √(diffraction² + jitter²),
  with diffraction FWHM ≈ 1.03 λ/D. Default jitter is 1″ RMS for a small-sat bus.
  Sub-second frames are registered, which freezes most of the jitter
  (lucky imaging).
- **Detector:** IMX455-class CMOS: 9576 px across, 3.76 µm pixels, QE 0.8,
  read noise 1.5 e⁻, dark 0.003 e⁻/s, full well 51 ke⁻. Pixel scale comes from the
  field of view. Focal length and f-ratio are derived (1° → 0.376″/px, f/10.3).
- **Photometry:** a clear 400–700 nm band, zero point ≈ 3×10⁶ photons s⁻¹ cm⁻²
  at V = 0, times optics throughput (0.75) and QE.
- **Sky:** zodiacal light only, since there's no atmosphere. Its brightness depends on
  solar elongation and ecliptic latitude (after Leinert et al. 1998): about 22.3 mag/arcsec²
  at the ecliptic poles, brighter toward the Sun.
- **Stacking:** N telescopes × t seconds are combined incoherently. Signal and
  variance add, so read noise is paid once per sub-frame. Resolution is that of
  ONE aperture, because interferometric phasing is not modelled.
- **Exposure planning:** sub-frames are capped at a maximum length (default 300 s, for
  cosmic rays and drift). For bright targets they're shortened to stay below
  70 % of full well. Jupiter, for example, ends up at about 50 ms frames.
- **Scene:** real catalogue stars to V 6.5. Fainter stars and galaxies are a
  *synthetic statistical population*: star counts depend on galactic latitude,
  galaxies use exponential discs. The field is seeded by sky position, so the same
  patch always looks the same. Planets and the Moon have the correct size, phase,
  magnitude, pole orientation, ring tilt and ring shadow, but *procedural*
  surface markings.
- **Not modelled yet:** extended nebulae and galaxies (M42, M31 …), lunar
  parallax from LEO, cosmic rays, flat-field errors, stray light near the
  limb, colour.

The 5σ limits are checked in tests:

| setup | 5σ point-source limit |
|---|---|
| 1 telescope × 60 s | V ≈ 20 |
| 100 × 60 s | about 2.5 mag deeper |
| 300 telescopes × 300 s, North Galactic Pole | V ≈ 24.1 |

## Attitude: slewing, settling, guide-star lock, jitter

Telescopes are no longer pointed instantly. Each satellite has its own
boresight (`src/attitude.rs`), and every repoint runs this sequence:

1. **Slew.** A time-optimal rest-to-rest manoeuvre (bang-bang, or bang-coast-bang)
   about the eigen-axis. Angular acceleration α = wheel torque / inertia, and
   rate is capped by the slew-rate limit and 80 % of wheel momentum.
2. **Settle.** The residual error e₀ (0.05 % of the slew angle, at least 20″)
   rings down as a damped oscillation with τ = 3 s until it's inside the 10″
   fine-guidance capture radius.
3. **Acquire.** The fine-guidance loop locks onto a guide star in the science
   field (3 s). A planet or the Moon serves as its own guide source.
4. **Locked.** Photons count only now. Pointing wanders with the steady-state
   jitter: 1″ RMS under fine guidance. If no guide star brighter than the limit
   (V 13) falls in the field, the star tracker alone gives about 6″ RMS. The odds
   of a guide star come from the star-count model, so narrow fields at high
   galactic latitude often lack one.

The default ~150 kg, bus-dominated spacecraft: I ≈ 20 kg·m² (0.6 m bus + 1 m
optical tube), 0.15 N·m usable wheel torque, 2 N·m·s momentum, 2 °/s rate limit.
That gives α ≈ 0.43 °/s². Full repoint times (slew + settle + lock):

| slew | repoint time |
|---|---|
| 1° | 8 s |
| 10° | 15 s |
| 45° | 36 s |
| 90° | 61 s |
| 180° | 108 s |

**Schedulers are slew-aware.** A telescope keeps its task while it can, so
nobody repoints for nothing. New work goes first to idle telescopes with the
shortest slew; pre-empting lower-priority work comes last. Mosaic tiles are
"sticky", filling outward from the centre. Targets that move slowly compared
with the rate limit (planets, a zenith stare) are tracked without breaking lock,
whatever the time step.

**What changes in the outputs:**
- Requests track their **response time**: approval to first locked photon,
  and to all requested telescopes locked. They also track repoints and mean slew.
- Each request's **effective jitter** and **fine-guided fraction** are
  integration-weighted. They set the PSF of its simulated image, so a stack
  taken without guide stars comes out visibly blurred.
- The result report adds a **pointing panel** with a pointing-error-vs-time trace
  for a typical repoint.
- In the views, orange = slewing, yellow = settling / acquiring, green =
  locked. Sky footprints sit at the *actual* boresight.

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
facet-sim --view unified --n 300 --target Jupiter --camfov 60 --look
facet-sim --n 50 --sso --mode survey --view orbit --zoom 20000
```

Web flags: `--control-addr --public-addr --no-control --no-public --no-web --token`.

Flags: `--n --planes --alt --inc --sso --star --aperture --rate --paused
--mode survey|stack|mosaic|neo --view orbit|sky|split|unified --camfov <deg> --target <planet or star>
--look --look-sun --fov <deg> --zoom <km> --coverage`.
`FACET_BENCH=1` prints the frame time.

## Accuracy and assumptions

Every performance number is an **unchecked soft estimate**:

- Stacked depth comes from the photometric and noise model described under
  *Simulated observation results*, assuming a typical dark sky of 22.0 mag/arcsec².
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
| `src/server.rs` | HTTP ports, auth, limits, request book |
| `src/attitude.rs` | slew / settle / guide-lock state machine, agility model |
| `src/imaging.rs` | hardware + photometry model, simulated images and reports |
| `src/webapp.rs` | sim ↔ web glue: commands, queue scheduling, status |
| `web/` | director and public pages, all-sky map |
| `assets/` | stars (HYG v4.0, CC BY-SA 4.0), coastlines (Natural Earth, public domain) |

`cargo test` runs 28 checks (`cargo test -- --ignored` also writes a gallery of sample results to /tmp): ephemeris vs ERFA, orbital period, SSO inclination,
nodal regression, Earth-cone geometry, stacking maths and TLE format.

## Next steps

- Request-load simulation: synthetic request generators (bursty public demand), slew
  costs and handover in the scheduler, plus throughput and latency statistics.
- Live planet streams: telescopes per planet over a day, and handover.
- Aperture trade study: sweep D and FOV against cost per satellite.
- Ground observer view: a topocentric sky from any site, to show what FACET does
  to ground astronomy.
- Optical-link budget and real link scheduling.
