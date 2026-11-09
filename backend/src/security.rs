use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use base64::Engine;
use hkdf::Hkdf;
use rand::RngCore;
use sha2::{Digest, Sha256};

#[derive(Clone)]
pub struct Vault {
    key: [u8; 32],
    key_id: String,
    previous: Option<([u8; 32], String)>,
}

impl Vault {
    pub fn new(
        master: &str,
        key_id: &str,
        previous_master: Option<&str>,
        previous_id: Option<&str>,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            key: derive_key(master)?,
            key_id: key_id.to_string(),
            previous: match (previous_master, previous_id) {
                (Some(k), Some(id)) => Some((derive_key(k)?, id.to_string())),
                _ => None,
            },
        })
    }

    pub fn key_id(&self) -> &str {
        &self.key_id
    }

    pub fn encrypt(&self, plaintext: &[u8], aad: &str) -> anyhow::Result<SecretBlob> {
        let cipher = Aes256Gcm::new_from_slice(&self.key).map_err(|e| anyhow::anyhow!(e))?;
        let mut nonce_bytes = [0u8; 12];
        rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = cipher
            .encrypt(
                nonce,
                aes_gcm::aead::Payload { msg: plaintext, aad: aad.as_bytes() },
            )
            .map_err(|_| anyhow::anyhow!("encryption failed"))?;
        Ok(SecretBlob {
            ciphertext,
            nonce: nonce_bytes.to_vec(),
            key_id: self.key_id.clone(),
            aad: aad.to_string(),
        })
    }

    pub fn decrypt(&self, blob: &SecretBlob) -> anyhow::Result<Vec<u8>> {
        let key = if blob.key_id == self.key_id {
            self.key
        } else if let Some((key, id)) = &self.previous {
            if blob.key_id == *id {
                *key
            } else {
                anyhow::bail!("unknown encryption key id");
            }
        } else {
            anyhow::bail!("unknown encryption key id");
        };
        let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| anyhow::anyhow!(e))?;
        cipher
            .decrypt(
                Nonce::from_slice(&blob.nonce),
                aes_gcm::aead::Payload {
                    msg: &blob.ciphertext,
                    aad: blob.aad.as_bytes(),
                },
            )
            .map_err(|_| anyhow::anyhow!("decryption failed"))
    }
}

#[derive(Clone, Debug)]
pub struct SecretBlob {
    pub ciphertext: Vec<u8>,
    pub nonce: Vec<u8>,
    pub key_id: String,
    pub aad: String,
}

fn derive_key(master: &str) -> anyhow::Result<[u8; 32]> {
    let material = if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(master.trim()) {
        if bytes.len() == 32 {
            bytes
        } else {
            master.as_bytes().to_vec()
        }
    } else {
        master.as_bytes().to_vec()
    };
    let hk = Hkdf::<Sha256>::new(Some(b"pommarket-credential-vault"), &material);
    let mut okm = [0u8; 32];
    hk.expand(b"aes-256-gcm", &mut okm)
        .map_err(|_| anyhow::anyhow!("hkdf expand failed"))?;
    Ok(okm)
}

pub fn hash_password(password: &str) -> anyhow::Result<String> {
    if password.len() < 10 {
        anyhow::bail!("password must be at least 10 characters");
    }
    let salt = SaltString::generate(&mut rand::rngs::OsRng);
    let hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    Ok(hash.to_string())
}

pub fn verify_password(password: &str, hash: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(hash) else {
        return false;
    };
    Argon2::default().verify_password(password.as_bytes(), &parsed).is_ok()
}

pub fn sha256_hex(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    hex::encode(digest)
}

pub fn random_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

pub fn credential_aad(user_id: uuid::Uuid, account_id: uuid::Uuid) -> String {
    format!("pommarket:v1:{user_id}:{account_id}:trading-credentials")
}

/// Remove material that looks like a private key or API secret from text that may reach logs.
pub fn redact(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '0' && i + 1 < chars.len() && chars[i + 1] == 'x' {
            let start = i;
            i += 2;
            let hex_start = i;
            while i < chars.len() && chars[i].is_ascii_hexdigit() {
                i += 1;
            }
            let hex_len = i - hex_start;
            if hex_len >= 32 {
                out.push_str("0x***");
            } else {
                out.extend(chars[start..i].iter());
            }
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_aad_binding() {
        let vault = Vault::new("master-key-material-for-tests", "k1", None, None).unwrap();
        let aad = "user:acct";
        let blob = vault.encrypt(b"super-secret-key", aad).unwrap();
        assert_eq!(vault.decrypt(&blob).unwrap(), b"super-secret-key");
        let mut wrong = blob.clone();
        wrong.aad = "other".into();
        assert!(vault.decrypt(&wrong).is_err());
    }

    #[test]
    fn redacts_long_hex() {
        let raw = "key 0x0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef end";
        let clean = redact(raw);
        assert!(!clean.contains("0123456789abcdef"));
        assert!(clean.contains("0x***"));
    }

    #[test]
    fn password_round_trip() {
        let hash = hash_password("correct horse battery").unwrap();
        assert!(verify_password("correct horse battery", &hash));
        assert!(!verify_password("wrong horse battery", &hash));
    }
}
