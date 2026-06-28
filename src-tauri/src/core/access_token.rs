//! 访问 Token 的解密侧(客户端)。对称于服务端 access_token::encrypt。
//!
//! `ak-base64url(salt(12)||ciphertext||tag)` → AES-256-GCM 解密 → {url,tok,uid,exp}。
//! master key 编译期注入(见 app.rs),salt 取自 Token 头 12 字节。
//!
//! 用户只持有 `ak-` 串(不透明票据),里面藏着真实地址与原始连接凭证;少了
//! 编译期内置的 master key,光有 Token 解不出明文。

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use base64::Engine;
use serde::{Deserialize, Serialize};

pub const PREFIX: &str = "ak-";

/// 解密后的明文负载。字段名固定(与服务端共享)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessPayload {
    pub url: String,
    pub tok: String,
    pub uid: String,
    #[serde(default)]
    pub exp: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccessTokenError {
    /// 不是 `ak-` 前缀 → 调用方应回退到旧的裸 token 流程。
    NotAccessToken,
    /// base64 / 长度 / JSON 结构错误。
    Format,
    /// AES-GCM 解密或 tag 校验失败(key 不匹配或密文被篡改)。
    Decrypt,
}

/// 是否是新版加密访问 Token(`ak-` 前缀)。
pub fn is_access_token(s: &str) -> bool {
    s.trim().starts_with(PREFIX)
}

/// 解密 `ak-` 串。封装:`base64url(salt(12) || ciphertext || tag)`。
pub fn decrypt_access_token(s: &str, key: &[u8; 32]) -> Result<AccessPayload, AccessTokenError> {
    let s = s.trim();
    let body = s.strip_prefix(PREFIX).ok_or(AccessTokenError::NotAccessToken)?;
    let framed = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(body)
        .map_err(|_| AccessTokenError::Format)?;
    // 至少要有 12B salt + 16B GCM tag。
    if framed.len() < 12 + 16 {
        return Err(AccessTokenError::Format);
    }
    let (salt, ct) = framed.split_at(12);
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let plaintext = cipher
        .decrypt(Nonce::from_slice(salt), ct)
        .map_err(|_| AccessTokenError::Decrypt)?;
    serde_json::from_slice(&plaintext).map_err(|_| AccessTokenError::Format)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> [u8; 32] {
        [7u8; 32]
    }

    /// 由服务端 `encrypt_access_token(payload, [7;32], [3;12])` 生成,见
    /// h5st-server/src/access_token.rs 的 print_vector 测试。两端封装必须一致。
    const VECTOR: &str = include_str!("../../tests/fixtures/access_token_vector.txt");

    #[test]
    fn roundtrip_via_known_vector() {
        let p = decrypt_access_token(VECTOR.trim(), &key()).unwrap();
        assert_eq!(p.url, "wss://real.example.com:8443/ws");
        assert_eq!(p.tok, "sk-h5st-abc");
        assert_eq!(p.uid, "u-001");
        assert_eq!(p.exp, 1790000000);
    }

    #[test]
    fn tampered_tag_fails() {
        let token = VECTOR.trim();
        let mut chars: Vec<char> = token.chars().collect();
        let last = chars.len() - 1;
        // 翻转最后一个字符(落在 GCM tag 区)→ 解密必败。
        chars[last] = if chars[last] == 'A' { 'B' } else { 'A' };
        let tampered: String = chars.into_iter().collect();
        assert_eq!(
            decrypt_access_token(&tampered, &key()),
            Err(AccessTokenError::Decrypt)
        );
    }

    #[test]
    fn wrong_key_fails() {
        let wrong = [9u8; 32];
        assert_eq!(
            decrypt_access_token(VECTOR.trim(), &wrong),
            Err(AccessTokenError::Decrypt)
        );
    }

    #[test]
    fn non_ak_prefix_is_not_access_token() {
        assert!(!is_access_token("sk-h5st-abc"));
        assert!(is_access_token("ak-whatever"));
        assert_eq!(
            decrypt_access_token("sk-h5st-abc", &key()),
            Err(AccessTokenError::NotAccessToken)
        );
    }
}
