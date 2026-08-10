use std::env;
use std::fs;
use std::path::Path;
#[cfg(windows)]
use std::path::PathBuf;
#[cfg(not(windows))]
use std::os::unix::io::RawFd;

use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::term::Term;

use crate::daemon::foreground_process_path;

/// Rich environmental context gathered from the active terminal window.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AiContext {
    /// Active Current Working Directory.
    pub cwd: Option<String>,
    /// Top-level directory contents (up to 35 items).
    pub directory_contents: Vec<String>,
    /// Detected project tech stack (Rust, Node.js, Python, Go, etc.).
    pub detected_stack: Option<String>,
    /// Active shell ($SHELL, e.g. bash, zsh, fish).
    pub shell: Option<String>,
    /// Operating System and distribution info.
    pub os: Option<String>,
    /// Recent visible lines from terminal screen buffer.
    pub recent_terminal_lines: Option<String>,
}

impl AiContext {
    /// Extract environmental context from the terminal state.
    pub fn extract<T: EventListener>(
        #[cfg(not(windows))] master_fd: RawFd,
        #[cfg(not(windows))] shell_pid: u32,
        terminal: Option<&Term<T>>,
    ) -> Self {
        #[cfg(not(windows))]
        let cwd_path = foreground_process_path(master_fd, shell_pid).ok();
        #[cfg(windows)]
        let cwd_path: Option<PathBuf> = None;

        let cwd_str = cwd_path.as_ref().map(|p| p.to_string_lossy().to_string());

        let (directory_contents, detected_stack) = match &cwd_path {
            Some(path) => {
                let contents = Self::list_directory(path);
                let stack = Self::detect_tech_stack(path, &contents);
                (contents, stack)
            },
            None => (Vec::new(), None),
        };

        let shell = Self::detect_shell();
        let os = Self::detect_os();
        let recent_terminal_lines = terminal.map(|term| Self::extract_recent_lines(term, 20));

        Self {
            cwd: cwd_str,
            directory_contents,
            detected_stack,
            shell: Some(shell),
            os: Some(os),
            recent_terminal_lines,
        }
    }

    /// List directory contents (ignoring heavy hidden files, capped at 35 items).
    fn list_directory(path: &Path) -> Vec<String> {
        let mut entries = Vec::new();
        if let Ok(read_dir) = fs::read_dir(path) {
            for entry in read_dir.flatten() {
                let file_name = entry.file_name().to_string_lossy().to_string();
                let file_type = entry.file_type().ok();
                let is_dir = file_type.map_or(false, |ft| ft.is_dir());

                let display_name = if is_dir {
                    format!("{}/", file_name)
                } else {
                    file_name
                };

                entries.push(display_name);
            }
        }

        // Sort alphabetically
        entries.sort();
        entries.truncate(35);
        entries
    }

    /// Detect project tech stack and build tools from files in CWD.
    fn detect_tech_stack(_path: &Path, contents: &[String]) -> Option<String> {
        let mut tags = Vec::new();

        let has_file = |name: &str| contents.iter().any(|c| c == name || c == &format!("{}/", name));

        if has_file("Cargo.toml") {
            tags.push("Rust (Cargo)");
        }
        if has_file("package.json") {
            tags.push("JavaScript/TypeScript (Node.js/npm)");
        }
        if has_file("pyproject.toml") || has_file("requirements.txt") || has_file("Pipfile") || has_file("setup.py") {
            tags.push("Python");
        }
        if has_file("go.mod") {
            tags.push("Go");
        }
        if has_file("Makefile") || has_file("CMakeLists.txt") {
            tags.push("C/C++ (Make/CMake)");
        }
        if has_file("pom.xml") || has_file("build.gradle") || has_file("build.gradle.kts") {
            tags.push("Java/Kotlin (Maven/Gradle)");
        }
        if has_file("Gemfile") {
            tags.push("Ruby (Bundler)");
        }
        if has_file("composer.json") {
            tags.push("PHP (Composer)");
        }
        if has_file("Dockerfile") || has_file("docker-compose.yml") || has_file("docker-compose.yaml") {
            tags.push("Docker");
        }
        if has_file(".git/") {
            tags.push("Git Repository");
        }

        if tags.is_empty() {
            None
        } else {
            Some(tags.join(", "))
        }
    }

    /// Detect the active shell.
    fn detect_shell() -> String {
        env::var("SHELL")
            .ok()
            .and_then(|s| {
                Path::new(&s)
                    .file_name()
                    .map(|f| f.to_string_lossy().to_string())
            })
            .unwrap_or_else(|| "bash".to_string())
    }

    /// Detect the operating system and distribution name.
    fn detect_os() -> String {
        #[cfg(target_os = "linux")]
        {
            if let Ok(release) = fs::read_to_string("/etc/os-release") {
                for line in release.lines() {
                    if let Some(pretty) = line.strip_prefix("PRETTY_NAME=") {
                        let clean = pretty.trim_matches('"').trim();
                        if !clean.is_empty() {
                            return format!("{} (Linux)", clean);
                        }
                    }
                }
            }
            "Linux".to_string()
        }

        #[cfg(target_os = "macos")]
        {
            "macOS (Unix)".to_string()
        }

        #[cfg(windows)]
        {
            "Windows".to_string()
        }

        #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
        {
            "Unix".to_string()
        }
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

    /// Format the system prompt embedding all rich context.
    pub fn format_prompt(&self, instruction: &str) -> String {
        let mut prompt = String::from(
            "You are Kourage, an intelligent terminal assistant. Convert the user's natural language instruction into a single, valid, raw shell command for the current environment.\n\n"
        );

        prompt.push_str("### Current Environment:\n");

        if let Some(os) = &self.os {
            prompt.push_str(&format!("- Operating System: {}\n", os));
        }

        if let Some(shell) = &self.shell {
            prompt.push_str(&format!("- Shell: {}\n", shell));
        }

        if let Some(cwd) = &self.cwd {
            prompt.push_str(&format!("- Current Working Directory: {}\n", cwd));
        }

        if let Some(stack) = &self.detected_stack {
            prompt.push_str(&format!("- Project / Tech Stack: {}\n", stack));
        }

        if !self.directory_contents.is_empty() {
            prompt.push_str(&format!("- Files in Directory: {}\n", self.directory_contents.join(", ")));
        }

        if let Some(recent_buffer) = &self.recent_terminal_lines {
            if !recent_buffer.trim().is_empty() {
                prompt.push_str("\n### Recent Terminal Output (for context / error diagnosis):\n```\n");
                prompt.push_str(recent_buffer);
                prompt.push_str("\n```\n");
            }
        }

        prompt.push_str(&format!(
            "\n### User Instruction:\n\"{}\"\n\n\
            ### Output Rules:\n\
            - ONLY return the raw, single-line shell command.\n\
            - Do NOT include markdown formatting, code fences, or backticks.\n\
            - Do NOT explain or include conversational text.\n\
            - Choose the exact command tailored to the current directory, tech stack, and OS.\n",
            instruction
        ));

        prompt
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_tech_stack() {
        let rust_contents = vec!["Cargo.toml".to_string(), "src/".to_string(), ".git/".to_string()];
        let stack = AiContext::detect_tech_stack(Path::new("."), &rust_contents);
        assert_eq!(stack, Some("Rust (Cargo), Git Repository".to_string()));

        let node_contents = vec!["package.json".to_string(), "node_modules/".to_string()];
        let stack_node = AiContext::detect_tech_stack(Path::new("."), &node_contents);
        assert_eq!(stack_node, Some("JavaScript/TypeScript (Node.js/npm)".to_string()));

        let empty_contents: Vec<String> = Vec::new();
        let stack_empty = AiContext::detect_tech_stack(Path::new("."), &empty_contents);
        assert_eq!(stack_empty, None);
    }

    #[test]
    fn test_format_prompt_contains_context() {
        let context = AiContext {
            cwd: Some("/home/sunidhi/project".to_string()),
            directory_contents: vec!["Cargo.toml".to_string(), "src/".to_string()],
            detected_stack: Some("Rust (Cargo)".to_string()),
            shell: Some("bash".to_string()),
            os: Some("Ubuntu 24.04 (Linux)".to_string()),
            recent_terminal_lines: Some("error: test failed".to_string()),
        };

        let formatted = context.format_prompt("run the tests");
        assert!(formatted.contains("/home/sunidhi/project"));
        assert!(formatted.contains("Rust (Cargo)"));
        assert!(formatted.contains("Ubuntu 24.04 (Linux)"));
        assert!(formatted.contains("error: test failed"));
        assert!(formatted.contains("\"run the tests\""));
    }
}
