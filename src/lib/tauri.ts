// Tauri 桥接。__TAURI__ 的注入与脚本执行有时序竞争,且 core/event 是分开注入
// 的;因此 invoke/listen 都在“调用时”实时从 window.__TAURI__ 取,绝不在模块
// 顶层固化(否则可能固化成空操作 → invoke 能用但事件全收不到)。
// capabilities/default.json 已授权 core:event:listen。

type TauriGlobal = {
  core?: { invoke: (cmd: string, args?: Record<string, unknown>) => Promise<unknown> };
  event?: {
    listen: (
      event: string,
      handler: (e: { payload: unknown }) => void,
    ) => Promise<() => void>;
  };
};

function tauri(): TauriGlobal {
  return (window as unknown as { __TAURI__?: TauriGlobal }).__TAURI__ ?? {};
}

export async function invoke<T = unknown>(
  cmd: string,
  args?: Record<string, unknown>,
): Promise<T> {
  const core = tauri().core;
  if (!core) throw new Error('Tauri 运行环境不可用');
  return core.invoke(cmd, args) as Promise<T>;
}

export async function listen<T = unknown>(
  event: string,
  handler: (payload: T) => void,
): Promise<() => void> {
  const ev = tauri().event;
  if (!ev) {
    console.error('Tauri event API 不可用,无法监听', event);
    return () => {};
  }
  return ev.listen(event, (e) => handler(e.payload as T));
}

/** 在系统默认浏览器打开链接(商品链接“用户自己去看”)。 */
export function openExternal(url: string): void {
  // Tauri 的 opener 插件未启用时退化到 window.open(webview 会拦,但聊胜于无)。
  const opener = (
    window as unknown as { __TAURI__?: { opener?: { openUrl: (u: string) => void } } }
  ).__TAURI__?.opener;
  if (opener?.openUrl) opener.openUrl(url);
  else window.open(url, '_blank');
}
