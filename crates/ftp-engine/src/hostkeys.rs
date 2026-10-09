//! SSH host key verification.
//!
//! 1. `~/.ssh/known_hosts` is authoritative when it knows the host.
//! 2. Otherwise the application's own known-hosts file is used: a host seen
//!    for the first time is recorded (trust on first use, like OpenSSH's
//!    `StrictHostKeyChecking=accept-new`); afterwards its key must match.
//! 3. A key that differs from a recorded one is always refused.

use std::path::Path;

use russh::keys::known_hosts::{check_known_hosts_path, learn_known_hosts_path};
use russh::keys::{HashAlg, PublicKey};

/// Outcome of a successful check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Trust {
    /// Listed in `~/.ssh/known_hosts` or the application file.
    Known,
    /// First connection; the key was recorded in the application file.
    Learned,
}

pub fn fingerprint(key: &PublicKey) -> String {
    key.fingerprint(HashAlg::Sha256).to_string()
}

fn changed(host: &str, key: &PublicKey, file: &Path, line: usize) -> String {
    format!(
        "the SSH host key of {host} changed ({}; recorded in {} line {line}). \
         This can mean someone is intercepting the connection. If the server's key \
         really changed, remove the old entry and try again.",
        fingerprint(key),
        file.display()
    )
}

/// Check `key` for `host:port`; `Err` carries a message for the user.
pub fn verify(
    host: &str,
    port: u16,
    key: &PublicKey,
    system: Option<&Path>,
    app: Option<&Path>,
) -> Result<Trust, String> {
    use russh::keys::Error;
    for file in [system, app].into_iter().flatten() {
        match check_known_hosts_path(host, port, key, file) {
            Ok(true) => return Ok(Trust::Known),
            Ok(false) => {}
            Err(Error::KeyChanged { line }) => return Err(changed(host, key, file, line)),
            // Missing or unreadable file: nothing recorded there.
            Err(_) => {}
        }
    }
    match app {
        Some(file) => {
            learn_known_hosts_path(host, port, key, file)
                .map_err(|e| format!("cannot record the SSH host key: {e}"))?;
            tracing::warn!(host, port, fingerprint = %fingerprint(key), "trusting new SSH host key");
            Ok(Trust::Learned)
        }
        None => Err(format!(
            "unknown SSH host key for {host} ({}); add it to ~/.ssh/known_hosts",
            fingerprint(key)
        )),
    }
}

/// `~/.ssh/known_hosts`.
pub fn system_known_hosts() -> Option<std::path::PathBuf> {
    dirs::home_dir().map(|h| h.join(".ssh").join("known_hosts"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use russh::keys::{Algorithm, PrivateKey};

    fn new_key() -> PublicKey {
        PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519)
            .unwrap()
            .public_key()
            .clone()
    }

    #[test]
    fn trust_on_first_use_then_strict() {
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path().join("known_hosts");
        let a = new_key();
        assert_eq!(
            verify("h.example", 22, &a, None, Some(&app)),
            Ok(Trust::Learned)
        );
        assert_eq!(
            verify("h.example", 22, &a, None, Some(&app)),
            Ok(Trust::Known)
        );
        // Another port is another host entry.
        assert_eq!(
            verify("h.example", 2222, &a, None, Some(&app)),
            Ok(Trust::Learned)
        );
        // A different key for a recorded host is refused.
        let b = new_key();
        assert!(verify("h.example", 22, &b, None, Some(&app))
            .unwrap_err()
            .contains("changed"));
        // ~/.ssh/known_hosts takes precedence.
        let system = dir.path().join("system_known_hosts");
        learn_known_hosts_path("h.example", 22, &b, &system).unwrap();
        assert_eq!(
            verify("h.example", 22, &b, Some(&system), Some(&app)),
            Ok(Trust::Known)
        );
        // Unknown host without an application file is refused.
        assert!(verify("other.example", 22, &a, None, None)
            .unwrap_err()
            .contains("unknown SSH host key"));
    }
}
