import { useState } from 'react';
import { Tag, Empty, cn, Modal, ModalContent } from '@talon-ui/react';
import { Copy } from 'lucide-react';
import { notify } from '../store/useStore';
import type { OrderRecord } from '../lib/types';

// 列:序号/时间/状态/凭证/inspectSkuId/youpinSkuId/商品名称/价格/成色/触发/订单号或失败原因。
// 去掉图片列(省空间);id 列等宽 mono,可手动选中复制。
const COLS = '48px 116px 56px 90px 1.3fr 1.1fr 1.4fr 80px 64px 56px 1.4fr';

function itemLink(youpin: string, inspect: string): string {
  return `https://item.m.jd.com/product/${youpin}.html?inspectSkuId=${inspect}`;
}

export function OrderTable({ items, pageBase = 0 }: { items: OrderRecord[]; pageBase?: number }) {
  // 商品信息(可复制)弹层:非空即打开。不再内嵌需登录的 iframe。
  const [info, setInfo] = useState<OrderRecord | null>(null);

  if (items.length === 0) {
    return (
      <div className="py-tp-8">
        <Empty description="暂无下单记录" />
      </div>
    );
  }

  return (
    <div className="overflow-x-auto">
      {/* 表头 */}
      <div
        className="grid gap-tp-3 border-b border-border px-tp-3 py-tp-2 text-xs text-text-tertiary"
        style={{ gridTemplateColumns: COLS }}
      >
        <span>序号</span>
        <span>时间</span>
        <span>状态</span>
        <span>凭证</span>
        <span>inspectSkuId</span>
        <span>youpinSkuId</span>
        <span>商品名称</span>
        <span>价格</span>
        <span>成色</span>
        <span>触发</span>
        <span>订单号 / 失败原因</span>
      </div>

      {/* 行 */}
      {items.map((r, i) => (
        <div
          key={r.id}
          className="grid items-center gap-tp-3 border-b border-subtle px-tp-3 py-tp-2 text-sm"
          style={{ gridTemplateColumns: COLS }}
        >
          <span className="text-text-tertiary">{pageBase + i + 1}</span>
          <span className="mono text-xs text-text-secondary">{fmtTime(r.createdAt)}</span>
          <span>
            <Tag tone={r.status === 'success' ? 'done' : 'blocked'} size="sm">
              {r.status === 'success' ? '成功' : '失败'}
            </Tag>
          </span>
          <span className="truncate selectable text-text-secondary" title={r.credential}>
            {r.credential || '—'}
          </span>
          <span className="mono truncate selectable text-text-secondary" title={r.inspectSkuId}>
            {r.inspectSkuId || '—'}
          </span>
          <span className="mono truncate selectable text-text-secondary" title={r.youpinSkuId}>
            {r.youpinSkuId || '—'}
          </span>
          <button
            className="truncate text-left text-text-primary hover:text-primary-300"
            title={`${r.shortName || '(手动提交,无商品名)'}（点击复制链接 / ID）`}
            onClick={() => setInfo(r)}
          >
            {r.shortName || `SKU ${r.youpinSkuId || '—'}`}
          </button>
          <span className="mono text-text-secondary">{r.price ? `¥${r.price}` : '—'}</span>
          <span className="text-text-secondary">{r.quality || '—'}</span>
          <span>
            <Tag tone={r.trigger === 'auto' ? 'info' : 'neutral'} size="sm">
              {r.trigger === 'auto' ? '自动' : '手动'}
            </Tag>
          </span>
          <span
            className={cn(
              'truncate selectable text-xs',
              r.status === 'success' ? 'mono text-text-secondary' : 'text-danger-500',
            )}
            title={r.status === 'success' ? r.orderId : r.error}
          >
            {r.status === 'success' ? r.orderId || '—' : r.error || '—'}
          </span>
        </div>
      ))}

      {/* 商品信息(可复制):列出链接 + 两个 SkuId,各一键复制。不再嵌 iframe。 */}
      <Modal open={!!info} onOpenChange={(o) => !o && setInfo(null)}>
        <ModalContent className="w-[440px] max-w-[440px] gap-tp-3 p-tp-5">
          <span
            className="truncate pr-tp-6 text-sm font-medium text-text-primary"
            title={info?.shortName}
          >
            {info?.shortName || '商品信息'}
          </span>
          {info && (
            <div className="flex flex-col gap-tp-2">
              <CopyRow
                label="商品链接"
                value={itemLink(info.youpinSkuId, info.inspectSkuId)}
                what="商品链接"
              />
              <CopyRow label="inspectSkuId" value={info.inspectSkuId} what="inspectSkuId" />
              <CopyRow label="youpinSkuId" value={info.youpinSkuId} what="youpinSkuId" />
            </div>
          )}
        </ModalContent>
      </Modal>
    </div>
  );
}

/** 一行可复制信息:标签 + 可选中的值 + 复制按钮。 */
function CopyRow({ label, value, what }: { label: string; value: string; what: string }) {
  function copy() {
    // Tauri webview 下 clipboard 可能因权限/焦点静默失败 —— 成功才提示"已复制",
    // 失败提示手动选中,避免误以为已复制。
    navigator.clipboard.writeText(value).then(
      () => notify(`已复制${what}`, 'hit'),
      () => notify('复制失败,请手动选中', 'err'),
    );
  }
  return (
    <div className="flex flex-col gap-tp-1">
      <span className="text-xs text-text-tertiary">{label}</span>
      <div className="flex items-center gap-tp-2">
        <span
          className="mono selectable flex-1 truncate rounded-md bg-bg-subtle px-tp-2 py-tp-1 text-xs text-text-secondary"
          title={value}
        >
          {value || '—'}
        </span>
        <button
          onClick={copy}
          className="inline-flex items-center gap-tp-1 rounded-md border border-border px-tp-2 py-tp-1 text-xs text-text-secondary hover:text-text-primary"
          title="复制"
        >
          <Copy size={13} />
          复制
        </button>
      </div>
    </div>
  );
}

function fmtTime(ms: number): string {
  const d = new Date(ms);
  const p = (n: number) => String(n).padStart(2, '0');
  return `${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`;
}
