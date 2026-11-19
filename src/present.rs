//! Presentation mode: scripted, narrated tours of the simulator for pitches.
//!
//! A *deck* (one JSON file in `presentations/`) is a list of chapters. Each
//! chapter has narration text (for captions and the teleprompter), a target
//! duration, and timed *cues* that drive the simulation: fleet size, time
//! rate, camera moves, views, scenarios, failures, and on-screen cards whose
//! numbers are filled in live from the running simulation.
//!
//! Narration audio is recorded per chapter and dropped into
//! `presentations/audio/<deck-id>/<chapter-id>.{wav,ogg,flac,mp3}`. When a
//! chapter has audio, the audio clock drives the cues, so picture and voice
//! stay in sync; re-recording one chapter never disturbs the others.
//!
//! Keys: Space pause · Left/Right chapter · Home restart · C captions ·
//! T teleprompter (recording) · Esc exit.

use crate::app::{App, View};
use crate::math::V3;
use egui::{Align2, Color32, FontId, RichText};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Instant;

pub const DIR: &str = "presentations";

#[derive(Clone, Serialize, Deserialize)]
pub struct Deck {
    pub id: String,
    pub title: String,
    pub audience: String,
    #[serde(default)]
    pub note: String,
    pub chapters: Vec<Chapter>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Chapter {
    pub id: String,
    pub title: String,
    /// Target length, s (used when the chapter has no audio yet).
    pub duration_s: f64,
    /// Narration, one paragraph per entry. Sentences become captions.
    pub narration: Vec<String>,
    #[serde(default)]
    pub cues: Vec<Cue>,
    /// Optional exact caption timing [[t, text], ...]; otherwise captions are
    /// spread over the chapter in proportion to their word counts.
    #[serde(default)]
    pub captions: Vec<(f64, String)>,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Cue {
    pub t: f64,
    #[serde(flatten)]
    pub action: Action,
}

fn d1() -> f64 {
    1.5
}
fn d8() -> f64 {
    8.0
}
fn d10() -> u32 {
    10
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(tag = "do", rename_all = "snake_case")]
pub enum Action {
    /// Number of FACET satellites.
    Fleet { n: usize },
    /// Simulation time rate (× real time).
    Rate { x: f64 },
    Play,
    Pause,
    /// Reset simulation time to now.
    Now,
    /// orbit | sky | unified | split
    View { view: String },
    /// Animate the 3-D camera to the given values (any subset) over `secs`.
    Camera {
        #[serde(default)]
        yaw_deg: Option<f64>,
        #[serde(default)]
        pitch_deg: Option<f64>,
        #[serde(default)]
        dist_km: Option<f64>,
        #[serde(default)]
        fov_deg: Option<f64>,
        #[serde(default = "d1")]
        secs: f64,
    },
    /// Slowly orbit the camera (degrees of yaw per second; 0 stops).
    Spin { deg_s: f64 },
    /// Centre the sky view on a named object.
    SkyLook {
        target: String,
        #[serde(default)]
        fov_deg: Option<f64>,
    },
    /// survey | stack | mosaic | neo | queue
    Mode { mode: String },
    /// Director's target (None clears it).
    Target {
        #[serde(default)]
        name: Option<String>,
    },
    /// Start a STARE campaign on a named target.
    Stare {
        target: String,
        #[serde(default = "d10")]
        eyes: u32,
    },
    StopStares,
    /// Submit an (auto-approved) public observation request.
    Request { target: String, telescopes: u32, exposure_s: f64 },
    /// Start planetary-defence exercise preset 1, 2 or 3.
    Defence { preset: usize },
    EndDefence,
    /// Fail a fraction of the fleet now.
    Fail { fraction: f64 },
    Restore,
    ReplaceDays { days: f64 },
    Programme {
        on: bool,
        #[serde(default)]
        load: Option<f64>,
    },
    /// links | rays | coverage | names | grid | footprints | traffic
    Show { what: String, on: bool },
    /// A card of text; `{placeholders}` are filled with live values.
    Card {
        title: String,
        #[serde(default)]
        lines: Vec<String>,
        #[serde(default = "d8")]
        secs: f64,
    },
    ClearCard,
    /// baseline | optimistic | pessimistic
    CostPreset { name: String },
    /// Run `then` once `event` happens (defence_discovered, defence_certain,
    /// defence_impact, failed, recovered).
    When { event: String, then: Vec<Action> },
}

pub fn load_decks() -> (Vec<Deck>, Vec<String>) {
    let mut decks = Vec::new();
    let mut errs = Vec::new();
    if let Ok(rd) = std::fs::read_dir(DIR) {
        let mut paths: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().map_or(false, |x| x == "json")).collect();
        paths.sort();
        for p in paths {
            match std::fs::read_to_string(&p).map_err(|e| e.to_string()).and_then(|s| serde_json::from_str::<Deck>(&s).map_err(|e| e.to_string())) {
                Ok(d) => decks.push(d),
                Err(e) => errs.push(format!("{}: {e}", p.display())),
            }
        }
    } else {
        errs.push(format!("no {DIR}/ folder next to the program (run from the project folder)"));
    }
    (decks, errs)
}

pub fn audio_path(deck: &str, ch: &str) -> Option<PathBuf> {
    for ext in ["wav", "ogg", "flac", "mp3"] {
        let p = Path::new(DIR).join("audio").join(deck).join(format!("{ch}.{ext}"));
        if p.exists() {
            return Some(p);
        }
    }
    None
}

/// Split narration into caption sentences.
pub fn sentences(ch: &Chapter) -> Vec<String> {
    let mut out = Vec::new();
    for para in &ch.narration {
        let mut cur = String::new();
        let chars: Vec<char> = para.chars().collect();
        for (k, &c) in chars.iter().enumerate() {
            cur.push(c);
            let end = matches!(c, '.' | '?' | '!') && chars.get(k + 1).map_or(true, |n| n.is_whitespace());
            if end {
                let t = cur.trim().to_string();
                if !t.is_empty() {
                    out.push(t);
                }
                cur.clear();
            }
        }
        let t = cur.trim().to_string();
        if !t.is_empty() {
            out.push(t);
        }
    }
    out
}

/// Caption schedule: explicit, or proportional to word count.
pub fn caption_times(ch: &Chapter, len: f64) -> Vec<(f64, String)> {
    if !ch.captions.is_empty() {
        return ch.captions.clone();
    }
    let s = sentences(ch);
    let words: Vec<f64> = s.iter().map(|x| x.split_whitespace().count().max(1) as f64).collect();
    let total: f64 = words.iter().sum::<f64>().max(1.0);
    let span = (len - 0.8).max(1.0);
    let mut t = 0.4;
    s.into_iter()
        .zip(words)
        .map(|(x, w)| {
            let at = t;
            t += span * w / total;
            (at, x)
        })
        .collect()
}

// ---------------------------------------------------------------- audio

#[cfg(feature = "audio")]
pub struct Audio {
    stream: Option<rodio::OutputStream>,
    sink: Option<rodio::Sink>,
    pub len: Option<f64>,
    pub error: Option<String>,
}

#[cfg(feature = "audio")]
impl Audio {
    pub fn new() -> Audio {
        Audio { stream: None, sink: None, len: None, error: None }
    }
    pub fn start(&mut self, path: &Path) -> bool {
        use rodio::Source;
        self.stop();
        if self.stream.is_none() {
            match rodio::OutputStreamBuilder::open_default_stream() {
                Ok(mut s) => {
                    s.log_on_drop(false);
                    self.stream = Some(s);
                }
                Err(e) => {
                    self.error = Some(format!("audio output: {e}"));
                    return false;
                }
            }
        }
        let Some(stream) = &self.stream else { return false };
        let file = match std::fs::File::open(path) {
            Ok(f) => f,
            Err(e) => {
                self.error = Some(format!("{}: {e}", path.display()));
                return false;
            }
        };
        match rodio::Decoder::try_from(file) {
            Ok(src) => {
                self.len = src.total_duration().map(|d| d.as_secs_f64());
                let sink = rodio::Sink::connect_new(stream.mixer());
                sink.append(src);
                self.sink = Some(sink);
                self.error = None;
                true
            }
            Err(e) => {
                self.error = Some(format!("{}: {e}", path.display()));
                false
            }
        }
    }
    pub fn stop(&mut self) {
        if let Some(s) = self.sink.take() {
            s.stop();
        }
        self.len = None;
    }
    pub fn active(&self) -> bool {
        self.sink.as_ref().map_or(false, |s| !s.empty())
    }
    pub fn pos(&self) -> Option<f64> {
        self.sink.as_ref().map(|s| s.get_pos().as_secs_f64())
    }
    pub fn set_paused(&self, p: bool) {
        if let Some(s) = &self.sink {
            if p {
                s.pause()
            } else {
                s.play()
            }
        }
    }
}

#[cfg(not(feature = "audio"))]
pub struct Audio {
    pub len: Option<f64>,
    pub error: Option<String>,
}

#[cfg(not(feature = "audio"))]
impl Audio {
    pub fn new() -> Audio {
        Audio { len: None, error: Some("built without audio (feature \"audio\")".into()) }
    }
    pub fn start(&mut self, _p: &Path) -> bool {
        false
    }
    pub fn stop(&mut self) {}
    pub fn active(&self) -> bool {
        false
    }
    pub fn pos(&self) -> Option<f64> {
        None
    }
    pub fn set_paused(&self, _p: bool) {}
}

// ---------------------------------------------------------------- runtime

struct CamAnim {
    from: (f64, f64, f64, f64),
    to: (f64, f64, f64, f64),
    t0: f64,
    secs: f64,
}

pub struct Presenter {
    pub decks: Vec<Deck>,
    pub errors: Vec<String>,
    pub deck: usize,
    pub ch: usize,
    pub t: f64,
    pub playing: bool,
    pub active: bool,
    pub picker: bool,
    pub captions: bool,
    pub prompter: bool,
    next_cue: usize,
    when: Vec<(String, Vec<Action>)>,
    card: Option<(String, Vec<String>, f64)>,
    cam: Option<CamAnim>,
    spin: f64,
    last: Instant,
    audio: Audio,
    has_audio: bool,
    len: f64,
}

impl Presenter {
    pub fn new() -> Presenter {
        let (decks, errors) = load_decks();
        Presenter {
            decks,
            errors,
            deck: 0,
            ch: 0,
            t: 0.0,
            playing: false,
            active: false,
            picker: false,
            captions: true,
            prompter: false,
            next_cue: 0,
            when: Vec::new(),
            card: None,
            cam: None,
            spin: 0.0,
            last: Instant::now(),
            audio: Audio::new(),
            has_audio: false,
            len: 0.0,
        }
    }

    pub fn reload(&mut self) {
        let (d, e) = load_decks();
        self.decks = d;
        self.errors = e;
    }

    pub fn start(&mut self, app: &mut App, deck: usize) {
        self.deck = deck.min(self.decks.len().saturating_sub(1));
        self.active = true;
        self.picker = false;
        app.presenting = true;
        self.begin_chapter(app, 0);
    }

    pub fn stop(&mut self, app: &mut App) {
        app.presenting = false;
        self.audio.stop();
        self.active = false;
        self.card = None;
        self.spin = 0.0;
    }

    fn begin_chapter(&mut self, app: &mut App, ch: usize) {
        let Some(deck) = self.decks.get(self.deck) else { return };
        if ch >= deck.chapters.len() {
            self.playing = false;
            return;
        }
        self.ch = ch;
        self.next_cue = 0;
        self.when.clear();
        self.card = None;
        self.cam = None;
        self.spin = 0.0;
        let chap = &deck.chapters[ch];
        self.audio.stop();
        self.has_audio = false;
        // In teleprompter mode give the speaker a 3-second lead-in.
        self.t = if self.prompter { -3.0 } else { 0.0 };
        self.playing = true;
        self.len = chap.duration_s;
        if !self.prompter {
            if let Some(p) = audio_path(&deck.id, &chap.id) {
                self.has_audio = self.audio.start(&p);
                if let Some(l) = self.audio.len {
                    self.len = l;
                }
            }
        }
        self.last = Instant::now();
        let _ = app;
    }

    fn chapter(&self) -> Option<&Chapter> {
        self.decks.get(self.deck).and_then(|d| d.chapters.get(self.ch))
    }

    /// Advance the clock, fire cues and animate (UI thread, App locked).
    pub fn update(&mut self, app: &mut App, ctx: &egui::Context) {
        let n_ch = self.decks.get(self.deck).map_or(0, |d| d.chapters.len());
        // Keys
        let (space, left, right, home, esc, c, t) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::Space),
                i.key_pressed(egui::Key::ArrowLeft),
                i.key_pressed(egui::Key::ArrowRight),
                i.key_pressed(egui::Key::Home),
                i.key_pressed(egui::Key::Escape),
                i.key_pressed(egui::Key::C),
                i.key_pressed(egui::Key::T),
            )
        });
        if esc {
            self.stop(app);
            return;
        }
        if space {
            self.playing = !self.playing;
            self.audio.set_paused(!self.playing);
        }
        if c {
            self.captions = !self.captions;
        }
        if t {
            self.prompter = !self.prompter;
            let ch = self.ch;
            self.begin_chapter(app, ch);
        }
        if right && self.ch + 1 < n_ch {
            let ch = self.ch + 1;
            self.begin_chapter(app, ch);
        }
        if left {
            let ch = self.ch.saturating_sub(1);
            self.begin_chapter(app, ch);
        }
        if home {
            self.begin_chapter(app, 0);
        }
        // Clock
        let now = Instant::now();
        let dt = now.duration_since(self.last).as_secs_f64().min(0.25);
        self.last = now;
        if self.playing {
            match (self.has_audio, self.audio.pos()) {
                (true, Some(p)) => self.t = p,
                _ => self.t += dt,
            }
        }
        // Cues
        let cues: Vec<Cue> = self.chapter().map(|c| c.cues.clone()).unwrap_or_default();
        while self.next_cue < cues.len() && cues[self.next_cue].t <= self.t {
            let a = cues[self.next_cue].action.clone();
            self.next_cue += 1;
            self.run(app, a);
        }
        // Event-triggered cues
        let pending = std::mem::take(&mut self.when);
        for (ev, then) in pending {
            if app.present_event(&ev) {
                for a in then {
                    self.run(app, a);
                }
            } else {
                self.when.push((ev, then));
            }
        }
        // Camera animation and spin
        if let Some(a) = &self.cam {
            let u = ((self.t - a.t0) / a.secs.max(0.01)).clamp(0.0, 1.0);
            let s = u * u * (3.0 - 2.0 * u);
            let lerp = |x: f64, y: f64| x + (y - x) * s;
            app.orbit_cam.yaw = lerp(a.from.0, a.to.0);
            app.orbit_cam.pitch = lerp(a.from.1, a.to.1);
            app.orbit_cam.dist_km = (a.from.2.ln() + (a.to.2.ln() - a.from.2.ln()) * s).exp();
            app.orbit_cam.fov_deg = lerp(a.from.3, a.to.3);
            if u >= 1.0 {
                self.cam = None;
            }
        }
        if self.playing && self.spin != 0.0 {
            app.orbit_cam.yaw += (self.spin * dt).to_radians();
        }
        if let Some((_, _, until)) = &self.card {
            if self.t > *until {
                self.card = None;
            }
        }
        // Auto-advance
        let end = if self.has_audio { !self.audio.active() && self.t > 0.5 } else { self.t >= self.len };
        if self.playing && end {
            if self.ch + 1 < n_ch {
                let ch = self.ch + 1;
                self.begin_chapter(app, ch);
            } else {
                self.playing = false;
            }
        }
    }

    fn run(&mut self, app: &mut App, a: Action) {
        match a {
            Action::Camera { yaw_deg, pitch_deg, dist_km, fov_deg, secs } => {
                let c = &app.orbit_cam;
                let from = (c.yaw, c.pitch, c.dist_km, c.fov_deg);
                let to = (
                    yaw_deg.map(f64::to_radians).unwrap_or(c.yaw),
                    pitch_deg.map(f64::to_radians).unwrap_or(c.pitch),
                    dist_km.unwrap_or(c.dist_km),
                    fov_deg.unwrap_or(c.fov_deg),
                );
                self.cam = Some(CamAnim { from, to, t0: self.t, secs });
            }
            Action::Spin { deg_s } => self.spin = deg_s,
            Action::Card { title, lines, secs } => self.card = Some((title, lines, self.t + secs)),
            Action::ClearCard => self.card = None,
            Action::When { event, then } => self.when.push((event, then)),
            other => app.present_action(&other),
        }
    }

    // ------------------------------------------------------------ drawing

    /// Overlays drawn on top of the full-screen views.
    pub fn overlay(&mut self, ctx: &egui::Context, app: &App) {
        let Some(deck) = self.decks.get(self.deck) else { return };
        let Some(ch) = deck.chapters.get(self.ch) else { return };
        let screen = ctx.screen_rect();
        let p = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("present")));
        // Chapter title, top left
        p.text(egui::pos2(24.0, 18.0), Align2::LEFT_TOP, &ch.title, FontId::proportional(26.0), Color32::from_rgb(230, 238, 255));
        p.text(
            egui::pos2(24.0, 52.0),
            Align2::LEFT_TOP,
            format!("{} · {} of {}{}", deck.title, self.ch + 1, deck.chapters.len(), if self.has_audio { "" } else { " · no narration audio" }),
            FontId::proportional(13.0),
            Color32::from_gray(150),
        );
        p.text(
            egui::pos2(screen.right() - 16.0, 16.0),
            Align2::RIGHT_TOP,
            "Space pause · Left/Right chapter · C captions · T teleprompter · Esc exit",
            FontId::proportional(12.0),
            Color32::from_gray(110),
        );
        // Progress bar across the deck
        let total: f64 = deck.chapters.iter().map(|c| c.duration_s).sum::<f64>().max(1.0);
        let done: f64 = deck.chapters[..self.ch].iter().map(|c| c.duration_s).sum::<f64>() + self.t.clamp(0.0, self.len) / self.len.max(1.0) * ch.duration_s;
        let bar = egui::Rect::from_min_size(egui::pos2(0.0, screen.bottom() - 4.0), egui::vec2(screen.width(), 4.0));
        p.rect_filled(bar, 0.0, Color32::from_rgb(25, 30, 40));
        p.rect_filled(egui::Rect::from_min_size(bar.min, egui::vec2(screen.width() * (done / total) as f32, 4.0)), 0.0, Color32::from_rgb(79, 163, 255));
        let mut acc = 0.0;
        for c in &deck.chapters {
            acc += c.duration_s;
            let x = screen.width() * (acc / total) as f32;
            p.line_segment([egui::pos2(x, bar.top() - 3.0), egui::pos2(x, bar.bottom())], egui::Stroke::new(1.0_f32, Color32::from_gray(90)));
        }
        if !self.playing {
            p.text(screen.center(), Align2::CENTER_CENTER, "paused", FontId::proportional(18.0), Color32::from_gray(170));
        }
        // Card
        if let Some((title, lines, _)) = &self.card {
            let vars = app.present_vars();
            let fill = |s: &str| {
                let mut out = s.to_string();
                for (k, v) in &vars {
                    out = out.replace(&format!("{{{k}}}"), v);
                }
                out
            };
            egui::Area::new(egui::Id::new("present_card")).fixed_pos(egui::pos2(24.0, 90.0)).order(egui::Order::Foreground).show(ctx, |ui| {
                egui::Frame::none().fill(Color32::from_rgba_unmultiplied(8, 12, 22, 225)).rounding(8.0).inner_margin(16.0).stroke(egui::Stroke::new(1.0_f32, Color32::from_rgb(40, 60, 90))).show(ui, |ui| {
                    ui.set_max_width(520.0);
                    ui.label(RichText::new(fill(title)).size(22.0).strong().color(Color32::from_rgb(159, 208, 255)));
                    for l in lines {
                        ui.add_space(4.0);
                        ui.label(RichText::new(fill(l)).size(17.0).color(Color32::from_gray(230)));
                    }
                });
            });
        }
        // Captions or teleprompter
        let caps = caption_times(ch, self.len);
        let cur = caps.iter().rposition(|(t, _)| *t <= self.t.max(0.0)).unwrap_or(0);
        if self.prompter {
            egui::Area::new(egui::Id::new("prompter")).anchor(Align2::CENTER_BOTTOM, egui::vec2(0.0, -24.0)).order(egui::Order::Foreground).show(ctx, |ui| {
                egui::Frame::none().fill(Color32::from_rgba_unmultiplied(0, 0, 0, 215)).rounding(8.0).inner_margin(18.0).show(ui, |ui| {
                    ui.set_width((screen.width() * 0.8).min(1100.0));
                    let left = self.len - self.t;
                    ui.label(
                        RichText::new(if self.t < 0.0 {
                            format!("Recording · starts in {:.0}", -self.t.floor())
                        } else {
                            format!("Recording · {:.0} s / {:.0} s · {:.0} s left", self.t, self.len, left.max(0.0))
                        })
                        .size(14.0)
                        .color(if self.t < 0.0 { Color32::from_rgb(255, 196, 107) } else { Color32::from_rgb(255, 110, 110) }),
                    );
                    for (k, (_, s)) in caps.iter().enumerate() {
                        let col = if k == cur && self.t >= 0.0 {
                            Color32::WHITE
                        } else if k < cur {
                            Color32::from_gray(90)
                        } else {
                            Color32::from_gray(170)
                        };
                        ui.label(RichText::new(s).size(if k == cur { 26.0 } else { 20.0 }).color(col));
                    }
                });
            });
        } else if self.captions {
            if let Some((_, s)) = caps.get(cur) {
                if self.t >= 0.0 {
                    egui::Area::new(egui::Id::new("caption")).anchor(Align2::CENTER_BOTTOM, egui::vec2(0.0, -28.0)).order(egui::Order::Foreground).show(ctx, |ui| {
                        egui::Frame::none().fill(Color32::from_rgba_unmultiplied(0, 0, 0, 170)).rounding(6.0).inner_margin(egui::Margin::symmetric(16.0, 8.0)).show(ui, |ui| {
                            ui.set_max_width((screen.width() * 0.75).min(1000.0));
                            ui.label(RichText::new(s).size(22.0).color(Color32::WHITE));
                        });
                    });
                }
            }
        }
    }

    /// Deck picker window (shown over the normal UI).
    pub fn picker_ui(&mut self, ctx: &egui::Context, app: &mut App) {
        let mut open = self.picker;
        let mut start = None;
        egui::Window::new("Presentation mode").open(&mut open).default_width(560.0).collapsible(false).show(ctx, |ui| {
            ui.label("Choose the presentation for the audience in front of you. Full screen; Esc returns here.");
            ui.checkbox(&mut self.prompter, "Teleprompter / recording mode (no audio; script on screen with a 3 s lead-in per chapter)");
            ui.separator();
            for (k, d) in self.decks.iter().enumerate() {
                let total: f64 = d.chapters.iter().map(|c| c.duration_s).sum();
                let with_audio = d.chapters.iter().filter(|c| audio_path(&d.id, &c.id).is_some()).count();
                ui.horizontal(|ui| {
                    if ui.button("Start").clicked() {
                        start = Some(k);
                    }
                    ui.vertical(|ui| {
                        ui.label(RichText::new(&d.title).strong());
                        ui.label(
                            RichText::new(format!(
                                "{} · {} chapters · about {}:{:02} · narration recorded for {}/{} chapters",
                                d.audience,
                                d.chapters.len(),
                                (total / 60.0).floor(),
                                (total % 60.0).round(),
                                with_audio,
                                d.chapters.len()
                            ))
                            .small()
                            .weak(),
                        );
                    });
                });
                ui.add_space(4.0);
            }
            if self.decks.is_empty() {
                ui.label(RichText::new(format!("No decks found in {DIR}/.")).color(Color32::from_rgb(255, 180, 60)));
            }
            for e in &self.errors {
                ui.label(RichText::new(e).small().color(Color32::from_rgb(255, 140, 120)));
            }
            if let Some(e) = &self.audio.error {
                ui.label(RichText::new(e).small().color(Color32::from_rgb(255, 140, 120)));
            }
            ui.separator();
            ui.label(RichText::new(format!("Audio: {DIR}/audio/<deck-id>/<chapter-id>.wav|ogg|flac|mp3")).small().weak());
            if ui.button("Reload decks").clicked() {
                self.reload();
            }
        });
        self.picker = open && start.is_none();
        if let Some(k) = start {
            self.start(app, k);
        }
    }
}

// ---------------------------------------------------------------- App side

impl App {
    fn resolve_dir(&self, name: &str) -> Option<V3> {
        if name.eq_ignore_ascii_case("defence") {
            return self.defence_marker().map(|(d, _)| d);
        }
        if let Some(i) = crate::astro::BODY_NAMES.iter().position(|b| b.eq_ignore_ascii_case(name)) {
            return Some(self.bodies[i].dir);
        }
        if let Some(t) = crate::app::FIXED_TARGETS.iter().find(|t| t.0.to_lowercase().contains(&name.to_lowercase())) {
            return Some(V3::from_radec(t.1.to_radians(), t.2.to_radians()));
        }
        self.names
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case(name))
            .map(|s| s.dir)
            .or_else(|| crate::catalog::find_dso(name).map(|d| d.dir))
    }

    pub fn present_action(&mut self, a: &Action) {
        use crate::app::Target;
        match a {
            Action::Fleet { n } => self.shell_params.n = (*n).clamp(2, 10_000),
            Action::Rate { x } => self.rate = x.max(0.0),
            Action::Play => self.playing = true,
            Action::Pause => self.playing = false,
            Action::Now => self.jd = crate::astro::jd_now(),
            Action::View { view } => {
                self.view = match view.as_str() {
                    "sky" => View::Sky,
                    "unified" => View::Unified,
                    "split" => View::Split,
                    _ => View::Orbit,
                }
            }
            Action::SkyLook { target, fov_deg } => {
                if let Some(d) = self.resolve_dir(target) {
                    self.look_at(d);
                }
                if let Some(f) = fov_deg {
                    self.sky_cam.fov_deg = *f;
                }
            }
            Action::Mode { mode } => {
                if let Some(m) = crate::obs::Mode::from_key(mode) {
                    self.mode = m;
                }
            }
            Action::Target { name } => {
                self.target = name.as_ref().and_then(|n| {
                    if let Some(i) = crate::astro::BODY_NAMES.iter().position(|b| b.eq_ignore_ascii_case(n)) {
                        Some(Target::Body(i))
                    } else {
                        self.resolve_dir(n).map(|dir| Target::Fixed { name: n.clone(), dir })
                    }
                });
            }
            Action::Stare { target, eyes } => {
                if let Some(w) = &self.web {
                    let mut sh = w.lock().unwrap();
                    let _ = crate::server::start_campaign(&mut sh, Some(target), None, None, Some(*eyes), Some(60.0), None);
                }
            }
            Action::StopStares => {
                if let Some(w) = &self.web {
                    let mut sh = w.lock().unwrap();
                    for c in sh.campaigns.iter_mut().filter(|c| c.active) {
                        c.active = false;
                        c.paused = false;
                        c.armed = c.preset;
                        c.assigned_now = 0;
                        c.locked_now = 0;
                    }
                }
            }
            Action::Request { target, telescopes, exposure_s } => self.demo_request(target, *telescopes, *exposure_s),
            Action::Defence { preset } => {
                let p = crate::defence::presets();
                let cfg = p[preset.saturating_sub(1).min(p.len() - 1)].clone();
                self.start_defence(cfg);
            }
            Action::EndDefence => {
                if let Some(sc) = self.defence.take() {
                    if let (Some(id), Some(w)) = (sc.campaign, &self.web) {
                        if let Some(c) = w.lock().unwrap().campaigns.iter_mut().find(|c| c.id == id) {
                            c.active = false;
                        }
                    }
                }
            }
            Action::Fail { fraction } => self.fail_now(*fraction, "presentation"),
            Action::Restore => self.restore_all(),
            Action::ReplaceDays { days } => self.resil.replace_days = *days,
            Action::Programme { on, load } => {
                self.gen.on = *on;
                if let Some(l) = load {
                    self.gen.load = *l;
                }
            }
            Action::Show { what, on } => match what.as_str() {
                "links" => self.show.links = *on,
                "rays" => self.show.rays = *on,
                "coverage" => self.show.coverage = *on,
                "names" => self.show.names = *on,
                "grid" => self.show.grid = *on,
                "footprints" => self.show.footprints = *on,
                "traffic" => self.traffic.cfg.show = *on,
                _ => {}
            },
            Action::CostPreset { name } => {
                let keep = (self.cost_model.follow_sim, self.cost_model.fleet);
                self.cost_model = match name.as_str() {
                    "optimistic" => crate::costs::Model::optimistic(),
                    "pessimistic" => crate::costs::Model::pessimistic(),
                    _ => crate::costs::Model::baseline(),
                };
                (self.cost_model.follow_sim, self.cost_model.fleet) = keep;
            }
            // presenter-side actions are handled in Presenter::run
            Action::Camera { .. } | Action::Spin { .. } | Action::Card { .. } | Action::ClearCard | Action::When { .. } => {}
        }
    }

    pub fn present_event(&self, ev: &str) -> bool {
        match ev {
            "defence_discovered" => self.defence.as_ref().map_or(false, |d| d.discovered()),
            "defence_certain" => self.defence.as_ref().map_or(false, |d| d.systems()[0].certain_s.is_some()),
            "defence_impact" => self.defence.as_ref().map_or(false, |d| d.impacted),
            "failed" => self.resil.n_failed() > 0,
            "recovered" => self.resil.n_failed() == 0,
            _ => false,
        }
    }

    /// Submit an auto-approved public request (for the "how it works" scene).
    pub fn demo_request(&mut self, target: &str, telescopes: u32, exposure_s: f64) {
        let Some(w) = self.web.clone() else { return };
        let mut sh = w.lock().unwrap();
        let (body, name, dir) = match sh.resolve(target) {
            Some(crate::server::Resolved::Body(i, n)) => (Some(i), n, self.bodies[i].dir),
            Some(crate::server::Resolved::Fixed(n, d)) => (None, n, d),
            None => return,
        };
        let id = sh.next_id;
        sh.next_id += 1;
        let (ra, dec) = dir.to_radec();
        let utc = crate::astro::format_utc(self.jd);
        sh.requests.push(crate::server::ObsRequest {
            id,
            submitter: "a member of the public".into(),
            target_name: name.clone(),
            body,
            ra_deg: ra.to_degrees(),
            dec_deg: dec.to_degrees(),
            telescopes,
            exposure_s,
            priority: 5,
            state: crate::server::ReqState::Approved,
            progress_s: 0.0,
            assigned_now: 0,
            blocked: None,
            submitted_utc: utc,
            completed_utc: None,
            note: "presentation".into(),
            ip: String::new(),
            dir,
            hw: None,
            obs_jd: 0.0,
            locked_now: 0,
            approved_jd: Some(self.jd),
            first_light_jd: None,
            full_lock_jd: None,
            jit2_s: 0.0,
            fine_s: 0.0,
            slews: 0,
            slew_deg_sum: 0.0,
            agility: None,
            programme: None,
            deadline_s: None,
            submit_jd: self.jd,
            case_idx: None,
            inv_idx: None,
        });
        sh.log("info", format!("Public request #{id}: {name}, {telescopes} × {exposure_s:.0} s (presentation)"));
    }

    /// Live values for card placeholders.
    pub fn present_vars(&self) -> Vec<(&'static str, String)> {
        let mut v = Vec::new();
        let n = self.shell.len();
        v.push(("fleet", format!("{n}")));
        v.push(("public_share", format!("{:.0} %", 100.0 * self.public_share)));
        let sci = self.science.summary(&self.gen.investigators);
        let f = |x: &serde_json::Value| x.as_f64().unwrap_or(0.0);
        v.push(("completed_per_day", format!("{:.0}", f(&sci["per_day"]["completed"]))));
        v.push(("tel_h_per_day", format!("{:.0}", f(&sci["per_day"]["tel_h"]))));
        v.push(("on_time", sci["on_time_pct"].as_f64().map(|p| format!("{p:.0} %")).unwrap_or("–".into())));
        v.push(("teams", format!("{}", sci["investigators_served"])));
        let s = &self.science;
        let duty = s.duty().unwrap_or(0.72);
        v.push(("duty", format!("{:.0} %", 100.0 * duty)));
        {
            let a = self.allsky();
            use crate::allsky::fmt_span;
            v.push(("allsky_time", fmt_span(a.passes[0].seconds)));
            v.push(("allsky_deep_time", fmt_span(a.passes[1].seconds)));
            v.push(("allsky_hour_time", fmt_span(a.passes[2].seconds)));
            v.push(("allsky_per_day", format!("{:.0}", a.per_day_60s)));
            for &(n, s60, _, _) in &a.stages {
                match n {
                    1000 => v.push(("allsky_time_1k", fmt_span(s60))),
                    10_000 => v.push(("allsky_time_10k", fmt_span(s60))),
                    _ => {}
                }
            }
            v.push(("allsky_deg2", format!("{:.0}", a.available_deg2)));
            v.push(("allsky_depth", format!("{:.1}", a.passes[0].depth_v)));
        }
        {
            let e = &self.env;
            let alt = self.shell.params.alt_km;
            if let Some(r) = e.trade.iter().find(|r| (r.alt_km - alt).abs() < 1.0) {
                v.push(("saa_pct", format!("{:.0} %", 100.0 * r.saa_frac)));
                v.push(("disposal_dv", format!("{:.0} m/s", r.disposal_dv)));
            }
            if let Some((m, _, _, cy)) = e.life {
                v.push(("lifetime", crate::environment::fmt_years(m)));
                v.push(("lifetime_cycle", crate::environment::fmt_years(cy)));
            }
            if e.exposed_s > 0.0 {
                v.push(("earthshine_lost", format!("{:.0} %", 100.0 * e.earthshine_lost_s / e.exposed_s)));
            }
        }
        let raw = s.bytes / 1e12 / (s.elapsed_s / 86_400.0).max(1e-9);
        v.push(("raw_tb_day", format!("{raw:.0} TB")));
        if let Some(g) = &self.resil.ground {
            v.push(("ground_duty", format!("{:.0} %", 100.0 * g.ground_duty)));
            v.push(("ground_equiv", format!("{:.0}", g.equivalent_ground_telescopes)));
            v.push(("ground_sky_now", format!("{:.0} %", 100.0 * g.ground_sky_now)));
            v.push(("facet_sky_now", format!("{:.0} %", 100.0 * g.facet_sky_now)));
        } else {
            for k in ["ground_duty", "ground_equiv", "ground_sky_now", "facet_sky_now"] {
                v.push((k, "…".into()));
            }
        }
        let r = &self.resil;
        let last = r.series.last();
        v.push(("alive", format!("{:.0} %", 100.0 * (1.0 - r.n_failed() as f64 / n.max(1) as f64))));
        v.push(("output", last.map(|x| format!("{:.0} %", 100.0 * x[2])).unwrap_or("–".into())));
        v.push(("observations", last.map(|x| format!("{:.0} %", 100.0 * x[3])).unwrap_or("–".into())));
        let dur = |x: Option<f64>| x.map(crate::defence::fmt_dur).unwrap_or_else(|| "not found".into());
        if let Some(d) = &self.defence {
            let [a, b] = d.systems();
            v.push(("pd_name", d.designation.clone()));
            v.push(("pd_size", format!("{:.0} m", d.cfg.diameter_m)));
            v.push(("pd_energy", format!("{:.0} Mt", d.energy_mt)));
            v.push(("pd_warning", dur(a.warning_s)));
            v.push(("pd_warning_ground", dur(b.warning_s)));
            v.push(("pd_ip", a.ip.map(|p| if p > 0.9999 { "certain".into() } else { format!("{:.1} %", 100.0 * p) }).unwrap_or("–".into())));
            v.push(("pd_certain", dur(a.certain_s)));
            v.push(("pd_left", crate::defence::fmt_dur((d.impact_jd - self.jd) * 86_400.0)));
        }
        let c = self.cost_result();
        let money = crate::costs::fmt_money;
        v.push(("cost_total", money(c.total)));
        v.push(("cost_years", format!("{}", self.cost_model.horizon_years)));
        v.push(("cost_fleet", format!("{}", c.fleet)));
        v.push(("cost_per_tel_h", c.cost_per_tel_hour.map(money).unwrap_or("–".into())));
        v.push(("cost_per_obs", c.cost_per_observation.map(money).unwrap_or("–".into())));
        v.push(("cost_unit", money(c.avg_unit_cost)));
        v.push(("cost_first_science", c.first_science_year.map(|y| y.to_string()).unwrap_or("–".into())));
        v.push(("cost_full_fleet", c.full_fleet_year.map(|y| y.to_string()).unwrap_or("–".into())));
        for (t, label) in [(100e6, "100m"), (250e6, "250m"), (500e6, "500m"), (1e9, "1b")] {
            let s = c
                .tranches
                .iter()
                .find(|x| x.0 == t)
                .map(|(_, d, f)| match d {
                    Some(d) => format!("{} satellites by {}", f.round(), d.floor() as i32),
                    None => format!("the whole plan ({} satellites)", f.round()),
                })
                .unwrap_or_default();
            v.push((match label {
                "100m" => "tranche_100m",
                "250m" => "tranche_250m",
                "500m" => "tranche_500m",
                _ => "tranche_1b",
            }, s));
        }
        let tj = self.traffic.json(raw, n);
        v.push(("streaked", format!("{:.1} %", tj["streaks"]["streaked_pct"].as_f64().unwrap_or(0.0))));
        v.push(("streak_cost", format!("{:.1} %", tj["streaks"]["avoidance_cost_pct"].as_f64().unwrap_or(0.0))));
        v.push(("relay_latency", tj["data"]["relay_latency_ms"].as_f64().map(|x| format!("{x:.0} ms")).unwrap_or("–".into())));
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_decks_parse_and_have_narration() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join(DIR);
        let mut n = 0;
        for e in std::fs::read_dir(&dir).unwrap().flatten() {
            let p = e.path();
            if p.extension().map_or(false, |x| x == "json") {
                let d: Deck = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
                let words: usize = d.chapters.iter().flat_map(|c| c.narration.iter()).map(|s| s.split_whitespace().count()).sum();
                let secs: f64 = d.chapters.iter().map(|c| c.duration_s).sum();
                eprintln!("{}: {} chapters, {:.0} s, {} words ({:.0} wpm)", d.id, d.chapters.len(), secs, words, words as f64 / secs * 60.0);
                assert!(!d.chapters.is_empty());
                // speaking pace within a natural range
                let wpm = words as f64 / secs * 60.0;
                assert!((110.0..=165.0).contains(&wpm), "{}: {wpm:.0} wpm", d.id);
                for c in &d.chapters {
                    assert!(!sentences(c).is_empty(), "{} / {} has no narration", d.id, c.id);
                    for cue in &c.cues {
                        assert!(cue.t <= c.duration_s + 1.0, "{} / {}: cue at {} past the end", d.id, c.id, cue.t);
                    }
                }
                n += 1;
            }
        }
        assert!(n >= 4, "expected at least 4 decks, found {n}");
    }

    #[test]
    fn captions_cover_the_chapter() {
        let ch = Chapter {
            id: "x".into(),
            title: "X".into(),
            duration_s: 20.0,
            narration: vec!["One two three. Four five six seven eight! Nine?".into()],
            cues: vec![],
            captions: vec![],
        };
        let c = caption_times(&ch, 20.0);
        assert_eq!(c.len(), 3);
        assert!(c[0].0 < c[1].0 && c[1].0 < c[2].0 && c[2].0 < 20.0);
    }
}
