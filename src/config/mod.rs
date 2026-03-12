use serde::{Deserialize, Serialize};
use std::fs;
use anyhow::Result;
use crate::core::Account;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum TransportType {
    Udp,
    Tcp,
}

impl Default for TransportType {
    fn default() -> Self {
        TransportType::Udp
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionSettings {
    pub bind_address: String,
    pub transport_type: TransportType,
}

impl Default for ConnectionSettings {
    fn default() -> Self {
        Self {
            bind_address: "0.0.0.0:5060".to_string(),
            transport_type: TransportType::Udp,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioSettings {
    pub input_device: Option<String>,
    pub output_device: Option<String>,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            input_device: None,
            output_device: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub accounts: Vec<Account>,
    #[serde(default)]
    pub connection: ConnectionSettings,
    #[serde(default)]
    pub audio: AudioSettings,
}

impl Config {
    pub fn load_from_file(path: &str) -> Result<Self> {
        let content = fs::read_to_string(path)?;
        let config: Config = toml::from_str(&content)?;
        Ok(config)
    }

    pub fn save_to_file(&self, path: &str) -> Result<()> {
        let content = toml::to_string(self)?;
        fs::write(path, content)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_deserialization_defaults() {
        let toml_str = "[[accounts]]\nname = \"test\"\nusername = \"user\"\ndomain = \"127.0.0.1\"";
        let config: Config = toml::from_str(toml_str).expect("Should parse with defaults");
        assert_eq!(config.accounts.len(), 1);
        assert_eq!(config.connection.bind_address, "0.0.0.0:5060");
        assert!(config.audio.input_device.is_none());
    }
}
