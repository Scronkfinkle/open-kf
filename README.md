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
cargo run --release -- --walk --spawn fleshpound   # any specimen: clot, gorefast, crawler, stalker, bloat, siren, husk, scrake, fleshpound, patriarch
cargo run --release -- --walk --spawn patriarch --god --input 300:record,600:record   # test: record a 5 s video (scripted F9)
cargo run --release -- --walk --spawn clot --god   # god mode: zeds hit you (logged) but you lose no health
cargo run --release -- --walk --zed --always-sever   # test: killing shots on limbs always sever them
cargo run --release -- --walk --give all             # test: carry every base-game weapon (or --give AK47AssaultRifle,Shotgun)
cargo run --release -- --walk --fps 30               # cap the frame rate (vsync still caps it at the monitor's refresh rate)
cargo run --release -- --walk --zed --zed-at -4512,-230,-3816   # test: the zed starts at a map position (Unreal X,Y,Z)
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
| F9 | start / stop recording a video to `work/videos/` (30 fps, H.264, up to 1280 wide); the window title shows `[REC]` |
| V | switch between flying and walking |
| 1 2 3 4 5 | weapon slots: melee, pistols, primary, specials, equipment; press again to cycle within a slot (KF's rules) |
| Mouse wheel | next / previous weapon |
| Left click (mouse captured) | fire |
| Middle click (mouse captured) | alt fire (KF's key): melee heavy attacks; full / semi auto switch on rifles (HUD shows [AUTO] / [SEMI]) |
| Right click (mouse captured) | iron sights on / off; on the Crossbow and M99 the 3D scope |
| R | reload |
| Z | spawn a zed in front of you (more spawn side by side); the HUD shows which |
| N | change what Z spawns (all ten specimens) |
| G | throw a frag grenade (HUD: FRAGS) |
| H | spawn a Gorefast in front of you |
| X | pause / resume zeds |
| F1 | god mode on / off (HUD shows "(GOD)") |
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
- Weapon inventory (2026-10-04): KF's starting kit (knife, 9mm, frag,
  syringe, welder), any base-game weapon given with `--give`, slot keys
  1-5 and the mouse wheel with KF's ordering and cycling rules, the
  carried weight slowing you as in KF (starting kit: 198 instead of 200).
  Melee alt attacks on middle click (KF's AltFire key), each with its own
  damage, reach, delay and animation (Axe 175 / 275, Knife 19 / 55, ...),
  and KF's rule that one fire mode waits for the other. Pistols, the Lever
  Action, M14, shotguns and launchers fire once per click
  (bWaitForRelease); rifles and SMGs keep firing while held.
- Bullet guns (W2, 2026-10-04): KF's spread (growing during a burst,
  halved when aiming), recoil that kicks the view up, exact fire rates,
  full / semi switch on middle click, firing / loop / end animations,
  reloads timed by ReloadRate (the Lever Action loads round by round and
  firing interrupts it), dry-click reloads, shots slowing you, and KF's
  switch timing (0.33 s down + 0.33 s up). Checked by unit tests and
  logged runs (AK47, M4, 9mm, Lever Action, Bullpup). Play-tested by you
  ("guns feel good").
- Flamethrower (W7b, 2026-10-04): hold to spray flames that arc a little,
  burst after 0.4 s or on a zed or wall, and burn everything within 150
  units, zeds and (close up) you; a 100-round tank, then reload. Checked
  by logged runs (Clots, a Scrake, emptying and reloading). Not
  play-tested by you; the flames not looked at.
- Zeds catch fire (W7a, 2026-10-04): the Dragon's Breath Trenchgun sets
  zeds burning for 10 s with flames on them; each second's burn hurts
  more than the last, burning zeds walk 20% slower, and after 6 s they
  switch to the burning walk. KF's Husk resists fire and the Bloat takes
  extra from it. The MAC10 does not burn: in KF only the Firebug perk
  makes it incendiary. Checked by unit tests and logged runs (Scrake,
  Gorefast, Clots). Not play-tested by you; the flames not looked at.
- Grenades, rockets, frags, pipe bombs and nails are drawn with their own
  models (2026-10-04). Checked by screenshot (pipe bomb).
- Crossbow and M99 (W6c, 2026-10-04): bolts / bullets go through every
  zed in line (losing a fifth each time), with KF's big headshot
  multipliers; Crossbow bolts stick in walls and can be picked back up.
  Checked by logged runs (pickup not tested). Not play-tested by you.
- Frag grenades and pipe bombs (W6b, 2026-10-04): G throws a frag (your
  weapon goes down and comes back up), it bounces and explodes after 2 s;
  pipe bombs are placed and go off when enough zeds come near. Checked by
  logged runs. Not play-tested by you.
- Grenades and rockets (W6a, 2026-10-04): the M79, M32, the M4 203's
  launcher (middle click, own grenade count) and the LAW (aim first) fire
  grenades / rockets that fly, explode with KF's falloff and line-of-sight
  rules, leave scorch marks, and are duds too close to you. The Fleshpound
  takes KF's full damage from explosives. Checked by logged runs. Not
  play-tested by you.
- Melee (W5, 2026-10-04): a swing hits the zed you aim at and every
  other zed in its arc (less the further off-centre), doubles damage from
  behind, and slows you; the Chainsaw cuts continuously while held.
  Checked by logged runs. Backstabs not tested in a run (zeds always
  faced the player). Not play-tested by you.
- Shotguns (W4, 2026-10-04): pellets fly as projectiles with tracers,
  pass through zeds losing damage as in KF, kick you back; the Hunting
  Shotgun fires one or both barrels and reloads itself; the HSG-1
  switches wide / narrow spread; the AA12 is full auto; the Nailgun's
  nails bounce. Checked by unit tests and logged runs. Not play-tested by
  you.
- Pistols (W3, 2026-10-04): Handcannon, 44 Magnum and MK23 bullets go
  through up to 5 zeds at half damage each time; dual pistols fire left
  and right in turn with their own flashes, shells and tracers, and
  replace the single pistol (keeping its ammo). Every base weapon now has
  its muzzle flash and shell effects. Checked by unit tests and logs;
  flashes not checked by eye on every gun.
- Reflex sights (SCAR, M4, Bullpup, 2026-10-04): clear glass with the red
  reticle instead of an opaque speckled disc. Checked by screenshot (M4).
- 3D scopes (2026-10-04): the Crossbow and M99 lenses show a live zoomed
  view with the reticle while aiming, as KF's default scope setting does.
  Checked by screenshots. Not play-tested by you.
  Guns deal KF's DamageMax (the 9mm now 35 every shot; it rolled 25-35
  before). Checked by unit tests, logs and a screenshot of the AK47. Not
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
- Gore, step D (2026-10-04): KF's blood decals: splats on walls and floors
  behind hits, drips under severed limbs, splats under brain chunks,
  streaks where corpses hit the ground. Checked by logs and screenshots. Not
  play-tested by you.
- Pathfinding (2026-10-04): zeds use the map's own navigation network
  (KF's ReachSpecs) and KF's hunting rules to walk around obstacles when
  they cannot walk straight at you, jump obstacles up to ~54 units, keep
  momentum when falling, and re-route when stuck. Zombie-only areas
  (KFZombieZoneVolume) block you but not zeds, as in KF; invisible blocking
  volumes no longer stop bullets. Jump pads and KF's JumpSpot jumps are not
  done. Checked by logged runs on KF-WestLondon; first part play-tested by
  you, the rest not yet.
- All ten specimens (2026-10-04, in progress): every KF specimen loads from
  the game data with its own stats, animations, ragdoll and severed parts
  (N picks which one Z spawns). Specials done so far: Crawler pounce,
  Stalker cloak (an approximate see-through look), Scrake chainsaw loop,
  charge and rage, Fleshpound rage (360 damage within 2 s, or 10-15 s
  chasing you without landing a hit, makes him charge at 2.3x speed with
  a red chest light; his first hit ends it, at 1.75x damage; he takes half
  damage from guns), Bloat vomit (within 250 units: KF's vomit stream and
  three globs that splat, leave vomit decals and start a 7-tick bile burn;
  on death he bursts, leaving his legs, unless he bled out headless).
  Siren scream (within 700 units: six pulses of up to 8 damage that pull
  you toward her, KF's red scream effect; she keeps walking at 65% speed
  while attacking; headless she dies at once or within 10 s).
  Husk fireball (shoots from any range he can see you, every 5.5-7.5 s,
  often at your feet: 25 fire damage in a 150-unit blast, knock-back, then
  burning for 12, 6, 3, 1; scorch marks).
  Patriarch, steps 1-2 of 6: claw (75, long reach) and impale (two hits of
  75) while walking, each hit knocks you back; he never loses his head and
  never flinches. Within 700 units he charges (running at 2.5x speed, up to
  6 s, ends on his first landed hit, which knocks you back 1.5x harder),
  about every 5 s. Chaingun: winds up, then 35-94 shots in bursts of about
  1 s (a shot every 0.05 s, 4-6 damage each), standing and turning to
  you; no tracer or muzzle flash drawn yet. Rocket: over 500 units away,
  a 2.4 s wind-up, then a rocket (2600 units/s, smoke trail) that explodes
  for up to 75 in a 500-unit radius with a scorch mark; every 10-25 s.
  Cloak: he arrives invisible until he sees you, and every 20 s or so
  (70%) sneaks up invisible for up to 10 s, running, uncloaking to claw.
  Knockdown and healing: below 3200, 2000 and 1250 health he is knocked
  down, runs off cloaked to a spot you cannot see, and injects a syringe
  for +1000 health (three times at most). Test: add
  `--input 400:hurt_zeds,401:hurt_zeds,...` (100 damage each).
- Firing effects: the 9mm's muzzle flash and ejected shells (with smoke)
  on the gun; tracers and bullet impacts (puff and bullet hole) when a shot
  hits the level. As in KF, a shot that hits a zed or nothing draws no
  tracer. The Patriarch's chaingun has its muzzle flash, tracers and
  impacts. All impacts use the default (rock) effect for now.
  Checked by logs; not looked at by eye yet.
  Checked by logs and screenshots. Not play-tested by you.
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

- **The Patriarch is unfinished.** His six planned steps work in scripted
  runs (melee, charge, chaingun, rocket, cloak and sneak, knockdown /
  escape / heal), but:
  - Not play-tested by you; the cloak, flash and effects are not checked
    by eye.
  - After falling off a ledge while escaping he can stand stuck (shared
    zed movement code); a 30 s give-up (ours, not KF's) makes him heal
    where he is.
  - Missing: the Entrance animation and boss-wave intro, the radial attack
    (needs 3 players), the zeds he summons on knockdown (needs the wave
    system), the syringe prop in his hand, the death camera and victory
    laugh, KF's refraction cloak shader (ours is a see-through stand-in),
    the commando "spotted" glow, voice lines and gun sounds, zed time.
  - Some timings are only unit-tested: the charge's 6 s limit, "over 700
    away ends a charge", "shot from closer than 100 while firing ->
    charge", the third heal.
  - See `docs/DESIGN.md`, "Patriarch", for the details.

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
- No sound.
- Weapons (in progress, see DESIGN "Weapons"): every base-game weapon can be
  carried (`--give`), switched to and fired, but only melee weapons,
  bullet guns and shotguns deal damage. No crouching, so no crouch
  accuracy bonus. Alt fires other than melee, the rifle toggles and the shotguns
  (medic darts, flashlights) do nothing yet. The flamethrower, husk gun,
  syringe, welder and ZED guns play their fire animation and use ammo but
  do nothing. The Trenchgun's fire
  damage is not done (W7). No DLC weapons. No trader.
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
