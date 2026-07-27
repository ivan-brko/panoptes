//! Horizontal auto-scroll for text that outgrew the row it sits in
//!
//! One cycle: hold at the start long enough to read the first words, pan one
//! column at a time to the far end, hold there, then jump back and repeat. The
//! hold at either end is what makes it readable rather than a ticker.
//!
//! Like the accordion transition ([`super::panes::PaneLayout::tick`]), the tick
//! returns whether anything actually moved, so a settled - or short enough -
//! line never holds the event loop at 60fps.

use std::time::{Duration, Instant};

/// How long the text rests before it starts moving, and again at the far end
const HOLD: Duration = Duration::from_millis(1200);

/// Time spent on each column of travel
const STEP: Duration = Duration::from_millis(90);

/// Where a scrolling line is up to, and when its cycle started
#[derive(Debug)]
pub struct Marquee {
    started_at: Instant,
    offset: usize,
}

impl Default for Marquee {
    fn default() -> Self {
        Self {
            started_at: Instant::now(),
            offset: 0,
        }
    }
}

impl Marquee {
    /// Columns the text is currently scrolled by
    pub fn offset(&self) -> usize {
        self.offset
    }

    /// Start the cycle over, back at the first column
    ///
    /// Returns whether that moved the text, so the caller can fold it into the
    /// same "is a frame owed" answer as [`Self::tick`].
    pub fn reset(&mut self, now: Instant) -> bool {
        self.started_at = now;
        let moved = self.offset != 0;
        self.offset = 0;
        moved
    }

    /// Advance to where `now` puts a line overflowing its row by `overflow`
    /// columns, returning whether the offset moved
    ///
    /// `overflow` is passed in per tick rather than stored because it depends
    /// on the pane's *current* width, which an accordion transition is still
    /// changing underneath.
    pub fn tick(&mut self, overflow: usize, now: Instant) -> bool {
        let offset = offset_at(overflow, now.saturating_duration_since(self.started_at));
        let moved = offset != self.offset;
        self.offset = offset;
        moved
    }
}

/// The offset `elapsed` into a cycle: hold, pan, hold, loop
fn offset_at(overflow: usize, elapsed: Duration) -> usize {
    if overflow == 0 {
        return 0;
    }

    let hold = HOLD.as_millis();
    let travel = STEP.as_millis() * overflow as u128;
    let cycle = hold + travel + hold;
    let at = elapsed.as_millis() % cycle;

    if at < hold {
        0
    } else if at < hold + travel {
        ((at - hold) / STEP.as_millis()) as usize
    } else {
        overflow
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    /// A line that fits never moves - which is also what keeps the event loop
    /// from spinning for every row that happens to be selected
    #[test]
    fn test_text_that_fits_never_scrolls() {
        let mut marquee = Marquee::default();
        let start = Instant::now();
        for step in [0, 500, 3_000, 30_000] {
            assert!(!marquee.tick(0, start + ms(step)), "moved at {step}ms");
            assert_eq!(marquee.offset(), 0);
        }
    }

    #[test]
    fn test_a_cycle_holds_pans_holds_then_loops() {
        let overflow = 10;
        let travel = STEP.as_millis() as u64 * overflow as u64;
        let hold = HOLD.as_millis() as u64;

        // The opening hold: still on the first column
        assert_eq!(offset_at(overflow, ms(0)), 0);
        assert_eq!(offset_at(overflow, ms(hold - 1)), 0);

        // Panning, one column per step
        assert_eq!(offset_at(overflow, ms(hold)), 0);
        assert_eq!(offset_at(overflow, ms(hold + STEP.as_millis() as u64)), 1);
        assert_eq!(offset_at(overflow, ms(hold + travel / 2)), overflow / 2);

        // The closing hold sits on the last column, never past it
        assert_eq!(offset_at(overflow, ms(hold + travel)), overflow);
        assert_eq!(offset_at(overflow, ms(hold + travel + hold - 1)), overflow);

        // ...and then it starts over
        assert_eq!(offset_at(overflow, ms(hold + travel + hold)), 0);
    }

    /// Only a column change is worth a frame; the two holds are not
    #[test]
    fn test_tick_owes_a_frame_only_while_the_text_moves() {
        let overflow = 5;
        let hold = HOLD.as_millis() as u64;
        let step = STEP.as_millis() as u64;
        let start = Instant::now();

        let mut marquee = Marquee::default();
        marquee.reset(start);

        assert!(!marquee.tick(overflow, start + ms(16)), "the opening hold");
        assert!(!marquee.tick(overflow, start + ms(hold)), "still holding");
        assert!(marquee.tick(overflow, start + ms(hold + step)), "panning");
        assert!(
            !marquee.tick(overflow, start + ms(hold + step + 16)),
            "same column"
        );
        assert!(
            marquee.tick(overflow, start + ms(hold + 2 * step)),
            "next column"
        );
    }

    #[test]
    fn test_reset_returns_to_the_first_column() {
        let start = Instant::now();
        let mut marquee = Marquee::default();
        marquee.reset(start);

        let panned = start + ms(HOLD.as_millis() as u64 + STEP.as_millis() as u64 * 3);
        marquee.tick(6, panned);
        assert_eq!(marquee.offset(), 3);

        assert!(marquee.reset(panned), "a reset from mid-pan owes a frame");
        assert_eq!(marquee.offset(), 0);
        assert!(!marquee.reset(panned), "a reset from the start does not");
    }
}
