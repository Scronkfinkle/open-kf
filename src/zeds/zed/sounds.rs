//! Zed voices and loops (S4b, S4b2): the class's sound names and playing a zed's sound events. See DESIGN.md, "Sound and music".

use super::*;

/// ZombieBoss's speech functions, called by AnimNotify_Script notifies in
/// his animations: PlaySound(SoundGroup'...', SLOT_Misc, 2.0, true, R).
/// PatriarchRadialTaunt needs 3 players around him: never solo.
pub(super) fn boss_speech(function: &str) -> Option<Line> {
    let (sound, radius) = match function {
        "PatriarchKnockDown" => ("KF_EnemiesFinalSnd.Patriarch.Kev_KnockedDown", 500.0),
        "PatriarchEntrance" => ("KF_EnemiesFinalSnd.Patriarch.Kev_Entrance", 500.0),
        "PatriarchVictory" => ("KF_EnemiesFinalSnd.Patriarch.Kev_Victory", 500.0),
        "PatriarchMGPreFire" => ("KF_EnemiesFinalSnd.Patriarch.Kev_WarnGun", 1000.0),
        "PatriarchMisslePreFire" => ("KF_EnemiesFinalSnd.Patriarch.Kev_WarnRocket", 1000.0),
        _ => return None,
    };
    Some(Line { sound, slot: crate::audio::mixer::Slot::Misc, volume: 2.0, radius, no_override: true })
}

pub(super) fn load_zed_sounds(defaults: &ClassDefaults, class: &ObjectHandle, kind: ZedKind) -> ZedSounds {
    use crate::weapons::weapon::sound_prop;
    let float = |p: &str, d: f32| match defaults.get(class, p) {
        Some((Value::Float(f), _)) => f,
        _ => d,
    };
    let object_at = |p: &str, i: u32| match defaults.get_at(class, p, i) {
        Some((Value::Object(r @ ObjectRef::Import(_)), lp)) => Some(lp.pkg.object_path(r)),
        Some((Value::Object(r @ ObjectRef::Export(_)), lp)) => Some(format!("{}.{}", lp.name, lp.pkg.object_path(r))),
        _ => None,
    };
    let bloat = kind == ZedKind::Bloat;
    let boss = kind == ZedKind::Patriarch;
    let fleshpound = kind == ZedKind::Fleshpound;
    let ambient = sound_prop(defaults, class, "AmbientSound").map(|sound| AmbientLoop {
        sound,
        volume: match defaults.get(class, "SoundVolume") {
            Some((Value::Byte(b), _)) => b,
            _ => 128,
        },
        radius: float("SoundRadius", 64.0) * float("AmbientSoundScaling", 1.0),
    });
    ZedSounds {
        moan: sound_prop(defaults, class, "MoanVoice"),
        moan_volume: float("MoanVolume", 1.5),
        pain: object_at("HitSound", 0),
        pain_volume: if boss { 2.0 * float("TransientSoundVolume", 1.0) } else { 1.25 },
        pain_on_fire: boss || fleshpound || kind == ZedKind::Scrake,
        death: if bloat { Some("KF_EnemiesFinalSnd.Bloat_DeathPop".into()) } else { object_at("DeathSound", 0) },
        death_volume: if bloat { 2.0 } else { 1.3 },
        headless_death: sound_prop(defaults, class, "HeadlessDeathSound"),
        decapitation: sound_prop(defaults, class, "DecapitationSound"),
        challenge: (0..4).filter_map(|i| object_at("ChallengeSound", i)).collect(),
        melee_hit: sound_prop(defaults, class, "MeleeAttackHitSound"),
        melee_hit_volume: if fleshpound { 1.25 } else { 2.0 },
        ambient,
        saw_loop: sound_prop(defaults, class, "SawAttackLoopSound"),
        chainsaw_off: sound_prop(defaults, class, "ChainSawOffSound"),
        rocket_fire: sound_prop(defaults, class, "RocketFireSound"),
        impale_hit: sound_prop(defaults, class, "MeleeImpaleHitSound"),
        mg_fire: sound_prop(defaults, class, "MiniGunFireSound"),
        mg_spin: sound_prop(defaults, class, "MiniGunSpinSound"),
    }
}

/// Plays the zed's sound events of this frame (KFMonster's calls; see
/// `ZedSounds`) and keeps its AmbientSound in step: the class's loop while
/// alive with a head (PlayDying and RemoveHead clear it; RemoveHead sets
/// MiscSound, None for every zed), the Scrake's saw loop while sawing.
pub(super) fn play_zed_sounds(commands: &mut Commands, entity: Entity, c: &ZedClass, z: &mut Zed, out: &mut MessageWriter<crate::audio::mixer::PlaySound>) {
    use crate::audio::mixer::{Emitter, PlaySound, Slot};
    let snd = &c.sounds;
    let at = Emitter::Entity(entity);
    for ev in std::mem::take(&mut z.sound_events) {
        let play = match ev {
            ZedSound::Pain => snd.pain.clone().map(|p| PlaySound::new(p, at).slot(Slot::Pain).volume(snd.pain_volume).radius(400.0)),
            // PlayDyingSound: SLOT_Pain, bNoOverride, radius 525.
            ZedSound::Death => {
                if let Some(off) = snd.chainsaw_off.clone() {
                    out.write(PlaySound::new(off, at).slot(Slot::Misc).volume(2.0).radius(525.0));
                }
                if z.decapitated {
                    snd.headless_death.clone().map(|p| PlaySound::new(p, at).slot(Slot::Pain).volume(1.3).radius(525.0).no_override())
                } else {
                    snd.death.clone().map(|p| PlaySound::new(p, at).slot(Slot::Pain).volume(snd.death_volume).radius(525.0).no_override())
                }
            }
            ZedSound::Decapitation => snd.decapitation.clone().map(|p| PlaySound::new(p, at).slot(Slot::Misc).volume(1.3).radius(525.0).no_override()),
            ZedSound::Skull => Some(PlaySound::new("KF_EnemyGlobalSndTwo.Impact_Skull", at).volume(2.0).radius(500.0).no_override()),
            // Monster: TransientSoundRadius 500.
            ZedSound::MeleeHit => snd.melee_hit.clone().map(|p| PlaySound::new(p, at).slot(Slot::Interact).volume(snd.melee_hit_volume).radius(500.0)),
            ZedSound::Moan => snd.moan.clone().map(|p| PlaySound::new(p, at).slot(Slot::Misc).volume(snd.moan_volume).radius(250.0)),
            ZedSound::Challenge if !snd.challenge.is_empty() => {
                let pick = z.random() as usize % snd.challenge.len();
                Some(PlaySound::new(snd.challenge[pick].clone(), at).slot(Slot::Talk).volume(1.0).radius(500.0))
            }
            ZedSound::Challenge => None,
            ZedSound::Line(l) => {
                let p = PlaySound::new(l.sound, at).slot(l.slot).volume(l.volume).radius(l.radius);
                Some(if l.no_override { p.no_override() } else { p })
            }
            ZedSound::Rocket => snd.rocket_fire.clone().map(|p| PlaySound::new(p, at).slot(Slot::Interact).volume(2.0).radius(500.0)),
            ZedSound::ImpaleHit => snd.impale_hit.clone().map(|p| PlaySound::new(p, at).slot(Slot::Interact).volume(2.0).radius(500.0)),
            ZedSound::Land(v) => Some(PlaySound::new("KF_PlayerGlobalSnd.Player_LandDirt", at).slot(Slot::Interact).volume(v).radius(500.0)),
        };
        if let Some(p) = play {
            out.write(p);
        }
    }
    let mg = z.boss.as_ref().and_then(|b| b.chaingun).map(|g| g.sound);
    let want = if z.health > 0.0 && !z.decapitated {
        match (&snd.saw_loop, &snd.ambient) {
            _ if mg == Some(crate::zeds::boss::MgSound::Fire) && snd.mg_fire.is_some() => {
                snd.mg_fire.clone().map(|sound| AmbientLoop { sound, volume: 255, radius: 400.0 })
            }
            _ if mg == Some(crate::zeds::boss::MgSound::Spin) && snd.mg_spin.is_some() => {
                snd.mg_spin.clone().map(|sound| AmbientLoop { sound, volume: 185, radius: 200.0 })
            }
            (Some(saw), Some(a)) if z.sawing => Some(AmbientLoop { sound: saw.clone(), ..a.clone() }),
            (_, a) => a.clone(),
        }
    } else {
        None
    };
    if want.as_ref().map(|a| &a.sound) != z.ambient_on.as_ref() {
        match &want {
            Some(a) => commands.entity(entity).insert(crate::audio::mixer::AmbientSound { sound: a.sound.clone(), volume: a.volume, radius: a.radius, pitch: 64, at_listener: false, ..default() }),
            None => commands.entity(entity).remove::<crate::audio::mixer::AmbientSound>(),
        };
        z.ambient_on = want.map(|a| a.sound);
    }
}
