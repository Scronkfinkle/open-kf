//! The Commando's view of cloaked zeds: a "spotted" (KFMonster.bSpotted)
//! Stalker or Patriarch is drawn with KF's red glow (FinalBlend
//! KFX.StalkerGlow) instead of the cloak. See DESIGN.md, "Commando: seeing
//! cloaked zeds".

use super::*;
use crate::game::perks::Vet;
use ue_assets::properties::read_export_properties;
use ue_assets::texture::{decode_rgba, read_texture};

/// ZombieStalker.Tick: spotted when the squared distance is below
/// GetStalkerViewDistanceMulti x 640000 (800 units squared).
const STALKER_VIEW_DISTANCE_SQ: f32 = 640000.0;
/// ZombieStalker.Tick: the check runs every 0.5 s (NextCheckTime).
const STALKER_CHECK_INTERVAL: f32 = 0.5;
/// ZombieStalker.Tick: she cloaks again (or glows) 1.2 s after the last
/// uncloak (LastUncloakTime).
const RECLOAK_DELAY: f32 = 1.2;
/// ZombieBoss.Tick: VisibleCollidingActors(class'KFHumanPawn', HP, 1000).
const BOSS_SPOT_RADIUS: f32 = 1000.0;
/// ZombieBoss.Tick: LastCheckTimes = Level.TimeSeconds + 0.8.
const BOSS_CHECK_INTERVAL: f32 = 0.8;

/// The player whose eyes decide whether cloaked zeds are spotted. In KF
/// each client checks its own player (ZombieStalker.LocalKFHumanPawn), so
/// being spotted is part of what one player sees, not of the zed itself;
/// passing the viewer in (instead of reading the player here) keeps that
/// open for more players later.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CloakViewer {
    /// The pawn's Location (cylinder centre), Bevy coordinates.
    pub location: Vec3,
    /// Health > 0.
    pub alive: bool,
    pub vet: Vet,
}

impl CloakViewer {
    /// Unreal units from the viewer to `at` (Bevy).
    pub fn distance(&self, at: Vec3) -> f32 {
        (at - self.location).length() / SCALE
    }

    /// ZombieStalker.Tick's test (no line-of-sight check: through walls too).
    pub fn spots_stalker(&self, at: Vec3) -> bool {
        let d = self.distance(at);
        self.alive && self.vet.shows_stalkers() && d * d < self.vet.stalker_view_distance_multi() * STALKER_VIEW_DISTANCE_SQ
    }

    /// ZombieBoss.Tick's test: any Commando level within 1000 units and in
    /// sight (`in_sight`: VisibleCollidingActors' trace, asked only when
    /// the rest passes).
    pub fn spots_boss(&self, at: Vec3, in_sight: impl FnOnce() -> bool) -> bool {
        self.alive && self.vet.shows_stalkers() && self.distance(at) < BOSS_SPOT_RADIUS && in_sight()
    }
}

impl Zed {
    fn log_spotted(&self, viewer: Option<&CloakViewer>, kind: &str) {
        runlog::kv(
            "zed_spotted",
            &format!(
                "id={} kind={kind} spotted={} distance={:.0} perk={}",
                self.id,
                self.spotted,
                viewer.map_or(-1.0, |v| v.distance(self.centre)),
                viewer.map_or("none".into(), |v| v.vet.label())
            ),
        );
    }

    /// ZombieStalker.Tick (the client part): every 0.5 s, spotted or not,
    /// then glow, uncloak or cloak, copied branch by branch. Not while
    /// zapped (the next check comes as soon as the zap ends) or dead.
    pub(super) fn stalker_cloak_tick(&mut self, viewer: Option<&CloakViewer>, dt: f32) {
        self.since_uncloak = (self.since_uncloak + dt).min(1e6);
        if self.zapped() {
            self.cloak_check = 0.0;
            return;
        }
        if self.health <= 0.0 {
            return;
        }
        self.cloak_check -= dt;
        if self.cloak_check > 0.0 {
            return;
        }
        self.cloak_check = STALKER_CHECK_INTERVAL;
        let spotted = viewer.is_some_and(|v| v.spots_stalker(self.centre));
        if spotted != self.spotted {
            self.spotted = spotted;
            self.log_spotted(viewer, "stalker");
        }
        if !spotted && !self.cloaked && self.glow {
            // UncloakStalker: the normal skin, the 1.2 s timer restarts.
            self.glow = false;
            self.since_uncloak = 0.0;
            self.cloak_dirty = true;
            runlog::kv("stalker_glow", &format!("id={} on=false reason=unspotted", self.id));
        } else if self.since_uncloak > RECLOAK_DELAY {
            if spotted && !self.glow {
                // CloakStalker, spotted: only the glow (bCloaked unchanged;
                // before the "no head, no cloak" test, so headless too).
                self.glow = true;
                self.cloak_dirty = true;
                runlog::kv("stalker_glow", &format!("id={} on=true cloaked={} decapitated={}", self.id, self.cloaked, self.decapitated));
            } else if !spotted && !(self.cloaked && !self.glow) && !self.decapitated {
                // CloakStalker, not spotted ("No head, no cloak").
                let was_glowing = self.glow;
                self.cloaked = true;
                self.glow = false;
                self.cloak_dirty = true;
                runlog::kv("stalker_cloak", &format!("id={} from_glow={was_glowing}", self.id));
            }
        }
    }

    /// ZombieBoss.Tick (the client part): while cloaked and not zapped,
    /// every 0.8 s, glow when a Commando sees him, back to the cloak when
    /// none does. Uncloaking clears it (UnCloakBoss: bSpotted = false).
    pub(super) fn boss_spot_tick(&mut self, viewer: Option<&CloakViewer>, in_sight: impl FnOnce() -> bool, dt: f32) {
        if !self.cloaked {
            if self.spotted || self.glow {
                self.spotted = false;
                self.glow = false;
                self.cloak_dirty = true;
                self.log_spotted(viewer, "patriarch");
            }
            return;
        }
        if self.zapped() {
            self.cloak_check = 0.0;
            return;
        }
        self.cloak_check -= dt;
        if self.cloak_check > 0.0 {
            return;
        }
        self.cloak_check = BOSS_CHECK_INTERVAL;
        let spotted = viewer.is_some_and(|v| v.spots_boss(self.centre, in_sight));
        if spotted != self.spotted {
            self.spotted = spotted;
            self.glow = spotted;
            self.cloak_dirty = true;
            self.log_spotted(viewer, "patriarch");
        }
    }

    /// What KF does to the skins when it sets the normal skin
    /// (UncloakStalker, RemoveHead, SetZappedBehavior, PlayDying): the
    /// glow goes too.
    pub(super) fn clear_glow(&mut self) {
        if self.glow {
            self.glow = false;
            self.cloak_dirty = true;
        }
    }
}

/// Loads FinalBlend KFX.StalkerGlow by following its chain in the package
/// (FinalBlend -> Shader StalkerGlowShader: SelfIllumination = Combiner ->
/// Material1 = TexPanner -> red texture; SelfIlluminationMask = texture)
/// and draws it as one unlit, additive material: red texture x the mask's
/// alpha. FB_Brighten as plain additive, the Combiner's StalkerSkin mix
/// and the panner / oscillator motion are left out (guesses: native
/// code); see DESIGN.md.
pub(super) fn load_stalker_glow(set: &PackageSet, images: &mut Assets<Image>, materials: &mut Assets<StandardMaterial>) -> Option<Handle<StandardMaterial>> {
    let result = build_glow(set, images, materials);
    if let Err(e) = &result {
        runlog::kv("stalker_glow_error", &format!("error=\"{e}\""));
    }
    result.ok()
}

fn named(set: &PackageSet, package: &str, name: &str) -> Option<ObjectHandle> {
    let lp = set.load(package)?;
    let export = (0..lp.pkg.exports.len()).find(|&i| lp.pkg.object_name(ObjectRef::Export(i)).eq_ignore_ascii_case(name))?;
    Some(ObjectHandle { package: lp, export })
}

/// The object a property of `h` points to.
fn follow(set: &PackageSet, h: &ObjectHandle, prop: &str) -> Result<ObjectHandle, String> {
    let props = read_export_properties(&h.package.pkg, h.export).map_err(|e| format!("{}: {e}", h.path()))?;
    match props.get(&h.package.pkg, prop) {
        Some(Value::Object(rf)) => set.resolve(&h.package, *rf).ok_or_else(|| format!("{}.{prop} does not resolve", h.path())),
        _ => Err(format!("{} has no {prop}", h.path())),
    }
}

/// Down through modifiers (TexPanner, TexOscillator...) to the texture.
fn texture_of(set: &PackageSet, mut h: ObjectHandle) -> Result<ObjectHandle, String> {
    for _ in 0..8 {
        if h.class_name() == "Texture" {
            return Ok(h);
        }
        h = follow(set, &h, "Material")?;
    }
    Err(format!("{}: no texture", h.path()))
}

fn rgba_of(h: &ObjectHandle) -> Result<(usize, usize, Vec<u8>), String> {
    let tex = read_texture(&h.package.pkg, h.export).map_err(|e| format!("{}: {e}", h.path()))?;
    let mip = tex.mips.first().ok_or_else(|| format!("{}: no mips", h.path()))?;
    let rgba = decode_rgba(tex.format, mip, None).ok_or_else(|| format!("{}: cannot decode", h.path()))?;
    Ok((mip.width as usize, mip.height as usize, rgba))
}

fn build_glow(set: &PackageSet, images: &mut Assets<Image>, materials: &mut Assets<StandardMaterial>) -> Result<Handle<StandardMaterial>, String> {
    use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
    let fb = named(set, "KFX", "StalkerGlow").ok_or("KFX.StalkerGlow not found")?;
    let fb_props = read_export_properties(&fb.package.pkg, fb.export).map_err(|e| e.to_string())?;
    let blending = match fb_props.get(&fb.package.pkg, "FrameBufferBlending") {
        Some(Value::Byte(b)) => *b,
        _ => 0,
    };
    let shader = follow(set, &fb, "Material")?;
    let combiner = follow(set, &shader, "SelfIllumination")?;
    let colour = texture_of(set, follow(set, &combiner, "Material1")?)?;
    let mask = texture_of(set, follow(set, &shader, "SelfIlluminationMask")?)?;
    let (w, h, mut rgba) = rgba_of(&colour)?;
    let (mw, mh, mask_rgba) = rgba_of(&mask)?;
    let mut sum = [0u64; 3];
    for y in 0..h {
        for x in 0..w {
            let a = mask_rgba[((y * mh / h) * mw + x * mw / w) * 4 + 3] as u32;
            let p = &mut rgba[(y * w + x) * 4..][..4];
            for (c, s) in p.iter_mut().take(3).zip(sum.iter_mut()) {
                *c = (*c as u32 * a / 255) as u8;
                *s += *c as u64;
            }
            p[3] = 255;
        }
    }
    let n = (w * h) as u64;
    runlog::kv(
        "stalker_glow_loaded",
        &format!(
            "final_blend={} frame_buffer_blending={blending} shader={} combiner={} colour={} mask={} size={w}x{h} average_rgb=({},{},{})",
            fb.path(),
            shader.path(),
            combiner.path(),
            colour.path(),
            mask.path(),
            sum[0] / n,
            sum[1] / n,
            sum[2] / n
        ),
    );
    let mut image = Image::new(
        Extent3d { width: w as u32, height: h as u32, depth_or_array_layers: 1 },
        TextureDimension::D2,
        rgba,
        TextureFormat::Rgba8UnormSrgb,
        bevy::asset::RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        ..default()
    });
    Ok(materials.add(StandardMaterial {
        base_color_texture: Some(images.add(image)),
        unlit: true,
        // FB_Brighten (6): added to what is behind.
        alpha_mode: AlphaMode::Add,
        cull_mode: None,
        double_sided: true,
        ..default()
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::perks::Perk;

    fn viewer(perk: Option<Perk>, level: u8) -> CloakViewer {
        CloakViewer { location: Vec3::ZERO, alive: true, vet: Vet { perk, level } }
    }

    /// Bevy point `units` Unreal units away.
    fn at(units: f32) -> Vec3 {
        Vec3::X * units * SCALE
    }

    #[test]
    fn stalker_spot_range_per_commando_level() {
        // sqrt(multi x 640000): 200, 400, 480, 560, 640, 800, 800.
        for (level, range) in [(0, 200.0), (1, 400.0), (2, 480.0), (3, 560.0), (4, 640.0), (5, 800.0), (6, 800.0)] {
            let v = viewer(Some(Perk::Commando), level);
            assert!(v.spots_stalker(at(range - 2.0)), "level {level} inside");
            assert!(!v.spots_stalker(at(range + 2.0)), "level {level} outside");
        }
    }

    #[test]
    fn only_living_commandos_spot() {
        assert!(!viewer(None, 6).spots_stalker(at(10.0)));
        assert!(!viewer(Some(Perk::Sharpshooter), 6).spots_stalker(at(10.0)));
        let mut dead = viewer(Some(Perk::Commando), 6);
        dead.alive = false;
        assert!(!dead.spots_stalker(at(10.0)));
    }

    /// A cloaked Stalker `units` away from a Commando 6, after `secs` of
    /// 0.1 s ticks.
    fn run(z: &mut Zed, v: &CloakViewer, secs: f32) {
        for _ in 0..(secs * 10.0).round() as usize {
            z.stalker_cloak_tick(Some(v), 0.1);
        }
    }

    #[test]
    fn stalker_glow_comes_and_goes_like_zombie_stalker_tick() {
        let mut z = Zed::test_clot();
        z.cloaked = true;
        let mut v = viewer(Some(Perk::Commando), 6);
        v.location = at(-900.0);
        run(&mut z, &v, 1.0);
        assert!(!z.spotted && !z.glow && z.cloaked, "900 units: not spotted");
        v.location = at(-700.0);
        run(&mut z, &v, 0.6);
        assert!(z.spotted && z.glow && z.cloaked, "700 units: glow, still cloaked");
        // Out of range while cloaked: back to the invisible skin.
        v.location = at(-900.0);
        run(&mut z, &v, 0.6);
        assert!(!z.spotted && !z.glow && z.cloaked);
        // After an attack (uncloaked): spotted again only glows 1.2 s
        // later, and stays uncloaked.
        z.cloaked = false;
        z.since_uncloak = 0.0;
        v.location = at(-100.0);
        run(&mut z, &v, 1.0);
        assert!(z.spotted && !z.glow && !z.cloaked, "before 1.2 s");
        run(&mut z, &v, 0.6);
        assert!(z.glow && !z.cloaked, "after 1.2 s");
        // Leaving range while uncloaked and glowing: UncloakStalker.
        v.location = at(-900.0);
        run(&mut z, &v, 0.6);
        assert!(!z.glow && !z.cloaked && z.since_uncloak < 1.0);
        // Headless: no cloak, but a Commando still sees the glow.
        z.decapitated = true;
        run(&mut z, &v, 3.0);
        assert!(!z.cloaked && !z.glow);
        v.location = at(-100.0);
        run(&mut z, &v, 0.6);
        assert!(z.glow && !z.cloaked);
    }

    #[test]
    fn boss_spot_any_level_within_1000_in_sight() {
        let v = viewer(Some(Perk::Commando), 0);
        assert!(v.spots_boss(at(990.0), || true));
        assert!(!v.spots_boss(at(1010.0), || true));
        assert!(!v.spots_boss(at(500.0), || false));
        assert!(!viewer(Some(Perk::Medic), 6).spots_boss(at(500.0), || true));
    }
}
