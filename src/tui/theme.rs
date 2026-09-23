//! Theme module for centralized color and style definitions
//!
//! This module provides semantic color tokens and styles used throughout the
//! UI. Every colour a view draws comes from here - raw `Color::` literals in
//! view code are a bug, because they are invisible to a theme change.
//!
//! A theme is one **palette** rendered at one capability **tier**, and the two
//! axes are independent:
//!
//! - The tier is how many colours the terminal can show. Truecolor is the
//!   design layer, 256-colour the fallback, and 16 ANSI colours the SSH
//!   baseline. Within a palette the tiers agree on every chromatic token -
//!   they differ only in how fine a grey ramp and how tinted a surface they
//!   can express - so the worst case is exactly the classic appearance. The
//!   tier is detected from `COLORTERM`/`TERM` at startup and can be forced
//!   with the `theme` key in `config.toml`.
//! - The palette is which colours those are: [`Palette::Peacock`] (the
//!   default, and byte-for-byte the look Panoptes has always had), `Io`,
//!   `Hera`, `Argus`. It restyles the *chrome* only - see [`Chrome`] - and is
//!   chosen live from pane 3, which is why the global below is swappable
//!   rather than pinned for the process.

use std::sync::{OnceLock, PoisonError, RwLock};

use ratatui::style::{Color, Modifier, Style};

use crate::config::{Palette, ThemeMode};

/// How rich a palette the terminal can display
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorSupport {
    /// 24-bit RGB (`COLORTERM=truecolor`)
    TrueColor,
    /// The 256-colour indexed palette (`TERM=*-256color`)
    Ansi256,
    /// The 16 named ANSI colours - always safe
    Ansi16,
}

/// Detect the terminal's colour support from the environment
///
/// `COLORTERM` is the primary signal (`truecolor` / `24bit`), with `TERM` as
/// the backstop. Anything unrecognised lands on the 16-colour baseline: a
/// palette that under-promises still renders correctly everywhere.
pub fn detect_color_support() -> ColorSupport {
    color_support_from(
        std::env::var("COLORTERM").ok().as_deref(),
        std::env::var("TERM").ok().as_deref(),
    )
}

/// The pure half of [`detect_color_support`], so tests need no env vars
fn color_support_from(colorterm: Option<&str>, term: Option<&str>) -> ColorSupport {
    if let Some(ct) = colorterm {
        let ct = ct.to_ascii_lowercase();
        if ct.contains("truecolor") || ct.contains("24bit") {
            return ColorSupport::TrueColor;
        }
    }
    if let Some(t) = term {
        let t = t.to_ascii_lowercase();
        if t.contains("direct") || t.contains("truecolor") {
            return ColorSupport::TrueColor;
        }
        if t.contains("256color") {
            return ColorSupport::Ansi256;
        }
    }
    ColorSupport::Ansi16
}

/// Application theme with all color definitions
///
/// `Copy` on purpose: the global is swappable now, so [`theme`] hands back a
/// snapshot rather than a borrow into a lock a render would have to hold.
/// Every field is a `Color`, so the copy is a couple of dozen bytes.
#[derive(Debug, Clone, Copy)]
pub struct Theme {
    // === Session States ===
    /// Session is starting up
    pub state_starting: Color,
    /// Claude is thinking/processing
    pub state_thinking: Color,
    /// Claude is executing a tool
    pub state_executing: Color,
    /// Claude is waiting for user input
    pub state_waiting: Color,
    /// Session is blocked on a permission dialog
    pub state_awaiting_approval: Color,
    /// Session was suspended by Panoptes to reclaim memory
    pub state_suspended: Color,
    /// Session has exited
    pub state_exited: Color,
    /// Session is recoverable from a previous Panoptes run
    pub state_resumable: Color,

    // === UI Elements ===
    /// Primary accent color (headers, titles)
    pub accent: Color,
    /// Text color for normal content
    pub text: Color,
    /// Text one step down: secondary content, help lines, tags
    pub text_dim: Color,
    /// Text two steps down: present but ignorable - inactive pane chrome
    pub text_faint: Color,
    /// Text drawn over a filled accent background (dialog buttons)
    pub text_inverted: Color,
    /// Color for selected/focused items
    pub selected: Color,
    /// Color for active items (running processes)
    pub active: Color,

    // === Backgrounds ===
    /// The terminal's own background, left untouched
    pub bg_base: Color,
    /// A surface one step above the base - the selected row's background
    pub bg_surface: Color,
    /// Background of mouse-selected terminal text
    pub text_selection_bg: Color,
    /// Foreground of mouse-selected terminal text
    ///
    /// Selection replaces the text's own colours rather than tinting them:
    /// agent output is already every colour there is, and a highlight that
    /// only tinted would be invisible over half of it.
    pub text_selection_fg: Color,

    // === Input Modes ===
    /// Color for input mode prompts
    pub input_prompt: Color,

    // === Notifications ===
    /// Color for items needing attention (waiting state with flag)
    pub attention_waiting: Color,
    /// Color for idle sessions needing attention
    pub attention_idle: Color,
    /// Attention badge color
    pub attention_badge: Color,

    // === Outcomes ===
    /// Something completed or is safe to take (green in every tier)
    pub success: Color,
    /// Something needs care before proceeding (yellow in every tier)
    pub warning: Color,
    /// Something is broken or destructive (red in every tier)
    pub danger: Color,

    // === Banners ===
    /// Error banner background
    pub error_bg: Color,
    /// Error banner foreground
    pub error_fg: Color,
    /// Warning banner background
    pub warning_bg: Color,
    /// Warning banner foreground
    pub warning_fg: Color,

    // === Borders ===
    /// The focused pane's border - the brightest chrome on screen
    pub border_focus: Color,
    /// Structural border: present, recessive (dividers, overlays)
    pub border: Color,
    /// An unfocused pane's border: inactive, ignore this
    pub border_dim: Color,
    /// Warning border color
    pub border_warning: Color,

    // === Dialog Keys ===
    /// Color for the confirming key in prompts (e.g. "y" / "Enter")
    pub confirm_key: Color,
    /// Color for the cancelling key in prompts (e.g. "n" / "Esc")
    pub cancel_key: Color,
    /// Color for default-item markers (e.g. "★" / "(default)")
    pub default_marker: Color,
}

/// The tokens a preset owns
///
/// Everything not listed here is either structure (the grey ramp, the text
/// colours) or meaning (states, outcomes, banners), and is the same in every
/// preset - a session list has to read identically whichever one is on.
///
/// The first four are chromatic and stay the *same named colour across all
/// three tiers*, for the reason the tiers were built that way in the first
/// place: the user's own terminal palette decides what "cyan" looks like. The
/// last three are surfaces, and surfaces are exactly what the richer tiers can
/// express and the baseline cannot - so a preset's tint lands there and only
/// there.
#[derive(Debug, Clone, Copy)]
struct Chrome {
    accent: Color,
    border_focus: Color,
    input_prompt: Color,
    default_marker: Color,
    bg_surface: Color,
    text_selection_bg: Color,
    text_selection_fg: Color,
}

/// The four named colours a preset reassigns, identical in every tier
fn chromatic_chrome(palette: Palette) -> (Color, Color, Color, Color) {
    match palette {
        // The historical assignment, unchanged
        Palette::Peacock => (Color::Cyan, Color::Cyan, Color::Magenta, Color::Yellow),
        // Yellow is the accent here, so the default marker moves off it and
        // takes the accent Peacock vacated
        Palette::Io => (Color::Yellow, Color::Yellow, Color::Magenta, Color::Cyan),
        // Magenta is the accent here, so the input prompt moves to cyan
        Palette::Hera => (Color::Magenta, Color::Magenta, Color::Cyan, Color::Yellow),
        Palette::Argus => (Color::Green, Color::Green, Color::Magenta, Color::Yellow),
    }
}

/// The three surfaces a preset tints, as far as the tier allows
fn surface_chrome(palette: Palette, support: ColorSupport) -> (Color, Color, Color) {
    match support {
        // The baseline leaves the row background to the terminal; the one
        // surface it must own is the text selection, which has to be visible
        // over arbitrary agent output. Each preset picks a legible pair.
        ColorSupport::Ansi16 => {
            let (bg, fg) = match palette {
                Palette::Peacock => (Color::Blue, Color::White),
                Palette::Io => (Color::Yellow, Color::Black),
                Palette::Hera => (Color::Magenta, Color::White),
                Palette::Argus => (Color::Green, Color::Black),
            };
            (Color::Reset, bg, fg)
        }
        // The 256-colour cube has no tone dark enough to tint a whole selected
        // row without shouting - its darkest chromatic cells are already at
        // 0x5f - so the row surface stays the neutral grey here and the preset
        // shows in the text selection, which wants saturation anyway.
        ColorSupport::Ansi256 => {
            let selection = match palette {
                Palette::Peacock => 24, // #005f87
                Palette::Io => 94,      // #875f00
                Palette::Hera => 54,    // #5f0087
                Palette::Argus => 22,   // #005f00
            };
            (
                Color::Indexed(236),
                Color::Indexed(selection),
                Color::Indexed(255),
            )
        }
        // Truecolor is where a preset fully lands: both surfaces are tuned to
        // the same luminance as Peacock's originals, tinted toward the hue.
        ColorSupport::TrueColor => {
            let (surface, selection) = match palette {
                Palette::Peacock => ((0x2c, 0x31, 0x36), (0x2d, 0x4f, 0x6d)),
                Palette::Io => ((0x36, 0x2f, 0x26), (0x6d, 0x52, 0x27)),
                Palette::Hera => ((0x33, 0x2c, 0x3a), (0x4a, 0x30, 0x70)),
                Palette::Argus => ((0x2a, 0x35, 0x2c), (0x2f, 0x5a, 0x3a)),
            };
            (
                Color::Rgb(surface.0, surface.1, surface.2),
                Color::Rgb(selection.0, selection.1, selection.2),
                Color::Rgb(0xe6, 0xe6, 0xe6),
            )
        }
    }
}

impl Chrome {
    /// The chrome of one preset at one tier
    fn of(palette: Palette, support: ColorSupport) -> Self {
        let (accent, border_focus, input_prompt, default_marker) = chromatic_chrome(palette);
        let (bg_surface, text_selection_bg, text_selection_fg) = surface_chrome(palette, support);
        Self {
            accent,
            border_focus,
            input_prompt,
            default_marker,
            bg_surface,
            text_selection_bg,
            text_selection_fg,
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::ansi16()
    }
}

impl Theme {
    /// The theme for one palette at one capability tier
    ///
    /// The tier decides the structure, the palette repaints the chrome over
    /// it. [`Palette::Peacock`]'s chrome is exactly what the tier bases
    /// already carry, so the default preset is byte-for-byte the old theme.
    pub fn new(palette: Palette, support: ColorSupport) -> Self {
        let base = match support {
            ColorSupport::TrueColor => Self::truecolor(),
            ColorSupport::Ansi256 => Self::ansi256(),
            ColorSupport::Ansi16 => Self::ansi16(),
        };
        base.with_chrome(Chrome::of(palette, support))
    }

    /// Repaint the preset-owned tokens, leaving structure and meaning alone
    fn with_chrome(self, chrome: Chrome) -> Self {
        Self {
            accent: chrome.accent,
            border_focus: chrome.border_focus,
            input_prompt: chrome.input_prompt,
            default_marker: chrome.default_marker,
            bg_surface: chrome.bg_surface,
            text_selection_bg: chrome.text_selection_bg,
            text_selection_fg: chrome.text_selection_fg,
            ..self
        }
    }

    /// The 16-colour baseline tier - the classic dark theme
    ///
    /// Every token the other tiers refine holds its historical value here,
    /// which is what makes this the worst case rather than a different look.
    pub fn ansi16() -> Self {
        Self {
            // Session states
            state_starting: Color::Blue,
            state_thinking: Color::Yellow,
            state_executing: Color::Cyan,
            state_waiting: Color::Green,
            // Distinct from thinking's plain Yellow: this one is on you
            state_awaiting_approval: Color::LightYellow,
            // Shares the tier's only grey with `text_dim`, which the
            // unfocused-pane dimmer cannot tell apart - so in this tier alone
            // a suspended row recesses with the ramp. The richer tiers give
            // suspended its own grey precisely to avoid that.
            state_suspended: Color::DarkGray,
            state_exited: Color::Red,
            // Magenta is unused by the live states, so a recoverable session
            // reads as its own category rather than a variant of "dead"
            state_resumable: Color::Magenta,

            // UI elements
            accent: Color::Cyan,
            text: Color::White,
            text_dim: Color::DarkGray,
            // One grey is all this tier has, so faint collapses into dim
            text_faint: Color::DarkGray,
            text_inverted: Color::Black,
            selected: Color::White,
            active: Color::Green,

            // Backgrounds: the user's terminal shows through in this tier
            bg_base: Color::Reset,
            bg_surface: Color::Reset,
            // The one place this tier cannot leave the background to the
            // terminal: a selection has to be visible over arbitrary agent
            // output, so it takes a colour of its own
            text_selection_bg: Color::Blue,
            text_selection_fg: Color::White,

            // Input modes - using Magenta to avoid conflict with Yellow (thinking/idle)
            input_prompt: Color::Magenta,

            // Notifications
            attention_waiting: Color::Green,
            attention_idle: Color::Yellow,
            attention_badge: Color::Yellow,

            // Outcomes
            success: Color::Green,
            warning: Color::Yellow,
            danger: Color::Red,

            // Banners
            error_bg: Color::Red,
            error_fg: Color::White,
            warning_bg: Color::Yellow,
            warning_fg: Color::Black,

            // Borders
            border_focus: Color::Cyan,
            border: Color::White,
            border_dim: Color::DarkGray,
            border_warning: Color::Yellow,

            // Dialog keys
            confirm_key: Color::Green,
            cancel_key: Color::Red,
            default_marker: Color::Yellow,
        }
    }

    /// The 256-colour tier: the baseline plus a finer structural grey ramp
    ///
    /// Chromatic tokens stay the named ANSI colours so the user's own terminal
    /// palette keeps deciding what "green" means; only the greys that carry
    /// hierarchy - faint text, dim borders, the surface tint - use the indexed
    /// ramp the baseline cannot express.
    pub fn ansi256() -> Self {
        Self {
            text_faint: Color::Indexed(238),
            border_dim: Color::Indexed(238),
            bg_surface: Color::Indexed(236),
            // A muted slate rather than the baseline's full blue: it has to
            // sit under a whole screen of text without shouting
            text_selection_bg: Color::Indexed(24),
            text_selection_fg: Color::Indexed(255),
            // Off the text ramp: suspended is a state, not structure, and
            // must not be caught by the unfocused-pane dimmer's ramp check
            state_suspended: Color::Indexed(245),
            ..Self::ansi16()
        }
    }

    /// The truecolor tier: the design layer
    ///
    /// Same restraint as [`Theme::ansi256`] - named ANSI for everything
    /// chromatic, RGB only for the structural greys, tuned against a dark
    /// background.
    pub fn truecolor() -> Self {
        Self {
            text_faint: Color::Rgb(0x4e, 0x4e, 0x4e),
            border_dim: Color::Rgb(0x3a, 0x3f, 0x44),
            bg_surface: Color::Rgb(0x2c, 0x31, 0x36),
            // See `ansi256`; the same slate, tuned
            text_selection_bg: Color::Rgb(0x2d, 0x4f, 0x6d),
            text_selection_fg: Color::Rgb(0xe6, 0xe6, 0xe6),
            // Off the text ramp; see `ansi256`
            state_suspended: Color::Rgb(0x87, 0x87, 0x87),
            ..Self::ansi16()
        }
    }

    /// Get the color for a session state
    pub fn session_state_color(&self, state: &crate::session::SessionState) -> Color {
        use crate::session::SessionState;
        match state {
            SessionState::Starting => self.state_starting,
            SessionState::Thinking => self.state_thinking,
            SessionState::Executing => self.state_executing,
            SessionState::AwaitingApproval => self.state_awaiting_approval,
            SessionState::Waiting => self.state_waiting,
            SessionState::Suspended => self.state_suspended,
            SessionState::Exited => self.state_exited,
            SessionState::Resumable => self.state_resumable,
        }
    }

    /// Badge colour for an attention reason
    ///
    /// Green means "done, your turn"; yellow means "blocked on you"; red means
    /// something went wrong.
    pub fn attention_color(&self, reason: &crate::session::AttentionReason) -> Color {
        use crate::session::AttentionReason;
        match reason {
            AttentionReason::TurnComplete => self.success,
            AttentionReason::Approval { .. } | AttentionReason::Stalled { .. } => self.warning,
            AttentionReason::Crashed { .. } | AttentionReason::TurnFailed { .. } => self.danger,
        }
    }

    // === Style Builders ===

    /// Style for headers/titles
    pub fn header_style(&self) -> Style {
        Style::default()
            .fg(self.accent)
            .add_modifier(Modifier::BOLD)
    }

    /// Style for muted text
    pub fn muted_style(&self) -> Style {
        Style::default().fg(self.text_dim)
    }

    /// Style for selected items
    pub fn selected_style(&self) -> Style {
        Style::default()
            .fg(self.selected)
            .bg(self.bg_surface)
            .add_modifier(Modifier::BOLD)
    }

    /// Style for input prompts
    pub fn input_style(&self) -> Style {
        Style::default().fg(self.input_prompt)
    }

    /// Style for error banners
    pub fn error_banner_style(&self) -> Style {
        Style::default().fg(self.error_fg).bg(self.error_bg)
    }

    /// Style for warning banners
    pub fn warning_banner_style(&self) -> Style {
        Style::default().fg(self.warning_fg).bg(self.warning_bg)
    }

    /// Style for mouse-selected terminal text
    pub fn text_selection_style(&self) -> Style {
        Style::default()
            .fg(self.text_selection_fg)
            .bg(self.text_selection_bg)
    }

    /// Style for attention badge based on state
    pub fn attention_badge_style(&self, is_waiting: bool) -> Style {
        let color = if is_waiting {
            self.attention_waiting
        } else {
            self.attention_idle
        };
        Style::default().fg(color)
    }
}

/// What the UI is currently wearing
///
/// The tier is settled once at startup - it is a property of the terminal, not
/// a preference - so only the palette moves after that, and it is kept here so
/// [`set_palette`] can rebuild the theme without re-detecting anything.
#[derive(Debug, Clone, Copy)]
struct Active {
    palette: Palette,
    support: ColorSupport,
    theme: Theme,
}

/// Global theme instance
///
/// Swappable, not pinned: the preset picker previews live, so a palette change
/// has to reach the next render. Renders run on the main loop and the only
/// writer is that same loop, so the lock is never contended.
static ACTIVE: OnceLock<RwLock<Active>> = OnceLock::new();

fn active() -> &'static RwLock<Active> {
    ACTIVE.get_or_init(|| {
        RwLock::new(Active {
            palette: Palette::Peacock,
            support: ColorSupport::Ansi16,
            theme: Theme::default(),
        })
    })
}

/// Install the theme for this run, from the configured tier and palette
///
/// Called at startup, before the first render. `Auto` detects the tier from
/// the environment; the other modes force one, for when detection is wrong.
/// The palette can be changed later with [`set_palette`]; the tier cannot.
pub fn init(mode: ThemeMode, palette: Palette) {
    let support = match mode {
        ThemeMode::Auto => detect_color_support(),
        ThemeMode::TrueColor => ColorSupport::TrueColor,
        ThemeMode::Ansi256 => ColorSupport::Ansi256,
        ThemeMode::Ansi16 => ColorSupport::Ansi16,
    };
    let mut guard = active().write().unwrap_or_else(PoisonError::into_inner);
    *guard = Active {
        palette,
        support,
        theme: Theme::new(palette, support),
    };
}

/// Repaint the UI in `palette`, keeping the tier [`init`] settled on
///
/// Takes effect on the next render, which is what makes the picker's highlight
/// a live preview of the whole dashboard rather than a swatch. Called once per
/// frame from the render path with whatever the state says should be worn, so
/// it has to be cheap and idempotent when nothing moved - hence the read
/// first, and no write at all in the overwhelmingly common case.
pub fn set_palette(palette: Palette) {
    if active()
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .palette
        == palette
    {
        return;
    }
    let mut guard = active().write().unwrap_or_else(PoisonError::into_inner);
    guard.palette = palette;
    guard.theme = Theme::new(palette, guard.support);
}

/// Get the current theme
///
/// A snapshot, not a borrow: the global moves under the picker, and a render
/// that held a reference into it would be holding the lock for its whole
/// duration. Falls back to the 16-colour baseline when [`init`] has not run -
/// which is the case in tests, keeping their colours independent of the
/// environment.
pub fn theme() -> Theme {
    active()
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .theme
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_theme_default() {
        let theme = Theme::default();
        assert_eq!(theme.accent, Color::Cyan);
        assert_eq!(theme.state_waiting, Color::Green);
    }

    #[test]
    fn test_theme_ansi16() {
        let theme = Theme::ansi16();
        assert_eq!(theme.input_prompt, Color::Magenta);
    }

    const TIERS: [ColorSupport; 3] = [
        ColorSupport::Ansi16,
        ColorSupport::Ansi256,
        ColorSupport::TrueColor,
    ];

    /// The live states, which every preset must render identically
    const STATES: [crate::session::SessionState; 7] = [
        crate::session::SessionState::Starting,
        crate::session::SessionState::Thinking,
        crate::session::SessionState::Executing,
        crate::session::SessionState::AwaitingApproval,
        crate::session::SessionState::Waiting,
        crate::session::SessionState::Exited,
        crate::session::SessionState::Resumable,
    ];

    /// The tiers refine a preset; they must not redefine it. Every token that
    /// predates the tiers holds the same value in all three *within a preset*,
    /// which is what makes tier detection safe to ship with no visible change
    /// - and what keeps a preset recognisable over SSH.
    #[test]
    fn test_tiers_agree_on_every_pre_tier_token() {
        for palette in Palette::ALL {
            let base = Theme::new(palette, ColorSupport::Ansi16);
            for support in [ColorSupport::Ansi256, ColorSupport::TrueColor] {
                let tier = Theme::new(palette, support);
                assert_eq!(tier.accent, base.accent, "{palette:?}");
                assert_eq!(tier.text, base.text, "{palette:?}");
                assert_eq!(tier.text_dim, base.text_dim, "{palette:?}");
                assert_eq!(tier.selected, base.selected, "{palette:?}");
                assert_eq!(tier.active, base.active, "{palette:?}");
                assert_eq!(tier.input_prompt, base.input_prompt, "{palette:?}");
                assert_eq!(
                    tier.attention_waiting, base.attention_waiting,
                    "{palette:?}"
                );
                assert_eq!(tier.attention_idle, base.attention_idle, "{palette:?}");
                assert_eq!(tier.attention_badge, base.attention_badge, "{palette:?}");
                assert_eq!(tier.error_bg, base.error_bg, "{palette:?}");
                assert_eq!(tier.error_fg, base.error_fg, "{palette:?}");
                assert_eq!(tier.warning_bg, base.warning_bg, "{palette:?}");
                assert_eq!(tier.warning_fg, base.warning_fg, "{palette:?}");
                assert_eq!(tier.border, base.border, "{palette:?}");
                assert_eq!(tier.border_focus, base.border_focus, "{palette:?}");
                assert_eq!(tier.border_warning, base.border_warning, "{palette:?}");
                assert_eq!(tier.confirm_key, base.confirm_key, "{palette:?}");
                assert_eq!(tier.cancel_key, base.cancel_key, "{palette:?}");
                assert_eq!(tier.default_marker, base.default_marker, "{palette:?}");
                assert_eq!(tier.bg_base, base.bg_base, "{palette:?}");
                // Suspended is deliberately absent: it is a structural grey,
                // and the greys are exactly what the tiers refine. On the
                // baseline it has to share the one grey with `text_dim`; the
                // richer tiers give it its own so the unfocused-pane dimmer
                // never catches it.
                for state in STATES {
                    assert_eq!(
                        tier.session_state_color(&state),
                        base.session_state_color(&state),
                        "{palette:?} {state:?}"
                    );
                }
            }
        }
    }

    /// The upgrade guarantee: turning presets on changes nothing until you
    /// pick one. Peacock at every tier is the theme that tier already had.
    #[test]
    fn test_peacock_is_byte_for_byte_the_old_palette() {
        for (support, old) in [
            (ColorSupport::Ansi16, Theme::ansi16()),
            (ColorSupport::Ansi256, Theme::ansi256()),
            (ColorSupport::TrueColor, Theme::truecolor()),
        ] {
            let new = Theme::new(Palette::Peacock, support);
            assert_eq!(format!("{new:?}"), format!("{old:?}"), "{support:?}");
        }
        // And it is what an unconfigured install gets
        assert_eq!(Palette::default(), Palette::Peacock);
    }

    /// Presets restyle the chrome, never the meaning. A session list, an error
    /// banner and a confirmation dialog read the same in all four - only the
    /// accent, the focused border, the surfaces, the prompt and the default
    /// marker move.
    #[test]
    fn test_presets_keep_every_state_and_outcome_token() {
        for support in TIERS {
            let peacock = Theme::new(Palette::Peacock, support);
            for palette in Palette::ALL {
                let t = Theme::new(palette, support);
                let at = format!("{palette:?}/{support:?}");
                for state in STATES {
                    assert_eq!(
                        t.session_state_color(&state),
                        peacock.session_state_color(&state),
                        "{at} {state:?}"
                    );
                }
                assert_eq!(t.state_suspended, peacock.state_suspended, "{at}");
                assert_eq!(t.success, peacock.success, "{at}");
                assert_eq!(t.warning, peacock.warning, "{at}");
                assert_eq!(t.danger, peacock.danger, "{at}");
                assert_eq!(t.attention_waiting, peacock.attention_waiting, "{at}");
                assert_eq!(t.attention_idle, peacock.attention_idle, "{at}");
                assert_eq!(t.attention_badge, peacock.attention_badge, "{at}");
                assert_eq!(t.error_bg, peacock.error_bg, "{at}");
                assert_eq!(t.error_fg, peacock.error_fg, "{at}");
                assert_eq!(t.warning_bg, peacock.warning_bg, "{at}");
                assert_eq!(t.warning_fg, peacock.warning_fg, "{at}");
                assert_eq!(t.border_warning, peacock.border_warning, "{at}");
                assert_eq!(t.confirm_key, peacock.confirm_key, "{at}");
                assert_eq!(t.cancel_key, peacock.cancel_key, "{at}");
                // Structure too: the text ramp and the greys are the tier's,
                // not the preset's
                assert_eq!(t.text, peacock.text, "{at}");
                assert_eq!(t.text_dim, peacock.text_dim, "{at}");
                assert_eq!(t.text_faint, peacock.text_faint, "{at}");
                assert_eq!(t.border, peacock.border, "{at}");
                assert_eq!(t.border_dim, peacock.border_dim, "{at}");
                assert_eq!(t.bg_base, peacock.bg_base, "{at}");
            }
        }
    }

    /// Every preset is actually distinguishable from every other, in every
    /// tier - including the baseline, where it can only reassign named colours
    #[test]
    fn test_presets_differ_from_each_other_in_every_tier() {
        for support in TIERS {
            for (i, a) in Palette::ALL.iter().enumerate() {
                for b in &Palette::ALL[i + 1..] {
                    let (ta, tb) = (Theme::new(*a, support), Theme::new(*b, support));
                    assert_ne!(ta.accent, tb.accent, "{a:?} vs {b:?} at {support:?}");
                    assert_ne!(
                        ta.text_selection_bg, tb.text_selection_bg,
                        "{a:?} vs {b:?} at {support:?}"
                    );
                }
            }
        }
    }

    /// A preset's accent must not collide with its own input prompt or its
    /// own default marker: all three can share one row, and the whole point of
    /// the marker is that it is not the thing it marks.
    #[test]
    fn test_no_preset_collides_with_its_own_prompt_or_marker() {
        for palette in Palette::ALL {
            let t = Theme::new(palette, ColorSupport::Ansi16);
            assert_ne!(t.accent, t.input_prompt, "{palette:?}");
            assert_ne!(t.accent, t.default_marker, "{palette:?}");
        }
    }

    /// Selected text has to stay legible: the baseline has no dark tints, so a
    /// preset that takes a bright named colour must invert its foreground
    #[test]
    fn test_text_selection_stays_legible_on_the_baseline() {
        for palette in Palette::ALL {
            let t = Theme::new(palette, ColorSupport::Ansi16);
            assert_ne!(t.text_selection_bg, t.text_selection_fg, "{palette:?}");
            let light_bg = matches!(t.text_selection_bg, Color::Yellow | Color::Green);
            assert_eq!(
                t.text_selection_fg,
                if light_bg { Color::Black } else { Color::White },
                "{palette:?}"
            );
        }
    }

    // The swappable global itself is exercised in `tests/palette_swap.rs`,
    // which gets its own process: every view test in this binary renders
    // through `theme()`, so a unit test that repainted it would race them.

    /// The richer tiers only touch the structural greys
    #[test]
    fn test_richer_tiers_refine_the_grey_ramp() {
        assert_eq!(Theme::ansi256().text_faint, Color::Indexed(238));
        assert_eq!(Theme::ansi256().bg_surface, Color::Indexed(236));
        assert!(matches!(Theme::truecolor().text_faint, Color::Rgb(..)));
        assert!(matches!(Theme::truecolor().border_dim, Color::Rgb(..)));
        // The baseline cannot express the ramp: faint collapses into dim,
        // and the surface stays the terminal's own background
        assert_eq!(Theme::ansi16().text_faint, Theme::ansi16().text_dim);
        assert_eq!(Theme::ansi16().bg_surface, Color::Reset);

        // Suspended sits off the text ramp wherever the palette allows, so
        // the unfocused-pane dimmer - which recesses the ramp by value -
        // never catches the one state colour that is a grey
        for tier in [Theme::ansi256(), Theme::truecolor()] {
            assert_ne!(tier.state_suspended, tier.text);
            assert_ne!(tier.state_suspended, tier.text_dim);
            assert_ne!(tier.state_suspended, tier.text_faint);
        }
    }

    #[test]
    fn test_color_support_detection() {
        use ColorSupport::*;
        assert_eq!(color_support_from(Some("truecolor"), None), TrueColor);
        assert_eq!(color_support_from(Some("24bit"), Some("xterm")), TrueColor);
        // TERM is the backstop when COLORTERM says nothing useful
        assert_eq!(color_support_from(None, Some("xterm-256color")), Ansi256);
        assert_eq!(
            color_support_from(Some(""), Some("screen-256color")),
            Ansi256
        );
        assert_eq!(color_support_from(None, Some("xterm-direct")), TrueColor);
        // Anything unrecognised lands on the baseline
        assert_eq!(color_support_from(None, Some("vt100")), Ansi16);
        assert_eq!(color_support_from(None, None), Ansi16);
    }

    #[test]
    fn test_attention_color_maps_reasons_to_outcomes() {
        use crate::session::AttentionReason;
        let t = Theme::ansi16();
        assert_eq!(t.attention_color(&AttentionReason::TurnComplete), t.success);
        assert_eq!(
            t.attention_color(&AttentionReason::Approval { tool: None }),
            t.warning
        );
        assert_eq!(
            t.attention_color(&AttentionReason::Stalled {
                tool: "Bash".to_string(),
                secs: 600
            }),
            t.warning
        );
        assert_eq!(
            t.attention_color(&AttentionReason::Crashed {
                reason: "signal 9".to_string()
            }),
            t.danger
        );
        assert_eq!(
            t.attention_color(&AttentionReason::TurnFailed { reason: None }),
            t.danger
        );
    }

    #[test]
    fn test_session_state_color() {
        use crate::session::SessionState;
        let theme = Theme::ansi16();

        assert_eq!(
            theme.session_state_color(&SessionState::Starting),
            Color::Blue
        );
        assert_eq!(
            theme.session_state_color(&SessionState::Thinking),
            Color::Yellow
        );
        assert_eq!(
            theme.session_state_color(&SessionState::Executing),
            Color::Cyan
        );
        assert_eq!(
            theme.session_state_color(&SessionState::Waiting),
            Color::Green
        );
        assert_eq!(
            theme.session_state_color(&SessionState::AwaitingApproval),
            Color::LightYellow
        );
        assert_eq!(theme.session_state_color(&SessionState::Exited), Color::Red);
    }

    #[test]
    fn test_global_theme() {
        let t = theme();
        assert_eq!(t.accent, Color::Cyan);
    }
}
