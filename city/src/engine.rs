//! The core of the live city: the road graph, travellers, and the checks that turn
//! `overrules` answers into allowed moves and speeds.

use std::cell::Cell;
use std::collections::HashMap;

use chrono::NaiveDateTime;
use overrules::eval::{Resolution, RuleEvaluation, Verdict};
use overrules::model::{Scoped, SpeedUnit};
use overrules::{
    Access, AccessType, Answer, Dimension, Heading, Network, Segment, Situation, TravelMode,
};
use serde::{Deserialize, Serialize};

// ─── world ──────────────────────────────────────────────────────────────────

/// A junction (Overture connector).
#[derive(Debug)]
pub struct Node {
    pub id: String,
    pub lon: f64,
    pub lat: f64,
}

/// The stretch of one segment between two consecutive connectors, travelled one way.
#[derive(Debug)]
pub struct Edge {
    pub seg: usize,
    pub from: usize,
    pub to: usize,
    pub from_at: f64,
    pub to_at: f64,
    pub heading: Heading,
    pub length_m: f64,
}

/// How much work `overrules` has done, for the "under the hood" counters.
#[derive(Debug, Default)]
pub struct Counters {
    pub resolves: Cell<u64>,
    pub rules: Cell<u64>,
}

#[derive(Debug)]
pub struct World {
    pub net: Network,
    pub nodes: Vec<Node>,
    pub node_of: HashMap<String, usize>,
    pub edges: Vec<Edge>,
    pub out: Vec<Vec<usize>>,
    pub counters: Counters,
}

impl World {
    pub fn from_geojson(text: &str) -> overrules::Result<Self> {
        let net = Network::new(overrules::io::geojson::read(text.as_bytes())?);
        let mut ids: HashMap<String, usize> = HashMap::new();
        let mut nodes: Vec<Node> = Vec::new();
        let mut edges = Vec::new();
        for (si, seg) in net.segments().iter().enumerate() {
            let mut cs: Vec<_> = seg.connectors.iter().collect();
            cs.sort_by(|a, b| a.at.total_cmp(&b.at));
            let mut node = |c: &overrules::model::ConnectorRef| {
                *ids.entry(c.connector_id.clone()).or_insert_with(|| {
                    let p = seg.line().point_at(c.at);
                    nodes.push(Node {
                        id: c.connector_id.clone(),
                        lon: p.lon,
                        lat: p.lat,
                    });
                    nodes.len() - 1
                })
            };
            for pair in cs.windows(2) {
                let (a, b) = (pair[0], pair[1]);
                if b.at - a.at < 1e-9 || a.connector_id == b.connector_id {
                    continue;
                }
                let (na, nb) = (node(a), node(b));
                let length_m = (b.at - a.at) * seg.length_m();
                for (from, to, from_at, to_at, heading) in [
                    (na, nb, a.at, b.at, Heading::Forward),
                    (nb, na, b.at, a.at, Heading::Backward),
                ] {
                    edges.push(Edge {
                        seg: si,
                        from,
                        to,
                        from_at,
                        to_at,
                        heading,
                        length_m,
                    });
                }
            }
        }
        let mut out = vec![Vec::new(); nodes.len()];
        for (i, e) in edges.iter().enumerate() {
            out[e.from].push(i);
        }
        Ok(Self {
            net,
            nodes,
            node_of: ids,
            edges,
            out,
            counters: Counters::default(),
        })
    }

    pub fn segment(&self, e: &Edge) -> &Segment {
        &self.net.segments()[e.seg]
    }

    /// `overrules::resolve`, counted.
    pub fn resolve<'a>(&self, seg: &'a Segment, s: &Situation) -> overrules::Result<Answer<'a>> {
        let c = &self.counters;
        c.resolves.set(c.resolves.get() + 1);
        let n = seg.access_restrictions.len()
            + seg.speed_limits.len()
            + seg.prohibited_transitions.len();
        c.rules.set(c.rules.get() + n as u64);
        overrules::resolve(seg, s)
    }

    /// The edge's geometry in the direction of travel.
    pub fn coords(&self, e: &Edge) -> Vec<[f64; 2]> {
        let mut v: Vec<[f64; 2]> = self
            .segment(e)
            .line()
            .slice(e.from_at, e.to_at)
            .0
            .iter()
            .map(|c| [c.x, c.y])
            .collect();
        if e.heading == Heading::Backward {
            v.reverse();
        }
        v
    }

    /// Where you are `metres` into an edge, and which way you face (degrees from north).
    pub fn point_on(&self, edge: usize, metres: f64) -> (f64, f64, f64) {
        let e = &self.edges[edge];
        let f = if e.length_m > 0.0 {
            (metres / e.length_m).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let line = self.segment(e).line();
        let at = line.point_at(e.from_at + f * (e.to_at - e.from_at));
        let mut b = line.bearing_at(&at);
        if e.heading == Heading::Backward {
            b = (b + 180.0) % 360.0;
        }
        (at.lon, at.lat, b)
    }

    pub fn node_pos(&self, n: usize) -> [f64; 2] {
        [self.nodes[n].lon, self.nodes[n].lat]
    }

    pub fn segment_label(&self, id: &str) -> String {
        self.net.get(id).map_or_else(
            || format!("segment {}… (outside the area)", &id[..id.len().min(8)]),
            name_of,
        )
    }

    /// "Broadway & Wall Street".
    pub fn place(&self, node: usize) -> String {
        let mut names: Vec<String> = self.out[node]
            .iter()
            .filter_map(|&e| self.segment(&self.edges[e]).name.clone())
            .collect();
        names.sort();
        names.dedup();
        match names.len() {
            0 => "an unnamed junction".to_owned(),
            1 => names[0].clone(),
            _ => format!("{} & {}", names[0], names[1]),
        }
    }

    /// `left`, `straight`, … for going from `prev` onto `next`.
    pub fn turn_name(&self, prev: Option<usize>, next: usize) -> &'static str {
        let Some(p) = prev else {
            return "start";
        };
        let ci = self.coords(&self.edges[p]);
        let co = self.coords(&self.edges[next]);
        let a = bearing(ci[ci.len().saturating_sub(2)], ci[ci.len() - 1]);
        let b = bearing(co[0], co[1.min(co.len() - 1)]);
        let d = (b - a + 540.0).rem_euclid(360.0) - 180.0;
        match d {
            d if d.abs() < 25.0 => "straight",
            d if d.abs() > 155.0 => "u-turn",
            d if d < -60.0 => "left",
            d if d < 0.0 => "bear left",
            d if d > 60.0 => "right",
            _ => "bear right",
        }
    }
}

pub fn name_of(seg: &Segment) -> String {
    match &seg.name {
        Some(n) => n.clone(),
        None => format!("unnamed {}", seg.class.as_deref().unwrap_or("road")),
    }
}

/// Approximate distance in metres, good enough for picking destinations.
pub fn crow_m(a: [f64; 2], b: [f64; 2]) -> f64 {
    let kx = (a[1].to_radians()).cos() * 111_320.0;
    let dx = (a[0] - b[0]) * kx;
    let dy = (a[1] - b[1]) * 110_540.0;
    dx.hypot(dy)
}

fn bearing(a: [f64; 2], b: [f64; 2]) -> f64 {
    let kx = a[1].to_radians().cos();
    ((b[0] - a[0]) * kx).atan2(b[1] - a[1]).to_degrees()
}

// ─── travellers ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Vehicle {
    Car,
    Van,
    Lorry,
    Bicycle,
    Foot,
}

impl Vehicle {
    pub const ALL: [Self; 5] = [Self::Car, Self::Van, Self::Lorry, Self::Bicycle, Self::Foot];

    /// The Overture travel mode. A van is a `truck`; a lorry is an `hgv`. The two are
    /// siblings under `motor_vehicle`, so rules for one don't apply to the other.
    pub fn mode(self) -> TravelMode {
        match self {
            Self::Car => TravelMode::Car,
            Self::Van => TravelMode::Truck,
            Self::Lorry => TravelMode::Hgv,
            Self::Bicycle => TravelMode::Bicycle,
            Self::Foot => TravelMode::Foot,
        }
    }

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn emoji(self) -> &'static str {
        match self {
            Self::Car => "🚗",
            Self::Van => "🚐",
            Self::Lorry => "🚛",
            Self::Bicycle => "🚲",
            Self::Foot => "🚶",
        }
    }

    pub fn noun(self) -> &'static str {
        match self {
            Self::Car => "cars",
            Self::Van => "vans",
            Self::Lorry => "lorries",
            Self::Bicycle => "bikes",
            Self::Foot => "pedestrians",
        }
    }

    pub fn singular(self) -> &'static str {
        match self {
            Self::Car => "car",
            Self::Van => "van",
            Self::Lorry => "lorry",
            Self::Bicycle => "bike",
            Self::Foot => "pedestrian",
        }
    }

    /// Height (m), weight (t), length (m), width (m), axles. Everyone has a size, so rules
    /// about size are never left open for want of one.
    pub fn dims(self) -> [(Dimension, f64); 5] {
        let (h, w, l, wd, ax) = match self {
            Self::Car => (1.5, 1.6, 4.6, 1.8, 2.0),
            Self::Van => (2.7, 3.5, 6.0, 2.1, 2.0),
            Self::Lorry => (4.1, 16.0, 10.0, 2.5, 3.0),
            Self::Bicycle => (1.8, 0.1, 1.8, 0.6, 2.0),
            Self::Foot => (1.8, 0.08, 0.5, 0.6, 0.0),
        };
        [
            (Dimension::Height, h),
            (Dimension::Weight, w),
            (Dimension::Length, l),
            (Dimension::Width, wd),
            (Dimension::AxleCount, ax),
        ]
    }

    /// Share of the speed limit actually driven, in city traffic. A game number.
    fn traffic_factor(self) -> f64 {
        match self {
            Self::Car => 0.6,
            Self::Van => 0.55,
            Self::Lorry => 0.5,
            _ => 1.0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Traveller {
    pub vehicle: Vehicle,
    pub using: Vec<String>,
    pub recognized: Vec<String>,
}

impl Traveller {
    pub fn new(vehicle: Vehicle) -> Self {
        Self {
            vehicle,
            using: Vec::new(),
            recognized: Vec::new(),
        }
    }

    /// The facts `overrules` gets about this traveller at a position on a road.
    pub fn situation(&self, at: f64, heading: Heading, time: NaiveDateTime) -> Situation {
        let mut s = Situation::new()
            .at(at.clamp(0.0, 1.0))
            .heading(heading)
            .mode(self.vehicle.mode())
            .time(time);
        for u in &self.using {
            s = s.using(u.clone());
        }
        for r in &self.recognized {
            s = s.recognized(r.clone());
        }
        for (d, v) in self.vehicle.dims() {
            s = s.vehicle(d, v);
        }
        s
    }
}

// ─── plain English ──────────────────────────────────────────────────────────

fn mode_words(m: &TravelMode) -> String {
    match m.as_str() {
        "vehicle" => "all vehicles".to_owned(),
        "motor_vehicle" => "motor vehicles".to_owned(),
        "car" => "cars".to_owned(),
        "truck" => "trucks and vans".to_owned(),
        "hgv" => "lorries (HGVs)".to_owned(),
        "bicycle" => "bikes".to_owned(),
        "foot" => "pedestrians".to_owned(),
        "bus" => "buses".to_owned(),
        "hov" => "car-pool vehicles".to_owned(),
        "motorcycle" => "motorcycles".to_owned(),
        "emergency" => "emergency vehicles".to_owned(),
        other => other.replace('_', " "),
    }
}

fn claim_words(s: &str) -> &str {
    match s {
        "to_deliver" => "delivering",
        "at_destination" => "local access",
        "as_customer" => "customers",
        "to_farm" => "farming",
        "for_forestry" => "forestry",
        "as_private" => "private access",
        "as_permitted" => "permit holders",
        "as_employee" => "staff",
        "as_student" => "students",
        "as_disabled" => "disabled badge holders",
        other => other,
    }
}

fn join_or(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [init @ .., last] => format!("{} or {last}", init.join(", ")),
    }
}

/// `for motor vehicles · travelling backward · Mo-Fr 07:00-10:00 · on 0–12% of the road`.
pub fn plain_scope<R: Scoped>(r: &R) -> String {
    let mut parts = Vec::new();
    if let Some(w) = r.when() {
        if let Some(m) = &w.mode {
            parts.push(format!(
                "for {}",
                join_or(&m.iter().map(mode_words).collect::<Vec<_>>())
            ));
        }
        if let Some(h) = w.heading {
            parts.push(format!("travelling {h}"));
        }
        if let Some(d) = &w.during {
            parts.push(format!("during {d}"));
        }
        if let Some(u) = &w.using {
            parts.push(format!(
                "when {}",
                join_or(
                    &u.iter()
                        .map(|x| claim_words(x).to_owned())
                        .collect::<Vec<_>>()
                )
            ));
        }
        if let Some(r) = &w.recognized {
            parts.push(format!(
                "for {}",
                join_or(
                    &r.iter()
                        .map(|x| claim_words(x).to_owned())
                        .collect::<Vec<_>>()
                )
            ));
        }
        for v in w.vehicle.iter().flatten() {
            let n = format!("{:.2}", v.value);
            let n = n.trim_end_matches('0').trim_end_matches('.');
            let unit = v
                .unit
                .map(|u| format!(" {}", u.as_str()))
                .unwrap_or_default();
            parts.push(format!(
                "if {} {} {n}{unit}",
                v.dimension.as_str().replace('_', " "),
                v.comparison.symbol()
            ));
        }
    }
    if let Some(b) = r.between() {
        let (lo, hi) = b.bounds();
        parts.push(format!(
            "on {:.0}–{:.0}% of the road",
            lo * 100.0,
            hi * 100.0
        ));
    }
    if parts.is_empty() {
        "always".to_owned()
    } else {
        parts.join(" · ")
    }
}

pub fn plain_access(r: &overrules::model::AccessRestriction) -> String {
    let what = match r.access_type {
        AccessType::Denied => "No entry",
        AccessType::Allowed => "Allowed",
        AccessType::Designated => "Designated",
    };
    format!("{what} {}", plain_scope(r))
}

pub fn plain_speed(r: &overrules::model::SpeedLimit) -> String {
    format!("{r} {}", plain_scope(r))
}

// ─── checks ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Ok,
    Uncertain,
    TurnBanned,
    Denied,
}

impl Status {
    pub fn is_legal(self) -> bool {
        matches!(self, Self::Ok | Self::Uncertain)
    }
}

/// Who decided: the Overture data read by `overrules`, or a rule the game adds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum By {
    Overrules,
    Game,
}

#[derive(Debug, Clone, Serialize)]
pub struct Reason {
    /// `ok`, `uncertain`, `turn_banned`, `denied`.
    pub kind: &'static str,
    pub by: By,
    pub headline: String,
    pub details: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct EdgeEval {
    pub status: Status,
    pub reasons: Vec<Reason>,
    pub off_limits: bool,
    pub limit_kmh: f64,
    pub limit_source: String,
    pub limit_by: By,
    pub travel_kmh: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Check {
    pub status: Status,
    pub reasons: Vec<Reason>,
    pub limit_kmh: f64,
    pub limit_source: String,
    pub limit_by: By,
    pub travel_kmh: f64,
    /// Closed only by the game's road-class defaults (a car onto a footway): not a real option.
    pub off_limits: bool,
}

impl Check {
    /// The reason that matters most.
    pub fn main_reason(&self) -> Option<&Reason> {
        self.reasons
            .iter()
            .find(|r| matches!(r.kind, "denied" | "turn_banned"))
            .or_else(|| self.reasons.iter().find(|r| r.kind == "uncertain"))
            .or_else(|| self.reasons.first())
    }

    /// The reason that matters most, for one-line messages.
    pub fn headline(&self) -> String {
        self.main_reason()
            .map_or_else(String::new, |r| r.headline.clone())
    }

    pub fn details(&self) -> Vec<String> {
        self.reasons
            .iter()
            .flat_map(|r| std::iter::once(r.headline.clone()).chain(r.details.iter().cloned()))
            .collect()
    }
}

/// A move already made: the edge, and when it started.
#[derive(Debug, Clone)]
pub struct Move {
    pub edge: usize,
    pub depart: NaiveDateTime,
}

fn verdict_icon(v: Verdict) -> &'static str {
    match v {
        Verdict::Match => "✓",
        Verdict::NoMatch => "✗",
        Verdict::Undetermined => "?",
    }
}

fn scope_lines(ev: &RuleEvaluation) -> Vec<String> {
    ev.scopes
        .iter()
        .map(|c| {
            format!(
                "{} {} {}: {}",
                verdict_icon(c.verdict),
                c.scope,
                c.requires,
                c.because
            )
        })
        .collect()
}

fn decided_by<R: Scoped>(
    what: &str,
    rules: &[R],
    res: &Resolution,
    text: impl Fn(&R) -> String,
) -> Vec<String> {
    let mut d = Vec::new();
    if let Some(i) = res.determining {
        d.push(format!(
            "{what} rule #{i} of {}: {}",
            rules.len(),
            text(&rules[i])
        ));
        d.extend(scope_lines(&res.rules[i]));
        let later = res.rules.len() - i - 1;
        if later > 0 {
            d.push(format!(
                "({later} later rule(s) did not match, so #{i} is the last match)"
            ));
        }
    }
    d
}

fn could_override_lines<R: Scoped>(
    rules: &[R],
    res: &Resolution,
    text: impl Fn(&R) -> String,
) -> Vec<String> {
    let mut d = Vec::new();
    for &i in &res.could_override {
        d.push(format!("rule #{i} might override: {}", text(&rules[i])));
        d.extend(
            scope_lines(&res.rules[i])
                .into_iter()
                .filter(|l| l.starts_with('?')),
        );
    }
    d
}

pub fn access_text(r: &overrules::model::AccessRestriction) -> String {
    format!("{} {}", r.access_type, r.scope_text())
}

pub fn speed_text(r: &overrules::model::SpeedLimit) -> String {
    format!("{r} {}", r.scope_text())
}

/// Game rule: road classes a vehicle doesn't use when no access rule says otherwise. Overture
/// publishes no such defaults and `overrules` deliberately doesn't invent them.
pub fn default_denied(v: Vehicle, seg: &Segment) -> Option<String> {
    let class = seg.class.as_deref().unwrap_or("unknown");
    let banned: &[&str] = match v {
        Vehicle::Car | Vehicle::Van | Vehicle::Lorry => &[
            "footway",
            "pedestrian",
            "steps",
            "path",
            "cycleway",
            "bridleway",
        ],
        Vehicle::Bicycle => &["motorway", "trunk", "footway", "steps", "pedestrian"],
        Vehicle::Foot => &["motorway", "trunk"],
    };
    banned
        .contains(&class)
        .then(|| format!("Game rule: {} don't use a {class} unless a rule lets them (Overture publishes no defaults)", v.noun()))
}

/// A rule that only says which way you may not go, and nothing about who: a one-way.
fn is_bare_one_way(r: &overrules::model::AccessRestriction) -> bool {
    r.access_type == AccessType::Denied
        && r.when.as_ref().is_some_and(|w| {
            w.heading.is_some()
                && w.mode.is_none()
                && w.during.is_none()
                && w.using.is_none()
                && w.recognized.is_none()
                && w.vehicle.is_none()
        })
}

/// Who a road's rules let in, when the rules that concern this traveller's mode only ever let
/// people in (private roads, permits, customers).
fn only_lets_in(seg: &Segment, mode: &TravelMode) -> Option<String> {
    let rules = &seg.access_restrictions;
    if rules.iter().any(|r| r.access_type == AccessType::Denied) {
        return None;
    }
    let concerns = |r: &&overrules::model::AccessRestriction| {
        r.when
            .as_ref()
            .and_then(|w| w.mode.as_ref())
            .is_none_or(|ms| ms.iter().any(|m| mode.is_within(m)))
    };
    let relevant: Vec<_> = rules.iter().filter(concerns).collect();
    if relevant.is_empty() {
        return None;
    }
    let mut who: Vec<String> = Vec::new();
    for r in relevant {
        let w = r.when.as_ref()?;
        let claims: Vec<&String> = w
            .recognized
            .iter()
            .chain(w.using.iter())
            .flatten()
            .collect();
        if claims.is_empty() {
            return None;
        }
        who.extend(claims.iter().map(|c| claim_words(c).to_owned()));
    }
    who.dedup();
    Some(join_or(&who))
}

fn default_limit_kmh(seg: &Segment) -> (f64, &'static str) {
    match seg.class.as_deref() {
        Some("motorway") => (80.0, "50 mph"),
        Some("trunk") => (64.0, "40 mph"),
        Some("service" | "living_street") => (24.0, "15 mph"),
        _ => (40.0, "25 mph, the NYC default"),
    }
}

/// What a traveller may do at one spot, with the game's own rules applied on top.
#[derive(Debug, Clone)]
pub struct AccessVerdict {
    pub status: Status,
    pub reason: Option<Reason>,
    pub off_limits: bool,
}

impl Traveller {
    pub fn access_at(
        &self,
        world: &World,
        seg: &Segment,
        at: f64,
        heading: Heading,
        t: NaiveDateTime,
    ) -> AccessVerdict {
        let ok = AccessVerdict {
            status: Status::Ok,
            reason: None,
            off_limits: false,
        };
        let rules = &seg.access_restrictions;
        let class_default = || {
            default_denied(self.vehicle, seg).map(|why| AccessVerdict {
                status: Status::Denied,
                reason: Some(Reason {
                    kind: "denied",
                    by: By::Game,
                    headline: format!(
                        "No {} on a {}",
                        self.vehicle.noun(),
                        seg.class.as_deref().unwrap_or("road")
                    ),
                    details: vec![why],
                }),
                off_limits: true,
            })
        };
        if rules.is_empty() {
            return class_default().unwrap_or(ok);
        }
        let Ok(ans) = world.resolve(seg, &self.situation(at, heading, t)) else {
            return ok;
        };
        let res = &ans.access;
        match ans.access_outcome() {
            Access::Rule(AccessType::Denied) => {
                let i = res.determining.unwrap_or(0);
                let mut details = decided_by("Access", rules, res, access_text);
                if self.vehicle == Vehicle::Foot && is_bare_one_way(&rules[i]) {
                    details.push("Game rule: a one-way that names no travel mode doesn't stop people on foot.".to_owned());
                    return AccessVerdict {
                        status: Status::Ok,
                        reason: Some(Reason {
                            kind: "ok",
                            by: By::Game,
                            headline: "One-way for traffic, walkers exempt".to_owned(),
                            details,
                        }),
                        off_limits: false,
                    };
                }
                if !res.is_settled() {
                    details.extend(could_override_lines(rules, res, access_text));
                    return AccessVerdict {
                        status: Status::Uncertain,
                        reason: Some(Reason {
                            kind: "uncertain",
                            by: By::Overrules,
                            headline:
                                "No entry, unless a fact the game doesn't know says otherwise"
                                    .to_owned(),
                            details,
                        }),
                        off_limits: false,
                    };
                }
                AccessVerdict {
                    status: Status::Denied,
                    reason: Some(Reason {
                        kind: "denied",
                        by: By::Overrules,
                        headline: format!("{} (rule #{i})", plain_access(&rules[i])),
                        details,
                    }),
                    off_limits: false,
                }
            }
            Access::Rule(a) => {
                let i = res.determining.unwrap_or(0);
                let mut details = decided_by("Access", rules, res, access_text);
                // "Designated for lorries" (a truck route) listed after a one-way wins by order,
                // which would send lorries the wrong way. Designation names a route, not a
                // direction, so the game keeps the one-way.
                let one_way = (a == AccessType::Designated
                    && rules[i].when.as_ref().is_some_and(|w| w.heading.is_none()))
                .then(|| {
                    (0..i).find(|&j| {
                        is_bare_one_way(&rules[j]) && res.rules[j].verdict == Verdict::Match
                    })
                })
                .flatten();
                if let Some(j) = one_way {
                    details.push(format!(
                        "Game rule: 'designated' marks a route for {}, not permission to ignore one-way rule #{j}.",
                        self.vehicle.noun()
                    ));
                    return AccessVerdict {
                        status: Status::Denied,
                        reason: Some(Reason {
                            kind: "denied",
                            by: By::Game,
                            headline: format!(
                                "One-way still applies on a route designated for {}",
                                self.vehicle.noun()
                            ),
                            details,
                        }),
                        off_limits: false,
                    };
                }
                if !res.is_settled() {
                    details.extend(could_override_lines(rules, res, access_text));
                    return AccessVerdict {
                        status: Status::Uncertain,
                        reason: Some(Reason {
                            kind: "uncertain",
                            by: By::Overrules,
                            headline: format!("{a} for now, but a later rule might override"),
                            details,
                        }),
                        off_limits: false,
                    };
                }
                AccessVerdict {
                    status: Status::Ok,
                    reason: Some(Reason {
                        kind: "ok",
                        by: By::Overrules,
                        headline: format!("{} (rule #{i})", plain_access(&rules[i])),
                        details,
                    }),
                    off_limits: false,
                }
            }
            Access::Default => {
                if let Some(v) = class_default() {
                    return v;
                }
                if let Some(who) = only_lets_in(seg, &self.vehicle.mode()) {
                    return AccessVerdict {
                        status: Status::Denied,
                        reason: Some(Reason {
                            kind: "denied",
                            by: By::Game,
                            headline: format!("Only for {who}"),
                            details: vec![
                                "overrules: no rule matched you, so the answer is 'no rule says' (not 'allowed').".to_owned(),
                                format!("Game rule: this road's rules only ever let in {who}, so everyone else is kept out."),
                            ],
                        }),
                        off_limits: false,
                    };
                }
                if !res.is_settled() {
                    return AccessVerdict {
                        status: Status::Uncertain,
                        reason: Some(Reason {
                            kind: "uncertain",
                            by: By::Overrules,
                            headline: "Open, unless a fact the game doesn't know says otherwise"
                                .to_owned(),
                            details: could_override_lines(rules, res, access_text),
                        }),
                        off_limits: false,
                    };
                }
                ok
            }
        }
    }

    /// Access and speed on one edge. Supplement the usual samples with every interval
    /// between access-rule boundaries, so short closures cannot fall between samples.
    pub fn edge_eval(
        &self,
        world: &World,
        e: &Edge,
        t: NaiveDateTime,
        samples: &[f64],
    ) -> EdgeEval {
        let seg = world.segment(e);
        let mut worst = AccessVerdict {
            status: Status::Ok,
            reason: None,
            off_limits: false,
        };
        let mut first_reason = None;
        let (lo, hi) = (e.from_at.min(e.to_at), e.from_at.max(e.to_at));
        let mut bounds: Vec<f64> = seg
            .access_restrictions
            .iter()
            .filter_map(|r| r.between)
            .flat_map(|b| [b.start, b.end])
            .filter(|&at| at > lo && at < hi)
            .collect();
        if !bounds.is_empty() {
            bounds.extend([lo, hi]);
            bounds.sort_by(f64::total_cmp);
            bounds.dedup();
        }
        let points = samples
            .iter()
            .map(|&f| e.from_at + f * (e.to_at - e.from_at))
            .chain(bounds.windows(2).map(|b| f64::midpoint(b[0], b[1])));
        for at in points {
            let v = self.access_at(world, seg, at, e.heading, t);
            if first_reason.is_none() {
                first_reason.clone_from(&v.reason);
            }
            if v.status > worst.status {
                worst = v;
            }
            if seg.access_restrictions.is_empty() || worst.status == Status::Denied {
                break;
            }
        }
        let reason = if worst.status == Status::Ok {
            first_reason
        } else {
            worst.reason
        };

        let (limit_kmh, limit_source, limit_by) = {
            let fallback = |why: &str| {
                let (k, label) = default_limit_kmh(seg);
                (k, format!("{label} ({why}, game default)"), By::Game)
            };
            if seg.speed_limits.is_empty() {
                fallback("no speed rules")
            } else {
                let mid = f64::midpoint(e.from_at, e.to_at);
                match world.resolve(seg, &self.situation(mid, e.heading, t)) {
                    Ok(ans) => match (
                        ans.speed_limit.determining,
                        ans.speed_limit_rule().and_then(|r| r.max_speed),
                    ) {
                        (Some(i), Some(s)) => {
                            let kmh = match s.unit {
                                SpeedUnit::KilometresPerHour => f64::from(s.value),
                                SpeedUnit::MilesPerHour => f64::from(s.value) * 1.609_344,
                            };
                            (
                                kmh,
                                format!(
                                    "{s} (speed rule #{i}: {})",
                                    plain_speed(&seg.speed_limits[i])
                                ),
                                By::Overrules,
                            )
                        }
                        _ => fallback("no speed rule matched"),
                    },
                    Err(_) => fallback("unreadable"),
                }
            }
        };
        let class = seg.class.as_deref().unwrap_or("");
        let travel_kmh = match self.vehicle {
            Vehicle::Bicycle => limit_kmh.min(16.0),
            Vehicle::Foot if class == "steps" => 2.0,
            Vehicle::Foot => 5.0,
            v => limit_kmh * v.traffic_factor(),
        };
        EdgeEval {
            status: worst.status,
            reasons: reason.into_iter().collect(),
            off_limits: worst.off_limits,
            limit_kmh,
            limit_source,
            limit_by,
            travel_kmh,
        }
    }

    /// Prohibited transitions completed by stepping onto `cand` after `trail`.
    pub fn turn_check(
        &self,
        world: &World,
        trail: &[Move],
        cand: &Edge,
        now: NaiveDateTime,
    ) -> Option<(Status, Reason)> {
        struct Visit {
            seg: usize,
            heading: Heading,
            entry: usize,
            exit: usize,
            leave_at: f64,
            leave_time: NaiveDateTime,
        }
        let mut visits: Vec<Visit> = Vec::new();
        for (k, m) in trail.iter().enumerate() {
            let e = &world.edges[m.edge];
            let leave_time = trail.get(k + 1).map_or(now, |n| n.depart);
            match visits.last_mut() {
                Some(v) if v.seg == e.seg && v.heading == e.heading => {
                    v.leave_at = e.to_at;
                    v.leave_time = leave_time;
                    v.exit = e.to;
                }
                _ => visits.push(Visit {
                    seg: e.seg,
                    heading: e.heading,
                    entry: e.from,
                    exit: e.to,
                    leave_at: e.to_at,
                    leave_time,
                }),
            }
        }
        let last = visits.last()?;
        if last.seg == cand.seg && last.heading == cand.heading {
            return None;
        }
        let segs = world.net.segments();
        let step = |node: usize, seg: usize| (world.nodes[node].id.as_str(), segs[seg].id.as_str());
        let mut uncertain = None;
        for k in (visits.len().saturating_sub(4)..visits.len()).rev() {
            let origin = &segs[visits[k].seg];
            if origin.prohibited_transitions.is_empty() {
                continue;
            }
            let mut steps: Vec<(&str, &str)> = visits[k + 1..]
                .iter()
                .map(|v| step(v.entry, v.seg))
                .collect();
            steps.push(step(cand.from, cand.seg));
            let s = self.situation(visits[k].leave_at, visits[k].heading, visits[k].leave_time);
            let Ok(ans) = world.resolve(origin, &s) else {
                continue;
            };
            let res = &ans.prohibited_transitions;
            let rules = &origin.prohibited_transitions;
            let matches = |i: usize| {
                let r = &rules[i];
                r.final_heading == cand.heading
                    && r.sequence.len() == steps.len()
                    && r.sequence
                        .iter()
                        .zip(&steps)
                        .all(|(a, b)| a.connector_id == b.0 && a.segment_id == b.1)
            };
            let describe = |i: usize| {
                let r = &rules[i];
                let path: Vec<String> = r
                    .sequence
                    .iter()
                    .map(|st| world.segment_label(&st.segment_id))
                    .collect();
                let mut d = vec![format!(
                    "Turn rule #{i} on {}: no {} → {} (ending {}) {}",
                    name_of(origin),
                    name_of(origin),
                    path.join(" → "),
                    r.final_heading,
                    r.scope_text()
                )];
                d.extend(scope_lines(&res.rules[i]));
                d
            };
            if let Some(&i) = res.in_force.iter().find(|&&i| matches(i)) {
                let onto = world.segment_label(
                    rules[i]
                        .sequence
                        .last()
                        .map_or("", |s| s.segment_id.as_str()),
                );
                return Some((
                    Status::TurnBanned,
                    Reason {
                        kind: "turn_banned",
                        by: By::Overrules,
                        headline: format!(
                            "No turn from {} onto {onto} (turn rule #{i})",
                            name_of(origin)
                        ),
                        details: describe(i),
                    },
                ));
            }
            if uncertain.is_none()
                && let Some(&i) = res.could_apply.iter().find(|&&i| matches(i))
            {
                uncertain = Some((
                    Status::Uncertain,
                    Reason {
                        kind: "uncertain",
                        by: By::Overrules,
                        headline: format!("Turn from {} might be banned", name_of(origin)),
                        details: describe(i),
                    },
                ));
            }
        }
        uncertain
    }

    pub fn combine(ev: EdgeEval, turn: Option<(Status, Reason)>) -> Check {
        let mut status = ev.status;
        let mut reasons = Vec::new();
        if let Some((s, r)) = turn {
            status = status.max(s);
            reasons.push(r);
        }
        reasons.extend(ev.reasons);
        Check {
            status,
            reasons,
            limit_kmh: ev.limit_kmh,
            limit_source: ev.limit_source,
            limit_by: ev.limit_by,
            travel_kmh: ev.travel_kmh,
            off_limits: ev.off_limits,
        }
    }

    pub fn check_move(
        &self,
        world: &World,
        trail: &[Move],
        edge: usize,
        t: NaiveDateTime,
    ) -> Check {
        let e = &world.edges[edge];
        let ev = self.edge_eval(world, e, t, &SAMPLES);
        let turn = self.turn_check(world, trail, e, t);
        Self::combine(ev, turn)
    }
}

/// Where along each stretch access is checked.
pub const SAMPLES: [f64; 3] = [0.15, 0.5, 0.85];

// ─── misc ───────────────────────────────────────────────────────────────────

pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed.max(1).wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    pub fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n.max(1) as u64) as usize
    }
    pub fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}

pub fn parse_time(s: &str) -> Result<NaiveDateTime, String> {
    NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M")
        .or_else(|_| NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S"))
        .map_err(|e| format!("bad time {s:?}: {e}"))
}

pub fn fmt_clock(t: NaiveDateTime) -> String {
    t.format("%a %H:%M:%S").to_string()
}
