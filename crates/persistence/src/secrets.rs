//! Encrypted storage for sensitive per-download data.
//!
//! Cookies, credentials and authorization headers are sealed with
//! ChaCha20-Poly1305 under a 256-bit master key. The master key is generated
//! on first use and kept in the operating system's credential store
//! (Windows Credential Manager, macOS Keychain, Secret Service on Linux) by
//! the desktop app; this module never sees where it lives.
//!
//! If no key is available (e.g. no keyring on a headless Linux box), the
//! engine keeps secrets in memory only and they are not persisted – the
//! download then needs fresh cookies after a restart. Secrets are never
//! written to disk in plaintext.

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use rand::RngCore;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use velox_types::{Credentials, HeaderPair};

use crate::{Database, DbError, DbResult};

/// Sensitive data attached to a download.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DownloadSecrets {
    /// `Cookie` header value.
    pub cookies: Option<String>,
    pub credentials: Option<Credentials>,
    /// Sensitive headers (Authorization, Proxy-Authorization, ...).
    pub headers: Vec<HeaderPair>,
}

impl DownloadSecrets {
    pub fn is_empty(&self) -> bool {
        self.cookies.as_deref().is_none_or(str::is_empty)
            && self.credentials.is_none()
            && self.headers.is_empty()
    }
}

impl std::fmt::Debug for DownloadSecrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DownloadSecrets")
            .field("cookies", &self.cookies.as_ref().map(|_| "<redacted>"))
            .field("credentials", &self.credentials)
            .field(
                "headers",
                &self.headers.iter().map(|h| &h.name).collect::<Vec<_>>(),
            )
            .finish()
    }
}

/// Header names whose values are treated as secrets.
pub fn is_sensitive_header(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "authorization" | "proxy-authorization" | "cookie" | "x-api-key" | "x-auth-token"
    )
}

/// Symmetric sealing with a master key.
#[derive(Clone)]
pub struct SecretBox {
    cipher: ChaCha20Poly1305,
}

impl std::fmt::Debug for SecretBox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretBox(<key>)")
    }
}

impl SecretBox {
    pub fn new(key: &[u8; 32]) -> Self {
        Self {
            cipher: ChaCha20Poly1305::new(Key::from_slice(key)),
        }
    }

    /// Fresh random 256-bit key.
    pub fn generate_key() -> [u8; 32] {
        let mut k = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut k);
        k
    }

    /// Encrypt `plaintext`, binding it to `aad` (the record id) so sealed
    /// blobs cannot be swapped between records.
    pub fn seal(&self, aad: &[u8], plaintext: &[u8]) -> DbResult<([u8; 12], Vec<u8>)> {
        let mut nonce = [0u8; 12];
        rand::rngs::OsRng.fill_bytes(&mut nonce);
        let ct = self
            .cipher
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: plaintext,
                    aad,
                },
            )
            .map_err(|_| DbError::Secret("encryption failed".into()))?;
        Ok((nonce, ct))
    }

    pub fn open(&self, aad: &[u8], nonce: &[u8], ciphertext: &[u8]) -> DbResult<Vec<u8>> {
        if nonce.len() != 12 {
            return Err(DbError::Secret("bad nonce".into()));
        }
        self.cipher
            .decrypt(
                Nonce::from_slice(nonce),
                Payload {
                    msg: ciphertext,
                    aad,
                },
            )
            .map_err(|_| DbError::Secret("decryption failed (wrong key or tampered data)".into()))
    }
}

impl Database {
    /// Encrypt and store secrets under `secret_id`.
    pub fn put_secrets(
        &self,
        sb: &SecretBox,
        secret_id: &str,
        secrets: &DownloadSecrets,
    ) -> DbResult<()> {
        let plain = serde_json::to_vec(secrets)?;
        let (nonce, ct) = sb.seal(secret_id.as_bytes(), &plain)?;
        self.with(|c| {
            c.execute(
                "INSERT INTO secrets (id, nonce, ciphertext, created_at) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(id) DO UPDATE SET nonce=excluded.nonce, ciphertext=excluded.ciphertext",
                params![secret_id, nonce.to_vec(), ct, velox_types::now_ms()],
            )?;
            Ok(())
        })
    }

    pub fn get_secrets(
        &self,
        sb: &SecretBox,
        secret_id: &str,
    ) -> DbResult<Option<DownloadSecrets>> {
        let row: Option<(Vec<u8>, Vec<u8>)> = self.with(|c| {
            Ok(c.query_row(
                "SELECT nonce, ciphertext FROM secrets WHERE id = ?1",
                [secret_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?)
        })?;
        match row {
            None => Ok(None),
            Some((nonce, ct)) => {
                let plain = sb.open(secret_id.as_bytes(), &nonce, &ct)?;
                Ok(Some(serde_json::from_slice(&plain)?))
            }
        }
    }

    pub fn delete_secrets(&self, secret_id: &str) -> DbResult<()> {
        self.with(|c| {
            c.execute("DELETE FROM secrets WHERE id = ?1", [secret_id])?;
            Ok(())
        })
    }

    /// Store an application-level secret (e.g. the proxy password).
    pub fn put_app_secret(&self, sb: &SecretBox, name: &str, value: &str) -> DbResult<()> {
        let s = DownloadSecrets {
            credentials: Some(Credentials {
                username: name.into(),
                password: value.into(),
            }),
            ..Default::default()
        };
        self.put_secrets(sb, &format!("app:{name}"), &s)
    }

    pub fn get_app_secret(&self, sb: &SecretBox, name: &str) -> DbResult<Option<String>> {
        Ok(self
            .get_secrets(sb, &format!("app:{name}"))?
            .and_then(|s| s.credentials)
            .map(|c| c.password))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seal_roundtrip_and_tamper_detection() {
        let db = Database::open_in_memory().unwrap();
        let sb = SecretBox::new(&SecretBox::generate_key());
        let secrets = DownloadSecrets {
            cookies: Some("session=abc".into()),
            credentials: Some(Credentials {
                username: "u".into(),
                password: "p".into(),
            }),
            headers: vec![HeaderPair {
                name: "Authorization".into(),
                value: "Bearer x".into(),
            }],
        };
        db.put_secrets(&sb, "s1", &secrets).unwrap();
        assert_eq!(db.get_secrets(&sb, "s1").unwrap().unwrap(), secrets);

        // Ciphertext does not contain plaintext.
        let raw: Vec<u8> = db
            .with(|c| {
                Ok(
                    c.query_row("SELECT ciphertext FROM secrets WHERE id='s1'", [], |r| {
                        r.get(0)
                    })?,
                )
            })
            .unwrap();
        assert!(!raw.windows(11).any(|w| w == b"session=abc"));

        // Wrong key fails.
        let other = SecretBox::new(&SecretBox::generate_key());
        assert!(db.get_secrets(&other, "s1").is_err());

        // Moving the blob to another id fails (AAD binding).
        db.with(|c| {
            c.execute("UPDATE secrets SET id='s2' WHERE id='s1'", [])?;
            Ok(())
        })
        .unwrap();
        assert!(db.get_secrets(&sb, "s2").is_err());
    }

    #[test]
    fn app_secret() {
        let db = Database::open_in_memory().unwrap();
        let sb = SecretBox::new(&SecretBox::generate_key());
        db.put_app_secret(&sb, "proxy", "pw").unwrap();
        assert_eq!(
            db.get_app_secret(&sb, "proxy").unwrap().as_deref(),
            Some("pw")
        );
        assert!(format!(
            "{:?}",
            DownloadSecrets {
                cookies: Some("x".into()),
                ..Default::default()
            }
        )
        .contains("redacted"));
        assert!(is_sensitive_header("Authorization"));
    }
}
