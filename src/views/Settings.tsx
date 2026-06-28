import { useEffect, useRef, useState } from 'react';
import { Button, Card, Input, Switch, cn } from '@talon-ui/react';
import { AlertCircle, CheckCircle2 } from 'lucide-react';
import { invoke } from '../lib/tauri';
import { notify, TX, useStore } from '../store/useStore';
import type { AppConfig, NotifyChannelConfig, NotifyEventKey } from '../lib/types';

/** 渠道类型元信息:展示名 + 加签提示。新增渠道只需在此加一项 + 给默认配置。 */
const CHANNEL_META: Record<string, { label: string; hint: string }> = {
  dingtalk: { label: '钉钉', hint: '机器人 Webhook;加签密钥可选(以 SEC 开头)' },
  feishu: { label: '飞书', hint: '自定义机器人 Webhook;加签密钥可选' },
};

/** 四类可推送事件。 */
const EVENT_OPTIONS: { key: NotifyEventKey; label: string }[] = [
  { key: 'order_success', label: '成交成功' },
  { key: 'order_failed', label: '成交失败' },
  { key: 'hit_alert', label: '命中告警' },
  { key: 'status_change', label: '状态变化' },
];

const ALL_EVENTS: NotifyEventKey[] = EVENT_OPTIONS.map((e) => e.key);

/** 渠道默认配置(新建/迁移用):默认订阅全部事件。 */
function defaultChannel(kind: string, webhook = '', secret = ''): NotifyChannelConfig {
  return { kind, enabled: webhook.trim() !== '', webhook, secret, events: [...ALL_EVENTS] };
}

/**
 * 从 config 推导初始渠道列表:
 *  - 有 notify_channels → 直接用,并补齐缺失的渠道类型(钉钉/飞书都展示出来)。
 *  - 否则若旧 dingtalk_webhook 非空 → 迁移成一个钉钉渠道。
 *  - 都没有 → 钉钉 + 飞书各一个空渠道(关闭态)。
 */
function initChannels(config: AppConfig | null): NotifyChannelConfig[] {
  const kinds = Object.keys(CHANNEL_META);
  let base: NotifyChannelConfig[] = [];
  if (config?.notify_channels && config.notify_channels.length > 0) {
    base = config.notify_channels.map((c) => ({ ...c, events: [...c.events] }));
  } else if (config?.dingtalk_webhook) {
    base = [defaultChannel('dingtalk', config.dingtalk_webhook, config.dingtalk_secret ?? '')];
  }
  // 确保每种渠道都有一条(没有的补空,方便用户开启)。
  for (const k of kinds) {
    if (!base.some((c) => c.kind === k)) base.push(defaultChannel(k));
  }
  // 按 CHANNEL_META 顺序排列。
  return kinds.map((k) => base.find((c) => c.kind === k)!);
}

export function Settings() {
  const config = useStore((s) => s.config);
  const setConfig = useStore((s) => s.setConfig);
  const conn = useStore((s) => s.conn);
  const connReason = useStore((s) => s.connReason);

  const [token, setToken] = useState(config?.token ?? '');
  // 签名通道固定 codex(getCurrentOrder→bd265 / submitOrder→cc85b)。paipai 通道已废弃,
  // 不再向用户暴露选择。保留常量以便 buildNext 持久化时写入正确值。
  const recipe = 'codex';
  const [channels, setChannels] = useState<NotifyChannelConfig[]>(() => initChannels(config));
  const [busy, setBusy] = useState(false);

  // config 是异步加载的:首次挂载时可能仍是 null(get_config 尚未返回),此时上面的
  // 惰性初始值是空表单。等 config 首次到达后同步一次表单——**只同步首次**,之后即便
  // config 因其它操作变化也不覆盖,以免打断用户正在进行的编辑。
  const hydrated = useRef(false);
  useEffect(() => {
    if (config && !hydrated.current) {
      hydrated.current = true;
      setToken(config.token ?? '');
      setChannels(initChannels(config));
    }
  }, [config]);

  /** 改某个渠道的字段。 */
  const patchChannel = (kind: string, patch: Partial<NotifyChannelConfig>) =>
    setChannels((prev) => prev.map((c) => (c.kind === kind ? { ...c, ...patch } : c)));

  /** 切换某渠道对某事件的订阅。 */
  const toggleEvent = (kind: string, ev: NotifyEventKey) =>
    setChannels((prev) =>
      prev.map((c) =>
        c.kind === kind
          ? {
              ...c,
              events: c.events.includes(ev)
                ? c.events.filter((e) => e !== ev)
                : [...c.events, ev],
            }
          : c,
      ),
    );

  /** 组装要持久化的 config:写 notify_channels,清空旧单钉钉字段避免双写。 */
  function buildNext(): AppConfig | null {
    if (!config) return null;
    return {
      ...config,
      token: token.trim(),
      sign_recipe: recipe,
      // 仅保留有 webhook 的渠道;trim 一遍。
      notify_channels: channels
        .filter((c) => c.webhook.trim() !== '')
        .map((c) => ({ ...c, webhook: c.webhook.trim(), secret: c.secret.trim() })),
      dingtalk_webhook: '',
      dingtalk_secret: '',
    };
  }

  async function saveAndConnect() {
    const next = buildNext();
    if (!next) return;
    setBusy(true);
    try {
      await invoke('save_config', { config: next });
      setConfig(next);
      if (!next.token) {
        notify('请填写访问 Token', 'err');
        return;
      }
      await invoke('connect');
      notify('正在准备研究功能…', 'info', TX.CONN);
    } catch (e) {
      notify(`研究功能准备失败: ${String(e)}`, 'err', TX.CONN);
    } finally {
      setBusy(false);
    }
  }

  async function saveOnly() {
    const next = buildNext();
    if (!next) return;
    try {
      await invoke('save_config', { config: next });
      setConfig(next);
      notify('设置已保存');
    } catch (e) {
      notify(`保存失败: ${String(e)}`, 'err');
    }
  }

  return (
    <div className="mx-auto flex max-w-2xl flex-col gap-tp-5">
      <h1 className="text-2xl font-semibold text-text-primary">设置</h1>

      <Card className="flex flex-col gap-tp-4 p-tp-5">
        <Field label="访问 Token">
          <Input
            className="selectable"
            placeholder="ak-…"
            value={token}
            onChange={(e) => setToken(e.target.value)}
          />
          <p className="mt-tp-1 text-xs text-text-tertiary">
            由管理员提供,已内含服务地址,无需单独填写地址。
          </p>
        </Field>

        <div className="flex gap-tp-3">
          <Button variant="primary" loading={busy} onClick={saveAndConnect}>
            保存并启用
          </Button>
          <Button variant="secondary" onClick={saveOnly}>
            仅保存
          </Button>
        </div>

        {conn === 'authed' ? (
          <div className="flex items-center gap-tp-2 text-sm text-status-done-fg">
            <CheckCircle2 size={15} />
            研究功能已就绪,Token 有效。
          </div>
        ) : connReason ? (
          <div className="flex items-start gap-tp-2 text-sm text-status-blocked-fg">
            <AlertCircle size={15} className="mt-px shrink-0" />
            <span>{connReason}</span>
          </div>
        ) : conn === 'connected' ? (
          <div className="text-sm text-text-tertiary">正在准备研究功能…</div>
        ) : null}
      </Card>

      {/* 通知渠道:钉钉 / 飞书,各含开关 + Webhook + 加签 + 事件订阅。 */}
      <Card className="flex flex-col gap-tp-4 p-tp-5">
        <div>
          <h2 className="text-base font-medium text-text-primary">通知渠道</h2>
          <p className="mt-tp-1 text-xs text-text-tertiary">
            成交成功 / 失败、命中告警、关键状态变化可分别推送到各渠道。
          </p>
        </div>
        {channels.map((c) => (
          <ChannelEditor
            key={c.kind}
            channel={c}
            onPatch={(patch) => patchChannel(c.kind, patch)}
            onToggleEvent={(ev) => toggleEvent(c.kind, ev)}
          />
        ))}
        <div>
          <Button variant="secondary" onClick={saveOnly}>
            保存
          </Button>
        </div>
      </Card>
    </div>
  );
}

function ChannelEditor({
  channel: c,
  onPatch,
  onToggleEvent,
}: {
  channel: NotifyChannelConfig;
  onPatch: (patch: Partial<NotifyChannelConfig>) => void;
  onToggleEvent: (ev: NotifyEventKey) => void;
}) {
  const meta = CHANNEL_META[c.kind] ?? { label: c.kind, hint: '' };
  return (
    <div className="flex flex-col gap-tp-3 rounded-md border border-border p-tp-3">
      <div className="flex items-center gap-tp-3">
        <span className="text-sm font-medium text-text-primary">{meta.label}</span>
        <div className="flex-1" />
        <label className="flex items-center gap-tp-2 text-xs text-text-secondary">
          {c.enabled ? '已启用' : '已停用'}
          <Switch checked={c.enabled} onCheckedChange={(v: boolean) => onPatch({ enabled: v })} />
        </label>
      </div>

      {/* 启用时才展开详细配置,关闭时只留开关,界面更清爽。 */}
      {c.enabled && (
        <>
          <Field label="Webhook">
            <Input
              className="selectable"
              placeholder={
                c.kind === 'dingtalk'
                  ? 'https://oapi.dingtalk.com/robot/send?access_token=…'
                  : 'https://open.feishu.cn/open-apis/bot/v2/hook/…'
              }
              value={c.webhook}
              onChange={(e) => onPatch({ webhook: e.target.value })}
            />
          </Field>
          <Field label="加签密钥(可选)">
            <Input
              className="selectable"
              placeholder="留空则不加签"
              value={c.secret}
              onChange={(e) => onPatch({ secret: e.target.value })}
            />
          </Field>
          <div className="flex flex-col gap-tp-2">
            <span className="text-xs text-text-secondary">推送事件</span>
            <div className="flex flex-wrap gap-tp-2">
              {EVENT_OPTIONS.map((ev) => {
                const on = c.events.includes(ev.key);
                return (
                  <button
                    key={ev.key}
                    type="button"
                    onClick={() => onToggleEvent(ev.key)}
                    className={cn(
                      'rounded-md border px-tp-3 py-tp-1 text-xs transition',
                      on
                        ? 'border-primary-600 bg-primary-600 text-white'
                        : 'border-border text-text-secondary hover:border-border-strong',
                    )}
                  >
                    {ev.label}
                  </button>
                );
              })}
            </div>
          </div>
          <p className="text-xs text-text-tertiary">{meta.hint}</p>
        </>
      )}
    </div>
  );
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <label className="flex flex-col gap-tp-1">
      <span className="text-sm text-text-secondary">{label}</span>
      {children}
    </label>
  );
}
