//! 通知渠道:把中性的 [`RenderedMessage`] 投递到一个具体的 IM 机器人。
//!
//! 新增一个渠道(企业微信、Telegram、邮件…)= 实现一个 [`Channel`]:把
//! `RenderedMessage` 转成该渠道的报文、POST 出去、按需加签。订阅过滤(该渠道关心
//! 哪几类事件)由 trait 默认逻辑统一处理,实现者只管「怎么发」。

use std::collections::HashSet;

use async_trait::async_trait;
use base64::Engine;
use hmac::{Hmac, Mac};
use serde_json::json;
use sha2::Sha256;

use super::event::EventKind;
use super::template::RenderedMessage;

type HmacSha256 = Hmac<Sha256>;

/// 一个可投递的通知渠道。`Send + Sync` 以便放进共享的 [`super::Notifier`]。
#[async_trait]
pub trait Channel: Send + Sync {
    /// 渠道名(日志用,如 "dingtalk" / "feishu")。
    fn name(&self) -> &'static str;
    /// 该渠道是否订阅了某事件类型。
    fn subscribes(&self, kind: EventKind) -> bool;
    /// 投递一条已渲染的消息。`client` 复用全局 reqwest 客户端。
    async fn send(&self, client: &reqwest::Client, msg: &RenderedMessage) -> anyhow::Result<()>;
}

/// 订阅集合:渠道勾选的事件类型。空集 = 不接收任何事件(等于禁用)。
#[derive(Debug, Clone, Default)]
pub struct Subscriptions(HashSet<EventKind>);

impl Subscriptions {
    pub fn new(kinds: impl IntoIterator<Item = EventKind>) -> Self {
        Self(kinds.into_iter().collect())
    }
    pub fn contains(&self, kind: EventKind) -> bool {
        self.0.contains(&kind)
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// 当前 unix 毫秒。注入而非内部取,便于测试加签确定性(测试传固定值)。
fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// `timestamp + "\n" + secret` 的 HMAC-SHA256,base64 编码。钉钉与飞书的加签算法
/// 完全一致(区别只在「签名放 URL 还是 body」),共用此函数。
fn hmac_sign(timestamp: i64, secret: &str) -> String {
    let string_to_sign = format!("{timestamp}\n{secret}");
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key length");
    mac.update(string_to_sign.as_bytes());
    let code = mac.finalize().into_bytes();
    base64::engine::general_purpose::STANDARD.encode(code)
}

// ───────────────────────────── 钉钉 ─────────────────────────────

/// 钉钉自定义机器人。加签(可选):secret 非空时,按时间戳算签名拼到 URL。
pub struct DingTalkChannel {
    webhook: String,
    secret: String,
    subs: Subscriptions,
}

impl DingTalkChannel {
    pub fn new(webhook: String, secret: String, subs: Subscriptions) -> Self {
        Self { webhook, secret, subs }
    }

    /// 计算实际请求 URL:有 secret 则追加 `&timestamp=..&sign=..`(URL 编码)。
    fn signed_url(&self, now: i64) -> String {
        if self.secret.trim().is_empty() {
            return self.webhook.clone();
        }
        let sign = urlencoding::encode(&hmac_sign(now, &self.secret)).into_owned();
        let sep = if self.webhook.contains('?') { '&' } else { '?' };
        format!("{}{}timestamp={}&sign={}", self.webhook, sep, now, sign)
    }
}

#[async_trait]
impl Channel for DingTalkChannel {
    fn name(&self) -> &'static str {
        "dingtalk"
    }
    fn subscribes(&self, kind: EventKind) -> bool {
        self.subs.contains(kind)
    }
    async fn send(&self, client: &reqwest::Client, msg: &RenderedMessage) -> anyhow::Result<()> {
        let url = self.signed_url(now_ms());
        // 钉钉用 markdown:title 作通知摘要标题,text 是 markdown 正文(对齐文档)。
        let body = json!({
            "msgtype": "markdown",
            "markdown": { "title": msg.title, "text": msg.to_markdown() },
        });
        let resp = client.post(&url).json(&body).send().await?;
        let status = resp.status();
        if !status.is_success() {
            anyhow::bail!("dingtalk http {status}");
        }
        // 钉钉签名错/关键词缺失/被限流时仍返回 HTTP 200,但 body 里 errcode!=0
        // (如 310000 "sign not match")。必须读 body 才能发现配置错误,否则会被
        // 当成发送成功。成功是 {"errcode":0,"errmsg":"ok"}。
        match resp.json::<serde_json::Value>().await {
            Ok(v) => {
                if let Some(code) = v.get("errcode").and_then(|c| c.as_i64()) {
                    if code != 0 {
                        let m = v.get("errmsg").and_then(|m| m.as_str()).unwrap_or("unknown");
                        anyhow::bail!("dingtalk errcode {code}: {m}");
                    }
                }
            }
            // 解析失败:无法确认是否真的送达,记一条警告而非静默当成功。
            Err(e) => tracing::warn!(error = %e, "dingtalk 响应解析失败,无法确认送达"),
        }
        Ok(())
    }
}

// ───────────────────────────── 飞书 ─────────────────────────────

/// 飞书自定义机器人。加签(可选):secret 非空时,签名放进请求 body 的 `sign` 字段
/// (与钉钉「放 URL」不同)。
pub struct FeishuChannel {
    webhook: String,
    secret: String,
    subs: Subscriptions,
}

impl FeishuChannel {
    pub fn new(webhook: String, secret: String, subs: Subscriptions) -> Self {
        Self { webhook, secret, subs }
    }
}

#[async_trait]
impl Channel for FeishuChannel {
    fn name(&self) -> &'static str {
        "feishu"
    }
    fn subscribes(&self, kind: EventKind) -> bool {
        self.subs.contains(kind)
    }
    async fn send(&self, client: &reqwest::Client, msg: &RenderedMessage) -> anyhow::Result<()> {
        // 飞书 text 消息:{ msg_type, content: { text }, [timestamp, sign] }
        let mut body = json!({
            "msg_type": "text",
            "content": { "text": msg.to_plain_text() }
        });
        if !self.secret.trim().is_empty() {
            let now = now_ms() / 1000; // 飞书时间戳是「秒」
            let sign = hmac_sign(now, &self.secret);
            body["timestamp"] = json!(now.to_string());
            body["sign"] = json!(sign);
        }
        let resp = client.post(&self.webhook).json(&body).send().await?;
        let status = resp.status();
        if !status.is_success() {
            anyhow::bail!("feishu http {status}");
        }
        // 飞书即使 200 也可能在 body 里返回 code!=0(如签名错误),尽力检测。
        match resp.json::<serde_json::Value>().await {
            Ok(v) => {
                if let Some(code) = v.get("code").and_then(|c| c.as_i64()) {
                    if code != 0 {
                        let m = v.get("msg").and_then(|m| m.as_str()).unwrap_or("unknown");
                        anyhow::bail!("feishu code {code}: {m}");
                    }
                }
            }
            // 解析失败:无法确认是否真的送达,记一条警告而非静默当成功。
            Err(e) => tracing::warn!(error = %e, "feishu 响应解析失败,无法确认送达"),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_secret_url_unchanged() {
        let ch = DingTalkChannel::new(
            "https://oapi.dingtalk.com/robot/send?access_token=abc".into(),
            String::new(),
            Subscriptions::new([EventKind::OrderSuccess]),
        );
        assert_eq!(ch.signed_url(123), "https://oapi.dingtalk.com/robot/send?access_token=abc");
    }

    #[test]
    fn with_secret_appends_timestamp_and_sign() {
        let ch = DingTalkChannel::new(
            "https://oapi.dingtalk.com/robot/send?access_token=abc".into(),
            "SECabc".into(),
            Subscriptions::new([EventKind::OrderSuccess]),
        );
        let url = ch.signed_url(1700000000000);
        assert!(url.contains("&timestamp=1700000000000"));
        assert!(url.contains("&sign="));
        // 签名经 URL 编码:base64 的 '+' '/' '=' 不应原样出现。
        let sign_part = url.split("&sign=").nth(1).unwrap();
        assert!(!sign_part.contains('+'));
        assert!(!sign_part.contains(' '));
    }

    #[test]
    fn subscription_filtering() {
        let ch = DingTalkChannel::new(
            "http://x".into(),
            String::new(),
            Subscriptions::new([EventKind::OrderSuccess, EventKind::OrderFailed]),
        );
        assert!(ch.subscribes(EventKind::OrderSuccess));
        assert!(ch.subscribes(EventKind::OrderFailed));
        assert!(!ch.subscribes(EventKind::HitAlert));
        assert!(!ch.subscribes(EventKind::StatusChange));
    }

    #[test]
    fn hmac_is_deterministic_and_base64() {
        let a = hmac_sign(1700000000000, "secret");
        let b = hmac_sign(1700000000000, "secret");
        assert_eq!(a, b);
        // 不同时间戳 → 不同签名。
        assert_ne!(a, hmac_sign(1700000000001, "secret"));
        // base64 字符集。
        assert!(base64::engine::general_purpose::STANDARD.decode(&a).is_ok());
    }
}
