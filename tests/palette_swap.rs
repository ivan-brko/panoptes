//! The theme global is swappable, and a swap reaches the next read
//!
//! This lives out here rather than in `src/tui/theme.rs` because it is the one
//! test that writes the process-wide theme: every view test in the lib binary
//! renders through `theme()`, and repainting it mid-run would race them. An
//! integration test file is its own binary, so the mutation is contained.

use panoptes::config::{Palette, ThemeMode};
use panoptes::tui::theme::{set_palette, theme, ColorSupport, Theme};
use ratatui::style::Color;

#[test]
fn set_palette_repaints_the_global_without_losing_the_tier() {
    // The tier is settled once; only the preset moves after that
    panoptes::tui::theme::init(ThemeMode::TrueColor, Palette::Peacock);
    assert_eq!(theme().accent, Color::Cyan);
    assert_eq!(
        theme().bg_surface,
        Theme::new(Palette::Peacock, ColorSupport::TrueColor).bg_surface
    );

    for palette in Palette::ALL {
        set_palette(palette);
        let expected = Theme::new(palette, ColorSupport::TrueColor);
        assert_eq!(theme().accent, expected.accent, "{palette:?}");
        // Still truecolor: a preset change must not silently drop the tier
        // back to the baseline, which is what would happen if `set_palette`
        // re-detected instead of reusing what `init` settled
        assert!(
            matches!(theme().bg_surface, Color::Rgb(..)),
            "{palette:?} lost the truecolor tier: {:?}",
            theme().bg_surface
        );
        assert_eq!(theme().bg_surface, expected.bg_surface, "{palette:?}");
    }
}

#[test]
fn every_preset_is_reachable_by_index_from_the_picker() {
    for (i, palette) in Palette::ALL.iter().enumerate() {
        assert_eq!(Palette::at(i), Some(*palette));
        assert_eq!(palette.index(), i);
    }
    assert_eq!(Palette::at(Palette::ALL.len()), None);
}
