# kf-rs

A from-scratch Rust + Bevy reimplementation of Killing Floor (2009), reading the
original game's files from an installed copy. Single-player, offline. No game
assets are included in this repository.

See `docs/DESIGN.md` for how it works and the plan.

## Requirements

- An installed copy of Killing Floor (Steam). Found automatically at
  `references/killing_floor` or the usual Steam folders. Set `KF_ROOT` to
  override.
- Nix with flakes. `direnv allow` or `nix develop` gives you Rust and the system
  libraries Bevy needs. The flake files must be tracked by git (`git add flake.nix
  flake.lock`) or Nix will not see them.

## Running the map viewer

```sh
cargo run --release                        # opens KF-WestLondon
cargo run --release -- --map KF-Offices    # any map name from the game's Maps folder
cargo run --release -- --frames 300        # quits by itself after 300 frames (for test runs)
cargo run --release -- --camera -4090,1300,-3650,-1.5708,0 --screenshot 60   # start at a saved view, screenshot, quit
cargo run --release -- --walk              # start walking instead of flying
cargo run --release -- --autowalk 4        # walk test: hold forward for 4 s (see logs/latest.log "walk" lines)
cargo run --release -- --input 120:fire,200:1 --screenshot 126,330   # scripted input + screenshots (tests)
cargo run --release -- --walk --zed        # a Clot spawns in front of you and comes at you
cargo run --release -- --walk --gorefast   # the same with a Gorefast
cargo run --release -- --walk --zed --always-sever   # test: killing shots on limbs always sever them
```

Saved views for checking the viewer are listed in `docs/test-views.md`.

| Control | Action |
| --- | --- |
| Left click | capture the mouse (needed to look around) |
| Escape | release the mouse |
| Mouse | look |
| W A S D | move |
| Space or E / Ctrl or Q | up / down |
| Shift | move 4x faster |
| F12 | screenshot to `work/screenshots/`, with a `.txt` command that recreates the view |
| V | switch between flying and walking |
| 1 / 2 | knife / 9mm |
| Left click (mouse captured) | fire |
| R | reload (9mm) |
| Right click (mouse captured) | iron sights on / off (9mm) |
| Z | spawn a zed in front of you (more spawn side by side); the HUD shows which |
| N | change what Z spawns (Clot, Gorefast) |
| G | spawn a Gorefast in front of you |
| X | pause / resume zeds |
| Space (walking) | jump |

Each run writes `logs/latest.log`: what was loaded (counts, load time), camera
position once per second (Bevy metres and Unreal units), frame timings, and
exit status.

Package inspection tool:

```sh
cargo run --release -p ue-assets --bin kfpkg -- scan                       # parse every package
cargo run --release -p ue-assets --bin kfpkg -- info Maps/KF-Farm.rom      # objects by class
cargo run --release -p ue-assets --bin kfpkg -- exports Maps/KF-Farm.rom Light
cargo run --release -p ue-assets --bin kfpkg -- props Maps/KF-Farm.rom Light # writes work/props/KF-Farm-Light.txt
cargo run --release -p ue-assets --bin kfpkg -- scanprops                  # read properties of every object
cargo run --release -p ue-assets --bin kfpkg -- scripts                    # UnrealScript source -> work/scripts/
cargo run --release -p ue-assets --bin kfpkg -- textures                   # decode every texture (checks)
cargo run --release -p ue-assets --bin kfpkg -- texture Textures/KillingFloorTextures.utx House2SkinNEW  # -> work/textures/*.png
cargo run --release -p ue-assets --bin kfpkg -- meshes                     # decode every static mesh (checks)
cargo run --release -p ue-assets --bin kfpkg -- level KF-Farm              # load a map like the viewer, report counts
cargo run --release -p ue-assets --bin kfpkg -- bspmaterials KF-WestLondon # per-material BSP report -> logs/
cargo run --release -p ue-assets --bin kfpkg -- zones KF-WestLondon [25]   # BSP zones; with a number, that zone's polygons
cargo run --release -p ue-assets --bin kfpkg -- terrain KF-Farm [X,Y]      # terrain layers, height check; ground height at X,Y
```

## What works (tested 2026-10-03)

- **Map viewer:** every one of the 40 maps opens with its BSP level geometry,
  placed static meshes and textures, and you can fly around it. Tested by load
  logs and by you: text, prop placement, scale and frame rate confirmed
  correct by you on KF-WestLondon. The tunnel fix was confirmed by screenshot.
  Other maps have not been looked at.
- Terrain: heightmap ground with blended texture layers and holes, on all 21
  maps that have terrain (checked by screenshot on KF-Farm, KF-Crash and
  KF-Hell).
- Walking with collision (milestone 2): KF player values (200 units/s,
  jump 325, gravity 950), cylinder collision against level geometry,
  blocking props, terrain and blocking volumes, step-ups, sliding along
  walls. Tested by logged walk runs on KF-Farm and KF-WestLondon, and
  play-tested by you: mechanics and collision feel correct. Pace and jump
  feel much better since the field of view was fixed. Exact feel is not
  confirmed yet (needs weapons for a fair comparison).
- First-person weapons (milestone 3): knife and 9mm with arms, textured and
  animated (select, idle, fire, put down, reload), switched with 1/2. The
  knife gives KF's +40 speed bonus. Placement uses KF's own formulas (view
  offset, display FOV, widescreen rule), as does view and weapon bob while
  walking. Play-tested by you: knife attack, pistol fire, reload and walking
  bob all work.
- Zeds (milestone 4, first part): Clots, textured, lit and animated, walk
  straight at you at their own speed (105), with the same collision as the
  player, and play their grapple attacks in melee range. No damage either way,
  no pathfinding. Checked by logs and screenshots, and play-tested by you
  ("looks great").
- Combat (milestone 5): the 9mm (25-35 damage, 15-round magazine, 105
  spare, reload, dry fire) and knife (19 damage, range 70, hit 0.45 s into
  the swing) damage Clots (130 health). KF's headshot rule: the head sphere
  and head health 25; headshots remove the head. Clots grab for 6 damage. A
  HUD shows health, ammo and kills. You respawn at the start on death.
  Play-tested by you: damage and headshots work.
- Pawn collision: you and the zeds block each other, and zeds block each
  other, as upright cylinders (as in Unreal). Corpses do not block. Checked
  by logged runs (you stop 47 units from a Clot's centre; three Clots never
  get closer than 52 to each other); play-tested by you.
- Decapitation as in KF: a headshot that takes the head off adds the head
  explosion damage; if the Clot survives (one 9mm headshot leaves it at ~31 of
  130) it flinches, walks on headless at 80% speed and bleeds out after 5 s.
  Hits on a headless zed count as headshots. Checked by unit tests, logs and
  screenshots; play-tested by you.
- Clot reactions and attacks from KF's scripts: body hits flinch by direction
  (upper body only, legs keep walking); heavy frontal hits stun for 1 s;
  grabs play on the upper body while the Clot keeps walking, pick attacks at
  random and pin you for 1.5 s (no moving or jumping; flinches break the
  grab); a headless Clot swipes with claws at double damage and reach after a
  2 s daze; falling and landing animations; idle instead of walking on the
  spot when blocked. Checked by unit tests, logs and screenshots; play-tested by you
  (2026-10-04).
- Ragdolls (Clot): dead Clots go limp and fall using KF's own ragdoll data
  (KarmaData/KF_Characters_Trip.ka), pushed along the shot, and lie on the
  ground for 30 s. Corpses do not block you, zeds or bullets. Checked by logs
  (all joints hold within 0.2 units, bodies rest in ~2 s) and screenshots;
  play-tested by you.
- Iron sights (ADS) on the 9mm: right click zooms the view (FOV 90 -> 75)
  over 0.25 s, brings the sights to the screen centre, plays KF's iron idle
  and fire animations, and halves the spread. Reloading or switching drops
  out of it. Checked by screenshots (sights at the screen centre) and logs;
  play-tested by you.
- Gorefast (2026-10-04): loaded from the game data (250 health, walks at
  120, hits for 15, its own ragdoll) with the shared flinch, headshot,
  bleed-out and ragdoll rules. Within 700 units it runs at 225, and 20% of
  its attacks while running are swings on the move. Checked by unit tests,
  logs and screenshots. Not play-tested by you.
- Gore, step A (2026-10-04): a gun headshot that takes the head off leaves
  KF's neck stump (also on the ragdoll) and throws three brain chunks that
  bounce and vanish after 6 s. Checked by logs and screenshots;
  play-tested by you.
- Gore, step B (2026-10-04): a killing shot to a thigh or forearm can take
  that limb off (KF's chance rule), throwing the zed's own severed piece and
  leaving a stump; a knife decapitation sends the head flying. Checked by
  logs and screenshots; play-tested by you (with `--always-sever`).
- Gore, step C (2026-10-04, in progress): KF's own particle effects, read
  from the game and simulated: the neck blood plume and flying meat on
  decapitation, BrainSplash, the pulsing limb jets, blood puffs on every
  hit, blood trails behind flying pieces and chunks. Checked by logs and
  screenshots; the decapitation and limb effects play-tested by you.
- Install discovery: finds the KF install and reads its build label.
- Package table reader: all 548 Unreal packages in the install parse
  (`cargo run --release -p ue-assets --bin kfpkg -- scan`). It reads names,
  imports and exports.
- Property reader: the settings of 559,872 of 559,875 objects across the install
  read cleanly. That covers positions, rotations, mesh and texture references,
  and light settings.
- Script source export: all 3757 UnrealScript classes are written to `work/scripts/`
  (gitignored), for studying how the game works.
- Texture decoding: DXT1/3/5, RGBA8, L8 and P8 (when the palette is in the
  same package). All 8535 textures read. Exported PNGs look correct.
- Static meshes: all 5021 decode (positions, normals, UVs, triangles,
  per-section materials) and pass consistency checks.
- Map contents: all 40 maps load. That covers BSP level geometry, placed meshes,
  player starts, materials and textures, with 0 unresolved references.

## What doesn't work yet

- Terrain decoration layers (grass and small meshes scattered on terrain) are
  not drawn.
- Sky zone (3D skybox): drawn on maps that have one, behind the scene and
  turning with the view (added after the tunnel report; not yet seen by a
  person). Maps without a sky zone, such as KF-BioticsLab, show flat grey-blue.
  The black tunnel ceilings and walls on KF-WestLondon really are black in the
  map data (`Engine.BlackTexture`).
- No baked lighting: everything is lit by one fixed sun plus ambient light, so
  maps look flatter and brighter than in the game.
- Animated or complex materials (panners, shaders, combiners) show their base
  texture only, without animation or blending tricks.
- Doors, breakable windows and scripted barriers (movers) do not block
  movement yet, because they don't move yet.
- No crouching. Ladders, water and swimming are not handled.
- No muzzle flash, blood decals (splats on walls and floors), impact effects or sound;  only the two starting
  weapons are available. Only the Clot and the Gorefast exist, and they
  chase in a straight line (no pathfinding), so they can get stuck on walls.
  Emitters and sound: not started.
- Frame rate not measured with the monitor on (during testing the monitor was
  off, which throttles to 1 fps).
- Exit crash: about half of all runs end in a segmentation fault or abort
  *after* the clean shutdown is logged, even with no map loaded (8 of 14 in
  one measurement). Cause unknown. It happens only when quitting.

- Sound is not read yet. Terrain, skyboxes, lightmaps and emitters are not read.
- G16 textures (terrain heightmaps). P8 textures whose palette is in another
  package (64 textures).
- 2 dialogue triggers in KF-Steamland have stale stored property sizes (see
  `docs/DESIGN.md`).
- Gameplay of any kind.

## License

Dual-licensed under MIT (`LICENSE-MIT`) or Apache 2.0 (`LICENSE-APACHE`), at your
option. This covers the source code only, not Killing Floor's game files.
