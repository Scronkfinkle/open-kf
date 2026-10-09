//! KF's weapon selection bar (HUDKillingFloor's inventory display): the
//! mouse wheel opens it and moves the highlight, Fire takes the
//! highlighted weapon, Escape closes it. hud.rs draws it from `WeaponBar`.
//! docs/DESIGN.md, "The weapon selection bar".

use bevy::prelude::*;

use crate::engine::runlog;

/// HUDKillingFloor's 5 inventory groups (Categorized[5]).
pub const GROUPS: usize = 5;

/// The bar's state, written by the weapon input, read by the HUD.
#[derive(Resource, Default)]
pub struct WeaponBar {
    /// bDisplayInventory.
    pub shown: bool,
    /// When the last fade in (shown) or fade out (hidden) started
    /// (InventoryFadeStartTime), in seconds of game time; None before the
    /// bar was ever opened.
    pub fade_start: Option<f32>,
    /// SelectedInventory: the highlighted weapon's class.
    pub highlighted: Option<String>,
    /// The weapons the bar shows (InventoryGroup 1-5, not thrown away), in
    /// inventory-list order: (class, group). Refreshed every frame.
    pub items: Vec<(String, u8)>,
    /// KFPlayerController.Fire sets bFire = 0: the click that took a weapon
    /// does not shoot. Ours: the button is ignored until it is let go.
    pub swallow_fire: bool,
}

impl WeaponBar {
    /// ShowInventory: shown, fade in, the highlight on the weapon in hand.
    fn show(&mut self, now: f32, in_hand: &str) {
        if !self.shown {
            self.shown = true;
            self.fade_start = Some(now);
            self.highlighted = Some(in_hand.to_string());
        }
    }

    /// HideInventory: fade out.
    pub fn hide(&mut self, now: f32, reason: &str) {
        if self.shown {
            self.shown = false;
            self.fade_start = Some(now);
            self.log(reason);
        }
    }

    /// Item counts per group ("1,1,0,0,2").
    pub fn group_counts(&self) -> String {
        let mut n = [0; GROUPS];
        for (_, g) in &self.items {
            n[usize::from(*g) - 1] += 1;
        }
        n.map(|c| c.to_string()).join(",")
    }

    pub fn log(&self, reason: &str) {
        runlog::kv(
            "weapon_bar",
            &format!(
                "shown={} highlighted={} groups={} reason={reason}",
                self.shown,
                self.highlighted.as_deref().unwrap_or("none"),
                self.group_counts()
            ),
        );
    }

    /// The wheel (KFPlayerController.NextWeapon / PrevWeapon -> the HUD's):
    /// open the bar if needed, then move the highlight one step.
    pub fn step(&mut self, now: f32, in_hand: &str, forward: bool) {
        if self.items.is_empty() {
            return;
        }
        self.show(now, in_hand);
        let groups: Vec<u8> = self.items.iter().map(|(_, g)| *g).collect();
        let at = self
            .highlighted
            .as_deref()
            .and_then(|h| self.items.iter().position(|(c, _)| c.eq_ignore_ascii_case(h)))
            .or_else(|| self.items.iter().position(|(c, _)| c.eq_ignore_ascii_case(in_hand)))
            .unwrap_or(0);
        let next = step_highlight(&groups, at, forward);
        self.highlighted = Some(self.items[next].0.clone());
        self.log(if forward { "next" } else { "prev" });
    }
}

/// HUDKillingFloor.NextWeapon / PrevWeapon: `groups` are the bar's items'
/// groups (1-5) in inventory-list order, `at` the highlighted item. The
/// next (previous) item of the same group, else the first (last) item of
/// the next (previous) group that has any, wrapping round; with only one
/// group, its first (last) item.
pub fn step_highlight(groups: &[u8], at: usize, forward: bool) -> usize {
    let mut cats: [Vec<usize>; GROUPS] = Default::default();
    for (i, &g) in groups.iter().enumerate() {
        cats[usize::from(g) - 1].push(i);
    }
    let cat = usize::from(groups[at]) - 1;
    let idx = cats[cat].iter().position(|&i| i == at).unwrap_or(0);
    let len = cats[cat].len();
    if forward {
        if idx + 1 < len {
            return cats[cat][idx + 1];
        }
        (1..GROUPS).map(|k| (cat + k) % GROUPS).find(|&c| !cats[c].is_empty()).map_or(cats[cat][0], |c| cats[c][0])
    } else {
        if idx > 0 {
            return cats[cat][idx - 1];
        }
        (1..GROUPS)
            .map(|k| (cat + GROUPS - k) % GROUPS)
            .find(|&c| !cats[c].is_empty())
            .map_or(cats[cat][len - 1], |c| *cats[c].last().expect("not empty"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The starting inventory as the bar sees it: knife (1), 9mm (2),
    /// syringe and welder (5).
    const START: [u8; 4] = [1, 2, 5, 5];

    #[test]
    fn next_walks_the_groups_and_wraps() {
        assert_eq!(step_highlight(&START, 1, true), 2); // 9mm -> syringe
        assert_eq!(step_highlight(&START, 2, true), 3); // syringe -> welder
        assert_eq!(step_highlight(&START, 3, true), 0); // welder -> knife
    }

    #[test]
    fn prev_takes_the_last_of_the_previous_group() {
        assert_eq!(step_highlight(&START, 0, false), 3); // knife -> welder
        assert_eq!(step_highlight(&START, 2, false), 1); // syringe -> 9mm
        assert_eq!(step_highlight(&START, 3, false), 2); // welder -> syringe
    }

    #[test]
    fn one_group_wraps_inside_it() {
        assert_eq!(step_highlight(&[3, 3], 1, true), 0);
        assert_eq!(step_highlight(&[3, 3], 0, false), 1);
    }
}
