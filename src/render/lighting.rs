//! Baked lighting from the map: BSP lightmaps and placed meshes' vertex
//! colours (DESIGN.md, "Baked lighting"). UE2 lit in screen (gamma) space:
//! colour = texture x light x K. Bevy works in linear light; with both the
//! texture and the light decoded as sRGB the product stays the same, and K
//! becomes K^2.2.

use bevy::asset::RenderAssetUsages;
use bevy::camera::Exposure;
use bevy::image::{ImageSampler, ImageSamplerDescriptor};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use ue_assets::bsp::LightmapTexture;
use ue_assets::texture::{Mip, TextureFormat as UeFormat, decode_rgba};

/// K: light 1.0 (255) times this is the texture's own colour. KF's rule:
/// BSP lightmaps are drawn with a doubling blend, texture x lightmap x 2
/// (details in the local RE.md).
pub const BRIGHTNESS: f32 = 2.0;

/// K in linear light (see the module note).
pub fn brightness_linear() -> f32 {
    BRIGHTNESS.powf(2.2)
}

/// StandardMaterial::lightmap_exposure that makes a lightmapped surface
/// texture x lightmap x K: Bevy multiplies the lightmap by it and by the
/// camera's exposure (default).
pub fn lightmap_exposure() -> f32 {
    brightness_linear() / Exposure::default().exposure()
}

/// A lightmap page as a Bevy image (top mip, decoded to RGBA, sRGB).
pub fn lightmap_image(t: &LightmapTexture, images: &mut Assets<Image>) -> Option<Handle<Image>> {
    let mip = Mip { width: t.width as usize, height: t.height as usize, data: t.mips.first()?.clone() };
    let rgba = decode_rgba(UeFormat::from_byte(t.format), &mip, None)?;
    Some(rgba_lightmap_image(rgba, t.width, t.height, images))
}

/// A lightmap page from RGBA bytes (a decoded saved page, or one built at
/// load), alpha forced to 255.
pub fn rgba_lightmap_image(mut rgba: Vec<u8>, width: u32, height: u32, images: &mut Assets<Image>) -> Handle<Image> {
    for p in rgba.as_chunks_mut::<4>().0.iter_mut() {
        p[3] = 255;
    }
    let mut image = Image::new(
        Extent3d { width, height, depth_or_array_layers: 1 },
        TextureDimension::D2,
        rgba,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor::linear());
    images.add(image)
}

/// The material lit by a lightmap only: its exposure set (see
/// `lightmap_exposure`) and no specular highlights (UE2 had none on BSP).
pub fn lightmapped(m: &StandardMaterial) -> StandardMaterial {
    StandardMaterial {
        lightmap_exposure: lightmap_exposure(),
        reflectance: 0.0,
        ..m.clone()
    }
}
