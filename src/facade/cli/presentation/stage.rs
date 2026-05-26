// EasyNet CLI — live stage renderer
// ==================================
//
// File: src/facade/cli/presentation/stage.rs
// Description: Reusable "running stages" UI. Every CLI command that
//              walks through a sequence of named steps (`easynet
//              start`, `easynet join`, ...) renders the same way
//              through this module:
//
//                ◐ <active stage, cyan shimmer>
//                ✓ <completed stage>
//                ✗ <failed stage: reason>
//                - <skipped stage>
//
//              Stage labels render flush-left; completion icons are
//              colored, the rest of the line stays in the
//              terminal's default foreground so a wall of stages
//              stays scannable. A background tick thread shimmers
//              the active stage label in cyan (project accent color)
//              every 60 ms; calling code never touches the
//              `ProgressBar` directly.
//
// Threading
// ---------
// `StageRenderer` is `!Sync`. Construct it on the thread that owns
// the boot loop; the renderer itself spawns one ticker thread
// internally and joins it on drop. The renderer cannot be cloned —
// there is one live spinner per command invocation.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use console::style;
use indicatif::{ProgressBar, ProgressStyle};

/// Interval at which the shimmer phase advances on the active
/// stage label. Fast enough to read as motion, slow enough not to
/// flood the terminal with repaints.
const SHIMMER_TICK: Duration = Duration::from_millis(60);

/// Spinner tick interval. Slower than [`SHIMMER_TICK`] because the
/// glyph rotation reads as motion on its own — a faster tick
/// becomes nervous, not informative.
const SPINNER_TICK: Duration = Duration::from_millis(120);

/// Width of the moving highlight band, in characters. The band
/// holds exactly one bright peak with one mid-intensity character
/// on each side; every character outside the band renders dim.
/// The band's center advances by one character per
/// [`SHIMMER_TICK`], so the eye reads "one highlight sweeping
/// left-to-right" rather than a striped pattern.
const SHIMMER_BAND_RADIUS: usize = 1;

/// Length of one full sweep cycle: the band's center walks from
/// position 0 to the end of the text and pauses before wrapping,
/// so wider text takes proportionally longer to traverse but the
/// "one peak" character of the animation is preserved.
const SHIMMER_TRAILING_PAUSE: usize = 4;

/// Render one character at the given offset from the band center.
/// `peak` (offset 0) is bold; `offset == ±SHIMMER_BAND_RADIUS` is
/// normal-weight; anything farther is dim. Single hue (cyan, the
/// project accent color) — see `presentation/banner.rs` §2: no
/// 256-colour gradients, no colour-for-emphasis-by-volume.
fn style_at_offset(ch: char, offset_from_peak: usize) -> console::StyledObject<char> {
    match offset_from_peak {
        0 => style(ch).cyan().bold(),
        o if o <= SHIMMER_BAND_RADIUS => style(ch).cyan(),
        _ => style(ch).cyan().dim(),
    }
}

/// Spinner glyphs. U+25D0..U+25D3 (◐◓◑◒) sit on the cap-height
/// baseline so they line up vertically with surrounding `✓` glyphs;
/// the default braille spinner sat on the bottom baseline and
/// produced the "spinner below the stage line" visual bug.
const SPINNER_GLYPHS: &[&str] = &["◐", "◓", "◑", "◒"];

/// Live stage renderer. Owns one indicatif `ProgressBar` and one
/// background ticker thread; methods are the only sanctioned way to
/// emit stage updates for any CLI command.
///
/// Lifecycle: build with [`StageRenderer::new`]; emit stage
/// transitions via [`StageRenderer::set_active`],
/// [`StageRenderer::stage_ok`], [`StageRenderer::stage_skipped`],
/// [`StageRenderer::stage_failed`], or one-off [`StageRenderer::info`];
/// drop the renderer (or call [`StageRenderer::finish`] explicitly)
/// to stop the ticker and clear the spinner.
pub struct StageRenderer {
    pb: ProgressBar,
    shimmer: Arc<ShimmerState>,
    ticker: Option<JoinHandle<()>>,
}

impl StageRenderer {
    /// Construct a renderer with no active stage. Call
    /// [`StageRenderer::set_active`] before completing any stages
    /// to give the spinner a label; otherwise it renders blank
    /// until the first `set_active`.
    pub fn new() -> Self {
        Self::with_initial_message(String::new())
    }

    /// Like [`StageRenderer::new`] but seeds the active label
    /// before the first stage event. Used by commands that need
    /// to show "waiting for X" before they have a real stage to
    /// report (the daemon socket probe in `start`'s boot watcher).
    pub fn with_initial_message(label: impl Into<String>) -> Self {
        let pb = ProgressBar::new_spinner();
        pb.set_style(
            ProgressStyle::with_template("{spinner:.cyan} {msg}")
                .unwrap_or_else(|_| ProgressStyle::default_spinner())
                .tick_strings(SPINNER_GLYPHS),
        );
        pb.enable_steady_tick(SPINNER_TICK);

        let shimmer = Arc::new(ShimmerState::new());
        shimmer.set(label);
        let ticker = spawn_shimmer_ticker(pb.clone(), shimmer.clone(), Instant::now());

        Self {
            pb,
            shimmer,
            ticker: Some(ticker),
        }
    }

    /// Tell the renderer which stage is currently in flight. The
    /// ticker picks up the new label on its next iteration and
    /// shimmers it in place.
    pub fn set_active(&self, name: impl Into<String>) {
        self.shimmer.set(name);
    }

    /// Mark the currently-active stage (or any named stage) as
    /// finished successfully. Prints one flush-left line above the
    /// spinner with a green `✓` icon.
    pub fn stage_ok(&self, name: &str) {
        self.pb
            .println(format!("{} {name}", style("✓").green().bold()));
    }

    /// Mark a stage as deliberately skipped. Prints a dim `-` icon
    /// and an explanatory suffix (typically `"skipped"` or
    /// `"skipped (reason)"`).
    pub fn stage_skipped(&self, name: &str, note: &str) {
        self.pb
            .println(format!("{} {name} {note}", style("-").dim()));
    }

    /// Mark a stage as failed. Prints a red `✗` icon, the stage
    /// name, and the reason. Does NOT stop the renderer — the
    /// caller decides whether to continue or `finish` and bail.
    pub fn stage_failed(&self, name: &str, reason: &str) {
        self.pb.println(format!(
            "{} {name}: {reason}",
            style("✗").red().bold()
        ));
    }

    /// Emit a free-form informational line above the spinner. Used
    /// for one-off notices that don't fit the stage shape (e.g.
    /// "boot stream lagged; skipped N events"). Renders with a
    /// dim `-` icon so it reads as ancillary, not as a stage
    /// completion.
    pub fn info(&self, line: &str) {
        self.pb
            .println(format!("{} {line}", style("-").dim()));
    }

    /// Stop the ticker, join it, and clear the spinner. After
    /// `finish` the renderer is inert. Idempotent — calling it
    /// twice is safe but only the first call does any work.
    ///
    /// Order matters: stop+join the ticker BEFORE clearing the
    /// spinner so the ticker cannot race against `finish_and_clear`
    /// and leave a stale shimmer line in scrollback.
    pub fn finish(&mut self) {
        if let Some(handle) = self.ticker.take() {
            self.shimmer.stop.store(true, Ordering::Relaxed);
            let _ = handle.join();
            self.pb.finish_and_clear();
        }
    }
}

impl Default for StageRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for StageRenderer {
    fn drop(&mut self) {
        self.finish();
    }
}

// ── Shimmer internals ─────────────────────────────────────────────

/// State shared between the foreground caller (who updates the
/// active stage name) and the background ticker thread (who reads
/// it and repaints).
struct ShimmerState {
    message: Mutex<String>,
    stop: AtomicBool,
}

impl ShimmerState {
    fn new() -> Self {
        Self {
            message: Mutex::new(String::new()),
            stop: AtomicBool::new(false),
        }
    }

    fn set(&self, msg: impl Into<String>) {
        if let Ok(mut g) = self.message.lock() {
            *g = msg.into();
        }
    }

    fn read(&self) -> String {
        self.message
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default()
    }
}

fn spawn_shimmer_ticker(
    pb: ProgressBar,
    state: Arc<ShimmerState>,
    start: Instant,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        while !state.stop.load(Ordering::Relaxed) {
            let msg = state.read();
            if !msg.is_empty() {
                let phase =
                    (start.elapsed().as_millis() / SHIMMER_TICK.as_millis().max(1)) as usize;
                pb.set_message(render_shimmer(&msg, phase));
            }
            std::thread::sleep(SHIMMER_TICK);
        }
    })
}

/// Render `text` with a single cyan highlight band that sweeps
/// from left to right as `phase` advances. The band has exactly
/// one bright peak; every character outside the band renders dim.
/// `phase` is computed by the ticker as elapsed-ms / SHIMMER_TICK,
/// so one full sweep takes `(text_len + SHIMMER_TRAILING_PAUSE) *
/// SHIMMER_TICK` milliseconds.
///
/// Exposed `pub(crate)` so unit tests can pin the rendering output
/// without going through the ticker.
pub(crate) fn render_shimmer(text: &str, phase: usize) -> String {
    if text.is_empty() {
        return String::new();
    }
    let chars: Vec<char> = text.chars().collect();
    let cycle = chars.len() + SHIMMER_TRAILING_PAUSE;
    let peak = phase % cycle;
    let mut out = String::with_capacity(chars.len() * 4);
    for (i, ch) in chars.iter().copied().enumerate() {
        // `abs_diff` keeps the band symmetric without integer
        // underflow when `i < peak`. Positions past the text edge
        // (peak >= chars.len()) fall into the trailing pause: the
        // whole string renders dim during that interval, so the
        // animation breathes between sweeps instead of snapping
        // back instantly.
        let offset = i.abs_diff(peak);
        out.push_str(&style_at_offset(ch, offset).to_string());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_shimmer_handles_empty_input() {
        assert_eq!(render_shimmer("", 0), "");
    }

    #[test]
    fn render_shimmer_phase_shifts_palette_slot_per_char() {
        // Two renders one slot apart must NOT produce identical
        // bytes: every character's palette slot rotates by one, so
        // the ANSI prefix on at least one character differs.
        //
        // `console::set_colors_enabled(true)` is required because
        // cargo test runs under a non-TTY harness; without it,
        // `console::style` returns plain text and the assertion
        // would fail on a true-positive ANSI strip rather than
        // catching a regression in the palette rotation.
        console::set_colors_enabled(true);
        let a = render_shimmer("kernel", 0);
        let b = render_shimmer("kernel", 1);
        assert_ne!(a, b, "shimmer phase shift must change output bytes");
    }
}
