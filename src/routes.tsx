import { createHashRouter, Navigate } from 'react-router-dom';
import { Gauge, KeyRound, ListChecks, Settings as SettingsIcon } from 'lucide-react';
import { App } from './App';
import { Dashboard } from './views/Dashboard';
import { Credentials } from './views/Credentials';
import { Orders } from './views/Orders';
import { Settings } from './views/Settings';

// Hash router:Tauri 用 file:// 协议加载,history 路由会 404,hash 路由最稳。
// 注:关注规则已并入「监控台」,不再有独立路由。
export const router = createHashRouter([
  {
    path: '/',
    element: <App />,
    children: [
      { index: true, element: <Navigate to="/dashboard" replace /> },
      { path: 'dashboard', element: <Dashboard /> },
      { path: 'credentials', element: <Credentials /> },
      { path: 'orders', element: <Orders /> },
      { path: 'settings', element: <Settings /> },
    ],
  },
]);

export const NAV_ITEMS = [
  { to: '/dashboard', label: '监控台', Icon: Gauge },
  { to: '/credentials', label: '凭证', Icon: KeyRound },
  { to: '/orders', label: '下单记录', Icon: ListChecks },
  { to: '/settings', label: '设置', Icon: SettingsIcon },
] as const;
