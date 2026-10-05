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
    pub blend: Blend,
    pub two_sided: bool,
    /// Classes passed through, e.g. ["Shader", "TexPanner", "Texture"], for logging.
    pub chain: Vec<String>,
    /// Apply the reflex-sight rule (see the Shader case); set by
    /// `resolve_skinned`. Off for level materials, where Opacity is often a
    /// plain mask (puddles, oil, water) that must not become the colour.
    pub opacity_from_combiner: bool,
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
            if let Some(b) = blend {
                out.blend = b;
            } else if has_opacity {
                out.blend = Blend::Masked;
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
            out.blend = match get_byte("FrameBufferBlending") {
                Some(7) => Blend::Invisible,
                Some(0) | None if get_bool("AlphaTest") => Blend::Masked,
                Some(0) | None => out.blend,
                _ if get_bool("AlphaTest") => Blend::Masked,
                Some(4) | Some(6) => Blend::Additive,
                _ => Blend::Translucent,
            };
        }
        "Combiner" => {
            // Material1, else Material2; also when Material1 ends without a
            // texture (the Stalker's cloak: a rotating environment map).
            if !follow("Material1", out) || out.texture.is_none() {
                follow("Material2", out);
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
