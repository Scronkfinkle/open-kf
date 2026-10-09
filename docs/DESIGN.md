# Open KF design

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
open-kf/                   Cargo workspace root (the repository)
  src/                     the game binary (Bevy app), one folder per area:
    main.rs                command line, plugins
    engine/                coordinates, run log, cameras, screenshots, video recording
    world/                 map loading, collision, zones, navigation, spawn volumes,
                           doors, breakable glass
    render/                skinned meshes, particles, decals, vision overlay, lighting
    player/                walking, pawn collision, pain flash, armour, character,
                           body/ (the third-person body: load, animate)
    weapons/               first-person weapons, firing, projectiles, bullet effects
      weapon/              the weapon in hand (mod.rs: types), load, input (firing,
                           reloading, switching), animate, inventory (and shop), sounds
    zeds/                  the specimens, the Patriarch, gore, ragdolls, vomit, fireballs
      zed/                 the Zed component (mod.rs: types), load, spawn, think (AI),
                           boss_ai, attacks, animate, effects, sounds, methods
    game/                  waves, damage and health, dosh, trader and shop, HUD, zed time
    audio/                 the mixer, music, map / player / trader sounds
    launcher/              the launcher window (no options given): choices, PLAY
                           starts the game again with them (see "The launcher")
    net/                   experimental multiplayer (branch multiplayer-lightyear):
                           --host / --join, the lobby over the network (lightyear),
                           other players' pawns and bodies (pawns.rs), the host's
                           zeds and waves shared with clients (zeds.rs), start
                           spots and wave-end respawns (starts.rs), the scoreboard
                           (scoreboard.rs), host-owned doors (doors.rs),
                           zed time decided by the host (zedtime.rs),
                           the host-info query on port + 1 (query.rs);
                           see docs/multiplayer-prototype.md
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
- **Which objects are actors (2026-10-08).** A map package keeps every
  object something still references, including actors deleted in the editor
  (`bDeleteMe` set). KF does not play those: it builds the level from the
  `Level` object's actor list (after its empty property list: count,
  capacity, one object reference per slot; then the level URL; then the BSP
  `Model` reference). `actor_list.rs` reads that list once per package and
  `Package::level_actor_exports()` hands out the listed exports, in list
  order (KF's own iteration order). Every map reader uses it: `level.rs`
  (meshes, brushes, lights, path nodes, pickups, projectors, player starts,
  use triggers), terrain, shops and teleporters, shopkeepers, zombie and pain
  volumes, map sounds, music trigger, level rules, emitters, jump pads,
  LevelInfo lookups. ReachSpecs whose start or end is not listed are left out
  of the nav graph (no live node's path list holds them). Objects that are
  not actors (Models, Polys, ReachSpecs, materials) are found by reference
  as before. Checked on all 40 maps with `kfpkg actors all`: the objects left
  out are exactly the `bDeleteMe` ones, no BSP zone uses a left-out ZoneInfo,
  and the Level's model equals the old "largest unowned Model" guess.
  Packages without a readable list fall back to every export
  (`level_actors_error` in the log).
- `level.rs` takes the BSP model from the level (fallback: the largest
  `Model` not referenced by an actor's `Brush` property). An actor is drawn as a static
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

- `src/engine/coords.rs` is the only place where Unreal coordinates become Bevy ones
  (`bevy = (ue.y, ue.z, -ue.x) / 50`). Rotations convert as `C·R·Cᵀ`.
- `src/world/map.rs` builds one Bevy mesh per BSP material, and one per static-mesh
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
- `src/engine/camera.rs`: fly camera starting at the first PlayerStart. Field of
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

### Sky fog, sky rotation, clear colour, fog blending (fix sky-fog, 2026-10-08)

Rules read from KF's native renderer (details in the local RE.md), done one
step at a time:

1. **Sky fog.** The sky view uses the fog of **its own zone** (the
   SkyZoneInfo's bDistanceFog, DistanceFogStart / End / Color), measured
   from the sky camera, never the player's zone fog and never blended. 24 of
   34 maps' sky zones have fog (KF-WestLondon: -700..2500, colour
   (86, 74, 54)). Plan: give the sky camera (and the scope's sky camera) a
   `DistanceFog` from the sky zone at spawn. Log `sky_fog`.
2. **Sky rotation.** The sky view is turned by the SkyZoneInfo's Rotation: a
   sky point in direction d from the SkyZoneInfo is seen in direction
   Rotation^-1 d. So the sky camera's rotation = Rotation x the main
   camera's rotation. KF-WestLondon and KF-Waterworks have Yaw about
   -180 deg, KF-Manor / Icebreaker / Biohazard Yaw 65 deg and Pitch -8 deg.
   The sign is checked against your real KF-WestLondon screenshot. Log
   `sky_rotation`.
3. **Clear colour.** KF clears the screen to black for a view in zone 0, and
   to the zone's fog colour when the camera's zone has bClearToFogColor and
   fog; otherwise it does not clear (we use black). Plan: the clear colour
   (`ClearColor`) follows the camera zone; the old grey-blue goes. Log
   `clear_colour` on change.
4. **Fog blending and volume fog.** The main view's fog fades over the new
   zone's DistanceFogBlendTime (ZoneInfo default 1 s) when the camera
   changes zone, following KF's per-player state (last fog start / end /
   colour, a timer in game time). Entering zone 0, a first zone change from
   zone 0, or a finished fade snaps. A zone without fog fades to start = end
   = the far plane (65536) with the last colour, and fog goes off when the
   fade ends. A PhysicsVolume with bDistanceFog that holds the camera
   overrides all of this with its own fog, without fading. Log `fog_blend`
   while fading.

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

Not covered: decoration layers (grass meshes); terrain lighting came later
(L3, below).

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

### Terrain lighting (L3, planned 2026-10-08)

**What KF does (from its native code; details in the local RE.md).** The
map stores a light colour for every heightmap vertex at the very end of each
TerrainInfo: a count, then R, G, B, A per vertex (row by row, y x width +
x; A always 255). The editor's lighting build computes them from the zone
ambient and the map's lights; the game never recomputes them, it only draws
them. Each terrain pixel is texture x vertex colour x 2, plus dynamic lights
(the flashlight). There is no sun on terrain. The game reads the array with
the current heightmap width: if fewer colours are stored than the heightmap
has vertices it uses white; if more (KF-Hell's TerrainInfo6: 65536 stored
for a 64 x 64 heightmap) the first ones.

**Plan.**
1. `ue-assets/terrain.rs` reads the array (checks the count in front of
   it), `kfpkg terrain MAP` prints stored count, vertex count, mean colour.
2. The terrain mesh's vertex colour becomes stored colour x K (K = 2, the
   same linear-light conversion as baked placed meshes) times the layer
   weight; the layer materials become unlit, with the baked-mesh swap
   (render/baked.rs) so the flashlight still lights terrain. If the colours
   cannot be read, the terrain keeps the old sun lighting and the log says
   `terrain_light ... missing`.
3. Log `terrain_light terrain=N stored= vertices= drawn_vertices= nonblack=
   mean_rgb=` per terrain.

**Fog on terrain layers (planned and built 2026-10-08).** KF draws layer 0
opaque and every further layer alpha-blended over it, each fogged, so the
fog colour counts once whatever is painted. Ours adds the upper layers
(order-independent), and before this fix each added layer brought its own
share of fog colour: up to double fog where an upper layer is painted.
Now layer 0 is fogged normally and the added layers fade toward black with
distance instead; the sum equals KF's blend exactly (unit test
`terrain_layer_weights_match_blending_and_sum_to_one`). To do that the
terrain always uses the baked-mesh material (render/baked.rs) with a flag
per layer (layer 0: scaled by its weight carried in the vertex alpha;
upper layers: fog to black). Being always lit costs about 1 ms a frame on
KF-Farm (headless: 37.5 -> 38.8 ms); the flashlight on terrain is now
scaled by the layer weights too. Terrain in the sky zone (no fog there)
and terrain without stored colours keep the plain material.

Effect on zeds and players standing on terrain: none. Actor lighting
already uses the map's own lights and zone ambient, not the terrain's
colours, and the made-up sun is not an actor light source.

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
4. Automated test: walk mode is the start mode (since 2026-10-06; `--fly` starts flying), and `--autowalk SECONDS`
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

### Player movement details from KF (2026-10-08, branch fix/player-movement)

Seven fixes, one commit each, in this order. All values come from KF's
scripts, class defaults and engine behaviour (details in the local RE.md).

- **PM1 Walking (Ctrl) and aiming slow the player to 40 %.** KF's "Walking"
  key (Ctrl in the default key list) and iron sights put the pawn in its
  walking state. Walking caps the acceleration at AccelRate x WalkingPct
  (1000 x 0.4) and the speed at GroundSpeed x 0.4 (80 units/s plus the
  other modifiers). A walking player does not walk off a ledge: when the
  floor below disappears, the move is undone and the speed set to 0
  (KF players cannot override this; only jumping leaves the ledge).
- **PM2 Eye height smoothing.** The view keeps its world height when the
  body steps up or down and catches up with the normal eye height (44
  above the centre) at `min(0.9, 10 x dt)` per frame. Eye height stays
  between -25 (half the collision height down) and the ceiling limit: 85
  above the centre, or 14 below a ceiling found by a line check up to 99
  above the centre. While falling the eye eases back to 44 the same way.
- **PM3 Landing dip.** Landing faster than 200 units/s downward dips the
  view: the eye height drops by `1.5 x min(0.65, 10 dt)` of itself per
  frame (each frame's drop x 0.03 adds to LandBob) until it is under 12 or
  LandBob is over 3, then recovers at `0.6 x min(0.9, 10 dt)` per frame to
  44. A step of more than 15 units in one frame cancels the dip. LandBob
  feeds the walking bob's vertical part (AppliedBob) and moves the weapon
  up by LandBob.
- **PM4 Camera bob twice.** KF's first-person camera adds the walking bob
  twice (once in the eye position and once more on top); the weapon gets
  it once (times its BobDamping).
- **PM5 Low health slows.** GroundSpeed = 200 x (health/100 x 0.3 + 0.7)
  before the weight factor, the melee bonus and the perk.
- **PM6 Falling damage.** On landing (vertical speed Vz < 0, Unreal units/s,
  limit 600, x2 in a volume with weaker gravity than -950): Vz below -600
  takes `100 x (-Vz - 600) / 600` damage of type Fell (armour does not stop
  it); KF also makes a noise for the zeds' hearing and a tiny view shake
  (not done: no hearing; the shake is covered by the hit's own shake).
  Players only (zeds never take falling damage in KF).
- **PM7 Gravity and zone velocity per physics volume.** Each map has a
  default physics volume and may place more; the pawn uses the
  highest-priority volume containing its centre (a later volume wins only
  with a strictly higher priority; the default volume is the start). While
  falling, its gravity replaces -950 and its ZoneVelocity is added to the
  move (not kept in the velocity). Only KF-MoonBase changes these: default
  gravity -320 with ZoneVelocity Z +15, and its low-gravity volumes -150
  with ZoneVelocity Z 30 to 55. The volume lookup is a shared helper so the
  zeds can use it later (their movement is unchanged for now).

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

**Steps 3-6 (done, `src/weapons/weapon.rs`).**
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

**In a network game (branch multiplayer-lightyear, step 3).** Only the
host runs zeds. Each `Zed` has a `net` part (`zeds/zed/net.rs`). On a
client every zed is a *puppet*: made by the normal spawn code from the
host's snapshot, it does not think or move by itself (`think.rs` skips
it); `Zed::apply_net` copies the host's position, yaw, state,
animation, health and flags into it, and its death, ragdoll, gore and
sounds run locally. On the host the AI hunts a list of players
(`Prey`: this game's own plus `combat::RemotePlayers`, the other
players' pawns) and picks one per zed with KF's FindNewEnemy /
SetEnemy rules (`choose_enemy`); hits on another player's pawn carry
`to_peer` (`PlayerDamaged`, `PlayerPush`) or are a `RemoteGrab`, and
net/zeds.rs sends them to that player's game. In single player the list
has one player and nothing changes. Details and limits:
docs/multiplayer-prototype.md, step 3.

**Step 4 additions (same branch).** The Bloat's globs and the Husk's and
Patriarch's projectiles test every player on the host
(`RemotePlayers::targets`) and send hits like melee; clients get
harmless copies (`ProjectileFx`) to see and hear. Other players'
weapons flash and sound on every game (`player/body/fire_fx.rs`, KF's
third-person rules: FireSound at the pawn, the attachment's
mMuzFlashClass on its `tip` bone, the full-auto AmbientFireSound loop).
The host picks start spots (`net/starts.rs`, KF's RatePlayerStart) and
brings dead players back at a wave end (a `PlayerStartMsg` with
`respawn`; the player's own game resets health, armour, inventory and
dosh to KF's new-pawn values). Doors belong to the host: `door.rs`
`DoorNet` takes clients' USE and welder requests and gives clients the
host's door state (`net/doors.rs` carries them). Each pawn update also
carries kills, dosh, deaths and health for the Tab scoreboard
(`net/scoreboard.rs`).

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

**Melee hit timing (all zeds, 2026-10-06).** KF has no fixed "hit at half
the swing": each attack animation carries AnimNotify_Script markers named
ClawDamageTarget (a "notify" is a timed event stored in the animation
file). Each marker is one damage check (KFMonster.ClawDamageTarget ->
MeleeDamageTarget: MeleeDamage -5% .. +5%, reach MeleeRange x 1.4 + both
radii). We used to do one check at 50% of every attack; now each marker
fires once when the animation passes it (`attacks::claw_times`,
`due_notifies`; log `zed_melee_hit` / `zed_attack_missed` with
`hit=N/M notify_at=..`). Times read with `kfpkg notifies
Animations/KF_Freaks_Trip.ukx <Anim>` (0..1 of the animation):

| Zed | Attack: notify times |
| --- | --- |
| Gorefast | GoreAttack1 0.256, 0.445; GoreAttack2 0.268, 0.466 |
| Clot | Claw 0.354, 0.650; Claw2 0.309, 0.566; ClotGrapple 0.375, 0.691; ClotGrappleTwo 0.271, 0.656; ClotGrappleThree 0.676 |
| Stalker | StalkerAttack1 0.240, 0.614; StalkerSpinAttack 0.464; JumpAttack 0.530 |
| Scrake | SawZombieAttack1 0.429, 0.467; SawZombieAttack2 0.353, 0.578; SawImpaleLoop 0.133 |
| Fleshpound | PoundAttack1 0.284, 0.592; PoundAttack2 0.407, 0.465, 0.539, 0.604; PoundAttack3 0.510; FPRageAttack 0.511, 0.515 |
| Crawler | ZombieLeapAttack 0.637; ZombieLeapAttack2 0.462; ZombieSpring 0.487 |
| Bloat | BloatChop2 0.394 |
| Siren | Siren_Bite / Siren_Bite2 0.484 |
| Husk (KF_Freaks2_Trip.ukx, Burns_anim) | Strike 0.452 |

So most attacks now hit twice (the Fleshpound's x 0.5 / x 0.25 per-hit
cut in ZombieFleshPound.ClawDamageTarget exists because of this). An
attack animation without a ClawDamageTarget notify does no damage (logged
`zed_melee_no_notify`). The Patriarch already worked this way.

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

**Particle rules read from KF's engine (built 2026-10-08, branch
fix/particles).** An audit compared our particle code with KF's compiled
particle code (details in the local RE.md). One change per step, in this
order, each its own commit:
- P1. Sprite size: KF puts a sprite's corners at Size from the centre, so a
  sprite is 2 x Size across (we drew 1 x Size). Mesh particles unchanged.
- P2. Turning with the effect: the start position and start velocity are
  turned only by UseRotationFrom (None: world axes; Actor: the effect's
  rotation; Offset: RotationOffset; Normal: the rotation of RotationNormal).
  Then Independent emitters add the effect's location; Relative ones keep
  the position local and are drawn with the effect's location and rotation
  (so a Relative + Actor emitter is turned twice, as in KF).
- P3. Spawn rates: while fewer slots than MaxParticles have ever been
  used, the rate is MaxParticles / average lifetime with
  AutomaticInitialSpawning, else InitialParticlesPerSecond; after that,
  ParticlesPerSecond. Particles go into a ring of MaxParticles slots (a
  new one replaces the oldest slot); a dead particle is respawned in its
  slot only with RespawnDeadParticles. An emitter is finished only when it
  has no spawn rate, does not respawn, and every slot is dead.
- P4. GetVelocityDirectionFrom: after turning, the start velocity is
  multiplied axis by axis with the direction from the particle to the
  effect (StartPositionAndOwner, negated) or the other way round
  (OwnerAndStartPosition), or gets StartVelocityRadialRange along it
  (AddRadial).
- P5. InitialDelayRange: an emitter waits a random time in this range
  before its first update; the effect is not finished while it waits.
- P6. StartLocationOffset is added to the start position before the
  shapes, and turned with it.
- P7. Fading per draw style: the fade fraction is (time - FadeOutStartTime)
  / (life - FadeOutStartTime) when fading out (this wins), else (FadeInEndTime
  - time) / FadeInEndTime when fading in; AlphaBlend subtracts FadeOutFactor.W
  (FadeInFactor.W) times it from the alpha, Modulated uses alpha 1 - W x it,
  the other styles subtract the factor's X/Y/Z times it from the colour.
  Opacity then scales the alpha (AlphaBlend, Modulated, AlphaModulate) or
  the colour (Translucent, Darken, Brighten).
- P8. Blends: Translucent adds, Brighten is a screen (texture + scene x
  (1 - texture)), Darken is scene x (1 - texture), AlphaModulate is
  premultiplied alpha; all four fogged toward black (no change), as KF
  does. Drawn with their own material (`BlendMaterial`).
- P9. Draw order: an effect's sub-emitters are drawn in list order (each
  mesh's bounds centred on the effect, nudged toward the camera by its
  list position, since Bevy sorts see-through meshes by that centre).
- P10. The particles spawned in one update each get their own birth time
  within it (and are aged by it), and for a moving Independent effect are
  spread along the path it moved, so trails do not clump per frame.
- Not done (lower priority in the audit): ColorScale / SizeScale curve
  edges, VelocityScale, collision details, ColorMultiplierRange, warmup,
  subdivision details, ZTest, round-robin AddLocationFromOtherEmitter,
  polar start shape; Modulated particles are not fogged.

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
0-6 degrees; since 2026-10-08 the FOV is used as for map decals, M6), the size from texture size x |DrawScale| (a negative scale
read as mirrored), only collision geometry (non-blocking decorative meshes
get no decals), the end of life is a 1 s fade (assumed). Since
2026-10-08 the strength also falls with the surface angle and back faces
get nothing, as for map decals (M6, "Surface angle").
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
fires three globs (`src/zeds/vomit.rs`): speed 400 with gravity, from 30 ahead
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
(Corrected 2026-10-08, P2: only emitters with UseRotationFrom Actor turn
with the effect, as in KF; the vomit spray is one of them, the head jet's
chunks (None) fly up world Z again.)
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
0.475, HuskMuzzle at 0.49. The fireball (`src/zeds/fireball.rs`) starts at the
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
chaingun, escape) goes in a new `src/zeds/boss.rs`, called from `zed.rs`, like
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
- When the game runs slower than 30 fps, a capture is put in the 1/30 s
  slot nearest the moment it was taken, and the empty slots before it
  repeat the *previous* picture (since 2026-10-07; before, the new
  picture filled them, so it showed up to a slot or more too early).
- Sound (added 2026-10-07): the recording gets exactly what the game
  sends to the speakers: our sound mixer plus the music, after the
  master volumes (KillingFloor.ini SoundVolume / MusicVolume). With
  `--mute` the recording still has sound, at the volume it would have on
  the speakers unmuted; the mute only silences the speakers. How:
  - The audio thread (the one feeding the speakers) copies the output
    into chunks of 1024 stereo samples (about 23 ms) while a recording
    runs, and passes them over a bounded queue (256 chunks, about 5 s).
    It never waits: if the queue is full, the chunk is dropped and
    counted (`audio_chunks_dropped` in `record_stop`; the gap becomes
    silence, so later sound stays in place).
  - An `audio-writer` thread places the chunks on the video's clock:
    "slot 0" is the moment F9 / `record` was handled; the first chunk is
    put where its real time falls after that (silence before it), and
    every later chunk at its sample count after the first. It writes raw
    samples to `<name>.audio.raw` next to the video.
  - Two files, then one: ffmpeg encodes the picture to `<name>.video.mp4`
    as before. When both finish, the raw sound is cut or padded with
    silence to exactly the video's length (frames / 30), and a second,
    quick ffmpeg run joins them into `<name>.mp4` (picture copied as is,
    sound encoded as AAC 192 kb/s), then deletes the two temporary files.
    We chose this over feeding ffmpeg the sound live through a second
    pipe: that needs named pipes (not portable to Windows), and ffmpeg
    reading two live pipes can stall when one runs ahead.
  - Sync measured on 2026-10-07 (pistol shots, `sound_play` log time vs
    the start of the shot in the file's sound): 10 to 68 ms late,
    GPU and software drawing (6.6 fps) alike, no growth over 22 s. The
    remaining lateness is the sound device asking for sound every 25 to
    46 ms: a sound waits for the next request. Against the picture
    (first changed frame after the shot) the sound is 3 to 68 ms late,
    i.e. within about 2 video frames.
  - If the sound cannot be added (ffmpeg error), the picture alone is
    saved as `<name>.mp4` and the raw sound is kept for a look.
- Stopping, or quitting while recording, closes the pipe and waits for
  ffmpeg to finish the file. The log has `record_start` (sound rate, how
  long starting took), `record_stop` (frames asked, frames written,
  repeats, sound chunks dropped), `record_audio` (sound chunks, where the
  sound starts, its length, gaps, `drift_ms`: how far the chunks' real
  times stray from their sample count) and `record_saved` (file, video
  frames and seconds, sound frames before and after fitting, how many
  were padded or cut, the join's exit status, bytes).
- Cost: one window-sized copy per recorded frame; expect a lower frame rate
  while recording at high resolutions.
- Bevy captures a window at most once per frame and silently drops a
  second request (extract_screenshots, "Duplicate render target"). So a
  frame with an F12 / `--screenshot` capture is skipped by the recorder;
  the next capture covers its slot. (Found when a `--screenshot` run never
  quit: its capture had been dropped.)
- The source is `src/engine/record.rs` (and the sound tap in
  `src/audio/capture.rs`).

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

**3D scopes (done 2026-10-04, after W2, on request).** `src/weapons/scope.rs`.
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

**W4 shotguns (done).** `src/weapons/projectile.rs`.
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
  stop): 3a^2 - 2a^3 of the time share a; other glide types move
  linearly. The curve is native code; checked against KF's engine
  (2026-10-08). When a key is reached part way through a frame, the rest
  of the frame's time carries on toward the next key if KeyFrameReached
  chains on (engine loop, also checked); before 2026-10-08 we dropped it.
- *Defaults (KFDoorMover).* InitialState TriggerToggle, MoveTime 1 (maps
  often set 0.5-2), MoverEncroachType 3 = ignore (12 doors set 1 =
  return, see "Doors push pawns" below). Blocks players, zeds, bullets and
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

**The Welder's screen (added 2026-10-06).** KF draws the weld percent on
the Welder model itself: Welder.InitMaterials puts a 256 x 256
ScriptedTexture (in a full-bright Shader) on Skins[3], and
Welder.RenderTexture draws the WelderScreen tile tinted BackColor
(128, 128, 128), then "Integrity:" at Y 50 and `ScreenWeldPercent@"%"`
(e.g. "42.50 %") at Y 85, centred, in NameFont (ROBtsrmVr24), coloured
R = 255 - 2p, G = 2.55p, B = 20 + p; "-" in white when there is no
target or the percent is 0. The target is WeldFire.LastHitActor of the
fire mode in use (set when a weld or unweld lands, not by aiming),
counted while within weaponRange x 1.5 = 135 units of the player.
`weapons/weapon/welder_screen.rs` does the same on the CPU: draws into an
image (redrawn only when the text or colour changes) and swaps the
Welder's slot-3 part to an unlit material showing it. `door.rs`
`WeldView.last_hit` holds each mode's LastHitActor (doors only: a hit on
a wall or nothing counts as no target). Assumed (native canvas code):
text width = glyph widths + Kerning, glyphs blended by the font page's
alpha, DrawTile's colour multiplies. The 2 decimals are UE2's float to
string ("%.2f", found in the game's engine files). The welder's fuel is not on the
screen in KF (it is the HUD's bar).

**Doors push pawns (added 2026-10-08, audit MOV-2).** Engine rule
(native, checked): every step of a moving door, before it moves, each
pawn it would overlap at its new pose is pushed. The push is the door's
own move plus, when it turns, 1.5 x how far the door's move carries the
point at the pawn's centre. The pawn makes a normal colliding move by the
push (sliding along walls; the door counts at its old pose). If the pawn
still overlaps the door at the new pose, the door's script decides
(Mover.EncroachingOn): KFDoorMover's default "ignore" moves on (the pawn
may end up inside the door; the next steps push it again), "return" (12
doors: KF-FilthsCross 10, KF-Crash 1, KF-WestLondon's StationBackGate)
stops the door that step and sends it back: to 'Open' if it was closing,
else to 'Close' (MakeGroupReturn; KFDoorMover's "stop" is the same). If
the pawn got clear and the push was over 2 units, it is moved back by half
the push, stopping at the door, so it ends next to the door, not 1.5x
away. Static meshes are not pushed; pawns without a controller, pickups
and gore get crushed or destroyed in KF (not done here: our corpses are
ragdolls, not pawns). As built (`door.rs` `encroach_pawns`, called from
the mover step): pawns are this game's player (walk mode) and the living
zeds (host or single player); overlap tests are the door's collision mesh
against the pawn cylinder; the moves use the walking code's sweep. Log
lines `door_push` (push, how far the pawn moved, whether it hit
something, how far it was pulled back, cleared) and `door_encroach`
(return). Not done / differences: remote players in network games are not
pushed by this game (their own game pushes them); the pawn-against-pawn
blocking is not part of the push sweep; the level's collision copy of the
door can be a frame behind, so the "old pose" during the push is
approximate; PlayMoverHitSound (the pawn's hit sound on a return) is not
played; return groups with a leader (KF-IceCave, all "ignore") are not
followed. A player trapped in an "ignore" door's arc is pushed in front
of the panel and can be carried round to the other side of a doorway
(seen on KF-Manor); that follows from the rule, not checked in KF.

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
- **T4, the trader woman (T4a done 2026-10-06).** The woman behind each shop's counter. Plan
  (from WeaponLocker, ShopVolume, KFPlayerController
  .ClientLocationalVoiceMessage, KFVoicePack, checked 2026-10-06):
  - She is a **WeaponLocker** actor the level designer placed in each
    shop (`placeable`, extends Actor). Every map with shops has them:
    KF-WestLondon 4, KF-Manor 5, KF-Farm 5, ... (a scan of all 39 map
    files: 35 maps have them; KF-MoonBase and KF-A-AliensTunnelBeta1-2
    have shops but no trader, KF-Menu and KFintro no shops). The other
    class, KFMod.Trader (a Decoration with the KFMapObjects.Trader
    mesh), is placed in no map: not used.
  - Class defaults: Mesh KF_Soldier_Trip.ShopKeeper_Trip (55 bones, one
    material, one animation), DrawType Mesh, collision cylinder 15 x 50
    (bCollideActors, bBlockActors). Per actor the maps set Location,
    Rotation, CullDistance (most 1000; 300 to 7000), sometimes
    DrawScale3D (KF-Departed, KF-Steamland 1.1), Skins (KF-FilthsCross,
    KF-Hospitalhorrors, KF-Suburbia: other outfits), and often
    bCollideActors / bBlockActors false.
  - Behaviour: PostBeginPlay and AnimEnd: `LoopAnim('Idle')`, so she
    idles forever. SetOpen (a player touching the open shop) only
    flips bClientTrigger; ClientTrigger's `PlayAnim('Gesture')` is
    commented out in KF's script, so nothing visible changes. She is
    never hidden: visible in waves and trader time alike, open shop or
    not. PreBeginPlay: every ShopVolume within 1000 units (visible from
    her) gets her as MyTrader (used for the buy menu's tag and the
    locational voice).
  - Voice: she says nothing herself. The radio lines are not
    locational; the in-shop Welcome (message 7, the only locational
    one, played at MyTrader's location) has no sender pawn and, by our
    reading (S5e), plays nothing. So no sound work in T4.
  - **T4a (done):** load her mesh, skins and Idle, spawn one per
    WeaponLocker (all modes: she is map content), placed like a zed
    (RotOrigin, mesh Origin and Scale, DrawScale x DrawScale3D, PrePivot,
    the actor's full Rotation), loop Idle at its own rate, hide beyond
    CullDistance (as the map decals do; distance to the actor's
    Location). Skinned on the CPU only while within CullDistance.
    Logs `shopkeeper_model`, `shopkeeper_spawned` (with her feet height
    against the floor of her collision cylinder), `shopkeepers_ready`,
    `shop_traders` (MyTrader per shop, distance only: the sight test is
    not done) and `shopkeeper_anim` every 5 s.
    As built (`game/shopkeeper.rs`, called from the map loader): her
    mesh has one sequence, Idle (301 frames at 30 a second, 10 s), mesh
    Origin (0, 0, 44.5), RotOrigin yaw -16384. Her feet come out 1 to 6
    units above the bottom of her collision cylinder on most maps
    (KF-Manor: 14 below; there the map shrinks her CollisionHeight).
    One model is loaded per distinct (mesh, Skins) pair. Bevy's
    VisibilityRange measures from the camera to the mesh entity, close
    to KF's distance to her Location (assumed equivalent).
  - **Later (T4b):** her collision cylinder (blocks the player where the
    map leaves it on), baked actor lighting (L4: she is lit like the
    zeds for now), the HUD's trader portrait (DisplayTraderPortrait).

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
  - FOV (degrees). 0: a box. Otherwise a frustum: the texture is its
    normal size at Location and each side moves out by tan(FOV/2) per unit
    of depth (confirmed 2026-10-08 from the engine's native projector code,
    details in the local RE.md; exact for square textures). The spawned
    decals use the same rule with their class FOV (ProjectedDecal 1,
    ROBloodSplatter 6).
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
  - Surface angle (2026-10-08, from the engine's native projector code;
    details in the local RE.md): KF draws each surface at strength
    max(0, -ProjDir . SurfaceNormal): full strength on a surface the
    projector faces straight on, about 0.71 at 45 degrees, nothing
    edge-on, and nothing on back faces (the ceiling under a floor decal,
    the far side of a thin wall). bProjectOnBackfaces turns this off (full
    strength on every side). The strength multiplies the gradient and the
    fades. Ours, per triangle: BSP and terrain triangles have a known
    winding (BSP: 2053 of 2055 polygons on KF-WestLondon wound one way;
    terrain is wound to face up), so their back faces are dropped.
    Static-mesh collision winding is not reliable (mirrored actors keep
    reversed triangles), so those triangles take the decal on whichever
    side faces the projector, at |dot|. Modulate decals carry the strength
    in vertex alpha, Add ones in rgb. Log: `decal_spawned` and
    `map_decal` lines give `angle_min` / `angle_max` /
    `backfaces_dropped`; `map_decals` the total dropped.
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
- K is a single brightness factor: 2. KF draws BSP lightmaps with a
  doubling blend (texture x lightmap x 2), read from its native code
  2026-10-08 (details in the local RE.md); it was a guess before.
- Zeds, weapons and other moving things kept the sun and ambient until
  L4 (now lit by the map's lights: "Actor lighting (L4)").

**Steps:**

- **L1, BSP lightmaps (implemented).** `bsp::read_lighting`; `kfpkg
  lighting <map>` checks it (all 35 maps read; every polygon's points
  equal its section vertices) and writes the pages to `work/lighting/`.
  Drawn per (material, page) with Bevy's `Lightmap`. Log `bsp_lightmaps`.
  KF-Clandestine, KF-Forgotten and KF-Hell save every page empty (the
  engine rebuilds them at load from the LightMaps entries: lights and
  per-texel shadow bits); since L1b we build them the same way.
- **L1b, stale pages rebuilt at load (implemented 2026-10-08, branch
  fix/stale-lightmaps; read from KF's native code, details in the local
  RE.md).** A page is really built from its surface lightmaps; the
  saved DXT page is a compressed copy made in the editor. Each page
  stores two revision numbers: its own and the one the saved copy was
  made from. KF uses the saved copy only when the two are equal;
  otherwise it builds the page at load. KF-Clandestine (1244 vs 0),
  KF-Forgotten (630 vs 0) and KF-Hell (65 vs 0) have every page out of
  date; KF-WestLondon and KF-Farm none. Steps:
  1. Read both revisions; "saved copy usable" = revisions equal
     (replaces "page is empty"). Log `pages_stale=N`.
  2. Build stale pages the way KF does, per surface lightmap (size
     SizeX x SizeY at OffsetX, OffsetY on a 512 x 512 page):
     - every texel starts at the zone's ambient: FGetHSV(AmbientHue,
       AmbientSaturation, AmbientBrightness) x 0.5, as a byte colour
       (x 255, rounded down, clamped). The zone is the lightmap's own
       (LevelInfo when it has no ZoneInfo).
     - for each light in its list (skipped if the actor is gone or
       deleted): the light's shadow bits (1 bit per texel inside the
       light's box MinX..MaxX, MinY..MaxY, rows of `stride` bytes) are
       softened with a 3 x 3 filter (weights 24 40 24 / 40 64 40 /
       24 40 24, total 320, each row's share rounded down to a byte
       separately; edges repeat the border bit), giving 0..254. A light
       with a one-row box or no bits is fully visible (255).
     - per texel (centre = Base + (x + 0.5) X + (y + 0.5) Y), the
       light's intensity byte: point lights round(shadow x A(d^2/R^2) x
       |h| / R), with R = 25 x (LightRadius + 1), h the light's height
       over the surface plane, A from a 4096-entry table (1 - 3s^2 +
       2s^3) / s, s = sqrt((i + 1) / 4096), indexed by d^2 / R^2 x 4093;
       0 at or beyond R. Sunlight: shadow x max(0, -dir . N) (two-sided
       surfaces: |dir . N|), rounded down. Spotlights: the point value x
       ((cos - e) / (1 - e))^2 inside the cone, e = 1 - LightCone / 256.
     - the light's colour is FGetHSV(LightHue, LightSaturation, 255) x
       LightBrightness / 255 x LevelInfo.Brightness (x 0 for LT_None and
       LT_BackdropLight); each channel adds min(255, 2 x intensity x
       colour) to the texel, saturating at 255 (LE_Negative subtracts).
     Validation: build KF-WestLondon's (current) pages with the same code
     and compare to the saved DXT pages texel by texel (`kfpkg
     lightmaps <map>`; DXT error only expected).
  3. Draw the built pages like saved ones; the three maps' walls lose the
     made-up sun fallback.
  As built: `lightmap_build.rs` (ue-assets), called from the map loader
  for out-of-date pages only (log `bsp_lightmaps_built`: pages, mean
  texel per page, lights used, about 0.1 s per map). Check `kfpkg
  lightmaps <map>`: on maps whose saved pages are current, built vs
  saved mean difference per channel KF-WestLondon 0.95, KF-Farm 0.99,
  KF-Manor 1.12, KF-Offices 0.69, KF-Biohazard 1.80 (of 255); averaged
  over 4 x 4 blocks (DXT's block size) WestLondon page 6 differs by 0.59
  on average, 3.1 at worst, so the larger single-texel differences are
  the saved page's DXT compression. Not exact: LightType pulse / subtle
  pulse lights use the middle of their wave (KF takes the value at the
  moment the page is built); effects other than none, spotlight, static
  spot, sunlight and negative are drawn as plain point lights and
  counted in the log (none on the three maps). Lights with brightness 0
  are listed by surfaces but add nothing (KF-Clandestine has many).
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
- **L3, terrain lighting.** Implemented 2026-10-08: see "Terrain lighting
  (L3)" under Terrain.
- **L4, moving things.** Zeds and weapons lit by the map's Light actors
  near them plus zone ambient, as UE2 lights actors; the made-up sun goes.
  See "Actor lighting (L4)" below.
- Not planned: KF's dynamic lights (muzzle flashes lighting walls),
  projected shadows of zeds, coronas.

Test: screenshots at fixed views before and after each step, and you
compare one view with the real game.

## Actor lighting (L4, planned 2026-10-07)

Why: zeds, the player's body, the first-person weapon and the trader
are lit by our made-up sun plus a flat ambient light, the same
everywhere, so a zed in a pitch-black corridor looks as bright as one
under a lamp.

**What KF does (Engine/Actor.uc, ZoneInfo.uc, class defaults; the
lighting code itself is native, so the formulas below are labelled):**

- Any actor with LightType other than LT_None is a light (placed Light
  actors, Sunlight, Spotlight, TriggerLight, the flashlight's glow).
  LightHue / LightSaturation give the colour (Unreal's saturation runs
  backwards: 255 is white), LightBrightness the strength, LightRadius
  the reach (25 x (LightRadius + 1) world units, as in UE1 and our
  flashlight), LightEffect the shape (LE_Sunlight: a direction with no
  reach limit; LE_Spotlight: a cone of LightCone; LE_NonIncidence /
  LE_QuadraticNonIncidence: no dependence on the surface angle).
- A mesh actor is lit per vertex by a few chosen lights: Actor.MaxLights
  ("limit to hardware lights active on this primitive"): Actor 4,
  KFMonster 5, Weapon 6, xPawn (the players) 8. bLightingVisibility
  (default true): "calculate lighting visibility for this actor with line
  checks", so a light behind a wall does not count.
- Plus the zone's ambient light: ZoneInfo AmbientBrightness / Hue /
  Saturation, and the actor's own AmbientGlow (KFMonster 0, KFPawn 0,
  KFWeapon 0, KFWeaponPickup 40). bUnlit actors ignore all of it.
- **Guesses** (native code): how the lights are picked (we take the
  strongest at the actor), the falloff with distance and the overall
  scale (both fitted against the map's own baked vertex colours, which
  UE2 computed from the same lights, see A4), the spotlight cone, and
  that the light sum is clamped at 1 before the x2 ("overbright")
  texture blend, like the baked meshes (K, above).

**How we do it:**

- A1. Read every light actor at map load (`ue_assets::level`, effective
  values with class defaults) and each zone's ambient. Log `map_lights`
  (count, by class, by effect).
- A2. `render/actor_light.rs`: a light-blocking mesh (the level's solid
  BSP, minus fake-backdrop sky walls, plus shadow-casting static meshes)
  for line checks; lights sorted into a grid. Per lit actor (component
  `ActorLight` on the actor): about 10 times a second, every light that
  reaches it, line-checked from its centre, strongest MaxLights kept (a
  Sunlight is lit if a line toward the sun leaves the level); each
  frame each light's share eases toward the new value (0.15 s) so lights
  do not pop on and off.
- A3. Lighting the mesh: our skinned meshes are already posed on the CPU
  every frame. Per vertex: ambient + sum over the chosen lights of
  colour x strength x max(0, normal . direction to the light), clamped at
  1, made linear, x K: written as the mesh's vertex colours on an unlit
  material, the same way the map's baked meshes are drawn (texture x
  colour x K). The light's falloff is taken at the actor's centre (one
  value per light per actor; UE2 hardware lights fall off per vertex: a
  small difference for a lamp more than an actor's height away).
  Applies to zeds (and their gore), players' bodies, the trader, the
  first-person weapon and hands (lit where the player stands). The
  flashlight (spot plus glow) is added as an extra light, so it still
  lights zeds. The sun stays for terrain and the three maps without
  saved lightmaps.
- A4. Check: with `KF_LIGHT_CALIBRATE=1`, compare our formula at baked
  static-mesh vertices with the colours UE2 stored there (log
  `light_calibrate`: fitted scale and error per falloff candidate), and
  pick the falloff.
- Log `actor_light` per actor once a second: zone, ambient, the chosen
  lights (name, class, distance, strength, blocked or not), and the
  resulting brightness.

Not done: light flicker / pulse types (LightType other than steady is
treated as steady), the muzzle-flash weapon light, bDramaticLighting,
ScaleGlow (its effect on lit actors in UE2 is not known), projected
shadows.

**As built (2026-10-07).** All of A1 to A4, plus placed and dropped
pickups (their own AmbientGlow: weapon pickups 40). What the fit
against the baked vertex colours showed (33 maps, 30000 vertices each,
`KF_LIGHT_CALIBRATE=1`, log `light_calibrate`; a vertex is compared
unless it is saturated):

- **A Sunlight lights only its own zone.** Maps with several Sunlights
  (KF-SirensBelch 11, KF-Bedlam 7, KF-Foundry 7) fit only that way
  (KF-SirensBelch correlation 0.46 -> 0.87). Ordinary lights reach into
  other zones (limiting them too made the fit slightly worse).
- **Static meshes block light** (those with bShadowCast, by their
  collision triangles): median correlation 0.71 with the BSP only, 0.85
  with meshes. Terrain does not block (not tried).
- **bDynamicLight lights are not in the baked colours** (spark emitters
  with LightBrightness 5000, TriggerLights): left out of the fit; actors
  still get them.
- **LE_StaticSpot** (effect 8, 78 lights on KF-Waterworks) is treated as
  a spotlight (guess); KF-Waterworks and KF-Wyre fit worst (correlation
  0.59).
- Falloff: Smooth (1 - smoothstep) median correlation 0.855, Linear
  0.845, Quadratic 0.833; best scale for Smooth 1.2 (most maps 1.1 to
  1.4). So a light gives colour x LightBrightness / 255 x 1.2 x falloff
  x incidence. That this also holds for actors is assumed.

Swapped materials on the first-person weapon (scope lens view, welder
screen) and on zeds (cloak, the Commando's spotted glow, the
Fleshpound's red device) are drawn as before (white vertex colours),
since they are displays or glows. `KF_LIGHT_SURVEY=1` logs, once per
map, the light a zed would get at every PathNode (`light_survey`:
percentiles, darkest and brightest spot); `KF_LIGHT_BSP_ONLY=1` lets
only the BSP block light (for comparing). Only parts on screen get new
vertex colours each frame.

**How KF lights actors (2026-10-08, after your play test: "a little too
dark").** Taken from KF's engine (docs/reverse-engineering.md); where each
rule lives is in `RE.md`, which stays local and is not in git.

- *Light cache*: each actor keeps up to 16 lights. Priority: Sunlight
  first, else LightBrightness x (1 - d^2 / (R + r)^2), with R the light's
  reach and r the actor's bounding-sphere radius; spotlights 0 outside
  their cone. Lights at 0 or below are dropped. In priority order the
  lights that are or were visible are used, up to MaxLights (at most 8; 4
  with bDramaticLighting). Was: strongest by light at the centre, checked
  ones only.
- *Line checks*: every 0.35 s per light, only for bStatic lights (the
  flashlight's glow and TriggerLights light through walls), from the
  actor's bounding-sphere centre to the light (Sunlight: 65536 units
  toward the sun), against the level and the shadow-casting static
  meshes (bShadowCast), stopping at the first hit, zero extent: as we
  had. A light's share then fades linearly from the old result to the new
  over 0.35 s (a new light fades in from 0). Was: every 0.1 s, eased.
- *Light strength*: point lights colour x 2 x (1 - smoothstep(d / R)) per
  vertex, sunlight colour x 1.75, times the surface angle. Light colour:
  FGetHSV(hue, saturation, 255) x LightBrightness / 255 x the level's
  brightness; FGetHSV has a brightness curve, so white at 255 is 0.82.
  Static meshes' baked colours are colour x 0.5 x a sample intensity of
  2 x smoothstep x angle for point lights, 2 x angle for Sunlight,
  ((cos - edge) / (1 - edge))^2 for the spotlight cone. Lit meshes are
  drawn at 2 x texture x light (skeletal meshes included). We now use:
  actor light = colour x 2 x falloff x angle (sun x 1.75) per vertex, at
  the vertex's own distance. The baked fit's 1.2 is 1.46 times KF's 0.82
  (not explained); KF's value matches your screenshot (below).
- *Ambient*: the zone's ambient colour, FGetHSV(AmbientHue,
  AmbientSaturation, AmbientBrightness), saved in the map, plus
  AmbientGlow / 255 (255: pulsing). KF-WestLondon's ambient brightness 2
  is 0.050 by that curve (saved vector: 0.0502, 0.0430, 0.0263, exactly
  ours), not 2 / 255 = 0.008: six times more. We use the saved vector. KF
  takes the brightest zone the actor's box touches; we take the zone at
  the centre.
- ScaleGlow is not used for lit actors.

Checked against your KF-WestLondon screenshot `precise.jpg` (pose
`--camera " -2760,1772,-3768,-3.6652,-0.0652"`, 1280 x 720): the hands
and gun average 30.8 brightness (of 255) in KF. Ours: before 1.5, with
the native rules 5.5 (the sun is blocked by a barricade's top, 73 units
above the eye), and 30.6 with the sun let through (44 with the fitted
1.2 colour). So in KF the sun reached the gun there; our pose may be off
by enough to put us in the barricade's shadow (**not known**).

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

## End of match screen (planned and built 2026-10-06)

"Your squad survived" / "Your squad was wiped out". `end_game.rs`, from
KFGameType.CheckEndGame, the MatchOver state (DeathMatch, KFGameType),
PlayerController's GameEnded state and HUDKillingFloor.DrawEndGameHUD.

What KF does:

- **When.** Won: the wave timer finds WaveNum > FinalWave with no zeds
  left (after the Patriarch). Lost: CheckMaxLives finds no player with a
  life left. KF's MaxLives is 1 (KillingFloor.ini, [KFMod.KFGameType]),
  so in solo the first death ends the game: there is no respawn.
  CheckEndGame sets GRI.EndGameType (2 won, 1 lost), puts every player
  in behind view (ClientSetBehindView) and every controller in its
  GameEnded state (P.GameHasEnded): players and zeds stop.
- **The picture.** DrawEndGameHUD draws Combiner'VictoryCombiner' or
  'DefeatCombiner' (package KFMapEndTextures) white, STY_Alpha, as a
  square of side Clamp(ClipY, 320, 1024) pixels centred on the screen,
  texels 0-1024. Its alpha is EndGameHUDTime x 255; HUD.Tick adds
  delta / 3 while it is under 1: a 3 s fade in.
  Each combiner adds (CombineOperation 3, CO_Add) a 1024 x 1024 DXT5
  picture (VictoryTexture: white text; DefeatTexture: the text on a
  blood splash) and a blurred glow (VictoryTextureOverlay: green;
  DefeatTextureOverlay: grey) moved by a TexOscillator (OT_Pan): Victory
  U 0, V 10 per second, amplitude 0.01 both; Defeat U 10, V 0.5 per
  second, amplitudes 0.01 and 0.015.
- **The HUD.** In GameEnded the player is spectating in behind view, so
  HUD.DrawHUD calls DrawSpectatingHud: no health / ammo bar, no weapon
  name, no first-person weapon. Won: the picture, then return (no
  circle, no messages). Lost: the picture, then DrawKFHUDTextElements
  (the circle, trader distance) and the messages on top.
- **Restart.** GameEnded.BeginState sets bFrozen and a 5 s timer; after
  it, Fire restarts the game (ServerReStartGame, PlayerCanRestartGame is
  always true). MatchOver.Timer (once a second) restarts on its own when
  Level.TimeSeconds > EndTime + RestartWait: EndTime = end +
  EndTimeDelay 4 (KillingFloor.ini, UnrealMPGameInfo), RestartWait 10
  (DeathMatch), so about 14 s after the end.
- **Sounds.** None: KFGameType's EndGameSoundName are all 'Sound' (no
  announcer). MatchOver's BossLaughtIt (a living Patriarch laughs) is
  already done (G3b).

What we do:

- `WaveGame.phase` Won / Lost (waves.rs) stays the trigger. New:
  `MatchOver` (end time, EndGameHUDTime) follows it.
- The picture is a Bevy UI material node (`EndGameMaterial`): a shader
  adds the two textures, the overlay's UV moved by Amplitude x
  sin(2 pi x Rate x time) (TexOscillator is native: the sine and the 2 pi
  are assumed; the matrix stored in the package fits Amplitude x sin).
  Alpha: picture alpha + overlay alpha (AO_Use_Mask with no mask is
  native, unknown: **guess**), times the fade.
- Zeds: on the end, all zeds go "braindead" (the existing DoBossDeath
  rule: no thinking, moving or attacking) except a pending boss laugh.
- The player: in waves mode a death no longer respawns (health stays
  0); hits are ignored once dead. After the end the player cannot move
  or fire; looking around still works (GameEnded.PlayerMove turns the
  view). On restart the player is put back at the start with 100 health
  and no armour.
- Restart: Fire after 5 s, or at the first whole second past 14 s, or
  Enter / test action `restart_game` (ours, kept). KF's RestartGame
  travels to the next map of the map list (bChangeLevels=True); we
  restart the waves on the same map (as before).
- The view goes behind the player's own body (ClientSetBehindView; see
  "The player's third-person body").
- Not done: the scoreboard ("Your squad survived!" /
  "Squad eliminated." from HUDBase, only when the scoreboard key is
  held: no scoreboard yet).

Logs: `end_game` (result, wave, end time, restart time), `end_game_fade`
(EndGameHUDTime reaching 1), `end_game_restart` (why: fire / timer /
key), `end_game_loaded` (textures and oscillator values read).

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

**In a network game (2026-10-07).** As in KF, only the host decides
(`ZedTimeRole::Host`): it rolls for every player's kills, with the
killer's perk (lobby record) for the extensions and the killer's pawn
for the 3 m check, and sends each start / extension, speed-up and end
to the clients (`net/zedtime.rs`). Clients (`ZedTimeRole::Client`)
never roll; they send their explosion and debug rolls to the host and
follow its commands, running the same countdown, easing and sounds.
Single player (`ZedTimeRole::Local`) is unchanged. Details and test
numbers: docs/multiplayer-prototype.md, "Shared zed time".

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
mixer of our own (`src/audio/mixer.rs`). It feeds `rodio`, the library Bevy's
audio already uses, so nothing new is downloaded. The mixer adds the
voices together sample by sample. Each frame the game updates every
voice's volume, left/right balance and pitch from the listener's position.
Our voices and the music meet in one rodio mixer whose output passes a
"tap" (`src/audio/capture.rs`) on its way to the speakers; the tap copies
the sound into video recordings (see "Video recording"). `--mute` is
applied after the tap: the speakers get silence, but everything is still
mixed at the normal volumes (so recordings keep their sound). If the
machine has no sound device, the game logs it (`audio_device ok=false`)
and a small thread pulls the mixed sound at real-time speed and throws it
away after the tap, so voices still end on time and recordings still
have sound.

**Distance falloff (a guess, to be checked by ear).** Since S4a: OpenAL's
"inverse distance clamped" model, the sound's Radius as the reference
distance and the ini's Rolloff 0.5: full volume inside the radius, then
radius / (radius + 0.5 x (distance - radius)); no cut-off, but voices too
quiet to hear (final gain under 0.002) are not started. S2 to S3 used a
linear fade to silence at the radius; KF's small zed radii (footsteps
100) showed that cannot be right.

**Volume cap (from KF's engine, 2026-10-07; details in local `RE.md`).**
Not a guess any more: KF clamps a sound's volume to 0..1, multiplies it by
the ini's SoundVolume (0.3), clamps again; the distance fade comes after.
So every volume of 1 or more plays the same: guns (1.8), the trader's
lines (ShoutVolume 2), the radio beep (10) and pickups (100) all end at
gain 0.3. `mixer.rs` (`play_volume`) does this for every sound. Until
2026-10-07 the player's own sounds were not capped (only world sounds
were), which made the trader's line play at 0.6 and the radio beep at
1.0, two to three times louder than in KF. The VoiceVolume setting is
for voice chat only, not game sounds. Zed time: the voices' pitch is multiplied by the game speed (assumed
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
- **S2. The mixer** (done 2026-10-05). `src/audio/mixer.rs`. Systems write a
  `PlaySound` message (KF's PlaySound arguments, its defaults filled in);
  `play_sounds` finds the sound (cached, a weighted random pick for
  groups; the map's own sounds, preloaded with `per_map`, and the
  packages only they use leave the cache at each map unload, shared
  sounds stay), applies the slot rule, skips sounds out of range, and enforces
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
  quietly recorded select sounds, peak 0.18 against 0.99). (Replaced
  2026-10-07: KF caps the volume at 1 *before* the master volume, see
  "Volume cap" above.) An
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
  - **S4b. Zed voices and loops** (done 2026-10-05, except the parts
    moved to S4b2 below). Each zed collects sound events (`ZedSound`)
    from the hit, death and AI code; `play_zed_sounds` plays them on the
    zed and keeps its AmbientSound (SoundVolume, radius SoundRadius x
    AmbientSoundScaling: a guess, e.g. 80 x 6.83 = 546, KF's comment says
    "10 meters") in step. Challenge: at the first sight and when it sees
    the player more than 7 s after the last, checked every 0.5 s (a
    guess at UE2's sight interval). KFMonster: moans (MoanVoice, SLOT_Misc,
    MoanVolume 1.5, radius 250; first after 2 + 36 x rand s, then every
    12 + 8 x rand s, whole seconds; never headless), pain (HitSound[0],
    SLOT_Pain, 1.25, radius 400, at most every 0.35 s, not for fire
    damage), death 0.2 s after dying (DeathSound / headless / gibbed),
    decapitation, Impact_Skull on headshots, the melee hit on the player,
    challenge sounds, AmbientSound loops (Scrake's chainsaw, Bloat,
    Husk, Patriarch).
  - **S4b2. The rest of the zeds** (done 2026-10-05, except gibbed
    deaths: our zeds are never gibbed whole). Speech comes from the
    AnimNotify_Script notifies (their function names) passed in his
    animations; the chaingun's loop from `boss::MgSound` (FireMGShot,
    the pause, EndState); the projectiles carry LAWProj's AmbientSound
    (255, 250) and play ExplosionSound at 2.0 (radius 500). The Patriarch's own sounds (speech
    from his AnimNotify_Script calls: KnockedDown, Entrance, Victory,
    WarnGun, WarnRocket, the taunts; Kev_SaveMe; the rocket; the
    chaingun's fire and spin loops; the impale hit), ragdoll bumps
    (Zomb_BodyImpact), the Husk fireball's flight loop and impact, the
    Bloat's acid puddle, zeds landing (Player_LandDirt), gibbed deaths.
  - **S4c. The player** (done 2026-10-05, `src/audio/player_sound.rs`; the
    damage code sends Hurt / Died; jumps, landings and footsteps are read
    from the walker each frame; surfaces default until materials carry a
    SurfaceType; no crouch or walk, so no quiet steps). Also the zed time
    sounds (KFPlayerController: Zedtime_Enter / _Exit, SLOT_Talk, 2.0,
    pitch 1.1 / TimeDilation, i.e. 1 / game speed for us). Pain (Inf_Player Wounding, SLOT_Pain, 0.6,
    radius 200, every 0.35 s at most), death, footsteps (CheckBob, about
    every 0.375 s at full speed, per surface, 0.45, x 0.4 crouched or
    walking), jump and landing, low-health breathing (under 25 %).
- **S5. The world** (researched 2026-10-05; notes with script quotes in
  `work/s5-research.md`, untracked), in parts:
  - **S5a. The map's sounds** (done 2026-10-05, `src/audio/map_sound.rs`).
    AmbientSound actors (2915 in the 40 maps; Engine.AmbientSound
    defaults SoundVolume 100, SoundRadius 100) and other actors with an
    AmbientSound (ZoneInfo on 3 maps, ...): a looping AmbientSound with
    `Falloff::Radius` (a guess at the native code: silent outside
    SoundRadius; inside, full volume with bFullVolume, else a linear
    fade) at SoundVolume / 128 x the ini's [Engine.AmbientSound]
    AmbientVolume (0.25). SoundEmitters (random one-shots): every
    EmitInterval +- a random part of EmitVariance, at the actor (volume
    and radius as its loop: a guess). ScriptedTriggers whose
    WAITFOREVENT matches a PlayerStart's Event play their PLAYSOUND when
    the player spawns (KF-WestLondon, KF-Farm: Merlin_Takeoff, Volume
    255, bAttenuate false: at the listener). The mixer skips the work of
    silent voices (most map loops at any moment). SoundPitch 0 is taken
    as 64. Several of KF's sounds are silent placeholders (zombiehoard2,
    WoodDoorShut, ChopperLands: 8 kHz, all zeros), as in the real game.
  - **S5b. Doors** (done 2026-10-05). Mover: DoOpen / DoOpenToKey
    OpeningSound, FinishedOpening OpenedSound, DoClose ClosingSound,
    FinishedClosing ClosedSound (KFDoorMover: none while welded or
    broken), PlaySound(X, SLOT_None, SoundVolume / 255, false,
    SoundRadius, SoundPitch / 64), Mover defaults 228 / 64 / 64;
    MoveAmbientSound loops while the door moves. Trader doors the same.
    KFDoorMover: a zed hitting a welded door plays Zomb_HitDoor_Wood /
    _Metal (SurfaceType 3) at 2.0, radius 200, at most every 0.5 s;
    breaking plays Door_Break_Wood / _Metal at 2.0, radius 5000. Each
    weld tick: PatchSounds.WelderFire at the weld point (KFHitEmitter:
    150 capped, radius 80). Not done: the key-door unlock sound (no key
    items). Also in this step (ours, not KF's): player landings slower
    than 100 units/s are not played, because our walker leaves the
    ground for a frame on steps and edges (UE2 stays walking).
  - **S5c. Explosions and projectiles** (done 2026-10-05). Each
    explosive, thrown and dart class's sounds (`ProjectileSounds`, read
    from its defaults): the flight loop (AmbientSound with SoundVolume,
    SoundRadius; Projectile's SoundVolume default 0 = none), the
    explosion (ExplodeSounds at random, or ExplosionSound; at 2.0 or
    ExplosionSoundVolume, e.g. the Husk Gun's 1.25 / 1.65 / 2.0), the
    Nade / pipe bomb bounce (ImpactSound, SLOT_Misc, when faster than
    50), the pipe bomb's beeps (each check 0.5 / radius 50; the
    countdown SLOT_Misc 2.0 / 150), the LAW / M79 dud (PTRD_deflect04).
    Not done: the flamethrower flames' and puddles' fire loops, the
    Siren's "disintegrate" sound, the crossbow / M99 bolt hits (S5d).
  - **S5d. Bullet impacts, glass, bolts, nails** (done 2026-10-05).
    ROBulletHitEffect: PlaySound(HitSound, SLOT_None, 1.0, false, 100) by
    SurfaceType; ours always the default entry (Impact_Dirt) until the
    materials' SurfaceType is read. Bullets on zeds: no impact sound (KF
    only spawns the effect for non-pawns). Glass: each crack or break plays
    bullethitglass / bullethitglass2 (KFHitEmitter, 150 capped, radius 80).
    Crossbow / M99 bolts: bullethitflesh4 on a zed (defaults), one of
    bullethitflesh2/3/4 on a wall at 0.75 (radius 300); picking a bolt up
    Ammo_GenericPickup (SLOT_Pain, 0.6, 400). Nails: 40% of bounces
    Impact_Metal. Not done: the bullet whiz (only the Patriarch's chaingun
    could cause it solo; native, not known), shell casings (none in KF).
  - **S5e. The trader** (done 2026-10-06, `src/audio/trader_voice.rs`). Buying
    a weapon or the vest: its PickupSound (MakeSomeBuyNoise, SLOT_Interface,
    255 capped, radius 120); a purchase refused for dosh or weight:
    KF_Trader.TooExpensive / TooHeavy (2.0; KF plays them on selecting the
    item in the list, ours on the refused purchase). Buying ammo and
    selling are silent, as KF. The radio lines (KFVoicePack TraderSound
    0-6) at KFGameType's moments: 0 "moving" after 20% of the wave is
    dead, 1 "almost open" at 80% (solo only farther than 30 m from the
    shop; KF checks at each kill, ours each second), 2 / 3 at OpenShops,
    6 at CloseShops (WaveNum < FinalWave - 1), 4 at 30 s left (not in a
    shop, nor after entering one since the last "closed"), 5 at 10 s (only
    in a shop). Each: Walkie_Beep (SLOT_Talk, 10) and the chat beep
    (bullethitflesh2) at once, the line 0.6 s later (SLOT_Interface, 2.0,
    bNoOverride). The Welcome line plays nothing (no sender: our reading).
    Not done: menu click sounds (no KF menus yet), the perk sound (no perks).
- **S6. Music** (done 2026-10-05, `src/audio/music.rs`). The map's
  KFMusicTrigger (Song, CombatSong, WaveBasedSongs[n].CombatSong /
  CalmSong, FadeInTime, FadeOutTime; every map's survey: none sets the
  fade times, so KF switches songs at once). `game.rs` keeps KF's
  MusicPlaying / CalmMusicPlaying and sends a cue at the same points as
  MatchInProgress.Timer (combat in a wave, calm in trader time) and at
  the Patriarch's entrance (BossBattleSong KF_Abandon, MTRAN_FastFade:
  1 s out, 1 s in). KFMusicInteraction's fade: the old song's volume
  goes down linearly, stops under 0.1 of it, then the next starts with
  PlayMusic's fade-in (native; assumed linear). Songs are streamed from
  `Music/<name>.ogg` (rodio's Ogg Vorbis decoder, already in the build
  through Bevy). The trigger's song fields are `localized`: the map's
  `System/<map>.int` section ([KFMusicTrigger0]) replaces them (found
  2026-10-06 after you heard DirgeDisunion1 in the real game: the map
  stores KFSIN8, the .int says DirgeDisunion1 plus a per-wave list). A
  name with no file still plays nothing (`music_missing`); left after
  the .int: KF3, KFRock, KFSIN7, smoothjazz as fallbacks on 7 maps. Volumes: SoundVolume and
  MusicVolume are read from the install's KillingFloor.ini (read only;
  0.3 and 0.1 shipped). Music ignores zed time.

Not planned: Doppler (DopplerFactor 1.0; small effect, later if missed),
EAX reverb (off in KF's ini), voice chat.

## Surface types (F1-F2 implemented 2026-10-06)

KF picks impact effects and step sounds by surface: dirt, metal, wood,
glass, concrete... (ESurfaceTypes, 20 used values, Material.uc /
Actor.uc). Until now every surface used the EST_Default entry.

**Where KF stores it.** `Material.SurfaceType` on every material object:
textures, and also shaders, combiners and final blends, which mostly set
their own (KillingFloorTextures: 59 of 69 shaders set one, often not
their texture's value). Actors have their own `Actor.SurfaceType` too
(StaticMeshActors in maps set it).

**How KF reads it.**
- Bullets (ROHitEffect.PostNetBeginPlay): a 16-unit trace into the
  surface returns HitMat; `HitMat.SurfaceType` picks the entry of
  ROBulletHitEffect.HitEffects[20] (sound, emitter, decal). The material
  object itself, not its texture: no looking through shaders.
- Footsteps, jumps, landings (KFPawn.FootStepping / GetSound): in a
  water volume the deep-water step; else the Base actor's SurfaceType if
  it is not the level and not 0; else a trace 16 units below the feet
  and that material's SurfaceType.
- Terrain: what material a trace on terrain returns is native; **guess**:
  the layer with the most alpha at that spot.

**How we do it.**
- F1. Data: each collision triangle carries (material surface, actor
  surface). BSP: the surface's material; static meshes: the section's
  material (the actor's Skins override) and the actor's SurfaceType;
  terrain: the guess above. Colliders get a per-triangle table; a ray
  that hit a collider is cast again against that collider's triangle
  mesh (parry, which avian uses) to learn the triangle index. Logged per
  map: triangles by surface.
- F2. Use: bullet impacts (sound, emitter and decal from HitEffects[20];
  8 more bullet-hole decal classes), footsteps, jumps and landings
  (SoundFootsteps / JumpSounds / LandSounds [20]).
- Not done: the deep-water step in water volumes; doors and glass panes
  have no SurfaceMap (KF's ROHitEffect trace ignores actors anyway); the
  static meshes' simplified collision models (we collide with the
  render triangles, so their materials).

## Hit effects: splashes, view shake, blur (milestone 14, planned 2026-10-06; E1-E4 implemented)

What the player sees when hurt, vomited on or screamed at. All from the
scripts (KFHumanPawn, KFPawn, KFPlayerController, HUDKillingFloor,
ZombieSiren, Engine.PlayerController, UnrealPlayer) and class defaults;
the parts that are native code are marked.

**Where it starts (Pawn.TakeDamage).** actualDamage = after ReduceDamage
and armour (ShieldAbsorb). Then PlayHit (camera jar, below) and, if
still alive, Controller.NotifyTakeHit (splash and damage shake). Both
skip a hit of 0 damage, a hit that kills, and god mode
(KFHumanPawn.TakeDamage returns at once with bGodMode). Bile ticks
(TakeBileDamage) go through the same Pawn.TakeDamage, with
DamTypeVomit. Ours: `apply_player_damage` sends one event per hit
after armour, carrying the KF damage type.

**Whole numbers (2026-10-06).** Pawn.TakeDamage and
MeleeDamageTarget take an int, so damage to the player loses its
fraction there (`apply_player_damage`; a raging Fleshpound's x 1.75 is
applied to the already cut value, as its MeleeDamageTarget override
does). Before this, zed claws hurt by MeleeDamage x 0.95..1.05 with the
fraction kept.

**Damage types (ZombieDamType defaults, the attack code).**
ZombieMeleeDamage (blunt): KFMonster's default, so Clot, Gorefast,
Crawler (and its pounce), Bloat, Husk, Scrake, Fleshpound, Patriarch
melee. DamTypeSlashingAttack: Stalker, Siren melee. DamTypeVomit: vomit
and bile ticks. SirenScreamDamage: the scream. Others (not
DamTypeZombieAttack): Husk fireball and burning (DamTypeBurned),
Patriarch rocket and our own explosives (DamTypeFrag), Patriarch
chaingun (plain DamageType), pain volumes, falling out of the world.

### E1. The hit splash (HUDKillingFloor.DisplayHit, DrawDamageIndicators)

- DisplayHit: for a DamTypeZombieAttack, DamageStartTime = its HUDTime
  (0.9; DamTypeVomit 1.5); else Clamp(Damage / 5, 0.2, 1.5).
  DamageHUDTimer = now + DamageStartTime. Vomit also sets VomitHudTimer =
  now + 0.8.
- Drawn each frame over the whole screen, before the HUD widgets: white,
  alpha Clamp(left / DamageStartTime x 200, 0, 200) (of 255).
  Texture by type: HUDDamageTex (DamTypeZombieAttack and
  SirenScreamDamage: KillingFloorHUD.BluntSplashNormal; Slashing:
  SlashSplashNormalFB; Vomit: ClassMenu.VomitFB); not a zombie attack:
  GoreSplashFB. The "uber" textures are never used: KF's DisplayHit
  reads HudBase.DamageTime[0], which only HudBase.DisplayHit sets, and
  KF's override never calls it.
- The materials: GoreSplashFB and VomitFB are FinalBlends with alpha
  blending (FrameBufferBlending 2). SlashSplashNormalFB uses blending 3
  (AlphaModulate); drawn with plain alpha blending here (a guess; the
  native blend for 3 is not in the scripts). VomitFB wobbles through
  TexOscillator VomOsc: U stretches (OscillationType 1) at 1.5 Hz, V
  pans at 0.5 Hz, both by 0.03 of the texture. The oscillator maths is
  native: taken as offset = amplitude x sin(2 pi x rate x time) (a
  guess).
- Health digits: while VomitHudTimer runs they are (196, 206, 0)
  instead of the usual colours ("poisoned").
- Logs `hit_splash` (type, damage, seconds). Check: vomit at a Bloat
  gives 1.5 s splashes and 0.8 s green digits on each bile tick.
- As built (2026-10-06): `PlayerDamaged.dam_type` (combat.rs `DamType`;
  the zed's from its ZombieDamType default, logged in
  `zed_class_loaded`); `apply_player_damage` sends `PlayerHurt` (damage
  after armour, alive, not god mode); hud.rs `display_hit` and the splash
  quad, the first one drawn (under the widgets, over the vision
  overlay). The textures and HUDTime come from the damage classes'
  defaults (`hud_splashes` log). The splash goes over the vision
  overlay; in KF DrawModOverlay runs in the same HUD pass, before or
  after not checked.

### E2. View shake and the hit-blur timer (render only; aim unchanged)

KF adds ShakeRot / ShakeOffset and the ambient shake to the camera only
(CalcFirstPersonView), never to the aim. Ours: added to the main and
sky cameras after all gameplay systems, removed before the next frame.
The first-person weapon is left unshaken (a guess: KF draws it at the
unshaken view pose inside the shaken view, so it may jiggle on screen
there; to compare in the game).

- **ShakeView(RotMag, RotRate, RotTime, OffsetMag, OffsetRate,
  OffsetTime)**: takes the new rotation shake only if VSize(RotMag) >
  the running one's, same for the offset. **ViewShake** each frame: the
  offset moves at its rate; **CheckShake** bounces it at the max and
  shrinks the max (Time > 1: max x (1/Time - 1) if Time x |max/rate| <=
  1, else -max; Time -= dt) until Time <= 1 ends it. Rotation the same,
  in Unreal rotation units (65536 = a full turn), wrapped. Ported as
  written (it depends on frame rate, as in KF).
- **DamageShake(Damage)** on every NotifyTakeHit: rotation (30 x
  Damage, 0, 0) at rate 120000 pitch for 0.15 + 0.005 x Damage; offset
  (0, 0, 0.03 x Damage) at rate (1, 1, 1) for 0.2.
- **PlayTakeHit jar** (KFHumanPawn, at most every 0.1 s: Pawn.PlayHit's
  LastPainTime): direction = from the hit point to the player, flat,
  turned into view space; JarrScale = Min(0.1 + Damage / 10, 1).
  DoHitCamEffects(dir, JarrScale, 2.0, 1.0). Bile ticks also do
  DoHitCamEffects(random 0..1 vector, 0.35, 2.0, 1.0).
- **DoHitCamEffects(dir, jar, blurTime, durScale)**: AddBlur(blurTime,
  0.8) (NewSchoolHitBlurIntensity, the PostFX path), then ShakeView with
  rotation (1000 x -dir.X, 0, 1000 x dir.Y) x jar at rate (10000 x
  -dir.X, 0, 10000 x dir.Y) x (2 - jar) for 4 x durScale; offset dir x 50
  x jar at rate +-200 (sign of dir) x (2 - jar) for 3 x durScale
  (JarrRotateMag/Rate/Duration, JarrMoveMag/Rate/Duration).
- **Siren scream (ZombieSiren.DoShakeEffect)**, each scream pulse,
  hit or not: within ScreamRadius 700 of the view: scale = (700 - dist) /
  700, BlurScale = scale; behind a wall (FastTrace) both x 0.25, else
  scale = 0.6 + 0.4 x scale (Lerp(scale, MinShakeEffectScale 0.6, 1)).
  SetAmbientShake(now + 0.25, 2, OffsetMag (0, 5, 1) x scale, 500,
  RotMag (150, 150, 150) x scale, 500); AddBlur(2, BlurScale x 0.85).
  **Ambient shake** (CalcFirstPersonView): full until the falloff start,
  then fades linearly over 2 s; offset = OffsetMag x falloff x
  sin(time x 500 x 2 pi) in world axes; rotation the same.
- **AddBlur(time, intensity)**: restarts the fade (BlurFadeOutTime =
  StartingBlurFadeOutTime = time), intensity = the higher of old and
  new. Each frame the blur amount = left / time x intensity. When the
  fade reaches 0, StopHitCamEffects: intensity 0, blur off, **and the
  view shake stops** (StopViewShaking). Dying does the same.
- Logs `view_shake` (start: source, magnitudes), `hit_blur` (start,
  end), and in `--frames` runs the per-frame shake offset and blur
  amount every 10 frames. Check by numbers: a 10-damage Clot hit gives
  pitch shake 300 units for 0.2 s, blur 0.8 fading to 0 over 2 s.
- As built (2026-10-06): `player/hit_cam.rs`. `PlayerDamaged.source`
  (the zed, blast centre or muzzle; None for bile, burning, the level)
  gives the jar direction; `PlayerHurt.bile_tick` the bile jar. The
  Siren's settings load from her class defaults (`ScreamShake`) and each
  scream pulse sends `SirenScreamShake`. The camera is shaken through
  its GlobalTransform in PostUpdate and put back in First; rotation is
  added in Unreal rotator units through the Unreal view matrix (roll
  sign follows `coords::ue_rotation_matrix`). The pawn's Rotation for
  the jar direction is the view's yaw (assumed). Found when testing: the
  DamageShake kick (rate 120000) reaches its max inside one frame and
  ends there (one-frame kick); the jar (bigger) usually wins; a jar
  settles in about 0.8 s. Logs `view_shake`, `hit_blur`, and `hit_cam`
  every 10 frames while active.

### E3. The blur itself (native: PostFX blur, a guess)

KFPlayerController.SetBlur(amount) turns on Red Orchestra's PostFX
blur pass with parameter `amount` (0..1); the filter is native code,
not in the scripts. The old non-PostFX fallback (UnderWaterBlur, a
MotionBlur camera effect, BlurAlpha 35) is a blend with the previous
frames. Ours: a full-screen pass after the scene and weapon, before
the vision overlay and HUD: the frame mixed with a blurred copy by
`amount`. The blur size is a guess, to be compared with the real game
by you.

As built (2026-10-06): `render/hit_blur.rs`, a Bevy FullscreenMaterial
pass on the weapon camera (the last 3D camera on the window, so sky,
scene and weapon are blurred; the vision overlay, trader arrow and HUD
come after and stay sharp). 12 taps on two rings (radius and half
radius) plus the centre; radius = amount x 1.1% of the screen height;
result = mix(frame, blurred, amount). The pass is on the camera only
while the amount is over 0 (`hit_blur_pass` on / off in the log). In
behind view (Patriarch cut scenes) the weapon camera is off, so no
blur there.

### E4. The near-death look (DrawModOverlay's NearDeathOverlay)

Alive and under HealthMax x 0.25, DrawModOverlay draws NearDeathOverlay
(KFX.NearDeathShader) instead of the sepia VisionOverlay, with the same
zone tint and modulate blend. NearDeathShader is the MaterialSequence
DeSat (looping, TotalTime 1.5): fade to InjuredGrain over 0.5 s, then to
Grain1 over 1.0 s (SequenceItems decoded by hand from `kfpkg raw`).
DrawTileScaled(Mat, SizeX, SizeY) takes scale factors (KF's film grain
passes ClipX / 1024 to fill the screen), so the tile is SizeX times the
texture and only its top-left texel shows: InjuredGrain (255, 13, 13),
Grain1 (174, 172, 174). So the view pulses between "red channel only"
and "a third darker". This also explains why the sepia look is a flat
colour (its texel is white). The sequence's fade is native: a straight
blend on the game clock (a guess). Only drawn where the vision overlay
is (maps or zones without it get no near-death look either, as in KF).
As built (2026-10-06): render/overlay.rs, log `near_death_overlay`
on / off.

Not planned here: the bloody
skin on the player's third-person body (InjuredOverlay), the bullet
whiz blur (HandleWhizSound; not checked whether the Patriarch's
chaingun triggers it).

## The player's character and first-person sleeves (implemented 2026-10-06)

Bug: each weapon drew its arms with its own default skin, so switching
weapons changed the sleeves. In KF the sleeves come from the player's
character.

**How KF does it.**
- Characters are records in `System/*.upl` files (xUtil.PlayerRecord):
  `Player=(DefaultName="Corporal_Lewis",...,Species=KFmod.SoldierSpecies,...)`.
  56 files, one record each. The name the player picked is
  `[DefaultPlayer] Character=` in the user's ini.
- KFPawn.Setup: if the record has no species, or the species is not a
  SPECIES_KFMaleHuman, use `GetDefaultCharacter()` = "Corporal_Lewis"
  (also the value in defuser.ini, KF's fresh-install defaults).
  FindPlayerRecord compares names ignoring case; an unknown name gives an
  empty record, so the default character.
- Each species class has `SleeveTexture` (KFSpeciesType, default
  `KF_Weapons_Trip_T.hands.hands_1stP_military_diff`; e.g.
  CivilianSpeciesThree, Mr_Foster's, sets
  `KF_Weapons2_Trip_T.hands.Office_Worker_Hands_1st_P`).
- KFWeapon.BringUp calls HandleSleeveSwapping: `Skins[SleeveNum] =
  SleeveTexture` (SleeveNum defaults to 1 in KFWeapon; weapons override
  it). It runs after PreloadAssets has set the SkinRefs skins, so the
  sleeve wins.

**How we do it.** `src/player/character.rs` reads the .upl records and
picks the character (`--character NAME`, else Corporal_Lewis; an unknown
name logs the valid names and falls back to the default, as KF would).
It reads the species' SleeveTexture through the class defaults. Weapon
loading puts that material on slot SleeveNum of every weapon (start
inventory and bought ones). Logged: `character` once, `weapon_sleeve`
per weapon.

Not done: voice packs (the body: next section). We do not read the
user's own `User.ini` choice (KF would use it); the user asked for KF's
default unless `--character` is given.

## The player's third-person body (planned and built 2026-10-06; TP1-TP3 implemented, TP4 switched off)

The player had only first-person arms. KF gives every player pawn a full
body, hidden from its own first-person view but seen by everyone else,
and by the player in behind view (the end of the match, F4).

**What KF does (scripts and class defaults, checked 2026-10-06).**
- **Body.** KFPawn.Setup -> SpeciesType.Setup: `LinkMesh(rec.Mesh)` and
  SetTeamSkin: `Skins[0] = rec.BodySkin`, `Skins[1] = rec.FaceSkin`
  (Corporal_Lewis: `KF_Soldier_Trip.British_Soldier1`,
  `KF_Soldier_Trip_T.Uniforms.brit_soldier_I_cmb`,
  `KFCharacters.GasMaskShader`). Team skins exist only for
  Sergeant_Powers (not done: we use BodySkin). Pawn values: KFPawn
  PrePivot (0,0,0), DrawScale 1, KFHumanPawn CollisionHeight 50 (the
  actor's Location is the cylinder centre).
- **Animation sets come from the weapon.** Inventory.AttachToPawn spawns
  the weapon's `AttachmentClass` and attaches it to the bone
  `KFPawn.GetWeaponBoneFor` = `WeaponR_Bone`. KFPawn.SetWeaponAttachment
  then copies the attachment's names into the pawn: MovementAnims[0-3]
  (forward, back, left, right; KF's are the `Jog*_<weapon>` runs),
  TurnLeft/RightAnim, Crouch*, Walk*, Air/Takeoff/Land/Dodge, idles,
  FireAnims / FireAltAnims / FireCrouch* [4], HitAnims[4],
  PostFireBlendStand/CrouchAnim. The attachment mesh is its `Mesh`
  default, or `MeshRef` (KFWeaponAttachment.PreloadAssets);
  KFWeaponAttachment DrawScale 1; no relative offset.
- **Firing** (KFWeaponAttachment.ThirdPersonEffects -> KFPawn.StartFiringX):
  FireAnims[Rand(4)] (FireAltAnims for mode 1) on channel 1, alpha 1,
  from FireRootBone (`CHR_Spine1`): PlayAnim once, or LoopAnim while
  firing if the attachment's bRapidFire (bAltRapidFire). AnimEnd(1):
  after a single shot, PostFireBlendStandAnim (tween 0.1), then
  AnimBlendToAlpha(1, 0, 0.12).
- **Reload.** KFWeapon.ReloadMeNow: `Instigator.SetAnimAction(
  WeaponReloadAnim)` (a weapon class default): channel 1 from
  FireRootBone, tween 0.1, FireState Ready -> blends out at its end.
- **Weapon change.** Pawn.ChangedWeapon -> PlayWeaponSwitch ->
  SetAnimAction('Weapon_Switch'): channel 1, blended out after its length
  + 0.1 s (KFPawn.SetAnimAction).
- **Hits.** Pawn.PlayHit -> xPawn.PlayTakeHit -> KFPawn.PlayDirectionalHit:
  the direction to the hit location in the pawn's axes; dot X > 0.7
  HitAnims[0], < -0.7 HitAnims[1], dot Y > 0 HitAnims[3], else [2];
  PlayAnim tween 0.1.
- **Death.** KF uses a Karma ragdoll (rec.Ragdoll, e.g.
  `British_Soldier1`); xPawn.PlayDyingAnimation's fallback without one
  is PlayDirectionalDeath: DeathB / DeathF / DeathL / DeathR by the
  velocity or hit direction, tween 0.2.
- **Movement, idle, turning, falling, aim pitch: native code** (Pawn's
  bPhysicsAnimUpdate; "channels 2 through 11 are used for animation
  updating"). Only the inputs are in the scripts: MovementAnims[4],
  BlendChangeTime 0.25 (xPawn), TurnLeft/RightAnim "scaled by turn
  speed", AirAnims / TakeoffAnims / LandAnims [Get4WayDirection],
  AirStillAnim, TakeoffStillAnim, IdleWeaponAnim, bDoTorsoTwist and
  `SetTwistLook(0, 256 x ViewPitch)` with SpineBone1 / SpineBone2
  (`CHR_Spine2`, `CHR_Spine3`).
- **Behind view.** KFGameType.CheckEndGame: ClientSetBehindView(true)
  for every player. PlayerController.CalcBehindView: the view rotation,
  the pawn's Location + 12 up, CameraDist (9) x CollisionRadius (20) =
  180 units back, shortened by a 10-unit box trace. The `BehindView` /
  `ToggleBehindView` commands work in standalone games; KF's User.ini
  binds `F4=ToggleBehindView`.

**What we do.**
- `src/player/body/`: the body is a component on the pawn, not on the
  camera. `PawnState` (on the pawn entity) holds what KF replicates to
  draw a pawn: Location, Velocity, on the ground, view yaw and pitch
  (Rotation, ViewPitch), the weapon class (the attachment), the shot
  counter and firing mode (FlashCount / FiringMode), reload and weapon
  switch counters (AnimAction), the last hit location and count
  (TakeHitLocation), dead. `PawnBody` holds the animation state and the
  drawn entities. The body systems read only `PawnState`; for the local
  player, small "feed" systems copy the walker, the camera's look
  angles, the weapon state and the hits into it. Another player's pawn
  in a network game (branch multiplayer-lightyear, `src/net/pawns.rs`)
  is a separate entity whose `PawnState` (local: false) is filled from
  the network, with a `PawnCharacter` naming its character.
  `BodyModels` holds every character loaded so far (the local player's
  is `local`; others are loaded the first time a pawn asks for them).
  Today the local pawn is the camera entity (it carries the `Walker`);
  the body's drawn root is a separate entity placed at the pawn's
  Location with the pawn's yaw.
- Reuses `SkinnedModel` (render/skinned.rs) with new functions to sample
  a sequence's bone locals, blend locals (whole body or from a bone), and
  pose from locals with extra bone turns (the aim pitch).
- **Owner no-see.** The body and its weapon are hidden while the pawn is
  the local player's own first-person view; shown in behind view and
  when the view is on a zed.
- **Behind view.** view_target.rs puts the camera behind the player's
  pawn by CalcBehindView's rule (now, not only for zeds). Turned on by
  the end of the match (as before), by F4 (KF's binding), by the test
  flag `--behind-view` and the test action `behind_view` (toggle).

Our guesses for the native parts (labelled in the code):
- Movement: the four MovementAnims blended by the velocity's direction
  in the pawn's axes (weights = the positive parts of forward / right,
  normalised), all at one shared phase; play rate = speed / 200
  (KFHumanPawn GroundSpeed), clamped 0.4 to 1.5; idle below 10 units/s;
  switching between idle / moving / air crossfades over BlendChangeTime
  0.25 s.
- Turning in place: TurnLeft/RightAnim while standing and the yaw
  changes faster than 4000 units/s (about 22 degrees/s); no torso twist
  (the body faces the view yaw).
- Aim pitch: the view pitch shared equally by SpineBone1 and SpineBone2,
  turning around the pawn's right axis; clamped to +-60 degrees.
- Jump and fall: TakeoffAnims[dir] (TakeoffStillAnim under 50 units/s)
  once when leaving the ground going up, then AirAnims[dir] /
  AirStillAnim; on landing LandAnims[dir] once (cut short by moving).
- Hit reactions only when standing, over the whole body. KF plays them
  on channel 0, under the native movement channels; we assume those
  cover them while moving (not in the scripts).
- Channel 1 has no tween in (anims start at full weight). A looping fire
  animation's loop end counts as AnimEnd only once firing has stopped
  (otherwise sustained fire would fade out each loop).
- Rand(4) for the fire animation uses our own fixed-seed random numbers.

Steps:
- TP1 (done): the body (record mesh and skins), idle / 4-way run, the
  weapon attachment in the right hand, owner no-see, behind view (end
  screen, F4, `--behind-view`).
- TP2 (done): firing, reloading, weapon switch, hit reactions.
- TP3 (done): jump / fall / land, turning in place, aim pitch.
- TP4 (tried, switched off): the death ragdoll. The zeds' ragdoll code
  is reused (`zeds::ragdoll::spawn` with rec.Ragdoll `British_Soldier1`
  from KF_Characters_Trip.ka, which loads: 16 parts, 15 joints). It does
  not settle: in two attempts the body kept shaking and sliding (about
  300 units in 5 s, parts at 1000-3000 units/s, joints pulled 20-30
  units apart). Cause found, not fixed: at death the joints are already
  past their limits (arm collars 19-23 degrees against a 6 degree cone,
  right upper arm 50 against 45), and still 15-18 degrees past with the
  collars put back to the mesh's reference pose. This ragdoll's joint
  axes also disagree with the bones by up to 30 degrees at load (the
  Clot's: 0), so it may have been made for another skeleton. Switched
  off by `PLAYER_RAGDOLL` (animate.rs). The soldier meshes have no
  DeathF/B/L/R (xPawn's fallback), so a dead body is hidden.
- Not done: crouch (we have no crouching), walking (no walk key),
  dual pistols' second gun (DualiesAttachment), the grenade throw anims
  (KFPawn.HandleNadeThrowAnim's Frag_<weapon>), the attachment's own
  effects (muzzle flash, shells, tracers from the third-person gun), team
  skins (Sergeant_Powers), the torso twist, foot placement
  (SPECIES_KFMaleHuman FeetBones), the attached emitter
  (rec.AttachedEmitter), the face skin (the soldier meshes have one
  material slot, so Skins[1] is unused), animations for other pawns
  (in network games other players' pawns have one: src/net/pawns.rs).

**At the end of the match.** Lost: the view goes behind where the
player died; the body is hidden (no ragdoll, see TP4). Won: KF's view is
still on the dead Patriarch (ZombieBoss.Died sets it, ClientSetBehindView
does not change the view target), so the player's own body is seen only
if it is in that view, as in KF.

Test tools: `--behind-view`, `--behind-yaw DEG` (KF's free camera,
CameraDeltaRotation.Yaw, "for checking out player models and
animations": 180 = from the front), test actions `behind_view`
(toggle) and `turn:DEG` (turns the view DEG degrees over 1 s).

Logs: `body_record` (record mesh and skins found), `body_loaded` (mesh,
bones, scale, bones used, sequences), `body_anims_missing` (names the
data asks for that the mesh lacks), `body_attachment` (per weapon class:
attachment class, mesh, anim set), `body_attachment_held`, `body_anim`
(each base or channel 1 change, with the reason), `body_state` (once a
second: location, speed, base anim and weights, channel 1, pitch, turn
rate, visible, hand and attachment bounds in actor space),
`body_visible`, `behind_view` (on / off and why), `behind_view_camera`
(once a second: distance), `scripted_turn`; with the ragdoll on:
`body_ragdoll_loaded`, `body_ragdoll_started`,
`body_ragdoll_limits_at_death`, `body_ragdoll` (every 0.5 s).

## Weapon flashlights (FL1 implemented 2026-10-06)

**What KF does (scripts and class defaults, checked 2026-10-06):**

- **Keys.** User.ini: `F=ToggleFlashlight`, `MiddleMouse=AltFire`.
  KFPawn.ToggleFlashlight: if the weapon in hand has `bTorchEnabled`,
  `Weapon.ClientStartFire(1)` (its alt fire). Otherwise it looks for a
  torch weapon in the inventory, in this order: Shotgun (and subclasses),
  BenelliShotgun, Dualies (not DualDeagle, Dual44Magnum, DualMK23Pistol,
  DualFlareRevolver), Single; switches to it with `bPendingFlashlight`,
  and when it is up (KFWeapon.Timer, end of BringUp) calls LightFire.
  Without any, nothing happens.
- **Torch weapons** (`bTorchEnabled = true`, own or inherited): Single
  (9mm), Dualies, Shotgun (and CamoShotgun), BenelliShotgun (and Golden),
  NailGun; DualiesDM (deathmatch only). Not the Bullpup or any other.
  FirstPersonFlashlightOffset: Single and NailGun (-20, -22, 8), Dualies
  (-15, 0, 5), Shotgun and Benelli (-25, -18, 8). Their alt fire is the
  light: SingleALTFire (9mm, Dualies), ShotgunLightFire (Shotgun,
  Benelli), NailGunALTFire. All three: ModeDoFire calls ToggleTorch, then
  the normal fire (FireAnim 'LightOn', FireSound KF_9MMSnd.Ninemm_AltFire1;
  the NailGun's FireSoundRef KF_NailShotgun.Vlad9000_Light_On), FireRate
  0.5 (WeaponFire default, so holding the button toggles every 0.5 s),
  AllowFire: not reloading, not throwing a grenade.
- **The toggle** (KFWeapon.ServerSpawnLight), only if FireMode[0] is not
  firing and not reloading: with no FlashLight actor yet, and battery
  >= 1 and alive: spawn Effect_TacLightProjector, play
  NineMM_AltFire1 (SLOT_Misc), PlayAnim(ModeSwitchAnim) (Single:
  'LightOn'; the others have none), light on. With one: flip it.
- **Off by itself** (KFWeapon.WeaponTick): dead, battery <= 0, or a weapon
  switch starting (PendingWeapon). OwnerEvent('ChangedWeapon') also turns
  it off, and the projector destroys itself when the pawn's weapon is no
  longer its weapon. So every weapon has its own light; switching away
  turns it off.
- **Battery** (KFHumanPawn.TorchBatteryLife, 500, one per player; the
  pawn's Timer every 1.5 s): light on: -10 (so 75 s from full); otherwise
  +20 up to 500 (37.5 s from empty). HUD: FlashlightDigits =
  100 x battery / 500, FlashlightIcon when on, FlashlightOffIcon when off,
  shown only with a torch weapon in hand.
- **The light** (Effect_TacLightProjector.Tick, every frame):
  - Start, first person: W + 0.2 x (LightBone - W) + the offset along the
    LightBone's X, Y, Z axes (W = the first-person weapon's location).
    Direction: the LightBone's X axis (so it sways with the weapon).
    Third person (other players, behind view): the WeaponAttachment's
    location and its 'FlashLight' bone's X axis (else the eye and the
    view rotation).
  - Trace 1800 units (actors too). BeamLength = distance to the hit.
  - The projector (a DynamicProjector): ProjTexture
    KillingFloorWeapons.Dualies.LightCircle (grey ring, brightest at 0.3 of
    the radius, half at 0.55, none at the edge), MaterialBlendingOp
    PB_Modulate, FrameBufferBlendingOp PB_Add (adds texture x circle),
    bGradient (fades with depth), MaxTraceDistance 1600, bClipBSP,
    bProjectOnUnlit, bNoProjectOnOwner, DrawScale 0.1. FOV = Lerp(Beam /
    1800, 30, 50) degrees. Pulled back so it never starts inside a wall
    (ProjectorPullbackDist 25).
  - The glow (Effect_TacLightGlow, a Light: LT_Steady, LightBrightness
    100, LightRadius 3, LightHue 0, LightSaturation 255 = white), at the
    hit point (50 units back on terrain). Beam <= 100: brightness 100,
    radius Lerp(Beam / 100, 0, 3.75); else brightness 100 x (1 - Beam /
    1800), radius Min(12, Lerp(Beam / 900, 3, 12)). (UE2 radius units:
    25 x (radius + 1) world units, so 100 to 325.)
- **First-person look**: TacLightShineAttachment (skeletal mesh
  KFWeaponModels.TacShine, DrawScale 0.15, unlit) on the weapon's
  LightBone, shown while the light is on (KFWeapon.AdjustLightGraphic).
- **Third-person look** (seen by others): on the attachment's
  'FlashLight' bone the same TacShine, stretched along Y by Beam / 90
  (0.02 to 1), and KFTacLightCorona (sprite FlashLightCorona3P, STY_Alpha,
  DrawScale 0.3; its dynamic light has brightness 0).

**What our renderer can light (checked in the code 2026-10-06):**

- BSP: lit Bevy material plus its lightmap (`lighting.rs`); the sun and
  ambient are told to skip lightmapped meshes, but Bevy spot and point
  lights do light them (`affects_lightmapped_mesh_diffuse`). Yes.
- Zeds, gore, terrain and placed meshes without baked colours: lit
  materials (sun + ambient). Yes.
- **Placed meshes with baked colours (most props: 1595 of 1669 on
  KF-WestLondon): drawn unlit** (texture x colour x K). A Bevy light
  cannot add to an unlit material, so **the flashlight does not light
  them** in this step. Fixing it needs our own material for those meshes
  (baked colour plus Bevy's dynamic lights): a renderer change for every
  map, so a separate step (FL2), to be checked on all 34 maps.
- The first-person weapon is on its own layer and unlit: not lit (KF:
  bNoProjectOnOwner).

**How we do it (FL1):**

- **The light belongs to whoever holds the weapon.** A `Flashlight`
  component (src/weapons/flashlight.rs) on the holder's entity carries
  the battery, on/off, the weapon whose light it is, and the beam start
  and direction for this frame. A generic system turns every holder's
  component into lights in the world (trace, KF's per-frame values, the
  Bevy lights), so the same code would show another player's light.
  Filling the beam start is the holder's side: for us, the first-person
  weapon's LightBone (src/weapons/weapon/torch.rs). A remote player would
  fill it from their third-person attachment's 'FlashLight' bone, as KF
  does. No networking is written (CLAUDE.md: offline only); this only
  keeps the door open. Today only the local player (the camera entity)
  has the component.
- The projector becomes a Bevy SpotLight from the beam start: outer angle
  FOV / 2 (KF's FOV lerp), inner angle 0.45 x outer (**guess**: fits the
  LightCircle's ring roughly; Bevy has spot textures only behind an
  optional renderer feature, so the ring itself is not drawn), shadows on
  (stands in for bClipBSP: no light through walls), range 1800 units.
- The glow becomes a Bevy PointLight near the hit point with KF's
  per-frame radius and brightness.
- **Brightness (guesses, to compare with the real game):** UE2 adds light
  in screen (gamma) space; Bevy in linear. At the hit point we aim for:
  projector texture x 0.58 (the circle's average where it is bright) x
  (1 - depth / 1600) (**guess**: GRADIENT_Fade taken as linear over
  MaxTraceDistance); glow texture x LightBrightness / 255 x K (K = 2, the
  overbright of `lighting.rs`). Each is turned into linear light
  (^2.2) and into Bevy lumens for the distance to the hit, so the spot
  on the wall gets that much whatever the distance (Bevy's own falloff
  then applies to things in front of or around it). The glow sits half
  its radius in front of the wall (**guess**: Bevy's inverse-square light
  would be infinitely bright exactly on the wall).
- The trace: world geometry and doors (`world_filter`) and the zeds'
  collision cylinders (KF's trace stops at actors too).
- First person: the TacShine mesh on the LightBone, unlit, on the weapon
  layer, shown while on. Its material (Shader berettaLightSHader,
  OB_Brighten over BeretaTacLightStream) is almost black in the data
  (brightest texel 11 of 255), so it is barely visible, in KF too as far
  as the data says.
- Battery, HUD box, sounds, animations as KF above. F is a single press
  (KF's exec starts the alt fire without a stop; we fire it once).
- Not done in FL1: lighting baked props (FL2), the third-person look
  (no other players), the LightCircle ring pattern, the projector's
  pull-back from walls (ProjectorPullbackDist: our spot starts at the
  beam start, Bevy shadows keep it from shining through), the "TERRAIN:
  glow 50 units back" rule (the glow always stands off). Brightness in
  already-lit places is lower than UE2's gamma-space add (we add in
  linear light): the light shows best in dark places, as in KF.
- **FL2, baked props (done 2026-10-08, after your play test:
  "flashlights don't work against tunnel walls and doors").** Not a
  regression of actor lighting: placed meshes with baked colours (the
  tunnel shells, doors, most props) were drawn unlit since L2, so no
  Bevy light could reach them; the flashlight lit only the BSP. Now
  they use `render/baked.rs`: Bevy's lit StandardMaterial with a small
  extra fragment shader that adds the baked colour (texture x colour x
  K, exactly as before) to Bevy's own lighting of the plain texture; a
  black 1 x 1 `Lightmap` on each such mesh keeps the sun and ambient
  light off it (as for the BSP), so only point and spot lights (the
  flashlight) add. That material on every baked mesh cost about 5 ms
  a frame (KF-WestLondon, headless), so each baked mesh keeps the old
  unlit material and switches to the lit one (with the lightmap) only
  while the flashlight's spot or glow can reach it (bounding spheres
  touch; log `baked_swap`, about 240 to 390 meshes with the light on).
  Glass panes and sky-zone meshes keep the old unlit material. Measured
  with the light off: unchanged from before (mean difference 0.00,
  KF-WestLondon street view). `KF_BAKED_UNLIT=1` turns FL2 off (for
  comparing).

Logs: `weapon_torch` (per torch weapon at load: offset, LightBone found,
switch anim), `flashlight_toggle`, `flashlight_refused`, `flashlight_off`,
`flashlight_battery`, `flashlight_beam` (once a second while on: start,
direction, what was hit, distance, FOV, light values), `flashlight_key`,
`flashlight_pending`, `flashlight_shine` / `flashlight_shine_shown`.

## Perks (veterancy) (stage 1 implemented 2026-10-06)

KF's perks are the seven `KFVeterancyTypes` subclasses (KFMod): Field
Medic, Support Specialist, Sharpshooter, Commando, Berserker, Firebug,
Demolitions (PerkIndex 0-6, in that order), each with a level 0-6
(`KFPlayerReplicationInfo.ClientVeteranSkill` and
`ClientVeteranSkillLevel`). Every perk effect is a static function of the
perk class that the game calls from many places; the base class returns
"no change" (1.0, 0, the same damage). With no perk at all
(ClientVeteranSkill none) every caller skips the call.

**Stages (your decision).**
- Stage 1 (now): you pick the perk and the level; no progress tracking.
- Stage 2 (later, not built): levels earned from stats saved locally.
  Plan at the end of this section.

### Choosing the perk (stage 1)

- `--perk NAME`: `medic`, `support`, `sharpshooter`, `commando`,
  `berserker`, `firebug`, `demolitions` (also `demo`, or KF's class name
  such as `KFVetSupportSpec`). Without it you have no perk, as before.
- `--perk-level N`: 0 to 6 (default 0). In KF each perk has its own level
  from your Steam stats (`PerkHighestLevelAvailable`); here the one level
  is used for whichever perk you pick.
- In game: the buy menu (at the trader) lists the perks; keys 1-7 pick
  one (KF's quick perk select, `KFQuickPerkSelect`, sits in the buy menu).
  Test action `perk:NAME` does the same from `--input`.
- KF's rule for when a change counts (`KFPlayerController.SelectVeterancy`,
  `KFGameType.DoWaveEnd`):
  - During a wave: nothing changes now; the pick is remembered and
    "You will become a 'X' at the end of this Wave" is shown. At the end
    of the wave the remembered perk is applied.
  - Between waves: the change is immediate ("You are now a 'X'"), but
    only once per wave (after the match has begun): a second change gets
    "You can only change your Perk once per Wave". The allowance comes back
    at the end of the next wave.
  - A change calls `KFHumanPawn.VeterancyChanged`: carry weight is
    recomputed (weapons over the new limit are dropped; we have no
    weapon pickups on the floor, so they are just removed, logged) and
    ammo over the new maximum is cut.
  - Starting items (`AddDefaultInventory`) come only with a new pawn
    (game start; we have no respawn), so changing perk later gives none.
- The buy menu opens on your perk's sale list (`BuyMenuFilterIndex`).
- HUD (`HUDKillingFloor.DrawHudPassA`): the perk icon bottom left
  (`OnHUDIcon`; level 6 uses `OnHUDGoldIcon`, (255,255,255,192)), size
  Min(36 x 1.5 x 1.4 x SizeX/1024, 36 x 1.5 x 1.4) at (0.007 ClipX,
  0.93 ClipY - size), with one star per level (`Hud_Perk_Star`, level 6:
  one gold star, `Hud_Perk_Star_Gold`), VetStarSize 12 scaled the same
  way, stacked upwards from the icon's bottom-right corner.
- Logs: `perk_selected` at startup (perk, level, source), `perk_change`
  for every request (accepted, deferred or refused, and why), and a
  `perk_mod` line whenever a modifier changes a value (what, weapon or
  damage type, the factor, before and after).

### Every perk effect, where KF applies it, and our status

"Ours" says whether the system the effect changes exists in our code
(yes / partly / no) and what stage 1 does (done = implemented and
checked from logs, LATER = left).

| Effect (static function) | Perks using it | KF calls it from | Ours | Stage 1 |
| --- | --- | --- | --- | --- |
| AddDamage (damage you deal) | all but Medic, Sharpshooter | KFMonster.TakeDamage (after the burn bookkeeping, before the headshot multiplier) | yes (combat::damage_zed) | done: every hit carries its damage type to damage_zed |
| GetHeadShotDamMulti | Sharpshooter (all perks 1.0) | KFMonster.TakeDamage, headshots and headless zeds, not melee (DamTypeMelee) or fire | yes | done |
| GetReloadSpeedModifier | Commando (all weapons), Sharpshooter, Firebug | KFWeapon.ReloadMeNow (ReloadRate = default / mod), ClientReload (anim rate x mod), WeaponTick | yes | done |
| GetFireSpeedMod | Sharpshooter (Winchester, Crossbow, M99), Berserker (KFMeleeGun: knives, axes, also the Welder) | KFFire / KFShotgunFire / KFMeleeFire.ModeDoFire (FireRate = default / mod, FireAnimRate x mod; melee also DamagedelayMin / mod) and the overrides that copy it (ChainsawFire, KSGFire, NailGunFire, WinchesterFire) | yes | done |
| ModifyRecoilSpread | Sharpshooter, Commando | KFFire / KFShotgunFire.ModeDoFire (Spread x mod, HandleRecoil kick x mod) and BoomStickAltFire, HuskGunFire, KSGFire, NailGunFire, WinchesterFire | yes (firing.rs) | done |
| GetMagCapacityMod | Medic (medic guns), Commando (rifles), Firebug (Flamethrower, MAC10) | KFWeapon.UpdateMagCapacity (every tick), GiveAmmo (start ammo x new / default capacity), ServerBuyAmmo (rounds per clip), the buy menu's clip price | yes | done |
| AddExtraAmmoFor (max ammo) | Support (shotguns, frags), Commando, Firebug, Demolitions (frags, pipe bombs, LAW) | KFWeapon.GiveAmmo / GetAmmoMulti, KFPawn.ServerBuyAmmo, KFAmmunition.HandlePickupQuery, Huskgun.GiveAmmo, VeterancyChanged | yes | done |
| GetAmmoPickupMod | Medic, Commando, Firebug | KFAmmoPickup.Touch (ammo boxes in the map) | yes (weapons/weapon/pickup.rs) | done |
| GetCostScaling (weapon and armour prices) | all | KFPawn.ServerBuyWeapon, ServerSellWeapon (unpaid weapons), ServerBuyKevlar; KFBuyMenuSaleList / InvList prices | yes (buy menu, armour) | done |
| GetAmmoCostScaling | Sharpshooter (bolts), Demolitions | KFPawn.ServerBuyAmmo, KFBuyMenuInvList | yes | done |
| AddDefaultInventory (start weapons, armour) | all (level 5-6; armour: Medic 5+, Berserker 6 below Suicidal, Firebug 6) | KFHumanPawn.AddDefaultInventory -> CreateInventoryVeterancy (SellValue = StartingWeaponSellPriceLevel5 200 / Level6 225; Demolitions' Level5 is 0) | yes | done |
| GetMovementSpeedModifier | Medic | KFHumanPawn.ModifyVelocity (GroundSpeed x mod after the weight and melee bonus) | yes (walk.rs) | done |
| GetMeleeMovementSpeedModifier | Berserker | KFHumanPawn.ChangedWeapon: InventorySpeedModifier = GroundSpeed x (BaseMeleeIncrease 0.2 + mod) - Weight x 2 | yes | done |
| AddCarryMaxWeight | Support | KFHumanPawn.VeterancyChanged (MaxCarryWeight 15 + n) | yes | done |
| GetWeldSpeedModifier | Support | WeldFire.Timer: weld damage x mod (an int) | yes (door.rs) | done (unit-tested; not tried in a run: no door found for the test) |
| GetSyringeChargeRate | Medic | Syringe.Tick / KFMedicGun.Tick: +10 x mod per regen tick | yes | done |
| GetHealPotency | Medic | SyringeAltFire.Timer (self heal), SyringeFire (others), HealingProjectile (darts), MedicNade | yes (game/healing.rs, net/heals.rs) | done (2026-10-07: teammates over the network) |
| ReduceDamage (damage you take) | Medic (bile), Berserker (all, bile more), Firebug (fire), Demolitions (explosives) | KFGameType.ReduceDamage (before the self-damage halving); KFPawn.TakeDamage returns early when it gives 0 (no burning) | yes (combat.rs) | done |
| GetBodyArmorDamageModifier | Medic | KFPawn.ShieldAbsorb (damage x mod before the vest's sums) | yes (armour.rs) | done |
| ZedTimeExtensions | Commando (3+), Berserker | KFGameType.Killed: a kill during zed time forces DramaticEvent(1.0) while extensions are left; reset when zed time ends | yes (zed_time.rs) | done |
| CanBeGrabbed | Berserker (not by Clots) | ZombieClot.ClawDamageTarget | yes (think.rs) | done |
| GetShotgunPenetrationDamageMulti | Support | ShotgunBullet / NailGunProjectile / TrenchgunBullet.ProcessTouch | yes (projectile.rs PenDamageReduction) | done |
| GetMAC10DamageType | Firebug (DamTypeMAC10MPInc: incendiary) | MAC10Fire.DoTrace | yes (FireType::Mac10 exists) | done |
| ExtraRange | Firebug (Flamethrower) | FlameTendril.Timer (bursts after 2 + n timers) | yes (projectile.rs flames) | done |
| GetNadeType | Medic (MedicNade), Firebug 3+ (FlameNade) | FragFire.GetDesiredProjectileClass | yes (load.rs `perk_nade`, projectile.rs `medic_pulse`) | done 2026-10-07 |
| SpecialHUDInfo (zed health bars) | Commando 1+ | HUDKillingFloor.DrawHudPassA -> DrawHealthBar | yes (hud.rs `zed_health_bars`) | done 2026-10-07 |
| ShowStalkers / GetStalkerViewDistanceMulti | Commando | ZombieStalker.Tick, ZombieBoss.Tick (the red "spotted" glow) | yes | done (see "Commando: seeing cloaked zeds") |
| CanMeleeStun | Berserker | no caller in the base game scripts | - | nothing to do |
| ShouldBecomeIncendiary, KilledShouldExplode | none (base false) | KFMonster.TakeDamage | - | nothing to do |

Weapon and damage-type matching: KF tests classes two ways, and we
copy which one each line uses: `Item == class'X'` (that exact class)
and `X(Other) != none` / `class<X>(DmgType) != none` (X or a subclass).
For the subclass tests we keep each weapon's, damage type's and ammo's
class chain (read with ClassDefaults at load); Berserker reads the damage
type's `bIsMeleeDamage` default.

Difficulty: the game's GameDifficulty (`--difficulty`, see
"Difficulty"; default Normal 2) picks the Suicidal / Hell on Earth
branches of Medic speed and Berserker armour. The Sharpshooter's Dualies
rule is still the below-7 side (D2).

Integers: KF keeps damage, weld damage, heal amounts and ammo maximums as
whole numbers; a perk factor's result is cut down to a whole number where
KF's variable is an int.

The full per-perk audit (every effect, formula by level, status, file)
is `docs/perks.md` (2026-10-07).

### Healing other players and network perks (2026-10-07)

- `game/healing.rs`: `Teammates` (the other players: position, health,
  name; filled by `net/heals.rs` from their `NetPawn` / `NetPlayer`,
  empty solo). The healer's game finds who is healed (SyringeFire's
  GetHealee: within 80 units in front; a dart touching their cylinder;
  the MedicNade's 175-unit cloud), pays itself (ReceiveRewardForHealing:
  int(healed / 100 x 60)) and writes `HealTeammate`; `net/heals.rs`
  sends it healer -> host -> the healed player's game
  (`HealRequest`, `PlayerEvent::Healed`), where it becomes GiveHealth.
- Whose perk: hits carry the shooter's perk to the host; burn ticks keep
  the igniter's (`Zed.burn_vet`, KF's BurnInstigator); hits the host
  sends to a player carry their damage type (`PlayerEvent::Hurt.dam`) so
  that player's own ReduceDamage applies; the Commando's glow and health
  bars are decided on each game for its own player (puppet zeds run the
  spotted check; the host sends a spotted Stalker as cloaked).

### How it is built (stage 1)

- `game/perks.rs`: the perks, `Vet` (perk + level) with one method per
  KF static function (unit-tested against the scripts' numbers), the
  `Veterancy` resource (in use, asked for, changed this wave), the change
  rule and its log.
- Class tests: weapons, ammo and damage types keep their class chain
  (`ClassDefaults::chain_names`); `ClassChain::is` / `is_a` copy KF's
  `==` / IsA tests. Damage types are interned (`DamType`) and carried by
  every hit: `ShotFired`, `MeleeSwing`, projectile stats, blasts, flames,
  burn ticks (`HitSource.dam` and `.vet`) and `PlayerDamaged.dam`.
- Weapons (`weapons/weapon/perk.rs`): each weapon keeps its defaults
  (FireRate, FireAnimRate, DamagedelayMin, ReloadRate, ReloadAnimRate,
  MagCapacity, MaxAmmo) and `apply_vet` derives the perk's values when a
  weapon is given or the perk changes (`sync_perk`). Which fire classes
  take the fire speed and recoil factors comes from reading each
  ModeDoFire override in the scripts (`perk_fire_rules`).
- KF quirks kept: the Sharpshooter's level 0 gets the 0.25 recoil branch
  (the script's else); prices use 32-bit floats (Support 6 shotgun:
  500 x 0.29999998 = 149.99997, shown as 149); the wave-end perk change
  uses up the trader-time change (SelectVeterancy sets
  bChangedVeterancyThisWave again).
- Our stand-ins (labelled): ClientMessage texts are shown in the
  KFCriticalEventPlus style (we have no console message area); weapons
  dropped by a carry-limit change are removed (no floor pickups yet); the
  normal (not gold) HUD icon colour is not set by KF's code there: white
  is a guess.
- Known differences: with a perk, damage taken is cut to a whole number
  first (KF's int; without a perk the old code keeps fractions, e.g.
  3.85 from a Clot); a respawn after death (debug mode) does not give
  the perk's armour again; the player starts holding the 9mm (KF calls
  ClientSwitchToBestWeapon, not checked).

### Commando: seeing cloaked zeds (built 2026-10-06)

In KF a Commando sees a cloaked Stalker (and the cloaked Patriarch) as a
red, see-through glow instead of the near-invisible cloak. KF calls a
zed in that state "spotted" (`KFMonster.bSpotted`).

**The rule, Stalker** (`ZombieStalker.Tick`, every 0.5 s, not while
zapped, only while she is alive): she is spotted when the viewing
player is alive, has a perk whose `ShowStalkers` is true (only
`KFVetCommando`) and is closer than
`sqrt(GetStalkerViewDistanceMulti x 640000)` units (640000 = 800
squared). Commando levels: 0 = 0.0625 (200 units), 1 = 0.25 (400),
2 = 0.36 (480), 3 = 0.49 (560), 4 = 0.64 (640), 5 and 6 = 1.0 (800).
No wall check: she is spotted through walls.
Check against KF's own perk text (`KFVetCommando` LevelEffects, also on
the user's pause-menu screenshot for level 0), at KF's 50 units per
metre (HUDKillingFloor: trader distance = units / 50): level 0 "4
meters" = 200 units, 1 "8m" = 400, 5 and 6 "16m" = 800 agree exactly;
levels 2-4 say 10m, 12m, 14m where the code gives 9.6, 11.2, 12.8 m (the
text is rounded up loosely). We follow the code.

Then, in the same tick (copied branch by branch):
- not spotted, not cloaked and wearing the glow: `UncloakStalker`
  (normal skin; restarts the 1.2 s timer);
- otherwise, 1.2 s after the last uncloak: spotted and not glowing ->
  `CloakStalker`, which, when spotted, only puts on the glow
  (`Skins[0] = Skins[1] = FinalBlend'KFX.StalkerGlow'`, `bUnlit`) and
  returns; it does not set `bCloaked`. Not spotted and not wearing the
  invisible skin -> `CloakStalker` the normal way (not when headless).
- KF quirks kept: the glow's branch comes before the "no head, no cloak"
  test, so a headless Stalker still glows for a Commando; an uncloaked
  Stalker (after attacking) glows 1.2 s later while still uncloaked.
- The glow is removed (normal skin) by what sets the normal skin in KF:
  an attack (`UncloakStalker`), losing the head (`RemoveHead`), a zap
  (`SetZappedBehavior`), death (`PlayDying`).

**The rule, Patriarch** (`ZombieBoss.Tick`, every 0.8 s while cloaked,
not zapped): spotted when a living player with `ShowStalkers` (any
Commando level) is within 1000 units and in sight
(`VisibleCollidingActors`: a line-of-sight trace). Spotted -> glow;
no longer spotted -> back to the cloak. Uncloaking clears it
(`UnCloakBoss` sets `bSpotted = false`).

**The look.** `KFX.StalkerGlow` is a FinalBlend (FrameBufferBlending 6
= FB_Brighten) over Shader `StalkerGlowShader`: Diffuse and
SelfIllumination = Combiner `StalkerGlowCombiner` (TexPanner
`GhostPanner` over the red texture `KFGhostOverlay`, alpha-blended with
`KFCharacters.StalkerSkin`), SelfIlluminationMask = texture
`CloakGradient`, Opacity = TexOscillator `DeCloakOSC` (CloakGradient
again), OutputBlending OB_Masked. We follow that chain in the package at
load time and draw: unlit, additive, colour = KFGhostOverlay x
CloakGradient's alpha (the self-illumination mask), which gives the faint
red body with brighter patches of the user's screenshot. Guesses
(native code we cannot read): FB_Brighten drawn as plain additive; the
combiner's mix with StalkerSkin left out (KFGhostOverlay has no alpha, so
the result is the red texture, matching the screenshot); the panner and
oscillator motion are not animated.

**Built for more players later.** Being spotted is a matter of what one
player sees: in KF each client checks its own player
(`LocalKFHumanPawn`). Our check takes a `CloakViewer` (position, alive,
perk) instead of reading the player directly, so another viewer can be
passed later; the per-zed `spotted` / glow state is the local view's.

**Not part of this:** the Commando's zed health bars
(`KFVetCommando.SpecialHUDInfo`, levels 1-6, 160 to 800 units) are a
separate HUD drawing path; next step.

### Stage 2 (plan only, not built): earning levels

- KF counts the stats in `KFSteamStatsAndAchievements` (ROEngine) and
  each perk's level needs (from `Requirements` in the perk classes):
  Medic: health healed on teammates (DamageHealedStat); Support: welding
  points (WeldingPointsStat) and shotgun damage (ShotgunDamageStat);
  Sharpshooter: headshot kills with its weapons (HeadshotKillsStat);
  Commando: Stalker kills with rifles (StalkerKillsStat) and rifle damage
  (BullpupDamageStat); Berserker: melee damage (MeleeDamageStat);
  Firebug: flame damage (FlameThrowerDamageStat); Demolitions: explosive
  damage (ExplosivesDamageStat).
- The counting is in script: the damage types' `AwardDamage` /
  `AwardKill` (KFWeaponDamageType subclasses), WeldFire (welding points),
  the heal paths (AddDamagedHealStats), headshot kills in KFMonster.
- The level thresholds are **not** in the scripts:
  `PerkHighestLevelAvailable` and `GetXProgressDetails` are native
  (compiled C++). Options: take them from the game's binaries, or from
  published tables (to be labelled as such). To be decided then.
- Our plan: a `PerkStats` resource with those counters, filled from the
  same events (damage_zed with its damage type, welding, healing, kills),
  saved to a small local file (e.g. `work/` or the user's config folder,
  never the game install) at wave end and on exit; the level of each perk
  is then the highest whose thresholds are met, replacing
  `--perk-level` (which would stay as an override for tests).
- With teammates gone (solo), healing teammates cannot be earned: KF
  has the same problem solo; decide then whether self heals count.

## Menus: the lobby, the perk page, the pause menu (planned and built 2026-10-06: M1-M4)

KF's pre-game lobby (`KFGui.LobbyMenu` with `LobbyFooter`), its
"Select Perk" page (`KFGui.KFProfilePage` holding `KFTab_Profile`) and
the in-game pause menu (`KFGui.KFInvasionLoginMenu` with
`KFTab_MidGamePerks`). Reference screenshots (untracked):
`references/lobby.png` (2497 x 1420), `references/perk_selection.png`
(2552 x 1427), `references/pause_menu.jpg` (2560 x 1440).

**Where the layout comes from.** Every GUI component of these pages is
a subobject in `System/KFGui.u` (e.g. `LobbyMenu.BGPerk`) with its own
`WinTop`, `WinLeft`, `WinWidth`, `WinHeight`, captions, colours and
textures; we read them at startup (log `menu_layout`). Values up to 1
are fractions of the parent (the screen, or the tab panel for
`KFTab_*`), as UE2's GUI does. Captions come from `KFGui.int`, perk
texts (`LevelEffects`, `Requirements`) from `KFMod.int`, the map's
title and description from `<map>.int` `[LevelSummary]` (else the
`.ucl` FallbackDesc: `KFMapStoryLabel.LoadStoryText` reads the map
record), character portraits from the `.upl` records, biographies from
`KFGui.int` `[DecoText]` (`KFTab_Profile.UpdateScroll`).

**Styles and fonts.** KF's style table (`KFGUIController`
DefaultStyleNames) maps a component's StyleName to a style class:
SquareButton = `KF_SquareButton`, FooterButton = `KF_FooterButton`,
TabButton = `KF_TabButton` (textures `KF_InterfaceArt_tex.Menu.Button`,
`button_Highlight`, `button_pressed`), Footer = `ROSTY_Footer`
(`Thin_border`), TextLabel = `KF_TextLabel` (200,200,200,200),
CheckBox = `ROSTY2CheckBox`, EditBox = `KF_EditBox` (`Innerborder`),
Header = `KF_Header` (`Tabdark`). Section boxes
(`GUISectionBackground`) draw `HeaderBase` =
`Med_border_SlightTransparent` (`AltSectionBackground`:
`Thin_border_SlightTransparent`). Fonts by name (GUI2K4.int):
UT2SmallFont = ROBtsrmVr7/8/10/12/14, UT2MenuFont = Vr8/10/12/14/16,
UT2DefaultFont = Vr10, picked by screen width; the perk lists use
`ROHUD.GetSmallMenuFont` (ROEngine.int MenuFontArrayNames: Vr18 at
1600+ wide, Vr14, 12, 9, 7).

**Native parts (not in the scripts), guessed and checked against the
screenshots:** `GUIFont.GetFont` (which size for which width: assumed
index 0-4 for widths under 640 / 800 / 1024 / 1280 / above);
`Canvas.DrawTileStretched` (assumed UE2's rule: the four corners at the
texture's own size, half the texture each, edges and middle stretched;
smaller boxes scale the corners down); `GUISectionBackground`'s caption
place; button auto-sizing (`ButtonFooter`, `KFTab_MidGamePerks`
autosize: fitted to the screenshots). Each is labelled in the code.

**Drawing.** The HUD's 2D canvas (hud.rs: textured quads in a pool of
Bevy UI image nodes, KF's bitmap fonts) is shared; the menus get their
own pool, drawn above the HUD. While the lobby or the perk page is open
the HUD and the first-person weapon are hidden (KF: the player has no
pawn yet). The pause menu draws over the HUD.

**One shared perk widget.** The perk list (`KFPerkSelectList.DrawPerk`:
icon box, name bar, "Lv N", progress bar; the selected row uses the
`_Highlighted` textures), "Perk Effects" (`LevelEffects[level]`, lines
split at `|`) and "Next Level Requirements" (`KFPerkProgressList`) are
one module used by both the perk page and the pause menu. Clicking a
row only highlights it; KF commits on SAVE (perk page) or "Select Perk"
(pause menu), both through `perks.rs` (`PerkRequest`) and its change
rules. Perk levels are chosen, not earned (perks stage 2 not built), and
KF's level thresholds are native: progress bars stay empty, the
requirement texts show "?" for the number (`%x` is the native
threshold) and "not tracked yet" where KF prints "14002/25000". The
number of requirement rows is taken as the number of `Requirements`
strings (native `GetPerkProgressDetailsCount`: a guess).

**Lobby flow (KF's solo flow).** The game opens in the lobby, the
player is not spawned, the wave timer does not run (KF only enters
MatchInProgress when everyone is ready). Ready
(`LobbyFooter.OnFooterClick`: SendSelectedVeterancyToServer, then
ServerRestartPlayer) closes the lobby and starts the match: the wave
timer starts ("game_start"), and the pawn gets the current perk's
starting items (`AddDefaultInventory`; if the perk changed in the lobby
the start items are swapped). In the lobby a perk change does not use up
the once-per-wave change (`bMatchHasBegun` is false). Select Perk opens
the perk page; SAVE (`KFTab_Profile.SaveSettings`) applies the perk and
the character and returns. Options is drawn but inert (no settings
page). Disconnect quits the game (KF returns to the main menu: we have
none). The chat line is drawn; typing does nothing (offline). The
player list is a list (`LobbyPlayers`) so more players can be added
later; solo: the local player, named from `--name`, else KF's
fresh-install default `[DefaultPlayer] Name=KFPlayer` (defuser.ini).
The wave circle shows "1/4" (WaveNumber + 1 / FinalWave). The video
panel (`LobbyMenu.DrawPerk` plays a random `Movies/MovieN.bik`): Bink
video cannot be played with what we have; the panel is drawn black.

**When the lobby opens (rule).** `--lobby` forces it, `--no-lobby`
skips it. Otherwise it opens only in `--mode waves` when none of
`--frames`, `--screenshot`, `--input` is given (test runs keep starting
straight in the game; plain `cargo run --release -- --mode waves`
opens the lobby). Debug mode (the default) never opens it by itself.

**Perk page.** "3D View" box: the character's model, drawn as KF's
`KFSpinnyWeap` (see "The 3D View and Change Character" below).
"Portrait" toggles 3D view / portrait. "Change Character" opens the
character select window (`KFModelSelect`); SAVE applies the character:
the first-person sleeves of every weapon are swapped at once
(character.rs), the third-person body is reloaded, and in a network
game the lobby sends the new character to the others. Biography from
`[DecoText]`.

**Pause menu.** Escape (KFPlayerController.ShowMidGameMenu) opens it;
in single player KF pauses the game (`SetPause(true)` when
`Level.NetMode == NM_StandAlone`; closing turns pause off). We pause
Bevy's virtual time (game time, physics, zeds, timers stop; log
`pause`). Escape while the buy menu is open closes the buy menu
instead (it is the top menu page in KF). Window title: the map name
(`SetTitle`, standalone: GetURLMap). Tabs, as `KFInvasionLoginMenu`
leaves them: Perks, Communication, Help. Perks tab: the shared perk
widget, "Select Perk" (a mid-wave pick waits for the wave end, as
perks.rs already does), Settings (inert: no settings page), Spectate
(inert: no spectating), Forfeit (KF: DISCONNECT, back to the main menu;
ours: quits, no main menu), Exit Game (KF asks "Are you sure?" in
KFQuitPage; ours quits at once). Communication and Help tabs: drawn,
contents not built.

**Test actions** (`--input FRAME:ACTION`): `lobby_ready`,
`lobby_select_perk`, `perk_pick:NAME` (highlight a row on the perk page
or the pause menu's Perks tab), `lobby_save`, `change_character` (=
`char_select_open`), `char_pick:NAME`, `char_select_ok`,
`char_select_cancel`, `char_scroll:ROWS`, `char_rotate:PIXELS`,
`toggle_portrait`, `pause_menu` (Escape), `pause_tab:NAME`,
`pause_select_perk`, `menu_dump` (logs every drawn box). Logs:
`menu_layout`, `menu_open` / `menu_close` (which page, why),
`lobby_ready`, `lobby_player`, `perk_page` (picked row, effects shown),
`character_change`, `pause`.

Steps: M1 shared canvas + GUI drawing + layout reading; M2 lobby screen
and its flow; M3 perk page; M4 pause menu; each checked by screenshot
against the reference and by logs.

**As built (code: `src/game/menus/`: `gui.rs` drawing and layout,
`perk_panel.rs` the shared perk widget, `lobby.rs`, `profile.rs`,
`pause.rs`, `mod.rs` state, input, pause).**
- The reference screenshots are crops of 2560 x 1440 frames (lobby: about
  52 px cut on the left, 11 on the right, about 14-20 at the top; perk
  page: about 4 px each side). Compared at `--window 2560x1440`, every
  box lands where the screenshot has it once the crop is allowed for.
- Fonts at 2560 wide: UT2MenuFont = ROBtsrmVr16, UT2SmallFont = Vr14,
  perk lists Vr18 (GetSmallMenuFont); sizes match the screenshots, so
  the GetFont guess holds at this width (other widths not compared).
- Fits (labelled in the code): the section caption 22 px in and centred
  in the 24-texel header strip; the story text 20 px in, 25 px down;
  footer buttons: longest caption + Padding x footer height, 5 px apart;
  the pause menu's tab buttons: caption + 44 px; its bottom buttons:
  caption "Server Browser" + 0.01 x width (ours about 15% wider than the
  screenshot's), 1.5 x the text height.
- The lobby's player names are drawn in the TextLabel colour (200 grey):
  the data says LabelColor (10,10,10,210) but the screenshot shows light
  text (a guess that the style wins).
- moCheckBox rows use StandardHeight (0.03 of the screen) from their
  top, which centres the names and check boxes on the bars as in the
  screenshot.
- Changing character during the game swaps the sleeve material of every
  loaded weapon at once (`weapons/weapon/sleeve.rs`, log
  `character_change`); KF would only do it for the next pawn.
- Ready gives the pawn the current perk's start items: the start items
  of the command-line perk are removed if the new perk does not give
  them, the new ones added with their SellValue, armour set (log
  `new_pawn_inventory`).
- Quirk found: `PackageSet::find_object("KFGui.LobbyMenu.X")` missed
  most subobjects; the menus load `System/KFGui.u` by its file instead
  (the name KFGui also fits `Textures/KFGui.utx`; cause not looked into).
- Not done: the movie (Bink); the Options /
  Settings pages; Spectate; KFQuitPage's "Are you sure?"; the
  Communication and Help tabs' contents; the window's close box; chat
  typing (offline); scrolling text boxes; the lobby countdown
  ("Game will auto-commence in"); keyboard focus / tabbing.

### The 3D View and Change Character (planned and built 2026-10-07)

**What KF does (KFTab_Profile, KFModelSelect, UT2k4ModelSelect,
SpinnyWeap; class defaults read with `kfpkg defaults`).**
- **The 3D View** (`KFTab_Profile`): a `KFSpinnyWeap` actor (SpinnyWeap:
  `bUnlit` true, so the model is drawn unlit; SpinRate 0, so no
  auto-rotation) with the record's mesh and skins (`Skins[0]` BodySkin,
  `Skins[1]` FaceSkin), `SetDrawScale(0.9)`, looping `Profile_idle`.
  Its rotation is Yaw 32768 + the player's rotation (it faces the
  camera). `InternalDraw` places it in front of the camera at
  `SpinnyDudeOffset.X + (ClipX / ClipY) x 120` (offset (120, 0, 0),
  ClipX / ClipY the screen's width / height: 333 units at 16:9) and
  draws it with `DrawActorClipped` into the `DropTarget` box at FOV
  `nfov` = 15. Dragging the mouse over the box turns it:
  `Yaw -= 256 x DeltaX`. "Portrait" (`Toggle3DView`) switches to the
  2D portrait and back.
- **Change Character** (`PickModel`) opens `KFGui.KFModelSelect`, a
  LockedFloatingWindow ("Select Character", WinLeft 0.125, WinTop 0.15,
  0.74 x 0.7 of the screen). Inside: `sb_Main` (AltSectionBackground at
  0.04, 0.075, 0.680742 x 0.555859, RightPadding 0.5) whose caption is
  the highlighted character's name, managing `CharList`
  (KFGUIVertImageListBox: 4 columns x 3 rows of portraits, CELL_FixedCount,
  borders 2 px, a vertical scroll bar); `i_bk` (changeme_texture at alpha
  128 with a drop shadow) where a second SpinnyWeap shows the
  highlighted character (offset (250, 1, -24), FOV 15, DrawScale 0.9,
  `Profile_idle`); OK and Cancel (AlignButtons: bottom right). The list
  is every record whose Menu does not contain "DUP" (all 56 have
  Menu="SP"); locked characters (Steam DLC, the native
  `CharacterAvailable`) cannot be checked offline: all are shown
  unlocked. Clicking a portrait highlights it and updates that window's
  model and caption at once; OK (`ModelSelectClosed`) sets the perk
  page's character (sChar): its 3D View, portrait and biography
  change; Cancel keeps the old one. SAVE applies it
  (`KFTab_Profile.SaveSettings`: ChangeCharacter).

**Plan.**
- `player/body/preview.rs`: preview slots (the perk page's, the model
  select's). Each slot: a render-to-texture camera on its own render
  layer (so the world does not draw it and it does not draw the world),
  cleared to transparent, sized to its box; the character's meshes
  (reusing `BodyModels` and its loading, unlit copies of the materials),
  posed each frame from `Profile_idle`, at KF's distance, offset, yaw and
  FOV. The menus fill a `CharacterPreview` request each frame (character,
  box size, yaw, offset, FOV); no request hides the slot.
- The menus draw the slot's image as a quad in the box (the HUD canvas
  takes it like any texture), over the section background.
- `game/menus/model_select.rs`: the window, the grid, OK / Cancel; new
  page `Page::ModelSelect` above the perk page; mouse wheel and the
  scroll bar scroll the grid one row at a time (`MyScrollBar.Step =
  NoVisibleCols`).
- Drag on the 3D View turns the model (KF's rule).
- Test actions: `char_select_open`, `char_pick:NAME`, `char_select_ok`,
  `char_select_cancel`, `char_rotate:PIXELS` (a drag), `char_scroll:ROWS`.
  Logs: `preview_character`, `preview_state` (every 5 s: the
  model's top / bottom / left / right in the box, pixels),
  `model_select` (opened / highlighted / closed).

**As built** (code: `src/player/body/preview.rs`,
`src/game/menus/model_select.rs`, `profile.rs`, `mod.rs`, `gui.rs`).
- Checked at 2560 x 1440 against `references/perk_selection.png`: the
  model's top lands 130 px into the 3D View box (screenshot: about 133),
  its feet at 1039 (screenshot: about 1009); size and place match by eye.
  The previews are unlit (SpinnyWeap bUnlit); the screenshot's shading
  comes from the textures.
- The model select's model is cut off at the thighs: that is what KF's
  numbers give (250 units away, FOV 15, 24 units below the view: about
  77 units of height fit in i_bk). Not compared with KF (no screenshot
  of it).
- i_bk draws `InterfaceArt_tex.Menu.changeme_texture` (a red box with a
  yellow border) at half alpha behind the model, as the data says; not
  compared with KF.
- The perk page stays drawn under the window (with its own model);
  under the window everything is darkened to 80/255 (PopupPageBase's
  fade, shown at once instead of over 0.35 s).
- Quirk: `ClassDefaults` cannot find the defaults of KFGui.u classes
  (it gathers property names from the script packages by name, and the
  name KFGui finds the texture package, as above). The menus read
  KFTab_Profile's `nfov` and `SpinnyDudeOffset` with KFGui.u's names
  added (gui.rs); the asset library is unchanged.
- Network: a client that picked Mr_Foster in the window and pressed SAVE
  sent `character=Mr_Foster`; the host drew it (`net_remote_pawn_spawned
  ... character=Mr_Foster`, `body_spawned character=Mr_Foster
  local=false`).
- Not done: locked (DLC) characters (all shown available); the window's
  close box, moving it, the Mr.Crow name sound; the scroll bar's grip
  cannot be dragged (wheel, arrow buttons and `char_scroll` scroll);
  double-click; the 0.35 s fade.

## Pickups: weapons, ammo boxes and vests lying in the map (planned and built 2026-10-07)

Words: a **pickup** is an item lying in the map that a player takes by
walking into it (KF's Pickup actors). A **spawn point** is a map spot
where KF may put a random weapon or vest (KFRandomItemSpawn). An **ammo
box** is a map spot with a box of ammo (KFAmmoPickup).

### What KF does (from the scripts and class defaults)

- **The maps**: 34 maps have 541 ammo boxes and 33 maps 468 spawn points
  (e.g. KF-WestLondon 14 and 14). Only KF-Aperture (3 M79s, 2 vests) and
  KF-Icebreaker (a Handcannon and a Katana) place weapons directly; one
  WebPickup in KF-Aperture is deleted in the map file.
- **Spawn point** (KFRandomItemSpawn, KFRandomSpawn defaults): a list of
  up to 11 pickup classes with weights. With `bForceDefault` (true by
  default) the class default list is used: Dual 9mms 3, Shotgun 1,
  Bullpup 3, Handcannon 3, Lever Action 2, Axe 1, Machete 1, Vest 2.
  6 spawn points (KF-Waterworks, KF-FilthsCross, KF-Manor, KF-Departed)
  set `bForceDefault=false` with their own list. A weight of 0 counts as
  1. The class is drawn with GetWeightedRandClass (Rand(total + 1),
  walking the tally).
- **Which are on** (KFGameType.SetupPickups, at the match start and at
  the start of every wave after the first, the boss wave included):
  every spawn point is switched off and every ammo box put to sleep, then
  `int(count x share)` random ones are switched on: Normal (our
  difficulty) 30% of spawn points and 50% of ammo boxes (Beginner 50/65,
  Hard 20/35, Suicidal and above 10/10). Ammo boxes appear at once.
- **A switched-on spawn point** (KFRandomSpawn.Timer, EnableMe 0.1 s):
  when no player is within 2000 units with a clear line from their eyes
  (PlayersCanSeeMe), it rolls a class and puts that pickup there
  (TurnOn); else it tries again 5-10 s later. Every 20-40 s
  (InitialWaitTime 20 + random) an unseen spawn point rolls again: the
  item can change while nobody looks. A switched-off one removes its
  item at once if nobody sees it, else 1-6 s later when unseen.
- **Taking a weapon from a spawn point** (KFWeaponPickup.Touch,
  KFGameType.WeaponPickedUp): the pickup is gone for good (its
  RespawnTime is set to 0), the spawn point is switched off, and another
  random switched-off spawn point is switched on after 30 s / number of
  players. A vest is not a KFWeaponPickup: taking it leaves its spawn
  point on, which rolls a new item at its next 20-40 s timer.
- **Taking ammo** (KFAmmoPickup.Touch, KFGameType.AmmoPickedUp): the box
  sleeps (hidden) until the next wave's SetupPickups, and another random
  sleeping box wakes after RespawnTime / living players (Ammo default 30
  s; 3 maps set 120), then waits until no player has a clear line to it
  (checked every second), 0.5 s more, and appears.
- **Directly placed pickups** (Aperture, Icebreaker): there from the
  start, not part of SetupPickups; after being taken they come back
  after their RespawnTime (Aperture's: 12000 s, i.e. never in practice;
  a KFWeaponPickup's default 100 s, waiting while a player sees it).
- **Touch** (Pickup.ValidTouch): the player's cylinder (radius 20, half
  height 50) overlaps the pickup's (weapons 20-35 x 5, ammo 20 x 10,
  vest 30 x 5) and a straight line from the player's centre to the
  pickup is clear. Touch happens when the overlap begins, and when a
  pickup appears on a player already standing in it (state Pickup's
  Begin: CheckTouching).
- **Weapons** (KFWeaponPickup.CheckCanCarry, KFWeapon.HandlePickupQuery,
  Single / Dualies / Deagle / DeaglePickup): too heavy for the carry
  limit: "You can not carry this weapon" (KFMainMessages 2, at most every
  0.5 s); already owned: "You already have this weapon" (KFMainMessages
  1); a Dual 9mm pickup with the 9mm owned turns it into the duals (the
  single's rounds join, as when buying); a Handcannon pickup with a
  Handcannon owned gives Dual Handcannons. A fresh pickup gives the
  weapon with its starting ammo and a full magazine (KFWeapon.GiveAmmo),
  SellValue -1 (it sells for 75% of its price, like a starting weapon).
  The new weapon is brought up if its Priority is higher than the held
  one's (Weapon.ClientWeaponSet, bNeverSwitchOnPickup=false in User.ini).
- **Ammo box**: every carried ammo type that accepts ammo pickups
  (KFAmmunition.bAcceptsAmmoPickups; not the chainsaw's) and is not full
  gets its AmmoPickupAmount (9mm 30, shotgun 8, LAW 2, flamethrower 80,
  M203 2, ...) x the perk's GetAmmoPickupMod (Medic: medic guns +20% a
  level up to 5; Commando: assault rifles 1.10 / 1.20 / 1.25; Firebug:
  fire weapons and MAC10 +10% a level; others 1.0; the Support Specialist
  has none), up to MaxAmmo. Types with AmmoPickupAmount 1 (frags, pipe
  bombs) get one round with chance 1 / GameDifficulty (50% on Normal). If
  nothing could take ammo the box stays. The Hunting Shotgun reloads both
  barrels if both were empty (BoomStick.AmmoPickedUp).
- **Vest** (Vest / ShieldPickup.Touch, KFPawn.AddShieldStrength): taken
  only below 100 armour; armour + 100, at most 100.
- **Message** (PickupMessagePlus: white, 3 s, at 90% of the screen
  height): the pickup's PickupMessage: "You got the Shotgun.", "You got
  the Bullpup", "You found another 9mm handgun", "Found Some Ammo!",
  "You found a Kevlar Assault Vest", ...
- **Sound** (Pickup.AnnouncePickup): the pickup's PickupSound at the
  pickup (SLOT_Interact) with its TransientSoundVolume / Radius: weapons
  their own (KF_PumpSGSnd.SG_Pickup, ...; default
  KF_BullpupSnd.Bullpup_Pickup) at 100, ammo KF_InventorySnd.Ammo_GenericPickup
  at 100, the vest KF_InventorySnd.Vest_Pickup at 150 radius 450.
- **Looks**: the pickup class's StaticMesh (KF_pickups_Trip.*,
  kf_generic_sm.pickups.Metal_Ammo_Box; 3 Christmas maps give their ammo
  boxes Workshop_SM.wsgift02b at DrawScale 0.4), DrawScale, DrawScale3D,
  PrePivot; CullDistance (ammo 4000, weapons 6500). They fall to the
  floor (Physics=PHYS_Falling) and lie still: no spin or bob (the base
  class spins only in PHYS_Rotating, which KF pickups do not use).

### Our design

- `crates/ue-assets/src/level.rs`: placed pickups and spawn points are
  collected (`LevelContents.pickups`: class, location, rotation, export)
  and no longer drawn as plain meshes (the 3 Christmas maps drew their
  gift-box ammo boxes as scenery).
- **`src/game/pickups/` (new), the pickup core** for every kind of
  pickup, so dosh tossing and weapon dropping can reuse it:
  - `classes.rs`: `PickupClass` read from class defaults (kind, mesh,
    scale, pre-pivot, collision cylinder, sound, message, cull distance,
    respawn time) and `PickupGives` (what taking it gives: a weapon with
    its weight and optional carried state, ammo, armour, cash).
  - `rules.rs`: KF's rules above as plain code with no Bevy in it (spawn
    points, ammo boxes, placed pickups, SetupPickups, WeaponPickedUp,
    AmmoPickedUp), fed the time and "can a player see this", unit-tested.
  - `mod.rs`: the `Pickups` resource: every pickup with a **network id**
    (spawn points first, then ammo boxes, then placed pickups, in map
    order; dropped items later get ids from 100000), its class, resting
    place (dropped to the floor with a trace), and whether it is shown.
    Systems: start the rules when the match begins (frame 10 and the
    lobby closed, as the wave timer) and on a restart; call SetupPickups
    at each wave start; run the timers; draw / remove the meshes; detect
    the local player's touches; play the sound and show the message.
  - `src/weapons/weapon/pickup.rs` (new): the inventory side. A
    `PickupUse` message asks "could my player take this?" (dry run) or
    "give it" (apply); the answer comes back as `PickupUsed`. It applies
    KF's weight, owned, ammo and armour rules above.
- **Who decides**: in single player and on a network host this game runs
  the rules. A touch is first checked against the player's own inventory
  (dry run); if it fits, the rules mark the pickup taken (log
  `pickup_collected`), the item is given, the sound plays, the message
  shows.
- **Network games** (the host owns the pickups, like the doors):
  - the host sends every client the list of shown pickups (id, class,
    place, rotation, what it gives) when it changes and every 2 s (for
    late joiners); clients draw exactly that list and run no rules.
  - a client's touch is dry-run against its own inventory (inventories
    are each game's own in this prototype), then sent as "I take pickup
    N (class C)". The host checks it (the pickup exists and is shown,
    the class has not changed, the player is alive and within reach of
    it: overlap distance + 150 units, for the 0.1 s the remote pawn is
    drawn behind); then marks it taken and tells the taker "yours" (its
    game gives the item and shows the message) and everyone else "gone"
    (they play the sound). A refused request gets `pickup_denied` with
    the reason (`not_shown`, `class_changed`, `dead`, `too_far`).
  - the host's own player goes through the same check, so whoever's
    request reaches the host first gets the item and the other is
    refused.
  - Not KF's model: KF's server knows every inventory and does the whole
    touch itself; ours trusts the client's own inventory check (fine
    between friends). A rare race (the client's inventory changed between
    its check and the host's answer) loses the item (logged
    `pickup_apply_failed`).
- **Logs**: `pickups_loaded` (counts per map), `pickup_setup` (which are
  on), `pickup_spawned` / `pickup_hidden` / `pickup_respawned` (with ids
  and classes), `pickup_touch`, `pickup_collected`, `pickup_denied`,
  `pickup_given`, `pickup_refused` (inventory rules), and on the network
  `net_pickups_sent` / `net_pickups_received`, `net_pickup_request`,
  `net_pickup_taken`.
- **Test inputs**: `warp_pickup:KIND[:DIST]` stands the player on (or
  DIST units from, facing) the lowest-id shown pickup of KIND (weapon,
  ammo, vest, any); `look_pickup:KIND` turns the view to it.
- **For dosh tossing and weapon dropping** (next agent): spawn a pickup
  with `Pickups::spawn_dynamic` (class, place, `PickupGives` with the
  cash amount or the weapon's magazine, ammo and sell value, lifetime);
  it gets an id from 100000, is drawn, touched, sent to clients and taken
  through the same path. The inventory side already applies
  `PickupGives::Cash` and a weapon's carried state (not tested: nothing
  creates them yet).
- **Guesses (not in the scripts)**: our random numbers are a fixed-seed
  generator (KF's FRand is not reproducible); when a switched-off spawn
  point removes its item after the 1-6 s delay we stop there (KF's code
  then falls into the turn-on check; what it does with the destroyed
  item depends on native engine code we cannot see); pickups lie flat
  with the spawn point's yaw (KFRandomSpawn tilts them by a random 1000-
  11000 pitch before they fall; bOrientOnSlope on landing is native and
  we assume it levels them); KFWeaponPickup's "player sees me"
  (LineOfSightTo, native) is a clear line from the eyes; the
  pickup-overlay shine (UV2Texture PickupOverlay) and AmbientGlow are not
  drawn.

### As built (2026-10-07; headless runs, not played by you)

- Files: `src/game/pickups/{mod,classes,rules}.rs`,
  `src/weapons/weapon/pickup.rs`, `src/net/pickups.rs`; small changes in
  `crates/ue-assets/src/level.rs`, `src/world/map.rs`,
  `src/weapons/weapon/{mod,load}.rs` (`pickup_amount`), `src/game/perks.rs`
  (`ammo_pickup_mod`), `src/game/hud.rs` (`MessageClass::Pickup`),
  `src/net/{mod,protocol}.rs`, `src/main.rs`.
- Loaded on KF-WestLondon (14 spawn points, 14 boxes), KF-Farm (18, 27),
  KF-EvilSantasLair (15, 20, gift-box meshes), KF-Aperture (9, 20 with
  RespawnTime 120, 5 placed: 3 M79s, 2 vests; the deleted WebPickup left
  out), KF-Manor (14, 14, 3 spawn points with their own lists). The
  plain scenery count drops by those actors (`skipped_actors
  ... "KFAmmoPickup:pickup": 14`).
- Setup: Normal gives `spawn_points_on=4/14`, `ammo_on=7/14` on
  KF-WestLondon at the match start (after the lobby), new random ones at
  the wave start (`pickup_setup reason=wave_start`; old items removed
  as nobody saw them). Items appear 0.1 s after the setup where nobody
  sees them and change every 20-40 s while unseen (`why=rerolled`).
- Taking: ammo box `ammo=[9mm Tactical:+30:total=150 Frag Grenade:+1]`
  (the frag at 50%: +1 then +0 in two takes); walking into a Bullpup
  from 150 units: `pickup_given ... ammo=40+120 switched=true`, "You got
  the Bullpup", `KF_BullpupSnd.Bullpup_Pickup` at the pickup; vest
  `armour=0->100`, a second vest `armour_full`; 9mm + Dual 9mm pickup:
  `replaced_single=true ammo=30+210`; a second Handcannon:
  `weapon=KFMod.DualDeagle replaced_single=true`; an owned Machete:
  "You already have this weapon"; too heavy (Shotgun 8 on 9 of 15, and
  a Handcannon at 15 of 15): "You can not carry this weapon" (KF checks
  the weight before "already owned", so an owned heavy gun shows the
  weight message, as in KF).
- Timers: a box taken at 7.17 s, another box shown at 37.71 s (30 s +
  0.5 s); a weapon taken at 8.93 s, another spawn point's item out at
  38.93 s (30 s / 1 player).
- Looks: screenshots `work/screenshots/KF-WestLondon-pk-c-*` (a Shotgun
  on the street; the vest), `KF-Farm-pk-*` (ammo box on the barn floor),
  `KF-EvilSantasLair-pk-*` (the gift box), `KF-Aperture-pk-*`
  (untracked). The ammo box's mesh is centred by its PrePivot (bounds
  y -1..43, PrePivot y 21), which confirms the sign. Weapons lie 5
  units above the floor (their collision cylinder's half height; the
  meshes start at their own z 0); the gift boxes about 5 units (if UE2
  scales PrePivot with DrawScale, as we assume; not checked).
- Network: docs/multiplayer-prototype.md, "Pickups shared".
- Not done: the pickup shine (UV2Texture overlay) and AmbientGlow, so
  dark guns on dark floors are hard to see; the random tilt of spawned
  items; bots' interest in pickups (no bots); KF's InventorySpot
  navigation marks. (Dropped weapons and tossed dosh: next section.)
  Found while testing, not changed: on KF-WestLondon the
  walking player falls through the road at about (-2769, 883, -3858)
  (a hole in our walking collision there; rays do hit the road).

### Tossed dosh and dropped weapons (planned 2026-10-07)

Words: **tossing dosh** is throwing some of your cash on the floor (KF's
TossCash); **dropping / throwing a weapon** puts the gun in your hands
on the floor (KF's ThrowWeapon). Both become pickups (above) that
anybody can walk into.

What KF does (scripts and class defaults):

- **Keys** (User.ini): `B=TossCash`, `Backslash=ThrowWeapon`.
- **TossCash** (KFPawn.TossCash, no amount given): 50 per press, no
  minimum kept and no rate limit (one toss per key press). The cash is
  first cut to a whole number; with 0 or less nothing happens; else
  min(50, cash) is tossed. The CashPickup appears at the pawn's centre +
  0.8 x CollisionRadius forward - 0.5 x CollisionRadius right (16 ahead,
  10 to the left), flying at view direction x (pawn velocity along it +
  500) + 200 up. Its CashAmount is the amount; the cash leaves the
  player. With more than one player the "dosh" voice line plays (not
  done: no voice lines yet).
- **CashPickup**: 22Patch.BankNote at DrawScale 0.4, cylinder 20 x 5,
  KF_InventorySnd.Cash_Pickup at volume 150. Anyone can take it,
  the thrower too (bOnlyOwnerCanPickup is false by default). Taking it
  adds the amount to the taker's cash; the message is
  "Found (50) Pounds." (CashPickup.GetLocalString). It does not
  come back.
- **Falling and fading** (Pickup.InitDroppedPickupFor, states
  FallingPickup, Pickup, FadeOut): it falls (PHYS_Falling, gravity 950)
  and can already be taken while falling, but not by a player it was
  touching when it appeared (a touch is the start of an overlap); when
  it lands, anyone standing on it takes it (CheckTouching). LifeSpan
  16 s from the toss; 8 s after landing (or after 8 s still falling) it
  fades out: 1 s spinning (yaw rate 60000) and shrinking (DrawScale goes
  down by its default per second), then it is gone. It can still be
  taken while fading.
- **ThrowWeapon** (PlayerController.ServerThrowWeapon, KFWeapon.CanThrow):
  not for bKFNeverThrow weapons (Knife, 9mm, Frag, Syringe, Welder), not
  while reloading, switching or before the next shot is allowed. Velocity:
  view direction x (pawn velocity along it + 150) + 100 up, plus the
  pawn's facing x 100 (KFWeapon.DropFrom; the dual pistols' DropFrom
  does not add it). Same start spot as the cash. Then the best weapon
  comes up (ClientSwitchToBestWeapon).
- **The dropped weapon** (KFWeaponPickup.InitDroppedPickupFor): keeps the
  magazine (MagAmmoRemaining), the ammo (AmmoAmount[0] and [1]) and the
  SellValue. Taking it (KFWeapon.GiveTo / GiveAmmo): the magazine as it
  was; the ammo as it was if it was **thrown** by a living player
  (bThrown); a weapon dropped by a dying player gives the class's fresh
  starting ammo instead (bThrown false), with the dead player's
  magazine. The taker's sell value is the pickup's.
- **Dual pistols** (Dualies / DualDeagle / Dual44Magnum /
  DualMK23Pistol.DropFrom): a living player keeps one pistol with half
  the ammo (total / 2, magazine / 2); the pickup gets the rest. The
  Dual 9mms drop a DualiesPickup; the others drop the single pistol's
  pickup (DeaglePickup, Magnum44Pickup, MK23Pickup).
- **Dropped weapons do not fade** (KFWeaponPickup overrides
  InitDroppedPickupFor without the 16 s LifeSpan and empties FadeOut).
  All dropped items, cash and weapons, are destroyed when the trader
  closes (KFGameType.CloseShops, at the next wave's start).
- **Death** (KFPawn.Died -> TossWeapon -> DropFrom): the weapon in hand
  is dropped, whatever it is (only bCanThrow is checked, which KF's
  weapons leave true: the knife or 9mm drop too), flying at view
  direction x (velocity along it + 500) + 200 up (+ facing x 100). KF
  does not drop dosh on death (only the 5% x difficulty loss, dosh.rs).
- **Perk change** (KFHumanPawn.VeterancyChanged): over the new carry
  limit, weapons (not bKFNeverThrow) are dropped in inventory order until
  the weight fits, each from the pawn centre + a random 10 units, at the
  pawn's velocity + facing x 100.

Our design:

- **Pickup core** (`src/game/pickups/`): `DropRequest` (pickup class,
  what it gives, start, velocity, yaw, why: toss / throw / perk / death)
  is turned into a pickup by the game that owns the pickups (single
  player or the host). The flight is worked out once when it is
  dropped (`drop.rs`, no Bevy in the step code: 30 steps a second,
  gravity 950, rays against the level; a wall stops the sideways
  motion; a floor (normal up >= 0.7) ends it) and stored with the pickup
  as a list of points, so every game draws the same arc. The pickup's
  `location` is where it lands. `fading` is set when the fade-out
  starts. Clients replay the arc from when they first see the pickup.
- **Timers** (host / single player): cash fades 8 s after landing (or
  8 s after the toss if still falling), is gone 1 s later, at most 16 s
  after the toss; weapons stay. All dropped items go when the trader
  closes (the wave start) and on a restart.
- **Touch** while falling uses the point on the arc; a player it
  overlapped when it appeared is not touched until the pickup lands on
  them (CheckTouching on landing).
- **Inventory side** (`src/weapons/weapon/drop.rs`): the keys B and
  Backslash (test inputs `toss_cash`, `throw_weapon`), the death drop
  and the perk drop build the requests (amount, magazine, ammo, sell
  value, the dual split). The cash or weapon leaves the inventory only
  when the pickup core answers "done" (`DropDone`): at once in single
  player and on the host, after the host's answer on a client. While a
  request waits, its cash is reserved and the same weapon cannot be
  thrown again.
- **Network**: a client's request goes to the host (`DropRequest`
  message); the host checks it (the player's pawn exists, the start
  is within 300 units of where the host draws it; a dead player only for
  the death drop), makes the pickup (host-owned, sent like the others)
  and answers `Dropped { token, id }` or a refusal with the reason. The
  scoreboard's cash follows (every game sends its own dosh).
- **Solo restart**: since a death now drops the gun, a restart after a
  loss gives the starting inventory again (as KF's map reload does).
- **Guesses**: the pickup's yaw is the player's yaw (KF spawns it with
  the spawner's rotation; we lay it flat); the "best weapon" after a
  throw is the highest Priority one (RateWeapon is close to it for
  players); pawn Rotation is taken as yaw only (the pawn's facing);
  our rays treat the pickup as a point with its collision height below
  it and a few units to the side, not a full cylinder.

As built (2026-10-07; headless runs, not played by you):

- Files: `src/game/pickups/drop.rs` (new: the arc, the timers, KF's
  velocities, 7 unit tests), `src/game/pickups/mod.rs` (`DropRequest`,
  `DropItem`, `DropDone`, `spawn_dropped`, landing / fading / expiry,
  trader-close and restart cleanup, touch on the arc, drawing the arc
  and the fade), `src/game/pickups/classes.rs` (`CarriedWeapon.thrown`,
  `.alt`), `src/weapons/weapon/drop.rs` (new: keys, death drop, perk
  drop, applying `DropDone`), `src/weapons/weapon/{pickup,perk,load,mod}.rs`,
  `src/game/end_game.rs` (solo restart gives the starting inventory),
  `src/net/{pickups,protocol,mod}.rs` (`DropRequest` message,
  `PickupNotice::Dropped`, `PROTOCOL_ID` `0x4F4B_4600_0007`).
- Toss (KF-WestLondon, standing): `drop_request ... gives=cash:50
  velocity=(-500, 9, 200)`, the bundle lands 284 units ahead after 0.57 s
  (`pickup_spawned ... flight=0.57s fade_at=+8.57s gone_at=+9.57s`), the
  thrower is not touched when it appears (`pickup_touch_skipped
  reason=inside_when_dropped`), `dosh=250->200`; walking onto it:
  `dosh=200->250`, "Found (50) Pounds."; left alone: `pickup_fading`
  8.0 s after landing, `pickup_hidden why=expired` 1.0 s later.
- Throw: Shotgun `velocity=(-250, 5, 100)`, lands 109 units ahead in
  0.43 s, `ammo=8+16`, the 9mm comes up; taken back: `ammo=8+16`. The
  Hunting Shotgun of a level 6 Support (sell value 225) thrown and taken
  back: `sell_value=Some(225.0)`. The 9mm: `drop_refused
  reason=never_throw`.
- Dual Handcannons thrown: a DeaglePickup with `mag8:total24`, the
  player keeps a Handcannon with `8+16`.
- Perk change (Support 6 with an AA12 and Dual Handcannons, weight 25,
  to Medic, limit 15): `perk_drop dropping=[DualDeagle, AA12AutoShotgun]`,
  weight 25 -> 13. The DeaglePickup falls at the player's feet (no
  velocity: KF's Dualies.DropFrom) and is taken back on landing (KF's
  CheckTouching does the same), so the player ends with the Dual
  Handcannons and weight 15; the AA12 is refused "too heavy".
- Death (no god mode, Hunting Shotgun in hand): `why=Death
  velocity=(-600, 11, 200) ... sell225:died`; the restart then gives
  the starting inventory (`respawn_inventory`).
- Trader close: a tossed bundle removed at the wave 2 start
  (`pickup_hidden ... why=close_shops`). A wave run with `aim_zed` /
  `fire` still earns dosh (19 kills, wave-end pot 237).
- Screenshots (untracked): `work/screenshots/KF-WestLondon-drop-c-*-1.png`
  (the dosh bundle on the floor), `KF-WestLondon-drop-d-*-1.png` (the
  thrown Hunting Shotgun).
- Network: docs/multiplayer-prototype.md, "Dosh and weapons dropped".
- Not done: the "dosh" voice line with other players (no voice lines
  yet); story-mode carried items (TossCarriedItems); the
  WeaponPickup.FallingPickup rule that a thrower running after his own
  rising weapon does not catch it (rare; our "inside when it appeared"
  rule covers the usual case); the dropped item's shadow and shine.

## Joining with only an address: the host-info query (planned and built 2026-10-07)

**Problem.** Every game loads its map at startup, before any network
code runs (`world/map.rs` `load_map` is a Startup system and about 35
others depend on it). So a joining player had to type the host's `--map`
and `--mode waves` too; with a wrong map the client quit after
connecting (`net_map_mismatch`), and without `--mode waves` it silently
never followed the host's waves (`game/waves.rs` returns early unless the
mode is Waves).

**What KF does.** An Unreal server answers small "server info" requests
on a second UDP port, the QueryPort, which is the game port + 1 (7708 for
KF's 7707). The server browser asks it for the map, the game type and the
player count before joining.

**Plan.** Do the same, before the Bevy app (the game engine's main
object) is built:

- Q1. `src/net/query.rs` (new). A "host info" message: a first line
  `OPENKF-QUERY 1` (a fixed tag, then the format version) and then one
  `key=value` line per fact: `protocol` (our `PROTOCOL_ID`), `game_port`,
  `map`, `mode`, `length`, `players`, `max_players`, `match_started`.
  Plain text, so new facts can be added later without breaking older
  games: a reader ignores keys it does not know and uses defaults for
  missing ones. Only a different version number on the first line means
  "cannot read". Unit tests: write then read; unknown keys ignored;
  missing keys; garbage refused.
- Q2. The host (`--host`) opens a plain UDP socket (std::net, works on
  Windows too) on game port + 1 and answers each request from a small
  background thread, so it answers even while the host is still loading
  its map. The map / mode / length are fixed at startup; the player count
  and "match started" are copied into it by a game system every frame.
  The request is padded to 512 bytes and the host answers only requests
  of at least that size, never with more bytes than it got (so the port
  cannot be used to multiply someone else's traffic). If the port is
  taken the host logs it and plays on (joiners then need `--map`).
- Q3. The joiner (`--join`), in `main.rs` before the app is built: send
  the request (up to 5 tries, 0.6 s each), read the answer, and use the
  host's map, mode and length in place of its own. The host wins when
  the player also typed `--map` / `--mode` / `--length` and they differ;
  the game prints a note and logs it. A different `protocol` (another
  version of our network code) stops with a clear message. No answer:
  stop with "no answer from a host at ADDR (query port P)...". The old
  map check after connecting stays as a safety net.
- Log lines: `net_query_listening`, `net_query_answered` (host);
  `net_query_sent`, `net_query_answer`, `net_query_override`,
  `net_query_failed` (joiner).

**As built.** As planned, with one addition: when the query gets no
answer but the player typed `--map`, the joiner warns and tries the old
way (for a firewall that lets only the game port through); without
`--map` it stops with the error. Why text and not a binary (postcard)
message: postcard reads fields by position, so adding a field would
break older games; `key=value` lines do not. A host needs UDP ports P
and P + 1 open, and two hosts on one machine need ports at least 2
apart. Test results: docs/multiplayer-prototype.md, "Joining with only
an address".

## The launcher: choosing options without typing (planned and built 2026-10-07: LA1-LA3)

**Goal.** A window where the player picks how to play (solo, host,
join), the map, the game options, the perk and character, and the
window and sound settings, then presses PLAY. No command line needed.

**How it starts.** The launcher opens when the program is started with
**no arguments at all** (a double-click), or with `--launcher` as the
first argument. **Any other argument skips it** and runs the game as
before, so scripts, tests and `scripts/headless.sh` runs are not
affected. What "no arguments" used to do (KF-WestLondon in debug mode)
is now `open-kf --mode debug` (the map already defaults to
KF-WestLondon).

**How PLAY works: a second copy of the program.** The launcher is its own
small Bevy app (Bevy: the game engine library). Bevy can open its
window system only once per program run, so the launcher cannot turn
itself into the game. Instead PLAY starts the program again (the
same file, found with `std::env::current_exe`) with the options as
command-line arguments (`std::process::Command`, which works the same
on Windows), closes the launcher window and waits for the game to end,
then exits with the game's exit code. The game and its argument parsing
stay as they are. Before starting, the launcher runs the game's own
argument check (`parse_args`, changed to take a list instead of reading
the real command line) on the arguments it built, so a mistake in the
"extra arguments" field is shown in the launcher instead of the game
quitting at once.

**What it reuses.** The KF menu drawing in `src/game/menus/gui.rs`:
`gui::load` (KF's fonts and menu textures from the install), `Painter`
(KF-style section boxes, buttons, text), `flush` (draws onto the screen).
These need only the install folder, not a loaded map, so the launcher
does not load any map. The perk list and icons come from
`game/perks.rs`, the character list and portraits from
`player/character.rs` (`model_select_records`, the same 56 characters as
KF's Select Character window), the map list from the install's `Maps`
folder (`*.rom` files, without `Entry`, `KFintro` and `KF-Menu`, which
are not playable maps). The host check uses `net/query.rs`.

**The screen.** One KF-style window (title bar "Open KF"), four
sections and a bottom row:

- *Play*: Solo / Host / Join buttons. Host: a port field (default 7707).
  Join: an address field (`192.168.1.20` or `192.168.1.20:7707`) and a
  CHECK HOST button that asks the host (in the background, so the
  window does not freeze) and shows its map, mode, length and players.
- *Map*: the list of maps (click one; mouse wheel scrolls). Hidden when
  joining: the map comes from the host.
- *Game*: mode (Waves / Debug), length (Short / Normal / Long), starting
  wave (From the start / 1-11). Hidden when joining. Length and wave
  are greyed out in debug mode (they do nothing there).
- *Player*: name (typed), perk (with its icon; or None), perk level
  0-6, character (with its portrait), each changed with `<` and `>`.
- *Display and sound*: window size (Default or a fixed size), frame
  limit (None, 30-240), vsync on/off, sound on/off. (2026-10-07: the
  window, frame limit and vsync rows moved to the *Graphics* section, see
  "Graphics settings in the launcher"; this box is now "Sound, menus".)
- Bottom: an "extra arguments" field (for test options such as
  `--god`), the command line that PLAY will run (so you can copy it),
  QUIT and PLAY. A reason is shown when PLAY cannot be used (a bad
  port, no address).

Typing: click a field (or Tab to the next one), type, Backspace
deletes, Enter or a click elsewhere finishes. Escape quits.

**Saved choices.** Written when PLAY is pressed, read when the launcher
opens: `settings/launcher.txt` in the folder the program runs from (next
to `logs/`), one `key=value` per line. The `.gitignore` whitelist does
not list it, so git ignores it. `--settings FILE` uses another file
(tests use this so they do not overwrite your choices).

**Testing without a mouse.** The launcher takes, after `--launcher`:
`--input FRAME:ACTION,...` (actions `click:ID` presses a button by its
id, `set:FIELD=VALUE` sets a choice, `dump` logs every button's box),
`--dry-run` (PLAY logs the command but starts nothing), `--screenshot
N`, `--frames N`, `--window WxH`, `--log FILE`, `--settings FILE`,
and `--mute` (the started game gets `--mute` too). It logs to
`logs/launcher.log`: `launcher_open`, `launcher_change field=... value=...`
for every change, `launcher_launch args="..."`, `launcher_child_exit`.

**Steps.**

- LA1. `parse_args` takes a list (no behaviour change); the launcher
  module (`src/launcher/`): the choices and the arguments they make
  (unit tests), the screen, the input, PLAY (start the game, wait).
- LA2. Saved choices (`settings/launcher.txt`).
- LA3. The CHECK HOST button.

**As built (2026-10-07; headless runs, not used by you yet).** As
planned, with these details:

- Files: `src/launcher/mod.rs` (start-up, input, PLAY, saved file, CHECK
  HOST), `src/launcher/choices.rs` (the choices, the arguments they make,
  the saved text; unit tests), `src/launcher/draw.rs` (the screen).
- The window opens at 1280 x 800. Text that does not fit is drawn in the
  next smaller KF font. The perk's icon and the character's portrait are
  shown under the Player rows (small at 1280 x 800).
- Defaults when nothing is saved: Solo, KF-WestLondon, Waves, Short,
  from the first wave, no perk, Corporal_Lewis, default window, no frame
  limit, vsync on, sound on. The launcher defaults to Waves (a real game)
  while the game's own default stays Debug.
- An empty name passes no `--name` (the game uses KF's defuser.ini
  name); the field shows "(KF's default)".
- The started game always gets `--character` and, unless joining,
  `--map` and `--mode` (so the command shown is complete).
- CHECK HOST asks 3 times, 0.5 s each, and shows the answer (map, mode,
  length, players) in the Map box, which is otherwise empty when joining.
- Enter outside a text field = PLAY; Escape outside a field = QUIT.
- `--settings FILE` and `--log FILE` keep test runs away from your saved
  choices and from `logs/launcher.log`.
- Test actions: `click:ID` (ids: `play:solo|host|join`,
  `focus:name|port|address|extra`, `spin:FIELD:-1|1`, `map:INDEX`,
  `maps.scroll:N`, `check_host`, `launch`, `quit`), `set:FIELD=VALUE`
  (fields as in the saved file), `type:TEXT`, `key:tab|enter|escape|backspace`,
  `dump`. Values cannot contain commas (`--input` splits on them).

### Pasting into the launcher's text fields (planned 2026-10-07: LP1)

**Goal.** Paste a join address (or a name, port, extra arguments) from
the system clipboard instead of typing it. The clipboard is the shared
copy/paste store of the desktop.

**Clipboard access.** Bevy 0.19 ships its own clipboard module
(`bevy::clipboard`, the `Clipboard` resource, added by `DefaultPlugins`).
It uses the arboard library for X11, Wayland and Windows, but only when
Bevy's `system_clipboard` feature is on; without it the clipboard is a
private buffer inside the program. So the change is one Bevy feature in
`Cargo.toml`, no new library of our own: arboard and its X11 / Wayland /
Windows parts were already in `Cargo.lock` (Bevy lists them as
optional), so the lock file does not change and the Flatpak's
`cargo build --locked` still works. Bevy's `wayland` feature (on by
default) turns on arboard's Wayland clipboard support
(`wayland-data-control`); when the desktop does not offer that, arboard
falls back to X11 (XWayland).

**How to paste.**
- Ctrl+V or Shift+Insert (or a keyboard's Paste key) pastes into the
  field being typed in.
- A right-click on a text field selects it and pastes.
- A small "Paste" button next to the Join address.

**Cleaning the pasted text.** Done in one tested function
(`launcher::paste_text`):
- Single-line fields (port, address, name) take the first non-empty
  line, trimmed; extra arguments join the lines with spaces.
- Allowed characters: port digits only; address letters, digits and
  `. : - _ [ ]` (IPv4, host names, `host:port`); name anything printable
  but `"` (KF's settings page removes quotes); extra anything printable.
  Other characters are dropped and counted.
- Length limits: port 5, address 100 (our choice), name 16 (KF's
  name box, `ROTab_GameSettings` `MaxWidth=16`), extra 1000 (our choice).
  Typed characters follow the same rules.
- Port and address are replaced by the paste (the launcher has no text
  cursor or selection, so appending an address to an old one would only
  make a broken one); name and extra get the paste added at the end.
- `1.2.3.4:7707` pasted into the address is kept whole (the game's
  `--join` already takes `ADDR:PORT`). Pasted into the host's port field,
  the part after the last `:` is used.

**Log.** `launcher_paste field=F via=ctrl+v|shift+insert|right_click|button|test
mode=replace|append kept=N dropped=N cut=N value="..."`, or
`launcher_paste_failed` with the reason (no field selected, empty or
unreadable clipboard).

**Test actions.** `clipboard:TEXT` puts TEXT on the clipboard (through
the same Bevy resource); `paste` pastes like Ctrl+V; `click:paste:address`
is the button.

## Graphics settings in the launcher (planned 2026-10-07: GX1-GX3)

**Goal.** The launcher gets a *Graphics* section: display mode, resolution,
vsync, frame limit, field of view, brightness, anti-aliasing and texture
filtering. Only settings the renderer really uses; nothing that is a
dead switch.

**How they reach the game: command-line options, as before.** The launcher
already turns every choice into a game option (`--window`, `--fps`,
`--no-vsync`). The new settings work the same way, so the game has one
way in, and test runs (`scripts/headless.sh`) never pick up your saved
settings by surprise. Options typed in the launcher's "extra arguments"
field come last on the command line, and for these options the last one
wins, so a typed option overrides the saved one. The choices are saved in
the launcher's existing file (`settings/launcher.txt`, new keys
`display`, `fov`, `brightness`, `msaa`, `anisotropy`; the old `window`,
`fps`, `vsync` keys stay). No second settings file.

**The settings.** (KF's values from `System/Default.ini`, read only; our
defaults keep today's look so nothing changes unless you pick something.)

| Setting | Game option | Values | Default | What it changes |
| --- | --- | --- | --- | --- |
| Display mode | `--display windowed\|borderless\|fullscreen` | Windowed, Borderless (a window the size of the screen, no border), Fullscreen (exclusive: the monitor switches to the resolution) | Windowed (KF: `StartupFullscreen=True`; windowed kept so nothing changes) | Bevy `WindowMode` |
| Resolution | `--window WxH` (exists) | Default, then every size the primary monitor reports (Bevy's `Monitor::video_modes`, from winit; works on Windows too); a fixed list when the monitor reports none | Default | Windowed: the window size. Fullscreen: the video mode (the monitor's own size when Default). Borderless: ignored (always the screen's size) |
| Vsync | `--no-vsync` (exists) | On / Off | On (KF D3D9: `UseVSync=False`) | `PresentMode` |
| Frame limit | `--fps N` (exists) | None, 30-240 | None | our frame limiter |
| Field of view | `--fov DEG` | 80-120, steps of 5 | 90 (KFPlayerController.InitFOV at 4:3) | the player's view angle (KF's horizontal degrees at 4:3, as now); iron sights still zoom from it. The weapon model keeps its own DisplayFOV, as in KF |
| Brightness | `--brightness PERCENT` | 50-200, steps of 10 | 100 (no change) | the 3D view's light times PERCENT/100 (sky, map, zeds, weapon; not the HUD or menus). A second full-screen modulate quad in the vision-overlay camera (render/overlay.rs: screen x 2 x colour), so 200 % is the most it can do. KF's own Brightness / Gamma / Contrast set the monitor's gamma ramp; we do not copy that (no gamma curve: a multiply only) |
| Anti-aliasing | `--msaa 0\|2\|4\|8` | Off, 2x, 4x, 8x | 4x (Bevy's default, today's look) | `Msaa` on every camera (they share the screen, so they must match). 2x and 8x are not supported by every graphics card: the game checks the card and falls back to 4x (logged) |
| Texture filtering | `--anisotropy 1\|2\|4\|8\|16` | Trilinear (1), Anisotropic 2x-16x | 8x (today's value; KF: `LevelOfAnisotropy=1`) | the map textures' sampler (`anisotropy_clamp`): sharper floors and walls seen at a slant |

**Not added (and why).** Render scale (drawing at a lower resolution and
stretching): Bevy has no simple switch for the window's main view; it would
need our own render-to-image path. Gamma / contrast curves: no cheap pass
for a curve over the whole frame (Bevy's colour grading runs only in its
own materials, not in ours). Shadows, detail levels: the renderer uses
KF's baked lighting and has none to turn down. Changing settings in the
pause menu: another branch is changing the pause menu; FOV and brightness
could be added there later (both are resources the game reads every frame).

**Logs.** The game logs `graphics_settings` at start (every value and where
it came from), `window_state` when the window is ready and whenever its
mode or size changes (mode, physical size, scale factor, present mode),
`msaa_applied` (cameras, the sample count, any fallback),
`fullscreen_mode` (the video mode picked for exclusive fullscreen, or why
none). The launcher logs `launcher_display_modes` (the monitor and the
sizes offered).

**Steps.**

- GX1. Game: the new options, `engine/graphics.rs` (the settings resource,
  MSAA on cameras, exclusive fullscreen, window logs), FOV in the camera
  and iron sights, brightness quad, anisotropy in map textures.
- GX2. Launcher: the Graphics section (column 2, under the map list), the
  resolution list from the monitor, the saved keys, unit tests.
- GX3. Headless checks: logs, screenshots at several resolutions and
  brightness values, persistence (save, reopen).

**As built (2026-10-07; headless runs only, not used by you yet).** As
planned, with these details:

- Files: `src/engine/graphics.rs` (new: settings, option parsing shared
  with the launcher, the window, MSAA check and apply, video mode pick,
  `window_state` log; unit tests), `src/main.rs` (options),
  `src/engine/camera.rs` and `src/weapons/weapon/animate.rs` (FOV),
  `src/game/trader_arrow.rs` (the arrow's distance uses the player's FOV,
  KF's `DefaultFOV / FovAngle`), `src/render/overlay.rs` (brightness quad),
  `src/world/map.rs` (anisotropy), `src/launcher/` (the section, the
  monitor's sizes, saved keys).
- Exclusive fullscreen opens borderless and switches to the video mode once
  Bevy knows the monitors (first frame). The mode is the chosen size with
  the highest refresh rate; if the monitor has no such size, the desktop's
  size (logged `no_such_mode_used_desktop_size`). Bevy's own "current
  mode" failed on the test display (it reports no refresh rate), so we
  always pick a listed mode.
- The Graphics section is in column 2 under the map list (the list keeps
  at least half the column; at 1280 x 800 it shows 7 maps). Field ids for
  tests: `spin:display|window|vsync|fps|fov|brightness|msaa|anisotropy:±1`.
  FOV and brightness stop at their ends; the others wrap.
- The game only gets an option when it differs from the default (so the
  command line stays short and old command lines behave as before).
- On the virtual test display (Xvfb, no window manager) borderless and
  fullscreen are requested and logged but the window stays 1280 x 720:
  only a real desktop can show whether they work. Vsync on/off is also
  only checked as the logged present mode.

## NuMenu: our own trader menu (planned 2026-10-07: NU1-NU4)

The trader's buy menu so far is a text list standing in for KF's
GUIBuyMenu (`buy_menu.rs`). It works, but it is hard to read: one long
text block, the sale list shows one perk at a time (Left/Right to page),
"For sale" and "Yours" are two lists you Tab between, and you only learn
that something is too heavy or too expensive from a word at the end of
the line. KF's own GUI has the same two side-by-side lists, small
power/speed/range bars and no "what happens if I buy this".

**NuMenu** is a new design of our own (not a copy of KF's) and becomes
the default. The old menu stays: `--trader-menu kf` (default `nu`), also
a "Trader menu" choice in the launcher.

### What is shared, what differs

Buying already goes through one path: a menu only *asks*
(`ShopRequest::Buy / Sell / Ammo` messages, `armour::BuyVest`), and
`weapons/weapon/inventory.rs` (`shop_requests`) and `player/armour.rs`
(`buy_vest`) decide, with KF's rules (CanBuyNow, price x perk scaling,
half-price duals, weight limit, partial ammo, refusal sounds). So the
split is small; no rule moves:

- Shared (unchanged): the requests and their handling, prices
  (`ammo_prices`, `armour::menu_row`, the sale price rule), opening and
  closing (E in an open shop with no wave; the wave start, leaving the
  shop, E / Escape / Backspace close it) in `buy_menu::menu_input`, the
  input freeze while open, the `buy:C`-style test actions.
- New shared helper: `buy_menu::shop_price(item, inv, vet)` (price and
  weight as shown, the half-price dual rule); `sale_rows` uses it, so the
  KF list shows exactly what it showed before.
- Per menu: the screen and its navigation. `BuyMenu.kind` says which one
  draws and reads the keys. The KF text menu's code is untouched except
  that it stands aside when `kind` is NuMenu.
- The catalogue gains comparable stats per weapon, read from the class
  defaults like the rest (`ShopStats`: damage per hit and hits per shot
  from FireModeClass[0]'s DamageMax / MeleeDamage / ProjectileClass Damage
  and ProjPerFire, FireRate, the weapon's MagCapacity, the ammo class's
  MaxAmmo). Display only; no game rule reads them.

Multiplayer: every player buys on their own game with their own dosh;
no shop message goes over the network (MULTIPLAYER.md, net/). So both
menus behave the same in solo and network games.

### Layout (1280x720 and up; sizes scale with the screen height)

```
+----------------------------------------------------------------------------------+
| TRADER   wave 2/4 starts in 0:42   [perk icons 1-7, yours lit]   CLOSE (Esc)     |
| DOSH £ 1250        CARRY [#########------]  9 / 15 kg                            |
+-------------------------+------------------------------+-------------------------+
| YOUR GEAR               | SHOP                         | SELECTED                |
| 9mm Tactical            | * SUPPORT  your perk  -30%   | AA12 Shotgun            |
|   ammo [####--] 120/240 |   Shotgun        £350   8 kg | Support  -30%: £2800    |
|   fill £80   sell --    |   AA12 Shotgun  £2800  10 kg | Damage    [####--] 40x10|
| Knife                   |   ...                        | Fire rate [###---] 5/s  |
| Frag grenades  3/5      |   MEDIC                      | Magazine  [##----] 20   |
|   +1 £40                |   MP7M Medic Gun £825  3 kg  | Total ammo[###---] 80   |
| Combat armour  25/100   |   ...   (OWNED, TOO HEAVY,   | Weight    [######] 10kg |
|   fill £225             |    NEED £ marks per row)     | Price     [####--] £2800|
|                         |                              | After: £1250 -> £-1550  |
| [REFILL ALL  £245  A]   |                              |  carry 9 -> 19 / 15 kg  |
| [ARMOUR      £225  V]   |                              | [ BUY  Enter/B ]        |
| [GRENADE     £40   G]   |                              |                         |
+-------------------------+------------------------------+-------------------------+
| last action: "Bought Shotgun for £350"      keys: Up/Down Tab 1-8 B S R C A V G   |
+----------------------------------------------------------------------------------+
```

- **Top bar:** wave and countdown, dosh in large type, the carry weight
  as a bar (green when there is room, amber when 2 kg or less are left,
  red when full), the seven perk icons (click or Shift+1-7 to change
  perk, KF's rule: once per trader time), CLOSE.
- **Your gear (left):** every owned weapon with its ammo as a bar
  (current / max), the fill-ammo price ("FULL" when full) and the sell
  value ("--" when it cannot be sold); grenades show "+1 £x"; the
  armour row with points / 100 and its fill price. Below: three buttons
  REFILL ALL (the sum of every fill price), ARMOUR (fill price),
  GRENADE (one grenade).
- **Shop (middle):** every weapon for sale, grouped by perk. Your perk's
  group comes first, starred, with its discount ("-30%"); the other
  groups follow in KF's order (Medic ... Demolitions, Neutral). Each row:
  name, price after discount, weight. A row's state shows as colour and
  a tag:
  - can buy: bone-white name, green price;
  - OWNED: dim, tag "OWNED" (selecting it shows sell / refill);
  - too expensive: price in red, tag "NEED £x" (how much is missing);
  - too heavy: weight in amber, tag "TOO HEAVY" (when both, too heavy
    is shown, because selling may fix the dosh but not the weight).
  Singles are hidden while their duals are owned (as KF). The list
  scrolls (wheel, or following the selection).
- **Selected (right):** the weapon's name, perk group and discount, then
  bars that compare it with the whole shop: damage per shot (damage x
  pellets), fire rate (shots per second), magazine, total ammo, weight,
  price. Bars use the square root of value / largest value in the shop,
  so a pistol is still visible next to the L.A.W.; the real number is
  printed next to each bar. Then "after buying": dosh before -> after and
  carry weight before -> after (red when it does not fit). Then the
  action buttons: BUY (not owned), or SELL / FILL AMMO / +1 MAG (owned).
  With the armour row selected: armour points and the fill price.
- **Footer:** the last action's result in plain words (green done, red
  refused, with the reason) and the key help.

Colours: a near-black translucent backdrop over the game, panels
slightly lighter, thin dark-red rules, KF's bitmap fonts (ROBtsrmVr)
and perk icons, bone-white text; green / amber / red only for states.

### Controls

| Key | Action |
| --- | --- |
| Up / Down | move the selection (in the focused column) |
| Tab, Left / Right | switch column (gear <-> shop) |
| PageUp / PageDown | previous / next perk group in the shop |
| 1-8 | jump to the shop's n-th group (1 = your perk's) |
| Enter or B | buy the selected shop weapon (on gear: fill its ammo) |
| S | sell the selected weapon |
| R | fill the selected weapon's ammo |
| C | buy one magazine for the selected weapon |
| A | refill all ammo |
| V | buy / fill armour |
| G | buy one grenade |
| Shift+1-7 | change perk (as the KF menu's 1-7) |
| E, Escape, Backspace | close (shared with the KF menu) |
| Mouse | click a row to select it, click buttons, wheel scrolls the shop |

While the menu is open the mouse cursor is shown and freed; it is
captured again when the menu closes (if it was before).

### Test actions and logs

Scripted (`--input FRAME:ACTION`): `nu:up`, `nu:down`, `nu:tab`,
`nu:enter`, `nu:group:N`, `nu:prev_group`, `nu:next_group`,
`nu:select:CLASS` (select a shop weapon by class, e.g.
`nu:select:Shotgun`; an owned weapon not in the shop selects its gear
row), `nu:buy`, `nu:sell`, `nu:fill`, `nu:clip`, `nu:fill_all`,
`nu:armour`, `nu:grenade`, `nu:close`, `nu:click:ID` (as a mouse click on
a box drawn last frame: `nu.buy`, `nu.sell`, `nu.fill`, `nu.clip`,
`nu.fill_all`, `nu.armour`, `nu.grenade`, `nu.close`, `nu.shop:N`,
`nu.gear:N`, `nu.group:N`, `nu.perk:N`), `nu:wheel:N`. The menus'
`menu_dump` logs every box drawn.
`end_wave` (waves.rs, for tests): the running wave ends now (no more
spawns, every zed dies), so the trader opens a few seconds later.

Log lines: `numenu_open` (dosh, weight, perk, discount, rows),
`numenu_select`, `numenu_buy weapon= price= dosh_after= weight_after=`
(written the frame after the request, from the real result),
`numenu_sell`, `numenu_ammo`, `numenu_fill_all count= cost=`,
`numenu_armour`, `numenu_refused action= weapon= reason=` (reason from
the same rules: too_expensive, too_heavy, owned, full, not_sellable,
or `unknown` when the shared logic refused for another reason),
`numenu_close`, `numenu_layout` (font sizes, panel boxes).

### Steps

- **NU1** `--trader-menu`, `BuyMenu.kind`, `shop_price`, catalogue
  stats, `end_wave` test action. KF menu unchanged (check: same log
  lines as before with `--trader-menu kf`).
- **NU2** NuMenu screen (drawn with the menus' painter and node pool).
- **NU3** NuMenu input: keys, mouse, test actions, logs.
- **NU4** Launcher choice "Trader menu" (saved as `trader=nu|kf`).

### As built (2026-10-07; headless runs, not played by you)

- `src/game/numenu.rs`: the model (`model`: groups, rows, states;
  `fill_all_plan`), the input system (PreUpdate, before
  `buy_menu::menu_input`, which still clears the keys afterwards), the
  drawing (called from `menus/mod.rs` `draw_menus` with the menus' painter
  and node pool, so it uses KF's fonts and perk icons and sits above the
  HUD). `buy_menu.rs`: `MenuKind`, `BuyMenu.kind`, `shop_price`,
  `single_hidden`, `ShopStats` (+ a `shop_stats` log line at map load).
  `main.rs`: `--trader-menu`. `waves.rs`: test action `end_wave`.
  Launcher: "Trader menu" row in "Display, sound, menus" (column 3's rows
  shrink a little on short windows so it fits at 720p).
- REFILL ALL fills every weapon's ammo, the grenades and the M4 203's
  grenades (one `ShopRequest::Ammo { fill: true }` each, in inventory
  order); short of dosh, the shared logic buys what it can, in that order.
- Prices are shown as KF shows them (int of the float): 500 x 0.6 shows
  £299 and charges 299.99997, as KF (so `numenu_buy price=300` after
  rounding the dosh difference).
- The KF menu's log lines are identical before and after, except
  `buy_menu open=true` now ends with `kind=kf|nu`.
- Not tested: real keys and a real mouse (the virtual display has none:
  the keys go through the same commands as the `nu:` actions, the clicks
  through the same boxes as `nu:click:`); the cursor being freed and
  captured again; a joining client's shopping (a host game was run).

## Release builds: Linux, Windows, Flatpak (planned and built 2026-10-07: R1-R3)

Goal: build files we can attach to a GitHub release, for several platforms,
from this Linux machine. Releases contain only our program. Killing Floor's
files are never bundled: the game finds the player's own install at run time
(`KF_ROOT`, then `references/killing_floor`, then Steam, see
`crates/ue-assets/src/install.rs`).

- **R1, Linux Nix package** (`nix build .#open-kf`): the game built by Nix,
  with the window/sound/GPU libraries written into its library path (rpath),
  as the dev shell does. For Nix users; also the base the others copy.
  Built with crane (a Nix library that builds Rust projects, caching the
  dependencies separately so rebuilds are fast).
- **R2, Windows** (`nix build .#windows`): cross-compiled (built on Linux for
  Windows) to `x86_64-pc-windows-gnu` with the MinGW compiler from nixpkgs.
  The Rust compiler comes from rust-overlay (prebuilt Rust with the Windows
  standard library, so nothing big is compiled from source). Output:
  `result/bin/open-kf.exe` plus any DLLs it needs, and a zip for the release.
- **R3, Flatpak** (`nix run .#flatpak`): Flatpak apps run on Flatpak's own
  runtime (`org.freedesktop.Platform`), not on Nix's libraries, so a Nix-built
  binary cannot be put in one. The script uses `flatpak-builder` (from nixpkgs)
  with the manifest in `packaging/flatpak/`, which compiles the game inside the
  Flatpak SDK with its Rust extension. Everything it downloads (runtimes, build
  cache) is kept in `work/flatpak/` (via `FLATPAK_USER_DIR`), not in your home
  flatpak folder. Output: `work/flatpak/open-kf.flatpak`, a single file people
  install with `flatpak install --user open-kf.flatpak`.
  - The game writes `logs/`, `settings/` and `work/` relative to the current
    folder. Inside the sandbox the app starts in its own data folder
    (`~/.var/app/io.github.scronkfinkle.OpenKF/data`) so those writes work.
  - Sandbox permissions: window (Wayland/X11), GPU, sound, network (for
    multiplayer), and read-only access to the Steam folders where KF lives.
  - The build downloads crates with network on (allowed for our own bundle;
    Flathub itself would need a pre-generated crate list, not done).
- Not in this step: GitHub Actions to build on each tag (can come next).

Results (2026-10-07):
- The Linux and Windows binaries are stripped of debug symbols in the Nix
  builds only (about 40% smaller; `cargo build` keeps them for backtraces).
  Linux 142 MB; Windows exe 147 MB, zip 50 MB.
- The Windows exe imports only DLLs that ship with Windows (checked with
  `objdump -p`), so the zip holds just the exe and the licences. It is a
  console program: on Windows a console window opens next to the game,
  showing the log.
- Under Wine on the virtual display, winit (the window library) panics
  listing monitor modes (`has_flag(mode.dmFields, REQUIRED_FIELDS)`) because
  of Wine's fake display modes. In a Wine virtual desktop
  (`wine explorer /desktop=kf,1920x1080 open-kf.exe`) it runs.
- The Flatpak uses runtime 26.08. Bundle 23 MB. Symlinks outside the granted folders do not resolve in
  the sandbox (a test with `references/killing_floor` failed until the
  real path was given).
- `scripts/headless.sh` takes `HEADLESS_BIN=path` to test a release build.

Steam install search (2026-10-07): after `KF_ROOT` and
`references/killing_floor`, `Install::discover` asks the steamlocate crate
for every Steam install (Linux: native, Flatpak, Snap; Windows: the
registry) and looks up app 1250 in each one's library list
(`libraryfolders.vdf`), so libraries on other drives are found. This
replaced the fixed list of Steam paths and the Flatpak wrapper's `KF_ROOT`
setting. If nothing is found, the error lists each Steam install and why
(not installed, or no Steam). Inside the Flatpak, a library outside the
granted Steam folders cannot be read until the player grants it with
`flatpak override`.

## Release CI on GitHub (planned and built 2026-10-07: C1-C3)

Goal: pushing a version tag (e.g. `v0.2.0`) on a commit that is on `main`
builds the release files and publishes a GitHub release with them.
Workflow: `.github/workflows/release.yml` (GitHub Actions: GitHub's own
build machines, which run the steps in that file).

- **C1, check:** runs on `push` of tags `v*`. Fails unless the tagged commit
  is on `main` (`git merge-base --is-ancestor`), so a tag on a side branch
  publishes nothing.
- **C2, build (two jobs side by side, Ubuntu machines with Nix installed):**
  - `windows`: `nix build .#windows`, uploads `open-kf-windows-x86_64.zip`.
  - `flatpak`: `nix run .#flatpak`, uploads `open-kf.flatpak`. Ubuntu
    24.04 blocks the sandbox tool (bubblewrap) from Nix by default
    (AppArmor's user-namespace restriction), so the job switches that
    restriction off first with `sysctl`.
  - No Linux binary in the release: the Nix one only runs on machines with
    Nix (its libraries are paths in `/nix/store`). Linux players use the
    Flatpak; Nix users can `nix run github:Scronkfinkle/open-kf`. (A `nix`
    job that only checked `nix build .#open-kf` was dropped 2026-10-07: the
    slowest job, 34 min, and not needed.)
- **C3, release:** after all builds pass, `gh release create <tag>` with
  both files and GitHub's generated notes (the commit list since the last
  release).
- Also runnable by hand (`workflow_dispatch`, "Run workflow" on GitHub or
  `gh workflow run release.yml`): builds and uploads the files to the run
  page, but publishes no release. For testing the workflow.
- No build cache between runs at first (each run builds from scratch,
  about 10-15 minutes per job). Can add one later if it is too slow.
- `.gitignore` allows `.github/workflows/*.yml`.

Results (2026-10-07, hand-started run 37684199947): every job passed on the
first try (release skipped, as intended). Times: windows 18m36s, flatpak
19m52s, nix 34m22s; the release waits for the slowest. The downloaded
Flatpak (22 MB) installed and ran headless on KF-WestLondon (install found
through Steam, `exit=Success`); the Windows zip (48 MB) holds the exe and
both licences. Not run yet: the release job (needs a real tag).

## Players blocking each other in network games (planned and built 2026-10-07: PC1-PC2)

**Problem.** In a network game players walk through each other. KF's
pawns block each other: Engine.Pawn has bCollideActors and bBlockActors
true, KFPawn CollisionRadius 20, KFHumanPawn CollisionHeight 50 (class
defaults, `kfpkg defaults`), and nothing in KFMod switches blocking off
between team mates. Our walk (`player/walk.rs`) already clips the
player's move against pawn cylinders (`player/pawn_collision.rs`, from
"Play-test fixes after combat", step 1), but it is only given the zeds.
The other players' pawns exist on every game (`net/pawns.rs`: a
`PawnState` with `local: false`, drawn where that player was 0.1 s ago)
but were never added to that list. The host's zeds already treat the
other players as blockers (`zeds/zed/think.rs`).

**How the network splits the work.** Our games are client-authoritative
(see `docs/multiplayer-prototype.md`, step 2): each game moves its own
player and the others believe it; the host does not move the clients'
pawns, so there is no server correction (`ClientAdjustPosition` in KF)
and nothing to rubber-band. "Blocking on the server" therefore means:
every game, the host included, blocks its own player against every other
player's pawn as it sees it. KF's real model is server-authoritative
(the server re-runs each move and blocks it); that stays later work.

**PC1: the other players block your walk.** Every frame, each living,
active remote pawn (`PawnState` not local, not dead) becomes a cylinder
(radius 20, half-height 50) in the walk's list next to the zeds, so
`clip_move` stops you at 2 x 20 + 0.5 = 40.5 units (centre to centre)
and slides you round them, on the ground and in the air. You collide
with the body you see (its drawn, 0.1 s old position). Works for any
number of players (one cylinder per remote pawn). Standing on another
player's head is not handled (as with zeds): KF makes you jump off a
pawn you land on (Pawn.BaseChange -> JumpOffPawn); not done.

**PC2: pushing apart when overlapping.** Since each game sees the other
players 0.1 s late, two players walking into each other head-on can end
up overlapping a little (each stops against where the other *was*), and
two players can also be put on the same spot (a respawn). `clip_move`
only stops further moves inward, so they would stay overlapped. Fix:
when my cylinder overlaps a remote one, my game moves me outward by half
the overlap (the other game moves its player by the other half), at most
`SEPARATE_SPEED` 50 units/s, through the world sweep so walls still
stop it. Overlaps up to 1 unit are left alone (a deadband: a player
stopped by `clip_move` rests exactly at contact, and rounding made it
push back and forth by under a unit). Exactly on the same spot, the player with the lower peer id
goes one way and the other the opposite way. **This is our own rule, a
guess:** KF never lets pawns overlap (its native movement refuses the
move), so it has no such push. The speed cap keeps it smooth; it can
leave the two a few units further apart than 40.5 (each keeps pushing
until it sees the other's push, 0.1 s later).

**Logs.** `walk_blocked by=player <peer>` (as for zeds), and
`pawn_contact` 10 times a second while another player is within 60
units: `peer`, centre distance in Unreal units, overlap, how far this
game pushed its player apart in that time.

**Test.** Host and client on one machine, headless, `--mode debug` (no
zeds to grab anyone), warped facing each other on KF-WestLondon's road
(X -4090, Y 100 and 2200), both holding forward (`warp:X;Y;Z;YAW`,
`walk_on`). Scripts (untracked): `work/pc/mp_collide.sh` (env `TAG`,
`HOST_IN`, `CLIENT_IN`, `HOST_FRAMES`, `CLIENT_FRAMES`, `MODE`) and
`work/pc/analyse.py TAG`.

**Results (headless, one machine, 127.0.0.1, 2026-10-07; not played by
you).**
- One player standing, the other walking into them: the walker stops at
  40.5 units (centre to centre) on both games' logs and stays there;
  walking in slightly off-centre, it slides round the standing player's
  side (`walk_blocked by=player <peer>`).
- Head-on, both holding forward: at the impact both games briefly see
  an overlap (largest 20.8 units, smallest distance 19.7: the 0.1 s
  delay), pushed apart within about 0.5 s, then held at 40.0-40.5
  units for the rest of the 12 s push (both games agree on both
  positions within a unit). Settled (distance staying within 40.5 +- 1)
  after 1.1 s (host) and 0.45 s (client); each player moved 38 / 43 units
  in total while close, 6 / 7 direction changes.
  The first try used 100 units/s and no deadband: settled after 1.2 s
  but shuffled back and forth all along (139 units moved, 40 direction
  changes); hence 50 and the deadband. Reason: each game removes the
  whole overlap it sees, not half, because it sees the other's push
  only 0.1 s later; a slower push overshoots less.
- On top of each other (one player warped 4 units from the other):
  pushed apart in opposite directions within 0.6 s, ending 47.4 units
  apart (7 more than contact: the overshoot above), on both games.
- Single player: no change (zeds still block, no `pawn_contact` lines).

## The Siren's scream destroys explosives (planned 2026-10-07: SX1-SX3)

### What KF does (from the scripts and class defaults)

- `ZombieSiren.SpawnTwoShots` (each scream pulse): unless zapped, and
  unless her target is a door (then only the door is hit, `ScreamDamage x
  0.6`), she calls her own `HurtRadius(ScreamDamage, ScreamRadius,
  SirenScreamDamage, ScreamForce, Location)`.
- `ZombieSiren.HurtRadius` uses `VisibleCollidingActors`: every colliding
  actor within `ScreamRadius` (700) of her centre that a line from her
  centre reaches without hitting the level (a "FastTrace": level only,
  pawns do not block). Zeds are skipped. Each takes
  `TakeDamage(damageScale x ScreamDamage)` with `damageScale = 1 -
  max(0, (distance - CollisionRadius) / ScreamRadius)` (an int).
- `ScreamDamage` is 8, x `DifficultyDamageModifer` (`KFMonster.
  PostBeginPlay`: Beginner 0.3, Normal 1.0, Hard 1.25, Suicidal 1.5, Hell
  on Earth 1.75; x 0.75 alone): never more than 14.
- Projectiles' `TakeDamage` with `SirenScreamDamage`:
  - `Nade` (the frag; `FlameNade` and `MedicNade` are subclasses): if the
    attacker is a `Monster` or the thrower, `Disintegrate`. `MedicNade`
    has its own `TakeDamage` that only disintegrates; it still does so
    after it went off, which ends its healing cloud (its `Timer` destroys
    it 0.1 s later).
  - `M79GrenadeProjectile` (M79, M32, M203 grenades) and `LAWProj` (LAW
    rocket; also the Husk Gun's and the ZED guns' projectiles, which are
    `LAWProj` subclasses without their own `TakeDamage`): always
    `Disintegrate`, duds too.
  - `PipeBombProjectile`: returns (nothing happens) if the damage is
    under 25; otherwise `Disintegrate` if it is 5 or more. Since the
    scream does at most 14, **a vanilla Siren never destroys a pipe
    bomb**. (KF's later DLC launchers, `SPGrenadeProjectile`,
    `SealSquealProjectile`, `SeekerSixRocketProjectile`, also
    disintegrate; we do not have those weapons.)
- `Disintegrate` (the same in every class): the projectile is hidden at
  once and removed 0.1 s later, **with no explosion and no damage**; it
  plays `DisintegrateSound` (`Inf_Weapons.panzerfaust60.
  faust_explode_distant02`, at volume 2.0) and spawns the
  `KFMod.SirenNadeDeflect` emitter at the hit point, facing up.
- Not part of this: `HealingProjectile` (the medic gun's dart) explodes
  on any damage, so a scream would pop a dart in flight; not done.

### Our design

- Each scream pulse already sends a `DoorBlast` message with `source:
  "siren_scream"` (doors and glass read it). A new system in
  `weapons/projectile.rs`, `scream_explosives`, reads those messages too
  and checks every player grenade/rocket (`PlayerExplosive`) and every
  thrown frag or pipe bomb (`PlayerThrown`): within the radius (from the
  projectile's centre, a guess for the engine's exact range test; only
  the pipe bomb has a size, 8 units), and a clear line from the Siren
  through the level. The rule per class is one small function so a unit
  test can check it.
- Disintegrate: the projectile entity is removed at once (KF hides it at
  once; the 0.1 s until it is destroyed changes nothing we draw), its
  trail is stopped, the sound and the `SirenNadeDeflect` effect play.
- Log line per projectile the scream reaches or misses:
  `scream_explosive result=disintegrated|ignored|blocked|out_of_range
  kind=frag|pipe|grenade weapon=... id=... at_unreal=(x, y, z)
  distance_unreal=... damage=... zed=...`.
- Only the host's (or single player's) own projectiles: a network
  client's grenades live on the client and the host's Siren does not
  reach them (not handled).

### Steps

- SX1: load `DisintegrateSound` per projectile class; add
  `KFMod.SirenNadeDeflect` to the effect library.
- SX2: `scream_explosives` and its rule function, with unit tests.
- SX3: headless runs: a Siren screaming at the player with a frag, a
  pipe bomb and a grenade nearby; read the log.

As built (2026-10-07; headless runs, not played by you): SX1-SX3 done as
above. Frags (resting and in flight) and an M79 dud were destroyed, a
frag 857 units away survived and exploded on its fuse, a pipe bomb was
ignored on every pulse (damage 5). Results and commands in MODLOG.md.

## Volume control: in the pause menu and the launcher (planned 2026-10-07: VC1-VC3)

**Goal.** Change how loud the game is while playing (from the pause
menu), hear the change at once, and find the same volume next time,
whether the game is started from the launcher or from the command line.
The launcher shows and sets the same saved values.

**What KF has (read 2026-10-07).** KF's Settings page has an Audio tab
(KFGui.KFAudioSettingsTab, PanelCaption "Audio"). Its "Sound System"
box (AudioBK1) holds two sliders: "Music Volume" (AudioMusicVolume) and
"Effects Volume" (AudioEffectsVolumeSlider), both moSlider with
MinValue 0 and **MaxValue 0.5**. Moving one writes the ini at once
(`set ini:Engine.Engine.AudioDevice MusicVolume` / `SoundVolume`,
InternalOnChange) and the music changes immediately (SetMusicVolume).
The values start from System/KillingFloor.ini [ALAudio.ALAudioSubsystem]
(shipped MusicVolume 0.1, SoundVolume 0.3; your ini has the same). KF
has no master volume. GUISlider: a click or a drag sets the value from
the mouse's x position (InternalCapturedMouseMove), Left / Right keys
move it 1% of the range (Adjust(0.01)). Look (RO styles, assumed to be
the ones KF uses): bar InterfaceArt_tex.Menu.SliderBarDisabled, knob
SliderGripBlurry (26 x 13), fill SliderFillBlurry.

**Our mixer has a clean split.** The sound effects (our voice mixer)
use `Audio::sound_volume`, the music uses `Audio::music_volume`; both
are read every frame (update_voices, run_music), so changing them is
heard at once (the voices' volume is ramped over one 256-sample block,
so no clicks). `--mute` silences the speakers after everything is mixed
(capture.rs), so it keeps overriding any volume.

**The three sliders.**

- *Master Volume* (ours, not KF's): 0 to 1, default 1; multiplies both.
- *Effects Volume* (KF's): 0 to 0.5; default: the install's ini value.
- *Music Volume* (KF's): 0 to 0.5; default: the install's ini value.

Heard volume: sounds = master x effects, music = master x music.

**Where it is saved: the launcher's file.** `settings/launcher.txt`
(the launcher's saved choices) gets three more lines:
`volume=1.00`, `effects_volume=default|0.300`,
`music_volume=default|0.100` ("default" = use KillingFloor.ini). The
game reads them at start (also when run without the launcher) and
rewrites only these three lines when a slider is let go, keeping every
other line as it was (or creates the file with only these lines). The
launcher reads them on opening and writes them with its other choices
on PLAY, so both always use the same values. The game takes
`--settings FILE` like the launcher (tests use it to keep away from your
file); the launcher passes its own `--settings` on to the game. We never
write KF's own KillingFloor.ini (the install is read-only).

**In the game: Settings in the pause menu.** The pause menu's Settings
button (inert until now) opens an "Audio" window above the pause menu
(KF opens its Settings page there), with the "Sound System" box and
the three sliders, and a Back button. Escape or Back returns to the
pause menu. The mouse drags a slider; Up / Down pick a slider, Left /
Right move it 1% of its range. A solo game stays paused while it is
open; a network game never pauses (as the pause menu). Sounds are not
stopped when Effects changes (KF does "stopsounds"; ours keeps them, so
you hear the change on what is playing).

**In the launcher: an "Audio" box** in column 3 under "Sound, menus"
(moved there from under the map list when the Graphics box was merged
into column 2: with both, the map list showed one row at 1280x800), with the same three
sliders (drawn with the same slider code). A small, separate box so it
merges easily with other launcher changes. (First tried between Play and
Player in column 1: at 1280 x 800 it pushed the Character row off the
Player box.)

**Logs.** `audio_volume master= effects= music= sound_gain= music_gain=
source=settings|default|menu|...` at start and on every change;
`volume_saved file= ...` when written; `sound_play` lines already carry
each sound's final gain (now including the master volume).

**Steps.**

- VC1. The saved values and the mixer: three fields in the launcher's
  choices, the game reads them at start, the "update three lines" save
  (unit tests), `--settings FILE` for the game, the logs.
- VC2. The pause menu's Audio window: a slider drawn like KF's, mouse
  drag, keys, test actions (`volume_page`, `volume:NAME=VALUE`,
  `volume_click:NAME@FRACTION`), saving on release.
- VC3. The launcher's Audio box.

**As built (2026-10-07; headless runs, not heard by you yet).** VC1-VC3
as planned. Files: `src/launcher/choices.rs` (`Volumes`, the three
fields, `with_volume_lines`), `src/launcher/mod.rs` (`read_volumes`,
`save_volumes`, the slider drag, `--settings` passed on),
`src/audio/mixer.rs` (`Audio::set_volumes` / `save_volumes`, the
`voices_regain` log), `src/audio/music.rs` (`music_gain` log),
`src/game/menus/audio_page.rs` (the window), `src/game/menus/mod.rs`
(page, drag, keys, test actions), `src/game/menus/gui.rs`
(`Painter::slider`, `slider_fraction`), `src/launcher/draw.rs` (the
box), `src/main.rs` (`--settings`). Details:

- The slider's marker (knob) is 1.4 x the box's height wide (a guess;
  KF's native drawing is not in the scripts); the GUISlider rule keeps
  half a marker at each end, so the first and last few pixels of the
  bar are the minimum and maximum.
- The window has a dark backing under KF's see-through frame (ours: the
  pause menu's text showed through).
- Test actions (game, `--input`): `volume_page` (or `pause_settings`)
  opens the window from the pause menu, `volume_back` closes it,
  `volume:NAME=VALUE` sets a slider on any page, `volume_click:NAME@F`
  clicks at fraction F of a slider's box, `volume_key:up|down|left|right`.
  Launcher: `volume_click:NAME@F`, and `set:volume=..`,
  `set:effects_volume=..|default`, `set:music_volume=..|default`. (rough order, to be planned in detail when reached)

2. **Walk around:** collision with BSP and static meshes, plus Unreal-style
   walking, jumping and gravity, with values taken from the scripts.
3. **Characters:** skeletal meshes and animations from `.ukx`, showing zeds
   standing in the map.
4. **Weapons:** first-person weapon meshes, firing, hit detection, damage.
5. **Zeds:** AI and pathfinding over the map's navigation points.
6. **Game loop (solo):** waves, trader, dosh, perks.
7. **Sound, music, HUD, menus.**

## Aim down sights: toggle or hold (planned 2026-10-07: AD1-AD3)

**Goal.** A setting for the aim button (right mouse, "iron sights" /
ADS = aim down sights): *Toggle* (press once to aim, press again to
stop) or *Hold* (aim only while the button is held). Changeable in the
pause menu (takes effect at once) and in the launcher, saved in the
same settings file as the volumes.

**What KF does (read 2026-10-07).** KF has both, as two key commands
(System/defuser.ini and your User.ini): `ToggleAiming` =
`ToggleIronSights` and `Aiming` = `IronSightZoomIn | onrelease
IronSightZoomOut`. The default binding is `RightMouse=ToggleAiming`,
so **KF's default is Toggle**; Hold means rebinding the key to
`Aiming`. Both go through the same rules (KFWeapon.uc):
ToggleIronSights / IronSightZoomIn refuse in the air (PHYS_Falling)
and while reloading or when CanZoomNow fails, and interrupt a
one-round-at-a-time reload first; IronSightZoomOut zooms out. Things
that drop the aim in KF: reloading (ReloadMeNow / ClientReload: ZoomOut),
putting the weapon down (PutDown: ZoomOut), opening the buy menu
(GUIBuyMenu.InitComponent: IronSightZoomOut), dying (the pawn and its
weapons are destroyed; the new ones start not aiming). The mid-game
(pause) menu does not zoom out in the scripts.

**What Open KF does now.** Toggle only (weapon/input.rs: right mouse
= ToggleIronSights), so the default stays the same.

**Rules (Toggle = KF; Hold = KF's `Aiming` plus our two choices).**

- Toggle: unchanged. A press aims or stops aiming. Anything that drops
  the aim leaves you not aiming; the next press aims again (one press,
  never two).
- Hold: pressing tries to aim (same refusals as KF). Letting go stops
  aiming. *Ours:* while the button stays held and you are not aiming
  (the press was refused, or a reload / switch / grenade / landing
  ended it), the game aims again as soon as it can (KF would need a new
  press). Refusals are logged on the press only.
- Both: the buy menu opening zooms out (KF). Dying zooms out (KF:
  the weapon is gone). A forced weapon change (throwing or selling the
  weapon in hand) zooms out (a gap until now: the aim flag survived the
  change). The pause menu: Toggle keeps the aim (KF); Hold drops it
  (*ours*: the menu takes the mouse, so the button counts as let go).

**Saved.** `settings/launcher.txt` gets one more line,
`aim=toggle|hold` (default toggle). The game reads it at start and
rewrites only that line when it is changed in the menu (the same
"rewrite only these lines" save as the volumes, made general); the
launcher saves it with its other choices on PLAY.

**In game.** The pause menu's Settings window (now titled "Settings")
gets a second box under "Sound System": "Controls", with one row
"Aim down sights" and two buttons, *Toggle* and *Hold* (the chosen one
lit, as the launcher's Solo / Host / Join). Up / Down reach the row,
Left / Right switch it. Test actions: `aim_mode:toggle|hold` (any page),
`aim_mode_click:toggle|hold` (clicks the button on screen),
`aim_down` / `aim_up` (hold / let go of the aim button; `aim` stays a
one-frame press).

**Launcher.** A small "Controls" box under the Audio box in column 3
(moved from the map column at the merge with the Graphics box), one row "Aim" with a `<` value `>` spinner (Toggle / Hold);
`set:aim=hold` for tests.

**Logs.** `aim_mode mode=toggle|hold source=...` at start and on every
change; `aim_saved file=...`; the existing `iron_sights weapon=..
aiming=true|false reason=..` lines get `mode=` and new reasons
(`hold_press`, `hold_retry`, `hold_release`, `menu`, `buy_menu`,
`death`, `force_switch`, `no_sights`).

**Steps.** AD1: the setting (choices field, general line save, game
resource, log) and the hold/toggle logic with the edge cases and test
actions. AD2: the pause menu row. AD3: the launcher box.

**As built (2026-10-07; headless runs, not played by you yet).** AD1-AD3
as planned. Files: `src/weapons/weapon/aim.rs` (new: `AimSetting`, the
resource read at start and saved on change), `src/weapons/weapon/input.rs`
(the aim block, `try_zoom_in`, the death zoom out, also on a debug-mode
instant respawn via the death counter), `src/weapons/weapon/inventory.rs`
(`force_change` zooms out), `src/launcher/choices.rs` (`aim_hold`, the
`aim` line, `with_lines` / `with_aim_line`), `src/launcher/mod.rs`
(`read_aim`, `save_aim`), `src/launcher/draw.rs` (the Controls box),
`src/game/menus/audio_page.rs` and `mod.rs` (the Controls box in the
Settings window, clicks, keys, test actions), `src/main.rs` (reads it).
Details:

- In Hold, a death with the button still held aims again on the next
  try (debug mode respawns at once, so the same frame): the button is
  held, so this is consistent, not stuck.
- Switching Toggle -> Hold while aimed (not holding the button) stops
  aiming at once (`hold_release`).
- The window's title is now "Settings" (it holds more than Audio).
- At 1280 x 800 the launcher's map list shows 5 rows (it scrolls).

## Combat physics fixes (planned 2026-10-08: CP-1 onward; details in the local RE.md)

An audit compared our projectile and ragdoll movement with KF's engine
code. Each step below is one commit.

**CP-1 Bouncing things fall at half gravity.** KF's engine moves a
falling object in small steps (at most 0.05 s). Each step it adds half
of gravity x the step time to the velocity and moves by the velocity.
For an object that does not bounce it then recomputes the velocity from
how far it actually moved and extrapolates to the end of the step, which
gives full gravity (950 units/s^2), and caps the speed at the zone's
terminal velocity (2500). For an object that bounces (bBounce: the frag,
fire and medic nades, the pipe bomb, nails after their first bounce)
that second part is skipped: the velocity only ever gains half of
gravity, so they fall at 475 units/s^2 and are never capped. A frag
thrown at 45 degrees therefore flies twice as far on its first arc as a
full-gravity throw would. We now use 475 for those (thrown objects and
bounced nails); 950 stays for everything else that falls.

**CP-2 Ragdoll start spin.** KFMonster computes the death spin as
RagInvInertia x (hit offset x push). The engine reads that number in
Unreal rotation units per second (65536 = one full turn) and converts it
to radians per second; we had treated it as an unknown unit and scaled
it so far up that almost every corpse spun at the 10 rad/s cap. The
engine also gives every body part the same spin and a velocity of push +
spin x (part - the zed's cylinder centre), so the spin turns around the
cylinder centre, not the pelvis. A typical Clot kill now starts at about
1.5 rad/s. With no hit to go by, KF uses a random direction x 18000
rotation units/s (1.73 rad/s) and no push; ours did not spin at all.

**CP-3 M79, M32 and M203 grenade flight.** These grenades use Red
Orchestra's "true ballistics" while their propellant lasts (0.25 s); the
LAW, Husk Gun and ZED guns switch it off and fly straight. Each frame:
- A start-up "fudge" scales speed and movement from 2.5% up to 100% over
  the first 0.1 s (so the grenade cannot pass through something right in
  front of the muzzle). It covers about 350 units in that time, not 800.
- Drag: the engine works in feet (18.4 units = 1 foot). It takes
  (speed in ft/s)^2 x G1(Mach) / 0.3 x dt x 0.00384 off the speed, where
  G1 is the standard drag table (0.2155 at the M79's Mach 0.39). It
  subtracts that number straight from the speed in units, without
  converting it back; we copy that. About 520 units/s^2 at 8000.
- Gravity 591.45 units/s^2 (32.144 ft/s^2), times the fudge.
When the propellant runs out the grenade falls like any non-bouncing
object: full gravity, and its speed is capped at the zone's terminal
velocity, 2500, on the first falling step (from about 7900). It
therefore drops much sooner than before: 300 units below the muzzle after
about 3500 units of flight instead of 8400.

**CP-5 Explosions push surviving zeds.** From the scripts: a zed that
survives a hit loses the hit's push (momentum) unless the damage type is
exactly the frag's, the pipe bomb's or the M79 / M32 / M203's (also the
Dwarf axe, SP grenade, Seal Squeal and Seeker Six, which we do not
have; the LAW, Husk Gun and fire nade are not on the list). The blast's
push is damage scale x MomentumTransfer (frag and pipe 100000, M79
75000) along the line from the blast to the zed's centre. On the ground
the upward part is raised to at least 0.4 x the push's size; then it
is divided by the zed's Mass (Clot, Crawler, Stalker, Siren 100;
Gorefast 350; Bloat, Husk 400; Scrake 500; Fleshpound 600; Patriarch
1000). Pushes of 50 units/s or less do nothing. Otherwise the zed starts
falling; if it already rises faster than 380 the upward part is halved;
and the push adds to its velocity. We apply it through the zeds' existing
falling movement, only to zeds that are walking, idle, attacking,
falling or landing. Zeds that are knocked down, raging, door-bashing or
in a Patriarch move are not pushed (a simplification). Log:
`zed_knockback`.

**Shotgun pellets and the extended cylinder.** Big zeds (Clot, Gorefast,
Bloat, Siren, Husk, Scrake, Fleshpound, Patriarch) carry a second
collision cylinder, the "extended" one, at head and shoulder height. In
KF it is a separate actor that passes any damage on to its zed. KF's
engine touches every actor a moving projectile crosses, once each. A
shotgun pellet (or a Trenchgun pellet or a nail) that crosses both
cylinders therefore damages the zed twice and loses PenDamageReduction
twice. The touch on the extended cylinder gets no pellet headshot
multiplier, because that cylinder is not the zed itself; the damage
type's headshot rule still applies. Crossbow and M99 bolts ignore a
zed, and anything attached to it, once they have hit it, so they hit
each zed once (ours already did that). We now do the same for pellets.
A shotgun pellet through a Scrake's chest does 35 + 17.5 and stops,
where before it did 35 + 17.5 to two different zeds. The tracer's end
point counts both touches too. Log: `projectile_hit ... cylinder=main
|extended`.

## Open questions

- Exact Unreal-unit-to-metre scale (step 0 picks a value; milestone 2 confirms
  it against player height and movement speed).
- ~~Whether every `.u` class includes its source text~~: yes, all 3757 do.

## Animation playback rules (planned 2026-10-08: ANIM-1, 8, 7, 3, 2)

KF's engine plays every skeletal animation with the same rules; these were
read from the engine itself (details in the local RE.md) and we follow them
for zeds and the first-person weapon. The player's third-person body only
gets ANIM-1 in this pass. One commit per step, in this order:

- **ANIM-1: a one-shot ends on its last key.** An animation of N frames has
  keys at frames 0 .. N-1. Played once, KF stops it on frame N-1 and holds
  that pose (that is when AnimEnd fires). We ran to frame N, which is the
  end of the "last key back to the first" segment used by loops, so a
  finished one-shot showed its *first* pose (e.g. the 9mm popped back up for
  about 0.06 s at the end of PutDown). Loops still run through all N frames.
  Zed "animation done" checks use the last frame too. Log:
  `weapon_anim_end ... held_frame=`.
- **ANIM-8: notifies at time 0 never fire.** KF fires a notify (a timed event
  in the animation) only when the frame goes past it, from strictly before
  to at-or-after; a sequence starts on frame 0 (or a hair after), so a
  notify sitting exactly at 0 never fires. Ours counted from -1. Affects 7
  notifies in the whole game (some player footsteps, a Crawler idle).
- **ANIM-7: the zed flinch layer tweens in and fades out.** The upper-body
  hit animation moves from the pose on screen to its first frame over
  0.1 s, plays, holds its last key, and its weight then fades linearly from
  1 to 0 over 0.12 s. A new flinch during the fade starts at full weight.
- **ANIM-3: zed walk speed and direction.** While moving, the movement
  animation plays at rate speed / (class default GroundSpeed x 1.1) (no
  clamp; the class default, not the zed's randomised or raging speed), and
  the direction picks one of four animations: forward if the movement
  direction is within about 35 degrees of facing (dot > 0.82), backward if
  within 35 degrees of behind, else left or right. A missing animation keeps
  the current one playing, as KF does. Log `zed_anim ... rate= dir=`.
- **ANIM-2: tweens.** Starting an animation with a tween time T moves every
  bone linearly (in time) from the pose last shown to the new animation's
  first frame over T seconds; only then does the animation clock start, and
  no notifies fire during the tween. Times used: zed actions and movement
  0.1 s, zed idle 0.25 s; weapon idle 0.2 s, reload 0.1 s, fire animations
  the fire mode's TweenTime (0.1 by default), select / put down / fire loop
  0. A looping animation asked for again while it plays only changes its
  rate. Because the clock waits, zed claw hits and attack ends come 0.1 s
  later than before (as in KF). Logs: `zed_anim_tween`, and the hit line's
  `since_anim_start=`.

## The classic KF trader menu (planned 2026-10-08: CT1-CT3)

`--trader-menu kf` used to show a text list standing in for KF's
GUIBuyMenu. This step draws it the way KF does (reference screenshot,
untracked: `references/classic_trader_menu.jpg`, 2560 x 1440), with the
menus' painter (`menus/gui.rs`: KF's textures and bitmap fonts, read from
the install at startup). The shop rules do not change: the menu still
only sends `ShopRequest` / `BuyVest` / `PerkRequest`, and
`weapons/weapon/inventory.rs` / `player/armour.rs` decide. NuMenu stays
the default.

**Where the layout comes from.** Every piece is a GUI component saved in
`System/KFGui.u` (read like the lobby's, log `menu_layout`); its class
defaults (`kfpkg defaults KFGui.<Class>`) give the list drawing numbers.

| Piece | KF component / class | Textures |
|---|---|---|
| page background | `GUIBuyMenu.PageBackground` | WhiteSquareTexture tinted 20,20,20 |
| header boxes | `GUIBuyMenu.HBGLeft / HBGCenter / HBGRight` | `Thin_border` |
| quick perk select (left header) | `GUIBuyMenu.QS` (`KFQuickPerkSelect`, `PB0-5`, `PSI0-5`), label `HBGLL` | `Perk_box`, `Perk_box_unselected`, perk OnHUDIcons |
| Perk / Store buttons | `GUIBuyMenu.PerkTabB / StoreTabB` | KF_SquareButton (`Button`, `button_Highlight`) |
| trader time, wave | `GUIBuyMenu.Time / Wave` (`UpdateHeader`) | - |
| current perk, filter icons (right header) | `GUIBuyMenu.Perk`, `GUIBuyMenu.filter` (`KFBuyMenuFilter`, `PSI0-8`, `RealignIcons`) | `Perk_box_unselected`, perk icons, `No_Perk_Icon`, `Favorite_Perk_Icon` |
| the store panel | `KFTab_BuyMenu`, docked under `GUIBuyMenu.PageTabs` | - |
| inventory (left) | `KFTab_BuyMenu.Inv`, `SaleB` ("Sell Weapon"), `MagB/MagL`, `FillB/FillL`, `InventoryBox` (`KFBuyMenuInvList.DrawInvItem`, 11 rows: 7 weapons, "Equipment", knife, grenades, armour) | `Thick_border_Transparent`, `Item_box_box/bar` (+ `_Highlighted`), `Innerborder_transparent`, `Button`, `button_Highlight`, `button_Disabled` |
| money | `KFTab_BuyMenu.MoneyBack`, `Cash`, `Money` | `Thin_border_Transparent`, `PatchTex.Statics.BanknoteSkin` |
| selected item info (middle) | `KFTab_BuyMenu.Item`, `SelectedItemL`, `ItemInf` (`GUIBuyWeaponInfoPanel`: `INameBG/IName`, `IImage`, `PowerCap/RangeCap/SpeedCap`, `PowerBar/RangeBar/SpeedBar` (`GUIWeaponBar`), `LWeightBG/LWeight`), `SaleValueBG/SaleValue` | `Med_border_Transparent`, `Innerborder_transparent`, `progress_bar`, the pickup's TraderInfoTexture |
| for sale (right) | `KFTab_BuyMenu.Sale`, `PurchaseB` ("Purchase Weapon"), `SaleBox` (`KFBuyMenuSaleList.DrawInvItem`, 10 rows, scroll bar) | `Thick_border_Transparent`, `Item_box_*` (+ `_Disabled`), `scrollbar` |
| description (bottom left) | `KFTab_BuyMenu.Info`, `IScrollText` (`SetInfoText`) | `Thin_border` |
| auto fill, exit (bottom right) | `KFTab_BuyMenu.AmmoExit`, `AutoFill`, `Exit` | `Thin_border`, KF_SquareButton |
| encumbrance (footer) | `GUIBuyMenu.Weight`, `WeightIcoBG`, `WeightIco`, `WeightB` (`KFWeightBar.MyOnDraw`) | `Thin_border`, `Perk_box_unselected`, `Hud_Weight`, `Progress` |

Texts are KF's defaults (KFGui.int says the same in English): "Trader
Closes in", "Wave", "Current Perk", "Lv", "Sell Value: £", "Auto Fill
Ammo", InfoText[0-2], "Weight: %i blocks", "Encumbrance Level",
"Equipment", "Buy", "Purchased", "Repair". The catalogue gains each
pickup's ItemShortName, Description (the weapon's), PowerValue /
RangeValue / SpeedValue, CorrespondingPerkIndex, TraderInfoTexture and
SecondaryAmmoShortName (display only).

**Native parts (not in the scripts), guessed and labelled in the code:**
the tab panel's docking (under PageTabs, to the bottom of the screen:
fitted to the screenshot); bBoundToParent children without
bScaleToParent (position inside the parent, size of the screen: fitted
to the filter icons); `GUIProgressBar` (fill = Value / High, fitted to
the screenshot's three bars); ImageStyle Justified (aspect kept,
centred); the list font (UT2SmallFont) and button font (UT2MenuFont);
disabled buttons drawn darker; double-click time 0.5 s.

**Input.** Mouse as in KF: click a row to select it (left or right
list; the other list loses its selection), double-click a sale row to
buy it or an inventory name to sell it, the row's clip / fill buttons,
the armour's buy / repair button, Purchase Weapon, Sell Weapon, Auto Fill
Ammo, Exit Trader Menu, the filter icons, the quick perk icons, the
wheel and the arrows scroll the sale list. Keys (ours, KF has none):
Up / Down move in the list, Tab switches list, Left / Right change the
filter, Enter buys / sells / buys armour, C clip, F fill, A auto fill,
1-7 change perk, E / Escape / Backspace close (shared). Test actions
`kf:up`, `kf:down`, `kf:tab`, `kf:left`, `kf:right`, `kf:enter`,
`kf:clip`, `kf:fill`, `kf:fill_all`, `kf:buy`, `kf:sell`, `kf:filter:N`,
`kf:select:CLASS`, `kf:click:ID` (a box drawn last frame), `kf:wheel:N`;
the old `menu_*` actions still work.

**Logs.** `classic_menu_open` (dosh, weight, filter, rows),
`classic_menu_layout` (window size and every panel's box, once per size),
`classic_menu_select`, `classic_menu_click`, `classic_menu_request`
(action, weapon, shown price), `classic_menu_result` (next frame: dosh
and weight before / after), `classic_menu_close`. `menu_dump` lists
every quad.

**Not in this step:** the Perks tab (the Perk button logs `not_built`),
favourites (the ninth filter shows an empty list; no Favorite button),
the item list's hover sounds, DLC / locked items (we sell base weapons
only), the description's typing effect (`CharDelay`), FontScale.

### Steps

- **CT1** catalogue display fields, component list, textures and fonts.
- **CT2** the drawing (`src/game/classic_menu.rs`), the text list goes.
- **CT3** the input, test actions and logs.

### As built (2026-10-08; headless runs, not played by you)

CT1-CT3 built together (they only work together). `src/game/classic_menu.rs`
(model `inv_rows`, input, drawing, 3 unit tests); `buy_menu.rs` lost the
text list and its keys (opening / closing / test actions `buy:C` etc.
unchanged), `ShopItem.info` added; `menus/gui.rs` reads the components,
class values, and fonts Vr20-26 (UT2LargeFont, UT2HeaderFont,
UT2ServerListFont); `menus/mod.rs` loads the menu's textures and the 48
trader pictures only with `--trader-menu kf`, and draws it. At 2560 x 1440
the boxes land within a few pixels of the screenshot's.

## Mouse sensitivity and invert mouse (planned 2026-10-08: MS1-MS3)

**Goal.** A mouse sensitivity setting and an "invert mouse" (up/down)
setting, as KF's Input settings tab has: a command-line option, a
launcher field and a row in the pause menu's Settings window, saved in
the same settings file as the volumes and the aim mode.

**What KF does (read 2026-10-08).**

- *Settings and defaults* (Engine.PlayerInput, `kfpkg defaults`, and
  the install's User.ini): `MouseSensitivity = 3`, `bInvertMouse =
  False`, `MouseSmoothingMode = 1` (on), `MouseSmoothingStrength = 0.3`,
  `MouseAccelThreshold = 0` (off), `MouseSamplingTime = 1/120 s`.
- *The options menu* (KFGui.KFInputSettings, built on GUI2K4's
  UT2K4Tab_IForceSettings; numbers from GUI2K4.u's components):
  "Mouse Sensitivity (Game)" is a number box from **0.25 to 25 in steps
  of 0.25**; "Invert Mouse" is a checkbox ("the Y axis of your mouse
  will be inverted"). Also there: "Mouse Smoothing" (checkbox),
  "Mouse Smoothing Strength" (0 to 1, step 0.05), "Mouse Accel.
  Threshold" (0 to 100, step 5), "Reduce Mouse Lag", "Mouse
  Sensitivity (Menus)".
- *Counts to turning* (scripts: PlayerInput / KFPlayerInput.PlayerInput,
  PlayerController.UpdateRotation; engine code for the first two
  factors, details in the local RE.md): each raw mouse count adds
  `Speed (2.0, User.ini: "Axis aMouseX Speed=2.0") x 0.01` to the axis;
  once per frame the engine multiplies every input axis by
  `20 / frame time`; the script multiplies by `MouseSensitivity x
  FOVScale`; the view turns by `32 x frame time x axis` Unreal rotation
  units (65536 = one full turn). The frame time cancels, so one count
  turns **12.8 x sensitivity x FOVScale units** = 0.0703 degrees x
  sensitivity x FOVScale. Up/down is the same with the mouse's y count
  flipped (mouse forward looks up); invert flips it back.
- *FOVScale* (KFPlayerInput): the current field of view / 90 (iron
  sights lower the FOV, so the mouse slows with the zoom); while a 3D
  scope is drawn (KF's default scope detail, aiming a scoped weapon)
  it is 24 / 90 instead.
- *Smoothing mode 1* spreads one mouse report over the frames until the
  next one when the game runs faster than the mouse reports; the total
  turn stays about the same. Not in this step (follow-up).

**What Open KF does now.** A fixed 0.002 radians per count, no FOV
scaling, no invert. At FOV 90 that equals KF sensitivity 1.63.

**Default chosen: KF's 3** (the formula above is read from KF, not
guessed), so looking around turns 1.84 times faster than before at the
default FOV, and slower while aiming down sights (KF's FOV scaling).
The old speed is sensitivity 1.63 (1.5 or 1.75 on the 0.25 steps).

**Saved.** Two more lines in `settings/launcher.txt`:
`mouse_sensitivity=3.00` and `invert_mouse=off`. The game reads them at
start and rewrites only these lines when they change in the menu; the
launcher saves them on PLAY.

**Command line.** `--sensitivity X` (0.25 to 25) and `--invert-mouse` /
`--no-invert-mouse` override the file for that run (not saved unless
changed in the menu).

**In game.** The Settings window's Controls box gets two rows under
"Aim down sights": "Mouse Sensitivity" (a slider over 0.25..25 with the
number; Left / Right step 0.25 as KF's box) and "Invert Mouse" (*Off* /
*On* buttons). Test actions: `mouse_sensitivity:X`, `invert_mouse:on|off`,
`sensitivity_click:FRACTION`, `invert_click:on|off`, and
`mouse_move:DX;DY` (feeds raw counts through the look code, as a mouse
would, for checking the conversion from the log).

**Launcher.** The Controls box gets two spinner rows: "Mouse
sensitivity" (labelled "Sensitivity"; `<` 3.00 `>`, steps of 0.25) and "Invert mouse" (Off/On).

**Logs.** `mouse_settings sensitivity=3.00 invert=false source=...` at
start (with `read=file|default|command_line` and the turn per count at
FOV 90) and on every change; `mouse_saved file=...`; `mouse_look` for
each `mouse_move` test action (counts, FOV, FOVScale, degrees turned).

**Steps.** MS1: the setting (choices fields, saved lines, command-line
options, game resource, conversion with FOV / scope scaling, invert,
logs, unit tests). MS2: the pause menu rows. MS3: the launcher rows.

**Follow-ups (KF has them, not built).** Mouse smoothing (on in KF by
default) and its strength, mouse acceleration threshold (off in KF by
default), menu mouse sensitivity (KF 1.25), "Reduce Mouse Lag" (a
renderer setting).

**As built (2026-10-08; headless runs, not played with a real mouse).**
MS1-MS3 as planned. Files: `src/engine/mouse.rs` (new: the counts-to-turn
rule, `MouseSettings` read at start and saved on change, 3 unit tests),
`src/engine/camera.rs` (`look` uses it, `mouse_move` test action),
`src/launcher/choices.rs` (the two fields and lines, `with_mouse_lines`,
slider helpers, 1 test), `src/launcher/mod.rs` (`read_mouse`,
`save_mouse`), `src/launcher/draw.rs` (Controls box: 3 rows),
`src/game/menus/audio_page.rs` and `mod.rs` (the two rows, clicks, drag,
keys, test actions), `src/main.rs` (options, 1 test). A slider drag
saves once when let go (or when Escape closes the window mid-drag).

## Difficulty: Beginner to Hell on Earth (planned 2026-10-08: D1-D2)

KF's difficulty is one number, `GameInfo.GameDifficulty` (set from the
`?Difficulty=` URL option or KillingFloor.ini, saved by the server
setup page). The values are fixed by the menu (`KFMod.int`
`GIPropsExtras[0]` "1;Beginner;2;Normal;4;Hard;5;Suicidal;7;Hell on
Earth", `KFGui.KFMapPage.GameDifficultyChange`): **1 Beginner, 2
Normal, 4 Hard, 5 Suicidal, 7 Hell on Earth**. The names shown are
`KFMod.int` `KFScoreBoard.SkillLevel[N]` and `KFGui.int` `LobbyMenu`
`BeginnerString` .. `HellOnEarthString`. The rules almost always test it
with thresholds (`>= 7`, `>= 5`, `>= 4`, `>= 2`, else Beginner, or the
`< 2 / < 4 / < 5` form), so "Suicidal and up" also means Hell on Earth.
KF copies it to the clients in `KFGameReplicationInfo.GameDiff` and
`BaseDifficulty` (int); the clients' own copy is only used for display
and a few client-side checks.

Until now every one of our rules used Normal (GameDifficulty 2):
`dosh::GAME_DIFFICULTY = 2.0` and hard-coded Normal values in the zed,
wave and cash code.

### Every rule that depends on difficulty (from the scripts)

"Solo" below = KF's `Level.Game.NumPlayers == 1`; we assume one player
everywhere (see "Not done"). B / N / H / S / HoE = 1 / 2 / 4 / 5 / 7.

| KF class.function | Rule | B | N | H | S | HoE | Our code | D1? |
|---|---|---|---|---|---|---|---|---|
| KFMonster.DifficultyHealthModifer (PostBeginPlay) | zed Health (int, cut) and HealthMax x | 0.5 | 1.0 | 1.35 | 1.55 | 1.75 | zeds/zed/spawn.rs | yes |
| KFMonster.DifficultyHeadHealthModifer | HeadHealth x | 0.5 | 1.0 | 1.35 | 1.55 | 1.75 | spawn.rs | yes |
| KFMonster.DifficultyDamageModifer | MeleeDamage, ScreamDamage, SpinDamConst/Rand = Max(int(x d), 1); x 0.75 more solo | 0.3 | 1.0 | 1.25 | 1.5 | 1.75 | zed/load.rs (`solo_damage`) | yes |
| ZombieScrake (sawing) | MeleeDamage = Max(DifficultyDamageModifer x default, 1) | as above | | | | | think.rs uses the class value | yes (same value) |
| KFMonster.PostBeginPlay | MovementSpeedDifficultyScale: Ground/Air/WaterSpeed x (OriginalGroundSpeed); HiddenGroundSpeed not scaled | 0.95 | 1.0 | 1.15 | 1.22 | 1.3 | think.rs speed | yes |
| KFGameType.SetupWave | TotalMaxMonsters = Clamp(WaveMaxMonsters x DifficultyMod x NumPlayersMod, 5, 800) | 0.7 | 1.0 | 1.3 | 1.5 | 1.7 | waves.rs `setup_wave` | yes |
| KFGameType.InitGame | StartingCash | 300 | 250 | 250 | 200 | 100 | dosh.rs | yes |
| KFGameType.InitGame | MinRespawnCash | 250 | 200 | 200 | 150 | 100 | net/starts.rs | yes |
| KFGameType.InitGame | TimeBetweenWaves (s) | 90 | 60 | 60 | 60 | 60 | waves.rs `do_wave_end` | yes |
| KFGameType.ScoreKill | KillScore = ScoringValue x (then x 1.75 Short), Max(1, int) | 2.0 | 1.0 | 0.85 | 0.65 | 0.65 | dosh.rs `kill_score` | yes |
| KFGameType.ScoreKill | player death: lose Score x GameDifficulty x 0.05 | 5% | 10% | 20% | 25% | 35% | dosh.rs (already used the number) | yes |
| KFGameType.CalcNextSquadSpawnTime | NextSpawnTime x 0.85 at Hard and up (before the sine term) | 1 | 1 | 0.85 | 0.85 | 0.85 | waves.rs `next_squad_time` | yes |
| KFBloatVomit.DifficultyDamageModifer | BaseDamage, Damage = Max(int(x d), 1) | 0.3 | 1.0 | 1.5 | 2.0 | 2.5 | zeds/vomit.rs | yes |
| HuskFireProjectile.PostBeginPlay | Damage x | 0.75 | 1.0 | 1.15 | 1.3 | 1.3 | zeds/fireball.rs | yes |
| BossLAWProj.PostBeginPlay | Damage x (solo; others in brackets) | 0.25 (0.375) | 0.375 (1.0) | 1.15 | 1.3 | 1.3 | fireball.rs | yes |
| ZombieBoss.PostBeginPlay | MGDamage x (solo; others in brackets) | 0.375 | 0.75 (1.0) | 1.15 | 1.3 | 1.3 | boss.rs `MG_DAMAGE` | yes |
| ZombieBoss.PostBeginPlay | HealingLevels from the scaled Health | | | | | | boss.rs `BossState::new` | yes (gets the scaled health) |
| KFGameType.SetupPickups | share of weapon / ammo pickups on | 50/65% | 30/50% | 20/35% | 10/10% | 10/10% | pickups/rules.rs (already takes the number) | yes |
| KFAmmoPickup / (our pickup.rs) | extra round with chance 1 / GameDifficulty | | | | | | already takes the number | yes |
| KFVetFieldMedic, KFVetBerserker, KFVetSharpshooter | Medic speed at >= 5, Berserker L6 armour below 5, Dualies headshot bonus below 7 / 8% at 7 | | | | | | perks.rs (Medic, Berserker take the number) | yes; Dualies: check in D2 |
| LobbyMenu / KFScoreBoard / KFLobbyTitleLabel | the name shown | | | | | | menus/lobby.rs, net/scoreboard.rs | yes |
| ZombieBloat.RangedAttack | ChargeChance (moving vomit) | 0.2 | 0.4 | 0.6 | 0.8 | 0.8 | zed/mod.rs `BLOAT_CHARGE_CHANCE` | D2 |
| ZombieGoreFast.RangedAttack | ChargeChance | 0.1 | 0.2 | 0.3 | 0.4 | 0.4 | `GOREFAST_CHARGE_CHANCE` | D2 |
| ZombieScrake.RangedAttack | ChargeChance / RagingChargeChance | 0.25/0.5 | 0.5/0.7 | 0.65/0.85 | 0.95/1.0 | 0.95/1.0 | think.rs | D2 |
| ZombieScrake.RangedAttack, TakeDamage | rage below 75% health at >= 5 (else 50%); at >= 5 TakeDamage also calls RangedAttack under 75% | | | | | | think.rs | D2 |
| ZombieScrake.PlayTakeHit | at >= 5 a flinch only while StunsRemaining != 0 | | | | | | | D2 |
| ZombieScrake / ZombieFleshPound.TakeDamage | crossbow headshot x 0.5 (Scrake), x 0.35 (FP) at >= 5 | | | | | | combat | D2 |
| ZombieFleshPound (StartCharging) | rage length 5 x m + FRand x 6 x m | 0.85 | 1.0 | 1.25 | 3.0 | 3.0 | think.rs | D2 |
| ZombieHusk.PostBeginPlay | ProjectileFireInterval x / BurnDamageScale x | 1.25/2.0 | 1/1 | 0.75/0.75 | 0.6/0.5 | 0.6/0.5 | zed/load.rs | D2 |
| AIController.PreBeginPlay | Skill = Clamp(Skill + GameDifficulty, 0, 3): KFMonsterController's leap/jump checks (`Skill > 1 + 2 x FRand`), HuskZombieController's aim | | | | | | not modelled | D2 |
| KFPawn.TakeDamage (vomit, Siren scream) | at >= 4 the view effects flag on the controller (bVomittedOn / bScreamedAt) | | | | | | | D2 |
| KFPawn (movement disabled) | at >= 5 no upward velocity while bMovementDisabled | | | | | | | D2 |
| Pawn / KFMonster / ZombieStalker.DoJump | above 2, MakeNoise(0.1 x GameDifficulty) on a jump | | | | | | | D2 |
| KFGameType.ReduceDamage | own damage x 0.5 at <= 3 in single player | | | | | | combat.rs (Normal) | D2 |
| Pickup / KFWeaponPickup / WeaponPickup | respawn sleep FMin(30, GameDifficulty x 8) standalone; respawn time x (0.33 + 0.22 x GD) at <= 3 | | | | | | | D2 (check which apply) |
| CashPickup | CashAmount x GameDiff x 0.5 | | | | | | not built (no cash pickups) | D2 |
| KFGameType.GetServerInfo / achievements | flags, achievements | | | | | | | never |

Number of players (NumPlayersHealthModifer, the x 0.75 solo damage,
NumPlayersMod in SetupWave, the boss's solo rules) stays "one player"
as before: it is a separate rule set, not part of this work.

### Our design

- `src/game/difficulty.rs`: `enum Difficulty` (five values), its
  GameDifficulty number, names, the `--difficulty` word
  (`beginner|normal|hard|suicidal|hoe`), and one function per rule in
  the table, each citing its KF class.function. The game's setting is
  kept like KF's `Level.Game.GameDifficulty`: one value for the whole
  run, set at startup (`difficulty::set_current`), read with
  `difficulty::current()`. A global (not a Bevy resource) because many
  of the users are plain functions (perks, pickups rules); the formulas
  themselves take the difficulty as a parameter so tests do not depend
  on it. `GameOptions.difficulty` holds it too, for the logs and
  sharing.
- Default **Normal**: everything we built so far uses Normal values, and
  KF's own default (KillingFloor.ini) is 2.
- Multiplayer: the host's difficulty wins. The joiner's host-info query
  gets a `difficulty=` key (like `length=`) and uses it before anything
  is loaded; `NetGame` carries it too for the log (protocol id raised).
  Zeds are simulated on the host, so their health, damage and speed come
  from the host anyway; the joiner needs it for its own dosh, starting
  cash, perks and the names shown.
- Launcher: a Difficulty spinner in the Game section (with Length),
  saved as `difficulty=`; passed as `--difficulty` in waves mode, not
  when joining.
- Logs: `difficulty level=hoe name="Hell on Earth" game_difficulty=7`
  at startup with the scales; `zed_spawned` gains `health= health_max=
  head_health= melee_damage= speed_scale=`; `wave_start` gains
  `difficulty_mod=`.

### Steps

- **D1** (this step): the setting, launcher, lobby/scoreboard names,
  network sharing, and every row marked "yes".
- **D2**: the zed behaviour rows (charge chances, Scrake and Fleshpound
  rage, Husk fire rate and fire resistance, crossbow resistances, AI
  skill), the player-side rows, pickup respawn times.

### As built (D1, 2026-10-08; headless runs, not played by you)

`src/game/difficulty.rs` (the enum, every D1 formula, the global setting,
4 unit tests); `dosh.rs` (StartingCash, KillScore scale, the death loss
now read the setting; `GAME_DIFFICULTY` / `STARTING_CASH` constants
gone), `net/starts.rs` (MinRespawnCash), `waves.rs` (`wave_total`,
TimeBetweenWaves, the x 0.85 squad time; `GameOptions.difficulty`),
`zeds/zed/spawn.rs` (health, head health, speed scale; the boss's
healing levels from the scaled health), `zed/load.rs` + `zed/mod.rs`
(melee and scream damage), `think.rs` (speed), `vomit.rs`,
`fireball.rs`, `boss.rs` (MG), `main.rs` (`--difficulty`, the host's
difficulty for a joiner), `net/query.rs` (`difficulty=` key),
`net/protocol.rs` + `net/mod.rs` (`NetGame.difficulty`, protocol id
...0009), `net/client.rs` / `server.rs` (log), `menus/lobby.rs` and
`net/scoreboard.rs` (the name), the launcher (`choices.rs`, `draw.rs`,
`mod.rs`: a Difficulty spinner under Length, saved as `difficulty=`).
SpinDamConst / SpinDamRand have no counterpart in our code (nothing
uses them yet).

## The weapon selection bar (planned 2026-10-08: WB1-WB3)

KF draws a row of boxes along the top of the screen when you roll the
mouse wheel (reference screenshot, untracked:
`references/weapon_top_hud.jpg`, 2560 x 1440). Each column is one
inventory group (1 melee, 2 pistols, 3 primary, 4 specials, 5 equipment);
each weapon is a box with its picture, the highlighted one in the lighter
"selected" box with its red picture. Rolling moves the highlight; a click
of Fire takes the highlighted weapon (and does not shoot).

**What KF does (scripts, `KFMod.HUDKillingFloor` and
`KFMod.KFPlayerController`):**

| Piece | KF source |
|---|---|
| wheel opens the bar and moves the highlight | `KFPlayerController.NextWeapon / PrevWeapon` call the HUD's `NextWeapon / PrevWeapon` (never the pawn's). `User.ini`: `MouseWheelUp=NextWeapon`, `MouseWheelDown=PrevWeapon` |
| opening | `HUDKillingFloor.ShowInventory`: shown, fade in, the highlight starts on the weapon in hand; the same roll then moves it one step |
| moving | `HUDKillingFloor.NextWeapon / PrevWeapon`: weapons sorted into the 5 groups in inventory-list order (`InventoryGroup > 0`, so not the grenade); next/previous inside the group, then the first/last of the next non-empty group, wrapping |
| Fire takes it | `KFPlayerController.Fire`: while the bar is shown, `HUD.SelectWeapon` and `bFire = 0` (no shot). `SelectWeapon`: hide (fade out), the weapon becomes `PendingWeapon`, the one in hand is put down (same rule as a slot key) |
| Escape | `KFPlayerController.ShowMidGameMenu`: while shown, Escape only hides the bar (no pause menu) |
| number keys | `SwitchWeapon` goes to the pawn, not the HUD: they switch at once and do not open, move or close the bar |
| timeout | none: nothing in the scripts hides the bar except Fire and Escape. (Not a guess: the only callers of `HideInventory` are those two.) |
| drawing | `HUDKillingFloor.DrawInventory`, last thing in `DrawHUD` |

**Layout (HUDKillingFloor defaults):** `InventoryX = 0.22`, `InventoryY`
not set (0), `InventoryBoxWidth = 0.1`, `InventoryBoxHeight = 0.075`,
`BorderSize = 0.005`, all times the screen width except Y (times the
height). Column `i` sits at x = (0.22 + 0.1 i) x width; its boxes stack
downwards, each 0.1 x 0.075 widths. An empty group is one box a quarter
as tall. Box textures `KillingFloorHUD.HUD.Hud_Rectangel_W_Stroke`
(`InventoryBackgroundTexture`) and `Hud_Rectangel_selected`
(`SelectedInventoryBackgroundTexture`), drawn with DrawTileStretched
(corners kept, middle stretched; native, same guess as the menus'
painter). Picture: the weapon's `HudImage` (`SelectedHudImage` when
highlighted), texels (0,0)-(256,192), inside the box minus the border.
Colour white; alpha fades over `InventoryFadeTime = 0.3` s in and out.
At 2560 wide: boxes 256 x 192 from x = 563, matching the screenshot.

**Our design.**
- `src/weapons/weapon/weapon_bar.rs` (new): the `WeaponBar` resource
  (shown, fade start, highlighted weapon class, and each frame the list
  of (class, group) in inventory order for the HUD) and the pure
  step function (port of the HUD's Next/PrevWeapon, unit tested).
- `input.rs`: the wheel (and test actions `next` / `prev`, which already
  exist) opens / steps the bar instead of switching; Fire while shown
  selects (through the existing switch path, so reloads still refuse a
  switch) and the click does not fire (we ignore the button until it is
  let go: our reading of `bFire = 0`). Death hides the bar (ours: KF's
  just stops drawing it without a pawn).
- `menus/mod.rs`: Escape while the bar is shown hides it instead of
  opening the pause menu.
- `hud.rs`: load the layout numbers, the two box textures and every
  weapon's two pictures at startup; draw the bar last.

**Logs.** `weapon_bar shown=… highlighted=… groups=…` on every
open/step/close (groups = item counts per group),
`weapon_bar_select weapon=…`, `weapon_bar_layout` (window size and every
box) once per window size and contents. Test actions: `next`, `prev`
(the wheel), `fire`.

### Steps

- **WB1** state + input (wheel opens/steps, Fire selects, Escape hides).
- **WB2** HUD drawing.
- **WB3** logs, test, MODLOG.

### As built (2026-10-08; headless runs, not played by you)

WB1-WB3 built together. `src/weapons/weapon/weapon_bar.rs` (state, step
rule, 3 unit tests); `input.rs`: the wheel steps the bar (it no longer
switches directly), Fire while shown selects, the switch rules moved
unchanged into `select_weapon` (shared by slot keys, quick heal,
flashlight and the bar); `menus/mod.rs`: Escape hides a shown bar;
`hud.rs`: layout, box textures, pictures (`HudImage`, or the
`HudImageRef` name for the later weapons: 47 of 48 weapons; the Frag has
none and is never in the bar), `draw_weapon_bar`, a 2560 x 1440 layout
unit test; the HUD's quad pool went from 160 to 320. At 2560 x 1440 the
boxes are 256 x 192 from x = 563, as in the reference screenshot.

## Muzzle-flash light (planned 2026-10-08: ML1-ML3)

When a gun fires in KF, the flash briefly lights the walls, floor and
zeds around it. Until now ours only drew the flash sprite.

### What KF does (scripts and class defaults, checked 2026-10-08)

- **Who turns it on.** Not the fire mode: the weapon's third-person
  attachment. KFWeaponAttachment.ThirdPersonEffects runs on every change
  of FlashCount (every shot) on every machine, also for your own gun
  (your attachment exists, hidden). If FlashCount > 0 and
  bDoFiringEffects it calls WeaponLight (then the 3rd-person flash and
  shell). WeaponLight (xWeaponAttachment, same in KFWeaponAttachment):
  if the shooter is seen in **first person** the *first-person weapon*
  gets bDynamicLight = true (so the weapon class's light shines);
  otherwise the *attachment* does. Then SetTimer(0.15): Timer turns both
  off again. Every shot restarts the 0.15 s, so a full-auto burst keeps
  the light on until 0.15 s after the last shot.
- **No light:** melee (KFMeleeAttachment: bDoFiringEffects = false),
  PipeBombAttachment and BlowerThrowerAttachment (WeaponLight is empty).
  The alt fire of SingleAttachment, ShotgunAttachment, DualiesAttachment
  and NailGunAttachment (FiringMode 1: the flashlight button) returns
  before any effect.
- **The light** (an Unreal light is a few numbers on any actor):
  KFWeapon and KFWeaponAttachment both default to LightType 1 (steady),
  LightEffect 13 (LE_NonIncidence: the surface's angle does not matter),
  LightHue 30, LightSaturation 150 (a warm orange-white), LightBrightness
  255, LightRadius 10 (= 25 x (10 + 1) = 275 Unreal units, 5.5 m).
  Weapons whose first-person light is switched off (LightType 0,
  LightBrightness 0): Crossbow, M99, M79, M32, Huskgun, SeekerSix, ZEDGun,
  Crossbuzzsaw, SealSqueal harpoon (and subclasses). SingleAttachment
  (the pistols' 3rd-person light): LightType 2 (pulse), LightRadius 0
  (25 units).
- **Where:** at the actor that carries it: the first-person weapon's
  location (KF draws it at the eye + PlayerViewOffset, about 20-30 units
  from the eye), or the attachment on the pawn's hand.
- Not in the scripts (native renderer): how a dynamic light falls off on
  walls. UE2 dynamic lights cast no shadows (they light through thin
  walls).

### Our design

- `src/weapons/muzzle_light.rs` (the world side, like flashlight.rs): a
  `MuzzleLight` component on every pawn that can shoot: its light values
  (read from the class defaults), where it is this frame, and a 0.15 s
  timer. `flash()` restarts the timer. One Bevy point light per pawn,
  made once and hidden while off (no spawning per shot), no shadows.
  While on, the same light is also pushed into `DynamicLights`, so
  (a) baked meshes near it switch to the lit material (`BakedSwap`,
  render/baked.rs) exactly as for the flashlight, and (b) zeds, bodies
  and the first-person weapon get it as vertex light (actor_light.rs).
- Local player (`src/weapons/weapon/muzzle_light.rs`): on each new shot
  (the shot counter that also feeds FlashCount) apply the attachment's
  rules above, then `flash()`; the light values are the *weapon* class's
  (first person); the position is the first-person weapon's location.
- Other players (`player/body/fire_fx.rs`): the same on their FlashCount
  changes, with the *attachment* class's light values, at the
  attachment's tip.
- **Unit conversion, same as the flashlight glow and the map lights:**
  colour = hue colour (`hue_colour`) x LightBrightness / 255 x 0.82 (the
  native colour scale actors use). Actors: as a map light of that colour
  (Smooth falloff over the radius, x2 hardware gain). Walls (Bevy point
  light): range = 1.12 x radius; strength set so that a wall facing the
  light at 0.3 x radius gets the UE2 value there (colour x falloff x
  K = 2, made linear), with `lumens_for` from flashlight.rs. **Guess**:
  Bevy's light falls off with 1/d^2, UE2's with the radius curve, so
  the match is exact only at 0.3 x radius (brighter closer, dimmer near
  the edge). Bevy also uses the surface angle (LE_NonIncidence ignores
  it). LT_Pulse (the 3rd-person pistol light) is drawn steady.
- `KF_MUZZLE_LIGHT=0` switches it off (to measure the frame-time cost).
- Logs: `muzzle_light_setup weapon=... type=... radius=... brightness=...
  color=...` once per weapon; `muzzle_light weapon=... on=true|false
  radius=... brightness=... color=... ons=N offs=N` on each switch;
  `frame_stats` for the cost.

### Steps

- **ML1** world side + local player.
- **ML2** other players' attachments.
- **ML3** logs, headless test at a dark spot, frame time, MODLOG.

### As built (ML1-ML3, 2026-10-08; headless runs, not played by you)

As planned. `weapons/muzzle_light.rs` (world side, 3 unit tests),
`weapons/weapon/muzzle_light.rs` (local player), `player/body/fire_fx.rs`
(other players, not tested: needs a network game). The light is placed
in Update after the flashlight's system (which clears `DynamicLights`).
Measured on KF-WestLondon's tunnel (AK-47, 30-round burst, 2 runs each,
`muzzle_light_burst` log): 38.3 / 38.1 ms per frame with the light,
37.2 / 40.1 ms with `KF_MUZZLE_LIGHT=0`: no cost above the noise. While
on, 33 baked meshes switch to the lit material there (`baked_swap`).
A Clot 118 units ahead: drawn light 0.35 with, 0.24 without.

## Map rotation and map voting without restarting (planned 2026-10-09)

**Goal:** at the end of a match the game moves to the next map by itself, in
single player and in multiplayer, without anyone closing the game. Everyone
stays connected; the lobby opens again on the new map. Map voting as in KF.

**What KF does** (read in its scripts and inis; details in MODLOG):
- The match ends (squad wiped, or the Patriarch / last wave beaten). The
  end screen shows; 14 s later (EndTimeDelay 4 + RestartWait 10) the game
  goes to the next map. The host (or the single player) may press Fire
  after 5 s to go early. Remote clients cannot.
- The next map comes from a map list (default: KF-BioticsLab, KF-Farm,
  KF-Manor, KF-Offices, KF-WestLondon). The position after the current map
  is used; it wraps round, skips maps that are not installed, and is saved.
  A current map that is not in the list moves the position on by one.
- Map voting exists but is off by default (bMapVote=False). When on: the
  vote window opens 5 s after the end (ScoreBoardDelay); voting lasts
  VoteTimeLimit (30 s in the ini); it ends early when everyone has voted;
  if nobody voted a random map is picked; ties are broken at random,
  avoiding the current map. "<map> has won !" is announced.
- While the new map loads players see KF's server loading screen (not
  the "Deploying to" one, which KF's map loading only uses for a ladder
  game): KF background art, the map's preview picture with title and
  author, ". . . LOADING", the map name and a random hint. Then the lobby. Name and perk are kept;
  cash goes back to the starting amount; kills, deaths and Ready are reset.
- KF's clients disconnect and reconnect on every map change. **We do
  better: our clients stay connected** and load the map in place.

**How we do it:** the map becomes something that can be unloaded and
loaded again inside the running game (today it is loaded once at startup
by ~25 separate one-time steps).

Steps (each its own branch, revertible on its own):

1. **Map lifecycle.** A `MapScoped` tag on every entity that belongs to
   the map (geometry, colliders, doors, glass, lights, decals, emitters,
   map sounds, traders, pickups, zeds, gibs, projectiles). The one-time
   startup steps become a load sequence that runs whenever a `ChangeMap`
   request arrives: unload (despawn tagged entities, remove per-map
   resources, clear "done once" flags) then load. Startup simply sends the
   first `ChangeMap`. Test: a debug option that hops through several maps
   in one run and logs entity / asset counts after each load (flat counts =
   nothing leaked), plus every map still loads as before.
2. **Map list + next map (single player).** The map list, KF's
   GetNextMap rules, the position saved in our own settings file (never in
   KF's inis). At the end of the match, Fire after 5 s / the 14 s timer
   sends `ChangeMap(next)` instead of restarting on the same map.
3. **Multiplayer map change.** The host announces the next map (shared
   match state gets the next map and a travel counter). Everyone unloads
   and loads it while staying connected. The lobby is reset (match not
   started, nobody ready) and opens again; cash, kills and deaths reset;
   name, perk and character are kept. Doors and pickups are synced again.
4. **Map voting.** KF's xVoting rules: pure logic module with unit tests,
   the vote window (list of maps, vote counts, time left), the client to
   host vote message and the shared vote state. Off by default as in KF;
   a launcher/command-line switch turns it on.
5. **Loading screen and launcher.** KF's loading screen while a map
   loads; launcher settings for the map list and map voting.

Steps 1, 4 (logic, window, messages) and the map-list part of 2 and 5 can
be built in parallel; steps 2 (wiring), 3 and the loading screen wait for
step 1.
