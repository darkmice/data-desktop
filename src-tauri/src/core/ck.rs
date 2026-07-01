//! Credential (凭证) parsing and real-param extraction.
//! Ported from Python `order.py` `_get_real_params` / `_extract_*`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// 凭证状态。区分三种"不可用/需关注":
/// - `RiskControlled`(风控 / 601):间歇性,慢点用同一把 CK 重试可能就成 → 前端橙色,
///   用户可手动「解除」改回 Active 再试。
/// - `Expired`(过期 / 302 / 登录失效):CK 真失效,需换新 CK → 前端红色。也允许手动
///   「解除」(比如用户刚更新了同名 CK 想重置状态)。
/// - `Disabled`:用户手动禁用,不参与提交流程,但仍保留在列表中并可继续验活。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CredStatus {
    /// 可用。
    Active,
    /// 触发风控(601),间歇性,可重试。
    RiskControlled,
    /// 登录态失效(302 / CK 过期),需换 CK。
    Expired,
    /// 用户手动禁用,不参与提交流程。
    Disabled,
}

impl Default for CredStatus {
    fn default() -> Self {
        CredStatus::Active
    }
}

/// A stored credential set (one "凭证" = one cookie jar + remark name).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Credential {
    /// User-defined remark name (e.g. "6715").
    pub name: String,
    /// Raw cookie string as pasted from the browser.
    pub cookie_str: String,
    /// 当前状态。提交流程据此判断是否跳过。
    /// 旧数据无此字段时默认 Active;同时兼容旧的布尔 `valid` 字段(见 valid()/反序列化)。
    #[serde(default)]
    pub status: CredStatus,
    /// 最近一次 CK 存活验证时间(ms epoch)。0 表示尚未验证/旧数据。
    #[serde(default)]
    pub last_alive_check_ms: i64,
    /// 最近一次存活验证结果。None 表示验证异常(签名/网络/未知响应),不据此禁用 CK。
    #[serde(default)]
    pub last_alive_ok: Option<bool>,
    /// 最近一次存活验证的人类可读结果,仅用于界面/日志提示。
    #[serde(default)]
    pub last_alive_message: String,
    /// 兼容旧持久化格式的布尔字段:旧数据只有 `valid`,无 `status`。仅用于读旧数据时
    /// 迁移(见 migrate_legacy_valid),新代码一律用 `status`。
    #[serde(default = "default_true", skip_serializing)]
    pub valid: bool,
}

fn default_true() -> bool {
    true
}

impl Credential {
    /// 是否可直接显示为正常可用。
    pub fn is_active(&self) -> bool {
        self.status == CredStatus::Active
    }

    /// 是否可参与提交流程。风控态允许重试;过期/手动禁用不参与。
    pub fn can_order(&self) -> bool {
        matches!(self.status, CredStatus::Active | CredStatus::RiskControlled)
    }

    /// 读旧持久化数据时调用:若 status 缺省为 Active 但旧 `valid=false`,迁移为 Expired
    /// (旧 valid=false 多是轮换时因 601/302 置的,保守归为需关注的失效态)。新数据无影响。
    pub fn migrate_legacy_valid(&mut self) {
        if self.status == CredStatus::Active && !self.valid {
            self.status = CredStatus::Expired;
        }
        // 统一让 valid 反映是否可提交,避免两字段不一致(虽然 valid 不再被读)。
        self.valid = self.can_order();
    }
}

/// Browser params the order body/params need, extracted from cookies.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RealParams {
    pub device_uuid: String,
    pub address_id: String,
    pub location_id: String,
    pub eid_token: String,
}

/// Parse a cookie string ("k1=v1; k2=v2") into an ordered map.
pub fn parse_cookies(cookie_str: &str) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for pair in cookie_str.split(';') {
        let pair = pair.trim();
        if let Some((k, v)) = pair.split_once('=') {
            let k = k.trim();
            if !k.is_empty() {
                map.insert(k.to_string(), v.trim().to_string());
            }
        }
    }
    map
}

/// True if the cookie jar carries a login token.
pub fn has_pt_key(cookies: &BTreeMap<String, String>) -> bool {
    cookies.contains_key("pt_key")
}

/// 导入准入只校验登录态 cookie。
///
/// JD app 端导出的可下单 CK 可能只有 `pt_key`/`pt_pin`/`pwdt_id`/`unpl` 这一类
/// 登录字段,不带 `__jda`、`shshshfpa/b/x`、`unionwsws` 等 H5/PC 指纹字段。
/// 下单 body 里的设备/地址字段有运行时兜底,所以导入阶段不能把这些非登录字段当
/// 成硬门槛,否则会误拒真实可用 CK。
pub const REQUIRED_KEYS: &[&str] = &["pt_key", "pt_pin"];

/// 返回 CK 中缺失的关键 key(按 REQUIRED_KEYS 顺序)。空 = 齐全。
/// 只判断"在不在"且值非空——不判断值是否正确。
pub fn missing_required_keys(cookies: &BTreeMap<String, String>) -> Vec<&'static str> {
    REQUIRED_KEYS
        .iter()
        .filter(|k| {
            cookies
                .get(**k)
                .map(|v| v.trim().is_empty())
                .unwrap_or(true)
        })
        .copied()
        .collect()
}

/// Extract the device/address params, mirroring Python `_get_real_params`.
pub fn extract_real_params(cookies: &BTreeMap<String, String>) -> RealParams {
    let device_uuid = cookies
        .get("visitkey")
        .cloned()
        .filter(|s| !s.is_empty())
        .or_else(|| {
            cookies
                .get("__jda")
                .and_then(|v| v.split('.').nth(1).map(str::to_string))
        })
        .unwrap_or_else(|| "0".to_string());

    let eid_token = cookies.get("3AB9D23F7A4B3C9B").cloned().unwrap_or_default();

    let mut address_id = cookies.get("commonAddress").cloned().unwrap_or_default();
    let mut location_id = cookies
        .get("mitemAddrId")
        .map(|v| v.replace('_', "-"))
        .unwrap_or_default();

    if location_id.is_empty() || address_id.is_empty() {
        if let Some(iploc) = cookies.get("ipLoc-djd") {
            let parts: Vec<&str> = iploc.split('.').collect();
            if parts.len() == 2 {
                if location_id.is_empty() {
                    location_id = parts[0].to_string();
                }
                if address_id.is_empty() {
                    address_id = parts[1].to_string();
                }
            }
        }
    }

    RealParams {
        device_uuid,
        address_id,
        location_id,
        eid_token,
    }
}

/// Build the `Cookie:` header value from a credential's cookie string.
pub fn cookie_header(cookies: &BTreeMap<String, String>) -> String {
    cookies
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "visitkey=8254774041151190157; __jda=1.devuuid.x; 3AB9D23F7A4B3C9B=EIDTOK; commonAddress=0; mitemAddrId=1_72_55674_0; pt_key=AAJ";

    #[test]
    fn extracts_real_params() {
        let c = parse_cookies(SAMPLE);
        let rp = extract_real_params(&c);
        assert_eq!(rp.device_uuid, "8254774041151190157");
        assert_eq!(rp.eid_token, "EIDTOK");
        assert_eq!(rp.address_id, "0");
        assert_eq!(rp.location_id, "1-72-55674-0");
    }

    #[test]
    fn device_uuid_falls_back_to_jda() {
        let c = parse_cookies("__jda=1.devuuid.x");
        assert_eq!(extract_real_params(&c).device_uuid, "devuuid");
    }

    #[test]
    fn detects_pt_key() {
        assert!(has_pt_key(&parse_cookies(SAMPLE)));
        assert!(!has_pt_key(&parse_cookies("a=b")));
    }

    #[test]
    fn missing_keys_lists_each_absent_required() {
        // SAMPLE 只有 pt_key,缺 pt_pin。
        let missing = missing_required_keys(&parse_cookies(SAMPLE));
        assert!(missing.contains(&"pt_pin"));
        assert!(!missing.contains(&"pt_key"));
        // 这些 H5/PC 指纹字段已不再强制(app 端 CK 不带),即使缺也不算 missing。
        assert!(!missing.contains(&"__jda"));
        assert!(!missing.contains(&"shshshfpa"));
        assert!(!missing.contains(&"shshshfpb"));
        assert!(!missing.contains(&"shshshfpx"));
        assert!(!missing.contains(&"unionwsws"));
        assert!(!missing.contains(&"3AB9D23F7A4B3C9B"));
        assert!(!missing.contains(&"3AB9D23F7A4B3CSS"));
        assert!(!missing.contains(&"shshshfpv"));
    }

    #[test]
    fn app_exported_minimal_ck_passes_import_gate() {
        let ck = "pt_key=app_open_x; pt_pin=jd_user; pwdt_id=jd_user; sid=; unpl=encoded";
        assert!(missing_required_keys(&parse_cookies(ck)).is_empty());
    }

    #[test]
    fn complete_ck_has_no_missing() {
        let full: String = REQUIRED_KEYS.iter().map(|k| format!("{k}=x; ")).collect();
        assert!(missing_required_keys(&parse_cookies(&full)).is_empty());
    }

    #[test]
    fn empty_value_counts_as_missing() {
        let c = parse_cookies("pt_key=; pt_pin=abc");
        let missing = missing_required_keys(&c);
        assert!(missing.contains(&"pt_key")); // 空值视为缺失
        assert!(!missing.contains(&"pt_pin"));
    }
}
