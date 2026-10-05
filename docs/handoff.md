# Handoff (2026-10-05)

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

## What to do next (your order: waves -> trader -> door respawns)

1. ~~T3b, armour~~ (done on the new machine, 2026-10-05; see MODLOG).
2. ~~D5, doors respawn at wave end~~ (done on the new machine, 2026-10-05).
3. **T2b, the trail.** T2b-1 (the whisp trail) done on the new machine,
   2026-10-05. T2b-2, the HUD 3D arrow (KFShopDirectionPointer), still
   to do.
4. **G3b, the grand entrance** (deferred until the trader core is in):
   camera on the Patriarch during Entrance, BossBattleSong, death camera.
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
