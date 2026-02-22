use std::path::PathBuf;

pub struct MqttConfig {
    pub port: u16,
    pub client_id: String,
    pub config_file: PathBuf,
    pub password_file: PathBuf,
}

impl MqttConfig {
    fn from_env(config_dir: &PathBuf) -> Self {
        let port: u16 = std::env::var("MQTT_PORT")
            .unwrap_or_else(|_| "1883".to_string())
            .parse()
            .unwrap_or(1883);
        let client_id = std::env::var("MQTT_CLIENT_ID")
            .unwrap_or_else(|_| "placenet-home".to_string());
        let config_file = config_dir.join("mosquitto.conf");
        let password_file = config_dir.join("mosquitto.passwd");
        Self { port, client_id, config_file, password_file }
    }
}

pub struct Config {
    pub mqtt: MqttConfig,
    pub config_dir: PathBuf,
}

impl Config {
    /// Load configuration from environment variables, falling back to defaults.
    pub fn from_env() -> Self {
        let config_dir = PathBuf::from(
            std::env::var("PLACENET_CONFIG_DIR").unwrap_or_else(|_| "config".to_string()),
        );
        let mqtt = MqttConfig::from_env(&config_dir);
        Self { mqtt, config_dir }
    }
}
