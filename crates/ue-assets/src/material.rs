//! Reduces an Unreal material (which may be a chain such as Shader -> TexPanner
//! -> Texture) to what the simple viewer can draw: one diffuse texture plus a
//! blend mode.

use crate::package::ObjectRef;
use crate::package_set::{ObjectHandle, PackageSet};
use crate::properties::{Value, read_export_properties};
use crate::reader::Reader;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Blend {
    #[default]
    Opaque,
    /// Alpha test: pixels are either fully drawn or not drawn.
    Masked,
    /// Semi-transparent.
    Translucent,
    /// Added to what is behind (Shader OB_Translucent, FinalBlend
    /// FB_Translucent / FB_Brighten: Unreal's "translucent" is additive).
    Additive,
    /// Never drawn (e.g. FinalBlend FB_Invisible, Shader OB_Invisible).
    Invisible,
}

#[derive(Clone, Default)]
pub struct SimpleMaterial {
    /// The texture to draw, if one was found.
    pub texture: Option<ObjectHandle>,
    /// A Shader's Opacity texture when it is not `texture`: its alpha is
    /// the material's alpha (KF's window glass, GlassShader).
    pub opacity: Option<ObjectHandle>,
    pub blend: Blend,
    pub two_sided: bool,
    /// Classes passed through, e.g. ["Shader", "TexPanner", "Texture"], for logging.
    pub chain: Vec<String>,
    /// Apply the reflex-sight rule (see the Shader case); set by
    /// `resolve_skinned`. Off for level materials, where Opacity is often a
    /// plain mask (puddles, oil, water) that must not become the colour.
    pub opacity_from_combiner: bool,
    /// A Shader with OutputBlending OB_Modulate: the scene behind is
    /// multiplied by twice the texture (mid-grey = no change). `blend`
    /// stays Translucent for the loaders that do not draw modulation.
    pub modulate: bool,
    /// A colour the texture is multiplied by, per channel (R, G, B; 1.0 =
    /// unchanged), in the texture's own 0-1 colour values. Set when a
    /// Combiner multiplies a texture by a ConstantColor (KF's coloured light
    /// cones: a white cone texture times a purple, red, green... colour).
    /// Only the map loader reads it; the other loaders ignore it.
    pub tint: Option<[f32; 3]>,
}

const MAX_DEPTH: usize = 8;

fn first_array_object(raw: &[u8], skip: usize) -> Option<ObjectRef> {
    let mut r = Reader::new(raw);
    for _ in 0..skip {
        r.compact_index().ok()?;
    }
    r.compact_index().ok().map(ObjectRef::from_raw)
}

/// Follows a material reference found in `handle`'s package.
pub fn resolve(set: &PackageSet, from: &ObjectHandle, rf: ObjectRef) -> SimpleMaterial {
    resolve_with(set, from, rf, false)
}

/// `resolve` for skinned meshes (weapons, characters): also applies the
/// reflex-sight rule.
pub fn resolve_skinned(set: &PackageSet, from: &ObjectHandle, rf: ObjectRef) -> SimpleMaterial {
    resolve_with(set, from, rf, true)
}

fn resolve_with(set: &PackageSet, from: &ObjectHandle, rf: ObjectRef, opacity_from_combiner: bool) -> SimpleMaterial {
    let mut out = SimpleMaterial {
        opacity_from_combiner,
        ..Default::default()
    };
    if let Some(h) = set.resolve(&from.package, rf) {
        walk(set, &h, &mut out, 0);
    }
    out
}

fn same(a: &ObjectHandle, b: &ObjectHandle) -> bool {
    a.export == b.export && a.package.name == b.package.name
}

/// The textures a Combiner's Material1 and Material2 lead to.
fn combiner_inputs(set: &PackageSet, combiner: &ObjectHandle) -> Vec<ObjectHandle> {
    let Ok(props) = read_export_properties(&combiner.package.pkg, combiner.export) else {
        return Vec::new();
    };
    ["Material1", "Material2"]
        .iter()
        .filter_map(|name| match props.get(&combiner.package.pkg, name) {
            Some(Value::Object(rf)) => set.resolve(&combiner.package, *rf),
            _ => None,
        })
        .filter_map(|h| {
            let mut m = SimpleMaterial::default();
            walk(set, &h, &mut m, 0);
            m.texture
        })
        .collect()
}

/// Engine.Combiner's EColorOperation (from its script): CO_Use_Color_From_Material1,
/// CO_Use_Color_From_Material2, CO_Multiply, CO_Add, CO_Subtract, ...
const CO_MULTIPLY: u8 = 2;

/// The colour a Combiner multiplies its texture by: CombineOperation
/// CO_Multiply with a ConstantColor as Material1 or Material2 (the other
/// input being the texture). Modulate2X / Modulate4X (Combiner flags, off by
/// default) scale it by 2 / 4.
fn combiner_tint(set: &PackageSet, h: &ObjectHandle, props: &crate::properties::PropertyList) -> Option<[f32; 3]> {
    let pkg = &h.package.pkg;
    let byte = |name: &str| match props.get(pkg, name) {
        Some(Value::Byte(b)) => *b,
        _ => 0,
    };
    let flag = |name: &str| matches!(props.get(pkg, name), Some(Value::Bool(true)));
    if byte("CombineOperation") != CO_MULTIPLY {
        return None;
    }
    let color = ["Material1", "Material2"].iter().find_map(|name| {
        let Some(Value::Object(rf)) = props.get(pkg, name) else {
            return None;
        };
        let c = set.resolve(&h.package, *rf)?;
        if c.class_name() != "ConstantColor" {
            return None;
        }
        let cp = read_export_properties(&c.package.pkg, c.export).ok()?;
        match cp.get(&c.package.pkg, "Color") {
            Some(Value::Color(rgba)) => Some(*rgba),
            // An unset Color keeps the struct default, black.
            _ => Some([0, 0, 0, 0]),
        }
    })?;
    Some(multiply_tint(color, flag("Modulate2X"), flag("Modulate4X")))
}

/// A ConstantColor (R, G, B, A bytes) as a multiplier. Plain CO_Multiply is
/// taken as texture x colour (1x); Modulate2X doubles it and Modulate4X
/// quadruples it. The 1x default is inferred from those flags existing in
/// the Combiner script, not read from the engine's drawing code (a guess).
pub fn multiply_tint(color: [u8; 4], modulate2x: bool, modulate4x: bool) -> [f32; 3] {
    let k = if modulate4x {
        4.0
    } else if modulate2x {
        2.0
    } else {
        1.0
    };
    [0, 1, 2].map(|i| color[i] as f32 / 255.0 * k)
}

fn walk(set: &PackageSet, h: &ObjectHandle, out: &mut SimpleMaterial, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    let class = h.class_name().to_string();
    out.chain.push(class.clone());
    let pkg = &h.package.pkg;
    let Ok(props) = read_export_properties(pkg, h.export) else {
        return;
    };
    let get_bool = |name: &str| matches!(props.get(pkg, name), Some(Value::Bool(true)));
    let get_byte = |name: &str| match props.get(pkg, name) {
        Some(Value::Byte(b)) => Some(*b),
        _ => None,
    };
    let follow = |name: &str, out: &mut SimpleMaterial| -> bool {
        if let Some(Value::Object(rf)) = props.get(pkg, name)
            && let Some(next) = set.resolve(&h.package, *rf)
        {
            walk(set, &next, out, depth + 1);
            return true;
        }
        false
    };

    // The texture an object property leads to (through any modifiers).
    let object = |props: &crate::properties::PropertyList, name: &str| -> Option<ObjectHandle> {
        match props.get(pkg, name) {
            Some(Value::Object(rf)) if *rf != ObjectRef::Null => {
                let h2 = set.resolve(&h.package, *rf)?;
                if name == "Opacity" {
                    let mut m = SimpleMaterial::default();
                    walk(set, &h2, &mut m, depth + 1);
                    m.texture
                } else {
                    Some(h2)
                }
            }
            _ => None,
        }
    };

    match class.as_str() {
        "Texture" => {
            out.texture = Some(h.clone());
            if get_bool("bMasked") && out.blend == Blend::Opaque {
                out.blend = Blend::Masked;
            }
            if get_bool("bAlphaTexture") && out.blend == Blend::Opaque {
                out.blend = Blend::Translucent;
            }
            out.two_sided |= get_bool("bTwoSided");
        }
        "Shader" => {
            out.two_sided |= get_bool("TwoSided");
            // EOutputBlending: OB_Normal, OB_Masked, OB_Modulate, OB_Translucent, OB_Invisible, ...
            let blend = match get_byte("OutputBlending") {
                Some(1) => Some(Blend::Masked),
                // OB_Translucent and OB_Brighten add; OB_Modulate and OB_Darken
                // stay plain translucent.
                Some(3) | Some(5) => Some(Blend::Additive),
                Some(2) | Some(6) => Some(Blend::Translucent),
                Some(4) => Some(Blend::Invisible),
                _ => None,
            };
            let has_opacity = matches!(props.get(pkg, "Opacity"), Some(Value::Object(rf)) if *rf != ObjectRef::Null);
            // Diffuse, else SelfIllumination; also when the Diffuse chain
            // ends without a texture (e.g. an environment-map cubemap).
            if !follow("Diffuse", out) || out.texture.is_none() {
                follow("SelfIllumination", out);
            }
            // OB_Normal with an Opacity: alpha blended by the Opacity's
            // alpha (was masked at 50%, which hid all of KF's window glass;
            // on/off alphas are still drawn masked by the map loader).
            out.modulate = get_byte("OutputBlending") == Some(2);
            if let Some(b) = blend {
                out.blend = b;
            } else if has_opacity {
                // Skinned meshes (zeds, weapons) keep the old masked rule:
                // their loader has no on/off-alpha check.
                out.blend = if out.opacity_from_combiner { Blend::Masked } else { Blend::Translucent };
            }
            if has_opacity
                && let Some(op) = object(&props, "Opacity")
                && out.texture.as_ref().is_none_or(|t| !same(t, &op))
            {
                out.opacity = Some(op);
            }
            // A Diffuse Combiner that has the Opacity texture as an input
            // (weapon reflex sights: reflection speckle + reticle, Opacity =
            // the reticle): the reticle is what shows, so take the Opacity
            // texture for colour and alpha, blended. The speckle layer is
            // dropped (approximation).
            if out.opacity_from_combiner
                && blend.is_none()
                && let Some(opacity) = object(&props, "Opacity")
                && let Some(diffuse) = object(&props, "Diffuse")
                && diffuse.class_name() == "Combiner"
                && combiner_inputs(set, &diffuse).iter().any(|t| same(t, &opacity))
            {
                out.texture = Some(opacity);
                out.blend = Blend::Translucent;
                out.chain.push("OpacityTexture".into());
            }
        }
        "FinalBlend" => {
            out.two_sided |= get_bool("TwoSided");
            follow("Material", out);
            // EFrameBufferBlending: Overwrite, Modulate, AlphaBlend, AlphaModulate, Translucent, Darken, Brighten, Invisible
            // AlphaTest only drops pixels below AlphaRef; with a blending
            // mode the material still blends (KF-WestLondon's phone booth
            // glass, FBGlass: FB_Translucent, AlphaTest, AlphaRef 13, was
            // masked at 50% and vanished).
            // The outer FinalBlend's blending replaces an inner Shader's
            // OB_Modulate (KF's street lamps: FB_AlphaBlend around one).
            out.modulate &= get_byte("FrameBufferBlending") == Some(1);
            out.blend = match get_byte("FrameBufferBlending") {
                Some(7) => Blend::Invisible,
                Some(0) | None if get_bool("AlphaTest") => Blend::Masked,
                Some(0) | None => out.blend,
                Some(4) | Some(6) => Blend::Additive,
                _ => Blend::Translucent,
            };
        }
        "Combiner" => {
            // Material1, else Material2; also when Material1 ends without a
            // texture (the Stalker's cloak: a rotating environment map).
            let before = out.blend;
            if !follow("Material1", out) || out.texture.is_none() {
                follow("Material2", out);
            }
            // An input texture's bAlphaTexture / bMasked does not make the
            // Combiner see-through: its alpha feeds the combine (e.g. the
            // weapons' diffuse + reflection, alpha = reflection mask). Only
            // for skinned meshes (with `opacity_from_combiner`) for now.
            if out.opacity_from_combiner {
                out.blend = before;
            }
            if let Some(t) = combiner_tint(set, h, &props) {
                let prev = out.tint.unwrap_or([1.0; 3]);
                out.tint = Some([prev[0] * t[0], prev[1] * t[1], prev[2] * t[2]]);
                out.chain.push("Tint".into());
            }
        }
        "MaterialSwitch" => {
            let current = match props.get(pkg, "Current") {
                Some(Value::Int(n)) => (*n).max(0) as usize,
                _ => 0,
            };
            if let Some(Value::Array { raw, .. }) = props.get(pkg, "Materials")
                && let Some(rf) = first_array_object(raw, current)
                && let Some(next) = set.resolve(&h.package, rf)
            {
                walk(set, &next, out, depth + 1);
            }
        }
        // Every other Modifier subclass (TexPanner, TexRotator, TexScaler,
        // TexOscillator, TexEnvMap, ColorModifier, OpacityModifier, ...) wraps
        // one material in its `Material` property.
        _ => {
            if class == "ColorModifier" {
                out.two_sided |= get_bool("RenderTwoSided");
            }
            follow("Material", out);
        }
    }
}

/// `Material.SurfaceType` of the material `rf` (seen from `from`): the
/// object's own value, not its texture's (ROHitEffect reads
/// HitMat.SurfaceType directly); 0 (EST_Default) when unset or unresolved.
pub fn surface_type(set: &PackageSet, from: &ObjectHandle, rf: ObjectRef) -> u8 {
    let Some(h) = set.resolve(&from.package, rf) else {
        return 0;
    };
    match crate::properties::read_export_properties(&h.package.pkg, h.export) {
        Ok(props) => match props.get(&h.package.pkg, "SurfaceType") {
            Some(crate::properties::Value::Byte(b)) => *b,
            _ => 0,
        },
        Err(_) => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multiply_tint_scales() {
        let purple = [73, 0, 164, 0];
        let t = multiply_tint(purple, false, false);
        assert!((t[0] - 73.0 / 255.0).abs() < 1e-6 && t[1] == 0.0 && (t[2] - 164.0 / 255.0).abs() < 1e-6);
        assert!((multiply_tint(purple, true, false)[2] - 2.0 * 164.0 / 255.0).abs() < 1e-6);
        assert!((multiply_tint(purple, true, true)[0] - 4.0 * 73.0 / 255.0).abs() < 1e-6);
    }

    /// Against the real install (skipped when none is found): KF's purple
    /// light cone Shader resolves to the cone texture, additive, tinted by
    /// Purple_Constant (73, 0, 164).
    #[test]
    fn light_cone_shader_is_tinted() {
        let Ok(install) = crate::install::Install::discover() else {
            eprintln!("no Killing Floor install: skipped");
            return;
        };
        let set = PackageSet::new(&install.root);
        let Some(h) = set.find_object("Asylum_T.Lighting.Light_Cone_SHDR", Some("Shader")) else {
            eprintln!("Asylum_T.Lighting.Light_Cone_SHDR not found: skipped");
            return;
        };
        let mut m = SimpleMaterial::default();
        walk(&set, &h, &mut m, 0);
        assert_eq!(m.blend, Blend::Additive);
        let tex = m.texture.as_ref().map(|t| t.path().to_ascii_lowercase());
        assert_eq!(tex.as_deref(), Some("asylum_t.lighting.light_cone"));
        let t = m.tint.expect("tint");
        assert!((t[0] - 73.0 / 255.0).abs() < 1e-6, "{t:?}");
        assert_eq!(t[1], 0.0);
        assert!((t[2] - 164.0 / 255.0).abs() < 1e-6, "{t:?}");
    }
}
