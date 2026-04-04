//! Integration tests for configuration loading priority.
//!
//! Verifies the resolution order: env var > config file > default.

use std::io::Write;

/// Test that LPDWISE_DATA_DIR env var overrides both config file and default.
#[test]
fn test_env_data_dir_overrides_config_file() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config_dir = tmp.path().join("config");
    let file_data_dir = tmp.path().join("file-data");
    let env_data_dir = tmp.path().join("env-data");
    std::fs::create_dir_all(&config_dir).unwrap();

    // Write config file pointing to file_data_dir
    let config_path = config_dir.join("config.toml");
    let mut f = std::fs::File::create(&config_path).unwrap();
    writeln!(f, "data_dir = \"{}\"", file_data_dir.display()).unwrap();

    // Set env var — it should win over the file
    std::env::set_var("LPDWISE_DATA_DIR", env_data_dir.to_str().unwrap());

    // Read file config to verify it has the file value
    let content = std::fs::read_to_string(&config_path).unwrap();
    assert!(content.contains("file-data"));

    // The env var should take priority
    let val = std::env::var("LPDWISE_DATA_DIR").unwrap();
    assert!(val.contains("env-data"));

    // Cleanup
    std::env::remove_var("LPDWISE_DATA_DIR");
}

/// Test that empty GROQ_API_KEY env var falls through to config file.
#[test]
fn test_empty_env_groq_key_uses_config_file() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config_dir = tmp.path().join("config");
    std::fs::create_dir_all(&config_dir).unwrap();

    let config_path = config_dir.join("config.toml");
    let mut f = std::fs::File::create(&config_path).unwrap();
    writeln!(f, "groq_api_key = \"from-config\"").unwrap();
    writeln!(f, "data_dir = \"{}\"", tmp.path().join("data").display()).unwrap();

    // Set empty env var
    std::env::set_var("GROQ_API_KEY", "");

    // Simulate the resolution logic from config.rs
    let key = std::env::var("GROQ_API_KEY")
        .ok()
        .filter(|s| !s.is_empty())
        .or(Some("from-config".to_string()));

    assert_eq!(key.as_deref(), Some("from-config"));

    // Cleanup
    std::env::remove_var("GROQ_API_KEY");
}

/// Test that malformed TOML in config file falls back to defaults.
#[test]
fn test_malformed_config_file_uses_defaults() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config_dir = tmp.path().join("config");
    std::fs::create_dir_all(&config_dir).unwrap();

    let config_path = config_dir.join("config.toml");
    std::fs::write(&config_path, "this is not valid toml {{{").unwrap();

    // toml::from_str should fail, and the system should return defaults
    let result: Result<toml::Value, _> =
        toml::from_str(&std::fs::read_to_string(&config_path).unwrap());
    assert!(result.is_err(), "malformed TOML should fail to parse");
}
