//! Theme module for centralized color and style definitions
//!
//! This module provides semantic color tokens and styles used throughout the
//! UI. Every colour a view draws comes from here - raw `Color::` literals in
//! view code are a bug, because they are invisible to a theme change.
//!
//! The palette comes in three capability tiers: truecolor is the design
//! layer, 256-colour the fallback, and 16 ANSI colours the SSH baseline. The
//! tiers agree on every token that existed before they did - they differ only
//! in how fine a grey ramp they can express - so the worst case is exactly
//! the classic appearance. The tier is detected from `COLORTERM`/`TERM` at
//! startup, and can be forced with the `theme` key in `config.toml`.

use ratatui::style::{Color, Modifier, Style};

use crate::config::ThemeMode;

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
#[derive(Debug, Clone)]
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

impl Default for Theme {
    fn default() -> Self {
        Self::ansi16()
    }
}

impl Theme {
    /// The theme for a detected capability tier
    pub fn for_support(support: ColorSupport) -> Self {
        match support {
            ColorSupport::TrueColor => Self::truecolor(),
            ColorSupport::Ansi256 => Self::ansi256(),
            ColorSupport::Ansi16 => Self::ansi16(),
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
            AttentionReason::Crashed { .. } => self.danger,
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

/// Global theme instance
static THEME: std::sync::OnceLock<Theme> = std::sync::OnceLock::new();

/// Install the theme for this run, from the configured mode
///
/// Called once at startup, before the first render. `Auto` detects the tier
/// from the environment; the other modes force one, for when detection is
/// wrong. A second call is ignored: the theme is pinned for the process.
pub fn init(mode: ThemeMode) {
    let support = match mode {
        ThemeMode::Auto => detect_color_support(),
        ThemeMode::TrueColor => ColorSupport::TrueColor,
        ThemeMode::Ansi256 => ColorSupport::Ansi256,
        ThemeMode::Ansi16 => ColorSupport::Ansi16,
    };
    if THEME.set(Theme::for_support(support)).is_err() {
        tracing::warn!("Theme already initialised; ignoring re-init");
    }
}

/// Get the current theme
///
/// Falls back to the 16-colour baseline when [`init`] has not run - which is
/// the case in tests, keeping their colours independent of the environment.
pub fn theme() -> &'static Theme {
    THEME.get_or_init(Theme::default)
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

    /// The tiers refine the palette; they must not redefine it. Every token
    /// that predates the tiers holds the same value in all three, which is
    /// what makes tier detection safe to ship with no visible change.
    #[test]
    fn test_tiers_agree_on_every_pre_tier_token() {
        let base = Theme::ansi16();
        for tier in [Theme::ansi256(), Theme::truecolor()] {
            assert_eq!(tier.accent, base.accent);
            assert_eq!(tier.text, base.text);
            assert_eq!(tier.text_dim, base.text_dim);
            assert_eq!(tier.selected, base.selected);
            assert_eq!(tier.active, base.active);
            assert_eq!(tier.input_prompt, base.input_prompt);
            assert_eq!(tier.attention_waiting, base.attention_waiting);
            assert_eq!(tier.attention_idle, base.attention_idle);
            assert_eq!(tier.attention_badge, base.attention_badge);
            assert_eq!(tier.error_bg, base.error_bg);
            assert_eq!(tier.error_fg, base.error_fg);
            assert_eq!(tier.warning_bg, base.warning_bg);
            assert_eq!(tier.warning_fg, base.warning_fg);
            assert_eq!(tier.border, base.border);
            assert_eq!(tier.border_focus, base.border_focus);
            assert_eq!(tier.border_warning, base.border_warning);
            assert_eq!(tier.confirm_key, base.confirm_key);
            assert_eq!(tier.cancel_key, base.cancel_key);
            assert_eq!(tier.default_marker, base.default_marker);
            assert_eq!(tier.bg_base, base.bg_base);
            // Suspended is deliberately absent: it is a structural grey, and
            // the greys are exactly what the tiers refine. On the baseline it
            // has to share the one grey with `text_dim`; the richer tiers
            // give it its own so the unfocused-pane dimmer never catches it.
            for state in [
                crate::session::SessionState::Starting,
                crate::session::SessionState::Thinking,
                crate::session::SessionState::Executing,
                crate::session::SessionState::AwaitingApproval,
                crate::session::SessionState::Waiting,
                crate::session::SessionState::Exited,
                crate::session::SessionState::Resumable,
            ] {
                assert_eq!(
                    tier.session_state_color(&state),
                    base.session_state_color(&state),
                    "{state:?}"
                );
            }
        }
    }

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
