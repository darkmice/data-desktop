// 统一订阅后端事件,写入全局 store。应用挂载后调用一次 setupEvents()。
//   conn          → 连接灯
//   log           → 运行日志
//   sku_hit       → 命中吐司
//   order_recorded→ 刷新记录统计 + 派发 DOM 事件给「下单记录」页实时插行

import { invoke, listen } from './tauri';
import { logKind, notify, TX, useStore } from '../store/useStore';
import type {
  Category,
  ConnStatus,
  OrderRecord,
  OrderStats,
  WatchParams,
} from './types';

/** 「下单记录」页监听这个事件实时插入新行。 */
export const ORDER_RECORDED = 'paipai:order-recorded';

let started = false;

export async function setupEvents(): Promise<void> {
  if (started) return;
  started = true;
  const s = useStore.getState();

  await listen<{ status: ConnStatus; reason?: string }>('conn', (p) => {
    s.setConn(p.status, p.reason);
    // 连接结果与「正在连接…」共用 TX.CONN → 覆盖同一条吐司,不叠加。
    if (p.status === 'disconnected' && p.reason) notify(p.reason, 'err', TX.CONN);
    if (p.status === 'authed') notify('研究功能已就绪', 'hit', TX.CONN);
  });

  await listen<{ msg: string }>('log', (p) => {
    const msg = p.msg ?? '';
    const kind = logKind(msg);
    // 后端推送的「成功/命中/失败/错误」是重要信息 → 吐司+日志;普通流水仅日志。
    if (kind === 'err' || kind === 'hit') notify(msg, kind);
    else s.pushLog(msg, kind);
  });

  // 服务端下发本 token 启用品类(鉴权后 + admin 改动后)。客户端只读消费。
  await listen<{ items: Category[] }>('categories', (p) => {
    s.setServerCategories(p.items ?? []);
  });

  // 服务端下发本 token 监控扫描参数(鉴权后 + admin 改动后)。客户端只读展示。
  await listen<WatchParams>('watch_params', (p) => {
    s.setServerParams({
      page_from: p.page_from ?? 1,
      page_to: p.page_to ?? 5,
      interval: p.interval ?? 3,
      max_threads: p.max_threads ?? 5,
    });
  });

  // 监控存活脉冲:服务端每扫完一圈下发一次(空载,不含任何扫描细节)。仅用于
  // 刷新「运行中 · 上次活动」指示器,不写日志、不弹吐司。
  await listen('heartbeat', () => {
    s.beat();
  });

  await listen<{ short_name?: string; price?: unknown; quality_name?: string }>(
    'sku_hit',
    (p) => {
      notify(`命中: ${p.short_name ?? ''} ¥${p.price ?? ''} ${p.quality_name ?? ''}`, 'hit');
    },
  );

  await listen<OrderRecord>('order_recorded', async (rec) => {
    // 刷新统计
    try {
      const stats = await invoke<OrderStats>('get_order_stats');
      useStore.getState().setStats(stats);
    } catch {
      /* 记录功能不可用时忽略 */
    }
    // 通知记录页插行
    window.dispatchEvent(new CustomEvent<OrderRecord>(ORDER_RECORDED, { detail: rec }));
  });
}
