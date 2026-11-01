use hmac::{Hmac, Mac};
use sha1::Sha1;

type HmacSha1 = Hmac<Sha1>;

/// TOTP as used by MyAAC `TokenAuth6238::verify`: SHA1, 6 digits, window ±3 steps of 30s.
pub fn verify(secret: &str, code: &str) -> bool {
    let Some(key) = decode_secret(secret) else {
        return false;
    };
    let Ok(presented) = code.trim().parse::<u32>() else {
        return false;
    };
    let now = chrono::Utc::now().timestamp().div_euclid(30);
    for delta in -3i64..=3 {
        let counter = now + delta;
        if counter < 0 {
            continue;
        }
        if hotp(&key, counter as u64) == presented {
            return true;
        }
    }
    false
}

fn decode_secret(secret: &str) -> Option<Vec<u8>> {
    let cleaned: String = secret
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '=')
        .collect();
    if cleaned.is_empty() {
        return None;
    }
    let upper = cleaned.to_ascii_uppercase();
    base32::decode(base32::Alphabet::Rfc4648 { padding: false }, &upper)
}

fn hotp(key: &[u8], counter: u64) -> u32 {
    let mut mac = HmacSha1::new_from_slice(key).expect("hmac accepts any key length");
    mac.update(&counter.to_be_bytes());
    let bytes = mac.finalize().into_bytes();
    let offset = (bytes[19] & 0x0f) as usize;
    let bin = u32::from(bytes[offset] & 0x7f) << 24
        | u32::from(bytes[offset + 1]) << 16
        | u32::from(bytes[offset + 2]) << 8
        | u32::from(bytes[offset + 3]);
    bin % 1_000_000
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc6238_sha1_time_59() {
        let key = decode_secret("GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ").expect("secret");
        assert_eq!(hotp(&key, 59 / 30), 287_082);
    }

    #[test]
    fn rfc6238_sha1_time_1111111109() {
        let key = decode_secret("GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ").expect("secret");
        assert_eq!(hotp(&key, 1_111_111_109 / 30), 81_804);
    }

    #[test]
    fn verify_accepts_current_code() {
        let secret = "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ";
        let key = decode_secret(secret).expect("secret");
        let counter = chrono::Utc::now().timestamp().div_euclid(30) as u64;
        let code = format!("{:06}", hotp(&key, counter));
        assert!(verify(secret, &code));
        assert!(!verify(secret, "abcdef"));
    }
}
