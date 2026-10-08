# Open KF

A from-scratch Rust + Bevy reimplementation of Killing Floor (2009), reading the
original game's files from an installed copy. It plays single-player, and
has experimental co-op multiplayer (see [MULTIPLAYER.md](MULTIPLAYER.md)).
No game assets are included in this repository.

![Open KF on KF-WestLondon: wave 1, Clots coming out of the tunnel, KF's HUD](docs/images/kf-westlondon.jpg)

*KF-WestLondon in Open KF: wave 1, shotgun in hand, the trader 29 m away.*

See `docs/DESIGN.md` for how it works and the plan.

## Requirements

An installed copy of Killing Floor (Steam). Open KF finds it through Steam
(every Steam library, including the Steam Flatpak and libraries on other
drives). Set `KF_ROOT` to its folder to override. **This will not work if
you don't have the original game files.**

## Playing

Download the latest build from the
[Releases page](https://github.com/Scronkfinkle/open-kf/releases):

- **Windows:** `open-kf-windows-x86_64.zip`. Unzip it and run `open-kf.exe`.
- **Linux:** `open-kf.flatpak`. Install it with
  `flatpak install --user open-kf.flatpak`, then start Open KF from your
  app menu or with `flatpak run io.github.scronkfinkle.OpenKF`. It can read
  the usual Steam folders; for a Steam library elsewhere, run
  `flatpak override --user --filesystem=/path/to/library:ro io.github.scronkfinkle.OpenKF`
  once. Its logs, settings and screenshots are in
  `~/.var/app/io.github.scronkfinkle.OpenKF/data/`.

On either system the launcher opens first. Pick Solo, Host or Join, a map, your name, perk and
character, then press **PLAY**. Your choices are remembered for next time.

![The Open KF launcher: Solo / Host / Join, the map list, perk and character, game and display options](docs/images/launcher.jpg)

*The launcher. The command it will run is shown at the bottom; the same
options can be typed on the command line (below).*

## Building from source

Needs Nix with flakes: `direnv allow` or `nix develop` gives you Rust and the
system libraries Bevy needs. The flake files must be tracked by git
(`git add flake.nix flake.lock`) or Nix will not see them. During
development the game also looks in `references/killing_floor` (a link to
your install) before asking Steam.

```sh
cargo run --release   # opens the launcher, as above
```

## Command-line options

The release builds take the same options: `open-kf.exe --map KF-Offices`, or
`flatpak run io.github.scronkfinkle.OpenKF --map KF-Offices`.

```sh
cargo run --release                        # no options: the launcher (pick map, perk, solo/host/join, then PLAY)
cargo run --release -- --mode debug        # what no options used to do: KF-WestLondon, no waves
cargo run --release -- --map KF-Offices    # any map name from the game's Maps folder
cargo run --release -- --frames 300        # quits by itself after 300 frames (for test runs)
cargo run --release -- --fly --camera -4090,1300,-3650,-1.5708,0 --screenshot 60   # start at a saved view, screenshot, quit
cargo run --release -- --fly               # start flying (the map viewer); V switches between flying and walking
cargo run --release -- --autowalk 4        # walk test: hold forward for 4 s (see logs/latest.log "walk" lines)
cargo run --release -- --fly --input 120:fire,200:1 --screenshot 126,330   # scripted input + screenshots (tests)
cargo run --release -- --zed        # a Clot spawns in front of you and comes at you
cargo run --release -- --gorefast   # the same with a Gorefast
cargo run --release -- --spawn fleshpound   # any specimen: clot, gorefast, crawler, stalker, bloat, siren, husk, scrake, fleshpound, patriarch
cargo run --release -- --spawn patriarch --god --input 300:record,600:record   # test: record a 5 s video (scripted F9)
cargo run --release -- --spawn clot --god   # god mode: zeds hit you (logged) but you lose no health
cargo run --release -- --zed --always-sever   # test: killing shots on limbs always sever them
cargo run --release -- --give all             # test: carry every base-game weapon (or --give AK47AssaultRifle,Shotgun)
cargo run --release -- --input 200:sound:KF_9MMSnd.9mm_Fire,300:sound_at:KF_9MMSnd.9mm_Fire@600   # test: play a sound at you, then 600 units to your right
cargo run --release -- --input 200:jump   # test action: jump (also "jump" in any --input list)
scripts/headless.sh --fly --camera -4090,1300,-3650,-1.5708,0 --screenshot 60   # the same, on a virtual display: no window opens
cargo run --release -- --no-vsync --frames 600   # test runs: frames do not wait for the display (a hidden window ran at 1 fps)
cargo run --release -- --mute                 # no sound (still logged)
cargo run --release -- --trader-menu nu       # our NuMenu trader menu instead of the classic KF one (the default)
cargo run --release -- --fps 30               # cap the frame rate (vsync still caps it at the monitor's refresh rate)
cargo run --release -- --window 1600x900 --display fullscreen   # graphics (also in the launcher): --display windowed|borderless|fullscreen,
                                              # --window WxH, --fov 80-120, --brightness 50-200 (%), --msaa 0|2|4|8, --anisotropy 1|2|4|8|16
cargo run --release -- --zed --zed-at -4512,-230,-3816   # test: the zed starts at a map position (Unreal X,Y,Z)
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
| Q | quick heal: bring out the Syringe, inject, switch back (HUD: SYRINGE %) |
| H | spawn a Gorefast in front of you |
| X | pause / resume zeds |
| F1 | god mode on / off (the debug line shows "(GOD)") |
| F2 | zed time now (debug) |
| F3 | show / hide the debug line at the top |
| Space (walking) | jump |
| E (in an open shop, between waves) | the trader's menu (NuMenu): mouse, or Up/Down, Tab, Enter/B buy, S sell, R fill ammo, A refill all, V armour, G grenade, Esc close |

Each run writes `logs/latest.log`: what was loaded (counts, load time), camera
position once per second (Bevy metres and Unreal units), frame timings, and
exit status.

## Multiplayer

Co-op for up to 6 players: one player hosts, the others join with just the
address and a name.

```sh
cargo run --release -- --map KF-Farm --mode waves --length short --host --name HostGuy
cargo run --release -- --join 127.0.0.1 --name ClientGal
```

Ports, what is shared, and known limits: [MULTIPLAYER.md](MULTIPLAYER.md).

## Building releases

Pushing a tag like `v0.2.0` on a commit on `main` builds the Windows zip and
the Flatpak on GitHub and publishes them as a release
(`.github/workflows/release.yml`). To build them locally:

```sh
nix build .#open-kf                    # Linux (Nix): result/bin/open-kf
nix build .#windows -o result-windows  # Windows: result-windows/open-kf-windows-x86_64.zip
nix run .#flatpak                      # Flatpak: work/flatpak/open-kf.flatpak
```

Release files contain only Open KF, never Killing Floor's files.

## Package inspection tool

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

## License

Dual-licensed under MIT (`LICENSE-MIT`) or Apache 2.0 (`LICENSE-APACHE`), at your
option. This covers the source code only, not Killing Floor's game files.
