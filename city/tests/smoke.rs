//! Runs the real play area natively: the city for a while, the game rules, and every claim the
//! tour makes on screen.

#![allow(clippy::unwrap_used)]

use std::time::Instant;

use chrono::Duration;
use geo::Point;
use overrules::Heading;
use overrules_city::engine::{By, SAMPLES, Status, Traveller, Vehicle, World, parse_time};
use overrules_city::inspect::{ROWS, ask, worst_planar};
use overrules_city::signs::signs;
use overrules_city::sim::Sim;
use serde_json::Value;

fn world() -> Option<World> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../web/data/area.json.gz");
    let Ok(file) = std::fs::File::open(path) else {
        eprintln!("skipping: {path} not built (run `just prep`)");
        return None;
    };
    let mut text = String::new();
    std::io::Read::read_to_string(&mut flate2::read::GzDecoder::new(file), &mut text)
        .expect("gzipped GeoJSON");
    Some(World::from_geojson(&text).expect("the play area reads"))
}

fn row(label: &str) -> usize {
    ROWS.iter()
        .position(|(l, _)| l.ends_with(label))
        .expect("a grid row")
}

/// What overrules says about `id` at a point, as the tour asks it.
fn q(
    world: &World,
    id: &str,
    ll: [f64; 2],
    who: &str,
    time: &str,
    heading: Heading,
    claim: &str,
) -> Value {
    let seg = world.net.get(id).expect("the road is in the area");
    let at = seg.line().project(Point::new(ll[0], ll[1])).at.fraction;
    let t = (!time.is_empty()).then(|| parse_time(time).expect("a time"));
    ask(world, seg, at, row(who), t, heading, claim)
}

fn outcome(v: &Value) -> (&str, bool, Option<u64>) {
    let decided = v["access"]["rules"]
        .as_array()
        .and_then(|r| r.iter().find(|x| x["decided"] == true))
        .and_then(|x| x["index"].as_u64());
    (
        v["access"]["outcome"].as_str().unwrap_or(""),
        v["access"]["settled"].as_bool().unwrap_or(false),
        decided,
    )
}

#[test]
fn the_city_runs() {
    let Some(world) = world() else { return };
    let mut clock = parse_time("2026-10-19T08:00").unwrap();
    let mut sim = Sim::new(&world, clock, 7);
    let t = Instant::now();
    let steps = 1800;
    for _ in 0..steps {
        let dt = 5.0 / 30.0;
        clock += Duration::milliseconds((dt * 1000.0) as i64);
        sim.step(&world, clock, dt);
    }
    let el = t.elapsed();
    eprintln!(
        "{} travellers, {steps} steps in {el:?} ({:?}/step): {} decisions, {} refusals",
        sim.agents.len(),
        el / steps,
        sim.stats.decisions,
        sim.stats.refusals
    );
    assert!(sim.agents.len() > 500);
    assert!(sim.stats.decisions > 1000);
    assert!(!sim.feed.is_empty());
    for v in Vehicle::ALL {
        let s = signs(&world, &Traveller::new(v), clock);
        assert!(!s["features"].as_array().unwrap().is_empty());
    }
}

#[test]
fn game_rules_do_what_they_say() {
    let Some(world) = world() else { return };
    let clock = parse_time("2026-10-19T08:10").unwrap();
    let tally = |v: Vehicle| {
        let tr = Traveller::new(v);
        let (mut denied, mut game) = (0, 0);
        for e in &world.edges {
            let ev = tr.edge_eval(&world, e, clock, &SAMPLES);
            if ev.off_limits {
                continue;
            }
            denied += usize::from(ev.status == Status::Denied);
            game += usize::from(ev.reasons.iter().any(|r| r.by == By::Game));
        }
        eprintln!("{v:?}: {denied} stretches refused, {game} decided by a game rule");
        denied
    };
    let (van, lorry, car, foot) = (
        tally(Vehicle::Van),
        tally(Vehicle::Lorry),
        tally(Vehicle::Car),
        tally(Vehicle::Foot),
    );
    // HGV rules apply to lorries, and truck routes don't send them the wrong way.
    assert!(
        lorry > van,
        "lorries ({lorry}) should be refused more than vans ({van})"
    );
    // One-ways that name no mode don't stop people on foot.
    assert!(foot * 3 < car, "walkers ({foot}) vs cars ({car})");
}

#[test]
fn the_tour_tells_the_truth() {
    let Some(world) = world() else { return };
    use Heading::{Backward, Forward};
    let greenway = (
        "f80e2915-6d4d-4b66-9955-44227cffe820",
        [-74.006626, 40.713599],
    );
    let e14 = (
        "ae552ea3-6519-4484-8080-e40790cafba2",
        [-73.990584, 40.734722],
    );
    let fdr = (
        "069c144d-c1b9-46d6-a24c-3f230e4346f5",
        [-73.998765, 40.708429],
    );
    let bridge = (
        "eae80a3f-2bd3-4119-a22c-c8692e77103e",
        [-73.992926, 40.703083],
    );
    let canal = (
        "d19de29b-115e-494b-8643-38ad470833ce",
        [-73.996334, 40.716263],
    );
    let rivington = (
        "2e39c957-e0a9-44ad-bbd1-4eee3c9411c5",
        [-73.989053, 40.720289],
    );
    let ask = |r: (&str, [f64; 2]), who, time, h, claim| q(&world, r.0, r.1, who, time, h, claim);

    // 02 · last match wins.
    assert_eq!(
        outcome(&ask(greenway, "On foot", "2026-10-25T03:00", Forward, "")),
        ("denied", true, Some(2))
    );
    assert_eq!(
        outcome(&ask(greenway, "Bike", "2026-10-25T03:00", Forward, "")),
        ("designated", true, Some(4))
    );
    // 03 · the busway opens at 22:00.
    assert_eq!(
        outcome(&ask(e14, "Car", "2026-10-19T21:59", Forward, "")).0,
        "denied"
    );
    assert_eq!(
        outcome(&ask(e14, "Car", "2026-10-19T22:01", Forward, "")).0,
        "none"
    );
    // 04 · the travel-mode tree.
    assert_eq!(
        outcome(&ask(fdr, "Car", "2026-10-19T08:05", Forward, "")).0,
        "designated"
    );
    assert_eq!(
        outcome(&ask(fdr, "Van", "2026-10-19T08:05", Forward, "")).0,
        "none"
    );
    assert_eq!(
        outcome(&ask(fdr, "Lorry", "2026-10-19T08:05", Forward, "")).0,
        "denied"
    );
    assert!(
        !outcome(&ask(
            fdr,
            "Some motor vehicle",
            "2026-10-19T08:05",
            Forward,
            ""
        ))
        .1
    );
    // 05 · no time, no guess.
    assert!(!outcome(&ask(greenway, "On foot", "", Forward, "")).1);
    assert!(outcome(&ask(greenway, "On foot", "2026-10-19T08:05", Forward, "")).1);
    // 06 · both size conditions must hold.
    assert_ne!(
        outcome(&ask(bridge, "Van", "2026-10-19T08:05", Forward, "")).0,
        "denied"
    );
    assert_eq!(
        outcome(&ask(bridge, "Lorry", "2026-10-19T08:05", Forward, "")).0,
        "denied"
    );
    // 07 · a one-step and a two-step turn ban.
    let turns = ask(canal, "Car", "2026-10-19T08:05", Forward, "");
    assert_eq!(turns["turns"]["in_force"].as_array().map(Vec::len), Some(2));
    // 08 · malformed hours, repaired.
    let riv = ask(
        rivington,
        "Car",
        "2026-10-24T13:00",
        Backward,
        "at_destination",
    );
    assert_eq!(outcome(&riv), ("allowed", true, Some(1)));
    assert!(riv["access"]["rules"][1]["repaired"] == true);
    // 09 · flat maths moves a boundary by more than 10 m.
    let w = worst_planar(&world);
    eprintln!(
        "worst planar boundary: {} at {:.3}: {:.1} m",
        w["name"], w["fraction"], w["error_m"]
    );
    assert!(w["error_m"].as_f64().unwrap_or(0.0) > 10.0);
}
