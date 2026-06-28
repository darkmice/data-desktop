//! 多渠道通知子系统。
//!
//! 设计目标:把「**什么事件**(成交成功/失败、命中告警、关键状态变化)」、
//! 「**渲染成什么文案**(模板)」、「**发到哪个渠道**(钉钉 / 飞书 / 未来更多)」
//! 三件事彻底解耦,后续新增一个渠道只需实现一个 [`Channel`],新增一类事件只需
//! 在 [`event`] 里加一个变体 + 在 [`template`] 里加它的渲染。
//!
//! 数据流:
//! ```text
//!   业务侧 ── NotifyEvent ──▶ Notifier.notify()
//!                               │  render(event) → RenderedMessage(中性中间表示)
//!                               └─ 扇出到每个启用渠道:Channel.send(&msg)
//! ```
//! 渲染只做一次,与渠道无关;每个渠道把中性的 [`RenderedMessage`] 转成自己的报文
//! 格式(钉钉 text、飞书 text…)。任一渠道发送失败互不影响(best-effort),错误
//! 仅记日志,不阻断下单主流程。

pub mod channel;
pub mod event;
pub mod template;

pub use channel::{Channel, DingTalkChannel, FeishuChannel};
pub use event::{NotifyEvent, OrderOutcome};
pub use template::{render, RenderedMessage};

use std::sync::Arc;

/// 通知分发器:持有一组已启用渠道,把一个事件渲染后扇出给全部渠道。
///
/// 渠道按配置构建(见 [`Notifier::from_channels`]);构建时已过滤掉「未启用 / 未配
/// webhook / 该事件未订阅」的渠道,所以 [`notify`](Self::notify) 只管发。
#[derive(Clone)]
pub struct Notifier {
    client: reqwest::Client,
    channels: Arc<Vec<Box<dyn Channel>>>,
}

impl Notifier {
    pub fn new(client: reqwest::Client, channels: Vec<Box<dyn Channel>>) -> Self {
        Self {
            client,
            channels: Arc::new(channels),
        }
    }

    /// 是否有任何渠道会响应该事件类型(用于业务侧在渲染前短路,省掉无谓的字符串构造)。
    pub fn has_subscriber(&self, event: &NotifyEvent) -> bool {
        self.channels.iter().any(|c| c.subscribes(event.kind()))
    }

    /// 渲染并扇出。每个订阅了该事件类型的渠道各自发送;失败仅记日志,不影响其它渠道
    /// 与主流程。无订阅者时直接返回(连模板都不渲染)。
    pub async fn notify(&self, event: NotifyEvent) {
        if !self.has_subscriber(&event) {
            return;
        }
        let msg = render(&event);
        let kind = event.kind();
        for ch in self.channels.iter() {
            if !ch.subscribes(kind) {
                continue;
            }
            if let Err(e) = ch.send(&self.client, &msg).await {
                tracing::warn!(channel = ch.name(), error = %e, "通知渠道发送失败");
            }
        }
    }
}
