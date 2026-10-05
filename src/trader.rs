//! Shops (T2a): the map's ShopVolumes, their KFTraderDoors and the
//! Teleporters players are booted to. Rules from KFGameType (SelectShop,
//! OpenShops, CloseShops, BootShopPlayers), ShopVolume, Teleporter.Accept
//! and HUDKillingFloor.DrawTraderDistance. The wave timer (game.rs) calls
//! `open_shops`, `close_shops` and `select_shop` at KF's moments; this
//! module's system does the touching and booting. See DESIGN.md, T2a.

use std::rc::Rc;

use bevy::prelude::*;
use ue_assets::class_defaults::ClassDefaults;
use ue_assets::package::ObjectRef;
use ue_assets::package_set::LoadedPackage;
use ue_assets::properties::{Value, read_export_properties};

use crate::coords::SCALE;
use crate::runlog;

pub struct Shop {
    pub name: String,
    /// Teleporters with this Tag are the boot spots.
    pub url: String,
    /// Actors with this Tag are triggered on open and close.
    pub event: String,
    pub location: Vec3,
    pub yaw: i32,
    pub polys: Vec<Vec<Vec3>>,
    pub always_closed: bool,
    pub always_enabled: bool,
    /// bCurrentlyOpen.
    pub open: bool,
    /// Indices into `Doors::trader` (Tag == Event).
    pub doors: Vec<usize>,
    /// Indices into `Shops::teleporters` (Tag == URL).
    pub teleporters: Vec<usize>,
}

pub struct Teleporter {
    pub name: String,
    pub tag: String,
    pub location: Vec3,
    pub yaw: i32,
}

#[derive(Resource, Default)]
pub struct Shops {
    pub shops: Vec<Shop>,
    pub teleporters: Vec<Teleporter>,
    /// KFGameReplicationInfo.CurrentShop.
    pub current: Option<usize>,
    /// bTradingDoorsOpen.
    pub doors_open: bool,
    /// The game asked for BootShopPlayers this tick.
    pub boot_requested: bool,
    /// The shop the player is in (for Touch on entering).
    inside: Option<usize>,
    /// Trader doors are matched to shops once door.rs has spawned them.
    doors_linked: bool,
    rng: u32,
}

/// A short on-screen message (KFMainMessages), shown for a few seconds.
#[derive(Resource, Default)]
pub struct HudNote {
    pub text: String,
    pub until: f32,
}

impl HudNote {
    pub fn show(&mut self, text: &str, now: f32) {
        self.text = text.to_string();
        self.until = now + 3.0;
    }
}

/// KFMainMessages (KFMod.int).
const SHOP_BOOT_MSG: &str = "You can't stay in this shop after closing";
const SHOP_IT_BASE: &str = "Press 'E' to TRADE";

impl Shops {
    fn rand(&mut self, n: usize) -> usize {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        if n == 0 { 0 } else { (self.rng as usize) % n }
    }

    /// SelectShop: a random not-always-closed shop; if that is the current
    /// one, the next in the list (or the first).
    pub fn select_shop(&mut self) {
        let list: Vec<usize> = (0..self.shops.len()).filter(|&i| !self.shops[i].always_closed).collect();
        if list.is_empty() {
            return;
        }
        let k = self.rand(list.len());
        self.current = Some(if Some(list[k]) != self.current {
            list[k]
        } else if k + 1 < list.len() {
            list[k + 1]
        } else {
            list[0]
        });
        if let Some(c) = self.current {
            runlog::kv("shop_selected", &format!("shop={} url={}", self.shops[c].name, self.shops[c].url));
        }
    }

    /// OpenShops: the always-enabled shops and the current one.
    pub fn open_shops(&mut self, doors: &mut crate::door::Doors) {
        self.doors_open = true;
        for i in 0..self.shops.len() {
            if !self.shops[i].always_closed && self.shops[i].always_enabled {
                self.open_shop(i, doors);
            }
        }
        if self.current.is_none() {
            self.select_shop();
        }
        if let Some(c) = self.current {
            self.open_shop(c, doors);
        }
    }

    /// CloseShops: every open shop, then the next one is picked.
    pub fn close_shops(&mut self, doors: &mut crate::door::Doors) {
        self.doors_open = false;
        for i in 0..self.shops.len() {
            if self.shops[i].open {
                self.close_shop(i, doors);
            }
        }
        self.select_shop();
    }

    /// ShopVolume.OpenShop / CloseShop: trigger the actors tagged Event.
    fn open_shop(&mut self, i: usize, doors: &mut crate::door::Doors) {
        if self.shops[i].open {
            return;
        }
        self.shops[i].open = true;
        self.trigger(i, doors, "open");
    }

    fn close_shop(&mut self, i: usize, doors: &mut crate::door::Doors) {
        if !self.shops[i].open {
            return;
        }
        self.shops[i].open = false;
        self.trigger(i, doors, "close");
    }

    fn trigger(&self, i: usize, doors: &mut crate::door::Doors, what: &str) {
        let s = &self.shops[i];
        for &d in &s.doors {
            if let Some(door) = doors.trader.get_mut(d) {
                door.shop_trigger(&s.name);
            }
        }
        runlog::kv("shop", &format!("shop={} event={what} doors={}", s.name, s.doors.len()));
    }

    /// The shop whose brush holds this point (Unreal units).
    fn shop_at(&self, p: Vec3) -> Option<usize> {
        self.shops.iter().position(|s| crate::zvolume::encompasses(&s.polys, p))
    }
}

/// Reads the ShopVolumes (not bObjectiveModeOnly) and Teleporters, and
/// logs the other actors their Event would trigger.
pub fn load_shops(defaults: &ClassDefaults, map: &Rc<LoadedPackage>) -> Shops {
    let pkg = &map.pkg;
    let mut shops = Shops {
        rng: 0x9E37_79B9,
        ..default()
    };
    let mut tags: Vec<(String, String, String)> = Vec::new();
    for i in 0..pkg.exports.len() {
        let class = pkg.export_class_name(i);
        let is_shop = class == "ShopVolume";
        let is_teleporter = defaults.class_of(map, i).is_some_and(|c| defaults.is_a(&c, "Teleporter"));
        if !is_shop && !is_teleporter && class != "KFTraderDoor" && !defaults.class_of(map, i).is_some_and(|c| defaults.is_a(&c, "Actor")) {
            continue;
        }
        let Ok(props) = read_export_properties(pkg, i) else { continue };
        let value = |n: &str| defaults.actor_value(map, i, &props, n);
        let name = |n: &str| match value(n) {
            Some(Value::Name(x)) => pkg.name(x).to_string(),
            Some(Value::Str(x)) => x,
            _ => String::new(),
        };
        let flag = |n: &str| matches!(value(n), Some(Value::Bool(true)));
        let location = match props.get(pkg, "Location") {
            Some(Value::Vector(v)) => Vec3::from_array(*v),
            _ => Vec3::ZERO,
        };
        let yaw = match props.get(pkg, "Rotation") {
            Some(Value::Rotator(r)) => r.yaw,
            _ => 0,
        };
        let object = pkg.object_name(ObjectRef::Export(i)).to_string();
        if is_shop {
            if flag("bObjectiveModeOnly") {
                continue;
            }
            shops.shops.push(Shop {
                name: object,
                url: name("URL"),
                event: name("Event"),
                location,
                yaw,
                polys: crate::zvolume::brush_polys(pkg, &props),
                always_closed: flag("bAlwaysClosed"),
                always_enabled: flag("bAlwaysEnabled"),
                open: false,
                doors: Vec::new(),
                teleporters: Vec::new(),
            });
        } else if is_teleporter {
            shops.teleporters.push(Teleporter { name: object, tag: name("Tag"), location, yaw });
        } else {
            let tag = name("Tag");
            if !tag.is_empty() {
                tags.push((tag, class.to_string(), object));
            }
        }
    }
    let mut lines = Vec::new();
    for s in shops.shops.iter_mut() {
        s.teleporters = (0..shops.teleporters.len()).filter(|&t| shops.teleporters[t].tag.eq_ignore_ascii_case(&s.url)).collect();
        let others: Vec<String> = tags
            .iter()
            .filter(|(t, c, _)| t.eq_ignore_ascii_case(&s.event) && c != "KFTraderDoor")
            .map(|(_, c, o)| format!("{o}:{c}"))
            .collect();
        lines.push(format!(
            "{}(url={} event={} teleporters={} always_closed={} always_enabled={} brush_polys={} not_simulated=[{}])",
            s.name,
            s.url,
            s.event,
            s.teleporters.len(),
            s.always_closed,
            s.always_enabled,
            s.polys.len(),
            others.join(" ")
        ));
    }
    runlog::kv("shops_loaded", &format!("shops={} teleporters={} {}", shops.shops.len(), shops.teleporters.len(), lines.join(" ")));
    shops
}

pub struct TraderPlugin;

impl Plugin for TraderPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Shops>()
            .init_resource::<HudNote>()
            .add_systems(Update, shop_touch.after(crate::game::wave_timer));
    }
}

type PlayerQuery<'w, 's> = Query<'w, 's, (&'static mut Transform, &'static mut crate::camera::FlyCamera, Option<&'static mut crate::walk::Walker>)>;

/// ShopVolume.Touch (entering a shop while no wave runs) and
/// BootShopPlayers when the game asks.
#[allow(clippy::too_many_arguments)] // Bevy system parameters
fn shop_touch(
    time: Res<Time>,
    game: Res<crate::game::WaveGame>,
    options: Res<crate::game::GameOptions>,
    mut shops: ResMut<Shops>,
    doors: Res<crate::door::Doors>,
    mut note: ResMut<HudNote>,
    mut player: PlayerQuery,
    (script, frames): (Res<crate::weapon::ScriptedInput>, Res<bevy::diagnostic::FrameCount>),
) {
    if options.mode != crate::game::GameMode::Waves {
        return;
    }
    let shops = &mut *shops;
    if !shops.doors_linked && !doors.trader.is_empty() {
        shops.doors_linked = true;
        for s in shops.shops.iter_mut() {
            s.doors = (0..doors.trader.len()).filter(|&d| doors.trader[d].info.tag.eq_ignore_ascii_case(&s.event)).collect();
        }
        let lines: Vec<String> = shops.shops.iter().map(|s| format!("{}:{:?}", s.name, s.doors.iter().map(|&d| doors.trader[d].info.name.as_str()).collect::<Vec<_>>())).collect();
        runlog::kv("shop_doors", &lines.join(" "));
    }
    let Ok((mut t, mut cam, mut walker)) = player.single_mut() else { return };
    let now = time.elapsed_secs();
    // Test action "warp_shop": stand at the current shop's Location.
    if script.0.iter().any(|(f, a)| *f == frames.0 && a == "warp_shop")
        && let Some(c) = shops.current
    {
        let l = shops.shops[c].location;
        let to = Vec3::new(l.y, l.z, -l.x) * SCALE;
        if let Some(w) = walker.as_mut() {
            w.center = to;
            w.velocity = Vec3::ZERO;
        }
        t.translation = to + Vec3::Y * crate::combat::PLAYER_EYE_HEIGHT * SCALE;
        runlog::kv("shop_warp_test", &format!("shop={}", shops.shops[c].name));
    }
    // The player's cylinder centre, Unreal units.
    let centre = walker.as_ref().map_or(t.translation - Vec3::Y * crate::combat::PLAYER_EYE_HEIGHT * SCALE, |w| w.center);
    let ue = Vec3::new(-centre.z, centre.x, centre.y) / SCALE;
    let inside = shops.shop_at(ue);
    let wave_running = matches!(game.phase, crate::game::Phase::Wave | crate::game::Phase::BossWave);
    let mut boot = std::mem::take(&mut shops.boot_requested) && inside.is_some();
    if inside != shops.inside {
        if let Some(s) = inside {
            runlog::kv("shop_touch", &format!("shop={} open={} wave_running={wave_running}", shops.shops[s].name, shops.shops[s].open));
            if !wave_running {
                if shops.shops[s].open {
                    note.show(SHOP_IT_BASE, now);
                } else {
                    boot = true;
                }
            }
        }
        shops.inside = inside;
    }
    let Some(s) = inside.filter(|_| boot) else { return };
    // BootPlayers: a random Teleporter tagged URL; Accept keeps the yaw
    // relative to the teleporter (bChangesYaw).
    let tels = shops.shops[s].teleporters.clone();
    if tels.is_empty() {
        runlog::kv("shop_boot_failed", &format!("shop={} reason=no_teleporters", shops.shops[s].name));
        return;
    }
    let k = shops.rand(tels.len());
    let tel = &shops.teleporters[tels[k]];
    let player_yaw = (-cam.yaw / std::f32::consts::TAU * 65536.0) as i32;
    let yaw = tel.yaw + 32768 + player_yaw - shops.shops[s].yaw;
    let to = Vec3::new(tel.location.y, tel.location.z, -tel.location.x) * SCALE;
    if let Some(mut w) = walker {
        w.center = to;
        w.velocity = Vec3::ZERO;
    }
    t.translation = to + Vec3::Y * crate::combat::PLAYER_EYE_HEIGHT * SCALE;
    cam.yaw = -(yaw as f32) / 65536.0 * std::f32::consts::TAU;
    cam.yaw = cam.yaw.rem_euclid(std::f32::consts::TAU);
    t.rotation = Quat::from_euler(EulerRot::YXZ, cam.yaw, cam.pitch, 0.0);
    note.show(SHOP_BOOT_MSG, now);
    runlog::kv(
        "shop_boot",
        &format!("shop={} teleporter={} to_unreal=({:.0}, {:.0}, {:.0}) yaw={}", shops.shops[s].name, tel.name, tel.location.x, tel.location.y, tel.location.z, yaw & 65535),
    );
    shops.inside = None;
}

/// "TRADER: N m" (DrawTraderDistance), Unreal units in.
pub fn distance_text(shops: &Shops, player_ue: Vec3) -> Option<String> {
    let s = &shops.shops[shops.current?];
    Some(format!("TRADER: {}m", ((s.location - player_ue).length() / 50.0) as i32))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shops(n: usize) -> Shops {
        Shops {
            shops: (0..n)
                .map(|i| Shop {
                    name: format!("S{i}"),
                    url: String::new(),
                    event: String::new(),
                    location: Vec3::ZERO,
                    yaw: 0,
                    polys: Vec::new(),
                    always_closed: i == 1,
                    always_enabled: false,
                    open: false,
                    doors: Vec::new(),
                    teleporters: Vec::new(),
                })
                .collect(),
            rng: 12345,
            ..default()
        }
    }

    #[test]
    fn select_shop_never_repeats_and_skips_always_closed() {
        let mut s = shops(3);
        let mut last = None;
        for _ in 0..50 {
            s.select_shop();
            assert_ne!(s.current, Some(1));
            assert_ne!(s.current, last);
            last = s.current;
        }
    }

    #[test]
    fn open_and_close_track_the_current_shop() {
        let mut s = shops(3);
        let mut doors = crate::door::Doors::default();
        s.open_shops(&mut doors);
        let c = s.current.unwrap();
        assert!(s.doors_open && s.shops[c].open);
        s.close_shops(&mut doors);
        assert!(!s.doors_open && s.shops.iter().all(|x| !x.open));
        assert_ne!(s.current, Some(c));
    }
}
