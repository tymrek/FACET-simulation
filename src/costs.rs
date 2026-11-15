//! Programme cost model: what FACET costs to build, launch and run, year by
//! year, and what each dollar buys in science (from the running simulation's
//! science ledger).
//!
//! Every price is an editable line item with an annual trend (% per year), so
//! some costs can rise and others fall over the programme. Mass-produced
//! hardware also follows a learning curve (Wright's law: each doubling of
//! cumulative units multiplies unit cost by the learning rate).
//!
//! Launch defaults are 2026 public list prices. Hardware, staffing, ground
//! and data costs are ASSUMPTIONS for a 150 kg smallsat programme, there to be
//! replaced with real quotes.

use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub struct Item {
    pub key: String,
    pub group: String,
    pub label: String,
    pub value: f64,
    pub unit: String,
    /// Annual price change, % per year (negative = getting cheaper).
    pub trend_pct: f64,
    pub note: String,
}

fn item(key: &str, group: &str, label: &str, value: f64, unit: &str, trend: f64, note: &str) -> Item {
    Item { key: key.into(), group: group.into(), label: label.into(), value, unit: unit.into(), trend_pct: trend, note: note.into() }
}

#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Debug)]
pub enum LaunchMode {
    /// Dedicated launches, each placing satellites into its own plane(s).
    Dedicated,
    /// Rideshare by mass (only for orbits rideshares fly to; see notes).
    Rideshare,
}

#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub struct Model {
    pub name: String,
    pub items: Vec<Item>,
    /// Follow the simulator's fleet size, or use `fleet`.
    pub follow_sim: bool,
    pub fleet: usize,
    pub horizon_years: usize,
    pub first_year: i32,
    pub launch_mode: LaunchMode,
    /// Planes placed per dedicated launch (more than 1 needs differential-drift
    /// plane separation, which takes months).
    pub planes_per_launch: f64,
}

impl Model {
    pub fn baseline() -> Model {
        let items = vec![
            // Satellite hardware (first-unit costs; learning curve applies)
            item("t1_bus", "Satellite (first unit)", "Bus: structure, power, ADCS, propulsion", 1.2e6, "$", -1.0, "Assumed for a ~150 kg smallsat bus"),
            item("t1_telescope", "Satellite (first unit)", "Telescope: 20 cm optics + structure", 0.6e6, "$", -1.0, "Assumed"),
            item("t1_camera", "Satellite (first unit)", "Focal plane: CMOS camera, electronics", 0.15e6, "$", -3.0, "Assumed; consumer-derived sensors keep getting cheaper"),
            item("t1_lasers", "Satellite (first unit)", "Optical terminals (2 mesh + 1 relay)", 0.45e6, "$", -5.0, "Assumed ~$150k each; terminal prices falling as volumes grow"),
            item("t1_ait", "Satellite (first unit)", "Assembly, integration & test", 0.25e6, "$", 3.0, "Assumed; mostly labour"),
            item("learning", "Satellite (first unit)", "Learning rate per doubling", 85.0, "%", 0.0, "Wright's law; 85 % is typical for aerospace series production"),
            item("mass_kg", "Satellite (first unit)", "Satellite mass", 150.0, "kg", 0.0, "Programme assumption"),
            item("spares_pct", "Satellite (first unit)", "Ground spares and launch losses", 3.0, "%", 0.0, "Assumed"),
            // Launch
            item("launch_dedicated", "Launch", "Dedicated launch price (Falcon 9 list)", 74.0e6, "$", 2.5, "SpaceX 2026 list price $74M (up from $67M in 2022)"),
            item("launch_capacity", "Launch", "Satellites per dedicated launch", 60.0, "sats", 0.0, "Assumed dispenser/volume limit for 150 kg satellites"),
            item("rideshare_kg", "Launch", "Rideshare price", 7_000.0, "$/kg", 2.5, "SpaceX Transporter 2026: $350k for 50 kg, $7,000/kg; SSO only"),
            item("launches_per_year", "Launch", "Launch cadence available", 8.0, "per year", 0.0, "Assumed booking capacity"),
            item("insurance_pct", "Launch", "Launch + first-year insurance", 6.0, "% of hw+launch", 0.0, "Assumed (typical range 5–10 %)"),
            // Development and programme
            item("nre", "Development & programme", "Development (design, prototypes, qualification)", 60.0e6, "$", 3.0, "Assumed non-recurring engineering"),
            item("dev_years", "Development & programme", "Development time before first launch", 2.0, "years", 0.0, "Assumed"),
            item("overhead_pct", "Development & programme", "Programme management & systems engineering", 15.0, "% of hardware", 0.0, "Assumed"),
            item("lifetime", "Development & programme", "Satellite design life", 5.0, "years", 0.0, "Assumed (Starlink cites ~5-year design life)"),
            // Operations
            item("fte_base", "Operations", "Operations staff (base)", 25.0, "FTE", 0.0, "Assumed: flight ops, science ops, software"),
            item("fte_per_100", "Operations", "Additional staff per 100 satellites", 1.0, "FTE", 0.0, "Assumed; automation keeps this low"),
            item("fte_cost", "Operations", "Loaded cost per FTE", 180_000.0, "$/yr", 3.5, "Assumed"),
            item("stations", "Operations", "Ground stations leased", 20.0, "stations", 0.0, "Matches the traffic model"),
            item("station_lease", "Operations", "Ground station lease", 250_000.0, "$/station/yr", 2.0, "Assumed"),
            // Data
            item("relay_tb", "Data", "Optical relay service", 5.0, "$/TB", -5.0, "ASSUMED: no public price for third-party relay"),
            item("archive_tb_month", "Data", "Archive storage", 4.0, "$/TB/month", -8.0, "Assumed blended cloud archive tier; storage prices fall"),
            item("processing_tb", "Data", "Pipeline processing", 2.0, "$/TB", -6.0, "Assumed cloud compute"),
        ];
        Model {
            name: "Baseline".into(),
            items,
            follow_sim: true,
            fleet: 1000,
            horizon_years: 10,
            first_year: 2027,
            launch_mode: LaunchMode::Dedicated,
            planes_per_launch: 1.0,
        }
    }

    pub fn optimistic() -> Model {
        let mut m = Model::baseline();
        m.name = "Optimistic".into();
        m.set("learning", 80.0);
        m.set("launch_dedicated", 50.0e6);
        m.set_trend("launch_dedicated", -5.0);
        m.set("launch_capacity", 120.0);
        m.set("nre", 45.0e6);
        m.set("lifetime", 7.0);
        m.set("launches_per_year", 12.0);
        m.planes_per_launch = 2.0;
        m
    }

    pub fn pessimistic() -> Model {
        let mut m = Model::baseline();
        m.name = "Pessimistic".into();
        m.set("learning", 90.0);
        m.set("t1_bus", 1.8e6);
        m.set("t1_telescope", 0.9e6);
        m.set_trend("launch_dedicated", 5.0);
        m.set("launch_capacity", 40.0);
        m.set("nre", 100.0e6);
        m.set("lifetime", 4.0);
        m.set("launches_per_year", 5.0);
        m.set("fte_base", 40.0);
        m.set("relay_tb", 20.0);
        m
    }

    pub fn get(&self, k: &str) -> f64 {
        self.items.iter().find(|i| i.key == k).map(|i| i.value).unwrap_or(0.0)
    }
    fn trend(&self, k: &str) -> f64 {
        self.items.iter().find(|i| i.key == k).map(|i| i.trend_pct).unwrap_or(0.0)
    }
    fn set(&mut self, k: &str, v: f64) {
        if let Some(i) = self.items.iter_mut().find(|i| i.key == k) {
            i.value = v;
        }
    }
    fn set_trend(&mut self, k: &str, v: f64) {
        if let Some(i) = self.items.iter_mut().find(|i| i.key == k) {
            i.trend_pct = v;
        }
    }
    /// Price of item `k` in programme year `y` (1 = first year).
    fn price(&self, k: &str, y: usize) -> f64 {
        self.get(k) * (1.0 + self.trend(k) / 100.0).powi(y as i32 - 1)
    }
}

/// Science delivered per satellite-day, from the simulation's ledger.
#[derive(Clone, Copy, Default, Serialize)]
pub struct SciencePerSat {
    pub tel_h: f64,
    pub completed: f64,
    pub etendue_m2deg2h: f64,
    pub tb_downlinked: f64,
    pub public_frac: f64,
    pub planes: usize,
}

#[derive(Clone, Default, Serialize)]
pub struct Year {
    pub year: i32,
    pub fleet_end: f64,
    pub built: f64,
    pub launches: f64,
    pub development: f64,
    pub satellites: f64,
    pub launch: f64,
    pub insurance: f64,
    pub programme: f64,
    pub operations: f64,
    pub data: f64,
    pub total: f64,
    pub cumulative: f64,
    pub tel_h: f64,
    pub completed: f64,
}

#[derive(Clone, Default, Serialize)]
pub struct Result {
    pub fleet: usize,
    pub years: Vec<Year>,
    pub total: f64,
    pub first_science_year: Option<i32>,
    pub full_fleet_year: Option<i32>,
    pub steady_annual: f64,
    pub avg_unit_cost: f64,
    pub cost_per_tel_hour: Option<f64>,
    pub cost_per_observation: Option<f64>,
    pub cost_per_etendue_hour: Option<f64>,
    pub cost_per_tb: Option<f64>,
    pub cost_per_public_hour: Option<f64>,
    /// (budget, date reached as fractional year, satellites in orbit then); None if beyond the horizon.
    pub tranches: Vec<(f64, Option<f64>, f64)>,
    pub notes: Vec<String>,
}

/// Cost of building units U+1 … U+k on a learning curve with first-unit cost t1.
fn learning_cost(t1: f64, lr_pct: f64, u: f64, k: f64) -> f64 {
    if k <= 0.0 {
        return 0.0;
    }
    let b = (lr_pct / 100.0).clamp(0.5, 1.0).log2();
    let p = 1.0 + b;
    // Continuous approximation of Σ n^b with the midpoint correction.
    t1 * (((u + k + 0.5).powf(p) - (u + 0.5).powf(p)) / p)
}

pub fn evaluate(m: &Model, fleet: usize, sci: &SciencePerSat) -> Result {
    let n = fleet.max(1) as f64;
    let mut r = Result { fleet, ..Default::default() };
    let dev_years = m.get("dev_years").round().max(0.0) as usize;
    let life = m.get("lifetime").max(0.5);
    let per_launch = match m.launch_mode {
        LaunchMode::Dedicated => {
            // A launch serves whole planes; capacity also limits it.
            let per_plane = n / sci.planes.max(1) as f64;
            (per_plane * m.planes_per_launch.max(1.0)).min(m.get("launch_capacity").max(1.0)).max(1.0)
        }
        LaunchMode::Rideshare => f64::INFINITY,
    };
    let cadence = m.get("launches_per_year").max(0.1);
    let mut fleet_now = 0.0f64;
    let mut built_total = 0.0f64;
    let mut archive_tb = 0.0f64;
    let mut cum = 0.0;
    let mut hw_total = 0.0;
    let mut units_total = 0.0;
    for y in 1..=m.horizon_years.max(1) {
        let mut yr = Year { year: m.first_year + y as i32 - 1, ..Default::default() };
        // Development
        if y <= dev_years.max(1) && dev_years > 0 {
            yr.development = m.price("nre", y) / dev_years as f64;
        } else if dev_years == 0 && y == 1 {
            yr.development = m.price("nre", y);
        }
        // Deployment and replacement
        let deploying = y > dev_years;
        let replace = if deploying { fleet_now / life } else { 0.0 };
        let mut to_launch = 0.0;
        if deploying {
            let missing = (n - fleet_now).max(0.0);
            let cap = match m.launch_mode {
                LaunchMode::Dedicated => cadence * per_launch,
                LaunchMode::Rideshare => n, // rideshare slots: assume booking is not the limit
            };
            to_launch = (missing + replace).min(cap.max(replace));
        }
        let launches = match m.launch_mode {
            LaunchMode::Dedicated => (to_launch / per_launch).ceil(),
            LaunchMode::Rideshare => 0.0,
        };
        let spares = 1.0 + m.get("spares_pct") / 100.0;
        let built = to_launch * spares;
        let t1: f64 = ["t1_bus", "t1_telescope", "t1_camera", "t1_lasers", "t1_ait"].iter().map(|k| m.price(k, y)).sum();
        let hw = learning_cost(t1, m.get("learning"), built_total, built);
        built_total += built;
        hw_total += hw;
        units_total += built;
        yr.built = built;
        yr.launches = launches;
        yr.satellites = hw;
        yr.launch = match m.launch_mode {
            LaunchMode::Dedicated => launches * m.price("launch_dedicated", y),
            LaunchMode::Rideshare => to_launch * m.get("mass_kg") * m.price("rideshare_kg", y),
        };
        yr.insurance = (hw + yr.launch) * m.get("insurance_pct") / 100.0;
        yr.programme = hw * m.get("overhead_pct") / 100.0;
        let fleet_start = fleet_now;
        fleet_now = (fleet_now - replace + to_launch).min(n).max(0.0);
        yr.fleet_end = fleet_now;
        let fleet_avg = 0.5 * (fleet_start + fleet_now);
        // Operations
        let fte = if deploying || y == dev_years { m.get("fte_base") + m.get("fte_per_100") * fleet_avg / 100.0 } else { m.get("fte_base") * 0.5 };
        yr.operations = fte * m.price("fte_cost", y) + m.get("stations") * m.price("station_lease", y);
        // Data
        let tb = sci.tb_downlinked * fleet_avg * 365.0;
        archive_tb += tb;
        yr.data = tb * (m.price("relay_tb", y) + m.price("processing_tb", y)) + (archive_tb - 0.5 * tb) * 12.0 * m.price("archive_tb_month", y);
        // Science
        yr.tel_h = sci.tel_h * fleet_avg * 365.0;
        yr.completed = sci.completed * fleet_avg * 365.0;
        yr.total = yr.development + yr.satellites + yr.launch + yr.insurance + yr.programme + yr.operations + yr.data;
        cum += yr.total;
        yr.cumulative = cum;
        if r.first_science_year.is_none() && fleet_now > 0.0 {
            r.first_science_year = Some(yr.year);
        }
        if r.full_fleet_year.is_none() && fleet_now >= n - 0.5 {
            r.full_fleet_year = Some(yr.year);
        }
        r.years.push(yr);
    }
    r.total = cum;
    r.avg_unit_cost = if units_total > 0.0 { hw_total / units_total } else { 0.0 };
    r.steady_annual = r.years.last().map(|y| y.total).unwrap_or(0.0);
    let tel_h: f64 = r.years.iter().map(|y| y.tel_h).sum();
    let done: f64 = r.years.iter().map(|y| y.completed).sum();
    let sat_days: f64 = r.years.iter().enumerate().map(|(k, y)| {
        let prev = if k == 0 { 0.0 } else { r.years[k - 1].fleet_end };
        0.5 * (prev + y.fleet_end) * 365.0
    }).sum();
    let div = |a: f64, b: f64| (b > 0.0).then(|| a / b);
    r.cost_per_tel_hour = div(cum, tel_h);
    r.cost_per_observation = div(cum, done);
    r.cost_per_etendue_hour = div(cum, sci.etendue_m2deg2h * sat_days);
    r.cost_per_tb = div(cum, sci.tb_downlinked * sat_days);
    r.cost_per_public_hour = div(cum, tel_h * sci.public_frac);
    // Spending is spread evenly through each year: interpolate the date and
    // fleet size at which each budget runs out.
    for t in [100e6, 250e6, 500e6, 1e9, 2e9, 5e9] {
        let mut hit = None;
        for (k, y) in r.years.iter().enumerate() {
            let (c0, f0) = if k == 0 { (0.0, 0.0) } else { (r.years[k - 1].cumulative, r.years[k - 1].fleet_end) };
            if y.cumulative >= t {
                let f = ((t - c0) / y.total.max(1.0)).clamp(0.0, 1.0);
                hit = Some((y.year as f64 + f, f0 + f * (y.fleet_end - f0)));
                break;
            }
        }
        match hit {
            Some((date, fleet)) => r.tranches.push((t, Some(date), fleet)),
            None => r.tranches.push((t, None, r.years.last().map(|y| y.fleet_end).unwrap_or(0.0))),
        }
    }
    if m.launch_mode == LaunchMode::Rideshare {
        r.notes.push("Rideshare flies to the mission's orbit (mostly sun-synchronous). FACET's 53° planes need dedicated launches or a drift strategy.".into());
    }
    if m.launch_mode == LaunchMode::Dedicated && m.planes_per_launch > 1.0 {
        r.notes.push("More than one plane per launch relies on differential nodal drift after deployment, which takes months per plane.".into());
    }
    if n / sci.planes.max(1) as f64 > m.get("launch_capacity") {
        r.notes.push("A plane holds more satellites than one launch carries: several launches per plane.".into());
    }
    r
}

pub fn fmt_money(x: f64) -> String {
    let a = x.abs();
    if a >= 1e9 {
        format!("${:.2}B", x / 1e9)
    } else if a >= 1e6 {
        format!("${:.1}M", x / 1e6)
    } else if a >= 1e3 {
        format!("${:.0}k", x / 1e3)
    } else {
        format!("${x:.2}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn learning_curve_matches_discrete_sum() {
        let exact: f64 = (1..=1000).map(|n: i32| (n as f64).powf(0.85f64.log2())).sum();
        let approx = learning_cost(1.0, 85.0, 0.0, 1000.0);
        assert!((approx / exact - 1.0).abs() < 0.02, "{approx} vs {exact}");
        // 100 % learning = no learning
        assert!((learning_cost(2.0, 100.0, 10.0, 5.0) - 10.0).abs() < 1e-9);
    }

    #[test]
    fn baseline_deploys_and_trends_move_costs() {
        let sci = SciencePerSat { tel_h: 21.0, completed: 8.0, etendue_m2deg2h: 0.6, tb_downlinked: 0.06, public_frac: 0.03, planes: 32 };
        let m = Model::baseline();
        let r = evaluate(&m, 1000, &sci);
        assert_eq!(r.years.len(), 10);
        assert_eq!(r.first_science_year, Some(m.first_year + 2));
        assert!(r.full_fleet_year.is_some());
        assert!(r.total > 1e8 && r.total < 1e10, "total {}", r.total);
        assert!(r.avg_unit_cost < m.get("t1_bus") + m.get("t1_telescope") + m.get("t1_camera") + m.get("t1_lasers") + m.get("t1_ait"));
        // Launch getting cheaper lowers the total
        let mut cheaper = m.clone();
        cheaper.set_trend("launch_dedicated", -10.0);
        assert!(evaluate(&cheaper, 1000, &sci).total < r.total);
        // A pessimistic world buys science at a higher price (it may spend less
        // in total, because it deploys more slowly).
        let p = evaluate(&Model::pessimistic(), 1000, &sci);
        let o = evaluate(&Model::optimistic(), 1000, &sci);
        for (name, x) in [("optimistic", &o), ("baseline", &r), ("pessimistic", &p)] {
            eprintln!(
                "{name:12} total {} · unit {} · per tel-h {} · first {:?} full {:?} · final-year {}",
                fmt_money(x.total), fmt_money(x.avg_unit_cost), fmt_money(x.cost_per_tel_hour.unwrap()), x.first_science_year, x.full_fleet_year, fmt_money(x.steady_annual)
            );
        }
        assert!(p.cost_per_tel_hour.unwrap() > r.cost_per_tel_hour.unwrap());
        assert!(o.cost_per_tel_hour.unwrap() < r.cost_per_tel_hour.unwrap());
    }
}
