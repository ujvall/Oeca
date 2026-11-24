use rand_core::OsRng; // requires 'getrandom' feature

use crate::error::*;

// https://www.iana.org/assignments/tls-parameters/tls-parameters.xml#tls-parameters-8
#[repr(u16)]
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum NamedCurve {
    Unsupported = 0x0000,
    P256 = 0x0017,
    P384 = 0x0018,
    X25519 = 0x001d,
}

impl From<u16> for NamedCurve {
    fn from(val: u16) -> Self {
        match val {
            0x0017 => NamedCurve::P256,
            0x0018 => NamedCurve::P384,
            0x001d => NamedCurve::X25519,
            _ => NamedCurve::Unsupported,
        }
    }
}

pub(crate) enum NamedCurvePrivateKey {
    EphemeralSecretP256(p256::ecdh::EphemeralSecret),
    EphemeralSecretP384(p384::ecdh::EphemeralSecret),
    StaticSecretX25519(x25519_dalek::StaticSecret),
}

pub struct NamedCurveKeypair {
    pub(crate) curve: NamedCurve,
    pub(crate) public_key: Vec<u8>,
    pub(crate) private_key: NamedCurvePrivateKey,
}

/// oeca: the only key-exchange curve this copy uses. P-256 and P-384 key
/// exchange run through crypto-bigint (RUSTSEC-2026-0387, a timing advisory),
/// and either side could otherwise pick one: the client by listing it first,
/// the server by choosing it. Browsers and webrtc-rs all offer X25519.
pub const KEY_EXCHANGE_CURVES: [NamedCurve; 1] = [NamedCurve::X25519];

/// oeca: what this side lists in its hello. In DTLS 1.2 the list also names
/// the curves of the ECDSA certificates a peer may use, and every WebRTC
/// certificate here is ECDSA P-256: without P-256 listed, Chrome refuses the
/// handshake (checked). P-256 is listed for the certificates only: the key
/// exchange is still X25519 (choose_curve, accept_curve), and certificate
/// signatures do not use crypto-bigint.
pub const OFFERED_CURVES: [NamedCurve; 2] = [NamedCurve::X25519, NamedCurve::P256];

/// oeca: the server's key-exchange curve among the client's: X25519, or none
/// (the handshake then fails), whatever the client prefers.
pub fn choose_curve(offered: &[NamedCurve]) -> Option<NamedCurve> {
    offered.iter().copied().find(|c| KEY_EXCHANGE_CURVES.contains(c))
}

/// oeca: whether a client goes on with the key-exchange curve the server chose.
pub fn accept_curve(chosen: NamedCurve) -> bool {
    KEY_EXCHANGE_CURVES.contains(&chosen)
}

fn elliptic_curve_keypair(curve: NamedCurve) -> Result<NamedCurveKeypair> {
    let (public_key, private_key) = match curve {
        NamedCurve::P256 => {
            let secret_key = p256::ecdh::EphemeralSecret::random(&mut OsRng);
            let public_key = p256::EncodedPoint::from(secret_key.public_key());
            (
                public_key.as_bytes().to_vec(),
                NamedCurvePrivateKey::EphemeralSecretP256(secret_key),
            )
        }
        NamedCurve::P384 => {
            let secret_key = p384::ecdh::EphemeralSecret::random(&mut OsRng);
            let public_key = p384::EncodedPoint::from(secret_key.public_key());
            (
                public_key.as_bytes().to_vec(),
                NamedCurvePrivateKey::EphemeralSecretP384(secret_key),
            )
        }
        NamedCurve::X25519 => {
            let secret_key = x25519_dalek::StaticSecret::random_from_rng(OsRng);
            let public_key = x25519_dalek::PublicKey::from(&secret_key);
            (
                public_key.as_bytes().to_vec(),
                NamedCurvePrivateKey::StaticSecretX25519(secret_key),
            )
        }
        _ => return Err(Error::ErrInvalidNamedCurve),
    };

    Ok(NamedCurveKeypair {
        curve,
        public_key,
        private_key,
    })
}

impl NamedCurve {
    pub fn generate_keypair(&self) -> Result<NamedCurveKeypair> {
        match *self {
            NamedCurve::X25519 => elliptic_curve_keypair(NamedCurve::X25519),
            NamedCurve::P256 => elliptic_curve_keypair(NamedCurve::P256),
            NamedCurve::P384 => elliptic_curve_keypair(NamedCurve::P384),
            _ => Err(Error::ErrInvalidNamedCurve),
        }
    }
}

#[cfg(test)]
mod oeca_tests {
    use super::*;

    #[test]
    fn only_x25519_is_negotiated() {
        // A client that prefers P-256 (or offers nothing else) gets X25519 or nothing
        assert_eq!(choose_curve(&[NamedCurve::P256, NamedCurve::X25519, NamedCurve::P384]), Some(NamedCurve::X25519));
        assert_eq!(choose_curve(&[NamedCurve::P256]), None);
        assert_eq!(choose_curve(&[NamedCurve::P384, NamedCurve::P256]), None);
        assert_eq!(choose_curve(&[NamedCurve::Unsupported]), None);
        assert_eq!(choose_curve(&[]), None);
        // A server that chooses P-256 or P-384 is refused
        assert!(accept_curve(NamedCurve::X25519));
        assert!(!accept_curve(NamedCurve::P256));
        assert!(!accept_curve(NamedCurve::P384));
        assert!(!accept_curve(NamedCurve::Unsupported));
        // Listed: X25519 first, and P-256 for the certificates only
        assert_eq!(OFFERED_CURVES, [NamedCurve::X25519, NamedCurve::P256]);
        assert_eq!(choose_curve(&OFFERED_CURVES), Some(NamedCurve::X25519));
    }
}
