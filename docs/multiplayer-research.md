# Multiplayer (6-player co-op): preliminary feasibility research

Written 2026-10-06. Research only: no code was changed. Nothing here has been
built or tested. Library facts were checked against crates.io and GitHub on
2026-10-06; KF facts come from the extracted scripts in `work/scripts/` (read
only), the dumped class defaults in `work/defaults_check/orig/`, and the
install's `System/KillingFloor.ini`.

**Before anything is implemented, CLAUDE.md rule 5 ("Single-player and
offline only") has to be changed by you.** This document does not touch that
rule. Nothing below involves anti-cheat, DRM or Steam login; Steam networking
is mentioned only to say we would not use it.

Labels used below:

- **[script]**: read in KF's UnrealScript source (file and function given).
- **[defaults]**: read in the dumped class defaults or KillingFloor.ini.
- **[recalled]**: general Unreal Engine 2 knowledge. That part of the engine is
  native C++ code we do not have, so these are not checked against KF.

---

## Short answer

- Co-op looks **feasible**. KF itself is a simple design: one machine (the
  "server") runs the whole game, and the others only send key presses and
  show what the server tells them. Zeds, waves, dosh and damage all live on
  the server. That is the easiest kind of multiplayer to copy.
- **Our code is not ready for it yet.** It assumes exactly one player
  everywhere: the camera entity *is* the player, and health, armour, dosh,
  perk, weapons and more are single global values. Turning those into "one
  per player" is the biggest job, and it is useful work even without
  networking (it is what bots or a split-screen test would need too).
- **Recommended library: lightyear** (version 0.30, made for our Bevy 0.19,
  and it already supports our physics library avian3d 0.7). It is built on top
  of bevy_replicon and adds the parts a shooter needs: client-side prediction,
  smoothing of other players' movement, bandwidth priority, and lag
  compensation. Writing our own would mean rebuilding those parts.
- Rough effort: a two-player LAN test with walking and zeds is weeks of work,
  not days. Full 6-player co-op with everything synced is months. These are
  guesses (see section 4).

---

## 1. How Killing Floor does multiplayer

### 1.1 Words used

- **Server**: the machine that decides what really happens. **Client**: a
  player's machine that shows the game and sends inputs.
- **Listen server**: one player's game is also the server (they "host"); KF's
  "Host game" menu. **Dedicated server**: a server program with no player
  sitting at it. KF supports both [recalled].
- **Replication**: copying a value or an event from server to client (or
  client to server) over the network.
- **Reliable / unreliable**: a reliable message is re-sent until it arrives;
  an unreliable one may be lost (fine for things that are re-sent often, like
  positions).
- **RPC (remote procedure call)**: "run this function on the other machine".
  In UnrealScript, a function listed in a `replication` block is an RPC.
- **Client-side prediction**: your own movement happens on your screen at
  once, before the server confirms it; if the server disagrees, your client is
  corrected. **Interpolation**: showing other things slightly in the past,
  smoothly between two received updates.
- **Relevancy / interest management**: the server only sends a client the
  things that client can see or needs.

### 1.2 The server-authoritative model

**Roles [script: Engine/Actor.uc replication block; recalled].** Every actor
(object in the world) has a `Role` (what this machine is for it) and a
`RemoteRole` (what the other side is):

- `ROLE_Authority`: the server's copy; the only one that decides.
- `ROLE_AutonomousProxy`: your own pawn on your own client. You move it
  yourself (prediction), the server checks.
- `ROLE_SimulatedProxy`: other players and zeds on your client. Your client
  guesses their motion from the last position and velocity it received.
- `ROLE_DumbProxy`: only the position is copied, no guessing.
- `ROLE_None`: not sent at all (e.g. `Emitter`, `Controller`, AI) [defaults].

Each class lists its replicated variables and RPCs in a `replication { }`
block. The condition decides direction and timing, for example
`reliable if (Role < ROLE_Authority) ServerStartFire` means "client calls,
runs on server"; `reliable if (Role == ROLE_Authority) ClientReload` means
"server calls, runs on the owning client". Variables are sent when they change
(`bNetDirty`), only on first send (`bNetInitial`), or only to the owner
(`bNetOwner`).

**Per-class network settings [defaults, work/defaults_check/orig/*.txt]:**

| Class | RemoteRole | NetUpdateFrequency (max sends/s) | NetPriority | Other |
|---|---|---|---|---|
| Actor (base) | DumbProxy | 100 | 1 | |
| Pawn (players, zeds; KFMonster and KFHumanPawn do not override) | SimulatedProxy | 100 (inherited) | 2 | bUpdateSimulatedPosition |
| Controller / AI | None | | | bOnlyRelevantToOwner (AI never leaves the server) |
| PlayerController | | | 3 | |
| ReplicationInfo (GRI, PRI) | SimulatedProxy | Info: 10; PRI: 1 | | bAlwaysRelevant |
| Weapon / Inventory | SimulatedProxy | Weapon: 2 | Weapon: 3, Inventory 1.4 | bOnlyRelevantToOwner |
| WeaponAttachment (3rd-person gun) | | 8 | | |
| Projectile | SimulatedProxy | | 2.5 | bNetTemporary (sent once) |
| KF grenades, pipe bombs, M99/crossbow bullets | | | | bNetTemporary = false (kept updated) |
| Mover (doors, KFDoorMover; KFGlassMover) | SimulatedProxy | 1 | 2.7 | bAlwaysRelevant |
| Pickup | | 0.1 | 1.4 | bAlwaysRelevant |
| Effects (KFHitEffect, bile explosions, muzzle flash) | | | | bNetTemporary, or spawned locally |

**Relevancy and bandwidth [recalled, with ini values checked].** The server
decides per client which actors to send: always for `bAlwaysRelevant` and the
client's own things, otherwise roughly "can this client see it" (a line of
sight test). An actor stays relevant for `RelevantTimeout=5.0` s after it was
last seen [defaults: KillingFloor.ini, `[IpDrv.TcpNetDriver]`]. When the
connection is full, actors are sent by `NetPriority` x time since last sent.
Other limits there: `NetServerMaxTickRate=30`, `LanServerMaxTickRate=35`
(server updates per second), `MaxClientRate=15000`,
`MaxInternetClientRate=10000` (bytes per second per client), port 7707,
`MaxPlayers=6`. User.ini: `ConfiguredInternetSpeed=10000`,
`ConfiguredLanSpeed=20000` (the client's NetSpeed). So KF fits 6 players and
up to 32 zeds (`MaxZombiesOnce = 32` [defaults: KFMod.KFGameType]) into about
10-15 KB/s per client at about 30 updates per second.

### 1.3 Player movement

[script: Engine/PlayerController.uc, functions ReplicateMove, ServerMove,
SendClientAdjustment, ClientAdjustPosition, ClientUpdatePosition]

1. Each frame the client builds a "saved move" (time stamp, frame length,
   acceleration from keys, jump/crouch/sprint bits, view direction), runs it
   locally at once (`ProcessMove`), and keeps it in a list (`SavedMoves`).
   Small similar moves are merged.
2. It sends `ServerMove` (unreliable) with the move plus the position it ended
   at. `DualServerMove` packs two moves so one lost packet does not lose a
   jump. Old "important" moves (jumps, direction changes) are re-sent.
3. The server replays the move with the **same frame length** the client used
   (it does not need a fixed tick), then compares positions. If the squared
   error is above 3 (about 1.7 Unreal units) and enough time has passed
   (`180 / NetSpeed` s; always after 0.3 s), it sends one correction per tick:
   `ClientAdjustPosition` / `ShortClientAdjustPosition` (unreliable).
4. The client snaps to the corrected position at that time stamp and replays
   all later saved moves (`ClientUpdatePosition`).
5. `MaxTimeMargin` checks stop a client from claiming more time than really
   passed (speed hacks) [script: ServerMove, line ~2182].

Other players' pawns arrive as `SimulatedProxy`: `Location`, `Rotation`,
`Velocity` [script: Actor.uc], plus `bIsCrouched`, `bIsWalking`, `AnimAction`,
`Health`, `bIronSights`, `bIsSprinting`... [script: Engine/Pawn.uc], and
`bAimingRifle` [script: KFHumanPawn.uc]. KF-only movement state: weight and
carry speed (`CurrentWeight`, `InventorySpeedModifier`), torch battery
(`bTorchOn`, `TorchBatteryLife`), owner only [script: KFHumanPawn.uc].

### 1.4 Weapons

[script: Engine/Weapon.uc ClientStartFire, ServerStartFire, StartFire,
IncrementFlashCount; Engine/WeaponFire.uc ModeDoFire; KFMod/KFFire.uc
ModeDoFire, DoTrace; Old2k4/InstantFire.uc DoFireEffect]

- Pressing fire calls `ClientStartFire` on the client: it starts firing
  locally (`StartFire`) and sends `ServerStartFire` (reliable). The server
  checks the fire timer and ammo and starts firing too; if it refuses, it
  sends back the true ammo (`ClientForceAmmoUpdate`).
- Each shot (`ModeDoFire`): **only the server** uses ammo and runs
  `DoFireEffect` → `DoTrace` (the hit test; it is a non-`simulated` function,
  so it never runs on clients). The shooter's client plays the gun animation,
  recoil, muzzle flash, shells and sound itself.
- **No lag compensation [recalled]**: the server traces against the zeds where
  they are *now* on the server, using the aim the client sent. With high ping
  you have to lead targets slightly. KF zeds are slow, which hides this.
- Everyone else sees the shot through the third-person gun
  (`WeaponAttachment`): the server bumps `FlashCount` (sent to non-owners) and
  `mHitLocation` / `SpawnHitCount` (impact point), and each client runs
  `ThirdPersonEffects` (flash, tracer, impact) [script: WeaponAttachment.uc,
  KFWeaponAttachment.uc ThirdPersonEffects].
- KF extra: during zed time the client does a cosmetic-only trace
  (`DoClientOnlyFireEffect`, `bDoClientRagdollShotFX`) so blood shows on
  ragdolls that exist only on that client [script: KFFire.uc].
- Projectiles (rockets, nades, arrows) are spawned on the server and sent to
  clients; most are `bNetTemporary` (sent once, each client flies its own
  copy), grenades/pipe bombs/arrows stay updated [defaults].
- Reload: `ReloadMeNow`, `ServerRequestAutoReload`, `ServerInterruptReload`
  (client → server); `ClientReload`, `ClientFinishReloading` (server → owner);
  `MagAmmoRemaining` replicated [script: KFWeapon.uc replication].
- Iron sights: `ServerSetAiming`; fire mode switch: `ServerChangeFireMode`;
  flashlight: `ServerSpawnLight` / `FlashLight`.
- Weapon switching: `ServerChangedWeapon` [script: Pawn.uc].

**Zed time (slow motion for everyone)** [script: KFGameType.uc DramaticEvent
and Tick; GameInfo.uc SetGameSpeed; LevelInfo.uc replication;
KFPlayerController.uc ClientEnterZedTime / ClientExitZedTime]. Only the
server decides it. It sets `Level.TimeDilation` (via `SetGameSpeed`), and
`TimeDilation` is a replicated variable of the level, so every client slows
down too. Sounds are sent as `ClientEnterZedTime` / `ClientExitZedTime`.

### 1.5 Zeds

- **AI runs only on the server.** `KFMonsterController` has `RemoteRole =
  None` (inherited from Controller) [defaults], so clients never see the AI.
- Clients get each zed as a pawn: position, rotation, velocity at up to 100
  updates/s (in practice capped by the 30 Hz server tick and bandwidth),
  `Health`, `AnimAction` (the current special animation: attack, hit,
  door bash) [script: Pawn.uc, KFMonster.uc SetAnimAction]. Walk/run
  animations are chosen on each client from the velocity [recalled].
- KF zed state sent when it changes [script: KFMonster.uc replication]:
  `bDecapitated`, `Gored`, `LookTarget`, `bBurnified`, `bAshen`, `bCloaked`,
  `bCrispified`, `bZapped`, `bHarpoonStunned`, and more. Each client draws the
  head coming off, burning, cloaking from these flags.
- **Death and ragdolls are client-side.** `PlayDying` sets `bTearOff = true`
  and `bReplicateMovement = false` [script: KFMonster.uc PlayDying]: the
  server stops updating that zed and each client runs its own ragdoll from
  `HitDamageType` / `TakeHitLocation` / `TearOffMomentum` [script: Pawn.uc].
  So corpses and gibs look different on every screen, and that is fine.
- Severed limbs (`GibHeadStump`) are actors updated at 10 Hz [defaults].

### 1.6 Game state

- **GameReplicationInfo** (always sent, 10 Hz max) [script:
  GameReplicationInfo.uc, KFGameReplicationInfo.uc]: `bWaveInProgress`,
  `TimeToNextWave`, `MaxMonsters`, `CurrentShop` (which trader is open),
  difficulty (`GameDiff`), `EndGameType`.
- **PlayerReplicationInfo** (one per player, always sent, 1 Hz max)
  [script: PlayerReplicationInfo.uc, KFPlayerReplicationInfo.uc]: name,
  `Score` (**in KF this is your dosh**: `ServerBuyWeapon` does
  `PlayerReplicationInfo.Score -= Price` [script: KFPawn.uc]), `Kills`,
  `Deaths`, `Ping`, `PlayerHealth`, `ClientVeteranSkill` /
  `ClientVeteranSkillLevel` (perk), `KillAssists`, `bBuyingStuff`.
- **Shop** [script: KFPawn.uc replication]: `ServerBuyWeapon`,
  `ServerSellWeapon`, `ServerBuyAmmo`, `ServerSellAmmo`, `ServerBuyKevlar`,
  `ServerBuyFirstAid`, `TossCash` are reliable client → server calls; the
  server checks (`CanBuyNow`, price, weight) and changes the inventory.
  Perk choice: `SelectVeterancy` [script: KFPlayerController.uc].
- **Doors / welding** [script: KFDoorMover.uc]: `WeldStrength`, `MaxWeld`,
  `bDoorIsDead` replicated; welding itself is a weapon (`WeldFire`) whose
  hit happens on the server. Doors are movers: always relevant, 1 Hz.
- **Pickups**: always relevant, 0.1 Hz, only resent when changed [defaults].
- **Chat**: `ServerSay` / `ServerTeamSay` (client → server), then
  `ClientMessage` / `TeamMessage` to everyone [script: PlayerController.uc].
  Voice commands (`SendVoiceMessage`, `ClientLocationalVoiceMessage`) are
  sound IDs, not audio. **Voice chat** (microphone) is native engine code
  (`VoiceChatReplicationInfo`, `ServerSpeak`) [script names only; codec
  recalled/unknown].

### 1.7 Summary table

| System | Decided where | How it is sent | Rough rate |
|---|---|---|---|
| Own movement | client predicts, server checks | `ServerMove` unreliable each frame; `ClientAdjustPosition` when wrong | every frame up; corrections rare |
| Other players' movement | server | position/velocity/rotation, unreliable | up to ~30/s, priority 2 |
| Zed movement | server (AI) | same as above | up to ~30/s each, if relevant |
| Zed special anims, flags (decap, burn, cloak) | server | replicated variables, sent on change | on change |
| Zed death / ragdoll / gibs | each client | `bTearOff` once, then local | once |
| Firing | client starts, server hits | `ServerStartFire`/`StopFire` reliable; `FlashCount`, `mHitLocation` to others | per press / per shot |
| Hit test, damage, ammo | server | result: `Health`, flags, ammo counts | on change |
| Projectiles | server | spawn once (`bNetTemporary`) or updated | once / ~30/s |
| Reload, sights, fire mode | client asks, server decides | reliable RPCs both ways | per action |
| Zed time | server | `Level.TimeDilation` + Client RPCs | on change |
| Waves, trader timer | server | GameReplicationInfo | ≤10/s, on change |
| Dosh, kills, perk, health for HUD | server | PlayerReplicationInfo | ≤1/s, on change |
| Shop | server | reliable `ServerBuy*` / `ServerSell*` | per click |
| Doors, welding | server | mover vars, always relevant | ≤1/s |
| Pickups | server | always relevant | ≤0.1/s, on change |
| Chat | server relays | reliable RPCs | per message |
| Gore, decals, particles, sounds | each client | from flags/events above | local |

---

## 2. Our codebase's readiness

Checked in the source (no build run).

**Setup.** Bevy **0.19.1** (Cargo.toml and Cargo.lock), avian3d 0.7.0 used
only for collision queries, our own `ue-assets` crate. About 27,600 lines in
`src/`, organised by area (engine, world, render, player, weapons, zeds, game,
audio), see `docs/DESIGN.md` "Architecture".

**Things that help multiplayer:**

- **Zed AI is already its own step** (`src/zeds/zed/think.rs`
  `think_and_move`), and zeds are entities with a `Zed` component, so "run AI
  only on the server" fits naturally.
- **Random numbers are our own, with fixed seeds** (`zeds/gore.rs` `Rng`,
  per-zed `random()` in `zeds/zed/methods.rs`, waves, trader). No clock-based
  randomness. Good for repeatable tests; not required for server-authoritative
  networking, but it helps debugging.
- **Hit tests are our own math** (`game/combat.rs` `ray_cylinder`,
  `zed_hit`, `is_headshot`): simple cylinders, not the physics engine. Adding
  "rewind zeds to where the shooter saw them" (lag compensation) would be a
  small, local change if we ever want it. KF itself did not do it.
- **Recent work kept some things per pawn:** the flashlight is a
  `Flashlight` component on the holder (`weapons/flashlight.rs`), the
  Commando's Stalker spotting takes a `CloakViewer` value instead of reading
  the player directly (DESIGN.md "seeing cloaked zeds"), and the character
  choice exists. **How far it goes:** these are written so a second pawn
  *could* use them, but today only the camera entity has a `Flashlight`, the
  sleeve/character is a single global choice, and the spotting check is still
  fed from the one player. They are open doors, not finished multi-pawn
  support.
- Logging by numbers (`logs/latest.log`) will make network testing (two
  headless runs comparing positions) workable.

**Things that assume one player (the main work):**

1. **The camera is the player.** The `FlyCamera` component marks the one
   player entity; it is referenced in 35 files, and 29 places query it
   directly (`With<FlyCamera>` / `Single<…FlyCamera>`). Zed AI targets "the
   player" from that query (`think.rs` line ~58), not "the nearest of several
   players".
2. **Player state is global resources.** About 80 `Resource` types exist;
   the player-owned ones include `PlayerHealth`, `Armour`, `Dosh`,
   `Veterancy` (perk), `Weapons`, `WeaponLoadout`, `ShopInventory`,
   `AmmoDisplay`, `Recoil`, `HitCam`, `Burning`, `BileBurn`, `KillCount`,
   `PlayerZone`, `ViewBob`, `WelderScreen`, `CharacterChoice`. Each must
   become a component on a pawn entity (or a per-player record, like KF's
   PlayerReplicationInfo). Messages like `PlayerDamaged` and `PlayerPush`
   also mean "the" player and need a target pawn.
3. **Input is read inside game systems.** 12 files read the keyboard/mouse
   directly (`ButtonInput<KeyCode>` etc.), including walking
   (`player/walk.rs`), firing/reloading (`weapons/weapon/input.rs`), doors,
   the buy menu, zed time debug keys. For networking, input must become a
   small "command" value per frame (move keys, look angles, fire, reload...)
   that the game reads instead, so the server can apply a remote player's
   command the same way. The `--input`/`--autowalk` test inputs
   (`ScriptedInput`) already point this way.
4. **Variable timestep.** All gameplay runs in `Update` with
   `time.delta_secs()` (41 `add_systems(Update…)`, zero in `FixedUpdate`).
   KF also used variable frame times and sent the frame length with each move,
   so this is not a blocker for a KF-style design. lightyear's prediction,
   however, works on a fixed tick (`FixedUpdate`), so walking (and anything
   predicted) would need to move to a fixed tick to use it.
5. **Zed time uses Bevy's global `Time<Virtual>` speed** (`game/zed_time.rs`).
   That maps directly to KF's `TimeDilation`: the server sets it, clients copy
   it. Small job.
6. **First-person and third-person are mixed.** Firing is driven from the
   first-person weapon (animations, `weapons/weapon/`). For other players we
   need third-person pawn meshes holding a third-person weapon, with flash
   and tracers from `FlashCount`-style events. We do not draw other human
   players at all today (not checked in depth; I found no third-person player
   model code).
7. **Sounds and effects are triggered where the gameplay happens.** On a
   client, they must be triggered by replicated events/flags instead. This
   touches many files but each change is small.
8. **The HUD, buy menu and end screen read the global resources**; they
   would read "my pawn's" components instead.

**Biggest refactors, in order:**

1. Player-global resources → per-pawn components; "the player" queries →
   "this pawn" / "all player pawns" (also: zed targeting picks among pawns).
2. Input → a per-pawn command value, read by walk/fire/use systems.
3. Gameplay vs presentation split: decide for each system whether it is
   "server" (AI, damage, waves, dosh) or "every machine" (gore, sound, HUD).
4. Fixed tick for movement (and maybe weapons) if we use lightyear's
   prediction.
5. Third-person view of other players (mesh, animation, weapon attachment).

Steps 1-3 are worth doing even in single-player: they make the code clearer,
allow bots, and allow automated tests with two pawns in one process.

---

## 3. Library vs our own

Versions and dates from crates.io / GitHub API on 2026-10-06. "Bevy" is the
Bevy version the latest release depends on. Ours is **0.19.1**.

| Option | Latest | Released / repo active | Bevy | License |
|---|---|---|---|---|
| lightyear | 0.30.1 | 2026-09-16 / pushed 2026-10-02, 1175 stars | 0.19 (table: 0.28-0.30 → 0.19) | MIT or Apache-2.0 |
| bevy_replicon | 0.44.3 | 2026-10-06 / pushed 2026-10-06, 653 stars | 0.19 (0.41-0.44 → 0.19) | MIT or Apache-2.0 |
| bevy_replicon_renet | 0.20.0 | 2026-09-01 | 0.19, replicon 0.44 | MIT or Apache-2.0 |
| bevy_replicon_renet2 | 0.19.1 | 2026-10-05 | 0.19, but replicon **0.41** | MIT or Apache-2.0 |
| bevy_replicon_quinnet | 0.20.0 | 2026-07-04 | 0.19, but replicon **0.41** | MIT or Apache-2.0 |
| renet / bevy_renet | 2.0.0 / 5.0.0 | 2026-01 / 2026-06; pushed 2026-06-20 | 0.19 | MIT or Apache-2.0 |
| renet2 / bevy_renet2 | 0.17.1 | 2026-10-05 | 0.19 | MIT or Apache-2.0 |
| naia (naia-bevy-*) | 0.25.0 | 2026-05-12 / pushed 2026-10-06, 1181 stars | **0.18** (main branch also 0.18) | MIT or Apache-2.0 |
| matchbox (bevy_matchbox) | 0.14.0 | 2026-02-13 / pushed 2026-06-02 | **0.18** (main branch also 0.18) | MIT or Apache-2.0 |
| quinn (QUIC) | 0.11.12 | 2026-09-14, very widely used | none (plain Rust) | MIT or Apache-2.0 |
| aeronet (used by lightyear) | 0.21.0 | 2026-09-23 | 0.19 | MIT or Apache-2.0 |

Sources: https://github.com/cBournhonesque/lightyear,
https://github.com/simgine/bevy_replicon,
https://github.com/simgine/bevy_replicon_renet,
https://github.com/UkoeHB/renet2, https://github.com/lucaspoffo/renet,
https://github.com/Henauxg/bevy_quinnet, https://github.com/naia-lib/naia,
https://github.com/johanhelsing/matchbox, https://github.com/quinn-rs/quinn,
https://github.com/aecsocket/aeronet, and `https://crates.io/api/v1/crates/<name>`.

### lightyear

- What it is: a full "server-authoritative multiplayer" kit for Bevy. Since
  recent versions its world replication **is bevy_replicon**
  (`lightyear_replication` 0.30.1 depends on `bevy_replicon ^0.44.1`), with
  lightyear adding the shooter-specific layers.
- Has (from its README, not tried): client-side prediction with rollback;
  snapshot interpolation; input buffering per tick with redundancy against
  packet loss; input delay setting; bandwidth cap with priorities ("priority
  accumulation", close to KF's NetPriority); **lag compensation** for hitting
  interpolated entities; interest management (via replicon); host-client mode
  (a listen server: one player hosts); UDP, WebSocket, WebTransport, and Steam
  (via aeronet). Has an avian3d 0.7 integration (`lightyear_avian3d`), the
  same avian version we use.
- Cautions: the docs book is marked "WIP"; the crate is split into ~30
  sub-crates and its API has changed a lot between versions (it is at 0.x).
  Prediction expects fixed-tick systems. 55 open issues. One main author.
- Host migration (someone else takes over if the host quits): not mentioned;
  assume no. KF did not have it either.

### bevy_replicon (+ renet / quinnet / renet2 transports)

- What it is: replication only, done well. Marks entities/components to copy,
  remote events both ways, per-client visibility, works as single-player,
  client, dedicated server or listen server with the same code (README).
- Does **not** include prediction, interpolation or lag compensation; the
  README points to add-on crates, some of which it marks unmaintained
  (bevy_replicon_snap, bevy_timewarp). We would write those ourselves (or use
  lightyear, which is exactly that layer on top of replicon).
- Transports: bevy_replicon_renet is up to date (replicon 0.44). The renet2
  and quinnet adapters still pin replicon 0.41, so mixing them with the
  newest replicon would need the older replicon. renet: UDP with its own
  secure "netcode" connection protocol; renet2 adds WebTransport/WebSocket and
  in-memory sockets; quinnet: QUIC (encrypted, built on quinn).

### naia

- Mature design (Tribes 2 model, like Unreal's), server-authoritative,
  rooms for interest management, tick-buffered input "for prediction and
  rollback", authority delegation, UDP and browser WebRTC.
- **Not on Bevy 0.19** (release and main branch both 0.18). We would have to
  wait or patch it. Prediction is helper-level, not a full rollback system
  (from README wording; not checked in code). Listen-server support: not
  verified.

### matchbox

- Peer-to-peer WebRTC sockets (each player connects to the others), with a
  small "signalling server" to introduce players and STUN/TURN servers to
  get through home routers. Usually paired with GGRS rollback (lockstep-style,
  needs a fully deterministic game).
- Poor fit: KF is server-authoritative with 32 AI zeds and physics; full
  determinism across 6 machines is a very large requirement. Also still on
  Bevy 0.18. Its useful part (getting through routers) could be reused as a
  transport later if needed.

### Hand-rolled (UDP or QUIC via quinn)

- Possible: KF's own design is simple (section 1), and we control the hit
  tests. quinn is a solid, heavily used QUIC library.
- But we would have to write and debug: connection handling, reliable and
  unreliable channels, change detection and delta sending per client,
  entity ID mapping, relevancy, priority under a bandwidth cap, client
  prediction with saved moves and replay, interpolation, clock sync. That is
  most of what lightyear already provides and tests. Estimate: several times
  the effort of the library route, with more bugs to find blind (we cannot
  see the game, so every network bug has to be found from logs).

### Getting friends connected (all options)

- **LAN / direct IP**: works with every option. Over the internet the host
  must forward a UDP port on their router (KF used 7707). Simple, but not
  friendly for non-technical players.
- **VPN overlay** (Tailscale, ZeroTier, etc.): players join a private
  network and then use "direct IP". No code needed from us. Easiest first
  step.
- **NAT punch-through / relay**: needs a public server (matchbox signalling
  + STUN/TURN, or a relay). That is running an online service; decide only
  after rule 5 is changed.
- **Steam networking**: lightyear/aeronet support it, but it means Steam
  accounts and SDK. Leave it out (rule 5 and the "no DRM/Steam auth" line).
- Every player needs their own KF install: assets are never sent over the
  network (rule 1 stays intact). Server and clients must check they use the
  same game version (a version/handshake check).

What I could verify: versions, dates, dependency versions, licenses, README
feature lists. What I could **not** verify: whether these features work well
for a game our size, real bandwidth with 32 zeds, quality of lightyear's
lag compensation, naia's listen-server support. Nothing was compiled or run.

---

## 4. Recommendation (preliminary)

**Use lightyear**, with its host-client mode (listen server, like KF's "Host
game") and UDP. Reasons: it is on our exact Bevy version and our exact avian
version; it gives prediction, interpolation, priorities and lag compensation,
which are the hard parts; under it is bevy_replicon, so if lightyear's upper
layers ever disappoint, we can drop to plain replicon and keep the replication
work. Fallback: **bevy_replicon + bevy_replicon_renet**, writing a KF-style
saved-move prediction ourselves (section 1.3 is a ready recipe). Not
recommended now: naia and matchbox (not on Bevy 0.19), hand-rolled (too much
to rebuild).

Design choices that follow KF: the server owns zeds, damage, dosh, waves,
shop, doors and zed time; only your own pawn is predicted; everything else is
interpolated; ragdolls, gore, decals and sounds stay local to each client;
hit tests on the server (optionally with lag compensation, which KF lacked).

### Staged plan (each stage revertible, tested by logs)

**Stage 0: single-player refactors (no networking, rule 5 not needed).**
Per-pawn components for health, armour, dosh, perk, weapons, inventory,
status effects; a `LocalPlayer` marker separate from the camera; input →
per-pawn command; zeds choose a target among all player pawns; a test option
to spawn a second, bot-driven or scripted pawn in the same game and log both.
Effort guess: **2-4 weeks** of sessions (many files, but mechanical).
Risk: low; it may change single-player behaviour slightly, which logs and
existing test views would catch.

**Stage 1: fixed tick for movement and firing.** Move walking and weapon
timing to `FixedUpdate` (e.g. 30 or 60 Hz) with smoothing of the camera
between ticks. Effort guess: **1-2 weeks**. Risk: medium (feel of movement,
zed time interaction with fixed tick).

**Stage 2 (needs rule 5 changed): two players on LAN, movement + zeds.**
Add lightyear; host and join by IP (`--host`, `--connect IP`); replicate
pawns, zeds (position, animation state, flags), deaths as "tear off" events;
own pawn predicted; two headless runs log positions to compare. Effort
guess: **3-6 weeks**. Risk: high, it is the first contact with the library
and with network bugs; animation of remote zeds/players from replicated
state is new work.

**Stage 3: combat.** Fire commands, server hit tests, damage, third-person
weapon and muzzle flash/tracers for other players, projectiles, reload, zed
time. Effort guess: **3-6 weeks**.

**Stage 4: game flow.** Waves and trader state, per-player dosh/kills/perk
(a PlayerReplicationInfo-like record), shop as server requests, doors and
welding, pickups, chat, end of match, joining mid-game / spectating while
dead. Effort guess: **3-5 weeks**.

**Stage 5: six players and the internet.** Bandwidth and priority tuning with
32 zeds, relevancy, simulated lag/loss tests, version handshake, connection
guide (VPN or port forwarding). Effort guess: **2-4 weeks**.

Total: very roughly **3-6 months** of steady work. Low confidence: we have
not tried the library, and much of the game is not finished in
single-player yet (the Patriarch is marked unfinished in DESIGN.md);
networking every new feature adds work to it from then on.

### Main risks

- **Cannot see the game**: network glitches (rubber-banding, jitter) are
  visual; we need good logging (corrections per second, position error,
  bandwidth) and your eyes for checks.
- **lightyear API churn**: 0.x with frequent breaking releases tied to Bevy
  versions; upgrading Bevy means upgrading lightyear at the same time.
- **Scope creep**: every single-player feature after this has to be written
  twice-aware (server vs client). Doing Stage 0 first limits this.
- **Animations of remote pawns**: our zed animation is driven by local AI
  state; on clients it must be driven by replicated state.
- **Bandwidth**: 32 zeds x 6 players at 30 Hz is fine in KF's budget only
  with priorities and relevancy; untested for us.
- **Rule 5**: must be changed by you first. Suggested wording if you want
  it: "Single-player and private co-op over LAN/direct IP only; no public
  servers, no Steam networking, nothing that touches anti-cheat or DRM."
  This is only a suggestion; the decision is yours.
