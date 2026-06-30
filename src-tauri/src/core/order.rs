//! Order (提交) flow, ported from Python `order.py` with the VERIFIED paipai
//! recipe: two steps (balance_getCurrentOrder_m → balance_submitOrder_m), each
//! signed via the server over WS with `raw_body=true` (bd265 + raw body), and
//! multi-credential rotation on risk/expiry.
//!
//! Signing and HTTP are injected as async traits so the flow is testable and
//! the Tauri layer can wire in the real WS-sign + reqwest client.

use serde_json::{json, Value};

use crate::core::ck::{self, Credential, RealParams};

const ORDER_URL: &str = "https://api.m.jd.com/client.action";

/// 下单链路统一的 `client`(= navigator.platform)。**签名(WS 传给服务端)与下单
/// form 必须用同一个值**,否则 seg7 指纹自相矛盾 → JD 601。h5st-probe 验证过的成功
/// 环境是全链 MacIntel。ws_client.rs 的 WS sign 请求引用此常量,build_params 也用它。
pub const ORDER_CLIENT_PLATFORM: &str = "MacIntel";

/// Result of a full order attempt.
#[derive(Debug, Clone, Default)]
pub struct OrderResult {
    pub success: bool,
    pub order_id: String,
    pub price: String,
    pub error: String,
    /// Name of the credential that produced this result.
    pub credential: String,
    pub product_name: String,
    pub quality: String,
    /// SKU identity this attempt targeted (carried through so callers — and the
    /// local order-history store — always know what was submitted).
    pub inspect_sku_id: String,
    pub youpin_sku_id: String,
}

impl OrderResult {
    fn fail(error: impl Into<String>) -> Self {
        Self {
            error: error.into(),
            ..Default::default()
        }
    }

    /// Stamp the SKU identity onto a result (called once per attempt).
    fn with_sku(mut self, inspect_id: &str, youpin_id: &str) -> Self {
        self.inspect_sku_id = inspect_id.to_string();
        self.youpin_sku_id = youpin_id.to_string();
        self
    }
}

/// Signs a payload over WS, returning the `h5st`. `raw_body` selects the paipai
/// recipe. Implemented by the Tauri layer against the live WS connection.
#[async_trait::async_trait]
pub trait Signer: Send + Sync {
    async fn sign(
        &self,
        function_id: &str,
        body_str: &str,
        t: i64,
    ) -> Result<String, String>;
}

/// Performs the actual HTTP POST to JD. Injected for testability.
#[async_trait::async_trait]
pub trait OrderHttp: Send + Sync {
    async fn post_client_action(
        &self,
        params: &[(String, String)],
        cookie_header: &str,
        youpin_id: &str,
    ) -> Result<Value, String>;
}

/// 是否应轮换到下一把凭证 = 当前凭证的【登录态失效】(CK 过期 / 未登录),
/// 换一把有效 CK 才可能成功。
///
/// ⚠️【601 不在此列】601 是风控(间歇性、对频率敏感,见 paipai-h5st-sign-recipe):
/// 同一把 CK 慢点重试可能就成。把 601 当凭证失效会把好凭证 valid=false 永久禁用,
/// 之后再下单直接跳过该凭证(连签名都不发)——这是之前的 bug。601 的处理见
/// order_once_inner:Step1 拿到 601 直接 fail 返回,但【不】触发轮换、不禁用凭证。
fn needs_rotation(err: &str) -> bool {
    // 601 一律不轮换(诊断串可能内嵌任意响应文本,先短路避免误判)。
    if err.starts_with("601") {
        return false;
    }
    // 只认明确的登录态失效信号(302=CK过期 / 未登录 / 登录失效 / 请先登录 / expired)。
    // 不含 "601"、不含过宽的 "invalid"(601 的 errorReason 可能含 invalid 误伤)。
    const KW: &[&str] = &["302", "未登录", "登录失效", "请先登录", "expired", "no access"];
    KW.iter().any(|k| err.contains(k))
}

/// Build the Step1 getCurrentOrder body —— 严格对齐 h5st-probe `build_get_current_order_body`
/// (验证过能下单那版):带 locationId,**不带** addressId(顶层)/balanceAtmosphereRequest。
/// `address_id` 入参保留以兼容调用方签名(已不参与 body 构造)。
fn build_body(
    inspect_id: &str,
    youpin_id: &str,
    _address_id: Option<&str>,
    rp: &RealParams,
) -> Value {
    let location = if rp.location_id.is_empty() {
        "1-72-2819-0".to_string()
    } else {
        rp.location_id.clone()
    };
    let form = json!({
        "supportTransport": false, "action": 1, "overseaMerge": false,
        "international": false, "netBuySourceType": 0, "appVersion": "3.0.8",
        "tradeShort": false, "inspectSkuId": inspect_id,
    });
    json!({
        "deviceUUID": rp.device_uuid, "appId": "wxae3e8056daea8727", "appVersion": "3.0.8",
        "tenantCode": "jgm", "bizModelCode": "3", "bizModeClientType": "M",
        "token": "3852b12f8c4d869b7ed3e2b3c68c9436", "externalLoginType": 1,
        "referer": "https://item.m.jd.com/", "resetGsd": true, "useBestCoupon": "1",
        "locationId": location, "packageStyle": true, "sceneval": "2",
        "sourceType": "m_inter_detail_balance", "isHK": false,
        "balanceCommonOrderForm": form,
        // resolution 跟随运行环境;固定一组合理默认(成功实测环境 = 390*844)。
        "balanceDeviceInfo": {"resolution": "390*844"},
        "cartParam": {"skuItem": {"skuId": youpin_id, "num": "1", "orderCashBack": false, "extFlag": {}}},
    })
}

/// 从 Step1 的 `balanceVendorBundleList` 提取 SKU 详情,构建 submitOrder 所需的
/// `balanceDataServerSkuVOList`。jdPrice / 三级类目 / buyNum 取 Step1 真实值,但
/// **`id` 用 youpin_id(入参商品),逐字段对齐 h5st-probe body.rs:116**——
/// probe 实测能下单的版本就是用 youpin 当 id(不是 Step1 的 balanceSkuList.skuId)。
/// 二者不一致会被 JD 风控判异常(表现为"提交过快")。
/// Step1 结构层级:vendor → bundleList → productionList → balanceSkuList。
fn build_sku_vo_list(b1: &Value, youpin_id: &str) -> Vec<Value> {
    let youpin = youpin_id.parse::<i64>().unwrap_or(0);
    let mut out = Vec::new();
    let venders = b1["balanceVendorBundleList"].as_array();
    let Some(venders) = venders else {
        return out;
    };
    for vender in venders {
        let Some(bundles) = vender["bundleList"].as_array() else {
            continue;
        };
        for bundle in bundles {
            let Some(prods) = bundle["productionList"].as_array() else {
                continue;
            };
            for prod in prods {
                let Some(skus) = prod["balanceSkuList"].as_array() else {
                    continue;
                };
                for sku in skus {
                    // category 是 [一级, 二级, 三级] 数组;缺失则 0。
                    let cat = sku["category"].as_array();
                    let cat_at = |i: usize| -> i64 {
                        cat.and_then(|c| c.get(i)).and_then(|v| v.as_i64()).unwrap_or(0)
                    };
                    // skuId 优先取数字,回退字符串解析;jdPrice 统一成字符串。
                    let sku_id = sku["skuId"].as_i64().unwrap_or_else(|| {
                        sku["skuId"].as_str().and_then(|s| s.parse().ok()).unwrap_or(0)
                    });
                    let jd_price = match &sku["jdPrice"] {
                        Value::String(s) => s.clone(),
                        Value::Number(n) => n.to_string(),
                        _ => "0".to_string(),
                    };
                    let buy_num = sku["num"].as_i64().unwrap_or(1);
                    let _ = sku_id; // probe 用 youpin 当 id,不用 Step1 的 skuId
                    out.push(json!({
                        "id": youpin,
                        "jdPrice": jd_price,
                        "buyNum": buy_num,
                        "firstCategoryId": cat_at(0),
                        "secondCategoryId": cat_at(1),
                        "thirdCategoryId": cat_at(2),
                        "promoId": 0,
                        "venderId": 0,
                        "type": 1,
                    }));
                }
            }
        }
    }
    out
}

/// 构建 submitOrder 的 dsList 埋点数组(对齐真实提交参数:67 项)。大部分为空/NULL
/// 占位,关键项用 deviceUUID + 时间戳拼接(与真实客户端一致)。
fn build_ds_list(device_uuid: &str, youpin_id: &str, now_ms: i64) -> Vec<Value> {
    let now_s = now_ms / 1000;
    // 对齐 h5st-probe:cookie_pprd_p 末尾 ".0";cookie_pprd_s 倒数第二段为 now_s-120。
    let pprd_p = format!("UUID.{device_uuid}-LOGID.{now_ms}.0");
    let pprd_s = format!("122270672.{device_uuid}.{now_s}.{now_s}.{}.2", now_s - 120);
    let ext17 = format!("122270672%7Cdirect%7C-%7Cnone%7C-%7C{device_uuid}");
    // fpa:取 deviceUUID 切片拼接(与 h5st-probe 一致,第5段取 16..28 共12位)。
    let uuid_seg = |a: usize, b: usize| -> &str {
        device_uuid.get(a..b.min(device_uuid.len())).unwrap_or("")
    };
    let fpa = format!(
        "{}-{}-b14a-3b22-{}-{}",
        uuid_seg(0, 8), uuid_seg(8, 12), uuid_seg(16, 28), now_s
    );

    // (paramName, Option<paramVal>)。None 表示该项只有 paramName、无 paramVal 字段。
    let items: &[(&str, Option<String>)] = &[
        ("report_time", Some(String::new())),
        ("deal_id", Some(String::new())),
        ("buyer_uin", None),
        ("pin", Some(String::new())),
        ("cookie_pprd_p", Some(pprd_p)),
        ("cookie_pprd_s", Some(pprd_s)),
        ("cookie_pprd_t", None),
        ("ip", Some(String::new())),
        ("visitkey", Some(device_uuid.to_string())),
        ("gen_entrance", Some(String::new())),
        ("deal_src", Some("7".into())),
        ("item_type", Some("1".into())),
        ("fav_unixtime", Some(String::new())),
        ("pay_type", Some("0".into())),
        ("ab_test", Some(String::new())),
        ("serilize_type", Some("0".into())),
        ("property1", Some("0".into())),
        ("property2", Some("0".into())),
        ("property3", Some("0".into())),
        ("property4", Some("0".into())),
        ("seller_uin", Some("0".into())),
        ("pp_item_id", Some(String::new())),
        ("openid", None),
        ("orderprice", Some(String::new())),
        ("actiontype", Some(String::new())),
        ("extinfo", Some(String::new())),
        ("ext1", Some(youpin_id.to_string())),
        ("ext2", Some(String::new())),
        ("ext3", Some(String::new())),
        ("ext4", Some(String::new())),
        ("ext5", Some(String::new())),
        ("ext6", None),
        ("ext7", Some(String::new())),
        ("ext8", Some("0".into())),
        ("ext9", Some("0|0|0|0|0||0|0".into())),
        ("ext10", Some("|||".into())),
        ("ext11", Some("http://wq.jd.com/wxapp/pages/pay/index/index".into())),
        ("ext12", Some("1".into())),
        ("ext13", Some(String::new())),
        ("ext14", Some(String::new())),
        ("ext15", Some(String::new())),
        ("ext16", Some(String::new())),
        ("ext17", Some(ext17)),
        ("ext18", None),
        ("ext19", Some(String::new())),
        ("ext20", None),
        ("fpa", Some(fpa)),
        // fpb:对齐 h5st-probe 的完整指纹串(deviceUUID 切片 + 固定长尾)。
        (
            "fpb",
            Some(format!(
                "BApXW{}-{}-{}_{}{}-BsBoM6po9xJ1O81KL9CAwE291aU5aIwTENsOtaXXi8Bsd7pk46sM52DtUxE",
                uuid_seg(0, 4), uuid_seg(4, 8), uuid_seg(8, 12), uuid_seg(12, 16), uuid_seg(16, 20)
            )),
        ),
        ("ext21", Some(String::new())),
        ("ext22", Some(String::new())),
        ("ext23", Some("NULL".into())),
        ("ext24", Some("NULL".into())),
        ("ext25", Some("NULL".into())),
        ("ext26", Some("NULL".into())),
        ("ext27", Some("NULL".into())),
        ("ext28", Some("NULL".into())),
        ("ext29", Some("NULL".into())),
        ("ext30", Some("NULL".into())),
        ("ext31", Some("NULL".into())),
        ("ext32", Some("NULL".into())),
        ("ext33", Some("NULL".into())),
        ("ext34", Some("NULL".into())),
        ("ext35", Some("NULL".into())),
        ("ext36", Some("NULL".into())),
        ("ext37", Some("NULL".into())),
        ("ext38", Some("NULL".into())),
        ("dt", Some(String::new())),
    ];
    items
        .iter()
        .map(|(name, val)| match val {
            Some(v) => json!({"paramName": name, "paramVal": v}),
            None => json!({"paramName": name}),
        })
        .collect()
}

/// 构建 submitOrder(Step2)完整 body —— 对齐真实浏览器提交参数,从 Step1 返回数据
/// (`b1` = Step1 的 `body`)提取下单会话凭据。缺这些字段 submitOrder 会因「订单会话
/// 对不上」而失败(这正是早期只发简单 body 下单不成功的根因)。
fn build_submit_body(
    b1: &Value,
    inspect_id: &str,
    youpin_id: &str,
    // 地址 ID 不再放进 body(addressId 已从 balanceCommonOrderForm 移除,改由顶层
    // locationId 携带省-市-区-地址ID)。入参保留以兼容调用方签名。
    _addr: &str,
    rp: &RealParams,
    now_ms: i64,
) -> Value {
    let device_uuid = rp.device_uuid.as_str();
    let ext = &b1["balanceExt"];

    // 精确 locationId:省-市-区-地址ID(来自 Step1 的 balanceAddress)。
    let ba = &b1["balanceAddress"];
    let location_id = if ba["id"].as_str().is_some() || ba["id"].as_i64().is_some() {
        let s = |k: &str| -> String {
            match &ba[k] {
                Value::String(v) => v.clone(),
                Value::Number(n) => n.to_string(),
                _ => String::new(),
            }
        };
        format!("{}-{}-{}-{}", s("provinceId"), s("cityId"), s("countyId"), s("id"))
    } else if !rp.location_id.is_empty() {
        rp.location_id.clone()
    } else {
        "1-72-2819-0".to_string()
    };

    // actualPayment / balanceId / transferDataStr — 直接取 Step1 返回。
    let actual_payment = match &b1["balanceTotal"]["factPrice"] {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    };
    let balance_id = match &b1["balanceId"] {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    };
    let transfer_str = match &b1["balanceTransferDataStr"] {
        Value::String(s) => s.clone(),
        other if !other.is_null() => other.to_string(),
        _ => "{}".to_string(),
    };

    let sku_vo_list = {
        let v = build_sku_vo_list(b1, youpin_id);
        if v.is_empty() {
            vec![json!({})]
        } else {
            v
        }
    };

    let ext_b = |k: &str, default: bool| -> bool { ext[k].as_bool().unwrap_or(default) };
    let ext_s = |k: &str| -> String { ext[k].as_str().unwrap_or("").to_string() };

    let balance_ext = json!({
        "sellLargeDay": match &b1["sellLargeDay"] { Value::Number(n) => n.to_string(), Value::String(s) => s.clone(), _ => "7".into() },
        "useBestCoupon": b1["useBestCoupon"].as_bool().unwrap_or(true),
        "selectedCouponNum": b1["selectedCouponNum"].as_i64().unwrap_or(0),
        "hasFreightInsurance": ext_b("hasFreightInsurance", false),
        "isInternational": b1["isInternational"].as_bool().unwrap_or(false),
        "wxAgent": ext_b("wxAgent", false),
        "subsidyPriceText": ext_s("subsidyPriceText"),
        "overseaMerge": b1["overseaMerge"].as_bool().unwrap_or(false),
        "checkIdInfo": ext_b("checkIdInfo", false),
        "isSupportTranport": ext_b("isSupportTranport", false),
        "bubbleTips": if ext["bubbleTips"].is_object() { ext["bubbleTips"].clone() } else { json!({"enable": true, "num": 1, "time": 3, "useNum": 0}) },
        "couponRedText": "",
        "supportPaymentSkuList": [youpin_id],
        "jdCombineType": 0,
        "knowledgeServiceStatus": 0,
        "cashierDeskEnable": true,
        "isTenBillion": false,
        "hasCwbg": false,
        "hasFreeShipping": true,
        "cashierPayFlag": "0",
        "hasBackupStorage": true,
        "ark": true,
        "baiDuJumpButtonSwitch": false,
        "noAddressMatchDegradeSwitch": true,
        "selectHealthPackageCard": false,
        "hasCouponHealth": false,
        "changeCouponPackage": false,
        "usedGovSubsidyToast": "因您已选择使用国家补贴，请修改国补商品的配送时间，以保证可顺利提单哦~",
        "silentPin2": false,
        "staticExtMap": {
            "creditEmptyImg": "https://img14.360buyimg.com/img/jfs/t1/312437/13/7270/18288/6842a01cFc07553d5/e6982b5b919001b8.png",
            "transBillToFirstPage": "true"
        },
        "paymentTypeSorting": false,
        "optimizedStyleSwitch": false,
        "hideTakeAway": false,
    });

    let user_action_info = json!({
        "opType": "unknow",
        "targetChannel": {"channelCode": "WXMiniPay"},
        "currentChannel": {"channelCode": "WXMiniPay"},
    })
    .to_string();

    let common_form = json!({
        "action": 1,
        "overseaMerge": b1["overseaMerge"].as_bool().unwrap_or(false),
        "international": b1["isInternational"].as_bool().unwrap_or(false),
        "netBuySourceType": 0,
        "appVersion": "3.0.8",
        "supportTransport": false,
        "tradeShort": false,
        "useChannelFlag": match &b1["useChannelFlag"] { Value::Number(n) => n.to_string(), Value::String(s) => s.clone(), _ => "10000000".into() },
        "hasSingleOrderGovSubsidy": b1["hasSingleOrderGovSubsidy"].as_bool().unwrap_or(false),
        "oldAgeStyle": false,
        "balanceRefreshByAction": "1",
        "locationAreaId": "",
        "userActionInfo": user_action_info,
        "sendGiftLocOnly": false,
        "multiAddressFlag": false,
        "extMap": {},
        "selectCreditRechargeByMe": false,
        // 文档(已与浏览器抓包对齐)明确:submitOrder 的 inspectSkuId 填 inspectSkuId
        // 本身(不是 youpin)。地址通过顶层 locationId 传,addressId 不放这里。
        "inspectSkuId": inspect_id,
        "supportTenBillion": false,
        "stockStoreModelParamList": [{}],
        "supportUserPrivacy": false,
        "userPrivacyChecked": true,
    });

    let cashier_back = format!(
        "https://trade.m.jd.com/buy/done.shtml?dealId=%24%7BorderId%7D&sceneval=2&fromPay=1&ptag=7039.27.14&gift_skuid={youpin_id}&gift_venderid=0&gift_cid=13771&normal=1"
    );

    let main_sku_ids = b1["mainSkuIdList"].as_array().cloned().unwrap_or_default();
    let license_list = b1["licenseList"].as_array().cloned().unwrap_or_default();

    json!({
        "deviceUUID": device_uuid,
        "appId": "wxae3e8056daea8727",
        "tenantCode": "jgm",
        "bizModelCode": "3",
        "bizModeClientType": "M",
        "token": "3852b12f8c4d869b7ed3e2b3c68c9436",
        "externalLoginType": 1,
        "appVersion": "3.0.8",
        "referer": "https://item.m.jd.com/",
        "checkPayPassport": false,
        "checkpwdV2": false,
        "isEncryptionMobile": true,
        "outStockVendorIdList": [0],
        "mainSkuIdList": main_sku_ids,
        "balanceDataServerSkuVOList": sku_vo_list,
        "balanceTableWareVoList": [{}],
        "cashierDeskBackUrl": cashier_back,
        "payType": "4",
        "subPayType": "",
        "licenseList": license_list,
        "balanceCommonOrderForm": common_form,
        "balanceExt": balance_ext,
        "actualPayment": actual_payment,
        "sendGift": {},
        "submitPutLocationRemarkParam": {},
        "dsList": build_ds_list(device_uuid, youpin_id, now_ms),
        "locationId": location_id,
        "packageStyle": true,
        "sceneval": "2",
        "sourceType": "m_inter_detail_balance",
        "balanceTransferDataStr": transfer_str,
        "isHK": false,
        // resolution 跟随运行环境;与 Step1 一致(成功实测环境 = 390*844)。
        "balanceDeviceInfo": {"resolution": "390*844"},
        "balanceId": balance_id,
        "sendGiftSelectAddrBySelf": true,
        "sendGiftHasSaveConsigneeAddress": false,
    })
}

/// Build the form params (ported from Python `_build_params`). `t` MUST equal
/// the `t` used inside the h5st signature.
fn build_params(
    function_id: &str,
    body_str: &str,
    youpin_id: &str,
    h5st: &str,
    t: i64,
    rp: &RealParams,
) -> Vec<(String, String)> {
    // 严格对齐 h5st-probe send.rs 的精简 form(10 项,验证过能下单那版):不发
    // osVersion/screen/networkType/d_brand/d_model/lang/sdkVersion/openudid/uuid/
    // x-api-eid-token —— 多余字段也可能被风控判异常。client 与 WS 签名同值(MacIntel)。
    let _ = rp; // 精简版不再从 CK 取 uuid/eid 进 form
    [
        ("appid", "m_core".to_string()),
        ("functionId", function_id.to_string()),
        ("body", body_str.to_string()),
        ("client", ORDER_CLIENT_PLATFORM.to_string()),
        ("clientVersion", "3.0.8".to_string()),
        ("loginType", "2".to_string()),
        ("t", t.to_string()),
        ("scval", youpin_id.to_string()),
        ("xAPIScval3", "m_inter_detail_balance".to_string()),
        ("h5st", h5st.to_string()),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect()
}

/// One step: sign + POST + parse. Returns (parsed_json, body_used_for_address).
async fn run_step<S: Signer, H: OrderHttp>(
    signer: &S,
    http: &H,
    function_id: &str,
    body: &Value,
    youpin_id: &str,
    cookie_header: &str,
    rp: &RealParams,
    now_ms: i64,
) -> Result<Value, String> {
    let body_str = serde_json::to_string(body).map_err(|e| e.to_string())?;
    let h5st = signer.sign(function_id, &body_str, now_ms).await?;
    let params = build_params(function_id, &body_str, youpin_id, &h5st, now_ms, rp);
    http.post_client_action(&params, cookie_header, youpin_id).await
}

/// Wall-clock unix millis. JD validates that the request `t` is close to server
/// time, so each step must use a FRESH timestamp — a value captured once and
/// threaded through a multi-credential rotation would lag by tens of seconds.
fn fresh_ms(fallback: i64) -> i64 {
    if fallback > 0 {
        // Tests pin a deterministic value via fallback; production passes 0.
        return fallback;
    }
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Full two-step order for one credential. Each step captures a fresh `t`
/// (unless `now_ms_test > 0`, which pins it for tests). The SKU identity is
/// stamped onto every result path via the thin wrapper below.
pub async fn order_once<S: Signer, H: OrderHttp>(
    signer: &S,
    http: &H,
    cred: &Credential,
    inspect_id: &str,
    youpin_id: &str,
    now_ms_test: i64,
) -> OrderResult {
    order_once_inner(signer, http, cred, inspect_id, youpin_id, now_ms_test)
        .await
        .with_sku(inspect_id, youpin_id)
}

async fn order_once_inner<S: Signer, H: OrderHttp>(
    signer: &S,
    http: &H,
    cred: &Credential,
    inspect_id: &str,
    youpin_id: &str,
    now_ms_test: i64,
) -> OrderResult {
    let cookies = ck::parse_cookies(&cred.cookie_str);
    let header = ck::cookie_header(&cookies);
    let rp = ck::extract_real_params(&cookies);

    // Step 1: getCurrentOrder → address id + price. Fresh t.
    let now1 = fresh_ms(now_ms_test);
    let body1 = build_body(inspect_id, youpin_id, None, &rp);
    let r1 = match run_step(signer, http, "balance_getCurrentOrder_m", &body1, youpin_id, &header, &rp, now1).await {
        Ok(v) => v,
        Err(e) => return OrderResult { credential: cred.name.clone(), ..OrderResult::fail(e) },
    };
    let b1 = &r1["body"];
    if b1["errorCode"].as_str() == Some("601") {
        // 保留 JD 的完整响应,便于判断 601 卡在哪层(签名层会是别的 code/msg,
        // 风控层 601 通常 code:0/msg:success 但 body.errorCode=601)。
        let diag = format!(
            "601 [code={} msg={} errReason={}] resp={}",
            r1["code"].as_str().unwrap_or(&r1["code"].to_string()),
            r1["message"].as_str().unwrap_or(""),
            b1["errorReason"].as_str().unwrap_or(""),
            serde_json::to_string(&r1).unwrap_or_default().chars().take(400).collect::<String>(),
        );
        return OrderResult { credential: cred.name.clone(), ..OrderResult::fail(diag) };
    }
    // 地址 ID 可能是字符串或数字(真实返回是数字,如 13017040608),两种都接受。
    let addr_val = |v: &Value| -> Option<String> {
        match v {
            Value::String(s) if !s.is_empty() => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        }
    };
    let addr = addr_val(&b1["balanceAddress"]["id"])
        .or_else(|| addr_val(&b1["address"]["id"]))
        .or_else(|| addr_val(&b1["addressInfo"]["id"]));
    let addr = match addr {
        Some(a) => a,
        None => {
            let reason = b1["errorReason"].as_str().unwrap_or("");
            let code = b1["errorCode"].as_str().unwrap_or("");
            let msg = if !reason.is_empty() {
                reason.to_string()
            } else if !code.is_empty() {
                format!("错误码: {code}")
            } else {
                "未获取到地址(可能售罄或不可购买)".to_string()
            };
            return OrderResult { credential: cred.name.clone(), ..OrderResult::fail(msg) };
        }
    };
    let price = b1["balanceTotal"]["factPrice"].to_string();

    // Step 2: submitOrder. Fresh t again (or pinned test value + 1). 用 Step1 返回
    // 数据(b1)重建完整 submitOrder body —— balanceId / transferDataStr / skuVOList /
    // dsList / balanceExt / actualPayment 等会话凭据缺一不可,否则订单会话对不上必失败。
    let now2 = if now_ms_test > 0 { now_ms_test + 1 } else { fresh_ms(0) };
    let body2 = build_submit_body(b1, inspect_id, youpin_id, &addr, &rp, now2);
    let r2 = match run_step(signer, http, "balance_submitOrder_m", &body2, youpin_id, &header, &rp, now2).await {
        Ok(v) => v,
        Err(e) => return OrderResult { credential: cred.name.clone(), price, ..OrderResult::fail(e) },
    };
    let b2 = &r2["body"];
    let order_id = b2["order"]["orderId"].as_str().unwrap_or("").to_string();
    if !order_id.is_empty() {
        let order_price = b2["order"]["orderPrice"].to_string();
        OrderResult {
            success: true,
            order_id,
            price: order_price,
            credential: cred.name.clone(),
            ..Default::default()
        }
    } else {
        let err = b2["errorReason"].as_str().or(b2["errorCode"].as_str()).unwrap_or("未知错误");
        OrderResult { credential: cred.name.clone(), price, ..OrderResult::fail(err) }
    }
}

/// Order with multi-credential rotation. Tries credentials starting at
/// `active_idx`; on a rotation-worthy failure, marks the credential invalid and
/// advances. Returns (result, updated_credentials, new_active_idx).
pub async fn order_with_rotation<S: Signer, H: OrderHttp>(
    signer: &S,
    http: &H,
    mut creds: Vec<Credential>,
    active_idx: usize,
    inspect_id: &str,
    youpin_id: &str,
    now_ms: i64,
    logs: &mut Vec<String>,
) -> (OrderResult, Vec<Credential>, usize) {
    if creds.is_empty() {
        return (OrderResult::fail("没有可用凭证"), creds, 0);
    }
    let n = creds.len();
    let mut idx = active_idx.min(n - 1);
    let mut tried = 0;

    while tried < n {
        // 只跳过【已过期】凭证(需换 CK);风控(RiskControlled)凭证仍允许重试
        // (601 间歇性),只是前端标橙提示。这样修好了"601 后再下单连签名都不发"的 bug。
        if creds[idx].status == ck::CredStatus::Expired {
            idx = (idx + 1) % n;
            tried += 1;
            continue;
        }
        logs.push(format!("使用凭证: {}", creds[idx].name));
        let res = order_once(signer, http, &creds[idx], inspect_id, youpin_id, now_ms).await;
        if res.success {
            // 成功 → 恢复为 Active(清掉之前可能的风控标记)。
            creds[idx].status = ck::CredStatus::Active;
            creds[idx].valid = true;
            return (res, creds, idx);
        }
        logs.push(format!("提交失败 ({}): {}", creds[idx].name, res.error));
        if needs_rotation(&res.error) {
            // 登录态失效(302/过期)→ 标记 Expired 并轮换到下一把。
            creds[idx].status = ck::CredStatus::Expired;
            creds[idx].valid = false;
            tried += 1;
            if n == 1 || tried >= n || !creds.iter().any(|c| c.is_active()) {
                logs.push("所有凭证均不可用，请更新凭证".into());
                return (
                    OrderResult { error: format!("所有凭证均不可用: {}", res.error), ..res },
                    creds,
                    idx,
                );
            }
            logs.push(format!("凭证 {} 已失效，自动切换", creds[idx].name));
            idx = (idx + 1) % n;
            continue;
        }
        // 非轮换失败(风控 601 等):标记风控态(前端橙色、可手动解除/重试),停在这里。
        // 不禁用、不轮换——同一把 CK 慢点重试可能就成。
        if res.error.contains("601") {
            creds[idx].status = ck::CredStatus::RiskControlled;
            creds[idx].valid = true;
        }
        return (res, creds, idx);
    }
    (OrderResult::fail("所有凭证均已尝试"), creds, idx)
}

/// Build the dingtalk product link (spec §6.1).
pub fn product_link(youpin_id: &str, inspect_id: &str) -> String {
    format!("https://item.m.jd.com/product/{youpin_id}.html?inspectSkuId={inspect_id}")
}

/// Default order HTTP impl over reqwest (used by the Tauri layer).
/// 完整 iPhone Safari UA(对齐 h5st-probe send.rs:带 Version/Mobile/Safari 段,
/// 不能用残缺串)。
const ORDER_UA: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_5 like Mac OS X) \
AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.5 Mobile/15E148 Safari/604.1";

/// 下单 HTTP 客户端 —— 用 wreq(impersonate Chrome)伪造 TLS/JA3/HTTP2 指纹。
/// JD 风控在传输层识别非浏览器客户端,普通 reqwest 的 TLS 指纹必被判 601;wreq 对齐
/// 真实 Chrome,这是 h5st-probe 能成功下单的关键。逐项照搬 h5st-probe send.rs。
pub struct WreqHttp {
    pub client: wreq::Client,
}

impl WreqHttp {
    /// 构建 impersonate Chrome 的 wreq 客户端(失败兜底为默认 client)。
    pub fn new() -> Self {
        let client = wreq::Client::builder()
            .emulation(wreq_util::Emulation::Chrome131)
            .build()
            .unwrap_or_default();
        WreqHttp { client }
    }
}

impl Default for WreqHttp {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl OrderHttp for WreqHttp {
    async fn post_client_action(
        &self,
        params: &[(String, String)],
        cookie_header: &str,
        youpin_id: &str,
    ) -> Result<Value, String> {
        use wreq::header::{HeaderMap, HeaderName, HeaderValue};
        let referer = format!("https://item.m.jd.com/product/{youpin_id}.html");
        let form: Vec<(String, String)> = params.to_vec();

        // header 严格对齐 h5st-probe send.rs(能下单那版)。
        let mut headers = HeaderMap::new();
        let set = |h: &mut HeaderMap, k: &'static str, v: &str| {
            if let Ok(val) = HeaderValue::from_str(v) {
                h.insert(HeaderName::from_static(k), val);
            }
        };
        set(&mut headers, "content-type", "application/x-www-form-urlencoded");
        set(&mut headers, "origin", "https://item.m.jd.com");
        set(&mut headers, "user-agent", ORDER_UA);
        set(&mut headers, "referer", &referer);
        set(&mut headers, "x-referer-page", &referer);
        set(&mut headers, "x-rp-client", "h5_2.1.0");
        set(&mut headers, "cookie", cookie_header);

        let resp = self
            .client
            .post(ORDER_URL)
            .headers(headers)
            .form(&form)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        let text = resp.text().await.map_err(|e| e.to_string())?;
        serde_json::from_str::<Value>(&text).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeSigner;
    #[async_trait::async_trait]
    impl Signer for FakeSigner {
        async fn sign(&self, _f: &str, _b: &str, _t: i64) -> Result<String, String> {
            Ok("fakeh5st".into())
        }
    }

    // HTTP that returns address on step1 and orderId on step2.
    struct OkHttp;
    #[async_trait::async_trait]
    impl OrderHttp for OkHttp {
        async fn post_client_action(&self, params: &[(String, String)], _c: &str, _y: &str) -> Result<Value, String> {
            let fid = params.iter().find(|(k, _)| k == "functionId").map(|(_, v)| v.clone()).unwrap_or_default();
            if fid.contains("getCurrentOrder") {
                Ok(json!({"body":{"balanceAddress":{"id":"8042010864"},"balanceTotal":{"factPrice":8711.13}}}))
            } else {
                Ok(json!({"body":{"order":{"orderId":"ORDER123","orderPrice":8711.13}}}))
            }
        }
    }

    // HTTP that always returns 601.
    struct RiskHttp;
    #[async_trait::async_trait]
    impl OrderHttp for RiskHttp {
        async fn post_client_action(&self, _p: &[(String, String)], _c: &str, _y: &str) -> Result<Value, String> {
            Ok(json!({"body":{"errorCode":"601"}}))
        }
    }

    fn creds(names: &[&str]) -> Vec<Credential> {
        names.iter().map(|n| Credential {
            name: n.to_string(),
            cookie_str: "pt_key=x; visitkey=v".into(),
            status: ck::CredStatus::Active,
            valid: true,
        }).collect()
    }

    /// 一个贴近真实的 Step1 `body` 返回(含会话凭据 + SKU 详情 + 地址)。
    fn step1_body() -> Value {
        json!({
            "balanceAddress": {"id": 13017040608i64, "provinceId": 20, "cityId": 1753, "countyId": 1754},
            "balanceTotal": {"factPrice": 1087.02},
            "balanceId": "6628841741352017921782308729906",
            "balanceTransferDataStr": "{\"sessionId\":\"aaec04de\",\"reqSkuMap\":{}}",
            "mainSkuIdList": [100264461867i64],
            "useChannelFlag": 10000000i64,
            "useBestCoupon": true,
            "sellLargeDay": 7,
            "licenseList": [],
            "balanceExt": {"subsidyPriceText": "超级补贴", "hasFreightInsurance": false},
            "balanceVendorBundleList": [{
                "bundleList": [{
                    "productionList": [{
                        "balanceSkuList": [{
                            "skuId": 100358632432i64,
                            "jdPrice": "1087.02",
                            "num": 1,
                            "category": [13765, 13769, 13771]
                        }]
                    }]
                }]
            }]
        })
    }

    #[test]
    fn submit_body_carries_step1_session_data() {
        let b1 = step1_body();
        let rp = RealParams {
            device_uuid: "17821833500701388191".into(),
            address_id: String::new(),
            location_id: String::new(),
            eid_token: String::new(),
        };
        let body = build_submit_body(&b1, "123736024424448", "100264461867", "13017040608", &rp, 1_782_308_729_987);

        // 会话凭据必须取自 Step1(缺则订单会话对不上)。
        assert_eq!(body["balanceId"], "6628841741352017921782308729906");
        assert_eq!(body["balanceTransferDataStr"], "{\"sessionId\":\"aaec04de\",\"reqSkuMap\":{}}");
        assert_eq!(body["actualPayment"], "1087.02");
        assert_eq!(body["mainSkuIdList"][0], 100264461867i64);

        // 精确 locationId = 省-市-区-地址ID(来自 Step1 balanceAddress)。
        assert_eq!(body["locationId"], "20-1753-1754-13017040608");

        // skuVOList:id 用 youpin_id(对齐 h5st-probe body.rs:116,不是 Step1 的
        // balanceSkuList.skuId);jdPrice / 三级类目仍取 Step1 真实值。
        let sku = &body["balanceDataServerSkuVOList"][0];
        assert_eq!(sku["id"], 100264461867i64); // = youpin_id,不是 Step1 skuId 100358632432
        assert_eq!(sku["jdPrice"], "1087.02");
        assert_eq!(sku["firstCategoryId"], 13765);
        assert_eq!(sku["secondCategoryId"], 13769);
        assert_eq!(sku["thirdCategoryId"], 13771);

        // 顶层键顺序必须是「插入序」(serde_json preserve_order),而非字母序——
        // JD 风控看 body 字段顺序特征。第一个 key 必须是 deviceUUID,不是 actualPayment。
        let serialized = serde_json::to_string(&body).unwrap();
        assert!(
            serialized.starts_with("{\"deviceUUID\":"),
            "body 顶层首字段应为 deviceUUID(插入序),实际: {}",
            &serialized[..serialized.len().min(40)]
        );

        // 关键固定字段。
        assert_eq!(body["appVersion"], "3.0.8");
        assert_eq!(body["payType"], "4");
        // addressId 已从 balanceCommonOrderForm 移除(对齐文档);地址走顶层 locationId。
        assert!(body["balanceCommonOrderForm"]["addressId"].is_null());
        // inspectSkuId 填 inspect_id 本身(不是 youpin)——对齐文档。
        assert_eq!(body["balanceCommonOrderForm"]["inspectSkuId"], "123736024424448");
        assert_eq!(body["balanceCommonOrderForm"]["appVersion"], "3.0.8");
        assert_eq!(body["balanceExt"]["supportPaymentSkuList"][0], "100264461867");
        assert_eq!(body["balanceExt"]["subsidyPriceText"], "超级补贴");
        // 文档新增:国补提示文案 + creditEmptyImg CDN URL。
        assert_eq!(
            body["balanceExt"]["usedGovSubsidyToast"],
            "因您已选择使用国家补贴，请修改国补商品的配送时间，以保证可顺利提单哦~"
        );
        assert_eq!(
            body["balanceExt"]["staticExtMap"]["creditEmptyImg"],
            "https://img14.360buyimg.com/img/jfs/t1/312437/13/7270/18288/6842a01cFc07553d5/e6982b5b919001b8.png"
        );

        // dsList 67 项,关键项填对。
        let ds = body["dsList"].as_array().unwrap();
        assert_eq!(ds.len(), 67);
        let find = |name: &str| ds.iter().find(|d| d["paramName"] == name).cloned().unwrap();
        assert_eq!(find("visitkey")["paramVal"], "17821833500701388191");
        assert_eq!(find("ext1")["paramVal"], "100264461867");
        assert_eq!(find("deal_src")["paramVal"], "7");
        // buyer_uin 只有 paramName、无 paramVal。
        assert!(find("buyer_uin").get("paramVal").is_none());
    }

    #[test]
    fn submit_body_falls_back_when_step1_sparse() {
        // Step1 返回稀疏(无 vendor list / 无地址)时不 panic,skuVOList 退化为 [{}]。
        let b1 = json!({"balanceTotal": {"factPrice": 50.0}});
        let rp = RealParams {
            device_uuid: "u".into(), address_id: String::new(),
            location_id: "1-72-2819-0".into(), eid_token: String::new(),
        };
        let body = build_submit_body(&b1, "i", "y", "addr1", &rp, 1000);
        assert_eq!(body["balanceDataServerSkuVOList"][0], json!({}));
        assert_eq!(body["locationId"], "1-72-2819-0"); // 无地址 → 退回 rp.location_id
        assert_eq!(body["actualPayment"], "50.0");
        assert_eq!(body["balanceId"], ""); // 缺失 → 空串
    }

    #[tokio::test]
    async fn happy_path_two_steps() {
        let r = order_once(&FakeSigner, &OkHttp, &creds(&["a"])[0], "i1", "y1", 1000).await;
        assert!(r.success);
        assert_eq!(r.order_id, "ORDER123");
    }

    /// 601 = 风控(间歇性),不是凭证失效:必须【不轮换、不禁用凭证】,失败返回但
    /// 凭证保持 valid,这样用户可以用同一把 CK 慢点重试。回归防护:之前的 bug 是
    /// 601 触发 needs_rotation → valid=false → 之后下单跳过该凭证(连签名都不发)。
    #[tokio::test]
    async fn risk_601_does_not_disable_credential() {
        let mut logs = Vec::new();
        let (r, updated, _) = order_with_rotation(
            &FakeSigner, &RiskHttp, creds(&["a", "b"]), 0, "i1", "y1", 1000,
            &mut logs,
        ).await;
        assert!(!r.success);
        assert!(r.error.contains("601"));
        // 关键:命中的凭证标为风控态(前端橙色)但仍可用(is_active 仅 Expired 才 false),
        // 不发生轮换切换。第一把(idx 0)被标 RiskControlled,但 status!=Expired → 下次仍会用。
        assert_eq!(updated[0].status, ck::CredStatus::RiskControlled, "601 标风控态");
        assert!(updated.iter().all(|c| c.status != ck::CredStatus::Expired), "601 不应标过期");
        assert!(!logs.iter().any(|l| l.contains("自动切换")), "601 不应触发轮换");
    }

    /// 真正的登录态失效(302/未登录)才轮换:第一把失效后切到下一把。
    #[tokio::test]
    async fn expired_ck_rotates_to_next_credential() {
        struct ExpiredHttp;
        #[async_trait::async_trait]
        impl OrderHttp for ExpiredHttp {
            async fn post_client_action(&self, _p: &[(String, String)], _c: &str, _y: &str) -> Result<Value, String> {
                Ok(json!({"body":{"errorCode":"302","errorReason":"no access"}}))
            }
        }
        let mut logs = Vec::new();
        let (r, updated, _) = order_with_rotation(
            &FakeSigner, &ExpiredHttp, creds(&["a", "b"]), 0, "i1", "y1", 1000,
            &mut logs,
        ).await;
        assert!(!r.success);
        // 两把都 302 → 都标 Expired → 最终"所有凭证均不可用"。
        assert!(updated.iter().all(|c| c.status == ck::CredStatus::Expired), "302 应标过期");
    }

    #[test]
    fn product_link_format() {
        assert_eq!(
            product_link("100221186437", "121918401832968"),
            "https://item.m.jd.com/product/100221186437.html?inspectSkuId=121918401832968"
        );
    }

    /// form 严格对齐 h5st-probe send.rs(验证过能下单那版):精简 10 项,client=MacIntel
    /// (与 WS 签名同值,否则 seg7 自相矛盾→601),且【不含】screen/lang/osVersion/
    /// sdkVersion/networkType/d_brand/d_model/openudid/uuid/x-api-eid-token。
    #[test]
    fn build_params_matches_h5st_probe_form() {
        let rp = RealParams {
            device_uuid: "7978913553131800874".into(),
            address_id: String::new(),
            location_id: String::new(),
            eid_token: "EIDTOK".into(),
        };
        let p = build_params("balance_getCurrentOrder_m", "{}", "100358632432", "H5ST", 1782364563948, &rp);
        let get = |k: &str| p.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
        // 必含的 10 项。
        assert_eq!(get("appid").as_deref(), Some("m_core"));
        assert_eq!(get("functionId").as_deref(), Some("balance_getCurrentOrder_m"));
        assert_eq!(get("body").as_deref(), Some("{}"));
        assert_eq!(get("client").as_deref(), Some("MacIntel")); // = ORDER_CLIENT_PLATFORM = WS 签名 client
        assert_eq!(get("clientVersion").as_deref(), Some("3.0.8"));
        assert_eq!(get("loginType").as_deref(), Some("2"));
        assert_eq!(get("t").as_deref(), Some("1782364563948"));
        assert_eq!(get("scval").as_deref(), Some("100358632432"));
        assert_eq!(get("xAPIScval3").as_deref(), Some("m_inter_detail_balance"));
        assert_eq!(get("h5st").as_deref(), Some("H5ST"));
        assert_eq!(p.len(), 10, "form 必须正好 10 项(对齐 h5st-probe)");
        // 确认多余字段已剔除。
        for k in ["screen", "lang", "osVersion", "sdkVersion", "networkType",
                  "d_brand", "d_model", "openudid", "uuid", "x-api-eid-token"] {
            assert!(get(k).is_none(), "{k} 不应出现在 form 里");
        }
    }

    /// Step1 body 对齐 h5st-probe(验证过能下单那版):resolution=390*844、
    /// **不带** 顶层 addressId / balanceAtmosphereRequest;inspectSkuId 用 inspect_id 本身。
    #[test]
    fn build_body_matches_success_capture() {
        let rp = RealParams {
            device_uuid: "u".into(), address_id: String::new(),
            location_id: String::new(), eid_token: String::new(),
        };
        let body = build_body("123375521898500", "100358632432", None, &rp);
        // resolution = 成功实测环境默认。
        assert_eq!(body["balanceDeviceInfo"]["resolution"], "390*844");
        // 对齐 h5st-probe:顶层 addressId / balanceAtmosphereRequest 都已删除。
        assert!(body.get("addressId").is_none(), "Step1 不应有顶层 addressId");
        assert!(body.get("balanceAtmosphereRequest").is_none(), "Step1 不应有 balanceAtmosphereRequest");
        // Step1 balanceCommonOrderForm 用 inspectSkuId(成功抓包一致)。
        assert_eq!(body["balanceCommonOrderForm"]["inspectSkuId"], "123375521898500");
        // 无 cookie 地址 → locationId 用默认。
        assert_eq!(body["locationId"], "1-72-2819-0");
    }

    /// 锁死 Step1 body 的完整字段集 == h5st-probe(验证过能下单那版)。
    /// 顶层 19 项(无 addressId / balanceAtmosphereRequest)+ commonOrderForm 8 项。
    /// 多一个/少一个字段都会失败 —— 防止以后误删/误增导致订单会话对不上。
    #[test]
    fn build_body_field_set_matches_success_capture() {
        let rp = RealParams {
            device_uuid: "8254774041151190157".into(),
            address_id: "0".into(),
            location_id: "1-72-55674-0".into(),
            eid_token: String::new(),
        };
        let body = build_body("119472350299143", "100181382451", None, &rp);

        let mut top: Vec<&str> = body.as_object().unwrap().keys().map(String::as_str).collect();
        top.sort_unstable();
        let mut expect_top = vec![
            "appId", "appVersion",
            "balanceCommonOrderForm", "balanceDeviceInfo", "bizModeClientType",
            "bizModelCode", "cartParam", "deviceUUID", "externalLoginType", "isHK",
            "locationId", "packageStyle", "referer", "resetGsd", "sceneval",
            "sourceType", "tenantCode", "token", "useBestCoupon",
        ];
        expect_top.sort_unstable();
        assert_eq!(top, expect_top, "Step1 body 顶层字段集必须与 h5st-probe 一致");

        let mut form: Vec<&str> = body["balanceCommonOrderForm"].as_object().unwrap()
            .keys().map(String::as_str).collect();
        form.sort_unstable();
        let mut expect_form = vec![
            "action", "appVersion", "inspectSkuId", "international",
            "netBuySourceType", "overseaMerge", "supportTransport", "tradeShort",
        ];
        expect_form.sort_unstable();
        assert_eq!(form, expect_form, "balanceCommonOrderForm 字段集必须与 h5st-probe 一致");

        // 关键值精确比对。
        assert_eq!(body["locationId"], "1-72-55674-0");
        assert_eq!(body["cartParam"]["skuItem"]["skuId"], "100181382451");
        assert_eq!(body["balanceCommonOrderForm"]["inspectSkuId"], "119472350299143");
    }

    /// 对齐 h5st-probe 后:Step1 body 不再含 addressId;address_id 入参被忽略;
    /// locationId 来自 rp.location_id(无则默认 1-72-2819-0)。
    #[test]
    fn build_body_ignores_address_id_and_uses_location() {
        let rp = RealParams {
            device_uuid: "u".into(), address_id: "13017040608".into(),
            location_id: "20-1753-1754-13017040608".into(), eid_token: String::new(),
        };
        // 即使传了显式 address_id,body 里也不该出现 addressId。
        let body = build_body("i", "y", Some("999"), &rp);
        assert!(body.get("addressId").is_none(), "Step1 不应有 addressId");
        // locationId 来自 rp.location_id。
        assert_eq!(body["locationId"], "20-1753-1754-13017040608");
        // location_id 为空 → 默认。
        let rp2 = RealParams {
            device_uuid: "u".into(), address_id: String::new(),
            location_id: String::new(), eid_token: String::new(),
        };
        assert_eq!(build_body("i", "y", None, &rp2)["locationId"], "1-72-2819-0");
    }
}
