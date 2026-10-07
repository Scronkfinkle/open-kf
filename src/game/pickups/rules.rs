//! KF's pickup rules as plain code (no Bevy): which spawn points and ammo
//! boxes are on, when items appear, change, go and come back. From
//! KFGameType (SetupPickups, WeaponPickedUp, AmmoPickedUp), KFRandomSpawn /
//! KFRandomItemSpawn (EnableMe, DisableMe, Timer, TurnOn), KFAmmoPickup
//! (state Sleeping) and Pickup / KFWeaponPickup (state Sleeping) for
//! pickups placed directly in a map. docs/DESIGN.md, "Pickups".
//!
//! Ids: spawn points 0.., then ammo boxes, then placed pickups, in map
//! order (`Rules::id_*`).

/// KFRandomSpawn defaults.
const INITIAL_WAIT_TIME: f64 = 20.0;
const RETRY_WAIT_TIME: f64 = 5.0;
/// Pickup.RespawnEffectTime.
const RESPAWN_EFFECT_TIME: f64 = 0.5;

/// A spawn point (KFRandomItemSpawn).
#[derive(Clone, Debug)]
pub struct SpawnPoint {
    /// PickupClasses (up to the first None) with PickupWeight (0 counts as
    /// 1), and whether the class is a KFWeaponPickup (its MySpawner is set,
    /// so taking it calls WeaponPickedUp).
    pub classes: Vec<(String, i32, bool)>,
    /// bIsEnabledNow.
    pub enabled: bool,
    /// The item it put out (myPickup), by class path.
    pub item: Option<String>,
    /// When its Timer() runs next (SetTimer), game seconds.
    timer: Option<f64>,
}

impl SpawnPoint {
    pub fn new(classes: Vec<(String, i32, bool)>) -> Self {
        let classes = classes.into_iter().map(|(c, w, wp)| (c, if w == 0 { 1 } else { w }, wp)).collect();
        // KFRandomItemSpawn.PostBeginPlay: DisableMe (nothing to remove).
        SpawnPoint { classes, enabled: false, item: None, timer: None }
    }

    fn weight_total(&self) -> i32 {
        self.classes.iter().map(|c| c.1).sum()
    }
}

/// An ammo box's place in KFAmmoPickup's states.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AmmoPhase {
    /// Sleeping, label Begin (bSleeping: hidden "forever").
    Asleep,
    /// Sleeping, label DelayedSpawn: Sleep(RespawnTime / players).
    Delayed { until: f64 },
    /// Label Respawn / TryToRespawnAgain: shows once no player has a clear
    /// line to it, checked every second.
    Waiting { next: f64 },
    /// RespawnEffect, Sleep(RespawnEffectTime).
    Effect { until: f64 },
    /// State Pickup: shown, can be taken.
    Shown,
}

#[derive(Clone, Debug)]
pub struct AmmoBox {
    pub class: String,
    pub respawn_time: f32,
    pub phase: AmmoPhase,
}

/// A pickup placed directly in the map (Pickup's / KFWeaponPickup's
/// states).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PlacedPhase {
    Shown,
    /// Sleeping, label Begin: Sleep(RespawnTime - RespawnEffectTime).
    Sleeping { until: f64 },
    /// KFWeaponPickup label Respawn: while PlayerSeezMe, Sleep(5 + 5 x FRand).
    Waiting { next: f64 },
    Effect { until: f64 },
    /// Destroyed (RespawnTime 0: GameInfo.ShouldRespawn false).
    Gone,
}

#[derive(Clone, Debug)]
pub struct Placed {
    pub class: String,
    pub respawn_time: f32,
    /// A KFWeaponPickup (waits while a player sees it before coming back).
    pub weapon_pickup: bool,
    pub phase: PlacedPhase,
}

/// What changed, for the game to draw and log.
#[derive(Clone, Debug, PartialEq)]
pub enum RuleEvent {
    Shown { id: u32, class: String, why: &'static str },
    Hidden { id: u32, why: &'static str },
}

/// What the rules need to know about the players (KF asks the level).
pub trait Senses {
    /// KFRandomSpawn.PlayersCanSeeMe for spawn point / pickup `id`: a
    /// player pawn within 2000 units with a clear line from its eyes.
    fn players_can_see(&self, id: u32) -> bool;
    /// KFAmmoPickup label Respawn: a clear line from the box to a player
    /// pawn (any distance).
    fn line_to_player(&self, id: u32) -> bool;
    /// KFWeaponPickup.PlayerSeezMe (a living player's LineOfSightTo).
    fn player_sees(&self, id: u32) -> bool;
    /// KFAmmoPickup.GetNumPlayers: living players, at least 1.
    fn living_players(&self) -> usize;
    /// GameInfo.GetNumPlayers: connected players.
    fn num_players(&self) -> usize;
}

/// KFGameType.SetupPickups' shares (weapons, ammo) for a GameDifficulty.
pub fn setup_shares(difficulty: f32) -> (f32, f32) {
    if difficulty >= 5.0 {
        (0.1, 0.1)
    } else if difficulty >= 4.0 {
        (0.2, 0.35)
    } else if difficulty >= 2.0 {
        (0.3, 0.5)
    } else {
        (0.5, 0.65)
    }
}

pub struct Rules {
    pub spawns: Vec<SpawnPoint>,
    pub ammo: Vec<AmmoBox>,
    pub placed: Vec<Placed>,
    rng: u32,
    pub events: Vec<RuleEvent>,
}

impl Rules {
    pub fn new(spawns: Vec<SpawnPoint>, ammo: Vec<AmmoBox>, placed: Vec<Placed>) -> Self {
        // Placed pickups start in state Pickup (auto state); ammo boxes
        // sleep (KFAmmoPickup.PostBeginPlay); spawn points are off.
        let mut r = Rules { spawns, ammo, placed, rng: 0x1F2E_3D4C, events: Vec::new() };
        for k in 0..r.placed.len() {
            r.placed[k].phase = PlacedPhase::Shown;
            let id = r.id_placed(k);
            r.events.push(RuleEvent::Shown { id, class: r.placed[k].class.clone(), why: "placed" });
        }
        r
    }

    pub fn id_ammo(&self, j: usize) -> u32 {
        (self.spawns.len() + j) as u32
    }
    pub fn id_placed(&self, k: usize) -> u32 {
        (self.spawns.len() + self.ammo.len() + k) as u32
    }

    /// Rand(n) (0 for n 0).
    fn rand(&mut self, n: usize) -> usize {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        if n == 0 { 0 } else { self.rng as usize % n }
    }

    /// FRand: [0, 1).
    fn frand(&mut self) -> f64 {
        self.rand(1 << 24) as f64 / (1u32 << 24) as f64
    }

    /// The class shown at `id`, if any.
    #[cfg(test)]
    pub fn shown(&self, id: u32) -> Option<&str> {
        let id = id as usize;
        let (s, a) = (self.spawns.len(), self.ammo.len());
        if id < s {
            self.spawns[id].item.as_deref()
        } else if id < s + a {
            (self.ammo[id - s].phase == AmmoPhase::Shown).then_some(self.ammo[id - s].class.as_str())
        } else {
            self.placed.get(id - s - a).filter(|p| p.phase == PlacedPhase::Shown).map(|p| p.class.as_str())
        }
    }

    /// Every shown pickup (id, class).
    #[cfg(test)]
    pub fn all_shown(&self) -> Vec<(u32, String)> {
        let n = (self.spawns.len() + self.ammo.len() + self.placed.len()) as u32;
        (0..n).filter_map(|id| self.shown(id).map(|c| (id, c.to_string()))).collect()
    }

    /// A restart (KF reloads the level): spawn points off with nothing out,
    /// ammo boxes asleep, placed pickups back.
    pub fn reset(&mut self) {
        for i in 0..self.spawns.len() {
            let s = &mut self.spawns[i];
            s.enabled = false;
            s.timer = None;
            if s.item.take().is_some() {
                self.events.push(RuleEvent::Hidden { id: i as u32, why: "restart" });
            }
        }
        for j in 0..self.ammo.len() {
            if self.ammo[j].phase == AmmoPhase::Shown {
                let id = self.id_ammo(j);
                self.events.push(RuleEvent::Hidden { id, why: "restart" });
            }
            self.ammo[j].phase = AmmoPhase::Asleep;
        }
        for k in 0..self.placed.len() {
            if self.placed[k].phase != PlacedPhase::Shown {
                self.placed[k].phase = PlacedPhase::Shown;
                let id = self.id_placed(k);
                self.events.push(RuleEvent::Shown { id, class: self.placed[k].class.clone(), why: "restart" });
            }
        }
    }

    /// KFGameType.SetupPickups. Returns (weapon spawn points switched on,
    /// ammo boxes shown).
    pub fn setup_pickups(&mut self, now: f64, difficulty: f32, senses: &dyn Senses) -> (Vec<u32>, Vec<u32>) {
        let (share_w, share_a) = setup_shares(difficulty);
        // int(Length x share): UnrealScript truncates the float.
        let num_w = (self.spawns.len() as f32 * share_w) as usize;
        let num_a = (self.ammo.len() as f32 * share_a) as usize;
        for i in 0..self.spawns.len() {
            self.disable_me(i, now, senses);
        }
        for j in 0..self.ammo.len() {
            self.ammo_sleep(j, "setup");
        }
        let (mut on_w, mut on_a) = (Vec::new(), Vec::new());
        // One try counter `j` for both loops, as KF's.
        let mut tries = 0;
        let mut i = 0;
        while i < num_w && tries < 10000 {
            let r = self.rand(self.spawns.len());
            if !self.spawns[r].enabled {
                self.enable_me(r, now);
                on_w.push(r as u32);
                i += 1;
            }
            tries += 1;
        }
        let mut i = 0;
        while i < num_a && tries < 10000 {
            let r = self.rand(self.ammo.len());
            if self.ammo[r].phase == AmmoPhase::Asleep {
                self.ammo[r].phase = AmmoPhase::Shown;
                let id = self.id_ammo(r);
                self.events.push(RuleEvent::Shown { id, class: self.ammo[r].class.clone(), why: "setup" });
                on_a.push(id);
                i += 1;
            }
            tries += 1;
        }
        on_w.sort_unstable();
        on_a.sort_unstable();
        (on_w, on_a)
    }

    /// KFRandomItemSpawn.EnableMe: on, Timer in 0.1 s.
    fn enable_me(&mut self, i: usize, now: f64) {
        self.spawns[i].enabled = true;
        self.spawns[i].timer = Some(now + 0.1);
    }

    /// KFRandomItemSpawn.EnableMeDelayed.
    fn enable_me_delayed(&mut self, i: usize, now: f64, delay: f64) {
        self.spawns[i].enabled = true;
        self.spawns[i].timer = Some(now + delay);
    }

    /// KFRandomSpawn.DisableMe.
    fn disable_me(&mut self, i: usize, now: f64, senses: &dyn Senses) {
        self.spawns[i].enabled = false;
        if self.spawns[i].item.is_none() || !senses.players_can_see(i as u32) {
            if self.spawns[i].item.take().is_some() {
                self.events.push(RuleEvent::Hidden { id: i as u32, why: "disabled" });
            }
            self.spawns[i].timer = None;
        } else {
            let d = 1.0 + self.frand() * 5.0;
            self.spawns[i].timer = Some(now + d);
        }
    }

    /// KFRandomSpawn.TurnOn: roll a class, put it out (replacing the old
    /// item), Timer in 20-40 s.
    fn turn_on(&mut self, i: usize, now: f64) {
        let c = self.weighted_class(i);
        let class = self.spawns[i].classes[c].0.clone();
        let rerolled = self.spawns[i].item.is_some();
        self.spawns[i].item = Some(class.clone());
        self.events.push(RuleEvent::Shown { id: i as u32, class, why: if rerolled { "rerolled" } else { "turned_on" } });
        let d = INITIAL_WAIT_TIME + INITIAL_WAIT_TIME * self.frand();
        self.spawns[i].timer = Some(now + d);
    }

    /// KFRandomSpawn.GetWeightedRandClass.
    fn weighted_class(&mut self, i: usize) -> usize {
        let total = self.spawns[i].weight_total().max(0) as usize;
        let rand_index = self.rand(total + 1) as i32;
        let w = &self.spawns[i].classes;
        let mut k = 0;
        let mut tally = w[0].1;
        while tally < rand_index && k + 1 < w.len() {
            k += 1;
            tally += w[k].1;
        }
        k
    }

    /// KFRandomSpawn.Timer.
    fn spawn_timer(&mut self, i: usize, now: f64, senses: &dyn Senses) {
        self.spawns[i].timer = None;
        let id = i as u32;
        if !self.spawns[i].enabled {
            if self.spawns[i].item.is_some() {
                if senses.players_can_see(id) {
                    let d = 1.0 + self.frand();
                    self.spawns[i].timer = Some(now + d);
                    return;
                }
                self.spawns[i].item = None;
                self.events.push(RuleEvent::Hidden { id, why: "disabled_unseen" });
            }
            // Guess: KF falls through into the turn-on check here (see
            // DESIGN.md); we stop.
            return;
        }
        if !senses.players_can_see(id) {
            self.turn_on(i, now);
        } else {
            let d = RETRY_WAIT_TIME + RETRY_WAIT_TIME * self.frand();
            self.spawns[i].timer = Some(now + d);
        }
    }

    fn ammo_sleep(&mut self, j: usize, why: &'static str) {
        if self.ammo[j].phase == AmmoPhase::Shown {
            let id = self.id_ammo(j);
            self.events.push(RuleEvent::Hidden { id, why });
        }
        self.ammo[j].phase = AmmoPhase::Asleep;
    }

    /// The timers.
    pub fn tick(&mut self, now: f64, senses: &dyn Senses) {
        for i in 0..self.spawns.len() {
            if self.spawns[i].timer.is_some_and(|t| now >= t) {
                self.spawn_timer(i, now, senses);
            }
        }
        for j in 0..self.ammo.len() {
            let id = self.id_ammo(j);
            match self.ammo[j].phase {
                AmmoPhase::Delayed { until } if now >= until => self.ammo[j].phase = AmmoPhase::Waiting { next: now },
                AmmoPhase::Waiting { next } if now >= next => {
                    self.ammo[j].phase = if senses.line_to_player(id) { AmmoPhase::Waiting { next: now + 1.0 } } else { AmmoPhase::Effect { until: now + RESPAWN_EFFECT_TIME } };
                }
                AmmoPhase::Effect { until } if now >= until => {
                    self.ammo[j].phase = AmmoPhase::Shown;
                    self.events.push(RuleEvent::Shown { id, class: self.ammo[j].class.clone(), why: "respawned" });
                }
                _ => {}
            }
        }
        for k in 0..self.placed.len() {
            let id = self.id_placed(k);
            match self.placed[k].phase {
                PlacedPhase::Sleeping { until } if now >= until => {
                    self.placed[k].phase = if self.placed[k].weapon_pickup { PlacedPhase::Waiting { next: now } } else { PlacedPhase::Effect { until: now + RESPAWN_EFFECT_TIME } };
                }
                PlacedPhase::Waiting { next } if now >= next => {
                    if senses.player_sees(id) {
                        let d = 5.0 + 5.0 * self.frand();
                        self.placed[k].phase = PlacedPhase::Waiting { next: now + d };
                    } else {
                        self.placed[k].phase = PlacedPhase::Effect { until: now + RESPAWN_EFFECT_TIME };
                    }
                }
                PlacedPhase::Effect { until } if now >= until => {
                    self.placed[k].phase = PlacedPhase::Shown;
                    self.events.push(RuleEvent::Shown { id, class: self.placed[k].class.clone(), why: "respawned" });
                }
                _ => {}
            }
        }
    }

    /// A player took the pickup at `id` (its Touch succeeded). Returns
    /// false if nothing is shown there.
    pub fn take(&mut self, id: u32, now: f64, senses: &dyn Senses) -> bool {
        let i = id as usize;
        let (s, a) = (self.spawns.len(), self.ammo.len());
        if i < s {
            let Some(class) = self.spawns[i].item.take() else { return false };
            self.events.push(RuleEvent::Hidden { id, why: "taken" });
            // Only a KFWeaponPickup knows its spawner (MySpawner); a vest
            // just goes, its spawn point stays on.
            let weapon = self.spawns[i].classes.iter().any(|c| c.0 == class && c.2);
            if weapon {
                self.weapon_picked_up(i, now, senses);
            }
            true
        } else if i < s + a {
            let j = i - s;
            if self.ammo[j].phase != AmmoPhase::Shown {
                return false;
            }
            self.ammo_sleep(j, "taken");
            self.ammo_picked_up(j, now, senses);
            true
        } else {
            let k = i - s - a;
            if self.placed.get(k).is_none_or(|p| p.phase != PlacedPhase::Shown) {
                return false;
            }
            self.events.push(RuleEvent::Hidden { id, why: "taken" });
            let t = self.placed[k].respawn_time as f64;
            self.placed[k].phase = if t != 0.0 { PlacedPhase::Sleeping { until: now + (t - RESPAWN_EFFECT_TIME).max(0.0) } } else { PlacedPhase::Gone };
            true
        }
    }

    /// KFGameType.WeaponPickedUp.
    fn weapon_picked_up(&mut self, i: usize, now: f64, senses: &dyn Senses) {
        self.disable_me(i, now, senses);
        let delay = 30.0 / senses.num_players().max(1) as f64;
        for _ in 0..10000 {
            let r = self.rand(self.spawns.len());
            if r != i && !self.spawns[r].enabled {
                self.enable_me_delayed(r, now, delay);
                return;
            }
        }
        self.enable_me_delayed(i, now, delay);
    }

    /// KFGameType.AmmoPickedUp: another sleeping box wakes (DelayedSpawn).
    fn ammo_picked_up(&mut self, j: usize, now: f64, senses: &dyn Senses) {
        let living = senses.living_players().max(1) as f64;
        let mut target = j;
        for _ in 0..10000 {
            let r = self.rand(self.ammo.len());
            if r != j && self.ammo[r].phase == AmmoPhase::Asleep {
                target = r;
                break;
            }
        }
        let t = self.ammo[target].respawn_time as f64 / living;
        self.ammo[target].phase = AmmoPhase::Delayed { until: now + t };
    }

    /// For the log: (spawn points on, items out, ammo boxes shown, ammo
    /// boxes on their way back).
    pub fn counts(&self) -> (usize, usize, usize, usize) {
        (
            self.spawns.iter().filter(|s| s.enabled).count(),
            self.spawns.iter().filter(|s| s.item.is_some()).count(),
            self.ammo.iter().filter(|a| a.phase == AmmoPhase::Shown).count(),
            self.ammo.iter().filter(|a| matches!(a.phase, AmmoPhase::Delayed { .. } | AmmoPhase::Waiting { .. } | AmmoPhase::Effect { .. })).count(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Blind {
        seen: bool,
        players: usize,
    }

    impl Senses for Blind {
        fn players_can_see(&self, _: u32) -> bool {
            self.seen
        }
        fn line_to_player(&self, _: u32) -> bool {
            self.seen
        }
        fn player_sees(&self, _: u32) -> bool {
            self.seen
        }
        fn living_players(&self) -> usize {
            self.players
        }
        fn num_players(&self) -> usize {
            self.players
        }
    }

    fn default_spawn() -> SpawnPoint {
        // KFRandomItemSpawn's default list.
        SpawnPoint::new(vec![
            ("Dualies".into(), 3, true),
            ("Shotgun".into(), 1, true),
            ("Bullpup".into(), 3, true),
            ("Deagle".into(), 3, true),
            ("Winchester".into(), 2, true),
            ("Axe".into(), 1, true),
            ("Machete".into(), 1, true),
            ("Vest".into(), 2, false),
        ])
    }

    fn rules(spawns: usize, ammo: usize) -> Rules {
        Rules::new(
            (0..spawns).map(|_| default_spawn()).collect(),
            (0..ammo).map(|_| AmmoBox { class: "Ammo".into(), respawn_time: 30.0, phase: AmmoPhase::Asleep }).collect(),
            vec![],
        )
    }

    #[test]
    fn setup_switches_on_kf_shares() {
        let mut r = rules(14, 14);
        let unseen = Blind { seen: false, players: 1 };
        // Normal: int(14 x 0.3) = 4 spawn points, int(14 x 0.5) = 7 boxes.
        let (w, a) = r.setup_pickups(0.0, 2.0, &unseen);
        assert_eq!((w.len(), a.len()), (4, 7));
        assert_eq!(r.counts(), (4, 0, 7, 0));
        // Items come out at the 0.1 s timer when nobody sees the spot.
        r.tick(0.2, &unseen);
        assert_eq!(r.counts(), (4, 4, 7, 0));
        // Beginner / Hard / Suicidal shares.
        assert_eq!(setup_shares(1.0), (0.5, 0.65));
        assert_eq!(setup_shares(4.0), (0.2, 0.35));
        assert_eq!(setup_shares(5.0), (0.1, 0.1));
    }

    #[test]
    fn seen_spawn_point_waits() {
        let mut r = rules(10, 0);
        let seen = Blind { seen: true, players: 1 };
        r.setup_pickups(0.0, 2.0, &seen);
        r.tick(0.2, &seen);
        assert_eq!(r.counts().1, 0);
        // Retries 5-10 s later; then unseen, it turns on.
        r.tick(4.9, &Blind { seen: false, players: 1 });
        assert_eq!(r.counts().1, 0);
        r.tick(10.2, &Blind { seen: false, players: 1 });
        assert_eq!(r.counts().1, 3);
    }

    #[test]
    fn taking_a_weapon_moves_the_spawn() {
        let mut r = rules(10, 0);
        let unseen = Blind { seen: false, players: 2 };
        let (on, _) = r.setup_pickups(0.0, 2.0, &unseen);
        r.tick(0.2, &unseen);
        // Make the first one a weapon for sure.
        let first = on[0] as usize;
        r.spawns[first].item = Some("Shotgun".into());
        assert!(r.take(first as u32, 1.0, &unseen));
        assert!(!r.take(first as u32, 1.0, &unseen), "already taken");
        assert!(!r.spawns[first].enabled);
        // Another one is on at once, its item 30 / 2 = 15 s later (t = 16).
        assert_eq!(r.counts(), (3, 2, 0, 0));
        r.tick(15.9, &unseen);
        assert_eq!(r.counts(), (3, 2, 0, 0));
        r.tick(16.1, &unseen);
        assert_eq!(r.counts(), (3, 3, 0, 0));
    }

    #[test]
    fn taking_a_vest_keeps_the_spawn_on() {
        let mut r = rules(10, 0);
        let unseen = Blind { seen: false, players: 1 };
        let (on, _) = r.setup_pickups(0.0, 2.0, &unseen);
        r.tick(0.2, &unseen);
        let first = on[0] as usize;
        r.spawns[first].item = Some("Vest".into());
        assert!(r.take(first as u32, 1.0, &unseen));
        assert!(r.spawns[first].enabled);
        // Its 20-40 s timer puts a new item out.
        r.tick(41.0, &unseen);
        assert!(r.spawns[first].item.is_some());
    }

    #[test]
    fn ammo_wakes_another_box_after_respawn_time() {
        let mut r = rules(0, 4);
        let unseen = Blind { seen: false, players: 1 };
        let (_, a) = r.setup_pickups(0.0, 2.0, &unseen);
        assert_eq!(a.len(), 2);
        assert!(r.take(a[0], 1.0, &unseen));
        assert_eq!(r.counts(), (0, 0, 1, 1));
        // 30 s, the line check, then the 0.5 s effect.
        r.tick(30.9, &unseen);
        assert_eq!(r.counts(), (0, 0, 1, 1));
        r.tick(31.0, &unseen);
        r.tick(31.1, &unseen);
        r.tick(31.5, &unseen);
        assert_eq!(r.counts(), (0, 0, 1, 1));
        r.tick(31.7, &unseen);
        assert_eq!(r.counts(), (0, 0, 2, 0));
        // A seen box waits.
        let mut r = rules(0, 4);
        let (_, a) = r.setup_pickups(0.0, 2.0, &unseen);
        let seen = Blind { seen: true, players: 1 };
        r.take(a[0], 1.0, &seen);
        r.tick(31.0, &seen);
        r.tick(40.0, &seen);
        assert_eq!(r.counts(), (0, 0, 1, 1));
    }

    #[test]
    fn weighted_class_matches_kf_tally() {
        let mut r = rules(1, 0);
        let mut seen = [0; 8];
        for _ in 0..16000 {
            seen[r.weighted_class(0)] += 1;
        }
        // Total weight 16, Rand(17): index 0 for 0..=3 (4 of 17), the
        // others by weight; the last (Vest) also takes Rand = 16.
        assert!(seen.iter().all(|&n| n > 0));
        assert!(seen[0] > seen[1] * 3, "{seen:?}");
    }

    #[test]
    fn placed_pickups_come_back_after_respawn_time() {
        let mut r = Rules::new(vec![], vec![], vec![Placed { class: "Deagle".into(), respawn_time: 100.0, weapon_pickup: true, phase: PlacedPhase::Shown }]);
        let unseen = Blind { seen: false, players: 1 };
        assert_eq!(r.all_shown().len(), 1);
        assert!(r.take(0, 0.0, &unseen));
        r.tick(99.6, &unseen);
        r.tick(99.7, &unseen);
        r.tick(100.3, &unseen);
        assert_eq!(r.all_shown().len(), 1);
    }
}
