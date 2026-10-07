# Multiplayer in Open KF

Co-op for up to 6 players, like KF's "Host game": one player **hosts**
(their game runs the zeds and the waves, and they play too) and the others
**join** it over the network. There is no dedicated server yet.

This is experimental. It has been tested in headless runs and in two games
on one machine; play over two real machines, with lag or packet loss, and
on Windows is **not tested**. The developer's log of how each part was built
and tested is `docs/multiplayer-prototype.md`; the background research is
`docs/multiplayer-research.md`.

## Hosting and joining

```sh
# The host: picks the map, mode and length, and plays
cargo run --release -- --map KF-Farm --mode waves --length short --host --name HostGuy

# A player joining it: only the address and a name are needed
cargo run --release -- --join 127.0.0.1 --name ClientGal
```

Or run `cargo run --release` with no options: the launcher window has
Solo / Host / Join buttons, and its CHECK HOST button shows what a host is
playing before you join.

- `--host [PORT]`: host on UDP port `PORT` (default 7707, KF's game port).
- `--join ADDR[:PORT]`: an IP address or host name, port 7707 if left out.
  Before it loads anything, the joining game asks the host which map, mode
  and length it plays and uses those. If you also type `--map`, `--mode` or
  `--length`, the host's choice wins (a `note:` line says so).
- `--name`, `--character`, `--perk`, `--perk-level` work as in single
  player.
- Everyone starts in KF's lobby: pick a perk, press Ready. The match starts
  when everyone is ready.

### Ports and firewalls

The host needs **two** UDP ports reachable: the game port (7707) and the
query port right after it (7708), which answers the joiner's "what are you
playing?" question. For a game over the internet, forward both on the
host's router.

Two hosts on one machine need ports at least 2 apart, e.g. `--host` and
`--host 7710`.

If the query port cannot be reached, a joiner can still connect the old
way by typing the host's settings itself:
`--join ADDR --map KF-Farm --mode waves`.

## What is shared

- The lobby: names, perks, ready state.
- Every player's body, movement, gun, shots and muzzle flashes, and the
  sound of their gun.
- The zeds and the waves (the host runs them; joiners see the same zeds,
  and their hits count on the host). Bile, fireballs and rockets hurt every
  player.
- Doors: opening, closing, welding, zeds breaking them.
- Zed time (slow motion) for everyone at once.
- Healing each other: the Syringe's left click on a player in front of
  you, medic gun darts, and the Medic grenade's cloud. The healer earns
  dosh, as in KF.
- Every player's own perk: their damage bonuses count on the host's
  zeds, their resistances on their own game, and a Commando sees cloaked
  Stalkers and zed health bars on their own screen.
- Pickups lying in the map, tossed dosh (**B**) and thrown weapons (**\\**).
- Dying and coming back at the end of the wave; if everyone dies the match
  restarts.
- The scoreboard (hold **Tab**): names, kills, health or DEAD, cash.

## Known limits

- **Trusting, not cheat-proof:** each game reports where its player is and
  which zeds it hit. Fine between friends.
- The wave-end bonus is per player (your own kills); KF splits the team's
  pot between the living players.
- No spectating while dead; players walk through each other.
- A joining player's grenades do not damage doors.
- Scoreboard: no perk icons, no ping, Assists always 0.
- No server browser: you need the host's address.
- `--wave` on a joiner does nothing (it follows the host's wave).

## Logs

A host writes `logs/latest-host.log`, a joiner `logs/latest-client.log`
(network lines start with `net_`). Look there first if a join fails.
