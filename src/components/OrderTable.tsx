import { useState } from 'react';
import { Tag, Empty, cn, Modal, ModalContent } from '@talon-ui/react';
import { ExternalLink } from 'lucide-react';
import { openExternal } from '../lib/tauri';
import type { OrderRecord } from '../lib/types';

const COLS = '120px 64px 40px 1fr 90px 70px 90px 60px 1.4fr';

/** JD 相对图片路径 → CDN URL(n1 质检图)。 */
function imgUrl(image: string): string {
  return image ? `https://img10.360buyimg.com/n1/${image}` : '';
}

function itemLink(youpin: string, inspect: string): string {
  return `https://item.m.jd.com/product/${youpin}.html?inspectSkuId=${inspect}`;
}

export function OrderTable({ items }: { items: OrderRecord[] }) {
  // 商品预览(iframe Modal):非空即打开。
  const [preview, setPreview] = useState<{ youpin: string; inspect: string; name?: string } | null>(
    null,
  );
  const open = (r: OrderRecord) => {
    if (r.youpinSkuId) {
      setPreview({ youpin: r.youpinSkuId, inspect: r.inspectSkuId, name: r.shortName });
    }
  };

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
        <span>时间</span>
        <span>状态</span>
        <span>图</span>
        <span>商品</span>
        <span>价格</span>
        <span>成色</span>
        <span>凭证</span>
        <span>触发</span>
        <span>订单号 / 失败原因</span>
      </div>

      {/* 行 */}
      {items.map((r) => (
        <div
          key={r.id}
          className="grid items-center gap-tp-3 border-b border-subtle px-tp-3 py-tp-2 text-sm"
          style={{ gridTemplateColumns: COLS }}
        >
          <span className="mono text-xs text-text-secondary">{fmtTime(r.createdAt)}</span>
          <span>
            <Tag tone={r.status === 'success' ? 'done' : 'blocked'} size="sm">
              {r.status === 'success' ? '成功' : '失败'}
            </Tag>
          </span>
          {r.image ? (
            <button title="查看商品" onClick={() => open(r)}>
              <img
                src={imgUrl(r.image)}
                alt=""
                loading="lazy"
                className="h-9 w-9 rounded border border-border object-cover"
              />
            </button>
          ) : (
            <span className="flex h-9 w-9 items-center justify-center rounded border border-border text-[10px] text-text-tertiary">
              无
            </span>
          )}
          <button
            className="truncate text-left text-text-primary hover:text-primary-300"
            title={r.shortName || '(手动提交,无商品名)'}
            onClick={() => open(r)}
          >
            {r.shortName || `SKU ${r.youpinSkuId || '—'}`}
          </button>
          <span className="mono text-text-secondary">{r.price ? `¥${r.price}` : '—'}</span>
          <span className="text-text-secondary">{r.quality || '—'}</span>
          <span className="truncate text-text-secondary">{r.credential || '—'}</span>
          <span>
            <Tag tone={r.trigger === 'auto' ? 'info' : 'neutral'} size="sm">
              {r.trigger === 'auto' ? '自动' : '手动'}
            </Tag>
          </span>
          <span
            className={cn(
              'truncate text-xs',
              r.status === 'success' ? 'mono text-text-secondary' : 'text-danger-500',
            )}
            title={r.status === 'success' ? r.orderId : r.error}
          >
            {r.status === 'success' ? r.orderId || '—' : r.error || '—'}
          </span>
        </div>
      ))}

      {/* 商品预览:手机尺寸 Modal,iframe 内嵌移动端详情页;无 footer。 */}
      <Modal open={!!preview} onOpenChange={(o) => !o && setPreview(null)}>
        <ModalContent className="w-[390px] max-w-[390px] gap-0 overflow-hidden p-0">
          {/* 标题栏:右侧给 Modal 自带关闭按钮留位(pr-tp-8),不再自加 X。 */}
          <div className="flex items-center gap-tp-2 border-b border-border py-tp-2 pl-tp-3 pr-tp-8">
            <span className="truncate text-sm font-medium text-text-primary">
              {preview?.name || '商品详情'}
            </span>
            <div className="flex-1" />
            <button
              onClick={() => preview && openExternal(itemLink(preview.youpin, preview.inspect))}
              className="inline-flex items-center gap-tp-1 text-xs text-text-tertiary hover:text-text-primary"
              title="用系统浏览器打开"
            >
              <ExternalLink size={13} />
              浏览器
            </button>
          </div>
          {preview && (
            <iframe
              src={itemLink(preview.youpin, preview.inspect)}
              title="商品详情"
              className="h-[78vh] w-full bg-bg-subtle"
              referrerPolicy="no-referrer"
              sandbox="allow-scripts allow-same-origin allow-popups allow-forms"
            />
          )}
        </ModalContent>
      </Modal>
    </div>
  );
}

function fmtTime(ms: number): string {
  const d = new Date(ms);
  const p = (n: number) => String(n).padStart(2, '0');
  return `${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`;
}
