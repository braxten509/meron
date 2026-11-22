//! Authenticates the peer and opens the encrypted setup-transfer channel.
//! The host and guest exchange framed messages using the displayed pairing code.
//! Both endpoints derive and verify their session keys before transferring settings.

use super::*;
use base64::Engine as _;

pub(super) fn new_code() -> String {
    let rng = SystemRandom::new();
    let mut bytes = [0u8; CODE_LEN];
    let _ = rng.fill(&mut bytes);
    bytes
        .iter()
        .map(|byte| CODE_ALPHABET[(*byte as usize) % CODE_ALPHABET.len()] as char)
        .collect()
}

/// A typed code as the computer made it: capitals, no dashes or spaces.
pub(super) fn normalize_code(code: &str) -> String {
    code.chars()
        .map(|c| c.to_ascii_uppercase())
        .filter(|c| CODE_ALPHABET.contains(&(*c as u8)))
        .collect()
}

fn spake_start(code: &str, host: bool) -> (Spake2<Ed25519Group>, Vec<u8>) {
    let password = Password::new(code.as_bytes());
    let (guest, host_id) = (Identity::new(GUEST_ID), Identity::new(HOST_ID));
    if host {
        Spake2::<Ed25519Group>::start_b(&password, &guest, &host_id)
    } else {
        Spake2::<Ed25519Group>::start_a(&password, &guest, &host_id)
    }
}

/// The channel's keys from SPAKE2's shared key.
fn derive(shared: &[u8], transcript: &[u8]) -> Result<Keys> {
    let prk = hkdf::Salt::new(hkdf::HKDF_SHA256, transcript).extract(shared);
    let mut out = [0u8; 96];
    prk.expand(&[b"meron setup copy v2"], Len(96))
        .and_then(|okm| okm.fill(&mut out))
        .map_err(|_| anyhow!("key derivation failed"))?;
    let mut keys = Keys {
        guest_to_host: [0; 32],
        host_to_guest: [0; 32],
        confirm: [0; 32],
    };
    keys.guest_to_host.copy_from_slice(&out[..32]);
    keys.host_to_guest.copy_from_slice(&out[32..64]);
    keys.confirm.copy_from_slice(&out[64..]);
    Ok(keys)
}

fn proof(keys: &Keys, who: &[u8], transcript: &[u8]) -> Vec<u8> {
    let key = hmac::Key::new(hmac::HMAC_SHA256, &keys.confirm);
    let mut message = who.to_vec();
    message.extend_from_slice(transcript);
    hmac::sign(&key, &message).as_ref().to_vec()
}

fn proof_ok(keys: &Keys, who: &[u8], transcript: &[u8], tag: &[u8]) -> bool {
    let key = hmac::Key::new(hmac::HMAC_SHA256, &keys.confirm);
    let mut message = who.to_vec();
    message.extend_from_slice(transcript);
    hmac::verify(&key, &message, tag).is_ok()
}

pub(super) fn b64_field(value: &Value, key: &str) -> Result<Vec<u8>> {
    let text = value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| fail("broken"))?;
    STANDARD.decode(text).map_err(|_| fail("broken"))
}

pub(super) async fn write_frame<W: AsyncWrite + Unpin>(writer: &mut W, bytes: &[u8]) -> Result<()> {
    writer
        .write_all(&(bytes.len() as u32).to_be_bytes())
        .await?;
    writer.write_all(bytes).await?;
    writer.flush().await?;
    Ok(())
}

pub(super) async fn read_frame<R: AsyncRead + Unpin>(
    reader: &mut R,
    max: usize,
) -> Result<Vec<u8>> {
    let mut len = [0u8; 4];
    reader.read_exact(&mut len).await?;
    let len = u32::from_be_bytes(len) as usize;
    if len > max {
        return Err(fail("broken"));
    }
    let mut buffer = vec![0u8; len];
    reader.read_exact(&mut buffer).await?;
    Ok(buffer)
}

async fn write_json<W: AsyncWrite + Unpin>(writer: &mut W, value: &Value) -> Result<()> {
    write_frame(writer, value.to_string().as_bytes()).await
}

async fn read_json<R: AsyncRead + Unpin>(reader: &mut R) -> Result<Value> {
    let bytes = read_frame(reader, MAX_HELLO).await?;
    serde_json::from_slice(&bytes).map_err(|_| fail("broken"))
}

/// The computer's side. `None`: the phone's code was wrong.
pub(super) async fn host_handshake(
    mut stream: TcpStream,
    code: &str,
    left: u32,
) -> Result<Option<Channel>> {
    let hello = read_json(&mut stream).await?;
    if hello.get("meron_setup").and_then(Value::as_u64) != Some(2) {
        return Err(fail("broken"));
    }
    let guest_message = b64_field(&hello, "spake")?;
    let (state, message) = spake_start(code, true);
    write_json(&mut stream, &json!({ "spake": STANDARD.encode(&message) })).await?;
    let transcript = [guest_message.as_slice(), message.as_slice()].concat();
    // A message that isn't a valid point can't come from a real phone.
    let shared = state.finish(&guest_message).map_err(|_| fail("broken"))?;
    let keys = derive(&shared, &transcript)?;
    let reply = read_json(&mut stream).await?;
    let tag = b64_field(&reply, "proof")?;
    if !proof_ok(&keys, b"guest", &transcript, &tag) {
        let _ = write_json(
            &mut stream,
            &json!({ "error": "wrong_code", "left": left.saturating_sub(1) }),
        )
        .await;
        return Ok(None);
    }
    write_json(
        &mut stream,
        &json!({ "proof": STANDARD.encode(proof(&keys, b"host", &transcript)) }),
    )
    .await?;
    Ok(Some(Channel {
        stream,
        send: keys.host_to_guest,
        recv: keys.guest_to_host,
    }))
}

/// The phone's side.
pub(super) async fn guest_handshake(mut stream: TcpStream, code: &str) -> Result<Channel> {
    let (state, message) = spake_start(code, false);
    write_json(
        &mut stream,
        &json!({ "meron_setup": 2, "spake": STANDARD.encode(&message) }),
    )
    .await?;
    let answer = read_json(&mut stream).await?;
    let host_message = b64_field(&answer, "spake")?;
    let transcript = [message.as_slice(), host_message.as_slice()].concat();
    let shared = state.finish(&host_message).map_err(|_| fail("broken"))?;
    let keys = derive(&shared, &transcript)?;
    write_json(
        &mut stream,
        &json!({ "proof": STANDARD.encode(proof(&keys, b"guest", &transcript)) }),
    )
    .await?;
    let reply = read_json(&mut stream)
        .await
        .map_err(|_| fail("wrong_code"))?;
    if reply.get("error").and_then(Value::as_str) == Some("wrong_code") {
        return Err(fail(
            if reply.get("left").and_then(Value::as_u64) == Some(0) {
                "too_many_attempts"
            } else {
                "wrong_code"
            },
        ));
    }
    let tag = b64_field(&reply, "proof")?;
    if !proof_ok(&keys, b"host", &transcript, &tag) {
        return Err(fail("wrong_code"));
    }
    Ok(Channel {
        stream,
        send: keys.guest_to_host,
        recv: keys.host_to_guest,
    })
}
