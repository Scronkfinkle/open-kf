//! KF's menu pages: the pre-game lobby (KFGui.LobbyMenu, LobbyFooter),
//! the perk page it opens (KFProfilePage / KFTab_Profile) and the pause
//! menu (KFInvasionLoginMenu / KFTab_MidGamePerks). See DESIGN.md,
//! "Menus: the lobby, the perk page, the pause menu".

pub mod gui;
mod lobby;
mod pause;
mod perk_panel;
mod profile;

use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};

use crate::engine::runlog;
use crate::game::perks::Perk;
use gui::{Gui, MenuSlot, POOL};
use perk_panel::PerkTexts;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    Lobby,
    /// KFProfilePage ("Select Perk" from the lobby).
    Profile,
    /// KFInvasionLoginMenu (Escape).
    Pause,
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
    pub pause_tab: PauseTab,
    /// The pause menu's highlighted perk row.
    pub pause_row: usize,
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
        self.stack.iter().any(|p| matches!(p, Page::Lobby | Page::Profile))
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
    /// (DefaultName, portrait texture path) of the characters to step
    /// through, sorted by name.
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
            .add_systems(PostUpdate, draw_menus);
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
    if settings.open {
        // The portraits and biographies are only needed by the lobby.
        let set = ue_assets::package_set::PackageSet::new(root);
        let defaults = ue_assets::class_defaults::ClassDefaults::new(&set);
        data.characters = crate::player::character::selectable_records(&set, &defaults, root).into_iter().map(|r| (r.name, r.portrait)).collect();
        extra.extend(data.characters.iter().map(|(_, p)| p.clone()).filter(|p| !p.is_empty()));
        let kfgui = gui::read_latin1(&root.join("System").join("KFGui.int"));
        data.bios = crate::audio::music::int_section(&kfgui, "DecoText").into_iter().map(|(k, v)| (k, unquote(&v))).collect();
    }
    gui::load(&mut gui, root, &extra, &mut images);
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
        "change_character" => vec!["profile.pick".into()],
        "toggle_portrait" => vec!["profile.3d".into()],
        "pause_select_perk" => vec!["pause.save".into()],
        "pause_forfeit" => vec!["pause.forfeit".into()],
        "pause_exit" => vec!["pause.exit".into()],
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
) {
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
        ids.push(id.clone());
    }
    let was_open = !state.stack.is_empty();
    for id in ids {
        apply(&id, &mut state, &vet, &data, had, &mut perk_requests, &mut new_pawn, &mut change_char, &mut exit, &mut cursor, &mut net);
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
            // KF opens KFModelSelect (a portrait grid, not built); our
            // stand-in steps to the next character.
            if !data.characters.is_empty() {
                let i = data.characters.iter().position(|(n, _)| n.eq_ignore_ascii_case(&state.profile_char)).map_or(0, |i| (i + 1) % data.characters.len());
                state.profile_char = data.characters[i].0.clone();
            }
            runlog::kv("character_pick", &format!("name={} applied_on=save", state.profile_char));
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
        "pause.settings" | "pause.spectate" => runlog::kv("menu_inert", &format!("button={id} reason=not_built")),
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
    state.stack.retain(|p| !matches!(p, Page::Lobby | Page::Profile));
    new_pawn.write(crate::game::perks::NewPawn { had });
    cursor.grab_mode = CursorGrabMode::Locked;
    cursor.visible = false;
    if reason == "ready" {
        runlog::kv("lobby_ready", &format!("player=\"{}\" perk={} ", data.player_name, vet.vet.label()));
    }
    runlog::kv("menu_close", &format!("page=Lobby reason={reason}"));
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
    match state.top() {
        Some(Page::Lobby) => lobby::draw(&mut p, &ctx),
        Some(Page::Profile) => profile::draw(&mut p, &ctx),
        Some(Page::Pause) => pause::draw(&mut p, &ctx),
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
    fn section_client_area_uses_image_offset_and_padding() {
        let r = gui::Painter::section_client(Rect::new(0.0, 0.0, 130.0, 145.0), [0.1, 0.0, 0.0, 0.0]);
        assert_eq!((r.min.x, r.min.y, r.max.x, r.max.y), (30.0, 35.0, 120.0, 135.0));
    }
}
