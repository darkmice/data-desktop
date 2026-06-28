//! 消息模板:把 [`NotifyEvent`] 渲染成与渠道无关的 [`RenderedMessage`]。
//!
//! 这里是**唯一**写文案的地方 —— 所有事件、所有失败原因的提示模板都集中于此,改文
//! 案不必碰渠道代码。渠道(钉钉/飞书…)只消费 [`RenderedMessage`],把它转成各自的
//! 报文格式。

use super::event::{NotifyEvent, OrderOutcome};

/// 渲染后的中性消息:一个标题 + 若干正文行。渠道自行决定如何拼接(钉钉/飞书 text
/// 都是把标题与正文拼成纯文本;若将来要发 markdown 卡片,也能从这结构重新组织)。
#[derive(Debug, Clone)]
pub struct RenderedMessage {
    /// 标题行(已含状态图标,如「✅ 提交成功」)。
    pub title: String,
    /// 正文行(键值/说明,逐行展示)。空行已剔除。
    pub lines: Vec<String>,
}

impl RenderedMessage {
    /// 拼成纯文本(标题 + 换行 + 各正文行)。多数纯文本渠道直接用它。
    pub fn to_plain_text(&self) -> String {
        let mut s = String::with_capacity(64 + self.lines.len() * 24);
        s.push_str(&self.title);
        for line in &self.lines {
            s.push('\n');
            s.push_str(line);
        }
        s
    }
}

/// 渲染一个事件 → 中性消息。
pub fn render(event: &NotifyEvent) -> RenderedMessage {
    match event {
        NotifyEvent::OrderSuccess(o) => render_order_success(o),
        NotifyEvent::OrderFailed(o) => render_order_failed(o),
        NotifyEvent::HitAlert {
            product_name,
            price,
            quality,
            inspect_sku_id,
            youpin_sku_id,
            link,
        } => render_hit_alert(product_name, price, quality, inspect_sku_id, youpin_sku_id, link),
        NotifyEvent::StatusChange { title, detail } => render_status_change(title, detail),
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

/// 仅当值非空才产出一行 `标签：值`,否则返回 None(由 flatten 剔除)。
fn kv(label: &str, value: &str) -> Option<String> {
    let v = value.trim();
    if v.is_empty() {
        None
    } else {
        Some(format!("{label}：{v}"))
    }
}

fn trigger_text(trigger: &str) -> &'static str {
    match trigger {
        "manual" => "手动",
        _ => "自动",
    }
}

/// 价格行:非空才产出 `价格：¥xxx`。
fn price_line(price: &str) -> Option<String> {
    let p = price.trim();
    if p.is_empty() {
        None
    } else {
        Some(format!("价格：¥{p}"))
    }
}

fn render_order_success(o: &OrderOutcome) -> RenderedMessage {
    let lines = [
        kv("名称", &name_or(&o.product_name, "(手动提交)")),
        kv("订单号", &o.order_id),
        kv("账号", &o.credential_name),
        price_line(&o.price),
        kv("成色", &o.quality),
        kv("质检报告编号", &o.inspect_sku_id),
        kv("触发", trigger_text(&o.trigger)),
        kv("🔗", &o.link),
    ]
    .into_iter()
    .flatten()
    .collect();
    RenderedMessage {
        title: "✅ 提交成功".to_string(),
        lines,
    }
}

fn render_order_failed(o: &OrderOutcome) -> RenderedMessage {
    let lines = [
        kv("名称", &name_or(&o.product_name, "(手动提交)")),
        kv("成色", &o.quality),
        kv("质检报告编号", &o.inspect_sku_id),
        kv("触发", trigger_text(&o.trigger)),
        kv("原因", &o.error),
        kv("账号", &o.credential_name),
        kv("🔗", &o.link),
    ]
    .into_iter()
    .flatten()
    .collect();
    RenderedMessage {
        title: "❌ 提交失败".to_string(),
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
) -> RenderedMessage {
    let _ = youpin_sku_id; // 备用标识,当前文案不展示但保留入参以便日后扩展
    let lines = [
        kv("名称", &name_or(product_name, "(未知商品)")),
        price_line(price),
        kv("成色", quality),
        kv("质检报告编号", inspect_sku_id),
        Some("⚠️ 未开启自动提交,请尽快人工处理".to_string()),
        kv("🔗", link),
    ]
    .into_iter()
    .flatten()
    .collect();
    RenderedMessage {
        title: "🔔 命中目标".to_string(),
        lines,
    }
}

fn render_status_change(title: &str, detail: &str) -> RenderedMessage {
    let lines = kv("说明", detail).into_iter().collect();
    RenderedMessage {
        title: format!("ℹ️ {title}"),
        lines,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            credential_name: String::new(),
        }
    }

    #[test]
    fn success_has_title_and_all_kv() {
        let m = render(&NotifyEvent::OrderSuccess(outcome()));
        assert_eq!(m.title, "✅ 提交成功");
        let text = m.to_plain_text();
        assert!(text.contains("名称：iPhone 15"));
        assert!(text.contains("订单号：ORD789"));
        assert!(text.contains("价格：¥3999"));
        assert!(text.contains("触发：自动"));
        assert!(text.contains("🔗：https://item.m.jd.com/x"));
    }

    #[test]
    fn failed_shows_reason_not_order_id() {
        let mut o = outcome();
        o.order_id = String::new();
        o.error = "所有凭证均不可用".into();
        let m = render(&NotifyEvent::OrderFailed(o));
        assert_eq!(m.title, "❌ 提交失败");
        let text = m.to_plain_text();
        assert!(text.contains("原因：所有凭证均不可用"));
        assert!(!text.contains("订单号"));
    }

    #[test]
    fn empty_fields_are_dropped() {
        // 手动提交:无商品名/成色 → 用占位、且空字段不产出行。
        let o = OrderOutcome {
            inspect_sku_id: "INSP".into(),
            trigger: "manual".into(),
            order_id: "ORD".into(),
            price: String::new(),
            ..Default::default()
        };
        let m = render(&NotifyEvent::OrderSuccess(o));
        let text = m.to_plain_text();
        assert!(text.contains("名称：(手动提交)"));
        assert!(text.contains("触发：手动"));
        assert!(!text.contains("价格")); // price 空 → 无价格行
        assert!(!text.contains("成色")); // quality 空 → 无成色行
    }

    #[test]
    fn hit_alert_has_manual_warning() {
        let m = render(&NotifyEvent::HitAlert {
            product_name: "Watch".into(),
            price: "1200".into(),
            quality: "99新".into(),
            inspect_sku_id: "I".into(),
            youpin_sku_id: "Y".into(),
            link: "L".into(),
        });
        assert_eq!(m.title, "🔔 命中目标");
        assert!(m.to_plain_text().contains("未开启自动提交"));
    }

    #[test]
    fn status_change_prefixes_icon() {
        let m = render(&NotifyEvent::StatusChange {
            title: "凭证已耗尽".into(),
            detail: "请尽快更新凭证".into(),
        });
        assert!(m.title.starts_with("ℹ️"));
        assert!(m.to_plain_text().contains("说明：请尽快更新凭证"));
    }

    #[test]
    fn order_success_includes_credential_name() {
        let o = OrderOutcome {
            product_name: "iPhone 15".into(),
            price: "100".into(),
            order_id: "OID1".into(),
            credential_name: "苹果17号".into(),
            ..Default::default()
        };
        let msg = render_order_success(&o);
        let text = msg.to_plain_text();
        assert!(text.contains("苹果17号"), "should contain credential name, got: {}", text);
    }
}
