# STATUS

**Mostly resolved 2026-10-05.** The orange cast was KF's vision overlay
(`HUDKillingFloor.DrawModOverlay`, KFX.SepiaShader), not the lighting:
the whole screen is multiplied by 2 x the zone's fog colour (brightened).
Now drawn (`src/overlay.rs`). On your matching screenshot (precise.jpg)
the per-channel ratios real/ours went from about (1.3, 1.1, 0.9) to
about equal; left wall (1.08, 1.05, 1.07), far buildings (0.98, 0.97,
1.0). Left: BSP and the scaffold tarp about 20% dark, the sky a little
less orange (layer order), the phone booth glass, a brighter fire decal.
The write-up below is the record of the investigation.

## Goal
Open KF: a Rust/Bevy rewrite of the Killing Floor 1 engine. Current step:
milestone 11, baked lighting (docs/DESIGN.md, "Baked lighting"). Make
KF-WestLondon look like the real game.

## Setup
- Games and exact versions: Killing Floor (Steam, run under Proton) at
  references/killing_floor. Display settings in System/KillingFloor.ini
  [WinDrv.WindowsClient]: Brightness=0.8, Contrast=0.7, Gamma=0.8,
  Bloom=False.
- Agent / model: Claude Code (Opus).
- Other tools and loaders: Bevy 0.19, our `kfpkg` tool.
- Where things are: reference screenshots in
  references/london_kf_screenshots/ (yours, from the real game; the
  second, 20261005131359_1.jpg, is the spawn view). Ours in
  work/screenshots/. Lightmap pages in work/lighting/.

## What works (tested)
- Reading the lighting: BSP lightmaps and per-vertex mesh colours read
  on all 35 maps. `kfpkg lighting <map>` checks counts and positions.
- The stored lightmap matches the stored shadow bits exactly. Checked on
  the road surface at the spawn (record 111, moonlight `Sunlight0`).
- Drawing: the level is lit only by its stored lighting. Light pools,
  hard shadows and coloured lamps show up in the right places.
- Mesh colour channel order fixed (R, G, B, A, not B, G, R, A). The
  viaduct went from blue to warm, as in the real game.

## What doesn't work yet
- Overall look against the real game: same structure, wrong brightness
  and colour (numbers below).
- KF-Clandestine, KF-Forgotten, KF-Hell save their lightmap pages empty.
  Their BSP keeps the old sun.
- Zeds, weapons and hands are still lit by the made-up sun (L4).
  Terrain is not lit yet (L3).

## Which game owns the player
Not applicable (a rewrite).

## The current problem
With the stored lighting drawn as texture x light x K (K = 2, UE2's
assumed "overbright"), our spawn view is too dark and too grey. The real
game is brighter in lit areas and clearly orange and saturated
everywhere, the sky and the haze included.

## Evidence
Average colours of matching regions, spawn view (real vs ours):

| Region | Real | Ours K=2, fog | Ours K=2, no fog | Ours K=4, fog |
| --- | --- | --- | --- | --- |
| Viaduct brick (mesh) | 126, 87, 54 | 93, 74, 54 | 90, 69, 47 | 116, 105, 91 |
| Scaffold tarp (mesh) | 106, 81, 49 | 76, 65, 48 | 56, 44, 27 | 84, 74, 61 |
| Right wall | 45, 35, 24 | 43, 37, 28 | 7, 5, 2 | 60, 49, 36 |
| Road, right | 31, 27, 21 | 43, 39, 34 | 18, 18, 17 | 55, 48, 39 |
| Pavement, near (BSP) | 69, 50, 34 | 38, 33, 26 | 14, 12, 10 | 53, 45, 34 |
| Sky | 146, 104, 63 | 77, 68, 53 | 77, 68, 53 | 126, 115, 101 |

- The real game's colours have about twice the red-to-blue ratio of
  ours. Every fog colour in the map is a neutral brown, e.g. the spawn
  zone ZoneInfo4 has DistanceFogColor (91, 80, 64), fog from -500 to
  4500.
- With fog off, our underlying light is very dark. Fog was lifting the
  shadows.
- The views are not exactly the same spot. Our camera is the first
  PlayerStart; yours is wherever KF spawned you, close by.

## What we've already tried
- Tonemapping off. UE2 had none, and Bevy's default film curve darkens
  and greys. Kept: slightly more contrast, colour cast unchanged.
- Fog off, to measure the light underneath. It showed the light is far
  too dark, but fog doesn't explain the orange.
- K = 4 instead of 2. Brightness lands near the real game (best guess K
  is about 3), but ours stays grey.
- Checked the lightmap's alpha channel for a hidden intensity
  multiplier. There isn't one: alpha only marks used and unused texels.
- Worked out UE2's gamma ramp from the ini values, using a formula I
  remember from UE2: 1.2 x^1.25 + 0.05. Not sure it's right; as
  computed, it's equal on all channels and can't add an orange cast.

## Ideas not tried yet
- A side-by-side at exactly the same spot and view. For example, you
  stand somewhere unmistakable (against the phone box, facing the
  ambulance), and I put our camera there.
- An in-game screenshot with fog off or lighting off, if KF's console
  allows it. That would separate the fog, the lighting and the display
  curve.
- The sky's layer order. Before the sky fix, our sky showed orange at
  some angles; the real sky is orange, so the layer on top may be the
  wrong one. A wrong sky also changes how warm the frame looks.
- Zone ambient. AmbientBrightness 1-2 with Hue 35 (orange) and
  Saturation 100. Small, but it's orange and adds to every surface.
- Whether UE2's fog is applied in gamma space. Bevy fogs in linear
  space, which washes colours out differently.

## Files that matter
- src/lighting.rs: K (BRIGHTNESS), lightmap upload, lightmap material.
- src/map.rs: where BSP and meshes get their lighting.
- src/camera.rs: tonemapping off.
- src/zones.rs: distance fog.
- crates/ue-assets/src/bsp.rs, crates/ue-assets/src/lighting.rs:
  readers.
- crates/ue-assets/src/bin/kfpkg.rs: `kfpkg lighting <map> [X,Y]`.
  The probe prints the polygon under a point, its lightmap page and
  coordinates, and the lightmap value there.

---

# Previous STATUS (resolved 2026-10-03, kept as a record)


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
Run: `nix develop -c cargo run --release -- --map KF-WestLondon --camera -4090,1100,-3650,-1.5708,-0.05 --zed --input 60:fire --frames 300`
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
