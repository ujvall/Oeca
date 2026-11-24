// Modified by Oeca from LocalSend (Apache-2.0): see OECA-VENDOR.txt and OECA-PATCHES.diff.
use crate::crypto;
use crate::crypto::token::{SigningTokenKey, VerifyingTokenKey};
use crate::model::transfer::FileDto;
use crate::util::base64;
use crate::webrtc::signaling::{ManagedSignalingConnection, WsServerSdpMessage};
use anyhow::Result;
use bytes::{Bytes, BytesMut};
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::io::{Read, Write};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;
use webrtc::api::APIBuilder;
use webrtc::data_channel::data_channel_init::RTCDataChannelInit;
use webrtc::data_channel::data_channel_message::DataChannelMessage;
use webrtc::data_channel::RTCDataChannel;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::peer_connection::certificate::RTCCertificate;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::peer_connection::RTCPeerConnection;

/// Nonce message exchanged by both peers
/// starting with the sending peer.
#[derive(Debug, Deserialize, Serialize)]
struct RTCNonceMessage {
    /// Nonce to be used to hash in combination with the public key.
    /// Encoded in base64 (url-safe without padding).
    nonce: String,
}

/// Sending peer sends the token.
#[derive(Debug, Deserialize, Serialize)]
struct RTCTokenRequest {
    /// Token using the received nonce and the public key.
    /// HASH_METHOD.HASH.SIGN_METHOD.SIGN
    token: String,
}

/// Receiving peer sends the token as well but
/// also the status for the next step.
/// After this step, the sending peer may close the connection due to `InvalidSignature`.
#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "status", rename_all = "SCREAMING_SNAKE_CASE")]
enum RTCTokenResponse {
    Ok { token: String },
    PinRequired { token: String },
    InvalidSignature,
}

/// This struct is used by both sender and receiver.
/// (1) The sender tries to resolve PIN first, waiting for `RTCPinReceivingResponse`.
/// (2) The receiver tries to resolve PIN, waiting for `RTCPinSendingResponse`.
///
/// This step may be skipped by the sending peer if `RTCInitResponse` is `Ok`.
/// In this case, the sending peer sends the `RTCPinSendingResponse` right away.
#[derive(Debug, Deserialize, Serialize)]
struct RTCPinMessage {
    pin: String,
}

/// Response to the PIN message by the receiving peer.
#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "status", rename_all = "SCREAMING_SNAKE_CASE")]
enum RTCPinReceivingResponse {
    Ok,
    PinRequired,
    TooManyAttempts,
}

/// Response to the PIN message by the sending peer.
#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "status", rename_all = "SCREAMING_SNAKE_CASE")]
enum RTCPinSendingResponse {
    Ok { files: Vec<FileDto> },
    PinRequired,
    TooManyAttempts,
}

/// Sent by receiving peer after receiving `RTCPinSendingResponse::Ok`.
#[derive(Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "status", rename_all = "SCREAMING_SNAKE_CASE")]
enum RTCFileListResponse {
    Ok {
        files: HashMap<String, String>,
    },
    Pair {
        #[serde(rename = "publicKey")]
        public_key: String,
    },
    Declined,
    InvalidSignature,
}

/// Pair response by the sending peer.
#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "status", rename_all = "SCREAMING_SNAKE_CASE")]
enum RTCPairResponse {
    Ok {
        #[serde(rename = "publicKey")]
        public_key: String,
    },
    PairDeclined,
    InvalidSignature,
}

#[derive(Debug)]
pub struct RTCFile {
    pub file_id: String,
    pub binary_rx: mpsc::Receiver<Bytes>,
}

#[derive(Debug)]
struct RTCFileState {
    file_id: String,
    size: u64,
    received: u64,
    binary_tx: mpsc::Sender<Bytes>,
}

#[derive(Debug, Deserialize, Serialize)]
struct RTCSendFileHeaderRequest {
    pub id: String,
    pub token: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct RTCSendFileResponse {
    pub id: String,
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Eq, PartialEq)]
pub enum RTCStatus {
    /// Received remote SDP offer/answer. Ready to start P2P connection.
    SdpExchanged,

    /// Opened data channel. Ready to further communication.
    Connected,

    /// PIN is required to proceed.
    PinRequired,

    /// Too many attempts. Connection is closed.
    TooManyAttempts,

    /// Declined by the receiving peer.
    Declined,

    /// Files are being sent.
    Sending,

    /// Data channel closed. Connection is closed.
    Finished,

    /// Error occurred. Connection is closed.
    Error(String),
}

#[derive(Debug, Eq, PartialEq)]
pub struct RTCFileError {
    pub file_id: String,
    pub error: String,
}

#[derive(Debug, Eq, PartialEq)]
pub struct PinConfig {
    pub pin: String,
    pub max_tries: u8,
}

const CHANNEL_LABEL: &str = "data";

/// oeca: the most an SDP may inflate to.
const MAX_SDP_BYTES: u64 = 64 * 1024;

/// oeca: the most one chunked control message (file list, token list) may be.
const MAX_CONTROL_BYTES: usize = 1024 * 1024;

/// oeca: after the receiver closed, how long its queued answers may take to be read.
const LAST_ANSWERS_TIMEOUT: Duration = Duration::from_millis(250);

/// oeca: the most channels a peer may open in one session. Each one costs
/// webrtc-rs a task and a 64 KiB read buffer.
const MAX_CHANNELS: usize = 4;

/// oeca: aborts the task when the session future ends or is dropped: a
/// detached one kept its buffers and the connection until the process exited.
struct AbortOnDrop<T>(tokio::task::JoinHandle<T>);

impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub async fn send_offer(
    signaling: &ManagedSignalingConnection,
    stun_servers: Vec<String>,
    certificate: Option<RTCCertificate>,
    target_id: Uuid,
    signing_key: SigningTokenKey,
    expecting_public_key: Option<Box<dyn VerifyingTokenKey + Send>>,
    pin: Option<PinConfig>,
    files: Vec<FileDto>,
    status_tx: mpsc::Sender<RTCStatus>,
    selected_files_tx: oneshot::Sender<HashSet<String>>,
    error_tx: mpsc::Sender<RTCFileError>,
    pin_tx: mpsc::Sender<oneshot::Sender<String>>,
    pair_tx: oneshot::Sender<oneshot::Sender<bool>>,
    mut sending_rx: mpsc::Receiver<RTCFile>,
    metadata_gate: Option<oneshot::Receiver<bool>>,
    extension: Option<Extension>,
) -> Result<()> {
    let (peer_connection, done_tx, mut done_rx) =
        create_peer_connection(stun_servers, certificate).await?;
    let close = CloseOnDrop::new(Arc::clone(&peer_connection));
    let mut extension = extension.unwrap_or_default();
    if let Some(tx) = extension.peer_connection.take() {
        let _ = tx.send(Arc::clone(&peer_connection));
    }

    let data_channel = peer_connection
        .create_data_channel(
            CHANNEL_LABEL,
            Some(RTCDataChannelInit {
                ordered: Some(true),
                max_packet_life_time: None,
                max_retransmits: None,
                protocol: None,
                negotiated: None,
            }),
        )
        .await?;
    close.set_channel(&data_channel);
    let mut limit = channel_limit(done_tx.clone());
    peer_connection.on_data_channel(Box::new(move |_| {
        let refused = limit();
        Box::pin(refuse_channel(refused))
    }));
    end_on_close(&data_channel, done_tx);

    let (connected_tx, mut connected_rx) = mpsc::channel::<()>(1);
    data_channel.on_open(Box::new(move || {
        Box::pin(async move {
            let _ = connected_tx.send(()).await;
        })
    }));

    let mut receive_rx = to_receive_stream(&data_channel, 16);
    let file_results = extension.file_results.take();

    let mut send_task = {
        let data_channel = Arc::clone(&data_channel);
        let status_tx = status_tx.clone();
        AbortOnDrop(tokio::spawn(async move {
            connected_rx
                .recv()
                .await
                .ok_or_else(|| anyhow::anyhow!("Data channel not opened"))?;

            wait_buffer_empty(&data_channel).await;

            tracing::debug!("Data channel opened. Exchanging nonce...");

            // Nonce exchange
            let nonce = {
                let mut local_nonce = crypto::nonce::generate_nonce();
                data_channel
                    .send_text(&serde_json::to_string(&RTCNonceMessage {
                        nonce: base64::encode(&local_nonce),
                    })?)
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to send nonce: {e}"))?;

                let mut remote_nonce = receive_nonce(&mut receive_rx).await?;

                // Final nonce: sender_nonce || receiver_nonce
                local_nonce.append(&mut remote_nonce);
                local_nonce
            };

            tracing::debug!("Nonce exchanged. Exchanging token...");

            // Token exchange
            let local_token = crypto::token::generate_token_nonce(&signing_key, &nonce)
                .map_err(|e| anyhow::anyhow!("Failed to generate token: {e}"))?;

            data_channel
                .send_text(&serde_json::to_string(&RTCTokenRequest {
                    token: local_token,
                })?)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to send token: {e}"))?;

            let token_response: RTCTokenResponse = match receive_rx.recv().await {
                Some(msg) => {
                    if !msg.is_string {
                        return Err(anyhow::anyhow!("Expected string message"));
                    }

                    serde_json::from_slice(&msg.data)?
                }
                None => {
                    return Err(anyhow::anyhow!("Failed to receive token response"));
                }
            };

            let remote_token = match &token_response {
                RTCTokenResponse::Ok { token } | RTCTokenResponse::PinRequired { token } => {
                    if let Some(expecting_public_key) = expecting_public_key {
                        if !crypto::token::verify_token_nonce(
                            &*expecting_public_key,
                            &token,
                            &nonce,
                        ) {
                            return Err(anyhow::anyhow!("Invalid token signature or nonce"));
                        }
                    }
                    token.to_owned()
                }
                RTCTokenResponse::InvalidSignature => {
                    return Err(anyhow::anyhow!(
                        "Invalid token signature from receiving peer"
                    ));
                }
            };

            tracing::debug!("Tokens exchanged.");

            if let RTCTokenResponse::PinRequired { .. } = token_response {
                tracing::debug!("PIN challenge by receiver... (I need to send correct PIN)");
                handle_pin::<()>(
                    &data_channel,
                    &status_tx,
                    &pin_tx,
                    &mut receive_rx,
                    false,
                    |data| {
                        let Ok(pin_res) = serde_json::from_slice::<RTCPinReceivingResponse>(&data)
                        else {
                            return ChallengePinResult::ParseError(anyhow::anyhow!(
                                "Failed to parse pin response"
                            ));
                        };

                        match pin_res {
                            RTCPinReceivingResponse::Ok => ChallengePinResult::Ok(()),
                            RTCPinReceivingResponse::PinRequired => ChallengePinResult::PinRequired,
                            RTCPinReceivingResponse::TooManyAttempts => {
                                ChallengePinResult::TooManyAttempts
                            }
                        }
                    },
                )
                .await?;

                tracing::debug!("PIN challenge done.");
            }

            if let Some(pin) = &pin {
                tracing::debug!("PIN challenge by sender... (I need to verify the correct PIN)");
                verify_pin(
                    pin,
                    &data_channel,
                    &status_tx,
                    &mut receive_rx,
                    true,
                    |data_channel, result| {
                        let data_channel = Arc::clone(&data_channel);
                        async move {
                            send_string_in_chunks(
                                &data_channel,
                                serde_json::to_string(&match result {
                                    VerifyPinResult::PinRequired => {
                                        RTCPinSendingResponse::PinRequired
                                    }
                                    VerifyPinResult::TooManyAttempts => {
                                        RTCPinSendingResponse::TooManyAttempts
                                    }
                                })?,
                                |data_channel, chunk| async move {
                                    data_channel.send(&chunk).await?;
                                    Ok(data_channel)
                                },
                            )
                            .await?;

                            send_delimiter(&data_channel).await?;

                            Ok(())
                        }
                    },
                )
                .await?;
            }

            status_tx.send(RTCStatus::Connected).await?;

            // oeca: the caller may hold the file list (names, sizes, types)
            // until it has authenticated this connection: nothing about the
            // files is sent before the gate opens
            if let Some(gate) = metadata_gate {
                if !matches!(gate.await, Ok(true)) {
                    return Err(anyhow::anyhow!("Not verified: no file metadata was sent"));
                }
            }

            {
                // send file list message
                let file_list_req = serde_json::to_string(&RTCPinSendingResponse::Ok { files })?;

                let result = send_string_in_chunks(
                    Arc::clone(&data_channel),
                    file_list_req,
                    |data_channel, chunk| async move {
                        data_channel.send(&chunk).await?;
                        Ok(data_channel)
                    },
                )
                .await;

                if let Err(e) = result {
                    let _ = status_tx.try_send(RTCStatus::Error(format!(
                        "Failed to send file list message: {e}"
                    )));
                    return Err(e.into());
                }

                if let Err(e) = send_delimiter(&data_channel).await {
                    let _ = status_tx.try_send(RTCStatus::Error(format!(
                        "Failed to send file list message: {e}"
                    )));
                    return Err(e.into());
                }
            }

            tracing::debug!("Sent file list message. Waiting for file tokens...");

            // Receive file tokens
            let file_list_res = {
                let bytes = receive_string_from_chunks(&mut receive_rx).await?;
                let parsed: RTCFileListResponse = serde_json::from_slice(&*bytes).map_err(|e| {
                    anyhow::anyhow!("Failed to deserialize file list response: {e}")
                })?;
                parsed
            };

            let file_map = match file_list_res {
                RTCFileListResponse::Ok { files } => files,
                RTCFileListResponse::Pair { public_key } => {
                    tracing::debug!("Pair request received. Sending pair response...");
                    let signature_identifier =
                        crypto::token::extract_signature_identifier(&remote_token)
                            .ok_or(anyhow::anyhow!("Failed to extract signature identifier"))?;
                    let parsed_key =
                        crypto::token::parse_public_key(&public_key, signature_identifier)?;
                    if !crypto::token::verify_token_nonce(&*parsed_key, &remote_token, &nonce) {
                        wait_buffer_empty(&data_channel).await;
                        data_channel
                            .send_text(&serde_json::to_string(&RTCPairResponse::InvalidSignature)?)
                            .await?;
                        return Err(anyhow::anyhow!("Invalid token signature or nonce"));
                    }

                    let (pair_res_tx, pair_res_rx) = oneshot::channel::<bool>();
                    if pair_tx.send(pair_res_tx).is_err() {
                        return Err(anyhow::anyhow!("Failed to send pair response"));
                    }

                    let pair_user_response = pair_res_rx.await?;
                    if pair_user_response {
                        data_channel
                            .send_text(&serde_json::to_string(&RTCPairResponse::Ok { public_key })?)
                            .await?;
                    } else {
                        data_channel
                            .send_text(&serde_json::to_string(&RTCPairResponse::PairDeclined)?)
                            .await?;
                    }

                    let file_list_res = {
                        let bytes = receive_string_from_chunks(&mut receive_rx).await?;
                        let parsed: RTCFileListResponse =
                            serde_json::from_slice(&*bytes).map_err(|e| {
                                anyhow::anyhow!("Failed to deserialize file list response: {e}")
                            })?;
                        parsed
                    };

                    match file_list_res {
                        RTCFileListResponse::Ok { files } => files,
                        RTCFileListResponse::Declined => {
                            tracing::debug!("Declined by the receiving peer.");
                            let _ = status_tx.send(RTCStatus::Declined).await;
                            return Ok(());
                        }
                        RTCFileListResponse::InvalidSignature => {
                            let _ = status_tx
                                .send(RTCStatus::Error(
                                    "Invalid signature (not expected)".to_owned(),
                                ))
                                .await;
                            return Err(anyhow::anyhow!("Invalid signature (not expected)"));
                        }
                        RTCFileListResponse::Pair { .. } => {
                            let _ = status_tx
                                .send(RTCStatus::Error("Unexpected pair response".to_owned()))
                                .await;
                            return Err(anyhow::anyhow!("Unexpected pair response"));
                        }
                    }
                }
                RTCFileListResponse::Declined => {
                    tracing::debug!("Declined by the receiving peer.");
                    let _ = status_tx.send(RTCStatus::Declined).await;
                    return Ok(());
                }
                RTCFileListResponse::InvalidSignature => {
                    // This is not expected because the public key is not sent yet.
                    // Likely a bug in the implementation on the receiving side.
                    let _ = status_tx
                        .send(RTCStatus::Error(
                            "Invalid signature (not expected)".to_owned(),
                        ))
                        .await;
                    return Err(anyhow::anyhow!("Invalid signature (not expected)"));
                }
            };

            // Publish selected files
            if selected_files_tx
                .send(file_map.keys().cloned().collect())
                .is_err()
            {
                let error = "Could not publish selection";
                let _ = status_tx.send(RTCStatus::Error(error.to_owned())).await;
                return Err(anyhow::anyhow!(error));
            }

            tracing::debug!("Received file tokens. Sending files...");

            // oeca: the receiver answers once per file (when it reads the next header)
            let mut unconfirmed = HashSet::<String>::new();
            while let Some(message) = sending_rx.recv().await {
                // oeca: read as they come: unread, they block the channel and hide its close
                while let Ok(answer) = receive_rx.try_recv() {
                    take_file_result(answer, &mut unconfirmed, &file_results).await?;
                }
                let file_token = match file_map.get(&message.file_id) {
                    Some(file_token) => file_token,
                    None => {
                        let _ = error_tx
                            .send(RTCFileError {
                                file_id: message.file_id,
                                error: "Failed to get file token".to_string(),
                            })
                            .await;

                        continue;
                    }
                };

                let header = RTCSendFileHeaderRequest {
                    id: message.file_id.clone(),
                    token: file_token.clone(),
                };

                if let Err(e) = data_channel
                    .send_text(serde_json::to_string(&header)?)
                    .await
                {
                    let _ = error_tx
                        .send(RTCFileError {
                            file_id: message.file_id,
                            error: e.to_string(),
                        })
                        .await;
                    continue;
                }

                let result = process_in_chunks(
                    Arc::clone(&data_channel),
                    message.binary_rx,
                    |data_channel, chunk| async move {
                        data_channel.send(&chunk).await?;
                        Ok(data_channel)
                    },
                )
                .await;

                if let Err(e) = result {
                    let _ = error_tx
                        .send(RTCFileError {
                            file_id: message.file_id,
                            error: e.to_string(),
                        })
                        .await;
                    continue;
                }
                unconfirmed.insert(message.file_id);
            }

            wait_buffer_empty(&data_channel).await;

            send_delimiter(&data_channel).await?;

            // oeca: one answer whatever the count, then closing, lost the last file
            take_remaining_results(&mut receive_rx, &mut unconfirmed, &file_results).await?;

            Ok(())
        }))
    };

    let offer = peer_connection.create_offer(None).await?;
    let mut gather_complete = peer_connection.gathering_complete_promise().await;
    peer_connection.set_local_description(offer).await?;
    let _ = gather_complete.recv().await;

    let session_id = Uuid::new_v4().to_string();
    let local_description = peer_connection
        .local_description()
        .await
        .ok_or_else(|| anyhow::anyhow!("Could not generate local_description"))?;

    signaling
        .send_offer(
            session_id.clone(),
            target_id,
            encode_sdp(&with_session_lines(&local_description.sdp, &extension.sdp_lines)),
        )
        .await?;

    let (tx_answer, rx_answer) = oneshot::channel();

    signaling
        .on_answer(session_id, |message| {
            // oeca: the session may be gone (cancelled): not a panic, which
            // ended the signaling listener and so the whole connection
            let _ = tx_answer.send(message.sdp);
        })
        .await;

    let remote_desc = rx_answer.await?;
    if let Some(tx) = extension.applied_answer.take() {
        let _ = tx.send(remote_desc.clone());
    }

    if let Err(e) = status_tx.send(RTCStatus::SdpExchanged).await {
        peer_connection.close().await?;
        return Err(e.into());
    }

    let answer = RTCSessionDescription::answer(decode_sdp(&remote_desc)?)?;

    peer_connection.set_remote_description(answer).await?;

    tokio::select! {
        result = &mut send_task.0 => {
            match result {
                Ok(Ok(_)) => tracing::debug!("Sending done."),
                Ok(Err(result)) => {
                    return Err(result);
                },
                Err(e) => {
                    tracing::error!("Sending error: {e}");
                    return Err(anyhow::anyhow!("Sending error: {e}"));
                }
            }
        }
        _ = done_rx.recv() => {
            // oeca: a receiver closes right after its last answer (or DECLINED),
            // which can still be queued
            let _ = tokio::time::timeout(LAST_ANSWERS_TIMEOUT, &mut send_task.0).await;
        }
    }

    let _ = status_tx.send(RTCStatus::Finished).await;
    if let Err(e) = data_channel.close().await {
        tracing::error!("Failed to close data channel: {e}");
    }

    peer_connection.close().await?;

    Ok(())
}

pub async fn accept_offer(
    signaling: &ManagedSignalingConnection,
    stun_servers: Vec<String>,
    certificate: Option<RTCCertificate>,
    offer: &WsServerSdpMessage,
    signing_key: SigningTokenKey,
    expecting_public_key: Option<Box<dyn VerifyingTokenKey + Send>>,
    pin: Option<PinConfig>,
    status_tx: mpsc::Sender<RTCStatus>,
    files_tx: oneshot::Sender<Vec<FileDto>>,
    selected_files_rx: oneshot::Receiver<Option<HashSet<String>>>,
    error_tx: mpsc::Sender<RTCFileError>,
    pin_tx: mpsc::Sender<oneshot::Sender<String>>,
    receiving_tx: mpsc::Sender<RTCFile>,
    mut user_error_tx: mpsc::Receiver<RTCSendFileResponse>,
    extension: Option<Extension>,
) -> Result<()> {
    let (peer_connection, done_tx, mut done_rx) =
        create_peer_connection(stun_servers, certificate).await?;
    let close = CloseOnDrop::new(Arc::clone(&peer_connection));
    let channel_slot = Arc::clone(&close.1);
    let mut extension = extension.unwrap_or_default();
    if let Some(tx) = extension.peer_connection.take() {
        let _ = tx.send(Arc::clone(&peer_connection));
    }
    let other_channels = extension.channels.take();

    let (data_channel_tx, mut data_channel_rx) = mpsc::channel::<Arc<RTCDataChannel>>(1);

    let mut limit = channel_limit(done_tx.clone());
    peer_connection.on_data_channel(Box::new(move |d: Arc<RTCDataChannel>| {
        if limit() {
            return Box::pin(refuse_channel(true));
        }
        if d.label() != CHANNEL_LABEL {
            // oeca: not LocalSend's channel: the caller's extension may want it
            if let Some(tx) = &other_channels {
                let _ = tx.try_send(d);
            }
            return Box::pin(async {});
        }

        let data_channel_tx = data_channel_tx.clone();
        // oeca: closed with the session; its close ends the session
        *channel_slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::clone(&d));
        end_on_close(&d, done_tx.clone());
        Box::pin(async move {
            let d_clone = Arc::clone(&d);
            d.on_open(Box::new(move || {
                let _ = data_channel_tx.try_send(d_clone);

                Box::pin(async {})
            }));
        })
    }));

    let mut receive_task = {
        let status_tx = status_tx.clone();
        AbortOnDrop(tokio::spawn(async move {
            let Some(data_channel) = data_channel_rx.recv().await else {
                return Err::<(), anyhow::Error>(anyhow::anyhow!("Data channel not found"));
            };

            // We convert on_message to a stream of messages
            // to improve readability using a sequential implementation
            let mut receive_rx = to_receive_stream(&data_channel, 16);

            tracing::debug!("Data channel opened. Exchanging nonce...");

            // Nonce exchange
            let nonce = {
                let mut remote_nonce = receive_nonce(&mut receive_rx).await?;

                let mut local_nonce = crypto::nonce::generate_nonce();
                data_channel
                    .send_text(&serde_json::to_string(&RTCNonceMessage {
                        nonce: base64::encode(&local_nonce),
                    })?)
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to send nonce: {e}"))?;

                // Final nonce: sender_nonce || receiver_nonce
                remote_nonce.append(&mut local_nonce);
                remote_nonce
            };

            tracing::debug!("Nonce exchanged. Exchanging token...");

            // Token exchange
            let remote_token = match receive_rx.recv().await {
                Some(msg) => {
                    if !msg.is_string {
                        return Err(anyhow::anyhow!("Expected string message"));
                    }

                    serde_json::from_slice::<RTCTokenRequest>(&msg.data)?.token
                }
                None => {
                    return Err(anyhow::anyhow!("Failed to receive token"));
                }
            };

            if let Some(expecting_public_key) = expecting_public_key {
                // Optionally, verify the token signature
                if !crypto::token::verify_token_nonce(&*expecting_public_key, &remote_token, &nonce)
                {
                    data_channel
                        .send_text(&serde_json::to_string(&RTCTokenResponse::InvalidSignature)?)
                        .await?;
                    wait_buffer_empty(&data_channel).await;
                    return Err(anyhow::anyhow!("Invalid token signature or nonce"));
                }
                tracing::debug!("Token signature verified.");
            }

            {
                let local_token = crypto::token::generate_token_nonce(&signing_key, &nonce)
                    .map_err(|e| anyhow::anyhow!("Failed to generate token: {e}"))?;

                data_channel
                    .send_text(&serde_json::to_string(&match pin {
                        Some(_) => RTCTokenResponse::PinRequired { token: local_token },
                        None => RTCTokenResponse::Ok { token: local_token },
                    })?)
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to send token: {e}"))?;
            }

            if let Some(pin) = &pin {
                tracing::debug!("PIN required. Challenging...");
                verify_pin(
                    pin,
                    &data_channel,
                    &status_tx,
                    &mut receive_rx,
                    false,
                    |data_channel, result| {
                        let data_channel = Arc::clone(&data_channel);
                        async move {
                            data_channel
                                .send_text(&serde_json::to_string(&match result {
                                    VerifyPinResult::PinRequired => {
                                        RTCPinReceivingResponse::PinRequired
                                    }
                                    VerifyPinResult::TooManyAttempts => {
                                        RTCPinReceivingResponse::TooManyAttempts
                                    }
                                })?)
                                .await?;
                            Ok(())
                        }
                    },
                )
                .await?;

                data_channel
                    .send_text(
                        serde_json::to_string(&RTCPinReceivingResponse::Ok)
                            .expect("Failed to serialize"),
                    )
                    .await?;

                tracing::debug!("PIN challenge done.");
            }

            tracing::debug!("Waiting for sender PIN status...");

            let pin_response = {
                let bytes = receive_string_from_chunks(&mut receive_rx).await?;
                let parsed: RTCPinSendingResponse =
                    serde_json::from_slice(&*bytes).map_err(|e| {
                        anyhow::anyhow!("Failed to deserialize file list response: {e}")
                    })?;
                parsed
            };

            let file_list = match pin_response {
                RTCPinSendingResponse::Ok { files } => files,
                RTCPinSendingResponse::PinRequired => {
                    tracing::debug!("PIN challenge by sender... (I need to send correct PIN)");
                    let result = handle_pin::<Vec<FileDto>>(
                        &data_channel,
                        &status_tx,
                        &pin_tx,
                        &mut receive_rx,
                        true,
                        |data| {
                            let Ok(pin_res) =
                                serde_json::from_slice::<RTCPinSendingResponse>(&data)
                            else {
                                return ChallengePinResult::ParseError(anyhow::anyhow!(
                                    "Failed to parse pin response"
                                ));
                            };
                            match pin_res {
                                RTCPinSendingResponse::Ok { files } => {
                                    ChallengePinResult::Ok(files)
                                }
                                RTCPinSendingResponse::PinRequired => {
                                    ChallengePinResult::PinRequired
                                }
                                RTCPinSendingResponse::TooManyAttempts => {
                                    ChallengePinResult::TooManyAttempts
                                }
                            }
                        },
                    )
                    .await?;

                    result
                }
                RTCPinSendingResponse::TooManyAttempts => {
                    let _ = status_tx
                        .send(RTCStatus::Error(
                            "Unexpected TooManyAttempts response".to_owned(),
                        ))
                        .await;
                    return Err(anyhow::anyhow!("Unexpected TooManyAttempts response"));
                }
            };

            status_tx.send(RTCStatus::Connected).await?;

            tracing::debug!("Received file list.");

            files_tx
                .send(file_list.clone())
                .map_err(|_| anyhow::anyhow!("Failed to send file list"))?;

            // Init: Receive user selection
            let Ok(selected_files) = selected_files_rx.await else {
                return Ok(());
            };

            let Some(selected_files) = selected_files else {
                // Declined by the user
                send_string_in_chunks(
                    Arc::clone(&data_channel),
                    serde_json::to_string(&RTCFileListResponse::Declined)?,
                    |data_channel, chunk| async move {
                        data_channel.send(&chunk).await?;
                        Ok(data_channel)
                    },
                )
                .await?;

                send_delimiter(&data_channel).await?;

                // oeca: let DECLINED leave before the connection closes (as the
                // end of a transfer does), or the sender may never get it
                let _ = tokio::time::timeout(Duration::from_secs(5), async {
                    wait_buffer_empty(&data_channel).await;
                })
                .await;

                return Ok(());
            };

            let file_tokens = selected_files
                .into_iter()
                .map(|file_id| {
                    let token = Uuid::new_v4().to_string();
                    (file_id, token)
                })
                .collect::<HashMap<String, String>>();

            send_string_in_chunks(
                Arc::clone(&data_channel),
                serde_json::to_string(&RTCFileListResponse::Ok {
                    files: file_tokens.clone(),
                })?,
                |data_channel, chunk| async move {
                    data_channel.send(&chunk).await?;
                    Ok(data_channel)
                },
            )
            .await?;

            send_delimiter(&data_channel).await?;

            tracing::debug!("Sent file tokens.");

            // Receive files
            let mut file_state: Option<RTCFileState> = None;
            while let Some(msg) = receive_rx.recv().await {
                if msg.is_string {
                    // End of last file
                    let last_file_id = file_state.as_ref().map(|s| s.file_id.clone());
                    file_state = None;

                    if let Some(last_file_id) = last_file_id {
                        let error = match user_error_tx.recv().await {
                            Some(result) => {
                                if result.success {
                                    None
                                } else {
                                    Some(result.error.map_or("Unknown error".to_string(), |e| e))
                                }
                            }
                            None => Some("Failed to receive file result".to_string()),
                        };

                        data_channel
                            .send_text(serde_json::to_string(&RTCSendFileResponse {
                                id: last_file_id,
                                success: error.is_none(),
                                error,
                            })?)
                            .await?;

                        if is_delimiter(&msg) {
                            // End of all files

                            // Wait for the last status to be sent
                            let _ = tokio::time::timeout(Duration::from_secs(5), async {
                                wait_buffer_empty(&data_channel).await;
                            })
                            .await;

                            break;
                        }
                    }

                    let header: RTCSendFileHeaderRequest = serde_json::from_slice(&msg.data)?;
                    match file_tokens.get(&header.id) {
                        Some(entry) => {
                            if header.token != *entry {
                                let _ = error_tx
                                    .send(RTCFileError {
                                        file_id: header.id,
                                        error: "Invalid token".to_string(),
                                    })
                                    .await;
                                continue;
                            }
                        }
                        None => {
                            let _ = error_tx
                                .send(RTCFileError {
                                    file_id: header.id,
                                    error: "File not found".to_string(),
                                })
                                .await;
                            continue;
                        }
                    }

                    let (tx, rx) = mpsc::channel::<Bytes>(4);

                    let size = {
                        let entry = file_list.iter().find(|f| f.id == header.id);
                        match entry {
                            Some(file) => file.size,
                            None => {
                                let _ = error_tx
                                    .send(RTCFileError {
                                        file_id: header.id,
                                        error: "Expected size to be available".to_string(),
                                    })
                                    .await;
                                continue;
                            }
                        }
                    };

                    file_state = Some(RTCFileState {
                        file_id: header.id.clone(),
                        size,
                        received: 0,
                        binary_tx: tx,
                    });

                    let _ = receiving_tx
                        .send(RTCFile {
                            file_id: header.id.clone(),
                            binary_rx: rx,
                        })
                        .await;
                } else {
                    // publish binary data
                    match &mut file_state {
                        Some(state) => {
                            state.received = state.received + msg.data.len() as u64;
                            if state.received > state.size {
                                // Sender transmitted more bytes than declared. Interrupt early
                                // to avoid writing a corrupt/oversized file.
                                let _ = error_tx
                                    .send(RTCFileError {
                                        file_id: state.file_id.clone(),
                                        error: format!(
                                            "Received more bytes than expected (expected {}, got {})",
                                            state.size, state.received
                                        ),
                                    })
                                    .await;

                                // Drop the state so the app-side receiver is closed and no
                                // further binaries for this file are forwarded.
                                file_state = None;
                                continue;
                            }

                            state.binary_tx.send(msg.data).await?;
                        }
                        None => {
                            let _ = error_tx
                                .send(RTCFileError {
                                    file_id: "unknown".to_string(),
                                    error: "Received binary data without a header".to_string(),
                                })
                                .await;
                        }
                    }
                }
            }

            Ok(())
        }))
    };

    let remote_desc_sdp = decode_sdp(&offer.sdp)?;
    let remote_desc = RTCSessionDescription::offer(remote_desc_sdp)?;
    peer_connection.set_remote_description(remote_desc).await?;

    let answer = peer_connection.create_answer(None).await?;

    let mut gather_complete = peer_connection.gathering_complete_promise().await;
    peer_connection.set_local_description(answer).await?;
    let _ = gather_complete.recv().await;

    let local_description = peer_connection
        .local_description()
        .await
        .ok_or_else(|| anyhow::anyhow!("generate local_description failed!"))?;

    signaling
        .send_answer(
            offer.session_id.clone(),
            offer.peer.id,
            encode_sdp(&with_session_lines(&local_description.sdp, &extension.sdp_lines)),
        )
        .await?;

    if let Err(e) = status_tx.send(RTCStatus::SdpExchanged).await {
        peer_connection.close().await?;
        return Err(e.into());
    }

    tokio::select! {
        result = &mut receive_task.0 => {
            match result {
                Ok(Ok(_)) => tracing::debug!("Receiving done."),
                Ok(Err(result)) => {
                    return Err(result);
                },
                Err(e) => {
                    tracing::error!("Receiving error: {e}");
                    return Err(anyhow::anyhow!("Receiving error: {e}"));
                }
            }
        }
        _ = done_rx.recv() => {
            tracing::debug!("Peer connection remotely closed.");
        }
    }

    let _ = status_tx.send(RTCStatus::Finished).await;
    peer_connection.close().await?;

    Ok(())
}

/// oeca: closes the peer connection however the session future ends,
/// including when the caller drops it (cancel, timeout, shutdown), so its
/// ICE / DTLS / SCTP tasks and sockets do not outlive the session. The data
/// channel is closed first: its SCTP stream reset reaches the other side at
/// once (it ends its session, `end_on_close`), instead of after ICE times out.
struct CloseOnDrop(
    Arc<RTCPeerConnection>,
    Arc<std::sync::Mutex<Option<Arc<RTCDataChannel>>>>,
);

impl CloseOnDrop {
    fn new(pc: Arc<RTCPeerConnection>) -> Self {
        Self(pc, Arc::new(std::sync::Mutex::new(None)))
    }

    fn set_channel(&self, channel: &Arc<RTCDataChannel>) {
        *self.1.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::clone(channel));
    }
}

impl Drop for CloseOnDrop {
    fn drop(&mut self) {
        let pc = Arc::clone(&self.0);
        let channel = self.1.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Ok(rt) = tokio::runtime::Handle::try_current() {
            rt.spawn(async move {
                let _ = tokio::time::timeout(Duration::from_secs(5), async {
                    if let Some(channel) = channel {
                        let _ = channel.close().await;
                    }
                    let _ = pc.close().await;
                })
                .await;
            });
        }
    }
}

/// oeca: what an app-level extension of a session needs, without touching
/// LocalSend's own messages: SDP lines added to the description this side
/// signals (peers that do not know them ignore them), the session's peer
/// connection (to open a channel of its own), and the data channels other
/// than LocalSend's "data" that the peer opens (ignored otherwise).
#[derive(Default)]
pub struct Extension {
    /// Session-level lines, e.g. "a=x-example:1"; added before the first m= line.
    pub sdp_lines: Vec<String>,
    pub peer_connection: Option<oneshot::Sender<Arc<RTCPeerConnection>>>,
    pub channels: Option<mpsc::Sender<Arc<RTCDataChannel>>>,
    /// Offer side: the answer this session applies (matched by session id), as
    /// signaled. Authenticate this one: other ANSWER messages can claim anything.
    pub applied_answer: Option<oneshot::Sender<String>>,
    /// Offer side: what the receiver answered for each file sent. Only these
    /// say that a file was saved.
    pub file_results: Option<mpsc::Sender<RTCSendFileResponse>>,
}

fn with_session_lines(sdp: &str, lines: &[String]) -> String {
    if lines.is_empty() {
        return sdp.to_string();
    }
    let added: String = lines.iter().map(|l| format!("{l}\r\n")).collect();
    match sdp.find("\nm=") {
        Some(i) => format!("{}{}{}", &sdp[..i + 1], added, &sdp[i + 1..]),
        None => format!("{sdp}{added}"),
    }
}

/// oeca: the receiver's answer about one file this session sent.
async fn take_file_result(
    answer: DataChannelMessage,
    unconfirmed: &mut HashSet<String>,
    file_results: &Option<mpsc::Sender<RTCSendFileResponse>>,
) -> Result<()> {
    if !answer.is_string {
        return Err(anyhow::anyhow!("Expected a file confirmation"));
    }
    let response: RTCSendFileResponse = serde_json::from_slice(&answer.data)?;
    if !unconfirmed.remove(&response.id) {
        return Err(anyhow::anyhow!("Confirmation of a file that was not sent"));
    }
    if let Some(tx) = file_results {
        let _ = tx.send(response).await;
    }
    Ok(())
}

/// oeca: an answer for every file still unconfirmed. A close first is a failure;
/// answers already queued are read before the close is seen.
async fn take_remaining_results(
    receive_rx: &mut mpsc::Receiver<DataChannelMessage>,
    unconfirmed: &mut HashSet<String>,
    file_results: &Option<mpsc::Sender<RTCSendFileResponse>>,
) -> Result<()> {
    while !unconfirmed.is_empty() {
        let answer = receive_rx
            .recv()
            .await
            .ok_or_else(|| anyhow::anyhow!("Closed before confirming every file"))?;
        take_file_result(answer, unconfirmed, file_results).await?;
    }
    Ok(())
}

/// oeca: counts the channels a peer opens; true (and the session ends) once
/// there are too many.
fn channel_limit(done_tx: mpsc::Sender<()>) -> impl FnMut() -> bool + Send + Sync {
    let mut opened = 0usize;
    move || {
        opened = opened.saturating_add(1);
        let refused = opened > MAX_CHANNELS;
        if refused {
            let _ = done_tx.try_send(());
        }
        refused
    }
}

/// oeca: webrtc-rs accepts no further channel until this returns, so a flood
/// cannot allocate while the session closes.
async fn refuse_channel(refused: bool) {
    if refused {
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

/// oeca: the other side closed the data channel (it cancelled, or it is
/// done): end this session now, as a lost connection would.
fn end_on_close(channel: &Arc<RTCDataChannel>, done_tx: mpsc::Sender<()>) {
    channel.on_close(Box::new(move || {
        let _ = done_tx.try_send(());
        Box::pin(async {})
    }));
}

/// oeca: which local addresses may be offered to a peer as ICE host
/// candidates (every one when unset). The embedding app decides, e.g. to keep
/// virtual adapters and VPN addresses out of what peers see.
pub type IpFilter = Arc<dyn Fn(std::net::IpAddr) -> bool + Send + Sync>;

static IP_FILTER: std::sync::RwLock<Option<IpFilter>> = std::sync::RwLock::new(None);

/// Applies to every peer connection made afterwards.
pub fn set_ip_filter(filter: Option<IpFilter>) {
    *IP_FILTER.write().unwrap_or_else(|e| e.into_inner()) = filter;
}

async fn create_peer_connection(
    stun: Vec<String>,
    certificate: Option<RTCCertificate>,
) -> Result<(Arc<RTCPeerConnection>, mpsc::Sender<()>, mpsc::Receiver<()>)> {
    let mut setting = webrtc::api::setting_engine::SettingEngine::default();
    if let Some(filter) = IP_FILTER.read().unwrap_or_else(|e| e.into_inner()).clone() {
        setting.set_ip_filter(Box::new(move |ip| filter(ip)));
    }
    let api = APIBuilder::new().with_setting_engine(setting).build();

    // oeca: a persistent DTLS certificate gives the peer a stable fingerprint
    let config = RTCConfiguration {
        ice_servers: vec![RTCIceServer {
            urls: stun,
            ..Default::default()
        }],
        certificates: certificate.into_iter().collect(),
        ..Default::default()
    };

    let peer_connection = api.new_peer_connection(config).await?;

    let (done_tx, done_rx) = mpsc::channel::<()>(1);
    let channel_done_tx = done_tx.clone();

    peer_connection.on_peer_connection_state_change(Box::new(move |s: RTCPeerConnectionState| {
        // oeca: Failed and Closed end the session too (before: only Disconnected)
        if matches!(
            s,
            RTCPeerConnectionState::Disconnected
                | RTCPeerConnectionState::Failed
                | RTCPeerConnectionState::Closed
        ) {
            let _ = done_tx.try_send(());
        }
        Box::pin(async {})
    }));

    Ok((Arc::new(peer_connection), channel_done_tx, done_rx))
}

async fn receive_nonce(receive_rx: &mut mpsc::Receiver<DataChannelMessage>) -> Result<Vec<u8>> {
    let remote_nonce = match receive_rx.recv().await {
        Some(msg) => {
            if !msg.is_string {
                return Err(anyhow::anyhow!("Expected string message"));
            }

            let nonce_msg: RTCNonceMessage = serde_json::from_slice(&msg.data)?;
            let remote_nonce = base64::decode(&nonce_msg.nonce)
                .map_err(|e| anyhow::anyhow!("Failed to decode remote nonce: {e}"))?;

            if !crypto::nonce::validate_nonce(&remote_nonce) {
                return Err(anyhow::anyhow!("Invalid remote nonce"));
            }

            remote_nonce
        }
        None => {
            return Err(anyhow::anyhow!("Failed to receive nonce"));
        }
    };

    Ok(remote_nonce)
}

enum ChallengePinResult<T> {
    Ok(T),
    PinRequired,
    TooManyAttempts,
    ParseError(anyhow::Error),
}

async fn handle_pin<T>(
    data_channel: &Arc<RTCDataChannel>,
    status_tx: &mpsc::Sender<RTCStatus>,
    pin_tx: &mpsc::Sender<oneshot::Sender<String>>,
    receive_rx: &mut mpsc::Receiver<DataChannelMessage>,
    receive_in_chunks: bool,
    parse_response: fn(Bytes) -> ChallengePinResult<T>,
) -> Result<T> {
    loop {
        status_tx.send(RTCStatus::PinRequired).await?;
        let (new_pin_tx, new_pin_rx) = oneshot::channel::<String>();

        if pin_tx.send(new_pin_tx).await.is_err() {
            return Err(anyhow::anyhow!("Failed to send PIN request"));
        }

        let Ok(pin) = new_pin_rx.await else {
            return Err(anyhow::anyhow!("PIN required but not provided"));
        };

        data_channel
            .send_text(&serde_json::to_string(&RTCPinMessage { pin })?)
            .await?;

        let bytes = match receive_in_chunks {
            true => receive_string_from_chunks(receive_rx).await?,
            false => {
                let Some(init_status) = receive_rx.recv().await else {
                    return Err(anyhow::anyhow!("Failed to receive initial message"));
                };

                if !init_status.is_string {
                    return Err(anyhow::anyhow!("Expected string message"));
                }

                init_status.data
            }
        };

        let pin_res = parse_response(bytes);
        match pin_res {
            ChallengePinResult::Ok(result) => {
                return Ok(result);
            }
            ChallengePinResult::PinRequired => {
                continue;
            }
            ChallengePinResult::TooManyAttempts => {
                status_tx.send(RTCStatus::TooManyAttempts).await?;
                return Err(anyhow::anyhow!("Too many requests"));
            }
            ChallengePinResult::ParseError(e) => {
                return Err(e);
            }
        }
    }
}

#[derive(Debug)]
enum VerifyPinResult {
    PinRequired,
    TooManyAttempts,
}

async fn verify_pin<F, Fut>(
    pin_config: &PinConfig,
    data_channel: &Arc<RTCDataChannel>,
    status_tx: &mpsc::Sender<RTCStatus>,
    receive_rx: &mut mpsc::Receiver<DataChannelMessage>,
    mut send_initial_notice: bool,
    send_result: F,
) -> Result<()>
where
    F: Fn(&Arc<RTCDataChannel>, VerifyPinResult) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    status_tx.send(RTCStatus::PinRequired).await?;

    let mut remote_pin = "".to_string();
    let mut pin_try = 0u8;

    loop {
        if remote_pin == pin_config.pin {
            return Ok(());
        }

        if pin_try >= pin_config.max_tries {
            let _ = tokio::time::timeout(Duration::from_secs(5), async {
                let _ = send_result(&data_channel, VerifyPinResult::TooManyAttempts).await;

                wait_buffer_empty(&data_channel).await;
            })
            .await;

            status_tx.send(RTCStatus::TooManyAttempts).await?;

            return Err(anyhow::anyhow!("Too many requests"));
        }

        if send_initial_notice || pin_try > 0 {
            send_initial_notice = false;
            send_result(&data_channel, VerifyPinResult::PinRequired).await?;
        }

        remote_pin = match receive_rx.recv().await {
            Some(msg) => {
                let pin_req: RTCPinMessage = serde_json::from_slice(&msg.data)?;
                pin_req.pin
            }
            None => {
                return Err(anyhow::anyhow!("Failed to receive pin"));
            }
        };

        pin_try += 1;
    }
}

fn encode_sdp(s: &str) -> String {
    let mut e = ZlibEncoder::new(Vec::new(), flate2::Compression::best());
    e.write_all(s.as_bytes())
        .map_err(|e| anyhow::anyhow!("Failed to compress SDP: {e}"))
        .unwrap();
    let compressed = e
        .finish()
        .map_err(|e| anyhow::anyhow!("Failed to finish compression of SDP: {e}"))
        .unwrap();
    base64::encode(&compressed)
}

fn decode_sdp(s: &str) -> Result<String> {
    let decoded_data =
        base64::decode(s).map_err(|e| anyhow::anyhow!("Base64 decode of SDP failed: {e}"))?;
    // oeca: bounded (the SDP is untrusted; zlib expands about 1000x)
    let mut d = ZlibDecoder::new(&*decoded_data).take(MAX_SDP_BYTES + 1);
    let mut result = String::new();
    d.read_to_string(&mut result)
        .map_err(|e| anyhow::anyhow!("Failed to decompress SDP: {e}"))?;
    if result.len() as u64 > MAX_SDP_BYTES {
        return Err(anyhow::anyhow!("SDP too large"));
    }
    Ok(result)
}

async fn send_delimiter(data_channel: &Arc<RTCDataChannel>) -> Result<()> {
    // Somehow, empty messages are not received by the other peer, so we send a non-empty message
    data_channel.send_text("0".to_string()).await?;
    Ok(())
}

fn is_delimiter(msg: &DataChannelMessage) -> bool {
    msg.is_string && msg.data.len() <= 1
}

async fn wait_buffer_empty(data_channel: &Arc<RTCDataChannel>) {
    while data_channel.buffered_amount().await != 0 {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

const CHUNK_SIZE: usize = 16 * 1024; // 16 KiB

/// Process incoming data in chunks of CHUNK_SIZE
/// The callback returns the same data_channel to avoid re-creating or lifetime issues.
/// Note: You need to send the delimiter (or any other string) after the last chunk.
pub async fn process_in_chunks<T, F, Fut>(
    mut data_channel: T,
    mut rx: mpsc::Receiver<Bytes>,
    mut callback: F,
) -> Result<()>
where
    F: FnMut(T, Bytes) -> Fut,
    Fut: Future<Output = Result<T>>,
{
    let mut buffer = BytesMut::with_capacity(CHUNK_SIZE);

    while let Some(data) = rx.recv().await {
        buffer.extend_from_slice(&data);

        while buffer.len() >= CHUNK_SIZE {
            let chunk = buffer.split_to(CHUNK_SIZE).freeze();

            // Process the chunk, reuse the data_channel
            data_channel = callback(data_channel, chunk).await?;
        }
    }

    // After the channel is closed, if there's leftover data, handle it as needed:
    if !buffer.is_empty() {
        callback(data_channel, buffer.freeze()).await?;
    }

    Ok(())
}

/// Convenience function for `process_in_chunks` that processes a string in chunks.
/// Note: You need to send the delimiter (or any other string) after the last chunk.
pub async fn send_string_in_chunks<T, F, Fut>(
    data_channel: T,
    string: String,
    callback: F,
) -> Result<()>
where
    F: FnMut(T, Bytes) -> Fut,
    Fut: Future<Output = Result<T>>,
{
    let (tx, rx) = mpsc::channel(1);

    tokio::spawn(async move {
        let _ = tx.send(Bytes::from(string)).await;
    });

    process_in_chunks(data_channel, rx, callback).await
}

async fn receive_string_from_chunks(
    receive_rx: &mut mpsc::Receiver<DataChannelMessage>,
) -> Result<Bytes> {
    let mut buffer = BytesMut::new();
    while let Some(msg) = receive_rx.recv().await {
        if msg.is_string {
            break;
        }

        // oeca: the sender may be nobody we trust, and may never send the delimiter
        if buffer.len() + msg.data.len() > MAX_CONTROL_BYTES {
            return Err(anyhow::anyhow!("Control message too large"));
        }
        buffer.extend_from_slice(&msg.data);
    }

    Ok(buffer.freeze())
}

fn to_receive_stream(
    data_channel: &Arc<RTCDataChannel>,
    buffer: usize,
) -> mpsc::Receiver<DataChannelMessage> {
    let (tx, rx) = mpsc::channel(buffer);

    data_channel.on_message(Box::new(move |msg: DataChannelMessage| {
        let tx = tx.clone();
        Box::pin(async move {
            let _ = tx.send(msg).await;
        })
    }));

    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_process_in_chunks() {
        let (tx, rx) = mpsc::channel(16);

        tokio::spawn(async move {
            let mut test_vec = vec![0; CHUNK_SIZE * 2 + 5];
            test_vec[CHUNK_SIZE..CHUNK_SIZE * 2]
                .iter_mut()
                .for_each(|x| *x = 1);
            test_vec[CHUNK_SIZE * 2..].iter_mut().for_each(|x| *x = 2);
            tx.send(Bytes::from(test_vec)).await.unwrap();
        });

        let mut chunks = Vec::new();

        let result = process_in_chunks(0, rx, |_, chunk| {
            chunks.push(chunk);
            async { Ok(0) }
        })
        .await;

        assert!(result.is_ok());
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].len(), CHUNK_SIZE);
        assert_eq!(chunks[1].len(), CHUNK_SIZE);
        assert_eq!(chunks[2].len(), 5);

        assert_eq!(chunks[0].iter().all(|x| *x == 0), true);
        assert_eq!(chunks[1].iter().all(|x| *x == 1), true);
        assert_eq!(chunks[2].iter().all(|x| *x == 2), true);
    }

    // oeca: the sender's reading of the receiver's per-file answers

    fn answer(text: &str) -> DataChannelMessage {
        DataChannelMessage {
            is_string: true,
            data: Bytes::from(text.to_string()),
        }
    }

    fn saved(id: &str, success: bool) -> DataChannelMessage {
        answer(&format!(r#"{{"id":"{id}","success":{success}}}"#))
    }

    /// Feeds `answers`, then closes (dropping the sender) unless `open`.
    async fn read(sent: &[&str], answers: Vec<DataChannelMessage>, open: bool) -> (Result<()>, Vec<(String, bool)>, HashSet<String>) {
        let (tx, mut rx) = mpsc::channel(16);
        for a in answers {
            tx.send(a).await.unwrap();
        }
        let _kept = open.then_some(tx);
        let mut unconfirmed: HashSet<String> = sent.iter().map(|s| s.to_string()).collect();
        let (results_tx, mut results) = mpsc::channel(16);
        let results_tx = Some(results_tx);
        // Bounded: a reading that waits for an answer never coming fails here
        let reading = take_remaining_results(&mut rx, &mut unconfirmed, &results_tx);
        let r = tokio::time::timeout(Duration::from_secs(5), reading)
            .await
            .unwrap_or_else(|_| Err(anyhow::anyhow!("timed out")));
        drop(results_tx);
        let mut got = Vec::new();
        while let Some(result) = results.recv().await {
            got.push((result.id, result.success));
        }
        (r, got, unconfirmed)
    }

    #[tokio::test]
    async fn every_sent_file_gets_its_own_answer() {
        let answers = vec![saved("1", true), saved("0", false), saved("2", true)];
        let (r, got, left) = read(&["0", "1", "2"], answers, true).await;
        assert!(r.is_ok());
        assert!(left.is_empty());
        let want = [("1", true), ("0", false), ("2", true)].map(|(id, ok)| (id.to_string(), ok));
        assert_eq!(got, want, "each answer passed on, a failure as a failure");
    }

    #[tokio::test]
    async fn a_missing_answer_is_a_failure() {
        let (r, got, left) = read(&["0", "1"], vec![saved("0", true)], false).await;
        assert!(r.is_err());
        assert_eq!(got, [("0".to_string(), true)]);
        assert_eq!(left, HashSet::from(["1".to_string()]));
    }

    #[tokio::test]
    async fn the_last_answer_before_the_close_counts() {
        // Queued, then the receiver closes at once: read, not lost
        let (r, got, _) = read(&["0"], vec![saved("0", true)], false).await;
        assert!(r.is_ok());
        assert_eq!(got, [("0".to_string(), true)]);
    }

    #[tokio::test]
    async fn duplicate_unexpected_and_malformed_answers_fail() {
        for (what, answers) in [
            ("duplicate", vec![saved("0", true), saved("0", true)]),
            ("unexpected", vec![saved("7", true)]),
            ("binary", vec![DataChannelMessage { is_string: false, data: Bytes::from_static(b"{}") }]),
            ("not json", vec![answer("saved")]),
            ("no success", vec![answer(r#"{"id":"0"}"#)]),
            ("success not a bool", vec![answer(r#"{"id":"0","success":"true"}"#)]),
        ] {
            let (r, got, _) = read(&["0", "1"], answers, true).await;
            assert!(r.is_err(), "{what}");
            assert!(got.len() <= 1 && got.iter().all(|(id, _)| id == "0"), "{what}: {got:?}");
        }
    }

    #[test]
    fn rtc_file_list_response_encoding() {
        let response = RTCFileListResponse::Pair {
            public_key: "123".to_string(),
        };

        let encoded = serde_json::to_string_pretty(&response).unwrap();

        assert_eq!(
            encoded,
            r#"{
  "status": "PAIR",
  "publicKey": "123"
}"#
        );

        let decoded: RTCFileListResponse = serde_json::from_str(&encoded).unwrap();

        assert_eq!(response, decoded);
    }

    /// The SDP's host candidates after gathering, under the filter set now.
    async fn host_candidates() -> usize {
        let (pc, _, _) = create_peer_connection(vec![], None).await.unwrap();
        pc.create_data_channel("probe", None).await.unwrap();
        let offer = pc.create_offer(None).await.unwrap();
        let mut gathered = pc.gathering_complete_promise().await;
        pc.set_local_description(offer).await.unwrap();
        let _ = gathered.recv().await;
        let sdp = pc.local_description().await.unwrap().sdp;
        pc.close().await.unwrap();
        sdp.matches("typ host").count()
    }

    #[tokio::test]
    async fn the_ip_filter_decides_which_host_candidates_are_offered() {
        set_ip_filter(Some(Arc::new(|_| false)));
        let none = host_candidates().await;
        set_ip_filter(Some(Arc::new(|ip: std::net::IpAddr| ip.is_loopback())));
        let loopback_only = host_candidates().await;
        set_ip_filter(None);
        let all = host_candidates().await;
        assert_eq!(none, 0, "everything filtered: no host candidate");
        assert!(loopback_only <= all, "{loopback_only} of {all}");
        assert!(all > 0, "unset: the default (every address)");
    }
}
