import { Component, type ErrorInfo, type ReactNode } from 'react';
import { Wrench } from 'lucide-react';

// 兜底错误边界:任何渲染异常都显示友好页面,绝不白屏。
interface State {
  error: Error | null;
}

export class ErrorBoundary extends Component<{ children: ReactNode }, State> {
  state: State = { error: null };

  static getDerivedStateFromError(error: Error): State {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error('UI 渲染异常:', error, info);
  }

  render() {
    if (this.state.error) {
      return (
        <div className="flex h-screen flex-col items-center justify-center gap-4 bg-bg-app p-8 text-center text-text-primary">
          <Wrench size={36} className="text-text-tertiary" />
          <h1 className="text-xl font-semibold">界面遇到了一点小问题</h1>
          <p className="max-w-md text-sm text-text-tertiary">
            程序仍在运行,可尝试重新打开。如反复出现,请重启应用。
          </p>
          <button
            onClick={() => this.setState({ error: null })}
            className="rounded-md bg-primary-600 px-4 py-2 text-sm font-medium text-white"
          >
            重试
          </button>
        </div>
      );
    }
    return this.props.children;
  }
}
