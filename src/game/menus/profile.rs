//! The perk page (KFGui.KFProfilePage holding KFTab_Profile), opened by
//! the lobby's "Select Perk". Boxes from KFGui.u, relative to the tab
//! panel (KFProfilePage.Panel).
//!
//! PLACEHOLDER: the "3D View" box. KF draws the character model there
//! (KFSpinnyWeap, `Profile_idle`); we have no third-person player body
//! yet (built on another branch). Until it is merged the box stays
//! empty; "Portrait" switches to the 2D portrait, which works.

use bevy::prelude::*;

use super::DrawCtx;
use super::gui::{Painter, State, named_font};
use super::perk_panel;

pub(super) fn draw(p: &mut Painter, c: &DrawCtx) {
    let gui = p.gui;
    let screen = p.screen;
    let white = [255, 255, 255, 255];
    let panel = gui.comp("KFProfilePage.Panel").rect(screen);

    // 3D View.
    let v = gui.comp("KFTab_Profile.BG3DView");
    p.section(v.rect(panel), &v.caption, false, "BG3DView");
    if c.state.profile_portrait {
        // ShowSpinnyDude with bRenderDude off: i_Portrait.Image =
        // PlayerRec.Portrait (ImageStyle Scaled).
        let portrait = c.data.characters.iter().find(|(n, _)| n.eq_ignore_ascii_case(&c.state.profile_char)).and_then(|(_, t)| gui.tex(t));
        p.tile(portrait, gui.comp("KFTab_Profile.PlayerPortrait").rect(panel), white, &format!("Portrait:{}", c.state.profile_char));
    }
    // else: the 3D character model (placeholder, see the module comment).
    // b_3DView's caption: ShowPortraitCaption while the model shows.
    let caption = if c.state.profile_portrait { "3D View" } else { "Portrait" };
    p.button("profile.3d", gui.comp("KFTab_Profile.Player3DView").rect(panel), caption, State::Blurry);
    let pick = gui.comp("KFTab_Profile.bPickModel");
    p.button("profile.pick", pick.rect(panel), &pick.caption, State::Blurry);

    // Select Perk: ShowPanel moves the list 6 px right, 38 px down, 10 px
    // narrower and 35 px shorter than its section.
    let bp = gui.comp("KFTab_Profile.BGPerks");
    let bpr = bp.rect(panel);
    p.section(bpr, &bp.caption, false, "BGPerks");
    let list = Rect::new(bpr.min.x + 6.0, bpr.min.y + 38.0, bpr.max.x - 4.0, bpr.max.y + 3.0);
    let row = c.state.profile_row.min(6);
    perk_panel::draw_list(p, list, &c.data.texts, row, c.vet.level, "profile.perk");

    let fx = gui.comp("KFTab_Profile.BGPerkEffects");
    p.section(fx.rect(panel), &fx.caption, false, "BGPerkEffects");
    perk_panel::draw_effects(p, gui.comp("KFTab_Profile.PerkEffectsScroll").rect(panel), &c.data.texts, row, c.vet.level);

    let nl = gui.comp("KFTab_Profile.BGPerksNextLevel");
    p.section(nl.rect(panel), &nl.caption, false, "BGPerksNextLevel");
    perk_panel::draw_requirements(p, gui.comp("KFTab_Profile.PerkProgressList").rect(panel), &c.data.texts, row);

    // Biography: lb_Scroll managed by the section (LeftPadding etc. 0.02),
    // LoadDecoText("KFGUI", DefaultName).
    let bio = gui.comp("KFTab_Profile.BGBiography");
    let bior = bio.rect(panel);
    p.section(bior, &bio.caption, false, "BGBiography");
    let text = c.data.bios.get(&c.state.profile_char.to_ascii_lowercase()).cloned().unwrap_or_default();
    p.scroll_text(named_font("UT2MenuFont", p.width()), &text, Painter::section_client(bior, [0.02; 4]), [225, 225, 225, 255], "Biography");

    // KFProfileAndAchievements_Footer: SAVE (FooterButton style), right
    // aligned, WinWidth 0.09 (bAutoSize off), ButtonHeight 0.035.
    let foot = Rect::new(0.0, screen.max.y - 0.05 * screen.height(), screen.max.x, screen.max.y);
    p.stretched(gui.tex("KF_InterfaceArt_tex.Menu.Thin_border"), foot, white, "ProfileFooter");
    let save = gui.comp("KFProfileAndAchievements_Footer.BackB");
    let bw = save.win[2] * foot.width();
    let bh = 0.035 * screen.height();
    let x = foot.max.x - 0.009 * foot.width() / 2.0 - bw;
    let y = foot.center().y - bh / 2.0;
    p.button("profile.save", Rect::new(x, y, x + bw, y + bh), &save.caption, State::Blurry);
}
