//! WS client to the API server. Owns the connection, authenticates with the
//! token, forwards rules/config, receives sku_hit/log/status, and implements
//! the order flow's `Signer` by round-tripping `sign`↔`signed` over WS.

use std::collections::HashMap;
use std::sync::Arc;

use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio_tungstenite::Connector;
use tungstenite::client::IntoClientRequest;
use tungstenite::Message;

use crate::core::order::{Signer, ORDER_CLIENT_PLATFORM};

/// Events surfaced from the WS connection to the app layer.
#[derive(Debug, Clone)]
pub enum WsEvent {
    HelloOk,
    Connected,
    Disconnected,
    Log(String),
    SkuHit(Value),
    /// Server pushed this token's enabled categories (full params). Sent right
    /// after auth and whenever the admin changes them. The client renders them
    /// read-only and lets the user pick which to scan.
    Categories(Value),
    /// Server pushed this token's resolved scan params (page/interval/threads).
    /// Sent after auth and on admin change. Read-only on the client.
    WatchParams(Value),
    Error(String),
    /// Liveness pulse from the server's monitor task (one per completed scan
    /// cycle). Empty by design — it only means "monitor is alive". The app layer
    /// stamps a timestamp the UI uses for its "running · last active N s ago"
    /// indicator and stall detection.
    Heartbeat,
    /// Auth failure (bad/expired/revoked token). Carries a human message; the
    /// server closes the connection right after. Surfaced distinctly so the UI
    /// can prompt at the connection point instead of burying it in the log.
    AuthFailed(String),
    /// The server force-closed this connection (admin kick / IP ban / connection
    /// limit). `code` is "kicked"|"banned"|"conn_limit"; the socket closes right
    /// after. Surfaced distinctly so the UI can explain WHY the connection ended.
    Kicked { code: String, message: String },
}

/// Handle to the live WS connection. Cloneable; sending is via an mpsc to the
/// writer task.
#[derive(Clone)]
pub struct WsClient {
    out_tx: mpsc::UnboundedSender<Message>,
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<Result<String, String>>>>>,
    sign_seq: Arc<std::sync::atomic::AtomicU64>,
    /// Master key for WS frame encryption (every business frame is AES-GCM
    /// encrypted to a binary frame; plaintext is rejected by the server).
    key: Arc<[u8; 32]>,
}

impl WsClient {
    /// Connect to `wss_url` (e.g. wss://host:8443/ws), authenticate with
    /// `token`, and spawn read/write tasks. `events` receives surfaced events.
    pub async fn connect(
        wss_url: &str,
        token: &str,
        insecure: bool,
        key: [u8; 32],
        events: mpsc::UnboundedSender<WsEvent>,
    ) -> Result<Self, String> {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

        let req = wss_url.into_client_request().map_err(|e| e.to_string())?;
        let connector = if insecure {
            let tls = rustls::ClientConfig::builder()
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(NoVerify))
                .with_no_client_auth();
            Some(Connector::Rustls(Arc::new(tls)))
        } else {
            None
        };

        let (ws, _) = tokio_tungstenite::connect_async_tls_with_config(req, None, false, connector)
            .await
            .map_err(|e| e.to_string())?;
        let (mut write, mut read) = ws.split();

        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Message>();
        let pending: Arc<Mutex<HashMap<String, oneshot::Sender<Result<String, String>>>>> =
            Arc::new(Mutex::new(HashMap::new()));

        // Writer task.
        tokio::spawn(async move {
            while let Some(msg) = out_rx.recv().await {
                if write.send(msg).await.is_err() {
                    break;
                }
            }
        });

        // Authenticate. 首帧也加密成二进制(服务端只认加密帧)。
        let hello = json!({"type":"hello","token":token});
        let hello_frame = crate::ws_frame::encode_client(&hello, &key)
            .map_err(|_| "encode hello failed".to_string())?;
        out_tx
            .send(Message::Binary(hello_frame))
            .map_err(|e| e.to_string())?;

        // Reader task. 入站业务帧都是加密二进制;解密失败/明文一律跳过。
        let pending_r = pending.clone();
        let ev = events.clone();
        let key_arc: Arc<[u8; 32]> = Arc::new(key);
        let key_r = key_arc.clone();
        tokio::spawn(async move {
            let _ = ev.send(WsEvent::Connected);
            while let Some(Ok(msg)) = read.next().await {
                let Message::Binary(bytes) = msg else { continue };
                let v: Value = match crate::ws_frame::decode_server(&bytes, &key_r) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                match v["type"].as_str().unwrap_or("") {
                    "hello_ok" => {
                        let _ = ev.send(WsEvent::HelloOk);
                    }
                    "log" => {
                        let _ = ev.send(WsEvent::Log(v["msg"].as_str().unwrap_or("").to_string()));
                    }
                    "sku_hit" => {
                        let _ = ev.send(WsEvent::SkuHit(v.clone()));
                    }
                    "heartbeat" => {
                        let _ = ev.send(WsEvent::Heartbeat);
                    }
                    "categories" => {
                        // Forward the items array as-is for the frontend to render.
                        let _ = ev.send(WsEvent::Categories(v["items"].clone()));
                    }
                    "watch_params" => {
                        // Forward the full params object (page_from/page_to/interval/max_threads).
                        let _ = ev.send(WsEvent::WatchParams(v.clone()));
                    }
                    "signed" => {
                        let id = v["id"].as_str().unwrap_or("").to_string();
                        if let Some(tx) = pending_r.lock().await.remove(&id) {
                            let _ = tx.send(Ok(v["h5st"].as_str().unwrap_or("").to_string()));
                        }
                    }
                    "sign_error" => {
                        let id = v["id"].as_str().unwrap_or("").to_string();
                        let msg = v["message"].as_str().unwrap_or("sign error").to_string();
                        if let Some(tx) = pending_r.lock().await.remove(&id) {
                            let _ = tx.send(Err(msg));
                        }
                    }
                    "error" => {
                        let code = v["code"].as_str().unwrap_or("");
                        let msg = v["message"].as_str().unwrap_or("").to_string();
                        // auth_failed / auth_error → distinct event (the server
                        // closes right after); everything else stays a log.
                        if code == "auth_failed" || code == "auth_error" {
                            let _ = ev.send(WsEvent::AuthFailed(msg));
                        } else {
                            let _ = ev.send(WsEvent::Error(msg));
                        }
                    }
                    "kicked" => {
                        // Governance close: admin kick / IP ban / connection limit.
                        // The server closes right after; surface why.
                        let _ = ev.send(WsEvent::Kicked {
                            code: v["code"].as_str().unwrap_or("").to_string(),
                            message: v["message"].as_str().unwrap_or("").to_string(),
                        });
                    }
                    _ => {}
                }
            }
            let _ = ev.send(WsEvent::Disconnected);
        });

        Ok(Self {
            out_tx,
            pending,
            sign_seq: Arc::new(std::sync::atomic::AtomicU64::new(1)),
            key: key_arc,
        })
    }

    fn send(&self, v: Value) -> Result<(), String> {
        // 每条出站业务帧加密成二进制(服务端拒明文)。
        let framed = crate::ws_frame::encode_client(&v, &self.key)
            .map_err(|_| "encode frame failed".to_string())?;
        self.out_tx
            .send(Message::Binary(framed))
            .map_err(|e| e.to_string())
    }

    pub fn set_rules(&self, rules: &Value) -> Result<(), String> {
        self.send(json!({"type":"set_rules","rules":rules}))
    }

    pub fn watch_config(&self, cfg: &Value) -> Result<(), String> {
        let mut v = cfg.clone();
        v["type"] = json!("watch_config");
        self.send(v)
    }

    pub fn start_watch(&self) -> Result<(), String> {
        self.send(json!({"type":"start_watch"}))
    }

    pub fn stop_watch(&self) -> Result<(), String> {
        self.send(json!({"type":"stop_watch"}))
    }

    /// Push this client's system/environment info to the server (sent once right
    /// after auth). Telemetry — best-effort, callers ignore the result.
    pub fn send_client_info(&self, info: &Value) -> Result<(), String> {
        let mut v = info.clone();
        v["type"] = json!("client_info");
        self.send(v)
    }

    /// Stream one operation-log line to the server for the admin live view.
    /// Telemetry — best-effort.
    pub fn send_op_log(&self, ts: i64, level: &str, msg: &str) -> Result<(), String> {
        self.send(json!({"type":"op_log","ts":ts,"level":level,"msg":msg}))
    }

    /// Report one order attempt to the server (persisted as an audit record).
    /// Telemetry — best-effort.
    pub fn send_order_report(&self, report: &Value) -> Result<(), String> {
        let mut v = report.clone();
        v["type"] = json!("order_report");
        self.send(v)
    }
}

#[async_trait::async_trait]
impl Signer for WsClient {
    async fn sign(&self, function_id: &str, body_str: &str, t: i64) -> Result<String, String> {
        let id = format!(
            "s{}",
            self.sign_seq.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        );
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id.clone(), tx);

        // 唯一一套签名 = 测试工具(h5st-probe)那套:sign_app_id 留空,服务端按
        // functionId 映射(getCurrentOrder→bd265 / submitOrder→cc85b),body 走 sha256
        // (raw_body=false)。曾经的第二套 paipai_h5(rdv6s/raw body)已废弃删除——
        // 服务端也强制规范化为这套,前端固定 codex。`t` 传入使 h5st 内层 t = 外层请求 t。
        if let Err(e) = self.send(json!({
            "type": "sign",
            "id": id,
            "function_id": function_id,
            "body": body_str,
            "appid": "m_core",
            // 与请求参数 clientVersion 一致(真实抓包为 3.0.8)。
            "client_version": "3.0.8",
            // ⭐ 关键:必须把 client(navigator.platform)传给服务端签名,且与下单 form 的
            // client 字段【完全一致】。服务端据此设 navigator.platform(烘进 seg7 指纹);
            // 若不传,服务端 fallback=iPhone,而 form 写死 MacIntel → seg7 自相矛盾 → JD 601。
            // 这是客户端一直 601 的真因(h5st-probe 全链 MacIntel 一致才成功)。
            // 与 order::build_params 的 ("client","MacIntel") 必须保持同值。
            "client": ORDER_CLIENT_PLATFORM,
            "sign_app_id": "",
            "raw_body": false,
            "t": t,
        })) {
            self.pending.lock().await.remove(&id);
            return Err(e);
        }

        match tokio::time::timeout(std::time::Duration::from_secs(15), rx).await {
            Ok(Ok(r)) => r,
            Ok(Err(_)) => Err("sign channel closed".into()),
            Err(_) => {
                self.pending.lock().await.remove(&id);
                Err("sign timeout".into())
            }
        }
    }
}

#[derive(Debug)]
struct NoVerify;
impl rustls::client::danger::ServerCertVerifier for NoVerify {
    fn verify_server_cert(
        &self,
        _: &rustls::pki_types::CertificateDer<'_>,
        _: &[rustls::pki_types::CertificateDer<'_>],
        _: &rustls::pki_types::ServerName<'_>,
        _: &[u8],
        _: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &rustls::pki_types::CertificateDer<'_>,
        _: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }
    fn verify_tls13_signature(
        &self,
        _: &[u8],
        _: &rustls::pki_types::CertificateDer<'_>,
        _: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }
    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        use rustls::SignatureScheme::*;
        vec![RSA_PKCS1_SHA256, ECDSA_NISTP256_SHA256, ED25519, RSA_PSS_SHA256]
    }
}
