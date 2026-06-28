//! WS 帧的应用层加密(客户端侧)。对称于服务端 ws::frame。
//!
//! 出站:`serde_json::Value` → AES-256-GCM(master key + 随机 nonce)→
//! `nonce(12)||ct||tag` 二进制(`Message::Binary`)。入站:二进制 → 解密 → Value。
//! 信道(ws/wss)无关,内容已是密文;明文文本帧由收发层拒收。

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use rand::RngCore;
use serde_json::Value;

const NONCE_LEN: usize = 12;
const TAG_LEN: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameError {
    /// AES-GCM 解密 / tag 校验失败。
    Crypto,
    /// 长度不足 / JSON 结构错误。
    Format,
}

fn random_nonce() -> [u8; NONCE_LEN] {
    let mut n = [0u8; NONCE_LEN];
    rand::thread_rng().fill_bytes(&mut n);
    n
}

fn seal(plaintext: &[u8], key: &[u8; 32], nonce: &[u8; NONCE_LEN]) -> Result<Vec<u8>, FrameError> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let ct = cipher
        .encrypt(Nonce::from_slice(nonce), plaintext)
        .map_err(|_| FrameError::Crypto)?;
    let mut framed = Vec::with_capacity(NONCE_LEN + ct.len());
    framed.extend_from_slice(nonce);
    framed.extend_from_slice(&ct);
    Ok(framed)
}

fn open(framed: &[u8], key: &[u8; 32]) -> Result<Vec<u8>, FrameError> {
    if framed.len() < NONCE_LEN + TAG_LEN {
        return Err(FrameError::Format);
    }
    let (nonce, ct) = framed.split_at(NONCE_LEN);
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    cipher
        .decrypt(Nonce::from_slice(nonce), ct)
        .map_err(|_| FrameError::Crypto)
}

/// 把一条出站 JSON 值加密成二进制帧(随机 nonce)。
pub fn encode_client(v: &Value, key: &[u8; 32]) -> Result<Vec<u8>, FrameError> {
    let plaintext = serde_json::to_vec(v).map_err(|_| FrameError::Format)?;
    seal(&plaintext, key, &random_nonce())
}

/// 解密一条入站二进制帧为 JSON 值。
pub fn decode_server(bytes: &[u8], key: &[u8; 32]) -> Result<Value, FrameError> {
    let plaintext = open(bytes, key)?;
    serde_json::from_slice(&plaintext).map_err(|_| FrameError::Format)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> [u8; 32] {
        [7u8; 32]
    }

    /// 由服务端 ws::frame::print_vector 生成(key=[7;32], nonce=[3;12],
    /// 明文 {"type":"hello","token":"sk-h5st-abc"})。两端封装必须一致。
    const VECTOR: &[u8] = include_bytes!("../tests/fixtures/ws_frame_vector.bin");

    #[test]
    fn decodes_cross_end_vector() {
        let v = decode_server(VECTOR, &key()).unwrap();
        assert_eq!(v["type"], "hello");
        assert_eq!(v["token"], "sk-h5st-abc");
    }

    #[test]
    fn encode_then_decode_roundtrips() {
        let v = serde_json::json!({"type":"start_watch"});
        let framed = encode_client(&v, &key()).unwrap();
        let back = decode_server(&framed, &key()).unwrap();
        assert_eq!(back, v);
    }

    #[test]
    fn tampered_fails() {
        let mut framed = VECTOR.to_vec();
        let last = framed.len() - 1;
        framed[last] ^= 0xff;
        assert_eq!(decode_server(&framed, &key()), Err(FrameError::Crypto));
    }

    #[test]
    fn wrong_key_fails() {
        assert_eq!(decode_server(VECTOR, &[9u8; 32]), Err(FrameError::Crypto));
    }

    #[test]
    fn too_short_is_format() {
        assert_eq!(decode_server(&[0u8; 8], &key()), Err(FrameError::Format));
    }
}
