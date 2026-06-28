import { create } from 'zustand';
import type {
  AppConfig,
  Category,
  ConnStatus,
  Credential,
  LogLine,
  OrderStats,
  Rule,
  WatchParams,
} from '../lib/types';

/** 监控参数缺省值(服务端下发前的占位)。 */
const DEFAULT_PARAMS: WatchParams = { page_from: 1, page_to: 5, interval: 3, max_threads: 5 };

const MAX_LOGS = 500;

interface AppStore {
  // 连接 / 监控
  conn: ConnStatus;
  /** 连接失败/被拒的友好原因(如凭据无效);连接成功时清空。 */
  connReason: string;
  watching: boolean;
  /** 最近一次监控心跳的本地接收时间(ms epoch);0 = 尚未收到。监控存活指示器据此
   *  显示「运行中 · 上次活动 N 秒前」并在超时后判定停滞。停止/断开时归零。 */
  lastBeat: number;
  // 数据
  config: AppConfig | null;
  creds: Credential[];
  activeIdx: number;
  rules: Rule[];
  /** 服务端下发的本 token 启用品类(只读消费;监控台据此勾选)。 */
  serverCategories: Category[];
  /** 服务端下发的本 token 监控扫描参数(只读展示)。 */
  serverParams: WatchParams;
  logs: LogLine[];
  stats: OrderStats;

  // setters
  setConn: (c: ConnStatus, reason?: string) => void;
  setWatching: (w: boolean) => void;
  /** 记录一次心跳(写入当前时间)。监控运行中由心跳事件调用。 */
  beat: () => void;
  setConfig: (c: AppConfig) => void;
  setServerCategories: (c: Category[]) => void;
  setServerParams: (p: WatchParams) => void;
  setCreds: (c: Credential[]) => void;
  setActiveIdx: (i: number) => void;
  setRules: (r: Rule[]) => void;
  setStats: (s: OrderStats) => void;
  pushLog: (msg: string, kind?: LogLine['kind']) => void;
  clearLogs: () => void;
}

export const useStore = create<AppStore>((set) => ({
  conn: 'disconnected',
  connReason: '',
  watching: false,
  lastBeat: 0,
  config: null,
  creds: [],
  activeIdx: 0,
  rules: [],
  serverCategories: [],
  serverParams: DEFAULT_PARAMS,
  logs: [],
  stats: { total: 0, success: 0, failed: 0 },

  setConn: (conn, reason = '') =>
    set((s) => ({
      conn,
      // authed(成功)与 connected(刚发起连接的过程态)都清空原因——开启新一轮
      // 连接时不该挂着上次的红字。其余(disconnected)保留有意义的原因:被踢/封/
      // Token 失效后,reader 会紧跟一个「无 reason」的 disconnected 帧;若用它覆盖,
      // 会瞬间清掉前一帧设置的「IP 被风控」等文案。故新 reason 为空时保留旧值。
      connReason:
        conn === 'authed' || conn === 'connected' ? '' : reason || s.connReason,
      watching: conn === 'disconnected' ? false : s.watching,
      // 断开即清空心跳时间戳,避免显示陈旧的「上次活动」。
      lastBeat: conn === 'disconnected' ? 0 : s.lastBeat,
    })),
  // 停监控时归零心跳;启监控时也归零(等首个心跳到来再点亮「运行中」)。
  setWatching: (watching) => set({ watching, lastBeat: 0 }),
  beat: () => set({ lastBeat: Date.now() }),
  setConfig: (config) => set({ config }),
  setServerCategories: (serverCategories) => set({ serverCategories }),
  setServerParams: (serverParams) => set({ serverParams }),
  setCreds: (creds) => set({ creds }),
  setActiveIdx: (activeIdx) => set({ activeIdx }),
  setRules: (rules) => set({ rules }),
  setStats: (stats) => set({ stats }),
  pushLog: (msg, kind = 'info') =>
    set((s) => {
      const line: LogLine = { ts: Date.now(), msg, kind };
      const logs = [...s.logs, line];
      if (logs.length > MAX_LOGS) logs.splice(0, logs.length - MAX_LOGS);
      return { logs };
    }),
  clearLogs: () => set({ logs: [] }),
}));

/** 应用主题到 <html data-theme>。dark 命中 dark 选择器,其余走默认(light)。 */
export function applyTheme(theme: string): void {
  document.documentElement.setAttribute('data-theme', theme === 'light' ? 'light' : 'dark');
}

// ---- Toast 桥接 ----
// talon 的 toast() 是 hook 产物(需组件上下文),而事件订阅等非组件代码也要弹吐司,
// 故由 <ToastBridge> 在挂载时把 toast() 注册到这里,供 notify() 调用。
type ToastTone = 'info' | 'success' | 'warning' | 'error';
interface ToastItem {
  id?: string;
  title?: string;
  description?: string;
  tone?: ToastTone;
  duration?: number;
}
interface ToastApi {
  toast: (item: ToastItem) => void;
  dismiss: (id?: string) => void;
}

let toastApi: ToastApi | null = null;
export function registerToast(api: ToastApi | null): void {
  toastApi = api;
}

const KIND_TONE: Record<LogLine['kind'], ToastTone> = {
  info: 'info',
  hit: 'success',
  err: 'error',
};

/** 预置事务 id:同一事务的过程态/结果态复用,后者覆盖前者(只留一条)。 */
export const TX = {
  /** 连接流程:正在连接 → 成功/失败 共用一条。 */
  CONN: 'tx-conn',
} as const;

/**
 * 重要提示统一入口:**同时**弹吐司 + 写运行日志。
 * 传 `txId` 时,同一事务的吐司会覆盖显示(先 dismiss 旧的再弹新的),
 * 避免"正在连接…"和"连接结果"叠成两条。普通流水仍用 pushLog(仅日志)。
 */
export function notify(msg: string, kind: LogLine['kind'] = 'info', txId?: string): void {
  useStore.getState().pushLog(msg, kind);
  if (!toastApi) return;
  // 同事务:先移除上一条同 id 吐司,再以同 id 弹新的 → 视觉上是"覆盖"。
  if (txId) toastApi.dismiss(txId);
  toastApi.toast({
    id: txId,
    description: msg,
    tone: KIND_TONE[kind],
    duration: kind === 'err' ? 6000 : 3500,
  });
}

/** 从日志文本推断级别(沿用旧前端的中文关键词规则)。 */
export function logKind(msg: string): LogLine['kind'] {
  if (msg.includes('成功') || msg.includes('命中')) return 'hit';
  if (msg.includes('失败') || msg.includes('错误')) return 'err';
  return 'info';
}
