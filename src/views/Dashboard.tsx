import { useEffect, useRef, useState } from 'react';
import { Button, Card, Input, Switch, Empty, cn } from '@talon-ui/react';
import { invoke } from '../lib/tauri';
import { notify, useStore } from '../store/useStore';
import type { AppConfig, Rule } from '../lib/types';
import { RulesTable } from './RulesTable';
import { RuleFormModal } from './RuleFormModal';

/** 心跳超过此时长(ms)未刷新即判定停滞 → 显示「连接不稳 · 正在重连」。
 *  心跳频率 = 每扫完一个品类一次 + 每轮末一次,正常间隔约几秒。阈值取 45s 是为了
 *  覆盖「单个品类页数多 + 某次接口偶发慢/重试」的最坏单品类耗时,避免正常运行被误报。 */
const HEARTBEAT_STALE_MS = 45_000;

export function Dashboard() {
  const { conn, watching, logs, config } = useStore();
  const { setWatching, pushLog, clearLogs, setConfig } = useStore();
  const lastBeat = useStore((s) => s.lastBeat);

  // 每秒重渲染一次,让「上次活动 N 秒前」实时跳动、超时自动转停滞态。仅监控中才跑。
  const [, forceTick] = useState(0);
  useEffect(() => {
    if (!watching) return;
    const id = window.setInterval(() => forceTick((n) => n + 1), 1000);
    return () => window.clearInterval(id);
  }, [watching]);

  // 关注品类由服务端下发(连接鉴权后推送);客户端只读勾选。监控参数(页/间隔/
  // 线程)也由服务端按凭据配置,但不在界面展示。
  const categories = useStore((s) => s.serverCategories);

  const [cats, setCats] = useState<string[]>([]);
  // 服务端下发的品类变化时,把已选项收敛到仍存在的 key(被删/禁用的自动取消)。
  // 仅**首次**拿到品类时默认全选;之后即使用户手动清空了全部,也不因服务端再次
  // 下发(如 admin 改动)而被重新全选——尊重用户的显式选择。
  const initialized = useRef(false);
  useEffect(() => {
    setCats((prev) => {
      const keys = new Set(categories.map((c) => c.key));
      const kept = prev.filter((k) => keys.has(k));
      if (!initialized.current && categories.length > 0) {
        initialized.current = true;
        return categories.map((c) => c.key);
      }
      return kept;
    });
  }, [categories]);

  const [net, setNet] = useState<number>(-1);
  const [api, setApi] = useState<number>(-1);

  // 手动下单输入值放 store(切换界面不清空,除非用户手动改/清)。
  const mInspect = useStore((s) => s.manualInspect);
  const mYoupin = useStore((s) => s.manualYoupin);
  const setMInspect = useStore((s) => s.setManualInspect);
  const setMYoupin = useStore((s) => s.setManualYoupin);

  const logRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    logRef.current?.scrollTo(0, logRef.current.scrollHeight);
  }, [logs]);

  // 速度轮询
  useEffect(() => {
    let alive = true;
    const tick = async () => {
      try {
        const r = await invoke<{ net_ms: number; api_ms: number }>('ping_jd');
        if (alive) {
          setNet(r.net_ms);
          setApi(r.api_ms);
        }
      } catch {
        /* ignore */
      }
    };
    void tick();
    const id = window.setInterval(tick, 10_000);
    return () => {
      alive = false;
      window.clearInterval(id);
    };
  }, []);

  const authed = conn === 'authed';

  async function toggleWatch() {
    if (watching) {
      try {
        await invoke('stop_watch');
        setWatching(false);
        notify('监控已停止', 'info');
      } catch (e) {
        notify(`停止失败: ${String(e)}`, 'err');
      }
      return;
    }
    if (cats.length === 0) {
      notify('请至少选择一个关注品类', 'err');
      return;
    }
    try {
      // 监控参数由服务端按 token 决定;客户端只传要扫的品类 key。
      await invoke('start_watch', { categoryKeys: cats });
      setWatching(true);
      notify('监控已启动', 'hit');
    } catch (e) {
      notify(`启动失败: ${String(e)}`, 'err');
    }
  }

  async function setAuto(v: boolean) {
    if (!config) return;
    const next: AppConfig = { ...config, auto_submit: v };
    await invoke('save_config', { config: next });
    setConfig(next);
    notify(v ? '已开启自动提交' : '已关闭自动提交,仅告警', 'info');
  }

  async function manualSubmit() {
    if (!mInspect.trim() || !mYoupin.trim()) {
      notify('请填写 inspectSkuId 和 youpinSkuId', 'err');
      return;
    }
    pushLog(`手动提交: ${mInspect}`); // 流水信息,仅日志
    try {
      const r = await invoke<{ success: boolean; order_id: string; price: string; error: string; logs: string[] }>(
        'manual_submit',
        { inspectId: mInspect.trim(), youpinId: mYoupin.trim() },
      );
      (r.logs || []).forEach((m) => pushLog(m)); // 明细仅日志
      if (r.success) notify(`提交成功 #${r.order_id} ¥${r.price}`, 'hit');
      else notify(`提交失败: ${r.error}`, 'err');
    } catch (e) {
      notify(`提交失败: ${String(e)}`, 'err');
    }
  }

  return (
    <div className="flex flex-col gap-tp-5">
      {/* 顶部:连接 + 速度 + 监控开关 */}
      <div className="flex items-center gap-tp-4">
        <h1 className="text-2xl font-semibold text-text-primary">监控台</h1>
        {watching && <LiveStatus lastBeat={lastBeat} />}
        <div className="flex-1" />
        <Metric label="网络" ms={net} />
        <Metric label="接口" ms={api} />
        <Button
          variant={watching ? 'danger' : 'primary'}
          disabled={!authed}
          onClick={toggleWatch}
        >
          {watching ? '停止监控' : '开始监控'}
        </Button>
      </div>

      {/* 监控配置 + 统计 */}
      <Card className="flex flex-col gap-tp-4 p-tp-5">
        <div className="flex flex-wrap items-center gap-tp-4">
          <span className="text-sm text-text-secondary">关注品类</span>
          {categories.length === 0 ? (
            <span className="rounded-md border border-dashed border-border px-tp-3 py-tp-1 text-sm text-text-tertiary">
              {authed ? '当前凭据暂未开通任何品类' : '就绪后自动同步关注品类'}
            </span>
          ) : (
            categories.map((c) => {
              const on = cats.includes(c.key);
              return (
                <button
                  key={c.key}
                  onClick={() =>
                    setCats((prev) =>
                      on ? prev.filter((k) => k !== c.key) : [...prev, c.key],
                    )
                  }
                  className={cn(
                    'rounded-md border px-tp-3 py-tp-1 text-sm transition',
                    on
                      ? 'border-primary-600 bg-primary-600 text-white'
                      : 'border-border text-text-secondary hover:border-border-strong',
                  )}
                >
                  {c.label}
                </button>
              );
            })
          )}
          <div className="flex-1" />
          <label className="flex items-center gap-tp-2 text-sm text-text-secondary">
            自动提交
            <Switch checked={config?.auto_submit ?? false} onCheckedChange={setAuto} />
          </label>
        </div>
      </Card>

      {/* 关注规则(移入监控台) */}
      <RulesSection />

      {/* 手动提交 */}
      <Card className="flex flex-col gap-tp-3 p-tp-5">
        <h2 className="text-base font-medium text-text-primary">手动提交</h2>
        <div className="grid grid-cols-[1fr_1fr_auto] gap-tp-3">
          <Input
            className="selectable"
            placeholder="inspectSkuId"
            value={mInspect}
            onChange={(e) => setMInspect(e.target.value)}
          />
          <Input
            className="selectable"
            placeholder="youpinSkuId"
            value={mYoupin}
            onChange={(e) => setMYoupin(e.target.value)}
          />
          <Button variant="primary" disabled={!authed} onClick={manualSubmit}>
            提交
          </Button>
        </div>
      </Card>

      {/* 运行日志 */}
      <Card className="flex flex-col gap-tp-2 p-tp-4">
        <div className="flex items-center justify-between">
          <h2 className="text-base font-medium text-text-primary">运行日志</h2>
          <Button variant="ghost" size="sm" onClick={clearLogs}>
            清空
          </Button>
        </div>
        <div
          ref={logRef}
          className="mono h-[240px] overflow-y-auto rounded-md bg-bg-subtle p-tp-3 text-xs leading-relaxed selectable"
        >
          {logs.length === 0 ? (
            <span className="text-text-tertiary">暂无日志</span>
          ) : (
            logs.map((l, i) => (
              <div
                key={i}
                className={cn(
                  l.kind === 'hit' && 'text-status-done-fg',
                  l.kind === 'err' && 'text-danger-500',
                  l.kind === 'info' && 'text-text-secondary',
                )}
              >
                <span className="mr-tp-2 text-text-tertiary">
                  {new Date(l.ts).toLocaleTimeString('zh-CN', { hour12: false })}
                </span>
                {l.msg}
              </div>
            ))
          )}
        </div>
      </Card>
    </div>
  );
}

/**
 * 关注规则:TanStack Table 只读表格 + modal 增/改。
 * 规则真源仍在客户端(本地落库 + 实时镜像到服务端驱动命中筛选)。
 */
function RulesSection() {
  const rules = useStore((s) => s.rules);
  const setRules = useStore((s) => s.setRules);

  const [modalOpen, setModalOpen] = useState(false);
  const [modalEditing, setModalEditing] = useState<Rule | undefined>(undefined);

  async function save(next: Rule[]) {
    setRules(next);
    try {
      await invoke('save_rules', { rules: next });
    } catch (e) {
      notify(`保存规则失败: ${String(e)}`, 'err');
    }
  }

  const reenable = async (id: string) => {
    await invoke('reenable_rule', { ruleId: id });
    setRules(await invoke<Rule[]>('get_rules'));
  };

  return (
    <Card className="flex flex-col gap-tp-4 p-tp-5">
      <div className="flex items-center justify-between">
        <h2 className="text-base font-medium text-text-primary">关注规则</h2>
        <Button
          variant="primary"
          size="sm"
          onClick={() => {
            setModalEditing(undefined);
            setModalOpen(true);
          }}
        >
          + 添加规则
        </Button>
      </div>

      {rules.length === 0 ? (
        <Empty description="暂无规则,点右上角添加" />
      ) : (
        <RulesTable
          rules={rules}
          onEdit={(r) => {
            setModalEditing(r);
            setModalOpen(true);
          }}
          onDelete={(r) => save(rules.filter((x) => x.id !== r.id))}
          onToggle={(r) =>
            save(rules.map((x) => (x.id === r.id ? { ...x, enabled: !x.enabled } : x)))
          }
          onReenable={(r) => reenable(r.id)}
        />
      )}

      <RuleFormModal
        open={modalOpen}
        rules={rules}
        editing={modalEditing}
        onClose={() => setModalOpen(false)}
        onSubmit={(rule) => {
          const next = modalEditing
            ? rules.map((x) => (x.id === rule.id ? rule : x))
            : [...rules, rule];
          save(next);
          setModalOpen(false);
        }}
      />
    </Card>
  );
}

/**
 * 监控存活指示器:让用户一眼确认「它还活着、在跑」,而不是死在那。三态——
 *   启动中(未收到首个心跳)→ 灰点 + 「正在启动…」
 *   运行中(心跳新鲜)       → 呼吸绿点 + 「运行中 · 上次活动 N 秒前」
 *   停滞(超时未收到心跳)   → 黄点 + 「连接不稳 · 正在重连…」
 * 心跳是服务端每扫完一圈下发的空脉冲,不含任何扫描细节(轮次/条数/品类一律不暴露)。
 */
function LiveStatus({ lastBeat }: { lastBeat: number }) {
  if (lastBeat === 0) {
    return (
      <Live dotClass="bg-text-tertiary" textClass="text-text-tertiary" label="正在启动…" />
    );
  }
  const ago = Date.now() - lastBeat;
  if (ago > HEARTBEAT_STALE_MS) {
    return (
      <Live
        dotClass="bg-status-pending-fg animate-pulse"
        textClass="text-status-pending-fg"
        label="连接不稳 · 正在重连…"
      />
    );
  }
  return (
    <Live
      dotClass="bg-status-done-fg animate-pulse"
      textClass="text-text-secondary"
      label={`运行中 · 上次活动 ${fmtAgo(ago)}`}
    />
  );
}

function Live({
  dotClass,
  textClass,
  label,
}: {
  dotClass: string;
  textClass: string;
  label: string;
}) {
  return (
    <div className="flex items-center gap-tp-2 rounded-full border border-border bg-bg-subtle px-tp-3 py-tp-1">
      <span className={cn('h-2 w-2 rounded-full', dotClass)} />
      <span className={cn('text-xs', textClass)}>{label}</span>
    </div>
  );
}

/** 毫秒 → 友好「N 秒/分钟前」。 */
function fmtAgo(ms: number): string {
  const s = Math.floor(ms / 1000);
  if (s <= 1) return '刚刚';
  if (s < 60) return `${s} 秒前`;
  const m = Math.floor(s / 60);
  return `${m} 分钟前`;
}

function Metric({ label, ms }: { label: string; ms: number }) {
  const color = ms < 0 ? 'text-danger-500' : ms < 200 ? 'text-status-done-fg' : ms < 500 ? 'text-status-pending-fg' : 'text-danger-500';
  return (
    <div className="text-right">
      <div className="text-[10px] uppercase tracking-wide text-text-tertiary">{label}</div>
      <div className={cn('mono text-sm', color)}>{ms < 0 ? '—' : `${ms}ms`}</div>
    </div>
  );
}
