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

/// Few-shot examples included in every prompt: (instruction, command).
pub(crate) const PROMPT_EXAMPLES: [(&str, &str); 10] = [
    ("list all files including hidden ones, newest first", "ls -lat"),
    ("list only the hidden files, sorted by modification time", "ls -ldt .[!.]*"),
    ("show disk usage of each folder here, biggest last", "du -sh */ | sort -h"),
    ("find all files larger than 100MB", "find . -type f -size +100M"),
    ("count lines of code in all rust files", "find . -name '*.rs' -print0 | xargs -0 wc -l"),
    ("what is using port 8080", "ss -ltnp | grep :8080"),
    ("show the last 20 commits briefly", "git log --oneline -20"),
    ("show free disk space", "df -h"),
    ("search for TODO in all files", "grep -rn TODO ."),
    ("show the 10 processes using the most memory", "ps aux --sort=-%mem | head -11"),
];

/// Most file names put in the prompt; every extra name costs prompt-processing time.
const MAX_LISTED_FILES: usize = 15;
/// Most terminal lines put in the prompt.
const MAX_OUTPUT_LINES: usize = 3;

/// Words that suggest the request depends on what is in the current directory.
const FILE_WORDS: [&str; 24] = [
    "file", "folder", "director", "dir", "here", "this", "current", "ls", "list", "show",
    "find", "delete", "remove", "rename", "move", "copy", "size", "large", "big", "hidden",
    "open", "edit", "archive", "zip",
];

/// Whether the request is likely to need the directory listing.
fn mentions_files(instruction: &str) -> bool {
    let lower = instruction.to_lowercase();
    FILE_WORDS.iter().any(|word| lower.contains(word))
}

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
                let is_dir = file_type.is_some_and(|ft| ft.is_dir());

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
    ///
    /// Few-shot examples steer small models away from plausible-looking but wrong commands.
    ///
    /// Layout matters for speed: Ollama caches the longest identical prompt prefix, so the
    /// fixed text (role, rules, examples) comes first and the per-request text comes last.
    pub fn format_prompt(&self, instruction: &str) -> String {
        let mut prompt = String::from(
            "You are Kourage, an intelligent terminal assistant. Convert the user's natural language instruction into a single, valid, raw shell command for the current environment.\n\n\
            ### Output Rules:\n\
            - ONLY return the raw, single-line shell command.\n\
            - Do NOT include markdown formatting, code fences, or backticks.\n\
            - Do NOT explain or include conversational text.\n\
            - Choose the exact command tailored to the current directory, tech stack, and OS.\n"
        );

        prompt.push_str("\n### Examples (GNU/Linux; adapt to the environment below):\n");
        for (request, command) in PROMPT_EXAMPLES {
            prompt.push_str(&format!("Instruction: \"{request}\"\nCommand: {command}\n"));
        }

        prompt.push_str("\n### Current Environment:\n");

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

        if !self.directory_contents.is_empty() && mentions_files(instruction) {
            let listing: Vec<&str> =
                self.directory_contents.iter().take(MAX_LISTED_FILES).map(String::as_str).collect();
            prompt.push_str(&format!("- Files in Directory: {}\n", listing.join(", ")));
        }

        if let Some(recent_buffer) = &self.recent_terminal_lines {
            let lines: Vec<&str> = recent_buffer.lines().filter(|l| !l.trim().is_empty()).collect();
            if !lines.is_empty() {
                let tail = &lines[lines.len().saturating_sub(MAX_OUTPUT_LINES)..];
                prompt.push_str("\n### Recent Terminal Output (for context / error diagnosis):\n```\n");
                prompt.push_str(&tail.join("\n"));
                prompt.push_str("\n```\n");
            }
        }

        prompt.push_str(&format!("\n### User Instruction:\n\"{instruction}\"\n"));

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

    #[test]
    fn prompt_has_examples_before_the_instruction() {
        let prompt = AiContext::default().format_prompt("do the thing");
        let examples = prompt.find("### Examples").expect("examples section");
        let instruction = prompt.find("### User Instruction").expect("instruction section");
        assert!(examples < instruction);
        assert!(prompt.contains("df -h"));
    }

    #[test]
    fn prompt_puts_fixed_text_before_changing_text() {
        // Ollama reuses its cache for an identical prefix, so everything that never changes
        // must come first and everything that varies per request must come last.
        let context = AiContext {
            cwd: Some("/work".into()),
            os: Some("Linux".into()),
            shell: Some("bash".into()),
            recent_terminal_lines: Some("some output".into()),
            ..Default::default()
        };
        let prompt = context.format_prompt("do the thing");
        let rules = prompt.find("### Output Rules").unwrap();
        let examples = prompt.find("### Examples").unwrap();
        let env = prompt.find("### Current Environment").unwrap();
        let output = prompt.find("### Recent Terminal Output").unwrap();
        let instruction = prompt.find("### User Instruction").unwrap();
        assert!(rules < examples && examples < env && env < output && output < instruction);
    }

    #[test]
    fn fixed_prefix_is_identical_across_requests() {
        let a = AiContext { cwd: Some("/a".into()), ..Default::default() }.format_prompt("one");
        let b = AiContext { cwd: Some("/b".into()), ..Default::default() }.format_prompt("two");
        let end = a.find("### Current Environment").unwrap();
        assert_eq!(a[..end], b[..b.find("### Current Environment").unwrap()]);
    }

    #[test]
    fn file_listing_only_when_request_mentions_files() {
        let context = AiContext {
            directory_contents: vec!["Cargo.toml".into(), "src/".into()],
            ..Default::default()
        };
        assert!(!context.format_prompt("what is my ip address").contains("Files in Directory"));
        assert!(context.format_prompt("delete the large files here").contains("Files in Directory"));
        assert!(context.format_prompt("show my folders").contains("Files in Directory"));
    }

    #[test]
    fn file_listing_is_capped() {
        let context = AiContext {
            directory_contents: (0..35).map(|i| format!("f{i:02}")).collect(),
            ..Default::default()
        };
        let prompt = context.format_prompt("list the files");
        assert!(prompt.contains("f14"));
        assert!(!prompt.contains("f15"));
    }

    #[test]
    fn terminal_output_is_limited_to_the_last_lines() {
        let context = AiContext {
            recent_terminal_lines: Some("l1\nl2\nl3\nl4\nl5".into()),
            ..Default::default()
        };
        let prompt = context.format_prompt("x");
        assert!(prompt.contains("l3\nl4\nl5"));
        assert!(!prompt.contains("l2"));
    }

    #[test]
    fn every_example_is_a_safe_single_line_command() {
        for (request, command) in PROMPT_EXAMPLES {
            assert!(!request.is_empty());
            assert_eq!(crate::ai::require_single_line(command).unwrap(), *command);
            assert!(!command.contains('`'), "backticks in example: {command}");
            assert!(!crate::ai::is_destructive_command(command), "unsafe example: {command}");
        }
    }
}
