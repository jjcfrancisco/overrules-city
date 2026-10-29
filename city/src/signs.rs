//! Rules drawn on the map, for one traveller at one moment: one-way arrows, closed roads,
//! roads whose rules depend on the time, and banned turns.

use chrono::NaiveDateTime;
use overrules::Heading;
use overrules::eval::{Verdict, evaluate};
use serde_json::{Value, json};

use crate::engine::{Status, Traveller, World, name_of};

pub fn signs(world: &World, tr: &Traveller, clock: NaiveDateTime) -> Value {
    let mut features = Vec::new();
    for seg in world.net.segments() {
        let fwd = tr.access_at(world, seg, 0.5, Heading::Forward, clock);
        let bwd = tr.access_at(world, seg, 0.5, Heading::Backward, clock);
        // Hide roads excluded only by the game's defaults. Explicit access rules
        // can still make a pedestrian street usable by bikes, for example.
        if fwd.off_limits && bwd.off_limits {
            continue;
        }
        let timed = seg
            .access_restrictions
            .iter()
            .any(|r| r.when.as_ref().is_some_and(|w| w.during.is_some()));
        if !seg.access_restrictions.is_empty() {
            let (fwd, bwd) = (fwd.status, bwd.status);
            let mut coords: Vec<[f64; 2]> = seg.geometry().0.iter().map(|c| [c.x, c.y]).collect();
            let no = |s: Status| s == Status::Denied;
            let kind = match (no(fwd), no(bwd)) {
                (false, true) => Some("oneway"),
                (true, false) => {
                    coords.reverse();
                    Some("oneway")
                }
                (true, true) => Some("closed"),
                (false, false) if fwd == Status::Uncertain || bwd == Status::Uncertain => {
                    Some("unsure")
                }
                (false, false) => timed.then_some("timed"),
            };
            if let Some(kind) = kind {
                features.push(json!({
                    "type": "Feature",
                    "geometry": {"type": "LineString", "coordinates": coords},
                    "properties": {"kind": kind, "timed": timed, "id": seg.id, "name": name_of(seg)},
                }));
            }
        }

        for (i, rule) in seg.prohibited_transitions.iter().enumerate() {
            let Some(first) = rule.sequence.first() else {
                continue;
            };
            let Some(conn) = seg
                .connectors
                .iter()
                .find(|c| c.connector_id == first.connector_id)
            else {
                continue;
            };
            let heading = rule
                .when
                .as_ref()
                .and_then(|w| w.heading)
                .unwrap_or(if conn.at >= 0.5 {
                    Heading::Forward
                } else {
                    Heading::Backward
                });
            let ev = evaluate(i, rule, &tr.situation(conn.at, heading, clock));
            let c = &world.counters;
            c.rules.set(c.rules.get() + 1);
            if ev.verdict == Verdict::NoMatch {
                continue;
            }
            // Just before the junction on the road the ban starts from, so bans on different
            // approaches to one junction don't sit on top of each other.
            let back = if heading == Heading::Forward {
                -1.0
            } else {
                1.0
            } * (12.0 / seg.length_m().max(1.0));
            let p = seg.line().point_at((conn.at + back).clamp(0.0, 1.0));
            let onto = world.segment_label(&first.segment_id);
            features.push(json!({
                "type": "Feature",
                "geometry": {"type": "Point", "coordinates": [p.lon, p.lat]},
                "properties": {
                    "kind": "noturn",
                    "sure": ev.verdict == Verdict::Match,
                    "id": seg.id,
                    "text": format!("No turn from {} onto {onto}", name_of(seg)),
                },
            }));
        }
    }
    json!({"type": "FeatureCollection", "features": features})
}
