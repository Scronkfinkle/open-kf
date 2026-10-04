//! A map's navigation network: NavigationPoints (PathNode, JumpSpot,
//! PlayerStart, ...) and the ReachSpecs joining them, precomputed by the
//! editor. See Engine/ReachSpec.uc for the fields and flags.

use std::collections::HashMap;

use crate::package::{ObjectRef, Package};
use crate::properties::{Value, read_export_properties};

/// ReachSpec reachFlags (EReachSpecFlags in Engine/ReachSpec.uc).
pub const R_WALK: u32 = 1;
pub const R_FLY: u32 = 2;
pub const R_SWIM: u32 = 4;
pub const R_JUMP: u32 = 8;
pub const R_DOOR: u32 = 16;
pub const R_SPECIAL: u32 = 32;
pub const R_LADDER: u32 = 64;
pub const R_PROSCRIBED: u32 = 128;
pub const R_FORCED: u32 = 256;
pub const R_PLAYERONLY: u32 = 512;

#[derive(Debug, Clone)]
pub struct NavNode {
    pub export: usize,
    pub name: String,
    pub class: String,
    /// Unreal world units.
    pub location: [f32; 3],
}

#[derive(Debug, Clone)]
pub struct NavEdge {
    /// Indices into `NavGraph::nodes`.
    pub from: usize,
    pub to: usize,
    pub distance: f32,
    /// The largest pawn cylinder that fits.
    pub radius: f32,
    pub height: f32,
    pub flags: u32,
    pub forced: bool,
    pub pruned: bool,
}

#[derive(Debug, Default)]
pub struct NavGraph {
    pub nodes: Vec<NavNode>,
    pub edges: Vec<NavEdge>,
    /// ReachSpecs that could not be used (missing start or end).
    pub broken_specs: usize,
}

impl NavGraph {
    /// Groups of nodes joined by edges in either direction, largest first:
    /// (sizes, group of each node).
    pub fn groups(&self) -> (Vec<usize>, Vec<usize>) {
        let mut parent: Vec<usize> = (0..self.nodes.len()).collect();
        fn find(p: &mut [usize], mut x: usize) -> usize {
            while p[x] != x {
                p[x] = p[p[x]];
                x = p[x];
            }
            x
        }
        for e in &self.edges {
            let (a, b) = (find(&mut parent, e.from), find(&mut parent, e.to));
            parent[a] = b;
        }
        let roots: Vec<usize> = (0..self.nodes.len()).map(|i| find(&mut parent, i)).collect();
        let mut sizes: HashMap<usize, usize> = HashMap::new();
        for &r in &roots {
            *sizes.entry(r).or_default() += 1;
        }
        let mut order: Vec<(usize, usize)> = sizes.into_iter().collect();
        order.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        let index: HashMap<usize, usize> = order.iter().enumerate().map(|(i, (r, _))| (*r, i)).collect();
        (order.iter().map(|(_, n)| *n).collect(), roots.iter().map(|r| index[r]).collect())
    }
}

/// Reads every ReachSpec in a map and the NavigationPoints they join.
pub fn read_nav(pkg: &Package) -> NavGraph {
    let mut g = NavGraph::default();
    let mut node_of: HashMap<usize, usize> = HashMap::new();
    let mut node = |g: &mut NavGraph, export: usize| -> Option<usize> {
        if let Some(&n) = node_of.get(&export) {
            return Some(n);
        }
        let props = read_export_properties(pkg, export).ok()?;
        let location = match props.get(pkg, "Location") {
            Some(Value::Vector(v)) => *v,
            _ => return None,
        };
        g.nodes.push(NavNode {
            export,
            name: pkg.object_name(ObjectRef::Export(export)).to_string(),
            class: pkg.export_class_name(export).to_string(),
            location,
        });
        node_of.insert(export, g.nodes.len() - 1);
        Some(g.nodes.len() - 1)
    };
    for i in 0..pkg.exports.len() {
        if pkg.export_class_name(i) != "ReachSpec" {
            continue;
        }
        let Ok(props) = read_export_properties(pkg, i) else {
            g.broken_specs += 1;
            continue;
        };
        let export_of = |name: &str| match props.get(pkg, name) {
            Some(Value::Object(ObjectRef::Export(e))) => Some(*e),
            _ => None,
        };
        let int = |name: &str| match props.get(pkg, name) {
            Some(Value::Int(v)) => *v as f32,
            Some(Value::Float(v)) => *v,
            Some(Value::Byte(v)) => *v as f32,
            _ => 0.0,
        };
        let (Some(s), Some(e)) = (export_of("Start"), export_of("End")) else {
            g.broken_specs += 1;
            continue;
        };
        let (Some(from), Some(to)) = (node(&mut g, s), node(&mut g, e)) else {
            g.broken_specs += 1;
            continue;
        };
        let forced = matches!(props.get(pkg, "bForced"), Some(Value::Bool(true)));
        g.edges.push(NavEdge {
            from,
            to,
            distance: int("Distance"),
            radius: int("CollisionRadius"),
            height: int("CollisionHeight"),
            flags: int("reachFlags") as u32 | if forced { R_FORCED } else { 0 },
            forced,
            pruned: int("bPruned") != 0.0,
        });
    }
    g
}
