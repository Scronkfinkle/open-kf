# Handoff (2026-10-06)

Where the work stands, for picking it up on another machine. The full
history is in `MODLOG.md`; the plans and rules are in `docs/DESIGN.md`
("Game loop: waves, trader, door respawns", milestone 9).

## Done this session (all play-tested by you unless noted)

| Commit | Step |
| --- | --- |
| 8d6f3f3, 4cdf288, a9ec2b7 | Baked lighting L1, L2, LV (vision overlay); window glass; KF-Farm black fix |
| bf3e977 | G3a: Patriarch wave rules (helper squads on knockdown, zeds stop on his death, KF's boss spawn search) |
| f6a9c25 | T1: dosh (start 250, kill rewards x 1.75 on Short, team pot at wave end, death penalty, HUD) |
| 0fdf9a6 | T2a: shops and trader doors (pick, open/close with the waves, boot to teleporters, "TRADER: Nm") |
| (this commit) | T3a: buy menu (weapons, selling, ammo, duals, weight) |

## Second session (2026-10-05, new machine)

Setup on the new machine: rebuilt `kfpkg` and re-exported the scripts
(`work/scripts/`) and the lightmap pages (`work/lighting/`).

| Commit | Step |
| --- | --- |
| 7e1987f | T3b: armour (Kevlar vest, KF's ShieldAbsorb, HUD, menu row) |
| 8ddfecd | D5: doors respawn at wave end |
| ae08869 | T2b-1: the trail to the trader (red whisps) |
| 2bd1d1c | T2b-2: the HUD arrow (size fitted to a real-game screenshot); `--window WxH` |
| c8212b1 | H1: KF's HUD bottom bar and cash |
| 33fb842 | H2: KF's bitmap fonts and the HUD texts |
| d87adcc | H3: the top-right circle (countdown, zeds left, wave) |
| bb3af0e | H4: KF's on-screen messages |
| 8f39d26 | Zed time (F2 forces it) |
| 263eb52 | G3b: the Patriarch's entrance, death and laugh views |

Real-game reference screenshots (1280 x 960, KF-WestLondon spawn) are in
`references/` (untracked). To compare: `--window 1280x960 --camera
-3110,1313,-3768,-3.72,0` matches their view.

### Decisions waiting on you

1. **KF's 1.1x game speed.** UE2 runs the whole game at 1.1 x real time
   (TimeDilation 1.1 normally): animations, movement, fire rates, timers.
   We run at 1.0. Match it everywhere? (It touches everything; zed time
   is unaffected either way.) See DESIGN.md, "Zed time", open question.
2. **Random seeds.** All randomness uses fixed seeds, so every run
   repeats exactly (good for tests); KF rolls differently each game.
   Random seeds for normal play, fixed for test runs?
3. **What next:** the end-of-game screen ("Your squad survived!" /
   "Squad eliminated."), or the lighting leftovers (item 5 below).

## Third session (2026-10-05/06): sound and music (milestone 10)

Plan and rules: DESIGN.md, "Sound and music (milestone 10)". Research
notes with KF script quotes (untracked): `work/s4-research.md`,
`work/s5-research.md`.

| Commit | Step | Checked by you |
| --- | --- | --- |
| 80b8399 | S1: reading KF's sounds (`kfpkg sounds`) | yes |
| 22ee340 | S2: our mixer (rodio), `--mute` | yes |
| f81da8e, 5ac3fe5, f5ed2f2, 0c563a4 | S3a-d: all weapon sounds | yes (S3c/d: you said go on) |
| f320d66 | S4a: zed animation sounds; inverse-distance fade | yes |
| b8d1d98 | S4b: zed voices and loops | yes |
| d8ab251 | S4c: the player's sounds, zed time sounds | yes |
| 9ee0c0c | S4b2: Patriarch, fireballs, vomit, corpses, landings | yes |
| 67d4cec | S6: music | **no** |
| be4dfe9 | `--no-vsync` test flag | (tooling) |
| 62f291b | S5a: the map's ambient sounds, helicopter | **no** |
| 339b480 | S5b: doors, welding; tiny landings filtered | **no** |
| e532813 | S5c: explosions and projectiles | **no** |
| 349f320 | S5d: bullet impacts, glass, bolts, nails | **no** |
| 7ebfa87 | S5e: trader lines and shop sounds | **no** |

### Your listening test for S5 and S6

`cargo run --release -- --mode waves --length short --walk` on
KF-WestLondon (and once on KF-Farm), then:

1. At the start: the helicopter taking off; the map's fires, engines and
   buzzing lights as you walk past them (they are silent beyond their
   own small radius: a guess, tell me if they carry too little or too
   far). KF-Farm: owls and creaking lamps now and then.
2. Music: KF-WestLondon plays KF_Wading in trader time and
   DirgeDisunion1 in wave 1 (the songs come from the map's .int file, as
   in the real game). `--wave 5`: KF_Abandon fades in at the Patriarch's
   entrance. Volumes come from your KillingFloor.ini (music 0.1).
3. Trader: the radio beep then "the shop is moving" at 20% of a wave,
   "almost open" at 80%, "shop's open" when the wave ends, "30 seconds",
   "10 seconds" (only in the shop), "closed" at the next wave. Buying
   plays the weapon's pickup sound; too expensive / too heavy talk back.
4. Doors (E on a door's trigger): opening and closing (many wooden doors
   close silently: KF's own sound file is a silent placeholder), zeds
   banging on welded doors, a door breaking, the welder's sparks.
5. Explosions: frag bounces and blast, LAW hiss and explosion, M79,
   pipe bomb beeping once armed.
6. Bullets hitting walls (all sound like dirt until surfaces are read),
   glass cracking and breaking, crossbow bolts.

Known guesses to judge by ear are listed in DESIGN.md (distance fades,
map ambient radius, the volume cap). Test runs while the window cannot
be shown (locked screen) need `--no-vsync`, or they run at 1 fps.

## What to do next (your order: waves -> trader -> door respawns)

1. ~~T3b, armour~~ (done on the new machine, 2026-10-05; see MODLOG).
2. ~~D5, doors respawn at wave end~~ (done on the new machine, 2026-10-05).
3. ~~T2b, the trail and the HUD arrow~~ (done on the new machine,
   2026-10-05).
4. ~~G3b, the grand entrance~~ (done on the new machine, 2026-10-05;
   no boss music: no sound system yet).
5. Lighting leftovers (deferred): BSP ~20% dark, sky layer order, terrain
   (L3), actors (L4), 3 maps with empty lightmap pages (L1b).

## Test commands

- Wave game: `cargo run --release -- --map KF-WestLondon --mode waves --length short --walk`
  (add `--god`; `--wave 5` starts at the Patriarch).
- Scripted shopping (no keyboard needed): `--input` with `next_wave`,
  `kill_zeds`, `warp_shop` (stand in the current shop), `add_dosh`,
  `buy_menu`, `buy:Shotgun`, `sell:Shotgun`, `ammo_clip:Shotgun`,
  `ammo_fill:Shotgun`, `ammo_clip2:M4203AssaultRifle`, `kill_boss`,
  `hurt_zeds`. Example in MODLOG's T3a entry.
- Checks: `cargo test --release --workspace` (103 pass),
  `cargo clippy --release --workspace` (clean).

## Working notes (from the assistant's memory on the old machine)

- Copy behaviour from KF's own scripts and class defaults, quote the
  rule, and label guesses. Scripts are exported to `work/scripts/` by
  `kfpkg scripts` (gitignored: re-export on the new machine; build the
  tool with `cargo build --release -p ue-assets --bin kfpkg`).
  `kfpkg defaults Package.Class` shows a class's own defaults only (look
  at the parent class for inherited values).
- Screenshots are allowed for visual checks (`--screenshot N`, saved in
  `work/screenshots/`, the run quits after the last one). Logs stay the
  main check (`logs/latest.log`).
- After any global rendering change, screenshot all maps and check
  nothing turned black (KF-Farm went black once).
- Odd zed behaviour at a spot: check the map's actors first
  (`map_features` log, `docs/map-audit.md`) before blaming the AI.
- For transient visuals, check the logs, then ask you to look.
- Shell quirks: in zsh, write `${f}:hurt_zeds`, not `$f:hurt_zeds`
  (`:h` is a zsh modifier). The `rtk` wrapper can mangle grep output: use
  `rtk proxy grep` or python, and judge tests and clippy by their summary
  lines.
