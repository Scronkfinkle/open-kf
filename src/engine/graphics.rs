//! Graphics settings: display mode, resolution, vsync, field of view,
//! brightness, anti-aliasing (MSAA) and texture filtering. They come from
//! command-line options (`--display`, `--window`, `--no-vsync`, `--fov`,
//! `--brightness`, `--msaa`, `--anisotropy`), which the launcher builds
//! from its Graphics section. See DESIGN.md, "Graphics settings in the
//! launcher".
//!
//! This file: the settings and their option parsing (shared with the
//! launcher), the primary window they make, and the systems that apply
//! what cannot be set when the window is made: MSAA on every camera, the
//! exact video mode for exclusive fullscreen, and the `window_state` log.
//! The field of view is read by engine/camera.rs and weapons/weapon/
//! animate.rs, the brightness by render/overlay.rs, the texture filtering
//! by world/map.rs.

use bevy::prelude::*;
use bevy::window::{Monitor, MonitorSelection, PresentMode, PrimaryMonitor, PrimaryWindow, VideoMode, VideoModeSelection, WindowMode, WindowResolution};

use crate::engine::runlog;

/// Windowed, borderless (a window covering the screen) or exclusive
/// fullscreen (the monitor switches to the chosen resolution).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum DisplayMode {
    #[default]
    Windowed,
    Borderless,
    Fullscreen,
}

impl DisplayMode {
    pub const ALL: [DisplayMode; 3] = [DisplayMode::Windowed, DisplayMode::Borderless, DisplayMode::Fullscreen];

    /// The `--display` word (and the saved value).
    pub fn word(self) -> &'static str {
        match self {
            DisplayMode::Windowed => "windowed",
            DisplayMode::Borderless => "borderless",
            DisplayMode::Fullscreen => "fullscreen",
        }
    }

    pub fn parse(s: &str) -> Option<DisplayMode> {
        DisplayMode::ALL.into_iter().find(|m| m.word().eq_ignore_ascii_case(s.trim()))
    }
}

/// Field of view (KF's horizontal degrees at 4:3): range and launcher step.
/// KF's own FOV command takes 80 and up in network games (PlayerController.FOV);
/// 120 as the top is ours.
pub const FOV_MIN: u32 = 80;
pub const FOV_MAX: u32 = 120;
pub const FOV_STEP: u32 = 5;
/// Brightness in percent of the light: range and launcher step. 200 is the
/// most the modulate quad can do (render/overlay.rs: screen x 2 x colour).
pub const BRIGHTNESS_MIN: u32 = 50;
pub const BRIGHTNESS_MAX: u32 = 200;
pub const BRIGHTNESS_STEP: u32 = 10;
/// MSAA sample counts offered (1 = off). 1 and 4 work on every graphics
/// card (WebGPU's rule); 2 and 8 are checked at start (`check_msaa`).
pub const MSAA_SAMPLES: [u32; 4] = [1, 2, 4, 8];
/// Anisotropic filtering levels offered (1 = plain trilinear).
pub const ANISOTROPY_LEVELS: [u16; 5] = [1, 2, 4, 8, 16];

pub const DEFAULT_FOV: u32 = 90;
pub const DEFAULT_BRIGHTNESS: u32 = 100;
/// Bevy's default (what the game always used before these settings).
pub const DEFAULT_MSAA: u32 = 4;
/// The map textures' value before these settings.
pub const DEFAULT_ANISOTROPY: u16 = 8;

/// `--fov DEG`.
pub fn parse_fov(s: &str) -> Result<u32, String> {
    let v: u32 = s.trim().parse().map_err(|_| format!("bad --fov value: {s}"))?;
    if !(FOV_MIN..=FOV_MAX).contains(&v) {
        return Err(format!("--fov must be {FOV_MIN} to {FOV_MAX}: {s}"));
    }
    Ok(v)
}

/// `--brightness PERCENT`.
pub fn parse_brightness(s: &str) -> Result<u32, String> {
    let v: u32 = s.trim().trim_end_matches('%').parse().map_err(|_| format!("bad --brightness value: {s}"))?;
    if !(BRIGHTNESS_MIN..=BRIGHTNESS_MAX).contains(&v) {
        return Err(format!("--brightness must be {BRIGHTNESS_MIN} to {BRIGHTNESS_MAX} (percent): {s}"));
    }
    Ok(v)
}

/// `--msaa 0|1|2|4|8|off` (0 and off mean 1 sample: no anti-aliasing).
pub fn parse_msaa(s: &str) -> Result<u32, String> {
    let v = match s.trim().to_ascii_lowercase().as_str() {
        "off" | "0" => 1,
        n => n.trim_end_matches('x').parse().map_err(|_| format!("bad --msaa value: {s}"))?,
    };
    if !MSAA_SAMPLES.contains(&v) {
        return Err(format!("--msaa must be 0 (off), 2, 4 or 8: {s}"));
    }
    Ok(v)
}

/// `--anisotropy 1|2|4|8|16`.
pub fn parse_anisotropy(s: &str) -> Result<u16, String> {
    let v: u16 = s.trim().trim_end_matches('x').parse().map_err(|_| format!("bad --anisotropy value: {s}"))?;
    if !ANISOTROPY_LEVELS.contains(&v) {
        return Err(format!("--anisotropy must be 1, 2, 4, 8 or 16: {s}"));
    }
    Ok(v)
}

/// The graphics settings this run uses.
#[derive(Resource, Clone, Debug, PartialEq)]
pub struct GraphicsSettings {
    pub display: DisplayMode,
    /// `--window WxH`: the window size (windowed) or the video mode
    /// (fullscreen); None: the system's default window / the desktop's mode.
    pub resolution: Option<(u32, u32)>,
    pub vsync: bool,
    /// The player's field of view (horizontal degrees at 4:3).
    pub fov: f32,
    /// Percent of the light (100: unchanged).
    pub brightness: u32,
    /// MSAA samples asked for (1: off). `check_msaa` may lower it.
    pub msaa: u32,
    /// Map textures' anisotropic filtering (1: trilinear only).
    pub anisotropy: u16,
}

impl Default for GraphicsSettings {
    fn default() -> Self {
        GraphicsSettings {
            display: DisplayMode::Windowed,
            resolution: None,
            vsync: true,
            fov: DEFAULT_FOV as f32,
            brightness: DEFAULT_BRIGHTNESS,
            msaa: DEFAULT_MSAA,
            anisotropy: DEFAULT_ANISOTROPY,
        }
    }
}

impl GraphicsSettings {
    pub fn present_mode(&self) -> PresentMode {
        if self.vsync { PresentMode::default() } else { PresentMode::AutoNoVsync }
    }

    /// The primary window for these settings. Exclusive fullscreen opens
    /// borderless first: the exact video mode (size, refresh rate, colour
    /// bits) is only known once the monitors are, and `pick_video_mode`
    /// then switches. (Bevy's "the monitor's current mode" was tried first:
    /// on a monitor that reports no refresh rate, e.g. the virtual display
    /// of test runs, it finds no mode and stays windowed.)
    pub fn window(&self) -> Window {
        let mut w = Window { title: "Open KF".into(), present_mode: self.present_mode(), ..default() };
        // Borderless always takes the screen's size; fullscreen the mode's.
        if self.display == DisplayMode::Windowed
            && let Some((x, y)) = self.resolution
        {
            // Scale factor 1: the size in real pixels (as `--window` always did).
            w.resolution = WindowResolution::new(x, y).with_scale_factor_override(1.0);
            w.resizable = false;
        }
        w.mode = match self.display {
            DisplayMode::Windowed => WindowMode::Windowed,
            DisplayMode::Borderless | DisplayMode::Fullscreen => WindowMode::BorderlessFullscreen(MonitorSelection::Primary),
        };
        w
    }

    /// The `graphics_settings` log line.
    pub fn log(&self, fps: Option<f64>) {
        runlog::kv(
            "graphics_settings",
            &format!(
                "display={} resolution={} vsync={} fps_limit={} fov={} brightness={} msaa={} anisotropy={}{}",
                self.display.word(),
                self.resolution.map_or("default".into(), |(w, h)| format!("{w}x{h}")),
                if self.vsync { "on" } else { "off" },
                fps.map_or("none".into(), |f| f.to_string()),
                self.fov,
                self.brightness,
                self.msaa,
                self.anisotropy,
                if self.display == DisplayMode::Borderless && self.resolution.is_some() { " resolution_ignored=borderless_uses_screen_size" } else { "" },
            ),
        );
    }
}

pub struct GraphicsPlugin;

impl Plugin for GraphicsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GraphicsSettings>()
            .add_systems(Startup, check_msaa)
            .add_systems(Update, (apply_msaa, pick_video_mode, log_window_state));
    }
}

/// The sample count every camera gets (after the graphics card check).
#[derive(Resource, Clone, Copy)]
struct AppliedMsaa(Msaa);

fn msaa_of(samples: u32) -> Msaa {
    match samples {
        1 => Msaa::Off,
        2 => Msaa::Sample2,
        8 => Msaa::Sample8,
        _ => Msaa::Sample4,
    }
}

/// 2x and 8x MSAA are optional in wgpu: a card without them would stop the
/// game with an error. Checks the screen's colour formats (RGBA and BGRA
/// 8-bit sRGB: the window's picture is one of them) and the 3D depth
/// format, and falls back to 4x.
fn check_msaa(mut commands: Commands, mut settings: ResMut<GraphicsSettings>, adapter: Option<Res<bevy::render::renderer::RenderAdapter>>) {
    use bevy::render::render_resource::TextureFormat;
    let wanted = settings.msaa;
    let supported = |n: u32| {
        n == 1
            || n == 4
            || adapter.as_ref().is_some_and(|a| {
                [TextureFormat::Rgba8UnormSrgb, TextureFormat::Bgra8UnormSrgb, bevy::core_pipeline::core_3d::CORE_3D_DEPTH_FORMAT].iter().all(|f| a.get_texture_format_features(*f).flags.sample_count_supported(n))
            })
    };
    let used = if supported(wanted) { wanted } else { DEFAULT_MSAA };
    settings.msaa = used;
    runlog::kv("msaa_check", &format!("wanted={wanted} used={used} fallback={} adapter_known={}", used != wanted, adapter.is_some()));
    commands.insert_resource(AppliedMsaa(msaa_of(used)));
}

/// Gives every new camera the chosen MSAA. All cameras drawing to the
/// window must agree: they draw into the same picture one after another.
fn apply_msaa(msaa: Option<Res<AppliedMsaa>>, mut cams: Query<&mut Msaa, Added<Camera>>) {
    let Some(msaa) = msaa else { return };
    let mut n = 0;
    for mut m in &mut cams {
        if *m != msaa.0 {
            *m = msaa.0;
        }
        n += 1;
    }
    if n > 0 {
        runlog::kv("msaa_applied", &format!("cameras={n} samples={}", msaa.0.samples()));
    }
}

/// The video mode for exclusive fullscreen at `size`: that size, the
/// highest refresh rate, then the most colour bits.
pub fn best_video_mode(modes: &[VideoMode], size: (u32, u32)) -> Option<VideoMode> {
    modes.iter().filter(|m| m.physical_size == UVec2::new(size.0, size.1)).max_by_key(|m| (m.refresh_rate_millihertz, m.bit_depth)).copied()
}

/// Exclusive fullscreen: once the primary monitor is known, switch the
/// window to the video mode of the chosen size (the desktop's size when
/// none is chosen or the monitor has no such mode; logged). Bevy's
/// "current mode" only when the monitor lists no mode of either size, or
/// after 120 frames without a monitor.
fn pick_video_mode(
    settings: Res<GraphicsSettings>,
    monitors: Query<&Monitor, With<PrimaryMonitor>>,
    mut window: Query<&mut Window, With<PrimaryWindow>>,
    mut frames: Local<u32>,
    mut done: Local<bool>,
) {
    if *done || settings.display != DisplayMode::Fullscreen {
        return;
    }
    *frames += 1;
    let Ok(mut win) = window.single_mut() else { return };
    let Ok(monitor) = monitors.single() else {
        if *frames > 120 {
            *done = true;
            runlog::kv("fullscreen_mode", "result=no_monitor_found using=desktop_mode");
            win.mode = WindowMode::Fullscreen(MonitorSelection::Primary, VideoModeSelection::Current);
        }
        return;
    };
    *done = true;
    let desktop = (monitor.physical_width, monitor.physical_height);
    let wanted = settings.resolution.unwrap_or(desktop);
    let mut sizes: Vec<String> = monitor.video_modes.iter().map(|m| format!("{}x{}", m.physical_size.x, m.physical_size.y)).collect();
    sizes.dedup();
    let (picked, result) = match best_video_mode(&monitor.video_modes, wanted) {
        Some(m) => (Some(m), if settings.resolution.is_some() { "chosen_size" } else { "desktop_size" }),
        None => (best_video_mode(&monitor.video_modes, desktop), "no_such_mode_used_desktop_size"),
    };
    let what = match picked {
        Some(m) => {
            win.mode = WindowMode::Fullscreen(MonitorSelection::Primary, VideoModeSelection::Specific(m));
            format!("{}x{} refresh_hz={:.2} bit_depth={}", m.physical_size.x, m.physical_size.y, m.refresh_rate_millihertz as f32 / 1000.0, m.bit_depth)
        }
        None => {
            win.mode = WindowMode::Fullscreen(MonitorSelection::Primary, VideoModeSelection::Current);
            "current_mode".into()
        }
    };
    runlog::kv(
        "fullscreen_mode",
        &format!(
            "result={} wanted={}x{} using={what} desktop={}x{} monitor=\"{}\" monitor_modes=[{}]",
            if picked.is_some() { result } else { "no_mode_listed" },
            wanted.0,
            wanted.1,
            desktop.0,
            desktop.1,
            monitor.name.as_deref().unwrap_or("?"),
            sizes.join(" ")
        ),
    );
}

fn mode_word(m: &WindowMode) -> String {
    match m {
        WindowMode::Windowed => "windowed".into(),
        WindowMode::BorderlessFullscreen(_) => "borderless".into(),
        WindowMode::Fullscreen(_, VideoModeSelection::Current) => "fullscreen(desktop_mode)".into(),
        WindowMode::Fullscreen(_, VideoModeSelection::Specific(v)) => format!("fullscreen({}x{}@{:.2}Hz)", v.physical_size.x, v.physical_size.y, v.refresh_rate_millihertz as f32 / 1000.0),
    }
}

/// `window_state`: the window's mode, size and present mode, when the
/// window is first there and whenever one of them changes.
fn log_window_state(window: Query<&Window, (With<PrimaryWindow>, Changed<Window>)>, mut last: Local<String>) {
    let Ok(w) = window.single() else { return };
    let s = format!(
        "mode={} physical={}x{} logical={:.0}x{:.0} scale_factor={:.2} present_mode={:?} resizable={}",
        mode_word(&w.mode),
        w.physical_width(),
        w.physical_height(),
        w.width(),
        w.height(),
        w.scale_factor(),
        w.present_mode,
        w.resizable
    );
    if *last != s {
        runlog::kv("window_state", &s);
        *last = s;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_parse_and_refuse() {
        assert_eq!(parse_fov("100"), Ok(100));
        assert!(parse_fov("79").is_err() && parse_fov("121").is_err() && parse_fov("wide").is_err());
        assert_eq!(parse_brightness("150%"), Ok(150));
        assert!(parse_brightness("40").is_err() && parse_brightness("210").is_err());
        assert_eq!(parse_msaa("off"), Ok(1));
        assert_eq!(parse_msaa("0"), Ok(1));
        assert_eq!(parse_msaa("8x"), Ok(8));
        assert!(parse_msaa("3").is_err() && parse_msaa("16").is_err());
        assert_eq!(parse_anisotropy("16"), Ok(16));
        assert!(parse_anisotropy("3").is_err() && parse_anisotropy("32").is_err());
        assert_eq!(DisplayMode::parse("Borderless"), Some(DisplayMode::Borderless));
        assert_eq!(DisplayMode::parse("full"), None);
    }

    #[test]
    fn window_for_each_mode() {
        let mut s = GraphicsSettings { resolution: Some((1280, 720)), ..default() };
        let w = s.window();
        assert_eq!(w.mode, WindowMode::Windowed);
        assert_eq!((w.physical_width(), w.physical_height()), (1280, 720));
        assert!(!w.resizable);
        s.display = DisplayMode::Borderless;
        let w = s.window();
        assert_eq!(w.mode, WindowMode::BorderlessFullscreen(MonitorSelection::Primary));
        // The size is the screen's: the chosen one is not set.
        assert!(w.resizable);
        s.display = DisplayMode::Fullscreen;
        // Borderless first; pick_video_mode switches once the monitor is known.
        assert_eq!(s.window().mode, WindowMode::BorderlessFullscreen(MonitorSelection::Primary));
        s.vsync = false;
        assert_eq!(s.window().present_mode, PresentMode::AutoNoVsync);
    }

    #[test]
    fn video_mode_pick() {
        let m = |w, h, hz, bits| VideoMode { physical_size: UVec2::new(w, h), bit_depth: bits, refresh_rate_millihertz: hz };
        let modes = [m(1920, 1080, 60000, 32), m(1920, 1080, 144000, 24), m(1920, 1080, 144000, 32), m(1280, 720, 60000, 32)];
        assert_eq!(best_video_mode(&modes, (1920, 1080)), Some(m(1920, 1080, 144000, 32)));
        assert_eq!(best_video_mode(&modes, (1280, 720)), Some(m(1280, 720, 60000, 32)));
        assert_eq!(best_video_mode(&modes, (800, 600)), None);
    }
}
