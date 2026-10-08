//! The Zed component's methods: hits and hit reactions, death, decapitation, the Patriarch's health levels, test constructors.

use super::*;

impl Zed {
    /// Marks the zed dead (death animation is started by the think system).
    pub fn kill(&mut self) {
        // ZombieStalker.PlayDying: the corpse shows the normal skin.
        if self.cloaked {
            self.cloaked = false;
            self.cloak_dirty = true;
        }
        self.clear_glow();
        self.spotted = false;
        self.health = 0.0;
        self.bleed_out = None;
        self.state = ZedState::Dead;
        self.dead_for = 0.0;
    }

    /// KFMonster.RemoveHead (the damage part is in combat.rs): the head is
    /// gone; if the zed is still alive it bleeds out after BleedOutDuration.
    pub fn remove_head(&mut self) {
        // ZombieStalker.RemoveHead: back to the normal skin ("No head, no
        // cloak").
        if self.cloaked {
            self.cloaked = false;
            self.cloak_dirty = true;
        }
        self.clear_glow();
        self.decapitated = true;
        self.head_health = 0.0;
        self.sound_events.push(ZedSound::Decapitation);
        self.since_decap = Some(0.0);
        if self.headless_claws {
            self.melee_damage *= 2.0;
            self.melee_range *= 2.0;
        }
        // RunningState.RemoveHead: stop running; RangedAttack never starts
        // it again once headless.
        self.running = false;
        self.run_attack_timeout = 0.0;
        if self.health > 0.0 {
            self.bleed_out = Some(self.bleed_out_duration);
            // ZombieSiren.RemoveHead: half the time she dies at once
            // (KilledBy), else within 10 x FRand() s.
            if self.quick_headless_death {
                let roll = (self.random() % 1000) as f32 / 1000.0;
                let when = if roll < 0.5 { 0.0 } else { 10.0 * (self.random() % 1000) as f32 / 1000.0 };
                self.bleed_out = Some(when);
            }
        }
        // ZombieScrake RunningState.RemoveHead: the rage ends.
        self.raging = false;
    }

    /// ZombieBoss.TakeDamage: knocked down below the next healing level.
    pub fn note_boss_health(&mut self) {
        let health = self.health;
        if let Some(b) = self.boss.as_mut()
            && b.check_knockdown(health)
        {
            runlog::kv(
                "boss_knockdown",
                &format!("id={} asked health={health:.0} level={} syringes={}", self.id, b.healing_levels[b.syringes], b.syringes),
            );
        }
    }

    /// ZombieBoss FireChaingun.TakeDamage: who shot him, `distance` from
    /// him in Unreal units.
    pub fn note_attacker_distance(&mut self, distance: f32) {
        if let Some(b) = self.boss.as_mut()
            && distance < crate::zeds::boss::MG_CLOSE_DAMAGE_DISTANCE
        {
            b.hit_from_close = true;
        }
    }

    /// ZombieFleshPound.TakeDamage: health lost within 2 s of the previous
    /// hit adds up (TwoSecondDamageTotal); over the threshold, with the head
    /// on and not raging already, the Fleshpound starts to rage.
    pub fn note_damage(&mut self, lost: f32) {
        let threshold = self.fp_rage_threshold;
        if threshold <= 0.0 {
            return;
        }
        if self.fp_since_damaged > 2.0 {
            self.fp_two_sec_damage = 0.0;
        }
        self.fp_since_damaged = 0.0;
        self.fp_two_sec_damage += lost;
        if self.fp_two_sec_damage > threshold && !self.decapitated && self.fp_rage.is_none() && !self.zapped() {
            self.fp_start_rage = true;
        }
    }

    /// bZapped.
    pub(crate) fn zapped(&self) -> bool {
        self.remaining_zap > 0.0
    }

    /// KFMonster.SetZapped: zapped already: back to a full ZapDuration;
    /// otherwise TotalZap grows, and at ZapThreshold the zed is zapped.
    pub(crate) fn set_zapped(&mut self, amount: f32) {
        self.since_zap = 0.0;
        if self.zapped() {
            self.total_zap = self.zap_threshold;
            self.remaining_zap = self.zap.duration;
        } else {
            self.total_zap += amount;
            if self.total_zap >= self.zap_threshold {
                self.remaining_zap = self.zap.duration;
                runlog::kv(
                    "zed_zapped",
                    &format!("zed={} threshold={:.2} duration={}", self.id, self.zap_threshold, self.zap.duration),
                );
            }
        }
    }

    /// KFMonster.Tick: a zap runs out (the threshold then grows x
    /// ZapResistanceScale); zap taken but not enough fades 1 per second
    /// once none came for 0.1 s. Returns true when a zap wears off.
    pub(super) fn zap_tick(&mut self, dt: f32) -> bool {
        self.since_zap += dt;
        if self.zapped() {
            self.remaining_zap -= dt;
            if self.remaining_zap <= 0.0 {
                self.remaining_zap = 0.0;
                self.zap_threshold *= self.zap.resistance;
                return true;
            }
        } else if self.total_zap > 0.0 && self.since_zap > 0.1 {
            self.total_zap = (self.total_zap - dt).max(0.0);
        }
        false
    }

    pub(super) fn random(&mut self) -> u32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        self.rng
    }

    /// ZombieGoreFast running, once per think: `dist` to the target (Unreal
    /// units), whether an attack is in progress (bShotAnim). Returns
    /// "started" or the reason it stopped, for the log.
    pub(super) fn update_running(&mut self, dist: f32, attacking: bool, dt: f32) -> Option<&'static str> {
        if !self.can_run || self.decapitated {
            return None;
        }
        if !self.running {
            // RunningState.BeginState: not while zapped.
            if self.zapped() {
                return None;
            }
            // RangedAttack: no attack started and the target within 700.
            if attacking || dist > GOREFAST_RUN_DISTANCE {
                return None;
            }
            self.running = true;
            self.charge_check = 0.0;
            self.run_attack_timeout = 0.0;
            return Some("started");
        }
        // RunningState.Tick: a moving attack ends the run when its time is up.
        if self.run_attack_timeout > 0.0 {
            self.run_attack_timeout -= dt;
            if self.run_attack_timeout <= 0.0 {
                self.run_attack_timeout = 0.0;
                self.running = false;
                return Some("run_attack_done");
            }
        }
        // CheckCharge: still within 700? Then sleep 0.5 + FRand() x 0.5.
        self.charge_check -= dt;
        if self.charge_check <= 0.0 {
            if dist < GOREFAST_RUN_DISTANCE {
                self.charge_check = 0.5 + 0.5 * (self.random() % 1000) as f32 / 1000.0;
            } else {
                self.running = false;
                return Some("target_far");
            }
        }
        None
    }

    /// The reaction to one damage event that left the zed alive, after
    /// KFMonster.PlayHit (FlipOver), PlayTakeHit and PlayDirectionalHit.
    /// `damage` includes any headshot multiplier; `hit` is the hit point and
    /// `attacker` the attacker's centre (Bevy space). Returns what was chosen.
    pub fn take_hit(&mut self, damage: f32, hit: Vec3, attacker: Vec3, melee: bool) -> Option<HitReaction> {
        if self.health <= 0.0 || damage <= 0.0 {
            return None;
        }
        // PlayTakeHit's pain sound: after the 0.5 s pain-animation gate and
        // the 0.35 s pain-sound gate (so 0.5 s), not for fire damage
        // (except the Fleshpound, Scrake and Patriarch). Ours is checked
        // before the hit-reaction rules below (a simplification).
        if self.since_pain_sound >= MIN_TIME_BETWEEN_PAIN_ANIMS {
            self.since_pain_sound = 0.0;
            if !self.hit_by_fire || self.pain_on_fire {
                self.sound_events.push(ZedSound::Pain);
            }
        }
        // ZombieBloat.HitCanInterruptAction: no hit reaction mid-attack.
        if self.no_hit_reactions {
            return None;
        }
        if self.uninterruptible && self.attack.is_some() {
            return None;
        }
        // PlayHit: a hit of more than Health / 1.5 knocks the zed down.
        if damage > self.default_health / 1.5 && self.can_flip {
            self.pending_reaction = Some(HitReaction::KnockDown);
            return self.pending_reaction;
        }
        // PlayTakeHit: at most one pain animation every 0.5 s; under 5
        // damage no animation for our damage types (9mm, knife).
        if self.since_pain_anim < MIN_TIME_BETWEEN_PAIN_ANIMS {
            return None;
        }
        self.since_pain_anim = 0.0;
        if damage < self.flinch_min_damage {
            return None;
        }
        // PlayDirectionalHit, in Unreal axes: X = facing, Y = right.
        let rel = (hit - self.centre) / SCALE;
        let mut dir = Vec2::new(-rel.z, rel.x);
        if dir.length() < 1.0 {
            let a = (self.random() % 3600) as f32 / 3600.0 * std::f32::consts::TAU;
            dir = Vec2::new(a.cos(), a.sin());
        }
        let dir = dir.normalize();
        let yaw = self.yaw * std::f32::consts::TAU / 65536.0;
        let (x, y) = (Vec2::new(yaw.cos(), yaw.sin()), Vec2::new(-yaw.sin(), yaw.cos()));
        let reaction = if dir.dot(x) > 0.7 {
            let close_melee = melee
                && (attacker - self.centre).length() / SCALE <= self.melee_range * 2.0
                && damage > 0.1 * self.default_health;
            if damage >= 0.5 * self.default_health || close_melee {
                self.stunned = STUN_TIME;
                HitReaction::Stun
            } else {
                HitReaction::Front
            }
        } else if dir.dot(x) < -0.7 {
            HitReaction::Back
        } else if dir.dot(y) > 0.0 {
            HitReaction::Right
        } else {
            HitReaction::Left
        };
        self.pending_reaction = Some(reaction);
        Some(reaction)
    }

    /// An explosion's push on a zed that survived it (KFMonster.TakeDamage
    /// keeps the momentum only for the frag, pipe bomb and M79 family
    /// damage types; Pawn.TakeDamage; KFMonster / Pawn.AddVelocity).
    /// `momentum` is Unreal units in Unreal axes. On the ground (walking)
    /// with bExtraMomentumZ its upward part is raised to at least 0.4 x its
    /// size; it is divided by the class Mass; pushes of 50 units/s or less
    /// are ignored; a walking zed starts falling; if it already rises
    /// faster than 380 the upward part is halved; it adds to the velocity.
    /// Returns the velocity added (Unreal units/s) and the new velocity,
    /// or None if nothing happened. See DESIGN.md "Combat physics fixes",
    /// CP-5.
    pub fn knockback(&mut self, momentum: Vec3, extra_z: bool) -> Option<(Vec3, Vec3)> {
        // A network puppet is moved by its host.
        if self.health <= 0.0 || self.net.puppet {
            return None;
        }
        // States whose movement is the falling code's (others are scripted
        // moves: knocked down, raging, the Patriarch's moves, door bashing;
        // not pushed here, a simplification).
        if !matches!(self.state, ZedState::Chase | ZedState::Idle | ZedState::Melee | ZedState::Falling | ZedState::Landing) {
            return None;
        }
        let walking = self.state != ZedState::Falling;
        let mut m = momentum;
        if walking && extra_z {
            m.z = m.z.max(0.4 * m.length());
        }
        m /= self.mass.max(1.0);
        if m.length() <= 50.0 {
            return None;
        }
        let mut vel = if walking {
            ue_dir(self.velocity) / SCALE
        } else {
            ue_dir(self.air_velocity + Vec3::Y * self.vertical_speed) / SCALE
        };
        if vel.z > 380.0 && m.z > 0.0 {
            m.z *= 0.5;
        }
        vel += m;
        let bevy = coords::dir(vel.to_array()) * SCALE;
        self.air_velocity = bevy.with_y(0.0);
        self.vertical_speed = bevy.y;
        if walking {
            self.state = ZedState::Falling;
            self.attack = None;
        }
        Some((m, vel))
    }

    /// A Patriarch for the hit rules (health 4000, head 25 x 1.3 x ...;
    /// only the fields the hit code reads), for tests.
    #[cfg(test)]
    pub fn test_patriarch() -> Zed {
        Zed {
            health: 4000.0,
            health_max: 4000.0,
            default_health: 4000.0,
            keeps_head: true,
            no_hit_reactions: true,
            ..Zed::test_clot()
        }
    }

    /// A Clot with KF's values, for tests.
    #[cfg(test)]
    pub fn test_clot() -> Zed {
        Zed {
            id: 0,
            class: 0,
            centre: Vec3::ZERO,
            radius: 26.0,
            half_height: 44.0,
            health: 130.0,
            health_max: 130.0,
            head_health: 25.0,
            decapitated: false,
            bleed_out: None,
            bleed_out_duration: 5.0,
            overlay: None,
            attack: None,
            since_decap: None,
            pending_reaction: None,
            since_pain_anim: f32::MAX,
            stunned: 0.0,
            default_health: 130.0,
            melee_range: 20.0,
            melee_damage: 6.0,
            headless_claws: true,
            can_run: false,
            running: false,
            charge_check: 0.0,
            run_attack_timeout: 0.0,
            rng: 12345,
            head_radius: 7.7,
            head: None,
            ext: None,
            dead_for: 0.0,
            last_hit: None,
            velocity: Vec3::ZERO,
            gore_hits: Vec::new(),
            stumps: Vec::new(),
            hidden_bones: Vec::new(),
            severed: Vec::new(),
            last_pose: Vec::new(),
            anim: ZedAnim::default(),
            next_piece: 0,
            effects: Vec::new(),
            since_hit: f32::MAX,
            router: Default::default(),
            air_velocity: Vec3::ZERO,
            mass: 100.0,
            motion: Default::default(),
            jump_cooldown: 0.0,
            door_bash: None,
            door_checked: None,
            on_pad: None,
            last_seen: f32::MIN,
            last_render: f32::MIN,
            last_view_check: f32::MIN,
            hidden: false,
            pouncing: false,
            since_pounce: f32::MAX,
            can_flip: true,
            sawing: false,
            saw_charging: false,
            raging: false,
            flinch_min_damage: 5.0,
            uninterruptible: false,
            quick_headless_death: false,
            ranged_wait: 0.0,
            bled_out: false,
            burst_done: false,
            pending_fx: Vec::new(),
            fp_two_sec_damage: 0.0,
            fp_since_damaged: f32::MAX,
            fp_start_rage: false,
            fp_rage: None,
            fp_frustration: 0.0,
            fp_frustration_limit: 10.0,
            fp_frustrated: false,
            fp_rage_threshold: 0.0,
            small_arms_scale: 1.0,
            motion_threat: 1.0,
            burn_down: 0,
            last_burn_damage: 0.0,
            heat: 0,
            fire_class: crate::game::combat::FireType::Flamethrower,
            burn_timer: 0.0,
            burn_vet: crate::game::perks::Vet::default(),
            burn_fx: None,
            burned_scale: 1.0,
            fire_resist: 1.0,
            total_zap: 0.0,
            remaining_zap: 0.0,
            since_zap: 1e6,
            zap_threshold: ZapValues::default().threshold,
            zap: ZapValues::default(),
            run_speed_lost: false,
            keeps_head: false,
            no_hit_reactions: false,
            boss: None,
            braindead: false,
            scoring_value: 7.0,
            killed_by_player: false,
            kill_paid: false,
            headshot_kill: false,
            zed_time_rolled: false,
            mg_flash: None,
            mg_flash_shots: 0,
            cloaked: false,
            since_uncloak: f32::MAX,
            cloak_check: 0.0,
            cloak_dirty: false,
            spotted: false,
            glow: false,
            yaw: 0.0,
            state: ZedState::Idle,
            vertical_speed: 0.0,
            sequence: None,
            frame: 0.0,
            looping: true,
            sounds_heard: (None, -1.0),
            overlay_sounds_heard: None,
            sound_events: Vec::new(),
            moan_at: -1.0,
            since_pain_sound: f32::MAX,
            hit_by_fire: false,
            last_challenge: f32::MIN,
            challenge_check: 0.0,
            ambient_on: None,
            pain_on_fire: false,
            meshes: Vec::new(),
            net: ZedNetSide::default(),
        }
    }

    /// A Gorefast's running state with KF's values, for tests.
    #[cfg(test)]
    pub fn test_gorefast() -> Zed {
        Zed {
            can_run: true,
            headless_claws: false,
            ..Zed::test_clot()
        }
    }

    #[cfg(test)]
    pub fn melee_values(&self) -> (f32, f32) {
        (self.melee_range, self.melee_damage)
    }

    /// Seconds since the player last saw this zed (CanKillMeYet's test).
    pub fn unseen_for(&self, now: f32) -> f32 {
        now - self.last_seen
    }

    pub fn is_dead(&self) -> bool {
        self.state == ZedState::Dead
    }

    /// Patriarch: knockdowns finished so far and SyringeCount.
    /// The Patriarch's bShotAnim for his entrance and laugh.
    pub fn boss_shot_anim(&self) -> bool {
        self.boss.is_some_and(|b| b.shot_anim())
    }

    pub fn boss_knockdowns(&self) -> Option<(u32, usize)> {
        self.boss.map(|b| (b.knockdowns_done, b.syringes))
    }

    /// The collision cylinder, if it still blocks (corpses do not).
    pub fn blocking_cylinder(&self) -> Option<Cylinder> {
        (self.state != ZedState::Dead).then_some(Cylinder {
            centre: self.centre,
            radius: self.radius * SCALE,
            half_height: self.half_height * SCALE,
        })
    }
}
