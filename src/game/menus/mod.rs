//! KF's menu pages: the pre-game lobby (KFGui.LobbyMenu, LobbyFooter),
//! the perk page it opens (KFProfilePage / KFTab_Profile) and the pause
//! menu (KFInvasionLoginMenu / KFTab_MidGamePerks). See DESIGN.md,
//! "Menus: the lobby, the perk page, the pause menu".

mod audio_page;
pub mod gui;
mod lobby;
mod model_select;
mod pause;
mod perk_panel;
mod profile;

use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};

use crate::engine::runlog;
use crate::game::perks::Perk;
use crate::launcher::choices::SLIDERS;
use gui::{Gui, MenuSlot, POOL};
use perk_panel::PerkTexts;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    Lobby,
    /// KFProfilePage ("Select Perk" from the lobby).
    Profile,
    /// KFInvasionLoginMenu (Escape).
    Pause,
    /// KFModelSelect ("Change Character" on the perk page), above it.
    ModelSelect,
    /// The volume sliders (the pause menu's Settings button), above it.
    Audio,
}

/// What the mouse is dragging (a drag that started on its box): a
/// preview it turns, a volume slider (index into `SLIDERS`) or the mouse
/// sensitivity slider.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Drag {
    Profile,
    ModelSelect,
    Volume(usize),
    Sensitivity,
}

/// The pause menu's tabs (KFInvasionLoginMenu keeps Panels 1-3).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum PauseTab {
    #[default]
    Perks,
    Communication,
    Help,
}

/// Which pages are open (a stack, top last) and their state.
#[derive(Resource, Default, Debug)]
pub struct MenuState {
    pub stack: Vec<Page>,
    /// The perk page's highlighted row (KFPerkSelectList index), its
    /// character (sChar) and 3D view / portrait toggle (bRenderDude).
    pub profile_row: usize,
    pub profile_char: String,
    pub profile_portrait: bool,
    /// The 3D View's turn by mouse drags (Unreal rotation units, added to
    /// Opened's Yaw 32768).
    pub profile_yaw: i32,
    /// KFModelSelect: the highlighted character (index into
    /// `MenuData::characters`), the list's top row, the model's turn.
    pub select_index: usize,
    pub select_top: usize,
    pub select_yaw: i32,
    /// A drag in progress.
    pub drag: Option<Drag>,
    pub pause_tab: PauseTab,
    /// The pause menu's highlighted perk row.
    pub pause_row: usize,
    /// The Audio window's slider picked with the Up / Down keys.
    pub audio_row: usize,
    /// The cursor was captured when the pause menu opened.
    cursor_was_grabbed: bool,
}

impl MenuState {
    /// The lobby is up (GRI.bMatchHasBegun is still false).
    pub fn lobby_open(&self) -> bool {
        self.stack.contains(&Page::Lobby)
    }

    /// The lobby and its perk page: no pawn yet, so no HUD and no weapon.
    pub fn hides_hud(&self) -> bool {
        self.stack.iter().any(|p| matches!(p, Page::Lobby | Page::Profile | Page::ModelSelect))
    }

    pub fn top(&self) -> Option<Page> {
        self.stack.last().copied()
    }
}

/// `--lobby` / `--no-lobby` and the rule for when the lobby opens
/// (DESIGN.md), and the player's name (`--name`, else KF's defuser.ini
/// `[DefaultPlayer] Name=`).
#[derive(Resource, Debug, Clone, Default)]
pub struct LobbySettings {
    pub open: bool,
    pub reason: &'static str,
    pub name: Option<String>,
}

/// One row of the lobby's player list (KFGRI.PRIArray). Solo: the local
/// player only; a list so more players can be added later.
#[derive(Clone, Debug)]
pub struct LobbyPlayer {
    pub name: String,
    pub perk: Option<Perk>,
    pub level: u8,
    pub ready: bool,
}

/// What the pages show that comes from the map and the install.
#[derive(Resource, Default)]
pub struct MenuData {
    pub texts: PerkTexts,
    /// Level.Title ([LevelInfo0] Title of the map's .int, else the map name).
    pub map_title: String,
    /// The map record's Description (KFMapStoryLabel.LoadStoryText).
    pub map_description: String,
    pub player_name: String,
    /// (DefaultName, portrait texture path) of the characters KFModelSelect
    /// lists (`character::model_select_records`), sorted by name.
    pub characters: Vec<(String, String)>,
    /// KFGui.int [DecoText] biographies (lower-case name -> text).
    pub bios: std::collections::HashMap<String, String>,
}

/// The boxes that took clicks last frame (id, physical pixels).
#[derive(Resource, Default)]
struct MenuHits(Vec<(String, Rect)>);

pub struct MenusPlugin;

impl Plugin for MenusPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MenuState>()
            .init_resource::<Gui>()
            .init_resource::<MenuData>()
            .init_resource::<MenuHits>()
            .init_resource::<LobbySettings>()
            .add_systems(PostStartup, load_menus)
            .add_systems(
                PreUpdate,
                menu_input.after(bevy::input::InputSystems).before(crate::game::buy_menu::menu_input),
            )
            .add_systems(Update, request_previews)
            .add_systems(PostUpdate, (sync_previews, draw_menus).chain().after(crate::player::body::PreviewSystems));
    }
}

#[allow(clippy::too_many_arguments)] // Bevy system parameters
fn load_menus(
    mut commands: Commands,
    mut gui: ResMut<Gui>,
    mut data: ResMut<MenuData>,
    mut state: ResMut<MenuState>,
    settings: Res<LobbySettings>,
    request: Res<crate::world::map::MapRequest>,
    character: Res<crate::player::character::CharacterChoice>,
    mut images: ResMut<Assets<Image>>,
    (preview, buy_menu, cat): (Option<Res<crate::player::body::CharacterPreview>>, Res<crate::game::buy_menu::BuyMenu>, Option<Res<crate::game::buy_menu::ShopCatalogue>>),
) {
    let started = std::time::Instant::now();
    let root = &request.install_root;
    data.texts = PerkTexts::load(root);
    let map_int = gui::read_latin1(&root.join("System").join(format!("{}.int", request.map)));
    data.map_title = gui::ini_value(&map_int, "LevelInfo0", "Title").unwrap_or_else(|| request.map.clone());
    let ucl = gui::read_latin1(&root.join("System").join(format!("{}.ucl", request.map)));
    data.map_description = gui::ini_value(&map_int, "LevelSummary", "Description")
        .or_else(|| ucl.split("FallbackDesc=\"").nth(1).and_then(|s| s.split('"').next()).map(str::to_string))
        .unwrap_or_default();
    // KF's fresh-install default name (System/defuser.ini); the user's
    // own User.ini is not read (as for the character).
    let defuser = gui::read_latin1(&root.join("System").join("defuser.ini"));
    data.player_name = settings
        .name
        .clone()
        .or_else(|| gui::ini_value(&defuser, "DefaultPlayer", "Name"))
        .unwrap_or_else(|| "Player".into());
    let mut extra: Vec<String> = Perk::ALL.iter().map(|p| p.icons().0.to_string()).collect();
    // The network scoreboard's boxes (net/scoreboard.rs).
    extra.push("InterfaceArt_tex.Menu.changeme_texture".to_string());
    if buy_menu.kind == crate::game::buy_menu::MenuKind::Kf {
        // The classic trader menu's textures and the weapons' trader
        // pictures (TraderInfoTexture), only when that menu is chosen.
        extra.extend(crate::game::classic_menu::TEXTURES.iter().map(|t| t.to_string()));
        if let Some(cat) = &cat {
            let mut images: Vec<String> = cat.items.iter().map(|i| i.info.image.clone()).filter(|i| !i.is_empty()).collect();
            images.sort();
            images.dedup();
            extra.extend(images);
        }
    }
    if settings.open {
        // The portraits and biographies are only needed by the lobby.
        data.characters = crate::player::character::model_select_records(root).into_iter().map(|r| (r.name, r.portrait)).collect();
        extra.extend(data.characters.iter().map(|(_, p)| p.clone()).filter(|p| !p.is_empty()));
        let kfgui = gui::read_latin1(&root.join("System").join("KFGui.int"));
        data.bios = crate::audio::music::int_section(&kfgui, "DecoText").into_iter().map(|(k, v)| (k, unquote(&v))).collect();
    }
    gui::load(&mut gui, root, &extra, &mut images);
    // The character previews' images (player/body/preview.rs), drawn like
    // any other texture; `sync_previews` follows their changes.
    if let Some(preview) = preview {
        for image in preview.images.iter().flatten() {
            let size = images.get(image).map_or(Vec2::ONE, |i| i.size_f32());
            gui.textures.push(crate::game::hud::HudTexture { image: image.clone(), size });
            let i = gui.textures.len() - 1;
            gui.previews.push(i);
        }
    }
    for i in 0..POOL {
        commands.spawn((
            Node { position_type: PositionType::Absolute, ..default() },
            ImageNode { image_mode: bevy::ui::widget::NodeImageMode::Stretch, ..default() },
            Visibility::Hidden,
            // Above the HUD (10 + its 96 slots).
            GlobalZIndex(1000 + i as i32),
            MenuSlot(i),
        ));
    }
    state.profile_char = character.0.clone().unwrap_or_else(|| crate::player::character::DEFAULT_CHARACTER.to_string());
    runlog::kv(
        "menu_data",
        &format!(
            "map_title=\"{}\" description_chars={} player=\"{}\" characters={} bios={} seconds={:.2}",
            data.map_title,
            data.map_description.len(),
            data.player_name,
            data.characters.len(),
            data.bios.len(),
            started.elapsed().as_secs_f64()
        ),
    );
    if settings.open {
        state.stack.push(Page::Lobby);
        runlog::kv("menu_open", &format!("page=Lobby reason={}", settings.reason));
    } else {
        runlog::kv("lobby_skipped", &format!("reason={}", settings.reason));
    }
}

fn unquote(v: &str) -> String {
    let v = v.trim();
    let v = v.strip_prefix('"').unwrap_or(v);
    v.strip_suffix('"').unwrap_or(v).to_string()
}

/// A pressed button or a test action, by id.
fn scripted_to_ids(action: &str, top: Option<Page>) -> Vec<String> {
    let perk_row = |name: &str| Perk::parse(name).map(|p| p.index());
    match action {
        "lobby_ready" => vec!["lobby.ready".into()],
        "lobby_select_perk" => vec!["lobby.perks".into()],
        "lobby_options" => vec!["lobby.options".into()],
        "lobby_disconnect" => vec!["lobby.disconnect".into()],
        "lobby_save" => vec!["profile.save".into()],
        "change_character" | "char_select_open" => vec!["profile.pick".into()],
        "char_select_ok" => vec!["select.ok".into()],
        "char_select_cancel" => vec!["select.cancel".into()],
        "toggle_portrait" => vec!["profile.3d".into()],
        "pause_select_perk" => vec!["pause.save".into()],
        "pause_forfeit" => vec!["pause.forfeit".into()],
        "pause_exit" => vec!["pause.exit".into()],
        "pause_settings" | "volume_page" => vec!["pause.settings".into()],
        "volume_back" => vec!["audio.back".into()],
        a => {
            if let Some(n) = a.strip_prefix("perk_pick:") {
                match (perk_row(n), top) {
                    (Some(i), Some(Page::Pause)) => vec![format!("pause.perk:{i}")],
                    (Some(i), _) => vec![format!("profile.perk:{i}")],
                    _ => {
                        runlog::kv("menu_action", &format!("action={a} refused=unknown_perk"));
                        Vec::new()
                    }
                }
            } else if let Some(t) = a.strip_prefix("pause_tab:") {
                vec![format!("pause.tab:{t}")]
            } else if let Some(n) = a.strip_prefix("char_pick:") {
                vec![format!("select.pick:{n}")]
            } else if let Some(n) = a.strip_prefix("char_scroll:") {
                vec![format!("select.scroll:{n}")]
            } else if let Some(n) = a.strip_prefix("char_rotate:") {
                vec![format!("rotate:{n}")]
            } else {
                Vec::new()
            }
        }
    }
}

/// The keyboard and mouse state the menus read and then clear.
type InputParams<'w> = (
    ResMut<'w, ButtonInput<KeyCode>>,
    ResMut<'w, ButtonInput<MouseButton>>,
    ResMut<'w, AccumulatedMouseMotion>,
    ResMut<'w, AccumulatedMouseScroll>,
);

#[allow(clippy::too_many_arguments)] // Bevy system parameters
fn menu_input(
    (mut keys, mut mouse, mut motion, mut scroll): InputParams,
    mut state: ResMut<MenuState>,
    hits: Res<MenuHits>,
    (script, frames): (Res<crate::weapons::weapon::ScriptedInput>, Res<bevy::diagnostic::FrameCount>),
    buy: Res<crate::game::buy_menu::BuyMenu>,
    mut window: Query<(&Window, &mut CursorOptions), With<PrimaryWindow>>,
    mut virt: ResMut<Time<Virtual>>,
    (vet, mut perk_requests, mut new_pawn, mut change_char): (
        Res<crate::game::perks::Veterancy>,
        MessageWriter<crate::game::perks::PerkRequest>,
        MessageWriter<crate::game::perks::NewPawn>,
        MessageWriter<crate::player::character::ChangeCharacter>,
    ),
    data: Res<MenuData>,
    mut exit: MessageWriter<AppExit>,
    mut start_vet: Local<Option<crate::game::perks::Vet>>,
    (mut net, mut net_start): (ResMut<crate::net::lobby::NetLobby>, MessageReader<crate::net::lobby::StartLocalMatch>),
    mut audio: Option<ResMut<crate::audio::mixer::Audio>>,
    (mut aim, mut mouse_set): (ResMut<crate::weapons::weapon::AimSetting>, ResMut<crate::engine::mouse::MouseSettings>),
    (mut buttons, mut left_held): (MessageReader<bevy::input::mouse::MouseButtonInput>, Local<bool>),
) {
    // Whether the left button is held, from the raw press / release
    // messages: `mouse` cannot tell, because this system clears it every
    // frame a menu is open (so the weapons do not fire), which ended every
    // slider and 3D-view drag one frame after the press.
    for b in buttons.read() {
        if b.button == MouseButton::Left {
            *left_held = b.state.is_pressed();
        }
    }
    // The perk the weapons were loaded with (start items), for Ready.
    let had = *start_vet.get_or_insert(vet.vet);
    let actions: Vec<&str> = script.0.iter().filter(|(f, _)| *f == frames.0).map(|(_, a)| a.as_str()).collect();
    let mut ids: Vec<String> = Vec::new();
    // KFPlayerController.ShowMidGameMenu (Escape): the buy menu, a GUI
    // page, closes first (buy_menu.rs); the lobby pages ignore it.
    let escape = keys.just_pressed(KeyCode::Escape) || actions.contains(&"pause_menu");
    if escape {
        match state.top() {
            Some(Page::Pause) => ids.push("pause.close".into()),
            Some(Page::Audio) => ids.push("audio.back".into()),
            // A popup page closes on Escape, cancelled (as Cancel).
            Some(Page::ModelSelect) => ids.push("select.cancel".into()),
            None if !buy.open => ids.push("pause.open".into()),
            _ => {}
        }
    }
    for a in &actions {
        ids.extend(scripted_to_ids(a, state.top()));
    }
    let Ok((win, mut cursor)) = window.single_mut() else { return };
    if mouse.just_pressed(MouseButton::Left)
        && !state.stack.is_empty()
        && let Some(pos) = win.physical_cursor_position()
        && let Some((id, _)) = hits.0.iter().rev().find(|(_, r)| r.contains(pos))
    {
        // A press on a 3D view starts a drag (the drop target captures the
        // mouse: OnSpinnyDudeCapturedMouseMove) instead of a click.
        match id.as_str() {
            "profile.drag" => state.drag = Some(Drag::Profile),
            "select.drag" => state.drag = Some(Drag::ModelSelect),
            // GUISlider (bCaptureMouse): the press sets the value at once
            // (InternalOnMousePressed), then it follows the mouse.
            // The aim mode buttons: the mode changes at once and is saved.
            s if s.starts_with("aim.mode:") => {
                if let Ok(hold) = crate::launcher::choices::parse_aim(&s["aim.mode:".len()..]) {
                    state.audio_row = audio_page::AIM_ROW;
                    aim.set(hold, "menu_click");
                }
            }
            // Invert mouse Off / On: changes at once and is saved.
            s if s.starts_with("mouse.invert:") => {
                if let Ok(on) = crate::launcher::choices::parse_on_off(&s["mouse.invert:".len()..]) {
                    state.audio_row = audio_page::INVERT_ROW;
                    mouse_set.set_invert(on, "menu_click");
                }
            }
            // The sensitivity slider: as the volume sliders (saved when
            // let go).
            audio_page::SENS_ID => {
                state.drag = Some(Drag::Sensitivity);
                state.audio_row = audio_page::SENS_ROW;
            }
            s if s.starts_with("volume.slider:") => {
                if let Some(i) = SLIDERS.iter().position(|(n, _)| audio_page::slider_id(n) == s) {
                    state.drag = Some(Drag::Volume(i));
                    state.audio_row = i;
                }
            }
            _ => ids.push(id.clone()),
        }
    }
    if let Some(d) = state.drag {
        if !*left_held {
            state.drag = None;
            // Let go of a slider: the value is saved (KF writes the ini on
            // every change; we write once per drag).
            if let (Drag::Volume(_), Some(a)) = (d, audio.as_deref()) {
                a.save_volumes("slider_released");
            }
            if d == Drag::Sensitivity {
                mouse_set.commit("slider_released");
            }
        } else if d == Drag::Sensitivity {
            if let Some(pos) = win.physical_cursor_position() {
                slide_sensitivity(&mut mouse_set, &hits.0, pos.x);
            }
        } else if let Drag::Volume(i) = d {
            if let (Some(a), Some(pos)) = (audio.as_deref_mut(), win.physical_cursor_position()) {
                slide_to(a, &hits.0, i, pos.x, "menu_drag");
            }
        } else if motion.delta.x != 0.0 {
            // Yaw -= 256 x DeltaX (mouse movement in pixels).
            turn(&mut state, d, motion.delta.x);
        }
    }
    // The Audio window's keys (GUISlider.InternalOnKeyEvent: Left / Right
    // move the focused slider 1% of its range; ours: Up / Down pick it)
    // and the scripted volume actions.
    if state.top() == Some(Page::Audio) {
        let mut keys_down: Vec<&str> = [(KeyCode::ArrowUp, "up"), (KeyCode::ArrowDown, "down"), (KeyCode::ArrowLeft, "left"), (KeyCode::ArrowRight, "right")]
            .into_iter()
            .filter(|(k, _)| keys.just_pressed(*k))
            .map(|(_, w)| w)
            .collect();
        keys_down.extend(actions.iter().filter_map(|a| a.strip_prefix("volume_key:")));
        for k in keys_down {
            let n = SLIDERS.len();
            // The rows: the sliders, then the aim mode, the mouse
            // sensitivity and invert mouse.
            let rows = audio_page::ROWS;
            match k {
                "up" => state.audio_row = (state.audio_row + rows - 1) % rows,
                "down" => state.audio_row = (state.audio_row + 1) % rows,
                // Left = Toggle, Right = Hold (the buttons' order).
                "left" | "right" if state.audio_row == audio_page::AIM_ROW => {
                    aim.set(k == "right", "menu_key");
                }
                // KF's box steps 0.25.
                "left" | "right" if state.audio_row == audio_page::SENS_ROW => {
                    let v = crate::launcher::choices::step_sensitivity(mouse_set.sensitivity, if k == "left" { -1 } else { 1 });
                    mouse_set.set_sensitivity(v, "menu_key");
                }
                // Left = Off, Right = On (the buttons' order).
                "left" | "right" if state.audio_row == audio_page::INVERT_ROW => {
                    mouse_set.set_invert(k == "right", "menu_key");
                }
                "left" | "right" => {
                    if let Some(a) = audio.as_deref_mut() {
                        let name = SLIDERS[state.audio_row.min(n - 1)].0;
                        let (v, max) = a.volumes.slider(name, a.ini_volumes).unwrap_or((0.0, 1.0));
                        let step = if k == "left" { -0.01 } else { 0.01 } * max;
                        set_slider(a, name, v + step, "menu_key");
                        a.save_volumes("key");
                    }
                }
                _ => runlog::kv("menu_action", &format!("action=volume_key:{k} refused=unknown_key")),
            }
            runlog::kv("audio_page", &format!("event=key key={k} row={}", state.audio_row));
        }
    }
    for a in &actions {
        // `mouse_sensitivity:X`: sets the sensitivity (any page; for tests).
        if let Some(v) = a.strip_prefix("mouse_sensitivity:") {
            match crate::launcher::choices::parse_sensitivity(v) {
                Ok(v) => {
                    mouse_set.set_sensitivity(v, "scripted");
                }
                Err(e) => runlog::kv("menu_action", &format!("action={a} refused=\"{e}\"")),
            }
        }
        // `invert_mouse:on|off`: sets invert mouse (any page; for tests).
        if let Some(v) = a.strip_prefix("invert_mouse:") {
            match crate::launcher::choices::parse_on_off(v) {
                Ok(on) => {
                    mouse_set.set_invert(on, "scripted");
                }
                Err(_) => runlog::kv("menu_action", &format!("action={a} refused=not_on_or_off")),
            }
        }
        // `invert_click:on|off`: clicks that button (as drawn last frame).
        if let Some(v) = a.strip_prefix("invert_click:") {
            let on = crate::launcher::choices::parse_on_off(v);
            match on.ok().filter(|on| hits.0.iter().any(|(id, _)| *id == audio_page::invert_id(*on))) {
                Some(on) => {
                    runlog::kv("audio_page", &format!("event=scripted_click button={}", audio_page::invert_id(on)));
                    state.audio_row = audio_page::INVERT_ROW;
                    mouse_set.set_invert(on, "menu_click");
                }
                None => runlog::kv("menu_action", &format!("action={a} refused=no_such_button_on_screen")),
            }
        }
        // `sensitivity_click:FRACTION`: a click at that fraction of the
        // sensitivity slider's box (as drawn last frame), then let go.
        if let Some(f) = a.strip_prefix("sensitivity_click:") {
            let rect = hits.0.iter().find(|(id, _)| id == audio_page::SENS_ID).map(|(_, r)| *r);
            match (rect, f.trim().parse::<f32>()) {
                (Some(r), Ok(f)) => {
                    let x = r.min.x + f * r.width();
                    runlog::kv("audio_page", &format!("event=scripted_click slider=mouse_sensitivity x={x:.0} box=({:.0},{:.0})-({:.0},{:.0})", r.min.x, r.min.y, r.max.x, r.max.y));
                    state.audio_row = audio_page::SENS_ROW;
                    slide_sensitivity(&mut mouse_set, &hits.0, x);
                    mouse_set.commit("slider_released");
                }
                _ => runlog::kv("menu_action", &format!("action={a} refused=no_such_slider_on_screen")),
            }
        }
        // `aim_mode:toggle|hold`: sets the aim mode (any page; for tests).
        if let Some(m) = a.strip_prefix("aim_mode:") {
            match crate::launcher::choices::parse_aim(m) {
                Ok(hold) => {
                    aim.set(hold, "scripted");
                }
                Err(_) => runlog::kv("menu_action", &format!("action={a} refused=not_toggle_or_hold")),
            }
        }
        // `aim_mode_click:toggle|hold`: clicks that button (as drawn last
        // frame), through the mouse's code.
        if let Some(m) = a.strip_prefix("aim_mode_click:") {
            let hold = crate::launcher::choices::parse_aim(m);
            match hold.ok().filter(|h| hits.0.iter().any(|(id, _)| *id == audio_page::aim_id(*h))) {
                Some(hold) => {
                    runlog::kv("audio_page", &format!("event=scripted_click button={}", audio_page::aim_id(hold)));
                    state.audio_row = audio_page::AIM_ROW;
                    aim.set(hold, "menu_click");
                }
                None => runlog::kv("menu_action", &format!("action={a} refused=no_such_button_on_screen")),
            }
        }
    }
    for a in &actions {
        let Some(audio) = audio.as_deref_mut() else { break };
        // `volume:NAME=VALUE`: sets a slider (any page; for tests).
        if let Some((name, v)) = a.strip_prefix("volume:").and_then(|s| s.split_once('=')) {
            match v.trim().parse::<f32>() {
                Ok(v) if SLIDERS.iter().any(|(n, _)| *n == name) => {
                    set_slider(audio, name, v, "scripted");
                    audio.save_volumes("scripted");
                }
                _ => runlog::kv("menu_action", &format!("action={a} refused=bad_slider_or_value")),
            }
        }
        // `volume_click:NAME@FRACTION`: a click at that fraction of the
        // slider's box (as drawn last frame), through the mouse's code.
        if let Some((name, f)) = a.strip_prefix("volume_click:").and_then(|s| s.split_once('@')) {
            let i = SLIDERS.iter().position(|(n, _)| *n == name);
            let rect = hits.0.iter().find(|(id, _)| *id == audio_page::slider_id(name)).map(|(_, r)| *r);
            match (i, rect, f.trim().parse::<f32>()) {
                (Some(i), Some(r), Ok(f)) => {
                    let x = r.min.x + f * r.width();
                    runlog::kv("audio_page", &format!("event=scripted_click slider={name} x={x:.0} box=({:.0},{:.0})-({:.0},{:.0})", r.min.x, r.min.y, r.max.x, r.max.y));
                    state.audio_row = i;
                    slide_to(audio, &hits.0, i, x, "menu_click");
                    audio.save_volumes("slider_released");
                }
                _ => runlog::kv("menu_action", &format!("action={a} refused=no_such_slider_on_screen")),
            }
        }
    }
    // The mouse wheel scrolls the character list one row (Step =
    // NoVisibleCols items).
    if state.top() == Some(Page::ModelSelect) && scroll.delta.y != 0.0 {
        ids.push(format!("select.scroll:{}", if scroll.delta.y > 0.0 { -1 } else { 1 }));
    }
    let was_open = !state.stack.is_empty();
    let sens_drag = state.drag == Some(Drag::Sensitivity);
    for id in ids {
        apply(&id, &mut state, &vet, &data, had, &mut perk_requests, &mut new_pawn, &mut change_char, &mut exit, &mut cursor, &mut net);
    }
    // Escape during a sensitivity drag closes the window: keep and save it.
    if sens_drag && state.drag != Some(Drag::Sensitivity) {
        mouse_set.commit("menu_closed");
    }
    // Network game: the server started the match and this player is ready.
    if net_start.read().count() > 0 && state.lobby_open() {
        start_match(&mut state, &vet, &data, had, &mut new_pawn, &mut cursor, "net_match_started");
    }
    // KF pauses a single-player game while the mid-game menu is open
    // (SetPause(true) when NetMode is NM_StandAlone; closing it unpauses).
    // A network game never pauses.
    let paused = state.stack.contains(&Page::Pause) && !net.active;
    if paused != virt.is_paused() {
        if paused {
            virt.pause();
        } else {
            virt.unpause();
        }
        runlog::kv("pause", &format!("paused={paused} frame={}", frames.0));
    }
    if was_open || !state.stack.is_empty() {
        keys.reset_all();
        mouse.reset_all();
        motion.delta = Vec2::ZERO;
        scroll.delta = Vec2::ZERO;
    }
}

#[allow(clippy::too_many_arguments)]
fn apply(
    id: &str,
    state: &mut MenuState,
    vet: &crate::game::perks::Veterancy,
    data: &MenuData,
    had: crate::game::perks::Vet,
    perk_requests: &mut MessageWriter<crate::game::perks::PerkRequest>,
    new_pawn: &mut MessageWriter<crate::game::perks::NewPawn>,
    change_char: &mut MessageWriter<crate::player::character::ChangeCharacter>,
    exit: &mut MessageWriter<AppExit>,
    cursor: &mut CursorOptions,
    net: &mut crate::net::lobby::NetLobby,
) {
    let top = state.top();
    let on = |p: Page| top == Some(p);
    runlog::kv("menu_action", &format!("id={id} page={top:?}"));
    match id {
        "pause.open" => {
            state.cursor_was_grabbed = cursor.grab_mode != CursorGrabMode::None;
            cursor.grab_mode = CursorGrabMode::None;
            cursor.visible = true;
            state.stack.push(Page::Pause);
            // KFInvasionLoginMenu opens on the Perks tab; its list starts
            // on the selected perk (InitList).
            state.pause_tab = PauseTab::Perks;
            state.pause_row = vet.selected.map_or(0, |p| p.index());
            runlog::kv("menu_open", "page=Pause reason=escape");
        }
        "pause.close" if on(Page::Pause) => {
            state.stack.pop();
            if state.cursor_was_grabbed {
                cursor.grab_mode = CursorGrabMode::Locked;
                cursor.visible = false;
            }
            runlog::kv("menu_close", "page=Pause reason=escape");
        }
        "lobby.ready" if on(Page::Lobby) && net.active => {
            // LobbyFooter.OnFooterClick in a network game: Ready
            // (bReadyToPlay, sent to the server) or, when ready, Unready
            // (ServerUnreadyPlayer). The lobby stays until the server
            // starts the match (net/lobby.rs sends StartLocalMatch).
            net.want_ready = !net.local_ready;
            runlog::kv("lobby_ready", &format!("player=\"{}\" perk={} ready={} net=true", data.player_name, vet.vet.label(), net.want_ready));
        }
        "lobby.ready" if on(Page::Lobby) => {
            // LobbyFooter.OnFooterClick: SendSelectedVeterancyToServer(true),
            // ServerRestartPlayer (the pawn spawns), bReadyToPlay; solo: the
            // match starts and the menu closes.
            start_match(state, vet, data, had, new_pawn, cursor, "ready");
        }
        "lobby.perks" if on(Page::Lobby) => {
            // KFProfilePage: the list starts on the selected perk.
            state.profile_row = vet.selected.map_or(0, |p| p.index());
            state.profile_portrait = false;
            // KFTab_Profile.Opened: Yaw 32768 (facing the camera).
            state.profile_yaw = 0;
            state.stack.push(Page::Profile);
            runlog::kv("menu_open", &format!("page=Profile reason=select_perk row={}", state.profile_row));
        }
        "lobby.options" if on(Page::Lobby) => runlog::kv("menu_inert", "button=Options reason=no_settings_page"),
        "lobby.disconnect" if on(Page::Lobby) => {
            // KF: DISCONNECT and back to the main menu; we have none.
            runlog::kv("menu_quit", "button=Disconnect");
            quit(exit, net);
        }
        "profile.save" if on(Page::Profile) => {
            // KFTab_Profile.SaveSettings: ChangeCharacter if sChar changed,
            // then SetSelectedVeterancy / SendSelectedVeterancyToServer;
            // KFProfilePage.SaveButtonClicked closes the page.
            if !data.characters.is_empty() || !state.profile_char.is_empty() {
                change_char.write(crate::player::character::ChangeCharacter(state.profile_char.clone()));
            }
            let perk = Perk::ALL[state.profile_row.min(6)];
            perk_requests.write(crate::game::perks::PerkRequest(perk));
            state.stack.pop();
            runlog::kv("menu_close", &format!("page=Profile reason=save perk={} character={}", perk.class(), state.profile_char));
        }
        "profile.pick" if on(Page::Profile) => {
            // KFTab_Profile.PickModel: OpenMenu("KFGui.KFModelSelect",
            // PlayerRec.DefaultName); HandleParameters highlights that
            // character; Opened turns the model to face the camera.
            state.select_index = data.characters.iter().position(|(n, _)| n.eq_ignore_ascii_case(&state.profile_char)).unwrap_or(0);
            state.select_top = model_select::top_row_showing(state.select_index, 0, data.characters.len());
            state.select_yaw = 0;
            state.stack.push(Page::ModelSelect);
            runlog::kv("menu_open", &format!("page=ModelSelect reason=change_character current={} characters={}", state.profile_char, data.characters.len()));
            log_highlight(state, data, "opened");
        }
        "select.ok" if on(Page::ModelSelect) => {
            // LockedFloatingWindow OK: CloseMenu(false); ModelSelectClosed
            // takes GetDataString (the highlighted name) as sChar and
            // SetPlayerRec updates the 3D View, portrait and biography.
            // SAVE applies it.
            state.stack.pop();
            if let Some((n, _)) = data.characters.get(state.select_index) {
                state.profile_char = n.clone();
            }
            runlog::kv("menu_close", &format!("page=ModelSelect reason=ok character={} applied_on=save", state.profile_char));
        }
        "select.cancel" if on(Page::ModelSelect) => {
            state.stack.pop();
            runlog::kv("menu_close", &format!("page=ModelSelect reason=cancel character={}", state.profile_char));
        }
        "profile.3d" if on(Page::Profile) => {
            state.profile_portrait = !state.profile_portrait;
            runlog::kv("profile_view", &format!("portrait={}", state.profile_portrait));
        }
        "pause.save" if on(Page::Pause) && state.pause_tab == PauseTab::Perks => {
            // KFTab_MidGamePerks.OnSaveButtonClicked: SetSelectedVeterancy,
            // SendSelectedVeterancyToServer (perks.rs applies KF's rule).
            let perk = Perk::ALL[state.pause_row.min(6)];
            perk_requests.write(crate::game::perks::PerkRequest(perk));
        }
        "pause.settings" if on(Page::Pause) => {
            // KF opens its Settings page (Audio is one of its tabs); ours
            // is only the volume window. A solo game stays paused under it
            // (the pause menu is still open); a network game never pauses.
            state.audio_row = 0;
            state.stack.push(Page::Audio);
            runlog::kv("menu_open", "page=Audio reason=settings_button");
        }
        "audio.back" if on(Page::Audio) => {
            state.stack.pop();
            state.drag = None;
            runlog::kv("menu_close", "page=Audio reason=back");
        }
        "pause.spectate" => runlog::kv("menu_inert", &format!("button={id} reason=not_built")),
        "pause.forfeit" | "pause.exit" if on(Page::Pause) => {
            // Forfeit: DISCONNECT, back to the main menu (none here); Exit
            // Game: KFQuitPage asks first (not built). Both quit.
            runlog::kv("menu_quit", &format!("button={id}"));
            quit(exit, net);
        }
        _ => {
            if let Some(i) = id.strip_prefix("profile.perk:").and_then(|n| n.parse::<usize>().ok()).filter(|_| on(Page::Profile)) {
                state.profile_row = i.min(6);
                runlog::kv("perk_page", &format!("page=Profile row={} perk={} level={}", state.profile_row, Perk::ALL[state.profile_row].class(), vet.level));
            } else if let Some(i) = id.strip_prefix("pause.perk:").and_then(|n| n.parse::<usize>().ok()).filter(|_| on(Page::Pause)) {
                state.pause_row = i.min(6);
                runlog::kv("perk_page", &format!("page=Pause row={} perk={} level={}", state.pause_row, Perk::ALL[state.pause_row].class(), vet.level));
            } else if let Some(i) = id.strip_prefix("select.cell:").and_then(|n| n.parse::<usize>().ok()).filter(|_| on(Page::ModelSelect)) {
                // GUIVertImageList.InternalOnClick -> SetIndex -> ListChange.
                if i < data.characters.len() {
                    state.select_index = i;
                    log_highlight(state, data, "click");
                }
            } else if let Some(n) = id.strip_prefix("select.pick:").filter(|_| on(Page::ModelSelect)) {
                match data.characters.iter().position(|(c, _)| c.eq_ignore_ascii_case(n)) {
                    Some(i) => {
                        state.select_index = i;
                        state.select_top = model_select::top_row_showing(i, state.select_top, data.characters.len());
                        log_highlight(state, data, "scripted");
                    }
                    None => runlog::kv("menu_action", &format!("action=char_pick:{n} refused=unknown_character")),
                }
            } else if let Some(n) = id.strip_prefix("select.scroll:").and_then(|n| n.parse::<i64>().ok()).filter(|_| on(Page::ModelSelect)) {
                let max = model_select::max_top_row(data.characters.len()) as i64;
                state.select_top = (state.select_top as i64 + n).clamp(0, max) as usize;
                runlog::kv("model_select", &format!("event=scroll top_row={} max_top_row={max}", state.select_top));
            } else if let Some(px) = id.strip_prefix("rotate:").and_then(|n| n.parse::<f32>().ok()) {
                // A scripted drag on the top page's 3D view.
                let d = if on(Page::ModelSelect) { Drag::ModelSelect } else { Drag::Profile };
                turn(state, d, px);
                runlog::kv("preview_turn", &format!("view={d:?} pixels={px} profile_yaw={} select_yaw={}", state.profile_yaw, state.select_yaw));
            } else if let Some(t) = id.strip_prefix("pause.tab:").filter(|_| on(Page::Pause)) {
                state.pause_tab = match t {
                    "communication" => PauseTab::Communication,
                    "help" => PauseTab::Help,
                    _ => PauseTab::Perks,
                };
                runlog::kv("pause_tab", &format!("tab={:?}", state.pause_tab));
            } else {
                runlog::kv("menu_action_ignored", &format!("id={id} page={top:?}"));
            }
        }
    }
}

/// Sets a volume slider (`master`, `effects`, `music`) if the value
/// changed; heard from the next frame on (Audio::set_volumes logs it).
fn set_slider(audio: &mut crate::audio::mixer::Audio, name: &str, v: f32, source: &str) {
    let mut vols = audio.volumes;
    if vols.set_slider(name, v) && vols != audio.volumes {
        audio.set_volumes(vols, &format!("{source} slider={name}"));
    }
}

/// A slider follows the mouse's x (physical pixels) over its box as drawn
/// last frame (GUISlider.InternalCapturedMouseMove).
fn slide_to(audio: &mut crate::audio::mixer::Audio, hits: &[(String, Rect)], i: usize, x: f32, source: &str) {
    let Some((name, _)) = SLIDERS.get(i) else { return };
    let Some((_, r)) = hits.iter().find(|(id, _)| *id == audio_page::slider_id(name)) else { return };
    let (_, max) = audio.volumes.slider(name, audio.ini_volumes).unwrap_or((0.0, 1.0));
    set_slider(audio, name, gui::slider_fraction(*r, x) * max, source);
}

/// The sensitivity slider follows the mouse's x (physical pixels) over
/// its box as drawn last frame, on KF's 0.25 steps (saved when let go).
fn slide_sensitivity(mouse: &mut crate::engine::mouse::MouseSettings, hits: &[(String, Rect)], x: f32) {
    let Some((_, r)) = hits.iter().find(|(id, _)| id == audio_page::SENS_ID) else { return };
    mouse.drag_sensitivity(crate::launcher::choices::sensitivity_at_fraction(gui::slider_fraction(*r, x)));
}

/// Turns a preview by a mouse movement of `dx` pixels
/// (KFTab_Profile.OnSpinnyDudeCapturedMouseMove: Yaw -= 256 x DeltaX).
fn turn(state: &mut MenuState, d: Drag, dx: f32) {
    let delta = -(256.0 * dx).round() as i32;
    let yaw = match d {
        Drag::Profile => &mut state.profile_yaw,
        Drag::ModelSelect => &mut state.select_yaw,
        Drag::Volume(_) | Drag::Sensitivity => return,
    };
    *yaw = (*yaw + delta).rem_euclid(65536);
}

/// Logs the character list's highlighted character (ListChange).
fn log_highlight(state: &MenuState, data: &MenuData, why: &str) {
    let name = data.characters.get(state.select_index).map_or("", |(n, _)| n.as_str());
    runlog::kv("model_select", &format!("event=highlight why={why} index={} name={name} top_row={}", state.select_index, state.select_top));
}

/// Quits; a network game first says goodbye (net/lobby.rs `leave`).
fn quit(exit: &mut MessageWriter<AppExit>, net: &mut crate::net::lobby::NetLobby) {
    if net.active {
        net.quit_requested = true;
    } else {
        exit.write(AppExit::Success);
    }
}

/// The lobby closes and the match starts for this player: the pawn gets
/// the perk's start items, the wave timer runs (waves.rs waits for the
/// lobby to close).
fn start_match(
    state: &mut MenuState,
    vet: &crate::game::perks::Veterancy,
    data: &MenuData,
    had: crate::game::perks::Vet,
    new_pawn: &mut MessageWriter<crate::game::perks::NewPawn>,
    cursor: &mut CursorOptions,
    reason: &str,
) {
    state.stack.retain(|p| !matches!(p, Page::Lobby | Page::Profile | Page::ModelSelect));
    state.drag = None;
    new_pawn.write(crate::game::perks::NewPawn { had });
    cursor.grab_mode = CursorGrabMode::Locked;
    cursor.visible = false;
    if reason == "ready" {
        runlog::kv("lobby_ready", &format!("player=\"{}\" perk={} ", data.player_name, vet.vet.label()));
    }
    runlog::kv("menu_close", &format!("page=Lobby reason={reason}"));
}

/// Tells player/body/preview.rs what each character preview shows this
/// frame: the perk page's 3D View (KFTab_Profile.InternalDraw) while the
/// page is open in 3D mode, and the character select window's model
/// (UT2k4ModelSelect.InternalOnDraw) while it is open.
fn request_previews(
    state: Res<MenuState>,
    gui: Res<Gui>,
    data: Res<MenuData>,
    window: Query<&Window, With<PrimaryWindow>>,
    preview: Option<ResMut<crate::player::body::CharacterPreview>>,
) {
    use crate::player::body::{PREVIEW_MODEL_SELECT, PREVIEW_PROFILE, PreviewRequest};
    let Some(mut preview) = preview else { return };
    let Ok(win) = window.single() else { return };
    let screen = Rect::new(0.0, 0.0, win.physical_width() as f32, win.physical_height() as f32);
    let size = |r: Rect| UVec2::new(r.width().round().max(1.0) as u32, r.height().round().max(1.0) as u32);
    // KFTab_Profile.InitComponent / KFModelSelect.InitComponent:
    // SetDrawScale(0.9).
    const DRAW_SCALE: f32 = 0.9;
    let profile = (gui.loaded && state.stack.contains(&Page::Profile) && !state.profile_portrait).then(|| {
        // SpinnyDudeOffset.X + (ClipX / ClipY) x 120 in front of the
        // camera (ClipX / ClipY: the whole canvas, saved before the clip
        // is set).
        let off = Vec3::new(
            gui.num("KFGui.KFTab_Profile.SpinnyDudeOffset.X", 120.0),
            gui.num("KFGui.KFTab_Profile.SpinnyDudeOffset.Y", 0.0),
            gui.num("KFGui.KFTab_Profile.SpinnyDudeOffset.Z", 0.0),
        );
        PreviewRequest {
            character: state.profile_char.clone(),
            size: size(profile::preview_box(&gui, screen)),
            fov_deg: gui.num("KFGui.KFTab_Profile.nfov", 15.0),
            offset: Vec3::new(off.x + screen.width() / screen.height() * 120.0, off.y, off.z),
            yaw: 32768 + state.profile_yaw,
            draw_scale: DRAW_SCALE,
        }
    });
    let select = (gui.loaded && state.top() == Some(Page::ModelSelect)).then(|| data.characters.get(state.select_index)).flatten().map(|(name, _)| {
        PreviewRequest {
            character: name.clone(),
            size: size(model_select::preview_box(&gui, screen)),
            fov_deg: gui.num("KFGui.KFModelSelect.nfov", 15.0),
            // KFModelSelect.UpdateSpinnyDude: (250, 1, -24); (250, 1, -14)
            // for the Juggernaut race and Axon / Cyclops / Virus (UT2004
            // characters; none of KF's records).
            offset: Vec3::new(250.0, 1.0, -24.0),
            yaw: 32768 + state.select_yaw,
            draw_scale: DRAW_SCALE,
        }
    });
    if preview.requests[PREVIEW_PROFILE] != profile {
        preview.requests[PREVIEW_PROFILE] = profile;
    }
    if preview.requests[PREVIEW_MODEL_SELECT] != select {
        preview.requests[PREVIEW_MODEL_SELECT] = select;
    }
}

/// Points the menus' preview textures at the previews' current images
/// (an image is replaced when its box changes size).
fn sync_previews(mut gui: ResMut<Gui>, preview: Option<Res<crate::player::body::CharacterPreview>>, images: Res<Assets<Image>>) {
    let Some(preview) = preview else { return };
    for (slot, image) in preview.images.iter().enumerate() {
        let (Some(&t), Some(image)) = (gui.previews.get(slot), image.as_ref()) else { continue };
        if gui.textures[t].image != *image
            && let Some(img) = images.get(image)
        {
            let size = img.size_f32();
            gui.textures[t] = crate::game::hud::HudTexture { image: image.clone(), size };
        }
    }
}

/// What the page drawers read.
pub(crate) struct DrawCtx<'a> {
    pub state: &'a MenuState,
    pub data: &'a MenuData,
    pub vet: &'a crate::game::perks::Veterancy,
    pub players: Vec<LobbyPlayer>,
    /// WaveNumber + 1 and FinalWave (None: no game data, "?/?").
    pub wave: Option<(usize, usize)>,
    /// The Ready button's caption (LobbyFooter: "Unready" when ready in a
    /// network game).
    pub ready_caption: &'static str,
    /// KFGRI.LobbyTimeout: above 0, "Game will auto-commence in: N".
    pub lobby_timeout: i32,
}

#[allow(clippy::too_many_arguments)] // Bevy system parameters
fn draw_menus(
    gui: Res<Gui>,
    state: Res<MenuState>,
    data: Res<MenuData>,
    vet: Res<crate::game::perks::Veterancy>,
    (game, game_data): (Res<crate::game::waves::WaveGame>, Option<Res<crate::game::waves::GameData>>),
    window: Query<&Window, With<PrimaryWindow>>,
    mut slots: Query<(&MenuSlot, &mut Node, &mut ImageNode, &mut Visibility)>,
    mut hits: ResMut<MenuHits>,
    mut shown: Local<usize>,
    (script, frames): (Res<crate::weapons::weapon::ScriptedInput>, Res<bevy::diagnostic::FrameCount>),
    net: Res<crate::net::lobby::NetLobby>,
    (mut nu, mut classic): (crate::game::numenu::NuDraw, crate::game::classic_menu::ClassicDraw),
    audio: Option<Res<crate::audio::mixer::Audio>>,
    (aim, mouse_set): (Res<crate::weapons::weapon::AimSetting>, Res<crate::engine::mouse::MouseSettings>),
) {
    if !gui.loaded {
        return;
    }
    // (The first-person weapon is hidden in the lobby by
    // engine/view_target.rs.)
    let Ok(win) = window.single() else { return };
    let mut p = gui::Painter::new(&gui, Vec2::new(win.width(), win.height()), win.scale_factor(), win.physical_cursor_position());
    let ctx = DrawCtx {
        state: &state,
        data: &data,
        vet: &vet,
        // A network game: every connected player (net/lobby.rs); solo:
        // the local player only.
        players: if net.active {
            net.players.clone()
        } else {
            vec![LobbyPlayer {
                name: data.player_name.clone(),
                perk: vet.selected,
                level: vet.level,
                ready: !state.lobby_open(),
            }]
        },
        wave: game_data.as_ref().map(|d| (game.wave_num + 1, d.waves.len())),
        ready_caption: if net.active && net.local_ready { "Unready" } else { "Ready" },
        lobby_timeout: if net.active { net.lobby_timeout } else { -1 },
    };
    let previews = |slot: usize| gui.previews.get(slot).copied();
    match state.top() {
        Some(Page::Lobby) => lobby::draw(&mut p, &ctx),
        Some(Page::Profile) => profile::draw(&mut p, &ctx, previews(crate::player::body::PREVIEW_PROFILE)),
        Some(Page::ModelSelect) => {
            // The perk page stays drawn under the popup; only the popup
            // takes clicks.
            profile::draw(&mut p, &ctx, previews(crate::player::body::PREVIEW_PROFILE));
            p.hits.clear();
            // PopupPageBase.FadeIn: the pages below fade from CurFade 200
            // to DesiredFade 80 (of 255) over FadeTime 0.35 s. Drawn as a
            // black layer at once (the fade's animation and the exact
            // native use of the colour are guesses).
            let screen = p.screen;
            p.fill(screen, [0, 0, 0, 255 - 80], "ModelSelect.Fade");
            model_select::draw(&mut p, &ctx, previews(crate::player::body::PREVIEW_MODEL_SELECT));
        }
        Some(Page::Pause) => pause::draw(&mut p, &ctx),
        Some(Page::Audio) => {
            // The pause menu stays drawn under the window, darkened (as
            // under KFModelSelect); only the window takes clicks.
            pause::draw(&mut p, &ctx);
            p.hits.clear();
            let screen = p.screen;
            p.fill(screen, [0, 0, 0, 255 - 80], "Audio.Fade");
            let view = audio.as_deref().map(|a| audio_page::AudioView { volumes: a.volumes, ini: a.ini_volumes, muted: a.capture.muted() });
            let drag = match state.drag {
                Some(Drag::Volume(i)) => Some(i),
                _ => None,
            };
            let controls = audio_page::ControlsView { aim_hold: aim.hold, sensitivity: mouse_set.sensitivity, invert: mouse_set.invert, sens_drag: state.drag == Some(Drag::Sensitivity) };
            audio_page::draw(&mut p, view.as_ref(), state.audio_row, drag, &controls);
        }
        // The trader's NuMenu (game/numenu.rs) uses the same painter.
        None if nu.showing() => crate::game::numenu::draw(&mut p, &mut nu),
        // The classic KF trader menu (game/classic_menu.rs), the same way.
        None if classic.showing() => crate::game::classic_menu::draw(&mut p, &mut classic),
        None => {}
    }
    if script.0.iter().any(|(f, a)| *f == frames.0 && a == "menu_dump") {
        let lines: Vec<String> = p
            .canvas
            .quads
            .iter()
            .filter(|q| !q.what.contains('\''))
            .map(|q| format!("{}:({:.0},{:.0})-({:.0},{:.0})", q.what, q.screen.min.x, q.screen.min.y, q.screen.max.x, q.screen.max.y))
            .collect();
        runlog::kv(
            "menu_dump",
            &format!("page={:?} window={:.0}x{:.0} quads={} hits=[{}] {}", state.top(), win.width(), win.height(), p.canvas.quads.len(), p.hits.iter().map(|(i, r)| format!("{i}:({:.0},{:.0})-({:.0},{:.0})", r.min.x, r.min.y, r.max.x, r.max.y)).collect::<Vec<_>>().join(" "), lines.join(" ")),
        );
    }
    hits.0 = std::mem::take(&mut p.hits);
    gui::flush(&p.canvas.quads, &gui, &mut slots, &mut shown);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn perk_pick_goes_to_the_open_page() {
        assert_eq!(scripted_to_ids("perk_pick:demo", Some(Page::Profile)), vec!["profile.perk:6".to_string()]);
        assert_eq!(scripted_to_ids("perk_pick:medic", Some(Page::Pause)), vec!["pause.perk:0".to_string()]);
        assert!(scripted_to_ids("perk_pick:nobody", Some(Page::Pause)).is_empty());
        assert_eq!(scripted_to_ids("pause_tab:help", Some(Page::Pause)), vec!["pause.tab:help".to_string()]);
    }

    #[test]
    fn lobby_hides_the_hud_pause_does_not() {
        let mut s = MenuState::default();
        assert!(!s.hides_hud() && !s.lobby_open());
        s.stack.push(Page::Lobby);
        s.stack.push(Page::Profile);
        assert!(s.hides_hud() && s.lobby_open());
        s.stack.clear();
        s.stack.push(Page::Pause);
        assert!(!s.hides_hud() && !s.lobby_open());
    }

    #[test]
    fn slider_value_follows_the_mouse_inside_the_marker_margins() {
        // 220 x 20 box: marker 28 wide, so 14 px in from each end is 0 / 1.
        let r = Rect::new(100.0, 0.0, 320.0, 20.0);
        assert_eq!(gui::slider_fraction(r, 114.0), 0.0);
        assert_eq!(gui::slider_fraction(r, 306.0), 1.0);
        assert!((gui::slider_fraction(r, 210.0) - 0.5).abs() < 1e-6);
        // Outside the box: the ends (a drag that leaves it).
        assert_eq!(gui::slider_fraction(r, -50.0), 0.0);
        assert_eq!(gui::slider_fraction(r, 9000.0), 1.0);
        assert_eq!(scripted_to_ids("volume_page", Some(Page::Pause)), vec!["pause.settings".to_string()]);
    }

    #[test]
    fn section_client_area_uses_image_offset_and_padding() {
        let r = gui::Painter::section_client(Rect::new(0.0, 0.0, 130.0, 145.0), [0.1, 0.0, 0.0, 0.0]);
        assert_eq!((r.min.x, r.min.y, r.max.x, r.max.y), (30.0, 35.0, 120.0, 135.0));
    }
}
