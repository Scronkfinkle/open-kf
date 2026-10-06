# Test views

Saved camera positions for checking the viewer by screenshot. Each command
opens the map at that view, saves a PNG to `work/screenshots/` (gitignored)
after 60 frames (early frames can miss objects whose shaders are still compiling), and quits. Run inside `nix develop` (or a direnv shell).

Camera format: `--camera X,Y,Z,YAW,PITCH`. Position is in Unreal units (as in
the log's `unreal=(...)`); yaw and pitch are in radians, where yaw 0 looks
along Unreal +X, yaw -1.5708 along +Y, and yaw 1.5708 along -Y.

Press F12 in the viewer to save a screenshot of any view; a `.txt` file next
to it holds the command that recreates it.

To take these without a window on your screen, swap `cargo run --release --`
for `scripts/headless.sh` (same options). It runs the game on a virtual X
display (Xvfb) with the GPU and removes the display afterwards.
`HEADLESS_SOFTWARE=1` draws on the CPU instead (slower, no GPU needed).

| Map | What it checks | Command | Expected |
| --- | --- | --- | --- |
| KF-WestLondon | Mirrored props (tunnel mesh has scale Y=-1) | `cargo run --release -- --fly --map KF-WestLondon --camera -4090,1300,-3650,-1.5708,0 --screenshot 60` | Curved brick tunnel shell between the rusty arches, bus inside. Broken if: black void between the arches. |
| KF-WestLondon | Sky zone | `cargo run --release -- --fly --map KF-WestLondon --camera -4190,2099,-3685,-4.817,-0.062 --screenshot 60` | Street with St Paul's dome and skyline in the distance under a brown sky. Broken if: flat grey-blue sky. |
| KF-WestLondon | `Skins` material overrides (two invisible blocks use the fully transparent `voidtex`) | `cargo run --release -- --fly --map KF-WestLondon --camera -4594,732,-3442,-6.7252,-0.2 --screenshot 60` | Plaza with phone booth behind a lamp post; orange sunset skyline. Broken if: large flat grey block in the plaza. |
| KF-Farm | Terrain height, layers, blending | `cargo run --release -- --fly --map KF-Farm --camera -890,-1048,2108,0,-0.25 --screenshot 60` | Rolling dirt hills with blended textures, farmhouse left, wheat field and barn right. Broken if: black ground or props floating. |
| KF-Crash | Terrain layers without AlphaMaps are skipped | `cargo run --release -- --fly  --map KF-Crash --screenshot 60` | Snow drifts with muddy wet patches in a container yard. Broken if: uniform grass everywhere. |
| KF-WestLondon | 9mm first-person placement | `cargo run --release -- --fly --map KF-WestLondon --camera -4090,1300,-3650,-1.5708,0 --screenshot 120` | Sleeve and gloved hand holding the pistol in the lower right, angled into the view. Broken if: tiny pistol in the corner (mesh scale or draw offset missing). |
| KF-WestLondon | Fire, switch, knife | `cargo run --release -- --fly --map KF-WestLondon --camera -4090,1300,-3650,-1.5708,0 --input 120:fire,200:1 --screenshot 126,330` | 1: pistol kicked up. 2: both hands, knife raised in the right hand. |
| KF-WestLondon | Zed facing, walk animation | `cargo run --release -- --map KF-WestLondon --camera -4090,1100,-3650,-1.5708,0 --zed --screenshot 100` | A pale Clot on the road, facing you, arms reaching forward mid-walk. Broken if: side-on or facing away (RotOrigin), or sunk into / floating above the road. |
| KF-WestLondon | Grabs damage the player, knife kills by headshot | `cargo run --release -- --map KF-WestLondon --camera -4090,1100,-3650,-1.5708,-0.12 --zed --input 300:1,380:fire,420:fire,460:fire,500:fire,540:fire --screenshot 700` | HUD "HEALTH 70  KILLS 1"; the Clot lies headless on the road. Log: player_hit x5 (6 each), knife hits 19 / 23.8 (headshot), decapitated=true. |
| KF-WestLondon | Iron sights (9mm) | `cargo run --release -- --map KF-WestLondon --camera -4090,1100,-3650,-1.5708,0 --input 100:aim,200:fire --screenshot 90,170,206` | 1: pistol at the hip, lower right. 2: zoomed in, pistol centred, front sight at the screen centre. 3: same, kicked up by the shot, ammo 14. |
| KF-WestLondon | Decapitation and bleed-out | `cargo run --release -- --map KF-WestLondon --camera -4090,1100,-3650,-1.5708,-0.05 --zed --input 60:fire --screenshot 72,130,700 --frames 800` | 1: headless Clot flinching (arms out, legs walking). 2: headless walk, hand on the stump. 3: Clot lying dead. Log: zed_decapitated, zed_bleeding_out (~31 health), zed_bled_out ~5 s later. |

| KF-WestLondon | 3D scope (M99) | `cargo run --release -- --camera -3110,1313,-3768,-2.93,0 --give M99SniperRifle --input 60:4,200:aim --screenshot 320` | Zoomed view; inside the M99's lens a live, enlarged picture with the blue mil-dot reticle; the ambulance front at the lens's left, as outside (not mirrored, right way up). Broken if: a solid grey disc (the fallback lens texture). |
