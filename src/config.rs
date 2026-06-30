use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::env;
use std::fs::{create_dir_all, read_to_string, remove_file, write};

/// Structure representing the application's configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub idle_timeout_minutes: u64,
    pub auto_pause_after_notification_seconds: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            idle_timeout_minutes: 5,
            auto_pause_after_notification_seconds: 30,
        }
    }
}

/// Loads the config from `~/.config/worktimer/config.toml`, creating it with defaults if absent.
pub fn load() -> Result<Config> {
    let mut config_dir = dirs::config_dir()
        .or_else(|| dirs::home_dir().map(|h| h.join(".config")))
        .context("Could not determine configuration directory")?;

    config_dir.push("worktimer");
    create_dir_all(&config_dir).context("Failed to create config directory")?;

    let config_path = config_dir.join("config.toml");
    if !config_path.exists() {
        let default_config = Config::default();
        let toml_str = toml::to_string_pretty(&default_config)
            .context("Failed to serialize default configuration")?;
        write(&config_path, toml_str)
            .with_context(|| format!("Failed to write default config to {:?}", config_path))?;
        return Ok(default_config);
    }

    let toml_str = read_to_string(&config_path)
        .with_context(|| format!("Failed to read config file from {:?}", config_path))?;

    let config: Config = toml::from_str(&toml_str)
        .with_context(|| format!("Failed to parse config file at {:?}", config_path))?;

    Ok(config)
}

/// Creates a desktop entry in `~/.local/share/applications/` so that the app appears in
/// "All Applications". Also ensures any old autostart file is deleted.
pub fn handle_desktop_integration() -> Result<()> {
    // 1. Remove old autostart file if it exists
    if let Some(mut autostart_dir) =
        dirs::config_dir().or_else(|| dirs::home_dir().map(|h| h.join(".config")))
    {
        autostart_dir.push("autostart");
        let autostart_file_path = autostart_dir.join("worktimer.desktop");
        if autostart_file_path.exists() {
            remove_file(&autostart_file_path).with_context(|| {
                format!("Failed to remove autostart file: {:?}", autostart_file_path)
            })?;
            tracing::info!("Autostart file removed from {:?}", autostart_file_path);
        }
    }

    // 2. Create the launcher desktop entry
    let mut apps_dir = dirs::data_dir()
        .or_else(|| dirs::home_dir().map(|h| h.join(".local/share")))
        .context("Could not determine local data directory")?;
    apps_dir.push("applications");
    create_dir_all(&apps_dir).context("Failed to create applications directory")?;

    let launcher_file_path = apps_dir.join("worktimer.desktop");
    let current_exe = env::current_exe().context("Failed to get current executable path")?;
    let current_exe_str = current_exe
        .to_str()
        .context("Executable path is not valid UTF-8")?;

    let desktop_content = format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=WorkTimer\n\
         Comment=Lightweight work timer\n\
         Exec={}\n\
         Icon=media-playback-start\n\
         Terminal=false\n\
         Categories=Utility;\n",
        current_exe_str
    );

    write(&launcher_file_path, desktop_content).with_context(|| {
        format!(
            "Failed to write launcher desktop file: {:?}",
            launcher_file_path
        )
    })?;
    tracing::info!(
        "Launcher desktop file created/updated at {:?}",
        launcher_file_path
    );

    Ok(())
}
