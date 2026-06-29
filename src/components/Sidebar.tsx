import { NavLink } from 'react-router-dom';
import { cn } from '@talon-ui/react';
import { Moon, Sun } from 'lucide-react';
import { NAV_ITEMS } from '../routes';
import { invoke } from '../lib/tauri';
import { applyTheme, useStore } from '../store/useStore';
import type { AppConfig } from '../lib/types';
import logoUrl from '../assets/app-icon.svg';

// 面向用户用「就绪」语义,不暴露「连接服务器」概念。
const CONN_META: Record<string, { dot: string; text: string }> = {
  authed: { dot: 'bg-status-done-fg', text: '就绪' },
  connected: { dot: 'bg-status-pending-fg', text: '准备中…' },
  disconnected: { dot: 'bg-border-border-strong', text: '未就绪' },
};

export function Sidebar() {
  const conn = useStore((s) => s.conn);
  const config = useStore((s) => s.config);
  const setConfig = useStore((s) => s.setConfig);
  const meta = CONN_META[conn] ?? CONN_META.disconnected;
  const theme = config?.theme === 'light' ? 'light' : 'dark';

  async function toggleTheme() {
    if (!config) return;
    const next: AppConfig = { ...config, theme: theme === 'dark' ? 'light' : 'dark' };
    applyTheme(next.theme); // 立即生效,避免等待持久化
    setConfig(next);
    try {
      await invoke('save_config', { config: next });
    } catch {
      /* 持久化失败不影响当前会话主题 */
    }
  }

  return (
    <aside className="flex w-[200px] shrink-0 flex-col border-r border-border bg-bg-surface-2">
      {/* 品牌 */}
      <div className="flex items-center gap-tp-3 border-b border-border px-tp-4 py-tp-4">
        <img src={logoUrl} alt="logo" className="h-8 w-8 shrink-0 rounded-md" />
        <div className="leading-tight">
          <div className="text-sm font-medium text-text-primary">商品数据研究助手</div>
          <div className="text-[10px] text-text-tertiary">仅供学习交流使用</div>
        </div>
      </div>

      {/* 导航 */}
      <nav className="flex flex-1 flex-col gap-px p-tp-2">
        {NAV_ITEMS.map((item) => (
          <NavLink
            key={item.to}
            to={item.to}
            className={({ isActive }) =>
              cn(
                'relative flex items-center gap-tp-3 rounded-md px-tp-3 py-tp-2 text-sm transition',
                isActive
                  ? 'bg-primary-600 font-medium text-white shadow-sm'
                  : 'text-text-secondary hover:bg-bg-subtle hover:text-text-primary',
              )
            }
          >
            {({ isActive }) => (
              <>
                {isActive && (
                  <span className="absolute left-0 top-1/2 h-4 w-[3px] -translate-y-1/2 rounded-r-full bg-primary-300" />
                )}
                <item.Icon
                  size={16}
                  strokeWidth={isActive ? 2.4 : 2}
                  className={isActive ? 'opacity-100' : 'opacity-70'}
                />
                {item.label}
              </>
            )}
          </NavLink>
        ))}
      </nav>

      {/* 主题切换 */}
      <button
        onClick={toggleTheme}
        className="flex items-center justify-between border-t border-border px-tp-4 py-tp-2 text-xs text-text-secondary transition hover:bg-bg-subtle hover:text-text-primary"
      >
        <span className="flex items-center gap-tp-2">
          {theme === 'dark' ? <Moon size={15} /> : <Sun size={15} />}
          {theme === 'dark' ? '深色' : '浅色'}主题
        </span>
        <span className="text-text-tertiary">切换</span>
      </button>

      {/* 连接状态灯 */}
      <div className="flex items-center gap-tp-2 border-t border-border px-tp-4 py-tp-3 text-xs text-text-secondary">
        <span className={cn('h-2 w-2 rounded-full', meta.dot)} />
        {meta.text}
      </div>
    </aside>
  );
}
