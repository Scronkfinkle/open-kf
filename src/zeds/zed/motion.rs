//! Zed movement state and rules that do not need the world: falling
//! bookkeeping (DESIGN.md, "Pathfinding").

/// Per-zed movement bookkeeping.
#[derive(Default, Clone, Copy, Debug)]
pub struct Motion {
    /// Seconds in the current fall (0 when not falling).
    pub fall_seconds: f32,
}

/// A fall longer than this is logged (`zed_fall_long`): a normal drop or
/// jump lasts well under a second.
pub const LONG_FALL: f32 = 3.0;
