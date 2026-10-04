//! Ragdolls from KF's Karma files (`KarmaData/*.ka`, see DESIGN.md "Gore and
//! ragdolls").
//!
//! On death, each ragdoll part (one per listed bone) becomes an avian dynamic
//! body, placed from the current animated pose, joined to its parent part
//! with KF's joint limits. Every frame the bodies are read back into bone
//! transforms and the mesh is skinned from them. Bones that are not parts
//! (head, hands, feet) keep their offset from the part that carries them.

use avian3d::prelude::*;
use bevy::prelude::*;
use ue_assets::karma::{self, JointKind, Shape, TwistType};

use crate::coords::{self, SCALE};
use crate::skinned::SkinnedModel;

/// Body masses in the file are ~0.1 each; only their ratios matter, so they
/// are scaled to a convenient range for the solver.
const MASS_UNIT: f32 = 100.0;
/// Smallest part mass, in file units (a typical part is ~0.1). The Clot's
/// chr_spine3 has 3.14e-6 (a placeholder value): a nearly massless link
/// between two heavy parts cannot pass corrections along, so the solver kept
/// moving spine3 and never pulled the ribcage, and that joint came apart.
pub const MIN_PART_MASS: f32 = 0.05;
/// A corpse goes to sleep (stops simulating) once every part moves slower
/// than these for half a second (m/s and rad/s; avian's defaults are 0.15).
/// With the near-locked limits from the file (locked twist, the spine's
/// 0.6 degree cones) the solver chatters: parts kept turning at ~0.5 rad/s
/// forever, so the default never let a corpse sleep and it jiggled. Karma
/// likewise put resting bodies to sleep. Values chosen from the speed log.
const SLEEP_LINEAR: f32 = 0.3;
const SLEEP_ANGULAR: f32 = 1.0;
/// See `joint_damping`.
const JOINT_ANGULAR_DAMPING: f32 = 1.0;
/// KarmaParams KMaxAngularSpeed (rad/s) and KMaxSpeed (Unreal units/s).
pub const MAX_ANGULAR_SPEED: f32 = 10.0;
pub const MAX_SPEED: f32 = 2500.0;
/// Inertia radius for parts with no collision shape (Unreal units).
const NO_SHAPE_RADIUS: f32 = 4.0;

/// One ragdoll, matched to a skeletal mesh's bones.
pub struct RagdollDef {
    pub name: String,
    /// Mesh units per file unit (1 / the asset's scale).
    unit: f32,
    parts: Vec<PartDef>,
    joints: Vec<JointDef>,
    /// The part with no parent (the pelvis).
    pub root: usize,
    /// For every mesh bone: the part that carries it (itself if it is a part,
    /// else its nearest ancestor part, else the root part).
    carrier: Vec<usize>,
}

struct PartDef {
    bone: usize,
    name: String,
    mass: f32,
    /// File units, bone frame.
    mass_offset: Vec3,
    primitives: Vec<karma::Primitive>,
}

struct JointDef {
    child: usize,
    parent: usize,
    kind: JointKind,
    pos1: Vec3,
    pos2: Vec3,
    primary1: Vec3,
    primary2: Vec3,
    orthogonal1: Vec3,
    orthogonal2: Vec3,
}

fn v3(a: [f32; 3]) -> Vec3 {
    Vec3::from_array(a)
}

impl RagdollDef {
    /// Matches the ragdoll's parts to the mesh's bones (by name, ignoring
    /// case) and checks the joint anchors against the bind pose.
    pub fn new(ka: &karma::Ragdoll, model: &SkinnedModel) -> Result<(Self, String), String> {
        let bones = &model.mesh.bones;
        let find = |name: &str| bones.iter().position(|b| b.name.eq_ignore_ascii_case(name));
        let mut parts = Vec::new();
        for p in &ka.parts {
            let bone = find(&p.bone).ok_or_else(|| format!("part {} has no bone in the mesh", p.bone))?;
            parts.push(PartDef {
                bone,
                name: p.bone.clone(),
                mass: p.mass,
                mass_offset: v3(p.mass_offset),
                primitives: p.primitives.clone(),
            });
        }
        let part_of = |name: &str| parts.iter().position(|p| p.name.eq_ignore_ascii_case(name));
        let mut joints = Vec::new();
        for j in &ka.joints {
            joints.push(JointDef {
                child: part_of(&j.part1).ok_or_else(|| format!("joint part {} is not a part", j.part1))?,
                parent: part_of(&j.part2).ok_or_else(|| format!("joint part {} is not a part", j.part2))?,
                kind: j.kind.clone(),
                pos1: v3(j.pos1),
                pos2: v3(j.pos2),
                primary1: v3(j.primary1),
                primary2: v3(j.primary2),
                orthogonal1: v3(j.orthogonal1),
                orthogonal2: v3(j.orthogonal2),
            });
        }
        let root = ka
            .parts
            .iter()
            .position(|p| p.parent.is_none())
            .ok_or("ragdoll has no root part")?;
        let carrier = (0..bones.len())
            .map(|mut b| loop {
                if let Some(i) = parts.iter().position(|p| p.bone == b) {
                    break i;
                }
                if b == 0 {
                    break root;
                }
                b = bones[b].parent;
            })
            .collect();
        let def = RagdollDef {
            name: ka.name.clone(),
            unit: 1.0 / ka.scale,
            parts,
            joints,
            root,
            carrier,
        };
        let report = def.check_bind_pose(&model.bind_pose());
        Ok((def, report))
    }

    /// In the bind pose both ends of each joint should be at the same point,
    /// and each joint's primary axes should agree. Reports the worst cases.
    fn check_bind_pose(&self, bind: &[(Quat, Vec3)]) -> String {
        let (mut worst_gap, mut gap_at) = (0.0f32, "");
        let (mut worst_angle, mut angle_at) = (0.0f32, "");
        let mut orthos = Vec::new();
        for j in &self.joints {
            let (cq, cp) = bind[self.parts[j.child].bone];
            let (pq, pp) = bind[self.parts[j.parent].bone];
            let from_child = cp + cq * (j.pos1 * self.unit);
            let from_parent = pp + pq * (j.pos2 * self.unit);
            let gap = from_child.distance(from_parent);
            if gap > worst_gap {
                (worst_gap, gap_at) = (gap, &self.parts[j.child].name);
            }
            let angle = (cq * j.primary1).angle_between(pq * j.primary2).to_degrees();
            if angle > worst_angle {
                (worst_angle, angle_at) = (angle, &self.parts[j.child].name);
            }
            let ortho = (cq * j.orthogonal1).angle_between(pq * j.orthogonal2).to_degrees();
            orthos.push(format!("{}:{ortho:.0}", self.parts[j.child].name));
        }
        let raised: Vec<String> = self
            .parts
            .iter()
            .filter(|p| p.mass < MIN_PART_MASS)
            .map(|p| format!("{}:{}", p.name, p.mass))
            .collect();
        format!(
            "ragdoll={} parts={} joints={} masses_raised_to_minimum=[{}] worst_anchor_gap_mesh_units={worst_gap:.2} at={gap_at} worst_primary_axis_angle_deg={worst_angle:.1} at={angle_at} orthogonal_axis_angles_deg=[{}]",
            self.name,
            self.parts.len(),
            self.joints.len(),
            raised.join(" "),
            orthos.join(" ")
        )
    }

    /// Hinge angles (as avian measures them, radians) for a pose, with each
    /// hinge's file limits, for checking the sign convention.
    pub fn hinge_angles(&self, pose: &[(Quat, Vec3)]) -> Vec<(String, f32, f32, f32)> {
        let frame = MeshFrame::identity();
        self.joints
            .iter()
            .filter_map(|j| {
                let JointKind::Hinge { low, high, .. } = j.kind else {
                    return None;
                };
                let (r1, _) = frame.body_from_bone(pose[self.parts[j.child].bone]);
                let (r2, _) = frame.body_from_bone(pose[self.parts[j.parent].bone]);
                let b1 = hinge_basis(&frame, j.primary1, j.orthogonal1);
                let b2 = hinge_basis(&frame, j.primary2, j.orthogonal2);
                let a1 = r1 * b1 * Vec3::X;
                let n1 = r1 * b1 * Vec3::X.any_orthonormal_vector();
                let n2 = r2 * b2 * Vec3::X.any_orthonormal_vector();
                let phi = n1.cross(n2).dot(a1).atan2(n1.dot(n2));
                Some((self.parts[j.child].name.clone(), phi, low, high))
            })
            .collect()
    }
}

/// Joint basis for a hinge: local X = hinge (primary) axis, and avian's
/// reference (X.any_orthonormal_vector() = Y) = the orthogonal axis.
fn hinge_basis(frame: &MeshFrame, primary: Vec3, orthogonal: Vec3) -> Quat {
    let p = frame.local_dir(primary);
    let o = (frame.local_dir(orthogonal) - p * p.dot(frame.local_dir(orthogonal))).normalize_or_zero();
    Quat::from_mat3(&Mat3::from_cols(p, o, p.cross(o))).normalize()
}

/// Joint basis for a ball joint. Avian measures swing and twist around
/// `twist_axis.any_orthonormal_vector()` (Y for twist_axis X) and twist
/// from twist_axis: so local Y = the primary (cone) axis and local X = the
/// orthogonal axis.
fn ball_basis(frame: &MeshFrame, primary: Vec3, orthogonal: Vec3) -> Quat {
    let p = frame.local_dir(primary);
    let o = (frame.local_dir(orthogonal) - p * p.dot(frame.local_dir(orthogonal))).normalize_or_zero();
    Quat::from_mat3(&Mat3::from_cols(o, p, o.cross(p))).normalize()
}

/// The mapping from a zed's mesh space (Unreal mesh units) to Bevy world
/// space: world = k * Q * p + b, with Q orthogonal (it includes the
/// Unreal-to-Bevy handedness flip) and k a uniform scale.
#[derive(Clone, Copy, Debug)]
pub struct MeshFrame {
    q: Mat3,
    k: f32,
    b: Vec3,
}

impl MeshFrame {
    /// `rot_origin`: the mesh's RotOrigin as an Unreal matrix. Matches
    /// zed.rs mesh_to_actor followed by the actor transform.
    pub fn new(rot_origin: Mat3, mesh_scale: f32, mesh_origin: Vec3, draw_scale: f32, pre_pivot: Vec3, actor: &Transform) -> Self {
        let c = coords::c();
        let rt = Mat3::from_quat(actor.rotation);
        MeshFrame {
            q: rt * c * rot_origin,
            k: SCALE * draw_scale * mesh_scale,
            b: actor.translation + rt * c * (SCALE * (pre_pivot - draw_scale * (rot_origin * (mesh_origin * mesh_scale)))),
        }
    }

    fn identity() -> Self {
        MeshFrame {
            q: coords::c(),
            k: SCALE,
            b: Vec3::ZERO,
        }
    }

    /// A bone transform in mesh space -> a body transform in Bevy space.
    /// The body's local axes are the bone's axes mapped through C.
    fn body_from_bone(&self, (bq, bp): (Quat, Vec3)) -> (Quat, Vec3) {
        let r = self.q * Mat3::from_quat(bq) * coords::c().transpose();
        (Quat::from_mat3(&r).normalize(), self.k * (self.q * bp) + self.b)
    }

    /// Inverse of `body_from_bone`.
    fn bone_from_body(&self, rot: Quat, pos: Vec3) -> (Quat, Vec3) {
        let m = self.q.transpose() * Mat3::from_quat(rot) * coords::c();
        (Quat::from_mat3(&m).normalize(), self.q.transpose() * (pos - self.b) / self.k)
    }

    /// A point in a bone's frame (mesh units) -> the body's local frame (metres).
    fn local(&self, v: Vec3) -> Vec3 {
        self.k * (coords::c() * v)
    }

    fn local_dir(&self, v: Vec3) -> Vec3 {
        (coords::c() * v).normalize_or_zero()
    }
}

/// Marks a ragdoll body (one part).
#[derive(Component)]
pub struct RagdollBody;

/// A live ragdoll, on the dead zed's entity.
#[derive(Component)]
pub struct RagdollState {
    pub bodies: Vec<Entity>,
    pub joints: Vec<Entity>,
    frame: MeshFrame,
    /// Each bone's transform relative to its carrier part at death.
    offsets: Vec<(Quat, Vec3)>,
    /// Per joint: child body, parent body, anchor in each body (metres), for
    /// checking how far joints pull apart.
    anchors: Vec<(usize, usize, Vec3, Vec3)>,
    /// Per ball joint: child, parent, joint bases in each body, swing and
    /// twist limits, and the largest swing and twist seen so far (radians).
    pub ball_joints: Vec<BallJointWatch>,
    /// Seconds since the ragdoll started, and when it came to rest (logged once).
    pub age: f32,
    pub rested_at: Option<f32>,
}

pub struct BallJointWatch {
    child: usize,
    parent: usize,
    basis1: Quat,
    basis2: Quat,
    pub cone: f32,
    pub twist: f32,
    pub max_swing: f32,
    pub max_twist: f32,
    pub twist_now: f32,
}

/// Swing (cone axis tilt) and twist (about the cone axis) of a ball joint,
/// radians. The cone axis is local Y of the joint bases (see `ball_basis`).
fn swing_twist(r1: Quat, b1: Quat, r2: Quat, b2: Quat) -> (f32, f32) {
    let rel = (r2 * b2).inverse() * (r1 * b1);
    let swing_q = Quat::from_rotation_arc(Vec3::Y, rel * Vec3::Y);
    let twist_q = swing_q.inverse() * rel;
    let (axis, angle) = twist_q.to_axis_angle();
    let angle = if angle > std::f32::consts::PI { std::f32::consts::TAU - angle } else { angle };
    let twist = if axis.dot(Vec3::Y).abs() > 0.5 { angle } else { 0.0 };
    ((rel * Vec3::Y).angle_between(Vec3::Y), twist)
}

/// Start motion for a ragdoll, Bevy space.
pub struct Launch {
    pub velocity: Vec3,
    pub angular_velocity: Vec3,
}

/// Spawns the bodies and joints of a ragdoll from a pose (mesh space).
pub fn spawn(
    commands: &mut Commands,
    def: &RagdollDef,
    pose: &[(Quat, Vec3)],
    frame: MeshFrame,
    launch: &Launch,
    zed_id: usize,
) -> RagdollState {
    let length = frame.k * def.unit; // metres per file unit
    let layers = CollisionLayers::new(crate::collision::GameLayer::Ragdoll, [crate::collision::GameLayer::World]);
    let root_pos = frame.body_from_bone(pose[def.parts[def.root].bone]).1;
    let mut bodies = Vec::new();
    for part in &def.parts {
        let (rot, pos) = frame.body_from_bone(pose[part.bone]);
        let shapes: Vec<(Vec3, Quat, Collider)> = part
            .primitives
            .iter()
            .map(|prim| {
                let tm = &prim.transform;
                let at = frame.local(v3(tm.position()) * def.unit);
                match prim.shape {
                    Shape::Sphere { radius } => (at, Quat::IDENTITY, Collider::sphere(radius * length)),
                    // Capsules run along the primitive's local Z; avian's along Y.
                    Shape::Sphyl { radius, height } => (
                        at,
                        Quat::from_rotation_arc(Vec3::Y, frame.local_dir(v3(tm.axis(2)))),
                        Collider::capsule(radius * length, height * length),
                    ),
                    Shape::Box { dims } => {
                        let x = frame.local_dir(v3(tm.axis(0)));
                        let y = frame.local_dir(v3(tm.axis(1)));
                        let r = Quat::from_mat3(&Mat3::from_cols(x, y, x.cross(y))).normalize();
                        (at, r, Collider::cuboid(dims[0] * length, dims[1] * length, dims[2] * length))
                    }
                }
            })
            .collect();
        // The file's inertia values are implausible (the Clot's forearm has a
        // radius of gyration of 0.2 units; the collars have zero), so inertia
        // and centre of mass come from the collision shapes, scaled to the
        // file's mass. Parts without shapes ("dynamics_only": the collars,
        // the Clot's spine3) have no collider, as in KF, and the inertia of a
        // solid sphere of NO_SHAPE_RADIUS.
        let mass = part.mass.max(MIN_PART_MASS) * MASS_UNIT;
        let collider = (!shapes.is_empty()).then(|| Collider::compound(shapes));
        let (inertia, center_of_mass) = match &collider {
            Some(c) => {
                let props = c.mass_properties(1.0);
                (
                    AngularInertia::new_with_local_frame(
                        props.principal_angular_inertia * (mass / props.mass.max(1e-9)),
                        props.local_inertial_frame,
                    ),
                    props.center_of_mass,
                )
            }
            None => {
                let r = NO_SHAPE_RADIUS * SCALE;
                (AngularInertia::new(Vec3::splat(0.4 * mass * r * r)), frame.local(part.mass_offset * def.unit))
            }
        };
        let velocity = launch.velocity + launch.angular_velocity.cross(pos - root_pos);
        let mut e = commands.spawn((
            RigidBody::Dynamic,
            Transform::from_translation(pos).with_rotation(rot),
            layers,
            Mass(mass),
            inertia,
            CenterOfMass(center_of_mass),
            (NoAutoMass, NoAutoAngularInertia, NoAutoCenterOfMass),
            // KFMonster.PawnKParams.
            (LinearDamping(0.15), AngularDamping(0.05), Friction::new(1.3), Restitution::new(0.2)),
            LinearVelocity(velocity),
            AngularVelocity(launch.angular_velocity),
            RagdollBody,
            SleepThreshold {
                linear: SLEEP_LINEAR,
                angular: SLEEP_ANGULAR,
            },
            Name::new(format!("ragdoll {zed_id} {}", part.name)),
        ));
        if let Some(c) = collider {
            e.insert(c);
        }
        bodies.push(e.id());
    }

    let mut joints = Vec::new();
    let mut anchors = Vec::new();
    let mut ball_joints = Vec::new();
    for j in &def.joints {
        let (e1, e2) = (bodies[j.child], bodies[j.parent]);
        let a1 = frame.local(j.pos1 * def.unit);
        let a2 = frame.local(j.pos2 * def.unit);
        anchors.push((j.child, j.parent, a1, a2));
        let id = match j.kind {
            JointKind::Skeletal {
                cone_x,
                cone_y,
                twist,
                twist_type,
                ..
            } => {
                // TWIST_TYPE: Free = no twist limit, Limited = +-twist,
                // Locked = no twist (see karma::TwistType).
                let twist = match twist_type {
                    TwistType::Free => std::f32::consts::PI,
                    TwistType::Limited => twist,
                    TwistType::Locked => 0.0,
                };
                // Avian's swing limit is a circle; KF's cone is an ellipse.
                let cone = cone_x.max(cone_y);
                let (basis1, basis2) = (ball_basis(&frame, j.primary1, j.orthogonal1), ball_basis(&frame, j.primary2, j.orthogonal2));
                ball_joints.push(BallJointWatch {
                    child: j.child,
                    parent: j.parent,
                    basis1,
                    basis2,
                    cone: cone_x.max(cone_y),
                    twist,
                    max_swing: 0.0,
                    max_twist: 0.0,
                    twist_now: 0.0,
                });
                let f1 = Isometry3d::new(a1, basis1);
                let f2 = Isometry3d::new(a2, basis2);
                let mut joint = SphericalJoint::new(e1, e2)
                    .with_local_frame1(f1)
                    .with_local_frame2(f2)
                    .with_twist_axis(Vec3::X);
                joint.swing_limit = Some(AngleLimit::new(-cone, cone));
                joint.twist_limit = Some(AngleLimit::new(-twist, twist));


                commands.spawn((joint, joint_damping())).id()
            }
            JointKind::Hinge { low, high, limited } => {
                let mut joint = RevoluteJoint::new(e1, e2)
                    .with_local_frame1(Isometry3d::new(a1, hinge_basis(&frame, j.primary1, j.orthogonal1)))
                    .with_local_frame2(Isometry3d::new(a2, hinge_basis(&frame, j.primary2, j.orthogonal2)));
                joint.hinge_axis = Vec3::X;
                if limited {
                    joint.angle_limit = Some(hinge_limit(low, high));
                }


                commands.spawn((joint, joint_damping())).id()
            }
        };
        joints.push(id);
    }

    let offsets = (0..pose.len())
        .map(|b| {
            let (cq, cp) = pose[def.parts[def.carrier[b]].bone];
            let (bq, bp) = pose[b];
            let inv = cq.inverse();
            (inv * bq, inv * (bp - cp))
        })
        .collect();
    RagdollState {
        bodies,
        joints,
        frame,
        offsets,
        anchors,
        ball_joints,
        age: 0.0,
        rested_at: None,
    }
}

/// Damping on every joint. The file gives CONE_DAMPING and TWIST_DAMPING 1
/// (Karma units, not known); avian's angular joint damping slows the
/// relative rotation of the two bodies. Value chosen by the limit and
/// speed logs.
fn joint_damping() -> JointDamping {
    JointDamping {
        linear: 0.0,
        angular: JOINT_ANGULAR_DAMPING,
    }
}

/// KF's hinge limits in avian's angle convention. They apply as written:
/// over the Clot's walk cycle, knee and elbow angles measured the way avian
/// measures them fall inside the file's limits, not the mirrored ones (load
/// log `ragdoll_hinge_check`).
fn hinge_limit(low: f32, high: f32) -> AngleLimit {
    AngleLimit::new(low, high)
}

impl RagdollDef {
    pub fn part_name(&self, i: usize) -> &str {
        &self.parts[i].name
    }
}

impl RagdollState {
    /// Updates the largest swing and twist seen on each ball joint.
    pub fn watch_limits(&mut self, body: impl Fn(Entity) -> Option<Transform>) {
        for w in &mut self.ball_joints {
            let (Some(t1), Some(t2)) = (body(self.bodies[w.child]), body(self.bodies[w.parent])) else {
                continue;
            };
            let (swing, twist) = swing_twist(t1.rotation, w.basis1, t2.rotation, w.basis2);
            w.max_swing = w.max_swing.max(swing);
            w.max_twist = w.max_twist.max(twist);
            w.twist_now = twist;
        }
    }

    /// Current twist (degrees) of the named parts' joints.
    pub fn twist_now(&self, def: &RagdollDef) -> String {
        self.ball_joints
            .iter()
            .filter(|w| ["chr_neck", "chr_spine2", "chr_lthigh"].contains(&def.parts[w.child].name.as_str()))
            .map(|w| format!("{}={:.0}", &def.parts[w.child].name[4..], w.twist_now.to_degrees()))
            .collect::<Vec<_>>()
            .join(",")
    }

    /// "child:swing/limit,twist/limit" in degrees for each ball joint.
    pub fn limit_report(&self, def: &RagdollDef) -> String {
        self.ball_joints
            .iter()
            .map(|w| {
                format!(
                    "{}:swing={:.0}/{:.0},twist={:.0}/{:.0}",
                    def.parts[w.child].name,
                    w.max_swing.to_degrees(),
                    w.cone.to_degrees(),
                    w.max_twist.to_degrees(),
                    w.twist.to_degrees()
                )
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Every joint's anchor gap (Unreal units), as "child:gap".
    pub fn joint_gaps(&self, def: &RagdollDef, body: impl Fn(Entity) -> Option<Transform>) -> Vec<String> {
        self.anchors
            .iter()
            .filter_map(|&(c, p, a1, a2)| {
                let (t1, t2) = (body(self.bodies[c])?, body(self.bodies[p])?);
                let gap = (t1.translation + t1.rotation * a1).distance(t2.translation + t2.rotation * a2) / SCALE;
                Some(format!("{}->{}:{gap:.1}", def.parts[c].name, def.parts[p].name))
            })
            .collect()
    }

    /// The joint whose two anchor points are furthest apart (Unreal units).
    pub fn worst_joint_gap(&self, def: &RagdollDef, body: impl Fn(Entity) -> Option<Transform>) -> Option<(String, f32)> {
        let mut worst: Option<(String, f32)> = None;
        for &(c, p, a1, a2) in &self.anchors {
            let (t1, t2) = (body(self.bodies[c])?, body(self.bodies[p])?);
            let gap = (t1.translation + t1.rotation * a1).distance(t2.translation + t2.rotation * a2) / SCALE;
            if worst.as_ref().is_none_or(|w| gap > w.1) {
                worst = Some((def.parts[c].name.clone(), gap));
            }
        }
        worst
    }

    /// Bone transforms in mesh space, read back from the bodies.
    pub fn pose(&self, def: &RagdollDef, body: impl Fn(Entity) -> Option<Transform>) -> Option<Vec<(Quat, Vec3)>> {
        let parts: Vec<(Quat, Vec3)> = self
            .bodies
            .iter()
            .map(|&e| body(e).map(|t| self.frame.bone_from_body(t.rotation, t.translation)))
            .collect::<Option<_>>()?;
        Some(
            self.offsets
                .iter()
                .enumerate()
                .map(|(b, &(oq, op))| {
                    let (cq, cp) = parts[def.carrier[b]];
                    (cq * oq, cp + cq * op)
                })
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two dynamic bodies joined by a ball joint, falling for a second:
    /// returns the distance between the joint's two anchor points.
    fn joint_gap_after_fall(ragdoll_components: bool, second_has_collider: bool) -> f32 {
        use bevy::time::TimeUpdateStrategy;
        use std::time::Duration;
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default(), PhysicsPlugins::default(), TransformPlugin, bevy::mesh::MeshPlugin));
        app.insert_resource(Gravity(Vec3::NEG_Y * 19.0));
        app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(1.0 / 64.0)));
        app.finish();
        let mut body = |at: Vec3, collider: bool| {
            let mut e = app.world_mut().spawn((RigidBody::Dynamic, Transform::from_translation(at), Mass(10.0)));
            if collider {
                e.insert(Collider::sphere(0.1));
            }
            if ragdoll_components {
                e.insert((
                    AngularInertia::new(Vec3::splat(0.04)),
                    CenterOfMass(Vec3::ZERO),
                    (NoAutoMass, NoAutoAngularInertia, NoAutoCenterOfMass),
                    CollisionLayers::new(crate::collision::GameLayer::Ragdoll, [crate::collision::GameLayer::World]),
                ));
            }
            e.id()
        };
        let a = body(Vec3::ZERO, true);
        let b = body(Vec3::new(0.5, 0.0, 0.0), second_has_collider);
        app.world_mut().spawn(
            SphericalJoint::new(a, b)
                .with_local_frame1(Isometry3d::from_translation(Vec3::new(0.25, 0.0, 0.0)))
                .with_local_frame2(Isometry3d::from_translation(Vec3::new(-0.25, 0.0, 0.0))),
        );
        // Push them apart sideways; the joint should hold.
        app.world_mut().entity_mut(a).insert(LinearVelocity(Vec3::new(-3.0, 0.0, 0.0)));
        for _ in 0..64 {
            app.update();
        }
        let ta = *app.world().get::<Transform>(a).unwrap();
        let tb = *app.world().get::<Transform>(b).unwrap();
        (ta.translation + ta.rotation * Vec3::new(0.25, 0.0, 0.0)).distance(tb.translation + tb.rotation * Vec3::new(-0.25, 0.0, 0.0))
    }

    /// One body with `n` others jointed to it, all pushed apart: returns the
    /// largest anchor gap after a second.
    fn star_gap(n: usize) -> f32 {
        use bevy::time::TimeUpdateStrategy;
        use std::time::Duration;
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default(), PhysicsPlugins::default(), TransformPlugin, bevy::mesh::MeshPlugin));
        app.insert_resource(Gravity(Vec3::NEG_Y * 19.0));
        app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(1.0 / 64.0)));
        app.finish();
        let hub = app.world_mut().spawn((RigidBody::Dynamic, Transform::IDENTITY, Collider::sphere(0.1), Mass(10.0))).id();
        let mut joints = Vec::new();
        for i in 0..n {
            let dir = Quat::from_rotation_y(i as f32 * 1.3) * Vec3::X;
            let e = app
                .world_mut()
                .spawn((
                    RigidBody::Dynamic,
                    Transform::from_translation(dir * 0.5),
                    Collider::sphere(0.1),
                    Mass(10.0),
                    LinearVelocity(dir * 3.0),
                ))
                .id();
            app.world_mut().spawn(
                SphericalJoint::new(e, hub)
                    .with_local_frame1(Isometry3d::from_translation(-dir * 0.25))
                    .with_local_frame2(Isometry3d::from_translation(dir * 0.25)),
            );
            joints.push((e, dir));
        }
        for _ in 0..64 {
            app.update();
        }
        let th = *app.world().get::<Transform>(hub).unwrap();
        joints
            .iter()
            .map(|&(e, dir)| {
                let te = *app.world().get::<Transform>(e).unwrap();
                (te.translation + te.rotation * (-dir * 0.25)).distance(th.translation + th.rotation * (dir * 0.25))
            })
            .fold(0.0, f32::max)
    }

    /// A static parent and a dynamic child joined by a ball joint built the
    /// way `spawn` builds them (primary axis = Bevy X for both), with swing
    /// limit `cone` and twist limit `twist`. The child starts spinning about
    /// `spin_axis`; returns (largest twist about X, largest swing away from X)
    /// reached in a second, in radians.
    fn limited_rotation(spin_axis: Vec3, cone: f32, twist: f32) -> (f32, f32) {
        use bevy::time::TimeUpdateStrategy;
        use std::time::Duration;
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default(), PhysicsPlugins::default(), TransformPlugin, bevy::mesh::MeshPlugin));
        app.insert_resource(Gravity(Vec3::ZERO));
        app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(1.0 / 64.0)));
        app.finish();
        let parent = app.world_mut().spawn((RigidBody::Static, Transform::IDENTITY)).id();
        let child = app
            .world_mut()
            .spawn((
                RigidBody::Dynamic,
                Transform::from_xyz(0.5, 0.0, 0.0),
                Collider::capsule(0.05, 0.4),
                Mass(10.0),
                AngularVelocity(spin_axis * 6.0),
            ))
            .id();
        // Primary axis X, orthogonal axis Y, in both bodies (unit frame).
        let frame = MeshFrame {
            q: Mat3::IDENTITY,
            k: 1.0,
            b: Vec3::ZERO,
        };
        // local_dir applies C; pass Unreal vectors that map to Bevy X and Y.
        let primary = coords::c().transpose() * Vec3::X;
        let orth = coords::c().transpose() * Vec3::Y;
        let basis = ball_basis(&frame, primary, orth);
        let mut joint = SphericalJoint::new(child, parent)
            .with_local_frame1(Isometry3d::new(Vec3::new(-0.5, 0.0, 0.0), basis))
            .with_local_frame2(Isometry3d::new(Vec3::ZERO, basis))
            .with_twist_axis(Vec3::X);
        joint.swing_limit = Some(AngleLimit::new(-cone, cone));
        joint.twist_limit = Some(AngleLimit::new(-twist, twist));
        app.world_mut().spawn(joint);
        let (mut max_twist, mut max_swing) = (0.0f32, 0.0f32);
        for _ in 0..64 {
            app.update();
            let r = app.world().get::<Transform>(child).unwrap().rotation;
            // Swing: how far the child's X axis turned away from X.
            max_swing = max_swing.max((r * Vec3::X).angle_between(Vec3::X));
            // Twist: rotation of the child's Y about its own X, after removing swing.
            let swing = Quat::from_rotation_arc(Vec3::X, r * Vec3::X);
            let twist_q = swing.inverse() * r;
            let (axis, angle) = twist_q.to_axis_angle();
            max_twist = max_twist.max(if axis.dot(Vec3::X).abs() > 0.5 { angle.abs() } else { 0.0 });
        }
        (max_twist, max_swing)
    }

    #[test]
    fn ball_joint_limits_act_on_the_right_axes() {
        // Spin about the bone (primary) axis: twist limit 0.15 should stop it.
        let (twist, _) = limited_rotation(Vec3::X, 0.5, 0.15);
        // Swing sideways: cone 0.3 should stop it.
        let (_, swing) = limited_rotation(Vec3::Z, 0.3, 1.0);
        eprintln!("twist reached {twist:.2} rad (limit 0.15), swing reached {swing:.2} rad (limit 0.30)");
        assert!(twist < 0.25, "twist {twist}");
        assert!(swing < 0.4, "swing {swing}");
    }

    #[test]
    fn avian_star_joints_hold() {
        let gaps: Vec<f32> = (1..=6).map(star_gap).collect();
        eprintln!("largest gap by number of joints on one body (1..6): {gaps:.4?}");
    }

    #[test]
    fn avian_ball_joint_holds() {
        let plain = joint_gap_after_fall(false, true);
        let ragdoll = joint_gap_after_fall(true, true);
        let no_collider = joint_gap_after_fall(true, false);
        eprintln!("joint gap after 1 s: plain {plain:.4} m, ragdoll-style {ragdoll:.4} m, second body without collider {no_collider:.4} m");
        assert!(plain < 0.01 && ragdoll < 0.01 && no_collider < 0.01);
    }

    #[test]
    fn body_and_bone_transforms_round_trip() {
        let actor = Transform::from_xyz(3.0, 1.0, -2.0).with_rotation(Quat::from_rotation_y(0.7));
        let rot_origin = coords::ue_rotation_matrix(ue_assets::properties::Rotator {
            pitch: 0,
            yaw: -16384,
            roll: 0,
        });
        let f = MeshFrame::new(rot_origin, 1.0, Vec3::new(0.0, 0.0, 2.0), 1.1, Vec3::new(0.0, 0.0, 5.0), &actor);
        let bone = (Quat::from_euler(EulerRot::XYZ, 0.3, -0.2, 1.1), Vec3::new(4.0, -7.0, 30.0));
        let (r, p) = f.body_from_bone(bone);
        let (q2, p2) = f.bone_from_body(r, p);
        assert!(q2.angle_between(bone.0) < 1e-4);
        assert!(p2.distance(bone.1) < 1e-3);
        // A point on the bone maps the same way through either path.
        let v = Vec3::new(1.0, 2.0, 3.0);
        let via_body = r * f.local(v) + p;
        let via_mesh = f.k * (f.q * (bone.0 * v + bone.1)) + f.b;
        assert!(via_body.distance(via_mesh) < 1e-4);
    }

    #[test]
    fn mesh_frame_matches_zed_mapping() {
        // world = actor * coords::pos(pre_pivot + D * R_o * ((p - origin) * s))
        let actor = Transform::from_xyz(1.0, 2.0, 3.0).with_rotation(Quat::from_rotation_y(-1.2));
        let r_o = coords::ue_rotation_matrix(ue_assets::properties::Rotator {
            pitch: 100,
            yaw: -16384,
            roll: 50,
        });
        let (s, origin, d, pre) = (1.3, Vec3::new(1.0, -2.0, 3.0), 1.1, Vec3::new(0.0, 0.0, 5.0));
        let f = MeshFrame::new(r_o, s, origin, d, pre, &actor);
        let p = Vec3::new(10.0, -4.0, 25.0);
        let ue = pre + d * (r_o * ((p - origin) * s));
        let expected = actor.translation + actor.rotation * coords::pos(ue.to_array());
        assert!((f.k * (f.q * p) + f.b).distance(expected) < 1e-4);
    }
}
