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

## 2026-10-04 Weapons step W5: melee

**Changed:** `combat.rs`: `resolve_swings` rewritten after
KFMeleeFire.Timer (traced zed stopped by walls, backstab x 2, wide hits
in sight within range x 1.1 at x cosine, melee headshot scale 1.25);
`MeleeSwing.min_dot` 0 = no wide hits. `weapon.rs`: ChainsawFire
(FireLoop like KFHighROFFire, damage at once, + Rand(maxAdditionalDamage),
no wide hits), ChopSlowRate. `zed.rs`: `yaw` and `dir_of` pub(crate).
Fixed: projectile and melee hits passed the hit distance converted twice
(logs showed 525 instead of ~22). DESIGN, README.
**Why:** Step W5.
**Tested how:** `cargo test --release`; Axe on three side-by-side Clots
and on a queue of three (`zed_line`); Chainsaw held on a Fleshpound and
its alt attack; Knife on a Clot.
**Result:** Tests 51/51. Axe swing: front Clot 175 (headshot) and the
one behind it 173 (x 0.99), both killed. Chainsaw: a hit every 0.1 s
for 14-18 (halved by the Fleshpound), fire loop / idle on release; alt
270. Knife: 19, headshot 23.8, distance 22.
**Still broken / not tested:** Backstabs not seen in a run (zeds face
the player); FlipOver, wall sparks, bloody skins, sounds not done. Not
play-tested by you.
**Next:** W6, projectiles and explosions.

## 2026-10-04 Weapons step W6a: grenades and rockets

**Changed:** `projectile.rs`: `ExplosiveStats`, `PlayerExplosive`
(flight, straight then falling, dud rule with impact damage, Explode:
effect, decal, HurtRadius on zeds with exposure, self damage halved,
no push). `weapon.rs`: grenade / rocket fire through the pellet path
(`PelletFire.explosive`), `total_ammo_only` (M79Fire, M203Fire,
LAWFire), `requires_aim` (LAW), `alt_ammo` (M4 203 grenades), HUD
[GRENADES n]; dry fire skipped for total-ammo weapons. `combat.rs`:
`HitSource.explosive`, ZombieFleshPound's full damage rule. `decals.rs`:
KFScorchMark. `particles.rs`: KFNadeLExplosion. `zed.rs`: test action
`zed_line_far`. `fireball.rs`: `axes_along` pub(crate). DESIGN, README.
**Why:** Step W6a.
**Tested how:** `cargo test --release`; M79 at a Clot 205 away (dud) and
at `zed_line_far`; LAW aimed; M32; M4 203 alt fire.
**Result:** Tests 51/51. Dud: 200 impact on the head (x 2) killed the
Clot. M79 at three Clots: 328, 257, 171 at 51, 132, 231 units (350 x
(1 - (d - 26) / 400)), all killed; M32 the same; M4 203 grenade killed
all three, its rifle magazine untouched, the next rifle shot waited out
the grenade's cooldown. LAW fired only when aimed.
**Still broken / not tested:** Self damage not triggered in a run.
Meshes, view shake and zed momentum not done. Not play-tested by you.
**Next:** W6b, frag grenade (G) and pipe bombs.

## 2026-10-04 Weapons step W6b: frag grenade (G) and pipe bombs

**Changed:** `projectile.rs`: shared `blast` (effect, decal, HurtRadius,
self damage); `ThrownStats` / `ThrownKind` and `PlayerThrown` (gravity,
bounce damping, rest, frag fuse with the first-bounce restart, pipe
arming / threat check / countdown). `weapon.rs`: `Action::Grenade` with
`NadePhase` (quick put-down, toss, nade at TossSpawnTime, quick
bring-up), G key and scripted `nade`; frag and pipe bomb values loaded;
pipe bombs fired through the pellet path. `zed.rs`: `motion_threat` from
MotionDetectorThreat. `combat.rs`: HUD FRAGS; self damage logged as
`zed=self`. `particles.rs`: KFNadeExplosion. DESIGN, README.
**Why:** Step W6b.
**Tested how:** `cargo test --release`; G at `zed_line_far`; a pipe
bomb dropped at the feet with a Scrake spawned.
**Result:** Tests 51/51. Frag: weapon down 0.13 s, toss, nade at 0.23 s,
second G refused during the throw; it bounced, rested, and exploded 2.45
s after the throw, killing the three Clots; 9 self damage. Pipe bomb:
rested 0.72 s, armed 1 s later, Scrake detected (threat 3), exploded 0.8
s later: Scrake 1370 (killed), self 661 (god mode).
**Still broken / not tested:** Shooting pipe bombs, Siren
disintegration, shrapnel, sounds, meshes in flight. Not play-tested by
you.
**Next:** W6c, Crossbow and M99.

## 2026-10-04 Weapons step W6c: Crossbow and M99

**Changed:** `projectile.rs`: `PenRule` (Pellet / Bolt), `StuckBolt`,
`BoltRoom`, `BoltPickedUp`, `pick_up_bolts`. `weapon.rs`: CrossbowArrow
and M99Bullet loaded as bolts (DamageTypeHeadShot multiplier),
CrossbowFire / M99Fire total-ammo only, bolt pickups add a round,
SpreadStyle SS_None fires straight. DESIGN, README.
**Why:** Step W6c.
**Tested how:** `cargo test --release`; Crossbow at `zed_line_far`; M99
at a Fleshpound.
**Result:** Tests 51/51. Crossbow: 300 then 240 through two Clots (both
killed), stuck in the wall; spread 0. M99: 675 (337.5 on the
Fleshpound), kick 150 back / 85 up.
**Still broken / not tested:** Bolt pickup not tried in a run. Meshes,
pinned bodies not done. Not play-tested by you.
**Next:** W7, fire: zeds burning, Flamethrower, Trenchgun, MAC10, Husk
gun.

## 2026-10-04 Projectile models (pipe bomb visible)

**Changed:** `projectile.rs`: `ProjectileModels` loaded at startup
(StaticMesh / StaticMeshRef of 7 classes), `attach_model`, `sync_bodies`
(transform follows the flight; resting pipe bombs lie flat), stats carry
their class. `package_set.rs`: `find_object` finds group-less
"Package.Name". `gore.rs`: `load_piece_with_mesh` uses it. `weapon.rs`:
passes the projectile class. DESIGN, README.
**Why:** You saw a dropped pipe bomb was invisible (no projectile was
drawn).
**Tested how:** load log; screenshots looking down at a dropped pipe
bomb; `cargo test --release`.
**Result:** All 7 models load (3 needed the group-less lookup). The pipe
bomb lies on the floor where it stopped. Tests 51/51.
**Still broken / not tested:** Grenades, rockets, frags and nails in
flight not looked at by eye. The Crossbow bolt (skeletal mesh), pellets
and the M99 bullet are not drawn.
**Next:** W7, fire.

## 2026-10-04 Fix: pipe bomb placed when the toss lets go

**Changed:** `weapon.rs`: `FireMode.spawn_delay` (PipeBombFire
ProjectileSpawnDelay 1.1 s), `Weapons.pending_spawn` (ammo used and bomb
spawned when it runs out), the next bomb's SelectAnim after Toss
(PipeBombExplosive.AnimEnd), `WeaponDef.gone` (the last bomb removes the
weapon and switches away). DESIGN.
**Why:** You saw the pipe bomb fall before the throw animation finished.
**Tested how:** `--give PipeBombExplosive`, clicking four times;
`cargo test --release`.
**Result:** Toss at 6.20, bomb at 7.30 (1.1 s), next bomb's select;
click: reload (dry fire), click: toss, bomb 1.1 s later; the weapon gone
and the 9mm brought up. Tests 51/51.
**Still broken / not tested:** The switch after the last bomb goes to
the previous weapon in slot order, not KF's best-rated one. Not looked
at by eye.
**Next:** W7, fire.

## 2026-10-04 Weapons W7a: zeds burning, Trenchgun

**Changed:** `combat.rs`: `FireType`, `HitSource.fire`, `ShotFired.fire`;
damage_zed applies the zed fire scales (Bloat, Husk), KFMonster's burn
rule (LastBurnDamage, FireDamageClass, x 1.5, HeatAmount, catching fire)
and skips the headshot multiplier for burned / flamethrower types.
`zed.rs`: burn state on `Zed`, `burn_zeds` (1 s ticks, flames effect
following the zed), speed x 0.8 while burning, BurningWalkFAnims after
crisp-up, `zed_class_fire` log. `weapon.rs`: `fire_type` from a damage
type's bDealBurningDamage, for bullets and projectiles.
`projectile.rs`: `ProjectileStats.fire`. `particles.rs`:
KFMod.KFMonsterFlame. `ue-assets/properties.rs`: the class-defaults
reader also counts Vector / Rotator / Color structs it could not decode
as bogus when choosing where defaults start. DESIGN, README.
**Why:** W7 of the weapons plan. The reader fix: DamTypeTrenchgun and
DamTypeBurned read from a wrong start, losing `bDealBurningDamage`.
**Tested how:** `--give Trenchgun` runs against Clots, a Scrake and a
Gorefast; `cargo test --release --workspace` (2 new tests: ignition at
15 / after 5 light hits, fire-type multipliers); reader compared over
all classes before and after (15 classes changed, all DamType* losing a
bogus value and gaining real ones).
**Result:** One Trenchgun shot lit a Scrake: 10 ticks of 30, 34, 37, 40,
44, 48, 51, 54, 57 (x 1.5 dealt), from 1000 to dead; flames spawned and
killed with it. Clots die from the shot itself. Tests 53 + 21 pass.
**Still broken / not tested:** The flames and burning walk not looked
at by eye; the burning walk not seen in a run (the test zeds were
attacking or dead by then). Flames start at the zed's centre, not on its
skeleton. Crisped skins, burn sounds, pain-animation rule not done. The
MAC10 stays non-burning (KF with no perk). Bloat / Husk fire scales only
by unit test (no weapon in W7a uses those damage types).
**Next:** W7b, the Flamethrower.

## 2026-10-04 Fix: own damage x 0.25 at Normal

**Changed:** `combat.rs`: `reduce_self_damage`, `SELF_DAMAGE`;
`apply_player_damage` reduces own damage (and the burn ticks it starts).
`projectile.rs`: `blast` sends the raw damage. DESIGN.
**Why:** Found while reading KFGameType.ReduceDamage for W7b: in single
player at Normal it halves your own damage twice (instigator == injured,
then GameDifficulty <= 3 in standalone). W6 halved it once.
**Tested how:** unit test `own_damage_is_quartered_at_normal`; `cargo
test --release --workspace`.
**Result:** A full M79 blast on yourself: 87 instead of 175. Tests 54 +
21 pass.
**Still broken / not tested:** Not checked in a run (the frag test
landed out of range). Difficulty is fixed at Normal.
**Next:** W7b, the Flamethrower.

## 2026-10-04 Weapons W7b: Flamethrower

**Changed:** `projectile.rs`: `FlameStats`, `PlayerFlame`, `move_flames`
(falling flame, 0.2 s speed resets, bursts on the second, on a zed or on
the level), `flame_burst` (Projectile.HurtRadius with DamTypeBurned, the
player included, burn mark, FuelFlame), `kill_effects_after`.
`weapon.rs`: FlameTendril values from the class defaults; FlameBurstFire
loops its animation like KFHighROFFire; it is not "total ammo only" and
does not fire while reloading; a misplaced doc comment fixed.
`particles.rs`: FlameThrowerFlameB, FuelFlame. DESIGN, README.
**Why:** W7 of the weapons plan.
**Tested how:** `--give FlameThrower` runs: three Clots, a Scrake, and
holding fire until empty then clicking; `cargo test --release
--workspace`; clippy.
**Result:** 100 flames in 6.93 s (0.07 s each), then the loop stops and
a click starts the 4.14 s reload. Flames burst at 0.40-0.42 s in the
air or on touch; Clots lit and killed; a Scrake lit (burn ticks 12 up to
43). Close to a zed, flames hurt you too (3-5 before the reduction, 0-1
after) and set you on fire for a few ticks. Tests 54 + 21 pass.
**Failure on the way:** the first runs kept firing with an empty tank
(FlameBurstFire is a CrossbowFire, so my Crossbow "ammo total only" rule
caught it), and played the fire animation once per flame instead of
looping it. Both fixed.
**Still broken / not tested:** Not looked at by eye (flames, trail,
burn marks). The second trail (an xEmitter) is not drawn; trails don't
grow. No sound. The view climbs while firing (KF's recoil; my scripted
test can't pull down, so later flames went high).
**Next:** W7c, the Husk Gun.

## 2026-10-04 Weapons W7c: Husk Gun

**Changed:** `weapon.rs`: `ChargeFire` (class by HoldTime, scaling, fuel
use), charge on press / fire on release, Charge then ChargeLoop
animations, the ChargeUp1stHusk effect on the tip, `husk_projectile`
(each class's effects); the Husk Gun's fireball now counts as a working
projectile. `projectile.rs`: `ExplosiveStats` gains `impact_on_touch`,
`fire`, `hurts_self`, and `fleshpound_mult` became optional; trails turn
backward only for the LAW's PanzerfaustTrail; the fireball models.
`decals.rs`: three burn mark sizes. `particles.rs`: the Husk Gun's
charge, trail and explosion effects. DESIGN, README.
**Why:** W7 of the weapons plan.
**Tested how:** `--give HuskGun`: a 0.15 s tap, a 1.3 s charge and a
4 s charge at a Scrake; a LAW shot for regressions; `cargo test
--release --workspace` (new: `husk_charge_picks_and_scales_the_fireball`).
**Result:** Tap: weak fireball, 1 fuel, impact 37.5, blast 26 in 165
units, Scrake lit. 1.32 s: medium, 4 fuel, impact 329, radius 282.
4 s: strong, 10 fuel, impact 750 (killed the Scrake), radius 450. No
damage to the player. Right trail, explosion and burn mark for each
class. LAW unchanged (backward trail, dud up close). Tests 55 + 21.
**Failure on the way:** the first run fired nothing: the W6 rule that
lists working projectile classes still left the Husk Gun's out.
**Still broken / not tested:** Not looked at (charge glow, fireballs,
burn marks). The glow does not grow with the charge; no camera shake or
sounds. Headshot impacts not seen in a run.
**Next:** W8, medic and equipment.

## 2026-10-04 Weapons W8a: Syringe, healing, quick heal

**Changed:** `combat.rs`: `GiveHealth` message, `PlayerHealth.to_give`,
`give_health` (KFPawn.GiveHealth: burn halved, cap, healthToGive),
`add_health` (10 per second), hits take 5 off healing to come, HUD shows
the Syringe charge. `weapon.rs`: `HealCharge` (charge, regen, costs,
inject delays), `syringe_fire` (no-target message; self heal with
InjectDelay), `QuickHeal` on Q. README (Q key), DESIGN.
**Why:** W8 of the weapons plan.
**Tested how:** runs without god mode: a Clot hitting me, Q from the
9mm, Q again while recharging, slot 5, left click, middle click while
recharging and after; `cargo test --release --workspace`; clippy.
**Result:** Q: Syringe out 0.35 s, AltFire, heal 0.1 s later (capped to
24 at 76 health), back to the 9mm 4.1 s after the injection. Q refused
at 130 charge (3.9 s after use: 10 per 0.3 s). Left click: the
"near another player" message. Middle click refused at 270 charge; at
full charge it healed 50 from 4 health. Tests 55 + 21.
**Failure on the way:** the first Q run brought the Syringe out but never
injected: the inject step ran on the same frame as the key press, with
the 9mm still in hand. It now waits for the Syringe.
**Still broken / not tested:** No on-screen message text (logged only).
No sounds. Not looked at by eye.
**Next:** W8b, medic gun darts.

## 2026-10-05 Weapons W8b: medic gun darts

**Changed:** `projectile.rs`: `DartStats`, `PlayerDart`, `move_darts`
(straight flight, bullet-hit effect on a zed or wall), dart models.
`weapon.rs`: medic guns get a `HealCharge` (not the Syringe's fire
modes) feeding the alt fire; HealingProjectile counts as a working
projectile; one dart per shot. `combat.rs`: HUD [DARTS %]. DESIGN,
README.
**Why:** W8 of the weapons plan.
**Tested how:** `--give MP7MMedicGun`: three alt fires at a Clot, a
primary shot, an alt fire 10 s later; `cargo test --release
--workspace`; clippy.
**Result:** Two darts (250 each) burst on the Clot with no damage; the
third was refused; after the charge refilled (250 in 7.5 s) the next
dart fired. Tests 55 + 21.
**Failure on the way:** the first run fired 250 darts per click (the
shotgun rule ProjPerFire x Load, with Load = the 250 charge).
**Still broken / not tested:** The MP5M, M7A3M and KrissM not run (same
classes and code path). The alt fire uses the primary's muzzle flash.
Not looked at by eye.
**Next:** W8c, Welder animations.

## 2026-10-05 Weapons W8c: Welder without doors

**Changed:** `weapon.rs`: `FireMode.weld`; WeldFire gives only the
no-target message (every 0.5 s while held), UnWeldFire nothing; no
animation. DESIGN, README.
**Why:** W8 of the weapons plan. The plan said "animates", but KF's
WeldFire.AllowFire refuses without a door in front, so it never animates
away from one; I followed the script.
**Tested how:** slot 5 twice to the Welder, a click, a 1.2 s hold, an
alt click; `cargo test --release --workspace`; clippy.
**Result:** One message per click, one every ~0.5 s while held, nothing
for alt fire, no animation (before: the fire animation and
"fire_not_implemented"). Tests 55 + 21.
**Still broken / not tested:** No doors, so no welding; the welder's
screen texture ("Integrity:") not drawn; messages are only logged.
**Next:** W9, ZED guns.

## 2026-10-05 Weapons W9a: zapping (ZED gun effect on zeds)

**Changed:** `zed.rs`: `ZapValues` from the class defaults, zap state on
`Zed`, `set_zapped`, `zap_tick` (in `burn_zeds`), half speed and the
burning walk while zapped, no run / rage / pounce / scream / cloak while
zapped, the lost run speed after a zap, test action `zap_zeds`.
`combat.rs`: damage x ZappedDamageMod. DESIGN.
**Why:** W9, the ZED guns, need it (they zap rather than only damage).
**Tested how:** unit tests `zap_builds_up_lasts_and_raises_the_threshold`,
`zapped_zeds_take_more_damage`; a run: a Gorefast running at the player,
`zap_zeds` mid-run; `cargo test --release --workspace`; clippy.
**Result:** Gorefast 225 (running) -> 60 zapped with WalkF_Fire for 4 s
-> 120 after (run speed lost, as in KF), next threshold 0.5. Tests 57 + 21.
**Still broken / not tested:** No zapped look (overlay material). The
blocked specials (Siren, Crawler, Fleshpound, Scrake, Patriarch) are
coded from the scripts but not run.
**Next:** W9b, the ZED MKII.

## 2026-10-05 Weapons W9b: ZED MKII

**Changed:** `weapon.rs`: ZED bolts and the zap orb read as explosives
with their own effects; bolts deal Damage on touch with no blast; one
projectile per shot for alt fires that override DoFireEffect
(`per_load`); `bModeExclusive`; ZED fires refuse during reloads.
`projectile.rs`: `ExplosiveStats.zap`, `blast` zaps (or does nothing for
radius 0), ZED models. `particles.rs`: ZED trails and impacts. DESIGN,
README.
**Why:** W9 of the weapons plan.
**Tested how:** `--give ZEDMKIIWeapon`: bursts and an alt fire at a
Scrake; alt fire then bolts; `cargo test --release --workspace`; clippy.
**Result:** Bolts every 0.125 s, 50 each; the orb (15 rounds, mag 24 ->
9) zapped the Scrake (1.5 >= 1.25) for 4 s, next threshold 2.5; bolts on
the zapped Scrake did 62.5 (x 1.25). Tests 57 + 21.
**Still broken / not tested:** Not looked at (bolts, orb, impacts).
Headshots and firing both modes at once not seen in a run. No sounds.
**Next:** W9c, the ZED Gun (its zapping beam).

## 2026-10-05 Weapons W9c: ZED Gun (stopped at working mechanics)

**Changed:** `weapon.rs`: `BeamFire` / `BeamState` (hold, ammo per
FireRate tick, recoil, Charge and ChargeDown animations, the charge glow;
no per-frame retry log when a glow class is missing). `projectile.rs`:
`BeamZap` message, `beam_zap` (trace, zap the zed touched, splash zap),
`BeamView`. New `zed_beam.rs`: the beam drawn as a textured strip.
`particles.rs`: ChargeUp1stZEDGun. DESIGN, README.
**Why:** W9 of the weapons plan. You said the ZED guns are optional (not
buyable normally), so I stopped at the mechanics.
**Tested how:** `--give ZEDGun`: bolts at a Clot line, then a held beam;
one screenshot mid-beam; `cargo test --release --workspace`; clippy.
**Result:** Bolts 85 each. The beam zapped all three Clots of a line
(the one it touched, then the others by the growing splash); ChargeDown
on release; the beam texture loads.
**Failures on the way:** the beam texture was not found at first (Skins
is a packed array, and the skin is a FinalBlend, not a texture); the
charge glow class was missing, which logged a retry every frame.
**Still broken / not tested:** The screenshot shows the gun's sleeves
plain white (KF fills that slot from the player's species) and its
screen as scrolling lines; left as is. The beam strip itself not seen
(the Clot was at the barrel in the shot). No sounds.
**Next:** your call; the weapons plan (W1-W9) is done.

## 2026-10-05 Doors D1: doors move and block

**Changed:** `crates/ue-assets/src/level.rs`: KFDoorMover settings
(`DoorInfo`) and KFUseTriggers (`UseTriggerInfo`), read with class
defaults. `map.rs`: doors spawn as their own entities, not as map
collision. New `door.rs`: mover keys and glide, TriggerToggle (open /
close, directional OpenToKey / CloseToFirst, DelayTime), USE on E (and
test action `use`) with ReFireDelay, zeds entering a trigger open its
doors, the player's trigger message (logged). `collision.rs`: `Door` and
`DoorTraces` layers, `zed_path_filter` for nav checks; ragdolls collide
with doors. Plan in DESIGN "Doors"; README.
**Why:** You asked for doors; first step of the plan.
**Tested how:** unit tests (open, glide, reverse mid-move, directional);
KF-Manor runs at KFDoorMover5: USE then walk; walk without USE; a Clot
behind the door; USE while the Clot approached; shots before and after
opening. Load logs on KF-Aperture, Farm, BioticsLab, WestLondon,
Hospitalhorrors. `cargo test --release --workspace`; clippy.
**Result:** USE opened the door in 1.25 s (yaw -16384 -> -1024) and the
player walked through; without USE the player stopped at x -1544
(blocked by KFDoorMover5). The Clot opened it to key 2 (away from
itself) and reached the player. A 9mm shot hit the closed door at 105
units, 892 after opening. Tests 60 + 21.
**Failures on the way:** a unit-test threshold was tighter than the
door's real speed (206 units per 0.01 s), not a bug.
**Still broken / not tested:** Not play-tested or looked at. A zed
standing in the trigger when the door shuts stays stuck behind it (as
the scripts say). Walkers pressing into a door or wall flicker into
falling (existing bug, also on BSP walls). Event-driven doors
(KF-Aperture) and other movers do not move. No sounds.
**Next:** D2, welding (or the wall flicker fix first, your call).

## 2026-10-05 Doors D2: welding

**Changed:** `door.rs`: weld state on doors and triggers (WeldStrength,
MaxWeld, sealed, Health), AddWeld / UnWeld / SetWeldStrength,
KFDoorMover.TakeDamage for the welder, bStartSealed, `WeldView` (door in
view) and `WeldHit` handling with the WelderHitEmitter. `weapon.rs`:
Welder fuel (300, +40/s), `weld_fire` (AllowFire rules and messages,
FireRate, delayed hit). `particles.rs`: preload WelderHitEmitter. DESIGN,
README.
**Why:** You asked for welders; D2 of the doors plan.
**Tested how:** unit tests (seal, cap, combat halving, sealed doors stay
shut, open doors not weldable, unweld below 0); KF-Manor run: Welder,
weld 8 s, USE, unweld, USE; load logs on KF-IceCave and KF-Steamland
(start-sealed doors). `cargo test --release --workspace`; clippy.
**Result:** 10 per weld hit every 0.2 s, fuel 280, 268, ... out after
about 5 s, then regen-limited; weld 270 of 400; USE gave "This door is
welded shut"; unweld 15 per hit to 0, unsealed, USE opened it. IceCave's
14 start-sealed doors at their percent (e.g. 160 of 400). Tests 63 + 21.
**Failures on the way:** none. Corrected W8c's comment: the welder's door
trace is 70 units, not 90.
**Still broken / not tested:** No HUD weld bar or welder screen (log
only). bDisallowWeld doors only covered by the code path, not a run.
Sparks not looked at. Perk weld speed not applied. A zed between you and
the door does not block the weld trace.
**Next:** D3, zeds bash welded doors.

## 2026-10-05 Doors D3a: zeds bash welded doors

**Changed:** `zed.rs`: DoorBash animation and Intelligence per class,
`ZedState::DoorBashing`, entering it on a bump into a sealed door,
`door_bashing` (the controller loop, hits on the ClawDamageTarget
notifies, leaving for a reachable enemy), test action `toggle_zeds`.
`door.rs`: `ZedDoorHit`, KFDoorMover.TakeDamage from zeds, DamageWeld,
GoBang with the break emitters, DoorPathNode and ExtraCost. `nav.rs`:
`extra_cost` per point used by routing, `probe_with`. `particles.rs`:
preload the break emitters. DESIGN (D3 split into D3a / D3b), README.
**Why:** You asked for welders; zeds breaking welded doors is the other
half. D3 was split: ranged door attacks (Bloat, Husk, Siren, Patriarch)
need their projectiles to damage doors and come next as D3b.
**Tested how:** unit tests (whole-number damage, floor of 5, break at 0,
welding halved after a zed hit); KF-Manor runs: weld 50 then a Clot;
weld 150 then a Fleshpound. `cargo test --release --workspace`; clippy.
**Result:** Clot: 5 per hit, two hits per DoorBash about every 1.6 s,
weld 50 -> 0 in 11 s, door broken (KFDoorExplodeWood), Clot then
reached the player. Fleshpound: three hits per bash, 28-30 each, 150
gone in two bashes. Path node JumpSpot30's cost went 560 .. 800 while
welded and back to 0 when broken. DoorPathNodes found for 8 of 8 doors.
Tests 65 + 21.
**Still broken / not tested:** Break effect not looked at. Siren and
Patriarch push against welded doors (no DoorBash animation; D3b). The
3 doors whose map sets another break effect use the class default.
Broken doors do not respawn (no waves). A zed that is already in a
trigger when the door is shut still gets stuck behind it unwelded.
**Next:** D3b, ranged door attacks; or D4 (grenades) — your call.

## 2026-10-05 Doors D3b: ranged door attacks and blasts

**Changed:** `door.rs`: `DoorBlast` (radius and direct damage to doors),
shared `go_bang`, a label on `ZedDoorHit`. `zed.rs`: per-class door
attacks in `door_bashing` (DoorBash, ZombieBarf / ShootBurns, Siren_Scream,
the Patriarch's rocket), the FindPath door check for
bCanDistanceAttackDoors zeds, Siren scream blasts reach doors, Siren and
Patriarch enter DoorBashing on a bump. `fireball.rs`: explosions send
`DoorBlast` (a door hit counts as direct). `nav.rs`: run-time reach tests
count closed doors. DESIGN, README.
**Why:** D3b of the doors plan.
**Tested how:** KF-Manor runs: Siren at a 50 weld; Siren screaming at the
player 214 units from a welded door; Bloat from PathNode109 at a 50 weld;
Patriarch at a 10 weld (and at 240, where he took another door); Clot
opening a shut unwelded door after the reach change.
`cargo test --release --workspace`; clippy.
**Result:** Siren: 5 per pulse, door broken in two screams. Scream radius:
the door 214 away lost 5 per pulse; two doors behind walls blocked. Bloat:
ZombieBarf from 280 units, 18 a barf, broken in three. Patriarch: rocket
hit the door directly, 63, broken; KFDoorMover2 399 away took 12 (6 off
health). At weld 240 the Patriarch routed round through the other door
(path cost). Clot still opens the shut door. Tests 65 + 21.
**Failures on the way:** with doors left out of the run-time reach test
the Bloat walked straight at the player and only bashed; counting doors
(as I believe native reachability does) gave the KF behaviour.
**Still broken / not tested:** Husk door attack not run (same code path as
the Bloat). Bloat vomit globs on doors not done. The radius test uses the
door's pivot, not its bounds. Effects not looked at.
**Next:** D4, grenades and unwelded door health from the player.

## 2026-10-05 Doors D4: grenades against doors

**Changed:** `door.rs`: `player_damage` (KFDoorMover.TakeDamage's player
rules), `DoorBlast.frag`, shared `damage_weld`, one `door_damage` log line
for zeds and the player. `projectile.rs`: every player blast sends a
`DoorBlast`, frag only for the Nade. `fireball.rs`, `zed.rs`: the new
field. DESIGN, README.
**Why:** D4, the last step of the doors plan.
**Tested how:** unit test (non-frag, under 50, unsealed half off health,
sealed off the weld); KF-Manor runs: three grenades at a shut door; weld
then a grenade; an M79 shell at the door. `cargo test --release
--workspace`; clippy.
**Result:** Grenade at 125 units: 210 damage, 105 off health (400 -> 295
-> 190 -> 85); KFDoorMover2 318 away took 72 (36). Welded about 170: one
grenade (186) broke it. M79: 260 at the door, ignored (frag=false).
Tests 66 + 21.
**Failures on the way:** my first M79 test fired into the steps in front
of the door (you spotted the staircase); the shell hit them inside its
300-unit arming distance and was a dud. Fired from the flat side instead.
**Still broken / not tested:** Break effects not looked at. Doors do not
respawn (no waves). Grenade blast reach uses the door's pivot, not its
shape.
**Next:** the doors plan is done; your call.

## 2026-10-05 Game loop G1: wave mode

**Changed:** New `game.rs`: `--mode waves|debug` (debug default, as
before), `--length`, `--wave N`; wave data from the class defaults and
the map; the KFGameType wave state machine; HUD line; win / loss /
restart. `properties.rs`: `string_array`, `struct_array`. `zed.rs`:
`SpawnZedAt` handling, `kill_zeds` test action, clearing on restart,
`is_dead` public. `map.rs`: loads the wave data in wave mode. `main.rs`:
the flags. `combat.rs`: wave text on the HUD. DESIGN (plan and G1),
README.
**Why:** You asked for waves, then the trader, then door respawns; G1
of the game-loop plan.
**Tested how:** unit tests (squad strings, spawn timing range, wave
masks); KF-Manor runs: waves 1-3 with `kill_zeds` every 4 s; `--wave 4`
with `next_wave` through the Patriarch to a win; `--wave 4` without god
mode to a loss and `restart_game`; a debug-mode run.
`cargo test --release --workspace`; clippy.
**Result:** game_data: Short waves 20/32/35/42, 27 squads, letters A-I as
KF, special squads in waves 3 and 4, WaveSpawnPeriod 2.5, 39 volumes.
Waves started 10 s in and 60 s after each end; wave 3's special squad (6)
came second; wave 4 capped at 32 alive; the Patriarch spawned, died, game
won. Loss detected on death; restart cleared 32 zeds. Debug mode logged
no wave data. Tests 69 + 21.
**Failures on the way:** WaveMaxMonsters is a byte, read as 0 at first
(every wave fell to the minimum 5).
**Still broken / not tested:** Spawning is at a volume's pivot (often in
view, some spots may be bad): G2. No stuck-zed cleanup, no x 0.75 zed
damage yet (G2). The Patriarch's own wave rules: G3. Not played.
**Next:** G2, KF's spawn volume rules.

## 2026-10-05 Game loop G2a: ZombieVolume spawning

**Changed:** New `zvolume.rs`: ZombieVolumes from the map (settings,
brush shape, RoomDoorsList, allowed / disallowed zeds), spawn point grid,
RateZombieVolume, PlayerCanSeePoint, Touch. `game.rs`: GameData holds
the volumes and zed info; FindSpawningVolume, SpawnInHere, failed spawns;
`SpawnZedAt` now gives the cylinder centre. `zed.rs`: uses it. DESIGN
(G2 split into G2a / G2b), README.
**Why:** G2 of the game-loop plan (you asked for the spawn rules next).
**Tested how:** unit tests (inside a brush, zed type filters); KF-Manor
runs: wave 1 and wave 3 with `kill_zeds`; `cargo test --release
--workspace`; clippy.
**Result:** 39 volumes, all with spawn points (3 to 121). Wave 1: squads
from volumes 942-1958 units away; wave 3: 11 squads, 942-7097 (median
1773), no failed or dropped squads; the Crawler / Stalker-only volumes
(38, 39) took squads with a Crawler and dropped the rest, as KF does.
Tests 71 + 21.
**Failures on the way:** 5 volumes had no spawn points: their pivots sit
20-42 units above the terrain and the 44-high tester overlapped it.
Lifting the tester (standing in for native SetLocation) fixed it.
**Still broken / not tested:** Not played. Distance fog ignored. The
native placement and Spawn fit tests are approximations. Hidden speed,
stuck-zed cleanup and the x 0.75 damage are G2b.
**Next:** G2b.

## 2026-10-05 Fix: zeds stuck behind the KF-WestLondon fence (jump pads); wall flicker

**Changed:** `nav.rs`: R_JUMP links used; JumpPads read (`add_jump_pads`)
with their pad -> target links kept; `nav_groups` log. `map.rs`: loads
the pads. `zed.rs`: a zed touching a pad is thrown with its JumpVelocity
from the pad's centre; `ZedWorld` system param (clippy). `walk.rs`:
`Mover::cast` re-sweeps a distance-0 hit on a surface the move runs along.
`game.rs`: clippy fixes. DESIGN, README.
**Why:** You saw zeds stuck at the tall fence behind the KF-WestLondon
start and remembered they jump high over it in KF.
**Tested how:** KF-WestLondon wave 1 runs (god mode, standing at the
start), logged routes, launches and positions; KF-Manor: walking into a
wall and into the shut door, a Clot opening the door. Tests, clippy.
**Result:** Before: the 5 zeds from ZombieVolume12 had no route
(`zed_path_none`), nav groups 11 (the pocket cut off). After: 1 group;
each of the 5 launched once from UTJumppad0 (up 729) and reached the
player. Pressing into a wall: 0 frames airborne (it flickered before).
Door still blocks at x -1544; the Clot still opens it. Tests 71 + 21,
clippy clean.
**Failures on the way:** launched where the zed first touched the pad,
the arc hit the fence top and zeds fell back and relaunched; launching
from the pad's centre (where the editor computed the arc from) clears it.
My clippy check earlier missed warnings from G1 (now fixed).
**Still broken / not tested:** 2 of 20 zeds still wedge in tight spots
(two surfaces at once) on KF-WestLondon; the pad launch point is an
approximation; players are not thrown by pads.
**Next:** commit G2a with this fix; then G2b.

## 2026-10-05 Game loop G2b: being seen, hidden speed, cleanup, solo damage

**Changed:** `zed.rs`: `solo_damage` (MeleeDamage, ScreamDamage x 0.75,
whole, at least 1), HiddenGroundSpeed per class, last seen / drawn / view
check per zed, hidden zeds at 300, `unseen_for`, `KillStuckZed` handling,
test action `kill_near_zeds`. `game.rs`: the stuck-zed cleanup. DESIGN
(plus the retro / map audit step you asked for), README.
**Why:** G2b; you agreed hidden speed and the damage apply in both modes.
**Tested how:** KF-WestLondon wave 1 with `kill_near_zeds` every 2 s;
KF-Manor debug runs with a Clot and a Siren; unit test; tests, clippy.
**Result:** Zed speed logs: mostly 300 (hidden), 105 when seen. Cleanup
killed 4 never-seen zeds (including the 2 that wedge in the air) at 5 or
fewer left, one a second; the wave ended. Clot hits 3.9-4.2 (was about
6); Siren scream 6 at full strength (was 8). Tests 72 + 21.
**Still broken / not tested:** "Drawn" is an approximation (view cone and
sight). Hidden speed is not applied to running / raging / charging
speeds. Not played.
**Next:** the retro and map audit, then G3 / D5 / trader.

## 2026-10-05 Retro and map audit

**Changed:** New `docs/map-audit.md` and `docs/retro-2026-10-05.md`.
`map.rs`: `map_features` log at load (map classes we simulate / do not,
with counts). Stale comments corrected (zed.rs module doc, nav.rs, the
Welder, map.rs movers, weapon.rs Projectile). DESIGN.
**Why:** You asked for a retro and a closer look at the maps, so map gaps
(like the jump pads) are not mistaken for AI errors.
**Tested how:** class counts over all 35 maps (`kfpkg exports`), class
defaults and scripts for the gameplay ones, props of 5 maps for fog,
teleporters, lava; a KF-WestLondon load for the new log line. Tests, clippy.
**Result:** 257 classes. Biggest gaps: 691 glass windows in 12 maps (not
blocking; KF zeds break them), distance fog in every sampled zone (KF's
sight checks skip beyond it; ours do not), trader rooms, lava (13 maps),
plain movers and the event system. KF-WestLondon logs simulated=[doors 16,
triggers 19, volumes 35, jump pads 6, ...] not_simulated=[KFGlassMover:71
Mover:3 KFTraderDoor:4 ...].
**Still broken / not tested:** the gaps listed in the audit.
**Next:** your call: glass and fog first (audit order), or carry on with
G3 / D5 / the trader.

## 2026-10-05 Map fixes M1: glass windows

**Changed:** New `glass.rs` (KFGlassMover: panes, damage, break, crack,
shards). `level.rs`: GlassInfo. `map.rs`: panes as own entities, the
cracked material, `map_features` lists glass as simulated. `combat.rs`:
shots and melee hitting a pane damage it. `walk.rs` / `zed.rs`: bumps
(zeds swing MeleeAnims[0]). `particles.rs`: the two glass emitters.
DESIGN (map-fixes plan M1-M4), map audit, README.
**Why:** You asked to fix the maps first; glass was first in the audit.
**Tested how:** KF-WestLondon runs at KFGlassMover94 (shots, a Clot
outside, a grenade). Tests, clippy.
**Result:** 71 panes, all with colliders, Health 5. Shot: broken by 35;
next shot passed through to a wall 1140 away. Clot (hidden, speed 300):
broke it on the bump. Grenade: 7 panes within 420 broken (195 at 146 ..
6 at 410). Tests 72 + 21.
**Failures on the way:** my first test shots hit the shop-front mesh's
wooden window bar (I aimed at the window's centre), which looked like a
collision bug; a screenshot showed the bar.
**Still broken / not tested:** pellets / arrows do not break glass; the
break event (TriggerEvent) not simulated; visuals not looked at; the
debug log file is shared by every running copy (a second run overwrites
it).
**Next:** M2, distance fog.

## 2026-10-05 Map fixes M2 (distance fog) and M3 (lava, KillZ)

**Changed:** `bsp.rs`: nodes keep back / front children; `point_zone`.
New `zones.rs` (zone fog, the player's zone, camera fog), `pain.rs` (pain
volumes, KillZ). `map.rs`: zone fog and pain volumes at load,
`map_features` (lava simulated). `zvolume.rs`: `brush_polys` shared; fog in
the rating and PlayerCanSeePoint. `zed.rs`: fog in the drawn / seen test,
level damage to zeds. `game.rs`: the player's fog in the spawn checks.
`combat.rs`: `LEVEL_DAMAGE` (level damage is not own damage). DESIGN,
map audit, README.
**Why:** next in the map-fix plan (you asked to keep working on the maps).
**Tested how:** KF-WestLondon: zones and fog logged, one screenshot; wave
1 spawns; standing in LavaVolume2; spawn point counts unchanged after the
refactor; unit test for touching a volume; tests, clippy.
**Result:** 26 zones, 14 with fog; start zone -500..4500; fog visible in
the screenshot. Wave 1 squads unchanged at the start (all nearby volumes
inside the fog). Lava: 1 on entering, then 1 a second (100 -> 94). Tests
73 + 21.
**Failures on the way:** level damage first used the own-damage marker
(would have been cut to a quarter); caught before running.
**Still broken / not tested:** point_zone is from memory of UE2, checked
only by the start zone looking right; the fog colour / look not compared
with KF; KillZ not seen triggering (nothing fell out).
**Next:** M4, the player on jump pads and KF-Offices teleporters.

## 2026-10-05 Map fixes M4: the player on jump pads; teleporters checked

**Changed:** `walk.rs`: a player touching a jump pad is thrown with its
JumpVelocity from the pad's centre. DESIGN, map audit, README.
**Why:** M4 of the map-fix plan.
**Tested how:** KF-WestLondon, starting on UTJumppad0; props of every
map's Teleporter / KFTraderTeleporter. Build; tests and clippy below.
**Result:** launched up 729, arc top about 280 above the pad, landed at
(-2036, 314) next to the target PathNode111 (-2027, 306). No map has an
enabled teleporter with a URL (KF-Offices' 6 are disabled), so nothing to
simulate.
**Still broken / not tested:** not played; pads launch from the centre
(approximation).
**Next:** shotgun pellets and arrows against glass, then the event system
/ movers (needs a plan).

## 2026-10-05 Glass: pellets and bolts break panes

**Changed:** `projectile.rs`: a pellet / nail / bolt hitting a pane does
its Damage to it (ShotgunBullet / CrossbowArrow HitWall). `walk.rs`: the
walk's map params as a type (clippy). DESIGN, audit, README.
**Why:** the gap left in M1.
**Tested how:** KF-WestLondon shop window with the Shotgun; tests, clippy.
**Result:** a shot broke KFGlassMover94 (pellet 35 vs Health 5). Tests
73 + 21, clippy clean.
**Next:** your call on the event system (movers, scripted triggers).

## 2026-10-05 Map fixes M5: placed emitters; the sky drawn unlit

**Changed:** `ue-assets/emitter.rs`: `read_emitter_actor`. `particles.rs`:
`load_effect` shared, map emitters loaded and spawned
(`spawn_map_emitters`). `map.rs`: unlit material copies for the sky zone,
bUnlit actors and PF_Unlit faces. `level.rs`: MeshActor.unlit. DESIGN,
audit, README.
**Why:** You saw fires missing on KF-WestLondon and the sky changing
colour with the view angle.
**Tested how:** screenshots of the sky in 4 directions before and 2
after; load logs on 4 maps; the taxi fire's particle log; tests, clippy.
**Result:** Sky: before, one side bright orange and the other dark (sun
lighting the dome); after, the same brownish clouds both ways. Emitters:
KF-WestLondon 37, KF-Manor 42, KF-Farm 7, KF-BioticsLab 14, none failed;
the taxi fire (Emitter8) keeps about 25 flames rising 100+ units.
**Failures on the way:** two screenshots wasted (one camera inside a
wall); per your earlier advice I stopped hunting and ask you to look.
**Still broken / not tested:** fire look not checked by eye; DrawScale of
emitter actors not applied; map decals (M6) and baked lighting not done.
**Next:** M6, map decals.

## 2026-10-05 Map fixes M5b: sky layers in a fixed order

**Changed:** `map.rs`: see-through sky-zone meshes get a depth_bias from
their distance to the sky camera, so they draw farthest first whatever
the view; log `sky_layer_order`. DESIGN.
**Why:** your two screenshots: same spot, only the pitch changed, and the
sky went from orange to grey-brown. Unlit (M5) did not fix it.
**Tested how:** retook both of your camera poses
(`--camera " -4197,438,-3778,-1.6274,-0.0258"` and `...,-1.6434,0.4102`);
tests, clippy.
**Result:** both poses now show the same grey-brown sky. Ordered layers:
StaticMeshActor1256 (fog ring, 60 units), StaticMeshActor126 (cylinder,
130), StaticMeshActor1291 (dome, 166). Tests 94 pass; clippy 1 warning,
an old one in `boss.rs` (not this change).
**Still broken / not tested:** not known whether KF's sky is the orange
or the grey-brown one (the orange came from a different layer being on
top); cylinder StaticMeshActor111 is opaque/masked so not reordered; not
checked in play by you.
**Next:** you check the sky and the fires; then M6, map decals.

## 2026-10-05 Map fixes M6: decals placed in maps

**Changed:** `ue-assets/level.rs`: `ProjectorInfo`, read for every placed
Projector (KFBloodSplatter included). `map.rs`: hands them to
`decals.rs` (`MapProjectors`). `decals.rs`: the projection pulled out into
`project()` (shared with the blood decals), now with a widening volume for
FOV and a surface filter (BSP / meshes / terrain); `load_map_decals`
resolves ProjTexture by blend (modulate, alpha blend, add);
`spawn_map_decals` builds them once, with bGradient fade and CullDistance.
Two unit tests. DESIGN (plan first), audit, README.
**Why:** your report of missing decals on London; the audit's M6.
**Tested how:** load logs on KF-WestLondon, KF-BioticsLab, KF-Hellride,
KF-Bedlam; one screenshot on KF-WestLondon; tests, clippy.
**Result:** KF-WestLondon built 71 of 72 (68 blood, 3 light);
KF-BioticsLab 49 (35 light); KF-Hellride 145 of 149; KF-Bedlam 64 (26
PB_None blood). No texture failed. Screenshot: blood splats on the road
by the ambulance. Tests 96 pass; clippy 1 warning, the old one in
`boss.rs`.
**Failures on the way:** the screenshot camera ended up inside a vehicle;
the road was visible through its window, so I left it.
**Still broken / not tested:** empty ones (KF-WestLondon Projector2, a
scorch on a mesh, z -1067; 4 KF-Hellride splats) probably sit on meshes
without collision (our surfaces are collision triangles). Guesses: the
wide FOV shape, PB_None drawn as modulate, linear gradient. Light
patterns are added as-is (KF multiplies them by the surface texture).
TexRotator projections (fans) do not turn. Not looked at by you.
**Next:** your call: baked lighting, or back to the game loop (G3, D5,
T1-T3).

## 2026-10-05 Fix: smoke drawn as grey squares (sprites with no texture)

**Changed:** `particles.rs`: a sprite emitter whose Texture is None is not
drawn (it still runs); `effect_loaded` notes now list each sprite's draw
style and material chain.
**Why:** your screenshot: big grey squares in the KF-WestLondon tunnel.
KF-WestLondon's Emitter23 (at -7053, 3295, -3762) has a smoke emitter
(SpriteEmitter23, alpha blend) with Texture = None; we drew it untextured.
**Tested how:** log (only 2 textureless sprites loaded: Emitter23 and
KFMod.WelderHitEmitter's SpriteEmitter42); screenshot down the tunnel
toward Emitter23; tests, clippy.
**Result:** no squares at Emitter23. Tests 96 pass; clippy only the old
`boss.rs` warning.
**Still broken / not tested:** assumed KF draws nothing for a textureless
sprite (not in the scripts; the welder's has no visible squares in KF).
My screenshot may not be your exact view; not checked by you.

## 2026-10-05 Fix: Clot gib props drawn see-through (cut-out alpha)

**Changed:** `map.rs`: a plain texture flagged bAlphaTexture whose alpha
is on/off only (under 1% of pixels in between) is drawn masked, not
blended (`alpha_is_binary`); log `material_see_through` lists every
non-opaque map material with its blend and chain.
**Why:** you saw a corpse prop in the KF-WestLondon tunnel drawn
transparent: StaticMeshActor288 / 1172 (22Patch.ClotGibLowerTorso /
ClotGibLeg) use kf_generic_t.Generic_Gibbs, bAlphaTexture, whose alpha is
a cut-out mask (33% at 0, 67% at 255, nothing between). Blended surfaces
write no depth, so the mesh's far side showed through its front.
**Tested how:** alpha histogram of the texture; the log; a screenshot of
the gib at (-7097, 2893); tests, clippy.
**Result:** KF-WestLondon: 4 textures switched blended -> masked
(Generic_Gibbs, LabCommon.voidtex, LondonCommon.SkyLine, Statics.Rubbish1);
soft-alpha ones (fog rings, moss, leaves, road stripes) stay blended. The
gib looks solid in the screenshot (partly behind the gun). Tests pass;
clippy only the old `boss.rs` warning.
**Still broken / not tested:** SkyLine is the sky's city silhouette, now
masked: not looked at. Other maps not checked. Not checked by you.

## 2026-10-05 Baked lighting L1 + L2: BSP lightmaps, mesh vertex colours

**Changed:** Plan in DESIGN ("Baked lighting", milestone 11).
`ue-assets/bsp.rs`: nodes keep section / first vertex / lightmap;
`read_lighting` reads the rest of the Model (sections with lightmap UVs,
lightmap records skipped, lightmap textures). `ue-assets/lighting.rs`:
`read_mesh_instance_colors`. `level.rs`: MeshActor.instance. `kfpkg
lighting <map>` (checks, writes pages to work/lighting) and `kfpkg raw`
(hex dump). `src/lighting.rs`: brightness K (2, a guess), lightmap
upload, lightmapped material. `map.rs`: BSP drawn with Bevy lightmaps;
meshes with baked colours drawn unlit with their own coloured copy; sun
and ambient no longer light lightmapped surfaces. README.
**Why:** you chose lighting next: maps looked flat and too bright.
**Tested how:** `kfpkg lighting` on all 35 maps; KF-WestLondon load log;
screenshots at the player start and in the tunnel (before and after);
KF-Hell load; tests, clippy.
**Result:** mesh colours = mesh vertex count on all 35 maps (e.g.
KF-WestLondon 1607, KF-Manor 1793). BSP lighting reads on all 35; all
polygons' points equal their section vertices. KF-WestLondon: 12 pages,
1986 polygons lightmapped, 69 unlit, 1595 actors baked, 74 not.
Screenshots: blue moonlight on the viaduct, light pooled at the
ambulance, a dark tunnel; before, everything was evenly lit. Tests 96
pass; clippy only the old `boss.rs` warning.
**Failures on the way:** first guesses at the Model layout (fixed-size
leaves, a lights array right after them) desynced; found the layout by
walking it until it ended on the last byte. KF-Clandestine, KF-Forgotten,
KF-Hell failed until I found their pages are saved empty.
**Still broken / not tested:** brightness K (x2) and the tonemapping not
compared with the game; those 3 maps' BSP keeps the sun; zeds, weapons,
hands still sun-lit (L4); terrain (L3) not done. Not checked by you.
**Next:** your comparison with the real game; then L3 / L4.

## 2026-10-05 Lighting vs the real game: colour order fixed, tonemapping off; stuck on the colour cast

**Changed:** `ue-assets/lighting.rs`: mesh colours are R, G, B, A (were read
as B, G, R). `camera.rs`: tonemapping off on both cameras (UE2 had none).
`kfpkg lighting <map> [X,Y]`: probe of the polygon under a point (page,
UVs, lightmap value). STATUS.md written (the old ragdoll record kept
below it).
**Why:** your 4 real-game screenshots of KF-WestLondon.
**Tested how:** average colours of 6 matching regions, real vs ours, at
the spawn; experiments with fog off and K = 4 (both reverted); the probe
and the moonlight's shadow bits against the stored page; tests, clippy.
**Result:** the viaduct went from blue to warm, as in the real game. The
stored lightmap matches its shadow bits exactly, so the data is right.
Still off: ours is too dark at K = 2 (about K = 3 matches brightness) and
grey where the game is orange and saturated (red-to-blue about half of
the real game's). Table in STATUS.md.
**Failures on the way:** two real attempts at the colour gap (tonemapping
off, the fog analysis) did not explain it, so I stopped and wrote
STATUS.md. I overwrote STATUS.md before looking at it; the old content is
restored below the new one.
**Still broken / not tested:** the colour cast; K; the sky layer order
(the real sky is orange); the 3 maps with empty pages; L3, L4.
**Next:** your call (STATUS.md "Ideas not tried yet").

## 2026-10-05 Lighting LV: KF's vision overlay (the orange look)

**Changed:** `src/overlay.rs`: a third camera (order 2, after the scene
and the weapon) draws a full-screen quad blended as 2 x source x screen,
the colour following HUDKillingFloor.DrawModOverlay (zone fog colour or
KFOverlayColor, brightened, eased from black). `zones.rs` / `map.rs`:
zones carry their overlay colour; KFSPLevelInfo.bUseVisionOverlay read.
`weapon.rs`: the weapon camera's tonemapping off as well. DESIGN (LV),
STATUS (marked mostly resolved), README.
**Why:** your matching screenshot (precise.jpg): the whole frame, sky
included, was more orange in the game; the scripts show KF draws
KFX.SepiaShader over the view (it reduces to white, OB_Modulate).
**Tested how:** same pose (`--camera " -2760,1772,-3768,-3.6652,-0.0652"`),
7 region averages real vs ours; log `vision_overlay`; tests, clippy.
**Result:** red/green/blue ratios real/ours now about equal (left wall
1.08, 1.05, 1.07; far buildings 0.98, 0.97, 1.0; before about 1.3, 1.1,
0.9). First try put the overlay at the weapon camera's order (1): the HUD
was cut off and Bevy warned; moved to 2. Tests 96 pass; clippy only the
old `boss.rs` warning.
**Still broken / not tested:** BSP and tarp about 20% dark; sky slightly
less orange; phone booth glass not drawn; a fire decal brighter in KF;
the fade between zones not watched; not checked by you.
**Next:** your notes: sky layer order, booth glass, fire decal.

## 2026-10-05 Fix: window glass invisible (phone booth glass)

**Changed:** `ue-assets/material.rs`: a Shader with an Opacity texture
(OutputBlending normal) is blended, not masked at 50%, and keeps the
Opacity texture (`SimpleMaterial.opacity`); FinalBlend AlphaTest no
longer overrides a blending FrameBufferBlending (it only drops alpha below
AlphaRef). Skinned meshes keep the old masked rule. `map.rs`: colour from
the diffuse, alpha from the Opacity texture (`texture_with_alpha`); the
on/off-alpha -> masked rule now applies to any blended map material; log
`glass_pane_material`. `kfpkg mesh <file> <name>`: sections, materials,
triangles by texture alpha.
**Why:** your note: the phone booth glass did not render.
**Tested how:** `kfpkg mesh` (the booth mesh has no glass: the glass is
4 KFGlassMover panes); log of pane materials; close-up and your matching
pose, side by side with precise.jpg; KF-Manor load; tests, clippy.
**Result:** all 71 KF-WestLondon panes (GlassShader /
GlassShaderDoubleSide: Diffuse = a combiner, Opacity = WindowGlassTex)
were masked at 50% and so invisible since M1; now blended: the booth
glass dulls the ambulance behind it as in the game (KF's is a bit
lighter and greener). Also now blended: FBGlass and FogFB (additive),
ChurchLampFB, HeavyFenceFB, RedPhoneBoothWindowFB. Tests 96 pass; clippy
only the old `boss.rs` warning.
**Still broken / not tested:** the glass's reflection layer is dropped;
additive map materials are still drawn as alpha blend; other maps' glass
and fences not looked at; not checked by you.

## 2026-10-05 Fix: KF-Farm completely black (vision overlay)

**Changed:** `overlay.rs`: until a zone gives a tint, the overlay leaves
the view unchanged (it started black and stayed black).
**Why:** you saw KF-Farm all black after LV. Its start zone (ZoneInfo19)
has fog but bNoKFColorCorrection, so no tint was ever chosen and the
screen was multiplied by black. KF's DrawModOverlay returns without
drawing in that case.
**Tested how:** KF-Farm screenshot and log; KF-WestLondon log
(`vision_overlay` still ZoneInfo4); tests, clippy.
**Result:** KF-Farm visible (moonlit field). Tests 96 pass; clippy only
the old `boss.rs` warning.
**Still broken / not tested:** KF-Farm's terrain still uses the sun
(L3); KF's fade-in from black at the start now only happens in zones
that tint. Other maps not checked.

## 2026-10-05 G3a: the Patriarch wave rules (helpers, his death, spawn)

**Changed:** `game.rs`: AddBoss as in KF (volume search, then again
ignoring the 5 s failed-spawn wait; every spawn point tried; 32 "at
once"); StartWaveBoss's 60 s limit (no volume in 60 s: the wave ends
without him); FinalSquads read from KFMonstersCollection (3 squads,
logged as `game_final_squads`); AddBossBuddySquad (8 helpers for one
player, at the end of each knockdown when FinalSquadNum == SyringeCount,
ignoring the zed limits); DoBossDeath. `zed.rs`: zeds go `braindead` when
he dies (no thinking, moving or attacking, still shootable, not counted),
as KF's GameEnded controllers; test action `kill_boss`. `boss.rs`: counts
finished knockdowns. `zvolume.rs`: the ignore-failed-time flag.
SpawnInHere's limits are now parameters.
**Why:** G3 in DESIGN.md. The boss wave spawned him and could be won, but
had no helpers, and his death left the other zeds fighting (the wave
could not end until you killed them all).
**Tested how:** `--mode waves --length short --wave 5` on KF-WestLondon
with scripted `hurt_zeds` and `kill_boss`; log lines `boss_spawned`,
`boss_knockdown`, `boss_helpers`, `boss_killed`, `zed_braindead`,
`wave_end`, `game_end`. Tests, clippy.
**Result:** First knockdown: squad 0 (4 Clots), 8 spawned in 3 passes
from two volumes. Second knockdown: squad 1 (3 Clots and a Crawler),
8 in 5 passes. Kill him with 8 helpers alive: all 8 braindead, wave
ends 0.1 s later, won 1 s after that. Tests 96 pass; clippy clean.
**Still broken / not tested:** the 60 s no-volume path not exercised;
braindead zeds drop their attack and idle (KF lets the animation play
out with no controller); no zed time on his death (no zed time at all
yet); the grand entrance and death camera are G3b. Not played by you.
**Next:** G3b (entrance) or the trader (T1-T3), your call.

## 2026-10-05 T1: dosh

**Changed:** new `dosh.rs`: the player's cash (starts at 250) and the
team pot, from KFGameType ScoreKill / ScoreKillAssists /
RewardSurvivingPlayers. A zed the player kills pays int(ScoringValue x
1.75 on Short) (Clot 12, Gorefast 21, Fleshpound 350, Patriarch 875) to
the player and the pot; the pot is paid out at each wave end; dying
costs 10%; a restart resets it. Zeds carry their class's ScoringValue
and whether the player killed them (`zed.rs`, `combat.rs`). `game.rs`
counts wave ends and restarts. The HUD shows "DOSH 250".
**Why:** T1 in DESIGN.md; the trader needs money to spend.
**Tested how:** waves run on KF-WestLondon with `kill_zeds` (logs
`dosh`), a debug run killing a Clot with the pistol, a screenshot of the
HUD, unit tests for the sums (kill score, the pot, the death penalty).
**Result:** wave 1: 20 kills paid 249 in total and the same again at the
wave end (748 after wave 1). Pistol kill: +12. Tests 99 pass; clippy
clean. The HUD font has no £ sign (it drew a box), so it says DOSH.
**Still broken / not tested:** the death penalty is only unit-tested
(solo waves end on death anyway); test kills pay (KF's KillZeds would
not); burn and bleed-out kills only follow the same code paths, not run.
Not played by you.
**Next:** T2, shops (ShopVolumes, trader doors, the trail).

## 2026-10-05 T2a: shops and trader doors

**Changed:** new `trader.rs`: the map's ShopVolumes and Teleporters;
SelectShop, OpenShops, CloseShops, BootShopPlayers and ShopVolume.Touch
as in KFGameType / ShopVolume / Teleporter.Accept. The wave timer
(`game.rs`) opens the trader 1 s into the between-wave time (not before
wave 1), closes it and picks the next shop at the wave start, and asks
for a boot every wave tick. KFTraderDoors are read like KFDoorMovers
(`level.rs`, `trader` flag) and moved by the same mover code, but kept
in their own list (`door.rs`, `Doors::trader`, collider marked
`TraderDoorCollider`) so welding, damage and zed bashing ignore them.
HUD: "TRADER: Nm" and KF's two shop messages for 3 s. Test action
`warp_shop`.
**Why:** T2a in DESIGN.md.
**Tested how:** KF-WestLondon waves runs with `kill_zeds`, `warp_shop`,
`next_wave` (logs `shops_loaded`, `shop_doors`, `shop_selected`, `shop`,
`door`, `shop_touch`, `shop_boot`) and screenshots; every map loaded in
waves mode (shops, teleporters, trader doors); unit tests for the shop
pick and open / close.
**Result:** WestLondon: 4 shops, 24 teleporters, 4 trader doors. After
wave 1 the church shop opens 1 s later, its door swings open in 1 s;
standing in it shows "Press 'E' to TRADE"; at wave 2 the door closes,
Corner Shop is picked for next time, and you are teleported to a church
exit with "You can't stay in this shop after closing". Walking into a
closed shop between waves boots you too. All 34 game maps have shops;
KF-Transit has 2 shops without teleporters (KF cannot boot there
either); KF-Suburbia's shops also trigger event counters (not
simulated). Tests 101 pass; clippy clean.
**Still broken / not tested:** "inside a shop" is the player's centre in
the brush, not KF's cylinder touch; the trail and HUD arrow (T2b), the
trader's animation and voice lines, buying (T3). Door sounds not played.
Not played by you.
**Next:** T2b (the trail) or T3 (buying).

## 2026-10-05 T3a: buying weapons and ammo

**Changed:** new `buy_menu.rs`: the catalogue (every base weapon's pickup:
Cost, AmmoCost, Weight, BuyClipSize, bKFNeverThrow; KFLevelRules' eight
per-perk sale lists), a keyboard text menu (E in the shop with no wave:
Up/Down, Left/Right perk filter, Tab For sale / Yours, Enter buy or sell,
C clip, F fill, Shift for second ammo, E or Backspace closes; the wave
start or leaving the shop closes it), and an input gate: while the menu
is open, keys, mouse buttons, mouse motion and wheel are cleared after
the menu reads them, so the player does not move, look or fire.
`weapon.rs`: ServerBuyWeapon / ServerSellWeapon / ServerBuyAmmo
(`shop_requests`): a bought weapon is loaded then (0.1 s) and inserted
where Pawn.AddInventory puts it (stored indices shifted); sold weapons
are marked gone and reloaded fresh if bought again; duals replace their
single (ammo merged) and give it back when sold; weight slows you as
before. Test actions `add_dosh`, `buy_menu`, `menu_*`, `buy:C`, `sell:C`,
`ammo_fill:C`, `ammo_clip:C`, `ammo_clip2:C`.
**Why:** T3a in DESIGN.md: dosh can now be spent.
**Tested how:** scripted shopping on KF-WestLondon after wave 1 (logs
`shop_buy`, `shop_sell`, `shop_ammo`, `shop_refused`, `weapon_given`,
`buy_menu`), screenshots of the menu; catalogue loaded on all 34 maps
(no map overrides the lists, nothing missing); unit tests for the sale
list and ammo prices.
**Result:** Shotgun 500, refused when owned; 8 shells for 20; Handcannon
refused at 228 dosh, bought after selling the Shotgun for 375; Dual
Handcannons at half price (500) with the single, weight 3 -> 5; AA12
to exactly 15 kg, then Dual 9mms refused as too heavy; selling the dual
Handcannons paid 187.5 and gave the single back; 9mm fill 120 rounds for
80; M4 203 grenade 1 for 10 (KF prices the second ammo at the gun's
AmmoCost). Tests 103 pass; clippy clean.
**Still broken / not tested:** the input freeze while the menu is open is
not tested by a run (scripted input bypasses the keyboard); the menu by
real keys not tried by me; a weapon sold while reloading or mid-throw
is swapped abruptly; armour (T3b), first aid, perks and their discounts
are not in; the trader's GUI look is a text list. Not played by you.
**Next:** T3b armour, or D5 door respawns, or T2b the trail.

## 2026-10-05 T3b: armour (the Kevlar vest)

**Changed:** new `src/armour.rs`: `Armour` (ShieldStrength and xPawn's
SmallShieldStrength), `absorb` (KFPawn.ShieldAbsorb copied line by line,
int in and out), `buy_kevlar` (KFPawn.ServerBuyKevlar), the `BuyVest`
request and its system (CanBuyNow, logs `shop_vest` / `shop_refused
request=vest`). `combat.rs`: `PlayerDamaged` gains `armor_stops` (the
damage type's bArmorStops); `apply_player_damage` runs the vest after
the self-damage reduction, not in god mode, and resets it on death;
`player_hit` logs `damage_before_armour`, `armour_before`, `armour`; HUD
"ARMOUR n". `zed.rs`: the Siren's scream skips armour; `pain.rs`:
falling out of the world skips it (Gibbed); every other source keeps it.
`buy_menu.rs`: a "Combat armour" row at the end of "Yours" (points/100,
fill price), Enter or F buys; test action `buy_vest`. `main.rs`
registers the plugin. DESIGN.md T3b plan; README; handoff.
**Why:** T3b, next in the handoff's order.
**Tested how:** read KFPawn.ShieldAbsorb / ServerBuyKevlar, Pawn.TakeDamage,
KFBuyMenuInvList, HUDKillingFloor; scanned `kfpkg defaults` of every
DamageType subclass for bArmorStops. 6 unit tests (absorb, breaking,
buying incl. partial and the no-armour quirk, menu prices). Scripted runs
on KF-WestLondon, waves, Short: wave 1 with `kill_zeds`, `warp_shop`,
`buy_vest`, menu on the vest row, `next_wave`, wave 2 without god,
`spawn_siren`. Screenshot of the menu.
**Result:** before wave 1 (shop shut): refused `not_in_open_shop_time`.
Trader time: `shop_vest bought=full cost=300.00 armour=100.00
dosh_left=448.00`; buying again (action, menu Enter, menu F) refused
`armour_full`. Wave 2: Husk fireball 24 -> you 6, vest 100 -> 82; claw
11.28 -> you 2, vest 39.25 -> 31; the breaking hit 10.87 on 3.25 -> you 6
(int(10) - 3.25), vest 0; Siren screams 1-5 left the vest unchanged.
After the death, the next hit shows armour 0. Menu row "Combat armour
armour 100/100 fill 0" and HUD "ARMOUR 100" in the screenshot. Tests
109 pass (was 103); clippy clean.
**Still broken / not tested:** the partial buy (some armour, short of
dosh) is only unit-tested, not run in the game. Not played by you with
real keys. No perks, so no perk armour discount or damage modifier, and
no starting armour. The vest's name is taken from BuyableVest's default
text, but `kfpkg defaults KFGui.BuyableVest` labels that value
`WinWidth` (the KFGui property names come out wrong; not looked into).
Found, not changed (separate steps): zed melee damage reaches the player
as a fraction (e.g. 10.81) where KF passes an int; god mode still
lowers healthToGive and starts burns, where KFHumanPawn.TakeDamage
returns first.
**Next:** D5, doors respawn at wave end.

## 2026-10-05 D5: doors come back at the wave end

**Changed:** `door.rs`: `RespawnDoors` message and `respawn_doors`
system; `Doors::respawn_door` (KFDoorMover.RespawnDoor with Mover.Reset
in TriggerToggle: shown, collision back, DoClose, the 0.001 s snap to
bShouldBeOpen's end, bStartSealed re-welded, Health = MaxWeld on every
door); `Door::should_be_open` (bShouldBeOpen, set when an open or close
is skipped on a sealed or hidden door); `door_layers` (collision layers,
shared with the spawn); test action `break_doors`. `game.rs`:
`do_wave_end` sends `RespawnDoors`. 5 unit tests. DESIGN.md D5 plan;
README; handoff.
**Why:** D5, next in the handoff's order.
**Tested how:** read KFGameType.DoWaveEnd, KFDoorMover (RespawnDoor,
GoBang, DoOpen/DoClose/DoOpenToKey/DoCloseToFirst), Mover (Reset,
SetResetStatus, DoClose, TriggerToggle). Found the maps with bStartSealed
doors (`kfpkg props` over all maps: KF-Aperture 2, KF-IceCave 14,
KFO-FrightYard 24, KF-Steamland / KFO-Steamland 3, KFO-Transit 1).
Logged run on KF-Aperture, waves, Short, `--god`: `break_doors` during
wave 1, `kill_zeds` to its end.
**Result:** 18 doors broken (`door_broken`), at the wave end
`doors_respawn broken_back=18 health_reset=81`; each `door_respawned`
key 0, closed, and KFDoorMover30 / 31 `sealed=true weld=120000` (their
StartSealedWeldPrc of the 120000 MaxWeld), the rest unwelded. Unit tests:
a door broken open glides shut over MoveTime and stays not-bClosed (the
welder refuses it), unbroken doors keep their weld and heal, an open
asked while hidden snaps open on respawn. Tests 114 pass; clippy clean.
**Still broken / not tested:** not looked at on screen (door visible
again, solid to the player and zeds): only the logs. The door-broken-open
quirk is only unit-tested. The 63 TriggerControl doors on KF-Aperture
are not simulated (as before), so `break_doors` skips them (no trigger).
Not played by you.
**Next:** T2b, the trader trail.

## 2026-10-05 T2b-1: the trail to the trader

**Changed:** new `src/trader_path.rs`: switching on at OpenShops and off
at CloseShops or on touching the open shop (`TraderPath`), a whisp every
1.1 s (KFGameType.ShowPathTo: current shop, its first teleporter, a nav
route to it, at most 16 points), its way points (TraderPathEffect,
including KF's unset-index RouteCache check), its flight (WillowWhisp
StartNextPath / Pathing.Tick plus PHYS_Projectile, copied), its sprites
(xEmitter, assumed from RedWhisp / WillowWhisp settings: 90 a second, at
most 150, 1.25 s, size 25-30 growing 13/s, red, 4 x 4 tiles, spin, a
small rise, fade in and out) drawn as one camera-facing additive mesh
with RedWhisp's texture (alpha kept). `main.rs` registers it. DESIGN.md:
T2b split into T2b-1 (this) and T2b-2 (the HUD arrow, later). 3 unit
tests.
**Why:** T2b, next in the handoff's order.
**Tested how:** read KFPlayerController (SetShowPathToTrader, Timer),
KFGameType (OpenShops, CloseShops, EndState, ShowPathTo), ShopVolume.Touch,
TraderPathEffect, RedWhisp, WillowWhisp, xEmitter, and the defaults.
Logged runs on KF-WestLondon, waves, Short, `--god`: wave 1 ended with
`kill_zeds`, trader time watched; a second run with `warp_shop`.
Screenshots during trader time.
**Result:** `trader_path on=true reason=open_shops` 1 s after the wave
end; 14 whisps 1.1-1.2 s apart, each with 8 way points; each ends at the
last point about 3.5 s after starting, at about (-3180, -395) against
the shop's (-3156, -448), 58 units (KF's reached distance is 80).
About 115 sprites per whisp in flight. `warp_shop`: `trader_path
on=false reason=touched_shop`, no whisp after it. First screenshot:
visible grey-edged squares around each sprite: the texture
(ROEffects.SmokeAlphab_t) holds its shape in alpha and I had dropped
it; with alpha kept the trail is soft red smoke. A unit test of mine
expected the whisp to pass close to a sharp corner; worked by hand,
KF's rules (move on once the velocity turns away) take it 404 units wide,
so the test was wrong, not the code; the test now checks that number.
Tests 117 pass; clippy clean.
**Still broken / not tested:** the smoke's look is assumed in parts
(native xEmitter: fade curve, spin units, the rise, alpha weighting); not
compared side by side with the real game, but you looked at it in our
game and said it looks great. The HUD arrow (T2b-2) is not done.
**Next:** T2b-2, the HUD arrow; or G3b.

## 2026-10-05 T2b-2: the arrow to the trader; `--window`

**Changed:** new `src/trader_arrow.rs`: KFShopDirectionPointer
(DebugObjects.Arrows.debugarrow1, its own texture, unlit, DrawScale
0.25) on its own camera (order 3, after the overlay) and render layer,
so it draws over everything; placed along the ray through the screen
point (width / 18, width / 18), turned toward the current shop (level
unless the shop is over 50 units above or below); hidden without a
current shop, with the buy menu open, or outside wave mode. Logs
`trader_arrow_ready`, `trader_arrow`. 2 unit tests. `main.rs`:
`--window WxH` (window size at scale factor 1, not resizable) for
comparisons with KF screenshots; the plugin registered.
**Why:** T2b-2, the second half of T2b.
**Tested how:** read HUDKillingFloor.DrawKFHUDTextElements and the
pointer's defaults. Screenshots, then your two real-game screenshots
(references/, 1280 x 960, KF-WestLondon spawn, first countdown).
Ours at `--window 1280x960`, `--camera -3110,1313,-3768,-3.72,0` (the
view turned until the lamppost sat at the same pixel as yours, x 465),
red pixels of the arrow measured with ImageMagick.
**Result:** first build: the arrow covered a quarter of the screen and
ran off its edge (you saw the same). KF's numbers (10 units along the
ray, DrawScale 0.25) give that size in our renderer, checked by hand
(6.5 units side-on at 10 units = 673 px at our focal length; measured
678). So the native ScreenToWorld (not in the scripts) must scale
differently; I could not find how, so the distance is fitted: real
arrow x 13-122, y 50-89, 1920 px; ours at 3x / 4x / 5x still too big;
7.5x: x 12-123, y 47-91, 2052 px; 8x: 16-120, 48-90, 1814 px; kept 7.75x:
x 14-121, y 48-90, 1934 px. Side by side the shapes and angle match. The
centre matched at every distance, so the placement formula is right.
In the normal widescreen window it sits small in the corner. Tests 119
pass; clippy clean.
**Still broken / not tested:** the 7.75 is fitted from one view; other
screen shapes and zoomed (iron sights) views are not compared with the
real game. KF's "Trader: 36m" text under the arrow is still our plain
HUD line. Not played by you.
**Next:** your call: G3b (the Patriarch's entrance), or the lighting
leftovers.

## 2026-10-05 H1: KF's HUD, the bottom bar

**Changed:** DESIGN.md: new milestone 12, "KF's HUD" (H1-H4). New
`src/hud.rs`: reads HUDKillingFloor's SpriteWidgets, NumericWidgets,
DigitsSmall / DigitsBig and KFHUDAlpha from the class defaults, loads
their textures, and draws each frame through a pool of Bevy UI image
nodes: DrawHudPassA's bottom bar (health with UpdateHud's colours,
armour, weight box and icon, grenades, per-weapon ammo boxes from its
IsA checks, secondary ammo, flashlight box, syringe, welder, MP7M / MP5M
charge) and the cash. Test action `hud_dump` logs every quad's pixel box
and the weapon. `combat.rs`: `AmmoDisplay` gains class, capacity,
hold-to-reload and welder fuel; our old text HUD is a small debug line
at the top, F3 toggles it. `weapon.rs`: fills those fields; alt ammo is
only reported when the weapon really has a second ammo (the 9mm had
reported an empty one). `main.rs`: registers the plugin. 2 unit tests.
**Why:** you asked for KF's HUD now that there is a good reference.
**Tested how:** compared with your screenshots at `--window 1280x960`
from the matched view (`--camera -3110,1313,-3768,-3.72,0`): side-by-side
crops of both bottom corners; `hud_dump` while cycling all weapons
given with `--give`; a widescreen screenshot.
**Result:** `hud_loaded sprites=34 numerics=12 textures=20
missing_textures=[] absent=[]`. Health box at (19, 898)-(109, 942); in
the screenshot about 20-108. First shot: two differences: the weight
box drawn 88 px wide instead of 135 (Bevy kept the texture's proportions;
now stretched) and a wrong secondary-ammo box for the 9mm where KF shows
the flashlight box (fixed both). After that the corners match the
screenshot: 100 / 0 boxes, flashlight 100, 15, 7, 3, the pound sign and
250. Per weapon: Shotgun (shells + flashlight), MP7M (charge box),
Flamethrower, Crossbow (arrowhead), Husk Gun, M4 203 (grenade box), LAW,
Welder, Syringe, Knife (no ammo boxes), 9mm, as DrawHudPassA. Tests 121
pass; clippy clean.
**Still broken / not tested:** texts need KF's fonts (H2): the weight
"1/15", weapon name, "Trader: Nm". The wave circle (H3), messages (H4).
The quick-syringe popup and the bile colour are not done. Wide windows
stretch the boxes as KF's formula says; not compared with the real game
at 16:9. Not played by you.
**Next:** H2, KF's fonts.

## 2026-10-05 H2: KF's fonts and the HUD texts

**Changed:** new `crates/ue-assets/src/font.rs`: reads UE2 Font objects
(glyph table, page textures, kerning, CharRemap) and looks glyphs up
(through the remap for remapped fonts); `kfpkg fonts` reads every Font in
the install. `src/hud.rs`: loads HUD.FontArrayNames and
SmallFontArrayNames (ROFontsTwo.ROArial24DS-7DS) with their pages;
Canvas.StrLen / DrawText; GetFontSizeIndex; the weight "1/15"
(LoadSmallFontStatic(5), scaled ClipX / 1024), DrawWeaponName and
DrawTraderDistance. Text is laid out in physical pixels like KF's canvas.
1 test in ue-assets, 1 in hud.rs.
**Why:** H2, next step of the HUD milestone.
**Tested how:** hex dump of ROFontsTwo.ROArial14DS (4367 bytes); `kfpkg
fonts` over the install; the remap order checked on ROFonts.ROBtsrmVr12;
screenshots at `--window 1280x960` from the matched view compared crop by
crop with your screenshot; the default HiDPI widescreen window.
**Result:** the layout added up to the object's exact size (1 + 2 + 256 x
17 + 3 + 9 = 4367). First scan: all 120 fonts "failed" with "0 bytes
left over": my end check used the reader's `is_empty()`, which means "no
data at all", not "nothing left" (fixed to `remaining()`); then
`summary fonts=120 failed=0`. Remapped fonts store (character code,
glyph index): ROBtsrmVr12 maps 260 (A with ogonek) to glyph 256, ASCII
to itself. `hud_fonts` loaded all 7 HUD fonts with their pages. Side by
side at 1280 x 960: "1/15", "9mm Tactical" and "Trader: 35m" (yours
36m: we stand a few units from your spawn) in the same fonts, sizes and
places. At 2556 wide the weight text grows and the weapon name does not
(KF's own rules: one scales with width / 1024, the other's size steps
stop at 1600). Tests 123 pass; clippy clean.
**Still broken / not tested:** the glyph advance (USize + Kerning) is
assumed from the native code (Kerning is 0 in these fonts). Fonts other
than the HUD's are read but not drawn anywhere yet. You confirmed it
working in the game.
**Next:** H3, the top-right wave / countdown circle.

## 2026-10-05 H3: the top-right circle; debug line hidden

**Changed:** `src/hud.rs`: DrawKFHUDTextElements' circle: between waves
Hud_Bio_Clock_Circle with the countdown ("mm:ss", LoadFont(2)), during a
wave Hud_Bio_Circle with the zeds left (LoadFont(1)) and "Wave N/F"
(LoadFont(5)); size Min(128 x SizeX / 1024, 128), fonts scaled Min(SizeX
/ 1024, 1); hidden while the buy menu is open. `combat.rs`: our debug
line starts hidden (F3 shows it).
**Why:** H3 of the HUD milestone.
**Tested how:** read DrawKFHUDTextElements and where KFGameType sets
GRI.MaxMonsters, WaveNumber and TimeToNextWave. Screenshots at `--window
1280x960` from the matched view through the first countdown and just
after the wave start, cropped next to your two screenshots; one in the
normal window.
**Result:** our clock showed 00:04, 00:03, 00:03, 00:02, 00:01, then
"20 / Wave 1/4" (wave_start zeds=20); "00:02" and "20 / Wave 1/4" match
yours in picture, font, size and place. Tests 123 pass; clippy clean.
**Still broken / not tested:** the boss-wave count includes the
Patriarch's helpers at once (KF only after a kill). The circle during
the boss wave and at the game's end not looked at. Not played by you.
**Next:** H4, KF's messages; or back to the gameplay list.

## 2026-10-05 H4: KF's on-screen messages

**Changed:** `src/hud.rs`: `LocalMessage` (class, switch, text),
HudBase's message list (unique per class, 8 at most, faded by time
left), LayoutMessage's font choice, DrawMessage / RenderComplexMessage;
WaitingMessage, KFMainMessages and KFCriticalEventPlus styles from their
scripts and defaults; KFFonts.KFBase02DS36 / DS24 loaded. Senders:
`game.rs` (next / final wave inbound at 4-1 s left, wave completed after
waves 1-3), `trader.rs` (press E to trade, shop boot; replaces our
HudNote, removed, as is its line in `combat.rs`), `door.rs` (welded
shut on USE; the door hint or the map's own text on touching a trigger,
through a small outbox). 1 test.
**Why:** H4 of the HUD milestone.
**Tested how:** read WaitingMessage, KFMainMessages, KFCriticalEventPlus,
TimerMessage, CriticalEventPlus, LocalMessage, HudBase (LocalizedMessage,
DisplayLocalMessages, DrawMessage, GetScreenCoords), HUDKillingFloor
(LayoutMessage, Message), KFGameType (the countdown and DoWaveEnd).
Screenshot at `--window 1280x960` from the matched view at 00:02,
compared with your first screenshot; logged runs through wave 1 with
`kill_zeds` and `warp_shop`, with screenshots.
**Result:** `hud_message ... NEXT WAVE INBOUND!` at 4, 3, 2, 1 s left;
next to yours: same font, size and place (ours slightly fainter: a
different moment of its 1 s fade; the "!" is missing in both, the font
has no glyph for it). Warping into the closed shop during the wave:
"You can't stay in this shop after closing". After wave 1: "WAVE
COMPLETED! / GET TO THE TRADER!" on two lines; in the open shop "Press
'E' to TRADE". Tests 124 pass; clippy clean.
**Still broken / not tested:** the wave-completed, trade, boot, welded
and door messages are not compared with the real game (no screenshot of
them); the door messages not seen on screen (logs only). End-of-game
text, zed time, pickup messages, announcer not done. You looked at it
in the game and said it looks great.
**Next:** your call: G3b (the Patriarch's entrance), lighting leftovers,
or the end-of-game screen.

## 2026-10-05 Zed time

**Changed:** DESIGN.md: milestone 13, "Zed time". New `src/zed_time.rs`:
`ZedTime` (KFGameType's bZEDTimeActive, CurrentZEDTimeDuration,
LastZedTimeEvent, bSpeedingBackUp, GameSpeed), `DramaticEvent` (the
10 s cooldown, x 2 / x 4 after 30 / 60 s, the roll), Tick (1.1 x real
seconds, easing back over the last 16.6% of 3 s), DoBossDeath (6 s,
forced), the kill rolls (KFGameType.Killed 0.05 within 3 m / 0.025, a
headshot kill 0.03), "ZED TIME ACTIVATED!" once per run; the speed is
Bevy's virtual clock. `zed.rs`: `headshot_kill`, `zed_time_rolled` on
each zed; `combat.rs` marks headshot kills; `projectile.rs`: both blast
callers send the explosion roll (2+ zeds 0.03, 4+ 0.05). Test action
`zed_time`; debug key F2 does the same (you asked for a hotkey). 3
tests.
**Why:** you asked for zed time.
**Tested how:** read KFGameType (Tick, DramaticEvent, Killed,
DoBossDeath), GameInfo.SetGameSpeed, LevelInfo, KFPlayerController
(ClientEnterZedTime, ClientExitZedTime, CheckZEDMessage), KFMonster,
ZombieBoss, the projectiles. Checked nothing gameplay-side reads the
real clock. Logged runs on KF-WestLondon: wave 1 with `zed_time`
forced; a wave of 20 kills (`kill_zeds`, credited to the player).
**Result:** forced: started at real 12.71 s, speed-up at 14.98, end at
15.43: 2.72 real seconds (3 / 1.1 = 2.727), easing 0.45 s (0.453);
game time 12.53 -> 13.25 (0.72 s, as 0.2 x 2.27 + the easing predicts);
the zed log (once a game second) went quiet 3 real seconds. "ZED TIME
ACTIVATED!" shown. Natural run, first seed: 20 rolls, 16 cooldown, 3
failed, the 4th eligible roll started it; it was 0.000 in the log, which
I checked by hand (0.0004: the generator is right). But a fixed seed
means it would start on that same kill every game, and the wave game
uses the very same seed: changed to its own seed. Second run: 9
cooldown, 11 failed, none started, as the odds allow. Tests 127 pass;
clippy clean.
**Still broken / not tested:** the Zedtime_Enter / _Exit sounds (no
sound system). The boss radial attack roll (never solo) and the Husk Gun
/ flare / ZED MKII / Husk fireball multi-kill rolls not done. Perk
extensions (no perks). The boss-death zed time not seen in a run. You
tried it in the game (with F2) and said it feels good. Found, not changed: UE2 runs the whole game at 1.1 x real
time (TimeDilation 1.1 normally); we run at 1.0 (see DESIGN.md, "Zed
time", open question). All randomness here uses fixed seeds, so every
run repeats; KF's FRand differs each game.
**Next:** your call on the two findings; then G3b or the lighting
leftovers.

## 2026-10-05 G3b: the Patriarch's grand entrance, death and laugh views

**Changed:** DESIGN.md: G3b plan. `boss.rs`: Entrance and VictoryLaugh
animations; BossState starts in no state, uncloaked (ZombieBoss has no
auto state), `start_entrance` / `entrance_step` (then InitialSneak,
cloaked), `start_laugh` / `laugh_step`, `shot_anim`; tests updated (two
assumed he spawned sneaking) and 1 added. `zed.rs`: the Patriarch spawns
uncloaked; `BossAction` (Entrance / Laugh) from the wave game, started
like the knockdown (full body, standing); `boss_shot_anim`. New
`view_target.rs`: `ViewTarget` and KF's CalcBehindView (the player's
view rotation; his centre + 12 up, back 9 x his radius, cut by a 10-unit
box cast), drawn by replacing the camera's GlobalTransform after
propagation; the weapon camera off meanwhile. `game.rs`: the boss-wave
tick's MakeGrandEntry and view hand-back, the death view at once,
BossLaughtIt on the first tick after the game ends, view reset on
restart. `weapon.rs`: `WeaponCamera` public.
**Why:** the last item on the handoff list.
**Tested how:** read KFGameType's boss-wave Timer, MatchOver,
BossLaughtIt, ZombieBoss (MakeGrandEntry, MakingEntrance, InitialSneak,
Died, SetBossLaught, SpectatorSpecialCalcView), PlayerController
(CalcBehindView, CameraDist). Logged runs with `--wave 5`: plain, with
`kill_boss`, and without god (the boss kills you); a screenshot during
the entrance.
**Result:** spawned 8.37 s; next tick 9.38 s: `view_target zed0
reason=entrance`, Entrance 5.87 s; 15.24 s end, InitialSneak; next tick
15.37 s view back to the player; his sneak then ended on finding the
player as before. Screenshot: third person behind him mid-entrance,
uncloaked, no first-person gun. `kill_boss`: view on him at once, the
6 s zed time 5.45 real s, game won. Killed by him: game lost at 21.51 s,
next tick 22.17 s VictoryLaugh (5.37 s) with the view on him. Tests 128
pass; clippy clean.
**Still broken / not tested:** BossBattleSong (no sound). A Patriarch
spawned outside the boss wave (debug) no longer starts with the
initial sneak (KF: only MakeGrandEntry leads there). The death and
laugh views not looked at in a screenshot. After a loss he keeps
attacking our debug respawn (as before). Not played by you.
**Next:** your call: the two open questions (1.1 speed, seeds), the
end-of-game screen, or the lighting leftovers.

## 2026-10-05 S1: reading KF's sounds (milestone 10 plan)

**Changed:** `docs/DESIGN.md`: new section "Sound and music (milestone
10)" with what the files hold, how KF plays sounds (PlaySound slots,
radius, pitch, AmbientSound), the music rules (KFMusicTrigger), KF's
audio ini values, the mixer choice and steps S1-S6.
`crates/ue-assets/src/sound.rs` (new): Sound and SoundGroup objects and
a PCM `.wav` decoder (8/16-bit, mono/stereo, any rate, `smpl` loop
points); 2 unit tests. `kfpkg sounds`: reads and decodes every sound in
the install.
**Why:** you asked for sound. This step only reads the data; nothing
plays yet.
**Tested how:** `./target/release/kfpkg sounds` (full output in
`work/sounds-scan.txt`); `cargo test --release --workspace`; clippy.
**Result:** `summary sounds=6687 groups=1630 group_members=7926
bad_members=0 failed=0 looped=149 truncated=0 total_seconds=15265`.
Formats: 6346 mono 16-bit, 165 mono 8-bit, 176 stereo 16-bit; all "WAV".
Tests 130 pass (2 new); clippy clean.
**Still broken / not tested:** no sound output (S2). Group members that
point into other packages are counted, not resolved, by the scan.
**Next:** S2, the mixer (a test action plays one sound, e.g. the 9mm
shot).

## 2026-10-05 S2: the sound mixer

**Changed:** `src/audio.rs` (new): `PlaySound` message (KF's
Actor.PlaySound with its defaults: volume 0.3, radius 300, pitch 1),
`SoundBank` (finds sounds and groups by name in the install, decodes,
caches; weighted random pick with a fixed seed), slots, the 32-voice
limit, distance fade and left/right balance from the player's view,
pitch x game speed (zed time), and the mixer itself on the audio thread.
Test actions `sound:NAME`, `sound_at:NAME@DIST`. `src/main.rs`: `--mute`,
Bevy's own audio plugin switched off. `Cargo.toml`: `rodio` as a direct
dependency (already in the build through Bevy; playback only). README and
DESIGN (S2) updated.
**Why:** step S2 of the sound plan.
**Tested how:** 4 new unit tests (fade, balance, a clip played to its end,
half speed doubles the length). A run: `--map KF-WestLondon --frames 500
--input "200:sound:kf_9mmsnd.9mm_fire,260:sound_at:kf_9mmsnd.9mm_fire@150,320:sound_at:kf_9mmsnd.9mm_fire@600,380:sound:kf_9mmsnd.nosuchsound,400:sound:KF_AK47Snd.AK47_Fire"`.
**Result:** `audio_device ok=true rate=44100 channels=2`; four
`sound_play` lines (group picks of 1.97 s and 2.00 s, distance 0 / 150 /
600), `sound_missing sound=kf_9mmsnd.nosuchsound`, voice count back to 2
after the first shots ended. Tests 134 pass; clippy clean.
**Still broken / not tested:** you listened to the test run (2026-10-05):
loud and clear, centre and right placement and the fade as described. The fade
curve (linear to the radius) and the voice-dropping rule are guesses;
pitch following zed time is assumed. Slot override and the voice limit
are not exercised by a run yet (no game sounds until S3). Sounds of an
entity that is removed fade out at once (KF keeps playing them where it
was). Rolloff=0.5 from the ini is not used.
**Next:** S3, weapon sounds (fire, the first-person stereo versions
`*_FireST`, dry fire, reload, select).

## 2026-10-05 S3a: weapon shots, dry fire, select, full-auto loop

**Changed:** `src/weapon.rs`: each fire mode reads its sounds
(`FireSounds`: FireSound / StereoFireSound / NoAmmoSound with their `Ref`
strings, TransientSoundVolume / Radius, random pitch, KFHighROFFire's
AmbientFireSound, FireEndStereoSound, AmbientFireVolume and radius);
each weapon its SelectSound and TransientSoundVolume. Sounds at
PlayFiring, the dry click (KFWeapon.Fire), the select (KFWeapon.BringUp),
and the full-auto loop and tail (`weapon_loop_sound`). Pitch rolls use
their own random stream, so the seeded spread and damage rolls are
unchanged. `src/audio.rs`: looping voices (wav loop points, else the
whole clip), the `AmbientSound` component, `PreloadSounds`, a per-actor
slot key (`actor`, so each weapon has its own slots), the gain cap at 1.
`kfpkg sounds <file>`: lists a package's sounds with length, peak and RMS
level. DESIGN (S3 split into S3a-c, the two volume guesses), README.
**Why:** S3 of the sound plan; S3 split in three because the
fast-firing guns, reloads and melee each work differently in KF.
**Tested how:** runs with `--give AK47AssaultRifle,MP7MMedicGun` and
scripted fire, and `--give all --frames 400`; 2 new unit tests (gain
cap, loop wrap; the loop test also covers a stall bug found while
reading the code); clippy; tests.
**Result:** 9mm: `9mm_Select volume=100.00`, then `9mm_FireST
volume=1.53` (1.8 x 0.85); the second shot stops the first
(`slot_override`). AK47: `AK47_Select`, a `AK47_FireST` every 0.10-0.12 s
while held, each cutting the last. The 9mm's last shot is no longer cut
by the AK's select (voices=2). MP7: `sound_ambient MP7_FireLoop
volume=255 radius=500` on fire_down, `MP7_tailST volume=2.01` and the loop
stopped 1.2 s later (magazine empty); the next click: `MP7_DryFire
volume=2.00` and the auto-reload. All 44 weapons preloaded: 0 missing,
50 ms in total. Tests 136 pass; clippy clean.
**Still broken / not tested:** you listened (2026-10-05): shots, select,
dry click and the full-auto loop and tail as described; the shotgun's
rack and all reloads are missing (animation sounds, S3b). Guesses: the gain cap
at 1, ambient volume 128 = 1.0. Always first-person sounds (also in the
Patriarch's behind views). Reload, melee swing and hit, chainsaw,
flamethrower, Husk Gun, ZED Gun, welder and syringe sounds are missing
(S3b, S3c). Weapon pickups (S5).
**Next:** S3b, the reload and other animation sounds.

## 2026-10-05 S3b: weapon animation sounds (reloads, shotgun rack, swings)

**Changed:** `crates/ue-assets/src/skeletal.rs`: notifies carry their
sound (`NotifySound`: path, Volume default 1, Radius default 0,
bAttenuate), from KFWeaponSoundNotify / CustomSoundNotify /
AnimNotify_Sound objects; `kfpkg notifies` prints it. `src/weapon.rs`:
`anim_sounds` plays the sound notifies the weapon animation passes each
frame (`notify_frame`, reset to -1 by `play`; wraps on loops); the
animation sounds are added to the weapon's preload. `src/skinned.rs`:
`all_notify_sounds`. DESIGN and README updated.
**Why:** you heard no reloads and no shotgun rack. Both are timed sounds
inside the weapon animations (the rack: two notifies in the shotgun's
Fire animation at 0.346 and 0.538).
**Tested how:** runs with scripted fire and reload on the 9mm and the
pump shotgun; `--give all --frames 400` for the preload; clippy; tests.
**Result:** 9mm Reload (2.00 s): `9mm_Single_Reload_000/026/036/049` at
+0.00, +0.85, +1.18, +1.55 s (the notifies at 0.002, 0.433, 0.600, 0.783).
Shotgun: each shot `SG_FireST`, then `SG_Reload` +0.30 s and
`SG_Reload02` +0.48 s (pump back and forward); its reload a shell sound
per shell until full (3 shells, then the animation ends). Preload: 44
weapons, 363 sounds, 0 missing, 93 ms. Melee swings get their whooshes
from the same notifies (Knife Fire at 0.375, Axe 0.297, ...). Tests 136
pass; clippy clean.
**Still broken / not tested:** you listened (2026-10-05): working. Melee hits, chainsaw,
flamethrower, welder, syringe, Husk Gun and ZED Gun sounds (S3c).
**Next:** S3c.

## 2026-10-05 S3c: melee hits, chainsaw, flamethrower, Husk Gun

**Changed:** `src/weapon.rs`: fire modes read MeleeHitSounds /
MeleeHitSoundRefs and MeleeHitVolume, FireStartSound,
AmbientChargeUpSound; AmbientFireSound now also for FlameBurstFire,
ChainsawFire and HuskGunFire; each weapon reads its attachment's
AmbientSound, SoundVolume and SoundRadius. `weapon_loop_sound` rewritten
as one state machine for the weapon attachment's AmbientSound (fire
loops, the chainsaw's start/end waits, the Husk Gun's charge, the idle
sound). Pending swings carry their hit sounds. `src/combat.rs`:
`MeleeSwing` carries the hit sounds (no longer Copy); `resolve_swings`
plays one per zed hit. `src/audio.rs`: `SoundBank::duration`
(GetSoundDuration). DESIGN (S3c done, S3d planned), README.
**Why:** S3c of the sound plan.
**Tested how:** scripted runs: knife on a Clot (`--spawn clot`), Husk
Gun charge and release, chainsaw idle / hold / release, flamethrower
hold / release; `--give all --frames 400`; clippy; tests.
**Result:** knife: `Knife_HitFlesh volume=1.00` at each of the hits
(after fixing my property name: I first read `MeleeHitSound`, KF's is
`MeleeHitSounds`, so hits were silent). Husk Gun: `ChargeUp` loop from
the charge start, `HuskGun_FireST pitch=0.98` and the loop stopped at the
release (hold 1.65 s). Chainsaw: `Chainsaw_Idle1 volume=230` from the
select; fire_down: `RevLong_Start` (0.64 s), the `RevLong_Loop` 0.65 s
later; fire_up: `RevLong_End` (1.05 s), the idle again 1.05 s later.
Flamethrower: `FireLoop` while held, `FT_Fire1Shot volume=2.01` at the
release. 44 weapons, 377 sounds preloaded, 0 missing. Tests 136 pass;
clippy clean.
**Still broken / not tested:** you said to go on (2026-10-05); whether you listened to
these is not recorded. The chainsaw hitting a
zed (its hit sound) not run. The Husk Gun's full-charge loop not run
(held under 3 s). S3d: ZED Gun beam and alarm, grenade and pipe-bomb
throw sounds, the flamethrower's empty click.
**Next:** S3d.

## 2026-10-05 S3d: grenade, pipe bomb, ZED Gun beam, flamethrower click

**Changed:** `src/weapon.rs`: the frag throw plays the Frag's
FireMode[0].FireSound at the toss start and its ThrowSound when the
grenade leaves the hand (Frag.StartThrow / ServerThrow); the pipe bomb
plays Axe_Fire when placed (PipeBombFire.Timer); the ZED Gun's beam
gets its charge-up and charge loops (MaxChargeTime 1) and the spin-down
at its end (ZEDGunAltFire.PlayFireEnd); the flamethrower clicks while
held empty (FlameBurstFire.AllowFire). The Husk Gun's full charge now
uses MaxChargeTime from its fire class (3, as before). DESIGN, README.
**Why:** S3d, the last weapon sounds.
**Tested how:** scripted runs: `nade`; ZED Gun altfire held 2.5 s;
PipeBombExplosive fire; flamethrower held until empty; `--give all
--frames 400`; clippy; tests.
**Result:** frag: `Axe_Fire` then `Nade_Throw` 0.22 s later (slot
override). ZED Gun: `ZedGunChargeUp` at the beam start,
`ZedGunChargeLoop` 0.98 s later, `KF_WEP_ZED_Secondary_SpinDown_S` at
the release (its animation also has a SpinUp notify, which plays). Pipe
bomb: `Axe_Fire` 1.1 s after the click (also its animation's
dryfire_rifle notify). Flamethrower: `FireLoop` until empty,
`FT_Fire1Shot`, then `FT_DryFire` every 0.08 s while held. 44 weapons,
380 sounds, 0 missing. Tests 136 pass; clippy clean.
**Still broken / not tested:** you said to keep going (2026-10-05); whether
you listened is not recorded. The ZED Gun's alarm
(no motion detector). The empty flamethrower's clicks come every
FireRate (0.08 s), each cutting the last: what the script says, may
sound like a rattle; to compare with the real game.
**Next:** S4, zeds and the player.

## 2026-10-05 S4a: zed animation sounds; the distance fade changed

**Changed:** `src/zed.rs`: each zed tracks how far each animation layer's
sound notifies have played (`sounds_heard`, `overlay_sounds_heard`;
`passed_sounds` handles new sequences, wraps and restarts) and plays the
AnimNotify_Sound notifies on itself (SLOT_None, radius 0 -> 300: guesses,
native code); each zed class's notify sounds are preloaded.
`src/audio.rs`: the distance fade is now OpenAL's inverse distance
clamped model (radius = reference distance, Rolloff 0.5 from the ini),
and voices too quiet to hear (gain under 0.002) are skipped instead of
"out of range". `src/weapon.rs`: a `PendingSwing` type alias (clippy).
DESIGN (S4 plan in three parts from the research in
`work/s4-research.md`; the fade paragraph), README.
**Why:** S4a. The fade: KF's zed sounds have small radii (footsteps 100,
moans 250, swishes 100); with S2's linear fade to silence at the radius
a moan 6 m away would be silent, unlike the real game. The ini's
Rolloff=0.5 is an OpenAL inverse-distance setting. Still a guess.
**Tested how:** `--spawn clot` and `--spawn siren` runs (900-1100
frames); 2 audio unit tests rewritten for the new fade; clippy (counted
from the raw output); tests.
**Result:** Clot: `Clot_StepDefault volume=0.25 radius=100` from 293
units away as it walked in, then `Clot_Swish` (21), `Clot_Attack` (10),
`Clot_Idle1Shot` (7) at 68 units while it attacked. Siren:
`KFPlayerSound.SirenScream1 volume=255.00 radius=150` (capped by the
mixer), `Siren_BitePlayer`, footsteps. All 10 zed classes' notify sounds
preloaded, 0 missing. Tests 136 pass; clippy 0 warnings.
**Correction:** the S3c and S3d entries say "clippy clean". That was
wrong: a `type_complexity` warning from S3c was there; the rtk wrapper's
filtered output and my `tail -1` hid it. Fixed in this step.
**Still broken / not tested:** you played a wave and said it works
(2026-10-05). The new fade changes
how everything sounds at a distance, also the weapons and the S2 test
shots (`sound_at:...@600` is now louder than before). AnimNotify_Sound's
slot and radius handling are guesses. Zed voices and loops: S4b.
**Next:** S4b.

## 2026-10-05 S4b: zed voices and loops

**Changed:** `src/zed.rs`: `ZedSounds` per class (MoanVoice, HitSound[0],
DeathSound[0] / Bloat_DeathPop, HeadlessDeathSound, DecapitationSound,
ChallengeSound[0..3], MeleeAttackHitSound, AmbientSound with
SoundVolume and SoundRadius x AmbientSoundScaling, the Scrake's
SawAttackLoopSound and ChainSawOffSound); `ZedSound` events pushed by
remove_head, take_hit (pain, 0.5 s gate, not for fire damage except
Fleshpound / Scrake / Patriarch), the death timer (0.2 s), the claw
and Patriarch melee hits, the moan timer (KFMonsterController MoanTime,
whole seconds) and a challenge check every 0.5 s; `play_zed_sounds`
plays them and keeps the zed's AmbientSound in step (off when dead or
headless, the Scrake's saw loop while sawing); the voice sounds are
preloaded. `src/combat.rs`: the fire flag for the pain rule and
Impact_Skull on headshots that leave the head on. `src/weapon.rs`:
`sound_prop` crate-visible. DESIGN (S4b done, S4b2 planned), README.
**Why:** S4b of the sound plan.
**Tested how:** `--spawn clot` runs: one with shots from frame 700, one
of 3200 frames (shots from 2600) for the moan timer; `--spawn scrake`;
clippy (counted, 0 warnings); tests.
**Result:** Clot: `Clot_Idle1Loop volume=50 radius=546` at spawn;
`Clot_Challenge` at first sight, then every 7.1 s while it saw me;
`Clot_HitPlayer volume=2.00` for each landed swipe; `Clot_Pain` per hit
(0.5 s apart at most); first `Clot_Talk` (moan) at 37.75 s, the next 13 s
later; at death the loop stopped and `Clot_Death` played 0.17 s later.
Scrake: `Scrake_Chainsaw_Idle volume=175 radius=683`, swapped to
`Scrake_Chainsaw_Impale` when sawing started, `Scrake_Chainsaw_HitPlayer`
every ~0.5 s. All 10 zed classes' voice and notify sounds preloaded, 0
missing. Tests 136 pass; clippy 0 warnings.
**Bugs found on the way:** the death sound and the loop's stop were
missing at first (the ragdoll branch skipped my code; moved before it);
the class's "pain from fire" flag went to the test-only constructor
(swapped); an `allow(clippy::too_many_arguments)` ended up on the wrong
function (moved back).
**Still broken / not tested:** you confirmed it working (2026-10-05). Decapitation and
Impact_Skull not triggered in a test run. Guesses: the ambient radius
(SoundRadius x AmbientSoundScaling), the challenge sight interval, the
pain sound checked before the hit-reaction rules. S4b2: the Patriarch's
own sounds, ragdoll bumps, Husk fireball, Bloat puddle, landing, gibs.
**Next:** S4b2 or S4c (the player), your choice.

## 2026-10-05 S4c: the player's sounds, zed time sounds

**Changed:** `src/player_sound.rs` (new): pain (xPawn.PlayTakeHit:
Inf_Player.playerhurt.Wounding, SLOT_Pain, 0.6, radius 200, 0.35 s apart
and 0.1 s after the last pain), death 0.2 s after dying (one of five
Inf_Player.playerdeath sounds, 0.75, radius 500), low-health breathing
(KFHumanPawn.Timer every 1.5 s under 25 health, ((50 - Health) / 5) x
0.3), jump (JumpDirt, SLOT_Pain, 0.5, radius 80), landing (LandDefault,
SLOT_Interact, min(1, -0.3 x Vz / 325)), footsteps (KFPawn.CheckBob's
step count, Player_StepDefault, 0.45, radius 125). `src/combat.rs`:
sends Hurt (not in god mode, as KF) and Died. `src/zed_time.rs`:
Zedtime_Enter and _Exit (SLOT_Talk, 2.0, radius 500, pitch 1 / game
speed). `src/walk.rs`: test action `jump` (the frame counter and the
scripted input share one system parameter: Bevy's limit of 16).
`src/main.rs`: the plugin. DESIGN, README.
**Why:** S4c of the sound plan.
**Tested how:** `--autowalk 2 --spawn clot` without god, `zed_time` at
frame 300; `--autowalk 4`; `--walk` with two `jump`s; clippy (counted);
tests.
**Result:** a pain grunt per Clot hit about every 1.2 s; breathing from
health 24 at volume 1.5, rising to 2.4; died at 39.756 s,
`playerdeath.Headshot` at 39.990 s; `Zedtime_Enter pitch=5.00` at the
start (speed 0.2), `Zedtime_Exit pitch=4.63` at the speed-up. Footsteps
every 0.367-0.384 s while walking at 198 (each cuts the last one's tail,
SLOT_Interact: KF's slot rule). Jumps: `JumpDirt` at the jump,
`LandDefault volume=0.28` on landing (fall speed 308). Tests 136 pass;
clippy 0 warnings.
**Still broken / not tested:** you confirmed it working (2026-10-05). Surfaces: always the
default step, jump and land sounds. No quiet steps (no crouch or walk
key). The headshot-on-player sound (impact_metal09) and the player's
decapitation are not done (zeds do not aim for the head).
**Next:** S4b2 (Patriarch and zed leftovers), then S5 (the world).

## 2026-10-05 S4b2: the Patriarch's sounds and the zed leftovers

**Changed:** `src/zed.rs`: the Patriarch's speech from the
AnimNotify_Script functions his animations pass (PatriarchEntrance,
KnockDown, Victory, MGPreFire, MisslePreFire: SLOT_Misc, 2.0, bNoOverride,
radius 500 / 1000), Kev_SaveMe when a knockdown ends, RocketFireSound at
the launch, MeleeImpaleHitSound for MeleeImpale hits, the chaingun's
AmbientSound (MiniGunFireSound 255 / 400 while shooting, MiniGunSpinSound
185 / 200 in the pauses); zeds landing (Player_LandDirt, min(1, 0.3 x
fall speed / JumpZ)); `passed_notifies` replaces `passed_sounds`.
`src/boss.rs`: `MgSound` in the chaingun state. `src/fireball.rs`: the
Husk fireball and the Patriarch's rocket fly with their AmbientSound
(husk_fireball_loop, Rocket_Propel) and play their ExplosionSound
(Husk_FireImpact, Rocket_Explode) at 2.0; preloaded. `src/vomit.rs`:
Bloat_AcidSplash where a glob lands (Actor defaults 0.3 / 300).
`src/decals.rs`: ragdoll impacts play Zomb_BodyImpact (every 0.25 s per
corpse at most, min(2, v^2 / 40000); KF's early return for nearby
impacts kept). DESIGN, README.
**Why:** S4b2.
**Tested how:** `--mode waves --wave 5 --god` (3600-4200 frames; one run
with `hurt_zeds` every 40 frames for the knockdowns); `--spawn husk`;
`--spawn bloat`; `zed_drop` and a Clot shot dead; clippy (counted);
tests.
**Result:** Patriarch: `Kev_Entrance` at the entrance, `Kev_WarnGun`,
`Kev_MG_GunfireLoop` / `Kev_MG_TurbineFireLoop` alternating with the
bursts, `Kev_WarnRocket`, `Kev_FireRocket`, `Kev_HitPlayer_Impale` (two
per impale, the animation's two hit notifies), `Kev_KnockedDown` then
`Kev_SaveMe` 2 s later; at the second knockdown `Kev_KnockedDown` was
skipped (slot_busy: Kev_SaveMe still playing; bNoOverride, as KF).
Husk: `husk_fireball_loop` while flying, `Husk_FireImpact volume=2.00`
on hitting me. Bloat: `Bloat_AcidSplash` as the vomit landed. Dropped
Clot: `Player_LandDirt volume=0.55`. Dead Clot: `Zomb_BodyImpact` 0.73,
then 0.02, then one too quiet to start. Tests 136 pass; clippy 0
warnings.
**Still broken / not tested:** you confirmed it working (2026-10-05). Gibbed deaths (none in
our game). The radial-attack taunt (needs 3 players). LAWProj's
AmbientVolumeScale (5) is not used (its meaning is native code).
**Next:** S5, the world.

## 2026-10-05 S6: music

**Changed:** `src/music.rs` (new): reads the map's KFMusicTrigger
(Song, CombatSong, WaveBasedSongs, fade times), turns cues into songs as
KFGameType.StartGameMusic, and fades as KFMusicInteraction. `src/audio.rs`:
`play_music` streams an `.ogg` through rodio (feature `lewton`, already
compiled through Bevy: no new libraries) with a volume knob and stop
switch (`MusicHandle`); SoundVolume and MusicVolume from the install's
KillingFloor.ini (read only). `src/game.rs`: MusicPlaying /
CalmMusicPlaying and the cues (combat at the wave timer, calm at the
countdown, the boss song at his entrance). `src/main.rs`, `Cargo.toml`.
DESIGN, README. One unit test (song choice).
**Why:** S6. You asked me (2026-10-05) to go through S5 and S6 without
waiting for your checks, committing along the way; S6 went first while
the S5 research ran.
**Tested how:** `--mode waves` on KF-Crash and KF-WestLondon with
`next_wave`; `--wave 5` on KF-WestLondon; a survey of every map's
KFMusicTrigger; clippy (counted); tests.
**Result:** `audio_volumes sound=0.3 music=0.1`. KF-Crash: trader time
`KF_Insect` (wave 1's calm song), the wave `KF_Pathogen` (its combat
song), switched at once (no fade times on any map). KF-WestLondon:
`KF_Mutagen`, then `music_missing song=KFSIN8` in the wave. Boss:
`cue=Boss song=KF_Abandon fade_in=1 fade_out=1`, KF_Mutagen stopped
0.9 s later (under 0.1 of its volume), KF_Abandon fading in. Tests 137
pass; clippy 0 warnings.
**Still broken / not tested:** not heard by you (no listening check on
the decoding: the log only shows the stream started). The fade-in shape
is assumed linear. Whether the real game also stays silent in waves on
maps with missing combat songs is not known (it would with UE2's
PlayMusic as far as the scripts show).
**Next:** S5 (the research is running).

## 2026-10-05 Test flag --no-vsync

**Changed:** `src/main.rs`: `--no-vsync` (PresentMode::AutoNoVsync).
README test-command list.
**Why:** while the display could not show the window (you were away;
probably a locked screen), every frame waited about 1 s for it, so
scripted test runs crawled at 1-2 fps (also with the previous commit,
checked). Found during S5a.
**Tested how:** `--map KF-Farm --walk --god --frames 300` with and
without the flag, the screen in that state.
**Result:** without: 1-2 fps (`frame_ms=1000`); with: 243 fps.
**Still broken / not tested:** nothing else changes; normal play keeps
vsync.
**Next:** S5a.

## 2026-10-05 S5a: the map's sounds

**Changed:** `src/map_sound.rs` (new): AmbientSound actors and other
actors with an AmbientSound become looping sounds at their location;
SoundEmitters play random one-shots; ScriptedTrigger sounds waiting for
the player start's event play at spawn; all preloaded. `src/audio.rs`:
`Falloff::Radius` for map ambients, `AmbientSound.scale` (the ini's
[Engine.AmbientSound] AmbientVolume, read from the install), the mixer
skips silent voices (2 new unit tests). Other AmbientSound users get
`..default()`. `src/main.rs`: the plugin. DESIGN (S5 parts), README.
**Why:** S5a; the S5 research report is in `work/s5-research.md`.
**Tested how:** KF-WestLondon, KF-Farm and KF-Manor, 4000 frames each
with `--no-vsync`; clippy (counted); tests.
**Result:** WestLondon `map_sounds loops=25 random=0
silent_ambient_actors=19`, Merlin_Takeoff at spawn; Farm loops=119
random=5, Merlin_Takeoff, `owl1` and `Kessel_Metal_Moan03` played; Manor
loops=33 random=4. 0 missing sounds. 242-274 fps with all loops running.
Tests 139 pass; clippy 0 warnings.
**Still broken / not tested:** not heard by you. Guesses: the radius
falloff and bFullVolume, the one-shots' volume and radius, bAttenuate
false = at the listener, SoundPitch 0. SoundOcclusion (BSP /
StaticMeshes on 874 loops) is not done: sounds pass through walls.
**Next:** S5b, doors.

## 2026-10-05 S5b: door sounds; tiny player landings filtered

**Changed:** `crates/ue-assets/src/level.rs`: DoorInfo reads the Mover
sounds (OpeningSound, OpenedSound, ClosingSound, ClosedSound,
MoveAmbientSound) and SoundVolume / SoundRadius / SoundPitch.
`src/door.rs`: doors collect sound events where Mover / KFDoorMover play
them (`DoorSound`), `door_sounds` plays them and keeps MoveAmbientSound on
while a door moves; zed hits on welded doors (0.5 s apart), breaking,
and the welder's WelderFire per weld tick. `src/player_sound.rs`:
landings slower than 100 units/s are skipped (ours: the walker's
one-frame falls on steps flooded the landing sound and cut the real one
off). DESIGN, README.
**Why:** S5b. The landing filter: found in the S5b test runs (33
landings while standing; the first real landing cut after 0.04 s).
**Tested how:** KF-WestLondon with `--camera` at the church and the
station doors' triggers, `use` twice, `break_doors`; clippy (counted);
tests.
**Result:** station doors: `PatchSounds.WoodDoorOpen volume=0.89
radius=64` at the open, `WoodDoorShut` at the close (a silent
placeholder in KF's data); the church doors have no sounds (as in the
map). `break_doors`: 13 `Door_Break_Wood`, 3 `Door_Break_Metal`.
Landings after the filter: 9, all real falls (fall speeds 117-1925).
Tests 139 pass; clippy 0 warnings. (A test-only DoorInfo needed the new
fields: clippy does not build tests, the test run caught it.)
**Still broken / not tested:** not heard by you. Zed hits on welded
doors and the welder sound not run in a test. Key-door unlock (no keys).
**Next:** S5c, explosions and projectiles.

## 2026-10-05 S5c: explosions and projectiles

**Changed:** `src/projectile.rs`: `ProjectileSounds` in ExplosiveStats
and ThrownStats, `flight` in DartStats; flight loops on rockets, Husk Gun
fireballs and darts; explosions (random pick), duds, grenade / pipe bomb
bounces, pipe bomb beeps (idle and countdown). `src/weapon.rs`:
`projectile_sounds` reads them from the projectile class (and the Husk
Gun's per-class ExplosionSoundVolume); `sound_array` crate-visible.
DESIGN, README.
**Why:** S5c.
**Tested how:** `--give LAW,M79GrenadeLauncher,PipeBombExplosive`
(M79 fired, LAW aimed and fired, pipe bomb placed), a frag (`nade`)
with and without a zed in front; tests; clippy (counted).
**Result:** M79: `Nade_Explode_1 volume=2.00 radius=500`. LAW:
`Rocket_Propel volume=255 radius=250` while flying, `Rocket_Explode`
at 2.0. Pipe bomb: `Nade_HitSurf` on landing, then `Keypad_beep01
volume=0.50 radius=50` every second once armed. Frag: three
`Nade_HitSurf` bounces, `Nade_Explode_1` after the fuse (a frag that
hits a zed drops without a fast bounce: no bounce sound, as KF's
ProcessTouch). Tests 139 pass; clippy 0 warnings (with `--all-targets`
there is one old warning in boss.rs test code, not from this work).
**Still broken / not tested:** not heard by you. The pipe bomb's
countdown beeps and the Husk Gun's explosion volumes not run. Flame
fire loops, the Siren's disintegrate sound: not done.
**Next:** S5d, bullet impacts.
