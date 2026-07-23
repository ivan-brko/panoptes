//! Accordion sizing and transition for the three-pane layout
//!
//! The focused pane is widened and the other two shrink. How aggressively
//! depends on the terminal: the accordion exists to fight scarcity, so it
//! backs off toward equal thirds once there is room for everyone.
//!
//! | terminal width | focused | each side | side mode |
//! |---|---|---|---|
//! | < 60           | 100%        | -       | hidden  |
//! | 60-87          | total - 20  | 10      | strip   |
//! | 88-139         | 50%         | 25%     | compact |
//! | 140-199        | 45%         | ~27%    | full    |
//! | >= 200         | 40%         | 30%     | full    |
//!
//! The 87/88 boundary is a deliberate discontinuity: below it three readable
//! panes are impossible, so the sides become strips and the focused pane
//! absorbs the freed width.
//!
//! Two invariants hold at every frame, including mid-transition:
//! - the three widths sum to exactly the terminal width, because only the two
//!   boundaries between panes are interpolated and the widths are derived from
//!   them;
//! - a pane's render density comes from its *current* width, so a pane can
//!   cross strip -> compact part-way through a transition.

use std::time::{Duration, Instant};

/// Width of a side pane while the focused pane is eating the terminal
pub const SIDE_STRIP_COLS: u16 = 10;
/// Narrowest pane that can still render the compact row format
pub const SIDE_COMPACT_MIN: u16 = 22;
/// Narrowest pane that can render the full row format
pub const SIDE_FULL_MIN: u16 = 40;
/// Below this terminal width only the focused pane is shown
pub const SINGLE_PANE_BELOW: u16 = 60;
/// Terminal width at which three readable panes first fit
const THREE_PANE_MIN: u16 = 88;

/// How long a focus change takes to settle
const TRANSITION: Duration = Duration::from_millis(140);

/// How much of a pane's content survives its current width
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SideMode {
    /// Not on screen at all (terminal too narrow for three panes)
    Hidden,
    /// A counter and nothing else, e.g. `S 7`
    Strip,
    /// The name and a short state, e.g. `● CC review [Exec]`
    Compact,
    /// Everything, e.g. `● CC 3: panoptes / pan-6 / review [Executing: Bash]`
    Full,
}

/// The density a pane of `width` columns can render at
///
/// Includes the pane's own border, which is why the thresholds are two wider
/// than the content widths they are named for.
pub fn side_mode(width: u16) -> SideMode {
    if width == 0 {
        SideMode::Hidden
    } else if width < SIDE_COMPACT_MIN {
        SideMode::Strip
    } else if width < SIDE_FULL_MIN {
        SideMode::Compact
    } else {
        SideMode::Full
    }
}

/// Width the focused pane should have at this terminal width
fn focused_width(total: u16) -> u16 {
    if total < SINGLE_PANE_BELOW {
        total
    } else if total < THREE_PANE_MIN {
        // Sides shrink to fixed strips so the focused pane stays usable
        total.saturating_sub(2 * SIDE_STRIP_COLS)
    } else if total < 140 {
        total / 2
    } else if total < 200 {
        (total as u32 * 45 / 100) as u16
    } else {
        (total as u32 * 40 / 100) as u16
    }
}

/// The two boundaries between the three panes, left to right
///
/// Boundaries rather than widths so that the widths derived from them always
/// sum to `total`, at rest and at every interpolated frame.
pub fn pane_boundaries(total: u16, focused: usize) -> (u16, u16) {
    let focused = focused.min(2);
    let focused_w = focused_width(total).min(total);
    let remaining = total - focused_w;

    // Below the single-pane threshold the two side panes are zero-width, which
    // `widths_from_boundaries` renders as "not there".
    let (first_side, second_side) = if total < SINGLE_PANE_BELOW {
        (0, 0)
    } else {
        // Odd remainders give the extra column to the left-most side pane, so
        // the split is a function of the width alone and never drifts.
        ((remaining + 1) / 2, remaining / 2)
    };

    let widths = match focused {
        0 => [focused_w, first_side, second_side],
        1 => [first_side, focused_w, second_side],
        _ => [first_side, second_side, focused_w],
    };
    (widths[0], widths[0] + widths[1])
}

/// The three pane widths implied by the boundaries
pub fn widths_from_boundaries(total: u16, boundaries: (u16, u16)) -> [u16; 3] {
    let b1 = boundaries.0.min(total);
    let b2 = boundaries.1.clamp(b1, total);
    [b1, b2 - b1, total - b2]
}

/// Resting widths and side density for a terminal width and focused pane
///
/// The returned [`SideMode`] describes the narrower of the two side panes, and
/// is only a summary: rendering asks [`side_mode`] about each pane's own
/// current width, which is what makes a mid-transition pane render honestly.
pub fn pane_widths(total: u16, focused: usize) -> ([u16; 3], SideMode) {
    let widths = widths_from_boundaries(total, pane_boundaries(total, focused));
    let focused = focused.min(2);
    let narrowest_side = widths
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != focused)
        .map(|(_, w)| *w)
        .min()
        .unwrap_or(0);
    (widths, side_mode(narrowest_side))
}

/// Ease-out cubic: fast at first, settling gently
fn ease_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

/// The animated split between the three panes
///
/// Holds the boundary pair it is moving from, the pair it is moving to, and
/// when it started. A second `Tab` mid-flight retargets from wherever the
/// panes currently are; nothing is ever queued, so holding `Tab` cannot
/// overshoot or build up a backlog.
#[derive(Debug, Clone)]
pub struct PaneLayout {
    /// Terminal width the boundaries are expressed in
    total: u16,
    /// Pane the layout is expanding toward
    focused: usize,
    /// Boundaries when the current transition started
    from: (f32, f32),
    /// Boundaries the transition is heading for
    to: (u16, u16),
    /// When the current transition started
    started_at: Instant,
    /// Whether a transition is still in flight
    animating: bool,
}

impl PaneLayout {
    /// A layout already settled at its resting widths
    pub fn new(total: u16, focused: usize, now: Instant) -> Self {
        let to = pane_boundaries(total, focused);
        Self {
            total,
            focused,
            from: (to.0 as f32, to.1 as f32),
            to,
            started_at: now,
            animating: false,
        }
    }

    /// The pane the layout is expanding toward
    pub fn focused(&self) -> usize {
        self.focused
    }

    /// Terminal width the current boundaries are expressed in
    pub fn total_width(&self) -> u16 {
        self.total
    }

    /// Whether a transition is still in flight
    pub fn is_animating(&self) -> bool {
        self.animating
    }

    /// Boundaries as of `now`, interpolated when a transition is in flight
    fn boundaries_at(&self, now: Instant) -> (f32, f32) {
        if !self.animating {
            return (self.to.0 as f32, self.to.1 as f32);
        }
        let t = ease_out(
            now.saturating_duration_since(self.started_at).as_secs_f32() / TRANSITION.as_secs_f32(),
        );
        (
            self.from.0 + (self.to.0 as f32 - self.from.0) * t,
            self.from.1 + (self.to.1 as f32 - self.from.1) * t,
        )
    }

    /// The three pane widths as of `now`; always sums to the terminal width
    pub fn widths_at(&self, now: Instant) -> [u16; 3] {
        let (b1, b2) = self.boundaries_at(now);
        widths_from_boundaries(self.total, (b1.round() as u16, b2.round() as u16))
    }

    /// Aim at a new resting position, starting from wherever the panes are now
    fn retarget(&mut self, total: u16, focused: usize, now: Instant) {
        let current = self.boundaries_at(now);
        let to = pane_boundaries(total, focused);
        if to == self.to && total == self.total && focused == self.focused {
            return;
        }
        self.total = total;
        self.focused = focused;
        // A shrunken terminal can leave the current boundaries outside it;
        // clamping keeps `widths_from_boundaries` honest on the first frame.
        self.from = (
            current.0.clamp(0.0, total as f32),
            current.1.clamp(0.0, total as f32),
        );
        self.to = to;
        self.started_at = now;
        self.animating = true;
    }

    /// Move focus to `focused`, animating from the current widths
    pub fn set_focus(&mut self, focused: usize, now: Instant) {
        self.retarget(self.total, focused, now);
    }

    /// Adopt a new terminal width, animating from the current widths
    ///
    /// A resize retargets rather than restarting: the panes glide to the new
    /// resting widths instead of jumping to them.
    pub fn set_total_width(&mut self, total: u16, now: Instant) {
        self.retarget(total, self.focused, now);
    }

    /// Advance the transition, returning whether a re-render is warranted
    ///
    /// Returns `false` once the transition has landed, which is what stops the
    /// layout from holding the event loop at 60fps forever.
    pub fn tick(&mut self, now: Instant) -> bool {
        if !self.animating {
            return false;
        }
        if now.saturating_duration_since(self.started_at) >= TRANSITION {
            self.animating = false;
            self.from = (self.to.0 as f32, self.to.1 as f32);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The worked examples from the width table
    #[test]
    fn test_pane_widths_match_the_table() {
        // (terminal width, focused pane 0, expected widths)
        let cases = [
            (80_u16, [60_u16, 10, 10]),
            (100, [50, 25, 25]),
            (120, [60, 30, 30]),
            (160, [72, 44, 44]),
            (220, [88, 66, 66]),
        ];
        for (total, expected) in cases {
            let (widths, _) = pane_widths(total, 0);
            assert_eq!(widths, expected, "focused pane 0 at {total} columns");
        }
    }

    #[test]
    fn test_focused_pane_gets_the_wide_slot_wherever_it_sits() {
        for focused in 0..3 {
            let (widths, _) = pane_widths(160, focused);
            assert_eq!(widths[focused], 72, "focused pane {focused}");
            assert_eq!(widths.iter().sum::<u16>(), 160);
        }
        // Pane 1 focused at 220: 88 in the middle, 66 either side
        let (widths, _) = pane_widths(220, 1);
        assert_eq!(widths, [66, 88, 66]);
    }

    #[test]
    fn test_widths_always_sum_to_the_terminal_width() {
        for total in 0..=300u16 {
            for focused in 0..3 {
                let (widths, _) = pane_widths(total, focused);
                assert_eq!(
                    widths.iter().sum::<u16>(),
                    total,
                    "{total} columns, pane {focused}"
                );
            }
        }
    }

    #[test]
    fn test_narrow_terminal_shows_only_the_focused_pane() {
        for focused in 0..3 {
            let (widths, mode) = pane_widths(59, focused);
            assert_eq!(widths[focused], 59);
            assert_eq!(widths.iter().sum::<u16>(), 59);
            assert_eq!(mode, SideMode::Hidden);
        }
    }

    #[test]
    fn test_the_87_88_boundary_is_a_discontinuity() {
        // 87: strips either side, focused absorbs the rest
        let (narrow, narrow_mode) = pane_widths(87, 0);
        assert_eq!(narrow, [67, 10, 10]);
        assert_eq!(narrow_mode, SideMode::Strip);

        // 88: three panes, sides wide enough for the compact format
        let (wide, wide_mode) = pane_widths(88, 0);
        assert_eq!(wide, [44, 22, 22]);
        assert_eq!(wide_mode, SideMode::Compact);
    }

    #[test]
    fn test_side_mode_thresholds() {
        assert_eq!(side_mode(0), SideMode::Hidden);
        assert_eq!(side_mode(SIDE_STRIP_COLS), SideMode::Strip);
        assert_eq!(side_mode(SIDE_COMPACT_MIN - 1), SideMode::Strip);
        assert_eq!(side_mode(SIDE_COMPACT_MIN), SideMode::Compact);
        assert_eq!(side_mode(SIDE_FULL_MIN - 1), SideMode::Compact);
        assert_eq!(side_mode(SIDE_FULL_MIN), SideMode::Full);
    }

    #[test]
    fn test_transition_settles_and_stops_asking_to_render() {
        let start = Instant::now();
        let mut layout = PaneLayout::new(160, 0, start);
        assert!(!layout.is_animating());
        assert!(!layout.tick(start), "a settled layout needs no frames");

        layout.set_focus(1, start);
        assert!(layout.is_animating());
        assert!(layout.tick(start + Duration::from_millis(50)));
        assert!(layout.is_animating());

        // The frame that lands the transition still renders, the next does not
        assert!(layout.tick(start + TRANSITION));
        assert!(!layout.is_animating());
        assert!(!layout.tick(start + TRANSITION + Duration::from_millis(16)));

        assert_eq!(layout.widths_at(start + TRANSITION), pane_widths(160, 1).0);
    }

    #[test]
    fn test_every_frame_of_a_transition_sums_to_the_terminal_width() {
        let start = Instant::now();
        let mut layout = PaneLayout::new(120, 0, start);
        layout.set_focus(2, start);

        for ms in 0..=200 {
            let now = start + Duration::from_millis(ms);
            let widths = layout.widths_at(now);
            assert_eq!(widths.iter().sum::<u16>(), 120, "at {ms}ms");
        }
    }

    #[test]
    fn test_retarget_mid_flight_never_queues_or_overshoots() {
        let start = Instant::now();
        let mut layout = PaneLayout::new(200, 0, start);

        // Interrupt half-way and go back where we came from
        layout.set_focus(1, start);
        let mid = start + Duration::from_millis(70);
        let mid_widths = layout.widths_at(mid);
        layout.set_focus(0, mid);

        // The reversal starts from the widths on screen, not from the target
        assert_eq!(layout.widths_at(mid), mid_widths);

        // And lands exactly on pane 0's resting widths, not past them
        layout.tick(mid + TRANSITION);
        assert_eq!(layout.widths_at(mid + TRANSITION), pane_widths(200, 0).0);
        assert!(!layout.is_animating());
    }

    #[test]
    fn test_resize_retargets_rather_than_restarting() {
        let start = Instant::now();
        let mut layout = PaneLayout::new(200, 0, start);
        layout.set_focus(1, start);
        let mid = start + Duration::from_millis(70);
        let before = layout.widths_at(mid);

        layout.set_total_width(120, mid);
        // Still heading for pane 1, now at the new width, and still animating
        assert!(layout.is_animating());
        assert_eq!(layout.focused(), 1);
        assert_eq!(layout.total_width(), 120);
        // The first frame after the resize is clamped into the new terminal
        assert_eq!(layout.widths_at(mid).iter().sum::<u16>(), 120);
        assert_ne!(before.iter().sum::<u16>(), 120);

        layout.tick(mid + TRANSITION);
        assert_eq!(layout.widths_at(mid + TRANSITION), pane_widths(120, 1).0);
    }

    #[test]
    fn test_ease_out_is_monotonic_and_bounded() {
        assert_eq!(ease_out(0.0), 0.0);
        assert_eq!(ease_out(1.0), 1.0);
        assert_eq!(ease_out(2.0), 1.0);
        assert_eq!(ease_out(-1.0), 0.0);
        let mut previous = 0.0;
        for step in 0..=100 {
            let value = ease_out(step as f32 / 100.0);
            assert!(value >= previous, "eased value must not go backwards");
            previous = value;
        }
    }
}
