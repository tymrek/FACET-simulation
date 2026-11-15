# FACET simulator

An Earth-centred planetarium with the FACET telescope shell running inside it.
Scale the constellation from 2 to 10,000 satellites live and see what it can
point at, stack, mosaic and sweep.

Rust + egui/eframe (OpenGL via glow). One native binary, data embedded.

## Build & run

```sh
cargo run --release
```

It starts with 1,000 satellites at 1× real time, with STARE campaigns armed for
the Moon and planets. In the *Target: stack/mosaic* modes the director's target
takes at most **25 telescopes** by default (slider *max eyes on target*,
`--target-eyes N`, 0 = every telescope that can see it); the rest keep
surveying. **Clear** (or Esc) drops the target, `--no-target` starts without
one, and clicking the sky only sets the target when *Click on the sky sets the
target* is ticked (`--click-target`).

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
{"cmd":"stare_start","target":"Jupiter","eyes":10}
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

- **Optics:** a Fourier-optics PSF (`src/optics.rs`), computed by Fraunhofer
  diffraction of the real pupil, not a Gaussian:
  - the aperture has a 33 % central obstruction and a 4-vane spider
    (0, 3 or 4 vanes, adjustable width);
  - the wavefront error is a fixed low-order Zernike mix (defocus,
    astigmatism, coma, trefoil, spherical), default 60 nm RMS, which gives
    Strehl ≈ 0.6 across the band;
  - monochromatic PSFs at 7 wavelengths from 420 to 690 nm are averaged;
  - the result is convolved with the pointing jitter, 1″ RMS by default.
    Sub-second frames are registered, which freezes most of the jitter
    (lucky imaging).

  The ideal scene (stars as points, galaxy profiles, shaded planets) is
  convolved with this PSF, integrated over the pixels. So images show Airy
  rings, diffraction spikes, aberrated cores and saturated halos. Limiting
  magnitudes use encircled energy measured from the same PSF. Reports give
  the Strehl ratio, optical vs delivered FWHM, EE50/EE80 radii and a PSF
  image. Tests check FWHM and encircled energy against Airy theory, the
  effect of the obstruction, Strehl, and the spikes.
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
| 1 telescope × 60 s | V ≈ 19.8 |
| 100 × 60 s | about 2.5 mag deeper |
| 300 telescopes × 300 s, North Galactic Pole | V ≈ 23.9 |

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

## STARE campaigns — Sustained Tracking And Rotating Eyes

A STARE campaign keeps *k* telescopes ("eyes") locked on one target
continuously, for a set number of hours or until stopped. It's the basis for
live planet streams and for continuous watch on a near-Earth object.

**How handover works:**
- Every telescope's line of sight to the target ends at a predictable moment:
  when its orbit carries the target into the Earth keep-out cone. The
  scheduler computes this window exactly from each satellite's orbital-plane
  geometry (`Shell::time_to_set`, tested against propagation).
- A member with less than a lead time left (a 60° repoint plus 20 s) becomes
  *outgoing*. It keeps integrating until it loses the target, but a
  replacement is recruited at once, so the new eye is slewed, settled and
  guide-locked before the old one drops out.
- Recruits are the free telescopes with the **longest** remaining window among
  those that can lock in time, which keeps handovers to a minimum.
- STARE eyes take priority over programme work. The public queue never takes them.

**24-hour verification** (`cargo test --release stare_24h -- --ignored --nocapture`):
one campaign on each planet and the Moon, starting 2026-11-15, 2 s steps, with
full slew, settle and lock dynamics.

| shell | eyes / target | full strength (≥ k locked) | ≥ 1 eye locked | longest gap | handovers / h |
|---|---|---|---|---|---|
| 1000 sats | 10 | 99.995–99.998 % | 100 % | 0 s | 13 |
| 1000 sats | 50 | 99.98–99.99 % | 100 % | 0 s | 67 |
| 100 sats | 10 | 99.96–99.99 % | 100 % | 0 s | 14 |

Targets inside the Sun exclusion (Mercury and Venus on that date) wait.
Tracking resumes on its own when they emerge. Because about 58 % of the shell
can see any direction at once, even a 100-satellite stage could stream five
planets around the clock. A 1,000-satellite cloud has capacity to spare.

**Live frames.** Each campaign publishes a frame every *cadence* (default
60 s): the locked eyes stacked over one interval, rendered with the same
imaging model as request results. The public page lists active campaigns
under **Live streams**, with *Watch* buttons and shareable links
(`http://host:8781/?stream=<id>`).

**Built-in campaigns.** At startup the Moon and all seven planets each have a
STARE campaign *armed*, so starting one is a single click (or **Start all
ready**). If a target is inside a keep-out zone (Mercury and Venus near the
Sun, for example), its campaign starts **PAUSED**: it holds no telescopes, the
console shows why, and it resumes on its own when the target leaves the zone.
Pausing and resuming are both logged. Stopping a built-in campaign returns
it to *ready*.

Ways to start one:
- the ops console's **STARE campaigns** panel: target name, or Shift-click the
  sky; eyes, frame cadence and duration;
- the desktop *Web access* panel;
- the command line: `--stare Jupiter --stare Saturn --stare-eyes 10`;
- the API: `{"cmd":"stare_start","target":"Jupiter","eyes":10,"frame_s":60,"duration_h":24}`,
  `{"cmd":"stare_activate","id":5}`, `{"cmd":"stare_activate_all"}`,
  `{"cmd":"stare_eyes","id":1,"eyes":20}`, `{"cmd":"stare_stop","id":1}`.

Each campaign reports full-strength and any-eye coverage, gaps, longest gap,
handovers, and integrated time. Events (lock, gap, recovery, Sun-exclusion
pause, end) go to the ops event log.

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

STARE flags: `--stare <target>` (repeatable), `--stare-eyes N`.

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
| `src/optics.rs` | Fourier-optics PSF: pupil, spiders, wavefront error, polychromatic, jitter; FFT convolution |
| `src/imaging.rs` | hardware + photometry model, simulated images and reports |
| `src/webapp.rs` | sim ↔ web glue: commands, queue scheduling, status |
| `web/` | director and public pages, all-sky map |
| `assets/` | stars (HYG v4.0, CC BY-SA 4.0), coastlines (Natural Earth, public domain) |

`cargo test` runs 36 checks (`cargo test -- --ignored` also writes a gallery of sample results to /tmp): ephemeris vs ERFA, orbital period, SSO inclination,
nodal regression, Earth-cone geometry, stacking maths and TLE format.

## Next steps

- Request-load simulation: synthetic request generators (bursty public demand), slew
  costs and handover in the scheduler, plus throughput and latency statistics.
- Live planet streams: telescopes per planet over a day, and handover.
- Aperture trade study: sweep D and FOV against cost per satellite.
- Ground observer view: a topocentric sky from any site, to show what FACET does
  to ground astronomy.
- Optical-link budget and real link scheduling.

## Science programme and science return

The sim starts in **Science operations** mode with a simulated science
programme running: 120 invented investigator teams submit observing requests
across ten science cases, and the request queue schedules them alongside
public requests, STARE campaigns and the director's target. Every team,
target and designation is synthetic; the case mix, team sizes, integration
times, priorities and deadlines are **assumptions** (see `src/programme.rs`).

| Case | Telescopes | Integration each | Priority | Deadline | Sky |
|---|---|---|---|---|---|
| NEO follow-up astrometry | 1–3 | 2–10 min | 6–8 | 24 h | ecliptic, 60–175° elongation |
| Supernova / transient follow-up | 4–16 | 10–30 min | 7–8 | 48 h | \|b\| > 30° |
| GRB afterglow (ToO) | 10–40 | 10–20 min | 10 | 2 h | anywhere |
| Stellar occultation timing | 4–12 | 2–5 min | 9 | 2 h | ecliptic, opposition side |
| Exoplanet transit photometry | 2–6 | 30–120 min | 4–5 | 12 h | anywhere |
| Variable-star monitoring | 1–2 | 5–15 min | 3–4 | 3 d | Galactic plane |
| Comet monitoring | 2–4 | 5–15 min | 5–6 | 48 h | ecliptic, 50–110° |
| Planetary atmospheres | 2–8 | 5–20 min | 4–6 | 24 h | Mercury … Neptune |
| Clusters & nearby galaxies | 4–12 | 15–40 min | 2–3 | 7 d | anywhere |
| Deep fields | 20–80 | 30–90 min | 1–2 | 14 d | \|b\| > 30° |

Proposers avoid targets inside the Sun and Moon keep-outs. **Demand** is
metered in telescope-seconds: at 1.0× work arrives as fast as the whole
fleet could integrate it; the default 1.5× makes the facility oversubscribed,
so priorities matter and low-priority deep fields wait. Desktop knobs:
*Science programme* panel (on/off, demand, number of teams);
`--no-programme`, `--load X`, `--teams N`.

**Allocation.** Public and director requests get first claim on the public
time share; the programme then takes every telescope still free (idle or
surveying). In Stack/Mosaic it uses the eyes beyond the target cap; Survey
and NEO-sweep modes dedicate the whole fleet, so the programme pauses there.
Anything left over runs the zenith survey.

**Science-return metrics** (ops console *Science return* panel, public page
summary, desktop panel, `science` in `/api/status`):

* *Open-shutter efficiency*: satellite time spent integrating; the rest is
  slew/settle/guide-star acquisition overhead, idle or safe hold. *Pointed
  science* is the part spent on requests, STARE and director targets.
* *Time use*: integration hours by programme, public, STARE, director,
  survey and NEO sweep.
* *Requests completed per day*, and *on time*: completed before the case's
  deadline, as a share of completed + expired. Requests that miss their
  deadline expire, because late NEO astrometry or a late GRB afterglow is
  worth little.
* *Median response* per case: submission to first locked integration
  (slew, settle and acquisition included).
* *Median depth* per case: 5σ point-source limit reached by the stacked
  integration against the zodiacal background at the target.
* *Étendue × time*: Σ collecting area × field of view × integration time
  (m²·deg²·h), the standard survey-speed figure of merit; it lets FACET be
  compared with other facilities on one number.
* *Raw data per day*: 16-bit full frames before compression. This sizes the
  downlink.
* *Teams served*, and the most-served teams by telescope-hours.

## Licence

The code is licensed under either of the **Apache License 2.0**
([LICENSE-APACHE](LICENSE-APACHE)) or the **MIT licence**
([LICENSE-MIT](LICENSE-MIT)), at your option. This permissive dual licence is
the Rust-ecosystem standard, chosen so that agencies, universities and
companies can use FACET's simulator as a shared reference without legal
friction. Apache-2.0 adds an explicit patent grant.

Bundled data keeps its own terms (details in [NOTICE](NOTICE)):
- the star catalogue derived from HYG v4.0 is **CC BY-SA 4.0**;
- the Natural Earth coastlines are public domain.

Unless you state otherwise, any contribution you intentionally submit for
inclusion is dual-licensed as above, without additional terms or conditions.

