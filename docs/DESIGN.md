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

## Weapons (milestone 7, W1 implemented 2026-10-04)

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
  medic dart alt fires. The Welder loads and animates, but doors are not
  simulated, so it has nothing to weld yet.
- **W9, ZED guns.**

**W1 findings (done).**
- 47 of 48 load (`--give all`, 6.3 s). The ZED Gun MKII fails: its class
  defaults only partly parse and no mesh package for it is in the install.
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

**Not covered.** Sound (the project has no audio yet), perks (KF with no
perk chosen uses the plain values; perk bonuses come with the game loop),
the trader and buying (W1's `--give` stands in), third-person weapon
models, the flashlight, zed time.

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
