import { useEffect } from 'react';
import { useToast } from '@talon-ui/react';
import { registerToast } from '../store/useStore';

// 把 talon 的 toast() 注册到 store,使非组件代码(events 订阅、helper)也能弹吐司。
// 不渲染任何内容。
export function ToastBridge() {
  const { toast, dismiss } = useToast();
  useEffect(() => {
    registerToast({ toast, dismiss });
    return () => registerToast(null);
  }, [toast, dismiss]);
  return null;
}
