# STATUS

**Resolved 2026-10-03.** Cause: the Clot's `chr_spine3` has a mass of
3.14e-6 in the file (a placeholder), ~35,000x lighter than the ribcage. A
nearly massless link between two heavy bodies cannot pass joint corrections
along: the solver kept moving spine3 and never pulled the ribcage. Fix: part
masses are raised to at least 0.05 file units (`MIN_PART_MASS` in
`src/ragdoll.rs`; raises spine3 and spine1). Ribcage joint gap now 0.1-0.2
units (was 15-82) over five runs; screenshots show intact bodies. The write-up
below is kept as the record of the investigation.

## Goal
A from-scratch Rust + Bevy rewrite of Killing Floor (2009) that reads the
installed game's files. Current milestone: gore and ragdolls (see
`docs/DESIGN.md`, "Gore and ragdolls"). Step 1, Clot ragdolls, is mostly
working, with one broken joint.

## Setup
- Game: Killing Floor (Steam), at `references/killing_floor`.
- Agent / model: Claude Code (Claude Opus 5.5).
- Tools: Rust stable, Bevy 0.19.1, avian3d 0.7.0 (physics), NixOS flake
  (`nix develop -c ...`).
- Where things are: ragdoll code in `src/ragdoll.rs`; the .ka reader in
  `crates/ue-assets/src/karma.rs`; death handling in `src/zed.rs`.

## What works (tested)
- Reading every Karma ragdoll file (`cargo run --release -p ue-assets --bin kfpkg -- karma`):
  3 files, 24 ragdolls, including one for every zed type.
- The Clot ragdoll matches the mesh exactly: joint anchors 0.00 mesh units
  apart in the bind pose, primary and orthogonal axes 0.0 degrees apart (load
  log `ragdoll_loaded`).
- Hinge sign convention confirmed: walk-cycle knee and elbow angles fall inside
  the file's limits as written (load log `ragdoll_hinge_check`).
- On death the Clot becomes 16 physics bodies with 15 joints, falls, and comes
  to rest in about 1.6 s. Legs and arms stay attached (joint gaps 0.0).
- avian joints hold in headless tests (`cargo test avian_`).

## What doesn't work yet
- One joint, ribcage to spine3, does not hold: see below.
- Gore (severed parts, blood) not started. Other zeds not loaded yet.

## Which game owns the player
Not applicable (a rewrite).

## The current problem
The joint between `chr_ribcage` (child) and `chr_spine3` (parent) pulls
apart, so the upper body (ribcage, neck, both arms, which stay correctly
joined to each other) separates from the spine. Every other joint holds.
Expected: a gap near 0. Seen: 15 to 82 Unreal units at rest, different in
each run. The gap is 0 at spawn and grows from the first physics step.

## Evidence
Run: `nix develop -c cargo run --release -- --map KF-WestLondon --camera -4090,1100,-3650,-1.5708,-0.05 --walk --zed --input 60:fire --frames 300`
then read `logs/latest.log`:
```
ragdoll_joint_gap id=0 age=0.02 worst_joint=chr_larmcollar gap_unreal=0.0
ragdoll_joint_gap id=0 age=0.03 worst_joint=chr_ribcage gap_unreal=4.9
ragdoll_joint_gap id=0 age=0.10 worst_joint=chr_ribcage gap_unreal=14.7
ragdoll_joint_gap id=0 age=1.85 worst_joint=chr_ribcage gap_unreal=63.8
ragdoll_joint_gaps ... chr_ribcage->chr_spine3:60.0 (all others 0.0, spine1->pelvis 0.1)
```
The Clot_Trip data for this joint: `skeletal`, CONE_HALF_ANGLE_X/Y 0.01,
TWIST_HALF_ANGLE 0.15, POS1 0,0,0, POS2 0.188,0,0. `chr_spine3` is
"dynamics_only" (no collision shape); the ribcage has a sphere (r 8.7). The
ribcage is the only part with four joints (spine3, neck, two collars).

## What we've already tried
- Missing joint solver: avian's `xpbd_joints` feature was off (we disabled
  default features), so no joint worked at all. Turned on; that fixed every
  other joint. Not the cause of this one.
- Mirrored hinge limits: wrong sign at first; fixed (confirmed by walk-cycle
  angles). Not related (this joint is a ball joint).
- File inertia: implausibly small (forearm radius of gyration 0.2 units).
  Replaced by inertia computed from the shapes. Fixed spinning limbs; not this.
- 20 solver substeps instead of 6: gap still ~49.
- Soft angle limits (compliance 1e-3): gap still ~61.
- No angle limits at all on the two near-locked spine joints: gap still 82.
  So the point constraint itself fails, not the limits.
- Swapping body1/body2 in the ball joints: gap still 59.
- Headless tests: a joint with a collider-less body holds; one body with four
  joints holds (gap 1.2 mm). With six joints avian's gap grows to 15 cm, but
  the ribcage has four.

## Ideas not tried yet
- Reproduce the full Clot ragdoll headlessly (parse the real .ka in a test
  that skips if the install is missing) and remove parts one at a time until
  the ribcage joint holds. That would find the trigger without the game.
- Check whether the ribcage collider starts inside the level or another
  solid (it is on the ragdoll layer, colliding with the world only).
- Give `chr_spine3` a collider, or merge it into spine2/ribcage (skip the
  part and carry the bone).
- Check avian's constraint-graph colouring / overflow handling for joints
  added in the same frame as their bodies.
- Spawn the joints one frame after the bodies.

## Files that matter
- `src/ragdoll.rs`: bodies, joints, coordinate mapping, the gap logs.
- `src/zed.rs`: `death_launch`, the ragdoll start in `think_and_move`, the
  ragdoll branch of `animate_zeds` (logs `ragdoll`, `ragdoll_joint_gap`,
  `ragdoll_parts`, `ragdoll_joint_gaps`).
- `crates/ue-assets/src/karma.rs`: the .ka reader.
- `Cargo.toml`: avian3d features (`xpbd_joints` must stay on).
