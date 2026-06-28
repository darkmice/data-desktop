import { useEffect } from 'react';
import { Outlet } from 'react-router-dom';
import { Sidebar } from './components/Sidebar';
import { Disclaimer } from './components/Disclaimer';
import { ConnectionBanner } from './components/ConnectionBanner';
import { setupEvents } from './lib/events';
import { invoke } from './lib/tauri';
import { applyTheme, useStore } from './store/useStore';
import type { AppConfig, Credential, OrderStats, Rule } from './lib/types';

export function App() {
  const s = useStore();

  // 启动初始化:订阅事件 + 拉取持久化数据(config/creds/rules 已由后端 load_persisted
  // 读回内存,这里只是同步到前端 store)。
  useEffect(() => {
    void setupEvents();
    void (async () => {
      let hasToken = false;
      try {
        const cfg = await invoke<AppConfig>('get_config');
        s.setConfig(cfg);
        applyTheme(cfg.theme);
        hasToken = !!cfg.token && cfg.token.trim() !== '';
      } catch {
        /* ignore */
      }
      try {
        s.setCreds(await invoke<Credential[]>('get_credentials'));
      } catch {
        /* ignore */
      }
      try {
        s.setRules(await invoke<Rule[]>('get_rules'));
      } catch {
        /* ignore */
      }
      try {
        s.setStats(await invoke<OrderStats>('get_order_stats'));
      } catch {
        /* ignore */
      }
      // 启动自动连接:配置里已有 token 就直接连,不用每次手点。失败由事件泵
      // 翻译成连接态/原因展示(和手动连接同一套反馈),这里静默即可。
      if (hasToken) {
        try {
          await invoke('connect');
        } catch {
          /* 连接失败会经 conn 事件反馈,无需在此弹错 */
        }
      }
    })();
    // 只跑一次
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <div className="flex h-screen overflow-hidden bg-bg-surface text-text-primary">
      <Sidebar />
      <main className="flex-1 overflow-y-auto p-tp-6">
        <ConnectionBanner />
        <Outlet />
      </main>
      <Disclaimer />
    </div>
  );
}
