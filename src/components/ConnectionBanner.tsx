import { useNavigate } from 'react-router-dom';
import { Banner, Button } from '@talon-ui/react';
import { useStore } from '../store/useStore';

// 已填、研究功能是否就绪」。就绪(authed)时不显示。
export function ConnectionBanner() {
  const conn = useStore((s) => s.conn);
  const connReason = useStore((s) => s.connReason);
  const config = useStore((s) => s.config);
  const navigate = useNavigate();

  if (conn === 'authed') return null;

  const hasToken = !!config?.token && config.token.trim() !== '';
  const preparing = conn === 'connected';
  // 有明确原因(Token 无效/受限断开)时优先展示。
  const reason = !!connReason;

  const title = reason
    ? '研究功能暂不可用'
    : preparing
      ? '正在准备研究功能…'
      : hasToken
        ? '研究功能准备中'
        : '请先填写研究测试 Token';

  const tone = preparing ? 'info' : 'warning';

  return (
    <div className="mb-tp-4">
      <Banner
        tone={tone}
        title={title}
        action={
          !preparing && (
            <Button variant="secondary" size="sm" onClick={() => navigate('/settings')}>
              去设置
            </Button>
          )
        }
      >
        {reason
          ? `${connReason}`
          : preparing
            ? '稍候,正在准备研究功能。'
            : hasToken
              ? '研究功能正在准备,稍候即可使用。'
              : '请前往设置填写研究测试 Token,即可开始数据研究。'}
      </Banner>
    </div>
  );
}
