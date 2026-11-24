//! Who is at the other end of a connection, in types that cannot be mixed up:
//!
//! - `ClaimedFingerprint`: read from signaling. Anyone can write one, so it
//!   only converts into nothing: it can be compared, never trusted.
//! - `Connected`: the fingerprint of the certificate the DTLS connection
//!   really used (the peer proved it holds that key), equal to the claim.
//! - `Authenticated`: a `Connected` peer that completed commit-reveal and
//!   stated, inside that connection, the room id of the peer we meant. Only
//!   this can be pinned or shown under a trusted name.
//!
//! The channel `oeca-nearby-v1`, opened by the offerer (the construction is
//! `osv`'s; ids and the acknowledgement travel beside it):
//!
//! 1. answerer: `{"ready":1,"peer":"<its room id>"}`
//! 2. offerer, if that is the id it sent the offer to and the certificate is
//!    the claimed one: `{"r":"<secret>","peer":"<its room id>"}`
//! 3. answerer, if the secret opens the commitment and that is the id the
//!    offer came from: `{"ok":1}`
//!
//! Step 1 and 2's ids are what stops a relay: it can pass SDP along, but the
//! two ends then name each other, not it.

use localsend::serde_json::{Value, from_slice, json};
use localsend::util::base64;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};
use webrtc::data_channel::RTCDataChannel;
use webrtc::data_channel::data_channel_message::DataChannelMessage;
use webrtc::peer_connection::RTCPeerConnection;

use super::osv;

/// A canonical certificate fingerprint: `sha-256 ab:cd:...` (32 lower-case bytes).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Fingerprint(String);

impl Fingerprint {
    pub fn parse(text: &str) -> Option<Self> {
        // ASCII whitespace and case only, as the browser's canonicalFingerprint
        let text = text
            .split_ascii_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_ascii_lowercase();
        // Only SHA-256 (what browsers and webrtc-rs use): weaker digests are refused
        let hex = text.strip_prefix("sha-256 ")?.as_bytes();
        let canonical = hex.len() == 95
            && hex.iter().enumerate().all(|(i, c)| {
                if i % 3 == 2 {
                    *c == b':'
                } else {
                    matches!(c, b'0'..=b'9' | b'a'..=b'f')
                }
            });
        canonical.then_some(Self(text))
    }

    /// Of a certificate's DER.
    pub fn of_cert(der: &[u8]) -> Self {
        let digest = localsend::crypto::hash::sha256(der);
        let hex: Vec<String> = digest.iter().map(|b| format!("{b:02x}")).collect();
        Self(format!("sha-256 {}", hex.join(":")))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// For display: the first 8 bytes, in groups.
    pub fn short(&self) -> String {
        let hex: Vec<char> = self.0[8..].chars().filter(|c| *c != ':').take(16).collect();
        hex.chunks(4)
            .map(|c| c.iter().collect::<String>().to_ascii_uppercase())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// What an SDP says. Not an identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaimedFingerprint(Fingerprint);

impl ClaimedFingerprint {
    pub(super) fn from_signaling(fingerprint: Fingerprint) -> Self {
        Self(fingerprint)
    }
}

/// The certificate of the connection itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Connected {
    fingerprint: Fingerprint,
}

impl Connected {
    /// The certificate DTLS used must be the one that was claimed.
    fn check(der: &[u8], claimed: &ClaimedFingerprint) -> Result<Self, &'static str> {
        if der.is_empty() {
            return Err("The connection has no certificate");
        }
        let fingerprint = Fingerprint::of_cert(der);
        if fingerprint != claimed.0 {
            return Err("The connection's certificate is not the announced one");
        }
        Ok(Self { fingerprint })
    }

    pub fn fingerprint(&self) -> &Fingerprint {
        &self.fingerprint
    }
}

/// A connected peer that passed the whole exchange.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Authenticated {
    connected: Connected,
    code: String,
}

impl Authenticated {
    pub fn fingerprint(&self) -> &Fingerprint {
        &self.connected.fingerprint
    }

    /// The same on both screens only if both hold this connection's certificates.
    pub fn code(&self) -> &str {
        &self.code
    }
}

pub(super) async fn connected(
    pc: &RTCPeerConnection,
    claimed: &ClaimedFingerprint,
) -> Result<Connected, &'static str> {
    let der = pc.sctp().transport().get_remote_certificate().await;
    Connected::check(&der, claimed)
}

/// This end, the peer meant, and the two values the code needs from signaling.
#[derive(Clone)]
pub(super) struct Exchange {
    pub mine: Fingerprint,
    pub my_id: String,
    /// The peer this side selected (send) or the offer came from (receive).
    pub peer_id: String,
    pub claimed: ClaimedFingerprint,
}

const OTHER_PEER: &str =
    "The connection belongs to another device than the one selected (possible relay)";
const BAD_REVEAL: &str = "Verification failed: the sender's reveal is missing or does not match its commitment (possible interception)";
const NO_PROOF: &str = "Verification did not complete";

impl Exchange {
    fn check_ready(&self, message: &[u8]) -> Result<(), &'static str> {
        let m: Value = from_slice(message).map_err(|_| NO_PROOF)?;
        if m.get("ready").and_then(Value::as_u64) != Some(1) {
            return Err(NO_PROOF);
        }
        if m.get("peer").and_then(Value::as_str) != Some(self.peer_id.as_str()) {
            return Err(OTHER_PEER);
        }
        Ok(())
    }

    fn check_reveal(&self, message: &[u8], commit: &str) -> Result<String, &'static str> {
        let m: Value = from_slice(message).map_err(|_| BAD_REVEAL)?;
        let secret = m.get("r").and_then(Value::as_str).ok_or(BAD_REVEAL)?;
        if base64::decode(secret).ok().map(|b| b.len()) != Some(32) || osv::commit(secret) != commit
        {
            return Err(BAD_REVEAL);
        }
        if m.get("peer").and_then(Value::as_str) != Some(self.peer_id.as_str()) {
            return Err(OTHER_PEER);
        }
        Ok(secret.to_string())
    }

    fn authenticated(&self, connected: Connected, secret: &str, nonce: &str) -> Authenticated {
        let code = osv::code(
            self.mine.as_str(),
            connected.fingerprint.as_str(),
            secret,
            nonce,
        );
        Authenticated { connected, code }
    }

    /// Offerer. The channel is returned to keep it open.
    pub async fn initiate(
        &self,
        pc: &RTCPeerConnection,
        secret: &str,
        nonce: &str,
    ) -> Result<(Authenticated, Arc<RTCDataChannel>), &'static str> {
        let dc = pc
            .create_data_channel(osv::CHANNEL, None)
            .await
            .map_err(|_| NO_PROOF)?;
        let mut inbox = inbox(&dc);
        self.check_ready(&inbox.recv().await.ok_or(NO_PROOF)?)?;
        // The connection is up (it carried that). Reveal only to the claimed key
        let connected = connected(pc, &self.claimed).await?;
        let reveal = json!({ "r": secret, "peer": self.my_id }).to_string();
        dc.send_text(reveal).await.map_err(|_| NO_PROOF)?;
        let ok: Value = from_slice(&inbox.recv().await.ok_or(NO_PROOF)?).map_err(|_| NO_PROOF)?;
        if ok.get("ok").and_then(Value::as_u64) != Some(1) {
            return Err(NO_PROOF);
        }
        Ok((self.authenticated(connected, secret, nonce), dc))
    }

    /// Answerer. The channel is returned to keep it open.
    pub async fn answer(
        &self,
        pc: &RTCPeerConnection,
        channels: &mut mpsc::Receiver<Arc<RTCDataChannel>>,
        commit: &str,
        nonce: &str,
    ) -> Result<(Authenticated, Arc<RTCDataChannel>), &'static str> {
        let dc = loop {
            let dc = channels.recv().await.ok_or(NO_PROOF)?;
            if dc.label() == osv::CHANNEL {
                break dc;
            }
        };
        let mut inbox = inbox(&dc);
        opened(&dc).await.map_err(|_| NO_PROOF)?;
        let connected = connected(pc, &self.claimed).await?;
        let ready = json!({ "ready": 1, "peer": self.my_id }).to_string();
        dc.send_text(ready).await.map_err(|_| NO_PROOF)?;
        let secret = self.check_reveal(&inbox.recv().await.ok_or(BAD_REVEAL)?, commit)?;
        dc.send_text(json!({ "ok": 1 }).to_string())
            .await
            .map_err(|_| NO_PROOF)?;
        Ok((self.authenticated(connected, &secret, nonce), dc))
    }
}

/// The exchange is three messages: more are dropped.
fn inbox(dc: &RTCDataChannel) -> mpsc::Receiver<Vec<u8>> {
    let (tx, rx) = mpsc::channel(4);
    dc.on_message(Box::new(move |m: DataChannelMessage| {
        if m.data.len() <= 1024 {
            let _ = tx.try_send(m.data.to_vec());
        }
        Box::pin(async {})
    }));
    rx
}

fn opened(dc: &RTCDataChannel) -> oneshot::Receiver<()> {
    let (tx, rx) = oneshot::channel();
    let mut tx = Some(tx);
    dc.on_open(Box::new(move || {
        if let Some(tx) = tx.take() {
            let _ = tx.send(());
        }
        Box::pin(async {})
    }));
    rx
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    /// Synthetic certificates: the bytes are the identity.
    pub fn fingerprint(seed: usize) -> Fingerprint {
        Fingerprint::of_cert(format!("synthetic certificate {seed}").as_bytes())
    }

    pub fn connected(seed: usize) -> Connected {
        let der = format!("synthetic certificate {seed}");
        let claimed = ClaimedFingerprint::from_signaling(fingerprint(seed));
        Connected::check(der.as_bytes(), &claimed).unwrap()
    }

    pub fn authenticated(seed: usize) -> Authenticated {
        exchange("me", "peer", seed).authenticated(connected(seed), &osv::random(), &osv::random())
    }

    fn exchange(my_id: &str, peer_id: &str, claimed: usize) -> Exchange {
        Exchange {
            mine: fingerprint(0),
            my_id: my_id.into(),
            peer_id: peer_id.into(),
            claimed: ClaimedFingerprint::from_signaling(fingerprint(claimed)),
        }
    }

    #[test]
    fn a_connection_is_only_the_certificate_it_really_used() {
        let claimed = ClaimedFingerprint::from_signaling(fingerprint(1));
        assert!(Connected::check(b"synthetic certificate 1", &claimed).is_ok());
        assert!(
            Connected::check(b"synthetic certificate 2", &claimed).is_err(),
            "a copied public fingerprint"
        );
        assert!(Connected::check(b"", &claimed).is_err(), "no connection");
    }

    #[test]
    fn the_offerer_reveals_only_to_the_peer_it_selected() {
        let a = exchange("a", "m", 1);
        assert_eq!(
            a.check_ready(br#"{"ready":1,"peer":"b"}"#),
            Err(OTHER_PEER),
            "m relayed the offer to b"
        );
        assert_eq!(a.check_ready(br#"{"ready":1}"#), Err(OTHER_PEER), "no id");
        assert_eq!(a.check_ready(br#"{"peer":"m"}"#), Err(NO_PROOF));
        assert_eq!(a.check_ready(b"not json"), Err(NO_PROOF));
        assert!(a.check_ready(br#"{"ready":1,"peer":"m"}"#).is_ok());
    }

    #[test]
    fn the_answerer_accepts_only_the_sender_it_was_told() {
        let (b, secret) = (exchange("b", "m", 1), osv::random());
        let commit = osv::commit(&secret);
        let said = |r: &str, peer: &str| json!({ "r": r, "peer": peer }).to_string();
        assert_eq!(
            b.check_reveal(said(&secret, "a").as_bytes(), &commit),
            Err(OTHER_PEER),
            "a's connection behind m's name"
        );
        assert_eq!(
            b.check_reveal(said(&osv::random(), "m").as_bytes(), &commit),
            Err(BAD_REVEAL),
            "another secret"
        );
        let no_id = json!({ "r": secret }).to_string();
        assert_eq!(b.check_reveal(no_id.as_bytes(), &commit), Err(OTHER_PEER));
        assert_eq!(
            b.check_reveal(said(&secret, "m").as_bytes(), &commit),
            Ok(secret.clone())
        );
    }

    #[test]
    fn fingerprints_are_canonical() {
        let f = fingerprint(1);
        assert_eq!(
            Fingerprint::parse(&f.as_str().to_uppercase().replace("SHA-256", "sha-256")),
            Some(f.clone())
        );
        assert_eq!(
            Fingerprint::parse(&format!("sha-256 {}", "z".repeat(95))),
            None
        );
        assert_eq!(Fingerprint::parse(&f.as_str().replace(':', "-")), None);
        assert_eq!(f.short().len(), 19);
    }
}
