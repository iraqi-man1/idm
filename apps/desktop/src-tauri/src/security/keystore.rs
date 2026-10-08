//! The 256-bit master key that encrypts cookies, credentials and the proxy
//! password is stored in the operating system's credential store:
//! Windows Credential Manager, macOS Keychain or the Secret Service on Linux.
//!
//! When no credential store is reachable (e.g. a headless Linux session),
//! `None` is returned and the engine keeps secrets in memory only.

use std::sync::mpsc;
use std::time::Duration;

const SERVICE: &str = "com.veloxdm.app";
const ACCOUNT: &str = "secrets-master-key-v1";

fn load_or_create_blocking() -> Result<[u8; 32], String> {
    let entry = keyring::Entry::new(SERVICE, ACCOUNT).map_err(|e| e.to_string())?;
    match entry.get_password() {
        Ok(hexkey) => {
            let bytes =
                hex::decode(hexkey.trim()).map_err(|_| "stored key is not hex".to_string())?;
            bytes
                .try_into()
                .map_err(|_| "stored key has the wrong length".to_string())
        }
        Err(keyring::Error::NoEntry) => {
            let key = velox_persistence::SecretBox::generate_key();
            entry
                .set_password(&hex::encode(key))
                .map_err(|e| e.to_string())?;
            // Read back to make sure the store really persisted it.
            let check = entry.get_password().map_err(|e| e.to_string())?;
            if check.trim() != hex::encode(key) {
                return Err("credential store did not persist the key".into());
            }
            Ok(key)
        }
        Err(e) => Err(e.to_string()),
    }
}

/// Load the master key, creating it on first run. Runs on its own thread
/// with a timeout because some keyrings show an unlock prompt.
pub fn load_master_key() -> Option<[u8; 32]> {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("velox-keystore".into())
        .spawn(move || {
            let _ = tx.send(load_or_create_blocking());
        })
        .ok()?;
    match rx.recv_timeout(Duration::from_secs(60)) {
        Ok(Ok(k)) => Some(k),
        Ok(Err(e)) => {
            tracing::warn!(error = %e, "OS credential store unavailable; secrets will not be persisted");
            None
        }
        Err(_) => {
            tracing::warn!(
                "timed out waiting for the OS credential store; secrets will not be persisted"
            );
            None
        }
    }
}
