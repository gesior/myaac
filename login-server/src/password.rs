use base64::Engine;
use md5::Md5;
use sha1::{Digest, Sha1};
use sha2::{Sha256, Sha512};
use subtle::ConstantTimeEq;

use crate::config::{Algorithm, PasswordConfig};

/// Hash a password the same way MyAAC `encrypt()` does.
///
/// Account salt (column `salt`, when present) is prepended. `database_salt` is appended.
/// `vahash` is base64 of the hex SHA-256 string, not of the raw digest.
pub fn hash_password(account_salt: &str, password: &str, cfg: &PasswordConfig) -> String {
    let mut material =
        String::with_capacity(account_salt.len() + password.len() + cfg.database_salt.len());
    material.push_str(account_salt);
    material.push_str(password);
    material.push_str(&cfg.database_salt);
    match cfg.algorithm {
        Algorithm::Plain => material,
        Algorithm::Sha1 => hex::encode(Sha1::digest(material.as_bytes())),
        Algorithm::Md5 => hex::encode(Md5::digest(material.as_bytes())),
        Algorithm::Sha256 => hex::encode(Sha256::digest(material.as_bytes())),
        Algorithm::Sha512 => hex::encode(Sha512::digest(material.as_bytes())),
        Algorithm::Vahash => {
            let hexed = hex::encode(Sha256::digest(material.as_bytes()));
            base64::engine::general_purpose::STANDARD.encode(hexed.as_bytes())
        }
    }
}

pub fn hashes_equal(calculated: &str, stored: &str) -> bool {
    if calculated.len() != stored.len() {
        return false;
    }
    calculated.as_bytes().ct_eq(stored.as_bytes()).unwrap_u8() == 1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(algorithm: Algorithm, database_salt: &str) -> PasswordConfig {
        PasswordConfig {
            algorithm,
            database_salt: database_salt.to_string(),
        }
    }

    #[test]
    fn sha1_of_secret() {
        let hashed = hash_password("", "secret", &cfg(Algorithm::Sha1, ""));
        assert_eq!(hashed, "e5e9fa1ba31ecd1ae84f75caaa474f3a663f05f4");
    }

    #[test]
    fn md5_of_secret() {
        let hashed = hash_password("", "secret", &cfg(Algorithm::Md5, ""));
        assert_eq!(hashed, "5ebe2294ecd0e0f08eab7690d2a6ee69");
    }

    #[test]
    fn salt_is_prefix_and_database_salt_is_suffix() {
        let hashed = hash_password("ab", "secret", &cfg(Algorithm::Sha1, "Z"));
        let expect = hex::encode(Sha1::digest(b"absecretZ"));
        assert_eq!(hashed, expect);
    }

    #[test]
    fn vahash_is_base64_of_hex_sha256() {
        let hashed = hash_password("", "abc", &cfg(Algorithm::Vahash, ""));
        let hexed = hex::encode(Sha256::digest(b"abc"));
        let expect = base64::engine::general_purpose::STANDARD.encode(hexed.as_bytes());
        assert_eq!(hashed, expect);
    }

    #[test]
    fn constant_time_compare() {
        assert!(hashes_equal("abc", "abc"));
        assert!(!hashes_equal("abc", "abd"));
        assert!(!hashes_equal("abc", "abcd"));
    }
}
