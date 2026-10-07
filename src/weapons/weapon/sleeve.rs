//! Changing the character during the game (the perk page's SAVE,
//! KFTab_Profile.SaveSettings -> ChangeCharacter): every weapon's
//! sleeve skin slot (SleeveNum) takes the new species' SleeveTexture, as
//! KFWeapon.HandleSleeveSwapping does on the next BringUp. In KF the new
//! character only takes over with the next pawn (the lobby: when Ready
//! spawns it); our weapons already exist, so their sleeve material is
//! swapped in place.

use bevy::prelude::*;
use ue_assets::class_defaults::ClassDefaults;
use ue_assets::material::{Blend, resolve_skinned};
use ue_assets::package::ObjectRef;
use ue_assets::package_set::ObjectHandle;

use super::{WeaponAssets, Weapons};
use crate::engine::runlog;
use crate::player::character::{ChangeCharacter, CharacterChoice};
use crate::world::map::MapRequest;

pub(super) fn change_character(
    mut requests: MessageReader<ChangeCharacter>,
    mut assets: NonSendMut<WeaponAssets>,
    weapons: Option<Res<Weapons>>,
    mut choice: ResMut<CharacterChoice>,
    request: Res<MapRequest>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let Some(name) = requests.read().last().map(|r| r.0.clone()) else { return };
    let Some(set) = assets.0.as_ref() else {
        runlog::kv("character_change", &format!("name={name} applied=false reason=no_assets"));
        return;
    };
    let defaults = ClassDefaults::new(set);
    let Some(chosen) = crate::player::character::choose(set, &defaults, &request.install_root, Some(&name)) else { return };
    choice.0 = Some(chosen.name.clone());
    let sleeve = chosen.sleeve.clone();
    let mut swapped = Vec::new();
    if let (Some(w), Some(s)) = (weapons.as_ref(), sleeve.as_ref()) {
        let simple = resolve_skinned(set, &ObjectHandle { package: s.package.clone(), export: 0 }, ObjectRef::Export(s.export));
        let image = simple.texture.as_ref().and_then(|t| crate::render::skinned::decode_image(t, &mut images));
        for def in &w.defs {
            for part in def.model.parts.iter().filter(|p| p.material_index == def.sleeve_num) {
                if let Some(mut m) = materials.get_mut(&part.material) {
                    m.base_color_texture = image.clone();
                    m.alpha_mode = match simple.blend {
                        Blend::Masked => AlphaMode::Mask(0.5),
                        Blend::Additive => AlphaMode::Add,
                        Blend::Translucent => AlphaMode::Blend,
                        _ => AlphaMode::Opaque,
                    };
                    swapped.push(format!("{}:{}", def.class, def.sleeve_num));
                }
            }
        }
    }
    drop(defaults);
    runlog::kv(
        "character_change",
        &format!("name={} sleeve={} weapons_swapped=[{}]", chosen.name, sleeve.as_ref().map_or("none".into(), |s| s.path()), swapped.join(" ")),
    );
    assets.1 = sleeve;
}
