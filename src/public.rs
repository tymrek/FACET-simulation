//! Public engagement: a modelled public user base sending real requests
//! through the same queue and fleet as everyone else, and the metrics a
//! funder or an outreach partner would ask for.
//!
//! Every adoption input here is **ASSUMED** (labelled as such in the UI and
//! on the public site): how many people use FACET each month, how they split
//! between casual visitors, amateur astronomers, classrooms and citizen
//! scientists, how often each comes back and what they ask for. They are
//! adjustable and meant to be anchored later against comparable programmes
//! (robotic-telescope education networks, online telescope services).
//!
//! What is *not* assumed is what happens to the requests: they are ordinary
//! public-pool requests, scheduled, slewed to, exposed and completed (or
//! not) by the simulated fleet, so waits, completions and turn-aways are
//! measured.
//!
//! Requests from real visitors to the public site are counted separately as
//! the "live" line and never mixed into the modelled figures.

use crate::app::App;
use crate::math::{deg, V3};
use crate::server::{ObsRequest, ReqState};
use serde::Serialize;
use std::collections::HashMap;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum Segment {
    Casual,
    Amateur,
    Classroom,
    Citizen,
}

impl Segment {
    pub const ALL: [Segment; 4] = [Segment::Casual, Segment::Amateur, Segment::Classroom, Segment::Citizen];
    pub fn label(self) -> &'static str {
        match self {
            Segment::Casual => "casual visitors",
            Segment::Amateur => "amateur astronomers",
            Segment::Classroom => "classrooms",
            Segment::Citizen => "citizen scientists",
        }
    }
    fn idx(self) -> usize {
        self as usize
    }
}

/// ASSUMED behaviour of one segment.
#[derive(Clone, Serialize)]
pub struct SegCfg {
    /// Share of the monthly active population.
    pub share: f64,
    /// Visits (sessions) per active user per month.
    pub visits_month: f64,
    /// Requests per visit (a classroom session: one per student group).
    pub req_visit: f64,
    pub telescopes: u32,
    pub exposure_s: f64,
    /// Fair-use quota: requests per user per day.
    pub quota_day: u32,
}

#[derive(Clone, Serialize)]
pub struct PublicCfg {
    pub on: bool,
    /// Monthly active users, ASSUMED.
    pub population: usize,
    pub seg: [SegCfg; 4],
    /// A public request not completed within this many hours is dropped.
    pub patience_h: f64,
    /// Interest spike for an asteroid passing within 2 lunar distances, and
    /// while a planetary-defence exercise is running.
    pub spike_neo: f64,
    pub spike_defence: f64,
}

impl Default for PublicCfg {
    fn default() -> Self {
        let s = |share, visits_month, req_visit, telescopes, exposure_s, quota_day| SegCfg { share, visits_month, req_visit, telescopes, exposure_s, quota_day };
        PublicCfg {
            on: true,
            population: 20_000,
            seg: [
                s(0.55, 1.2, 1.5, 1, 60.0, 3),
                s(0.25, 8.0, 3.0, 4, 300.0, 10),
                s(0.10, 4.0, 8.0, 2, 120.0, 40),
                s(0.10, 12.0, 5.0, 8, 300.0, 20),
            ],
            patience_h: 48.0,
            spike_neo: 4.0,
            spike_defence: 3.0,
        }
    }
}

#[derive(Clone, Default)]
struct User {
    first_jd: f64,
    last_jd: f64,
    visits: u32,
    today: u32,
    today_day: i64,
}

#[derive(Clone, Default, Serialize)]
pub struct DayStats {
    pub day: i64,
    pub submitted: [u32; 4],
    pub completed: [u32; 4],
    pub quota: [u32; 4],
    pub full: [u32; 4],
    pub expired: [u32; 4],
    pub wait_s: Vec<f32>,
    pub new_users: u32,
    pub active_users: u32,
    pub classroom_sessions: u32,
    pub tel_s_demand: f64,
}

#[derive(Default)]
pub struct Public {
    pub cfg: PublicCfg,
    users: Vec<User>,
    seg_users: [Vec<u32>; 4],
    rng: u64,
    acc: [f64; 4],
    open: HashMap<u64, (u32, u8, f64)>,
    pub days: Vec<DayStats>,
    pub live: LiveStats,
    live_seen: std::collections::HashSet<u64>,
    live_done: std::collections::HashSet<u64>,
    pop_key: usize,
}

#[derive(Default, Clone, Serialize)]
pub struct LiveStats {
    pub submitted: u32,
    pub completed: u32,
    pub open: u32,
    pub wait_s: Vec<f32>,
}

fn median(v: &[f32]) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    Some(s[s.len() / 2] as f64)
}

/// Requests arrive mostly in local evenings (classrooms: school hours on
/// weekdays in term). Three population centres: Americas (UTC−6, 30 %),
/// Europe/Africa (UTC+1, 35 %), Asia-Pacific (UTC+8, 35 %).
fn diurnal(seg: Segment, jd: f64) -> f64 {
    let regions = [(-6.0, 0.30), (1.0, 0.35), (8.0, 0.35)];
    let ut_h = ((jd + 0.5).fract()) * 24.0;
    let dow = ((jd + 1.5).floor() as i64).rem_euclid(7); // 0 = Sunday
    let month = crate::astro::format_utc(jd)[5..7].parse::<u32>().unwrap_or(1);
    let mut w = 0.0;
    for (tz, frac) in regions {
        let h = (ut_h + tz).rem_euclid(24.0);
        let f = match seg {
            Segment::Classroom => {
                let dom = crate::astro::format_utc(jd)[8..10].parse::<u32>().unwrap_or(1);
                let term = !matches!(month, 7 | 8) && !(month == 12 && dom > 20) && !(month == 1 && dom < 3);
                let weekday = (1..=5).contains(&dow);
                if term && weekday && (9.0..15.0).contains(&h) {
                    3.4
                } else {
                    0.0
                }
            }
            _ => 0.25 + 2.2 * (-((h - 21.0) / 3.0).powi(2)).exp() + if dow == 0 || dow == 6 { 0.2 } else { 0.0 },
        };
        w += frac * f;
    }
    w
}

impl Public {
    fn next(&mut self) -> u64 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        x
    }
    fn unif(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn ensure_population(&mut self) {
        if self.pop_key == self.cfg.population && !self.users.is_empty() {
            return;
        }
        self.pop_key = self.cfg.population;
        if self.rng == 0 {
            self.rng = 0x9e37_79b9_7f4a_7c15;
        }
        self.users.clear();
        self.seg_users = Default::default();
        for s in Segment::ALL {
            let n = (self.cfg.population as f64 * self.cfg.seg[s.idx()].share).round() as usize;
            for _ in 0..n {
                self.seg_users[s.idx()].push(self.users.len() as u32);
                self.users.push(User::default());
            }
        }
    }
    fn day_mut(&mut self, day: i64) -> &mut DayStats {
        if self.days.last().map_or(true, |d| d.day != day) {
            // Close the previous day: users active and new on it.
            if let Some(prev) = self.days.last_mut() {
                let pd = prev.day as f64;
                prev.active_users = self.users.iter().filter(|u| u.last_jd >= pd && u.last_jd < pd + 1.0).count() as u32;
                prev.new_users = self.users.iter().filter(|u| u.first_jd >= pd && u.first_jd < pd + 1.0).count() as u32;
            }
            self.days.push(DayStats { day, ..Default::default() });
            if self.days.len() > 60 {
                self.days.remove(0);
            }
        }
        self.days.last_mut().unwrap()
    }
    pub fn reset(&mut self) {
        let cfg = self.cfg.clone();
        *self = Public { cfg, ..Default::default() };
    }
}

impl App {
    /// Generate public demand for this step and track its requests.
    pub fn public_tick(&mut self, dt: f64) {
        let Some(web) = self.web.clone() else { return };
        if dt <= 0.0 {
            return;
        }
        let jd = self.jd;
        let day = jd.floor() as i64;
        let (sun, moon) = (self.bodies[0].dir, self.bodies[1].dir);
        let o = self.obs_params.clone();
        let blocked = |d: V3| crate::obs::sky_blocked(&o, d, sun, moon).is_some();
        let bodies: Vec<V3> = self.bodies.iter().map(|b| b.dir).collect();
        let neo_spike = self.neo.approaches.iter().any(|a| (a.jd - jd).abs() < 1.0 && a.dist_ld < 2.0);
        let defence = self.defence.is_some();
        let public_eyes = (self.public_share * self.shell.len() as f64).max(1.0);
        let neo_t = self.gen.neo_targets.clone();
        let mut sh = web.lock().unwrap();
        let p = &mut self.public;
        p.ensure_population();
        p.day_mut(day);

        // ---- live line: real visitors' requests (no programme, not modelled)
        let mut live_open = 0;
        for r in sh.requests.iter() {
            if r.programme.is_none() && !r.ip.is_empty() {
                if p.live_seen.insert(r.id) {
                    p.live.submitted += 1;
                }
                if r.state.is_open() {
                    live_open += 1;
                } else if r.state == ReqState::Done && p.live_done.insert(r.id) {
                    p.live.completed += 1;
                    p.live.wait_s.push(((jd - r.submit_jd) * 86_400.0) as f32);
                }
            }
        }
        p.live.open = live_open;

        // ---- completions and drop-outs of modelled requests
        let patience = p.cfg.patience_h / 24.0;
        let mut closed: Vec<(u64, u8, Option<f64>, bool)> = Vec::new();
        for r in sh.requests.iter_mut() {
            if let Some(&(_, seg, t0)) = p.open.get(&r.id) {
                match r.state {
                    ReqState::Done => closed.push((r.id, seg, Some((jd - t0) * 86_400.0), false)),
                    ReqState::Rejected | ReqState::Cancelled | ReqState::Expired | ReqState::Failed => closed.push((r.id, seg, None, true)),
                    _ if jd - t0 > patience => {
                        r.state = ReqState::Expired;
                        r.assigned_now = 0;
                        r.locked_now = 0;
                        closed.push((r.id, seg, None, true));
                    }
                    _ => {}
                }
            }
        }
        for (id, seg, wait, exp) in closed {
            p.open.remove(&id);
            let d = p.days.last_mut().unwrap();
            if let Some(w) = wait {
                d.completed[seg as usize] += 1;
                d.wait_s.push(w as f32);
            } else if exp {
                d.expired[seg as usize] += 1;
            }
        }
        if !p.cfg.on {
            return;
        }

        // ---- arrivals
        let cap = (2.0 * public_eyes).max(100.0) as usize;
        let mut spike = 1.0;
        if neo_spike {
            spike *= p.cfg.spike_neo;
        }
        if defence {
            spike *= p.cfg.spike_defence;
        }
        let mut new_reqs: Vec<ObsRequest> = Vec::new();
        for s in Segment::ALL {
            let c = p.cfg.seg[s.idx()].clone();
            let users = p.seg_users[s.idx()].len() as f64;
            // Visits per day, shaped by the daily and weekly cycle (the shape
            // factors normalise each cycle to a mean of 1).
            let visits_day = users * c.visits_month / 30.44;
            let shape = if s == Segment::Classroom { 1.0 / (3.4 * 0.25 * 5.0 / 7.0) } else { 1.0 / 0.794 };
            let lam = visits_day * diurnal(s, jd) * shape * if s == Segment::Casual { spike } else { 1.0 } * dt / 86_400.0;
            p.acc[s.idx()] += lam;
            while p.acc[s.idx()] >= 1.0 {
                p.acc[s.idx()] -= 1.0;
                if p.seg_users[s.idx()].is_empty() {
                    break;
                }
                let k = (p.next() % p.seg_users[s.idx()].len() as u64) as usize;
                let uid = p.seg_users[s.idx()][k];
                {
                    let u = &mut p.users[uid as usize];
                    if u.today_day != day {
                        u.today_day = day;
                        u.today = 0;
                    }
                    if u.first_jd == 0.0 {
                        u.first_jd = jd;
                    }
                    u.last_jd = jd;
                    u.visits += 1;
                }
                if s == Segment::Classroom {
                    p.days.last_mut().unwrap().classroom_sessions += 1;
                }
                // This visit's requests: the mean req_visit, integer part plus
                // a chance of one more.
                let n_req = c.req_visit.floor() as u32 + u32::from(p.unif() < c.req_visit.fract());
                for _ in 0..n_req.max(1) {
                    let d = p.days.last_mut().unwrap();
                    d.submitted[s.idx()] += 1;
                    if p.users[uid as usize].today >= c.quota_day {
                        d.quota[s.idx()] += 1;
                        continue;
                    }
                    if p.open.len() + new_reqs.len() >= cap {
                        d.full[s.idx()] += 1;
                        continue;
                    }
                    let Some((name, body, dir)) = pick_target(p, s, &bodies, &neo_t, &blocked) else {
                        continue;
                    };
                    p.users[uid as usize].today += 1;
                    let d = p.days.last_mut().unwrap();
                    d.tel_s_demand += c.telescopes as f64 * c.exposure_s;
                    let id = sh.next_id;
                    sh.next_id += 1;
                    let (ra, dec) = dir.to_radec();
                    p.open.insert(id, (uid, s as u8, jd));
                    new_reqs.push(ObsRequest {
                        id,
                        submitter: format!("{} #{}", match s { Segment::Casual => "visitor", Segment::Amateur => "amateur", Segment::Classroom => "class", Segment::Citizen => "citizen" }, uid),
                        neo: neo_t.iter().find(|x| x.1 == name).map(|x| x.2),
                        target_name: name,
                        body,
                        moving: None,
                        ra_deg: deg(ra),
                        dec_deg: deg(dec),
                        telescopes: c.telescopes,
                        exposure_s: c.exposure_s,
                        priority: 0,
                        state: ReqState::Approved,
                        progress_s: 0.0,
                        assigned_now: 0,
                        blocked: None,
                        submitted_utc: sh.utc_now.clone(),
                        completed_utc: None,
                        note: format!("modelled public demand: {}", s.label()),
                        ip: String::new(),
                        dir,
                        hw: None,
                        obs_jd: 0.0,
                        locked_now: 0,
                        approved_jd: Some(jd),
                        first_light_jd: None,
                        full_lock_jd: None,
                        jit2_s: 0.0,
                        fine_s: 0.0,
                        slews: 0,
                        slew_deg_sum: 0.0,
                        agility: None,
                        sched: Default::default(),
                        programme: None,
                        deadline_s: None,
                        submit_jd: jd,
                        case_idx: None,
                        inv_idx: None,
                        mosaic: None,
                    });
                }
            }
        }
        sh.requests.extend(new_reqs);
        sh.prune();
    }

    pub fn public_json(&self) -> serde_json::Value {
        let p = &self.public;
        // Trailing 24 h is the last complete day when there is one.
        let full: Vec<&DayStats> = if p.days.len() > 1 { p.days[..p.days.len() - 1].iter().collect() } else { p.days.iter().collect() };
        let last = full.last().copied();
        let sum = |f: &dyn Fn(&DayStats) -> u32| -> u32 { full.iter().map(|d| f(d)).sum() };
        let ndays = full.len().max(1) as f64;
        let completed: u32 = sum(&|d| d.completed.iter().sum());
        let submitted: u32 = sum(&|d| d.submitted.iter().sum());
        let quota: u32 = sum(&|d| d.quota.iter().sum());
        let fullq: u32 = sum(&|d| d.full.iter().sum());
        let expired: u32 = sum(&|d| d.expired.iter().sum());
        let waits: Vec<f32> = full.iter().flat_map(|d| d.wait_s.iter().copied()).collect();
        let jd = self.jd;
        let month_users = p.users.iter().filter(|u| u.last_jd > 0.0 && jd - u.last_jd < 30.0).count();
        let returning = p.users.iter().filter(|u| u.last_jd > 0.0 && jd - u.last_jd < 30.0 && u.visits >= 2).count();
        let per_seg: Vec<serde_json::Value> = Segment::ALL
            .iter()
            .map(|&s| {
                let i = s as usize;
                serde_json::json!({
                    "segment": s.label(),
                    "submitted_per_day": full.iter().map(|d| d.submitted[i]).sum::<u32>() as f64 / ndays,
                    "completed_per_day": full.iter().map(|d| d.completed[i]).sum::<u32>() as f64 / ndays,
                    "assumed": p.cfg.seg[i],
                })
            })
            .collect();
        // Capacity: telescope-seconds demanded vs the public pool's supply.
        let duty = self.science.duty().unwrap_or(0.72);
        let supply = self.public_share * self.shell.len() as f64 * 86_400.0 * duty;
        let demand = full.iter().map(|d| d.tel_s_demand).sum::<f64>() / ndays;
        let util = if supply > 0.0 { demand / supply } else { 0.0 };
        let stages: Vec<serde_json::Value> = [100usize, 500, 1000, 5000, 10_000]
            .iter()
            .map(|&n| {
                let sup = self.public_share * n as f64 * 86_400.0 * duty;
                let per_user = if p.cfg.population > 0 { demand / p.cfg.population as f64 } else { 0.0 };
                serde_json::json!({ "n": n, "users_served": if per_user > 0.0 { sup / per_user } else { 0.0 } })
            })
            .collect();
        serde_json::json!({
            "on": p.cfg.on,
            "population": p.cfg.population,
            "days_measured": full.len(),
            "submitted_per_day": submitted as f64 / ndays,
            "completed_per_day": completed as f64 / ndays,
            "images_per_year": completed as f64 / ndays * 365.25,
            "turned_away_quota_per_day": quota as f64 / ndays,
            "turned_away_full_per_day": fullq as f64 / ndays,
            "expired_per_day": expired as f64 / ndays,
            "median_wait_s": median(&waits),
            "users_30d": month_users,
            "returning_share": if month_users > 0 { returning as f64 / month_users as f64 } else { 0.0 },
            "new_users_last_day": last.map(|d| d.new_users),
            "active_users_last_day": last.map(|d| d.active_users),
            "classroom_sessions_per_day": full.iter().map(|d| d.classroom_sessions).sum::<u32>() as f64 / ndays,
            "citizen_measurements_per_day": full.iter().map(|d| d.completed[3]).sum::<u32>() as f64 / ndays,
            "open_now": p.open.len(),
            "segments": per_seg,
            "public_share": self.public_share,
            "pool_utilisation": util,
            "stages": stages,
            "live": {
                "submitted": p.live.submitted,
                "completed": p.live.completed,
                "open": p.live.open,
                "median_wait_s": median(&p.live.wait_s),
            },
        })
    }

    pub fn public_panel(&mut self, ui: &mut egui::Ui) {
        use egui::RichText;
        ui.label(RichText::new("Public engagement").strong());
        let c = &mut self.public.cfg;
        ui.checkbox(&mut c.on, "model public demand (assumed adoption)");
        ui.add(egui::Slider::new(&mut c.population, 0..=1_000_000).logarithmic(true).text("monthly active users (assumed)"));
        ui.add(egui::Slider::new(&mut self.public_share, 0.0..=1.0).text("public share of the fleet"));
        let j = self.public_json();
        let f = |k: &str| j[k].as_f64().unwrap_or(0.0);
        if j["days_measured"].as_u64().unwrap_or(0) == 0 {
            ui.label(RichText::new("Measuring the first simulated day…").small().weak());
        }
        crate::provenance::line(ui, "public_service", format!(
            "{:.0} requests/day · {:.0} images delivered/day (≈ {:.1} M a year) · median wait {}",
            f("submitted_per_day"),
            f("completed_per_day"),
            f("images_per_year") / 1e6,
            j["median_wait_s"].as_f64().map(crate::allsky::fmt_span).unwrap_or("–".into())
        ));
        crate::provenance::line(ui, "public_service", format!(
            "{} users in the last 30 days, {:.0} % returning · {:.0} classroom sessions/day · {:.0} citizen-science measurements/day",
            j["users_30d"],
            100.0 * f("returning_share"),
            f("classroom_sessions_per_day"),
            f("citizen_measurements_per_day")
        ));
        crate::provenance::line(ui, "public_service", format!(
            "Turned away per day: {:.0} over fair-use quota, {:.0} queue full, {:.0} gave up waiting · public pool {:.0} % utilised",
            f("turned_away_quota_per_day"),
            f("turned_away_full_per_day"),
            f("expired_per_day"),
            100.0 * f("pool_utilisation")
        ));
        crate::provenance::line(ui, "public_live", format!("Live (real visitors to the public site): {} requests, {} open", j["live"]["submitted"], j["live"]["open"]));
    }
}

fn messier() -> &'static [usize] {
    use std::sync::OnceLock;
    static M: OnceLock<Vec<usize>> = OnceLock::new();
    M.get_or_init(|| {
        let d = crate::catalog::dso();
        (0..d.len()).filter(|&k| d[k].messier.is_some()).collect()
    })
}

fn pick_target(p: &mut Public, s: Segment, bodies: &[V3], neo: &[(V3, String, usize)], blocked: &dyn Fn(V3) -> bool) -> Option<(String, Option<usize>, V3)> {
    let dso = crate::catalog::dso();
    let names = ["Moon", "Mercury", "Venus", "Mars", "Jupiter", "Saturn", "Uranus", "Neptune"];
    for _ in 0..12 {
        let r = p.unif();
        let pick: Option<(String, Option<usize>, V3)> = match s {
            Segment::Casual | Segment::Classroom if r < 0.55 => {
                let k = 1 + (p.next() % 8) as usize;
                Some((names[k - 1].to_string(), Some(k), bodies[k]))
            }
            Segment::Amateur if r < 0.2 => {
                let k = 1 + (p.next() % 8) as usize;
                Some((names[k - 1].to_string(), Some(k), bodies[k]))
            }
            Segment::Citizen if r < 0.5 && !neo.is_empty() => {
                let k = (p.next() % neo.len() as u64) as usize;
                Some((neo[k].1.clone(), None, neo[k].0))
            }
            Segment::Citizen => {
                let v = crate::catalog::variables();
                let k = (p.next() % v.len() as u64) as usize;
                Some((v[k].name.clone(), None, v[k].dir))
            }
            Segment::Amateur if r < 0.55 => {
                let l = &crate::programme::real_lists().clusters_galaxies;
                let k = l[(p.next() % l.len() as u64) as usize];
                Some((dso[k].name(), None, dso[k].dir))
            }
            _ => {
                let m = messier();
                let k = m[(p.next() % m.len() as u64) as usize];
                Some((dso[k].name(), None, dso[k].dir))
            }
        };
        if let Some(t) = pick {
            if !blocked(t.2) {
                return Some(t);
            }
        }
    }
    None
}
