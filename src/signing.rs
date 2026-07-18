use hmac::{Hmac, Mac};
use rand_core::{OsRng, RngCore};
use sha2::Sha256;
use thiserror::Error;

type HmacSha256 = Hmac<Sha256>;

pub fn generate_secret() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    format!("whsec_{}", hex::encode(bytes))
}

#[derive(Debug, Error)]
pub enum SignError {
    #[error("unable to initialize HMAC-SHA256")]
    Hmac,
}

/// En-tête `X-Webhook-Signature` : `t=<unix>,v1=<hex>`.
/// Le MAC couvre `"{timestamp}.{body}"` avec le secret de l'endpoint.
pub fn signature_header(secret: &str, timestamp: i64, body: &[u8]) -> Result<String, SignError> {
    let digest = sign(secret, timestamp, body)?;
    Ok(format!("t={timestamp},v1={digest}"))
}

pub fn sign(secret: &str, timestamp: i64, body: &[u8]) -> Result<String, SignError> {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).map_err(|_| SignError::Hmac)?;
    mac.update(timestamp.to_string().as_bytes());
    mac.update(b".");
    mac.update(body);
    Ok(hex::encode(mac.finalize().into_bytes()))
}

/// Vérifie la signature et rejette un timestamp trop éloigné de `now_unix`.
pub fn verify_signature(
    secret: &str,
    body: &[u8],
    header: &str,
    now_unix: i64,
    tolerance_secs: i64,
) -> bool {
    let Some((timestamp, presented)) = parse_header(header) else {
        return false;
    };
    let Ok(expected) = sign(secret, timestamp, body) else {
        return false;
    };
    let fresh = (now_unix - timestamp).abs() <= tolerance_secs;
    fresh && constant_time_eq(expected.as_bytes(), presented.as_bytes())
}

fn parse_header(header: &str) -> Option<(i64, &str)> {
    let mut timestamp = None;
    let mut signature = None;
    for part in header.split(',') {
        let part = part.trim();
        if let Some(value) = part.strip_prefix("t=") {
            timestamp = value.parse::<i64>().ok();
        } else if let Some(value) = part.strip_prefix("v1=") {
            signature = Some(value);
        }
    }
    Some((timestamp?, signature?))
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut diff = 0u8;
    for (a, b) in left.iter().zip(right.iter()) {
        diff |= a ^ b;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::{sign, signature_header, verify_signature};

    const SECRET: &str = "whsec_test";
    const TIMESTAMP: i64 = 1_700_000_000;
    const BODY: &[u8] = br#"{"ok":true}"#;
    const DIGEST: &str = "85876387ad9d6be57a04653bc0729da757049f58afb10ba6cac3bedaecf4fda3";

    #[test]
    fn matches_the_known_hmac_vector() {
        assert_eq!(sign(SECRET, TIMESTAMP, BODY).unwrap(), DIGEST);
        assert_eq!(
            signature_header(SECRET, TIMESTAMP, BODY).unwrap(),
            format!("t={TIMESTAMP},v1={DIGEST}")
        );
    }

    #[test]
    fn body_or_secret_change_the_digest() {
        let original = sign(SECRET, TIMESTAMP, BODY).unwrap();
        assert_ne!(sign(SECRET, TIMESTAMP, b"{}").unwrap(), original);
        assert_ne!(sign("whsec_other", TIMESTAMP, BODY).unwrap(), original);
        assert_ne!(sign(SECRET, TIMESTAMP + 1, BODY).unwrap(), original);
    }

    #[test]
    fn verify_accepts_a_fresh_header_and_rejects_tampering() {
        let header = signature_header(SECRET, TIMESTAMP, BODY).unwrap();
        assert!(verify_signature(SECRET, BODY, &header, TIMESTAMP, 300));
        assert!(!verify_signature(SECRET, b"{}", &header, TIMESTAMP, 300));
        assert!(!verify_signature(
            "whsec_other",
            BODY,
            &header,
            TIMESTAMP,
            300
        ));
        assert!(!verify_signature(
            SECRET,
            BODY,
            &header,
            TIMESTAMP + 301,
            300
        ));
        assert!(!verify_signature(SECRET, BODY, "nope", TIMESTAMP, 300));
    }
}
