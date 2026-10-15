//! The living city: people, bikes, cars and trucks that move around on their own and obey
//! the rules, deciding at every junction with `overrules`.

use std::collections::VecDeque;

use chrono::NaiveDateTime;
use serde::Serialize;

use crate::engine::{By, Check, Move, Rng, Status, Traveller, Vehicle, World, fmt_clock, name_of};

/// How many of each, in `Vehicle::ALL` order.
const POPULATION: [usize; 5] = [220, 40, 25, 100, 220];
const FEED_LEN: usize = 60;

#[derive(Debug, Clone, Serialize)]
pub struct Considered {
    pub name: String,
    pub turn: &'static str,
    pub status: Status,
    pub chosen: bool,
    pub headline: String,
    pub details: Vec<String>,
    pub limit: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Decision {
    pub clock: String,
    pub place: String,
    pub options: Vec<Considered>,
}

#[derive(Debug)]
pub struct Agent {
    pub traveller: Traveller,
    pub edge: usize,
    pub pos_m: f64,
    pub speed_mps: f64,
    pub limit: String,
    pub trail: Vec<Move>,
    pub decision: Option<Decision>,
    pub decisions: u32,
}

/// A rule stopping someone from going somewhere, as it happened.
#[derive(Debug, Clone, Serialize)]
pub struct FeedItem {
    pub seq: u64,
    pub clock: String,
    pub agent: usize,
    pub emoji: &'static str,
    pub lon: f64,
    pub lat: f64,
    pub kind: Status,
    pub by: By,
    pub text: String,
    pub details: Vec<String>,
}

#[derive(Debug, Default, Clone, Copy, Serialize)]
pub struct SimStats {
    pub decisions: u64,
    pub refusals: u64,
}

pub struct Sim {
    pub agents: Vec<Agent>,
    pub feed: VecDeque<FeedItem>,
    pub stats: SimStats,
    seq: u64,
    rng: Rng,
}

impl Sim {
    pub fn new(world: &World, clock: NaiveDateTime, seed: u64) -> Self {
        let mut sim = Self {
            agents: Vec::new(),
            feed: VecDeque::new(),
            stats: SimStats::default(),
            seq: 0,
            rng: Rng::new(seed),
        };
        for (v, &n) in Vehicle::ALL.iter().zip(&POPULATION) {
            for _ in 0..n {
                if let Some(a) = sim.spawn(world, *v, clock) {
                    sim.agents.push(a);
                }
            }
        }
        sim
    }

    fn spawn(&mut self, world: &World, v: Vehicle, clock: NaiveDateTime) -> Option<Agent> {
        let traveller = Traveller::new(v);
        for _ in 0..200 {
            let edge = self.rng.below(world.edges.len());
            let e = &world.edges[edge];
            let class = world.segment(e).class.as_deref().unwrap_or("");
            // Pedestrians mostly on footways, so they don't all walk down the middle of roads.
            if v == Vehicle::Foot && class != "footway" && self.rng.unit() < 0.8 {
                continue;
            }
            let ev = traveller.edge_eval(world, e, clock, &[0.5]);
            if ev.status != Status::Ok {
                continue;
            }
            return Some(Agent {
                speed_mps: ev.travel_kmh / 3.6,
                limit: ev.limit_source,
                traveller,
                edge,
                pos_m: self.rng.unit() * e.length_m,
                trail: vec![Move {
                    edge,
                    depart: clock,
                }],
                decision: None,
                decisions: 0,
            });
        }
        None
    }

    pub fn step(&mut self, world: &World, clock: NaiveDateTime, dt: f64) {
        for i in 0..self.agents.len() {
            self.agents[i].pos_m += self.agents[i].speed_mps * dt;
            let mut hops = 0;
            while self.agents[i].pos_m >= world.edges[self.agents[i].edge].length_m && hops < 8 {
                hops += 1;
                let over = self.agents[i].pos_m - world.edges[self.agents[i].edge].length_m;
                if !self.advance(world, i, clock) {
                    let v = self.agents[i].traveller.vehicle;
                    if let Some(a) = self.spawn(world, v, clock) {
                        self.agents[i] = a;
                    }
                    break;
                }
                self.agents[i].pos_m = over;
            }
        }
    }

    /// Picks the next edge at the end of agent `i`'s edge. False if stuck.
    fn advance(&mut self, world: &World, i: usize, clock: NaiveDateTime) -> bool {
        let a = &self.agents[i];
        let here = world.edges[a.edge].to;
        let prev = Some(a.edge);
        let opts: Vec<(usize, Check, &'static str)> = world.out[here]
            .iter()
            .map(|&e| {
                (
                    e,
                    a.traveller.check_move(world, &a.trail, e, clock),
                    world.turn_name(prev, e),
                )
            })
            .filter(|(_, c, _)| !c.off_limits)
            .collect();
        self.stats.decisions += 1;

        let weight = |turn: &str| match turn {
            "straight" => 4.0,
            "bear left" | "bear right" => 2.0,
            "u-turn" => 0.0,
            _ => 1.0,
        };
        let legal: Vec<usize> = (0..opts.len())
            .filter(|&k| opts[k].1.status.is_legal())
            .collect();
        let total: f64 = legal.iter().map(|&k| weight(opts[k].2)).sum();
        let pick = if total > 0.0 {
            let mut r = self.rng.unit() * total;
            legal.iter().copied().find(|&k| {
                r -= weight(opts[k].2);
                r <= 0.0
            })
        } else {
            legal.first().copied()
        };
        let Some(pick) = pick else {
            return false;
        };

        let emoji = a.traveller.vehicle.emoji();
        let pos = world.node_pos(here);
        let place = world.place(here);
        for (k, (e, c, _)) in opts.iter().enumerate() {
            if k == pick || c.status.is_legal() {
                continue;
            }
            self.stats.refusals += 1;
            let details = c.details();
            // Plain one-ways are everywhere; keep the feed for rules with more to them.
            let interesting = c.status == Status::TurnBanned
                || details.iter().any(|d| {
                    ["during", "vehicle", "using", "recognized", "mode"]
                        .iter()
                        .any(|k| d.contains(k))
                });
            if !interesting && self.rng.unit() > 0.05 {
                continue;
            }
            self.seq += 1;
            let name = name_of(world.segment(&world.edges[*e]));
            self.feed.push_back(FeedItem {
                seq: self.seq,
                clock: fmt_clock(clock),
                agent: i,
                emoji,
                lon: pos[0],
                lat: pos[1],
                kind: c.status,
                by: c.main_reason().map_or(By::Overrules, |r| r.by),
                text: format!("skipped {name}: {}", c.headline()),
                details,
            });
            if self.feed.len() > FEED_LEN {
                self.feed.pop_front();
            }
        }

        let options = opts
            .iter()
            .enumerate()
            .map(|(k, (e, c, turn))| Considered {
                name: name_of(world.segment(&world.edges[*e])),
                turn,
                status: c.status,
                chosen: k == pick,
                headline: c.headline(),
                details: c.details(),
                limit: c.limit_source.clone(),
            })
            .collect();
        let (edge, check) = (opts[pick].0, &opts[pick].1);
        let a = &mut self.agents[i];
        a.decision = Some(Decision {
            clock: fmt_clock(clock),
            place,
            options,
        });
        a.decisions += 1;
        a.speed_mps = check.travel_kmh / 3.6;
        a.limit = check.limit_source.clone();
        a.edge = edge;
        a.trail.push(Move {
            edge,
            depart: clock,
        });
        if a.trail.len() > 6 {
            a.trail.remove(0);
        }
        true
    }

    /// `[lon, lat, vehicle, bearing]` per agent.
    pub fn positions(&self, world: &World) -> Vec<f64> {
        let mut out = Vec::with_capacity(self.agents.len() * 4);
        for a in &self.agents {
            let (lon, lat, b) = world.point_on(a.edge, a.pos_m);
            out.extend([lon, lat, a.traveller.vehicle.index() as f64, b]);
        }
        out
    }
}
