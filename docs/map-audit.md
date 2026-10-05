# Map audit (2026-10-05)

Why: zeds stuck at the KF-WestLondon fence looked like an AI bug but were
jump pads we did not simulate. This lists every placed class in the 35
`KF-` maps that can change where pawns go or what happens to them, what it
does in KF, and whether we simulate it. Each map load now logs a
`map_features` line (simulated / not simulated, with counts) so a strange
spot can be checked against the map first.

How it was made: `kfpkg exports` on every map (465318 exports, 257
classes; `work/audit/exports.tsv`, gitignored), then class defaults and
scripts for the ones that matter. Most of the 257 are visual or editor
leftovers (lights, emitters, sounds, textures, projectors, materials,
ambient sounds, story-mode sequence actions in 2 maps, editor cameras).
KMeshProps and ConvexVolume are not placed actors: they are collision data
inside static meshes.

## Simulated

| Class | Maps / count | In KF | Ours |
| --- | --- | --- | --- |
| StaticMeshActor, BSP (Brush/Model), TerrainInfo | all | level geometry | drawn, collision |
| BlockingVolume, KFZombieZoneVolume | 34 / 7925, 32 / 895 | invisible walls; zone volumes block players only (class blocker) | collision layers |
| KFDoorMover, KFUseTrigger | 32 / 707, 30 / 397 | doors | D1-D4 |
| ZombieVolume | 33 / 3532 | zed spawn areas | G2a |
| PathNode, ZombiePathNode, InventorySpot, PlayerStart, JumpSpot... | all | navigation | nav.rs (walk, forced, door, jump links) |
| UTJumppad | 20 / 138 | throws pawns at JumpTarget | zeds and the player (M4), from the pad's centre (approximation) |
| SkyZoneInfo | 29 / 29 | sky box | drawn |
| KFGlassMover | 12 / 691 | breakable windows | M1 (2026-10-05): block, break from shots, pellets, bolts, melee, blasts, bumps |
| ZoneInfo distance fog | 31 / 638 | sight checks skip beyond DistanceFogEnd | M2: drawn and used by the spawn and seen checks |
| LavaVolume (pain volumes), ZoneInfo.KillZ | 13 / 42 | 1-2 burn a second; falling below KillZ kills | M3 |

| Emitter (placed) | 34 / 988 | fires, smoke, ambient effects | M5: loaded from the map and running |
| Projector, KFBloodSplatter | 22 / 322, 29 / 1176 | placed decals: blood, scorch marks, light patterns | M6: built at load; only on solid surfaces |

## Not simulated, by how much it matters

**High (changes what zeds or the player can do in many maps)**

| Class | Maps / count | In KF | Ours | Effect |
| --- | --- | --- | --- | --- |
| KFTraderDoor, ShopVolume, WeaponLocker | 34 maps | trader rooms open between waves | not simulated | trader milestone (T2) |

**Medium**

| Class | Maps / count | In KF | Ours | Effect |
| --- | --- | --- | --- | --- |
| Mover (plain), KFElevator | 17 / 159, 1 / 1 | scripted barriers, lifts, gates (e.g. KF-WestLondon's street barrier that rises when the helicopter leaves) | drawn, not blocking, not moving | areas KF closes are open; lifts do not lift |
| ClientMover | 7 / 229 | decorative moving parts (fans, wheels) | static, not blocking | mostly visual |
| ScriptedTrigger + Action_*, Trigger, KFProxyTrigger, UseTrigger, BETimedTrigger, NetworkTrigger | 14 / 172 and others | the event system: buttons, timed events, KF-Aperture's puzzle doors | not simulated | anything driven by events stays put |
| Teleporter with a URL | none enabled in any map (KF-Offices' 6 are bEnabled false) | would teleport on touch | nothing to do | Teleporters are trader boot spots (T2) |
| KFTraderTeleporter | 4 / 102 | older trader kick-out spots | not simulated | T2 |
| KFRandomItemSpawn, KFAmmoPickup, weapon pickups | 34 maps | pickups | not simulated | no pickups yet |

**Low**

| Class | Maps / count | In KF | Ours |
| --- | --- | --- | --- |
| WaterVolume, PhysicsVolume, KFPhysicsVolume | 3 / 5, 4 / 43, 1 / 8 | swimming, other gravity, zed jump scaling (MoonBase) | not simulated |
| xKicker, KFDecoTrampoline | KF-HillbillyHorror | bounce pads | not simulated |
| KActor | 2 / 24 | Karma physics objects | static collision |
| KF_GnomeSmashable | 7 / 246 | smashable gnomes (achievement) | static mesh |
| BlockingVolume_Toggleable, StaticMeshActor_Hideable | KF-Transit | event-toggled walls / meshes | always on |
| FluidSurfaceInfo, Drapes, Blinds, Tarp, flag | few | water surface, cloth | drawn as-is or not at all |

## Suggested order

1. Glass windows (block; zeds break them on bump; shots break them).
2. Distance fog in the sight checks (spawn rules, hidden speed).
3. The trader (T1-T3) covers trader doors, shops, trader teleporters.
4. Lava damage; KF-Offices teleporters; the player on jump pads.
5. Plain movers need the event system first; a later milestone.
