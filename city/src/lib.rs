//! overrules · live city: a Lower Manhattan where everyone follows Overture's transport rules,
//! evaluated by `overrules`. This file is the browser API; it returns JSON.

pub mod engine;
pub mod inspect;
pub mod signs;
pub mod sim;

use chrono::{Duration, NaiveDateTime};
use engine::{Traveller, Vehicle, World, fmt_clock, name_of, parse_time};
use geo::Point;
use overrules::{Heading, Segment};
use serde_json::{Value, json};
use sim::Sim;
use wasm_bindgen::prelude::*;

fn err(e: impl std::fmt::Display) -> JsError {
    JsError::new(&e.to_string())
}

#[wasm_bindgen]
pub struct City {
    world: World,
    clock: NaiveDateTime,
    sim: Sim,
    /// The traveller the map signs and the road panel are worked out for.
    viewer: Traveller,
}

#[wasm_bindgen]
impl City {
    /// Builds the road network from Overture GeoJSON and fills it with travellers.
    #[wasm_bindgen(constructor)]
    pub fn new(geojson: &str, start_time: &str, seed: u32) -> Result<Self, JsError> {
        let world = World::from_geojson(geojson).map_err(err)?;
        let clock = parse_time(start_time).map_err(err)?;
        let sim = Sim::new(&world, clock, u64::from(seed));
        Ok(Self {
            world,
            clock,
            sim,
            viewer: Traveller::new(Vehicle::Car),
        })
    }

    /// Size of the area: roads, junctions, rules and travellers.
    pub fn summary(&self) -> String {
        let segs = self.world.net.segments();
        json!({
            "segments": segs.len(),
            "junctions": self.world.nodes.len(),
            "agents": self.sim.agents.len(),
            "access_rules": segs.iter().map(|s| s.access_restrictions.len()).sum::<usize>(),
            "speed_rules": segs.iter().map(|s| s.speed_limits.len()).sum::<usize>(),
            "turn_rules": segs.iter().map(|s| s.prohibited_transitions.len()).sum::<usize>(),
        })
        .to_string()
    }

    pub fn set_time(&mut self, time: &str) -> Result<(), JsError> {
        self.clock = parse_time(time).map_err(err)?;
        Ok(())
    }

    /// Advances the city by `game_s` seconds of game time.
    pub fn tick(&mut self, game_s: f64) {
        let game_s = game_s.clamp(0.0, 60.0);
        self.clock += Duration::milliseconds((game_s * 1000.0) as i64);
        self.sim.step(&self.world, self.clock, game_s);
    }

    /// `[lon, lat, vehicle, bearing]` for every traveller.
    pub fn agents(&self) -> Vec<f64> {
        self.sim.positions(&self.world)
    }

    /// Clock and counters: cheap, for a few times a second.
    pub fn frame(&self) -> String {
        let c = &self.world.counters;
        json!({
            "clock": fmt_clock(self.clock),
            "time": self.clock.format("%H:%M").to_string(),
            "day": self.clock.format("%A").to_string(),
            "resolves": c.resolves.get(),
            "rules": c.rules.get(),
            "decisions": self.sim.stats.decisions,
            "refusals": self.sim.stats.refusals,
        })
        .to_string()
    }

    /// Refusals newer than `after`.
    pub fn feed(&self, after: f64) -> String {
        let items: Vec<_> = self
            .sim
            .feed
            .iter()
            .filter(|f| f.seq as f64 > after)
            .collect();
        serde_json::to_string(&items).unwrap_or_default()
    }

    /// One traveller: who, where, and why it went this way.
    pub fn agent(&self, i: usize) -> Result<String, JsError> {
        let a = self
            .sim
            .agents
            .get(i)
            .ok_or_else(|| err("no such traveller"))?;
        let e = &self.world.edges[a.edge];
        let seg = self.world.segment(e);
        Ok(json!({
            "id": i,
            "vehicle": a.traveller.vehicle,
            "road": name_of(seg),
            "segment_id": seg.id,
            "class": seg.class,
            "heading": e.heading,
            "speed_kmh": a.speed_mps * 3.6,
            "limit": a.limit,
            "decisions": a.decisions,
            "decision": a.decision,
        })
        .to_string())
    }

    /// Rules drawn on the map for the chosen vehicle, now.
    pub fn signs(&self) -> String {
        signs::signs(&self.world, &self.viewer, self.clock).to_string()
    }

    /// Which vehicle the signs and the road panel are for: `car`, `van`, `lorry`, `bicycle`
    /// or `foot`.
    pub fn set_viewer(&mut self, vehicle: &str) -> Result<(), JsError> {
        let v: Vehicle = serde_json::from_value(Value::String(vehicle.to_owned())).map_err(err)?;
        self.viewer = Traveller::new(v);
        Ok(())
    }

    /// Every rule on a road where it was clicked, for the chosen vehicle now. With
    /// `row`/`col` ≥ 0, for that cell of the what-if grid instead, claiming `claim`.
    pub fn inspect(
        &self,
        segment_id: &str,
        lon: f64,
        lat: f64,
        row: i32,
        col: i32,
        claim: &str,
    ) -> Result<String, JsError> {
        let (seg, at) = self.locate(segment_id, lon, lat)?;
        let cell = (row >= 0 && col >= 0).then_some((row as usize, col as usize));
        Ok(
            inspect::inspect(&self.world, seg, at, &self.viewer, self.clock, cell, claim)
                .to_string(),
        )
    }

    /// The what-if grid for a road where it was clicked, everyone claiming `claim` (a purpose
    /// such as `at_destination`, a status such as `as_private`, or nothing).
    pub fn what_if(
        &self,
        segment_id: &str,
        lon: f64,
        lat: f64,
        claim: &str,
    ) -> Result<String, JsError> {
        let (seg, at) = self.locate(segment_id, lon, lat)?;
        Ok(inspect::what_if(&self.world, seg, at, self.clock, claim).to_string())
    }

    /// One question about a road, for the tour. `time` empty means unknown; `heading` is
    /// `forward` or `backward`.
    #[allow(clippy::too_many_arguments)]
    pub fn ask(
        &self,
        segment_id: &str,
        lon: f64,
        lat: f64,
        row: usize,
        time: &str,
        heading: &str,
        claim: &str,
    ) -> Result<String, JsError> {
        let (seg, at) = self.locate(segment_id, lon, lat)?;
        let t = if time.is_empty() {
            None
        } else {
            Some(parse_time(time).map_err(err)?)
        };
        let h: Heading = heading.parse().map_err(err)?;
        Ok(inspect::ask(&self.world, seg, at, row, t, h, claim).to_string())
    }

    /// Part of a road between two fractions, as coordinates.
    pub fn slice(&self, segment_id: &str, from: f64, to: f64) -> Result<String, JsError> {
        let seg = self.segment(segment_id)?;
        let c: Vec<[f64; 2]> = seg
            .line()
            .slice(from, to)
            .0
            .iter()
            .map(|c| [c.x, c.y])
            .collect();
        Ok(serde_json::to_string(&c).unwrap_or_default())
    }

    /// A whole road's shape and name.
    pub fn road(&self, segment_id: &str) -> Result<String, JsError> {
        let seg = self.segment(segment_id)?;
        let c: Vec<[f64; 2]> = seg.geometry().0.iter().map(|c| [c.x, c.y]).collect();
        Ok(json!({"name": name_of(seg), "class": seg.class, "coords": c, "length_m": seg.length_m()}).to_string())
    }

    /// The rule boundary that flat-map maths would move furthest.
    pub fn worst_planar(&self) -> String {
        inspect::worst_planar(&self.world).to_string()
    }
}

impl City {
    fn segment(&self, id: &str) -> Result<&Segment, JsError> {
        self.world.net.get(id).ok_or_else(|| err("unknown road"))
    }

    /// The road and how far along it a point is.
    fn locate(&self, id: &str, lon: f64, lat: f64) -> Result<(&Segment, f64), JsError> {
        let seg = self.segment(id)?;
        Ok((seg, seg.line().project(Point::new(lon, lat)).at.fraction))
    }
}
