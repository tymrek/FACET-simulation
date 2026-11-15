//! Embedded web access: a token-protected director's control port and an open
//! public-access port. Both are plain HTTP/1.1 (tiny_http), one thread each.
//!
//! The simulator owns the truth. The server threads only:
//!   * read the latest JSON status snapshot the sim publishes (~4 Hz),
//!   * queue `Command`s for the sim to apply on its next frame,
//!   * manage the observation-request book (shared, under one mutex).
//!
//! Put a TLS-terminating reverse proxy (nginx, caddy) in front of either port
//! before exposing it beyond a trusted network: tokens travel in clear text.

use crate::math::V3;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Read;
use std::sync::{Arc, Mutex};
use tiny_http::{Header, Method, Request, Response, Server};

const CONTROL_HTML: &str = include_str!("../web/control.html");
const PUBLIC_HTML: &str = include_str!("../web/public.html");
const SKYMAP_JS: &str = include_str!("../web/skymap.js");
const CONTROL_JS: &str = include_str!("../web/control.js");
const PUBLIC_JS: &str = include_str!("../web/public.js");
const STYLE_CSS: &str = include_str!("../web/style.css");

const MAX_BODY: u64 = 8 * 1024;
const MAX_OPEN_PER_IP: usize = 5;
const MAX_OPEN_TOTAL: usize = 500;
const MAX_KEPT: usize = 2000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ReqState {
    Pending,
    Approved,
    Observing,
    Done,
    Rejected,
    Cancelled,
}

impl ReqState {
    pub fn is_open(self) -> bool {
        matches!(self, ReqState::Pending | ReqState::Approved | ReqState::Observing)
    }
    pub fn is_schedulable(self) -> bool {
        matches!(self, ReqState::Approved | ReqState::Observing)
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ObsRequest {
    pub id: u64,
    pub submitter: String,
    pub target_name: String,
    /// Index into astro::bodies for moving targets; otherwise fixed RA/Dec.
    pub body: Option<usize>,
    pub ra_deg: f64,
    pub dec_deg: f64,
    /// Telescopes requested simultaneously (they stack).
    pub telescopes: u32,
    /// Integration per telescope, seconds.
    pub exposure_s: f64,
    pub priority: i32,
    pub state: ReqState,
    /// Accumulated telescope-seconds.
    pub progress_s: f64,
    pub assigned_now: u32,
    pub blocked: Option<String>,
    pub submitted_utc: String,
    pub completed_utc: Option<String>,
    pub note: String,
    #[serde(skip)]
    pub ip: String,
    #[serde(skip)]
    pub dir: V3,
}

impl ObsRequest {
    pub fn required_s(&self) -> f64 {
        self.telescopes as f64 * self.exposure_s
    }
}

/// Commands from the director's console, applied by the simulator thread.
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Command {
    Shell {
        n: Option<usize>,
        planes: Option<usize>,
        phasing: Option<usize>,
        alt_km: Option<f64>,
        inc_deg: Option<f64>,
        sso: Option<bool>,
        raan_span_deg: Option<f64>,
    },
    Mode {
        mode: String,
    },
    Target {
        name: Option<String>,
        ra_deg: Option<f64>,
        dec_deg: Option<f64>,
    },
    Time {
        rate: Option<f64>,
        playing: Option<bool>,
        now: Option<bool>,
    },
    Telescope {
        aperture_cm: Option<f64>,
        fov_deg: Option<f64>,
        exp_s: Option<f64>,
        base_lim_mag: Option<f64>,
        overhead_s: Option<f64>,
        sun_excl_deg: Option<f64>,
        moon_excl_deg: Option<f64>,
        limb_margin_deg: Option<f64>,
        neo_elong_deg: Option<f64>,
        neo_band_deg: Option<f64>,
    },
    // Applied directly by the server thread (request book):
    Request {
        id: u64,
        action: String,
        priority: Option<i32>,
        telescopes: Option<u32>,
    },
    AutoApprove {
        on: bool,
    },
}

/// A resolvable target: named star, solar-system body, or catalogue object.
#[derive(Clone)]
pub enum Resolved {
    Body(usize, String),
    Fixed(String, V3),
}

pub struct Shared {
    pub status_json: String,
    pub stars_json: String,
    pub commands: Vec<Command>,
    pub requests: Vec<ObsRequest>,
    pub next_id: u64,
    pub auto_approve: bool,
    pub token: String,
    pub utc_now: String,
    /// Lower-case name → target.
    pub names: HashMap<String, Resolved>,
    pub control_url: Option<String>,
    pub public_url: Option<String>,
    pub errors: Vec<String>,
}

pub type SharedRef = Arc<Mutex<Shared>>;

impl Shared {
    pub fn resolve(&self, name: &str) -> Option<Resolved> {
        self.names.get(&name.trim().to_lowercase()).cloned()
    }
}

pub fn random_token() -> String {
    let mut buf = [0u8; 16];
    let ok = std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut buf))
        .is_ok();
    if !ok {
        // Fallback: not cryptographic, but never empty.
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
            ^ (std::process::id() as u128) << 64;
        buf.copy_from_slice(&t.to_le_bytes());
    }
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

fn header(k: &str, v: &str) -> Header {
    Header::from_bytes(k.as_bytes(), v.as_bytes()).expect("static header")
}

fn send(req: Request, code: u16, ctype: &str, body: String) {
    let resp = Response::from_string(body)
        .with_status_code(code)
        .with_header(header("Content-Type", ctype))
        .with_header(header("Cache-Control", "no-store"))
        .with_header(header("X-Content-Type-Options", "nosniff"))
        .with_header(header("Referrer-Policy", "no-referrer"))
        .with_header(header(
            "Content-Security-Policy",
            "default-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:",
        ));
    let _ = req.respond(resp);
}

fn json(req: Request, code: u16, v: serde_json::Value) {
    send(req, code, "application/json", v.to_string());
}

fn err(req: Request, code: u16, msg: &str) {
    json(req, code, serde_json::json!({ "error": msg }));
}

fn read_body(req: &mut Request) -> Option<String> {
    if req.body_length().map_or(false, |l| l as u64 > MAX_BODY) {
        return None;
    }
    let mut s = String::new();
    req.as_reader().take(MAX_BODY + 1).read_to_string(&mut s).ok()?;
    if s.len() as u64 > MAX_BODY {
        return None;
    }
    Some(s)
}

fn query_param(url: &str, key: &str) -> Option<String> {
    let q = url.split_once('?')?.1;
    q.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        (k == key).then(|| v.to_string())
    })
}

fn path_of(url: &str) -> &str {
    url.split('?').next().unwrap_or("/")
}

fn ct_eq(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn authorised(req: &Request, token: &str) -> bool {
    let from_header = req
        .headers()
        .iter()
        .find(|h| h.field.equiv("X-Facet-Token"))
        .map(|h| h.value.as_str().to_string());
    let given = from_header.or_else(|| query_param(req.url(), "token"));
    given.map_or(false, |g| ct_eq(&g, token))
}

fn clean(s: &str, max: usize) -> String {
    s.chars()
        .filter(|c| !c.is_control())
        .take(max)
        .collect::<String>()
        .trim()
        .to_string()
}

fn static_route(path: &str, director: bool) -> Option<(&'static str, &'static str)> {
    match path {
        "/skymap.js" => Some(("application/javascript; charset=utf-8", SKYMAP_JS)),
        "/style.css" => Some(("text/css; charset=utf-8", STYLE_CSS)),
        "/control.js" if director => Some(("application/javascript; charset=utf-8", CONTROL_JS)),
        "/public.js" if !director => Some(("application/javascript; charset=utf-8", PUBLIC_JS)),
        _ => None,
    }
}

fn requests_json(sh: &Shared, director: bool) -> serde_json::Value {
    let mut list: Vec<&ObsRequest> = sh.requests.iter().collect();
    list.sort_by_key(|r| (!r.state.is_open(), std::cmp::Reverse(r.priority), r.id));
    let v: Vec<serde_json::Value> = list
        .iter()
        .take(400)
        .map(|r| {
            let mut j = serde_json::to_value(r).unwrap_or_default();
            j["required_s"] = serde_json::json!(r.required_s());
            if director {
                j["ip"] = serde_json::json!(r.ip);
            }
            j
        })
        .collect();
    serde_json::Value::Array(v)
}

fn status_with(sh: &Shared, director: bool) -> String {
    // status_json is an object; splice requests (and director extras) in.
    let mut v: serde_json::Value =
        serde_json::from_str(&sh.status_json).unwrap_or_else(|_| serde_json::json!({}));
    v["requests"] = requests_json(sh, director);
    v["auto_approve"] = serde_json::json!(sh.auto_approve);
    v.to_string()
}

#[derive(Deserialize)]
struct Submit {
    submitter: Option<String>,
    target: Option<String>,
    ra_deg: Option<f64>,
    dec_deg: Option<f64>,
    telescopes: Option<u32>,
    exposure_s: Option<f64>,
    note: Option<String>,
}

fn handle_submit(shared: &SharedRef, mut req: Request) {
    let ip = req.remote_addr().map(|a| a.ip().to_string()).unwrap_or_default();
    let Some(body) = read_body(&mut req) else {
        return err(req, 413, "request too large");
    };
    let sub: Submit = match serde_json::from_str(&body) {
        Ok(s) => s,
        Err(_) => return err(req, 400, "malformed JSON"),
    };
    let mut sh = shared.lock().unwrap();
    let open_total = sh.requests.iter().filter(|r| r.state.is_open()).count();
    let open_ip = sh.requests.iter().filter(|r| r.state.is_open() && r.ip == ip).count();
    if open_total >= MAX_OPEN_TOTAL {
        drop(sh);
        return err(req, 503, "queue full, try later");
    }
    if open_ip >= MAX_OPEN_PER_IP {
        drop(sh);
        return err(req, 429, "you already have 5 open requests");
    }
    let (body_idx, name, dir) = match (&sub.target, sub.ra_deg, sub.dec_deg) {
        (Some(t), _, _) if !t.trim().is_empty() => match sh.resolve(t) {
            Some(Resolved::Body(i, n)) => (Some(i), n, V3::Z),
            Some(Resolved::Fixed(n, d)) => (None, n, d),
            None => {
                drop(sh);
                return err(req, 404, "unknown target name; use a planet, a bright-star name, or RA/Dec");
            }
        },
        (_, Some(ra), Some(dec)) if ra.is_finite() && (-90.0..=90.0).contains(&dec) => {
            let ra = ra.rem_euclid(360.0);
            (None, format!("RA {ra:.3}° Dec {dec:+.3}°"), V3::from_radec(ra.to_radians(), dec.to_radians()))
        }
        _ => {
            drop(sh);
            return err(req, 400, "give a target name or ra_deg/dec_deg");
        }
    };
    let telescopes = sub.telescopes.unwrap_or(10).clamp(1, 1000);
    let exposure_s = sub.exposure_s.unwrap_or(300.0);
    if !(1.0..=86_400.0).contains(&exposure_s) {
        drop(sh);
        return err(req, 400, "exposure_s must be 1–86400");
    }
    let (ra, dec) = dir.to_radec();
    let id = sh.next_id;
    sh.next_id += 1;
    let state = if sh.auto_approve { ReqState::Approved } else { ReqState::Pending };
    let utc = sh.utc_now.clone();
    let r = ObsRequest {
        id,
        submitter: {
            let s = clean(sub.submitter.as_deref().unwrap_or(""), 40);
            if s.is_empty() { "anonymous".into() } else { s }
        },
        target_name: name,
        body: body_idx,
        ra_deg: ra.to_degrees(),
        dec_deg: dec.to_degrees(),
        telescopes,
        exposure_s,
        priority: 0,
        state,
        progress_s: 0.0,
        assigned_now: 0,
        blocked: None,
        submitted_utc: utc,
        completed_utc: None,
        note: clean(sub.note.as_deref().unwrap_or(""), 200),
        ip,
        dir,
    };
    sh.requests.push(r);
    // Bound memory: drop the oldest closed requests.
    if sh.requests.len() > MAX_KEPT {
        let excess = sh.requests.len() - MAX_KEPT;
        let mut removed = 0;
        sh.requests.retain(|r| {
            if removed < excess && !r.state.is_open() {
                removed += 1;
                false
            } else {
                true
            }
        });
    }
    drop(sh);
    json(req, 201, serde_json::json!({ "id": id, "state": state }));
}

fn apply_request_cmd(sh: &mut Shared, id: u64, action: &str, priority: Option<i32>, telescopes: Option<u32>) -> Result<(), &'static str> {
    let r = sh.requests.iter_mut().find(|r| r.id == id).ok_or("no such request")?;
    if let Some(p) = priority {
        r.priority = p.clamp(-100, 100);
    }
    if let Some(t) = telescopes {
        r.telescopes = t.clamp(1, 10_000);
    }
    match action {
        "approve" if r.state == ReqState::Pending => r.state = ReqState::Approved,
        "reject" if r.state.is_open() => r.state = ReqState::Rejected,
        "cancel" if r.state.is_open() => r.state = ReqState::Cancelled,
        "requeue" if !r.state.is_open() => {
            r.state = ReqState::Approved;
            r.progress_s = 0.0;
            r.completed_utc = None;
        }
        "update" => {}
        "approve" | "reject" | "cancel" | "requeue" => return Err("not valid in this state"),
        _ => return Err("unknown action"),
    }
    if !r.state.is_schedulable() {
        r.assigned_now = 0;
    }
    Ok(())
}

fn control_loop(server: Server, shared: SharedRef) {
    for mut req in server.incoming_requests() {
        let path = path_of(req.url()).to_string();
        let method = req.method().clone();
        let token = shared.lock().unwrap().token.clone();
        match (method, path.as_str()) {
            (Method::Get, "/") => send(req, 200, "text/html; charset=utf-8", CONTROL_HTML.into()),
            (Method::Get, p) if static_route(p, true).is_some() => {
                let (ct, body) = static_route(p, true).unwrap();
                send(req, 200, ct, body.into())
            }
            _ if !authorised(&req, &token) => err(req, 401, "missing or wrong token"),
            (Method::Get, "/api/stars") => {
                let s = shared.lock().unwrap().stars_json.clone();
                send(req, 200, "application/json", s)
            }
            (Method::Get, "/api/status") => {
                let s = status_with(&shared.lock().unwrap(), true);
                send(req, 200, "application/json", s)
            }
            (Method::Post, "/api/cmd") => {
                let Some(body) = read_body(&mut req) else {
                    err(req, 413, "request too large");
                    continue;
                };
                let cmd: Command = match serde_json::from_str(&body) {
                    Ok(c) => c,
                    Err(e) => {
                        err(req, 400, &format!("bad command: {e}"));
                        continue;
                    }
                };
                let mut sh = shared.lock().unwrap();
                let res = match &cmd {
                    Command::Request { id, action, priority, telescopes } => {
                        apply_request_cmd(&mut sh, *id, action, *priority, *telescopes)
                    }
                    Command::AutoApprove { on } => {
                        sh.auto_approve = *on;
                        Ok(())
                    }
                    Command::Target { name: Some(n), .. } if sh.resolve(n).is_none() => Err("unknown target name"),
                    _ => {
                        sh.commands.push(cmd.clone());
                        Ok(())
                    }
                };
                drop(sh);
                match res {
                    Ok(()) => json(req, 200, serde_json::json!({ "ok": true })),
                    Err(e) => err(req, 400, e),
                }
            }
            _ => err(req, 404, "not found"),
        }
    }
}

fn public_loop(server: Server, shared: SharedRef) {
    for req in server.incoming_requests() {
        let path = path_of(req.url()).to_string();
        let method = req.method().clone();
        match (method, path.as_str()) {
            (Method::Get, "/") => send(req, 200, "text/html; charset=utf-8", PUBLIC_HTML.into()),
            (Method::Get, p) if static_route(p, false).is_some() => {
                let (ct, body) = static_route(p, false).unwrap();
                send(req, 200, ct, body.into())
            }
            (Method::Get, "/api/stars") => {
                let s = shared.lock().unwrap().stars_json.clone();
                send(req, 200, "application/json", s)
            }
            (Method::Get, "/api/status") => {
                let s = status_with(&shared.lock().unwrap(), false);
                send(req, 200, "application/json", s)
            }
            (Method::Post, "/api/request") => handle_submit(&shared, req),
            _ => err(req, 404, "not found"),
        }
    }
}

/// Start both listeners. Failures (port in use, …) are recorded in `errors`
/// and shown in the app rather than aborting the simulator.
pub fn start(shared: &SharedRef, control_addr: Option<&str>, public_addr: Option<&str>) {
    let token = shared.lock().unwrap().token.clone();
    if let Some(addr) = control_addr {
        match Server::http(addr) {
            Ok(s) => {
                let url = format!("http://{}/?token={}", display_addr(addr), token);
                eprintln!("FACET director console: {url}");
                shared.lock().unwrap().control_url = Some(url);
                let sh = shared.clone();
                std::thread::Builder::new()
                    .name("facet-control".into())
                    .spawn(move || control_loop(s, sh))
                    .ok();
            }
            Err(e) => shared.lock().unwrap().errors.push(format!("control port {addr}: {e}")),
        }
    }
    if let Some(addr) = public_addr {
        match Server::http(addr) {
            Ok(s) => {
                let url = format!("http://{}/", display_addr(addr));
                eprintln!("FACET public access node: {url}");
                shared.lock().unwrap().public_url = Some(url);
                let sh = shared.clone();
                std::thread::Builder::new()
                    .name("facet-public".into())
                    .spawn(move || public_loop(s, sh))
                    .ok();
            }
            Err(e) => shared.lock().unwrap().errors.push(format!("public port {addr}: {e}")),
        }
    }
}

/// 0.0.0.0 isn't browsable; show localhost for a wildcard bind.
fn display_addr(addr: &str) -> String {
    addr.replace("0.0.0.0", "localhost").replace("[::]", "localhost")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_time_eq() {
        assert!(ct_eq("abc", "abc"));
        assert!(!ct_eq("abc", "abd"));
        assert!(!ct_eq("abc", "abcd"));
    }

    #[test]
    fn query() {
        assert_eq!(query_param("/api/status?token=xyz&a=1", "token").as_deref(), Some("xyz"));
        assert_eq!(query_param("/api/status", "token"), None);
    }

    #[test]
    fn commands_parse() {
        let c: Command = serde_json::from_str(r#"{"cmd":"shell","n":500,"sso":true}"#).unwrap();
        assert!(matches!(c, Command::Shell { n: Some(500), sso: Some(true), .. }));
        let c: Command = serde_json::from_str(r#"{"cmd":"request","id":3,"action":"approve"}"#).unwrap();
        assert!(matches!(c, Command::Request { id: 3, .. }));
    }

    #[test]
    fn token_is_hex_32() {
        let t = random_token();
        assert_eq!(t.len(), 32);
        assert!(t.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
