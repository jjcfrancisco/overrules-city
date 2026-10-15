//! The road panel: every rule on a road, evaluated for someone at a moment, and the
//! "what if" grid that asks the same road for other travellers and other times.

use chrono::{Datelike, Duration, NaiveDateTime, NaiveTime};
use overrules::eval::{RuleEvaluation, Verdict};
use overrules::model::Scoped;
use overrules::{Access, Heading, Segment, Situation, TravelMode};
use serde_json::{Value, json};

use crate::engine::{
    By, Traveller, Vehicle, World, access_text, fmt_clock, name_of, plain_access, plain_scope,
    plain_speed, speed_text,
};

/// Rows of the grid: who is asking. `None` is a motor vehicle of unknown kind.
pub const ROWS: [(&str, Option<Vehicle>); 6] = [
    ("🚗 Car", Some(Vehicle::Car)),
    ("🚐 Van", Some(Vehicle::Van)),
    ("🚛 Lorry", Some(Vehicle::Lorry)),
    ("🚙 Some motor vehicle", None),
    ("🚲 Bike", Some(Vehicle::Bicycle)),
    ("🚶 On foot", Some(Vehicle::Foot)),
];

/// Columns of the grid: when. `None` is "time not given".
pub fn columns(now: NaiveDateTime) -> [(String, Option<NaiveDateTime>); 4] {
    let next = |weekday: u32, h: u32, m: u32| {
        let wd = now.weekday().num_days_from_monday();
        let days = (i64::from(weekday) - i64::from(wd)).rem_euclid(7);
        let d = now.date() + Duration::days(days);
        d.and_time(NaiveTime::from_hms_opt(h, m, 0).unwrap_or_default())
    };
    [
        (format!("Now ({})", now.format("%a %H:%M")), Some(now)),
        ("Fri 17:30".to_owned(), Some(next(4, 17, 30))),
        ("Sun 03:00".to_owned(), Some(next(6, 3, 0))),
        ("Time unknown".to_owned(), None),
    ]
}

/// The facts for a grid cell: only what the row, the column and the claimed purpose or
/// status say, nothing else.
pub fn profile(
    row: usize,
    time: Option<NaiveDateTime>,
    at: f64,
    heading: Heading,
    claim: &str,
) -> Situation {
    let (_, v) = ROWS[row.min(ROWS.len() - 1)];
    let mut s = Situation::new()
        .at(at)
        .heading(heading)
        .mode(v.map_or(TravelMode::MotorVehicle, Vehicle::mode));
    if let Some(t) = time {
        s = s.time(t);
    }
    if let Some(v) = v {
        for (d, x) in v.dims() {
            s = s.vehicle(d, x);
        }
    }
    match claim {
        "" => s,
        "to_deliver" | "at_destination" | "as_customer" | "to_farm" | "for_forestry" => {
            s.using(claim)
        }
        _ => s.recognized(claim),
    }
}

fn verdict_name(v: Verdict) -> &'static str {
    match v {
        Verdict::Match => "match",
        Verdict::NoMatch => "no_match",
        Verdict::Undetermined => "undetermined",
    }
}

/// One rule list, each rule with its plain-English text, raw text, and what overrules said.
fn rule_rows<R: Scoped>(
    rules: &[R],
    evals: &[RuleEvaluation],
    deciding: &[usize],
    last_match: bool,
    plain: impl Fn(&R) -> String,
    raw: impl Fn(&R) -> String,
) -> Vec<Value> {
    let top = deciding.iter().max().copied();
    evals
        .iter()
        .zip(rules)
        .map(|(ev, r)| {
            let decided = deciding.contains(&ev.index);
            // Last match wins: a match before the deciding rule was overridden by it.
            let overridden =
                last_match && ev.verdict == Verdict::Match && top.is_some_and(|t| ev.index < t);
            let repaired = ev.scopes.iter().any(|c| c.because.contains("(read \""));
            json!({
                "index": ev.index,
                "plain": plain(r),
                "raw": raw(r),
                "verdict": verdict_name(ev.verdict),
                "decided": decided,
                "overridden": overridden,
                "repaired": repaired,
                "scopes": ev.scopes,
            })
        })
        .collect()
}

fn outcome_name(a: Access) -> &'static str {
    match a {
        Access::Rule(t) => t.as_str(),
        Access::Default => "none",
    }
}

/// Every rule on `seg`, as overrules answers it for `s`.
pub fn evaluate_all(world: &World, seg: &Segment, s: &Situation) -> Value {
    let Ok(ans) = world.resolve(seg, s) else {
        return Value::Null;
    };
    let turns = &seg.prohibited_transitions;
    let turn_plain = |r: &overrules::model::ProhibitedTransition| {
        let path: Vec<String> = r
            .sequence
            .iter()
            .map(|st| world.segment_label(&st.segment_id))
            .collect();
        format!("No turn onto {} ({})", path.join(" → "), plain_scope(r))
    };
    let turn_raw = |r: &overrules::model::ProhibitedTransition| {
        format!("ending {} {}", r.final_heading, r.scope_text())
    };
    json!({
        "access": {
            "outcome": outcome_name(ans.access_outcome()),
            "settled": ans.access.is_settled(),
            "could_override": ans.access.could_override,
            "rules": rule_rows(&seg.access_restrictions, &ans.access.rules, ans.access.determining.as_slice(), true, plain_access, access_text),
        },
        "speed": {
            "rule": ans.speed_limit_rule().map(ToString::to_string),
            "settled": ans.speed_limit.is_settled(),
            "rules": rule_rows(&seg.speed_limits, &ans.speed_limit.rules, ans.speed_limit.determining.as_slice(), true, plain_speed, speed_text),
        },
        "turns": {
            "in_force": ans.prohibited_transitions.in_force,
            "could_apply": ans.prohibited_transitions.could_apply,
            "rules": rule_rows(turns, &ans.prohibited_transitions.rules, &ans.prohibited_transitions.in_force, false, turn_plain, turn_raw),
        },
    })
}

/// The panel for `seg` at fraction `at`: what the game's traveller faces, or a grid cell.
pub fn inspect(
    world: &World,
    seg: &Segment,
    at: f64,
    tr: &Traveller,
    clock: NaiveDateTime,
    cell: Option<(usize, usize)>,
    claim: &str,
) -> Value {
    let p = seg.line().point_at(at);
    let cols = columns(clock);
    let (who, when) = match cell {
        Some((r, c)) => (
            ROWS[r.min(ROWS.len() - 1)].0.to_owned(),
            cols[c.min(cols.len() - 1)].0.clone(),
        ),
        None => (
            format!("{} {}", tr.vehicle.emoji(), tr.vehicle.singular()),
            fmt_clock(clock),
        ),
    };
    let headings: Vec<Value> = Heading::ALL
        .iter()
        .map(|&h| {
            let s = match cell {
                Some((r, c)) => profile(r, cols[c.min(cols.len() - 1)].1, at, h, claim),
                None => tr.situation(at, h, clock),
            };
            let mut v = evaluate_all(world, seg, &s);
            // For the game's own traveller, also say what the game makes of it.
            if cell.is_none() {
                let g = tr.access_at(world, seg, at, h, clock);
                v["game"] = json!({
                    "status": g.status,
                    "by": g.reason.as_ref().map(|r| r.by),
                    "headline": g.reason.as_ref().map(|r| r.headline.clone()),
                    "game_rule": g.reason.as_ref().is_some_and(|r| r.by == By::Game),
                    "note": g.reason.as_ref().and_then(|r| r.details.iter().find(|d| d.starts_with("Game rule")).cloned()),
                });
            }
            v["heading"] = json!(h);
            v
        })
        .collect();
    json!({
        "id": seg.id,
        "name": name_of(seg),
        "class": seg.class,
        "subclass": seg.subclass,
        "length_m": seg.length_m(),
        "at": at,
        "point": [p.lon, p.lat],
        "who": who,
        "when": when,
        "cell": cell.map(|(r, c)| [r, c]),
        "headings": headings,
    })
}

/// The same road asked by every row at every column, both ways.
pub fn what_if(world: &World, seg: &Segment, at: f64, clock: NaiveDateTime, claim: &str) -> Value {
    let cols = columns(clock);
    let rows: Vec<Value> =
        ROWS.iter()
            .enumerate()
            .map(|(r, (label, _))| {
                let cells: Vec<Value> = cols
                .iter()
                .map(|(_, t)| {
                    let one = |h| {
                        world.resolve(seg, &profile(r, *t, at, h, claim)).map_or(json!(null), |a| {
                            json!({
                                "outcome": outcome_name(a.access_outcome()),
                                "settled": a.access.is_settled(),
                                "rule": a.access.determining,
                            })
                        })
                    };
                    json!({"forward": one(Heading::Forward), "backward": one(Heading::Backward)})
                })
                .collect();
                json!({"label": label, "cells": cells})
            })
            .collect();
    json!({
        "columns": cols.iter().map(|(l, _)| l.clone()).collect::<Vec<_>>(),
        "rows": rows,
        "rules": seg.access_restrictions.len(),
    })
}

/// One question, for the tour: row `row` of the grid, at `time` (None: unknown), going
/// `heading`, claiming `claim`.
pub fn ask(
    world: &World,
    seg: &Segment,
    at: f64,
    row: usize,
    time: Option<NaiveDateTime>,
    heading: Heading,
    claim: &str,
) -> Value {
    let mut v = evaluate_all(world, seg, &profile(row, time, at, heading, claim));
    v["who"] = json!(ROWS[row.min(ROWS.len() - 1)].0);
    v["when"] = json!(time.map_or_else(|| "time unknown".to_owned(), fmt_clock));
    v["heading"] = json!(heading);
    v["name"] = json!(name_of(seg));
    v
}

/// Where flat-map maths would put a rule boundary, against where it really is: the
/// boundary in the area that would move furthest.
pub fn worst_planar(world: &World) -> Value {
    let mut best: Option<(f64, &Segment, f64, String)> = None;
    for seg in world.net.segments() {
        let bounds = seg
            .access_restrictions
            .iter()
            .filter_map(|r| r.between.map(|b| (b, plain_access(r))))
            .chain(
                seg.speed_limits
                    .iter()
                    .filter_map(|r| r.between.map(|b| (b, plain_speed(r)))),
            );
        for (b, text) in bounds {
            for f in [b.start, b.end] {
                if f <= 0.0 || f >= 1.0 {
                    continue;
                }
                let err = overrules::lr::planar::error_at(seg.line(), f);
                if best.as_ref().is_none_or(|x| err > x.0) {
                    best = Some((err, seg, f, text.clone()));
                }
            }
        }
    }
    let Some((err, seg, f, text)) = best else {
        return Value::Null;
    };
    let g = seg.line().point_at(f);
    let p = overrules::lr::planar::point_at(seg.line(), f);
    json!({
        "id": seg.id,
        "name": name_of(seg),
        "fraction": f,
        "error_m": err,
        "rule": text,
        "geodesic": [g.lon, g.lat],
        "planar": [p.x(), p.y()],
        "length_m": seg.length_m(),
    })
}
