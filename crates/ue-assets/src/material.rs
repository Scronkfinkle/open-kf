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
    let mut out = SimpleMaterial::default();
    if let Some(h) = set.resolve(&from.package, rf) {
        walk(set, &h, &mut out, 0);
    }
    out
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
                Some(2) | Some(3) | Some(5) | Some(6) => Some(Blend::Translucent),
                Some(4) => Some(Blend::Invisible),
                _ => None,
            };
            let has_opacity = matches!(props.get(pkg, "Opacity"), Some(Value::Object(rf)) if *rf != ObjectRef::Null);
            if !follow("Diffuse", out) {
                follow("SelfIllumination", out);
            }
            if let Some(b) = blend {
                out.blend = b;
            } else if has_opacity {
                out.blend = Blend::Masked;
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
                _ => Blend::Translucent,
            };
        }
        "Combiner" => {
            if !follow("Material1", out) {
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
