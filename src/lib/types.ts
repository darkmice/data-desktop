// 与 Rust 端结构对齐的前端类型。
// 注意命名风格:
//  - AppConfig / Credential / Rule:Rust 用 snake_case 且无 serde rename,
//    所以这里也用 snake_case(invoke 传参/返回都按此)。
//  - OrderRecord:Rust 标了 #[serde(rename_all="camelCase")],故用 camelCase。

/** 通知事件 key(与 Rust EventKind::as_key 对齐)。 */
export type NotifyEventKey = 'order_success' | 'order_failed' | 'hit_alert' | 'status_change';

/** 单个通知渠道配置(与 Rust NotifyChannelConfig 对齐)。 */
export interface NotifyChannelConfig {
  /** "dingtalk" | "feishu" */
  kind: string;
  enabled: boolean;
  webhook: string;
  /** 加签密钥,可选 */
  secret: string;
  /** 订阅的事件 key 列表 */
  events: NotifyEventKey[];
}

export interface AppConfig {
  server_url: string;
  token: string;
  insecure_tls: boolean;
  auto_submit: boolean;
  auto_submit_delay_secs: number;
  /** 旧版单钉钉配置,仅向后兼容(新 UI 写 notify_channels)。 */
  dingtalk_webhook: string;
  dingtalk_secret: string;
  /** 多渠道通知配置(钉钉/飞书/…)。 */
  notify_channels: NotifyChannelConfig[];
  /** "paipai"(通道A) | "codex"(通道B) */
  sign_recipe: string;
  /** "dark" | "light" */
  theme: string;
}

/**
 * 关注品类。**由服务端下发**(全局目录 + 按 token 启用),客户端只读消费:
 * 连接鉴权后服务端通过 `categories` 事件推送本 token 启用的品类,用户在监控台
 * 勾选要扫的(按 key)。客户端不再持有或编辑品类参数。
 */
export interface Category {
  /** 稳定唯一标识(服务端目录 key)。 */
  key: string;
  /** 显示名,如 "iPhone"。 */
  label: string;
  /** ★核心:决定返回商品类型(仅展示用,客户端不编辑)。 */
  spu_id: string;
  category_id: string;
  vender_id: string;
  std_category_id: string;
}

/**
 * 监控扫描参数。**由服务端下发**(全局默认 + 按 token 覆盖,在管理端配置);
 * 客户端只读展示,不再编辑。
 */
export interface WatchParams {
  page_from: number;
  page_to: number;
  interval: number;
  max_threads: number;
}

/** 凭证状态(与 Rust CredStatus 对齐:serde 把无字段枚举序列化为变体名字符串)。
 *  - Active:可用
 *  - RiskControlled:触发风控(601),间歇性,可手动解除后重试(前端橙色)
 *  - Expired:登录态失效(302/CK 过期),需换 CK(前端红色)
 *  - Disabled:用户手动禁用,不参与提交流程 */
export type CredStatus = 'Active' | 'RiskControlled' | 'Expired' | 'Disabled';

export interface Credential {
  name: string;
  cookie_str: string;
  status: CredStatus;
  last_alive_check_ms?: number;
  last_alive_ok?: boolean | null;
  last_alive_message?: string;
  /** 兼容旧字段;新代码用 status。 */
  valid?: boolean;
}

export interface Rule {
  id: string;
  label: string;
  /** ★匹配键:精确 youpinSkuId。为空则规则未生效。 */
  youpin_sku_id?: string | null;
  /** 价格区间;0 或空 = 该侧不限。 */
  price_min?: number | null;
  price_max?: number | null;
  qty: number;
  used: number;
  enabled: boolean;
}

export interface OrderRecord {
  id: number;
  createdAt: number;
  status: 'success' | 'failed';
  trigger: 'manual' | 'auto';
  inspectSkuId: string;
  youpinSkuId: string;
  shortName: string;
  price: string;
  quality: string;
  /** JD 相对图片路径(展示层拼 CDN 前缀);可能为空。 */
  image: string;
  orderId: string;
  credential: string;
  error: string;
  ruleId: string;
}

export interface OrderStats {
  total: number;
  success: number;
  failed: number;
}

export interface OrderPage {
  items: OrderRecord[];
  total: number;
}

export type ConnStatus = 'connected' | 'authed' | 'disconnected';

export interface LogLine {
  ts: number;
  msg: string;
  kind: 'info' | 'hit' | 'err';
}
