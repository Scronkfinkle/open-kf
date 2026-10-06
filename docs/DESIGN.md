# kf-rs design

A from-scratch Rust + Bevy reimplementation of Killing Floor (2009) that reads
the original game's files from an installed copy. Single-player and offline only.

## What we know about the original game (checked 2026-10-03)

- **Engine:** Unreal Engine 2.5, the UT2004 branch. `System/Build.ini` says
  `UT2004_Build_[2004-11-11_10.48]`. Of the 548 packages, 545 start with the magic
  number `0x9E2A83C2`. Standard Unreal uses `0x9E2A83C1`, so this is a Tripwire
  variant. The rest of the header is the normal layout and is not encrypted. The
  other 3 use the standard number (`Textures/KFGui.utx`,
  `StaticMeshes/KFMuzzleFlashes.usx`, and `Textures/CellExample.utx`, which is
  also the only version-121 file). All are licensee version **29**. Every other
  file is package version **128**.
- **Install:** `references/killing_floor` points at the Steam install. It is the
  Windows build (`KillingFloor.exe`). There are no Linux binaries. That is fine
  because we never run or load the original engine.
- **Everything is an "Unreal package".** A package is a container file holding
  many named objects. The extension only tells you what is usually inside:

  | Extension | Contents | Count |
  | --- | --- | --- |
  | `.rom` (Maps/) | a level: geometry, placed actors, lighting, sometimes embedded textures and meshes | 40 |
  | `.utx` (Textures/) | textures and materials | 173 |
  | `.usx` (StaticMeshes/) | static (non-animated) meshes | 83 |
  | `.ukx` (Animations/) | skeletal meshes and animations (zeds, player, weapons) | 66 |
  | `.uax` (Sounds/) | sound effects | 156 |
  | `.u` (System/) | game code compiled from UnrealScript | ~30 |
  | `.ogg` (Music/) | music, a standard format and not a package | 75 |

- **Game logic is mostly UnrealScript** (Unreal's scripting language) in
  `KFMod.u`, `KFChar.u` and others. The packages appear to contain the original
  script source text: the readable string `class ZombieClot_XMas extends ZombieClot`
  is in `KFChar.u`. If that holds, we can read how waves, zeds and weapons work
  from the source instead of decompiling bytecode. Not yet verified for every class.
- **Low-level engine behaviour** (movement physics, collision, rendering) is in
  native Windows DLLs. We rebuild it from public knowledge of Unreal Engine 2 and
  by checking numbers against the scripts. We do not disassemble the DLLs unless
  we hit a wall, and we ask first if it comes to that.

## Hard boundaries (from CLAUDE.md)

- Game files are read in place, never copied into the repo. Anything extracted
  for study (dumped properties, exported textures, script source) goes to
  `work/`. `work/` lives inside the project but is gitignored.
- No networking, no Steam, no anti-cheat. Only solo play.
- `.gitignore` is a whitelist. New kinds of source files must be added to it
  on purpose.

## Relation to the hl2-rs reference

`references/hl2-rs` is a similar project for Half-Life 2. It does **not** use
Bevy: it uses `miniquad`, a much smaller graphics library. The Source and Unreal
file formats are unrelated, so we copy its approach, not its code:

- File-format reading is a separate crate with no graphics code (its
  `source-assets`, our `ue-assets`). That crate can be tested without a window.
- Install discovery: a default path, overridable by an environment variable.
- Validation is recorded with real numbers: parse every installed file and log
  the counts and failures.

Gildor's UE Viewer (umodel) is the best public reference for Unreal 2 file
formats, KF1 included. It is GPL-licensed C++, so we use it as documentation
only and do not translate its code line for line.

## Architecture

```
kf-rs/                     Cargo workspace root
  src/                     the game binary (Bevy app): rendering, input, camera, gameplay
  crates/ue-assets/        library: reads Unreal packages and converts objects into plain
                           Rust data (meshes, textures, actors). No Bevy dependency.
  work/        (ignored)   extracted data for study
  logs/        (ignored)   one log file per run, for checking results by numbers
  docs/DESIGN.md           this file
```

**Coordinates.** Unreal uses Z-up, left-handed axes (X forward, Y right, Z up).
Bevy uses Y-up, right-handed axes (X right, Y up, -Z forward). All conversion
happens in one function at the boundary between `ue-assets` and the game:
`bevy = (ue.y, ue.z, -ue.x) * SCALE`. This mapping also flips handedness, which
is required. `SCALE` turns Unreal units into metres. Its exact value is not
settled (around 1/50). It will be a single constant.

**Logging.** Every run writes `logs/latest.log` with plain `key=value` lines,
for example `map=KF-Farm bsp_tris=48210 static_meshes=912 missing_textures=3`.
Since nobody on the agent side can see the screen, these numbers are how a
change gets verified.

## Milestone 1: map viewer

Goal: `cargo run --release -- --map KF-Farm` opens a window with the map's level
geometry and static meshes drawn with their textures, and a free-fly camera.
No gameplay.

Each step is small enough to revert by itself and ends with a test you can run.

0. **Scaffolding.** *(Done 2026-10-03, Bevy 0.19.1.)* Turn the project into a workspace. Add an empty `ue-assets`
   crate. Add Bevy (latest stable, version checked at the time). Add the Linux
   system libraries Bevy needs (Vulkan, ALSA, udev, X11/Wayland) to `flake.nix`.
   Add install discovery (default `references/killing_floor`, override with
   `KF_ROOT`) and the log file.
   *Test:* `cargo run` opens an empty Bevy window and writes the install path to
   `logs/latest.log`.

1. **Package reader.** *(Done 2026-10-03: 548/548 packages parse.)* Read the header, the name table (all strings the package
   uses), the import table (objects borrowed from other packages) and the export
   table (objects defined in this package). Add a small CLI tool, `kfpkg`.
   *Test:* `cargo run -p ue-assets --bin kfpkg -- scan` parses every package in
   the install and prints names/imports/exports counts per file plus a failure
   count. Target: 0 failures.

2. **Property reader.** *(Done 2026-10-03: 559,872 of 559,875 objects read; see
   "Stale property sizes" below for the 3 failures.)* Unreal objects store their settings as a list of tagged
   properties (name, type, value). Actor positions, rotations and texture
   references are all stored this way.
   *Test:* `kfpkg props Maps/KF-Farm.rom StaticMeshActor` dumps actor properties
   to `work/`. Counts and a few sample positions are logged.

3. **(Research) Script source export.** *(Done 2026-10-03: 3757/3757 classes. Only
   95 files contain `defaultproperties`. Default values such as zed health are
   stored as binary class defaults, which must be decoded separately before
   milestone 5.)* Pull the embedded UnrealScript text out
   of the `.u` files into `work/scripts/`. This becomes the main reference for
   gameplay in later milestones. It stays out of git.
   *Test:* the file count in `work/scripts/` matches the number of classes found.

4. **Textures.** *(Done 2026-10-03: 8535 textures read, 0 failures. P8 textures
   whose palette is in another package wait for cross-package lookup in step 6.
   G16, the 16-bit terrain heightmap format, is not decoded yet.)* Decode texture objects (DXT1/3/5 compressed, RGBA8, 8-bit
   palettised) and their mipmaps.
   *Test:* `kfpkg texture` exports a handful to PNG in `work/`, and a format
   histogram over all `.utx` files reports 0 unknown formats.

5. **Static meshes.** *(Done 2026-10-03: 5021/5021 meshes, every vertex inside
   its bounding box, materials = sections for all.)* Decode vertices, UVs, triangle sections and material
   references.
   *Test:* vertex/triangle counts and bounding boxes are logged for every mesh in
   `StaticMeshes/`, with 0 failures.

6. **Level contents.** *(Done 2026-10-03: all 40 maps load. BSP reads in every
   map, every placed mesh and texture resolves and decodes. `kfpkg level <map>`
   reports the counts.)* In a `.rom`, find the level's actor list. Read static
   mesh actor placements (position, rotation, scale). Read the BSP model (BSP is
   the brush-built level geometry: walls, floors, rooms) and turn its polygons
   into triangles.
   *Test:* for KF-Farm, log actor counts by class, BSP polygon count and the
   overall bounding box.

7. **Bevy viewer.** *(Done 2026-10-03: all 40 maps open. Visual result not yet
   checked by a person.)* Send steps 4–6 to Bevy: BSP and static meshes with diffuse
   textures, simple sun plus ambient lighting, and a fly camera (WASD + mouse).
   *Test:* `cargo run --release -- --map KF-Farm` shows the map. The log records
   meshes spawned, triangles, missing textures, camera position once per second,
   and frame time.

Deferred past milestone 1 (listed so nobody mistakes them for done): terrain,
skyboxes, full materials (shaders, combiners, panners), baked lightmaps,
zone/portal visibility culling, emitters.

## Level loading (step 6)

- `package_set.rs` loads packages on demand and follows imports. An import's
  path is `Package.Group.Object`. The lookup matches the import's class too,
  because one package can hold two objects with the same path and different
  classes.
- `level.rs` scans the map's exports. The BSP is the largest `Model` that is
  not referenced by an actor's `Brush` property. An actor is drawn as a static
  mesh if it has a `StaticMesh`, is not `bHidden`, and has `DrawType = 8`. If
  `DrawType` is unset, the class must be `StaticMeshActor` or end in `Mover`:
  class default values are not decoded yet, so this list stands in for them.
- `bsp.rs` turns each BSP node into a convex polygon. Surfaces flagged invisible,
  portal or fake-backdrop (sky) are not drawn. Texture coordinates are
  U = (P - Base) . TextureU / texture width, and V likewise.
- `material.rs` follows Shader → Diffuse, Modifier → Material, Combiner →
  Material1 and MaterialSwitch → Materials[Current] down to a texture, and
  derives opaque, masked, translucent or invisible.

**Rotation convention (partly verified).** Unreal rotators: 65536 units = 360°.
The formula used is the standard Unreal one. Its forward axis (cos P cos Y,
cos P sin Y, sin P) matches UnrealScript's `vector(Rotator)`. The roll and
side axes are **not verified**. The planned check was to compare rotated brush
polygon normals with BSP normals, but no map has a rotated CSG brush. In
KF-Farm, 625 of 2895 placed meshes have non-zero pitch or roll.

## Viewer (step 7)

- `src/coords.rs` is the only place where Unreal coordinates become Bevy ones
  (`bevy = (ue.y, ue.z, -ue.x) / 50`). Rotations convert as `C·R·Cᵀ`.
- `src/map.rs` builds one Bevy mesh per BSP material, and one per static-mesh
  section shared by every actor that uses it. Textures are uploaded once each.
  DXT textures stay compressed on the GPU (BC1/2/3), 229 of 234 on
  KF-WestLondon, cutting texture memory from 709 MB to 115 MB.
- **Winding, measured not assumed.** After the axis change, BSP polygons must
  be reversed (decided per polygon with Newell's normal against the surface
  normal: 2053 of 2055 flip on WestLondon). Static mesh triangles must *not*
  be reversed: 111681 of 111985 face their vertex normals in file order. Both
  numbers are logged on every load (`flipped_to_match_normal`,
  `winding_agree`).
- **Mirrored actors.** An actor with a negative scale on an odd number of axes
  (e.g. `DrawScale3D = (1, -1, 1)`) is a mirror image. Mirroring reverses
  triangle winding. Unreal compensates for this; Bevy does not, so such actors
  use a separate copy of the mesh with reversed triangles. Without it,
  back-face culling hid the KF-WestLondon car tunnel shell. 43 of 1676 actors
  on that map are mirrored.
- **Skins.** An actor's `Skins` array replaces the mesh's material per section
  (`Skins[i]` for section i; Null keeps the mesh's own). 169 actors on
  KF-WestLondon use it, including the skybox mesh and two invisible
  `voidtex` blocking Movers that had shown as grey boxes.
- Lighting is a fixed sun plus ambient light. Lightmaps are not used.
- **Screenshots** (F12, or `--camera ... --screenshot N`) go to
  `work/screenshots/` with a `.txt` command that recreates the view. You
  allowed screenshots as a visual check alongside the logged numbers. Saved
  views are in `docs/test-views.md`.
- `src/camera.rs`: fly camera starting at the first PlayerStart. Field of
  view matches KF: PlayerController DefaultFOV 90 (horizontal, for 4:3) with
  KFPlayerController bUseTrueWideScreenFOV, so the vertical angle stays at
  the 4:3 value, 73.74°, on any screen. Bevy's default was 45°, which made the
  view about 1.7x zoomed in.

## Sky zone (implemented 2026-10-03; skyline confirmed by screenshot on KF-WestLondon)

Note: the sky zone was not the cause of the tunnel bug. That was a mirrored
mesh, see "Mirrored actors" above. It is still the correct way to draw
fake-backdrop surfaces and the sky.

**Problem (first diagnosis).** On KF-WestLondon you reported tunnels without
inner walls, though the beams over them were there. `kfpkg zones KF-WestLondon 25` shows the tunnel
zone: road and pavements (drawn), a ceiling and west wall of
`Engine.BlackTexture` (drawn, black), and the east wall flagged
`FAKE_BACKDROP`, which the viewer skips, leaving a hole. In Unreal Engine 2 a
fake-backdrop surface is a window onto the **sky zone**: a separate small
scene (on WestLondon around (-17457, -6856, 5012), holding the
`London_Skybox` mesh plus fog cylinders) drawn as if it surrounds the viewer.

**How UE2 draws it.** The sky zone is rendered from the `SkyZoneInfo`
actor's position, turning with the player's view but not moving with them.
Then the normal scene is drawn on top, and backdrop surfaces are left
uncovered.

**Plan.**
1. Find the sky zone: the BSP zone whose zone actor's class contains
   `SkyZone`.
2. Put sky-zone geometry on its own render layer (Bevy `RenderLayers`, layer 1):
   BSP nodes whose front zone is the sky zone, and static meshes whose location
   is inside the sky zone's BSP bounds. Everything else stays on layer 0.
3. Add a second camera (order -1, layer 1) at the SkyZoneInfo position that
   copies the main camera's rotation every frame. The main camera (order 0,
   layer 0) stops clearing the colour, so the sky shows wherever the main
   scene draws nothing, including through backdrop surfaces.
4. The sun lights both layers.
5. Log: sky zone index and actor, sky BSP polygon and actor counts. With no
   sky zone, behave exactly as now.

Not covered: sky zones that scroll or are scaled, and fog.

## Known file-format quirks

**Structs are nested tagged lists.** Only Vector, Rotator and Color are fixed
binary inside tagged properties. Every other struct (Scale, Plane, Range,
RangeVector, PointRegion, TerrainLayer, KF's own) is a nested tagged list,
determined by trying both readings on four packages. The reader walks
nested lists by their own tags, because their stored size can be stale too
(7 of 10 TerrainLayer values overran it). Before this fix TerrainInfo read 3
properties instead of 23.

**Stale property sizes.** Each tagged property records its size in bytes. In a
few objects the recorded size is too small. The real data is longer by one
byte for each large object reference inside it. Probable cause, not verified:
the editor computed the size before objects were renumbered, and a reference
that grew from 2 to 3 bytes made the stored size wrong. The original engine is
unaffected because it reads arrays element by element using the class
definition. Seen in: `KF-Suburbia.rom` StaticMesh `Vehicle.PoliceCar`
(`Materials`, off by 2) and `KF-Steamland.rom` `KF_DialogueSpot1`/`2`
(`Dialogues`, off by 1 in the one examined). Plan: decode arrays whose type we
know (such as `StaticMesh.Materials` in step 5) element by element, and do not
trust the stored size for them. Arrays of unknown type still rely on the size.
This is implemented in `properties::known_struct_arrays`, currently only
`StaticMesh.Materials`, and it fixed the police car. The two dialogue spots
still fail.

**Empty tail mipmaps.** In 85 non-square DXT textures the smallest mipmaps
(1x1, 2x1, 4x1) have 0 bytes of data. The renderer should use only the mips
that have data. Two DXT5 textures have no data at all.

**MipZero** (a texture's stored average colour) is a quarter of the true
average on every channel. Verified on `LondonCommon.WaterCubemapImage`: decoded
(116,108,86,255), stored (29,27,21,63).

**Stale BSP vertices.** The BSP vertex pool contains entries that point past
the point list (7978 of 51650 in KF-Farm). No node uses them; only vertices
used by nodes are checked.

**Arrays of structs are tagged.** Each element of a struct array (seen in
`StaticMesh.Materials`) is its own tagged property list ending in `None`. It is
not a fixed binary layout.

## Terrain (implemented 2026-10-03; checked by screenshot on KF-Farm, KF-Crash, KF-Hell)

18 of 35 `KF-` maps have terrain (outdoor ground). A `TerrainInfo` actor holds:
`TerrainMap` (a G16 16-bit heightmap texture), `TerrainScale` (e.g. 160, 160,
64 on KF-Farm), `Location`, up to 32 `Layers` (texture, `AlphaMap` blend
mask, `TerrainMatrix` mapping world position to texture coordinates),
`QuadVisibilityBitmap` (holes) and `EdgeTurnBitmap` (which diagonal splits
each quad). Before this plan the property reader stopped early on
TerrainInfo, because a TerrainLayer struct had a stale size. Fixed first:
non-binary structs are now read by their own tags.

Steps:
1. Decode G16 heightmaps.
2. Build the height grid. Assumed formula (standard UE2, **to be verified**):
   world = Location + ((x - W/2)·Scale.X, (y - H/2)·Scale.Y,
   (h - 32768)·Scale.Z/256). Verification: compare the terrain height under
   every PathNode and PlayerStart with the actor's Z. The offsets should
   cluster tightly (actors sit a fixed height above the ground). A wrong
   formula or a flipped axis scatters them.
3. Mesh: one quad per heightmap cell, split along the diagonal given by
   EdgeTurnBitmap, holes from QuadVisibilityBitmap, normals from the heights.
4. Texturing the way UE2 does it, in passes: one mesh per layer, the layer
   texture tiled by its TerrainMatrix, per-vertex alpha from the layer's
   AlphaMap. Layer 0 opaque, later layers alpha-blended on top. Triangles
   where a layer has zero weight are left out.
5. Terrain inside the sky zone goes on the sky layer.
6. Log: heightmap size, height range, PathNode offset statistics, triangles,
   layers. Check by screenshot on KF-Farm.

Not covered: decoration layers (grass meshes), terrain lighting.

**Results.** The height formula holds: on every terrain checked, ground actors
sit a median 43-44 units above the computed ground (KF-Farm: 805 of 847
within 16 units of the median). `QuadVisibilityBitmap` bit set = visible:
ground actors are mostly over visible quads (KF-Farm 847 vs 112 over holes;
the rest stand on BSP floors inside buildings). Layer alpha is the AlphaMap's
alpha channel (RGB is a constant 127). A layer without an AlphaMap contributes
nothing; it is an unpainted leftover (KF-Crash, KF-Wyre, KF-Steamland). Layer
textures are uploaded with alpha forced opaque, because their alpha is often
a specular mask. Blending: layer 0 is opaque, scaled by its effective weight;
the others are additive, scaled by theirs. That gives the same result as
layer-over-layer blending regardless of draw order. All 21 terrain maps load
(up to 4 terrains per map, heightmaps from 32x32 to 512x512, including
non-square ones).

## Walking with collision (milestone 2, implemented 2026-10-03; play test pending)

**Class defaults (done first).** A class export holds its compiled script
and then its default property values. The script's serialized length is
not stored, so `properties::find_class_defaults` finds the defaults from the
end: the first offset from which a property list parses and ends exactly at
the end of the data, with every name being a property declared somewhere in
the game's code. Checked: `Engine.Pawn` gives GroundSpeed 440, JumpZ 420,
AccelRate 2048, BaseEyeHeight 64 (the standard UE2 values).

KF player values (KFHumanPawn and the classes it inherits from):

| Value | Unreal units | Source class |
| --- | --- | --- |
| GroundSpeed | 200 /s | KFHumanPawn |
| AccelRate | 1000 /s² | KFHumanPawn |
| JumpZ | 325 /s | KFHumanPawn |
| AirControl | 0.15 | KFHumanPawn |
| MaxFallSpeed | 600 /s | KFHumanPawn |
| CollisionRadius / CollisionHeight | 20 / 50 (cylinder half-height) | KFPawn / KFHumanPawn |
| BaseEyeHeight | 44 above cylinder centre | KFPawn |
| CrouchHeight | 34 | KFHumanPawn |
| WalkingPct / CrouchedPct | 0.4 / 0.4 | xPawn |
| Gravity | -950 /s² | PhysicsVolume (maps may override per volume) |
| GroundFriction | 8 | PhysicsVolume |

Not in the data (native engine constants, values from UE2 knowledge, **not
verified**): max step height 35, minimum walkable floor normal Z 0.7.

**Steps.**
1. Class-default lookup with inheritance (`class_defaults.rs`). An actor's
   value is its own property, else the nearest class in its chain that sets
   it. Use it for DrawType/bHidden in level loading, replacing the
   hard-coded class list, and for collision flags.
2. Collision world using the avian3d physics library, for shape queries only
   (no rigid-body simulation yet):
   - BSP: one triangle mesh of all polygons except not-solid (0x8) and
     portal surfaces. Invisible surfaces still block.
   - Static meshes whose actor has bCollideActors, bBlockActors and
     bBlockNonZeroExtentTraces, using only sections whose material has
     EnableCollision.
   - Terrain: visible quads.
   - BlockingVolumes: convex hull of their brush polygons.
   - Check: a downward ray from every PathNode must hit something about 44
     units below. Log hits, misses and distance statistics.
3. Walking controller, an Unreal-style cylinder: accelerate toward input at
   AccelRate up to GroundSpeed, ground friction, gravity, jump, step up to
   35 units, slide along walls, walkable floors only. **V** toggles walk/fly.
   Log position, speed, on-ground state and floor normal.
4. Automated test: `--walk` starts in walk mode, and `--autowalk SECONDS`
   holds W.

**What was learned implementing it.**
- Blocking volumes must use their real polygons, not a convex hull. The
  KF-WestLondon car tunnels have hollow arch-shaped volumes, and a hull
  filled the walkable tunnel. With polygons, 0 PathNodes start inside a
  collider.
- Brush space to world: subtract PrePivot, rotate, add Location. This
  lines BlockingVolume44 up with the tunnel shell to within a few units.
- Movers don't block yet. KF-WestLondon has an invisible street barrier (a
  Mover tagged ChopperTakeOffTC) that rises when the helicopter leaves; until
  movers move, it would block the street forever.
- Resting gap: the cylinder rests two skins (1 unit) above the floor. Resting
  at exactly the cast's stop distance made every horizontal sweep report
  the floor as a hit at distance 0, so walking stuttered.
- Velocity after a ground move becomes the movement actually achieved, as in
  Unreal, so walking into a wall shows speed 0. Expected from the log: the player settles on the floor (eye
   height 44 + 50 above the ground), then moves at about 200 units/s until
   blocked.

## Weapons in first person (milestone 3, planned 2026-10-03)

**What the data says.** Weapon classes in KFMod.u give, via class defaults:
the first-person mesh (`Single`: `KF_Weapons_Trip.9mm_Trip`; `Knife`:
`KF_Weapons_Trip.Knife_Trip`), `Skins` overrides, `PlayerViewOffset`
(9mm: 20, 25, -10), `DisplayFOV` (9mm 70, knife 75), animation names
(`Idle`, `Select`, `PutDown`, `Reload`; fire animations are in the fire-mode
classes), `Weight`, and `bSpeedMeUp` (knife speed bonus). The meshes
(`SkeletalMesh`) and their animation sets (`MeshAnimation`, e.g. `9mm_anim`)
are in `Animations/KF_Weapons_Trip.ukx`. Both have no properties: all binary,
layout to be worked out and verified like the BSP.

**Steps.**
1. Skeletal mesh reader: vertices, triangles, UVs, materials, the bone
   hierarchy (reference pose), per-vertex bone weights. Checks: indices in
   range, every bone's parent earlier in the list, weights per vertex summing
   to 1, consistent counts across all 66 `.ukx` packages. `kfpkg skelmeshes`.
2. Animation reader: bone names, sequences (name, frame count, rate), per-bone
   tracks (rotation and position keys with times). Checks: bone counts match
   the mesh, keys within sequence length, unit-length rotations. `kfpkg anims`.
3. Show a weapon in its reference pose in front of the camera, using Bevy
   skinning (joint entities). Check by screenshot.
4. Play animations: sample tracks each frame, drive the joints. Start with
   `Idle`; `Select` when switching. Check by screenshot sequence and logged
   joint data.
5. First-person placement: attach to the camera with PlayerViewOffset and
   the weapon's own DisplayFOV (a separate camera pass for the weapon, so it
   never clips into walls, like UE2).
6. Weapon switching between the starting weapons (knife, 9mm) with number
   keys, a fire animation on click (no damage yet), and the knife speed bonus
   from KFHumanPawn's formula.

Each step is logged, and screenshot views are added to `docs/test-views.md`.

**Findings (steps 1-2, done).**
- The SkeletalMesh layout is described at the top of `skeletal.rs`. The raw
  source mesh is six "lazy arrays", each prefixed with the absolute file
  offset of its end. They are found by locating the self-checking chain
  instead of decoding the level-of-detail data before them.
- MeshAnimation sequence records start with a float not in standard UE2
  (meaning unknown; probably a Red Orchestra addition). With it, every
  animation set parses to its exact end.
- **Rotation convention:** non-root bone rotations, in both the reference
  skeleton and the animation keys, are stored inverted (conjugated). Checked
  by measurement on the 9mm: inverted gives a median 2.25 units from vertex to
  dominant bone (46 as stored), and the first Idle frame changes edge lengths
  1.5% (11% as stored).
- Animation sets can name bones a mesh doesn't have (zed sets share a larger
  skeleton). Tracks are matched to mesh bones by name.
- Weapon meshes include the arms and hands (the 9mm has 62 bones).

**Steps 3-6 (done, `src/weapon.rs`).**
- CPU skinning in Unreal mesh space, then (point - MeshOrigin) * MeshScale
  (5 for KF weapons), then `coords::pos`. Without MeshScale the weapon
  showed tiny in the screen corner.
- Placement: a weapon camera (order 1, layer 2, own depth) copies the main
  camera. The weapon is offset by PlayerViewOffset x 0.9 / DisplayFOV x 100
  (Engine.Pawn.CalcDrawOffset). Weapon FOV: DisplayFOV widened like KFWeapon
  CalcFOVForAspectRatio, which keeps the 4:3 vertical angle (same rule as the
  world view, confirmed in KFPlayerController.InitFOV).
- Animations: Select, Idle (looping), Fire (FireModeClass FireAnim or the
  knife's FireAnims list, at FireAnimRate, limited by FireRate), PutDown, then
  switch and Select, and Reload. Tracks are sampled with slerp/lerp between
  keys.
- Knife speed bonus = 200 x BaseMeleeIncrease (0.2) - Weight x 2 = +40 units/s
  (KFHumanPawn).
- Weapon materials are unlit and double-sided for now.
- View and weapon bob (`walk.rs` ViewBob, after KFPawn.CheckBob and
  Pawn.WeaponBob): speed x 0.9, Bob 0.006; side = right x Bob x speed x
  sin(8t); vertical = 0.75 x Bob x speed x sin(16t), with t advancing at
  0.3 + 0.7 x speed / GroundSpeed. The camera moves by WalkBob (EyePosition).
  The weapon moves by BobDamping x WalkBob sideways and (0.45 + 0.55 x
  BobDamping) x WalkBob vertically (9mm 6, knife 8). Measured: side +-1.08,
  vertical +-0.81 units at 200 units/s. Landing bob is not done.
- Not done: damage, ammo, muzzle flash, sound, landing bob, other weapons.

## Zeds (milestone 4, steps 1-4 implemented 2026-10-03)

**What the data says.** `KFChar.ZombieClot_STANDARD` (mesh
`KF_Freaks_Trip.CLOT_Freak` with a Skins override) inherits from
`KFMod.ZombieClotBase` and `KFMonster`: GroundSpeed 105, Health 130,
DrawScale 1.1, PrePivot (0, 0, 5), walk animation `ClotWalk`, idle
`Idle_LargeZombie`, melee `ClotGrapple` / `ClotGrappleTwo` /
`ClotGrappleThree`, MeleeRange 20, CollisionRadius 26. The Clot mesh has
RotOrigin yaw -16384 (a quarter turn), unlike the weapons.

**Steps.**
1. Move the skinning code from `weapon.rs` into a shared `skinned.rs` (mesh
   parts, pose sampling, CPU skinning), used by weapons and zeds. Weapons
   must look the same afterwards (test views).
2. Place a Clot in the world: UE2 mesh-to-world transform (MeshOrigin,
   MeshScale, RotOrigin, DrawScale, PrePivot, actor rotation and location),
   lit with per-frame normals, playing its idle. Check: lowest skinned
   vertex at the bottom of its collision cylinder (on the floor), and a
   screenshot.
3. Animation selection: idle when still, walk when moving, melee when close.
4. Simple behaviour: walk toward the player at GroundSpeed with the same
   cylinder movement as the player (no pathfinding yet), stop at melee range
   and attack (animation only). A key spawns a Clot in front of you. Log
   positions and states.
5. Later: pathfinding over PathNodes/ReachSpecs, other zed types, spawning
   from ZombieVolumes, damage both ways.

**Findings.**
- RotOrigin is applied as stored (`rot * ((p - MeshOrigin) * MeshScale)`).
  Checked from the data: the Clot's ankle-to-toe direction is mesh +Y, and
  yaw -16384 turns it to +X, the actor's forward. A no-RotOrigin test was
  side-on. Grapple animations twist sideways, which first looked like a
  facing bug.
- With DrawScale 1.1 the Clot is ~96 units tall. Its lowest idle vertex is 50
  below the cylinder centre, against a CollisionHeight of 44, so the feet dip
  about 6 units into the floor. Not corrected; cause unknown (could be how KF
  draws it too).
- `ClassDefaults::get_array_names` reads fixed-array defaults such as
  `MeleeAnims[0..2]`.
- Each zed gets its own copies of the meshes (`SkinnedModel::new_instance`);
  materials are shared. Walking reuses `walk::Mover` with the zed's
  cylinder size.

## Combat (milestone 5, implemented 2026-10-03)

**What the data says.**
- 9mm (SingleFire): DamageMin 25, DamageMax 35, Spread 0.015, FireRate 0.175,
  MagCapacity 15, damage type DamTypeDualies (KFWeaponDamageType:
  HeadShotDamageMult 1.1, bCheckForHeadShots).
- Knife (KnifeFire/KFMeleeFire): MeleeDamage 19, weaponRange 70, damage delay
  0.45 s after the swing starts, WideDamageMinHitAngle 0.75 (cone),
  DamTypeKnife < DamTypeMelee: HeadShotDamageMult 1.25.
- Zeds (KFMonster.TakeDamage / IsHeadShot): Health -= damage. A headshot
  test uses a sphere at the head bone (HeadBone 'head', radius HeadRadius 7 x
  HeadScale 1.1, centre moved HeadHeight 2 x HeadScale along the bone's X
  axis), against the shot line from the hit point. A headshot multiplies
  damage by HeadShotDamageMult and subtracts it from HeadHealth (25); at
  <= 0 the head is removed. Clot MeleeDamage 6; a melee hit counts if the
  target is within MeleeRange x 1.4 + both collision radii.
- Player: Health 100 (Pawn default).

**Steps.**
1. Player health, a HUD (health, ammo), zed melee damage at mid-attack,
   death and respawn at the player start.
2. 9mm: ammo (15 in the magazine, reload refills, dry fire when empty), a
   shot line from the eye with spread, hit test against the world (avian ray)
   and zed cylinders (nearest wins), headshot test as KF does, damage, zed
   death (KnockDown animation, corpse removed after 10 s; ragdolls later).
3. Knife: hit after the 0.45 s delay, range 70, the cone rule, same damage
   path.
4. Log every hit: weapon, target, distance, damage, headshot, remaining
   health and head health. Test with scripted input against a paused Clot.

Not covered yet: blood and impact effects, muzzle flash, sound, perks, zed
flinch animations, ragdolls, the Clot's grab slowing the player.

**Findings.**
- KF zeds have a second, "extended" collision cylinder at head height
  (bUseExtendedCollision; Clot: ColOffset (0,0,48), ColRadius 25, ColHeight
  5), fixed to the actor at Location + (ColOffset >> Rotation). Without it,
  level shots from the player's eye (95 above the floor) pass over the Clot's
  main cylinder (top at 88). Shots test both cylinders.
- The Clot's head centre is ~15 units below the player's eye, so level shots
  miss the head sphere (7.7 radius) and hit the body. You aim down for
  headshots, as in KF. Logged per hit as `headshot_check`.
- The melee start distance must be MeleeRange + both radii (66). My first
  version added a 10-unit margin, so the Clot stopped at 76, outside the
  damage check (MeleeRange x 1.4 + radii = 74), and never hurt the player.
- Int-typed defaults (Health) need reading as integers; first read as 100
  (fallback) instead of 130.
- KnockDown ends standing up (KF uses ragdolls for death). The death pose
  plays to the frame where the root bone is lowest (frame 50 for the Clot)
  and holds there. Decapitation collapses the head bone and its children
  into the head bone origin.
- HeadBone 'head' is matched to 'CHR_Head' by suffix (`find_bone`); the
  mesh's own alias table is not read.

## Play-test fixes after combat (planned 2026-10-03)

Feedback: "no collision with enemies", "head shots are hard because there's
no ADS".

**Step 1: pawn collision.** In Unreal, pawns block each other as upright
cylinders. The player and every living zed get a cylinder test before each
world move. The horizontal part of the move is clipped where it would enter
another cylinder (a swept circle test, in 2D) and the rest slides along that
cylinder's side. The world sweep then runs on the clipped move, so walls
still win. Zeds test against the player and against each other. Corpses do
not block (KF uses ragdolls, which do not block pawns). Standing on top of a
zed is not handled; cylinders only overlap vertically if their heights
overlap. Each block is logged (`pawn_blocked`).

**Step 2: iron sights (ADS, aim down sights).** Values from the class
defaults and scripts:
- Right mouse toggles (KFWeapon.ToggleIronSights). Only weapons with
  bHasAimingMode (the 9mm, not the knife). Not allowed while falling,
  reloading, or switching weapons. Reloading or switching drops out of it.
- Zoom over ZoomTime 0.25 s: player FOV to PlayerIronSightFOV 75
  (horizontal at 4:3, converted like DefaultFOV), weapon DisplayFOV to
  ZoomedDisplayFOV (Single: 65).
- Animations: idle Idle_Iron (IdleAimAnim), fire Fire_Iron (FireAimedAnim).
- Spread x 0.5 while aiming (KFFire.GetSpread).
- Weapon position: the native code that moves the weapon into the sights is
  not in the scripts. Guess: the draw offset (PlayerViewOffset) blends to
  zero, because the iron animations are made to line the sights up with the
  view at offset zero. This is not confirmed and may need adjusting from a
  screenshot. ZoomInRotation (a small roll during the zoom) is left out for
  now.
- No movement slowdown while aiming in the scripts.

**Findings (both steps implemented 2026-10-03).**
- Pawn collision: the player stops 47 units from a Clot's centre (20 + 26 +
  0.5); three converging Clots keep 52 apart.
- The offset-to-zero guess for the aimed weapon was right: with the draw
  offset at zero, Idle_Iron puts the 9mm's front sight at the screen centre
  (screenshot, 5 px off at 2556x1382).
- The class-defaults finder accepted struct-member names (Range's `Min`,
  `Max`) as top-level property names, so for 231 classes it locked onto a
  false start and read 1-3 garbage properties first (KFMod.SingleFire lost
  FireAimedAnim; Engine.Ladder read `VSize = true`). Now only properties
  declared by classes count. All 40 maps load identically before and after.
  `kfpkg defaults --all` lists the start and count for every class.

**Dead zeds stay standing (investigated 2026-10-03, left for later).**
KnockDown is a stagger-and-crouch animation, not a fall: held at its lowest
frame (50) the Clot is crouched on its feet, which looks frozen. KF never
animates deaths; KFMonster.PlayDyingAnimation starts a Karma ragdoll. The
ragdoll skeletons are in `KarmaData/*.ka` (format not read yet); Clot
defaults include RagdollLifeSpan 30, RagDeathVel 100, RagShootStrength 200,
RagSpinScale 7.5. Options: real ragdolls (decode .ka, one physics body per
bone with joints, skin from the bodies), or a quick scripted topple. You
chose to leave it for now.

## Gore and ragdolls (milestone 6, planned 2026-10-03)

**Order.** 1. Ragdolls for the Clot. 2. Severed heads and limbs. 3. Blood (simple
splats and spurts; the full particle-emitter system later). 4. Other zeds.

**What KF does on death** (KFMonster.PlayDyingAnimation): stops animating and
hands the skeleton to a Karma ragdoll named by the class's KFRagdollName
(Clot: "Clot_Trip"). Start velocity = 0.6 x the zed's horizontal velocity +
RagDeathVel (100) x shot direction (+ RagDeathUpKick, 0 for the Clot). Spin =
RagInvInertia (4) x (hit offset, scaled sideways by RagSpinScale 7.5, capped at
RagMaxSpinAmount 100) cross the start push. Body parameters (KFMonster.PawnKParams):
linear damping 0.15, angular damping 0.05, friction 1.3, restitution 0.2.
LevelInfo: KarmaTimeScale 0.9, KarmaGravScale 1. Corpses last RagdollLifeSpan
30 s.

**The ragdoll files.** `KarmaData/*.ka` are XML. `KF_Characters_Trip.ka`
holds one ASSET per zed (Clot_Trip, GoreFast_Trip, Crawler_Trip, Stalker_Trip,
Bloat_Trip, Siren_Trip, Burns_Trip = Husk, Scrake_Trip, FleshPound_Trip,
Patriarch_Trip). An asset has:
- `scale` 0.02: file units x 50 = mesh units (to be confirmed against bone
  lengths, see step 1 checks).
- GEOMETRY per bone: PRIMITIVEs, `sphere` (RADIUS) or `sphyl` (capsule:
  RADIUS, HEIGHT = length of the straight part, axis = the primitive's local
  Z), each with a 4x4 TM (row-major, rows = axes, last row = position) in the
  bone's frame.
- MODEL: which geometry a part uses, and its MASS.
- PART: one physics body per listed bone (the Clot: 16, pelvis is the root).
  Bones not listed (head, hands, feet, fingers) ride along with the nearest
  listed ancestor.
- JOINT between a part and its parent: `skeletal` (ball joint: elliptical
  swing cone CONE_HALF_ANGLE_X/Y around PRIMARY_AXIS, twist +-TWIST_HALF_ANGLE)
  or `hinge` (rotation about PRIMARY_AXIS between LOW_LIMIT and HIGH_LIMIT).
  POS1/2 and the axes are given in each part's own frame.
- NO_COLLISION pairs: parts that must not collide with each other.

**Step 1 plan.**
1. `ue-assets/src/karma.rs`: parse the .ka XML (small hand-written reader,
   no new dependency) into assets, parts, primitives, joints. `kfpkg karma`
   lists every asset with counts. Unit test on a small inline sample.
2. Check the file against the mesh before simulating anything: for every joint,
   POS2 x 50 should equal the child bone's bind position in the parent bone's
   frame, and each part name should match a mesh bone. Log mismatches.
3. On death: one avian dynamic body per part, placed from the current
   animated pose; colliders from the primitives; joints with KF's limits.
   Approximations: avian's swing limit is a circle, so the elliptical cone
   uses the larger half-angle; no joint stiffness/damping values; ragdoll
   parts do not collide with each other at first (simpler, more stable).
4. Ragdoll bodies collide with the level only. They go on their own collision
   layer so player movement, zed movement and bullets ignore them.
5. Each frame: bone transforms from the bodies (unlisted bones keep their
   offset from their ancestor part), then skin the mesh as usual.
6. Launch with KF's start velocity and spin. Gravity 950 units/s^2.
7. Log per corpse: pelvis height, speed, and when it comes to rest. Check by
   screenshots after the kill.

**Step 1 findings (2026-10-03).**
- The .ka bone frames are exactly the mesh's bone frames (anchor gaps 0.00,
  axes 0.0 degrees in the bind pose), and file units x 50 = mesh units.
- Hinge limits apply as written in avian's angle convention (walk-cycle check).
- avian measures ball-joint swing and twist around
  `twist_axis.any_orthonormal_vector()` (Y for X), not around twist_axis as
  its docs say; joint frames are built to match the code.
- avian joints need the `xpbd_joints` cargo feature; without it they compile
  and do nothing.
- The file's INERTIA values are implausibly small (forearm radius of gyration
  0.2 units, collars 0); inertia comes from the collision shapes instead.
- KF's Clot has NO_COLLISION for all 120 part pairs, so parts never collide
  with each other, which matches using one collision layer for ragdolls.
- The ribcage-spine3 joint pulled apart because the file gives chr_spine3 a
  placeholder mass of 3.14e-6: a near-massless link between heavy bodies
  cannot pass joint corrections along. Part masses are raised to at least
  0.05 file units (typical parts are ~0.1). Fixed; see `STATUS.md`.
- TWIST_TYPE (2026-10-03, after your play test): 2 is treated as locked
  twist, 1 as limited, 0 as free, after MathEngine's MdtSkeletal twist
  options as I remember them (not confirmed from KF's files). Type-1 joints
  carry deliberate small angles (Clot spine1 0.2 rad); type-2 joints mostly the
  unused default 1.570796. Reading type 2 as +-90 degrees let necks and spines
  twist to 90 degrees each. CONE_TYPE is 2 everywhere; meaning unknown
  (possibly a "slot" cone); treated as a cone.
- Corpses sleep below 0.3 m/s and 1.0 rad/s (avian default 0.15/0.15): the
  near-locked limits make the solver chatter, which kept parts turning at
  ~0.5 rad/s and never let a corpse sleep (the jiggle).
- Deviations from KF, all deliberate: inertia from shapes; minimum part
  mass; circular swing cones; rigid limits (Karma's were springs, stiffness 1000);
  higher sleep thresholds; spin capped at KMaxAngularSpeed because
  RagInvInertia's units are unknown; KarmaTimeScale 0.9 not applied.

## Decapitation and bleed-out (planned 2026-10-03)

You pointed out that headless zeds keep going for a while (it is how you stop
a Bloat exploding). KFMonster confirms it:
- **Head removal** (TakeDamage): on a headshot, HeadHealth -= damage (already
  multiplied by HeadShotDamageMult); the head comes off if HeadHealth <= 0 or
  the damage exceeds the remaining Health.
- **RemoveHead**: the head "explodes" for LastDamageAmount + 0.25 x HealthMax
  extra damage. That goes back through TakeDamage with the zed already headless,
  so it is multiplied by HeadShotDamageMult again. If the zed survives,
  BleedOutTime = now + BleedOutDuration (KFMonster 5 s; Bloat and Scrake 6,
  Fleshpound 7). Also: GroundSpeed x 0.8, aim worse, 'Claw3' attacks replaced by
  Claw2 / Claw1 ("no head so biting is out"), walk animations replaced by
  HeadlessWalkAnims (WalkF_Headless ...), and HitF plays on the upper body
  (animation channel 1, blended from SpineBone1 = CHR_Spine2 up).
- **Every later hit** on a headless zed counts as a headshot for damage
  (`bDecapitated || bIsHeadShot`).
- **Tick**: once BleedOutTime passes, the zed dies (DamTypeBleedOut). With no
  hit momentum, the ragdoll gets no push (KF picks a random spin; not copied).
- Example, 9mm (about 30 after the 1.1 multiplier) headshot on a Clot (130):
  head off, extra (30 + 32.5) x 1.1 = 69, total ~99, so ~31 health left; the
  headless Clot walks on for 5 s, then drops.

**Steps.** 1. Damage rules and bleed-out timer, logged (`zed_decapitated`,
`zed_bled_out`). 2. Headless walk animations, speed x 0.8, Claw3 swap.
3. HitF on the upper body: a second animation layer for bones from
CHR_Spine2 up. Not covered: aim penalty (zeds have no ranged attacks yet),
sounds.

## Clot animation sweep (2026-10-03)

All 43 sequences in the Clot's animation set, where KF uses them (class
defaults up the chain KFChar.ZombieClot -> ZombieClotBase -> KFMonster ->
Old2k4.Skaarj -> Monster, and the scripts), and whether we do.

Used by us: ClotWalk, Idle_LargeZombie, ClotGrapple / Two / Three, HitF (on
decapitation), WalkF_Headless. KnockDown only as the fallback when a class has
no ragdoll.

Used by KF, not by us yet:
- **Body-hit flinch** HitReactionF/B/L/R (KFHitFront/Back/Left/Right):
  KFMonster.PlayTakeHit -> PlayDirectionalHit for hits of 5+ damage, at most
  every MinTimeBetweenPainAnims (0.5 s), picked by the hit direction relative
  to the zed's facing. Played on the upper body (DoAnimAction: channel 1 from
  SpineBone1), so the legs keep walking.
- **Stun** HitF / HitF2 / HitF3 (HitAnims, random): instead of HitReactionF
  for a hit from the front if damage >= half the default health (65 for the
  Clot), or a melee hit from within 2 x MeleeRange (40) doing over 10% of
  default health (13): the knife (19) qualifies. bSTUNNED for StunTime (1 s):
  CanAttack is false. StunsRemaining -1 = unlimited.
- **Knock-down** KnockDown (FlipOver): a hit doing more than Health / 1.5
  (KFMonster.PlayHit), or a melee hit while the player moves faster than 300.
  Full body; the controller waits for the animation to end.
- **Clot grab details** (ZombieClot): the grapple plays on the upper body from
  FireRootBone (CHR_Spine1), and during attacks the Clot keeps accelerating
  toward its target, so it keeps walking while grabbing; the attack is picked
  at random (Rand(3)); a landed grab pins the player for GrappleDuration (1.5 s)
  unless the Clot is headless; the grab animation stops if the player gets out
  of reach.
- **Headless Clot attacks** Claw, Claw, Claw2 (ZombieClot.RemoveHead), with
  MeleeDamage x 2 and MeleeRange x 2; any grab is released.
- **Falling** InAir (AirAnims), Landed (LandAnims), Jump2 (AirStillAnim /
  TakeoffStillAnim), from Old2k4.Skaarj defaults.
- **Turning in place** TurnLeft / TurnRight (KFMonster TurnLeftAnim /
  TurnRightAnim), played by the engine when an idle pawn turns.
- Needs features we do not have: WalkB/L/R_Headless, RunR (sideways and
  backwards movement; our zeds only walk forward), WalkF/B/L/R_Fire (burning,
  needs fire weapons), DoorBash (needs doors), ClotPunt (kicking physics
  objects), ZombieFeed (feeding on a dead player; we respawn instantly).

Implemented 2026-10-03/04 in this order: hit flinches, stun and knock-down;
grab fixes; headless claws; falling, landing and standing/turning. Correction
to the stun note above: the close-melee stun cannot happen to a Clot in
practice (centres stay 46 apart, the rule needs 40).

Not used by any script or default: HeadLoss, Idle_Headless, ClotWalk2, RunL,
Jump, DodgeF/B/L/R (KFMonster bCanDodge = false), walkF_FireTwo,
WalkF_FireThree (all three BurningWalkFAnims entries are WalkF_Fire).

## Gorefast (steps 1-2 implemented 2026-10-04)

**What the data says.** `KFChar.ZombieGorefast_STANDARD` -> `ZombieGoreFast`
(the script, `KFChar/ZombieGoreFast.uc`) -> `KFMod.ZombieGoreFastBase` (the
defaults) -> `KFMonster`. Mesh `KF_Freaks_Trip.GoreFast_Freak`, Health 250,
HeadHealth 25 (KFMonster), HeadScale 1.5, HeadHeight 2.5, GroundSpeed 120,
MeleeDamage 15, MeleeRange 30, DrawScale 1.2, PrePivot (0, 0, 10),
RotationRate yaw 45000, walk `GoreWalk`, idle `GoreIdle`, attacks
GoreAttack1, GoreAttack2, GoreAttack1, extended collision (offset Z 52,
radius 25, height 10), ragdoll `GoreFast_Trip` (15 parts, in
KF_Characters_Trip.ka). No grab, no headless claw switch (those are
ZombieClot only). It has no left forearm (bLeftArmGibbed, HideBone).

**The Gorefast's own rule: running** (`ZombieGoreFast.RangedAttack`,
`state RunningState`):
- When it wants to attack but cannot yet (not in reach), has its head and is
  within 700 units of the target, it starts running: GroundSpeed x 1.875
  (225) and walk animation `ZombieRun`.
- While running, it checks every 0.5-1.0 s (random) whether the target is
  still within 700; if not, it goes back to walking.
- An attack while running: with ChargeChance (0.2 at Normal difficulty,
  GameDifficulty 2) it is a moving attack (`ClawAndMove`): GoreAttack1 or
  GoreAttack2 on the upper body from FireRootBone, and it keeps running at
  the target until a timer of GoreAttack1's length runs out, then walks.
  Otherwise a normal full-body attack, and it stops running.
- Losing its head stops the running for good (bDecapitated).
- Assumption (not in the scripts; the AI controller's timing is native
  code): "wants to attack" is checked every frame while chasing.

**Steps.**
1. Load the Gorefast and let you spawn it: G key, scripted action
   `gorefast`, and `--gorefast` at start. The Clot-only rules (grab on the
   upper body, pin, headless claws) become per-class flags instead of name
   checks. It walks, attacks, flinches, loses its head and ragdolls with the
   shared rules. Check: load log (sequences, bones, ragdoll report), a
   scripted run and screenshots.
2. Running and charge attacks as above. Check: logs of state changes, speed
   and attacks in a scripted run.

## Gore steps A-C (steps A-B implemented 2026-10-04)

Order agreed with you: A. gun decapitation (neck stump, brain chunks);
B. severed limbs and the knife's flying head; C. simple blood.

**What KF does** (KFMonster.DoDamageFX -> ProcessHitFX -> DecapFX):
- The hit that takes the head off records a decapitation effect.
  DamTypeDecapitation is for guns, DamTypeMeleeDecapitation for melee. The
  head bone is then hidden (bone scale 0), which we already do.
- Gun (`DecapFX(.., bSpawnDetachedHead=false)`): HideBone(HeadBone) spawns
  `ROEffects.SeveredHeadAttachment` (skeletal mesh
  `Gear_anm.NeckAttach_Gore`, its Skins from the class defaults, DrawScale
  SeveredHeadAttachScale 1) attached to the mesh tag `neck`, plus the
  particle effect DismembermentJetHead (step C). Then three brain chunks
  (KFGibBrain, KFGibBrainb, KFGibBrain) at the head bone, and BrainSplash
  (particles, step C).
- Melee: a separate flying head (DetachedHeadClass) instead: step B.
- Brain chunks (`KFSpawnGiblet`, KFGib, Old2k4.Gib): static meshes
  `KillingFloorStatics.Gib1` / `Gib2`, DrawScale 0.3 x (CollisionRadius x
  CollisionHeight) / 1100. They spawn at the head bone, rotated as the zed
  with pitch, yaw and roll each jittered by up to +-0.06 x 32768. Velocity =
  the zed's velocity + that rotation's up axis x (250 + 0-25%). They fall at
  950 and spin at a random rate (RandSpin 64000, i.e. up to +-64000 rotation
  units/s per axis). On hitting anything: velocity = 0.4 x the reflected
  velocity (DampenFactor), new random spin (100000); below 20 units/s they
  stop. They last 6 s (LifeSpan). Collision cylinder radius 5, height 2.5.
- **Mesh tags.** `neck`, `head`, `rarm`, ... are not bone names but
  aliases stored in the skeletal mesh (UE2 TagAliases / TagNames /
  TagCoords). Found by scanning the mesh data (`kfpkg meshtags`). Clot: 16
  tags (`neck` -> CHR_Neck at offset (-1, 0, 0), `head` -> CHR_Head, `rarm`
  -> CHR_RArmUpper, `lthigh` -> CHR_LCalf, ...). Gorefast: 15, including
  `lfarm` -> CHR_LArmForeArm, a bone its mesh does not have. Each tag has an
  offset frame (origin + 3 axes) in its bone's space.

**Step A plan.**
1. `ue-assets`: read the tag table (aliases, bones, frames) from skeletal
   meshes; `SkinnedModel` keeps the tags and can give a tag's frame for a pose.
2. `gore.rs`: load the stump model and the two gib meshes at startup.
3. On a gun decapitation: give the zed a stump, placed every frame at its
   `neck` tag (animated or ragdoll), and spawn the three chunks.
4. Gib movement: our own small simulation (as KF's, not Karma): gravity,
   bounce with damping off the level, stop below 20, despawn after 6 s.
   Approximation: collision is a sphere of radius 2.5, not the 5 x 2.5
   cylinder.
5. Logs: `decap_fx` (where, how many chunks), `gib` (spawn velocity, each
   bounce, rest position, removal). Screenshots of the stump.

Assumption to check by screenshot: an attached actor does not take on the
zed's DrawScale (the stump is drawn at its own scale 1).
Answered in step B's research: the stump scale is set per zed class
(SeveredHeadAttachScale: Clot 0.8, Gorefast 1.0), so step A drew the Clot's
neck stump 25% too large. Fixed in step B.

**Step B: what KF does** (KFMonster.TakeDamage -> DoDamageFX ->
ProcessHitFX -> HideBone / SpawnSeveredGiblet):
- The hit bone comes from native `CalcHitLoc(HitLocation, HitRay)`. Its name
  is compared with the mesh tags (`lthigh`, `lfarm`, `lleg`, `righthand`, ...),
  so the name is the tag of the bone hit.
- Only on the killing hit: neck -> head; lfoot / lleg -> lthigh;
  righthand / rshoulder / rarm -> rfarm (left the same); spine / none ->
  FireRootBone. The limb comes off with chance |Health - Damage x
  GibModifier| / 130 (Health after the hit, GibModifier 1 for the 9mm and
  knife). Only lthigh, rthigh, lfarm, rfarm can come off; other bones
  (spine, head) do nothing. A decapitating hit never severs a limb.
- A severed limb: the piece (DetachedLegClass / DetachedArmClass) flies from
  the tag's position, rotated as the bone. Direction: the Z axis of
  HitNormal = Normal(Normal(attacker - hit) + VRand() x 0.2 + (0, 0, 2.8)),
  jittered by GibPerterbation 0.25 (x 32768). Speed 100-150 (MaxSpeed 100),
  plus the zed's velocity. Brain chunks too: 3 for a leg, 2 for an arm, at 250.
  The limb's bone is scaled to 0 (`lthigh` = CHR_LCalf: the leg goes from the
  knee down; `rfarm` = CHR_RArmForeArm: from the elbow), and a stump
  (`ROEffects.SeveredLegAttachment` / `SeveredArmAttachment`, meshes
  `Gear_anm.LegAttach_Gore` / `ArmAttach_Gore`) goes on tag `lleg` / `rarm`,
  scaled by SeveredLeg/ArmAttachScale (Clot 0.8, Gorefast 0.9).
- The Gorefast has no left forearm (bLeftArmGibbed), so `lfarm` never comes off.
- Knife decapitation (DamTypeMeleeDecapitation -> SpecialHideHead): the neck
  stump without brain chunks, and the head (DetachedHeadClass) flies off from
  the head bone along HitNormal's Z axis (jitter 0.06) at 100-150, +50 upward.
- Pieces (ROEffects.SeveredAppendage): fall, bounce keeping 35% of their speed
  (DampenFactor 0.35), random spin, stop below 20 and then lie flat (pitch =
  the floor's pitch + 16384), last 8 s.
- Pieces per zed: Clot arm `KF_Gibbs_Trip.Clot_Arm` (a one-bone skeletal
  mesh), leg `kf_gore_trip_sm.limbs.Clot_Leg_resource`, head
  `kf_gore_trip_sm.gibbs.clothead`; Gorefast `gorefast_arm_resource`,
  `gorefast_leg_resource`, `kf_gore_trip_sm.heads.gorefasthead`.

**Step B plan.**
1. Hit bone: the bone segment (bone to its first child) nearest the shot's
   line, named by its tag if it has one. An approximation of native code.
2. Combat records each damage event (damage, health after, melee, hit
   point, shot direction, attacker). The zed system turns them into
   effects: gun decapitation (step A), knife decapitation, or a limb roll on
   the killing hit.
3. Hidden bones become a list (head, severed limbs), applied to the
   animated mesh and the ragdoll. Stumps become a list (neck, arms, legs).
4. Pieces share the chunks' movement code, with their own bounce, life and
   landing rule.
5. Logs: `gore_hit_bone` (bone, tag, distance to the line), `limb_roll`
   (chance, roll), `limb_severed`, `piece_*`. Screenshots.

Not covered: shooting corpses to sever more limbs (our bullets pass
through corpses), explosion gibbing (no explosives yet), particles and
sounds.

## Gore step C: particle effects (C1-C4 implemented 2026-10-04)

You chose the faithful route: read KF's own particle effects instead of
drawing look-alikes.

**What the data says.** A KF effect (e.g. `KFMod.DismembermentJetHead`) is
an `Emitter` actor whose `Emitters` default lists sub-emitter objects
(`SpriteEmitter`, `MeshEmitter`, ...) stored in the same package. Each is a
plain property list (texture, lifetime, size and colour over life, start
velocity, acceleration, spawn rate, ...); our property reader already reads
all 331 SpriteEmitters in KFMod.u with no failures. The *simulation* is
native engine code, so it is rebuilt from what the properties mean
(UE2 particle system; behaviour from memory where not obvious, labelled).

Gore effects and where KF uses them:
- `DismembermentJetHead` (9 sprite + 5 mesh emitters): gun decapitation
  neck (NeckSpurtEmitterClass, attached at `neck`).
- `DismembermentJetDecapitate` (7 sprite + 1 mesh): knife decapitation
  (NeckSpurtNoGibEmitterClass).
- `DismembermentJetLimb` (5 sprite): severed limb stumps (LimbSpurtEmitterClass).
- `BrainSplash` (2 sprite): at the head on decapitation.
- `ROBloodSpurt` (1 sprite): trail on flying severed pieces
  (BleedingEmitterClass). `BloodTrail` (1 sprite): trail on brain chunks.
- `ROBloodPuff*`: bullet hits on flesh (from the damage type's hit effects;
  to be checked).

49 properties are used by these; textures from `kf_fx_trip_t.Gore` and
`Effects_Tex`; meshes `kf_gore_trip_sm.gibbs.Brain_Chunk_1..3`, `eyeball`,
`EffectsSM.PlayerGibbs.Chunk1_Gibb`.

**Steps.**
- C1. `ue-assets`: read an Emitter class into a list of emitter
  definitions (each property with the engine's class defaults filled in;
  structs and arrays decoded). `kfpkg emitter CLASS` prints one. Check:
  all gore effects read, values match the property dumps.
- C2. A particle system in Bevy for sprite emitters: spawning
  (InitialParticlesPerSecond / ParticlesPerSecond, MaxParticles,
  RespawnDeadParticles, AutomaticInitialSpawning), start location /
  velocity / size / spin ranges, acceleration, velocity loss, lifetime,
  fade in / out, ColorScale and SizeScale over life, texture
  subdivisions, draw style (alpha / translucent / modulated), facing
  (UseDirectionAs), local vs world coordinates, AddLocationFromOtherEmitter.
  Effects attached to a bone tag follow it. Logs: live particles per
  emitter, effect start / end. Wire up the neck, limb and BrainSplash
  effects first.
- C3. Mesh emitters (the meat bits in the neck jets).
- C4. Trails on flying pieces and chunks; bullet-hit blood puffs.
- Later: floor and wall blood splats (projected decals, a separate engine
  feature).

**Findings.** UE2's Modulated draw style is 2 x source x destination: KF's
blood textures are dark red on a 127 grey background (= no change). Drawn
here as Bevy's multiply blend with the texture colour doubled at load.
UseColorScale is off on every gore emitter, so their ColorScale curves are
unused. Independent-coordinate emitters use world axes for velocity (the
head jet's chunks fly up world Z); Relative ones move and turn with the
effect, whose frame is the bone tag's (AttachEmitterEffect sets a zero
relative rotation).

## Gore step D: blood decals (implemented 2026-10-04)

**What KF does.** Blood on walls and floors is UE2 Projectors (a texture
projected onto level geometry inside a box), all ProjectedDecal subclasses:
- Projector: FrameBufferBlendingOp PB_Modulate (multiply the framebuffer;
  the textures have a ~120 grey background, so 2x as for particles),
  projects onto BSP, static meshes and terrain, not actors.
- ProjectedDecal: PushBack 24 (the box starts 24 units in front of the
  spawn point), MaxTraceDistance 60 (box depth), RandomOrient (random roll),
  FadeInTime 0.125, LifeSpan = class LifeSpan - 1 (Rand(1) is always 0),
  then AbandonProjector(LifeSpan).
- ROBloodSplatter: Splatter_001..006 (256 px), DrawScale 0.25, LifeSpan 20.
  ROSmallBloodDrops: Drip_001..003 (128 px), DrawScale 0.15, LifeSpan 20.
  KFBloodSplatterDecal: KFX.BloodSplat1..3 (512 px), DrawScale
  Rand(2) - 0.65 (so 0.35 or -0.65), LifeSpan 10. KFBloodStreakDecal:
  KFX.BloodStreak, DrawScale Rand(2) - 0.6, PushBack 5, no random roll.

When:
1. Every hit (KFMonster.TakeDamage, bCausesBlood; a hit within 0.2 s of the
   last one only 20% of the time): ProjectileBloodSplat traces 350 units
   along the shot from the hit point; on a wall, an ROBloodSplatter at
   WallHit + 20 x (WallNormal + VRand()), facing into the wall.
2. Severed pieces (SeveredAppendage.HitWall): ROSmallBloodDrops where they
   bounce fast (over MaxSpeed / 3) and where they stop.
3. Brain chunks (KFGib.HitWall): when one stops, KFBloodPuff; 20% of the
   time it traces 350 down and puts a KFBloodSplatterDecal on the floor.
4. Ragdolls (KFMonster.KImpact): a KFBloodStreakDecal where a body part hits
   a surface, at most every BloodStreakInterval (0.25 s) and only after
   moving 0.75 m since the last one.

**Plan.** Decal meshes are built on the CPU: level triangles inside the
box (a grid over the collision geometry), clipped to the box, facing the
projector, textured by projecting onto the box's Y/Z plane, drawn with the
particles' 2x modulate material, faded in, removed at the end of their
life. Approximations, labelled: an orthographic box (FOV ignored; FOV is
0-6 degrees), the size from texture size x |DrawScale| (a negative scale
read as mirrored), only collision geometry (non-blocking decorative meshes
get no decals), the end of life is a 1 s fade (assumed).
Steps: D1 decal library and projection, with hit splats on walls (1);
D2 drips and floor splats (2, 3); D3 ragdoll streaks (4).

## Pathfinding (P1-P4 implemented 2026-10-04)

**What the map has.** UE2 maps carry a precomputed navigation network.
KF-WestLondon: 210 PathNodes, 21 JumpSpots, 20 ZombiePathNodes, 6
PlayerStarts (all NavigationPoints, each with a `PathList` of outgoing
ReachSpecs) and 1506 ReachSpecs. A ReachSpec (Engine/ReachSpec.uc) has
Start, End, Distance, CollisionRadius / CollisionHeight (the largest pawn
that fits) and reachFlags: R_WALK 1, R_FLY 2, R_SWIM 4, R_JUMP 8, R_DOOR
16, R_SPECIAL 32, R_LADDER 64, R_PROSCRIBED 128, R_FORCED 256,
R_PLAYERONLY 512.

**What KF's zeds do** (KFMonsterController, state ZombieHunt; MonsterController
state Hunting): loop { PickDestination; MoveToward(MoveTarget) or
MoveTo(Destination) }, so they re-plan each time they reach a target.
- While hunting, a zed's collision is shrunk to 24 x 44 (bigger zeds fit
  the same paths).
- PickDestination: if the enemy is directly reachable (native
  ActorReachable), walk straight to it. Otherwise FindBestPathToward(Enemy).
- FindBestPathToward: MoveTarget = FindPathToward(Enemy) (native shortest
  path over the ReachSpecs; RouteCache holds the route). If RouteCache[1] is
  directly reachable, go to it instead (shortcut). If another zed stands
  between the zed and MoveTarget, that node gets +200 cost and the route is
  re-planned. If the same MoveTarget comes up more than 3 times running, 60%
  of the time it is marked blocked (+10000) and the route re-planned.
- Not copied: the initial FindRandomDest wander, doors (welding, bashing;
  our doors do not move or block), LastSeenPos hunting when no path exists.

**Plan.**
- P1. `ue-assets`: read the navigation network (every actor with a
  PathList; its ReachSpecs). `kfpkg nav MAP` reports node classes, edge
  flags, connected groups. Check on several maps.
- P2. Load it with the map (Bevy space); check each walk edge against our
  collision (a cylinder sweep) and log how many are blocked, to see how
  well our collision matches Unreal's.
- P3. Zed routing: "directly reachable" = an unobstructed sweep of the
  zed's cylinder (24 x 44) to the target at step height, with floor under
  it the whole way (our stand-in for native ActorReachable). Route search
  = Dijkstra from the nav points the zed can reach directly to those that
  can reach the player directly, over edges with flags walk / forced (no
  jump, fly, swim, ladder, special, proscribed, player-only) and sizes
  fitting 24 x 44. Then KF's shortcut, zed-avoidance and stuck rules.
  Re-plan on reaching the target and at least every 0.5 s (assumed; KF
  re-plans when a move ends or times out).
- P4. Test: a scripted spawn behind an obstacle (`zed_at` test action with
  a position), logs of route, target nodes and progress; screenshots.

**Findings.** All 40 maps' networks read. On KF-WestLondon our walk test
agrees with 1183 of 1248 usable links; the rest are jump pads, holes in our
collision, or places our collision blocks. The upper level reaches the street
only through JumpSpots (jumping down over a wall), so zeds need KFMonster's
JumpZ (320) to use it. KF's "same target 3 times" rule cannot catch a zed
alternating between two points; our addition gives up a link after two
failed moves along it (until zeds can jump). Some sizes: KF shrinks zeds over
27 x 46 to 24 x 44 when hunting; the Clot and Gorefast keep 26 x 44, so the
walk test uses the zed's own hunting size.

## Collision layers (2026-10-04)

Unreal blocking volumes are not walls for everything. BlockingVolume
defaults: bBlockActors (movement), bBlockKarma (ragdolls), but not
bBlockZeroExtentTraces (so bullets pass). With bClassBlocker only the
classes in BlockedClasses are blocked; KF's KFZombieZoneVolume ("blocks
ONLY humans") lists KFHumanPawn, so zeds walk through. Our layers: World
(BSP, meshes, terrain: everything), Blocking (plain volumes),
PlayerBlocking / ZedBlocking (class blockers, by the class chains
KFHumanPawn-KFPawn-xPawn-... and KFMonster-Skaarj-Monster-xPawn-...),
TraceBlocking (volumes that do block traces), Ragdoll. Filters: player
movement, zed movement, bodies (ragdolls, gore pieces), traces.

## All specimens (planned 2026-10-04)

KF's ten specimens are `KFChar.Zombie*_STANDARD` classes (KFChar script,
KFMod `Zombie*Base` defaults, KFMonster below). Meshes: KF_Freaks_Trip
(Clot, Gorefast, Crawler, Stalker, Bloat, Siren, Scrake, FleshPound,
Patriarch) and KF_Freaks2_Trip (Husk: Burns_Freak).

**Order.**
- S1. Load all ten with the shared rules we already have (stats, walk,
  melee from MeleeAnims, flinches, stun, decapitation and bleed-out,
  ragdolls, severed parts, gore effects, routes). N cycles through them.
  Check: load log per class (sequences, ragdoll, pieces), a scripted spawn
  of each with a screenshot.
- Then each one's own script, smallest first, one step each:
  Crawler (pounce: a leap attack), Stalker (cloak until close), Scrake
  (chainsaw, rages and runs when hurt), Fleshpound (rage: charges when
  damaged), Bloat (bile vomit, bursts on death), Siren (scream that hurts
  in a radius), Husk (fireball projectile), Patriarch (boss: cloak,
  chaingun, rockets, heals; the largest script).

**Fleshpound (done 2026-10-04).** ZombieFleshPound + FleshpoundZombieController:
non-explosive damage x 0.5. Health lost within 2 s of the previous hit adds
up (TwoSecondDamageTotal); over RageDamageThreshold 360, with the head on,
he plays PoundRage standing still (state `Enraging`), then charges for
5-11 s (Normal): GroundSpeed x 2.3, PoundRun, every attack FPRageAttack at
x 1.75, the chest device swapped to FPRedBloomShader. A landed hit ends the
rage. Chasing 10-15 s without attacking (RageFrustrationThreshhold) starts
a frustrated rage that only a hit ends. PoundAttack1/2 are repeated-hit
animations in KF (x 0.5 / x 0.25 per hit); we land one hit, so they do
less than KF in total (a known difference). No flipping over.

**Bloat (done 2026-10-04).** ZombieBloat, KFBloatVomit (Old2k4.BioGlob),
KFPawn bile. Out of melee reach, within 250 units and with his head he
plays ZombieBarf (standing, or with ChargeChance 0.4 on the upper body
while walking). Timing comes from the animation's own notifies, which the
mesh reader now reads (`Sequence::notifies`; `kfpkg notifies`):
AnimNotify_Effect at 0.424 starts ROEffects.KFVomitJet on CHR_Head (offset
and rotation from the notify), AnimNotify_Script SpawnTwoShots at 0.444
fires three globs (`src/vomit.rs`): speed 400 with gravity, from 30 ahead
and 64 up, aimed at the player, side ones at +-1200 yaw. A glob touching
the player does HurtRadius(4, 120) and flies on; reaching the level it
leaves a VomitDecal and blows up for 3 + 4 = 7 within 120. HurtRadius
scales by 1 - (distance - 20) / 120, whole points. Any vomit damage sets
the player's BileCount to 7: every 0.5 s, 2-4 damage. On death (not a
headless bleed-out) BileExplosion(Headless) plays, SpineBone2
(CHR_Spine3) and everything above it is hidden, and BileJet throws 4 globs
up (11 degrees from vertical, random direction). Hits never interrupt his
attacks; no flipping over.
Particle change made for the vomit jet: world-space emitters now turn their
start velocity with the effect (before, velocities stayed in world axes);
PTDU_Up sprites (stretched along their velocity) are drawn.
Known gaps: the glob's own look (plain lit material), VomGroundSplash (an
xEmitter, old particle system), leading a moving target, vomit hurting
other zeds.

**Siren (done 2026-10-04).** ZombieSiren. The Bloat's vomit and the
Siren's scream share one "ranged attack" path (animation, distance, the
SpawnTwoShots notify times, effect notifies, chance to play on the upper
body). RangedAttack is tried every 0.1 s in KF (MonsterController
FireWeaponAt returns false -> SetTimer(0.1)), so she screams back to back
while within ScreamRadius 700, out of bite reach and with her head.
Siren_Scream plays on the upper body from SpineBone1 (so do her bites); her
Tick keeps her walking at GroundSpeed x 0.65 during any attack. Six
SpawnTwoShots notifies (0.42 .. 0.83) are six HurtRadius pulses:
ScreamDamage 8 x (1 - (distance - 20) / 700), whole points, if in sight,
and momentum scale x ScreamForce -150000 along her-to-player (a pull).
Pawn.TakeDamage on the player: on the ground the upward part becomes at
least 0.4 x the size, divided by Mass 400, AddVelocity (`walk::PlayerPush`).
Headless: 50% dies at once, else within 10 x FRand() s, no bites. No flip.
The SirenScream effect (an additive mesh) is an AnimNotify_Effect; mesh
emitters now use their material's blending. Not done: DoShakeEffect (view
shake and blur), zapped (zed time), glass shattering.

**Husk (done 2026-10-04).** ZombieHusk, HuskZombieController,
HuskFireProjectile (LAWProj), KFPawn fire. The shared ranged attack with
ShootBurns (full body, standing): at any range within 65535 in sight
(FireWeaponAt needs Focus; now checked for every ranged attack), not before
NextFireProjectileTime (ProjectileFireInterval 5.5 + FRand() x 2 after each
shot). Notifies: HuskChargeUp at 0.26 (attached to Barrel), SpawnTwoShots at
0.475, HuskMuzzle at 0.49. The fireball (`src/fireball.rs`) starts at the
Barrel bone, aimed by AdjustAim: lead by the player's velocity, then with
bTrySplash half the time (Skill 2 assumed) at the floor under the player,
else the middle, else the head, whichever is in sight. Straight flight at
1800, FlameThrowerFlameB trail, explodes on the level, the player or
another zed: FlameImpact, FlameThrowerBurnMark decal, HurtRadius(25, 150,
DamTypeBurned, 125000) scaled by distance and by exposure (head and root in
sight, half each). Fire damage over 2 sets the player burning: every 1.5 s
LastBurnDamage halves and is taken, 5 times at most (25 -> 12, 6, 3, 1).
Hits do not interrupt his attacks. Not done: aim error, other zeds getting
out of the way or taking fire damage, view shake, HuskChargeUp's beam
emitter (beam emitters are not drawn), FlameThrowerFlame (an xEmitter).

## Patriarch (B1-B6 implemented 2026-10-04; UNFINISHED)

**Status: unfinished.** The six planned steps are in, but not play-tested;
he can get stuck after falling while escaping (shared movement code); and
the parts listed under "Not planned" and each step's "Not done" are
missing. The README's "What doesn't work yet" has the list.

Sources: ZombieBoss, ZombieBossBase (defaults), BossZombieController,
BossLAWProj / LAWProj (KFChar, KFMod), animation notifies of
`Patriarch_anim` (`kfpkg notifies Animations/KF_Freaks_Trip.ukx
Patriarch_anim`). Normal difficulty, one player, as for the other zeds.

**What he is.** Health 4000, GroundSpeed 120, MeleeDamage 75, damageForce
170000, Mass 1000, collision 26x44 (inherited) plus the extended head
cylinder (ColOffset Z 65, radius 27, height 25). Things he does NOT do,
unlike other zeds: lose his head (`RemoveHead` is empty), flinch
(`PlayDirectionalHit` is empty), flip over, get stunned by melee
(bMeleeStunImmune), burn-panic, step out of the way. He has no MeleeAnims;
his own RangedAttack picks every attack.

**RangedAttack, in order** (D = distance to the player; "only enemy" is
always true with one player):
1. Close enough (`IsCloseEnuf`: flat distance < both radii + 25, heights
   overlap): MeleeImpale if Health > 1500 and 50%, else MeleeClaw. Both on
   the upper body from SpineBone1 (he keeps walking). ClawDamageTarget
   notifies: MeleeClaw at 0.50 (reach ClawMeleeDamageRange 85, hits
   everyone in front of him); MeleeImpale at 0.50 and 0.609 (reach
   ImpaleMeleeDamageRange 45, two hits). Damage 75 -5%..+5% per hit.
   (MeleeClaw2 exists in the mesh but is never used by the script.)
2. 20 s since the last sneak: 30% wait another 20 s, else cloak and
   sneak (below).
3. Charging and D < 200 (or only enemy): nothing more.
4. Not charging, D < 700, 5-10 s since the last charge, and not the 15%
   "wants the chaingun" roll: charge.
5. D > 500 and the rocket timer passed: not in sight or 25%: try again in
   0-5 s; else PreFireMissile, rocket at its end, FireEndMissile; next
   rocket in 10-25 s.
6. Chaingun timer passed: not in sight or 15%: try again in 0-4 s; else
   PreFireMG, then FireMG looped, FireEndMG; next in 5-15 s.

**Charge** (state Charging): RunF at GroundSpeed x 2.5, at most 6 s and
1-2 attacks; while attacking x 1.25 and keeps moving at the player; a
landed hit ends it, push x 1.5. Ends if the player gets over 700 away.
**Charge from damage:** meant to be: damage within 10 s of the previous
adds up; over 200 from a player within 700: charge. It never happens in KF:
TakeDamage only sets LastDamageTime inside the "within 10 s" branch, and
LastDamageTime starts at 0, so ChargeDamage is reset to 0 on every hit. We
copy KF and leave it out. (The same counter is used by the chaingun's
"charge instead" rule, so only the "shot from closer than 100" half works.)

**How often he decides:** BossZombieController.TimedFireWeaponAtEnemy
re-arms its timer at 0.01 s because FireWeaponAt always returns false, so
RangedAttack runs about every frame while he sees the player, and its
FRand() rolls (the 15% chaingun wish, the 5 + 5 x FRand() s charge gap)
are re-rolled each time. In practice he charges about 5 s after the last
charge ended whenever the player is within 700 and he is not attacking.

**Chaingun** (state FireChaingun): 35-94 shots. Bursts of 0.75-1.25 s, a
shot every 0.05 s, pauses of 0.5-1.25 s. Each shot: a trace from the `tip`
bone at the player, spread VRand x 0.06, 4-6 damage (MGDamage 6 x 0.75
solo Normal, + Rand(3), whole points), push 500. Lost sight for 0.25-0.6 s
ends it. Shot from closer than 100, or 200+ damage from closer than 500:
charge instead.

**Rocket** (BossLAWProj): from `tip`, aimed with lead, speed 2600 (max
3000), explodes on touch: 200 x 0.375 = 75 in radius 500, momentum 125000,
RocketMarkDirt decal. Same pattern as the Husk fireball (`fireball.rs`).

**Sneak** (SneakAround): cloaked (patriarch_invisible shaders,
patriarch_fizzle overlay), hunts the player, at most 10 s, ends after his
first melee hit. Cloak drops when he attacks.

**Knockdown and healing.** Each time health drops under a healing level
(Health / 1.25, / 2, / 3.2 = 3200, 2000, 1250), and he has healed fewer
than 3 times: KnockDown (full body), then cloak and escape: run at x 2.5
to a hiding spot (a navigation point within 2500 that the player cannot
see, scored by distance), claw only if cornered, then Heal: NotifySyringeB
at 0.464 adds Health / 4 = 1000; a syringe bone (Syrange1..3) is hidden
per heal.

**Steps** (one at a time, each its own change and log entry):
- B1. Rules and melee: no decapitation, no flinches/stun/flip; IsCloseEnuf
  starts MeleeClaw / MeleeImpale on the upper body; damage at the
  ClawDamageTarget notifies with each attack's reach; impale hits twice.
  Check: log `zed_attack` / `player_hit` with `--god`; a headshot run
  shows no `decapitated`.
  Done: hits also push the player (damageForce 170000 / Mass 400 = 425
  units/s, upward at least 0.4 x that). Other zeds' melee does not push
  yet (KF does: their damageForce), a known gap outside this step.
- B2. Charge. Check: speed in the log (300 = 120 x 2.5), attacks per
  charge, 6 s limit. Done; the sneak branch of RangedAttack (checked
  before the charge) waits for B5.
- B3. Chaingun. Check: shot count, burst timing, damage per shot. Done
  without effects: the tracer (KFNewTracer, its velocity set per shot),
  muzzle flash (MuzzleFlash3rdMG on `tip`) and ROBulletHitEffect are a
  later step (B3b). New zed state `BossBusy` (full body, stands, turns to
  his aim); `boss::Chaingun::step` is the state's Begin loop and AnimEnd.
  Quirk copied from KF: losing sight sets a 0.25-0.6 s timeout at every
  FireMG end (every 0.37 s), and the loop only checks it when awake, so he
  often keeps firing at where the player was last seen for a while.
- B4. Rocket. Check: rocket timing, flight time, explosion damage at
  distance. Done: `fireball.rs` now has two projectile kinds (Husk
  fireball, Boss rocket) sharing LAWProj's flight and HurtRadius; the
  rocket's mesh is named only in StaticMeshRef, loaded with
  `gore::load_piece_with_mesh`. Aim: MonsterController.AdjustAim as for the
  Husk without bTrySplash (lead, middle, else head); aim error not done.
- B5. Cloak and sneak. Check: cloak/uncloak log lines; a screenshot.
  Done: he spawns in InitialSneak (MakeGrandEntry's state after the
  Entrance animation, which we skip), cloaked until he first sees the
  player. SneakAround: RangedAttack's second branch (20 s since the last
  sneak, 30% put off another 20 s), before the charge checks, so it can cut
  a charge short. Both sneak states extend Escaping: run at x 2.5 with RunF
  (normal speed while attacking), only MeleeClaw, uncloak to attack, push x
  1.5; SneakAround.MeleeDamageTarget ends the sneak at the first damage
  check, hit or miss; otherwise it ends after 10 s. The cloak is drawn like
  the Stalker's (a faint copy of each texture), not KF's refraction shader;
  the commando "spotted" glow needs perks (not done). RangedAttack rolls
  the 15% chaingun wish before its bShotAnim check, as KF does.
- B6. Knockdown, escape, heal. Check: knockdown at < 3200, hide
  spot not visible to the player, +1000 health, syringe count. Done
  (tested with the scripted `hurt_zeds` action, 100 damage each, instead
  of a new option). TakeDamage -> `check_knockdown` (levels from his
  starting health, truncated); KnockDown (full body, 2.03 s) ends charges,
  the chaingun (its EndState wait) and rockets, and also healing; then
  cloaked Escaping: SyrRetreat.FindHideSpot (points within 2500, unseen by
  the player, reachable; the score above), FindRandomDest when he cannot
  see the player; our route-finder runs to it (the spot as its goal);
  claws only if caught. On arrival BeginHealing: uncloak, Heal (5.03 s),
  NotifySyringeA (syringe count, Syrange bone hidden) and NotifySyringeB
  (+1000). KF quirks kept: no knockdown while sneaking (the sneak states
  extend Escaping); the heal is not capped at HealthMax (3100 -> 4100).
  Our addition: an escape that has not arrived after 30 s heals where he
  is. Not done: AddBossBuddySquad (game mode: zeds summoned on
  knockdown), the BossHPNeedle prop in his hand, voice lines.

**Not planned:** the entrance animation and the boss-wave intro, the radial
attack (needs 3 players around him), the victory laugh and death camera,
zed time, pipe-bomb damage scaling, voice lines (no sound yet).

**Code placement.** `zed.rs` is already over 3000 lines with per-zed
special cases in one think function. Boss-only logic (his attack choice,
chaingun, escape) goes in a new `src/boss.rs`, called from `zed.rs`, like
`fireball.rs` and `vomit.rs`. A bigger reorganisation of `zed.rs` would be
a separate decision.

## Firing effects: muzzle flashes, shells, tracers, impacts (F1-F3, F5 implemented 2026-10-04)

Today a shot draws nothing. What KF does (KFFire, WeaponFire,
KFWeaponAttachment, ROHitEffect; 9mm = SingleFire / SingleAttachment):

- **First-person flash:** FlashEmitterClass (9mm: ROEffects.MuzzleFlash1stMP)
  is spawned once, attached to the weapon's FlashBoneName (`tip`), and
  `Trigger`ed on every shot (WeaponFire.FlashMuzzleFlash). It is drawn with
  the weapon (Canvas.DrawActor at DisplayFOV), i.e. on our weapon layer.
- **Shells:** ShellEjectClass (ROEffects.KFShellEject9mm) attached to
  ShellEjectBoneName (`Shell_eject`), also `Trigger`ed per shot, also drawn
  with the weapon. Its particles are in world space (they stay behind as
  the player moves).
- **Weapon light:** WeaponLight turns the first-person weapon's dynamic
  light on for 0.15 s per shot.
- **Tracer and impact:** KFFire.DoTrace calls the attachment's UpdateHit
  only when the shot hits the level or a non-pawn actor; ThirdPersonEffects
  then spawns ROBulletHitEffect at the hit and one KFNewTracer particle
  from the weapon's `tip` (GetEffectStart in first person) toward the hit:
  velocity 7500 (mTracerSpeed) along the shot, lifetime (distance - 50) /
  7500 (mTracerPullback 50), so it ends at the hit. **KF quirk, kept:** a
  shot that hits a zed, or hits nothing, draws no tracer and no impact
  (only the zed's own blood).
- **ROBulletHitEffect** (ROHitEffect): traces 16 units ahead for the hit
  material's SurfaceType and picks one of 20 entries: a bullet-hole decal,
  an impact emitter and a sound. Default (no material): BulletHoleDirt and
  ROBulletHitRockEffect.
- **Patriarch chaingun** (ZombieBoss.AddTraceHitFX, every shot): a
  MuzzleFlash3rdMG on `tip` (spawned once; SpawnParticle(1) on later
  shots), a KFNewTracer particle at 10000 with lifetime (distance - 50) /
  10000, and ROBulletHitEffect at the hit point whatever was hit.

What our particle system lacks: emitters that only spawn on Trigger
(TriggerDisabled false, SpawnOnTriggerRange, SpawnOnTriggerPPS), the
native SpawnParticle(n) with the script's per-shot StartVelocityRange and
LifetimeRange, effects that live on with no particles (auto_destroy false,
LifeSpan 0), following a bone every frame, and drawing on the weapon layer.

**Steps** (one change each):
- F1. Particles: read the trigger settings; `ParticleEffect::trigger()`,
  `spawn_particles(n)` with optional velocity and lifetime for emitter 0;
  persistent effects; a render-layer choice. Check: unit tests on spawn
  counts; the load log for the five classes.
- F2. 9mm first person: flash on `tip` and shells on `Shell_eject`, on the
  weapon layer, following the bones; the 0.15 s weapon light. Check: log
  per shot (particles spawned, bone positions); a screenshot mid-shot.
- F3. Tracers and impacts for player shots that hit the level (default
  surface). Check: tracer lifetime vs. distance in the log; decal count.
- F4. Surface types: keep each collision triangle's material, read its
  SurfaceType, pick the matching impact, decal (and later sound).
- F5. Patriarch chaingun: flash, tracer and impact per shot.

**As built.** `particles.rs`: `trigger()` uses a table copied from each
class's Trigger script (MuzzleFlash1stMP 2+1, KFShellEject9mm casing + 3
smoke, MuzzleFlash3rdMG ...); `spawn_particles` / `spawn_all` (requests
spawned on the next update, at most MaxParticles alive, oldest dropped);
`set_start` (tracer velocity and lifetime); `SpawnOptions` (persistent,
render layer); class lookup ignores case. `weapon.rs`: the weapon's
FireFx (classes and bones from the fire mode and weapon), bone frames in
world space each frame (the 9mm mesh has MeshScale 5, so its `tip` is ~123
units in front of the eye, as KF places it), `weapon_fire_fx` triggers per
shot. `bullet_fx.rs`: tracers (one reused emitter per shooter) and
ROBulletHitEffect (default surface only). The Patriarch's mMuzzleFlash
follows `tip` through the zed effect anchors. **KF quirk, kept:** his first
AddTraceHitFX only spawns the flash, so the first chaingun shot of his life
has no flash.

**Not done:** the weapon light (0.15 s dynamic light; no dynamic lights
yet), smoke (SmokeEmitterClass is unset for the 9mm), the third-person
pistol flash and shells (no player body), impact sounds and the tracer's
fly-by sound (no sound), surface types (F4).

**Open question:** bullet holes vanish after 2 s here. Our decal loader
uses 3 s when a class sets no LifeSpan, then ProjectedDecal's
FMax(0.5, LifeSpan - 1). The bullet-hole classes set none, so KF's would be
Actor's 0 -> 0.5 s x DecalStayScale (1.0 in Default.ini), yet holes seem
to last longer in KF; how the engine keeps an abandoned projector is native
code I cannot read. Left as it is until checked against the game.

## Video recording (implemented 2026-10-04)

A debugging aid, not part of KF. F9 starts and stops recording; the
scripted input action `record` does the same (for tests). Each recording
is `work/videos/<map>-<unix time>.mp4` (gitignored).

- Frames: Bevy's screenshot capture of the window (the same as F12), asked
  for at a steady 30 frames per second of game time. If the game runs
  slower than 30 fps, the last frame is repeated so the video plays back at
  real speed.
- Encoding: the raw pixels go through a pipe to an `ffmpeg` process
  (H.264, scaled down to at most 1280 wide), run on its own thread so the
  game does not wait for disk writes. `ffmpeg` comes from the flake's dev
  shell (`pkgs.ffmpeg`); the copy in the user profile cannot start inside
  the shell (its libraries clash with the shell's LD_LIBRARY_PATH).
- The window title says "REC" while recording; nothing is drawn on screen,
  so the video shows only the game.
- Stopping, or quitting while recording, closes the pipe and waits for
  ffmpeg to finish the file. The log has `record_start`, `record_stop`
  (frames asked, frames written, repeats) and `record_saved` (file,
  bytes, ffmpeg exit status).
- Cost: one window-sized copy per recorded frame; expect a lower frame rate
  while recording at high resolutions.
- Bevy captures a window at most once per frame and silently drops a
  second request (extract_screenshots, "Duplicate render target"). So a
  frame with an F12 / `--screenshot` capture is skipped by the recorder;
  the next capture covers its slot. (Found when a `--screenshot` run never
  quit: its capture had been dropped.)
- The source is `src/record.rs`.

## Weapons (milestone 7, W1-W9 implemented 2026-10-05)

Goal: every base-game weapon, with KF's own numbers and firing rules. You
asked to skip DLC weapons.

**Which weapons.** KF marks paid weapons with a Steam `AppID` on the weapon
class (checked by `KFSteamStatsAndAchievements.PlayerOwnsWeaponDLC`). 30
classes have one (Golden, Camo, Neon, the Halloween and summer packs,
Thompsons, Dwarf axe, Scythe, flare revolvers...): skipped. Of the rest, these
are not trader weapons and are skipped too: the `*DM` / `*SP` variants
(deathmatch and story-mode copies), MachinePistol (no mesh), StunNade and Bat
(leftovers), Claws, the story-mode dummies. Left, 48 weapons (`BASE_WEAPONS` in `weapon.rs`):

| Kind (KF fire class) | Weapons |
| --- | --- |
| Melee (KFMeleeFire) | Knife, Machete, Axe, Katana, Claymore, Chainsaw |
| Bullets (KFFire, KFHighROFFire) | 9mm, Dual 9mm, Handcannon (Deagle), Dual Handcannons, 44 Magnum, Dual 44, MK23, Dual MK23, Lever Action (Winchester), Bullpup, AK47, SCAR, M4, MKb42, FN FAL, M14 EBR, MAC10, MP7M, MP5M, M7A3M, Kriss (primary fire) |
| Pellets (KFShotgunFire, projectiles) | Shotgun, Hunting Shotgun (Boomstick), AA12, Benelli, KSG, Trenchgun, Nailgun |
| Projectiles and explosives | Crossbow, M99, LAW, M79, M32, M4 203 (grenade alt fire), Frag, Pipe bomb, Husk gun |
| Fire | Flamethrower (also Trenchgun, MAC10, Husk gun, which set zeds alight) |
| Equipment | Syringe, Welder, medic dart alt fires |
| Special | ZED Gun (achievement unlock, not DLC), ZED Gun MKII |

**Shared rules found in the scripts (checked 2026-10-04).**
- `KFFire.DoTrace` always deals **DamageMax**, not a roll between DamageMin
  and DamageMax. Our 9mm rolls 25-35; KF deals 35. Fixed in W1.
- Penetrating bullets: Deagle, 44, MK23 (and their duals), MAC10 override
  `DoTrace`; details read in W3.
- Shotguns fire `ProjPerFire` pellet projectiles (`KFShotgunFire.DoFireEffect`,
  random spread per pellet). Each pellet goes through zeds, keeping
  `PenDamageReduction` of its damage per zed, until damage falls to
  `PenDamageReduction / MaxPenetrations` of the start (`ShotgunBullet.ProcessTouch`).
  Firing pushes you back by `KickMomentum`.
- Recoil (`KFFire.HandleRecoil`): pitch and yaw kicks between half and full
  `maxVerticalRecoilAngle` / `maxHorizontalRecoilAngle`, plus speed x
  `RecoilVelocityScale`, plus HealthMax / Health x 5, spread over `RecoilRate`.
- Semi / full auto: `FireMode[0].bWaitForRelease`; `KFWeapon.DoToggle`
  flips it (the alt-fire key on rifles with no other alt fire).
- Inventory: KF starts you with Knife, 9mm, Frag, Syringe, Welder
  (KFHumanPawn RequiredEquipment). Carry limit `MaxCarryWeight` 15.
  Slots by `InventoryGroup` (1 melee, 2 pistols, 3 primary, 4 specials,
  5 equipment, 0 grenade), ordered by `GroupOffset`.
- Many weapons added after release name their mesh by string (`MeshRef`,
  `SkinRefs`, `HandSkinRef`) instead of `Mesh`; the loader must read both.
- KF's default keys (System/defuser.ini): G ThrowNade, Q QuickHeal, F
  ToggleFlashlight, middle mouse AltFire, right mouse ToggleAiming, mouse
  wheel Next/PrevWeapon, R reload.

**Steps.** One weapon family per step; each step logs every shot (weapon,
damage, hits, penetrations) and adds unit tests for the rules.
- **W1, any weapon from data + inventory.** Load any weapon class by name
  (Mesh or MeshRef). Startup check: load all 48 and log which fail. KF's
  starting inventory; number keys pick a slot and pressing again cycles
  inside it; mouse wheel cycles all. Test flag `--give all` or
  `--give AK47AssaultRifle,Shotgun` (ignores the weight limit, logged).
  Fix DamageMax. Move "spawn Gorefast" off G (to H) for KF's grenade key.
- **W2, bullet guns.** All KFFire / KFHighROFFire weapons above: auto and
  semi fire, the toggle on middle mouse, recoil, per-weapon spread, aimed
  spread, iron sights, ammo and reloads, muzzle effects (reusing F1-F5).
- **W3, pistols.** Penetration (Deagle family), dual pistols (alternating
  hands, both muzzles), and KF's rule that picking up a second pistol makes
  a dual (read when reached).
- **W4, shotguns.** Pellet projectiles with penetration, kick momentum,
  shell-by-shell reloads (`KFWeaponShotgun`), the Boomstick's two-barrel
  fire, the Nailgun's nails.
- **W5, melee.** Primary and heavy secondary attacks for every melee
  weapon (damage, range, delay, hit cone, from each fire class), the
  Chainsaw's continuous fire.
- **W6, projectiles and explosions.** Grenades and rockets (M79, M32,
  LAW, M4 203), Frag on G, pipe bombs (proximity), Crossbow bolts and M99
  bullets (penetration, headshots). The explosion code from the Husk and
  Patriarch rockets is shared.
- **W7, fire.** Zeds catching fire (KFMonster's burning: damage over time,
  `SetBurningBehavior`), then the Flamethrower, Trenchgun, MAC10 and
  Husk gun (charged shots).
- **W8, medic and equipment.** Syringe (heal yourself; Q quick heal),
  medic dart alt fires. The Welder: doors are not simulated, so it has
  nothing to weld yet (see W8c).
- **W9, ZED guns.**

**W1 findings (done).**
- 47 of 48 loaded at first (`--give all`, 6.3 s); the ZED Gun MKII's
  defaults only partly parsed. Fixed in W2 (defaults reader): 48 of 48.
- Inventory order is Pawn.AddInventory's: inside a group, falling
  `Priority` (melee: Axe, Machete, Knife). Number keys follow
  Pawn.SwitchWeapon: the first weapon of that group after the one in hand,
  so pressing again cycles. The wheel steps by (group, GroupOffset). The
  Frag is never picked (Frag.WeaponChange / NextWeapon).
- The AK47's `SkinRefs` path ("KF_Weapons2_Trip_T.Rifles.AK47_cmb") does
  not exist (the package has "Rifle.AK47_cmb"); the mesh's own material
  is used. What KF does then is not checked.
- Fire kind comes from the fire class defaults: MeleeDamage = melee,
  ProjectileClass = projectile (not done: animation and ammo only, logged
  `fire_not_implemented`), DamageMax = bullet. WeldFire counts as no
  damage (it only welds doors).
- Weight: starting kit weighs 1 (the Frag), so walking speed is 198.3
  (measured 198); the health factor of ModifyVelocity is not done.

**Melee alt attacks (part of W5, done early on request).**
- Both fire modes load (`FireModeClass[1]` through `ClassDefaults::get_at`).
  Alt fire is the middle mouse button: KF's `defuser.ini` and your
  `User.ini` bind MiddleMouse=AltFire; right mouse is ToggleIronSights,
  which does nothing on weapons without bHasAimingMode.
- Weapon.ReadyToFire / StartFire: a mode cannot fire while the other's
  button is held, and waits for the other's cooldown as well.
- Each swing has its own damage timer (KFMeleeFire.ModeDoFire SetTimer),
  so a swing still lands if another starts before it.
- `bWaitForRelease` per mode: one shot per click. A click during the
  cooldown is kept until it can fire, if the button is still held (my
  choice; KF's exact handling of such a click is not checked).
- Still to do in W5: KFMeleeFire.Timer hits the traced zed plus every
  other zed within 1.1 x range in the cone (damage x cosine), doubles
  damage on backstabs, and the Chainsaw's held fire.

**W2 bullet guns (done).** Rules copied, with where they come from:
- Spread (`firing.rs`): KFFire.GetSpread from Default.Spread, + MaxSpread / 6
  per shot within 0.5 s of the last, up to MaxSpread; aiming x 0.5;
  semi auto x 0.85 if bAccuracyBonusForSemiAuto. Direction:
  InstantFire.DoFireEffect, aim + VRand() x FRand() x Spread.
- Recoil (`firing.rs`): KFFire.HandleRecoil (RandRange(max / 2, max) up
  and sideways, sideways sign random unless bRecoilRightOnly; + speed x
  RecoilVelocityScale; + HealthMax / Health x 5), spread over RecoilRate
  by KFPlayerController.RecoilHandler. Kicks within the window add up;
  the view does not return. Frame-rate dependent, as in KF.
- Fire rate: NextFireTime += FireRate (the remainder carries over).
- Full / semi auto: bWaitForRelease; middle mouse toggles it on the
  weapons whose AltFire calls DoToggle (AK47, Bullpup, FN FAL, M4, MAC10,
  MKb42, SCAR; AA12 and KSG in W4). Not on the M4 203 (empty AltFire).
- Animations: KFFire.PlayFiring (FireAnim on the first shot of a press,
  then FireLoopAnim; aimed variants), KFHighROFFire's FireLoop state
  (loop animation while held in full auto), FireEndAnim on release
  (Weapon.StopFire) and after FireAnim (Weapon.AnimEnd).
- Reloads (KFWeapon.Tick): done after ReloadRate (the animation is cut to
  idle then); bHoldToReload adds one round per ReloadRate; firing with
  2+ rounds in (WinchesterFire / KFShotgunFire.AllowFire), aiming or
  switching interrupt it; other reloads cannot be interrupted and block
  switching and aiming. A click on an empty magazine reloads if
  bModeZeroCanDryFire.
- Shots slow you: velocity x 0.1 (FireRate > 0.25) or x 0.5, on the
  ground (KFFire.ModeDoFire).
- Switching (Weapon.BringUp / PutDown / KFWeapon.Timer): SelectAnim and
  PutDownAnim at 1.36x, ready after BringUpTime 0.33 s, gone after
  PutDownTime 0.33 s, plus DownDelay right after a shot. Before this,
  switches waited for the whole animation at 1x (about 3x too slow).
- Class defaults reader fix: it took the first byte offset that parsed;
  for 37 of 1798 classes that was a false start that swallowed real
  values (BullpupAmmo, M14EBRAmmo, MKb42Ammo, AA12Ammo, M7A3MFire's
  recoil, ZEDMKIIWeapon's MeshRef...). Now the offset with the most
  properties wins; all 37 gained properties, none lost. The ZED Gun MKII
  now loads (48 of 48).
- Data quirks seen: the Lever Action's select animation is named
  "Select " (trailing space), so like KF (exact name match, I believe)
  it draws with no select animation; the Kriss has no Fire or Fire_Iron,
  so semi-auto shots show no animation; many weapons lack FireEnd.

**3D scopes (done 2026-10-04, after W2, on request).** `src/scope.rs`.
- Only the Crossbow and M99 have bHasScope. No ini sets KFScopeDetail,
  so KF uses KF_ModelScope: Crossbow.RenderTexture draws the world from
  the eye along the view (DrawPortal) into a 512 x 512 ScriptedTexture at
  scopePortalFOV (Crossbow 12, M99 13.33), Combiner (reticle x view,
  CO_Multiply, AO_Use_Mask), on Skins[lenseMaterialID] (2) while aiming;
  the player view zooms to PlayerIronSightFOV (32 / 30) as for iron
  sights.
- Here: a sky-zone camera and a world camera render into an image; a
  reticle quad (CommandoCross / Scope.MilDot) in front of the world
  camera is drawn over it by its alpha; the lens part's material is
  swapped for an unlit one showing the image. The reticle textures are
  transparent inside and black at the lines and edge, so I draw them
  over the view; a literal multiply would make the lens black. My
  reading of the Combiner, not checked against the original.
- Lens UVs cover 0-1, so the whole 12 degree image fills the lens, which
  takes about 11 degrees of the 32 degree view: about 1:1 with the
  surroundings, the zoom coming from the view itself (the texture scope
  mode prints "Zoom: 2.50", roughly 90 / 32). Checked by screenshots:
  not mirrored, right way up.
- Not done: KF_TextureScope and KF_ModelScopeHigh (other settings);
  the FN FAL ACOG needs nothing (its lens is a masked material).

**W3 pistols (done).**
- Penetration (DeagleFire.DoTrace, identical in DeagleFire, Magnum44Fire,
  MK23Fire and the three dual versions): up to 5 zeds, the damage
  halved after each and cut to a whole number (int(HitDamage)), stopped
  by the level. Checked: three Clots in a line took 115, 57, 28; the
  9mm stops at the first.
- Dual pistols (DualiesFire / Dualies): shots alternate FireAnim and
  FireAnim2 (right, left; aimed and hip swap separately); the right shot
  flashes at FlashBoneName and ejects at ShellEject2BoneName, the left at
  altFlashBoneName and ShellEjectBoneName; the tracer starts at the
  firing gun. Aiming plays GOTO_Iron, leaving GOTO_Hip (stretched to
  ZoomTime).
- Getting duals replaces the single (Dualies.GiveTo and the three
  copies): magazine = single's + single MagCapacity (capped), ammo =
  dual InitialAmount + all the single's (capped at MaxAmmo). Ammo
  instances' fate not checked. Picking up a second single (pickups) is
  not done: there are no pickups yet.
- Found on the way: most guns had no muzzle flash or shell ejection; the
  effect library listed only the 9mm's classes. Now every base weapon's
  FlashEmitterClass / ShellEjectClass loads, with each class's Trigger
  counts. The class defaults reader was changed again: the fullest-list
  rule misread KFShellEjectEBR; now the candidate with the fewest bogus
  (non-delegate Raw) values wins, then the most values. Compared with
  the original reader over all 3757 classes: 449 change, every change
  drops bogus values (often a leading "User" of type 0) or adds real
  ones.

**Reflex sights (after W3, on request).** The SCAR, M4 and Bullpup lens
is `Shader Rifles.reflex_sight_A_unlit`: Diffuse and SelfIllumination a
Combiner (CombineOperation 6) of a panning glass-speckle texture and the
reticle `Reflexsight_A`; Opacity the reticle. The resolver took the
speckle and drew it opaque. Rule (`material::resolve_skinned`, skinned
meshes only): when the Opacity texture is an input of the Diffuse
Combiner, use it for colour and alpha, alpha-blended; the speckle layer
is dropped. `kfpkg materials` lists where the rule applies: 39 of 1023
Shaders in all packages, many of them level materials whose Opacity is a
plain mask (puddles, oil, water), hence skinned meshes only. Skinned
meshes now draw Translucent as alpha blend (was opaque); for them a
Combiner no longer passes an input texture's bAlphaTexture / bMasked up
as transparency (its alpha feeds the combine: the Shotgun's diffuse +
reflection, alpha = reflection mask, came out see-through). Levels keep
the old reading (`kfpkg materials`: 95 Combiners would differ).

**W4 shotguns (done).** `src/projectile.rs`.
- Pellets are projectiles (ShotgunBullet): Speed 3500, LifeSpan 3,
  straight; each zed on the path takes the damage (x HeadShotDamageMult
  1.5 on a headshot, and KFMonster.TakeDamage multiplies again by the
  damage type's 1.1: a KF quirk, kept), then damage x PenDamageReduction;
  gone once damage / start <= PenDamageReduction / MaxPenetrations
  (Shotgun 2 zeds, Hunting Shotgun 3, AA12 / KSG / nails 4). Walls:
  ROBulletHitEffect and gone.
- KFShotgunFire.DoFireEffect: StartProj = eye + X x ProjSpawnOffset.X (+ Y,
  Z offsets from the hip), pulled back to a wall in between;
  ProjPerFire x AmmoPerFire pellets, each X >> (Spread x (FRand() - 0.5)
  on yaw, pitch, roll) (SS_Random); AddVelocity(KickMomentum >> view) on
  the ground; ModeDoFire's x 0.1 / 0.5 slow-down. Spread is always the
  default (no aiming bonus). Recoil: KFShotgunFire.HandleRecoil (either
  side; speed x 3).
- Hunting Shotgun (BoomStick): left = one barrel (BoomStickAltFire,
  0.25 s, Fire_Last and 2.75 s when it empties the gun), middle = both
  (BoomStickFire, 20 pellets, AmmoPerFire 2, 2.75 s); middle with one
  barrel fires the single barrel (ClientStartFire); both barrels reload
  by themselves ReloadCountDown 2.5 s after the last (WeaponTick); no
  manual reload with one barrel loaded.
- HSG-1 (KSG): middle click toggles wide spread (x 2.05, KSGFire), HUD
  [WIDE] / [NARROW]. AA12: middle click full / semi auto.
- Nails (NailGunProjectile): bounce twice (velocity reflected x 0.65,
  falling with gravity after the first), then stop. A head pinned to the
  wall and nails stuck for 5 s are not drawn.
- Tracers: each pellet gets a tracer (a pool of 32 emitters; KF spawns a
  KFTracer per pellet) drawn at fire time along its first straight path
  to the wall or to the zed it will stop in (zeds as they are then).
- Not done: the pellet meshes, Trenchgun fire damage (W7), the Shotgun /
  Combat Shotgun / Nailgun flashlight alt fire.

**W5 melee (done).** `combat.rs` resolve_swings.
- KFMeleeFire.Timer: a trace from the eye along the view, weaponRange
  long, stopped by the level, hits the first zed; from behind (direction
  player -> zed along the zed's facing) the damage is doubled. If
  WideDamageMinHitAngle > 0, every other living zed in sight within
  weaponRange x 1.1 (+ its radius) of the player, with view . direction >
  WideDamageMinHitAngle, takes damage x that cosine at 0.7 of its height;
  after a backstab the wide hits are doubled too (KF quirk, kept).
  KFMonster.TakeDamage checks melee headshots with HeadShotCheckScale
  1.25 (was 1.0 here). Before W5 the swing hit only the nearest zed.
- KFMeleeFire.ModeDoFire: velocity x ChopSlowRate (KFMeleeGun 0.5,
  Machete 0.35, Axe 0.2) on the ground.
- Chainsaw (ChainsawFire): held fire is a FireLoop (looping Fire
  animation, FireEndAnim Idle on release); every FireRate 0.1 s it hits
  at once for MeleeDamage + Rand(maxAdditionalDamage) (14-18), traced
  zed only. Its alt fire is a normal swing (270).
- Not done: FlipOver (a zed knocked down when the player moves over 300
  units/s: the player cannot), HitEffectClass sparks on walls, the
  bloody weapon skin, melee hit sounds.

**W6 projectiles and explosions**, in three parts: W6a grenades and
rockets (done), W6b frag and pipe bomb, W6c Crossbow and M99.

W6a (`projectile.rs` PlayerExplosive):
- M79GrenadeProjectile family (M79, M32, M4 203's M203): Speed 8000,
  straight for StraightFlightTime 0.25 s, then falling (Tick); LAWProj:
  Speed 2600, straight. A PanzerfaustTrail behind each.
- ProcessTouch / HitWall: closer than sqrt(ArmDistSquared) (300; LAW 500)
  to where the player is now, it is a dud: a touched zed takes
  ImpactDamage (200) with the impact type's HeadShotDamageMult (2.0), and
  the grenade drops and vanishes after 1 s. Otherwise Explode:
  KFNadeLExplosion / LawExplosion 20 units out, KFScorchMark /
  RocketMarkDirt, HurtRadius.
- HurtRadius: each zed whose cylinder reaches DamageRadius takes Damage x
  (1 - max(0, (distance - its radius) / DamageRadius)) x
  KFMonster.GetExposureTo (head 0.4, root 0.3, feet 0.15 each, by line of
  sight; feet approximated at the cylinder's bottom); no headshots
  (bCheckForHeadShots false). The player: x KFPawn exposure (head, root),
  x 0.25 at Normal (KFGameType.ReduceDamage halves own damage, then
  halves it again at difficulty <= 3 in single player; fixed after W7a), no push
  (KFHumanPawn.TakeDamage zeroes a player's momentum).
- ZombieFleshPound.TakeDamage now as in KF: listed explosive types x 1,
  frag and pipe bomb x 2, others x 0.5 or x 0.75 for a headshot by a type
  with HeadShotDamageMult >= 1.5 (was a flat x 0.5).
- Ammo: M79Fire / M203Fire / LAWFire need only the ammo total
  (AmmoAmount); the M4 203's grenades have their own count (M203Ammo, 6;
  HUD [GRENADES n]); LAWFire fires only when aimed and zoomed in.
- Not done: the grenade / rocket meshes, view shake, momentum on zeds
  (MomentumTransfer is logged only), explosions setting off other
  explosives, a dud's rest (it just stops).

W6b (`projectile.rs` PlayerThrown, `weapon.rs` Action::Grenade):
- G (KFPawn.ThrowGrenade): with a frag left, the weapon's next shot due
  within 0.1 s and no reload (a one-round reload is interrupted): the
  weapon goes down (PutDownAnim, QuickPutDownTime 0.15), the frag's Toss
  animation plays (Frag.StartThrow), the Nade spawns at TossSpawnTime 0.2
  (FragFire.DoFireEffect: eye + X x 25 - Y x 10; speed mHoldSpeedMin 850
  + the player's speed along the view), and at TossTime 0.366 the weapon
  comes back up (QuickBringUpTime 0.15). HUD: FRAGS n.
- Nade: falls, bounces (Velocity = -VNorm x 0.25 + parallel x 0.4), at
  rest under 20; a zed it touches stops it; explodes ExplodeTimer 2 s after
  the throw, restarted by the first bounce (KF quirk). KFNadeExplosion,
  KFScorchMark, HurtRadius 300 in 420.
- Pipe bomb (PipeBombExplosive, fired like a gun): tossed at Speed 50, at
  rest it arms after 1 s, then every 1 s (0.5 s while some threat) adds
  up the MotionDetectorThreat (from each zed class: Clot / Crawler 0.34,
  Gorefast 0.5, Stalker 0.25, Bloat / Husk 1, Siren 2, Scrake 3,
  Fleshpound 5, Patriarch 10) of zeds in sight within 150; at 1 or more,
  5 beeps 0.15 s apart, then KFNadeLExplosion and HurtRadius 1500 in 350.
- PipeBombFire.ModeDoFire: the click plays Toss; the bomb (and the ammo
  use) comes ProjectileSpawnDelay 1.1 s later (Timer). After Toss the
  next bomb comes out (PipeBombExplosive.AnimEnd: SelectAnim); the
  magazine holds 1, so the next click reloads (bModeZeroCanDryFire). The
  last one removes the weapon and switches away (KF picks the best rated
  weapon; here the previous one in slot order).
- Frag and pipe bomb damage counts double on the Fleshpound.
- One `blast` function serves all explosives.
- Not done: shooting a pipe bomb to set it off (PipeBombProjectile.
  TakeDamage), the Siren disintegrating explosives, shrapnel pieces, the
  pipe bomb's light and beeps (no sound), the frag mesh in flight.

W6c (`projectile.rs` PenRule::Bolt):
- CrossbowArrow (Speed 15000) and M99Bullet (Speed 28000), straight:
  every zed on the path takes Damage (x HeadShotDamageMult 4 / 2.25 on a
  headshot, with DamageTypeHeadShot, whose own multiplier is 1), then
  Damage / 1.25 and Velocity x 0.85; walls: ROBulletHitEffect and the
  projectile sticks. A stuck Crossbow bolt is picked up (+1 bolt, if the
  Crossbow has room) by touching it (CrossbowArrow state OnWall, 25 x 25)
  until its LifeSpan (10 s) ends.
- CrossbowFire / M99Fire need only the ammo total (no reload step).
- SpreadStyle SS_None (the Crossbow) now fires straight.
- Projectile models (added after W6 on request): the classes' StaticMesh
  / StaticMeshRef for the M79 family, LAW, frag, pipe bomb and nails, at
  DrawScale, pointing along the flight; a pipe bomb at rest lies flat
  (HitWall: pitch and roll 0). `PackageSet::find_object` now also finds
  "Package.Name" when the export is inside a group (the pipe bomb's and
  40 mm grenade's StaticMeshRef omit the group, and KF loads them).
- Not done: the arrow / bullet meshes, bodies pinned to walls
  (BodyAttacher), the M99's 3D scope is from the scope step.

**W7 fire**, in three parts: W7a zeds burning and the Trenchgun (done),
W7b the Flamethrower, W7c the Husk Gun.

W7a (`combat.rs` FireType / damage_zed, `zed.rs` burn_zeds):
- A damage type burns if its class has `bDealBurningDamage`
  (DamTypeFlamethrower, DamTypeBurned and its subclass DamTypeHuskGun,
  DamTypeTrenchgun, DamTypeMAC10MPInc). `FireType` keeps the classes the
  rules tell apart.
- Order in damage_zed, as the zed subclasses call KFMonster.TakeDamage
  last: the zed's own scale (Fleshpound; Bloat x 1.5 for exactly
  DamTypeBurned; Husk x BurnDamageScale 0.25 at Normal for DamTypeBurned
  and DamTypeFlamethrower), then KFMonster's fire rule: if not burning or
  the hit is bigger than LastBurnDamage, remember it, and FireDamageClass
  becomes the Trenchgun / MAC10 type or else DamTypeFlamethrower; damage x
  1.5 (not the MAC10); a zed not burning catches fire at 15 or more, or
  after more than 4 lighter hits (HeatAmount): BurnDown 10, GroundSpeed x
  0.8, flames (KFMonsterFlame). DamTypeBurned / DamTypeFlamethrower (and
  the Husk Gun's) never get the headshot or headless multiplier.
- Every second (KFMonster.Timer): TakeFireDamage(LastBurnDamage + 3 or 4)
  with FireDamageClass, back through TakeDamage, so each tick raises
  LastBurnDamage and the burn grows (Scrake run: 30, 34, 37 ... 57 before
  x 1.5). BurnDown 0: speed back, flames killed (also on death).
- Below CrispUpThreshhold (5 ticks left), ZombieCrispUp: the forward walk
  becomes BurningWalkFAnims (WalkF_Fire). The Husk's mesh has no
  WalkF_Fire; it keeps its walk.
- The MAC10 does **not** burn: MAC10Fire takes its damage type from the
  perk (GetMAC10DamageType), and with no perk that is DamTypeMAC10MP. Only
  the Firebug perk makes it DamTypeMAC10MPInc.
- Approximations: flames come from the zed's centre (KF spawns them on
  the skeleton, UseSkeletalLocationAs); the crisped skin
  (BurnSkinEmbers_cmb), the burning zeds' worse aim, "no pain animation
  while crisped and burning", the burn sound (AmbientSound) are not done.
  Restoring speed after a burn uses the zed's normal speed; KF sets
  default.GroundSpeed, which also undoes the headless slow-down.

W7b (`projectile.rs` PlayerFlame / flame_burst, the Flamethrower):
- FlameBurstFire (a CrossbowFire / KFShotgunFire): holding fire enters
  state FireLoop, which loops FireLoopAnim 'Fire' (no per-shot animation,
  like KFHighROFFire) and fires a FlameTendril every FireRate 0.07 s; it
  ends on release or an empty magazine (FireEndAnim 'Idle'). Its own
  AllowFire needs a round in the 100-round magazine and never fires while
  reloading (so it is not "total ammo only" like the Crossbow). Recoil is
  KFShotgunFire's: 150-300 units up per flame (about 18 degrees a second
  while held; you pull down against it).
- FlameTendril: Speed 2300 along the aim plus TossZ 200 upward, falling.
  Its Timer every 0.2 s sets the speed back to 2300 (keeping the
  direction); on the second (0.4 s, no perk) it explodes in the air.
  Touching a zed explodes it there; hitting the level explodes it at the
  wall.
- Explode: Projectile.HurtRadius(12, 150, DamTypeBurned, no push), the
  engine's plain version: every zed whose cylinder reaches 150 units and
  whose centre is in sight takes 12 x (1 - (distance - its radius) / 150)
  (no exposure check), with the W7a burn rules (DamTypeBurned: 18 after x
  1.5 lights a zed at once). The player is hit too when close (own
  damage, x 0.25 at Normal; over 2 before the reduction sets the player on
  fire). A FlameThrowerBurnMark decal and a FuelFlame (Killed after 1 s:
  it has no Parent) at the spot.
- Trail: FlameThrowerFlameB follows the flame and is Killed where it
  bursts. Not done: the HitFlame xEmitter trail (FlameThrowerFlame; no
  xEmitter support), the trails growing x 1.5 / x 1.8 per Timer, sounds.

W7c (`weapon.rs` ChargeFire, `projectile.rs` Husk fields; the Husk Gun):
- HuskGunFire is bFireOnRelease: a press (when ready, with at least 1
  fuel) starts the charge: PlayPreFire plays Charge (Charge_Iron when
  aiming), then HuskGun.AnimEnd loops ChargeLoop(_Iron); the
  ChargeUp1stHusk effect sits on the 'tip' bone. Letting go fires:
  GetDesiredProjectileClass by HoldTime (under 0.99 s weak, under 1.98 s
  medium, else strong), PostSpawnProjectile scales it (ImpactDamage x
  HoldTime x 2.5, Damage x (1 + HoldTime / 3), DamageRadius x (1 +
  HoldTime / 1.5); from 3 s on x 7.5, x 2, x 3), and ModeDoFire uses
  int(1 + 3 x HoldTime) fuel, 10 at full charge, at most what is left.
  Then Fire / Fire_Iron, the muzzle flash, recoil (up to 1500 up),
  NextFireTime = now + 0.75. The charge is dropped if the weapon leaves
  its ready state (switching).
- Fuel counts as one total (AllowFire: AmmoAmount >= 1; MagCapacity 1,
  ReloadRate 0.01), like the M79.
- HuskGunProjectile (a LAWProj, Speed 1800, straight, ArmDistSquared 0 so
  never a dud): ProcessTouch gives the zed ImpactDamage (x 1.5 on a
  headshot, then DamTypeHuskGunProjectileImpact's own x 1.5), then
  explodes. Its HurtRadius is the LAW's (exposure-scaled) but skips the
  Instigator: the Husk Gun never hurts you. DamTypeHuskGun burns (a
  DamTypeBurned subclass: no headshot bonus; the Husk and Bloat
  exact-class fire scales do not apply; the Fleshpound takes the
  small-arms x 0.5, it is not in his explosives list).
- Effects by class: trails FlameThrowerHusk_Weak / _Medium / _Strong
  (pointing along the flight, unlike the LAW's backward rocket trail),
  explosions FlameImpact_*, burn marks FlameThrowerBurnMark_Small /
  _Medium / _Large; the EffectsSM.Ger_Tracer model at DrawScale 2.
- Not done: the charge glow growing with the charge, the third-person
  charge light, the camera shake, sounds.

**W8 medic and equipment**, in three parts: W8a Syringe, healing and
quick heal, W8b medic gun darts, W8c the Welder (all done).

W8a (`weapon.rs` HealCharge / syringe_fire / QuickHeal, `combat.rs`
GiveHealth / add_health):
- The Syringe's AmmoCharge: 500 at most, +10 every AmmoRegenRate (0.3 s)
  in its Tick, held or not (empty to full in 15 s). HUD: SYRINGE %.
- SyringeFire (left, 250 charge) heals the teammate in front of you
  within 80 units; alone there is none, so it only gives the "You must be
  near another player to heal them!" message (logged, at most every
  0.5 s).
- SyringeAltFire (alt, 500 charge): only below HealthMax with a full
  charge; plays AltFire, FireRate 3.6; InjectDelay 0.1 s later it uses the
  charge and calls GiveHealth(HealBoostAmount, 100). HealBoostAmount is
  50 (Syringe.PostBeginPlay with one player).
- KFPawn.GiveHealth: halves a burn on you; cuts the heal to what fits
  under 100 counting healing still to come (so 50 at 76 health gives
  24); adds it to healthToGive. AddHealth (Tick) pays it out: every
  0.1 s or more, int(10 x elapsed) health. Every hit you take (and every
  bile tick) takes 5 off healthToGive (KFPawn.TakeDamage).
- QuickHeal (Q): refused at full health or under 95% charge; brings the
  Syringe out; when it is ready (Syringe.Timer), fires the alt mode; if
  that fails at full health or under 75% charge it gives up, else retries
  every 0.2 s; FireRate + 0.5 s after the injection, back to the last
  weapon. With the Syringe already in hand it injects at once.
- Not done: the HUD's on-screen messages (only logged), sounds.

W8b (`projectile.rs` PlayerDart, `weapon.rs` medic HealCharge):
- KFMedicGun (MP7M, MP5M, M7A3M, KrissM) keeps HealAmmoCharge like the
  Syringe: 500 at most, +10 every AmmoRegenRate (0.3 s; MP5M and KrissM
  0.2 s). HUD: [DARTS %].
- Alt fire (MP7MAltFire / M7A3MAltFire, KFShotgunFire): AllowFire: not
  reloading, HealAmmoCharge >= AmmoPerFire (250), so two darts from full.
  DoFireEffect spawns ProjPerFire (1) darts, not ProjPerFire x Load as the
  shotgun rule does. Recoil up to 1500 up.
- HealingProjectile: Speed 10000, straight (its native ballistics for the
  first 0.1 s not reproduced), LifeSpan 10. It heals only a player it
  touches (HealBoostAmount: MP7M 20, MP5M / M7A3M 30, KrissM 40); a zed
  or the level makes it Explode: a ROBulletHitEffect, no damage (its
  HurtRadius is empty). Alone, darts only fly and burst. The MP7_Dart
  model is drawn.
- Not done: the alt fire's own muzzle flash (MuzzleFlash1stKar; the
  primary's flash plays), sounds.

W8c (`weapon.rs` FireMode.weld): WeldFire.AllowFire needs a KFDoorMover
within weaponRange (90 units) of the view; without one there is no shot
and no animation, only NoWeldTargetMessage ("You must be near a weldable
door to use the welder.", at most every 0.5 s while held; logged).
UnWeldFire (alt) fails silently. Doors are not simulated, so this is
always the case for now; welding (WeldStrength, the screen showing
"Integrity: n %", KFWelderHitEffect, the 40 per second charge) comes with
doors. Before this the Welder played its fire animation and logged
"not implemented".

**W9 ZED guns**, in three parts: W9a zapping on zeds, W9b the ZED MKII,
W9c the ZED Gun (all done; the ZED guns are optional, see W9c).

W9a (`zed.rs` set_zapped / zap_tick, `combat.rs` damage):
- KFMonster.SetZapped(amount): already zapped: back to a full ZapDuration
  (4 s); otherwise TotalZap += amount, and at ZapThreshold the zed is
  zapped. Tick: the zap runs out after ZapDuration and ZapThreshold grows
  x ZapResistanceScale (2; the Patriarch 1); zap below the threshold
  fades 1 per second once none came for 0.1 s.
- Thresholds / damage multipliers (Zombie*Base defaults): most 0.25 /
  x 2; Bloat, Siren 0.5 / x 1.5; Husk 0.75 / x 2; Scrake 1.25 / x 1.25;
  Fleshpound 1.75 / x 1.25; Patriarch 5 / x 1.25.
- Zapped (SetZappedBehavior and the zed scripts): GroundSpeed = original
  x 0.5 (the Patriarch charging or escaping: x 1.5), the burning walk
  animation, damage taken x ZappedDamageMod (after the zed's own scales,
  before the fire rule), uncloaked; no Gorefast run, Scrake rage,
  Fleshpound rage, Crawler pounce, Siren scream (a scream in progress
  does no damage), Stalker or Patriarch cloaking.
- UnSetZappedBehavior sets the normal speed; a run, rage or charge that
  was going keeps it (KF's states only set their speed on entry), until
  it ends.
- Test action `zap_zeds` (SetZapped(10) on every zed).
- Not done: the zapped overlay material (ZED_overlay_Hit_Shdr), the
  zapped hit effect, the worse aim of zapped zeds.

W9b (the ZED MKII; `weapon.rs`, `projectile.rs` ExplosiveStats.zap):
- ZEDMKIIFire: full auto every 0.125 s from a 30-round magazine
  (ZEDMKIIAmmo 120 / 240). ZEDMKIIPrimaryProjectile (a LAWProj, Speed
  1500, straight, never a dud): ProcessTouch deals Damage 50 (x 1.5 on a
  headshot, then DamTypeZEDGunMKII's x 1.1) and explodes with no
  HurtRadius (BlowUp only makes noise). Fleshpound: small-arms rule.
- ZEDMKIIAltFire: needs 15 rounds in the magazine (AmmoPerFire), one orb
  (DoFireEffect spawns ProjPerFire, not x Load), FireRate 0.75, Alt_Fire
  animation, recoil up to 1500. ZEDMKIISecondaryProjectile (Speed 1000):
  its HurtRadius SetZaps every living zed within 300 (no line of sight)
  with ZapAmount 1.5 and damages nothing.
- Both modes have bModeExclusive false: each fires while the other does.
  ZED fire modes never fire while reloading (their AllowFire).
- Each projectile class's own trail, impact effect and burn mark
  (ExplosionEmitter, FlameTrailEmitterClass, ExplosionDecal), and the
  ZED_FX_Energy_Card model.

W9c (the ZED Gun; `weapon.rs` BeamFire, `projectile.rs` beam_zap,
`zed_beam.rs`). The ZED guns cannot be bought in normal KF play; you
said they are optional, so W9c stops at working mechanics.
- ZEDGunFire: bolts like the MKII's (Damage 85, every 0.2 s, 100-round
  magazine).
- ZEDGunAltFire (a beam while held): every FireRate (0.12 s) ModeDoFire
  uses a round from the magazine, kicks the view (up to 100 up, no
  movement term) and sets bDoHit; every frame (ModeTick) a TraceRange
  (2500) trace from GetFirstPersonBeamFireStart (eye + ProjSpawnOffset)
  zaps the zed it reaches by the frame time; on a bDoHit frame that hit
  anything, every other zed in sight within 250 x ChargeUpTime /
  MaxZedSphereChargeTime (3 s) of the end gets SetZapped(FireRate x
  0.75). It starts with the Charge animation and the ChargeUp1stZEDGun
  glow, and ends (release, empty magazine, reload) with ChargeDown.
- The beam is drawn as a straight strip with ZEDBeamEffect's texture
  (FinalBlend -> TexPanner -> Texture), additive, facing the camera. KF's
  is an xEmitter beam that waves; the sparks and splash sphere are not
  drawn. The strip width (12 units) is a guess.
- Known problems, left as they are: the gun's sleeve slot (Skins[SleeveNum
  2]) is empty in the mesh, and KF fills it from the player's species
  (HandleSleeveSwapping), so the sleeves draw plain white; its screen (a
  ScriptedTexture showing the threat display) draws as scrolling lines;
  its laser threat indicator (WeaponTick) is not done.

**Not covered.** Sound (the project has no audio yet), perks (KF with no
perk chosen uses the plain values; perk bonuses come with the game loop),
the trader and buying (W1's `--give` stands in), third-person weapon
models, the flashlight, zed time.

## Doors (milestone 8, implemented 2026-10-05: D1-D4)

Goal: KF's doors. They open and close with the USE key (E), zeds open the ones
that are not welded, the Welder seals them, and zeds bash welded doors until
they break.

**What the maps hold (checked 2026-10-05, all 35 `KF-` maps).** 707
`KFDoorMover` actors and 397 `KFUseTrigger`s. Every door is a static mesh
(DrawType 8); none are BSP brushes. Other movers (plain `Mover`,
`ClientMover`, `KFGlassMover` windows, the KF-WestLondon street barrier) are
not part of this milestone and keep being drawn but not blocking.

**Rules from the scripts (checked 2026-10-05).**
- *Position.* A mover sits at `BasePos + KeyPos[KeyNum]`, turned
  `BaseRot + KeyRot[KeyNum]` (`Mover.BeginPlay`). `InterpolateTo(k, time)`
  moves from where it is now to key `k` in `time` seconds. Opening past key
  1 chains key by key (`KeyFrameReached`) up to `NumKeys - 1` (default 2).
- *Glide.* `MoverGlideType` defaults to MV_GlideByTime (smooth start and
  stop). The curve is native code; I recall it as 3a^2 - 2a^3 from the
  Unreal 1 public source. **Not verified against KF.**
- *Defaults (KFDoorMover).* InitialState TriggerToggle, MoveTime 1 (maps
  often set 0.5-2), MoverEncroachType 3 = ignore: a door swings through
  pawns, it never pushes or stops. Blocks players, zeds, bullets and
  ragdolls (Actor defaults; 23 doors switch blocking off). DamageThreshold
  50, ZombieDamageReductionFactor 0.85.
- *Trigger.* A `KFUseTrigger` is an invisible cylinder (CollisionRadius /
  Height, typically 128 x 80) whose Event names the door Tag; every door
  with that Tag belongs to it (double doors). Pressing USE calls UsedBy on
  every trigger the player's cylinder touches (`PlayerController.ServerUse`).
  UsedBy: ignored if less than ReFireDelay (KF default 2, maps mostly 1)
  since the last use (`LastAttempt` is an int, so the time is rounded down,
  copied). Unsealed, unlocked doors toggle. `bDirectionalOpen` (242 of 397
  triggers): open to key 1 if the user stands on the trigger's facing side,
  key 2 otherwise, so a door swings away from you.
- *Toggle (TriggerToggle).* Closed or still opening (KeyNum 0 or
  KeyNum < PrevKeyNum): open; else close. A door is `bClosed` only when it
  has finished closing.
- *Zeds and triggers.* `KFUseTrigger.Touch`: a zed entering the trigger
  opens each door that is unlocked, unsealed and at key 0. The player
  entering gets the message "Press USE Key" (or the map's Message) at most
  every 0.6 s.
- *Welding.* Welder WeldFire: `GetDoor` traces 70 units from the eye
  (the fire class's weaponRange, from KFMeleeFire; the Welder's own 90
  is only used for its screen read-out); a
  door must be closed (`bClosed`) to weld. Each hit (FireRate 0.2) adds
  MeleeDamage 10 to the trigger's WeldStrength, shared by all its doors, up
  to MaxWeldStrength (KF 400, maps set 500 / 600). While zeds hit the door
  (within 1 s), welding counts x CombatSealReduction 0.5. Alt fire unwelds
  the same way. Weld > 0 means sealed: sealed doors do not open, and USE
  says "This door is welded shut." `bStartSealed` doors start at
  StartSealedWeldPrc percent (19 doors); `bDisallowWeld` doors refuse (13).
- *Zeds against doors.* A zed that bumps a closed or sealed door is told to
  `BreakUpDoor`; while the door stays sealed it plays DoorBash and each hit
  does its melee damage x 0.85 (at least 5) to the weld; at 0 weld the door
  breaks (`GoBang`): hidden, no collision, wood or metal break emitter. A
  sealed door makes its path node cost 500 + weld x 6 more, so zeds prefer
  other routes. Husk and Bloat may attack doors from range.
- *Players against doors.* Players damage doors only with DamTypeFrag (the
  hand grenade) of 50 or more, unless the door has bSmallArmsDamage. An
  unsealed door has Health = MaxWeld and takes half damage.
- Doors come back at the end of each wave (`DoWaveEnd` -> RespawnDoor);
  there are no waves yet.

**Steps.**
- **D1, doors move and block.** Read KFDoorMover and KFUseTrigger from the
  map. Each door becomes its own entity (mesh + its own collider on a new
  `Door` collision layer that players, zeds, bullets and ragdolls collide
  with) instead of map geometry. Mover interpolation with keys and glide.
  E (USE, KF's key) in walk mode uses touching triggers, with the toggle,
  directional and refire rules. Zeds entering a trigger open its doors.
  Test action `use` for scripted runs. Log every state change with the
  door's position and yaw. Doors in other states (TriggerControl,
  TriggerOpenTimed: 73, opened by map events) and doors without a trigger
  stay at their start key and block; logged by name so we can check none
  of them blocks a main route.
- **D2, welding.** The Welder finds the door, weld / unweld amounts, the
  shared strength, sealed doors refusing to open, bStartSealed,
  bDisallowWeld, messages and weld percent logged (no HUD text yet).
- **D3a, zeds bash welded doors.** Bump -> DoorBash animation and damage,
  the 0.85 factor, GoBang (break emitters, door gone), path cost for
  sealed doors. Every zed with a DoorBash animation (all but the Siren
  and the Patriarch).
- **D3b, ranged door attacks.** bCanDistanceAttackDoors zeds (class
  defaults: Bloat, Husk, Siren, Patriarch) attack a sealed door their
  path trace hits from where they are; ZombieSiren.DoorAttack
  (Siren_Scream), ZombieBoss.DoorAttack (a rocket); blasts (rockets,
  fireballs, screams) hurt doors in range.
- **D4, grenades and unwelded door health.** DamTypeFrag damage,
  bSmallArmsDamage doors, Health.

**D1 as built (`door.rs`).** Doors are drawn by a root entity per door
(mesh parts as children) and collide through a separate kinematic
collider, both moved every frame from the mover state kept in Unreal
units. New collision layers `Door` (players, zeds, bodies) and
`DoorTraces` (bullets, bBlockZeroExtentTraces). Nav link checks and zed
reach probes use `zed_path_filter`, which leaves doors out, so the paths
KF built through doorways stay usable. Touch is "entered the trigger
cylinder this frame". Log lines: `doors_loaded`, `door` (open, opened,
close, closed with key, yaw, position), `use_pressed`, `message`.

What was learned:
- Rotation keys are relative to BaseRot: KF-Manor KFDoorMover5 goes from
  yaw -16384 to -1024 (KeyRot[1] 15360) or -31744 (KeyRot[2] -15360).
- KF-Hospitalhorrors has 25 doors no trigger names (nothing in the map
  opens them); KF-Aperture's 63 TriggerControl doors are driven by
  KFProxyTrigger and button movers. Both stay at their start key.
- A zed already inside the trigger when the door shuts does not reopen
  it: Touch fires only on entering, KFDoorMover.Bump -> BreakUpDoor only
  acts on sealed doors, and Controller.NotifyHitMover is empty. Copied;
  whether native code helps in KF is unknown.
- Pressing into a wall makes the floor check touch the wall at distance
  0, so walkers flicker into falling. Happens on BSP walls too (not
  door-specific); left for its own fix.

**D2 as built.** `door.rs` keeps `WeldView` (the door the view ray hits
first, with its distance) and applies `WeldHit` messages as
KFDoorMover.TakeDamage with the welder damage types, through the
trigger's AddWeld / UnWeld. `weapon.rs` `weld_fire`: ReadyToFire,
AllowFire (door within the mode's weaponRange 70; NoWeldTargetMessage /
CantWeldTargetMessage at most every 0.5 s; unweld needs weld > 0; fuel >=
AmmoPerFire), fuel, FireRate 0.2, the hit DamagedelayMin 0.1 later
traced from the view at that moment, WelderHitEmitter on the door.
Fuel (`WeldFuel`) regenerates in every frame, held or not (Welder.Tick).
Quirks copied: UnWeld has no floor, so the strength can go below 0 and
the next weld starts from there; a welder hit on a sealed door that is
not bClosed damages the weld instead. Not copied: perks
(GetWeldSpeedModifier; no perks yet), a zed between you and the door
(KF's trace would hit the zed; ours sees through zeds), welder damage to
zeds that step into the trace in the 0.1 s, the welder's screen.
Noted for D3: Mover.BeginPlay sets the Timer only in net games, so in
single player KFDoorMover.Timer never clears bZedHittingDoor; once a zed
has hit a door, welding it stays halved.

**D3a as built.** `zed.rs`: `ZedState::DoorBashing` and `door_bashing`.
A zed whose ground move is blocked by a door collider that is sealed,
visible and not bZombiesIgnore enters it (KFDoorMover.Bump ->
BreakUpDoor; for a closed but unsealed door the KF loop would end at once,
so we do not enter). Each pass: the full-body DoorBash; each
ClawDamageTarget notify of that animation sends `ZedDoorHit` (MeleeDamage
-5% .. +5%; DoorBash notifies: Clot 2, Fleshpound 3, Scrake 2, others 1);
then the wait the latent code gives (next 0.25 s poll after the animation
+ 0.1 s); then, for Intelligence >= BRAINS_Mammal (Clot 3 by KFMonster's
default, Husk 2, Bloat 1), `nav::probe_with` with doors blocking stands
in for ActorReachable(Enemy): reachable -> back to the chase. `door.rs`
`zed_damage`: Max(5, int(int(claw) x 0.85)) off the shared weld (or half
of it off Health when unsealed); weld 0 -> every door of the trigger goes
GoBang: hidden, collider on no layers (kept for a later RespawnDoor),
KFDoorExplodeWood / Metal (SurfaceType 3 = EST_Metal; the map override of
the effect class on 3 doors is not read). `door_path_costs`: DoorPathNode
found once colliders exist (ray hits along each link from points within
800), ExtraCost every 0.5 s into `NavNetwork::extra_cost`, which routing
adds when entering a point. Test action `toggle_zeds` (the X key).

**D3b as built.** `door.rs` `DoorBlast`: radius damage to doors measured
to the door's Location (its pivot), scale 1 - distance / radius, a
direct hit at full damage and left out of the radius part
(Projectile.HitWall), a line-of-sight test (level only) for the Siren's
VisibleCollidingActors, none for LAWProj's CollidingActors. Our test is
the pivot inside the radius; KF's CollidingActors checks the door's
collision bounds (native), so doors whose pivot is just outside may be
missed. Sent by `fireball.rs` (Husk fireball, Patriarch rocket; a hit on
a door collider is direct) and by the Siren's scream pulses. `zed.rs`
`door_bashing` per class: DoorBash; ZombieBarf / ShootBurns (22 per
SpawnTwoShots) when bDistanceAttackingDoor; Siren_Scream (ScreamDamage x
0.6 per pulse, none while zapped; headless: nothing, the loop waits);
the Patriarch's missile (boss_busy aims at the door, then back to the
loop). FindPath's check: when a bCanDistanceAttackDoors zed takes a new
path point, a ray to it that hits a sealed door starts DoorBashing with
bDistanceAttackingDoor. Changed with it: the router's run-time reach
tests (ActorReachable / pointReachable stand-ins) now count closed doors
(native; assumed, not verified), so a zed behind a shut door follows the
path through the doorway instead of walking straight at the player; the
start-up link check still ignores doors. Not done: Bloat vomit globs
sticking to doors and bursting there (KFBloatVomit.HitWall on a mover).

**D4 as built.** `projectile.rs` `blast` sends every player blast as a
`DoorBlast` (no line of sight: Nade, M79GrenadeProjectile and LAWProj
use CollidingActors), flagged `frag` only for the Nade (MyDamageType
DamTypeFrag; the M79, M32, M203, LAW and pipe bomb have their own types,
so KF doors ignore them). `door.rs` `player_damage`: KFDoorMover
.TakeDamage's filter ((not bSmallArmsDamage and not DamTypeFrag) or
int damage < DamageThreshold 50: nothing), then unsealed: half off
Health (MaxWeld at the start), sealed: the whole off the weld; 0 breaks.
No map sets bSmallArmsDamage, so bullets and melee never hurt doors.
Log line `door_damage` (by=zedN or player). Test spot note: on
KF-Manor the floor steps up 42 units just in front of KFDoorMover5 on
the -2075 side; shoot from the flat side (x -1980, yaw 0).

Not in this milestone: sounds (no sound yet), keys for locked doors
(bKeyLocked, 3 doors: stay locked), on-screen messages and the weld bar
(HUD milestone), door respawn (waves milestone), other movers.

## Game loop: waves, trader, door respawns (milestone 9, planned 2026-10-05; G1, G2a, G2b implemented)

Goal: play a KF game solo: waves of zeds from the map's spawn volumes, the
trader between waves with dosh to spend, broken doors back each wave, the
Patriarch at the end. Rules researched from KFGameType, ZombieVolume,
ShopVolume, KFPawn and the class defaults (checked 2026-10-05; numbers
are for one player on Normal difficulty).

**Modes (your requirement).** `--mode waves` runs the game loop. `--mode
debug` is today's behaviour, unchanged: no waves, no trader, zeds from
`--spawn` / `--zed-at` and the spawn keys, every test action as now.
Debug stays the default so every existing test command still means the
same thing (your call: it can be the other way round). `--length
short|normal|long` picks the game length; KF's own default (KillingFloor.ini
KFGameLength=0) is Short.

**Rules from the scripts.**
- *Flow* (KFGameType state MatchInProgress, Timer once a second): a 10 s
  countdown, wave 1 (no trader before it), then each wave end: doors
  respawn, team dosh paid out, WaveNum + 1, 60 s of trader time
  (TimeBetweenWavesNormal), next wave. After the last wave the boss wave
  (the Patriarch, ZombieBoss_STANDARD); killing him wins. Dying loses
  (solo: no respawn).
- *Waves:* Short 4, Normal 7, Long 10, then the Patriarch. Zeds per wave
  (WaveMaxMonsters x 1.0 Normal x 1.0 for one player): Normal 20, 28, 32,
  32, 35, 40, 42; Short 20, 32, 35, 42. At most 32 alive at once.
- *Which zeds:* 27 standard squads ("4A1G" = 4 Clots and a Bloat; letters A
  Clot, B Crawler, C Gorefast, D Stalker, E Scrake, F Fleshpound, G Bloat,
  H Siren, I Husk); each wave's WaveMask enables some. A squad is drawn at
  random without repeats until the list is used up, then it refills. Some
  waves have a special squad (Fleshpounds, Scrakes, Sirens) that comes in
  on odd passes through the list.
- *Spawn timing:* the next squad after WaveSpawnPeriod (map's KFLevelRules,
  KF-Manor 2.5 s; x 1.1 in later waves), stretched by up to 3x by a sine
  of the wave's elapsed time.
- *Where:* ZombieVolumes. Each gets an 11 x 11 grid of spawn points that a
  zed fits on. A volume is rated (desirability, distance to the player,
  time since last used, a random part) and refused if: the player can see
  it or is inside it, it is within MinDistanceToPlayer (600), it failed in
  the last 5 s, one of its RoomDoorsList doors is welded, or it does not
  allow that zed type. A spawn point the player can see is skipped.
- *Wave end:* every zed spawned and dead. With 5 or fewer left, a zed not
  seen for 8 s is killed (one a second) so a lost zed cannot stall the
  game.
- *Zed scaling:* one player on Normal: normal health, damage x 0.75
  (KFMonster DifficultyDamageModifer). We do not apply the 0.75 yet.
- *Dosh:* start 250. A kill pays ScoringValue (Clot 7 .. Fleshpound 200,
  Patriarch 500; x 1.75 on Short) at once, and the same again into the
  team pot paid out to the survivors at wave end (so solo, twice; read
  from the code, not seen in game). Death costs 10%.
- *Trader:* one of the map's ShopVolumes (KF-Manor 5), random, never the
  same twice running; its KFTraderDoor opens for trader time and closes at
  the wave start; anyone inside then is teleported to one of its 6
  Teleporters. USE inside the open shop opens the buy menu. A trail shows
  the way (RedWhisp along the path) and a HUD arrow points at it.
- *Buying* (KFPawn.ServerBuy*): weapons at Pickup Cost (no perk: no
  discount), carry weight <= 15, not already owned, dual pistols half price
  with the single; a bought weapon comes with InitialAmount ammo. Selling
  pays 75%. Ammo per magazine at AmmoCost (partial if short of dosh);
  grenades 40 each, up to 5; armour 300 for 100 points. Base-game weapons
  only (no DLC), as in the weapons milestone.

**Steps.** Each step logs its state changes and is tested by a logged run.
- **G1, modes and the wave state machine.** `--mode`, `--length`; the
  countdown, wave start / end, between-wave time, WaveNum, boss wave,
  won / lost; zeds per wave and squads (the squad tables and masks from
  the class defaults); a HUD line (wave, zeds left, countdown). Spawning
  in G1 is simple (random ZombieVolume centre) to get the loop running.
- **G2a, ZombieVolume spawning.** Read the volumes (with their array
  properties: RoomDoorsList, DisallowedZeds, OnlyAllowedZeds), the spawn
  point grid, the rating and refusals, sight checks.
- **G2b, being seen.** LastSeenOrRelevantTime, HiddenGroundSpeed (unseen
  zeds move at 300), the stuck-zed cleanup, zed damage x 0.75 for one
  player (MeleeDamage, ScreamDamage, SpinDam; whole numbers, at least 1).
  Hidden speed and the damage scaling apply in debug mode too (your
  call: they are KF zed rules, not wave rules).
- **Retro and map audit (done 2026-10-05).** `docs/map-audit.md` (every
  placed class that matters, KF behaviour, ours, priority) and
  `docs/retro-2026-10-05.md`; maps log `map_features` at load.
- **G3a, the Patriarch wave rules (done 2026-10-05).** From KFGameType (MatchInProgress
  Timer, StartWaveBoss, AddBoss, AddBossBuddySquad, DoBossDeath) and
  ZombieBoss (state KnockDown, Died):
  - StartWaveBoss: TotalMaxMonsters 1, MaxMonsters 1, WaveEndTime = now
    + 60. Each tick while TotalMaxMonsters > 0 and before WaveEndTime:
    AddBoss. After that (or once he is spawned) the wave ends when
    NumMonsters is 0, so if no volume takes him within 60 s the wave
    ends empty and the game is won (a KF quirk, kept).
  - AddBoss: FinalSquadNum = 0; if no LastZVol, FindSpawningVolume
    (boss rating), then again ignoring the 5 s failed-spawn wait; none:
    try next tick. SpawnInHere with bTryAllSpawns (every spawn point
    tried, in random order with repeats, not 3) and 32 "at once".
  - Helpers: at the end of his KnockDown animation, if FinalSquadNum ==
    SyringeCount, AddBossBuddySquad: TotalZeds 8 for one player; up to
    10 passes, each takes FinalSquads[FinalSquadNum] (KFMonstersCollection
    defaults, 3 squads), trims it so the total stays at 8, finds a
    volume (normal rating) and spawns ignoring MaxMonsters and
    TotalMaxMonsters (999); then FinalSquadNum + 1. So at most one helper
    group per syringe, the first knockdown uses squad 0.
  - DoBossDeath: every other zed goes to state GameEnded (TurnOff: it
    freezes in place, animation stopped, ignores damage, its controller
    is destroyed), so it no longer counts as a monster; NumMonsters
    drops to 0, the wave ends, WaveNum passes FinalWave and the next
    tick ends the game won. Not done: the double-length zed time (no
    zed time yet) and the death camera on him (G3b).
- **G3b, the grand entrance (done 2026-10-05).** From KFGameType's
  boss-wave Timer, ZombieBoss (MakeGrandEntry, MakingEntrance,
  InitialSneak, Died, SetBossLaught), KFGameType.BossLaughtIt and
  PlayerController.CalcBehindView:
  - Before MakeGrandEntry the Patriarch has no state (ZombieBoss has no
    auto state): he hunts normally, uncloaked. Ours today starts him in
    InitialSneak at spawn (the entrance skipped); that moves to after the
    entrance. A Patriarch spawned outside the boss wave (debug Z / N or
    `spawn_patriarch`) never gets MakeGrandEntry, so, as in KF, he never
    starts with InitialSneak.
  - The boss-wave Timer (once a game second), first time TotalMaxMonsters
    <= 0 and NumMonsters > 0 (he has spawned): MakeGrandEntry: bShotAnim,
    no movement, the Entrance animation (full body, waits), state
    MakingEntrance (ignores RangedAttack) for GetAnimDuration('Entrance'),
    then InitialSneak (cloak). The view goes to him (SetViewTarget,
    bBehindView). BossBattleSong: no sound yet.
  - Each later Timer while he is the view target: once bShotAnim is off
    (the animation done), the view returns to the player's own.
  - ZombieBoss.Died: the view goes to him in third person (with
    DoBossDeath's 6 s zed time, done); it stays until the game is over
    (ours: until the restart).
  - The players lose in the boss wave (BossLaughtIt): a living Patriarch
    plays VictoryLaugh (full body) and the view goes to him
    (SetBossLaught sets bSpecialCalcView, but ZombieBoss has no
    SpecialCalcView, so it is the plain behind view; copied).
  - The behind view (CalcBehindView): rotation = the player's own view
    rotation (mouse look still turns it, roll 0); location = his
    Location + 12 up, then back along the view by Dist = CameraDist 9 x
    his default CollisionRadius (26: 234 units), shortened to a 10-unit
    box trace hitting the level. The player's own pawn still takes input
    (KF does not stop it). The first-person weapon is not drawn in behind
    view. Ours: the render camera's GlobalTransform is replaced after
    transform propagation, so gameplay keeps the player's own view;
    the weapon camera is switched off.
  - Logs `boss_entrance` (start, end), `view_target` (boss / player,
    reason).
  - As built: the behind view's trace is a 10-unit box shape cast
    against the level (World, TraceBlocking). Not done: BossBattleSong
    (no sound), bBlockCloseCamera / CameraDeltaRotation (zero in KF's
    defaults), the view's zone fog and sounds (they stay the player's).
- **D5, doors respawn at wave end (done 2026-10-05).** KFDoorMover.RespawnDoor: back, shut
  (or open if it was), bStartSealed doors re-welded. Plan (from
  KFGameType.DoWaveEnd, KFDoorMover.RespawnDoor / DoOpen / DoClose /
  DoOpenToKey / DoCloseToFirst, Mover.Reset and TriggerToggle.Reset):
  - When: DoWaveEnd (also after the boss wave), every KFDoorMover
    (KFTraderDoor is a plain Mover: not included). `game.rs` sends a
    `RespawnDoors` message from `do_wave_end`; `door.rs` handles it.
  - Every door: Health = MaxWeld (a damaged but unbroken door is healed;
    its weld is left as it is).
  - A broken door (bDoorIsDead) only: shown again, collision back,
    no longer dead; then Reset(): Mover.Reset runs DoClose (one key back
    over MoveTime, KeyFrameReached carries it down to key 0) and leaves
    the state's latent code stopped. bClosed is not touched (only the
    state code sets it), so a door broken while open comes back closed
    but not bClosed and cannot be welded until opened and shut again
    (KF quirk, copied). Then, if bShouldBeOpen, InterpolateTo(last key,
    0.001); else if KeyNum != 0, InterpolateTo(0, 0.001). Then
    bStartSealed doors: bSealed, trigger weld 0, AddWeld(MaxWeld x
    StartSealedWeldPrc / 100).
  - bShouldBeOpen (new `Door::should_be_open`): set true by DoOpen /
    DoOpenToKey and false by DoClose / DoCloseToFirst only when they are
    skipped because the door is sealed or hidden. Our use and touch code
    (like KFUseTrigger) does not trigger sealed or hidden doors, so in
    practice it stays false and doors come back shut.
  - Logs `door_respawned` (door, key, closed, sealed, weld) and
    `doors_respawn` (count broken, count healed).
  - Test action `break_doors` (every door with a trigger goes bang) to
    check without waiting for zeds.
- **T1, dosh (done 2026-10-05).** Starting cash, kill rewards, team pot, death penalty, HUD.
  From KFGameType (ScoreKill, ScoreKillAssists, RewardSurvivingPlayers,
  the StartingCash table) for GameDifficulty 2 (KillingFloor.ini):
  - PRI.Score (the player's dosh, a float, shown whole) starts at
    StartingCashNormal 250; a restart resets it.
  - A zed killed by the player (Died with the player as Killer: a shot,
    blast, burn, or bleeding out after the player took its head) pays
    KillScore = Max(1, int(ScoringValue x 1.0 (Normal) x 1.75 (Short
    only))). ScoringValue from the KFMod Zombie*Base defaults: Clot 7,
    Crawler 10, Gorefast 12, Stalker 15, Bloat 17, Husk 17, Siren 25,
    Scrake 75, Fleshpound 200, Patriarch 500. ScoreKillAssists gives it
    to everyone who damaged the zed, split by damage; solo that is all
    of it to the player. Team.Score also gets KillScore.
  - Kills that are not the player's pay nothing: the level (lava,
    KillZ), the stuck-zed cleanup (KilledBy itself), the boss's death.
  - DoWaveEnd (boss wave too): RewardSurvivingPlayers: a living player
    gets the whole Team.Score, which goes to 0. A dead one gets nothing
    (MinRespawnCash only matters with respawns: not solo).
  - The player's death (ScoreKill on the player): Score -= Score x
    GameDifficulty x 0.05 (10%; the script's comment says 15%), and the
    team pot loses 10% of the new score. Solo waves end there; it
    matters in debug mode.
  - HUD: the dosh as a whole number ("DOSH 250"; the HUD font has no £); logs `dosh` on each
    change (reason, amount, total, team pot).
  - Test kills (`kill_zeds` and the like) count as the player's, as they
    already count in the kill counter (not KF: its KillZeds pays
    nothing). Logged so it is clear.
- **T2, shops.** ShopVolumes, trader doors, which shop, booting players
  out with the teleporters, the trail (and an arrow / distance on the HUD).
  Split in two:
  - **T2a, shops and trader doors (done 2026-10-05).** From KFGameType (SelectShop,
    OpenShops, CloseShops, BootShopPlayers, the MatchInProgress Timer),
    ShopVolume, KFTraderDoor, Teleporter.Accept, HUDKillingFloor:
    - Load every ShopVolume (brush, URL, Event, bAlwaysClosed,
      bAlwaysEnabled) and every Teleporter (Tag, Location, Rotation).
      KF-WestLondon: 4 shops, 24 teleporters.
    - KFTraderDoor (a Mover, InitialState TriggerToggle, MoveTime 1) is
      read like a KFDoorMover and moved by the same mover code, but kept
      apart from the doors you weld (no trigger, no welding, no damage,
      zeds do not bash it; to them it is a wall).
    - OpenShop / CloseShop trigger every actor whose Tag is the shop's
      Event: the trader doors toggle. Other actors with that tag are
      logged as not simulated.
    - SelectShop: random among the not-always-closed shops; if it is the
      current one, the next in the list (wrapping).
    - Timer, between waves: once WaveNum != InitialWave (0) and the
      doors are not open, OpenShops (bAlwaysEnabled shops, then the
      current shop, picked if none). Any between-wave tick with no
      current shop picks one (so it is known before wave 1).
    - Timer, during a wave (boss wave too): if the doors are open,
      CloseShops (every open shop, then SelectShop for next time), then
      boot whoever is in any shop.
    - Touch while no wave is running: entering a shop that is not open
      boots you; entering the open one shows "Press USE to TRADE".
      Touching is approximated as the player's centre inside the brush.
    - BootPlayers: to a random Teleporter whose Tag is the shop's URL;
      yaw = teleporter yaw + 32768 + your yaw - the shop's yaw
      (bChangesYaw); message "You can't stay in this shop after
      closing". A shop with no such teleporters cannot boot.
    - HUD: "TRADER: N m" whenever a shop is current, N = int(distance /
      50) (DrawTraderDistance; KF draws it always, waves included);
      messages for 3 s.
    - Not done here: the trail, the HUD arrow (T2b), pawn collision off
      during trader time (no other players), the trader's voice lines and
      animation (WeaponLocker), USE in the shop (T3).
  - **T2b, the way there.** The red whisp trail along the path every
    TraderPathInterval, and the HUD's 3D arrow. Split in two:
    - **T2b-1, the trail (done 2026-10-05)** (`trader_path.rs`). From KFPlayerController
      (SetShowPathToTrader, Timer; TraderPathInterval 1.1,
      bWantsTraderPath true), KFGameType (OpenShops, CloseShops,
      ShowPathTo), ShopVolume.Touch, TraderPathEffect / RedWhisp,
      WillowWhisp:
      - On when the shops open (OpenShops): one whisp at once, then
        one every 1.1 s. Off when they close (CloseShops, the wave
        start) and when the player touches the open shop
        (ShopVolume.Touch). Once off it stays off until the next trader
        time, even after leaving the shop (KF quirk, copied).
      - Each tick (ShowPathTo): only if there is a current shop, it has
        a teleporter (TelList[0]: the first Teleporter whose Tag is the
        shop's URL) and FindPathToward that teleporter finds a route.
        The route: our nav route from the player (start points within
        reach, as the zeds' FindPathToward) to the teleporter, up to 16
        points (RouteCache).
      - The whisp's way points (TraderPathEffect.PostBeginPlay): [0] =
        200 units ahead along the view (cut short by a wall trace);
        then up to 10 route points, from RouteCache[1] if
        RouteCache[i] (i unset, so 0) exists, RouteCache[1] exists and
        is reachable in a straight line; then the shop's Location if
        fewer than start + 10 points were taken. No height offset
        (WillowWhisp adds CollisionHeight; KF's version does not).
        Velocity = 500 toward [0] plus the player's velocity.
      - Its flight (WillowWhisp, script, exact): StartNextPath (next way
        point; acceleration 1200 toward it; velocity halved; Z = half
        (Z + accel Z)); each tick acceleration re-aimed, velocity +=
        accel x dt in the script and again by PHYS_Projectile; next way
        point once within 80 units or once it overshoots (velocity turns
        away after having pointed at it). After the last point: no new
        sprites, gone 1.5 s later. LifeSpan 10 s. Flies through walls.
      - Its sprites (xEmitter, native: not in the scripts, so assumed
        from the settings and labelled): 90 a second (mRegenRange) at
        the head, at most 150, still (mSpeedRange 0), each 1.25 s
        (mLifeRange), size 25-30 units growing 13 a second, colour
        (255, 40, 40), a random tile of RedWhisp's 4 x 4 texture
        (Skins[0]), random spin, drawn additively (Style 6), fading in
        and out (mAttenuate; the exact curve is a guess), drifting up a
        little (mMassRange -0.03 to -0.01 under gravity; a guess).
        The texture (ROEffects.SmokeAlphab_t) is grey with its shape in
        the alpha; the glow is weighted by that alpha (assumed: drawn
        without it, the sprites are visible grey squares).
      - Logs `trader_path` (on / off, reason), `trader_whisp`
        (spawned, route, way points, or why not), `trader_whisp_end`.
    - **T2b-2, the arrow (done 2026-10-05).** KFShopDirectionPointer
      (DebugObjects.Arrows.debugarrow1, DrawScale 0.25) drawn in the
      top-left corner over the view, pointing at the shop (level unless
      the shop is more than 50 units above or below), whenever the
      match has begun and there is a current shop (waves too). Plan
      (HUDKillingFloor.DrawKFHUDTextElements, KFShopDirectionPointer):
      - Not drawn without a current shop, while the buy menu is open
        (bShopping), or outside wave mode.
      - Place: Pos = ScreenToWorld(SizeX / 18, SizeX / 18) x 10 x
        (DefaultFOV / FovAngle) + the view location: 10 units along the
        ray through the screen point (SizeX / 18, SizeX / 18), both
        coordinates from the width (KF's code), further when zoomed so
        it keeps its size. The ray is taken from our camera's own
        projection. **As built:** with those numbers the arrow came out
        about 7.75 times too big against a real-game screenshot
        (ScreenToWorld is native; its scale is unknown), so the
        distance is multiplied by a fitted 7.75 (`ARROW_DISTANCE_FIT`,
        measured at 1280 x 960 from the same spot and view: real x
        13-122, y 50-89; ours x 14-121, y 48-90). The place on screen
        matched without fitting.
      - Turn: rotator(shop - pawn), made level unless the shop's
        Location is more than 50 units above or below the pawn's.
      - Drawn after the Z buffer is cleared (C.DrawActor(None, False,
        True)): on top of everything, through walls and the weapon. Its
        own camera (order 3, after the vision overlay: the arrow is not
        tinted; assumed order) and render layer. Unlit (Effects
        bUnlit), DrawScale 0.25, the mesh's own texture
        (KillingFloorHUD.Generic.debuggarrow1, opaque).
      - Logs `trader_arrow_ready` (mesh, bounds) and `trader_arrow`
        every 2 s (shown, yaw, pitch, distance).
- **T3, buying.** A simple keyboard buy menu (text list on screen):
  weapons, sell, ammo (clip / fill), grenades, armour; all the
  server-side rules above. Armour itself (absorbing damage) needs reading
  KFPawn's armour rules first; may become its own step.
  - **T3a, weapons and ammo (done 2026-10-05).** From KFPawn (CanBuyNow, ServerBuyWeapon,
    ServerSellWeapon, ServerBuyAmmo), KFHumanPawn (CanCarry,
    MaxCarryWeight 15), ShopVolume.UsedBy, KFLevelRules:
    - What is for sale (KFBuyMenuSaleList): one list at a time, picked
      with the perk filter (Medic, Support, Sharpshooter, Commando,
      Berserker, Firebug, Demolitions, Neutral: the map's KFLevelRules
      MediItemForSale .. NeutItemForSale, else the class defaults),
      keeping the base-game weapons we have (BASE_WEAPONS). Hidden:
      weapons owned, bKFNeverThrow weapons (knife, 9mm, frag, syringe,
      welder), a single pistol while its duals are owned. A dual whose
      single is owned shows half its Cost (int) and half its Weight.
      Per item from its pickup and weapon class: ItemName, Cost,
      AmmoCost, Weight, BuyClipSize. Logged at load.
    - The menu opens with USE (E) while standing in the open shop with
      no wave running (UsedBy -> ShowBuyMenu); Escape or E closes it, and
      it closes when the wave starts (BootPlayers closes menus). While it
      is open the player does not move, look or fire (KF's menu takes
      the mouse; the game keeps running). Keys: Up / Down move, Left /
      Right change the perk filter, Tab switches between "For sale" and
      "Yours", Enter buys or sells, F
      fills the selected weapon's ammo, C buys one magazine, Shift+C /
      Shift+F the same for its second ammo (M4 203 grenades).
    - CanBuyNow: no wave in progress and inside a shop (any shop; KF's
      check is any touching ShopVolume).
    - Buy: refused if owned, if CurrentWeight + ItemWeight > 15 (the
      list's weight, halved as above), or if Score < Price. Price = Cost
      (no perk discount: perks are not in), halved for the dual version
      of an owned single (Dualies is not halved: KF has no such rule for
      it). The weapon comes with its
      InitialAmount ammo (FillToInitialAmmo); SellValue = Price x 0.75;
      it is brought up (ClientForceChangeWeapon). A dual replaces its
      single (Dualies.GiveTo), as `--give` already does, ammo merged.
    - Sell: + SellValue (int(Cost x 0.75) if never set, e.g. starting
      weapons); selling duals gives the single back (Dualies -> 9mm;
      the others with SellValue = Price / 2, and the duals pay Price /
      2). Knife, 9mm, Frag, Syringe, Welder cannot be sold (bKFNeverThrow
      / not in the lists: checked from the class defaults).
    - Ammo: refused at MaxAmmo. Clip price = AmmoCost; amount = MagCapacity
      (1 for a secondary ammo, BuyClipSize for the Husk Gun) for a clip,
      MaxAmmo - AmmoAmount for a fill; Price = int(amount / MagCapacity x
      AmmoCost). Short of dosh: amount x Score / Price, at least 1, paid
      pro rata; Score is then rounded down (int) after a full buy, as KF.
      Frags and pipe bombs are ammo of their weapon (KF lists them so).
    - Loading: a weapon is loaded from the game files the first time it
      is bought (the package set is kept for it); about 0.1-0.3 s, during
      trader time.
    - Logs `shop_catalogue`, `shop_buy`, `shop_sell`, `shop_ammo`,
      `shop_refused` (reason), `buy_menu` (open / close).
  - **T3b, armour (done 2026-10-05).** Vest (300 for 100 points, partial), ShieldStrength
    and KFPawn's absorption rules, the HUD. Plan (from KFPawn.ShieldAbsorb,
    KFPawn.ServerBuyKevlar, Pawn.TakeDamage, KFBuyMenuInvList,
    HUDKillingFloor):
    - State (`armour.rs`, resource `Armour`): ShieldStrength (a float,
      0-100) and xPawn's SmallShieldStrength (float). Both 0 at the start
      and after a death (a new pawn); no perks, so no starting armour.
    - Which hits armour stops: Pawn.TakeDamage calls ShieldAbsorb only if
      the damage type's bArmorStops and the damage (after ReduceDamage)
      is over 0. bArmorStops is true (DamageType's default) for every KF
      damage type except SirenScreamDamage, Fell, Crushed (and
      DamTypePoundCrushed), Gibbed, Suicided, DamTypeTelefragged,
      Drowned, Depressurized (scanned with `kfpkg defaults` over every
      DamageType subclass). Ours: the Siren's scream and falling out of
      the world (Gibbed) skip armour; zed hits, bile, fire, the Husk,
      the Patriarch, pain volumes and your own explosives go through it.
      `PlayerDamaged` gains `armor_stops`.
    - ShieldAbsorb, copied line by line (int damage in, int out,
      truncated): while ShieldStrength > SmallShieldStrength the vest
      takes 0.75 x damage and you take 0.25 x damage; when the vest runs
      out mid-hit you take the rest less what the vest had. The second
      half (ShieldStrength >= 0.5 x damage) only runs once
      SmallShieldStrength is above 0, which nothing in KF's solo game
      sets: copied anyway, with a comment. No perk modifier
      (GetBodyArmorDamageModifier) as perks are not in.
    - God mode: KFHumanPawn.TakeDamage returns first, so armour is not
      used up.
    - Buying (ServerBuyKevlar): refused outside CanBuyNow or at 100.
      Cost = 300 x (100 - ShieldStrength) / 100 (a float, taken off the
      float Score); with enough dosh, ShieldStrength = 100. Short of
      dosh: only if ShieldStrength > 0 (KF quirk: with no armour and
      under 300 nothing happens), buy int(Score / 3) points for
      int(3 x points). SmallShieldStrength is not touched.
    - Menu: a "Combat armour" row at the end of "Yours" (KF adds the vest
      after the weapons), showing int(ShieldStrength)/100 and the fill
      price int((100 - ShieldStrength) x 3); Enter or F buys. Test
      action `buy_vest`.
    - HUD: "ARMOUR n" next to health (HUDKillingFloor draws
      ArmorDigits = ShieldStrength always, as an int).
    - Logs: `shop_vest` (bought, cost, armour), `shop_refused`
      (request=vest), and `player_hit` gains armour before / after.

**G1 as built (`game.rs`).** `load_game_data` (called by the map loader
in wave mode) reads the length's WaveInfo array (WaveMask int,
WaveMaxMonsters byte), StandardMonsterSquads (27 strings) and the
KFMonstersCollection MonsterClasses (MID letter, MClassName) and special
squads (struct arrays: `properties::string_array` / `struct_array` decode
them), EndGameBossClass, the map's KFLevelRules WaveSpawnPeriod and its
ZombieVolumes. `wave_timer` runs MatchInProgress.Timer once a game second:
the countdown (10, then 60), SetupWave, AddSquad (special squad on odd
passes; as in KF the first AddSquad of a game, having no volume yet,
draws a fresh squad over SetupWave's), CalcNextSquadSpawnTime, DoWaveEnd,
the boss wave, won / lost. Zeds are spawned through `SpawnZedAt`, handled
in `zed.rs` next to the debug spawns. G1 spawns at a random volume's pivot
dropped to the floor (G2 replaces this). Losing: KF ends the game when the
solo player dies; we stop the loop and show "YOU DIED"; Enter (test
action `restart_game`) clears the zeds and starts over (the debug respawn
at the start is kept). Test aids: `--wave N` (start at wave N; one past
the last is the Patriarch), test actions `next_wave` (end the countdown)
and `kill_zeds`. HUD: wave, zeds left, countdown.

**G2a as built (`zvolume.rs`).** Volumes read with their class defaults
(SpawnDesirability 3000, MinDistanceToPlayer 600, TouchDisableTime 10,
the b*Zeds flags...), the brush polygons in world space (Encompasses:
point inside by ray parity), RoomDoorsList (door by object name),
DisallowedZeds / OnlyAllowedZeds (class names; each wave zed's chain is
checked with ClassDefaults.is_a), each zed's ZombieFlag and cylinder.
Spawn points are built on the first wave tick (colliders exist then).
Native SetLocation's move to a free spot is approximated by lifting the
26 x 44 tester up to 44 units: without it 5 KF-Manor volumes, whose pivots
sit 20-42 above the terrain, had no points. `game.rs`: FindSpawningVolume
(LastSpawningVolume x 0.2, refusal reasons logged when none is left),
SpawnInHere (type filter edits the squad in place: refused zeds are lost,
as in KF; ZombieCountMulti; 3 random points the player cannot see;
native Spawn's fit test approximated by the zed's own cylinder not
overlapping the level, raised to stand where the tester stood), the
failed / TryToSpawnInAnotherVolume path, Touch disabling a volume for 10 s
when the player walks in. Distance fog is not modelled (KF skips the
sight test beyond DistanceFogEnd).

**Fix while testing G2a (you found zeds stuck behind the tall fence
behind the KF-WestLondon start).** That spawn area is a pocket left by a
JumpPad: UTJumppad0 throws zeds over the fence (JumpVelocity (-268, 254,
729), JumpZModifier 3.7) to PathNode111, and every other link out of it is
a jump (R_JUMP) link. Our nav had dropped both (jump and special links),
so the pocket was cut off (`nav_groups` now logs connected groups: 11
groups before, 1 after). Now: R_JUMP links are used (the start-up check
drops the ones our zeds cannot jump); JumpPads are read from the map
(`NavNetwork::add_jump_pads`; KF-WestLondon has 6), their pad -> target
link skips the check, and a zed touching a pad (40 x 43 cylinder) is
thrown with its JumpVelocity (JumpPad.PostTouch). Launched from the pad's
centre: KF launches on first touch (up to 66 units off), which on that
pad hits the fence top; the editor computed the velocity from the centre,
and whatever native detail gets KF's zeds over is not known
(approximation). Players are not thrown yet. Also fixed in `walk.rs`
(shared by the player and zeds): a sweep along a surface the cylinder
touches reported a hit at distance 0 (walking flickered into falling
against walls; a falling zed sliding down a wall in a corner stuck in the
air); such a hit is now re-swept from a skin off the surface, on
whichever side is free. Still seen on KF-WestLondon: 2 of 20 zeds wedged
in the falling state in tight spots (two surfaces at once); KF's answer
to stuck zeds is the cleanup in G2b.

**G2b as built.** `zed.rs`: MeleeDamage and ScreamDamage x 0.75, whole,
at least 1 (`solo_damage`, at class load: every zed, both modes). Each
frame for a zed with its head and not zapped: "drawn" (LastRenderTime,
native) is approximated as within 60 degrees of the view and in clear
sight; drawn in the last 5 s = seen; otherwise every second a sight
check from 0.8 of its half height up to the player's eye: clear = seen,
else hidden. Hidden zeds walk at HiddenGroundSpeed (300) instead of their
GroundSpeed; their special speeds (running, raging, charging, headless)
are left alone (KF's states set those themselves; their interplay with
the hidden speed is not checked). `game.rs`: with every zed spawned and 5
or fewer alive, one zed a tick that CanKillMeYet (unseen over 8 s; from
the final wave on, any) is killed with no credit (`KillStuckZed`). Test
action `kill_near_zeds` (zeds within 500 of the player die).

Not in this milestone: pickups lying in the map (SetupPickups), dropped
weapons, perks, multiplayer, voice lines and sounds.

## Map fixes (milestone 10, planned 2026-10-05; M1-M6 implemented)

Goal: the map features the audit (`docs/map-audit.md`) found missing that
change what zeds and the player do, so map gaps stop looking like AI
errors. You asked for these before the rest of the game loop.

- **M1, glass windows (KFGlassMover, 691 in 12 maps).** Rules
  (KFGlassMover.uc, checked 2026-10-05): Health 50 (maps set their own,
  e.g. 5); blocks pawns, bullets and Karma; not path-colliding ("They do
  not block paths"). TakeDamage from anything: Health -= damage; at 0
  BreakWindow (no collision, hidden, BreakWindowGlassEmitter), and every
  pane sharing its Tag (unless the Tag is empty or the class name) cracks
  with Health 1 (ShatterOtherWindows); above 0 a WindowGlassEmitter
  (ShardWindow). Health 1 at the start = cracked (Skins[0] =
  ShaderCrackedGlass). Bump: the player does nothing to an uncracked pane;
  anything moving at 10+ deals its speed as damage; a zed also plays
  MeleeAnims[0] standing still (HandleBumpGlass, WaitForAnim) and deals its
  MeleeDamage. The Siren's scream always shatters glass in its radius
  (damage 100000). Event triggers on break are not simulated.
  As built (`glass.rs`): panes are their own entities (hidden when
  broken) with a static collider on the Door / DoorTraces layers (block
  players, zeds, bodies, bullets; path checks ignore them). Damage from
  instant-hit shots and melee traces that hit a pane (combat.rs), blasts
  (`DoorBlast`, read by glass.rs too), player bumps (walk.rs) and zed bumps
  (zed.rs: speed + MeleeDamage, MeleeAnims[0] full body, no damage to
  the player). Pellets, nails and bolts hitting a pane do their Damage to
  it (HitWall on a non-static actor) and stop.
  Checked on KF-WestLondon: a 9mm shot broke KFGlassMover94 (35 vs Health
  5) and the next shot went through; a Clot walking into it broke it
  (speed 300, hidden); a grenade broke the 7 panes within 420. Found on
  the way: a shot at the window's centre hits the wooden window bar of
  the shop-front mesh first, as it should.
- **M2, distance fog.** ZoneInfo bDistanceFog / DistanceFogEnd per zone;
  the zone a point is in (BSP); KF's sight checks skip what is beyond the
  fog (RateZombieVolume, PlayerCanSeePoint, KFMonster.Tick's hidden-speed
  check); the fog drawn.
  As built (`zones.rs`): the BSP nodes keep their back / front children;
  `Model::point_zone` walks the tree (UModel::PointRegion as I remember
  it). Zone fog from each ZoneInfo (zone 0 and zones without one: the
  LevelInfo). The player's zone each frame (`PlayerZone`, logged on
  change) sets the camera's DistanceFog (linear, start..end, the zone's
  colour). Sight checks use the player zone's fog end: RateZombieVolume's
  view of the volume, PlayerCanSeePoint, and the zeds' drawn / seen test
  (a zed beyond the fog is neither). KF-WestLondon: 26 zones, the start
  zone fogs -500..4500.
- **M3, lava (LavaVolume, 13 maps).** Pain volumes: DamagePerSec (1-2) of
  Burned while inside. As built (`pain.rs`): every PhysicsVolume with
  bPainCausing and DamagePerSec > 0; a pawn touching (cylinder centre, top
  or bottom inside the brush) takes int(DamagePerSec) on entering and
  everything inside again each second (the volume's timer). Zeds take it
  with no credit; the player as level damage (`LEVEL_DAMAGE`, not reduced
  like own damage). Also ZoneInfo.KillZ (-10000 everywhere): a pawn below
  it dies (FellOutOfWorld). Damage type Burned is taken as plain damage.
- **M4, the player on jump pads, KF-Offices URL teleporters.** As built
  (`walk.rs`): touching a pad sets the player's velocity to its
  JumpVelocity, from the pad's centre (as for zeds). KF-WestLondon
  UTJumppad0 landed the player at (-2036, 314), on its target PathNode111
  (-2027, 306). Teleporters: every Teleporter with a URL in the 35 maps
  has bEnabled false (KF-Offices' six: the mapper put the tag in the URL)
  and nothing triggers them, so in KF they never teleport on touch;
  Teleporters only matter as the trader's boot-out spots (T2). Nothing to
  simulate here.
- **M5, emitters placed in maps, and the sky (you saw both).** Placed
  Emitter actors (fires, smoke; KF-WestLondon 37, KF-Manor 42) were never
  drawn. `emitter::read_emitter_actor` reads an Emitter actor's own
  Emitters sub-objects (else its class's); `particles.rs` loads each as
  "map:<name>" and starts it once at its Location / Rotation (sky layer
  when in the sky zone). The actor's DrawScale is not applied (not known
  whether UE2 scales particles by it). The sky changed colour with the
  view: its dome (KF-WestLondon London_Skybox) was lit by our one sun,
  KF lit it with baked lighting we do not read. Sky-zone surfaces are now
  unlit, and Unreal's own unlit flags are honoured (bUnlit actors,
  PF_Unlit BSP faces). That was only half of it: the dome, cloud
  cylinders and fog ring are see-through layers within 170 units of the
  sky camera, and Bevy sorts see-through things by distance along the
  view direction, so turning the camera swapped which layer was on top.
  Each see-through sky-zone mesh now gets a fixed order from its distance
  to the sky camera (farther drawn first, via depth_bias; log
  `sky_layer_order`). Assumption: KF drew them back to front the same
  way; not checked against the game. Baked lighting itself (StaticMeshInstance vertex
  colours, BSP light maps) is a later rendering task.
- **M6, decals placed in maps (implemented 2026-10-05).** Projector actors: 1176
  KFBloodSplatter (blood) and 322 plain Projector (light patterns, writing,
  leaves) over the 35 maps; KF-WestLondon 68 + 4. All are static
  (bStatic; KFBloodSplatter abandons its projector at start), so each is
  built once at load with the blood decals' projection (`decals.rs`: level
  triangles clipped to the projector's volume, textured by position in it).
  What a Projector sets, and what we do with it:
  - Location, Rotation (X = projection direction, Y = texture U, Z = V),
    ProjTexture (resolved like map materials: shaders, combiners -> their
    texture).
  - Size: the texture's pixel size x DrawScale x DrawScale3D (Y, Z); depth
    MaxTraceDistance (default 1000).
  - FOV (degrees). 0 or 1 (most of them): a box. Larger (light patterns
    20-90): a frustum. **Guess** (the engine's projector code is native and
    not in the SDK): the texture is its normal size at Location and the
    volume widens by FOV/2 each side with distance.
  - FrameBufferBlendingOp: Modulate (default; blood) -> our 2x modulate
    decal material; Add (light patterns) -> additive; AlphaBlend ->
    alpha blend. PB_None (67 KFBloodSplatters on a few maps, on the same
    grey-background opaque textures) -> treated as Modulate. **Guess**:
    drawn opaque they would be grey squares, which no mapper would ship.
  - MaterialBlendingOp Modulate (light patterns: the light is multiplied by
    the surface's own texture): not simulated, the light pattern is added
    as-is (brighter on dark surfaces than KF).
  - bGradient (most light patterns): fades to nothing with depth.
    **Assumed** linear over MaxTraceDistance (KF uses a GRADIENT_Fade
    texture).
  - bProjectBSP / bProjectStaticMesh / bProjectTerrain: which surfaces.
    Our surfaces are collision triangles, so static meshes with simplified
    collision show the decal on their collision shape (may float or clip).
  - bProjectOnBackfaces: not checked (collision winding is unreliable);
    both sides take the decal, edge-on surfaces do not.
  - CullDistance: drawn only within it (Bevy VisibilityRange).
  Log: `map_decals` (built, empty, per blend) and one `map_decal` line
  per projector that lands on nothing or has no texture. Code:
  `decals::project` (shared with the blood decals), `load_map_decals`,
  `spawn_map_decals`; `level::ProjectorInfo`. The projected UVs are
  interpolated straight across each triangle, so a wide frustum hitting
  a surface at a slant is slightly warped (KF divides per pixel).
  Test: KF-WestLondon load log; screenshot of a blood splat; you look at
  KF-Hospital or another light-pattern map.
- Later (needs an event system): plain movers, lifts, scripted triggers.

## Baked lighting (milestone 11, planned 2026-10-05; L1, L2 implemented)

Why: everything is lit by one made-up sun plus ambient light, so maps look
flat and too bright. KF lit its maps offline in the editor and stored the
result in the map file; we read and draw that.

**What a map stores (worked out from the KF-WestLondon data; checked by
`kfpkg lighting <map>`):**

- **Placed meshes.** Each lit StaticMeshActor points to a
  StaticMeshInstance: one colour per mesh vertex (compact count, then B,
  G, R, A). KF-WestLondon 1607 of 1607 and KF-Manor 1793 of 1793 have
  exactly as many colours as the mesh has vertices. Mostly dark (most
  under 32 of 255).
- **BSP (the level's brush geometry).** The rest of the Model after the
  part we read (`bsp.rs`), in order: Bounds (25-byte boxes), LeafHulls
  (ints), Leaves (3 compact + 8 bytes), Lights (compact refs), RootOutside
  and Linked (ints), then:
  - Sections (render batches): vertices of 40 bytes (position, texture
    UV, lightmap UV, normal), an int, material, an int, PolyFlags, and the
    lightmap texture index (-1: none; unlit and sky surfaces).
  - LightMaps (one per lit surface, 1049 on KF-WestLondon): 7 compact
    numbers (sizes and offsets), a world-to-lightmap matrix, 3 vectors, the
    lights with a shadow bit per texel, a compact and an int. Not needed
    to draw: the vertices already carry lightmap UVs.
  - LightMapTextures (12 on KF-WestLondon): the lightmaps they hold, two
    mips of DXT3 (512 x 512 and 256 x 256), format, width, height.
  The walk ends exactly at the Model's last byte. Each BSP node already
  names its section and first vertex there (fields we skipped).
- **Zone ambient**: ZoneInfo AmbientBrightness / Hue / Saturation
  (KF-WestLondon 1-2: almost none).
- **Terrain**: not looked at yet (L3).

**How it is drawn:**

- Placed meshes: the colours as vertex colours on an unlit material:
  texture x colour x K.
- BSP: Bevy's own lightmap support (the `Lightmap` component, UV_1): the
  lit material's diffuse is multiplied by the lightmap; the sun and the
  ambient light are told not to light lightmapped surfaces, so only the
  lightmap counts. Its exposure is set so the result is texture x
  lightmap x K, the same as the meshes.
- K is a single brightness factor. **Not known**: UE2 may double light
  ("overbright", K = 2) or not (K = 1). Start with 2; you compare with
  the real game at the same spot.
- Zeds, weapons and other moving things keep the sun and ambient until
  L4.

**Steps:**

- **L1, BSP lightmaps (implemented).** `bsp::read_lighting`; `kfpkg
  lighting <map>` checks it (all 35 maps read; every polygon's points
  equal its section vertices) and writes the pages to `work/lighting/`.
  Drawn per (material, page) with Bevy's `Lightmap`. Log `bsp_lightmaps`.
  KF-Clandestine, KF-Forgotten and KF-Hell save every page empty (the
  engine rebuilds them at load from the LightMaps entries: lights and
  per-texel shadow bits); there the BSP keeps the old sun for now
  (L1b, later: rebuild them the same way).
- **L2, mesh vertex lighting (implemented).** Each lit actor gets its own
  copy of its mesh parts with the colours (parts remember which mesh
  vertex each of their vertices came from), drawn unlit. Actors without
  colours keep the sun. Log `mesh_lighting` (KF-WestLondon: 1595 baked,
  74 not, 0 mismatched).
- **LV, KF's vision overlay (found 2026-10-05 from your screenshots).**
  The orange look is not lighting: `HUDKillingFloor.DrawModOverlay` draws
  KFX.SepiaShader over the whole screen every frame. That material comes
  down to white (Grain2 x Grain2, x2 / x4, clamped) with OB_Modulate, so
  the screen is multiplied by 2 x tint / 255. The tint is the player
  zone's DistanceFogColor (KFOverlayColor if bNewKFColorCorrection),
  brightened per channel to c + round(c (1 - c/255) - 2), eased toward a
  new zone's colour each tick by round(|diff| x 0.1) + 0.0625, starting
  from black (KF's fade-in). Zones without fog keep the current tint;
  bNoKFColorCorrection zones are skipped; a KFSPLevelInfo with
  bUseVisionOverlay false turns it off. Drawn by a third camera, last,
  with a full-screen quad whose blend is 2 x source x screen (UE2's
  modulate). The HUD text is not tinted. Log `vision_overlay` on each
  target change. Measured on your matching screenshot (ZoneInfo4, x1.16,
  1.04, 0.86): it accounts for most of the colour gap.
- **L3, terrain lighting.** Find where KF keeps it; draw it.
- **L4, moving things.** Zeds and weapons lit by the map's Light actors
  near them plus zone ambient, as UE2 lights actors; the made-up sun goes.
- Not planned: KF's dynamic lights (muzzle flashes lighting walls),
  projected shadows of zeds, coronas.

Test: screenshots at fixed views before and after each step, and you
compare one view with the real game.

## KF's HUD (milestone 12, planned 2026-10-05)

Replace our one-line text HUD with KF's own (HUDKillingFloor), drawn
from the game's textures, fonts and layout. Reference: the two real-game
screenshots in `references/` (1280 x 960, KF-WestLondon spawn, first
countdown), compared at `--window 1280x960`.

**What the data gives.** Every widget's texture, texture rectangle,
TextureScale, PosX / PosY, pivot and colours decode from the class
defaults (`kfpkg defaults KFMod.HUDKillingFloor`), as do the digit sets
(DigitsSmall / DigitsBig: KillingFloorHUD.Generic.HUD, 11 rectangles) and
the font names. KF's own settings (System/defuser.ini) have HudScale 1
and HudCanvasScale 1.

**What is native (not in the scripts), so worked out and checked
against the screenshots:** DrawSpriteWidget and DrawNumericWidget. The
script's own sizing (DrawHudPassA's weight box: texels x TextureScale x
HudCanvasScale x ResScale x HudScale, ResScaleX = width / 640, ResScaleY
= height / 480) and position (PosX x width, PosY x height with
HudCanvasScale 1) are assumed for both. First check: the health box
should be at x 19, 90 x 45 px at 1280 x 960; the screenshot shows about
20 and 88 x 45.

**Drawing.** A 2D layer in screen pixels (Bevy UI image nodes or an
orthographic camera of textured quads, decided in H1): one quad per
widget, texture rectangle as UVs, tint as colour, RenderStyle 5 (alpha)
blending. Over the arrow (T2b-2) and everything else.

Steps:
- **H1, the bottom bar from widgets and digits (done 2026-10-05).** Health, armour, weight
  box and icon (its "1/15" text needs fonts: H2), grenades, the ammo
  boxes by weapon (clips, bullets in clip, the per-weapon icons and
  special cases in DrawHudPassA: LAW, crossbow, M79, Husk Gun, pipe
  bombs, flamethrower, shotguns, ZED gun; the secondary ammo; the
  flashlight box), syringe, welder and medic gun charge, cash (pound
  icon and DigitsBig). KFHUDAlpha 200 on the tints (SetHUDAlpha). Our
  text HUD moves to a small debug line at the top, toggled with F3
  (on until H3 is done, then off by default).
  As built: the sizing rule above matched the screenshot box for box
  (health box x 19-109 vs about 20-108). Images are stretched to their
  box (Bevy keeps proportions by default; that made the weight box,
  128 x 64 drawn 1.5 times wide, too narrow). The flashlight box shows
  for bTorchEnabled weapons with battery 100 and the light off (we have
  no flashlight yet). Not done: the quick-syringe popup, the bile
  colour on the health digits. On wide windows the boxes stretch
  sideways (ResScaleX and ResScaleY differ); not compared with KF.
- **H2, fonts and text (done 2026-10-05).** A reader for UE2 Font objects (ROFontsTwo,
  ROFonts, KFFonts: glyph rectangles on textures; the byte layout is
  worked out with `kfpkg raw` and checked by drawing known strings),
  then the texts: the weight "1/15" (LoadSmallFontStatic(5), scaled
  width / 1024), the weapon name under the cash (DrawWeaponName), and
  "Trader: 36m" under the arrow (DrawTraderDistance, colour 255,50,50).
  As built: the Font layout (`ue_assets::font`, worked out from the
  bytes): empty property list; Characters (compact count; per glyph
  StartU, StartV, USize, VSize int32 and a page byte); Textures (compact
  count of object references); Kerning int32; CharRemap (compact count
  of (character code u16, glyph index u16)); IsRemapped int32. All 120
  Font objects in the install read to their last byte (`kfpkg fonts`).
  Text is laid out in physical pixels (KF's canvas), then divided by the
  window's scale factor. Glyphs advance by USize + Kerning (assumed;
  Kerning is 0 in every HUD font). Compared at 1280 x 960 with the
  screenshot: same font, size and place for all three texts.
- **H3, the top-right circle (done 2026-10-05).** Trader time: Hud_Bio_Clock_Circle with
  the countdown (mm:ss). Waves: Hud_Bio_Circle with the zeds left and
  "Wave 1/4" (DrawKFHUDTextElements). Then our debug line goes off by
  default. As built: CircleSize = Min(128 x SizeX / 1024, 128) at (ClipX -
  CircleSize, 2), fonts LoadFont(2) / (1) / (5) scaled Min(SizeX / 1024,
  1). The zed count stands in for GRI.MaxMonsters as zeds to come plus
  zeds alive (KF updates it at SetupWave and on each kill; spawning keeps
  the sum; in the boss wave KF counts the helpers only after a kill, we
  at once). The boss wave reads "Wave 5/4" on Short, as KF's code gives
  (GRI.WaveNumber is WaveNum, which is FinalWave then). Compared with
  both screenshots: "00:02" and "20 / Wave 1/4" match.
- **H4, messages (done 2026-10-05, without the end-of-game text).**
  HudBase's LocalMessages (8, a message of the same class replaces the
  old one: bIsUnique; faded by the time left: bFadeMessage) and
  DisplayLocalMessages / DrawMessage, with HUDKillingFloor.LayoutMessage
  (WaitingMessage switches 1-3 and 5 use the WaitingFont, KFBase02DS36
  above 1024 wide, DS24 at or below; the rest GetFontSizeIndex) and
  WaitingMessage.RenderComplexMessage (scale ClipX / 1024, lines split at
  '|', each centred). Classes: WaitingMessage (1 next wave inbound at 4-1
  s left, 3 final wave inbound before the boss, 2 wave completed after
  waves 1-3, 4 welded shut on USE, 6 the door hint on touching a door
  trigger whose message has "USE"), KFMainMessages (0 booted from the
  shop, 3 press E to trade), KFCriticalEventPlus (a door trigger's other
  messages). '%Use%' is shown as E. Our HudNote is gone. Not done: the
  end-of-game text (DrawEndGameHUD), zed time (5), weapon pickup
  messages (KFMainMessages 1, 2, 4), the announcer voice
  (WarningMessage).
- Not planned: hints, chat, the voice meter, other players' names and
  bars, perk icons and stars (no perks yet), the weapon-select bar.

Logs: `hud_loaded` (widgets, textures found or missing, fonts), and on
F3 or a test action `hud_dump` (each drawn widget's pixel box) so the
layout can be checked against the screenshots by numbers.

## Zed time (milestone 13, implemented 2026-10-05)

KF's slow motion (KFGameType.DramaticEvent, Tick, Killed, DoBossDeath;
KFPlayerController.ClientEnterZedTime / ClientExitZedTime). One step,
`zed_time.rs`:

- **Speed.** GameInfo.SetGameSpeed(T): Level.TimeDilation = 1.1 x T.
  Zed time is SetGameSpeed(ZedTimeSlomoScale 0.2): everything runs at 0.2
  of normal. Ours: Bevy's virtual clock at relative speed 0.2 (every
  gameplay system, physics and ragdolls included, uses it; mouse look
  reads raw motion and stays full speed, as in KF; the frame limiter,
  frame stats, recording and screenshots use the real clock).
- **Length.** Tick: CurrentZEDTimeDuration -= DeltaTime x 1.1 /
  TimeDilation, i.e. 1.1 x real seconds: ZEDTimeDuration 3 lasts 2.73
  real seconds. When under 16.6% is left: ClientExitZedTime (once) and
  the speed eases back: SetGameSpeed(Lerp(left / (0.498), 1.0, 0.2)),
  from 0.2 to 1. At 0: normal speed, extensions used reset.
- **DramaticEvent(chance, duration).** Not within 10 game seconds of the
  last event unless the chance is 1; chance x 4 if the last event was
  over 60 s ago, x 2 if over 30 s; FRand() <= chance starts it (duration
  3 unless given), LastZedTimeEvent = now (game time: LastZedTimeEvent
  starts at 0, so none in the first 10 s of a game; copied).
- **Who calls it.** KFGameType.Killed, for a zed the player killed and
  more than 0.1 s after the last event: 0.05 if within 150 units (3 m,
  VSizeSquared < 22500) of the player, else 0.025 (perk extensions:
  no perks yet). KFMonster.TakeDamage: a headshot that kills, 0.03 (after
  Killed's roll, so usually blocked by its 10 s rule if that one
  fired). Explosions (Nade, LAWProj, M79, pipe bomb, ... HurtRadius):
  4+ zeds killed 0.05, 2+ 0.03. ZombieBoss radial attack hitting
  someone: 0.3; not done: he only does it with 3 or more players around
  him (NumPlayersSurrounding >= 3), never solo. DoBossDeath: forced, 6 s
  (ZEDTimeDuration x 2). Not done: the Husk Gun's, flare revolver's,
  ZED MKII's and Husk fireball's own multi-kill rolls (their projectiles
  in our code do not count kills yet).
- **Player side.** ClientEnterZedTime: "ZED TIME ACTIVATED!"
  (WaitingMessage 5) the first time ever (bHadZED, saved in the
  player's ini; ours: once per run), and the Zedtime_Enter / _Exit
  sounds (we have no sound yet: not done). No screen effect in the
  scripts.
- Logs `zed_time` (start: reason, chance, roll, duration; speed-up;
  end), `dramatic_event` (refused: cooldown or roll). Test action
  `zed_time` and the debug key F2 (DramaticEvent(1.0)). The rolls use a fixed seed like the
  rest of the project (each run repeats exactly).

**Open question (not part of this step): the engine's normal speed.**
UE2 runs the whole game at TimeDilation 1.1 (LevelInfo default 1.1,
SetGameSpeed(1) gives 1.1): animations, movement, fire rates and
game-time timers run 10% faster than the wall clock. We run at 1.0.
Zed time copies the ratios, so it is unaffected; matching KF's 1.1
everywhere is a separate decision.

## Sound and music (milestone 10, planned 2026-10-05)

**What the files hold (checked 2026-10-05).** A `Sound` object in a `.uax`
package is a short header followed by a complete `.wav` file: an empty
property list, FileType (a name, "WAV"), Likelihood (float, 1.0), then a
lazy array (an int32 file offset, a compact byte count) holding the
`RIFF/WAVE` bytes. A `SoundGroup` (KF's "pick one at random" list) is an
empty property list, then a compact count and object references, e.g.
`KF_9MMSnd.9mm_Fire` = Fire1, Fire2, Fire3 (Fire4 exists but is not in
the group). A raw byte scan of all 156 packages found 6286 wav files,
all uncompressed PCM (format tag 1): 6151 mono, 135 stereo; 6122 16-bit,
164 8-bit; rates 44100 (2612), 32000 (2276), 22050 (1282), 8000, 11025,
48000, 96000, 16000. No Ogg inside packages. Music is 75 plain `.ogg`
files in `Music/`.

**How KF plays them (Engine/Actor.uc).** `PlaySound(Sound, Slot,
Volume, bNoOverride, Radius, Pitch, Attenuate)`. Defaults when omitted:
Volume = TransientSoundVolume (Actor default 0.3), Radius =
TransientSoundRadius (300), Pitch 1.0. Slots (SLOT_None, Misc, Pain,
Interact, Ambient, Talk, Interface): a new sound in an actor's slot
stops the one already playing there, unless bNoOverride is set and the
old one is still going (SLOT_None never overrides). Looping sounds:
`AmbientSound` on any actor, with SoundVolume (0-255, default 128),
SoundRadius (default 64) and SoundPitch (64 = normal). The mixing
itself (distance falloff, panning) is native code, so it is not in the
scripts. `System/KillingFloor.ini [ALAudio.ALAudioSubsystem]` gives the
settings: Channels=32 (voices at once), SoundVolume=0.3,
AmbientVolume=0.5, MusicVolume=0.1, Rolloff=0.5, DopplerFactor=1.0,
Use3DSound=False (so plain stereo panning, no HRTF).

**Music (KFMod/KFGameType.uc).** The map's `KFMusicTrigger`
(MapSongHandler) names a calm Song and a CombatSong, optionally per wave
(WaveBasedSongs), with FadeInTime / FadeOutTime. Calm plays during trader
time, combat during waves. The Patriarch fight switches to BossBattleSong
(ClientSetMusic, MTRAN_FastFade).

**Mixer choice.** Bevy's built-in audio can play a file and do simple
left/right panning. It cannot do KF's rules: a range past which a sound is
silent, slots that cut off the previous sound, 32 voices with the
quietest dropped first, and pitch following zed time. So we write a small
mixer of our own (`src/audio.rs`). It feeds `rodio`, the library Bevy's
audio already uses, so nothing new is downloaded. The mixer adds the
voices together sample by sample. Each frame the game updates every
voice's volume, left/right balance and pitch from the listener's position.
If the machine has no sound device, the game logs it and runs silent.

**Distance falloff (a guess, to be checked by ear).** Since S4a: OpenAL's
"inverse distance clamped" model, the sound's Radius as the reference
distance and the ini's Rolloff 0.5: full volume inside the radius, then
radius / (radius + 0.5 x (distance - radius)); no cut-off, but voices too
quiet to hear (final gain under 0.002) are not started. S2 to S3 used a
linear fade to silence at the radius; KF's small zed radii (footsteps
100) showed that cannot be right. Zed time: the voices' pitch is multiplied by the game speed (assumed
from how KF sounds in zed time; to be checked).

**Steps** (each one logged as `sound_play` / `sound_stop` / `music` lines,
with a test you can run):

- **S1. Read sounds** (done 2026-10-05). `ue-assets/src/sound.rs`: Sound
  and SoundGroup, the wav header (rate, channels, bits), samples and `smpl`
  loop points. `kfpkg sounds` reads every sound in the install: 6687
  sounds (6660 in `.uax`, 15 in maps, 12 in `.usx`), 1630 groups with
  7926 members, 0 failures, 149 with loop points, about 4.2 hours in all.
  Likelihood (a member's weight in its group's random pick) is 1.0 for
  all but 3 sounds. No audio output yet.
- **S2. The mixer** (done 2026-10-05). `src/audio.rs`. Systems write a
  `PlaySound` message (KF's PlaySound arguments, its defaults filled in);
  `play_sounds` finds the sound (cached, a weighted random pick for
  groups), applies the slot rule, skips sounds out of range, and enforces
  32 voices (drops the quietest if quieter than the new one: a guess).
  `update_voices` sets each voice's left/right volume from the player's
  view each frame and its pitch x the game speed. The audio thread mixes
  in blocks of 256 frames with linear resampling and volume ramps (no
  clicks), then clamps. Master SoundVolume 0.3 from the ini. Bevy's own
  audio plugin is switched off. Emitters: an entity (followed; owns
  slots), a fixed point, or the listener (no falloff or panning). Test
  actions `sound:NAME` and `sound_at:NAME@DIST` (volume 1.8, KF's gun
  volume); `--mute`.
- **S3. Weapons**, in three parts:
  - **S3a. Shots, dry fire, select, the full-auto loop** (done
    2026-10-05). PlayFiring's sound (WeaponFire, KFFire, KFShotgunFire,
    BoomStickFire, WeldFire, SyringeAltFire, FragFire all do the same):
    KFFire family in first person: StereoFireSound (falls back to
    FireSound) at TransientSoundVolume x 0.85, pitch 1 +-
    RandomPitchAdjustAmt if bRandomPitchFireSound; other classes:
    FireSound at TransientSoundVolume (WeaponFire default 0.5); always
    SLOT_Interact, so a new shot cuts the last one's tail (UE2's slot
    rule). Dry fire (KFWeapon.Fire, bModeZeroCanDryFire): NoAmmoSound at
    2.0. Select (KFWeapon.BringUp): SelectSound, SLOT_Interact, the
    weapon's TransientSoundVolume (KFWeapon: 100). KFHighROFFire in full
    auto (state FireLoop): AmbientFireSound loops on the weapon
    (AmbientFireVolume 255, radius 500), then FireEndStereoSound at
    AmbientFireVolume / 127 when it stops. Each weapon has its own slots
    (in KF each is its own actor). Each weapon's sounds are preloaded
    when it is first carried (`sound_preload`; all 44: 0 missing).
  - **S3b. Animation sounds** (done 2026-10-05). Reloads, the pump
    shotgun's rack (two notifies in its Fire animation, at 0.35 and
    0.54), melee swing whooshes and other timed sounds are
    `KFWeaponSoundNotify` objects in the weapon animations (Sound,
    Volume, Radius, bAttenuate; class defaults Volume 1, Radius 0,
    from Engine.CustomSoundNotify). KFWeaponSoundNotify.Notify plays
    them on the player (Instigator.PlaySound, SLOT_None). Ours: each
    frame, the notifies between the last frame and this one (also
    across a loop's wrap; a new animation starts at -1 so time-0
    notifies play). 363 weapon sounds in all are preloaded, 0 missing.
  - **S3c. Melee hits, chainsaw, flamethrower, Husk Gun** (done
    2026-10-05). Melee: one of MeleeHitSounds (or MeleeHitSoundRefs) at
    random per zed hit at MeleeHitVolume (1): the traced zed's on the
    weapon (at the player), wide hits' on each zed (KFMeleeFire.Timer);
    the chainsaw's held fire the same. The weapon attachment's
    AmbientSound (`weapon_loop_sound`): fire loops for KFHighROFFire,
    FlameBurstFire and ChainsawFire; the chainsaw plays FireStartSound
    and starts its loop after GetSoundDuration, and after FireEndSound
    is silent for its length, then idles (ChainsawAttachment's
    AmbientSound Chainsaw_Idle1, SoundVolume 230, radius 300).
    HuskGunFire: AmbientChargeUpSound while charging, AmbientFireSound
    (ChargedLoop) at full charge (MaxChargeTime 3). 377 weapon sounds
    preloaded, 0 missing.
  - **S3d. The rest** (done 2026-10-05). Frag (KFPawn.ThrowGrenade):
    Frag.StartThrow plays FireMode[0].FireSound (Axe_Fire) and
    ServerThrow, TossSpawnTime later, ThrowSound (Nade_Throw), both
    SLOT_Interact at 2.0, so the second cuts the first. Pipe bomb:
    PipeBombFire.Timer plays Sound'KF_AxeSnd.Axe_Fire' (written in the
    script) when the bomb is placed. ZED Gun alt fire: AmbientChargeUp
    then the charge loop after MaxChargeTime (1 s); at the end
    PlayFireEnd plays StereoFireSound (the spin-down). Flamethrower
    (FlameBurstFire.AllowFire): NoAmmoSound every FireRate while held
    empty. Not done: the ZED Gun's AlarmSound (its motion detector is
    not built); FragFire's ReloadSound in state LoadNext (only used when
    the frag is fired as a weapon, which ours never is). The welder and
    syringe have no fire sounds of their own (their animations'
    notifies play).

  Two volume rules found in S3a (both **guesses**, labelled in
  `audio.rs`): a voice's final volume is volume x fade x master, capped
  at 1 (OpenAL's AL_MAX_GAIN; KF passes 1.8 for guns and 100 for the
  quietly recorded select sounds, peak 0.18 against 0.99). An
  AmbientSound's SoundVolume counts 128 as 1.0 (KF ends a 255 loop with
  a tail at 255/127). Not used yet: the ini's AmbientVolume 0.5.
- **S4. Zeds and the player** (researched 2026-10-05; the full notes with
  script quotes are in `work/s4-research.md`, untracked), in three parts:
  - **S4a. Zed animation sounds, and the distance fade revisited** (done
    2026-10-05; all 10 zed classes' notify sounds preload, 0 missing). Zed
    footsteps, claw swishes, attack grunts, the Siren's scream, the
    Husk's fireball, the Bloat's vomit, the burning walks: all plain
    `AnimNotify_Sound` notifies in the zed animations (e.g. ClotWalk:
    21 x Clot_StepDefault, volume 0.25, radius 100). Played on the zed,
    SLOT_None (the native code's slot is not in the scripts: a guess).
    KF's ranges are small (footsteps 100, moans 250, swishes 100), so the
    S2 fade (silent at the radius) would make zeds inaudible past a few
    metres, unlike the real game. New guess: OpenAL's inverse distance
    clamped model with the ini's Rolloff 0.5 and the radius as the
    reference distance: full volume inside the radius, then
    radius / (radius + 0.5 x (distance - radius)). Sounds too quiet to
    matter are not started.
  - **S4b. Zed voices and loops.** KFMonster: moans (MoanVoice, SLOT_Misc,
    MoanVolume 1.5, radius 250; first after 2 + 36 x rand s, then every
    12 + 8 x rand s, whole seconds; never headless), pain (HitSound[0],
    SLOT_Pain, 1.25, radius 400, at most every 0.35 s, not for fire
    damage), death 0.2 s after dying (DeathSound / headless / gibbed),
    decapitation, Impact_Skull on headshots, the melee hit on the player,
    challenge sounds, AmbientSound loops (Scrake's chainsaw, Bloat,
    Husk, Patriarch), and the Patriarch's own sounds.
  - **S4c. The player.** Pain (Inf_Player Wounding, SLOT_Pain, 0.6,
    radius 200, every 0.35 s at most), death, footsteps (CheckBob, about
    every 0.375 s at full speed, per surface, 0.45, x 0.4 crouched or
    walking), jump and landing, low-health breathing (under 25 %).
- **S5. The world.** The map's AmbientSound actors, doors (open, close,
  weld), explosions, bullet impacts (bullet_fx's ImpactSound), pickups,
  trader and zed-time sounds.
- **S6. Music.** KFMusicTrigger's calm and combat songs per wave, the
  fades and the boss song.

Not planned: Doppler (DopplerFactor 1.0; small effect, later if missed),
EAX reverb (off in KF's ini), voice chat.

## Later milestones (rough order, to be planned in detail when reached)

2. **Walk around:** collision with BSP and static meshes, plus Unreal-style
   walking, jumping and gravity, with values taken from the scripts.
3. **Characters:** skeletal meshes and animations from `.ukx`, showing zeds
   standing in the map.
4. **Weapons:** first-person weapon meshes, firing, hit detection, damage.
5. **Zeds:** AI and pathfinding over the map's navigation points.
6. **Game loop (solo):** waves, trader, dosh, perks.
7. **Sound, music, HUD, menus.**

## Open questions

- Exact Unreal-unit-to-metre scale (step 0 picks a value; milestone 2 confirms
  it against player height and movement speed).
- ~~Whether every `.u` class includes its source text~~: yes, all 3757 do.
