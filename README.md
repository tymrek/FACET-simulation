# FACET simulator

An Earth-centred planetarium with the FACET telescope shell running inside it.
Scale the constellation from 2 to 10,000 satellites live and see what it can
point at, stack, mosaic and sweep.

Rust + egui/eframe (OpenGL via glow). One native binary, data embedded.

**Status.** A planning and pitch tool for the [FACET concept proposal](https://www.kermyt.com/FACET/).
Every performance figure it shows is an unchecked soft estimate (see
[Accuracy and assumptions](#accuracy-and-assumptions)). It is not a validated
detection-performance model: that is the first job of the proposed concept study.
Where its assumptions differ from the proposal documents, the
[simulator page](https://www.kermyt.com/FACET/facet-simulator.html) lists the differences.

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
Some distributions ship an older Cargo. If yours does, install a user-local
toolchain with rustup. It lives in `~/.rustup` and `~/.cargo` and leaves the
system `cargo` alone:

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

## All-sky survey speed

The public site and the presentation variables (`{allsky_time}`,
`{allsky_time_1k}`, `{allsky_time_10k}`, `{allsky_deep_time}`,
`{allsky_hour_time}`, `{allsky_per_day}`, `{allsky_deg2}`, `{allsky_depth}`)
report how long the fleet would take to photograph the whole sky not hidden
by the Sun or Moon. It is a throughput estimate (`src/allsky.rs`), not a
scheduled run:

    pass time = fields × (exposure + overhead) / (working telescopes × duty)

with fields = observable area ÷ field area × 1.1 (tile overlap) and duty the
dark-sky-equivalent observing share measured in the run (after slews, the
South Atlantic Anomaly and Earthshine). The Earth blocks each telescope's view
but never the fleet's. Typical results with 1° fields, 60 s per field, 15 s
overhead and ~72 % duty: about 66 min at 1,000 telescopes and about 7 min at
10,000 (to V ≈ 20.3), ~4.6 TB raw per pass. The card also shows 5-minute and
1-hour passes for each build-out stage, and the time with a typical 20 % of
the fleet surveying.

### All-sky survey mode (measured)

`--mode allsky`, or **All-sky survey → Dedicate the fleet** in the controls,
makes the fleet actually tile the sky (`src/survey.rs`). The sphere is cut
into ~45,500 fixed 1° tiles (rings of declination, ~10 % overlap). Each free
telescope takes the nearest undone tile it can see within 75° of its zenith
and 32° of its current pointing, slews, settles and integrates through the
attitude model; only locked, dark-sky-equivalent integration counts, so
slews, the South Atlantic Anomaly, Earthshine and sunward stray light all
cost what they cost. Telescopes in the anomaly hand their tiles back; a
closer free telescope may take over a tile whose claimant is still slewing.
A pass is complete when every tile is done or inside a keep-out. The sky
view shows coverage filling in; the panel, status API and public site report
each pass's time and the time to 95 %.

Measured with defaults: **1,000 telescopes: ~75 min per full pass, 95 % in
~67 min; 10,000 telescopes: ~13–15 min per full pass, 95 % in ~8 min**, at
~79 s of telescope time per field (60 s exposure + ~19 s overhead). The
last few percent are tiles hugging the Sun and Moon keep-out edges. The
option *free telescopes tile during Science operations* runs the same
survey on idle telescopes instead of the zenith stare.

## Public engagement

A modelled public user base (`src/public.rs`) sends requests through the same
queue and fleet as everyone else, in the public share of telescope time.

* **Assumed adoption** (adjustable; to be anchored later against comparable
  programmes): 20,000 monthly active users (`--public-users N`), split
  55 % casual visitors (1.2 visits a month, 1–2 requests, one telescope,
  60 s, mostly the Moon, planets and Messier objects), 25 % amateur
  astronomers (8 visits, 3 requests, 4 telescopes × 300 s, galaxies and
  clusters), 10 % classrooms (4 sessions a month in term-time school hours,
  8 requests, 2 × 120 s) and 10 % citizen scientists (12 visits, 5 requests,
  8 × 300 s, real variable stars and real NEOs). Requests arrive mostly in
  local evenings across three population centres; interest spikes during
  very close asteroid approaches and defence exercises. Fair-use quotas
  per user per day; a request not delivered within 48 h is dropped.
* **Measured, not assumed:** waits, deliveries, turn-aways (quota, queue full,
  gave up waiting) and how full the public share runs, because every request
  is scheduled and observed by the simulated fleet.
* **Reported** (panel, public site, presentation variables
  `{public_images_year}`, `{public_requests_day}`, `{public_wait}`,
  `{public_classrooms_day}`, `{public_citizen_day}`): requests and images a
  day and a year, median wait, users over 30 days and the share returning,
  classroom sessions, citizen-science measurements, and the number of
  monthly users each build-out stage could serve at the same mix.
* Requests from real visitors to the public site are counted separately as
  the "live" line, never mixed into the modelled figures. `--no-public-demand`
  switches the model off.

Typical result at 1,000 telescopes with a 25 % public share: ~9,600 requests
a day, ≈ 3.5 million images a year, median wait ~10 min, public time
70–90 % used, i.e. room for roughly 23,000 monthly users at this mix. The
30-day user figures settle after the first simulated month.

## Catalogues

* **Stars:** HYG v4.1, 108,071 stars to V 10 with 443 proper names. The sky
  view shows fainter stars as it zooms (V 6.5 at 90°, ~8 at 20°, 10 below
  ~5°). Simulated request images use real stars to V 9 and a statistical
  population below that.
* **Deep sky:** OpenNGC, 12,162 galaxies, clusters and nebulae (107 Messier
  objects), drawn at true size and orientation (ellipses: galaxies; circles:
  clusters; squares: nebulae), labelled when zoomed in. Any of them can be a
  target by designation, Messier number or common name (`--target M31`,
  `--target "NGC 7000"`, public requests for "Andromeda Galaxy").
* **Variable stars:** 5,221 HYG variables with magnitude ranges.
* **Science programme on real objects:** cluster and nearby-galaxy requests
  pick real clusters and galaxies brighter than 11th magnitude; supernova
  and transient follow-up happens in real host galaxies (brighter than 14th,
  |b| > 20°); variable-star monitoring uses the real variables; deep fields
  are the well-known extragalactic survey fields (HDF-N, HUDF, COSMOS, EGS,
  UDS, Lockman Hole, ELAIS-N1, XMM-LSS, Subaru Deep Field, Euclid Deep Field
  North). NEO follow-up targets real known asteroids brighter than V 20.5 and
  outside the keep-outs. Transients, comets, occultations, GRBs and transit
  hosts remain synthetic.
* **Known near-Earth asteroids:** the IAU Minor Planet Center's NEA orbits
  (43,182 objects, 2,537 potentially hazardous; snapshot with epoch
  2026 Jun 9). Two-body positions and H, G brightness, refreshed every half
  simulated hour, drawn in the sky view (hazardous ones in red). The panel
  and public site show how many are observable right now by one telescope
  and by 16 stacked, how many are bright but inside a keep-out, and close
  approaches within 20 lunar distances over the next 60 days. Checked on
  Apophis: the model puts the 2029 flyby at 0.090 LD on 14 April 01:07 UTC
  (published: ~0.099 LD on 13 April ~21:46 UTC). Planetary perturbations are
  ignored, so dates and distances degrade over years and through close
  approaches.

## Space environment: South Atlantic Anomaly, Earthshine, orbital lifetime

Three effects that anyone who has operated a LEO telescope will ask about.
They are deliberately simple engineering models; the numbers below come from
`src/environment.rs` and the **Space environment** panels (native and director
console).

**South Atlantic Anomaly (SAA).** Earth's magnetic dipole is offset from the
geocentre and tilted, so the inner proton belt dips into LEO over South
America and the South Atlantic. Satellites crossing it take a heavy flux of
energetic protons: bright hits and tracks on the detector, extra noise,
electronics upsets and cumulative detector damage.

* Model: >10 MeV trapped-proton flux as two lobes (the main anomaly and the
  weaker south-west-African lobe it is slowly separating into), drifting west
  ~0.3°/yr, peak flux rising with altitude (e-folding ~60 km). **Assumed**
  calibration: inside the 10 p/cm²/s contour a Hubble-like orbit (540 km,
  28.5°) spends ~11 % of its time, an ISS-like orbit ~5 %, FACET at
  550 km / 53° ~9 %.
* Policy (`--saa pause|flag|off`): *safe detectors* (default, what Hubble
  does: no science exposure inside), *keep exposing* (frames flagged, hits
  must be rejected) or *ignore* (for comparison).
* The orbit view draws the safe-mode contour at shell altitude, with its
  footprint on the ground, and colours satellites inside it magenta.
* Time inside appears as its own slice of the science-return time bar and is
  taken out of the duty cycle used for the ground comparison.
* **Why it favours a fleet:** a single telescope loses whole minutes at a time;
  across the shell only ~9 % of satellites are inside at any moment and the
  rest keep working.
* **False transients:** particle hits (an **assumed** 2 p/cm²/s of cosmic rays
  outside the anomaly, plus the anomaly flux inside) on the 8.6 cm² sensor, of
  which an **assumed** 1 % look star-like in a single frame. That is tens of
  millions of single-frame false transients a day across the fleet; requiring
  the same event in two telescopes on the same field cuts chance matches to
  hundreds a day, and in three to effectively none. Multi-telescope
  confirmation is how FACET separates real fast flashes from radiation.
* Proton fluence per satellite per year is reported as the detector-damage
  budget.

**Earthshine.** Sunlight scattered off the Earth's limb raises the background
for pointings near the limb. **Assumed** V ≈ 19.0 mag/arcsec² at 10° above a
fully sunlit limb, fading 6 mag per decade of limb angle (≈ 20.8 at 20°, 21.9
at 30°), scaled by how sunlit the limb point is (`--no-earthshine` to switch
off). The cost is charged as dark-sky-equivalent exposure time: requests take
longer to reach their depth, and the lost share is reported (typically 5–10 %
of exposure time in a busy queue). The duty cycle shown against the ground uses
dark-sky-equivalent time.

**Sunward stray light and the Sun keep-out.** Sunlight scattered past the
baffle into the field. **Assumed** parametric point-source-transmittance law:
V ≈ 21.0 mag/arcsec² at 45° from the Sun for a good vaned baffle (19.5 for a
plain tube, 22.5 with a deployable sunshade), fading as elongation⁻³ (7.5 mag
per decade) and tapering to nothing between 60° and 100° as the Sun drops
behind the aperture plane. It is added to the zodiacal light in queue depths,
the defence search and per-exposure efficiency (charged, like Earthshine, as
dark-sky-equivalent time; typically ~5–7 % fleet-wide, concentrated in the
sunward search band). The **Sun keep-out trade** table (native and console)
shows what each keep-out angle from 20° to 60° buys and costs: the share of
days Venus and Mercury are outside it over the next three years (from the
sim's ephemeris), open sky, and depth at the keep-out edge against a dark sky,
with the stray-light part shown separately. With a good baffle the cost near
the Sun is dominated by the zodiacal light itself (≈ −1.4 mag at 45°,
−2.0 at 30°), the stray light adding only ~0.2 mag. Venus is outside a 45°
keep-out ~15 % of the time and a 30° one ~53 %; Mercury needs 25° or less.

**Orbital lifetime and disposal.** Circular-orbit drag decay with the CIRA-72
exponential atmosphere (mean solar activity) and an **assumed** solar-cycle
swing (×1 at 150 km rising to ×3.5 above 600 km between mean and max), cycle
maximum 2024.8, period 11 yr. Spacecraft: **assumed** 150 kg, 1.2 m² mean
tumbling cross-section, C_D 2.2 (all adjustable).

* At 550 km a dead satellite re-enters in ~6.7 yr at mean solar activity
  (~2.3 yr at solar maximum, ~19 yr at minimum; ~8 yr following the cycle from
  late 2026). That does **not** meet a 5-year post-mission disposal rule
  passively, so the design needs an ~84 m/s disposal burn to a 250 km perigee
  while the satellite still works.
* The **altitude trade** table (400–800 km) shows the tension: higher shells
  live longer and spend more time in the anomaly; lower shells clean themselves
  up but need more station-keeping. 500 km meets a 5-year rule passively in
  all but solar-minimum conditions, at the cost of ~7 % SAA time instead of ~9 %
  — in FACET's favour on both counts, against slightly more drag make-up.

Not modelled: ghost images and glints from bright sources in the field, the outer-belt electron "horns" at high latitude, solar particle
events, detector annealing, and drag make-up propellant for station-keeping.

## Presentation mode

Scripted, narrated walkthroughs of the simulation for pitching. Open it from
**Presentation mode…** in the controls, or start straight into a deck with
`--present <deck-id>`.

Four decks ship in `presentations/`:

| Deck id | Audience | Length |
|---|---|---|
| `science-7min` | science funders, agencies, observatories | ~7:00 |
| `investor-7min` | investors, commercial partners | ~6:55 |
| `science-90s` | science funders, short cut | ~1:23 |
| `investor-90s` | investors, short cut | ~1:20 |

Each deck is a list of chapters. A chapter has narration, a target duration
and timed cues that drive the live simulation: fleet size, time rate, camera,
sky pointing, STARE campaigns, the planetary-defence scenario, failures and
cost presets. Cards and narration can contain live values such as
`{completed_per_day}`, `{equivalent_ground}`, `{cost_total}` or `{pd_lead}`, which
are filled in from the running sim. The numbers on screen are therefore the
simulation's numbers, not hard-coded ones. `presentations/SCRIPTS.md` is a
readable copy of all four scripts, with per-chapter word counts and file names.
Text in [square brackets] is for you to fill in (the ask, the amount, contact
details).

**Narration audio.** Record one file per chapter as
`presentations/audio/<deck-id>/<chapter-id>.wav` (or `.ogg`, `.flac`, `.mp3`).
When a chapter has audio, the audio clock drives its timeline: cues fire
against playback position, so the visuals stay in sync with the recording.
Chapters without audio run on their target duration and show captions.
Captions are timed sentence by sentence, in proportion to word count.

**Recording against the timeline.** Tick *Teleprompter* in the picker, or press
`T` while a deck runs. Each chapter gets a 3-second lead-in. After that the
current sentence is shown large, with the following lines below it, along with
elapsed and remaining time. The scene plays underneath, so you read in time
with what the audience will see.

Keys: `Space` pause · `Left`/`Right` previous/next chapter · `Home` restart ·
`C` captions · `T` teleprompter · `Esc` exit. View headers, legends and hints are
hidden while presenting.

Audio playback uses rodio (the `audio` feature, on by default). On Linux it
needs ALSA headers (`sudo apt install libasound2-dev`). To build without audio,
use `cargo build --release --no-default-features`; decks then run on captions.

## Resilience

Desktop *Resilience* panel:

* **Inject failures:** Fail 10 %, Fail 30 %, Lose a plane, Restore all.
* **Random failures:** a rate in % per year (`--fail-rate`).
* **Replacement:** a delay in days after which failed satellites are back in
  service (`--replace-days`).
* **Scripted failure:** `--fail-at HOURS:FRACTION`, for staging a pitch.

Failed satellites take no work and their laser links go down. The ops
console shows them as failed: they are counted, raise an alert, and the
inspector offers no "return to service".

An hourly record compares fleet alive % with science output (integration
per satellite-hour) and completed observations, both relative to the healthy
fleet. Losing 30 % of the fleet costs about 30 % of telescope-hours but only
about 10–20 % of completed observations, because the scheduler sheds
low-priority long jobs first. Replacements restore output, and the backlog
clears above 100 %.

## FACET vs a ground network

Every ~20 s a background job compares FACET with the same number of
identical 20 cm telescopes spread over 12 good observatory sites (Atacama,
Canary Islands, Hawaii, Arizona, Baja California, South Africa, Namibia,
Australia, Spain, Uzbekistan, Tibetan plateau):

* **Observing conditions:** the Sun is below −12°, targets are above 30°
  and at least 15° from the Moon. Each site has an assumed clear fraction
  (55–85 %) and 90 % uptime, sampled over 40 weather realisations (Monte
  Carlo).
* **Depth:** 1.8″ seeing, V 21.3 mag/arcsec² sky and 0.25 mag extinction
  (all assumed), against FACET's Fourier-optics PSF and zodiacal sky.
* **Reported:** duty cycle, sky observable now, queue targets observable now
  and within 24 h, depth, and the number of ground telescopes needed to match
  one FACET telescope at equal depth. Throughput ratio × 10^(0.8 Δm) gives
  about 9–10 with defaults. The ops console has the table; the public page
  has a short "Why put telescopes in space?" card.

FACET's delivered image FWHM (about 2.7″: pixel scale and pointing jitter
dominate) is currently worse than good ground seeing, so most of the
advantage comes from duty cycle and dark sky. That is the obvious target
for the optics work. Ground benefits the model leaves out: large apertures
are cheap on the ground, and it can be serviced and upgraded.

## Cost model

**Open cost model window** in the simulator's controls (or `--costs`) opens a
separate native window, which can sit on a second monitor. Results recompute
live as you edit, and from the running simulation's science figures.

* **Line items**, each with a value now and an annual trend in % per year
  (negative = getting cheaper):
  * satellite first-unit costs (bus, telescope, focal plane, optical
    terminals, assembly/integration/test), with a learning curve (Wright's
    law) across the production run;
  * launch: dedicated price, satellites per launch, rideshare $/kg, cadence,
    insurance;
  * development, overhead and design life;
  * operations staff and ground stations;
  * data: relay, archive, processing.

  Hover a row for its source or assumption.
* **Programme settings:** fleet size (follows the simulator or set by hand),
  horizon, start year, dedicated or rideshare launch, and planes per
  dedicated launch. More than one plane per launch relies on differential
  nodal drift, which takes months.
* **Outputs:**
  * programme cost, first-science and full-fleet years, annual cost,
    average satellite cost;
  * cost per telescope-hour, per completed observation, per m²·deg²·h of
    étendue × time, per TB delivered and per public telescope-hour, using
    the simulator's science ledger (typical values until six simulated
    hours have run);
  * a stacked annual-spend chart with fleet growth, a year-by-year table,
    and funding tranches: when a budget of $100M … $5B runs out and how many
    satellites are in orbit by then.
* **Scenarios:** Baseline, Optimistic and Pessimistic presets; **Save** /
  **Load** write and read `facet-costs.json` in the working folder.

Launch defaults are 2026 public list prices: Falcon 9 dedicated $74M (up
from $67M in 2022) and Transporter rideshare $7,000/kg (up from $5,000/kg at
the programme's start), so both trend upward by default. Rideshares fly to
sun-synchronous orbits, so FACET's 53° planes need dedicated launches or a
drift strategy. Every other line item is an **assumption** to be replaced
with quotes.

In the baseline, launch dominates. One dedicated launch per 31-satellite
plane uses a fraction of a Falcon 9, so the number of planes per launch is
the biggest lever on programme cost.

## Performance and threading

The simulation and the window run on separate threads:

* **Simulation thread** (`facet-sim`): advances time at about 120 Hz,
  independent of the window's frame rate. At high time rates each tick is
  split into sub-steps of at most 60 s of simulated time, within a ~30 ms
  compute budget, so scheduling, attitude and STARE handovers keep their
  fidelity. It also publishes the web status.
* **UI thread** (egui): locks the simulation briefly to draw and to apply
  control changes.
* **Data parallelism** (rayon, all cores): orbit propagation, visibility and
  shadow tests, laser-link checks, queue and STARE candidate scans, attitude
  dynamics, sky coverage, other-operator propagation, streak prediction,
  conjunction screening and relay/ground-station sampling.
* **Web server**: its own threads; result images render in background
  threads.

Conjunction screening has a per-tick budget. At extreme time rates it falls
behind rather than stalling the simulation; the backlog is reported, and
rates are normalised by the satellite-days actually screened.

`FACET_BENCH=1` prints tick time, sub-steps and thread count;
`FACET_PROFILE=1` prints a per-phase breakdown of each step. On a 2-core
test machine with software rendering, 10,000 satellites at 1 h/s run in real
time at about 110 ms per tick. The old single-threaded build managed about
a third of that rate.

## Space traffic, data path and neighbourhood notices

Other operators' satellites and debris share FACET's sky and altitude. The
desktop *Space traffic & data path* panel and the ops console's matching
panel cover three operational questions. The populations are modelled as
circular orbits with J2:

* **Starlink**: about 9,950 satellites at **480 km**, matching SpaceX's 2026
  move of its 550 km shell down to 480 km. Shells are at 53°, 43°, 70° and
  97.6°; the split between them is assumed.
* **OneWeb**: 648 satellites in 12 planes at 87.9° and about 1,200 km.
* **Other objects**: an **assumed** population of 1,500 pieces of debris,
  other operators' satellites and small satellites within ±50 km of FACET's
  altitude. About 30 % are active and share ephemerides.

Flags: `--starlink N`, `--starlink-alt KM`, `--no-oneweb`,
`--other-objects N`.

### Data path

Raw data scales with the fleet: about 150 TB/day at 1,000 satellites and
about 1.5 PB/day at 10,000. The panel compares four on-board products:

| Product | Volume | Note |
|---|---|---|
| Raw 16-bit frames | 1 | |
| Lossless compressed | 1/2.5 | Rice; ratio assumed |
| Co-added + compressed | 1/10 | 4 sub-frames co-added; ratio assumed |
| Source catalogue + cutouts | 1 % | assumed |

It compares them against two ways down:

* **Starlink optical relay** (`--no-relay` to disable). SpaceX sells its
  "mini laser" terminal (rated 25 Gb/s at up to 4,000 km) to third-party
  spacecraft builders such as Muon Space and Starcloud, but the first
  customer satellites only fly from 2027, so service terms and the throughput
  a third party actually gets are still **assumed**. Defaults are a
  conservative 10 Gb/s per FACET terminal and 1,500 km range; line of sight
  must clear 100 km altitude.
* **Direct-to-ground stations** (generic, assumed): 20 polar-heavy sites ×
  2 antennas × 1.2 Gb/s × 70 %.

It reports utilisation, capacity margin, how long on-board storage takes to
fill when demand exceeds capacity, alert latency via the relay (about
30 ms), and the median wait for a ground pass. At 10,000 satellites, raw
frames are 4× what the generic ground network can take and compressed
frames 1.6×. Co-adding or catalogue products fit the ground network; the
relay carries anything. Flag: `--product raw|compressed|coadded|catalogue`.

### Streak avoidance

Each step a sample of integrating telescopes is checked for sunlit
satellites (Starlink, OneWeb, other FACETs) crossing the field during the
sub-exposure. Paths come from known ephemerides and are tested in the
gnomonic plane, with line of sight clearing the atmosphere.

* With avoidance **on**, the exposure is gated for the crossing plus 2 s.
  The lost time is charged to the science ledger.
* With it **off** (`--no-streak-avoidance`), the panel reports the share of
  frames that would carry a streak.

OneWeb, above FACET, crosses upward-looking fields. Starlink, below FACET,
only appears when a telescope points low toward the limb.

### Neighbourhood notices

Every FACET satellite is re-screened every 6 h over a 30 h window against
objects within 15 km of its altitude:

* Candidate passes are found at the mutual line of nodes, then refined to
  time of closest approach (TCA) and miss distance by golden-section search
  on the true separation. The unit test checks this against brute force to
  10 m.
* Probability of collision (Pc) uses the 2-D encounter-plane formula with
  radial / in-track / cross-track covariances that grow with time to TCA.
  The covariances are assumed: GNSS-quality for FACET,
  operator-ephemeris quality for active objects, catalogue quality for
  debris. Hard-body radius is 5 m.
* Notices are issued for a miss under 1 km or Pc ≥ 1e-6. At Pc ≥ 1e-4 (a
  common operator threshold) FACET plans an along-track avoidance
  manoeuvre sized for a 5σ in-track separation; Δv is logged. Manoeuvres
  are logged, not flown: the slot is assumed to be restored.
* A circular Starlink shell at 480 km never comes within 70 km of FACET at
  550 km. Starlink notices would come only from satellites in transit.

The panel reports close approaches, notices, high-risk events and
manoeuvres per day, Δv per satellite per year, and the closest approach.

## Planetary defence exercise

A synthetic Earth impactor, played out twice side by side: **FACET + ground**
and **ground only**. Start it from the desktop *Planetary defence exercise*
panel (preset or custom: diameter, days to impact, v∞, radiant elongation from
the Sun, follow-up eyes) or with `--defence 1|2|3`. The sim sets the time rate
so the exercise takes about 2½ minutes. The ops console shows a *Planetary
defence* panel (comparison table, impact-probability chart, the object's
current state), the risk corridor on the ground map and the object on the
sky; the public page shows a plain-language card.

| Preset | Size | Out | Approach | Typical result |
|---|---|---|---|---|
| 1 Sunward Tunguska-class | 50 m, 9 Mt | 10 d | radiant 55° from the Sun | FACET finds it ~1 day out and is certain within the hour; ground never sees it |
| 2 Chelyabinsk-class from the Sun | 20 m, 0.7 Mt | 4 d | radiant 50° | FACET: hours of warning; ground: none |
| 3 Night-side city-killer | 140 m, 120 Mt | 60 d | radiant 150° | ground finds it first (deeper); FACET makes impact certain ~6 days sooner |

**Read the presets as illustrations of the method, not performance claims.**
FACET discovery here needs one trailed 60 s exposure from a single telescope.
Synthetic tracking (shift-and-stack of many short frames along trial
velocities) is not modelled, and neither is sunward stray light near the
Sun keep-out edge (see `TODO.md`). Both matter most for exactly the sunward
presets, in opposite directions: synthetic tracking would find fainter objects,
and stray light would raise the background. The ground model is a generic
composite with assumed parameters, not any real survey.

**Method.**
* *Orbit*: heliocentric two-body, universal-variable Kepler propagation. The
  true orbit is built backwards from the impact (object inside Earth's
  gravitationally focused capture disc at the impact epoch). Earth's gravity
  on the approach is not integrated; observers are geocentric.
* *Brightness*: H from diameter and albedo, H–G phase function (G = 0.15).
  Objects coming out of the daytime sky are seen at large phase angles,
  which is why they are faint and dangerous.
* *FACET search*: the telescopes actually surveying (the **NEO search
  reserve**, default 10 % of the fleet held back from the science
  programme, plus any spare eyes) cover the open sky at
  N × FOV² / (exposure + overhead). A detection needs the object brighter than
  the trailed single-exposure limit outside the keep-out zones; neighbouring
  telescopes confirm it one step later over the laser links. Then a STARE
  campaign (default 30 eyes) rate-tracks it, giving one stacked astrometric
  epoch per minute (σ ≈ 0.6·FWHM/SNR, 0.05″ floor).
* *Ground survey and follow-up*: a generic composite with **assumed**
  parameters: discovery to V 21.0, follow-up to V 22.0, solar elongation
  ≥ 60°, three sessions a day, 60 % clear, 50 % chance a night's survey covers
  the object, 0.3″ astrometry.
* *Orbit determination*: linearised weighted least squares with numerical
  partials through the propagator and a weak prior.
* *Impact probability*: covariance mapped to the b-plane, integrated over
  the capture disc. Linear methods are optimistic on very short arcs.
  Palermo scale from energy (stony density assumed), probability and time
  to impact. The risk corridor is sampled from the b-plane uncertainty.

Desktop: *% of fleet held for NEO search* sets the reserve
(`--search-share 10`). This is the trade-off a real FACET would face:
programme science against search, and it scales with fleet size.

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

