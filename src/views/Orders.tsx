import { useCallback, useEffect, useState } from 'react';
import { Button, Card, Pagination, SegmentedControl, Statistic } from '@talon-ui/react';
import { OrderTable } from '../components/OrderTable';
import { invoke } from '../lib/tauri';
import { ORDER_RECORDED } from '../lib/events';
import { useStore } from '../store/useStore';
import type { OrderPage, OrderRecord, OrderStats } from '../lib/types';

const PAGE_SIZE = 20;

export function Orders() {
  const stats = useStore((s) => s.stats);
  const setStats = useStore((s) => s.setStats);
  const pushLog = useStore((s) => s.pushLog);

  const [filter, setFilter] = useState<'all' | 'success' | 'failed'>('all');
  const [page, setPage] = useState(1);
  const [items, setItems] = useState<OrderRecord[]>([]);
  const [total, setTotal] = useState(0);

  const load = useCallback(async () => {
    try {
      const res = await invoke<OrderPage>('get_orders', { filter, page, pageSize: PAGE_SIZE });
      setItems(res.items);
      setTotal(res.total);
      setStats(await invoke<OrderStats>('get_order_stats'));
    } catch (e) {
      pushLog(`读取记录失败: ${String(e)}`, 'err');
    }
  }, [filter, page, setStats, pushLog]);

  useEffect(() => {
    void load();
  }, [load]);

  // 实时:新记录到达 → 第1页且 all/匹配过滤时插顶;否则只靠 stats(events 已刷)。
  useEffect(() => {
    const onRecorded = (e: Event) => {
      const rec = (e as CustomEvent<OrderRecord>).detail;
      const matches =
        filter === 'all' || (filter === 'success' ? rec.status === 'success' : rec.status === 'failed');
      if (page === 1 && matches) {
        setItems((prev) => [rec, ...prev].slice(0, PAGE_SIZE));
        setTotal((t) => t + 1);
      } else if (matches) {
        setTotal((t) => t + 1);
      }
    };
    window.addEventListener(ORDER_RECORDED, onRecorded);
    return () => window.removeEventListener(ORDER_RECORDED, onRecorded);
  }, [filter, page]);

  async function clearAll() {
    if (!window.confirm('确定清空所有下单记录?此操作不可恢复。')) return;
    try {
      await invoke('clear_orders');
      setPage(1);
      await load();
      pushLog('下单记录已清空');
    } catch (e) {
      pushLog(`清空失败: ${String(e)}`, 'err');
    }
  }

  return (
    <div className="flex flex-col gap-tp-5">
      <h1 className="text-2xl font-semibold text-text-primary">下单记录</h1>

      {/* 统计 */}
      <div className="grid grid-cols-3 gap-tp-4">
        <Card className="p-tp-4">
          <Statistic label="总下单" value={stats.total} />
        </Card>
        <Card className="p-tp-4">
          <Statistic label="成功" value={stats.success} />
        </Card>
        <Card className="p-tp-4">
          <Statistic label="失败" value={stats.failed} />
        </Card>
      </div>

      {/* 工具行 */}
      <div className="flex items-center justify-between">
        <SegmentedControl
          value={filter}
          onValueChange={(v) => {
            setFilter(v as typeof filter);
            setPage(1);
          }}
          items={[
            { value: 'all', label: '全部' },
            { value: 'success', label: '成功' },
            { value: 'failed', label: '失败' },
          ]}
        />
        <Button variant="ghost" size="sm" onClick={clearAll}>
          清空记录
        </Button>
      </div>

      {/* 表格 */}
      <Card className="p-tp-2">
        <OrderTable items={items} pageBase={(page - 1) * PAGE_SIZE} />
      </Card>

      {/* 分页 */}
      {total > PAGE_SIZE && (
        <div className="flex justify-end">
          <Pagination
            page={page}
            total={total}
            pageSize={PAGE_SIZE}
            onChange={(p) => setPage(p)}
          />
        </div>
      )}
    </div>
  );
}
