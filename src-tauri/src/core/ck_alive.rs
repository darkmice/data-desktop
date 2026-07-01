//! CK liveness verification via JD `productFavoriteList`.
//!
//! This is deliberately separate from the order flow: the endpoint is simple,
//! needs login + h5st, and gives a clear `code=0` / `code=3` signal without
//! touching the risky submit-order path.

use serde_json::{Map, Value};

use crate::core::ck::{self, Credential};

pub const FUNCTION_ID: &str = "productFavoriteList";
pub const REQUEST_APPID: &str = "plus_business";
pub const SIGN_APPID: &str = "m_core";
pub const SIGN_APP_ID: &str = "bd265";
const VERIFY_URL: &str = "https://api.m.jd.com/client.action";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyResult {
    pub alive: bool,
    pub favorite_count: usize,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CkAliveSignature {
    pub h5st: String,
    pub client: String,
    pub client_version: String,
}

#[async_trait::async_trait]
pub trait RemoteCkAliveSigner: Send + Sync {
    /// Request h5st from the server signer. The desktop client must not produce
    /// h5st locally; it only uses the returned params to send the CK-bearing JD
    /// verification request.
    async fn sign_ck_alive(&self, body_str: &str, t: i64) -> Result<CkAliveSignature, String>;
}

pub fn favorite_body_string() -> String {
    let mut body = Map::new();
    body.insert("page".into(), Value::String("1".into()));
    body.insert("pagesize".into(), Value::String("10".into()));
    body.insert("sortType".into(), Value::String("time_desc".into()));
    body.insert("filterType".into(), Value::String("ALL".into()));
    body.insert("externalVersion".into(), Value::String("26.4.28".into()));
    Value::Object(body).to_string()
}

pub async fn verify<S: RemoteCkAliveSigner>(
    signer: &S,
    http: &reqwest::Client,
    cred: &Credential,
    now_ms: i64,
) -> Result<VerifyResult, String> {
    let cookies = ck::parse_cookies(&cred.cookie_str);
    let missing = ck::missing_required_keys(&cookies);
    if !missing.is_empty() {
        return Ok(VerifyResult {
            alive: false,
            favorite_count: 0,
            message: format!("Cookie 缺少必要字段: {}", missing.join("、")),
        });
    }

    let body = favorite_body_string();
    let signed = signer.sign_ck_alive(&body, now_ms).await?;
    let cookie_header = ck::cookie_header(&cookies);

    let params = [
        ("appid", REQUEST_APPID.to_string()),
        ("functionId", FUNCTION_ID.to_string()),
        ("body", body),
        ("client", signed.client),
        ("clientVersion", signed.client_version),
        ("loginType", "2".to_string()),
        ("t", now_ms.to_string()),
        ("scval", String::new()),
        ("h5st", signed.h5st),
    ];

    let resp = http
        .post(VERIFY_URL)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("User-Agent", "jdapp;iPhone;15.8.30;;;M/5.0")
        .header("Referer", "https://paipai.m.jd.com/")
        .header("Cookie", cookie_header)
        .form(&params)
        .send()
        .await
        .map_err(|e| format!("CK 验活请求失败: {e}"))?;

    let value: Value = resp
        .json()
        .await
        .map_err(|e| format!("CK 验活响应解析失败: {e}"))?;
    interpret_response(&value)
}

pub fn interpret_response(value: &Value) -> Result<VerifyResult, String> {
    let code = value
        .get("code")
        .map(value_to_code)
        .unwrap_or_else(String::new);

    if code == "3" {
        return Ok(VerifyResult {
            alive: false,
            favorite_count: 0,
            message: "CK失效(code:3)".into(),
        });
    }

    if code == "0" {
        let favorites = value
            .pointer("/result/favoriteList")
            .and_then(Value::as_array)
            .ok_or_else(|| "CK 验活未知响应: code=0 但缺少 favoriteList".to_string())?;
        return Ok(VerifyResult {
            alive: true,
            favorite_count: favorites.len(),
            message: format!("CK有效,收藏夹{}条", favorites.len()),
        });
    }

    Err(format!("CK 验活未知状态(code:{code})"))
}

fn value_to_code(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn favorite_body_keeps_expected_shape() {
        assert_eq!(
            favorite_body_string(),
            r#"{"page":"1","pagesize":"10","sortType":"time_desc","filterType":"ALL","externalVersion":"26.4.28"}"#
        );
    }

    #[test]
    fn response_code_zero_is_alive_even_when_empty() {
        let r = interpret_response(&json!({"code":0,"result":{"favoriteList":[]}})).unwrap();
        assert!(r.alive);
        assert_eq!(r.favorite_count, 0);
    }

    #[test]
    fn response_code_three_is_expired() {
        let r = interpret_response(&json!({"code":3,"echo":"no access"})).unwrap();
        assert!(!r.alive);
    }
}
