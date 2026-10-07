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
