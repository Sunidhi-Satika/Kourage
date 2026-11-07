use serde::Serialize;
use alacritty_config_derive::ConfigDeserialize;

#[derive(ConfigDeserialize, Serialize, Debug, Clone, PartialEq)]
pub struct Voice {
    pub llm_api_url: String,
    pub whisper_model_path: String,
    pub model: String,
    pub temperature: f32,
    pub timeout_secs: u64,
    /// Load the model in the background at startup so the first prompt isn't a cold start.
    pub warm_up: bool,
}

impl Default for Voice {
    fn default() -> Self {
        Self {
            llm_api_url: "http://localhost:11434/api/generate".to_string(),
            whisper_model_path: "/path/to/your/whisper/model.bin".to_string(),
            model: "qwen2.5-coder:3b".to_string(),
            temperature: 0.0,
            timeout_secs: 15,
            warm_up: true,
        }
    }
}

/// Fold the legacy `[voice]` section into `[ai]` before deserializing. Returns a warning to log
/// when the legacy section was present.
pub fn normalize_legacy_section(config: &mut toml::Value) -> Option<String> {
    let table = config.as_table_mut()?;
    let legacy = table.remove("voice")?;

    let (merged, warning) = match table.remove("ai") {
        Some(ai) => (
            crate::config::serde_utils::merge(legacy, ai),
            "Config has both [ai] and the old [voice] section; merged them, with [ai] taking \
             precedence. Move your settings to [ai] and remove [voice].",
        ),
        None => (
            legacy,
            "The [voice] config section is deprecated; rename it to [ai].",
        ),
    };
    table.insert("ai".to_string(), merged);
    Some(warning.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use toml::Table;

    #[test]
    fn voice_default_config() {
        let voice = Voice::default();
        assert_eq!(voice.llm_api_url, "http://localhost:11434/api/generate");
        assert_eq!(voice.whisper_model_path, "/path/to/your/whisper/model.bin");
        assert_eq!(voice.model, "qwen2.5-coder:3b");
        assert_eq!(voice.temperature, 0.0);
        assert_eq!(voice.timeout_secs, 15);
    }

    #[test]
    fn voice_deserialize_custom_config() {
        let toml_str = r#"
            llm_api_url = "http://192.168.1.100:11434/api/generate"
            whisper_model_path = "~/.config/alacritty/models/ggml-base.bin"
            model = "llama3:8b"
            temperature = 0.2
            timeout_secs = 30
        "#;
        let table: Table = toml::from_str(toml_str).unwrap();
        let voice = Voice::deserialize(toml::Value::Table(table)).unwrap();

        assert_eq!(voice.llm_api_url, "http://192.168.1.100:11434/api/generate");
        assert_eq!(voice.whisper_model_path, "~/.config/alacritty/models/ggml-base.bin");
        assert_eq!(voice.model, "llama3:8b");
        assert_eq!(voice.temperature, 0.2);
        assert_eq!(voice.timeout_secs, 30);
    }

    fn ui_config_from(toml_str: &str) -> crate::config::ui_config::UiConfig {
        let table: Table = toml::from_str(toml_str).unwrap();
        crate::config::ui_config::UiConfig::deserialize(toml::Value::Table(table)).unwrap()
    }

    #[test]
    fn ai_section_configures_the_llm() {
        let config = ui_config_from("[ai]\nmodel = \"llama3:8b\"\ntimeout_secs = 40\n");
        assert_eq!(config.ai.model, "llama3:8b");
        assert_eq!(config.ai.timeout_secs, 40);
    }

    #[test]
    fn legacy_voice_section_still_works() {
        let config = ui_config_from("[voice]\nmodel = \"old:1b\"\n");
        assert_eq!(config.ai.model, "old:1b");
    }

    fn table(toml_str: &str) -> toml::Value {
        toml::Value::Table(toml::from_str::<Table>(toml_str).unwrap())
    }

    #[test]
    fn legacy_only_section_is_renamed_with_a_warning() {
        let mut config = table("[voice]\nmodel = \"old:1b\"\n");
        let warning = normalize_legacy_section(&mut config).expect("warning");
        assert!(warning.contains("[voice]") && warning.contains("[ai]"), "{warning}");
        assert_eq!(config, table("[ai]\nmodel = \"old:1b\"\n"));
    }

    #[test]
    fn when_both_sections_exist_ai_wins_and_voice_fills_gaps() {
        let mut config = table(
            "[voice]\nmodel = \"old\"\ntimeout_secs = 5\n[ai]\nmodel = \"new\"\n",
        );
        let warning = normalize_legacy_section(&mut config).expect("warning");
        assert!(warning.contains("both"), "{warning}");
        assert_eq!(config, table("[ai]\nmodel = \"new\"\ntimeout_secs = 5\n"));
    }

    #[test]
    fn warm_up_defaults_to_true() {
        assert!(Voice::default().warm_up);
        assert!(ui_config_from("[ai]\nmodel = \"x\"\n").ai.warm_up);
    }

    #[test]
    fn warm_up_can_be_disabled() {
        let config = ui_config_from("[ai]\nwarm_up = false\n");
        assert!(!config.ai.warm_up);
    }

    #[test]
    fn legacy_voice_section_accepts_warm_up() {
        let config = ui_config_from("[voice]\nwarm_up = false\n");
        assert!(!config.ai.warm_up);
    }

    #[test]
    fn config_without_legacy_section_is_untouched() {
        let mut config = table("[ai]\nmodel = \"x\"\n");
        assert_eq!(normalize_legacy_section(&mut config), None);
        assert_eq!(config, table("[ai]\nmodel = \"x\"\n"));
    }
}
