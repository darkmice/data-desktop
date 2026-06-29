import { useState } from 'react';
import { Button, Card, Input, Tag, Empty, cn } from '@talon-ui/react';
import { invoke } from '../lib/tauri';
import { notify, useStore } from '../store/useStore';
import type { Credential } from '../lib/types';

export function Credentials() {
  const creds = useStore((s) => s.creds);
  const activeIdx = useStore((s) => s.activeIdx);
  const setCreds = useStore((s) => s.setCreds);
  const setActiveIdx = useStore((s) => s.setActiveIdx);

  const [name, setName] = useState('');
  const [cookie, setCookie] = useState('');

  async function refresh() {
    try {
      setCreds(await invoke<Credential[]>('get_credentials'));
    } catch (e) {
      notify(`读取凭证失败: ${String(e)}`, 'err');
    }
  }

  async function add() {
    if (!cookie.trim()) {
      notify('请粘贴凭证内容', 'err');
      return;
    }
    try {
      const n = await invoke<number>('import_credential', {
        name: name.trim(),
        cookieStr: cookie.trim(),
      });
      notify(`已导入凭证(${n} 项字段)`);
      setName('');
      setCookie('');
      await refresh();
    } catch (e) {
      notify(`导入失败: ${String(e)}`, 'err');
    }
  }

  async function use(i: number) {
    await invoke('use_credential', { index: i });
    setActiveIdx(i);
  }

  async function remove(i: number) {
    await invoke('delete_credential', { index: i });
    await refresh();
  }

  // 手动解除风控/过期状态 → 改回 Active,可重新参与下单。
  async function clearStatus(i: number) {
    try {
      await invoke('clear_credential_status', { index: i });
      notify('已解除状态,凭证恢复可用');
      await refresh();
    } catch (e) {
      notify(`解除失败: ${String(e)}`, 'err');
    }
  }

  return (
    <div className="mx-auto flex max-w-3xl flex-col gap-tp-5">
      <h1 className="text-2xl font-semibold text-text-primary">凭证管理</h1>

      <Card className="flex flex-col gap-tp-3 p-tp-5">
        <div className="grid grid-cols-[160px_1fr] gap-tp-3">
          <Input
            className="selectable"
            placeholder="备注名(如 苹果17)"
            value={name}
            onChange={(e) => setName(e.target.value)}
          />
          <Input
            className="selectable"
            placeholder="粘贴凭证内容(整段)"
            value={cookie}
            onChange={(e) => setCookie(e.target.value)}
          />
        </div>
        <div>
          <Button variant="primary" onClick={add}>
            导入凭证
          </Button>
        </div>
      </Card>

      <Card className="flex flex-col gap-tp-2 p-tp-4">
        {creds.length === 0 ? (
          <Empty description="暂无凭证" />
        ) : (
          creds.map((c, i) => (
            <div
              key={i}
              className={cn(
                'flex items-center gap-tp-3 rounded-md border px-tp-3 py-tp-2',
                i === activeIdx ? 'border-primary-700 bg-primary-900/20' : 'border-border',
              )}
            >
              <span className="mono shrink-0 text-xs text-text-tertiary">#{i + 1}</span>
              <span className="flex-1 truncate text-sm text-text-primary" title={c.name || '未命名'}>
                {c.name || '未命名'}
              </span>
              {(() => {
                // 状态标签(不同色):可用=绿,风控=橙(可重试),过期=红(需换CK)。
                const status = c.status ?? (c.valid === false ? 'Expired' : 'Active');
                if (status === 'RiskControlled') {
                  // progress = 橙/黄色调:风控冷却中,可手动解除后重试。
                  return <Tag tone="progress" size="sm">风控</Tag>;
                }
                if (status === 'Expired') {
                  return <Tag tone="blocked" size="sm">已过期</Tag>;
                }
                return <Tag tone="done" size="sm">可用</Tag>;
              })()}
              {(c.status ?? 'Active') !== 'Active' && (
                <Button
                  variant="ghost"
                  size="sm"
                  className="shrink-0 whitespace-nowrap"
                  onClick={() => clearStatus(i)}
                  title="解除风控/过期状态,恢复可用"
                >
                  解除
                </Button>
              )}
              {i === activeIdx ? (
                <Tag tone="info" size="sm">
                  当前
                </Tag>
              ) : (
                <Button
                  variant="ghost"
                  size="sm"
                  className="shrink-0 whitespace-nowrap"
                  onClick={() => use(i)}
                >
                  设为当前
                </Button>
              )}
              <Button
                variant="ghost"
                size="sm"
                className="shrink-0 whitespace-nowrap"
                onClick={() => remove(i)}
              >
                删除
              </Button>
            </div>
          ))
        )}
      </Card>
    </div>
  );
}
