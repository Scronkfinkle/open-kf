//! Step 4: the shared scoreboard (KF's KFScoreBoard, shown while Tab is
//! held: User.ini `Tab=ScoreToggle`, "ShowScores | OnRelease HideScores").
//!
//! Every game puts its player's kills, dosh, deaths and health into its
//! pawn update (net/pawns.rs), so every game has everyone's. The layout
//! follows KFScoreBoard.UpdateScoreBoard: one box per player on 70% of the
//! screen width starting 10 lines down, the title (difficulty | Wave N |
//! the map's title) at 13% of the height, columns PLAYER (7.5%), Kills
//! (50%), Assists (60%), Status (70%: "N HP" green / gold / red, or
//! "DEAD" in red), Cash (80%, "£ N", yellow; "£ N.NNK" from 1000), and
//! the local player's name in red, in ROHud's small menu font, on
//! BoxMaterial boxes at alpha 128. Not drawn: the perk icons and stars and
//! the PING column (no ping measured yet). Assists are always 0 (not
//! counted yet).

use bevy::prelude::*;

use super::lobby::NetLobby;
use super::protocol::{NetPawn, NetPlayer};
use crate::engine::runlog;

/// Test inputs: "scores_on" / "scores_off" (Tab held / released).
pub(super) fn build(app: &mut App) {
    app.init_resource::<Scoreboard>().add_systems(Update, (collect_scores, draw_scoreboard).chain());
}

/// One scoreboard row (a KFPlayerReplicationInfo's shown values).
#[derive(Clone, Debug, PartialEq)]
struct Row {
    peer: u64,
    name: String,
    kills: u32,
    assists: u32,
    dosh: i32,
    deaths: u32,
    health: i32,
    dead: bool,
    me: bool,
}

#[derive(Resource, Default)]
struct Scoreboard {
    rows: Vec<Row>,
    shown: bool,
    /// What was last logged (and when), and what is on screen.
    logged: Option<(Vec<Row>, f32)>,
    drawn_key: String,
    /// The node pool is spawned (draw_scoreboard).
    root: Option<Entity>,
    /// Real seconds when my match began (GRI.ElapsedTime from then).
    started_at: Option<f32>,
}

/// KFScoreBoard.InOrder: kills, then assists, then cash (more first),
/// then the name.
fn sort_rows(rows: &mut [Row]) {
    rows.sort_by(|a, b| b.kills.cmp(&a.kills).then(b.assists.cmp(&a.assists)).then(b.dosh.cmp(&a.dosh)).then(a.name.cmp(&b.name)));
}

/// The Cash column: "£" @ int(Score), or "£" @ (Score / 1000) $ "K" from
/// 1000 (UnrealScript prints a float with two decimals).
fn cash_text(dosh: i32) -> String {
    if dosh >= 1000 { format!("£ {:.2}K", dosh as f32 / 1000.0) } else { format!("£ {dosh}") }
}

/// StreamBase.FormatTimeDisplay style: [h:]m:ss.
fn format_time(seconds: f32) -> String {
    let s = seconds.max(0.0) as u32;
    let (h, m, sec) = (s / 3600, (s / 60) % 60, s % 60);
    if h > 0 { format!("{h}:{m:02}:{sec:02}") } else { format!("{m}:{sec:02}") }
}

#[allow(clippy::too_many_arguments)] // Bevy system parameters
fn collect_scores(
    time: Res<Time<Real>>,
    lobby: Res<NetLobby>,
    players: Query<&NetPlayer>,
    pawns: Query<&NetPawn>,
    (kills, dosh, health): (Res<crate::game::combat::KillCount>, Res<crate::game::dosh::Dosh>, Res<crate::game::combat::PlayerHealth>),
    keys: Res<ButtonInput<KeyCode>>,
    (script, frames): (Res<crate::weapons::weapon::ScriptedInput>, Res<bevy::diagnostic::FrameCount>),
    buy: Res<crate::game::buy_menu::BuyMenu>,
    mut board: ResMut<Scoreboard>,
    mut scripted: Local<bool>,
) {
    let now = time.elapsed_secs();
    if lobby.start_sent && board.started_at.is_none() {
        board.started_at = Some(now);
    }
    let mut rows: Vec<Row> = players
        .iter()
        .map(|p| {
            let me = lobby.my_peer == Some(p.peer);
            match (me, pawns.iter().find(|n| n.peer == p.peer)) {
                (true, _) => Row { peer: p.peer, name: p.name.clone(), kills: kills.0, assists: 0, dosh: dosh.score as i32, deaths: health.deaths, health: health.health.max(0.0).round() as i32, dead: health.dead, me },
                (false, Some(n)) => Row { peer: p.peer, name: p.name.clone(), kills: n.state.kills, assists: 0, dosh: n.state.dosh, deaths: n.state.deaths, health: n.state.health, dead: n.state.dead, me },
                // No pawn yet (still in the lobby): KF shows the PRI's starting values.
                (false, None) => Row { peer: p.peer, name: p.name.clone(), kills: 0, assists: 0, dosh: crate::game::dosh::starting_cash() as i32, deaths: 0, health: 100, dead: false, me },
            }
        })
        .collect();
    sort_rows(&mut rows);
    for (f, a) in &script.0 {
        if *f == frames.0 {
            match a.as_str() {
                "scores_on" => *scripted = true,
                "scores_off" => *scripted = false,
                _ => {}
            }
        }
    }
    let shown = lobby.start_sent && !buy.open && (keys.pressed(KeyCode::Tab) || *scripted);
    if shown != board.shown {
        board.shown = shown;
        runlog::kv("scoreboard_shown", &format!("shown={shown} rows={}", rows.len()));
    }
    // The data every game holds about every player (logged when it
    // changes, at most once a second).
    let changed = board.logged.as_ref().is_none_or(|(r, _)| *r != rows);
    if changed && board.logged.as_ref().is_none_or(|(_, t)| now - t >= 1.0) && lobby.start_sent {
        let line: Vec<String> = rows
            .iter()
            .map(|r| format!("{}{}:kills={}:dosh={}:deaths={}:health={}:dead={}", r.name, if r.me { "(me)" } else { "" }, r.kills, r.dosh, r.deaths, r.health, r.dead))
            .collect();
        runlog::kv("scoreboard", &line.join(" | "));
        board.logged = Some((rows.clone(), now));
    }
    board.rows = rows;
}

/// HudBase's colours (Engine.HUD defaults) and MakeColor(255, 255, 125).
const WHITE: [u8; 4] = [255, 255, 255, 255];
const RED: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const GOLD: [u8; 4] = [255, 255, 0, 255];
const CASH: [u8; 4] = [255, 255, 125, 255];
/// ScoreBoard BoxMaterial (KFScoreBoard default).
pub const BOX_MATERIAL: &str = "InterfaceArt_tex.Menu.changeme_texture";
/// UI image nodes for the scoreboard (one per glyph or box); above the HUD
/// (10..) and the end-game screen (200), under the menus (1000..).
const POOL: usize = 600;

#[derive(Component)]
struct ScoreSlot(usize);

#[allow(clippy::too_many_arguments, clippy::type_complexity)] // Bevy system parameters
fn draw_scoreboard(
    mut commands: Commands,
    time: Res<Time<Real>>,
    mut board: ResMut<Scoreboard>,
    window: Query<&Window>,
    title: Option<Res<crate::world::map::LevelTitle>>,
    game: Res<crate::game::waves::WaveGame>,
    health: Res<crate::game::combat::PlayerHealth>,
    gui: Option<Res<crate::game::menus::gui::Gui>>,
    mut slots: Query<(&ScoreSlot, &mut Node, &mut ImageNode, &mut Visibility)>,
    mut shown_quads: Local<usize>,
) {
    let Some(gui) = gui.filter(|g| g.loaded) else { return };
    if board.root.is_none() {
        // The node pool (spawned once; `root` only marks it done).
        for i in 0..POOL {
            commands.spawn((
                Node { position_type: PositionType::Absolute, ..default() },
                ImageNode { image_mode: bevy::ui::widget::NodeImageMode::Stretch, ..default() },
                Visibility::Hidden,
                GlobalZIndex(300 + i as i32),
                ScoreSlot(i),
            ));
        }
        board.root = Some(Entity::PLACEHOLDER);
        return;
    }
    let Ok(win) = window.single() else { return };
    let mut p = crate::game::menus::gui::Painter::new(&gui, Vec2::new(win.width(), win.height()), win.scale_factor(), None);
    let elapsed = board.started_at.map_or(0.0, |s| time.elapsed_secs() - s);
    // DrawTitle: SkillLevel[BaseDifficulty] | Wave N | Level.Title; then
    // "Elapsed Time:" or, dead, OutFireText (bOutOfLives).
    let title_line = format!("{} | Wave {} | {}", crate::game::difficulty::current().name(), game.wave_num + 1, title.map_or(String::new(), |t| t.0.clone()));
    let info_line = if health.dead { "   You are dead. Fire to view other players.".to_string() } else { format!("Elapsed Time: {}", format_time(elapsed)) };
    if board.shown {
        let (cx, cy) = (p.width(), p.screen.height());
        // UpdateScoreBoard: GetSmallMenuFont, YL from StrLen("Test").
        let font = crate::game::menus::gui::small_menu_font(cx);
        let yl = p.text_size(font, "Test").y.max(1.0);
        let n = board.rows.len().max(1) as f32;
        let space = 0.25 * yl;
        let message_foot = 1.5 * 7.0 * yl;
        let row_h = ((1.25 + (cy - 0.67 * message_foot)) / n - space).clamp(1.2 * yl, 2.125 * yl);
        let header_y = 10.0 * yl;
        let box_w = 0.7 * cx;
        let box_x = (0.5 * (cx - box_w)).floor();
        let box_w = cx - 2.0 * box_x;
        let col = |f: f32| box_x + f * box_w;
        // The boxes: STY_Alpha, white, alpha 128, DrawTileStretched(BoxMaterial).
        let tex = gui.tex(BOX_MATERIAL);
        for i in 0..board.rows.len() {
            let r = Rect::new(box_x, header_y + (row_h + space) * i as f32, box_x + box_w, header_y + (row_h + space) * i as f32 + row_h);
            match tex {
                Some(_) => p.stretched(tex, r, [255, 255, 255, 128], "score_box"),
                None => p.fill(r, [255, 255, 255, 128], "score_box"),
            }
        }
        let centred = |p: &mut crate::game::menus::gui::Painter, s: &str, x: f32, y: f32, c: [u8; 4]| {
            let w = p.text_size(font, s).x;
            p.text(font, s, Vec2::new((x - 0.5 * w).round(), y.round()), c, "score_text");
        };
        let title_y = cy * 0.13;
        centred(&mut p, &title_line, cx / 2.0, title_y, RED);
        centred(&mut p, &info_line, cx / 2.0, title_y + yl, RED);
        let head_y = header_y - 1.1 * yl;
        p.text(font, "PLAYER", Vec2::new(col(0.075).round(), head_y.round()), WHITE, "score_text");
        centred(&mut p, "Kills", col(0.50), head_y, WHITE);
        centred(&mut p, "Assists", col(0.60), head_y, WHITE);
        centred(&mut p, "Status", col(0.70), head_y, WHITE);
        centred(&mut p, "Cash", col(0.80), head_y, WHITE);
        let rows = board.rows.clone();
        for (i, r) in rows.iter().enumerate() {
            let y = header_y + 0.5 * (row_h - yl) + (row_h + space) * i as f32;
            p.text(font, &r.name, Vec2::new(col(0.075).round(), y.round()), if r.me { RED } else { WHITE }, "score_text");
            centred(&mut p, &r.kills.to_string(), col(0.50), y, WHITE);
            centred(&mut p, &r.assists.to_string(), col(0.60), y, WHITE);
            let (status, c) = if r.dead {
                ("  DEAD".to_string(), RED)
            } else {
                (format!("{} HP", r.health), if r.health >= 80 { GREEN } else if r.health >= 50 { GOLD } else { RED })
            };
            centred(&mut p, &status, col(0.70), y, c);
            centred(&mut p, &cash_text(r.dosh), col(0.80), y, CASH);
        }
        let key = format!("{cx}x{cy} {title_line} {info_line} {:?}", board.rows);
        if key != board.drawn_key {
            runlog::kv(
                "scoreboard_drawn",
                &format!("rows={} screen={cx}x{cy} font={font} yl={yl} row_h={row_h:.0} box_texture={} title=\"{title_line}\" info=\"{}\" quads={}", board.rows.len(), tex.is_some(), info_line.trim(), p.canvas.quads.len()),
            );
            board.drawn_key = key;
        }
    } else {
        board.drawn_key.clear();
    }
    // Copy the quads to the node pool (as the menus' gui::flush).
    let quads = &p.canvas.quads;
    let n = quads.len().min(POOL);
    let touch = n.max(*shown_quads);
    for (slot, mut node, mut image, mut vis) in &mut slots {
        if slot.0 >= touch {
            continue;
        }
        let Some(q) = quads.get(slot.0) else {
            *vis = Visibility::Hidden;
            continue;
        };
        node.left = Val::Px(q.screen.min.x);
        node.top = Val::Px(q.screen.min.y);
        node.width = Val::Px(q.screen.width());
        node.height = Val::Px(q.screen.height());
        let t = &gui.textures[q.texture];
        if image.image != t.image {
            image.image = t.image.clone();
        }
        image.rect = Some(q.uv);
        image.color = Color::srgba_u8(q.tint[0], q.tint[1], q.tint[2], q.tint[3]);
        *vis = Visibility::Inherited;
    }
    *shown_quads = n;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: &str, kills: u32, dosh: i32) -> Row {
        Row { peer: 0, name: name.into(), kills, assists: 0, dosh, deaths: 0, health: 100, dead: false, me: false }
    }

    #[test]
    fn rows_sort_like_kf_and_cash_reads_like_kf() {
        let mut rows = vec![row("b", 3, 100), row("a", 3, 100), row("c", 5, 0), row("d", 3, 900)];
        sort_rows(&mut rows);
        let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["c", "d", "a", "b"]);
        assert_eq!(cash_text(250), "£ 250");
        assert_eq!(cash_text(1250), "£ 1.25K");
        assert_eq!(format_time(75.0), "1:15");
        assert_eq!(format_time(3725.0), "1:02:05");
    }
}
