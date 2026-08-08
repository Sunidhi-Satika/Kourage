use serde::Serialize;
use alacritty_config_derive::ConfigDeserialize;

#[derive(ConfigDeserialize, Serialize, Debug, Clone, PartialEq, Eq)]
pub struct Voice {
    #[config(default = "http://localhost:11434/api/generate")]
    pub llm_api_url: String,
    #[config(default = "/path/to/your/whisper/model.bin")]
    pub whisper_model_path: String,
}

impl Default for Voice {
    fn default() -> Self {
        Self {
            llm_api_url: "http://localhost:11434/api/generate".to_string(),
            whisper_model_path: "/path/to/your/whisper/model.bin".to_string(),
        }
    }
}
