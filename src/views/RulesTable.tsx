import {
  useReactTable,
  getCoreRowModel,
  getPaginationRowModel,
  flexRender,
  createColumnHelper,
} from '@tanstack/react-table';
import { Button, Switch, Tag, cn } from '@talon-ui/react';
import type { Rule } from '../lib/types';

interface Props {
  rules: Rule[];
  onEdit: (r: Rule) => void;
  onDelete: (r: Rule) => void;
  onToggle: (r: Rule) => void;
  onReenable: (r: Rule) => void;
}

/** 规则状态:与 Dashboard 原有卡片逻辑完全一致。 */
function ruleStatus(r: Rule): { tone: 'done' | 'neutral' | 'idle' | 'blocked'; label: string } {
  const configured = !!r.youpin_sku_id && r.youpin_sku_id.trim() !== '';
  const exhausted = r.used >= r.qty;
  if (!configured) return { tone: 'idle', label: '未生效' };
  if (exhausted) return { tone: 'blocked', label: '已用尽' };
  if (!r.enabled) return { tone: 'neutral', label: '已停用' };
  return { tone: 'done', label: '生效中' };
}

function priceRange(r: Rule): string {
  const lo = r.price_min != null ? `¥${r.price_min}` : '不限';
  const hi = r.price_max != null ? `¥${r.price_max}` : '不限';
  if (lo === '不限' && hi === '不限') return '不限';
  return `${lo} ~ ${hi}`;
}

const col = createColumnHelper<Rule>();

const PAGE_SIZE = 50;

export function RulesTable({ rules, onEdit, onDelete, onToggle, onReenable }: Props) {
  const columns = [
    col.display({
      id: 'index',
      header: '#',
      cell: ({ row }) => (
        <span className="mono text-xs text-text-tertiary">{row.index + 1}</span>
      ),
    }),
    col.accessor('label', {
      header: '备注名',
      cell: ({ getValue }) => (
        <span className="text-sm text-text-primary">{getValue() || '—'}</span>
      ),
    }),
    col.accessor('youpin_sku_id', {
      header: 'youpinSkuId',
      cell: ({ getValue }) => (
        <span className="mono text-xs text-text-secondary">{getValue() || '—'}</span>
      ),
    }),
    col.display({
      id: 'price',
      header: '价格区间',
      cell: ({ row }) => (
        <span className="mono text-xs text-text-secondary">{priceRange(row.original)}</span>
      ),
    }),
    col.display({
      id: 'quota',
      header: '配额',
      cell: ({ row }) => {
        const r = row.original;
        return (
          <span className="mono text-xs text-text-secondary" title={`已成交 ${r.used} / 配额 ${r.qty}`}>
            {r.used}/{r.qty}
          </span>
        );
      },
    }),
    col.display({
      id: 'status',
      header: '状态',
      cell: ({ row }) => {
        const { tone, label } = ruleStatus(row.original);
        return (
          <Tag tone={tone} size="sm">
            {label}
          </Tag>
        );
      },
    }),
    col.display({
      id: 'enabled',
      header: '启用',
      cell: ({ row }) => (
        <Switch
          checked={row.original.enabled}
          onCheckedChange={() => onToggle(row.original)}
        />
      ),
    }),
    col.display({
      id: 'actions',
      header: '操作',
      cell: ({ row }) => {
        const r = row.original;
        const exhausted = r.used >= r.qty && !!r.youpin_sku_id && r.youpin_sku_id.trim() !== '';
        return (
          <div className="flex items-center gap-tp-2">
            {exhausted && (
              <Button variant="ghost" size="sm" onClick={() => onReenable(r)}>
                重启
              </Button>
            )}
            <Button variant="ghost" size="sm" onClick={() => onEdit(r)}>
              编辑
            </Button>
            <Button variant="ghost" size="sm" onClick={() => onDelete(r)}>
              删除
            </Button>
          </div>
        );
      },
    }),
  ];

  const table = useReactTable({
    data: rules,
    columns,
    getCoreRowModel: getCoreRowModel(),
    getPaginationRowModel: getPaginationRowModel(),
    initialState: { pagination: { pageSize: PAGE_SIZE, pageIndex: 0 } },
  });

  const { pageIndex, pageSize } = table.getState().pagination;
  const totalPages = table.getPageCount();

  return (
    <div className="flex flex-col gap-tp-2">
      <div className="overflow-x-auto rounded-md border border-border">
        {/* 表头 */}
        <div className="border-b border-border bg-bg-subtle">
          {table.getHeaderGroups().map((hg) => (
            <div
              key={hg.id}
              className="grid items-center gap-tp-3 px-tp-3 py-tp-2 text-xs text-text-tertiary"
              style={{ gridTemplateColumns: '32px 1fr 160px 140px 64px 72px 52px 120px' }}
            >
              {hg.headers.map((h) => (
                <div key={h.id}>
                  {flexRender(h.column.columnDef.header, h.getContext())}
                </div>
              ))}
            </div>
          ))}
        </div>

        {/* 行 */}
        <div>
          {table.getRowModel().rows.length === 0 ? (
            <div className="py-tp-6 text-center text-sm text-text-tertiary">暂无规则</div>
          ) : (
            table.getRowModel().rows.map((row) => (
              <div
                key={row.id}
                className={cn(
                  'grid items-center gap-tp-3 border-b border-subtle px-tp-3 py-tp-2 last:border-0',
                  !row.original.youpin_sku_id && 'opacity-60',
                )}
                style={{ gridTemplateColumns: '32px 1fr 160px 140px 64px 72px 52px 120px' }}
              >
                {row.getVisibleCells().map((cell) => (
                  <div key={cell.id}>
                    {flexRender(cell.column.columnDef.cell, cell.getContext())}
                  </div>
                ))}
              </div>
            ))
          )}
        </div>
      </div>

      {/* 分页控件(超过一页才显示) */}
      {totalPages > 1 && (
        <div className="flex items-center justify-end gap-tp-3 text-xs text-text-secondary">
          <Button
            variant="ghost"
            size="sm"
            disabled={!table.getCanPreviousPage()}
            onClick={() => table.previousPage()}
          >
            上一页
          </Button>
          <span>
            第 {pageIndex + 1} / {totalPages} 页（共 {rules.length} 条，每页 {pageSize} 条）
          </span>
          <Button
            variant="ghost"
            size="sm"
            disabled={!table.getCanNextPage()}
            onClick={() => table.nextPage()}
          >
            下一页
          </Button>
        </div>
      )}
    </div>
  );
}
