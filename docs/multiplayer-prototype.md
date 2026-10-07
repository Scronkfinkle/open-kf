# Multiplayer prototype (experimental branch `multiplayer-lightyear`)

## How to play-test in the morning (host + client on one machine)

Build once, then run these two commands in two terminals (the second
one within a minute of the first):

```
cargo build --release

# Terminal 1: the host (runs the zeds and the waves, and plays)
cargo run --release -- --map KF-WestLondon --mode waves --length short --host --name HostGuy

# Terminal 2: a second player joining it
cargo run --release -- --map KF-WestLondon --mode waves --length short --join 127.0.0.1 --name ClientGal --character Baddest_Santa
```

What you should see (all checked in headless test runs, none played by
a person yet):

1. **Lobby**: both windows list both players. Press Ready in both; the
   match starts in both about 6 s after the host opened.
2. **Start spots**: the two of you start at different PlayerStarts (a
   few metres apart near the first one), not inside each other.
3. **The other player's gun**: when the other player fires you hear
   their shot from where they stand (quieter further away) and see a
   small flash at the tip of their gun. A full-auto gun bought at the
   trader (MAC10, MP7, M4...) plays its firing loop while held.
4. **Scoreboard**: hold **Tab** (KF's key). Both players, their kills,
   status (HP, or DEAD), and cash. Your own name is red.
5. **Doors**: E opens and closes a door, and the other window sees it.
   The Welder (press 5 twice) seals it in both; "This door is welded
   shut" if you then press E. Zeds that break a door break it in both.
6. **Dying**: a dead player cannot move or shoot (the view goes
   behind) while the other plays on. When the wave ends the dead player
   comes back at a start spot with 100 HP, the starting weapons (and
   perk items), and at least £200. If both die, the match is lost and
   restarts about 14 s later, both at new start spots.
7. **Bloats, Husks, the Patriarch**: their bile, fireballs and rockets
   now hurt the joining player too, and the joining player sees them fly.
8. **Zed time** (slow motion): when it starts, both windows slow down
   together and both play the zed time sound, whoever made the kill. F2
   in either window forces one (a debug key). A Commando's kills during
   zed time extend it for both.

Known limits (details in "Step 4 as built" below):
- Both windows play sound on one machine; the mouse works in the window
  that has focus.
- Trusting prototype: each game says where its player is and which zeds
  it hit; fine between friends, not cheat-proof.
- Each game's own: pickups, dosh tossing, and the wave-end
  team bonus (each player gets the bonus of their own kills; KF splits
  the team's pot between living players). Zed time is shared (see
  "Shared zed time" at the end).
- Scoreboard: no perk icons, no ping, Assists always 0.
- Players walk through each other. A dead player just waits (no
  spectating the others).
- A joining player's grenades do not damage doors (the host's doors).
- Not tested: Patriarch rockets hitting a client (same code as the
  Husk's fireball, which was tested), three or more players with the
  step 4 features, two real machines, lag and packet loss.

Test logs go to `logs/latest-host.log` and `logs/latest-client.log`.


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

## Step 3 plan: one shared match (host runs zeds and waves)

Words: **puppet** = a zed on a client's screen that only copies the
host's zed (it does not think, walk or decide anything itself).
**Snapshot** = one message with the state of every zed at one moment.

Order (stop at a solid point; what is left is listed at the end):

1. **Waves from the host.** Clients do not run the wave timer and never
   spawn zeds themselves (no wave spawns, no test spawn keys). The host's
   wave state (phase, wave number, final wave, countdown, zeds still to
   come, zeds alive, waves ended, restarts, current shop and whether the
   trader is open) is a replicated `NetWave` record. A client copies it
   into its own `WaveGame`, so its HUD, trader, music, dosh wave reward
   and end screen work unchanged (`waves.rs`, `follow_remote_wave`).
2. **Zeds from the host.** 20 times a second the host sends each client a
   `ZedSnapshot` (unreliable, newest wins): per zed its id, kind,
   position, yaw, state, animation (sequence, frame, looping, the
   upper-body layer), health, head health and flags (dead, headless,
   cloaked, raging, burning). Packed small (positions as whole half
   units, angles and frames as integers); bytes per second are logged.
   A client makes a puppet the first time it sees an id (with the normal
   zed spawn code, so meshes, gore and sounds are the same), draws it
   0.1 s in the past blended between snapshots (as the pawns), and
   starts an animation when the host's changes. A dead flag kills the
   puppet: the ragdoll and death sound run on the client (KF's
   PlayDying is client-side too). A headless flag takes the head off
   with its gore. A zed missing from a snapshot is gone on the host.
3. **Zeds hunt every player.** On the host each zed picks its enemy by
   KF's FindNewEnemy (KFMonsterController: the nearest living player;
   checked again when the enemy dies or is out of sight, here at most
   every 0.5 s), and KF's SetEnemy rule when a client hurts it (switch to
   the attacker unless the current enemy is in sight and closer). The
   other players' positions come from their pawns (step 2) and they block
   zeds like the host's player. A hit on a client's player (melee,
   pounce, Siren scream, Patriarch melee and chaingun, Clot grab, push) is
   sent to that client and applied there: **health stays per machine**.
4. **Client shots hit host zeds** (prototype rule, **client-trusted**):
   when a client's weapon damages a puppet (the normal `damage_zed`, so
   gore and flinches show at once), the client sends what went into that
   call (zed id, damage, headshot, multipliers, damage type, its perk,
   hit point). The host makes the same `damage_zed` call on its zed with
   the client's perk, credits the kill to that player, and sends back a
   `KillCredit` (kill count + dosh on the client). KF's real way: the
   server traces the shot itself (later).

Not in this step (later): server-side movement checks, shared zed time,
shop/doors/welding/pickups shared, voice/chat, players blocking each
other, the Bloat's bile, the Husk's and Patriarch's projectiles hurting
clients (they fly at the client but only the host's own player can be
hit), ZED-gun zaps from clients, dead players respawning at wave end.

## Step 3 as built (2026-10-06, overnight)

### How to play a shared wave (two games on one machine)

Build once: `cargo build --release`. Then, in two terminals:

```
# Terminal 1: the host (runs the zeds and the waves, and plays)
cargo run --release -- --map KF-WestLondon --mode waves --length short --host --name HostGuy

# Terminal 2: a player joining it
cargo run --release -- --map KF-WestLondon --mode waves --length short --join 127.0.0.1 --name ClientGal --character Baddest_Santa
```

Press Ready in both lobbies. Both games show the same countdown, then
the same wave: the same zeds in the same places on both screens (the
host's), the same "zeds left" number and wave number in the circle at
the top right. Shoot a zed on either screen and it bleeds, loses its
head and dies on both; the kill and its dosh go to whoever shot it.
Zeds go for the nearest player and hurt whoever they hit. The wave ends
on both, the trader opens on both (the same shop). The match is lost
only when every player is dead.

Test-only inputs added: `aim_zed` (turn the view to the nearest living
zed's head, for `--input` runs). Test scripts (not in the repository):
`work/mp3_test.sh` (host + client; env `TAG`, `HOST_GOD`, `CLIENT_GOD`,
`HOST_FRAMES`, `CLIENT_FRAMES`, `SHOTS_FROM`/`SHOTS_TO` for the client's
`aim_zed`+`fire` loop, `CLIENT_EXTRA`, `HOST_ARGS`, `CLIENT_ARGS`) and
`work/mp3_analyse.py HOST_LOG CLIENT_LOG` (lag, position error and
bandwidth of the zeds).

### What was built

- `src/net/zeds.rs` (new): the host shares its wave state, sends zed
  snapshots, feeds the other players' pawns to the zeds, forwards hits
  on other players, applies clients' hits on zeds and credits their
  kills. The client follows the wave state, smooths the snapshots for
  the puppets, reports its hits and applies the hits and credits it
  gets.
- `src/zeds/zed/net.rs` (new): what a zed sends (`ZedNet`: id, kind,
  position in half units, yaw, state, main and upper-body animation,
  health, head health, flags; about 22 bytes), puppets
  (`apply_net`, `drive_puppets`, `SpawnPuppet`).
- `src/zeds/zed/think.rs`: zeds hunt a list of players (`Prey`) and
  pick one by `choose_enemy`; other players block zeds; puppets skip the
  AI; every hit on a player says which player (`to_peer`). A zed's
  "unseen" timer (used by the speed-up of unseen zeds and the stuck-zed
  cleanup) also counts another player's sight on the host.
- `src/game/waves.rs`: `WaveShare` (what is shared) and
  `follow_remote_wave` (a client's wave timer: copies it, plays the
  music, trader lines and "wave inbound" messages, opens and closes the
  shops); the host loses only when every player is dead; a `wave_hud`
  log line; zeds asked to spawn are counted at once (before, the "zeds
  left" number dipped for one frame each time a squad spawned: also in
  single player, a one-frame display glitch).
- `src/game/combat.rs`: `PlayerDamaged.to_peer`, `RemotePlayers`,
  `RemoteGrab`, `NetHit` (one `damage_zed` call, reported by a client);
  `damage_zed` records a puppet's hits, notes who hit a zed, and counts
  only this game's own kills on the HUD. `dosh.rs` pays only this
  game's player's kills. `walk.rs`: `PlayerPush.to_peer`.
- `src/zeds/zed/spawn.rs`: a client spawns only puppets (no wave or test
  spawns). `effects.rs`: puppets do not take burn ticks or zap ticks
  (the host does). `engine/camera.rs`: the `aim_zed` test input.
- `src/net/protocol.rs`: `NetWave` (replicated), `ZedSnapshot`
  (`ZedChannel`, unreliable sequenced), `PlayerEvent`, `KillCredit`,
  `NetHit` (`GameChannel`, reliable ordered, both ways). `PROTOCOL_ID`
  is now `0x4F4B_4600_0003`. `Cargo.toml`: postcard (lightyear's own
  encoder, already a dependency of it) to log snapshot sizes.

### Results (headless, one machine, 127.0.0.1; not played by you)

Runs: `work/mp3_test.sh` with tags a to h; logs `logs/mp3-<tag>-host.log`
and `logs/mp3-<tag>-client.log` (untracked). Both games ran at about
28 frames a second.

1. **The client spawns no zeds of its own.** Run e (about 4 minutes,
   two waves started): the host logged 52 `zed_spawned ... puppet=false`
   (ids 0-51), the client 52 `zed_spawned ... puppet=true` with the same
   ids and classes, and none with `puppet=false`. Example: host
   `t=29.183 zed_spawned id=0 class=KFChar.ZombieClot_STANDARD
   centre_unreal=(-1912, 1280, -3818)`, client `t=25.382 zed_spawned
   id=0 ... centre_unreal=(-1911, 1262, -3818) puppet=true` (the
   client's clock is 3.97 s behind the host's; the puppet appears about
   0.17 s after the zed).
2. **Same zeds, same places** (`work/mp3_analyse.py`, run e, 3073
   samples of one zed position on the client compared with the host's
   by wall clock): the client draws the host's zed from **about 121 ms
   earlier** (median 121, 90% under 124, worst 140 ms). A **moving** zed
   is drawn **about 34 Unreal units behind** where the host has it at
   the same moment (median 33.6, 90% under 37.6, worst 92.7; a Clot is
   52 units wide); standing zeds 0. The blend itself adds at most 1.4
   units. 14 of 238 seconds had a frame or more drawn past the newest
   snapshot (`late_frames`).
3. **Bandwidth** (zed snapshots only, postcard payload without packet
   headers): about 22 bytes per zed; 20 snapshots a second; the busiest
   second 14.4 KB/s to one client with 32 zeds alive (the biggest
   snapshot 692 bytes); median 9.5 KB/s during a wave. Each extra
   client gets its own copy.
4. **Zeds hurt the client** (run a, client without `--god`): host
   `net_player_event_sent ... event=Hurt { amount: 11.5104, zed_id: 4,
   ... ZombieMelee ... }`, client `net_player_event Hurt {...}` then
   `player_hit zed=4 damage=11 ... health_left=89 god=false`, the hit
   flash and view shake. Run c (the host walked away, the client stood
   still): Clots grabbed the client (`player_pinned by_zed=6
   seconds=1.5` on the client, 20 times) and hit it from 100 to 0
   health; then the zeds that hunted it changed to the host
   (`zed_enemy ... to=local reason=enemy_dead`, 4 times).
5. **The client's shots hurt and kill host zeds** (run e): 46 hits sent
   (`net_zed_hit_sent`), 46 applied on the host (`net_zed_hit_applied`),
   0 dropped. The same result on both sides, e.g. client `net_zed_hit_sent
   zed=6 ... damage=35.0 headshot=true puppet_health_now=13.4` and host
   `net_zed_hit_applied ... zed=6 ... health=130.0->13.4 killed=false
   decapitated=true`. Kills: the client killed 18 puppets, the host
   killed the same 18 zeds from the client's hits (the same ids in run b:
   0 1 2 3 4 5 6 7 8 9 11 13 15 16 17 18 19); 18 `net_kill_credit` sent,
   18 `dosh reason=kill_credit` on the client (12 dosh a Clot, 21 a
   Gorefast at Short length, as single player); the host's own dosh
   stayed 250. A zed the client beheaded and that then bled out was
   credited to the client too (zed 12, run b). Hit to host about 30 ms,
   kill credit back about 13 ms (from the two logs and the clock offset).
6. **Both players shoot** (run g): host 11 kills paid to the host, client
   7 credited to the client, 18 in total; the host's kills show on the
   client (`net_zed_puppet_change id=8 headless`, `... id=6 dead`). Twice
   both shot the same zed in the same moment: the host had already
   killed it (`net_zed_hit_dropped ... reason=already_dead`), so the
   client saw it die without a credit.
7. **Waves match** (run e): the "zeds left" numbers in the circle were
   the same series on both games (`wave_hud`, after the one-frame dip
   fix, run f: `20 19 18 17 16 15 14` on both). Wave 1 ended on both
   (host `t=77.169 wave_end wave=1`, client `t=73.245 wave_follow
   phase=Countdown wave=2 ... countdown=60`), the client got its
   wave-end pot (`dosh reason=wave_end amount=225`), the trader opened
   on both (`shop shop=ShopVolume1 event=open`), and wave 2 started on
   both 60 s later (`wave_start wave=2 ... zeds=32` /
   `wave_follow phase=Wave wave=2 of=4 zeds=32`).
8. **Other specimens hurt the client too** (run e, wave 2: Clots,
   Crawlers, Gorefasts, Stalkers, a Siren and a Husk): the client took
   72 Siren scream pulses (`siren_scream ... target=<client peer>` on the
   host, `player_hit ... type=SirenScream` on the client), 64 slashes
   (Stalkers), 110 claws and bites, and 14 Clot grabs, all with `--god`
   (logged, no health lost). 106 Crawler pounces started; none touched a
   player.
9. **Match lost only when everyone is dead** (run d, no `--god`): the
   host's player died at t=43.5 (`kill_player`), the game went on; the
   client died at its t=46.8; the host then logged `game_end
   result=lost` (t=51.0) and the client showed "wiped out" 0.06 s later
   by the host's clock (`wave_follow phase=Lost`, `end_game
   result=wiped_out`). Both restarted together 14 s later and both
   players were alive again (`end_game_over ... health=100`).
10. **Late joiner** (run h, joined 50 s after the host started, wave 1
   running): it received all 20 zeds at once and its HUD started in the
   wave (`game_start mode=waves follow_host=true ... phase=Wave`). The
   first puppets were asked for before the game's first frames, so they
   appeared 1.8 s later (the retry is now 0.5 s; not re-tested).
11. **Screenshots** (untracked): `work/screenshots/KF-WestLondon-mp3-f-client-1791347821-1.png`
    (the client, 9mm in hand, Clots in the tunnel, corpses and blood,
    "14 / Wave 1/4", £322) and
    `work/screenshots/KF-WestLondon-mp3-b-client-1791346678-1.png`
    (Clots by the ambulance, ragdolled corpses, "10 / Wave 1/4").
12. **Single player unchanged**: a waves run with `aim_zed` + `fire`
    (`logs/sp3b.log`): 0 `net_` lines, 0 `zed_enemy` lines, 20 zeds
    spawned (none puppets), 18 kills each paid (`dosh reason=kill`),
    zeds hit the player (`zed_melee_hit ... target=local`), no lightyear
    output. A lobby + pause run: `menu_close page=Lobby reason=ready`,
    `pause paused=true` / `false`, 0 `net_` lines.
13. `cargo clippy --release --workspace`: no warnings (only nix's "git
    tree is dirty"); with `--all-targets` still the one older test-code
    warning (`src/zeds/boss.rs:1005`). `cargo test --release
    --workspace`: 164 + 24 pass (3 new: the snapshot blend, yaw the
    short way, a puppet copying a host zed).

### Choices to know about

- **Client-trusted hits.** A client says "I hit zed N with this"; the
  host believes it and makes the same `damage_zed` call (with the
  client's perk). A cheating client could kill anything. KF's real way:
  the client sends its fire, the server traces it (later work).
- **Health is per machine.** The host decides which zed hits which
  player and how hard; the hit player's own game applies it (armour,
  god mode, death). The host only learns a client is dead from its pawn
  (`dead` in the pawn update).
- **Puppet health.** A puppet takes the client's own hits at once (gore
  and flinches show without waiting); the host's figure only lowers it
  further (other players' hits); the Patriarch's comes from the host as
  is (he heals). So a puppet can die on the client a moment before the
  host confirms it, and in a race (another player's hit first) the
  client may see a kill that is not credited to it.
- **Which player a zed hunts**: KF's FindNewEnemy (nearest living
  player, no threat assessment), re-checked when its enemy dies or (every
  0.5 s here) is out of its sight, and SetEnemy when hurt (stays on the
  old enemy if it is in sight and closer, for Mammal brains and up). Two
  players standing on the same spot make zeds switch back and forth
  often (`zed_enemy ... reason=not_visible`): they are equally near.
- **Where the other players are** for the zeds: their pawns as the host
  draws them (0.1 s in the past plus the trip, about 0.12 s).
- **Zed time, the trader's shop, doors, welding and pickups are each
  game's own.** The shop opens and closes with the host's, at the same
  shop.

### What does not work yet / not tested

- The Bloat's bile, the Husk's fireballs and the Patriarch's rockets
  fly at a client's pawn on the host but can only hurt the host's own
  player; the client is never hit by them (on the client, a dying
  Bloat's burst is local and can hurt the client). The Siren's scream,
  zed melee, the Crawler's pounce, the Patriarch's melee and chaingun,
  and Clot grabs do reach clients (screams, Stalker slashes and grabs
  seen in run e; no Crawler pounce landed in a test, and the Patriarch
  was not tested).
- ZED-gun zaps from a client only zap its puppet (the host's zed is not
  zapped). Burning: the host burns its zed from the client's fire hits
  (burn ticks are credited to the last attacker); the client shows the
  flames from the host's flag.
- Dead players are not brought back at the end of a wave (KF respawns
  them); a dead host or client stays dead until the match restarts.
- Not tested: the boss wave and the Patriarch in a network game, other
  specimens than Clots and Gorefasts (Crawler pounce, Siren scream,
  Bloat, Husk, Scrake, Fleshpound) over the network, more than one
  client, two real machines, packet loss, lag. Not played by you.
- Clients standing far from the host: the host's KF-style "is this zed
  seen" now counts clients' sight (no fog check for them); the stuck-zed
  cleanup still killed 2 zeds that nobody had seen (`zed_cleanup ...
  unseen_seconds=9999`), as in single player.

### What step 4 needs

1. **Server-side shots** (KF's way): clients send their fire (origin,
   direction, weapon, time); the host traces against its zeds, rewound
   to what the client saw (lag compensation), and applies damage.
2. **The other ranged attacks reach clients**: projectiles (bile,
   fireballs, rockets) checked against every player's pawn on the host.
3. **Respawning** dead players at the wave end (KF: RestartPlayer for
   players waiting), a spectator view while dead.
4. **A shared scoreboard**: kills and dosh on `NetPlayer` (KF's PRI
   Kills / Score), the team pot split between living players
   (RewardSurvivingPlayers), dosh tossing.
5. **Shared world state**: doors and welding, pickups, the trader's
   stock if it ever becomes limited.
6. **Zed time from the host** for everyone (Level.TimeDilation; see the
   step 1 note about lightyear's clock).
7. **Bandwidth**: send only zeds that changed, fewer for far ones (KF:
   relevancy and NetPriority), if 32 zeds x 5 clients (about 70 KB/s
   from the host) is too much on a real connection.

## Step 4 plan: the gaps a real two-player test hits first

Order of work (stop at a solid point; what is left is listed in the
results):

1. **Remote weapons are heard and flash.** When another player's shot
   counter (`flash_count`, already in the pawn update) goes up, their
   game's body code does what KF's third-person code does on every other
   machine:
   - the sound: WeaponFire / KFFire.PlayFiring on a non-owner plays the
     mode's FireSound (not the StereoFireSound the owner hears) with
     SLOT_Interact, TransientSoundVolume, TransientSoundRadius and the
     random pitch, at the pawn (so it fades with distance). Full-auto
     modes (KFHighROFFire's FireLoop state) play no per-shot sound but set
     the attachment's AmbientSound to AmbientFireSound
     (AmbientFireVolume, AmbientFireSoundRadius) while firing, and
     FireEndSound at AmbientFireVolume / 127 when it stops.
   - the flash: KFWeaponAttachment.ThirdPersonEffects -> DoFlashEmitter:
     the attachment's mMuzFlashClass emitter, spawned once and attached
     to the attachment's `tip` bone, SpawnParticle(1) per shot. The
     dynamic light (WeaponLight) is not done.
2. **Different start spots.** The host picks every player's start with
   KF's rules (GameInfo.FindPlayerStart over the map's PlayerStarts,
   rated by DeathMatch.RatePlayerStart, which KFGameType reaches through
   Invasion and TeamGame for players: primary starts first, -1,000,000
   for a start inside another pawn, -(10000 - distance) for one in sight
   within 3000 units, -1500 in the same zone, +3000 x FRand) and sends
   each player a `PlayerStart` message; the player is moved there. At
   the match start the spots are picked one after another so each sees
   the earlier players at their new spots. Same on a match restart.
3. **Dead players come back when the wave ends** (KFGameType state
   MatchInProgress, wave end: every player without a pawn gets Score =
   Max(MinRespawnCash, Score) and ServerReStartPlayer: a new pawn at a
   player start with full health and the starting inventory). The host
   sends the `PlayerStart` message with `respawn: true`; the dead
   player's game revives its player there (health 100, armour and
   weapons as a new pawn of its perk, dosh raised to MinRespawnCash).
   While dead in a network game the player cannot move or fire (KF:
   spectating). The match is lost only when every player is dead at the
   same time (the host's check used the host's death count, which a
   respawn would have broken).
4. **Bile, fireballs and rockets hurt clients.** On the host the Bloat's
   globs, the Husk's fireballs and the Patriarch's rockets test every
   player (the host's own and the other players' pawns) and send the
   hit to the hit player's game (as zed melee in step 3). The host also
   tells the clients to spawn a harmless copy of each fireball, rocket
   and glob, so they see and hear them; the client's own globs (from a
   dying Bloat's burst) no longer hurt it (the host's do).
5. **Shared scoreboard.** Each game puts its player's kills, dosh,
   deaths and health into its pawn update; everyone gets everyone's. KF's
   scoreboard (KFScoreBoard, held with Tab: User.ini `Tab=ScoreToggle`,
   F1 ShowScores) is drawn with its layout: rows on 70% of the screen,
   name, kills, assists, health, dosh. Logged as `scoreboard` lines.
6. **Doors and welding shared** (if time is left): the host owns the
   doors; clients send use and weld requests and copy the host's door
   state.

Protocol number goes up to `0x4F4B_4600_0004`.

## Step 4 as built (2026-10-07, overnight)

### What was built

1. **Other players' shots are heard and flash** (`src/player/body/fire_fx.rs`,
   new). For every pawn that is not this game's player, each rise of its
   shot counter plays the fire mode's FireSound at the pawn
   (SLOT_Interact, TransientSoundVolume and TransientSoundRadius, random
   pitch for KFFire guns), and triggers the attachment's mMuzFlashClass
   emitter, spawned once on its `tip` bone and moved with it every frame
   (`SpawnParticle(1)` per shot). Full-auto modes (KFHighROFFire,
   FlameBurstFire without bWaitForRelease) get AmbientFireSound as a
   loop on the pawn while firing and FireEndSound when they stop. The
   attachment loader reads these (`body/load.rs`: `muzzle_flash`, `tip`,
   `fire_sounds`); the 8 third-person flash classes the base weapons use
   are added to the effect list (`render/particles.rs`). The remote pawn
   entity now has a Transform (where its sounds play from). Not done:
   WeaponLight (the 0.15 s dynamic light), shell casings, tracers and
   hit effects from other players' shots.
2. **Start spots** (`src/net/starts.rs`, new; `world/map.rs` keeps the
   map's PlayerStarts with bEnabled / bPrimaryStart, read in
   `ue-assets` level.rs). The host rates every start with
   DeathMatch.RatePlayerStart (the class KFGameType reaches for players)
   against the pawns already placed and sends each player its spot
   (`PlayerStartMsg`). At the match start players are placed one after
   another by peer id; on a restart everyone is placed again. Not done:
   the water-volume check and the same-zone -1500 (no zone lookup for a
   start spot).
3. **Respawn at the wave end** (`starts.rs`; `inventory.rs`
   `respawn_inventory`; `perks.rs` `RespawnPawn`). When a wave ends
   (DoWaveEnd) or the boss wave begins (bRespawnOnBoss=True), every dead
   player gets a start spot with `respawn`; their game revives them:
   health 100, armour 0 (or the perk's start armour), every carried
   weapon dropped and KF's starting weapons plus the perk's given fresh,
   the 9mm in hand, dosh raised to MinRespawnCashNormal 200. A dead
   player cannot walk or fire in a network game, and does not get the
   wave-end pot (`dosh.rs`: only living players, as
   RewardSurvivingPlayers). The host's "everyone is dead" check uses
   "dead now and died in this game" (it used the death count, which a
   respawn or a restart would have broken).
4. **Bile, fireballs, rockets** (`zeds/vomit.rs`, `zeds/fireball.rs`,
   `game/combat.rs` `RemotePlayers::targets`, `net/zeds.rs`). On the
   host each glob and projectile tests every player (touch and blast);
   hits on another player go to their game as before (`to_peer`).
   Clients get a `ProjectileFx` per glob / fireball / rocket and fly a
   harmless copy (no damage, no door damage); a puppet Bloat's death
   burst no longer makes its own globs (the host's come over).
5. **Scoreboard** (`src/net/scoreboard.rs`, new). Each pawn update now
   carries kills, dosh, deaths and health. Tab (or the test inputs
   `scores_on` / `scores_off`) draws KFScoreBoard's layout in
   ROHud.GetSmallMenuFont with BoxMaterial (changeme_texture) boxes:
   the title "Normal | Wave N | KF-WestLondon", "Elapsed Time" (or "You
   are dead..."), columns PLAYER, Kills, Assists, Status, Cash, sorted
   like KF (kills, assists, cash, name). Every game logs a `scoreboard`
   line when anyone's numbers change.
6. **Doors and welding** (`world/door.rs` `DoorNet`, `src/net/doors.rs`
   new). The host owns the doors: a client's E press sends
   `DoorRequest::Use` (the host runs KFUseTrigger.UsedBy), a client's
   Welder hits send `DoorRequest::Weld` (the host runs TakeDamage with
   the welder types). The host sends every door's state (open or shut
   and to which key, weld, sealed, broken) when one changes and every
   2 s; a client plays the open / close itself, copies the weld, and
   breaks or brings back doors as the host says. A client's zeds
   (puppets) do not open doors, its blasts do not damage them, and it
   does not respawn them itself.

Protocol number `0x4F4B_4600_0004` (step 3 builds cannot join).

New test inputs: `aim_player` (look at the nearest other player),
`warp:X;Y;Z;YAW` (put the player's centre there, Unreal units and yaw
degrees), `scores_on` / `scores_off`. Test script (not in the
repository): `work/mp4_test.sh` (env `TAG`, `HOST_FRAMES`,
`CLIENT_FRAMES`, `HOST_GOD` / `CLIENT_GOD`, `HOST_SHOOT` /
`CLIENT_SHOOT` "FROM-TO" for aim_zed+fire loops, `HOST_EXTRA` /
`CLIENT_EXTRA` more inputs, `HOST_OPTS` / `CLIENT_OPTS` more options).
Logs `logs/mp4-<tag>-host.log` / `-client.log` (untracked).

### Results (headless, one machine, 127.0.0.1; not played by you)

1. **Remote shots** (run a): the client fired the 9mm 38 times (its
   `net_pawn_sent ... flash=38`); the host logged 38 `remote_fire ...
   weapon=KFMod.Single ... sound=KF_9MMSnd.9mm_Fire flash=triggered
   flash_class=roeffects.MuzzleFlash3rdPistol tip_unreal=(-3136,1287,-3775)`
   and played `sound_play ... 9mm_Fire slot=Interact volume=1.80
   radius=400 distance=44 gain=0.300` (the owner hears 9mm_FireST at
   gain 0.459). AK47 (run g): 54 `remote_fire` with
   MuzzleFlash3rdMP. MAC10 (run h): `remote_fire_loop ...
   looping=Some(0)`, `sound_ambient ... MAC10_Fire_Loop volume=255
   radius=500`, then `MAC10_Fire_Loop_End_M ... distance=578
   gain=0.278` when the burst ended (4 bursts, 4 loops). Screenshots:
   `work/screenshots/KF-WestLondon-mp4-g-host-1791350919-12.png` and
   `...-1791350921-17.png` (the host looks at Santa firing the AK; a
   small yellow flash at the barrel's tip).
2. **Start spots** (runs b, c, l, m): `net_start_assigned peer=0
   name="HostGuy" start=5 of=6 ... others=0` and `peer=... name="ClientGal"
   start=3 of=6 ... others=1`; the client `net_start_received`, then
   `net_moved_to_start reason=match_start`. Spots differed in every run
   (5/3, 5/1, 1/2, 0/2; about 190-430 units apart). After a lost match (run m)
   both were placed again: `reason=restart` (2 and 3), both alive
   (`end_game_over ... health=100`).
3. **Respawn** (run b: host dead; run c: client dead): host killed at
   t=45.2 (`player_died`, `net_dead_state dead=true`), the client played
   on; `wave_end wave=1` at t=85.1, `net_respawn_check event=wave_end
   dead=[0]`, `net_respawned health=100 dosh=225->225`,
   `respawn_inventory dropped=[Knife Single Frag Syringe Welder]
   given=[...same five...]`. Run c: the client died at its t=43.4; the
   host's `net_respawn_check ... dead=[<client>]`, the client
   `net_respawned`, and the host saw the client's body come back
   (`body_anim who=RemotePawn_... kind=Idle reason=revived`). The host's
   game was not lost while one player lived.
4. **Bile and fireballs on the client** (runs d, e; the host killed
   itself so the zeds went for the client): Bloat globs `vomit_touch ...
   target=<client> damage=3/4`, `vomit_landed ... other_players=[<client>:2]`;
   the client logged 79 `player_hit ... kind=Vomit` plus the bile burn.
   Husk: 3 fireballs, `fireball_exploded ... hit=player ...
   other_players=[<client>:20]`, the client 3 `player_hit zed=0
   kind=Fire` and the burn ticks. The client saw every glob and fireball
   fly (`net_projectile`, `vomit_spawned`, `fireball_spawned`,
   `fireball_exploded ... harmless=true`). The Patriarch (spawned for
   the test) used his chaingun on the client (47 hits) but fired no
   rocket in the run: rockets on a client not tested.
5. **Scoreboard** (run h, both shooting): both logs carry the same table,
   e.g. client `scoreboard HostGuy:kills=8:dosh=346:... |
   ClientGal(me):kills=1:dosh=262:...`; dead players show
   `dead=true` / "DEAD" (runs l, m). Screenshots:
   `work/screenshots/KF-WestLondon-mp4-h-client-1791351036-1.png` (client:
   HostGuy 8 kills £346, ClientGal in red 1 kill £262) and
   `...-mp4-g-host-1791350934-35.png` (host).
6. **Doors** (runs i, k; the client warped next to KFDoorMover6/7):
   client `door_use_sent trigger=KFUseTrigger1` -> host
   `door_use_remote ... event=open by=player` -> client `door ...
   event=open by=host` about 0.1 s after the press; the same for
   closing. Welding: 28 client hits `weld_hit ... sent_to_host`, host
   `weld_hit_remote ... welded added=10.0 weld=...` up to 280, the
   client's door sealed; E then gave the client "This door is welded
   shut" (`message ... (client)`); 18 unweld hits down to 10 (a zed's
   hit then took it to 1); then a host zed bashed the door down (`door_broken`) and it broke on the client too
   (`event=broken by=host`). Zeds opening doors on the host opened them
   on the client (KFDoorMover17/18/13).
7. **Single player unchanged**: waves with `aim_zed`+`fire` and `--god`
   (`logs/sp4-waves3.log`): 20 zeds, 20 kills paid, wave end pot 249,
   0 `net_` lines; without `--god` (`sp4-waves2.log`) the player died
   and the game was lost at once and restarted on fire, as before; a
   lobby + pause run (`sp4-lobby.log`): `menu_close page=Lobby
   reason=ready`, `pause paused=true` / `false`, 0 `net_` lines;
   debug mode (`sp4-debug.log`): 0 `net_` lines.
8. `cargo clippy --release --workspace`: no warnings (only nix's "git
   tree is dirty"); with `--all-targets` the older test-code warning in
   `src/zeds/boss.rs:1005` only. `cargo test --release --workspace`:
   166 + 24 pass (2 new: RatePlayerStart, the scoreboard's order and
   cash text).

### Problems found on the way

- After a lost match the host lost again in the restart frame: its
  player was still flagged dead there (it is revived just after). The
  check is now "dead now and died in this game".
- A client could get a respawn message a frame before its own wave
  state showed the wave end, and so collect the wave pot just after
  coming back: respawns now wait until the client's wave phase has left
  "Wave".
- The first scoreboard used Bevy's default font, which has no "£"; it
  now draws with KF's own menu font and box texture.
- Seen, not changed (older, single player too): on a restart the zeds
  left from the lost game are cleared by killing them, and those kills
  are paid to the player (`dosh reason=kill` right after
  `dosh reason=new_game`).

### Not done / not tested

- Patriarch rockets hitting a client (not fired in the test); three or
  more players with the step 4 features; a late joiner's start spot;
  real machines, lag, packet loss. Not played by you.
- The team pot is per machine (each player's own kills), not split
  between the living players as RewardSurvivingPlayers does.
- No spectating while dead; the dead player's own body is hidden.
- Remote shots: no dynamic muzzle light, shells, tracers or impact
  effects; KF also skips the flash when the pawn was not rendered in
  the last 0.2 s (we only skip it for hidden bodies).
- Doors: a client's grenades and other explosives do not hurt doors;
  the client does not see the host's door-hit sounds while welded doors
  are bashed (it hears the break).
- Scoreboard: no perk icons and stars, no ping column, Assists 0, names
  not clipped.

### What step 5 could do

1. Server-side shots (KF's way) instead of client-trusted hits.
2. Spectating while dead (KF: Fire cycles through the living players).
3. The team pot split at the wave end (RewardSurvivingPlayers), dosh
   tossing between players.
4. ~~Zed time decided by the host for everyone.~~ Done: "Shared zed
   time" below.
5. Pickups (ammo boxes, weapons on the floor) owned by the host.
6. Player-to-player collision.

## Shared zed time (2026-10-07)

Zed time is KF's slow motion: after some kills the whole game runs at
0.2 of normal speed for about 2.7 seconds. Before this change each game
in a network match rolled its own (so the host could be in slow motion
while the client was not). Now, as in KF, the host decides for everyone.

### How KF does it (scripts)

- Only the server runs KFGameType: `Killed` rolls for every zed killed
  by any player (0.05 within 3 m of the killer, else 0.025, not within
  0.1 s of the last event), and while zed time is on, forces an
  extension (`DramaticEvent(1.0)`) for as long as the **killer's** perk
  has `ZedTimeExtensions` left (KFVetCommando: level - 2 from level 3).
  `DramaticEvent` has the 10 s cooldown and the x2 / x4 chance after 30 /
  60 s; `Tick` counts the time down and eases the speed back over the
  last 16.6%; `DoBossDeath` forces 6 s.
- `SetGameSpeed` sets `Level.TimeDilation`, which every client receives,
  and the server calls `ClientEnterZedTime` / `ClientExitZedTime` on
  every player, which play the Zedtime_Enter / _Exit sounds and the
  first-time "ZED TIME ACTIVATED!" message on each player's game.

### What was built

- `game/zed_time.rs`: a role (`ZedTimeRole`: Local = single player,
  unchanged; Host; Client) and a small link resource (`ZedTimeNet`).
  - **Host**: rolls for every kill, a client's too. A client's hit is
    applied on the host's zed, so the zed dies there with `damaged_by` =
    that client (KF's Killer). The roll then uses that client's perk and
    level (from the lobby record, `NetPlayer`) for the Commando
    extensions, and that client's pawn for the 3 m check. Every start,
    extension, speed-up and end is sent to every client
    (`ZedTimeCommand`).
  - **Client**: does not roll. Its other rolls (explosions killing 2+ /
    4+ zeds, the F2 key, the test action `zed_time`) go to the host as
    `ZedTimeRequest`, and the host rolls them. It follows the host's
    commands: start / extend (sets the time left, slows to 0.2, plays
    Zedtime_Enter, shows the first-time message), and end. The countdown
    and the easing back run on the client with the same rule, so the
    speed-up and the Zedtime_Exit sound happen there by themselves
    (logged next to the host's speed-up for comparison).
- `net/zedtime.rs` (new): the two messages (reliable `GameChannel`),
  the host's list of players' perks and positions, sending and
  receiving. `PROTOCOL_ID` `0x4F4B_4600_0005`.
- **Smooth bodies and zeds in slow motion.** Other players' bodies and
  the host's zeds are drawn from updates stamped with the sender's real
  clock, so they slow down by themselves when the sender slows. Two
  fixes: when an update is late, a body or zed coasts along its last
  velocity, which is per game second, so the coast is now scaled by the
  game speed (`net/pawns.rs`, `net/zeds.rs`); and a zed's velocity
  measured between two snapshots (real time) is turned back into game
  units per game second (it is what the zed's ragdoll uses when it
  dies). Animations play on the game clock, which every game now slows
  together.
- New log lines: `zed_time ... wall=... role=Host|Client` (network games
  only), `net_zed_time_sent`, `net_zed_time` (client receives),
  `zed_time_host` (client: the host's speed-up / end next to its own),
  `zed_time_request` / `net_zed_time_request`, `dramatic_event ...
  peer=` (whose kill), `perk_mod kind=zed_time_extension ... peer=`;
  smoothness: `net_puppet_motion` (the client's zeds: biggest and mean
  move per frame, twice a second, with the game speed) and
  `max_step_uu` / `game_speed` on `net_remote_pawn`.
- `keep_zed_time_speed` (step 1's fix for lightyear resetting the clock
  speed) stays; it now multiplies in the shared speed.

### Results (headless, one machine, 127.0.0.1; not played by you)

Test script (not in the repository): `work/mpz_test.sh` (as
`mp4_test.sh`, logs `logs/mpz-<tag>-host.log` / `-client.log`),
`work/mpz_motion.py LOG FROM TO` prints the motion lines in a wall-clock
window.

1. **Same moment on both** (wall clock, the same machine): run a, host's
   forced zed time: host start 133.719, client 133.743 (24 ms later);
   extension 135.827 / 135.834 (7 ms); speed-up 138.094 / 138.107
   (13 ms); end 138.546 / 138.560 (14 ms). Run b: 21 ms (natural
   start), 25 ms (forced start), 21 ms / 37 ms (speed-up / end). A
   client's request (run a, `zed_time` on the client): client 178.690 ->
   host 178.703 -> client starts 178.721 (31 ms round trip).
2. **A client's kill starts it for everyone** (run b): host
   `dramatic_event reason=kill chance=0.025 started=true roll=0.044
   chance=0.100 peer=<client>` right after `net_zed_hit_applied
   peer=<client> zed=19 ... killed=true`; both games then
   `zed_time event=start reason=kill` (21 ms apart), both played
   Zedtime_Enter and showed "ZED TIME ACTIVATED!", both played
   Zedtime_Exit at the speed-up.
3. **The killer's perk** (run a: client Commando level 5, host no perk):
   during the host's forced zed time the client killed a zed: host
   `perk_mod kind=zed_time_extension perk=KFVetCommando:5 used=1 of=3
   zed=8 peer=<client>`, then `zed_time event=start
   reason=perk_extension`; the client extended 7 ms later. (With the
   host's own perk, none, there would have been no extension.)
4. **Same length, same speed** (run b, forced): host start -> end 2.70 s
   of real time, client 2.72 s. Game time / real time from start to
   speed-up: host 0.45 / 2.28 = 0.20, client 0.48 / 2.28 = 0.21;
   natural one: host 0.20, client 0.21. Run a with the extension: host
   4.83 s real, client 4.82 s.
5. **No jumps** (run b, the host walking during zed time, seen on the
   client): the host's body moved 6-8 Unreal units a frame before, 1.2-1.5
   during zed time (0.2x), up again during the easing, no spike
   (`max_step_uu`, 10 lines a second); its game-time speed stayed about
   198 (so its walk animation plays at the normal rate on the slowed
   clock). Zeds on the client: mean move per frame 2.2-2.3 units before,
   0.47-0.59 during, max 1.0-1.6 during (no spike).
6. lightyear printed no warning or resync during zed time; zed snapshots
   kept arriving 20 a second (`net_zed_receive_rate`), 0 late frames.
7. **Single player unchanged** (`logs/spz-waves.log`: waves, `--god`,
   aim_zed + fire): `dramatic_event reason=kill_near ... started=true`,
   `zed_time event=start ... game_time=48.85 real_time=49.16`, speed-up,
   end at real 51.86 (0.45 s of game time in 2.26 s), 0 `net_` or `wall=`
   lines; Commando 5 (`logs/spz-commando.log`): `perk_mod
   kind=zed_time_extension perk=KFVetCommando:5 used=1 of=3`, as before.
8. `cargo clippy --release --workspace`: no warnings (only nix's "git
   tree is dirty"). `cargo test --release --workspace`: 167 + 24 pass
   (new: the lobby perk -> extensions; the coast and the zed velocity in
   zed time).

### Not done / not tested

- A player who joins during zed time stays at normal speed until the
  next one (no state is sent on join).
- The client's countdown starts when the host's message arrives, so it
  ends that much later (on one machine 7-25 ms); the host's end message
  then ends it at once.
- The Patriarch's death (6 s) on a network game: not tested (the host
  runs it as before; clients ignore their own puppet's death).
- A client's explosions are rolled on the host from the client's own
  count of zeds killed (client-trusted, like its hits).
- Two real machines, lag, packet loss; more than one client. Not played
  by you.
