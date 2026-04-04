use std::path::{Path, PathBuf};

/// Application configuration with XDG-compliant directory layout.
///
/// Field resolution priority: environment variable > config file > default.
#[derive(Debug, Clone)]
pub struct AppConfig {
    pub groq_api_key: Option<String>,
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub media_dir: PathBuf,
    pub archive_dir: PathBuf,
    pub models_dir: PathBuf,
    pub logs_dir: PathBuf,
}

/// Partial config deserialized from config.toml.
/// All fields optional — missing fields fall back to defaults.
#[derive(Debug, Default, serde::Deserialize)]
struct FileConfig {
    groq_api_key: Option<String>,
    data_dir: Option<PathBuf>,
}

/// Errors from configuration loading.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("failed to create directory {path}: {source}")]
    CreateDir {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("failed to read config file {path}: {source}")]
    ReadFile {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("failed to parse config file {path}: {source}")]
    ParseFile {
        path: PathBuf,
        source: toml::de::Error,
    },

    #[error("could not determine home directory")]
    NoHomeDir,
}

/// Load application configuration.
///
/// Resolution order for each field:
/// 1. Environment variable (highest priority)
/// 2. Config file (`~/.config/libre-pdwise/config.toml`)
/// 3. Built-in default
///
/// Automatically creates all data sub-directories.
pub fn load_config() -> Result<AppConfig, ConfigError> {
    let config_dir = resolve_config_dir()?;
    let file_config = read_config_file(&config_dir);

    let data_dir = resolve_data_dir(&file_config)?;

    let media_dir = data_dir.join("media");
    let archive_dir = data_dir.join("archive");
    let models_dir = data_dir.join("models");
    let logs_dir = data_dir.join("logs");

    // Env overrides config file for groq_api_key
    let groq_api_key = std::env::var("GROQ_API_KEY")
        .ok()
        .filter(|s| !s.is_empty())
        .or(file_config.groq_api_key);

    ensure_dirs(&[&media_dir, &archive_dir, &models_dir, &logs_dir])?;

    Ok(AppConfig {
        groq_api_key,
        config_dir,
        data_dir,
        media_dir,
        archive_dir,
        models_dir,
        logs_dir,
    })
}

fn resolve_config_dir() -> Result<PathBuf, ConfigError> {
    let base = dirs::config_dir().ok_or(ConfigError::NoHomeDir)?;
    Ok(base.join("libre-pdwise"))
}

fn resolve_data_dir(file_config: &FileConfig) -> Result<PathBuf, ConfigError> {
    // Env var takes priority, then config file, then XDG default
    if let Ok(val) = std::env::var("LPDWISE_DATA_DIR") {
        if !val.is_empty() {
            return Ok(PathBuf::from(val));
        }
    }
    if let Some(ref dir) = file_config.data_dir {
        return Ok(dir.clone());
    }
    let base = dirs::data_dir().ok_or(ConfigError::NoHomeDir)?;
    Ok(base.join("libre-pdwise"))
}

/// Read and parse the config file, returning defaults on any error.
fn read_config_file(config_dir: &Path) -> FileConfig {
    let path = config_dir.join("config.toml");
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return FileConfig::default(),
    };
    toml::from_str(&content).unwrap_or_default()
}

fn ensure_dirs(dirs: &[&PathBuf]) -> Result<(), ConfigError> {
    for dir in dirs {
        std::fs::create_dir_all(dir).map_err(|e| ConfigError::CreateDir {
            path: dir.to_path_buf(),
            source: e,
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Test that env var GROQ_API_KEY overrides the config file value.
    #[test]
    fn test_env_overrides_config_file_groq_key() {
        let tmp = tempfile::TempDir::new().unwrap();
        let config_dir = tmp.path().join("config");
        let data_dir = tmp.path().join("data");
        std::fs::create_dir_all(&config_dir).unwrap();

        // Write a config file with a groq key
        let config_path = config_dir.join("config.toml");
        let mut f = std::fs::File::create(&config_path).unwrap();
        writeln!(f, "groq_api_key = \"from-file\"").unwrap();
        writeln!(f, "data_dir = \"{}\"", data_dir.display()).unwrap();

        let file_config = read_config_file(&config_dir);
        assert_eq!(file_config.groq_api_key.as_deref(), Some("from-file"));

        // Simulate env override
        std::env::set_var("GROQ_API_KEY", "from-env");
        let key = std::env::var("GROQ_API_KEY")
            .ok()
            .filter(|s| !s.is_empty())
            .or(file_config.groq_api_key);
        assert_eq!(key.as_deref(), Some("from-env"));

        // Clean up env
        std::env::remove_var("GROQ_API_KEY");
    }

    /// Test that missing config file produces defaults without error.
    #[test]
    fn test_missing_config_file_returns_defaults() {
        let tmp = tempfile::TempDir::new().unwrap();
        let config_dir = tmp.path().join("nonexistent");
        let file_config = read_config_file(&config_dir);
        assert!(file_config.groq_api_key.is_none());
        assert!(file_config.data_dir.is_none());
    }

    /// Test that data sub-directories are created.
    #[test]
    fn test_ensure_dirs_creates_subdirectories() {
        let tmp = tempfile::TempDir::new().unwrap();
        let media = tmp.path().join("media");
        let archive = tmp.path().join("archive");
        ensure_dirs(&[&media, &archive]).unwrap();
        assert!(media.is_dir());
        assert!(archive.is_dir());
    }
}
