/**
 * Talon Pilot 设计系统的 Tailwind v3 配置:用 talon 的 preset 拿到全部
 * token(颜色/间距/字号/控件高度/动效/focus-ring 等),content 必须包含
 * @talon-ui/react 的 dist,否则它内部用到的 class 会被 purge 掉。
 */
const talonPreset = require('@talon-ui/tokens/preset');

/** @type {import('tailwindcss').Config} */
module.exports = {
  presets: [talonPreset],
  darkMode: ['selector', '[data-theme="dark"]'],
  content: [
    './index.html',
    './src/**/*.{ts,tsx}',
    './node_modules/@talon-ui/react/dist/**/*.{js,cjs}',
  ],
  theme: {
    extend: {
      // talon 三档边框里 preset 只暴露了 default / strong,补上最淡的 subtle
      // (深色 #1f2a40,更贴近背景),用于表格行分隔等需要弱化的细线。
      borderColor: {
        subtle: 'var(--tp-border-subtle)',
      },
    },
  },
};
