//! The lobby page (KFGui.LobbyMenu.InternalOnPreDraw / DrawPerk, its
//! LobbyFooter, KFPlayerReadyBar, KFLobbyChat). Boxes from KFGui.u;
//! where native code places things, the fits to references/lobby.png are
//! labelled.

use bevy::prelude::*;

use super::DrawCtx;
use super::gui::{Align, Painter, State, named_font};
use super::perk_panel;


pub(super) fn draw(p: &mut Painter, c: &DrawCtx) {
    let gui = p.gui;
    let screen = p.screen;
    let w = p.width();
    let white = [255, 255, 255, 255];
    let menu_font = named_font("UT2MenuFont", w);

    // label_TimeOutCounter (LobbyMenu.Timer: LobbyTimeout <= 0 ->
    // WaitingForOtherPlayers, else AutoCommence$":" @ LobbyTimeout; KFGui.int).
    let l = gui.comp("LobbyMenu.TimeOutCounter");
    let status = if c.lobby_timeout > 0 { format!("Game will auto-commence in: {}", c.lobby_timeout) } else { "Waiting for players to be ready...".to_string() };
    p.text_in(menu_font, &status, l.rect(screen), l.text_align(), true, l.color.unwrap_or(white), "TimeOutCounter");

    // The six player rows: first the players not ready, then the rest
    // (InternalOnPreDraw), empty rows after.
    let mut order: Vec<_> = c.players.iter().filter(|pl| !pl.ready).collect();
    order.extend(c.players.iter().filter(|pl| pl.ready));
    for i in 0..6 {
        let bar = gui.comp(&format!("LobbyMenu.Player{}BackDrop", i + 1)).rect(screen);
        // KFPlayerReadyBar.ResizeMe: PerkBG a square of the bar's height
        // (Item_box_box, ImageStyle Scaled), PlayerBG the rest, 10% in
        // from top and bottom (Item_box_bar, Stretched).
        let h = bar.height();
        p.tile(gui.tex("KF_InterfaceArt_tex.Menu.Item_box_box"), Rect::new(bar.min.x, bar.min.y, bar.min.x + h, bar.max.y), white, &format!("PerkBG{i}"));
        p.stretched(gui.tex("KF_InterfaceArt_tex.Menu.Item_box_bar"), Rect::new(bar.min.x + h, bar.min.y + 0.1 * h, bar.max.x, bar.min.y + 0.9 * h), white, &format!("PlayerBG{i}"));
        let rb = gui.comp(&format!("LobbyMenu.ReadyBox{i}"));
        // moCheckBox is bStandardized: StandardHeight (0.03 of the screen)
        // high from its top; caption and check box are centred in that.
        let rbr = rb.rect(screen);
        let rbr = Rect::new(rbr.min.x, rbr.min.y, rbr.max.x, rbr.min.y + 0.03 * screen.height());
        // moCheckBox: the check box after CaptionWidth, a square of
        // StandardHeight (0.03 of the screen: bStandardized), centred.
        let side = 0.03 * screen.height();
        let bx = rbr.min.x + rb.caption_width * rbr.width();
        let check = Rect::new(bx, rbr.center().y - side / 2.0, bx + side, rbr.center().y + side / 2.0);
        p.tile(gui.tex("KF_InterfaceArt_tex.Menu.Item_box_box_Disabled"), check, white, &format!("ReadyBox{i}"));
        let Some(pl) = order.get(i) else { continue };
        if pl.ready {
            p.tile(gui.tex("KF_InterfaceArt_tex.Menu.Checkbox"), check, white, &format!("ReadyCheck{i}"));
        }
        // SetCaption(Left(PlayerName, 20)) in LabelFont. Colour: the data says
        // LabelColor (10,10,10,210), but the screenshot shows light text:
        // the TextLabel style colour (KF_TextLabel 200,200,200,200) is used
        // (a guess: the style wins over LabelColor).
        let name: String = pl.name.chars().take(20).collect();
        let cap = Rect::new(rbr.min.x, rbr.min.y, rbr.min.x + rb.caption_width * rbr.width(), rbr.max.y);
        p.text_in(named_font(&rb.font, w), &name, cap, Align::Left, true, [200, 200, 200, 200], &format!("PlayerName{i}"));
        if let Some(perk) = pl.perk {
            // PlayerPerk: OnHUDIcon, a square of its own height
            // (InitComponent), moved in by half the size difference.
            let pp = gui.comp(&format!("LobbyMenu.Player{}P", i + 1)).rect(screen);
            let side = pp.height();
            let x = pp.min.x + (bar.height() - side) / 2.0;
            p.tile(gui.tex(perk.icons().0), Rect::new(x, pp.min.y, x + side, pp.min.y + side), white, &format!("PlayerPerk{i}"));
            // PlayerVetLabel: LvAbbrString @ level @ VeterancyName.
            let vl = gui.comp(&format!("LobbyMenu.Player{}Veterancy", i + 1));
            let text = format!("Lv {} {}", pl.level, c.data.texts.names[perk.index()]);
            p.text_in(named_font(&vl.font, w), &text, vl.rect(screen), vl.text_align(), true, vl.color.unwrap_or(white), &format!("PlayerVet{i}"));
        }
    }

    // The video panel (ADBG, sized in DrawPerk: 320 x 240 x ClipX / 1024
    // plus borders). KF plays Movies/MovieN.bik (Bink): we cannot decode
    // it, so the movie area stays black.
    let ad = gui.comp("LobbyMenu.ADBG").rect(screen);
    let s = w / 1024.0;
    let ad = Rect::from_corners(ad.min, ad.min + Vec2::new(320.0 * s + 10.0, 240.0 * s + 37.0));
    p.section(ad, "", false, "ADBG");
    let movie = Rect::from_corners(ad.min + Vec2::new(5.0, 30.0), ad.min + Vec2::new(5.0 + 320.0 * s, 30.0 + 240.0 * s));
    p.fill(movie, [0, 0, 0, 255], "Movie(not_played)");

    // Map, difficulty (LobbyMenu: BeginnerString .. HellOnEarthString by
    // KFGRI.BaseDifficulty) and wave.
    p.section(gui.comp("LobbyMenu.GameInfoB").rect(screen), "", true, "GameInfoB");
    for (name, text) in [("LobbyMenu.CurrentMapL", format!("Current Map: {}", c.data.map_title)), ("LobbyMenu.DifficultyL", format!("Difficulty Level: {}", crate::game::difficulty::current().name()))] {
        let l = gui.comp(name);
        p.text_in(named_font(&l.font, w), &text, l.rect(screen), l.text_align(), true, l.color.unwrap_or(white), name);
    }
    // WaveBG: Hud_Bio_Circle, ImageStyle Justified (a square at the
    // left); WaveLabel.WinWidth = its height, the text centred in it.
    let wb = gui.comp("LobbyMenu.WaveB").rect(screen);
    let side = wb.height().min(wb.width());
    let sq = Rect::from_corners(wb.min, wb.min + Vec2::splat(side));
    p.tile(gui.tex("KillingFloorHUD.HUD.Hud_Bio_Circle"), sq, white, "WaveB");
    let wl = gui.comp("LobbyMenu.WaveL");
    let wave = c.wave.map_or("?/?".to_string(), |(n, f)| format!("{n}/{f}"));
    p.text_in(named_font(&wl.font, w), &wave, sq, Align::Center, true, wl.color.unwrap_or(white), "WaveL");

    // The map's story (KFMapStoryLabel in the StoryBoxBackground; inset
    // fitted to the screenshot: 20 px in, 25 down).
    let story = gui.comp("LobbyMenu.StoryBoxBackground").rect(screen);
    p.section(story, "", true, "StoryBox");
    let text_box = Rect::new(story.min.x + 20.0, story.min.y + 25.0, story.max.x - 10.0, story.max.y - 10.0);
    p.scroll_text(menu_font, &c.data.map_description, text_box, [225, 225, 225, 255], "Story");

    // The character portrait (DrawPortrait: the record of the URL's
    // Character; i_Portrait 30 px below the box top, 36 px shorter).
    let pb = gui.comp("LobbyMenu.PlayerPortraitB").rect(screen);
    p.section(pb, "", false, "PlayerPortraitB");
    let pi = gui.comp("LobbyMenu.PlayerPortrait").rect(screen);
    let pi = Rect::new(pi.min.x, pb.min.y + 30.0, pi.max.x, pb.min.y + 30.0 + pb.height() - 36.0);
    let portrait = c.data.characters.iter().find(|(n, _)| n.eq_ignore_ascii_case(&c.state.profile_char)).and_then(|(_, t)| gui.tex(t));
    p.tile(portrait, pi, white, &format!("Portrait:{}", c.state.profile_char));

    // "Current Perk" (DrawPerk: nothing without a selected perk).
    let bg = gui.comp("LobbyMenu.BGPerk");
    let bgr = bg.rect(screen);
    p.section(bgr, &bg.caption, false, "BGPerk");
    let fx = gui.comp("LobbyMenu.BGPerkEffects");
    p.section(fx.rect(screen), &fx.caption, false, "BGPerkEffects");
    if let Some(perk) = c.vet.selected {
        let at = Vec2::new(bgr.min.x + 5.0, bgr.min.y + 30.0);
        let (iw, ih) = (bgr.width() - 10.0, bgr.height() - 37.0);
        perk_panel::draw_row(p, at, iw, ih, perk, c.vet.level, &c.data.texts.names[perk.index()], false, false, true);
        perk_panel::draw_effects(p, gui.comp("LobbyMenu.PerkEffectsScroll").rect(screen), &c.data.texts, perk.index(), c.vet.level);
    }
    // PerkClickLabel: clicking the perk area opens the perk page.
    p.hit("lobby.perks", gui.comp("LobbyMenu.PerkClickArea").rect(screen));

    // KFLobbyChat: "Say:" and the edit box (moEditBox, StandardHeight
    // 0.03, CaptionWidth 0.1; KF_EditBox: Innerborder). Offline: inert.
    let chat = gui.comp("LobbyMenu.ChatBox").rect(screen);
    let eb = gui.comp("KFLobbyChat.ebSend");
    let ebr = eb.rect(chat);
    let ebr = Rect::new(ebr.min.x, ebr.min.y, ebr.max.x, ebr.min.y + 0.03 * screen.height());
    let split = ebr.min.x + eb.caption_width * ebr.width();
    p.text_in(menu_font, eb.caption.trim_end(), Rect::new(ebr.min.x, ebr.min.y, split, ebr.max.y), Align::Left, true, [200, 200, 200, 200], "ChatSay");
    p.stretched(gui.tex("KF_InterfaceArt_tex.Menu.Innerborder"), Rect::new(split, ebr.min.y, ebr.max.x, ebr.max.y), white, "ChatEdit");

    // LobbyFooter (ButtonFooter at the bottom, 0.05 high; style Footer =
    // ROSTY_Footer: Thin_border).
    let foot = Rect::new(0.0, screen.max.y - 0.05 * screen.height(), screen.max.x, screen.max.y);
    p.stretched(gui.tex("KF_InterfaceArt_tex.Menu.Thin_border"), foot, white, "LobbyFooter");
    let buttons = [("lobby.perks", "LobbyFooter.Perks"), ("lobby.options", "LobbyFooter.Options"), ("lobby.ready", "LobbyFooter.ReadyButton"), ("lobby.disconnect", "LobbyFooter.Cancel")];
    let captions: Vec<String> = buttons
        .iter()
        .map(|(id, comp)| if *id == "lobby.ready" { c.ready_caption.to_string() } else { gui.comp(comp).caption })
        .collect();
    footer_buttons(p, foot, &buttons.iter().map(|(id, _)| *id).collect::<Vec<_>>(), &captions, 0.035);
}

/// ButtonFooter with bAutoSize and bFixedWidth, Alignment right: every
/// button as wide as the longest caption plus Padding (0.16) x the
/// footer's height (GetPadding: a guess fitted to the screenshot),
/// ButtonHeight x the screen height high, centred in the footer, 5 px
/// apart (GetSpacer: fitted), right edge Margin (0.009) / 2 x width in.
pub(super) fn footer_buttons(p: &mut Painter, foot: Rect, ids: &[&str], captions: &[String], button_height: f32) {
    let font = named_font("UT2MenuFont", p.width());
    let longest = captions.iter().map(|c| p.text_size(font, c).x).fold(0.0, f32::max);
    let bw = longest + 0.16 * foot.height();
    let bh = button_height * p.screen.height();
    let gap = 5.0 * p.width() / 2560.0;
    let total = bw * ids.len() as f32 + gap * (ids.len() as f32 - 1.0);
    let mut x = foot.max.x - 0.009 * foot.width() / 2.0 - total;
    let y = foot.center().y - bh / 2.0;
    for (id, cap) in ids.iter().zip(captions) {
        p.button(id, Rect::new(x, y, x + bw, y + bh), cap, State::Blurry);
        x += bw + gap;
    }
}
