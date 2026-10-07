# Perks: audit of every perk effect (2026-10-07)

What each of KF's seven perks (the `KFVet*` classes in KFMod, base class
`KFVeterancyTypes`) does at levels 0-6, where KF applies it, and whether
Open KF does the same. Scope: you pick the perk and the level
(`--perk NAME --perk-level 0-6`, launcher, lobby, buy menu keys 1-7);
earning levels and saving progress are out of scope.

How this was checked: every static function of the seven classes was
read in the extracted scripts (`work/scripts/KFMod/KFVet*.uc`), then the
matching method in `src/game/perks.rs` was compared line by line, then
every caller of that method in our code was read (grep for the method
name). "Done" below means the formula matches the script and the caller
uses it in the place KF does. Network: "MP" says whose perk is used in a
network game (each player has their own; `NetPlayer.perk` / `.level`).

Status words: **done**, **partly**, **missing**, **wrong** (was wrong,
see the "fixed" note), **n/a** (KF never calls it, or no perk overrides it).

**Summary of the audit (before the 2026-10-07 work):** every formula in
`src/game/perks.rs` matched the scripts and every one had a caller.
Missing: the Firebug's and the Medic's grenades (GetNadeType), the
Commando's zed health bars (SpecialHUDInfo), healing other players.
Wrong in network games only: a joining player's perk resistances to the
host's bile / Husk fire / Patriarch rockets (damage type lost), burn
ticks on the host using the host's perk, the Commando's Stalker glow on
a joiner (never shown) and a hosting Commando un-cloaking Stalkers for
everyone. All of these were then built or fixed (MODLOG.md, 2026-10-07).

Formulas use L = level (0-6), L5 = min(L, 5).

## Field Medic (KFVetFieldMedic)

| Effect | KF rule | Status | Our code | Note |
| --- | --- | --- | --- | --- |
| Syringe recharge (GetSyringeChargeRate) | L0 1.10; L1-4 1.25 + 0.25 L; L5 2.5; L6 3.0 (charge +10 x this per tick, an int) | done | perks.rs `syringe_charge_rate`, weapons/weapon/input.rs | also the medic guns' dart charge |
| Heal potency (GetHealPotency) | L0 1.10; L1-2 1.25; L3-5 1.5; L6 1.75 | done (added 2026-10-07 for others) | perks.rs `heal_potency`; input.rs (self heal, Syringe primary on a teammate), projectile.rs (darts, MedicNade), game/healing.rs, net/heals.rs | see "Teammate healing" |
| Move speed (GetMovementSpeedModifier) | below Suicidal: L0-1 1.0, else 1.05 + 0.05 (L-2) (1.25 at 6) | done | perks.rs `movement_speed`, walk via weapons/weapon/perk.rs | we play Normal difficulty |
| Bile resistance (ReduceDamage, DamTypeVomit) | own bile shooter 0; L0 0.90, L1 0.75, L2-4 0.50, L5-6 0.25 | done (was wrong for joiners, fixed 2026-10-07) | perks.rs `reduce_damage`, game/combat.rs | joining players lost the damage type of host-sent bile: fixed 2026-10-07 |
| Medic gun magazine (GetMagCapacityMod) | MP7M/MP5M/M7A3M/KrissM/BlowerThrower: L1+ 1 + 0.2 L5 | done | `mag_capacity`, weapon/perk.rs | |
| Medic ammo boxes (GetAmmoPickupMod) | same list (+ Camo/Neon ammo): 1 + 0.2 L5 | done | `ammo_pickup_mod`, weapon/pickup.rs | DESIGN.md said LATER; ammo boxes now exist and use it |
| Discount (GetCostScaling) | Vest and medic guns 0.9 - 0.1 L | done | `cost_scaling`, buy_menu.rs, armour.rs | |
| Better armour (GetBodyArmorDamageModifier) | L0-5 1 - 0.1 L; L6 0.25 | done | `body_armor`, combat.rs | |
| Start items (AddDefaultInventory) | L5+ armour 100; L6 MP7M (sell 225) | done | `default_inventory`, weapon/inventory.rs, load.rs | |
| Grenade (GetNadeType) | always MedicNade: Damage 50 in 175 units to zeds and HealBoostAmount 10 x potency to players, at the explosion and 8 more times 1 s apart | done (added 2026-10-07) | perks.rs `nade_class`, load.rs `perk_nade`, projectile.rs `medic_pulse` | no smoke-loop sound; KF quirk kept: the thrower is paid for healing himself |

## Support Specialist (KFVetSupportSpec)

| Effect | KF rule | Status | Our code | Note |
| --- | --- | --- | --- | --- |
| Carry weight (AddCarryMaxWeight) | L0 0; L1-4 1 + L; L5 8; L6 9 (on top of 15) | done | `carry_weight_bonus`, weapon/perk.rs | |
| Welding (GetWeldSpeedModifier) | L0-3 1 + 0.25 L; L4+ 2.5 | done | `weld_speed`, input.rs | joiner's welds computed on the joiner |
| Max ammo (AddExtraAmmoFor) | FragAmmo 1 + 0.2 L; shotgun ammo L1 1.1, L2 1.2, L3-5 1.25, L6 1.3 | done | `extra_ammo`, weapon/perk.rs | 5 frags -> 11 at 6 |
| Damage (AddDamage) | shotgun types: L0 1.1, else 1 + 0.1 L; DamTypeFrag L1 1.05, L2+ 0.9 + 0.1 L | done | `add_damage`, combat.rs | MP: the hit carries the shooter's perk to the host |
| Penetration (GetShotgunPenetrationDamageMulti) | L0 r + (1-r)/10; else r + (1-r)/5.5555 x L5 | done | `shotgun_penetration`, projectile.rs | |
| Discount | shotguns 0.9 - 0.1 L | done | `cost_scaling` | |
| Start items | L5 Shotgun (200); L6 BoomStick (225) | done | `default_inventory` | |

## Sharpshooter (KFVetSharpshooter)

| Effect | KF rule | Status | Our code | Note |
| --- | --- | --- | --- | --- |
| Headshot damage (GetHeadShotDamMulti) | listed types: L0-3 1.05 + 0.05 L, L4 1.3, L5 1.5, L6 1.6, else 1.0; then x 1.05 (L0) or x (1 + 0.1 L5) | done | `headshot_damage`, combat.rs | not for melee or fire; Dualies only below Hell on Earth (we play Normal) |
| Recoil/spread (ModifyRecoilSpread) | Crossbow, Winchester, 9mm(s), Deagle(s), M14, M99, SPSniper: L1 0.75, L2 0.5, else 0.25 (L0 too: script quirk, kept) | done | `recoil_spread`, weapon/perk.rs, firing.rs | |
| Fire rate (GetFireSpeedMod) | Winchester, Crossbow, M99, SPSniper: L1+ 1 + 0.1 L | done | `fire_speed` | |
| Reload (GetReloadSpeedModifier) | rifles and pistols list: L1+ 1 + 0.1 L | done | `reload_speed` | |
| Discount | handcannons, MK23, .44, M14, M99, SPSniper: 0.9 - 0.1 L | done | `cost_scaling` | |
| Bolt discount (GetAmmoCostScaling) | Crossbow ammo 1 - 0.07 L | done | `ammo_cost_scaling` | |
| Start items | L5 Winchester; L6 Crossbow | done | `default_inventory` | |

## Commando (KFVetCommando)

| Effect | KF rule | Status | Our code | Note |
| --- | --- | --- | --- | --- |
| Sees cloaked Stalkers (ShowStalkers, GetStalkerViewDistanceMulti) | spotted within sqrt(m x 640000): m = 0.0625, 0.25, 0.36, 0.49, 0.64, 1, 1 | done (fixed for joiners) | zeds/zed/spotted.rs, think.rs | single player worked; a joining Commando never saw the glow (puppet zeds skipped the check) and a hosting Commando made spotted Stalkers visible to everyone: both fixed 2026-10-07 |
| Sees cloaked Patriarch | any level, within 1000 units, in sight | done (fixed for joiners) | spotted.rs `boss_spot_tick` | same joiner fix |
| Zed health bars (SpecialHUDInfo) | L1+: bars over living zeds closer than 160/320/480/640/800/800 units (L1..6); not cloaked unless spotted or zapped; HUDKillingFloor.DrawHealthBar (2 x CollisionHeight above, within 2000 of the camera, in sight, 50 x 6 grey box, red fill) | done (added 2026-10-07) | game/hud.rs `zed_health_bars`, perks.rs `commando_health_bar_range_sq` | not compared with a real KF screenshot |
| Rifle magazine (GetMagCapacityMod) | rifle list: L1 1.1, L2 1.2, L3+ 1.25 | done | `mag_capacity` | |
| Rifle ammo boxes / max ammo | same tiers | done | `ammo_pickup_mod`, `extra_ammo` | |
| Damage (AddDamage) | rifle types: L0 1.05, else 1 + 0.1 L5 | done | `add_damage` | |
| Recoil (ModifyRecoilSpread) | rifles: L0-3 0.95 - 0.05 L; L4-5 0.7; L6 0.6 | done | `recoil_spread` | |
| Reload (GetReloadSpeedModifier) | every weapon: 1.05 + 0.05 L | done | `reload_speed` | |
| Zed time extensions | L3+: L - 2 | done | `zed_time_extensions`, zed_time.rs, net/zedtime.rs | MP: killer's perk |
| Discount | rifles 0.9 - 0.1 L | done | `cost_scaling` | |
| Start items | L5 Bullpup; L6 AK47 | done | `default_inventory` | |

## Berserker (KFVetBerserker)

| Effect | KF rule | Status | Our code | Note |
| --- | --- | --- | --- | --- |
| Melee damage (AddDamage, bIsMeleeDamage) | L0 1.1; else 1 + 0.2 L5 | done | `add_damage` | |
| Melee swing speed (GetFireSpeedMod) | KFMeleeGun, Crossbuzzsaw: 1.05/1.10/1.10/1.15/1.20/1.25 (L1-6) | done | `fire_speed` | also the Welder (a KFMeleeGun in KF too) |
| Melee move speed (GetMeleeMovementSpeedModifier) | L0 .05, L1 .10, L2 .15, L3-5 .20, L6 .30 (added to 0.2) | done | `melee_movement_speed`, weapon/perk.rs | |
| Damage taken (ReduceDamage) | bile: .90 .75 .65 .50 .35 .25 .20; all other: 1 .95 .90 .85 .80 .70 .60 | done (fixed for joiners: bile) | `reduce_damage`, combat.rs | |
| Clots can't grab (CanBeGrabbed) | not by ZombieClot | done | think.rs, net/zeds.rs | MP: checked on the grabbed player's own game |
| Zed time extensions | min(L, 4) | done | `zed_time_extensions` | |
| Discount | melee weapons 0.9 - 0.1 L | done | `cost_scaling` | |
| Start items | L5 Machete; L6 Axe, and armour 100 below Suicidal | done | `default_inventory` | |
| CanMeleeStun | true | n/a | - | no caller anywhere in KF's scripts |

## Firebug (KFVetFirebug)

| Effect | KF rule | Status | Our code | Note |
| --- | --- | --- | --- | --- |
| Fire damage (AddDamage) | DamTypeBurned, DamTypeFlamethrower, Husk impact, Flare impact (+subclasses): L0 1.05, else 1 + 0.1 L | done (burn ticks fixed for MP) | `add_damage`, combat.rs, zeds/zed/effects.rs | burn ticks on the host used the host's perk for a zed a joiner set alight; now the igniter's (KF: BurnInstigator) |
| Fire resistance (ReduceDamage) | same types: L0-3 0.5 - 0.1 L; L4+ 0 | done (fixed for joiners) | `reduce_damage` | Husk fireballs on a joiner lost the damage type: fixed |
| Flamethrower range (ExtraRange) | L0-2 0, L3-4 1, L5-6 2 extra timers | done | `flame_extra_range`, projectile.rs | |
| Magazine (GetMagCapacityMod) | Flamethrower 1 + 0.1 L; MAC10 1 + 0.12 L5 | done | `mag_capacity` | |
| Max ammo / ammo boxes | flame, MAC10, Husk, Trenchgun, Flare: 1 + 0.1 L | done | `extra_ammo`, `ammo_pickup_mod` | |
| Reload | Flamethrower, MAC10, Trenchgun, Flare(s): L1+ 1 + 0.1 L | done | `reload_speed` | |
| Incendiary MAC10 (GetMAC10DamageType) | DamTypeMAC10MPInc | done | `mac10_incendiary` | |
| Grenade (GetNadeType) | L3+ FlameNade (Damage 80, DamTypeFlameNade, sets zeds on fire) | done (added 2026-10-07) | perks.rs `nade_class`, load.rs `perk_nade`, input.rs, projectile.rs | sound volume a guess |
| Discount | flame weapons 0.9 - 0.1 L | done | `cost_scaling` | |
| Start items | L5+ FlameThrower (200); L6 armour 100 | done | `default_inventory` | |

## Demolitions (KFVetDemolitions)

| Effect | KF rule | Status | Our code | Note |
| --- | --- | --- | --- | --- |
| Explosive damage (AddDamage) | frag, pipe, M79, M32, M203, rockets, SP grenade, SealSqueal, SeekerSix (+subclasses): L0 1.05, else 1 + 0.1 L | done | `add_damage` | |
| Explosive resistance (ReduceDamage) | same types: 0.75 - 0.05 L | done (fixed for joiners) | `reduce_damage` | the Patriarch's rockets on a joiner lost the damage type: fixed |
| Max ammo (AddExtraAmmoFor) | frags 1 + 0.2 L; pipe bombs 1 + 0.5 L; LAW 1 + 0.2 L | done | `extra_ammo` | 2 pipes -> 8 at 6 |
| Discount | pipe bomb 0.5 - 0.04 L; launchers 0.9 - 0.1 L | done | `cost_scaling` | |
| Ammo discount (GetAmmoCostScaling) | pipe 0.5 - 0.04 L; launcher ammo 1 - 0.05 L | done | `ammo_cost_scaling` | |
| Start items | L5 pipe bomb (sell 0); L6 pipe bomb + M79 (225) | done | `default_inventory` | |

## Base class functions no KF perk overrides

`ShouldBecomeIncendiary`, `KilledShouldExplode` (base false; called in
KFMonster.TakeDamage) and `CanMeleeStun` (no caller): nothing to do.

## Teammate healing (added 2026-10-07)

KF heals other players three ways, each multiplied by the healer's
GetHealPotency, and pays the healer dosh (`ReceiveRewardForHealing`:
int(healed / 100 x 60)): the Syringe's primary fire (a player within 80
units in front, `SyringeFire.GetHealee`), the medic guns' darts
(`HealingProjectile`), and the Medic grenade. All three are built
(`game/healing.rs`, `net/heals.rs`): the healer's game finds the player
(from the other players' shared position and health), pays itself and
sends the heal; the host passes it to the healed player's game, which
applies GiveHealth (each game owns its player's health). Solo there is
no one to heal (the Medic grenade still heals yourself, as in KF).
Syringe.PostBeginPlay's heal is 50 solo and 20 (HealBoostAmount) with
other players; both are used that way now.

## Whose perk is used in a network game

- Damage you deal (AddDamage, headshots): your hit carries your perk to
  the host (`NetHit.perk/level`); burn ticks use the igniter's perk.
- Damage you take (ReduceDamage, armour): on your own game, with the
  damage type now sent along with the host's hits.
- Weapons, ammo, prices, speed, carry weight, welding, grenades: your
  own game.
- Zed time extensions: the killer's perk (host).
- Clot grabs: checked on the grabbed player's own game.
- Stalker / Patriarch glow and zed health bars: your own game, from the
  zeds the host sends.
- Heals: computed with the healer's perk on the healer's game.

## Remaining (not done, and why)

- Nothing in the seven perk classes is missing for a chosen perk and
  level. Earning levels, stats and saving are out of scope.
- Known approximations, each labelled in the code and MODLOG: GetHealee
  without its line-of-sight check; the healee's healthToGive is unknown
  to the healer (reward only); "You healed X" shown when the heal lands;
  the FlameNade's sound volume; no MedicNade smoke-loop sound; the
  Commando's bars not compared with a real KF screenshot; kill credit
  for a burn kill goes to the last hitter (KF: the igniter).
- Not tested in a run: the Patriarch's glow on a joiner, the
  Commando's bars on a joiner, a teammate healed by a MedicNade cloud,
  bile/rocket resistances on a joiner (same path as the fire one,
  which was tested).

Anything later than this audit is in MODLOG.md.
