use serde::Serialize;
use alacritty_config_derive::ConfigDeserialize;

#[derive(ConfigDeserialize, Serialize, Debug, Clone, PartialEq)]
pub struct Voice {
    pub llm_api_url: String,
    pub whisper_model_path: String,
    pub model: String,
    pub temperature: f32,
    pub timeout_secs: u64,
}

impl Default for Voice {
    fn default() -> Self {
        Self {
            llm_api_url: "http://localhost:11434/api/generate".to_string(),
            whisper_model_path: "/path/to/your/whisper/model.bin".to_string(),
            model: "qwen2.5-coder:3b".to_string(),
            temperature: 0.0,
            timeout_secs: 15,
        }
    }
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
}

