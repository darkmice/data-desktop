import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { RouterProvider } from 'react-router-dom';
import { ToastProvider } from '@talon-ui/react';
import { router } from './routes';
import { ErrorBoundary } from './components/ErrorBoundary';
import { ToastBridge } from './components/ToastBridge';
import './index.css';

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <ErrorBoundary>
      <ToastProvider>
        {/* 把 toast() 注册到 store,使非组件代码(事件订阅等)也能弹吐司 */}
        <ToastBridge />
        <RouterProvider router={router} />
      </ToastProvider>
    </ErrorBoundary>
  </StrictMode>,
);
