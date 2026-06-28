//! Tauri glue: app state, commands exposed to the frontend, and the auto-submit
//! reaction to `sku_hit`. Service address + token come from a config file /
//! settings; the UI shows only a connection light.

use std::sync::{Arc, OnceLock};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::{mpsc, Mutex};

use crate::core::ck::{self, Credential};
use crate::core::history::{Filter, HistoryStore, OrderRecord};
use crate::core::notify::{
    Channel, DingTalkChannel, FeishuChannel, NotifyEvent, Notifier, OrderOutcome,
};
use crate::core::notify::channel::Subscriptions;
use crate::core::notify::event::EventKind;
use crate::core::order::{self, OrderResult, ReqwestHttp};
use crate::core::rules::{self, Rule};
use crate::ws_client::{WsClient, WsEvent};

/// Persisted client config (lives in a settings file, not the UI).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    /// WSS endpoint, e.g. "wss://127.0.0.1:8443/ws". Hidden from normal users.
    pub server_url: String,
    pub token: String,
    /// Accept self-signed dev cert.
    #[serde(default)]
    pub insecure_tls: bool,
    #[serde(default)]
    pub auto_submit: bool,
    /// 旧版单一钉钉配置。**保留仅为向后兼容**:旧 config 升级后,若 `notify_channels`
    /// 为空而这两个字段非空,启动时会自动迁移成一个钉钉渠道(见 `build_notifier`)。
    /// 新 UI 写入 `notify_channels`,不再写这里。
    #[serde(default)]
    pub dingtalk_webhook: String,
    #[serde(default)]
    pub dingtalk_secret: String,
    /// 多渠道通知配置(钉钉/飞书/…)。每个渠道含开关、webhook、加签密钥、订阅的事件。
    #[serde(default)]
    pub notify_channels: Vec<NotifyChannelConfig>,
    /// Signing recipe: "paipai" (paipai_h5/rdv6s + raw body) or "codex"
    /// (bd265/cc85b by functionId + sha256 body). Default "codex" — 与浏览器真实
    /// 抓包的 h5st(中间段 bd265)一致。
    #[serde(default = "default_recipe")]
    pub sign_recipe: String,
    /// UI theme: "dark" | "light". Default "dark".
    #[serde(default = "default_theme")]
    pub theme: String,
    // NOTE: categories are no longer a client concern. They live on the server
    // (a global catalog + per-token enablement) and are pushed to the client over
    // WS after auth. Any `categories` field in a persisted config from an older
    // build is ignored (serde drops unknown fields).
}

fn default_recipe() -> String {
    // 默认 codex:sign_app_id 留空,服务端按 functionId 映射(getCurrentOrder→bd265,
    // submitOrder→cc85b),与浏览器真实抓包的 h5st(中间段 bd265)一致。
    "codex".into()
}

fn default_theme() -> String {
    "dark".into()
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            // Local plaintext WS by default (server runs with H5ST_PLAINTEXT on
            // 127.0.0.1, no cert to trust). Switch to wss:// for remote deploys.
            server_url: "ws://127.0.0.1:8443/ws".into(),
            // Pre-filled local-test token (matches the running plaintext server).
            // Config is not persisted, so this saves re-typing on every launch.
            // Remove (set to String::new()) before any non-local build.
            token: option_env!("PAIPAI_DEV_TOKEN").unwrap_or("").into(),
            insecure_tls: true,
            auto_submit: false,
            dingtalk_webhook: String::new(),
            dingtalk_secret: String::new(),
            notify_channels: Vec::new(),
            sign_recipe: default_recipe(),
            theme: default_theme(),
        }
    }
}

/// 单个通知渠道的持久化配置。`kind` 决定用哪个 [`Channel`] 实现。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotifyChannelConfig {
    /// 渠道类型:"dingtalk" | "feishu"。
    pub kind: String,
    /// 总开关。关 → 该渠道不构建。
    #[serde(default)]
    pub enabled: bool,
    pub webhook: String,
    /// 加签密钥(可选,留空则不加签)。
    #[serde(default)]
    pub secret: String,
    /// 订阅的事件 key(见 EventKind::as_key):order_success / order_failed /
    /// hit_alert / status_change。
    #[serde(default)]
    pub events: Vec<String>,
}

/// 事件 key → EventKind。未知 key 返回 None(被忽略)。
fn event_kind_from_key(key: &str) -> Option<EventKind> {
    match key {
        "order_success" => Some(EventKind::OrderSuccess),
        "order_failed" => Some(EventKind::OrderFailed),
        "hit_alert" => Some(EventKind::HitAlert),
        "status_change" => Some(EventKind::StatusChange),
        _ => None,
    }
}

/// 把一个渠道配置的 events 列表转成订阅集合。
fn subs_from_keys(keys: &[String]) -> Subscriptions {
    Subscriptions::new(keys.iter().filter_map(|k| event_kind_from_key(k)))
}

/// 根据 config 构建通知分发器。
///
/// 渠道来源优先级:
/// 1. `notify_channels`(新 UI 写入):逐个构建启用、有 webhook、有订阅的渠道。
/// 2. 若 `notify_channels` 为空但旧 `dingtalk_webhook` 非空 → **迁移**:构造一个钉钉
///    渠道,默认订阅全部四类事件(等价旧行为 + 失败/命中/状态也推)。
///
/// 返回的 Notifier 只含「会真正发」的渠道;没有可用渠道时它是空的(notify 直接 no-op)。
fn build_notifier(cfg: &AppConfig, client: reqwest::Client) -> Notifier {
    let mut channels: Vec<Box<dyn Channel>> = Vec::new();

    if !cfg.notify_channels.is_empty() {
        for c in &cfg.notify_channels {
            if !c.enabled || c.webhook.trim().is_empty() {
                continue;
            }
            let subs = subs_from_keys(&c.events);
            if subs.is_empty() {
                continue; // 没订阅任何事件 = 等于禁用,不构建
            }
            match c.kind.as_str() {
                "dingtalk" => channels.push(Box::new(DingTalkChannel::new(
                    c.webhook.clone(),
                    c.secret.clone(),
                    subs,
                ))),
                "feishu" => channels.push(Box::new(FeishuChannel::new(
                    c.webhook.clone(),
                    c.secret.clone(),
                    subs,
                ))),
                other => tracing::warn!(kind = other, "未知通知渠道类型,忽略"),
            }
        }
    } else if !cfg.dingtalk_webhook.trim().is_empty() {
        // 旧配置迁移:单钉钉 → 订阅全部四类事件。
        let subs = Subscriptions::new([
            EventKind::OrderSuccess,
            EventKind::OrderFailed,
            EventKind::HitAlert,
            EventKind::StatusChange,
        ]);
        channels.push(Box::new(DingTalkChannel::new(
            cfg.dingtalk_webhook.clone(),
            cfg.dingtalk_secret.clone(),
            subs,
        )));
    }

    Notifier::new(client, channels)
}

pub struct AppState {
    pub config: Mutex<AppConfig>,
    pub creds: Mutex<Vec<Credential>>,
    pub rules: Mutex<Vec<Rule>>,
    pub active_idx: Mutex<usize>,
    pub ws: Mutex<Option<WsClient>>,
    pub http: ReqwestHttp,
    /// Serializes order attempts so concurrent submits (two sku_hit events, or a
    /// hit racing a manual submit) cannot interleave their read-modify-write of
    /// creds/active_idx. Held for the whole attempt.
    pub order_lock: Mutex<()>,
    /// Local order-history store (SQLite). Initialized in `setup` once the app
    /// data dir is known; `None` until then (and if opening failed → history
    /// degrades gracefully, the rest of the app keeps working).
    pub history: OnceLock<HistoryStore>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            config: Mutex::new(AppConfig::default()),
            creds: Mutex::new(Vec::new()),
            rules: Mutex::new(Vec::new()),
            active_idx: Mutex::new(0),
            ws: Mutex::new(None),
            http: ReqwestHttp {
                client: reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(10))
                    .build()
                    .expect("reqwest"),
            },
            order_lock: Mutex::new(()),
            history: OnceLock::new(),
        }
    }

    /// The history store if it opened successfully.
    fn history(&self) -> Option<&HistoryStore> {
        self.history.get()
    }
}

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn now_secs() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// 客户端内置 master key,用于解密访问 Token(`ak-`)。生产由 GitHub Actions
/// 注入 `DATA_ACCESS_KEY`(64 hex);缺省为本地开发占位 key,**切勿用于生产**。
/// 占位 key = 32 字节全 0x07,与服务端本地 `H5ST_ACCESS_KEY` 对齐即可联调。
fn master_key() -> [u8; 32] {
    const DEV_PLACEHOLDER: &str =
        "0707070707070707070707070707070707070707070707070707070707070707";
    let hex_str = option_env!("DATA_ACCESS_KEY").unwrap_or(DEV_PLACEHOLDER);
    let mut k = [7u8; 32];
    if let Ok(bytes) = hex::decode(hex_str.trim()) {
        let n = bytes.len().min(32);
        k[..n].copy_from_slice(&bytes[..n]);
    }
    k
}

// ---- 持久化辅助 ----
// 凭证/规则/配置变更后写入本地 KV(settings 表),启动时由 load_persisted 读回。
// 写失败只记日志,不阻断命令。KV 键:"config" / "creds" / "rules"。

const KV_CONFIG: &str = "config";
const KV_CREDS: &str = "creds";
const KV_RULES: &str = "rules";

fn persist<T: Serialize>(state: &AppState, key: &str, value: &T) {
    let Some(store) = state.history() else { return };
    if let Ok(s) = serde_json::to_string(value) {
        if let Err(e) = store.kv_set(key, &s) {
            tracing::warn!(error = %e, key, "persist failed");
        }
    }
}

/// 启动时把本地 KV 里的 config/creds/rules 读回内存状态。
fn load_persisted(state: &AppState) {
    let Some(store) = state.history() else { return };
    if let Ok(Some(s)) = store.kv_get(KV_CONFIG) {
        // Categories are no longer stored client-side; a `categories` field from
        // an older persisted config is simply ignored by serde.
        if let Ok(mut cfg) = serde_json::from_str::<AppConfig>(&s) {
            // 兜底:历史配置里 server_url 可能被存成空串(旧版本遗留),空串会让
            // connect 时报 "HTTP format error: empty string"。空则回退编译期默认地址。
            if cfg.server_url.trim().is_empty() {
                cfg.server_url = AppConfig::default().server_url;
            }
            *state.config.blocking_lock() = cfg;
        }
    }
    if let Ok(Some(s)) = store.kv_get(KV_CREDS) {
        if let Ok(creds) = serde_json::from_str::<Vec<Credential>>(&s) {
            *state.creds.blocking_lock() = creds;
        }
    }
    if let Ok(Some(s)) = store.kv_get(KV_RULES) {
        if let Ok(rules) = serde_json::from_str::<Vec<Rule>>(&s) {
            *state.rules.blocking_lock() = rules;
        }
    }
}

/// 运行目录配置文件名。放在可执行文件**同目录**,用于部署时配置 WS 服务地址,
/// 无需重新打包、也不必进 AppData 改库。文件内容(只读 server_url):
///   { "server_url": "ws://1.2.3.4:8443/ws" }
const RUNTIME_CONFIG_FILE: &str = "config.json";

/// 运行目录配置文件结构(目前只放 WS 地址)。其余字段忽略。
#[derive(Deserialize)]
struct RuntimeConfigFile {
    #[serde(default)]
    server_url: String,
}

/// 从可执行文件同目录读 `config.json`,把其中的 `server_url` 作为**默认值**应用。
///
/// 语义(用户可覆盖):仅当当前 server_url 还是编译期默认地址(即用户没在界面里
/// 显式设过、也没持久化过别的地址)时,才用文件里的地址覆盖。这样:
///   - 部署方改运行目录 config.json → 新装/未配置的客户端默认连该地址;
///   - 用户在界面改过地址(已落库非默认)→ 尊重用户,文件不覆盖。
fn apply_runtime_config_file(state: &AppState) {
    let exe_dir = match std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())) {
        Some(d) => d,
        None => return,
    };
    let path = exe_dir.join(RUNTIME_CONFIG_FILE);
    let Ok(content) = std::fs::read_to_string(&path) else {
        return; // 文件不存在 = 不配置,用默认/已存值
    };
    let Ok(file_cfg) = serde_json::from_str::<RuntimeConfigFile>(&content) else {
        tracing::warn!(path = %path.display(), "运行目录 config.json 解析失败,忽略");
        return;
    };
    let url = file_cfg.server_url.trim();
    if url.is_empty() {
        return;
    }
    let mut cfg = state.config.blocking_lock();
    // 只在「仍是编译期默认地址」时用文件值覆盖 —— 用户已显式改过就不动。
    if cfg.server_url == AppConfig::default().server_url {
        tracing::info!(server_url = url, "应用运行目录 config.json 的服务地址");
        cfg.server_url = url.to_string();
    }
}

// ---- commands ----

#[tauri::command]
pub async fn get_config(state: State<'_, Arc<AppState>>) -> Result<AppConfig, String> {
    Ok(state.config.lock().await.clone())
}

#[tauri::command]
pub async fn save_config(state: State<'_, Arc<AppState>>, config: AppConfig) -> Result<(), String> {
    let recipe = crate::ws_client::SignRecipe::from_str(&config.sign_recipe);
    *state.config.lock().await = config.clone();
    // If already connected, apply the recipe live (no reconnect needed).
    if let Some(ws) = state.ws.lock().await.as_ref() {
        ws.set_recipe(recipe);
    }
    persist(&state, KV_CONFIG, &config);
    Ok(())
}

#[tauri::command]
pub async fn get_credentials(state: State<'_, Arc<AppState>>) -> Result<Vec<Credential>, String> {
    Ok(state.creds.lock().await.clone())
}

#[tauri::command]
pub async fn import_credential(
    state: State<'_, Arc<AppState>>,
    name: String,
    cookie_str: String,
) -> Result<usize, String> {
    let cookies = ck::parse_cookies(&cookie_str);

    // 导入即校验:缺关键 key 直接拒绝,只提示缺哪些(不说原因)。
    let missing = ck::missing_required_keys(&cookies);
    if !missing.is_empty() {
        return Err(format!("Cookie 缺少必要字段：{}", missing.join("、")));
    }

    let valid = ck::has_pt_key(&cookies);
    let count = cookies.len();
    {
        let mut creds = state.creds.lock().await;
        creds.push(Credential {
            name: if name.is_empty() { "未命名".into() } else { name },
            cookie_str,
            valid,
        });
    }
    persist(&state, KV_CREDS, &*state.creds.lock().await);
    Ok(count)
}

#[tauri::command]
pub async fn delete_credential(state: State<'_, Arc<AppState>>, index: usize) -> Result<(), String> {
    {
        let mut creds = state.creds.lock().await;
        if index < creds.len() {
            creds.remove(index);
            // Keep active_idx pointing at the same logical credential: shift down
            // if we removed at or before it, and clamp into range.
            let mut idx = state.active_idx.lock().await;
            if *idx > index || *idx >= creds.len() {
                *idx = idx.saturating_sub(1);
            }
            if creds.is_empty() {
                *idx = 0;
            }
        }
    }
    persist(&state, KV_CREDS, &*state.creds.lock().await);
    Ok(())
}

#[tauri::command]
pub async fn use_credential(state: State<'_, Arc<AppState>>, index: usize) -> Result<(), String> {
    *state.active_idx.lock().await = index;
    Ok(())
}

#[tauri::command]
pub async fn get_rules(state: State<'_, Arc<AppState>>) -> Result<Vec<Rule>, String> {
    Ok(state.rules.lock().await.clone())
}

/// Mirror rules to the server, best-effort. The client is the source of truth;
/// a dead/closed connection must NOT fail the local save. On send failure (e.g.
/// "channel closed" after a silent disconnect) we drop the stale handle and log,
/// but return Ok so local persistence still happens.
async fn mirror_rules_best_effort(state: &AppState, rules: &[Rule]) {
    let mut guard = state.ws.lock().await;
    if let Some(ws) = guard.as_ref() {
        if let Err(e) = ws.set_rules(&serde_json::to_value(rules).unwrap_or(json!([]))) {
            tracing::warn!(error = %e, "rule mirror failed; dropping connection");
            *guard = None; // stale handle → drop so future ops skip the mirror
        }
    }
}

/// Save rules locally and (if connected) mirror to the server. Local save is
/// authoritative; server mirror is best-effort and never fails the command.
#[tauri::command]
pub async fn save_rules(state: State<'_, Arc<AppState>>, rules: Vec<Rule>) -> Result<(), String> {
    *state.rules.lock().await = rules.clone();
    mirror_rules_best_effort(&state, &rules).await;
    persist(&state, KV_RULES, &rules);
    Ok(())
}

#[tauri::command]
pub async fn reenable_rule(state: State<'_, Arc<AppState>>, rule_id: String) -> Result<(), String> {
    // Mutate and snapshot under one lock so a concurrent record_success cannot
    // interleave and produce a stale mirror.
    let rules = {
        let mut rules = state.rules.lock().await;
        if let Some(r) = rules.iter_mut().find(|r| r.id == rule_id) {
            r.reenable();
        }
        rules.clone()
    };
    mirror_rules_best_effort(&state, &rules).await;
    persist(&state, KV_RULES, &rules);
    Ok(())
}

/// Connect to the server, authenticate, and start receiving events.
#[tauri::command]
pub async fn connect(app: AppHandle, state: State<'_, Arc<AppState>>) -> Result<(), String> {
    let cfg = state.config.lock().await.clone();
    if cfg.token.trim().is_empty() {
        return Err("未配置研究测试 Token".into());
    }

    // 访问 Token(ak-)→ 解密拿真实地址 + 原始凭证;否则按旧裸 token + server_url 走。
    // 错误分级:解密失败=Token 无效;本地判过期=Token 过期;url 连不通=联系管理员更新。
    let (resolved_url, resolved_tok) = if crate::core::access_token::is_access_token(&cfg.token) {
        match crate::core::access_token::decrypt_access_token(&cfg.token, &master_key()) {
            Ok(p) => {
                if p.exp != 0 && p.exp <= now_secs() {
                    return Err("访问 Token 已过期,请联系管理员更新".into());
                }
                (p.url, p.tok)
            }
            Err(_) => return Err("访问 Token 无效,请联系管理员重新获取".into()),
        }
    } else {
        (cfg.server_url.clone(), cfg.token.clone())
    };

    let (ev_tx, mut ev_rx) = mpsc::unbounded_channel::<WsEvent>();
    let recipe = crate::ws_client::SignRecipe::from_str(&cfg.sign_recipe);
    // wss + 后端 https 无证书:insecure 恒 true(复用 ws_client 的 NoVerify 验证器)。
    // url 连不通 → 提示联系管理员更新 Token(而非暴露底层网络错误)。
    let ws = match WsClient::connect(&resolved_url, &resolved_tok, true, recipe, master_key(), ev_tx).await {
        Ok(ws) => ws,
        Err(e) => {
            tracing::warn!(error = %e, "WS 连接失败");
            return Err("无法连接服务,请联系管理员更新 Token".into());
        }
    };
    *state.ws.lock().await = Some(ws);

    // Event pump → forward to frontend + drive auto-submit.
    let app2 = app.clone();
    let state2 = state.inner().clone();
    tokio::spawn(async move {
        while let Some(ev) = ev_rx.recv().await {
            eprintln!("[ws-event] {ev:?}");
            match ev {
                WsEvent::Connected => emit(&app2, "conn", json!({"status":"connected"})),
                WsEvent::HelloOk => {
                    emit(&app2, "conn", json!({"status":"authed"}));
                    // Push this client's system info once, right after auth, so the
                    // server's monitoring page can show who/where. collect() shells
                    // out to platform tools (blocking) → run it on the blocking pool
                    // so it never stalls a tokio worker / the event pump. Best-effort.
                    let info = tokio::task::spawn_blocking(|| {
                        serde_json::to_value(crate::core::sysinfo::collect()).unwrap_or(json!({}))
                    })
                    .await
                    .unwrap_or_else(|_| json!({}));
                    if let Some(ws) = state2.ws.lock().await.as_ref() {
                        let _ = ws.send_client_info(&info);
                    }
                }
                WsEvent::Disconnected => {
                    // Drop the dead handle so later commands see None (and skip the
                    // server mirror) instead of sending into a closed channel.
                    *state2.ws.lock().await = None;
                    emit(&app2, "conn", json!({"status":"disconnected"}));
                }
                WsEvent::Log(m) => emit(&app2, "log", json!({"msg": m})),
                WsEvent::Categories(items) => {
                    // Server-pushed enabled categories → frontend renders read-only.
                    emit(&app2, "categories", json!({"items": items}))
                }
                WsEvent::WatchParams(p) => {
                    // Server-pushed scan params → frontend shows read-only.
                    emit(&app2, "watch_params", p)
                }
                WsEvent::Error(m) => emit(&app2, "log", json!({"msg": format!("错误: {m}")})),
                WsEvent::Heartbeat => {
                    // Monitor liveness pulse → forward to the UI's "alive" indicator.
                    // Empty payload; the frontend stamps its own receive time.
                    emit(&app2, "heartbeat", json!({}));
                }
                WsEvent::AuthFailed(m) => {
                    // Token rejected → drop the handle. Phrase around the研究 Token,
                    // never around "connecting to a server".
                    *state2.ws.lock().await = None;
                    let reason = match m.as_str() {
                        "unknown token" => "研究测试 Token 无效,请在设置中检查后重试",
                        "token expired" => "研究测试 Token 已过期,请在设置中更换后重试",
                        "token revoked" => "研究测试 Token 已停用,请在设置中更换后重试",
                        _ => "研究测试 Token 校验未通过,请在设置中检查后重试",
                    };
                    emit(&app2, "conn", json!({"status":"disconnected","reason": reason}));
                    emit(&app2, "log", json!({"msg": reason}));
                }
                WsEvent::Kicked { code, message: _ } => {
                    // Server-initiated close. The user must NEVER learn there is a
                    // server that can ban/kick/limit — surface as a JD-风控 /
                    // research-feature-unavailable problem. `code` only picks wording;
                    // the raw server `message` is intentionally ignored (never shown).
                    *state2.ws.lock().await = None;
                    let reason = match code.as_str() {
                        // 封禁:伪装成 JD 风控,不暴露是服务端封的。
                        "banned" => "当前 IP 已被风控,请更换网络后重试",
                        // 连接数上限:多开自家客户端时引导关掉其它的,不提"服务端限制"。
                        "conn_limit" => "研究功能已在其它设备使用,请关闭后重试",
                        // kicked / 其它:统一为"研究功能暂不可用"。
                        _ => "研究功能暂时不可用,请稍后重试",
                    };
                    emit(&app2, "conn", json!({"status":"disconnected","reason": reason}));
                    emit(&app2, "log", json!({"msg": reason}));
                }
                WsEvent::SkuHit(sku) => {
                    emit(&app2, "sku_hit", sku.clone());
                    // Run the order flow in its own task so a slow submit does
                    // not block the event pump (signed/log/status events keep
                    // flowing). Concurrency is bounded by `order_lock`.
                    let app3 = app2.clone();
                    let state3 = state2.clone();
                    tokio::spawn(async move {
                        on_sku_hit(app3, state3, sku).await;
                    });
                }
            }
        }
    });
    Ok(())
}

fn emit(app: &AppHandle, event: &str, payload: Value) {
    let _ = app.emit(event, payload);
}

/// Emit a client-originated operation log to the local UI AND mirror it up to the
/// server (for the admin live view). `level` is "info"|"hit"|"err". The server
/// mirror is best-effort: a dead/missing connection is silently skipped (the
/// local emit always happens). Used for logs that ORIGINATE on the client (order
/// flow); server-originated logs already live on the server and aren't echoed back.
async fn relay_log(app: &AppHandle, state: &AppState, level: &str, msg: &str) {
    emit(app, "log", json!({ "msg": msg }));
    if let Some(ws) = state.ws.lock().await.as_ref() {
        let _ = ws.send_op_log(now_ms(), level, msg);
    }
}

/// Persist one order attempt to the local history store and notify the frontend.
/// Called from both submit entry points (manual + auto) with the final result of
/// a multi-credential rotation. SKU identity / name / quality come from the call
/// site (the most authoritative source), falling back to the result's own SKU.
/// Storage failures are logged, never fatal to the submit flow.
#[allow(clippy::too_many_arguments)]
fn record_order(
    app: &AppHandle,
    state: &AppState,
    result: &OrderResult,
    trigger: &str,
    inspect_id: &str,
    youpin_id: &str,
    short_name: &str,
    quality: &str,
    image: &str,
    rule_id: &str,
) {
    let Some(store) = state.history() else { return };
    let rec = OrderRecord::new(
        now_ms(),
        result.success,
        trigger,
        if inspect_id.is_empty() { &result.inspect_sku_id } else { inspect_id },
        if youpin_id.is_empty() { &result.youpin_sku_id } else { youpin_id },
        short_name,
        &result.price,
        quality,
        image,
        &result.order_id,
        &result.credential,
        &result.error,
        rule_id,
    );
    match store.insert(&rec) {
        Ok(saved) => {
            emit(app, "order_recorded", serde_json::to_value(&saved).unwrap_or(json!({})));
            // Mirror the order record up to the server as an audit record.
            // Best-effort: try_lock so a contended/missing connection never blocks
            // or fails the local record path.
            if let Ok(guard) = state.ws.try_lock() {
                if let Some(ws) = guard.as_ref() {
                    let report = json!({
                        "ts": saved.created_at,
                        "success": saved.status == "success",
                        "trigger": saved.trigger,
                        "inspect_sku_id": saved.inspect_sku_id,
                        "youpin_sku_id": saved.youpin_sku_id,
                        "short_name": saved.short_name,
                        "price": saved.price,
                        "order_id": saved.order_id,
                        "error": saved.error,
                        "rule_id": saved.rule_id,
                    });
                    let _ = ws.send_order_report(&report);
                }
            }
        }
        Err(e) => emit(app, "log", json!({"msg": format!("记录保存失败: {e}")})),
    }
}

/// React to a matched new SKU: if auto-submit is on, run the order flow.
async fn on_sku_hit(app: AppHandle, state: Arc<AppState>, sku: Value) {
    let cfg = state.config.lock().await.clone();
    let notifier = build_notifier(&cfg, state.http.client.clone());

    let inspect_id = sku["inspect_sku_id"].as_str().unwrap_or("").to_string();
    let youpin_id = sku["youpin_sku_id"].as_str().unwrap_or("").to_string();
    let rule_id = sku["matched_rule_id"].as_str().unwrap_or("").to_string();
    let short_name = sku["short_name"].as_str().unwrap_or("").to_string();
    let quality = sku["quality_name"].as_str().unwrap_or("").to_string();
    let image = sku["main_image"].as_str().unwrap_or("").to_string();
    let price = sku["price"].as_str().map(|s| s.to_string()).unwrap_or_else(|| {
        sku["price"].as_f64().map(|f| f.to_string()).unwrap_or_default()
    });

    // 未开启自动提交:这是「命中告警」场景 —— 推渠道提醒人工介入,然后结束(不自动下单)。
    if !cfg.auto_submit {
        if inspect_id.is_empty() || youpin_id.is_empty() {
            return;
        }
        let link = order::product_link(&youpin_id, &inspect_id);
        notifier
            .notify(NotifyEvent::HitAlert {
                product_name: short_name,
                price,
                quality,
                inspect_sku_id: inspect_id,
                youpin_sku_id: youpin_id,
                link,
            })
            .await;
        return;
    }

    if inspect_id.is_empty() || youpin_id.is_empty() {
        return;
    }

    relay_log(&app, &state, "hit", &format!("命中: {short_name} → 开始提交")).await;

    let ws = match state.ws.lock().await.as_ref().cloned() {
        Some(w) => w,
        None => return,
    };

    // Serialize the whole attempt: snapshot creds/idx, run, write back — all
    // under order_lock so concurrent attempts cannot stomp each other's state.
    let _order_guard = state.order_lock.lock().await;
    let creds = state.creds.lock().await.clone();
    let active = *state.active_idx.lock().await;

    let mut logs: Vec<String> = Vec::new();
    let (result, updated, new_active) = order::order_with_rotation(
        &ws, &state.http, creds, active, &inspect_id, &youpin_id, now_ms(), &mut logs,
    )
    .await;
    for m in logs {
        emit(&app, "log", json!({"msg": m}));
    }

    // Persist rotation outcome (still under order_lock).
    {
        let new_creds = updated;
        let mut creds_guard = state.creds.lock().await;
        let len = new_creds.len();
        *creds_guard = new_creds;
        let mut idx_guard = state.active_idx.lock().await;
        *idx_guard = if len == 0 { 0 } else { new_active.min(len - 1) };
    }

    if result.success {
        // quota: used += 1; auto-stop if reached, mirror to server — both under
        // one rules lock so a concurrent reenable cannot interleave.
        let (stopped, rules_now) = {
            let mut rules = state.rules.lock().await;
            let stopped = rules::record_success(&mut rules, &rule_id);
            (stopped, rules.clone())
        };
        ws.set_rules(&serde_json::to_value(&rules_now).unwrap_or(json!([]))).ok();
        persist(&state, KV_RULES, &rules_now);
        relay_log(&app, &state, "hit", &format!(
            "提交成功 #{} ¥{}{}", result.order_id, result.price,
            if stopped { " (规则已达上限,自动停止)" } else { "" }
        )).await;
        let link = order::product_link(&youpin_id, &inspect_id);
        notifier
            .notify(NotifyEvent::OrderSuccess(OrderOutcome {
                product_name: short_name.clone(),
                price: result.price.clone(),
                quality: quality.clone(),
                inspect_sku_id: inspect_id.clone(),
                youpin_sku_id: youpin_id.clone(),
                link,
                trigger: "auto".into(),
                order_id: result.order_id.clone(),
                error: String::new(),
                credential_name: result.credential.clone(),
            }))
            .await;
        // 规则因达配额自动停止 → 关键状态变化,另推一条运维提醒。
        if stopped {
            notifier
                .notify(NotifyEvent::StatusChange {
                    title: "规则已达配额,自动停止".into(),
                    detail: format!("商品 {short_name} 对应的规则已达上限并停止"),
                })
                .await;
        }
    } else {
        relay_log(&app, &state, "err", &format!("提交失败: {}", result.error)).await;
        let link = order::product_link(&youpin_id, &inspect_id);
        notifier
            .notify(NotifyEvent::OrderFailed(OrderOutcome {
                product_name: short_name.clone(),
                price: result.price.clone(),
                quality: quality.clone(),
                inspect_sku_id: inspect_id.clone(),
                youpin_sku_id: youpin_id.clone(),
                link,
                trigger: "auto".into(),
                order_id: String::new(),
                error: result.error.clone(),
                credential_name: result.credential.clone(),
            }))
            .await;
    }

    // Persist to local history (success or failure), then notify the frontend.
    record_order(
        &app, &state, &result, "auto", &inspect_id, &youpin_id, &short_name, &quality, &image, &rule_id,
    );
}

/// Start the monitor: push config + rules, then start_watch.
///
/// Both categories AND scan params are SERVER concerns now: the frontend sends
/// only the selected category KEYS (chosen from the server-pushed enabled set).
/// The server resolves keys → full params from its catalog (intersected with the
/// token's enabled set) and pulls the scan knobs (page/interval/threads) from
/// this token's server-side watch params. The client carries neither.
#[tauri::command]
pub async fn start_watch(
    state: State<'_, Arc<AppState>>,
    category_keys: Vec<String>,
) -> Result<(), String> {
    let ws = state.ws.lock().await.as_ref().cloned().ok_or("研究功能尚未就绪")?;
    let rules = state.rules.lock().await.clone();

    if category_keys.is_empty() {
        return Err("请至少选择一个关注品类".into());
    }

    ws.set_rules(&serde_json::to_value(&rules).unwrap_or(json!([])))?;
    ws.watch_config(&json!({ "category_keys": category_keys }))?;
    ws.start_watch()?;
    Ok(())
}

#[tauri::command]
pub async fn stop_watch(state: State<'_, Arc<AppState>>) -> Result<(), String> {
    if let Some(ws) = state.ws.lock().await.as_ref() {
        ws.stop_watch()?;
    }
    Ok(())
}

/// Manual submit for a specific inspect/youpin pair.
#[tauri::command]
pub async fn manual_submit(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    inspect_id: String,
    youpin_id: String,
) -> Result<Value, String> {
    let ws = state.ws.lock().await.as_ref().cloned().ok_or("研究功能尚未就绪")?;
    // Same serialization + write-back-clamp as auto submit.
    let _order_guard = state.order_lock.lock().await;
    let creds = state.creds.lock().await.clone();
    let active = *state.active_idx.lock().await;
    let mut logs = Vec::new();
    let (result, updated, new_active) = order::order_with_rotation(
        &ws, &state.http, creds, active, &inspect_id, &youpin_id, now_ms(),
        &mut logs,
    )
    .await;
    {
        let len = updated.len();
        *state.creds.lock().await = updated;
        *state.active_idx.lock().await = if len == 0 { 0 } else { new_active.min(len - 1) };
    }

    // Persist to local history (manual trigger, no rule). Manual submit has no
    // product name/quality/image (user enters only the SKU ids), so those stay empty.
    record_order(
        &app, &state, &result, "manual", &inspect_id, &youpin_id, "", "", "", "",
    );

    // 渠道推送:手动提交与自动命中走同一套 Notifier,成交成功/失败都推(渠道按各自
    // 订阅过滤)。手动提交无商品名/成色,这些字段留空(模板会用占位/略过)。
    {
        let cfg = state.config.lock().await.clone();
        let notifier = build_notifier(&cfg, state.http.client.clone());
        let link = order::product_link(&youpin_id, &inspect_id);
        let outcome = OrderOutcome {
            product_name: String::new(),
            price: result.price.clone(),
            quality: String::new(),
            inspect_sku_id: inspect_id.clone(),
            youpin_sku_id: youpin_id.clone(),
            link,
            trigger: "manual".into(),
            order_id: result.order_id.clone(),
            error: result.error.clone(),
            credential_name: result.credential.clone(),
        };
        let event = if result.success {
            NotifyEvent::OrderSuccess(outcome)
        } else {
            NotifyEvent::OrderFailed(outcome)
        };
        notifier.notify(event).await;
    }

    Ok(json!({
        "success": result.success, "order_id": result.order_id,
        "price": result.price, "error": result.error,
        "credential": result.credential, "logs": logs
    }))
}

/// 下单记录:分页查询(filter = all|success|failed)。
#[tauri::command]
pub async fn get_orders(
    state: State<'_, Arc<AppState>>,
    filter: String,
    page: i64,
    page_size: i64,
) -> Result<Value, String> {
    let store = state.history().ok_or("记录功能不可用")?;
    let limit = page_size.clamp(1, 200);
    let offset = (page.max(1) - 1) * limit;
    let page = store
        .list(Filter::from_str(&filter), limit, offset)
        .map_err(|e| e.to_string())?;
    Ok(json!({ "items": page.items, "total": page.total }))
}

/// 下单记录:汇总统计(总数/成功/失败)。
#[tauri::command]
pub async fn get_order_stats(state: State<'_, Arc<AppState>>) -> Result<Value, String> {
    let store = state.history().ok_or("记录功能不可用")?;
    let st = store.stats().map_err(|e| e.to_string())?;
    Ok(serde_json::to_value(st).unwrap_or(json!({})))
}

/// 下单记录:清空全部。
#[tauri::command]
pub async fn clear_orders(state: State<'_, Arc<AppState>>) -> Result<(), String> {
    let store = state.history().ok_or("记录功能不可用")?;
    store.clear().map_err(|e| e.to_string())
}

/// Measure latency to JD: TCP/HTTP ping to item page + paipai API round-trip.
#[tauri::command]
pub async fn ping_jd(state: State<'_, Arc<AppState>>) -> Result<Value, String> {
    let client = &state.http.client;
    let net = timed(|| async {
        client.get("https://item.m.jd.com/").send().await.map(|_| ()).map_err(|e| e.to_string())
    })
    .await;
    let api = timed(|| async {
        client
            .post("https://api.m.jd.com/api")
            .form(&[("appid", "paipai_inspect"), ("functionId", "x"), ("body", "{}")])
            .send()
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    })
    .await;
    Ok(json!({"net_ms": net, "api_ms": api}))
}

async fn timed<F, Fut>(f: F) -> i64
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<(), String>>,
{
    let start = std::time::Instant::now();
    match f().await {
        Ok(_) => start.elapsed().as_millis() as i64,
        Err(_) => -1,
    }
}

pub fn run() {
    let state = Arc::new(AppState::new());
    tauri::Builder::default()
        .plugin(tauri_plugin_store::Builder::new().build())
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            get_config,
            save_config,
            get_credentials,
            import_credential,
            delete_credential,
            use_credential,
            get_rules,
            save_rules,
            reenable_rule,
            connect,
            start_watch,
            stop_watch,
            manual_submit,
            get_orders,
            get_order_stats,
            clear_orders,
            ping_jd,
        ])
        .setup(|app| {
            // Open the local order-history DB under the app data dir. Failure is
            // non-fatal: history degrades, the rest of the app runs.
            let state = app.state::<Arc<AppState>>();
            match app.path().app_data_dir() {
                Ok(dir) => {
                    if let Err(e) = std::fs::create_dir_all(&dir) {
                        tracing::warn!(error = %e, "create app_data_dir failed; history disabled");
                    } else {
                        match HistoryStore::open(dir.join("app.db")) {
                            Ok(store) => {
                                let _ = state.history.set(store);
                                // 读回上次持久化的 config / 凭证 / 规则。
                                load_persisted(&state);
                            }
                            Err(e) => tracing::warn!(error = %e, "open local db failed; persistence disabled"),
                        }
                    }
                }
                Err(e) => tracing::warn!(error = %e, "no app_data_dir; history disabled"),
            }
            // 在读回持久化配置之后,应用运行目录 config.json 的服务地址(仅当用户
            // 未显式改过地址时作为默认)。即使上面的 DB 打开失败也执行,保证部署方
            // 能通过运行目录文件配置地址。
            apply_runtime_config_file(&state);
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn master_key_constant_is_32_bytes() {
        // 编译期注入或本地占位,任一路径都必须产出 32 字节 key。
        let k = master_key();
        assert_eq!(k.len(), 32);
    }

    #[test]
    fn dev_placeholder_decrypts_cross_end_vector() {
        // 本地占位 key(全 0x07)= 服务端测试向量用的 key,所以内置的解密链路
        // 能还原 Task 1/2 的固定向量(回归:确认 master_key 接线没断)。
        let token = include_str!("../tests/fixtures/access_token_vector.txt").trim();
        let p = crate::core::access_token::decrypt_access_token(token, &master_key()).unwrap();
        assert_eq!(p.url, "wss://real.example.com:8443/ws");
        assert_eq!(p.tok, "sk-h5st-abc");
    }
}
