//! Environmental context gathered for the command-generating model.

use std::fs;
use std::path::Path;

use crate::platform::Platform;

/// Most directory entries kept in the context.
const MAX_DIRECTORY_ENTRIES: usize = 35;

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
    /// Gather context for `cwd` (if known) on `platform`.
    pub fn gather(
        cwd: Option<&Path>,
        platform: &dyn Platform,
        recent_terminal_lines: Option<String>,
    ) -> Self {
        let (directory_contents, detected_stack) = match cwd {
            Some(path) => {
                let contents = list_directory(path);
                let stack = detect_tech_stack(&contents);
                (contents, stack)
            },
            None => (Vec::new(), None),
        };

        Self {
            cwd: cwd.map(|p| p.to_string_lossy().to_string()),
            directory_contents,
            detected_stack,
            shell: Some(platform.shell()),
            os: Some(platform.os_description()),
            recent_terminal_lines,
        }
    }
}

/// List directory contents sorted, directories with a trailing '/', capped at 35 items.
fn list_directory(path: &Path) -> Vec<String> {
    let mut entries = Vec::new();
    if let Ok(read_dir) = fs::read_dir(path) {
        for entry in read_dir.flatten() {
            let file_name = entry.file_name().to_string_lossy().to_string();
            let file_type = entry.file_type().ok();
            let is_dir = file_type.is_some_and(|ft| ft.is_dir());

            let display_name = if is_dir { format!("{}/", file_name) } else { file_name };

            entries.push(display_name);
        }
    }

    // Sort alphabetically
    entries.sort();
    entries.truncate(MAX_DIRECTORY_ENTRIES);
    entries
}

/// Detect project tech stack and build tools from files in CWD.
fn detect_tech_stack(contents: &[String]) -> Option<String> {
    let mut tags = Vec::new();

    let has_file = |name: &str| contents.iter().any(|c| c == name || c == &format!("{}/", name));

    if has_file("Cargo.toml") {
        tags.push("Rust (Cargo)");
    }
    if has_file("package.json") {
        tags.push("JavaScript/TypeScript (Node.js/npm)");
    }
    if has_file("pyproject.toml")
        || has_file("requirements.txt")
        || has_file("Pipfile")
        || has_file("setup.py")
    {
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
    if has_file("Dockerfile") || has_file("docker-compose.yml") || has_file("docker-compose.yaml")
    {
        tags.push("Docker");
    }
    if has_file(".git/") {
        tags.push("Git Repository");
    }

    if tags.is_empty() { None } else { Some(tags.join(", ")) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_tech_stack() {
        let rust_contents = vec!["Cargo.toml".to_string(), "src/".to_string(), ".git/".to_string()];
        let stack = detect_tech_stack(&rust_contents);
        assert_eq!(stack, Some("Rust (Cargo), Git Repository".to_string()));

        let node_contents = vec!["package.json".to_string(), "node_modules/".to_string()];
        let stack_node = detect_tech_stack(&node_contents);
        assert_eq!(stack_node, Some("JavaScript/TypeScript (Node.js/npm)".to_string()));

        let empty_contents: Vec<String> = Vec::new();
        let stack_empty = detect_tech_stack(&empty_contents);
        assert_eq!(stack_empty, None);
    }

    /// Fresh empty directory under the system temp dir, removed by the caller.
    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("kourage-core-{}-{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    struct MockPlatform;

    impl Platform for MockPlatform {
        fn os_description(&self) -> String {
            "TestOS 1.0".into()
        }

        fn shell(&self) -> String {
            "testsh".into()
        }
    }

    #[test]
    fn gather_never_panics_on_unreadable_directory() {
        let missing = Path::new("/definitely/not/a/real/kourage/dir");
        let context = AiContext::gather(Some(missing), &MockPlatform, None);
        assert!(context.directory_contents.is_empty());
        assert_eq!(context.detected_stack, None);
        assert_eq!(context.cwd.as_deref(), Some("/definitely/not/a/real/kourage/dir"));
    }

    #[test]
    fn gather_caps_listing_at_35() {
        let dir = temp_dir("cap");
        for i in 0..40 {
            fs::write(dir.join(format!("f{i:02}")), "").unwrap();
        }
        fs::create_dir(dir.join("a_sub")).unwrap();
        let context = AiContext::gather(Some(&dir), &MockPlatform, None);
        fs::remove_dir_all(&dir).unwrap();

        assert_eq!(context.directory_contents.len(), 35);
        assert_eq!(context.directory_contents[0], "a_sub/");
        assert_eq!(context.directory_contents[1], "f00");
        assert_eq!(context.directory_contents[34], "f33");
    }

    #[cfg(unix)]
    #[test]
    fn gather_handles_non_utf8_names() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let dir = temp_dir("non-utf8");
        fs::write(dir.join(OsStr::from_bytes(b"bad\xffname")), "").unwrap();
        fs::write(dir.join("Cargo.toml"), "").unwrap();
        let context = AiContext::gather(Some(&dir), &MockPlatform, None);
        fs::remove_dir_all(&dir).unwrap();

        assert_eq!(context.directory_contents, vec!["Cargo.toml", "bad\u{FFFD}name"]);
        assert_eq!(context.detected_stack.as_deref(), Some("Rust (Cargo)"));
    }

    #[test]
    fn mock_platform_gives_deterministic_context() {
        let gather = || AiContext::gather(None, &MockPlatform, Some("last line".into()));
        let expected = AiContext {
            cwd: None,
            directory_contents: Vec::new(),
            detected_stack: None,
            shell: Some("testsh".into()),
            os: Some("TestOS 1.0".into()),
            recent_terminal_lines: Some("last line".into()),
        };
        assert_eq!(gather(), expected);
        assert_eq!(gather().format_prompt("list files"), gather().format_prompt("list files"));
    }
}
