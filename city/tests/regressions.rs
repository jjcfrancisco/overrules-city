//! Small road networks that exercise the rule-handling failures found in review.

#![allow(clippy::unwrap_used)]

use overrules::Heading;
use overrules_city::engine::{Status, Traveller, Vehicle, World, parse_time};
use overrules_city::signs::signs;
use serde_json::json;

#[test]
fn moves_check_short_closures_between_sample_points() {
    let world = World::from_geojson(
        &json!({
            "type": "FeatureCollection",
            "features": [{
                "type": "Feature",
                "geometry": {"type": "LineString", "coordinates": [[-74.0, 40.7], [-74.0, 40.701]]},
                "properties": {
                    "id": "greenway", "subtype": "road", "class": "cycleway",
                    "connectors": [{"connector_id": "a", "at": 0.0}, {"connector_id": "b", "at": 1.0}],
                    "access_restrictions": [
                        {"access_type": "allowed"},
                        {"access_type": "denied", "between": [0.91, 0.94], "when": {"mode": ["foot"]}},
                        {"access_type": "allowed", "between": [0.92, 0.93]}
                    ]
                }
            }]
        })
        .to_string(),
    )
    .unwrap();
    let time = parse_time("2026-10-19T08:00").unwrap();
    let walker = Traveller::new(Vehicle::Foot);
    let cyclist = Traveller::new(Vehicle::Bicycle);
    for (i, edge) in world.edges.iter().enumerate() {
        assert_eq!(
            walker.check_move(&world, &[], i, time).status,
            Status::Denied
        );
        // A later permission in the middle of the closure must not hide either side.
        assert_eq!(
            walker
                .access_at(&world, world.segment(edge), 0.925, edge.heading, time)
                .status,
            Status::Ok
        );
        assert_eq!(cyclist.check_move(&world, &[], i, time).status, Status::Ok);
    }
}

#[test]
fn signs_honor_explicit_access_on_default_excluded_road_classes() {
    let world = World::from_geojson(
        &json!({
            "type": "FeatureCollection",
            "features": [{
                "type": "Feature",
                "geometry": {"type": "LineString", "coordinates": [[-74.0, 40.7], [-74.0, 40.701]]},
                "properties": {
                    "id": "doyers", "subtype": "road", "class": "pedestrian",
                    "connectors": [{"connector_id": "a", "at": 0.0}, {"connector_id": "b", "at": 1.0}],
                    "access_restrictions": [
                        {"access_type": "denied", "when": {"heading": "backward"}},
                        {"access_type": "allowed", "when": {"heading": "forward", "mode": ["bicycle"]}}
                    ]
                }
            }]
        })
        .to_string(),
    )
    .unwrap();
    let time = parse_time("2026-10-19T08:00").unwrap();
    let cyclist = Traveller::new(Vehicle::Bicycle);
    let seg = world.net.get("doyers").unwrap();
    assert_eq!(
        cyclist
            .access_at(&world, seg, 0.5, Heading::Forward, time)
            .status,
        Status::Ok
    );
    assert_eq!(
        cyclist
            .access_at(&world, seg, 0.5, Heading::Backward, time)
            .status,
        Status::Denied
    );
    let signs = signs(&world, &cyclist, time);
    assert!(
        signs["features"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| { f["properties"]["id"] == "doyers" && f["properties"]["kind"] == "oneway" })
    );
}
