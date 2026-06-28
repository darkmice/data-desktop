import { useState } from 'react';
import { Button } from '@talon-ui/react';
import logoUrl from '../assets/app-icon.svg';

// 每次启动的免责声明。用受控覆盖层(非 Radix Trigger),启动即显示,同意后关闭。
export function Disclaimer() {
  const [open, setOpen] = useState(true);
  if (!open) return null;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-bg-overlay p-6">
      <div className="w-full max-w-xl rounded-xl border border-border bg-bg-surface p-6 shadow-2xl">
        <div className="mb-tp-1 flex items-center gap-tp-3">
          <img src={logoUrl} alt="logo" className="h-8 w-8 shrink-0 rounded-md" />
          <h3 className="text-lg font-semibold text-text-primary">使用前声明</h3>
        </div>
        <div className="mb-tp-5 text-xs uppercase tracking-widest text-text-tertiary">
          仅供学习交流使用
        </div>
        <p className="leading-relaxed text-text-secondary">
          本工具仅供个人学习、技术研究与数据观察,不提供任何自动交易服务。使用者须遵守目标平台规则及相关法律法规,因使用产生的一切后果由使用者自行承担。
        </p>
        <div className="mt-tp-6 flex justify-end">
          <Button variant="primary" onClick={() => setOpen(false)}>
            同意并继续
          </Button>
        </div>
      </div>
    </div>
  );
}
