# Multiplayer prototype (experimental branch `multiplayer-lightyear`)

Started 2026-10-06 (overnight). The plan comes first, then what was
built and tested. Background and the choice of library:
`docs/multiplayer-research.md`.

Words used (the research doc explains more):
- **Server / host**: the game that makes the decisions. **Listen server**:
  the host also plays in the same game (KF's "Host game"). This is not a
  dedicated server.
- **Client**: a player's game that connects to the host.
- **Replication**: the server copying a value to the clients.
- **PRI**: KF's PlayerReplicationInfo, the record of each player that
  every player's game receives (name, perk, ready; later also dosh, kills
  and so on).
- **Peer id**: the number that identifies a player on the network.

## Step 1 plan: listen server + the lobby over the network

1. **Library.** lightyear 0.30.1 (made for Bevy 0.19; the research's
   pick). Only the parts we need: client, server, replication, netcode
   (the connection handshake) and UDP. Prediction and interpolation are
   built in for step 2. Fallback if it does not build: bevy_replicon +
   bevy_replicon_renet.
2. **Code.** Everything new goes in `src/net/`:
   - `mod.rs`: the `--host` / `--join` options, the plugin, and the
     `NetMode` resource that other code can ask ("is this a network
     game?").
   - `protocol.rs`: what is sent. A `NetPlayer` record (KF's PRI: peer
     id, name, perk, level, ready, character), a `NetGame` record (KF's
     GameReplicationInfo: map, mode, length, match started, lobby
     countdown), and one client-to-server message, `LobbyRequest` (my
     name, perk, level, ready, character).
   - `server.rs`: starts the UDP server and the host's own local player
     (lightyear's "host-client" mode), makes one `NetPlayer` per
     connected player, applies `LobbyRequest`s, runs KF's "start when
     everyone is ready" rule, and removes players who leave.
   - `client.rs`: connects, sends my lobby choices when they change,
     checks that the host's map is the one this game loaded, and leaves.
   - `lobby.rs`: turns the replicated `NetPlayer`s into the lobby's six
     rows, and tells the menus when to start the match.
3. **Without `--host` / `--join` nothing changes.** lightyear's plugins
   are not even added (they change Bevy's clocks), so single player runs
   exactly as before.
4. **KF's start rule** (KFGameType state PendingMatch, `Timer()` every
   second; values from KillingFloor.ini): wait until at least one player
   has been connected for more than NetWait = 5 s. Then start when every
   player is ready. With more than 2 players, once 65% are ready (or after
   300 s), a LobbyTimeout = 20 s countdown runs ("Game will
   auto-commence in: N"), and when it ends everyone is made ready.
   The Ready button changes to Unready until the match begins
   (LobbyFooter). A player who joins after the start plays as soon as
   they press Ready (LobbyFooter: `if GRI.bMatchHasBegun ClientCloseMenu`).
5. **Map.** The host's `--map`. Each client loads the map from its own KF
   install when it starts (no game files are ever sent), so the client
   must be started with the same `--map`. If the host's map is
   different, the client logs it, says which `--map` to use, and quits.
6. **In the game (this step only).** After the start, each game runs its
   own zeds and waves (they are not shared yet). The pause menu does not
   pause a network game (KF only pauses standalone games).
7. **Ready for step 2.** Each player has a peer id (0 = the host). The
   server keeps a map from peer id to the player's `NetPlayer` entity,
   with an empty slot for the pawn entity, so pawns can be attached later.
8. **Logs.** `net_*` events. Two games on one machine must not share
   `logs/latest.log`, so there is a new `--log FILE` option, and network
   games write `logs/latest-host.log` / `logs/latest-client.log` unless
   told otherwise.

Port: 7707, KF's own (KillingFloor.ini `[URL] Port=7707`).

## As built (2026-10-06)

### How to run it (one machine, two windows)

Build once: `cargo build --release`. Then, in two terminals:

```
# Terminal 1: the host (plays and hosts)
cargo run --release -- --map KF-WestLondon --mode waves --length short --host --name HostGuy

# Terminal 2: a player joining it
cargo run --release -- --map KF-WestLondon --mode waves --length short --join 127.0.0.1 --name ClientGal
```

Both games open in the lobby. Each player sees both rows. "Select Perk"
then SAVE changes your perk on both screens; Ready ticks your box on
both screens. When both are ready (and at least about 6 s after the host
started), both games leave the lobby and start their match.

Options:
- `--host [PORT]`: listen server; port 7707 if none is given.
- `--join ADDR[:PORT]`: an IP address or a host name, e.g. `127.0.0.1`,
  `192.168.1.20`, `192.168.1.20:7710`.
- `--log FILE`: where the log goes (needed for a third game on the same
  machine, e.g. `--log logs/latest-client2.log`).
- `--name NAME`: as before.
- A network game always opens the lobby; `--no-lobby` with `--host` /
  `--join` is refused with an error message.
- Over a LAN: the host's firewall must let UDP port 7707 in. Over the
  internet: port forwarding or a VPN like Tailscale (research doc).
  Neither was tested; only 127.0.0.1 was.

### What works (tested headless, logs in `logs/`, not played by you)

All runs with `scripts/headless.sh ... --mute --no-vsync` on one machine.

1. **Host alone**: `net_server_started port=7707`, the host's own player
   joins at once (`net_player_joined peer=0 host=true`); Ready at
   t=10.13 s, `net_wait_over elapsed=6`, `net_match_start players=1` at
   t=11.06 (KF's 5 s NetWait), `menu_close page=Lobby
   reason=net_match_started`, `game_start`, the waves run.
2. **Host + client** (final build):
   - client: `net_connected server=127.0.0.1:7707 after_s=1.28`;
     host: `net_player_joined peer=2699700957494690621 host=false
     players=2`, then `net_lobby_update ... from=client name="ClientGal"
     level=3`.
   - perk changes cross over: host `net_lobby_update peer=0 from=host
     perk=Some(0)` (Field Medic) and `... from=client perk=Some(6)`
     (Demolitions); the client's `net_lobby` line shows
     `0:"HostGuy":KFVetFieldMedic:L0` and its own
     `"ClientGal":KFVetDemolitions:L3`.
   - Ready / Unready: the client's ready=true, ready=false (Unready),
     ready=true each reached the host (an earlier run).
   - start: host `net_match_start players=2` at t=23.36; both games
     `net_local_match_start`, `menu_close page=Lobby
     reason=net_match_started`, `game_start ... final_wave=4`; both ran
     wave 1 with their own zeds (`zed_spawned` lines in both logs).
   - Screenshots of both lobbies: `work/screenshots/KF-WestLondon-latest-host-1791342701-1.png`
     and `...-latest-client-1791342700-1.png` (untracked). Both show the
     two rows "HostGuy Lv 0 Field Medic" (not ready) and "ClientGal Lv 3
     Demolitions" (ticked); the client's button reads "Unready".
3. **Three players, KF's countdown**: host and ClientGal ready, SlowPoke
   never ready. `net_lobby_countdown lobby_timeout=19` ... `=1`, then
   `net_lobby_timeout event=everyone_made_ready`, `net_match_start
   players=3` 21 s after the second Ready; SlowPoke's game started too.
4. **Leaving**:
   - client presses Disconnect in the lobby: client `net_leave
     side=client action=disconnect`, host `net_player_left ...
     name="ClientGal"` about 2 s later on the host's clock (immediately;
     the two clocks start apart), the row disappears.
   - host presses Disconnect: host `net_leave side=host
     action=stop_server`; client `net_disconnected reason=ByPeer(None)
     we_left=false`, prints "Lost the connection to the host" and quits.
   - a game that just ends (`--frames`): the other side notices after
     netcode's 3 s timeout (`net_player_left`, or on a client
     `net_disconnected reason=TransportError("ConnectionTimedOut")` and
     quit).
5. **Wrong map**: host KF-WestLondon on port 7710, client `--map KF-Farm
   --join 127.0.0.1:7710`: `net_map_mismatch host_map=KF-WestLondon
   my_map=KF-Farm`, message "the host plays KF-WestLondon; this game
   loaded KF-Farm. Start again with --map KF-WestLondon", exit code 1;
   the host removed the player 0.75 s after it joined.
6. **Late joiner**: host alone started at t=11.1; LateLarry joined at
   t=24.4 (`net_game_info ... match_started=true`), pressed Ready, and
   its game started at once (`net_local_match_start`).
7. **Single player unchanged**: `--mode waves --lobby --name Solo --input
   100:lobby_ready,300:pause_menu,330:pause_menu --frames 400`: log in
   `logs/latest.log`, no `net_` lines, no lightyear output,
   `menu_close page=Lobby reason=ready`, `game_start`, `pause
   paused=true` / `false` as before.
8. **Pause in a network game**: the pause menu opens and closes, no
   `pause` line (the game keeps running, as in KF).
9. **Zed time in a network game**: found broken and fixed (see below).
   After the fix: host zed time 0.46 s of game time in 2.28 s of real
   time, a client 0.48 s in 2.27 s, single player 0.45 s in 2.25 s.
10. `cargo clippy --release --workspace`: no warnings (the only
    "warning" line is nix saying the git tree is dirty). `cargo test
    --release --workspace`: 159 + 24 pass (7 new: the start rule, the
    request checks, the `--join` address).

### Problems found and how they were solved

- **lightyear panicked at startup** ("`LastConfirmedInput` does not
  exist", in its prediction code) although nothing is predicted yet.
  lightyear's examples insert a `PredictionManager` resource; we do the
  same at startup (`net/mod.rs`).
- **lightyear sets the speed of Bevy's game clock every frame** (its
  clock synchronisation), which undid zed time in network games (a host
  test: 2.25 s of game time passed in 2.25 s of zed time instead of
  0.45 s). Fix: just before Bevy advances the clock, the zed-time
  speed is multiplied in again (`keep_zed_time_speed`). This is a step-1
  patch: in KF the server decides zed time for everyone
  (Level.TimeDilation). With step 3 the server's zed time must reach the
  clients, and whether lightyear's clock sync copes with a slowed clock
  over long periods is not tested.
- **netcode's address check**: clients name the server by the address
  they used (127.0.0.1, a LAN address), while the server listens on all
  addresses; the check is switched off (`server_addr_check: false`).

### Choices to know about

- **No security.** The netcode key is all zeros: anyone who can reach
  the port can join. Fine for LAN / friends over a VPN, not for a public
  server. Nothing touches anti-cheat, DRM or Steam.
- **Version check**: `PROTOCOL_ID` in `net/mod.rs`; games with different
  numbers refuse each other, and lightyear also compares the list of
  sent types when a client connects. Raise the number when
  `protocol.rs` changes.
- **The host's own choices** are written straight into its record (it
  is the server); only real clients send `LobbyRequest` messages.
- **lightyear's tick is 1/64 s**, which is Bevy's own fixed-step rate,
  so the ragdoll physics step as before in network games.
- **Player order** in the lobby: KF's (not ready first, then ready;
  within each group, host first then by peer id), the same on every
  screen.
- **Max 6 players** (host + 5 clients, KF's MaxPlayers); a 7th is
  refused by netcode (not tested).
- A client that loses the host **quits** (KF goes back to its main menu;
  we have none), also in the middle of a game.

### What does not work yet / not tested

- Nothing in the game itself is shared: you do not see the other
  players, zeds and waves are separate on every machine, dosh, kills and
  damage are each machine's own. (Steps 2 and 3.)
- The lobby chat line, Options page and the movie are as in single
  player (inert / black).
- After the match starts, a perk change in the pause menu is sent and
  the server's record follows it, but nobody can see it (no scoreboard
  yet). Not tested.
- The end of the match ("restart") is each game's own: the host's game
  restarted wave 1 by itself after its player died in a test.
- Not tested: two real machines, a LAN, the internet, a 7th player,
  packet loss or lag, a client with a different build (protocol check),
  `--join` with a host name other than `localhost`, mouse clicks (the
  virtual display has no mouse: all tests used `--input`).
- Not played by you.

### What step 2 (other players' pawns and bodies) needs

1. **Spawn a pawn per player on the server** when that player's match
   starts (`net_local_match_start` is the moment on each side; the
   server already has `NetPlayers` (peer id -> record) and `PlayerSlot.pawn`
   to store it). The local player keeps its first-person camera; every
   other player's pawn gets the third-person body (`player/body`, which
   can already load any character: `NetPlayer.character` is replicated).
2. **Movement sharing.** Two choices:
   - simple first: each client sends its own position, view rotation,
     crouch and velocity a few times a second (client-authoritative,
     like KF's `ServerMove` without the checks), the server copies them
     to the others, and the others smooth them (lightyear's
     interpolation, already compiled in). Good enough to see each other
     walk.
   - KF's real design (server checks every move, client prediction with
     corrections) needs the walking code on a fixed tick (research doc,
     stage 1) and input as a per-pawn command value (research doc,
     stage 0). That is weeks, not one night.
3. **Body animation from replicated state**: the body's animation is
   picked from movement today; for remote pawns it must be picked from
   the received velocity / crouch / weapon (KF: `AnimAction`,
   `bIsCrouched`, the weapon attachment).
4. **The "one player" assumptions** (research doc section 2) start to
   matter: zeds target "the" player (`think.rs`), the HUD and damage read
   global resources. For step 2 alone (just seeing each other) they can
   stay; for step 3 (shared zeds) they cannot.

## Step 2: seeing the other players (2026-10-06, overnight)

### Design

Words: **pawn** = a player's body in the world (KF's Pawn).
**Interpolation** = drawing something between two known positions so it
moves smoothly although updates arrive only a few times a second.
**Client-authoritative** = each game decides where its own player is
and the others believe it.

- **What is sent** (`net/protocol.rs`): `PawnUpdate`, what is needed to
  draw a pawn on another screen (the same things KF replicates for other
  players' pawns): position, velocity, on the ground, view yaw and
  pitch, weapon class, shot counter (KF's FlashCount), firing and fire
  mode, reload counter, hit counter and where the last hit came from,
  dead, plus a sequence number and the sender's clock. Positions in
  Bevy metres, angles in radians (as `PawnState`).
- **Client to server**: once its match has started, every client sends
  its `PawnUpdate` 20 times a second as a message on an unreliable,
  sequenced channel (`PawnChannel`: a lost update is simply replaced by
  the next one; an old one arriving late is dropped).
- **On the server**: each player gets a `NetPawn` entity (peer id + the
  latest update) with their first update; the host's own state is
  written there directly (it is the server). lightyear copies every
  `NetPawn` to every game (replication), so late joiners receive all
  existing pawns at once, and a despawned `NetPawn` disappears
  everywhere. The player's record (`PlayerSlot.pawn`) points at it;
  when a player leaves, their `NetPawn` is despawned with the record.
- **On every game** (`net/pawns.rs`): each `NetPawn` that is not mine
  gets a local "remote pawn" entity with a `PawnState` (local: false)
  and a `PawnCharacter` (the character name from that player's
  `NetPlayer`). The existing body code draws it exactly like the local
  player's body: same animation rules (idle / move / turn / jump / land,
  fire, weapon switch, reload, hit), same weapon attachment. Remote
  bodies are visible in first person; the local body stays hidden in
  first person as before.
- **Smoothing**: each update is placed on the sender's clock. The
  receiver keeps the smallest "arrival time minus send time" it has
  seen (it drifts up by 1 ms per update so clock drift cannot freeze
  it), and draws the remote pawn where its player was 0.1 s ago on that
  clock, blending the two updates around that moment (position,
  velocity, yaw by the short way round, pitch). The other values (weapon,
  counters, dead) come from the earlier update. If the next update is
  late, the pawn keeps moving along its last velocity for at most 0.1 s.
  A jump of more than 400 units between two updates (a respawn) is not
  slid across. lightyear's own interpolation was not used: it works on
  replicated components with its own timeline and prediction settings;
  for one value per player the hand-made version is small, testable
  (2 unit tests) and its timing shows in our log.
- **Characters**: `BodyModels` used to hold one character; it now holds
  every character loaded so far (`local` = the local player's) and
  loads another the first time a pawn asks for it (`by_name`).
  A character change of the local player respawns only the local body.
- **Not KF's model.** KF is server-authoritative: the client sends its
  moves (ServerMove), the server moves the pawn itself, checks it, and
  corrects the client (ClientAdjustPosition), and fire, damage and
  pickups are decided on the server. That needs the walk on a fixed tick
  and input as commands (research doc, stages 0-1): later work, not
  built. Our way trusts each game completely (cheating is trivial; fine
  for friends on a LAN).
- **Not added**: no collision with remote pawns (you walk through each
  other), zeds ignore remote pawns (each game's zeds only chase its own
  player), your shots do not hit other players, no sounds or muzzle
  flashes from remote weapons (the body only plays the animations), and
  no name above their heads.

### How to run it (two players on one machine)

Build once: `cargo build --release`. Then, in two terminals:

```
# Terminal 1: the host
cargo run --release -- --map KF-WestLondon --mode waves --length short --host --name HostGuy

# Terminal 2: a player joining it, as Santa
cargo run --release -- --map KF-WestLondon --mode waves --length short --join 127.0.0.1 --name ClientGal --character Baddest_Santa
```

Press Ready in both lobbies. After the match starts, each window shows
the other player's body (the host's Corporal Lewis soldier, the client's
Santa) moving, turning, aiming up and down, switching weapons (1-5 /
mouse wheel), firing and reloading. A third player:
`... --join 127.0.0.1 --name Third --character Ash_Harding --log logs/latest-client2.log`.

New test inputs (for `--input`): `walk_on` / `walk_off` (hold / release
forward). The test scripts used for this step are in `work/` (not in
the repository): `work/mp2_test.sh` (host + client), `work/mp2_test3.sh`
(three games, one joining late) and `work/mp2_analyse.py SENDER_LOG
RECEIVER_LOG` (lag and error from two logs).

Log lines: `net_pawn_sent` (every update a game sends: seq, its clock,
wall clock, position in Unreal units, weapon, counters),
`net_pawn_created` / `net_pawn_relay` (server: a player's first update;
updates per second per player), `net_remote_pawn_spawned` /
`net_remote_pawn_removed`, `net_remote_pawn` (10 times a second per
remote pawn: the position drawn, the moment drawn, the age of the newest
update, updates per second, frames drawn past the newest update), and
the body lines now say whose body (`who=local` or
`who=RemotePawn_<peer>`).

### Results (headless, one machine, 127.0.0.1; not played by you)

All runs `scripts/headless.sh ... --mute --no-vsync --fps 60 --god`.
With two or three games on one machine each ran at about 28 frames a
second.

1. **Both see each other.** Host: `net_remote_pawn_spawned
   peer=... name="ClientGal" character=Baddest_Santa`, `body_loaded
   character=Baddest_Santa wanted=Baddest_Santa`, `body_spawned
   character=Baddest_Santa local=false`, `body_visible
   who=RemotePawn_... visible=true ... first_person=true`. Client: the
   same for `HostGuy` / `Corporal_Lewis`. Screenshots (untracked, in
   `work/screenshots/`): `KF-WestLondon-latest-host-1791344960-1.png`
   (the host sees Santa holding the 9mm, close up) and
   `KF-WestLondon-latest-client-1791344983-1.png` (the client sees the
   gas-mask soldier by the ambulance, gun raised); also
   `...-host-1791344375-3.png` and `...-host-1791344254-2.png` (Santa
   further down the road).
2. **Rate**: `net_pawn_relay peer=0:updates=20 peer=...:updates=20`
   every second; receivers count 20 updates/s (lowest 14-17 in a second).
3. **Lag and error** (`work/mp2_analyse.py`, comparing the sender's
   `net_pawn_sent` with the receiver's `net_remote_pawn` by wall clock):
   - a remote pawn is drawn where its player was **about 115 ms earlier**
     (median 116-117 ms, 90% under 126 ms, worst 160 ms), both ways;
   - while walking (198 units/s) it is **19-24 Unreal units behind**
     (median 22 and 19; a player is 50 units wide); standing still: 0;
   - the blend itself adds at most 2.6 units (drawn vs. where the sender
     really was at the moment drawn);
   - frames drawn past the newest update: about 6-9% on the host (the
     client sends from a 28 fps game, so its updates come 36 or 71 ms
     apart), under 1% on the client. Since the 0.1 s coast was added the
     pawn keeps moving through those frames.
4. **Animations cross over** (host log, the client's body; the client
   log shows the same for the host's): `kind=Move reason=moving` while
   walking, `Turn(false) sequence=TurnR_Single9mm` when turning on the
   spot, `channel=1 sequence=Fire_Single9mm reason=fire` per shot, then
   `Blend_Single9mm reason=post_fire_blend`, `body_attachment_held
   weapon=KFMod.Knife`, `Weapon_Switch`, `Idle_Knife`, `Attack2_Knife
   reason=fire`, back to the 9mm, `Reload_Single9mm reason=reload`.
5. **Three players**: each game draws the other two with their own
   characters (`latest-client2` screenshot `...-client2-1791344549-1.png`:
   Santa and the soldier). Updates per player 20/s on the server.
6. **Late joiner** (started 45 s after the host, match already running):
   on connecting it received both existing pawns at once
   (`net_remote_pawn_spawned` x2 at t=5.46, before its own Ready); the
   other two games drew it as soon as it pressed Ready
   (`net_remote_pawn_spawned ... name="LateLarry" character=Ash_Harding`).
7. **Leaving**: when a client's game ended, the host logged
   `net_player_left ... reason=link_removed` and in the same frame
   `net_remote_pawn_removed`; the other client removed it at the same
   moment (`net_remote_pawn_removed`, t=37.58 on both). Detected after
   netcode's 3 s timeout when a game just ends; a Disconnect press is
   immediate (step 1).
8. **Character chosen in the lobby**: a client that changed to
   Captian_Wiggins in the lobby appeared as Captian_Wiggins on the host
   (`body_spawned character=Captian_Wiggins local=false`).
9. **Single player unchanged**: `--map KF-WestLondon --behind-view
   --input 100:fire,200:1,300:reload --frames 400`: 0 `net_` lines, no
   lightyear output, `body_spawned character=Corporal_Lewis local=true`,
   the attachment follows the weapon. The lobby character change
   (`change_character`, `lobby_save`) still gives `body_reloaded
   character=DAR bodies_respawned=1`.
10. `cargo clippy --release --workspace`: no warnings (only nix's "git
    tree is dirty"). With `--all-targets` there is one older warning in
    test code (`src/zeds/boss.rs:1005`, not touched here). `cargo test
    --release --workspace`: 161 + 24 pass (2 new: the blend with the
    coast, and the teleport rule).

### Not done / not tested

- Not tested: a death seen from another game (all runs used `--god`;
  dead bodies are hidden, as in single player, since the soldier meshes
  have no death animation), hits seen from another game (zeds of each
  game only hit their own player), jumping seen from another game
  (a `jump` input was in a run, but that client's own Clots were holding
  it, `player_pinned`, so it neither walked nor jumped), a
  character change in the middle of a match (there is no menu for it;
  `follow_pawn_character` would respawn the body), two real machines,
  packet loss, lag, more than 3 players. Not played by you.
- Both games pick their own start spot, so two players can start inside
  each other (seen once: both at (-3110, 1313)). KF's server picks
  them; step 3.
- A joiner still in the lobby sees the other players' bodies behind its
  lobby screen (harmless).
- Zed time is still each game's own, and slows the remote bodies'
  animations too (their positions keep real time).
- The `NetPawn` is replicated to its own owner too (ignored there; a
  little wasted bandwidth).
- The protocol number is now `0x4F4B_4600_0002`: step 1 builds cannot
  join step 2 games.

### What step 3 (zeds and waves shared) needs

1. **The server runs the zeds and the waves; clients only draw them.**
   Each zed becomes a replicated entity (kind, position, velocity,
   rotation, health state, animation action, head off / limbs off,
   dead); clients spawn a drawn-only zed per replicated one and blend it
   like the remote pawns (the same snapshot code can be reused). Client
   games must not spawn their own zeds or run the wave timer.
2. **Zeds must see every player.** Today `zeds/zed/think.rs` and the
   attacks target "the" player (the camera entity, global resources).
   On the server they need a list of player pawns: positions from the
   `NetPawn`s (client-authoritative, as now) plus the host's own, and
   per-player health (today `PlayerHealth` is one global resource).
   Damage to a client's player must be sent to that client.
3. **Shots must reach the server.** A client's shot (trace or
   projectile) has to hit the server's zeds: either the client sends
   "I fired from here in this direction" and the server traces (KF's
   way), or the client sends "I hit zed N for X damage" (simpler, more
   trusting). Then deaths, gibs, dosh and kills come back from the
   server.
4. **Shared game state**: wave number, zeds left, the trader time and
   shop open/closed, the match end, zed time (KF: Level.TimeDilation
   decided by the server; see step 1's note about lightyear's clock),
   each player's dosh and kills (on `NetPlayer`, KF's PRI).
5. **Spawn spots** chosen by the server, so players do not start inside
   each other.
