//! Facts about the machine the terminal runs on, behind a trait so tests can fake them.

use std::env;
use std::path::Path;

/// Source of operating-system and shell descriptions for the prompt.
pub trait Platform {
    /// Operating system and distribution, e.g. `"Ubuntu 24.04 (Linux)"`.
    fn os_description(&self) -> String;
    /// Name of the active shell, e.g. `"bash"`.
    fn shell(&self) -> String;
}

/// The real machine.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemPlatform;

impl Platform for SystemPlatform {
    fn os_description(&self) -> String {
        #[cfg(target_os = "linux")]
        {
            if let Ok(release) = std::fs::read_to_string("/etc/os-release") {
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

    fn shell(&self) -> String {
        env::var("SHELL")
            .ok()
            .and_then(|s| Path::new(&s).file_name().map(|f| f.to_string_lossy().to_string()))
            .unwrap_or_else(|| "bash".to_string())
    }
}
