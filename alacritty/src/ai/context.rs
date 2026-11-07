//! Terminal-side context gathering; the context itself lives in `kourage_core`.

#[cfg(not(windows))]
use std::os::unix::io::RawFd;
#[cfg(windows)]
use std::path::PathBuf;

use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::term::Term;
use kourage_core::{AiContext, SystemPlatform};

#[cfg(not(windows))]
use crate::daemon::foreground_process_path;

/// Extract environmental context from the terminal state.
pub fn extract_ai_context<T: EventListener>(
    #[cfg(not(windows))] master_fd: RawFd,
    #[cfg(not(windows))] shell_pid: u32,
    terminal: Option<&Term<T>>,
) -> AiContext {
    #[cfg(not(windows))]
    let cwd = foreground_process_path(master_fd, shell_pid).ok();
    #[cfg(windows)]
    let cwd: Option<PathBuf> = None;

    let recent_lines = terminal.map(|term| extract_recent_lines(term, 20));

    AiContext::gather(cwd.as_deref(), &SystemPlatform, recent_lines)
}

/// Extract recent non-empty lines from the terminal screen buffer.
fn extract_recent_lines<T: EventListener>(term: &Term<T>, max_lines: usize) -> String {
    let total_screen_lines = term.screen_lines();
    if total_screen_lines == 0 {
        return String::new();
    }

    let start_line_num = total_screen_lines.saturating_sub(max_lines);
    let end_line_num = total_screen_lines.saturating_sub(1);

    let start_point = Point::new(Line(start_line_num as i32), Column(0));
    let end_point = Point::new(Line(end_line_num as i32), term.last_column());

    let raw_text = term.bounds_to_string(start_point, end_point);

    // Filter out excessive trailing empty lines while keeping structure
    let lines: Vec<&str> = raw_text.lines().collect();
    let trimmed_lines: Vec<&str> = lines
        .into_iter()
        .rev()
        .skip_while(|l| l.trim().is_empty())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();

    trimmed_lines.join("\n")
}
