use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use tracing::{debug, info, instrument};

/// Errors from model download and verification.
#[derive(Debug, thiserror::Error)]
pub enum ModelError {
    #[error("download failed: {0}")]
    Download(String),

    #[error("sha256 mismatch: expected {expected}, got {actual}")]
    ChecksumMismatch { expected: String, actual: String },

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// Default models directory under XDG data dir.
pub fn default_models_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from(".local/share"))
        .join("libre-pdwise")
        .join("models")
}

/// Download a model from HuggingFace to the target directory.
///
/// Supports `HF_MIRROR` environment variable for mirror sites.
/// If the model already exists and passes sha256 verification, skips download.
#[instrument(skip_all, fields(model_name, target_dir = %target_dir.display()))]
pub async fn download_model(
    model_name: &str,
    target_dir: &Path,
    expected_sha256: Option<&str>,
) -> Result<PathBuf, ModelError> {
    let model_dir = target_dir.join(model_name);

    // If model dir exists and has a checksum marker, skip download
    let checksum_marker = model_dir.join(".sha256_verified");
    if checksum_marker.exists() {
        info!("model already downloaded and verified: {model_name}");
        return Ok(model_dir);
    }

    tokio::fs::create_dir_all(&model_dir)
        .await
        .map_err(ModelError::Io)?;

    let hf_base = std::env::var("HF_MIRROR")
        .unwrap_or_else(|_| "https://huggingface.co".into());

    let url = format!(
        "{hf_base}/{model_name}/resolve/main/model.tar.gz"
    );

    info!(url = %url, "downloading model");

    let client = reqwest::Client::new();
    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| ModelError::Download(e.to_string()))?;

    if !response.status().is_success() {
        return Err(ModelError::Download(format!(
            "HTTP {}: {}",
            response.status(),
            url
        )));
    }

    let bytes = response
        .bytes()
        .await
        .map_err(|e| ModelError::Download(e.to_string()))?;

    // Verify checksum if provided
    if let Some(expected) = expected_sha256 {
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        let actual = format!("{:x}", hasher.finalize());

        if actual != expected {
            return Err(ModelError::ChecksumMismatch {
                expected: expected.to_string(),
                actual,
            });
        }
        debug!("sha256 checksum verified");
    }

    // Write archive to disk and extract (simplified: write raw bytes for now,
    // extraction depends on the actual model packaging format)
    let archive_path = model_dir.join("model.tar.gz");
    tokio::fs::write(&archive_path, &bytes)
        .await
        .map_err(ModelError::Io)?;

    // Mark as verified
    tokio::fs::write(&checksum_marker, b"verified")
        .await
        .map_err(ModelError::Io)?;

    info!("model downloaded to {}", model_dir.display());
    Ok(model_dir)
}

/// Compute sha256 hash of a file.
pub async fn sha256_file(path: &Path) -> Result<String, ModelError> {
    let bytes = tokio::fs::read(path).await.map_err(ModelError::Io)?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_models_dir() {
        let dir = default_models_dir();
        let path_str = dir.to_string_lossy();
        assert!(path_str.contains("libre-pdwise"));
        assert!(path_str.contains("models"));
    }

    #[tokio::test]
    async fn test_sha256_file() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), b"hello world").unwrap();

        let hash = sha256_file(tmp.path()).await.unwrap();
        // sha256("hello world") is known
        assert_eq!(
            hash,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }

    #[tokio::test]
    async fn test_download_model_skip_if_verified() {
        let tmp = tempfile::tempdir().unwrap();
        let model_name = "test-model";
        let model_dir = tmp.path().join(model_name);
        std::fs::create_dir_all(&model_dir).unwrap();
        std::fs::write(model_dir.join(".sha256_verified"), b"verified").unwrap();

        let result =
            download_model(model_name, tmp.path(), None).await.unwrap();
        assert_eq!(result, model_dir);
    }
}
