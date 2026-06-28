# 数据研究助手 — 桌面客户端

基于 [Tauri 2](https://tauri.app/) + React 18 + TypeScript 的跨平台桌面客户端。

## 访问 Token(推荐)

客户端只需填**一个访问 Token**(`ak-…`),它由管理员在后台生成,内部已加密打包了
服务地址与连接凭证 —— 用户无需(也无法)单独看到真实地址。

使用流程:

1. **管理员**:在服务端 admin 后台「生成访问 Token」填入服务地址(`wss://…/ws`)、
   用户标识、过期时间 → 得到一串 `ak-…`,发给用户。
2. **用户**:打开客户端 → 设置 → 把 `ak-…` 粘进「访问 Token」→ 保存并启用。

换线上地址时,管理员只需重新生成并下发新的 `ak-…`,客户端无需重装。

> 连接错误提示分级:`ak-` 无效 → 「访问 Token 无效」;已过期 → 「访问 Token 已过期」;
> 地址连不通 → 「无法连接服务,请联系管理员更新 Token」。

### master key

访问 Token 用 AES-256-GCM 加密,master key 两端必须同值:

- 服务端:环境变量 `H5ST_ACCESS_KEY`(64 hex)。
- 客户端:打包时由 GitHub Secrets 的 `DATA_ACCESS_KEY`(64 hex)编译期注入。

生成一把:`openssl rand -hex 32`。

## 旧版:配置服务地址(config.json,兼容保留)

若仍使用旧版裸 token(`sk-h5st-…`),可在**可执行文件同目录**放 `config.json` 指定地址:

```json
{
  "server_url": "ws://你的服务器IP:8443/ws"
}
```

- 文件不存在时,使用内置默认地址(`ws://127.0.0.1:8443/ws`)。
- 该地址作为**默认值**:用户若在应用界面里改过地址,以用户的为准(文件不覆盖)。
- 明文用 `ws://`(对应服务端 `H5ST_PLAINTEXT=true`),加密用 `wss://`。
- 填入 `ak-` 访问 Token 时,此文件被忽略(地址从 Token 解出)。

参见 [config.example.json](config.example.json)。

## 跨平台构建产物

GitHub Actions 会在打 `v*` 标签时自动编译并发布到 Release:

| 平台 | 安装包 |
|---|---|
| Windows | `.msi` / `.exe` |
| macOS (Apple Silicon) | `.dmg` / `.app` |
| macOS (Intel) | `.dmg` / `.app` |
| Linux | `.deb` / `.AppImage` |

## 发布一个版本

```bash
git tag v0.1.0
git push origin v0.1.0
```

推送标签后,[Actions](../../actions) 会并行编译四个目标平台,完成后在 [Releases](../../releases) 生成对应版本并附上全部安装包。

也可在 Actions 页手动触发(workflow_dispatch)做一次性验证构建,产物以 artifact 形式提供(不创建 Release)。

## 本地开发

需要 [Node 20+](https://nodejs.org/)、[pnpm 10+](https://pnpm.io/)、[Rust stable](https://rustup.rs/),以及各平台的 [Tauri 前置依赖](https://tauri.app/start/prerequisites/)。

```bash
pnpm install          # 安装前端依赖
pnpm tauri dev        # 开发模式(热重载)
pnpm tauri build      # 本地打包当前平台
```

> 仅前端构建:`pnpm build`(产物在 `dist/`)。

## 技术栈

- **前端**:React 18 / Vite 5 / TypeScript / Tailwind CSS 3 / [@talon-ui/react](https://www.npmjs.com/package/@talon-ui/react) / zustand
- **壳层**:Tauri 2(Rust)

## 目录结构

```
src/              前端源码(React)
src-tauri/        Tauri 壳 + Rust 业务逻辑
  src/            Rust 源码(WS 客户端、业务流程、凭证、通知等)
  tauri.conf.json Tauri 配置
  icons/          应用图标
.github/workflows/release.yml  跨平台构建 + 发布
```
