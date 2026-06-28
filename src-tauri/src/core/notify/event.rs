//! 通知事件:业务侧产生的、与渠道无关的结构化事件。每个变体携带渲染所需的全部
//! 字段;渲染逻辑在 [`super::template`],渠道无关。

use serde::{Deserialize, Serialize};

/// 事件类型标签。用于渠道订阅过滤(每个渠道在配置里勾选关心哪几类),以及给不同
/// 类型挑选不同的标题/图标。与 [`NotifyEvent`] 的变体一一对应。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    /// 成交成功。
    OrderSuccess,
    /// 成交失败。
    OrderFailed,
    /// 规则命中但未自动提交(auto_submit 关),提醒人工介入。
    HitAlert,
    /// 关键状态变化(凭证耗尽 / 风控触发 / 规则达配额停止等运维级提醒)。
    StatusChange,
}

impl EventKind {
    /// 配置里持久化用的稳定 key(与前端事件勾选项对齐)。
    pub fn as_key(self) -> &'static str {
        match self {
            EventKind::OrderSuccess => "order_success",
            EventKind::OrderFailed => "order_failed",
            EventKind::HitAlert => "hit_alert",
            EventKind::StatusChange => "status_change",
        }
    }
}

/// 一次成交尝试的结果快照(成功/失败共用,字段按场景填充)。
#[derive(Debug, Clone, Default)]
pub struct OrderOutcome {
    /// 商品名(手动提交可能为空)。
    pub product_name: String,
    pub price: String,
    pub quality: String,
    /// 质检报告编号(= inspect_sku_id)。
    pub inspect_sku_id: String,
    pub youpin_sku_id: String,
    /// 移动端商品详情链接。
    pub link: String,
    /// 触发方式:"auto"(规则自动)| "manual"(手动提交)。
    pub trigger: String,
    /// 成功时的订单号。
    pub order_id: String,
    /// 失败时的原因文案。
    pub error: String,
    /// 命中使用的凭证备注名(CK 备注)。
    pub credential_name: String,
}

/// 业务侧上报的通知事件。
#[derive(Debug, Clone)]
pub enum NotifyEvent {
    /// 下单成功。
    OrderSuccess(OrderOutcome),
    /// 下单失败。
    OrderFailed(OrderOutcome),
    /// 规则命中但未自动提交 —— 提醒人工去抢。
    HitAlert {
        product_name: String,
        price: String,
        quality: String,
        inspect_sku_id: String,
        youpin_sku_id: String,
        link: String,
    },
    /// 关键运维状态变化。`title` 一句话主旨,`detail` 可选补充。
    StatusChange { title: String, detail: String },
}

impl NotifyEvent {
    pub fn kind(&self) -> EventKind {
        match self {
            NotifyEvent::OrderSuccess(_) => EventKind::OrderSuccess,
            NotifyEvent::OrderFailed(_) => EventKind::OrderFailed,
            NotifyEvent::HitAlert { .. } => EventKind::HitAlert,
            NotifyEvent::StatusChange { .. } => EventKind::StatusChange,
        }
    }
}
