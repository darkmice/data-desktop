//! 消息模板:把 [`NotifyEvent`] 渲染成与渠道无关的 [`RenderedMessage`]。
//!
//! 这里是**唯一**写文案的地方 —— 所有事件、所有失败原因的提示模板都集中于此,改文
//! 案不必碰渠道代码。渠道(钉钉/飞书…)只消费 [`RenderedMessage`]:钉钉渲染成
//! markdown(标题 + 正文),飞书等纯文本渠道用 [`RenderedMessage::to_plain_text`]。
//!
//! 文案对齐《钉钉通知完整文档》:公共信息块(账号/品名/成色/价格/youpinSkuId/
//! inspectSkuId/商品链接)+ 场景标题图标 + 时间。失败场景按 error 关键词智能选标题
//! (风控🚫/无货📦/过快⏱️/登录失效⚠️/其它❌),一个 OrderFailed 覆盖文档多类失败。

use super::event::{NotifyEvent, OrderOutcome};

/// 渲染后的中性消息:一个标题 + 若干正文行。
/// - 钉钉:`title` 作 markdown 标题(`### 标题`),`lines` 作正文(每行末尾两空格换行)。
/// - 飞书等纯文本:`to_plain_text()` 把标题与正文拼成纯文本。
#[derive(Debug, Clone)]
pub struct RenderedMessage {
    /// 标题行(已含状态图标,如「✅ 提交成功」)。
    pub title: String,
    /// 正文行(键值/说明,逐行展示)。空行已剔除。
    pub lines: Vec<String>,
}

impl RenderedMessage {
    /// 拼成纯文本(标题 + 换行 + 各正文行)。飞书等纯文本渠道用它。
    pub fn to_plain_text(&self) -> String {
        let mut s = String::with_capacity(64 + self.lines.len() * 24);
        s.push_str(&self.title);
        for line in &self.lines {
            s.push('\n');
            s.push_str(line);
        }
        s
    }

    /// 拼成钉钉 markdown 正文:`### 标题` + 空行 + 各正文行(行末加两个空格,确保
    /// markdown 渲染为换行而非合并成一段)。
    pub fn to_markdown(&self) -> String {
        let mut s = String::with_capacity(96 + self.lines.len() * 28);
        s.push_str("### ");
        s.push_str(&self.title);
        s.push('\n');
        for line in &self.lines {
            s.push('\n');
            s.push_str(line);
            s.push_str("  "); // 行末两空格 = markdown 硬换行
        }
        s
    }
}

/// 渲染一个事件 → 中性消息。`now_ms` 注入(便于测试确定性),用于「时间」行。
pub fn render(event: &NotifyEvent, now_ms: i64) -> RenderedMessage {
    match event {
        NotifyEvent::OrderSuccess(o) => render_order_success(o, now_ms),
        NotifyEvent::OrderFailed(o) => render_order_failed(o, now_ms),
        NotifyEvent::HitAlert {
            product_name,
            price,
            quality,
            inspect_sku_id,
            youpin_sku_id,
            link,
        } => render_hit_alert(
            product_name,
            price,
            quality,
            inspect_sku_id,
            youpin_sku_id,
            link,
            now_ms,
        ),
        NotifyEvent::StatusChange { title, detail } => render_status_change(title, detail, now_ms),
    }
}

/// 商品名缺省占位(手动提交无商品名时)。
fn name_or(name: &str, fallback: &str) -> String {
    if name.trim().is_empty() {
        fallback.to_string()
    } else {
        name.to_string()
    }
}

/// 仅当值非空才产出一行 `**标签:** 值`(markdown 加粗标签),否则 None(由 flatten 剔除)。
fn kv(label: &str, value: &str) -> Option<String> {
    let v = value.trim();
    if v.is_empty() {
        None
    } else {
        Some(format!("**{label}:** {v}"))
    }
}

/// 价格行:非空才产出 `**价格:** ¥xxx`。
fn price_line(price: &str) -> Option<String> {
    let p = price.trim();
    if p.is_empty() {
        None
    } else {
        Some(format!("**价格:** ¥{p}"))
    }
}

fn trigger_text(trigger: &str) -> &'static str {
    match trigger {
        "manual" => "手动下单",
        _ => "自动下单",
    }
}

/// 公共信息块(对齐文档「二、公共信息模块」):账号 / 品名 / 成色 / 价格 /
/// youpinSkuId / inspectSkuId / 商品链接。空字段自动略过。
fn common_lines(o: &OrderOutcome) -> Vec<String> {
    [
        kv("备注账号", &o.credential_name),
        kv("品名", &name_or(&o.product_name, "(手动提交)")),
        kv("成色", &o.quality),
        price_line(&o.price),
        kv("youpinSkuId", &o.youpin_sku_id),
        kv("inspectSkuId", &o.inspect_sku_id),
        // 商品链接单独成行(不加粗标签,直接给可点链接)。
        (!o.link.trim().is_empty()).then(|| o.link.clone()),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// 「时间」行(对外展示用当前时间,东八区)。
fn time_line(now_ms: i64) -> String {
    format!("**时间:** {}", fmt_cst(now_ms))
}

/// 总用时行:对外展示按真实值 ÷2(真实 1000ms → 显示 500毫秒)。0 表示未计时 → 不产出。
fn elapsed_line(elapsed_ms: u64) -> Option<String> {
    if elapsed_ms == 0 {
        return None;
    }
    let shown_ms = elapsed_ms.div_ceil(2);
    Some(format!("**总用时:** {shown_ms}毫秒"))
}

fn render_order_success(o: &OrderOutcome, now_ms: i64) -> RenderedMessage {
    let mut lines = common_lines(o);
    // 成功专属:订单号 / 触发 / 总用时 / 时间。
    lines.extend(
        [
            kv("订单号", &o.order_id),
            kv("触发", trigger_text(&o.trigger)),
            elapsed_line(o.elapsed_ms),
        ]
        .into_iter()
        .flatten(),
    );
    lines.push(time_line(now_ms));
    RenderedMessage {
        title: format!(
            "🎉 下单成功: {}",
            if o.order_id.is_empty() {
                "—"
            } else {
                &o.order_id
            }
        ),
        lines,
    }
}

/// 失败标题图标:按 error 关键词把文档多类失败(风控/无货/过快/登录失效)归并到
/// 同一个 OrderFailed,标题随原因变化。匹配只用文档明确列出的词
/// (601/风控、无货/库存、过快、未登录/登录失效/请先登录),不用裸「登录」以免误命中
/// 「代登录失败」「登录校验」等非 Cookie 失效场景 → 这些落到默认 ❌ 下单失败。
fn failed_title(error: &str) -> &'static str {
    let e = error;
    if e.contains("601") || e.contains("风控") {
        "🚫 风控拦截"
    } else if e.contains("无货") || e.contains("库存") || e.contains("stockState") {
        "📦 无货/库存不足"
    } else if e.contains("过快") {
        "⏱️ 提交过快"
    } else if e.contains("未登录") || e.contains("登录失效") || e.contains("请先登录") {
        "⚠️ 登录失效"
    } else {
        "❌ 下单失败"
    }
}

/// 品名前 20 字(用于标题摘要,对齐文档第六节「标题汇总」),空名给占位。
/// 多 SKU 监控时,钉钉通知栏摘要能直接看出是哪个商品。
fn name_prefix(product_name: &str) -> String {
    let n = product_name.trim();
    if n.is_empty() {
        return "(手动提交)".to_string();
    }
    n.chars().take(20).collect()
}

fn render_order_failed(o: &OrderOutcome, now_ms: i64) -> RenderedMessage {
    let mut lines = common_lines(o);
    lines.extend(
        [
            kv("触发", trigger_text(&o.trigger)),
            kv("失败原因", &o.error),
        ]
        .into_iter()
        .flatten(),
    );
    lines.push(time_line(now_ms));
    RenderedMessage {
        // 图标分类 + 品名前20字,对齐文档「❌/🚫/📦/⏱️ {品名前20字}」。
        title: format!(
            "{} {}",
            failed_title(&o.error),
            name_prefix(&o.product_name)
        ),
        lines,
    }
}

fn render_hit_alert(
    product_name: &str,
    price: &str,
    quality: &str,
    inspect_sku_id: &str,
    youpin_sku_id: &str,
    link: &str,
    now_ms: i64,
) -> RenderedMessage {
    let lines = [
        kv("品名", &name_or(product_name, "(未知商品)")),
        kv("成色", quality),
        price_line(price),
        kv("youpinSkuId", youpin_sku_id),
        kv("inspectSkuId", inspect_sku_id),
        (!link.trim().is_empty()).then(|| link.to_string()),
        Some("⚠️ 未开启自动提交,请尽快人工处理".to_string()),
        Some(time_line(now_ms)),
    ]
    .into_iter()
    .flatten()
    .collect();
    RenderedMessage {
        // 对齐文档「🆕 {品名前20字}」,多 SKU 时通知栏摘要能看出是哪个商品。
        title: format!("🆕 {}", name_prefix(product_name)),
        lines,
    }
}

fn render_status_change(title: &str, detail: &str, now_ms: i64) -> RenderedMessage {
    let lines = [kv("说明", detail), Some(time_line(now_ms))]
        .into_iter()
        .flatten()
        .collect();
    RenderedMessage {
        title: format!("ℹ️ {title}"),
        lines,
    }
}

/// 把 unix 毫秒格式化成「YYYY-MM-DD HH:MM:SS」东八区(CST)。无外部 crate:用
/// civil-from-days 算法。本工具受众在国内,文档样例也都是 CST,故固定东八区。
fn fmt_cst(ms: i64) -> String {
    let secs = ms.div_euclid(1000) + 8 * 3600; // 东八区偏移
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (hh, mm, ss) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let (y, mo, d) = civil_from_days(days);
    format!("{y:04}-{mo:02}-{d:02} {hh:02}:{mm:02}:{ss:02}")
}

/// Howard Hinnant 的 civil_from_days:从「1970-01-01 起的天数」反算 (年,月,日)。
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    // 2026-06-26 10:55:48 CST == 1782, 计算用固定毫秒。这里取一个已知 CST 时间戳。
    // 1782 不重要:用一个确定 ms 验证格式串结构即可。
    const T: i64 = 1_782_000_000_000; // 任意固定值,保证测试确定性

    fn outcome() -> OrderOutcome {
        OrderOutcome {
            product_name: "iPhone 15".into(),
            price: "3999".into(),
            quality: "95新".into(),
            inspect_sku_id: "INSP123".into(),
            youpin_sku_id: "YP456".into(),
            link: "https://item.m.jd.com/x".into(),
            trigger: "auto".into(),
            order_id: "ORD789".into(),
            error: String::new(),
            credential_name: "苹果17号".into(),
            elapsed_ms: 2000,
        }
    }

    #[test]
    fn success_has_title_common_block_and_elapsed_milliseconds() {
        let m = render(&NotifyEvent::OrderSuccess(outcome()), T);
        assert!(m.title.starts_with("🎉 下单成功: ORD789"));
        let text = m.to_plain_text();
        assert!(text.contains("**备注账号:** 苹果17号"));
        assert!(text.contains("**品名:** iPhone 15"));
        assert!(text.contains("**价格:** ¥3999"));
        assert!(text.contains("**youpinSkuId:** YP456"));
        assert!(text.contains("**inspectSkuId:** INSP123"));
        assert!(text.contains("https://item.m.jd.com/x"));
        assert!(text.contains("**订单号:** ORD789"));
        assert!(text.contains("**触发:** 自动下单"));
        // 真实 2000ms → 显示 2000/2 = 1000 毫秒。
        assert!(text.contains("**总用时:** 1000毫秒"), "got: {text}");
        assert!(text.contains("**时间:**"));
    }

    #[test]
    fn markdown_wraps_title_and_lines() {
        let m = render(&NotifyEvent::OrderSuccess(outcome()), T);
        let md = m.to_markdown();
        assert!(md.starts_with("### 🎉 下单成功"));
        assert!(md.contains("**订单号:** ORD789  ")); // 行末两空格硬换行
    }

    #[test]
    fn failed_title_by_error_keyword() {
        // 标题 = 图标分类 + 品名前20字。断言图标前缀 + 品名都在。
        let mut o = outcome(); // product_name = "iPhone 15"
        o.order_id = String::new();

        o.error = "601 风控拦截".into();
        let t = render(&NotifyEvent::OrderFailed(o.clone()), T).title;
        assert!(
            t.starts_with("🚫 风控拦截") && t.contains("iPhone 15"),
            "got: {t}"
        );

        o.error = "商品无货".into();
        assert!(render(&NotifyEvent::OrderFailed(o.clone()), T)
            .title
            .starts_with("📦 无货/库存不足"));

        o.error = "提交过快".into();
        assert!(render(&NotifyEvent::OrderFailed(o.clone()), T)
            .title
            .starts_with("⏱️ 提交过快"));

        o.error = "登录失效,请重新登录".into();
        assert!(render(&NotifyEvent::OrderFailed(o.clone()), T)
            .title
            .starts_with("⚠️ 登录失效"));

        o.error = "服务器开小差".into();
        assert!(render(&NotifyEvent::OrderFailed(o.clone()), T)
            .title
            .starts_with("❌ 下单失败"));

        // 裸「登录」不应误命中登录失效:「代登录失败」应落到默认 ❌。
        o.error = "代登录失败".into();
        assert!(render(&NotifyEvent::OrderFailed(o), T)
            .title
            .starts_with("❌ 下单失败"));
    }

    #[test]
    fn failed_shows_reason_not_order_id() {
        let mut o = outcome();
        o.order_id = String::new();
        o.error = "所有凭证均不可用".into();
        let m = render(&NotifyEvent::OrderFailed(o), T);
        let text = m.to_plain_text();
        assert!(text.contains("**失败原因:** 所有凭证均不可用"));
        assert!(!text.contains("订单号"));
    }

    #[test]
    fn empty_fields_are_dropped() {
        let o = OrderOutcome {
            inspect_sku_id: "INSP".into(),
            trigger: "manual".into(),
            order_id: "ORD".into(),
            price: String::new(),
            ..Default::default()
        };
        let m = render(&NotifyEvent::OrderSuccess(o), T);
        let text = m.to_plain_text();
        assert!(text.contains("**品名:** (手动提交)"));
        assert!(text.contains("**触发:** 手动下单"));
        assert!(!text.contains("**价格:**")); // price 空 → 无价格行
        assert!(!text.contains("**成色:**")); // quality 空 → 无成色行
        assert!(!text.contains("**总用时:**")); // elapsed_ms=0 → 无用时行
    }

    #[test]
    fn hit_alert_has_manual_warning_and_link() {
        let m = render(
            &NotifyEvent::HitAlert {
                product_name: "Watch".into(),
                price: "1200".into(),
                quality: "99新".into(),
                inspect_sku_id: "I".into(),
                youpin_sku_id: "Y".into(),
                link: "https://item.m.jd.com/y".into(),
            },
            T,
        );
        assert_eq!(m.title, "🆕 Watch"); // 🆕 + 品名前20字
        let text = m.to_plain_text();
        assert!(text.contains("未开启自动提交"));
        assert!(text.contains("https://item.m.jd.com/y"));
    }

    #[test]
    fn status_change_prefixes_icon() {
        let m = render(
            &NotifyEvent::StatusChange {
                title: "凭证已耗尽".into(),
                detail: "请尽快更新凭证".into(),
            },
            T,
        );
        assert!(m.title.starts_with("ℹ️"));
        assert!(m.to_plain_text().contains("**说明:** 请尽快更新凭证"));
    }

    #[test]
    fn fmt_cst_known_epoch() {
        // 1636927200s = 2021-11-14 22:00:00 UTC == 2021-11-15 06:00:00 CST(+8h)
        assert_eq!(fmt_cst(1_636_927_200_000), "2021-11-15 06:00:00");
        // epoch 0 == 1970-01-01 00:00:00 UTC == 1970-01-01 08:00:00 CST
        assert_eq!(fmt_cst(0), "1970-01-01 08:00:00");
        // 含分秒:1636927400s = 22:03:20 UTC == 06:03:20 CST
        assert_eq!(fmt_cst(1_636_927_400_000), "2021-11-15 06:03:20");
    }
}
