# MODLOG

## 2026-10-03 Project setup and milestone 1 plan

**Changed:** Ran `cargo init` (creates `Cargo.toml`, `src/main.rs`, and a git repo).
Replaced the generated `.gitignore` with a whitelist. Added `MODLOG-template.md`,
`STATUS-handoff.md`, `README.md`, `docs/DESIGN.md`.
**Why:** Starting the project. Plan before code.
**Tested how:** Ran `git status --untracked-files=all` and `git check-ignore`.
Inspected the KF install: `System/Build.ini`, the first bytes of
`Maps/KF-Farm.rom`, file-type counts per folder.
**Result:** Git sees only the 9 source/config/doc files. `references/` and
`.direnv/` are ignored. KF is Unreal Engine 2.5 (UT2004 build 2004-11-11), with
package version 128 and licensee 29. The UnrealScript source text appears to be
embedded in the `.u` files.
**Still broken / not tested:** No code beyond cargo's hello-world. Whether *all*
script source is embedded is not verified. Nothing committed.
**Next:** Milestone 1 step 0 (workspace, Bevy, flake system libraries, install
discovery, log file).

## 2026-10-03 Milestone 1 step 0: workspace, Bevy window, install discovery, run log

**Changed:** `Cargo.toml` (now a workspace, Bevy 0.19.1, faster dev profile),
`crates/ue-assets/` (new library crate; `src/install.rs` finds the install),
`src/main.rs` (Bevy window, `--map`/`--frames` options, per-second frame stats),
`src/runlog.rs` (writes `logs/latest.log`), `flake.nix` (Bevy's Linux libraries
plus `LD_LIBRARY_PATH`). Ran `git add flake.nix flake.lock`: Nix ignores
untracked flake files in a git repo, so the dev shell broke once `cargo init`
created the repo.
**Why:** Foundation for every later step.
**Tested how:** `cargo test --workspace` (2 install tests), `cargo clippy
--workspace`, `cargo run -- --frames 180`, `KF_ROOT=/tmp/nope cargo run`,
`cargo run -- --bogus`.
**Result:** Tests 2/2 pass, clippy reports no warnings. The run log showed
`install="references/killing_floor" build="UT2004_Build_[2004-11-11_10.48]"`,
then `fps=52, 61, 60`, `frame_limit_reached frame=180`, `exit=Success`. Bad
`KF_ROOT` prints `KF_ROOT=/tmp/nope does not contain System/Build.ini` and logs
`reason=install_not_found`. Bad argument prints usage.
**Still broken / not tested:** Bevy prints one warning at window creation
(`Couldn't get swap chain texture after configuring. Cause: 'Outdated'`). It
appears harmless because frames render normally after it. Not tested on Wayland
or Windows. `--map` is accepted but does nothing yet.
**Next:** Step 1, the Unreal package reader and the `kfpkg scan` tool.

## 2026-10-03 Licenses

**Changed:** Added `LICENSE-MIT` and `LICENSE-APACHE`, whitelisted both in
`.gitignore`, set `license = "MIT OR Apache-2.0"` in `Cargo.toml`.
**Why:** You chose MIT + Apache 2.0.
**Tested how:** `git status` shows both files. The Apache text was copied from
Bevy 0.19.1's own `LICENSE-APACHE` (the standard text ending at "END OF TERMS
AND CONDITIONS", without the optional appendix) rather than typed from memory.
**Result:** Both files are visible to git.
**Still broken / not tested:** The MIT copyright line says "kf-rs contributors".
Change it if you want your name there.
**Next:** Step 1.

## 2026-10-03 Milestone 1 step 1: Unreal package reader and `kfpkg` tool

**Changed:** New `crates/ue-assets/src/reader.rs` (byte reader: integers, floats,
Unreal "compact index" variable-length integers, strings),
`crates/ue-assets/src/package.rs` (header, name, import and export tables, plus
consistency checks), `crates/ue-assets/src/bin/kfpkg.rs` (commands `scan`, `info`,
`exports`). `lib.rs` registers the modules.
**Why:** Every later step reads objects out of packages. This gives the table of
contents.
**Tested how:** 11 unit tests (compact-index round trips, a hand-built tiny
package, bad magic, truncated file, reference encoding). Ran `kfpkg scan` over
the whole install. Spot-checked `info` and `exports` on KF-Farm, a `.ukx` and
`KFChar.u`. Ran clippy.
**Result:** `summary files=548 ok=548 failed=0 names=680006 imports=50717
exports=647705 megabytes=6296 seconds=3.6`. Found that KF uses its own magic
number `0x9E2A83C2` in 545 files. The other 3 use the standard `0x9E2A83C1`, and
one of those is version 121. KF-Farm's contents make sense: 2818
StaticMeshActor, 953 PathNode, 299 Light, 91 ZombieVolume, 41 KFDoorMover, 256
TerrainSector. `KFChar.u` contains class `ZombieClot`. Clippy is clean.
My first test helper had two byte-counting mistakes; the reader was right both
times. The tool used to crash with "Broken pipe" when piped into `head`; fixed.
**Still broken / not tested:** Object contents are not read. The checks prove the
tables are internally consistent (every reference in range, every data block
inside the file). They do not prove each object's data is understood.
**Next:** Step 2, the tagged-property reader (actor positions, rotations, mesh and
texture references).

## 2026-10-03 Milestone 1 step 2: tagged property reader

**Changed:** New `crates/ue-assets/src/properties.rs`. It reads property lists,
skips saved script state on actors that have it, and decodes byte, int, bool,
float, object, name, string, Vector, Rotator, Color, Plane and Scale. Other
structs, arrays and other types stay as raw bytes. `kfpkg` gained `props <file>
[CLASS]`, which writes to `work/props/`, and `scanprops`. Failure lines now
include the object's file offset.
**Why:** Positions, rotations and mesh/texture references are all stored as
properties.
**Tested how:** 2 new unit tests (array-index lengths, struct decoding), so 13
in total. Ran `kfpkg scanprops` over all 548 packages. Ran `kfpkg props` on
KF-Farm StaticMeshActor and Light. Traced the failing objects byte by byte with a
throwaway script in /tmp. Ran clippy.
**Result:** First run: 218,884 failures. My own bug: struct values were decoded
by a helper, but the "all bytes used" check looked at the wrong reader. After
the fix: `summary objects=559875 ok=559872 failed=3`. Actor classes end exactly
at the object's end: StaticMeshActor 114542/114542, Light 20777/20777, Brush
26174/26174, PathNode 17935/17935. KF-Farm sample: `StaticMesh =
HedgehogSM.bridge.wooden_bridge`, `Location = (X=-648, Y=832, Z=1936)`,
`Rotation = (Pitch=0, Yaw=-1024, Roll=0)`. The 3 failures are all stale stored
sizes. KF-Suburbia `Vehicle.PoliceCar` `Materials` says 23 bytes but holds 25.
KF-Steamland `KF_DialogueSpot1` `Dialogues` says 1093 bytes but the next tag
starts at 1094. Details are in DESIGN.md under "Known file-format quirks".
**Still broken / not tested:** Those 3 objects. The police car will be fixed in
step 5 by decoding `Materials` by type. Not investigated: 1 ZombieVolume and 4
ZoneInfo objects whose property lists end before their data ends (leftover
bytes, cause unknown). Arrays and unknown structs are not decoded.
**Next:** Step 3, exporting script source text to `work/scripts/`.

## 2026-10-03 Milestone 1 step 3: UnrealScript source export

**Changed:** New `crates/ue-assets/src/script_text.rs`, which reads `TextBuffer`
objects: an empty property list, two editor cursor numbers, then the text.
`kfpkg scripts` writes `work/scripts/<Package>/<Class>.uc`.
**Why:** The original game logic, in readable form, is the main reference for
later gameplay milestones. It stays in `work/`, out of git.
**Tested how:** Ran `kfpkg scripts`. Compared the source count with the class
count per package. Read the start of `work/scripts/KFChar/ZombieClot.uc`. Ran
`git status` to confirm `work/` is ignored.
**Result:** `summary classes=3757 sources_written=3757 megabytes=11.2`, with 0
empty, 0 missing and 0 failed across all 30 `.u` files. ZombieClot.uc starts
`class ZombieClot extends ZombieClotBase` and has readable functions such as
`ClawDamageTarget`. Only 95 files contain `defaultproperties`, and none in
KFChar. Default values are stored as binary class data instead.
**Still broken / not tested:** Binary class defaults (health, speeds, damage) are
not decoded. That is needed before milestone 5.
**Next:** Step 4, texture decoding.

## 2026-10-03 Milestone 1 step 4: texture decoding

**Changed:** New `crates/ue-assets/src/texture.rs`: mip array reader, palette
reader, and decoders for DXT1/3/5 (written here, no library), RGBA8 (stored
BGRA), L8 and P8. `kfpkg textures` scans every texture. `kfpkg texture <file>
<name>` writes a PNG to `work/textures/` and prints the decoded average colour
next to the stored `MipZero`. Added the `png` crate (0.18.1) to `ue-assets`, used
only by `kfpkg`.
**Why:** The map viewer needs textures.
**Tested how:** 5 new unit tests (DXT1 solid block, DXT1 transparency, DXT5
alpha, RGBA8 channel order, block rounding). Ran `kfpkg textures` over the
install. Exported 4 textures and viewed the PNGs. Compared decoded averages with
stored `MipZero`.
**Result:** `summary textures=8535 read_failed=0 no_mips=64 size_mismatch=85
trailing_bytes=0`. Decoded: Dxt1 4030/4030, Dxt3 966/966, Dxt5 2960/2963,
Rgba8 385/385, P8 97/161, G16 0/30. All 85 size mismatches are empty 1x1, 2x1 or
4x1 tail mips of non-square textures. The 3 undecoded DXT5 have empty data
(2 of them have 0 bytes in mip 0). PNGs: the ambulance (DXT5) and house atlas
(DXT1) look correct, with readable text and correct colours. Average-colour
check: P8 `Bus_Window2` decoded (7,6,5,255) = stored (7,6,5,255). RGBA8
`WaterCubemapImage` decoded (116,108,86,255), stored (29,27,21,63), exactly 1/4.
The `no_mips=64` textures were not investigated; they are probably cubemaps,
whose faces are separate textures.
**Still broken / not tested:** G16 (terrain heightmaps). P8 with a palette in
another package. The `no_mips=64` group is not confirmed to be cubemaps. Note:
the `grep` wrapper on this machine (rtk) mangles some piped output, so log
analysis was done in Python.
**Next:** Step 5, static meshes.

## 2026-10-03 Milestone 1 step 5: static meshes

**Changed:** New `crates/ue-assets/src/static_mesh.rs` (bounds, sections, vertex
stream, colour/alpha streams skipped, UV streams, index stream, validation).
`properties.rs`: new `Value::StructArray` and `known_struct_arrays(class)`. An
array listed there is decoded element by element as tagged structs, ignoring
its stored size. Currently only `StaticMesh.Materials`. `kfpkg meshes` scans
every mesh. Also fixed two clippy lints from step 4.
**Why:** The map viewer draws static meshes. This also applies the planned fix
for the stale-size police car from step 2.
**Tested how:** Ran `kfpkg meshes` over the install. It checks that every index
is below the vertex count, sections fit inside the index list, every UV set has
one entry per vertex, and every vertex is inside the stored bounding box. Ran
`kfpkg scanprops` again, `cargo test` (18 pass) and clippy (clean).
**Result:** `summary meshes=5021 ok=5021 failed=0 vertices=2864046
triangles=2765293 meshes_with_vertices_outside_bounds=0 no_materials=0
materials_ne_sections=0`. KF-Suburbia `Vehicle.PoliceCar` now reads `Materials =
[(EnableCollision=true, Material=Dregs.PoliceCar_Shader), (EnableCollision=true,
Material=Dregs.LightBar_Shader)]`, with 2083 vertices and 1942 triangles.
scanprops: `ok=559873 failed=2` (only the two KF_DialogueSpot remain).
**Still broken / not tested:** Collision data and vertex colours are not read.
The layout was inferred from the standard UE2 format and confirmed only by the
consistency checks above; the mesh has not been drawn yet. The police car
is 72 units long, which is small for a car. It is probably scaled on the actor
(DrawScale); not yet checked.
**Next:** Step 6, level contents (actor list, BSP geometry).

## 2026-10-03 Milestone 1 step 6: level contents (BSP, placed meshes, materials)

**Changed:** New in `crates/ue-assets/src/`: `package_set.rs` (on-demand package
loading, import resolution), `bsp.rs` (Model: vectors, points, nodes, surfs,
verts, zones, plus validation and UV helper), `level.rs` (BSP model choice,
static mesh actors, player starts), `material.rs` (material chain → texture and
blend mode). `kfpkg level <map>` loads a map the way the viewer will.
**Why:** The viewer needs everything in a map, resolved to meshes and textures.
**Tested how:** Worked out the BSP layout with a throwaway Python probe in
`/tmp/kfprobe`, outside the repo. Each field was confirmed by cross-checks: the
highest node surface index is 1109 with 1110 surfaces; the highest render-bound
index is 1008 with 1009 bounds; vertex indices used by nodes are all below the
point count. Then implemented it in Rust with those checks built in. Ran
`kfpkg level` on all 40 maps. Tried to verify the rotation formula against
rotated brushes (none exist in any map). Ran clippy and tests.
**Result:** KF-Farm: `bsp nodes=2283 surfs=1110 drawn_polygons=2206
triangles=6542 skipped={"sky_backdrop": 77}`, `mesh_actors=2895`, `unique
meshes=270 decoded=270 unresolved=0`, `unique textures=202 decodable=202`,
`player_starts=6`, 0.4 s. All 40 maps: BSP reads, 0 failed meshes, 0
undecodable textures, 0 missing packages. The first full run had 1 unresolved
mesh (KF-Icebreaker `Icebreaker_T.ic_porte_02`). Cause: that package has a
Texture and a StaticMesh with the same path, and my lookup kept only one. Fixed
by matching the import's class as well; after the fix there were 0 unresolved.
Two layout mistakes along the way, both caught by the cross-checks: I first
read the node tail as 8 bytes (it is 12, ending with a lightmap index), and my
first node-splitting script was 4 bytes off.
**Still broken / not tested:** Nothing is drawn yet. Rotation roll and side
axes are unverified (see DESIGN.md). Class defaults are unknown, so actors
whose class draws a mesh only through its defaults may be missing. The skipped
list for KF-Farm is empty, but other maps were not inspected one by one. Sky
surfaces are skipped. Terrain is not read. Lightmaps are not read.
**Next:** Step 7, the Bevy viewer.

## 2026-10-03 Milestone 1 step 7: Bevy map viewer

**Changed:** New `src/coords.rs` (Unreal↔Bevy conversion, 4 unit tests),
`src/map.rs` (map loading into Bevy: BSP meshes per material, static mesh
parts shared between actors, texture upload with BC compression, materials,
sun + ambient light, spawn point, Newell-normal test), `src/camera.rs` (fly
camera, logs position every second). `src/main.rs`: default map
KF-WestLondon, plugins registered, frame stats now use wall-clock time.
**Why:** Milestone 1 goal: explore a map.
**Tested how:** `cargo run -- --frames N` on KF-WestLondon and KF-Menu, and
reading `logs/latest.log`. All 40 maps opened for 2 frames each inside `nix
develop`. Release build run on 5 maps. Used throwaway Python checks on BSP
winding (node plane vs surface normal vs polygon order). clippy clean, 23 tests
pass. **The rendered image was not looked at**, following the "log, don't
look" rule.
**Result:** KF-WestLondon: `polygons=2055 triangles=5730 meshes=61`,
`actors=1676 spawned=1676 unresolved=0 entities=1799`, `uploaded=234
compressed=229 failed=0 megabytes=115.3`, load 0.27-0.30 s. All 40 maps load
(0.01-0.99 s) with 0 texture failures.
Problems found from the numbers and fixed:
1. Static mesh winding: my initial reversal made only 288/111985 triangles face
   their normals. Removed the reversal → 111681/111985.
2. BSP winding: a first-three-vertices test gave a mixed 1783/2055 flips. Node
   planes match surface normals 2055/2055, so the normals were right; the test
   was fooled by collinear starting points. Newell's method → 2053/2055,
   consistent on every map.
3. Texture memory 709 MB → 115 MB by keeping DXT compressed (BC1-3).
4. Frame rate showed 4 fps; real wall-clock was about 1 fps even for an empty
   scene. Cause: the monitor was off (DPMS "Monitor is Off"), which throttles
   presentation. The log used Bevy's virtual clock, which caps frames at
   250 ms; it now uses wall-clock time.
Exit crash: segfault/abort after `shutdown exit=Success` in all 42 runs
started through `timeout` (empty scene included). 7 later runs without
`timeout` exited 0. Cause unknown. Step 0's test missed this because it
reported the exit code of `tail`, not of the game.
Also: running `target/debug/kf-rs` outside `nix develop` panics
(`libxkbcommon-x11.so` not found). Expected; the libraries come from the dev
shell.
**Still broken / not tested:** Visual correctness (textures right way up,
rotations, scale, UV alignment) is unverified until a person looks. Frame rate
with the monitor on is unknown. No terrain, sky, lightmaps or collision.
Rotation roll is unverified.
**Next:** Your visual check. Then the likely next steps: terrain (needed for 18
maps), skybox, then milestone 2 (walking with collision).

## 2026-10-03 Tunnel walls: draw the sky zone (3D skybox)

**Changed:** `crates/ue-assets/src/bsp.rs` now reads the zone list
(`Model::zone_actors`) and checks node zone indices. `kfpkg` gained
`bspmaterials <map>` and `zones <map> [ZONE]` (diagnostics). `src/map.rs`
finds the sky zone (the zone whose actor class contains `SkyZone`) and puts its
BSP polygons, and static meshes standing inside its bounds, on render layer 1.
It writes a `sky_zone` log line, and the sun lights both layers. `src/camera.rs`
adds a sky camera (order -1, layer 1) at the SkyZoneInfo position that copies
the main camera's rotation every frame; the main camera no longer clears the
colour when a sky exists. Plan written to DESIGN.md first.
**Why:** You reported that KF-WestLondon tunnels have no inner walls, though the
beams over them show.
**Tested how:** You stopped at Unreal (-4190, 2099, -3685) according to the
camera log. `kfpkg zones KF-WestLondon` places that inside zone 25 (x -4608 to
-3570, y 1824 to 3872). `kfpkg zones KF-WestLondon 25` lists its polygons. Ran
the viewer on WestLondon, Farm, Offices, Manor and BioticsLab and read the
`sky_zone` line. Ran clippy and tests (23 pass).
**Result:** The tunnel's east wall (nodes 373/374) is flagged `FAKE_BACKDROP`
(0x400080): a window onto the sky zone, which the viewer skipped. The ceiling
and west wall are `Engine.BlackTexture` and were drawn black. The open ends
are zone portals. Sky zones found: WestLondon zone 8 (6 BSP polygons, 4 props:
`London_Skybox`, 2 fog cylinders, a fog ring); Farm zone 2 (6 polygons,
5 props); Offices zone 28 (10, 7); Manor zone 12 (6, 4); BioticsLab none
(unchanged behaviour). I first suspected alpha-channel cutouts; the
`bspmaterials` report ruled that out (walls with low-alpha textures are already
opaque).
Exit crash, corrected: it is not tied to `timeout`. In 14 direct runs, 8
crashed (segfault or abort) after `shutdown exit=Success`, including 4 of 6
with no map loaded.
**Still broken / not tested:** Not looked at. Whether the tunnel now looks
right is for you to check. Sky zone scaling, scrolling and fog are not done.
Choosing sky props by bounding box is approximate. The exit crash is not
investigated.
**Next:** Your check of the WestLondon tunnels. Then candidates: the exit crash
(core dumps exist, so a backtrace is possible), terrain, milestone 2.

## 2026-10-03 Screenshots and saved camera views

**Changed:** New `src/screenshot.rs`: F12 saves a PNG to `work/screenshots/` plus
a `.txt` with the command that recreates the view. `--screenshot N` takes one
at frame N and quits once it is written. `src/camera.rs`: `--camera
X,Y,Z,YAW,PITCH` start pose (Unreal units, radians, same numbers as the
camera log). `src/main.rs`: both options parsed. New `docs/test-views.md`
lists reusable views.
**Why:** You suggested it, so visual bugs can be checked and rechecked. You
allowed me to view screenshots (an exception to "log, don't look"; saved to
memory).
**Tested how:** `--camera -4190,2099,-3685,-4.817,-0.062 --screenshot 10` on
KF-WestLondon. Bad `--camera` input prints usage.
**Result:** The PNG was written (`event=screenshot ... automatic=true`, then
`screenshot_saved_quitting`, then `shutdown exit=Success`). The image showed the
street, props and the sky zone (St Paul's dome on the skyline).
**Still broken / not tested:** F12 itself was not pressed by me (the monitor was
off); it uses the same code path as the automatic shot. Not tested on
Windows.
**Next:** Use it to check the tunnel.

## 2026-10-03 Fix: mirrored props were invisible (KF-WestLondon tunnel)

**Changed:** `src/map.rs`: actors whose scale has a negative product
(mirrored) get a separate mesh copy with reversed triangle winding. The
`static_meshes_loaded` log line now includes `mirrored_actors`.
**Why:** You reported the tunnel was still wrong after the sky zone change.
**Tested how:** Screenshots from inside and at the entrance of the tunnel
(views in `docs/test-views.md`). Listed every actor in and near the tunnel
from `kfpkg props`, then searched by mesh name. Ran clippy and tests (23 pass).
**Result:** The tunnel shell is the `Car_Tunnel` mesh, placed with
`DrawScale3D=(X=1, Y=-1, Z=1)` and yaw -32768. Its mesh bounds x -1693..-943,
y -4..2102, after mirror, rotation and placement at (-5533, 1800), land exactly
on tunnel zone 25 (x -4590..-3840, y 1796..3902). Mirroring flipped its
winding, so back-face culling hid its inside. Before the fix: black void
between the arches. After: curved brick shell visible along the whole tunnel
(screenshot). 43 of 1676 actors on WestLondon are mirrored (tunnel pieces,
hotel windows and pillars, metal sheets, railings, 2 doors), and all are fixed
by the same change. The first diagnosis (sky-zone window) was real but not the
cause of what you saw. Lesson: a screenshot found in minutes what the numbers
did not.
**Still broken / not tested:** Other maps not inspected by screenshot. The
black flat ceiling and wall in the tunnel zone are hidden behind the shell
now, as in the original.
**Next:** Your look at the tunnel, then the quit crash, terrain or milestone 2.

## 2026-10-03 Fix: actor `Skins` material overrides were ignored (grey block)

**Changed:** `crates/ue-assets/src/level.rs`: `MeshActor.skins` read from the
`Skins` property (an array of object references). `src/map.rs`: each mesh part
remembers its section index, and an actor's non-null `Skins[section]`
replaces the mesh's material. Log line now includes `skins_applied` and
`invisible_parts`. `kfpkg level` lists materials without textures, the
`Skins` count, and BasicCube actors. New view in `docs/test-views.md`.
**Why:** Your screenshot showed a large grey block in a plaza on KF-WestLondon.
**Tested how:** `kfpkg level KF-WestLondon` named the two untextured
materials. The block is two `Mover`s (Mover1, Mover2) using
`GKStaticMeshes.basicShapes.BasicCube` (no material of its own), scaled
13x5x4 and 7x3x1, tagged `ChopperTakeOffTC`, rising 1342 units when
triggered. Their `Skins` is `KillingFloorLabTextures.LabCommon.voidtex`, a DXT3
alpha texture whose decoded and stored average colour are both (0,0,0,0): fully
transparent. Retook your view and the tunnel view by screenshot. Ran clippy and
tests (23 pass).
**Result:** `skins_applied=169 invisible_parts=0`. The grey block is gone (now
invisible, as in the game), and the plaza behind it shows. The sky changed
to an orange sunset skyline, because the skybox mesh has a Skins override too.
The tunnel view is unchanged. You note the helicopter (tag `ChopperTakeOffTC`)
flies away as you spawn, so these blocks are probably barriers that move away
at the start.
**Still broken / not tested:** `materials_without_texture` is now 3 on
WestLondon (was 2); the new one was not identified. Other maps not inspected.
**Next:** Your call: quit crash, terrain, or milestone 2.

## 2026-10-03 Fix: structs inside properties are nested tagged lists

**Changed:** `crates/ue-assets/src/properties.rs`. Structs other than Vector,
Rotator and Color are read as nested tagged lists by their own tags
(`Value::TaggedStruct`), ignoring the stored size when it is too small.
`PropertyList.stale_sizes` counts the overruns. Removed the wrong binary
decoders for Plane and Scale.
**Why:** Starting terrain, TerrainInfo on KF-Farm read only 3 properties. The
`Layers[0]` struct stated 228 bytes, but its nested list ended one byte later;
the reader took that `None` as the end of the whole object.
**Tested how:** A throwaway script tried reading every struct value as a nested
list, across KF-Farm, KF-WestLondon, KF-Steamland and KFMod.u: Vector (12841
no / 16695 garbage) and Rotator are never valid lists, while Scale 3912/3912,
PointRegion 14476/14476, RangeVector, Range, Plane and TerrainLayer always are.
Reran scanprops, meshes and textures over the whole install. Tests pass.
**Result:** TerrainInfo now reads 23 properties: 7 layers with textures,
AlphaMaps and TerrainMatrix, plus visibility and edge bitmaps. Install-wide
checks unchanged: `ok=559873 failed=2`, meshes 5021/5021, textures 8535 with 0
read failures.
**Still broken / not tested:** The 2 KF_DialogueSpot objects (an array, not a
struct) still fail.
**Next:** Terrain.

## 2026-10-03 Terrain

**Changed:** `texture.rs`: G16 decoding (`g16_values`, 1 new test). New
`crates/ue-assets/src/terrain.rs` (TerrainInfo: heights, layers, visibility
and edge-turn bitmaps, `vertex`, `height_at`). `kfpkg terrain <map> [X,Y]`
gives layers, a height check against PathNodes/PlayerStarts, a hole check, and
the ground height at a point. `src/map.rs`: `spawn_terrains` (grid mesh,
normals, holes, diagonal per EdgeTurnBitmap, one mesh per layer with
per-vertex weights, layer 0 opaque and others additive, sky-zone terrain on
the sky layer), `texture_opaque` (alpha forced opaque), `MeshBuilder` vertex
colours. Screenshot repro commands now use frame 60.
**Why:** 18 of 35 KF maps had no ground.
**Tested how:** `kfpkg terrain` on KF-Farm, KF-Manor, KF-Crash and KF-Hell. All
21 terrain maps loaded in the viewer, with terrain log lines read. Screenshots
of KF-Farm (5 views), KF-Crash and KF-Hell. clippy clean, 24 tests pass.
**Result:** Height formula confirmed: median actor-above-ground 43.8 (KF-Farm,
805/847 within 16 units), 43.9 (Manor), 43.8 (Crash), 43.8 (Hell). Hole bit
polarity confirmed (847 vs 112 on KF-Farm). All 21 maps load terrain with 0
errors. KF-Farm: 256x256, 55938 visible triangles, 7 layers. Screenshots show
correctly blended terrain under the props.
Problems met:
1. The first eye-level KF-Farm screenshot showed black ground. The same view
   re-taken was fine, and top-down shots were fine. Most likely, the new
   shaders (vertex colour, additive) were still compiling at frame 10; Bevy
   skips objects until their shader is ready. Not proven. Screenshots now
   use frame 60.
2. KF-Crash and KF-Wyre drew 1 of 4 layers: a layer with no AlphaMap was
   treated as full coverage and hid the others. Such layers are unpainted;
   they now count as zero. After the fix: 3 of 4 drawn (the 4th is the
   unpainted one).
**Still broken / not tested:** Decoration layers (grass meshes), terrain
lighting, inverted terrain (no map has one; untested). 18 of 21 maps not seen
by screenshot.
**Next:** Milestone 2: walking with collision. First, decode class defaults
for movement values.

## 2026-10-03 Milestone 2 step 1: class defaults with inheritance

**Changed:** `properties::find_class_defaults` (locates a class's default
property list from the end of its data). New
`crates/ue-assets/src/class_defaults.rs` (lookup up the class chain).
`level.rs`: `read_level_with` uses class defaults for DrawType, bHidden and
collision flags (`MeshActor.blocks_player`), and collects blocking brush
volumes and PathNode positions. `kfpkg defaults <Package.Class>`.
**Why:** Movement values and collision flags live in binary class defaults.
**Tested how:** `kfpkg defaults` on Engine.Pawn, UnrealGame.UnrealPawn,
XGame.xPawn, KFMod.KFPawn, KFMod.KFHumanPawn, Engine.Actor,
Engine.PhysicsVolume, Engine.StaticMeshActor, Engine.BlockingVolume and
KFMod.KFZombieZoneVolume. `kfpkg level` on KF-WestLondon and KF-Farm.
**Result:** Engine.Pawn: GroundSpeed 440, JumpZ 420, AccelRate 2048,
BaseEyeHeight 64 (standard UE2 values). KF player: GroundSpeed 200, AccelRate
1000, JumpZ 325, AirControl 0.15, CollisionRadius 20, CollisionHeight 50,
BaseEyeHeight 44; gravity -950. Every class's defaults were found except
Core.Object (the root, which has none). Drawn-actor counts are unchanged
(WestLondon 1676, Farm 2895), so the old hard-coded class list matched.
Blocking: WestLondon 757 of 1676 mesh actors plus 87 volumes; Farm 1823 of
2895 plus 154. KFZombieZoneVolume blocks players ("blocks ONLY humans",
according to its script).
**Still broken / not tested:** The defaults finder is a search, not a parse
of the class format; a wrong match is possible in principle. The values
checked so far are all plausible.
**Next:** Collision world.

## 2026-10-03 Milestone 2 steps 2-4: collision world and walking

**Changed:** `Cargo.toml`: avian3d 0.7.0 (shape queries). `bsp.rs`:
`Model.polys`, `read_polys`. `static_mesh.rs`: `section_collides` from
EnableCollision. New `src/collision.rs` (static trimesh colliders for BSP,
props, terrain and each blocking volume, plus a PathNode ground-ray check).
`src/map.rs` collects collision geometry. New `src/walk.rs` (cylinder walker:
UE2-style CalcVelocity, ground move with step-up and slide, floor snap,
falling, air control, jump, V toggle, start settling, blocked logging, 2 unit
tests). `src/camera.rs`: flying pauses while walking. `main.rs`: `--walk`,
`--autowalk SECONDS`. `kfpkg polys`, `kfpkg brush <map> <actor>`.
**Why:** Milestone 2: walk the maps.
**Tested how:** `kfpkg polys` over all maps. Viewer logs on KF-WestLondon,
KF-Farm and KF-Offices (collision_check). `--autowalk` runs on KF-Farm and
KF-WestLondon with screenshots. clippy clean, 26 tests pass.
**Result:** Polys: `polys_objects=41220 failed=0`; every Model's polys
reference is valid. Ground-ray check after the fixes: WestLondon 210/210 hits,
Offices 183/183, Farm 953/953 (948 within 8 units of 44); 0 rays start
inside a collider. Walking: KF-Farm 4 s at 200 units/s over a hill
(floor normal 0.87-0.98), braking to 0 in ~0.5 s. KF-WestLondon 1454 units at a
steady 200 units/s, down and up a 10-unit curb, stopping at BlockingVolume12
(screenshot: scaffolding).
Problems found and fixed along the way:
1. 50/210 PathNodes on WestLondon started inside volumes. 25 were in zombie-only
   volumes (correct). 25 were in BlockingVolume44/45, whose convex hulls filled
   the hollow tunnel arches. Fixed by using real polygons.
2. The walker started 20 units sunk into the ground (it was placed from the fly
   camera, 30 units above PlayerStart). Fixed by settling onto the floor on
   entering walk mode: centre 51 above the terrain at the KF-Farm spawn.
3. Walking stopped after 64 units on WestLondon, blocked by the invisible
   Mover barrier (BasicCube 13x5x4, face exactly at x=-3194). Movers are now
   excluded from collision until they are simulated.
4. Walking stuttered (speed 0 to 200, 513 units in 8 s): resting at exactly the
   cast's stop distance made the floor a hit at distance 0 for every
   horizontal sweep. Rest at two skins: smooth 200 units/s.
5. Speed showed 200 while blocked; velocity now becomes the actual movement.
**Still broken / not tested:** Not play-tested by a person. Movers (doors,
windows, barriers) don't block. No crouch, ladders or water. Step height 35
and walkable slope 0.7 are engine constants not found in data. A brief
on-ground flicker right after stopping (vertical -16/-32 for half a second)
was not investigated. Per-map gravity (PhysicsVolume overrides) is not read;
-950 is always used.
**Next:** Your play test.

## 2026-10-03 Field of view set to KF's (73.74° vertical)

**Changed:** `src/camera.rs`: both cameras use a 73.74° vertical field of view
instead of Bevy's 45° default. The `camera_spawned` log line includes it.
**Why:** You reported walking pace and jump height feel wrong, while the
mechanics feel right. I checked the scripts: KF computes GroundSpeed each frame
as 200 x health x weight factors, plus a melee bonus, x perk and inventory
modifiers. With the 9mm at full health that is about 200, the value used. Jump
uses JumpZ 325, and no map overrides gravity (-950). The numbers match the
game, but the view did not: PlayerController DefaultFOV = 90 and
KFPlayerController bUseTrueWideScreenFOV = true give 73.74° vertical. A 45°
view is ~1.7x zoomed in, which makes motion look slower and jumps lower.
**Tested how:** Tunnel test view retaken by screenshot. Log shows
`vertical_fov_deg=73.74`. clippy clean.
**Result:** Much wider view (the whole bridge wall, tunnel and street in
frame).
**Still broken / not tested:** Whether pace and jump now feel right is for you
to judge. Melee (knife) speed bonus, weight and perk modifiers, KF's view bob
and stamina-limited jumping (ROPawn: 2 s between jumps, stamina cost) are not
implemented.
**Next:** Your re-test of pace and jump.

## 2026-10-03 Play-test result: walking

**Changed:** README only.
**Why:** Record your play-test.
**Tested how:** You played `--walk` on KF-WestLondon.
**Result:** Mechanics and collisions feel correct; everything that should block
does, except movers (known). After the field-of-view fix, pace and jump "feel
way better". Exact values not yet confirmed; a fair comparison needs weapons
(the knife speed bonus, weight).
**Still broken / not tested:** As in the walking entries above.
**Next:** Your choice: weapons and first-person view, movers (doors), or the
exit crash.

## 2026-10-03 Milestone 3 steps 1-2: skeletal meshes and animations

**Changed:** New `crates/ue-assets/src/skeletal.rs` (SkeletalMesh: textures,
material slots, scale/origin/rotation, reference skeleton, animation reference,
raw points/wedges/triangles/influences found via the lazy-array chain;
MeshAnimation: bones, motion chunks with tracks, sequences). `kfpkg
skelmeshes`, `kfpkg anims`. Weapons plan written to DESIGN.md first.
**Why:** First-person weapons are skeletal meshes with animations.
**Tested how:** Worked out the layouts with probes in /tmp/kfprobe on
`KF_Weapons_Trip.9mm_Trip` / `9mm_anim`. Each structure was confirmed by
internal checks: unit quaternions and parents before children for bones;
lazy-array end offsets matching exactly; index ranges; weights summing to 1;
the sequence list ending exactly at the end of the data. Then ran both scans
over the install. Settled the rotation convention by measurement.
**Result:** `summary skeletal_meshes=398 ok=398 failed=0
meshes_with_bad_weights=0`; `summary mesh_animations=188 ok=188 failed=0
sequences=3647 tracks=186148 tracks_with_bad_keys=0
tracks_with_non_unit_rotations=29` (normalised when used). 9mm: 62 bones
(RootBone, Hands_Hub01, Hands_LArmCollarbone...), 7382 points, 10524
triangles, 2 materials; sequences Fire, Idle, Idle_Iron, Fire_Iron, LightOn,
PutDown, Reload, Select. Convention: non-root rotations are conjugated (vertex
to bone median 2.25 vs 46; Idle frame 0 edge change 1.5% vs 11%).
Dead ends: expected a single raw-array start after the bones (it is behind the
LOD models); assumed the sequence float was trailing (it is leading); a
3-byte count assumption for 7382 (it is 2 bytes).
**Still broken / not tested:** The 62-byte block after the LOD settings is
assumed to be impostor settings plus unknown fields; it is zero in all 398
meshes. The leading sequence float's meaning is unknown. 23 meshes have
animation bones not in the mesh (matched by name).
**Next:** Draw and animate the weapon in first person.

## 2026-10-03 Milestone 3 steps 3-6: first-person weapons

**Changed:** New `src/weapon.rs` (loads KFMod.Knife and KFMod.Single through
class defaults: mesh, Skins, PlayerViewOffset, DisplayFOV, fire mode
animation and rate, bSpeedMeUp; CPU skinning; weapon camera; action state
machine; scripted test input). `src/walk.rs`: ground speed includes the
weapon bonus. `src/screenshot.rs`: `--screenshot` takes a list of frames.
`src/main.rs`: `--input FRAME:ACTION,...`. Docs and test views updated.
**Why:** Milestone 3.
**Tested how:** Screenshots on KF-WestLondon: placement, 9mm fire, switch to
knife, knife idle, knife swing (frames 302-326). Offline Python check of the
knife fire2 bone positions. Read KFWeapon/KFPlayerController FOV code and
Pawn.CalcDrawOffset. Log lines weapon_loaded, weapon_action and weapon_pose.
clippy clean, 26 tests pass.
**Result:** Both weapons load (Knife 51 bones / 8284 triangles, 9mm 62 / 10524)
in 0.10 s. The action sequence runs as logged: Select, Idle (5 fps), Fire
(45 fps = 30 x 1.5), PutDown, switch, Select, Idle, knife Fire (a random one of
Fire/Fire2/fire3/Fire4). First screenshot: weapon tiny in the corner. Fixed by
applying MeshScale (5) and CalcDrawOffset's 0.9/DisplayFOV*100 factor (found in
Engine/Pawn.uc). Now the sleeve and hand hold the pistol in the lower right
like KF. The knife swing goes right then sweeps left (bones stay within 36
units). Mid-swing a camouflage-textured upper sleeve passes close to the
camera; whether KF shows that too is not known.
**Still broken / not tested:** Not play-tested by you. Camo sleeve during the
knife swing unconfirmed. No damage, ammo, muzzle flash, sound, weapon bob or
lighting on weapons (unlit). Only the two starting weapons.
**Next:** Your play test.

## 2026-10-03 View and weapon bob; reload timing

**Changed:** `src/walk.rs`: ViewBob resource computed with KFPawn.CheckBob's
formula; the camera follows it while walking; the walk log includes bob values
(now every 0.1 s). `src/weapon.rs`: weapon offset by Pawn.WeaponBob with each
weapon's BobDamping. Reload is allowed once the fire cooldown has passed, not
only after the fire animation finishes.
**Why:** Your play test: knife and pistol fire look great, but there was no
walking motion and reload is broken. The KF weapons have no walk or run
sequences; the motion is UE2's code-driven bob.
**Tested how:** Read Engine/Pawn.uc (CheckBob, WeaponBob, CalcDrawOffset),
KFMod/KFPawn.uc (CheckBob override), ROPawn.EyePosition; defaults: Bob 0.006,
BobSpeedModifier 0.9, BobScaleModifier 1, BobDamping 6 (9mm) / 8 (knife).
`--autowalk 3` on KF-Farm with the bob logged. Reload: screenshots through it,
and an offline check of the 9mm_Mag bone. clippy clean, tests pass.
**Result:** Bob side +-1.08, vertical +-0.81 units at 200 units/s, period ~0.85 s,
matching the formula. Reload: the animation plays (60 frames at 30 fps = 2 s,
matching ReloadRate 2). The magazine bone (233 vertices) follows the left palm
mid-reload, so the data looks correct. Possible cause of "broken": R was
ignored during the 1.36 s fire animation; now allowed after the 0.175 s
cooldown. Not confirmed this is what you saw.
**Still broken / not tested:** What exactly is wrong with reload is not known
yet (asked you). Landing bob is not done. The bob has not been seen in motion by
me (screenshots are still frames).
**Next:** Your description of the reload problem; your feel of the bob.

## 2026-10-03 Play-test result: weapons

**Changed:** README only.
**Why:** Record your play-test.
**Tested how:** You played `--walk` with both weapons.
**Result:** Knife attack, pistol fire, reload and walking bob all work. The
reload problem was the input timing (R ignored during the fire animation),
fixed in the previous entry.
**Still broken / not tested:** Landing bob, damage, ammo, muzzle flash,
sound, other weapons.
**Next:** Your choice.

## 2026-10-03 Milestone 4 steps 1-4: Clots that chase you

**Changed:** New `src/skinned.rs` (skinning moved out of weapon.rs, plus lit
normals, mesh instances, `upload_to`). `src/weapon.rs` uses it. New
`src/zed.rs` (Clot class from defaults, UE2 mesh-to-world transform, chase
in a straight line with the shared cylinder Mover, turning at RotationRate,
melee animations in range, falling, Z to spawn, X to pause, `zed_*` log
lines). `walk.rs`: Mover is public with `new(radius, half_height)` and
`snap_to_floor`. `class_defaults.rs`: `get_array_names`. `main.rs`: `--zed`.
Plan in DESIGN.md first.
**Why:** You agreed to zeds next.
**Tested how:** Weapon test views re-shot after the refactor (identical).
`--walk --zed` runs on KF-WestLondon with screenshots of walk and melee.
Offline Python check of the Clot's facing from foot bones. clippy clean, 26
tests pass.
**Result:** Clot: 25 bones, 4400 triangles, GroundSpeed 105, RotationRate yaw
45000, MeleeRange 20, collision 26x44; melee anims ClotGrapple, Two, Three.
Log: spawned 300 units ahead, chased at 105 units/s (300 -> 221 -> 116), went
to melee at 74 units, then cycled the three grapples. Screenshot: facing you,
arms forward, walking on the road.
Dead end: from the melee screenshots I thought the facing was 90 degrees off;
a no-RotOrigin test was clearly side-on. The foot-bone calculation showed the
stored RotOrigin is right; the grapples just twist.
**Still broken / not tested:** Not play-tested by you. No pathfinding (gets
stuck on walls), no damage either way, only the Clot, feet ~6 units into the
floor, zeds don't collide with each other or the player. Animation speed is
not scaled to movement speed.
**Next:** Your play test.

## 2026-10-03 Play-test result: Clots

**Changed:** README only.
**Why:** Record your play-test.
**Tested how:** You played `--walk --zed`.
**Result:** "Looks great": appearance, movement and animations accepted.
**Still broken / not tested:** As in the previous entry (no pathfinding, no
damage, only the Clot, feet ~6 units into the floor).
**Next:** Your choice.

## 2026-10-03 Milestone 5: combat

**Changed:** New `src/combat.rs` (messages ShotFired / MeleeSwing /
PlayerDamaged; player health and respawn; HUD; ray-vs-cylinder and KF's
IsHeadShot; damage, headshots and decapitation; kill count; 3 unit tests).
`src/weapon.rs`: combat stats, ammo and headshot multiplier read from the
fire mode, damage type and ammo classes; spread and damage rolls; knife damage
delay; reload refills the magazine; dry fire. `src/zed.rs`: health, head
health, head sphere and extended collision tracked each frame, melee damage
halfway through attacks, death (KnockDown held at its lowest frame) and corpse
removal after 10 s, integer defaults. `src/skinned.rs`: `find_bone`,
`pose_with_bones`, `pose_collapsed` (decapitation).
`class_defaults.rs`: unchanged. Plan in DESIGN.md first.
**Why:** You chose combat.
**Tested how:** Scripted runs on KF-WestLondon with `--walk --zed --input`:
9mm fired every 15 frames at an approaching Clot (level and pitched down),
and knife swings while being grabbed. Screenshots of the HUD and corpse.
Log lines hit / miss / headshot_check / player_hit / zed_died. clippy clean,
29 tests pass.
**Result:** 9mm: 5 body shots (27.1, 25.5, 30.3, 27.1, 26.2) killed a 130-health
Clot. Pitched down: the 5th shot passed 4.8 units from the head centre and was a
headshot (28.8 damage), decapitating. Knife: 19 body, 23.8 headshot (x1.25);
head health 25 -> 1.2 -> 0, decapitated. Grabs: 6 damage every ~1.25 s (100 ->
70). HUD: "HEALTH 70  KILLS 1".
Problems found and fixed along the way:
1. Every shot missed at first: the eye (95) is above the Clot's main cylinder
   (88). KF adds an extended head-height cylinder; now implemented.
2. Health read as 100 (default) because Health is an integer property.
3. Grabs never damaged the player: my melee start distance (76) was beyond
   KF's damage reach (74).
4. The corpse stood back up (KnockDown recovers); now held at its lowest frame.
5. Decapitated heads stayed on; now collapsed.
**Still broken / not tested:** Not play-tested by you. No blood, muzzle flash,
sound, ragdolls, zed flinch animations, grab slowing, perks. Zeds still chase
in a straight line. Respawn is instant.
**Next:** Your play test.

## 2026-10-03 Pawn collision (player and zeds block each other)

**Changed:** New `src/pawn_collision.rs`: clips the horizontal part of a move
so one upright cylinder cannot enter another, sliding along its side (swept
circle test). `walk.rs` clips the player's moves against living zeds before
the world sweep; `zed.rs` clips each zed's move against the player (only when
walking, not flying) and the other living zeds. `Zed::blocking_cylinder`
(none once dead). New scripted input action `zed` (spawn a zed); extra zeds
spawn 60 units apart sideways. Plan in DESIGN.md ("Play-test fixes after
combat").
**Why:** Your play test: "there's no collision with enemies".
**Tested how:** 5 new unit tests (head-on stop, glancing slide, ignoring a
cylinder above, overlap may separate but not close, vertical part kept).
Scripted runs on KF-WestLondon: (1) `--walk --zed --autowalk 4`, walking into
an attacking Clot; (2) `--walk --zed --input 8:zed,10:zed`, three Clots
converging on a standing player; (3) the earlier knife test, for regressions.
clippy clean, 34 tests pass.
**Result:** (1) The player stops at 47 units from the Clot's centre (20 + 26
+ 0.5 gap) and stays there (`walk_blocked by=zed 0`). (2) Pair gaps never
under 52 (26 + 26); the side Clots are deflected by the middle one and all
three attack from ~65 units. (3) Same knife kill as before (23.8 headshots,
decapitated).
**Still broken / not tested:** Not play-tested by you. You cannot stand on a
zed's head (cylinders only block sideways). Zeds still chase in a straight
line, so a crowd can push the back ones into walls.
**Next:** Iron sights (ADS).

## 2026-10-03 Class defaults: fix false starts

**Changed:** `properties::add_class_property_names` (new, shared by the game
and kfpkg): only properties declared directly by a class count as valid
default names, not struct members or function parameters. New
`kfpkg defaults --all`.
**Why:** KFMod.SingleFire's defaults began at a false start ("Max", "Min" are
Range struct members), losing FireAimedAnim, which iron sights need.
**Tested how:** `kfpkg defaults --all` before and after (231 classes changed);
compared old and new output for Mutator, Ladder, LevelInfo and the 11 classes
that lost their defaults; `kfpkg level` on all 40 maps with the old and new
rule.
**Result:** Every changed class I inspected went from garbage first entries
to correct ones; the 11 "lost" classes only ever had one garbage entry. All 40
maps load identically (only hash-map print order differs).
**Still broken / not tested:** Only a sample of the 231 changed classes was
inspected by hand.
**Next:** Iron sights.

## 2026-10-03 Iron sights (ADS) for the 9mm

**Changed:** `weapon.rs`: IronSights read from the class defaults
(PlayerIronSightFOV 75, ZoomedDisplayFOV 65, ZoomTime 0.25, FastZoomOutTime
0.2, IdleAimAnim Idle_Iron, FireAimedAnim Fire_Iron). Right click (or
scripted `aim`) toggles; refused while switching, reloading or in the air;
reload and switch zoom out fast. Zoom blends the player FOV, weapon FOV and
draw offset (to zero); spread halved while aimed. `camera.rs`: new `ViewFov`
resource applied to the main and sky cameras; `vertical_fov` moved here.
**Why:** Your play test: "head shots are hard because there's no ADS".
**Tested how:** Scripted runs on KF-WestLondon with screenshots at hip, aimed,
and aimed firing; a second run with aim, fire, reload, aim during reload,
switch to knife, aim with knife. clippy clean, 34 tests pass.
**Result:** Sights land on the screen centre when aimed (screenshot). Log:
zoom done in 0.23 s, view FOV 90 -> 75, weapon FOV 70 -> 65, idle_iron and
fire_iron play, reload drops out (0.18 s), aim refused during reload and with
the knife.
**Still broken / not tested:** Not play-tested by you. Not done: ZoomInRotation
(a small roll during the zoom), mouse sensitivity change while zoomed (not
checked whether KF does this), aim sounds. The transition path is a straight
blend; KF's native interpolation may curve differently.
**Next:** Your play test; then dead-zed behaviour (see question).

## 2026-10-03 Investigated: dead Clots stay standing

**Changed:** `zed.rs`: corpses log `zed_corpse` (seconds dead, sequence,
frame) once a second. Findings in DESIGN.md.
**Why:** Your report: the Clot freezes on death instead of falling.
**Tested how:** Scripted 9mm headshot kill on KF-WestLondon with screenshots
at 0.02, 0.15, 0.32, 0.65 and 1.65 s after death.
**Result:** The death animation does play (frame 26.5 at 0.9 s), but KnockDown
only staggers, clutches and crouches; it never lies down. KF uses Karma
ragdolls (KarmaData/*.ka) instead of a death animation. You chose to leave it
for now.
**Still broken / not tested:** Corpses still end crouched on their feet.
**Next:** Your choice of next feature.

## 2026-10-03 Ragdolls, step 1 (Clot): mostly working, one joint broken

**Changed:** New `crates/ue-assets/src/karma.rs` (reads KF's Karma ragdoll
XML; `kfpkg karma`). New `src/ragdoll.rs`: on death, one avian body per
ragdoll part with KF's shapes and masses, ball and hinge joints with KF's
limits, skinning from the bodies. `zed.rs`: ragdoll on death with KF's launch
(0.6 x zed velocity + RagDeathVel 100 along the shot, capped spin); corpses
last 30 s (KF's RagdollLifeSpan, was 10). `combat.rs`: records hit point and
direction. `collision.rs`: collision layers (ragdolls collide with the level
only; movement and bullets ignore them); gravity 950. `Cargo.toml`: avian
`xpbd_joints` feature on. `skinned.rs`: `skin`, `bind_pose`. Plan in
DESIGN.md first.
**Why:** You asked for ragdolls and gore.
**Tested how:** 2 parser tests, 2 coordinate tests, 2 headless avian joint
tests; load-time checks against the bind pose and the walk cycle; scripted
9mm kills on KF-WestLondon with logs (`ragdoll`, `ragdoll_joint_gap(s)`,
`ragdoll_parts`) and screenshots. clippy clean, 40 tests pass.
**Result:** Every zed ragdoll in the game parses. The Clot ragdoll fits the
mesh exactly (anchor gaps 0.00, axis angles 0.0). The body falls and rests in
~1.6 s; legs and arms stay joined. Problems found and fixed on the way:
(1) joints did nothing at all: the `xpbd_joints` feature was off; (2) hinge
limits mirrored by me: wrong, the file's signs are right; (3) the file's
inertia values are implausibly small and made limbs spin up; now computed from
the shapes.
**Still broken / not tested:** The ribcage-to-spine3 joint pulls apart (15 to
82 units at rest), detaching the upper body from the spine. Five attempts
failed; written up in `STATUS.md`. Spin magnitude is capped because
RagInvInertia's units are unknown. Elliptical cones are approximated by
circles. Not play-tested by you. Gore not started.
**Next:** Your call (see STATUS.md "Ideas not tried yet").

## 2026-10-03 Ragdolls: fix the ribcage joint (minimum part mass)

**Changed:** `src/ragdoll.rs`: part masses below 0.05 file units are raised
to 0.05 (`MIN_PART_MASS`); the load log lists raised parts. STATUS.md marked
resolved.
**Why:** The ribcage-to-spine3 joint pulled apart (STATUS.md). Cause: the
file gives chr_spine3 a mass of 3.14e-6 (a placeholder, ~35,000x lighter than
the ribcage); a near-massless link between two heavy bodies cannot pass joint
corrections along, so the solver only ever moved spine3. Found by listing
all part masses; spine1 (0.0103, the next lightest) was the only other joint
with any gap (0.1).
**Tested how:** Five scripted kills on KF-WestLondon (three 9mm headshots, one
body-shot kill, one knife headshot) with the joint-gap log and screenshots.
clippy clean, 40 tests pass.
**Result:** Ribcage joint gap 0.1-0.2 units in every run (was 15-82); all
other joints 0.0-0.1. Bodies rest in 1.4-2.0 s. Screenshots: headless body
lying flat; body-shot corpse face down with head on; knife corpse on its back,
arms up. Raised parts: chr_spine1 (0.0103), chr_spine3 (3.14e-6).
**Still broken / not tested:** Not play-tested by you. In the knife run no
"rested" line was logged before the run ended (not checked further). Spin
magnitude capped (unknown units); elliptical cones approximated by circles.
**Next:** Ragdoll step done for the Clot; then severed heads and limbs.

## 2026-10-03 Ragdolls: stop over-twisting and jiggling

**Changed:** `karma.rs`: reads CONE_TYPE and TWIST_TYPE. `ragdoll.rs`:
TWIST_TYPE 2 joints are locked against twist, type 1 limited to the file's
angle, type 0 free; the soft (compliant) limits left over from the joint hunt
are removed; joint angular damping 1 (the file's CONE/TWIST_DAMPING);
corpses sleep below 0.3 m/s and 1.0 rad/s. New logs: `ragdoll_limits_at_death`,
`ragdoll_limits` (largest swing/twist per joint vs its limit), angular speed,
current twists and sleeping body count in the `ragdoll` line. New test: ball
joint limits act on the right axes.
**Why:** Your play test: corpses jiggle on the floor; the head and ribcage
can spin ~180 degrees.
**Tested how:** Scripted 9mm kills on KF-WestLondon, comparing configurations
on the same logged numbers (largest twist per joint, late speed and spin,
sleeping bodies); headless avian tests; screenshots. clippy clean, 41 tests.
**Result:**
- Twist: every type-2 joint was being allowed +-90 degrees (I had read
  TWIST_HALF_ANGLE as the range), and the neck, mid-spine and thighs went to
  their 90 degree limits within a second, even with no launch spin. With
  locked twist the neck reaches 2 degrees, mid-spine 7, thighs 13-18.
- The soft limits from the joint hunt let joints overshoot; removed (they had
  not helped that bug).
- Jiggle: parts kept turning at ~0.5 rad/s forever, above avian's default
  sleep threshold (0.15), so corpses never slept. With 1.0 rad/s / 0.3 m/s all
  16 parts sleep within ~2-3 s.
- Ruled out: launch spin (twist reached the limits without it), joint damping
  1 vs 10 (no clear change; 5 made the start more violent).
**Still broken / not tested:** The TWIST_TYPE numbering (0 free, 1 limited,
2 locked) is from my memory of MathEngine's Karma API, not confirmed from
KF's files; it fits the data and the result. CONE_TYPE is 2 everywhere
(possibly Karma's "slot" cone); still treated as a circular cone. The death
pose starts some joints past their swing limits (collars 18 vs 6 degrees), so
they snap on the first step. Not play-tested by you.
**Next:** Your play test.

## 2026-10-03 Decapitation: headless zeds bleed out (KF rules)

**Changed:** `combat.rs` `damage_zed` follows KFMonster.TakeDamage /
RemoveHead: the head comes off when head health reaches 0 or the hit exceeds
remaining health; the head explosion deals (hit + 0.25 x HealthMax) x
HeadShotDamageMult more; every hit on a headless zed gets the headshot
multiplier. `zed.rs`: `remove_head` starts a bleed-out timer
(BleedOutDuration, Clot 5 s); the zed dies when it runs out (no ragdoll push;
the kill counts for you); headless zeds walk with HeadlessWalkAnims at 80%
speed; Claw3 attacks swap to Claw2/Claw1 (no effect on the Clot, whose attacks
are grapples). Before, losing the head always killed. Plan in DESIGN.md.
**Why:** You remembered headless zeds survive a while (stopping Bloats from
exploding); KFMonster.RemoveHead and Tick confirm it.
**Tested how:** 3 unit tests with KF's numbers (9mm headshot leaves 31.25 of
130 and starts a 5 s bleed-out; a hurt Clot dies from the head explosion; the
knife needs two headshots). Scripted 9mm headshot on KF-WestLondon with logs
and a screenshot. clippy clean, 44 tests pass.
**Result:** Head off at the first shot (hit 29.8 + explosion 68.5), 31.7
health left; the headless Clot kept walking (headless walk, hand on the
stump), grabbed the player 3 times, and bled out 4.98 s after the shot, then
fell as a ragdoll with no push. Screenshot confirms the headless walk.
**Still broken / not tested:** HitF flinch on decapitation not yet (next
step). KF gives bleed-out deaths a random ragdoll spin; ours has none. Aim
penalty and sounds not done. Not play-tested by you.
**Next:** HitF on the upper body.

## 2026-10-03 Decapitation flinch (HitF on the upper body)

**Changed:** `skinned.rs`: `pose_layered`, a second animation layer that
replaces the base animation from a given bone down (UE2 AnimBlendParams on
channel 1, alpha 1). `zed.rs`: when the head comes off, HitF plays once on
the upper body from SpineBone1 (CHR_Spine2) while the legs keep walking, as
KFMonster.RemoveHead / DoAnimAction do. Logs `zed_flinch`, `zed_flinch_done`.
**Why:** Last step of the decapitation plan.
**Tested how:** Scripted 9mm headshot on KF-WestLondon, screenshots 0-0.6 s
after the shot, log timing. clippy clean, 44 tests pass.
**Result:** The flinch starts with the decapitation and ends after 1.35 s
(41 frames); screenshot shows arms thrown out while the legs stride, body
joined at the spine.
**Still broken / not tested:** KF blends the layer in over 0.1 s; ours
switches instantly (a small pop). Not play-tested by you.
**Next:** Your play test.

## 2026-10-03 Hit reactions: body-shot flinches, stun, knock-down

**Changed:** `zed.rs`: `Zed::take_hit` follows KFMonster.PlayHit /
PlayTakeHit / PlayDirectionalHit: hits of 5+ damage play HitReactionF/B/L/R
by direction (at most every 0.5 s) on the upper body from CHR_Spine2, so the
legs keep walking; a frontal hit of >= half the default health (65), or a close
melee hit (within 40) over 13, plays a random HitAnims stun (HitF/2/3) and
stops attacks for 1 s; a hit over Health / 1.5 (86.7) plays the full-body
KnockDown and the zed stands still until it ends. The upper-body layer is now
general (replaces the HitF-only flinch). `combat.rs`: passes hit point,
attacker and melee to the reaction; the head explosion is its own damage
event first, as in KF, so a frontal decapitation stuns.
**Why:** Step 1 of the animation sweep.
**Tested how:** 2 unit tests (directions, stun, knock-down, under-5, close
melee, cooldown). Scripted runs on KF-WestLondon: body shots and a headshot,
with logs and a screenshot. clippy clean, 46 tests pass.
**Result:** Body shot: `reaction=Front sequence=HitReactionF`, screenshot
shows the upper body thrown back while the legs stride. Decapitation:
`reaction=Stun sequence=HitF3 stunned=1.0`; the Clot started its next attack
only after the stun.
**Still broken / not tested:** Correction to my sweep report: the knife does
not stun Clots in practice (the close-melee rule needs the attacker within 40
units, but the two collision cylinders keep centres 46 apart). Knock-down is
not reachable with the 9mm or knife (max ~38 per hit; the head explosion
~69); tested only by unit test. KF also plays the flinch alongside a
knock-down; we play only the knock-down. Layers switch without KF's 0.1 s
blend. Not play-tested by you.
**Next:** Grab fixes.

## 2026-10-04 Clot grab: walks while grabbing, pins the player

**Changed:** `zed.rs`: attacks are picked at random (Rand(3)); the Clot's
three grapples play on the upper body from FireRootBone (CHR_Spine1) while it
keeps walking toward you (ZombieClot.DoAnimAction / Tick); the layer is shared
with hit reactions, so a flinch interrupts a grab. A landed grab pins the
player for GrappleDuration (1.5 s) unless the Clot is headless; the grab
animation stops if you get more than MeleeRange + both radii (66) away; no
melee damage while stunned or in the 2 s after losing the head (KFMonster
DECAP); grab damage varies +-5% (ClawDamageTarget). Other attacks stay full
body. `combat.rs`: `PlayerPinned` resource, released when it runs out, when
the grabbing Clot dies or loses its head, or when the player dies. `walk.rs`:
while pinned, no input, no jump, zero velocity on the ground
(KFPawn.ModifyVelocity); `held=` in the walk log.
**Why:** Step 2 of the animation sweep.
**Tested how:** Scripted runs on KF-WestLondon: walking into a Clot; then
shooting it while held. Logs and a screenshot. clippy clean, 46 tests pass.
**Result:** Grabs land with varied damage (6.22, 6.07) and pin the player
(`held=true`, speed 0 with forward held); successive grabs keep the player
held. Shooting the Clot: flinches interrupted every grab attempt, the pin ran
out after 1.5 s (`reason=time_up`) and the player walked on. Screenshot shows
the Clot lunging with arms out, mouth open.
**Still broken / not tested:** Release on the grabber's death or
decapitation is not tested in a run (the pin had already expired). The grab
being broken by walking away is not tested (no scripted backward walk). Not
play-tested by you.
**Next:** Headless Clot claw attacks.

## 2026-10-04 Headless Clot: claw attacks at double damage and reach

**Changed:** `zed.rs`: when a Clot loses its head (ZombieClot.RemoveHead) its
attacks become Claw, Claw, Claw2 (full body, random), MeleeDamage and
MeleeRange double (6 -> 12, 20 -> 40); the doubled range also applies to the
attack start, the hit check and the close-melee stun rule. Any grab is
released (from the previous step).
**Why:** Step 3 of the animation sweep.
**Tested how:** Unit test (values double on decapitation). Scripted headshot
on KF-WestLondon with logs and a screenshot. clippy clean, 47 tests pass.
**Result:** After the 1 s stun the headless Clot attacked with `Claw` from
farther away; the hit landed 2.5 s after decapitation (after the 2 s daze)
for 11.9 damage, no pin. Screenshot shows the headless swipe with both arms
out.
**Still broken / not tested:** In KF the Clot keeps sliding toward you during
a full-body claw (ZombieClot.Tick accelerates while attacking); ours stands
still for full-body attacks. Not play-tested by you.
**Next:** Falling and turning animations.

## 2026-10-04 Falling, landing and standing/turning animations

**Changed:** `zed.rs`: falling zeds play InAir (AirAnims), then Landed
(LandAnims) once on landing, standing still until it ends; a chasing zed that
is not actually moving (e.g. pressed against the player) plays its idle
instead of walking on the spot, or TurnLeft / TurnRight while turning. New
scripted action `zed_drop` (spawn 200 units up). New log `zed_anim` on every
animation change.
**Why:** Step 4 of the animation sweep.
**Tested how:** Scripted drop and approach on KF-WestLondon with logs and a
screenshot. clippy clean, 47 tests pass.
**Result:** InAir for the 0.63 s fall (screenshot: arms up, knee raised),
Landed for 0.23 s, then ClotWalk; on reaching the player, Idle_LargeZombie
instead of walking in place.
**Still broken / not tested:** The standing / turning choice is made by
native engine code in KF, not the scripts; mine is an approximation
(standing below 10 units/s, turning above 2000 rotation units/s). Turn
animations not seen in a test (needs the player circling a blocked zed).
KF keeps moving during Landed; ours stands for its 0.23 s. Not play-tested
by you.
**Next:** Your play test of all four steps.


## 2026-10-04 Gorefast, step 1: loaded and spawnable

**Changed:** `zed.rs`: the Gorefast (`KFChar.ZombieGorefast_STANDARD`) is
loaded next to the Clot; G spawns one in front of you (scripted action
`gorefast`, `--gorefast` at start). Zed types are a `ZedKind` (Clot,
Gorefast) instead of name checks; spawning picks the class by kind.
`main.rs`: `--gorefast`. Plan in `docs/DESIGN.md`, "Gorefast".
**Why:** You picked "more zed types", starting with the Gorefast.
**Tested how:** Load log; two scripted runs on KF-WestLondon (spawn and
approach; let it hit, then shoot it to death) with screenshots. clippy clean,
tests pass.
**Result:** All its values come from the class data (Health 250, speed 120,
MeleeDamage 15, MeleeRange 30, DrawScale 1.2, head radius 10.5). Its ragdoll
`GoreFast_Trip` matches the mesh (anchor gap 0.00). It walked 300 units in
with `GoreWalk`, attacked with GoreAttack1/2, hit for 15.5 and 15.2. A 9mm
headshot took the head off (head explosion 99.6, left at 95 health, bleeding
out); it died at 0 and ragdolled, every joint within 0.1 units. Screenshots:
textured, blade arm, no left forearm; mid-swing; headless corpse on the road.
**Still broken / not tested:** No running yet (step 2). Its skin is the
base texture only (KF draws it through a Combiner material). Not play-tested
by you.
**Next:** Step 2, running and charge attacks.

## 2026-10-04 Gorefast, step 2: running and charge attacks

**Changed:** `zed.rs`: ZombieGoreFast's RunningState. If the Gorefast has its
head, is not attacking and is within 700 units of you, it runs (GroundSpeed
x 1.875 = 225, animation ZombieRun). Every 0.5-1.0 s it checks whether you
are still within 700, and walks again if not. While running, each attack
has a 20% chance (ChargeChance at Normal) to be a moving attack: GoreAttack1/2
on the upper body while it keeps running, for GoreAttack1's length.
Otherwise it makes a normal full-body attack and stops running. Losing its
head ends the running for good. The Clot's "grab breaks when you walk away"
rule now applies only to Clots. New test action `gorefast_far` (spawn 900
away). New log `gorefast_run`; the `zed` line now has `speed_unreal` and
`running`.
**Why:** Step 2 of the Gorefast plan.
**Tested how:** 2 new unit tests (start/stop rules, moving attack timeout,
head loss); scripted run on KF-WestLondon with `gorefast_far` and two
screenshots. clippy clean, 28 tests pass.
**Result:** Walked at 120 from 900 away; `gorefast_run running=true
reason=started distance_unreal=698`, then 225 units/s with ZombieRun. Its
first attack happened to be a charge (`GoreAttack2 layered=true
charge=true`), hit for 15.5. Then a full-body attack ended the run
(`reason=full_body_attack`); after that it kept attacking in melee range
without running, as in KF. Screenshot shows it running in.
**Still broken / not tested:** When KF's AI calls RangedAttack is decided by
native code; I assume every frame while chasing. The `target_far` stop is
tested only by unit test (no scripted backing away). In KF a running
Gorefast also keeps sliding forward during a moving attack when pressed
against you; ours is blocked by your cylinder (speed 0, plays GoreIdle on
the legs). Not play-tested by you.
**Next:** Your play test. Then another zed type, or gore steps 2-3.

## 2026-10-04 B key: choose what Z spawns

**Changed:** `zed.rs`: new `ZSpawn` resource; B cycles it through the loaded
zed types (Clot -> Gorefast -> Clot), Z spawns the chosen type. G still
spawns a Gorefast directly. Scripted action `cycle_zed`; log
`z_spawn_selected`. `combat.rs`: the HUD shows `Z: CLOT` / `Z: GOREFAST`.
README controls updated.
**Why:** You asked for a hotkey to rotate what Z spawns.
**Tested how:** Scripted run pressing B three times, log and screenshot.
clippy clean, 28 tests pass.
**Result:** Log: Gorefast, Clot, Gorefast in turn; the HUD reads
`Z: GOREFAST` after the third press.
**Still broken / not tested:** Pressing Z itself was not tested by script
(scripted input has no Z key; the `zed`/`gorefast` actions bypass it).
Play-tested by you: works. You noted B will likely be the buy menu later, so
this key will need to move then. Moved to N the same day, at your request.
**Next:** Your play test.

## 2026-10-04 Gore step A: neck stump and brain chunks on gun decapitation

**Changed:** `ue-assets/skeletal.rs`: skeletal meshes now read their attach
tags (UE2 TagAliases / TagNames / TagCoords, e.g. `neck` -> CHR_Neck with an
offset frame), found by scanning the mesh data; `kfpkg meshtags FILE MESH`
prints them. `skinned.rs`: `tag_frame` gives a tag's frame for a pose. New
`gore.rs`: loads the stump (`ROEffects.SeveredHeadAttachment`, mesh
`Gear_anm.NeckAttach_Gore`) and the brain chunk meshes (`KFGibBrain`,
`KFGibBrainb`: `KillingFloorStatics.Gib1/Gib2`). On a gun decapitation
(KFMonster.DecapFX): the zed gets a stump placed every frame at its `neck`
tag (also on the ragdoll), and three chunks fly up from the head bone at
250-312 units/s, jittered as KF does. Chunks fall at 950, bounce keeping
40% of their speed (KFGib DampenFactor), stop below 20, spin at random rates
and vanish after 6 s. `zed.rs` / `combat.rs`: decapitation sets a pending
effect, spawned by the animation system. Logs: `gore_stump_loaded`,
`gore_gib_loaded`, `decap_fx`, `gib_spawned`, `gib_bounce`, `gib_rest`,
`gib_removed`. Plan and findings in `docs/DESIGN.md`, "Gore steps A-C".
**Why:** Gore step A, agreed with you.
**Tested how:** `kfpkg skelmeshes` (all 398 skeletal meshes still read);
scripted headshots on a Gorefast and a Clot (then killed into a ragdoll) on
KF-WestLondon, with logs and screenshots. clippy clean, tests pass.
**Result:** Tag tables: Clot 16 tags, Gorefast 15. Decapitation logs
`decap_fx ... stump=true brain_chunks=3`; chunks left at 264-309 units/s,
first bounce after ~0.8 s, rested after 4 bounces at 1.4 s on the road, removed at 6 s.
Screenshots: the stump (ragged meat) sits on the neck of the walking
headless Gorefast and Clot and stays on the ragdoll; chunks visible in the air.
**Still broken / not tested:** The stump's size is an assumption (drawn at
its own scale 1, not the zed's 1.1/1.2); it looks right in screenshots but
is not compared with the real game. Chunk collision is a sphere (KF: a small
cylinder). Chunks do not bounce off zeds or you (KF's don't either, as far
as the defaults show). No blood trail, neck jet or splash yet (particles,
step C); no sounds. Knife decapitation still only removes the head (step B).
Play-tested by you: "Head shots work great!" 
**Next:** Your play test, then step B (severed limbs, knife head).

## 2026-10-04 Gore step B: severed limbs, knife decapitation, stump scale fix

**Changed:** `zed.rs`: combat now records every damage event (`GoreHit`);
`apply_gore` turns them into KF's DoDamageFX / ProcessHitFX effects. On the
killing hit, the hit bone (nearest bone segment to the shot's line, named
by its mesh tag; KF does this natively) is mapped as KF does (lleg/lfoot ->
lthigh, rarm/righthand/rshoulder -> rfarm, ...). Thighs and forearms come
off with chance |health after - damage| / 130. A severed limb hides its
bone (lthigh = CHR_LCalf: knee down; rfarm = CHR_RArmForeArm: elbow down),
puts a stump on `lleg`/`rleg`/`larm`/`rarm` (SeveredLeg/ArmAttachment,
scaled 0.8 Clot / 0.9 Gorefast), and throws the zed's own piece
(SeveredLegClot etc.) plus 2-3 brain chunks. Knife decapitation: neck stump
and the zed's head flies off (SeveredHeadClot / SeveredHeadGorefast), no
chunks. The Gorefast's missing left forearm never comes off. Neck stump now
uses SeveredHeadAttachScale (Clot 0.8, was 1.0). `gore.rs`: pieces share
the chunks' movement (bounce 35%, life 8 s, lie flat on landing);
`skinned.rs`: several hidden bones at once; `coords.rs`: rotator from axes.
Gore random numbers are seeded with the clock (identical runs gave the same
roll every time). Logs: `gore_piece_loaded`, `gore_hit_bone`, `limb_roll`,
`limb_severed`, `piece_spawned`, `piece_bounce`, `piece_rest`,
`piece_removed`.
**Why:** Gore step B, agreed with you.
**Tested how:** Unit tests (29 pass, incl. a rotator round trip); scripted
kills on KF-WestLondon at several aims, logs and a screenshot.
**Result:** All 6 piece classes and 3 stumps load. Hit bones come out as
expected: thigh -> `lleg` -> lthigh, collar -> `lshoulder` -> lfarm, upper
arm -> `rarm` -> rfarm, spine -> nothing. One leg severed in 6 kills (chances
0.25-0.48): the leg and 3 chunks came to rest on the road after 0.3-0.65 s.
Screenshot: corpse with the lower leg gone and a stump at the knee.
**Still broken / not tested:** Arms severing and the knife's flying head
not yet seen in a run. Not checked against the real game: piece
orientation in flight, stump placement on limbs. The hit-bone choice is an
approximation of native code. Corpses cannot be shot apart further (our
bullets pass through corpses). No blood particles or sounds (step C).
Play-tested by you with `--always-sever`: "It works! Nice!" 
**Next:** Your play test.

## 2026-10-04 Test switch --always-sever

**Changed:** `main.rs`, `zed.rs`: `--always-sever` makes the limb chance 1.0
(a killing hit on a thigh, shin, shoulder or arm always takes it off).
Without it, KF's chance rule applies as before.
**Why:** You asked for 100% severing, temporarily, to test step B.
**Tested how:** Scripted leg kill with the switch.
**Result:** `limb_roll ... chance=1.00 ... severed=true`, then `limb_severed`.
**Still broken / not tested:** Nothing new. Remove the switch later if it is
not wanted.
**Next:** Your play test of step B.

## 2026-10-04 Gore step C1: reading KF's particle effects

**Changed:** New `ue-assets/src/emitter.rs`: reads an Emitter actor class
(e.g. `KFMod.DismembermentJetHead`) into its sub-emitters, every value
resolved (the emitter's own property, else the engine class default from
Engine.ParticleEmitter / SpriteEmitter / MeshEmitter); ranges, vectors and
the ColorScale / SizeScale / VelocityScale arrays decoded. Textures and
meshes are returned separately (`EmitterAssets`). `kfpkg emitter
Package.Class` prints one. `ObjectHandle` now prints as its path. Plan in
`docs/DESIGN.md`, "Gore step C: particle effects".
**Why:** Step C, the faithful route: KF's own effects.
**Tested how:** `kfpkg emitter` on all 11 gore effect classes; compared
DismembermentJetHead's values with the raw property dump.
**Result:** All read: DismembermentJetHead 14 emitters (9 sprite, 5 mesh),
JetDecapitate 8, JetLimb 5, BrainSplash 2, ROBloodSpurt 1, BloodTrail 1,
KFGibJet 1, ROBloodPuff* 1-3. Values match the dump (e.g. SpriteEmitter104:
75 particles at 90/s, life 0.45-0.85 s, size 5 x 1.25, 8x4 texture frames,
spawned on emitter 5's particles). Findings: most gore sprites use draw style
2 (Modulated: they darken what is behind them); UseColorScale is off on all of
them, so their ColorScale curves are unused; the head jet also throws 13 meat
meshes (eyeball, chunk, three brain pieces) upward at 300-1000 units/s.
**Still broken / not tested:** Nothing is drawn yet (C2).
**Next:** C2, the particle simulation.

## 2026-10-04 Gore step C2 (+ mesh emitters from C3): KF's particle effects drawn

**Changed:** New `src/particles.rs`: loads 11 KF effects at startup
(DismembermentJetHead / JetDecapitate / JetLimb, BrainSplash, ROBloodSpurt,
BloodTrail, KFGibJet, ROBloodPuff*) and simulates them: spawning
(InitialParticlesPerSecond until MaxParticles, then ParticlesPerSecond if
dead particles respawn), start location (box / sphere,
AddLocationFromOtherEmitter), velocity, acceleration, velocity loss,
MaxAbsVelocity, collision bounces with DampingFactor (mesh chunks), lifetime,
fade in / out, size scale over life, colour scale (when UseColorScale),
spin, texture subdivisions, camera-facing and UpAndNormal (streak) sprites,
relative vs world coordinates, draw styles (Modulated = 2x multiply,
Translucent = add, AlphaBlend). Mesh emitters draw their static meshes.
`zed.rs`: gun decapitation starts DismembermentJetHead on the `neck` tag
plus BrainSplash at the head; knife decapitation DismembermentJetDecapitate;
a severed limb DismembermentJetLimb on the stump tag. Attached effects follow
their tag every frame (also on ragdolls). Logs: `effect_loaded`,
`effect_spawned`, `effect_status` (alive/spawned per emitter every 0.5 s),
`effect_removed`.
**Why:** Step C2, the faithful route.
**Tested how:** Scripted gun decapitation and a forced limb sever on
KF-WestLondon, logs and screenshots. clippy clean, tests pass.
**Result:** All 11 effects load with textures and meshes. First attempt drew
black squares: KF's blood textures have a mid-grey (127) background because
UE2's Modulated style is 2 x source x destination; plain multiply turned the
grey dark and dozens of stacked particles black. Fixed by doubling the
texture colour at load. Now: a dark blood plume rises from the neck, 13 meat
pieces (eyeball, chunk, brains) fly up and bounce, blood mist follows them;
BrainSplash ends after 0.77 s; the limb jet keeps pulsing at the stump.
**Still broken / not tested:** Faint square edges remain where many
modulated particles overlap (the textures' compression noise around 121-125
grey darkens slightly; KF would do the same, not compared). Rules marked
"assumed" in `particles.rs` (spawn rates, world axes for independent
emitters, sprite width = size, mesh spin axes, curve ends) are from what the
settings mean, not from engine code. No mipmaps on particle textures
(shimmer at distance). BlendBetweenSubdivisions not blended. Trails on
flying pieces and bullet-hit blood puffs not wired up (C4). Frame rate with
many effects not measured. Not play-tested by you.
**Next:** Your play test; then C4 (trails, bullet-hit puffs).

## 2026-10-04 Particle fixes: slab allocator errors, black squares, invisible sprites

**Changed:** `particles.rs`: (1) every particle mesh has all its attributes
(position, normal, UV, colour) from the start and is never empty (one
zero-size triangle when no particles are alive). (2) Modulated sprites use
their own small material and shader (`ModulateMaterial`) that outputs the
texture colour without tone mapping, with back-face culling off.
**Why:** You reported a flood of `bevy_render::slab_allocator: Use-after-free`
errors and black squares. My test runs had been discarding the console
output, so I had not seen the errors.
**Tested how:** Same scripted decapitation run with the console captured;
screenshots. A diagnostic run with the shader forced to pure red showed
which sprites were drawn.
**Result:** Errors: 4262 per run before, 0 after (mesh layout changing and
empty meshes were the cause). Squares: the standard material tone-maps its
output (the camera is not HDR), so "white = no change" reached the blend as
grey and darkened everything behind. The custom shader fixed that, but then
only the streak sprites drew: the default back-face culling dropped every
camera-facing sprite (the red test showed only the streak). With culling
off: blood droplets burst from the neck, mist follows the flying chunks, no
squares, no darkening.
**Still broken / not tested:** Additive and alpha-blended particles still use
the standard (tone-mapped) material; none of the gore effects drawn so far
use them. Not play-tested by you.
**Next:** Your play test; then C4 (trails, bullet-hit puffs).

## 2026-10-04 Gore step C4: hit puffs and blood trails

**Changed:** `zed.rs`: every damage event spawns the damage type's
PawnDamageEmitter, ROBloodPuff for the 9mm and knife (KFMonster.OldPlayHit:
at the hit point pushed one CollisionRadius away from the attacker, X axis
toward the attacker). `gore.rs`: brain chunks carry a BloodTrail (KFGib
TrailClass, LifeSpan 1.8 s), severed pieces an ROBloodSpurt
(SeveredAppendage BleedingEmitterClass, as long as the piece); trails follow
their piece (PHYS_Trailer, position only); a piece's trail is destroyed when
it lands or is removed, a chunk's trail is killed when the chunk goes.
`particles.rs`: Emitter.Kill() (no new particles, removed when the last
dies) and a LifeSpan set by the spawner.
**Why:** Step C4. Research: KF's weapon damage types have no GetHitEffects
override and pawns have no BloodEffect, so ROBloodPuff (DamageThreshold 0)
is the whole hit effect for the 9mm and knife.
**Tested how:** Scripted body shots and a forced limb kill on KF-WestLondon,
console captured, logs and a screenshot. clippy clean, tests pass.
**Result:** 0 console errors. 5 hits gave 5 ROBloodPuffs (1 particle,
gone after 0.62 s); 3 BloodTrails and 1 ROBloodSpurt on the pieces; a small
spray shows behind the Clot on a body hit.
**Still broken / not tested:** The puff is small and subtle; not compared
with the game. Not done: chunk bounce effects (KFHumanGibGroup BloodHitClass
= KFBloodPuff, an old-style xEmitter plus floor splats) and all blood decals
(ProjectileBloodSplat on walls, drips, streaks): decals are a separate
engine feature. Not play-tested by you.
**Next:** Your play test; then blood decals, or something else.

## 2026-10-04 Gore step D: blood decals

**Changed:** New `src/decals.rs`: KF's ProjectedDecal Projectors rebuilt as
CPU decal meshes: level triangles inside the projector box (a grid over the
collision geometry, kept when colliders are built) are clipped to the box,
textured by their position in it, lifted 0.4 units, drawn with the 2x
modulate material, faded in over FadeInTime and out over the last second,
removed after LifeSpan - 1 (as ProjectedDecal.PostBeginPlay). Classes read
from the game: ROBloodSplatter, ROSmallBloodDrops, KFBloodSplatterDecal,
KFBloodStreakDecal (textures, DrawScale, PushBack, MaxTraceDistance,
RandomOrient, FadeInTime, LifeSpan). Wired up: (1) hits splat the surface
within 350 units along the shot (ProjectileBloodSplat; a hit within 0.2 s of
the last only 20% of the time); (2) severed pieces drip where they bounce
fast and where they stop; (3) a stopped brain chunk splats the floor 20% of
the time (KFBloodPuff); (4) ragdoll parts hitting surfaces leave streaks
(KFMonster.KImpact rules, via avian collision events). `ue-assets`:
`ClassDefaults::get_array_objects`. `particles.rs`: the modulate material
and texture decoder are shared. Logs: `decal_surfaces`,
`decal_class_loaded`, `decal_spawned`, `decal_no_surface`, `decal_removed`.
**Why:** Gore step D (finish the gore before pathfinding, as you asked).
**Tested how:** Scripted kills on KF-WestLondon (body shots, a forced limb
sever, headshots), console captured, logs and screenshots. clippy clean,
tests pass.
**Result:** 219,744 level triangles indexed. All four classes load with
their textures. Seen in runs: wall/floor splats behind shots (64 units),
drips under a severed leg, a 179-unit floor splat under a stopped chunk, 2-3
streaks per corpse; 0 console errors. Screenshots: dark-red splats on the
road, no square edges.
**Still broken / not tested:** Approximations (labelled in the code): the
projector is an orthographic box (KF's FOV of 0-6 degrees ignored); size =
texture size x |DrawScale| (KF's negative scales read as mirrored); only
blocking geometry gets decals; the 1 s end fade is assumed; ragdoll impacts
use avian's collision-start events, not Karma's. The old-style KFBloodPuff
particle (xEmitter) on chunk landing is not drawn. Not play-tested by you.
**Next:** Your play test; then pathfinding.

## 2026-10-04 Pathfinding, first part: KF's navigation network and hunting routes

**Changed:** New `ue-assets/src/nav.rs`: reads a map's navigation network
(every ReachSpec and the NavigationPoints it joins: PathNode, JumpSpot,
ZombiePathNode, PlayerStart, ...); `kfpkg nav MAP|all`. New `src/nav.rs`:
the network in Bevy space (links a walking zed may use: walk / forced /
door flags, at least 24 x 44), a walk test standing in for native
ActorReachable (our movement code walks the zed's cylinder there in 32-unit
steps: slide, step up, fall up to KFMonster MaxFallSpeed's height), a
startup check that walks every link and drops the ones our zeds cannot
walk, Dijkstra route search, and KF's hunting rules
(KFMonsterController.ZombieHunt / FindBestPathToward): walk straight at the
player when walkable, else head for the next point on the shortest route,
RouteCache[1] shortcut, +200 on a point another zed blocks, BlockedWay
(+10000) after more than 3 identical results (60%). Our additions: a zed
right above or below its target ends the move after 1 s (it would circle),
and gives up on a link it failed twice (no jumping yet). `zed.rs`: zeds
steer along the route (still face you when in reach or attacking); facing
wraps at one turn. `map.rs` loads the network. `--zed-at X,Y,Z` test
switch. Logs: `nav_loaded`, `nav_link_check`, `nav_link_unwalkable`,
`zed_path`, `zed_path_none`, `zed_route`, `zed_route_blocked`,
`zed_route_failed`, `zed_link_given_up`.
**Why:** Zeds walked in a straight line and got stuck on walls.
**Tested how:** `kfpkg nav all` (all 40 maps); KF-WestLondon runs with Clots
spawned at six navigation points, console captured; regression runs of the
earlier Clot and Gorefast scenarios. clippy clean, tests pass.
**Result:** Every map's network reads (KF-WestLondon: 315 points, 1506
ReachSpecs, 1248 usable). Our walk test agrees with 1183 of them; the 65
others: 20 jump pads (correctly unusable), 10 where our collision has a
hole, 35 where something in our collision blocks a path Unreal had clear.
Clots spawned on the street 1000-1300 units away arrived in 12-20 s, some
using routes. Three spawns on other levels did not arrive: the upper level
connects to the street only by jumping down over a wall (JumpSpots), and two
spots sit where our collision differs from Unreal's.
**Still broken / not tested:** Zeds cannot jump (KFMonster JumpZ 320), so
jump links are unusable and some areas are cut off. Collision differences
(holes, extra blockers) still to investigate; they would affect the player
too. The first plan right after spawning fails while you are still landing
(harmless). Zeds sometimes switch between "straight at you" and "follow the
route" as the walk test flickers. Doors, the FindRandomDest wander and
LastSeenPos hunting are not done. Not play-tested by you.
**Next:** Your play test; then zed jumping (and horizontal motion while
falling), then the collision differences.

## 2026-10-04 Zed jumping and falling momentum

**Changed:** `zed.rs`: a zed blocked by the level (not by a pawn) jumps the
obstacle when a jump clears it: JumpZ from the class (KFMonster 320, so
about 54 units of lift), carried forward at its ground speed, at most one
try a second (native PickWallAdjust; rule from memory). Falling now keeps
horizontal momentum (walking off a ledge, jumps; KFMonster AirControl 0.05
not applied) and stops rising at ceilings. Landing plays only after a real
fall (impact faster than half JumpZ, assumed); small drops go straight back
to walking. `walk.rs`: `Mover::jump_over`. `nav.rs`: the walk test allows
such jumps; drops are allowed down to KFMonster MaxFallSpeed's height
(2500 -> about 3289 units). Log `zed_jump`, `nav_probe_jumped`.
**Why:** You asked for zed jumping before the collision work.
**Tested how:** Runs on KF-WestLondon: the link check, a zed spawned on the
upper level, a zed behind a 37-unit rise, the street spawns and the
Gorefast scenario, console captured. Tests pass.
**Result:** 3 more links walkable with jumps (1186 of 1248). Tiny falls no
longer play the landing animation (0 landing animations in 3 street runs,
arrival 14.5 s instead of 19.6 s for one). The upper level still does not
connect: its JumpSpots stand on a wall (BlockingVolume35) about 99 units
above the floor, more than a 54-unit jump; in KF too CanMakeJump would
likely refuse it, and zeds do not normally spawn up there.
**Still broken / not tested:** No zed was seen jumping in a run: the 37-unit
rise was climbed by normal stepping. KF's JumpSpot-specific jumps
(SuggestMovePreparation / EAdjustJump aimed jumps, KF's JumpTime) are not
done. Not play-tested by you.
**Next:** Collision differences (the ambulance snag; 32 blocked links, 10
holes).

## 2026-10-04 Collision: class-blocking volumes, layers, stuck zeds

**Changed:** `ue-assets/level.rs`: blocking volumes keep bClassBlocker /
BlockedClasses (resolved in the right package) and bBlockZeroExtentTraces.
`collision.rs`: collision layers. Level geometry blocks everything; plain
blocking volumes block movement, ragdolls and flying gore but not bullets
(BlockingVolume bBlockZeroExtentTraces false, bBlockKarma true); class
blockers block only the listed classes (KFZombieZoneVolume: "blocks ONLY
humans", so zeds pass); new filters `player_filter`, `zed_filter`,
`body_filter`; `world_filter` is now for zero-width traces (bullets, blood
traces, particles). `walk.rs`: `Mover` takes its filter. `ragdoll.rs`,
`gore.rs`: bodies collide with blocking volumes. `nav.rs`: walk test in
16-unit steps (closer to real movement) with a bigger step budget, slides
off edges when landing; zeds that make under a quarter of their expected
progress in a second count as stuck (a straight walk at the player is then
off for 3 s; a route move ends, feeding the stuck rule). Logs
`class_blocker`, `zed_stuck`; `collision_spawned` lists volume layers.
**Why:** You saw a zed catch on the ambulance; the link check showed most
blocked paths ran into KFZombieZoneVolumes.
**Tested how:** Link check on KF-WestLondon and KF-Farm; KF-Offices and
KF-BioticsLab loads; three runs with a zed routing around the ambulance;
dropping the player at suspected holes; regression runs. Console captured,
clippy clean, tests pass.
**Result:** KF-WestLondon: 15 KFZombieZoneVolumes now block only the player;
5 list PlayerController and block nothing that walks (they stop spectators).
Walkable links: 1229 of 1248 (was 1183); the rest are jump pads and
JumpSpot jumps. KF-Farm: 5382 of 5400. Zeds placed behind the ambulance
arrived in 13.5-14.6 s, routing round its end. The "holes" were the walk
test landing on kerb edges, not holes: the player stands there fine.
**Still broken / not tested:** I could not reproduce your ambulance catch
exactly; the stuck check is untested in a real snag. A zed spawned inside a
prop stays stuck (KF would refuse such a spawn). Jump pads and JumpSpot
jumps are not done. Bullets passing through volumes is not checked in a run.
The link check takes 1.7 s at load on KF-Farm. Not play-tested by you.
**Next:** Your play test.

## 2026-10-04 All ten specimens loaded (step S1)

**Changed:** `zed.rs`: all of KF's specimens load (Clot, Gorefast, Crawler,
Stalker, Bloat, Siren, Husk, Scrake, Fleshpound, Patriarch; the
`KFChar.Zombie*_STANDARD` classes) with the shared rules (stats, walk,
melee, flinches, decapitation, ragdolls, severed parts, gore effects,
routes). Spawning is class-driven: N cycles all ten for Z; `--spawn NAME`
at start (works with `--zed-at`); test action `spawn_<name>`. Attack log
lines show the animation length and rate. `ue-assets/material.rs`: a
Combiner whose Material1 (or a Shader whose Diffuse) leads to no texture
falls back to Material2 / SelfIllumination; new `Blend::Additive` for
Shader OB_Translucent / OB_Brighten and FinalBlend FB_Translucent /
FB_Brighten, drawn additively on skinned models only (the map keeps its
old handling). `skinned.rs`: log `skinned_material` (material chain per
part).
**Why:** You asked for all the monsters.
**Tested how:** Load log; each new specimen spawned in a scripted run with
the console captured; screenshots. clippy clean, tests pass.
**Result:** All ten load in about 1 s, each with its ragdoll from
KF_Characters_Trip.ka. Each of the eight new ones walks up and attacks;
hits with their KF MeleeDamage (+-5%): Crawler 6, Stalker 9, Bloat 14,
Siren 13, Husk 15, Scrake 20, Fleshpound 35. Screenshots: all textured. Two
fixes on the way: the Stalker was plain white (her skin is a cloak shader
whose first branch is an environment map) and the Fleshpound's chest
device was a black square (an additive "Brighten" shader drawn solid).
(An early survey seemed to show no hits: my runs quit at the screenshot
frame, 2 s in. Not a bug.)
**Still broken / not tested:** No specimen has its special behaviour yet
(pounce, cloak, rage, bile, scream, fireball, boss attacks); the Patriarch
has no usable melee yet (his attacks are all special). The Husk's ragdoll
is 5.6 units off its mesh at the left calf (all others 0.00). Health is
KF's base value (no difficulty or player-count scaling). The Combiner /
Shader fallback can also change some map textures (from grey to a
texture); not checked. Not play-tested by you.
**Next:** Specials, smallest first: Crawler pounce.

## 2026-10-04 Crawler: pounce

**Changed:** `zed.rs`: CrawlerController.FireWeaponAt / ZombieCrawler.DoPounce:
when out of reach, roughly facing the target (KF compares facing with the
un-normalised vector to it, so almost any forward angle passes; copied as
written), 4.5 - FRand() x 3 s after the last pounce, and IsInPounceDist
(within MeleeRange x 5 = 250 units, landing at the target's height), the
Crawler leaps at PounceSpeed (330) with JumpZ (350) upward, playing
ZombieSpring; touching the player mid-leap hurts once (ZombieCrawler.Bump,
MeleeDamage +-5%). FlipOver returns false (no knock-down); flinches play
from NeckBone. Falling zeds are now blocked by pawns too (sideways).
`bStunImmune` is declared in KFMonster but used nowhere, so stuns stay.
Logs `crawler_pounce`, `crawler_pounce_hit`.
**Why:** Specials, step 1 (Crawler).
**Tested how:** Scripted runs on KF-WestLondon, console captured.
**Result:** Pounce from 245 units after 2.3 s; airborne 0.6 s; bump hit
5.8; lands against the player (47 units, was 7 before pawns blocked falls);
then claws as before.
**Still broken / not tested:** Mid-air claw attacks (MeleeAirAnims
InAir_Attack1/2 on the upper body) not done. Not play-tested by you.
**Next:** Stalker.

## 2026-10-04 Stalker: cloak

**Changed:** `zed.rs`: ZombieStalker's cloak. She starts cloaked
(PostBeginPlay); a melee attack uncloaks her (SetAnimAction); every 0.5 s
she cloaks again once 1.2 s have passed since the last uncloak (Tick);
losing her head or dying shows her normal skin for good (RemoveHead,
PlayDying). New `ZedParts` component (each zed's mesh-part entities) and
`apply_cloaks` system swap the materials. Logs `stalker_cloak`,
`stalker_uncloak`.
**Why:** Specials, step 2 (Stalker).
**Tested how:** Scripted runs, console captured, screenshots.
**Result:** Uncloaks at each attack, recloaks 1.2 s after; cloaked she is a
faint shimmer, barely visible; uncloaked her textured skin.
**Still broken / not tested:** The cloak look is an approximation (KF's
`stalker_invisible` is a refraction shader: a rotating glass-reflection
environment map masked by her skin, with oscillating opacity); here a 15%
see-through skin. The 0.25 s decloak flash overlay (KFX.FBDecloakShader) and
the Commando's glow outline (needs perks) are not done. Not play-tested by
you.
**Next:** Scrake.

## 2026-10-04 Scrake: chainsaw loop, charging, rage

**Changed:** `zed.rs`: ZombieScrake. The first swing (SawZombieAttack1/2,
upper body so he keeps walking) enters SawingLoop; in it, while in reach,
he repeats SawImpaleLoop (full body) at MeleeDamage x 0.6; out of reach the
loop ends and damage returns. Entering the loop he charges (GroundSpeed x
AttackChargeRate 2.5, ChargeF walk) with ChargeChance 0.5, or 0.7 under
half health (Normal difficulty). Not attacking, with his head and under half
health he rages (RunningState: GroundSpeed x 3.5, ChargeF); losing his head
ends it. He flinches only at 150+ damage (PlayTakeHit); flinch threshold is
now per class. Test action `hurt_zeds` (100 damage to every zed). Logs
`scrake_sawing`, `scrake_rage`, `zed_hurt_test`.
**Why:** Specials, step 3 (Scrake).
**Tested how:** Scripted runs, console captured.
**Result:** First swing 20.7, then the loop every 0.53 s for 11.4-12.5
(20 x 0.6 +-5%). Hurt to 400 health he raged at 297 units/s (85 x 3.5)
from 1100 units, sawed on arrival, raged again after you respawned.
**Still broken / not tested:** The saw loop sound, exhaust emitter and
difficulty-dependent rage point (0.75 on Suicidal+) not done; Normal only.
Not play-tested by you.
**Next:** Fleshpound.

## 2026-10-04 Fleshpound: rage, half damage, red device

**Changed:** `zed.rs`: ZombieFleshPound. Damage taken within 2 s of the
previous hit adds up; over 360 (RageDamageThreshold), head on, he plays
PoundRage in place (new state `Enraging`), then charges 5-11 s at
GroundSpeed x 2.3 with PoundRun, attacks only with FPRageAttack at x 1.75,
and a landed hit ends the rage. Frustration: 10-15 s chasing without an
attack starts a rage that does not time out. PoundAttack1 x 0.5,
PoundAttack2 x 0.25 per hit. His chest device (material part 1) switches to
KFCharacters.FPRedBloomShader while raging (cloak system generalised).
No flip-over. `combat.rs`: per-zed damage scale (FP 0.5) and every hit feeds
the 2 s damage counter. Logs `fleshpound_rage`.
**Why:** Specials, step 4 (Fleshpound).
**Tested how:** Scripted runs on KF-WestLondon (`hurt_zeds` x4 in 0.5 s;
9mm shots), console captured, screenshot.
**Result:** 400 damage in 0.5 s -> rage start, PoundRage 2.35 s, charge
6.4 s planned, FPRageAttack hit 63 (35 x 1.75), rage ended by the hit;
afterwards PoundAttack3 35.7, PoundAttack1 18.1. 9mm hits 12.8-15.1 (half),
five shots did not trigger a rage (as in KF). Screenshot: red device while
raging. One console ERROR "Failed to send screenshot: sending on a closed
channel" from the final screenshot at shutdown (test harness, not game).
**Still broken / not tested:** Frustration rage not tested (needs you out
of reach for 10+ s). Rage time-out path not seen (the hit came first).
Rage bumping other zeds aside (450 damage) not done. Repeated-hit attacks
land one hit. Not play-tested by you.
**Next:** Bloat.

## 2026-10-04 Bloat: vomit, bile burn, death burst; animation notifies

**Changed:** `ue-assets/skeletal.rs`: animation sequences keep their
notifies (time, AnimNotify_Script name, AnimNotify_Effect class, bone,
offset, rotation); `kfpkg notifies <file> <MeshAnimation>` prints them.
`zed.rs`: ZombieBloat RangedAttack (ZombieBarf within 250, moving with
chance 0.4), notify-timed KFVomitJet on the head and SpawnTwoShots (three
globs), death burst (BileExplosion, SpineBone2 hidden, BileJet's 4 globs;
not after a headless bleed-out), no flinch mid-attack, no flip. Attached
effects can now follow a bone with an offset. New `vomit.rs`: KFBloatVomit
globs (flight with gravity, touch damage, landing blow-up, VomitDecal).
`combat.rs`: KFPawn bile burn (7 ticks of 2-4 every 0.5 s); PlayerDamaged
has a `bile` flag. `decals.rs`: VomitDecal (ProjTexture). `particles.rs`:
KFVomitJet, BileExplosion(Headless) loaded; world-space emitters turn their
start velocity with the effect; PTDU_Up sprites.
**Why:** Specials, step 5 (Bloat).
**Tested how:** Scripted runs on KF-WestLondon with console capture and
screenshots; you play-tested the vomit.
**Result:** Barf at 249 units; jet at 0.42, globs at 0.44 of the animation;
globs landed 0.35 s later in front of the player for 2-5 each; bile ticks
2-4 x 7; a glob touching the player did 3. Kill by body shots: BileExplosion
spawned, 4 globs up, landed 0.92 s later; screenshot shows legs only, meat
and bile. No console errors. You confirmed the stream and the ground spot
are visible.
**Still broken / not tested:** Headless bleed-out (no burst) not run. The
particle velocity change may alter existing blood effects; not rechecked.
Glob look approximate; VomGroundSplash not done; no target leading; vomit
does not hurt other zeds.
**Next:** Siren.

## 2026-10-04 Siren: scream pulses and pull; shared ranged attack

**Changed:** `zed.rs`: the Bloat's vomit became a shared ranged attack
(animation, range, all SpawnTwoShots notify times, effect notifies, moving
chance). ZombieSiren: Siren_Scream within ScreamRadius 700 on the upper body
from SpineBone1 (bites too), 0.65 speed while attacking, six scream pulses
(HurtRadius 8 / 700 with ScreamForce -150000 pull, line of sight), headless
death (50% at once, else within 10 s; no bites), no flip. `walk.rs`:
`PlayerPush` message (Pawn.TakeDamage momentum: upward at least 0.4 x size
on the ground, / Mass 400, AddVelocity). `particles.rs`: ROEffects.SirenScream
loaded; mesh emitters use their material's blend (additive, translucent,
masked; see-through ones unlit) and log `effect_mesh_material`.
**Why:** Specials, step 6 (Siren).
**Tested how:** Scripted run on KF-WestLondon, console captured, screenshots.
**Result:** Scream started at 298 units; pulses at 171, 59, 47 units did 6,
7, 7 (scale 0.78-0.96); each pull added ~294-358 units/s toward her and
118-144 up; the player was dragged ~96 units. 41 damage over a full close
scream. First screenshot showed the scream ball as an opaque dark sphere
filling the view (mesh emitters ignored blending); after the fix it is
additive red swirls. No console errors.
**Still broken / not tested:** View shake and blur not done. Headless death
not run. The blend change affects other mesh emitters (gibs are opaque or
masked, unchanged by the log). Not play-tested by you.
**Next:** Husk.

## 2026-10-04 Husk: fireball, burning

**Changed:** `zed.rs`: the Husk joins the shared ranged attack (ShootBurns,
range 65535, ProjectileFireInterval wait, Barrel bone); every ranged attack
now needs the player in sight; `shoot_fireball` (HuskZombieController
AdjustAim: lead, feet/middle/head); hits do not interrupt his attacks.
The three message writers in think_and_move are one tuple parameter (Bevy's
16-parameter limit). New `fireball.rs`: HuskFireProjectile flight, trail,
explosion (FlameImpact, scorch decal, HurtRadius with exposure, knock-back).
`combat.rs`: `PlayerDamaged.kind` (Plain, Vomit, Fire) replaces `bile`;
KFPawn burning. `decals.rs`: FlameThrowerBurnMark. `particles.rs`:
HuskChargeUp, HuskMuzzle, FlameImpact, FlameThrowerFlameB loaded.
**Why:** Specials, step 7 (Husk).
**Tested how:** Scripted runs on KF-WestLondon, console captured, screenshot.
**Result:** First shot 1.0 s after seeing the player (notify 0.475), aimed at
the feet or middle; fireball hit the player 0.12 s later for 20-25, pushed
126 back and 228 up; burn 12, 6, 3, 1 at 1.5 s, then out. Next shot 5.3-5.8
s later; a shot from 1007 units hit after 0.5 s. Screenshot: impact flames
and smoke, scorch mark, muzzle glow. No console errors.
**Still broken / not tested:** Aim error, zeds dodging or being hurt by the
fireball, view shake, the charge-up beam not drawn. Not play-tested by you.
**Next:** Patriarch.

## 2026-10-04 God mode for test runs

**Changed:** `combat.rs`: `PlayerHealth.god`; when it is on, hits are still
logged (`player_hit ... god=true`) but take no health. F1 toggles it
(`god_mode on=...` in the log) and the HUD shows "(GOD)". `main.rs`: `--god`
starts with it on. README: new flag and key.
**Why:** So the player does not die and respawn during long scripted or
screenshot runs.
**Tested how:** `cargo run --release -- --walk --spawn clot --god --frames 600`,
read `logs/latest.log`; `cargo test --release`.
**Result:** Six Clot hits of 5.7-6.2 logged with `health_left=100 god=true`, no
`player_died`. Tests 29/29 pass. All player damage (melee, bile, fire) goes
through the one function that now checks god mode.
**Still broken / not tested:** The F1 key and the HUD "(GOD)" text are not
tested (no scripted key for F1; I can't see the screen). The Clot's grab still
pins the player in god mode (by design: only health is protected).
**Next:** Patriarch plan in `docs/DESIGN.md`.

## 2026-10-04 Patriarch plan, and step B1: his melee and hit rules

**Changed:** `docs/DESIGN.md`: new "Patriarch" section (his attack choice,
charge, chaingun, rocket, sneak, knockdown and healing, from ZombieBoss,
BossZombieController, BossLAWProj and his animation notifies; steps B1-B6).
New `src/boss.rs`: MeleeClaw / MeleeImpale with their ClawDamageTarget
notify times and reaches (85 / 45), IsCloseEnuf, the impale-or-claw choice.
`zed.rs`: the Patriarch's attacks start when close enough, on the upper
body from SpineBone1; hits land at each notify (impale twice), 75 -5..+5%,
and push the player (damageForce 170000); he never flips, flinches or is
stunned (`no_hit_reactions`). `combat.rs`: he keeps his head
(`keeps_head`; ZombieBoss.RemoveHead is empty). README updated.
**Why:** Patriarch step B1. Before this he walked up and stood still: he
has no MeleeAnims, so the shared melee code had nothing to play.
**Tested how:** `cargo test --release` (new: `boss_close_enough`,
`boss_impale_only_above_1500_and_half_the_time`,
`patriarch_keeps_head_and_never_flinches`).
`cargo run --release -- --walk --spawn patriarch --god --frames 1500`, and a
run with the camera pitched up at his head firing the 9mm
(`--camera -3110,1313,-3768,3.1416,0.55 --input 60:2,350:fire,...`).
**Result:** Tests 32/32 pass. Log: `boss_loaded claw_hits=[0.5]
impale_hits=[0.5, 0.6086956]`. Attacks every ~1.5 s once within 71 units;
the hit lands 0.77 s into the 1.53 s animation, the impale's second 0.17 s
later; damage 71.5-77.6; push `velocity_add_unreal=(421, -8, 170)`. Mix of
MeleeImpale and MeleeClaw at full health. Headshot: `damage=29.8
headshot=true head_health_left=0.0 decapitated=false`, no flinch animations.
**Still broken / not tested:** Not play-tested by you. Impale-vs-claw
below 1500 health not seen in game (unit test only). The melee push is only
for the Patriarch; other zeds' hits do not push yet. Noticed, not
investigated: once the player was knocked aside, the log shows him
switching TurnLeft / BossIdle / TurnRight within 50 ms (t=17.6), which may
look like a twitch; probably the shared turn-in-place code.
**Next:** B2, his charge.

## 2026-10-04 Patriarch step B2: charge

**Changed:** `boss.rs`: `BossState` (charge, LastChargeTime and
LastForceChargeTime timers), `decide` (RangedAttack beyond melee, with
Charging's override), `tick` (6 s limit, attacks used up), `charge_hit`.
`BossClass` now has ChargingAnim (RunF) and `transition`. `zed.rs`: the
decision runs every frame he sees the player; charging he runs with RunF at
GroundSpeed x 2.5 (x 1.25 and the normal walk while attacking); each hit
check uses one of the charge's 1-2 attacks, a landed hit ends the charge
and pushes x 1.5; `transition` plays on the upper body when a charge
starts. DESIGN and README updated.
**Why:** Patriarch step B2.
**Tested how:** `cargo test --release` (new `boss_charge_start_and_limits`,
`boss_charge_ends_on_hit_or_attacks_used`);
`cargo run --release -- --walk --spawn patriarch --god --frames 1800`.
**Result:** Tests 34/34 pass. Log: `boss_charge start attacks=2
distance_unreal=299`, animation RunF, `speed_unreal=300`; MeleeImpale 0.7 s
later; `boss_charge_attack landed=true ... ended=true`, `boss_charge end
reason=hit`; the 1.5x push left the player 134 away, so the impale's second
hit missed. Next charge 7.3 s after the first ended (he was attacking in
between).
**Still broken / not tested:** Not play-tested by you. The 6 s timeout and
the "over 700 away" end are tested by unit tests only (the scripted player
cannot run away). Charge from damage left out on purpose: it never happens
in KF (see DESIGN, a bug in ZombieBoss.TakeDamage). Whether the
`transition` animation is visible is not checked (I can't look).
**Next:** B3, the chaingun.

## 2026-10-04 Patriarch step B3: chaingun

**Changed:** `boss.rs`: `Chaingun` (state FireChaingun: PreFireMG, FireMG
repeated, FireEndMG; bursts, pauses, the lost-sight timeout), the chaingun
branch of RangedAttack (`chaingun_wait` = LastChainGunTime, 15% put off,
35-94 shots, the 15% "wants the chaingun" wish that blocks a charge),
`end_chaingun`, MG constants. `zed.rs`: new state `BossBusy`;
`boss_busy` (turn to the aim, sight check from the `tip` bone, animations,
shot from closer than 100 -> charge) and `boss_mg_shot` (trace from `tip`,
VRand x 0.06 spread, straight ahead if still turning more than 2000, 4-6
damage, momentum 500). `combat.rs`: `ray_cylinder` shared; damage tells
the zed how far away the shooter was. DESIGN and README updated.
**Why:** Patriarch step B3.
**Tested how:** `cargo test --release` (new `boss_chaingun_choice`,
`boss_chaingun_bursts`, `boss_chaingun_stops_when_sight_lost`); `cargo run
--release -- --walk --spawn patriarch --god --zed-at -4000,1313,-3818
--frames 2400`.
**Result:** Tests 37/37 pass. First game run: the log said `boss_chaingun
start` but he kept walking and fired nothing: the walking code set the
state back to Chase in the same frame. Fixed (walking skips `BossBusy`).
Second run: `boss_chaingun start shots=88` at 295 units, PreFireMG, `end
shots_left=0 seconds=9.72`, FireEndMG, `done` 1.5 s later; 88
`boss_mg_shot`, all `hit=player`, damage 4 (27x), 5 (26x), 6 (35x); none
fired while turning. A test assumption was wrong first: I expected losing
sight to stop him within ~0.6 s; KF's script resets that timeout every
FireMG loop, so it only runs out on a short roll (test rewritten to match).
**Still broken / not tested:** No tracer, muzzle flash or bullet impact
drawn (B3b). "Shot from closer than 100 while firing -> charge" is not
tested (no scripted way to knife him mid-burst). Shots only hit the
player, not other zeds. Not play-tested by you. Seen in the run, not
caused by this step: after a hit knocked the player onto something ~64
units higher, he flipped between Falling and walking many times a second
(the shared jump-obstacle code).
**Next:** B3b (chaingun effects) or B4 (rocket).

## 2026-10-04 Patriarch step B4: rocket

**Changed:** `boss.rs`: the rocket branch of RangedAttack (over 500 away;
25% put off for 0-5 s, else next in 10-25 s) and `Missile` (state
FireMissile: PreFireMissile, fire at its end, FireEndMissile). `zed.rs`:
`boss_busy` runs the rocket too (turns to the player, fires from `tip`);
`shoot_fireball` takes the projectile kind and start bone (bTrySplash only
for the Husk); `turn_toward` shared; the rocket's log line is
`boss_rocket_shot`. `fireball.rs`: `Projectile` kinds with their values
(BossLAWProj: speed 2600, 75 damage in radius 500, PanzerfaustTrail
pointing back, LawExplosion, RocketMarkDirt). `gore.rs`:
`load_piece_with_mesh` (mesh named in StaticMeshRef) sharing the static
mesh code with `load_piece`. `particles.rs`: PanzerfaustTrail and
LawExplosion loaded. `decals.rs`: `RocketMark`. DESIGN and README updated.
**Why:** Patriarch step B4.
**Tested how:** `cargo test --release` (new `boss_missile_choice_and_timing`;
three older boss tests needed their setups changed because the rocket check
now comes first); `cargo run --release -- --walk --spawn patriarch --god
--zed-at X,1313,-3818 --frames 1500..2400` with X = -4000, -4200, -4400;
Husk check: `--spawn husk --god --zed-at -4200,1313,-3818 --frames 1200`.
**Result:** Tests 38/38 pass. Loads: `fireball_loaded
class=KFChar.BossLAWProj draw_scale=0.7`, PanzerfaustTrail, LawExplosion,
RocketMarkDirt. At -4000 no rocket was fired (put off twice, then he
closed in and charged): not a bug, the rolls. At -4200: `boss_missile
start` at 1079 units, `boss_rocket_shot aim=middle` 2.37 s later (PreFireMissile
71 frames / 30), it hit the player after 0.42 s (~2550 units/s) for 75,
`done` 1.05 s later, next in 15.2 s. At -4400 the same, 75 at 891 units.
Husk unchanged: aimed at the feet, hit for 19, burn 9, 4, 2, 1.
**Still broken / not tested:** Not play-tested by you; the trail,
explosion and scorch mark are not checked by eye. A near miss (blast
damage falling off with distance) is not seen in game, only the Husk's
shared code covers it. Aim error, view shake, the rocket hurting other
zeds: not done.
**Next:** B5, cloak and sneak.

## 2026-10-04 Video recording (F9)

**Changed:** New `src/record.rs`: F9 (or scripted action `record`) toggles
recording; window captures at 30 fps of real time (a late capture covers
the slots it missed), numbered and put back in order on a writer thread,
piped as raw pixels to `ffmpeg` (H.264, max 1280 wide) into
`work/videos/<map>-<time>.mp4`, with ffmpeg's messages in
`<same name>.ffmpeg.log`. Quitting while recording finishes the file. The
title shows `[REC]`. ffmpeg is checked when recording starts.
`screenshot.rs`: `ScreenshotFrame` marks frames with a screenshot; the
recorder skips them. `flake.nix`: `pkgs.ffmpeg` in the dev shell. README
and DESIGN updated.
**Why:** You asked for short video clips toggled on and off.
**Tested how:** Inside `nix develop`: (1) `--walk --spawn patriarch --god
--input 300:record,600:record --frames 800`; ffprobe on the file; average
brightness once a second. (2) `--input 300:record --screenshot 450`
(screenshot during recording, quit while recording); average colour of the
screenshot vs. the video frame at the same time. (3) Outside the shell:
`--input 200:record --frames 300`. `cargo test`, `cargo clippy`.
**Result:** (1) `record_saved captures_written=150 ... ffmpeg_status="exit
status: 0"`; ffprobe: h264 1280x778, 30 fps, 150 frames, 5.000 s; the game
stayed at 60 fps; brightness 66 every second (not blank). (2) First try
hung until the timeout: Bevy drops a second capture of the window in one
frame, so the screenshot never arrived and the game never quit (the killed
run left an unfinished 164 MB file, deleted). Fixed with
`ScreenshotFrame`; second try: screenshot saved, `record_stop reason=quit`,
79 of 80 captures written (the last was still in flight), exit Success.
Average colour: screenshot 63/57/50, video 63/57/51 (channel order right).
(3) prints `recording: ffmpeg cannot run; start the game inside nix
develop ...`. Tests 38/38 pass, no clippy warnings.
**Still broken / not tested:** The F9 key itself is not tested (scripted
action only); not watched by a person. Your user-profile ffmpeg cannot run
inside the dev shell (glibc clash), hence the flake copy. Resizing the
window mid-recording drops frames of the new size (logged as
`wrong_size`). No sound (the game has none yet).
**Next:** Patriarch B5 (cloak and sneak).

## 2026-10-04 Firing effects F1-F3, F5: flashes, shells, tracers, impacts

**Changed:** Plan in `docs/DESIGN.md` ("Firing effects"). `ue-assets`
`emitter.rs`: trigger settings read (TriggerDisabled, ResetOnTrigger,
SpawnOnTriggerRange/PPS). `particles.rs`: `trigger()` (per-class table
from the effect scripts), `spawn_particles`, `spawn_all`, `set_start`,
`SpawnOptions` (persistent effects, render layer), case-insensitive class
lookup; five effect classes loaded. `weapon.rs`: each weapon's flash and
shell classes and bones; bone frames in world space; `weapon_fire_fx`
spawns them on the weapon layer and triggers them per shot; `ShotFired`
carries the tip. New `bullet_fx.rs`: tracers (one reused KFNewTracer per
shooter) and ROBulletHitEffect (default surface: rock puff + BulletHoleDirt
decal). `combat.rs`: player shots that hit the level send them (KF quirk
kept: none for zed hits or misses). `zed.rs`: the Patriarch's chaingun
sends them per shot; his MuzzleFlash3rdMG follows `tip` (first shot no
flash, KF quirk kept) and is killed on death. `decals.rs`: `BulletHole`.
`skinned.rs`: unused `pose` removed. README updated.
**Why:** You asked for tracers and muzzle flashes; the pistol drew nothing
when firing.
**Tested how:** `cargo run --release -- --walk --input
120:fire,180:fire,240:fire --frames 400` (in `nix develop`); `--walk
--spawn patriarch --god --zed-at -4200,1313,-3818 --frames 1100`; `cargo
test --release --workspace`; clippy.
**Result:** First try: no effects, `weapon_fx_missing
class=roeffects.MuzzleFlash1stMP` every frame: the class path came back
lowercase and the effect lookup was case-sensitive (fixed; and it now tries
once). Then per 9mm shot: flash 2+1 particles, shell 1 casing + 3 smoke
(`alive/spawned=[2/2 1/1]`, `[1/1 3/3]`); tracer from the tip
(-3244,1283,-3784) to a wall 1426 away, lifetime 0.184 (= (1426-50)/7500);
impact puff and bullet-hole decal at the hit. I first thought the tip (~123
units ahead) was wrong; logging showed MeshScale 5 on the 9mm mesh, so it
is where our drawn gun's muzzle is. Patriarch at ~1100 units: 52 shots, 52
tracers, 52 impacts, 52 decals; 15 hit the player, 37 the wall behind
(spread 0.06 at that range); the flash's 8 emitters each got particles.
Tests 38 + 21 pass, clippy clean.
**Still broken / not tested:** Not looked at by eye (no screenshot
inspected, per "log, don't look"): sizes, orientation and colours of the
flash, shells and tracers are unverified. The Patriarch's first tracer
starts ~100 units lower than the rest (PreFireMG's last pose; not looked
into). Bullet holes last 2 s; KF's real time is an open question (DESIGN).
No weapon light, sounds, or surface types yet.
**Next:** F4 (surface types), after you have looked at it.

## 2026-10-04 Tracers drawn as streaks (sprite width/height, direction, speed stretch)

**Changed:** `ue-assets` `emitter.rs`: ScaleSizeXByVelocity / Y / Z read; a
RangeVector property's missing axes now take the class default's (KF stores
only differing members; StartSizeRange defaults to 100 per axis).
`particles.rs` sprites: separate width and height when UniformSize is off;
UseDirectionAs Right (the sprite's width along the velocity, facing the
camera); width/height scaled by speed x ScaleSizeByVelocityMultiplier
(clamped 1..ScaleSizeByVelocityMax; the exact native rule is assumed).
**Why:** You saw tracers as small white squares. KFNewTracer is a 10 x 3
sprite whose width is stretched by speed x 0.001 (7500 -> about 75 units
long) and turned along its velocity; we drew every sprite as a square of
Size.X facing the camera.
**Tested how:** Dumped the settings of all 26 loaded effects: only
KFNewTracer has UniformSize off, so no other effect changes. Short firing
run (`--walk --input 120:fire,180:fire`); tests; clippy.
**Result:** Tracers still spawn and time out as before (lifetime 0.184 at
1426 units); no errors. Tests 38 + 21 pass, clippy clean.
**Still broken / not tested:** Not looked at by eye. Whether the streak's
length, thickness and brightness match KF is unverified; the speed-stretch
formula is my reading of the setting names, not KF's code.
**Next:** Compare with your in-game screenshot.

## 2026-10-04 Patriarch step B5: cloak and sneak

**Changed:** `boss.rs`: `Sneak` (InitialSneak from spawn, SneakAround),
`sneak_step` (the states' 0.5 s Begin loop: initial ends when he sees the
player, SneakAround after 10 s, re-cloak when not attacking), `end_sneak`,
the sneak branch of RangedAttack (20 s gap, 30% put off; ends a charge);
RangedAttack now rolls the chaingun wish before its bShotAnim check (KF's
order). `zed.rs`: sneaking = claw only (uncloaks), x 2.5 run with RunF
(normal speed while attacking), push x 1.5, the sneak ends at the claw's
damage check; cloak drawn per part (`cloak_parts`, Stalker-style); he
spawns cloaked. Tests updated (the boss now starts sneaking) and two new.
DESIGN and README updated.
**Why:** Patriarch step B5.
**Tested how:** `cargo test --release` (new `boss_initial_sneak_until_seen`,
`boss_sneak_every_20s_and_10s_long`); `cargo run --release -- --walk
--spawn patriarch --god --frames 3000`.
**Result:** Tests 40/40 pass. Log: spawned at t=4.83, `boss_sneak end
reason=found_player initial=true` at 5.24 (0.4 s: first loop pass); at
33.11 `boss_sneak start distance_unreal=101`, `uncloak reason=attack` 0.12 s
later, `end reason=melee landed=true` at the claw's hit. Charges, chaingun
and rocket as before.
**Still broken / not tested:** Not looked at by eye (cloak look). A long
sneak from far away (running cloaked) did not happen in the run; the 10 s
limit is unit-tested only. KF's refraction cloak shader, the commando
glow, and the shadow being hidden while cloaked are not done. Not
play-tested by you.
**Next:** B6, knockdown, escape and healing.

## 2026-10-04 Patriarch step B6: knockdown, escape, healing

**Changed:** `boss.rs`: `BossState::new(health)` (HealingLevels 3200, 2000,
1250; HealingAmount 1000), `check_knockdown`, `start_knockdown` (ends
charge / chaingun / rocket / healing), `knockdown_step` (cloak, Escaping),
`escape_step`, `begin_healing`, `heal_step` (syringe at 0.068, +1000 at
0.464), `escaping()`; RangedAttack does nothing while escaping. `zed.rs`:
knockdown start (BossBusy), `boss_busy` runs KnockDown and Heal (hides
Syrange1..3), `boss_escape` + `find_hide_spot` (SyrRetreat), the route-finder
aims at the hiding spot; escaping uses the sneak rules (x 2.5, RunF, claw
only, push x 1.5). `combat.rs` and the `hurt_zeds` test action call
`note_boss_health`. Two new tests. DESIGN and README updated.
**Why:** Patriarch step B6, the last planned one.
**Tested how:** `cargo test --release`; `cargo run --release -- --walk
--spawn patriarch --god --input <hurt_zeds at frames 400-408, 1900-1921,
3300-3307, 4400-4420> --frames 5200` (first try failed: my zsh loop turned
`$f:hurt_zeds` into `$f:h...`, a shell modifier; rerun with `${f}`).
**Result:** Tests 42/42 pass. Level 1: knocked down at 3100 (2.03 s),
`hide_spot=PathNode186 distance_unreal=1251 spot_seen_by_player=false`,
arrived in 6.0 s, syringe 1 at +0.35 s, 3100 -> 4100 at +2.35 s, done at
+5.05 s. Level 2: at 1900, PathNode73 (1312 away), arrived in 4.8 s,
1900 -> 2900, syringes 2. Level 3: at 1200 (health then fell to 100 from
the test's continued hurts); clawed the player twice while escaping (he was
beside him), ran cloaked at 300, then fell off a ledge near (-2969, 493,
-3790) and stood stuck (`zed_stuck` every second) until the run ended 11 s
in, before healing.
**Still broken / not tested:** The stuck after falling is the shared
movement code (likely the same spot as the falling / walking flip-flop
seen in B3); the 30 s give-up would heal him there; not investigated. A
third heal and "no fourth knockdown" are unit-tested only. Not looked at
by eye; not play-tested by you.
**Next:** The Patriarch's planned steps are done. Open: the stuck-after-
fall in the shared movement code; the not-planned parts (entrance, radial
attack, death camera, buddy squad, needle prop).

## 2026-10-04 Note: the Patriarch is unfinished

**Changed:** README ("What doesn't work yet") and DESIGN ("Patriarch")
now say the Patriarch is unfinished and list what is missing or untested.
**Why:** You asked for a note so it is not mistaken for done.
**Tested how:** Docs only.
**Result:** -
**Still broken / not tested:** See the list in the README.
**Next:** The stuck-after-falling movement bug, or whatever you choose.

## 2026-10-04 Weapons plan, and step W1: any weapon from data, inventory

**Changed:** DESIGN: new "Weapons" section (which weapons, rules from
the scripts, steps W1-W9). `weapon.rs`: any weapon class loads from its
defaults (Mesh or MeshRef, Skins or SkinRefs); `BASE_WEAPONS` (48, no
DLC); KF's starting kit; `--give all|Class,...` (`WeaponLoadout`);
inventory order (Pawn.AddInventory), slot keys 1-5 (Pawn.SwitchWeapon),
mouse wheel (KFWeapon.NextWeapon); `FireKind` from the fire class; guns
deal DamageMax; carried weight -> `weight_speed_mult`. `skinned.rs`:
`Skins.named`. `package_set.rs`: `find_object(path)`. `walk.rs`: speed x
weight factor. `combat.rs`: HUD shows the weapon name. `zed.rs`: spawn
Gorefast moved from G to H. `main.rs`: `--give`. Three unit tests.
**Why:** You asked for the weapons, without DLC.
**Tested how:** `cargo test --release`; `cargo run --release -- --walk
--give all --frames 200`; scripted switching (`--give
Machete,Axe,AK47AssaultRifle,Shotgun --input
100:1,200:1,300:1,400:1,500:3,600:3,700:3,800:next,900:prev,1000:5,1100:2`);
AK47 and 9mm against a Fleshpound; `--autowalk 3` on KF-Farm; a
screenshot of the AK47.
**Result:** Tests 45/45. 47 of 48 weapons load in 6.3 s. Order
Axe, Machete, Knife, 9mm, Frag, Syringe, Welder, Shotgun, AK47; key 1
cycles Axe -> Machete -> Knife -> Axe, key 3 Shotgun <-> AK47, wheel
steps correctly, key 4 with nothing there does nothing. AK47 hits for
22.5 (45 halved by the Fleshpound), 9mm headshot 19.2 (35 x 1.1 halved).
Walk speed 198 (was 200). The AK47 draws textured (screenshot).
**Still broken / not tested:** The ZED Gun MKII does not load (defaults
partly unparsed; no mesh package found). The AK47's SkinRefs path is
wrong in the game data; the mesh's own material is used. Projectile
weapons (shotguns, launchers, ...) animate and use ammo but deal no
damage. Not play-tested by you; most new weapons not looked at by eye.
**Next:** W2, bullet guns: auto/semi fire, the toggle, recoil, each gun's
reload.

## 2026-10-04 Melee alt attacks (middle click), one shot per click

**Changed:** `weapon.rs`: `FireMode` + `load_fire_mode` (both
FireModeClass entries); `Action::Fire { mode }`; middle mouse = alt fire
(scripted `altfire`); per-mode cooldowns with Weapon.ReadyToFire's
rules; a list of pending melee swings; bWaitForRelease per mode;
`melee_swing` and `alt_fire_not_implemented` logs. `properties.rs` /
`class_defaults.rs`: `get_at` for array elements. README, DESIGN.
**Why:** You saw no alt attack on melee weapons.
**Tested how:** `cargo test --release`; `--walk --god --give Axe,Katana
--spawn clot --input 60:1,400:altfire,401:altfire,402:altfire,560:fire,562:altfire,...`;
the same with `--give Axe --spawn fleshpound`.
**Result:** Tests 45/45. Katana HardAttack (205) took a Clot's head off;
three alt presses in a row gave one swing; fire then alt 2 frames later
gave only the first. Against the Fleshpound (halves all non-explosive
damage, as ZombieFleshPound.TakeDamage does): Axe PowerAttack 137.5,
swing 87.5, Knife Stab 27.5, slash 9.5 = 275 / 175 / 55 / 19 halved.
Animations found: hardattack, powerattack, stab.
**Still broken / not tested:** Melee still hits one zed and has no
backstab x2 (W5). Non-melee alt fires do nothing. One shot per click for
pistols etc. is not checked with a real mouse (scripted input cannot
hold a button). Not play-tested by you.
**Next:** W2 bullet guns, unless you want W5 melee first.

## 2026-10-04 Class defaults reader: pick the fullest property list

**Changed:** `crates/ue-assets/src/properties.rs` `find_class_defaults`:
of all byte offsets that read as a property list to the end, the one
with the most properties wins (was: the first).
**Why:** The Bullpup started with 10 rounds: BullpupAmmo's defaults were
read from a false start (byte 66) where a bogus "Range" value swallowed
MaxAmmo and InitialAmount (real start: byte 93).
**Tested how:** `kfpkg defaults` for all 1798 classes in KFMod, KFChar
and Engine before and after (`work/defaults_check/`), diffed;
`cargo test --release`; `--give all`.
**Result:** 37 classes changed, each gaining properties, none losing:
e.g. BullpupAmmo 4 -> 6 (InitialAmount 160), M14EBRAmmo 2 -> 7,
MKb42Ammo, AA12Ammo, M7A3MFire (RecoilRate 0.085 and vertical recoil 500
were missing), ZEDMKIIWeapon 23 -> 44 (MeshRef found). 48 of 48
weapons now load. Tests 49/49.
**Still broken / not tested:** Classes outside those three packages not
diffed. "Most properties" is a heuristic, not the real layout.
**Next:** W2.

## 2026-10-04 Weapons step W2: bullet guns

**Changed:** New `src/firing.rs` (KF spread, recoil kick and recoil
buffer, `apply_recoil`, 4 unit tests). `weapon.rs`: FireMode gains the
firing animations, high-ROF, fire-while-reloading, spread and recoil
values; weapon reload values (ReloadAnim/Rate, ReloadRate, bHoldToReload),
bModeZeroCanDryFire, toggle-on-alt list, select / put-down / idle names,
rates and times; state split from animation (`set_action`, `play`,
`play_firing`, `play_fire_end`, `interrupt_reload`, `allow_reload`,
`start_reload`); input rewritten (StartFire / ModeDoFire / StopFire,
NextFireTime carry-over, toggle, dry fire, reload timer, switch timers,
DownDelay); scripted `fire_down` / `fire_up`; logs `gun_shot`,
`weapon_anim`, `weapon_anims_missing`, `reload_*`, `fire_mode_toggle`.
`walk.rs`: shots scale the velocity. `combat.rs`: HUD [AUTO]/[SEMI].
`class_defaults.rs`: `is_a`. DESIGN, README.
**Why:** Step W2 of the weapons plan.
**Tested how:** `cargo test --release`; scripted runs: AK47 at a
Fleshpound (hold fire 1.7 s, toggle, semi shot, reload); 9mm and M4
(hold, aimed hold); Lever Action (3 shots, reload, fire mid-reload,
reload, aim mid-reload, full reload); Bullpup + 9mm while walking on
KF-Farm; switch timings; the melee test from last step again.
**Result:** Tests 49/49. AK47 spread 0.015 -> +0.02 per shot -> 0.12
cap, semi 0.0128; recoil 255-505 up, right only. M4: 25 shots in 1.80 s
(0.075 each), fire_loop / fire_iron_loop, aimed spread 0.004; camera
pitch +0.35 rad over the burst. AK47 reload 3.0 s (animation 3.03 s),
14 -> 30. Lever Action: a round per 0.667 s, fire at 8 rounds and aim
both interrupt. Walking 188 drops after each 9mm shot and recovers.
Switch: 0.317 s down + 0.33 s up (was over 1 s each); 0.071 s
DownDelay after a shot. Melee results unchanged.
**Still broken / not tested:** Nothing play-tested by you; recoil feel
and animations not looked at by eye. Holding the button through a
reload and the Kriss / Lever Action animation gaps follow the data
(see DESIGN). No crouch bonus (no crouching). A click during the
cooldown of a semi gun is kept until it can fire (my choice).
**Next:** W3, pistols: penetration, dual pistols.

## 2026-10-04 3D scopes (Crossbow, M99)

**Changed:** New `src/scope.rs` (render image 512 x 512, scope sky and
world cameras, reticle quad, lens material, `ScopeRequest`).
`weapon.rs`: `WeaponScope` from bHasScope / lenseMaterialID /
scopePortalFOV and the reticle textures named in UpdateScopeMode; while
aiming, the lens part shows the scope image; logs `weapon_scope`,
`weapon_scope_lens_uv`, `scope_portal`. `skinned.rs`: parts record their
material slot. `main.rs`: plugin. DESIGN, README, test-views.
**Why:** You saw scopes not rendering (iron sights fine).
**Tested how:** Screenshots aiming the Crossbow, M99, FN FAL ACOG and
M14 EBR before; Crossbow and M99 after, and the M99 turned toward the
ambulance (test-views entry). `cargo test --release`.
**Result:** Before: the Crossbow and M99 lenses were solid grey (the
fallback texture CBLens). After: live view with reticle, portal FOV 12 /
13.33, not mirrored. The ACOG already worked (masked lens material); the
M14 has no scope (peep sight). Lens UVs 0-1. Tests 49/49.
**Still broken / not tested:** How the reticle combines with the view is
my reading of the Combiner (drawn over by alpha), not checked against
KF; the M99's reticle shows light blue and a pale ring, which may be
wrong for that reason. Texture-scope and high-detail settings not done.
Not play-tested by you.
**Next:** W3, pistols.

## 2026-10-04 Weapons step W3: pistols; weapon effects; defaults reader rule

**Changed:** `combat.rs`: shots pass through up to `max_penetrations`
zeds, nearest first, damage halved and truncated. `weapon.rs`: fire
modes know FireAnim2 / FireAimedAnim2 and their penetration count;
effects per hand (`FxHand`); dual pistols alternate hands, tracer from
the firing gun, GOTO_Iron / GOTO_Hip; duals replace their single
(`merge_dual_ammo`, unit test). `zed.rs`: test action `zed_line`.
`particles.rs`: 27 more effect classes (every base weapon's flash and
shell) with their Trigger counts; status log shows positions.
`properties.rs`: defaults candidate with fewest non-delegate Raw values
wins, then most values. DESIGN, README.
**Why:** Step W3. Effects and reader rule: found while testing.
**Tested how:** `cargo test --release`; Handcannon and 9mm at
`zed_line`; dual 9mms (hip and aimed); the three other duals given with
their singles; screenshot of the left-hand dual shot; all 3757 script
classes through the original and new defaults reader (a git worktree
of f440bd8 under work/), diffed.
**Result:** Tests 50/50. Handcannon: 115, 57, 28 through three Clots;
9mm: 35 to the first only. Duals alternate fireright / fireleft with the
matching flash and shell; aimed fireright_iron / fireleft_iron; merges
30/210 (9mm), 16/80, 12/116, 24/120. 53 effects load, none missing.
Reader: 449 classes change, all for the better as far as I can tell.
**Still broken / not tested:** I spent too long trying to see flashes in
single screenshots (they appear ~4 frames after the shot); you confirmed
the Handcannon's; the other guns' flashes are not checked by eye. The
duals' ammo merge is my reading of GiveTo. The M4's tip bone X axis
points sideways, so its flash may point the wrong way (not checked).
**Next:** W4, shotguns.

## 2026-10-04 Reflex sights (SCAR, M4, Bullpup)

**Changed:** `crates/ue-assets/src/material.rs`: `resolve_skinned`; for
skinned meshes, a Shader whose Opacity texture feeds its Diffuse
Combiner takes that texture for colour and alpha, blended.
`skinned.rs`: uses it; Translucent -> alpha blend. `kfpkg materials`:
lists Shaders the rule touches. DESIGN, README.
**Why:** You saw the SCAR, M4 and Bullpup sights drawn wrong; they
showed an opaque speckle disc (screenshot of the SCAR).
**Tested how:** Screenshots aiming the SCAR and M4 before, M4 after;
`kfpkg materials`; `cargo test --release`.
**Result:** M4 sight: clear, red dot in the centre. The rule matches 39
of 1023 Shaders across all packages; kept off for level materials (many
use a plain mask as Opacity). Tests 50/50.
**Still broken / not tested:** The glass speckle layer (panning, added)
is dropped. SCAR and Bullpup not looked at after the fix (same material
as the M4). Shotgun tracers: shotguns do not fire pellets yet (W4).
**Next:** W4, shotguns.

## 2026-10-04 Frame rate cap (--fps)

**Changed:** `main.rs`: `--fps N` (1-1000); `limit_frame_rate` in the
Last schedule waits until the next frame is due (sleep, then spin the
last millisecond); after a slow frame it restarts from now rather than
catching up. Logged as `frame_limit`. README.
**Why:** You asked for an option to lock the frame rate at launch.
**Tested how:** `--walk --fps 30 --frames 400`, `--fps 60`, and no cap;
`frame_stats` in logs/latest.log.
**Result:** --fps 30: 30-31 fps, 33.3 ms per frame. --fps 60: 60-61,
16.7 ms. No cap: 60 (vsync at the monitor's refresh rate).
**Still broken / not tested:** Values above the refresh rate cannot be
reached while vsync is on (there is no option to turn vsync off yet).
**Next:** W4, shotguns.

## 2026-10-04 Weapons step W4: shotguns

**Changed:** New `src/projectile.rs` (player projectiles: pellets and
nails; penetration rule; walls; bounces; per-pellet tracers;
`penetration_limit` + test). `weapon.rs`: `FireKind::Pellets`,
`PelletFire` (projectile values, ProjPerFire, AmmoPerFire, Spread,
KickMomentum, ProjSpawnOffset), shotgun recoil, BoomStick rules
(`LastShot`, FireLastRate, auto reload, one-barrel redirect, no reload
with one barrel), `AltToggle` (fire mode / KSG wide spread), HUD
[WIDE]/[NARROW]; weapon_input's message writers grouped (Bevy's 16
parameter limit). `walk.rs`: `PlayerAddVelocity` (Pawn.AddVelocity).
`bullet_fx.rs`: `Shooter::PlayerPellet`. `combat.rs`: helpers
pub(crate). DESIGN, README.
**Why:** Step W4 (and your note that the shotgun had no tracers).
**Tested how:** `cargo test --release`; Shotgun at `zed_line`; Hunting
Shotgun (one, one, wait, both, both); KSG toggle; AA12 held; Nailgun.
**Result:** Tests 51/51. Shotgun: 7 pellets, 35 / 17.5 per zed, 52.5 /
26.2 on headshots; two Clots killed; kick (85 back, 15 up). Hunting
Shotgun: 10 pellets per barrel, fire_last when emptied, reloaded 2.5 s
later, both barrels 20 pellets with fire_both; clicks in the 2.75 s
cooldown ignored. KSG spread 1000 -> 2050. AA12 every 0.20 s. Nails
bounce twice.
**Still broken / not tested:** Not play-tested by you; tracers not looked
at by eye (each pellet's tracer is drawn at fire time to where it will
stop). Pellet meshes not drawn; stuck nails and pinned heads not drawn;
Trenchgun fire damage is W7; flashlights not done.
**Next:** W5, melee: the cone hitting every zed, backstabs, the
Chainsaw's held fire.

## 2026-10-04 Fix: the Shotgun drawn see-through

**Changed:** `crates/ue-assets/src/material.rs`: for skinned meshes a
Combiner keeps the blend it had before its inputs (an input texture's
bAlphaTexture no longer makes it translucent). `kfpkg materials` also
lists Combiners whose blend differs between level and skinned reading.
DESIGN.
**Why:** You saw the pump Shotgun transparent. Its Shotgun_D texture
has bAlphaTexture (the alpha is the reflection mask inside a Combiner);
since the reflex-sight fix, skinned Translucent draws blended.
**Tested how:** materials of every weapon and zed (`--give all` load
log); screenshot of the Shotgun; `cargo test --release`.
**Result:** Shotgun opaque. Remaining non-opaque skinned materials: the
sights (meant), the two chainsaw blades (scrolling blade texture with
alpha, now blended; not looked at), glows (additive), hair / gore
(masked). Tests 51/51. Levels unchanged (95 Combiners would differ;
not switched).
**Still broken / not tested:** Whether UE2 really ignores input alpha
in Combiners is my reading, not checked against the engine.
**Next:** W5.
