# Tinc Connect（TincLink Windows 客户端）

Vue 3 + Tauri 2 + Rust 本地服务。客户端已完成登录、严格配置包校验、RSA 密钥生成/复用、公钥上传、Tinc 生命周期管理、Windows TAP 地址与 `/32` 路由管理，以及基于 ICMP 的真实连通状态判定。

当前仓库正在固化 `v1.0-competition-2026-rc1` 候选版本。已有开发机实机隧道连通记录，但干净 Windows 首装、卸载、重启和重装的 24 项验收尚未完成，因此不得将当前安装包标记为最终发行版。Linux 仅有代码与测试基础，Android 未实现。

继续开发先读 [项目交接](docs/HANDOVER.md)，其中记录了实际验证结果、未完成事项和接手步骤。

## 目录导航

```text
apps/desktop/        桌面应用
  src/               Vue 页面、样式和前端调用
  src-tauri/         Tauri 原生入口、应用配置和图标
apps/agent/          后台守护程序、IPC 监听和系统服务入口
crates/              共享核心逻辑、IPC 协议和平台能力
docs/                开发说明、接口契约、架构和安全边界
scripts/
  dev/               本地开发环境与初始化
  test/              本地端到端验证
  build/             打包工具准备
packaging/           Windows / Linux 服务安装与卸载
.github/workflows/   自动构建与检查
```

根目录的 `package.json` 和 `Cargo.toml` 分别是前端与 Rust 工作区入口；对应 lock 文件用于固定依赖。
`node_modules/`、`target/`、`.tools/`、`.runtime/` 是依赖、构建或本地产物，VS Code 默认隐藏。完整目录职责和操作说明见 [开发指南](docs/README.md)。

## Windows 本地启动

以下命令都在项目根目录执行。需要 Node.js 22、WebView2 Runtime，以及 Rust 工具链；本项目已有 `.tools` 工具链时，环境脚本会自动使用它。

### 1. 首次准备

依赖只需在首次运行或 lock 文件变化后安装：

```powershell
. .\scripts\dev\env.ps1
npm.cmd ci
cargo build --locked -p vpn-agent -p tinc-desktop
```

然后创建开发配置。`TincBinary` 必须是实际存在的 `tincd.exe`，`Interface` 必须是准备给 Tinc 使用的真实网卡名：

```powershell
powershell -NoProfile -File .\scripts\dev\init.ps1 `
  -TincBinary 'D:\Codes\Java\KenDeJi_RuoYi\tinc_cli_gui\windows\Tinc\tincd.exe' `
  -Interface 'tinc'
```

上面的 Tinc 路径只是示例，运行前必须替换。初始化会创建 `.runtime/settings.json`，已有该文件时不要重复执行；脚本会拒绝覆盖。

### 2. 启动后台 Agent

打开第一个 PowerShell：

```powershell
. .\scripts\dev\env.ps1
if (-not (Test-Path .runtime/settings.json)) {
  throw '缺少 .runtime/settings.json，请先完成上面的首次初始化'
}
cargo run --locked -p vpn-agent -- --dev .runtime/settings.json
```

保持这个终端运行。看到错误时先检查 `.runtime/settings.json` 中的 Tinc 路径、网卡名和数据目录。

### 3. 启动桌面界面

打开第二个 PowerShell：

```powershell
. .\scripts\dev\env.ps1
npm.cmd run desktop:dev
```

该命令会同时启动 Vite 和 Tauri 窗口，不需要另外运行 `npm.cmd run dev`。两个终端都要加载环境脚本，桌面端才能连接开发模式的 Named Pipe。

只想在浏览器查看静态界面时，可以单独运行 `npm.cmd run dev`；浏览器预览不能调用本地 Agent，也不会模拟 VPN 数据。

### 4. 验证代码

```powershell
. .\scripts\dev\env.ps1
npm.cmd run build
npm.cmd run test:ui
npm.cmd run test:packaging
cargo test --locked --workspace --exclude tinc-desktop
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo build --locked -p vpn-agent --example process_fixture
npm.cmd run test:e2e
```

服务安装、打包和测试说明见 [开发指南](docs/README.md)。普通用户 UI 不自动提权；高权限服务由管理员单独安装。


<!-- • 明日接手提示词已生成并保存：

  docs/NEXT_SESSION_PROMPT.md

  明天新开会话时，直接发送：

  继续开发 Tinc 新客户端。请先完整阅读并执行：

  D:\Codes\Java\KenDeJi_RuoYi\tinc_client\docs\NEXT_SESSION_PROMPT.md

  项目根目录：

  D:\Codes\Java\KenDeJi_RuoYi\tinc_client

  根据文档记录的当前进度继续，不要重新开始，不要修改旧客户端目录，不要猜测后端协议或网络前缀。

  提示词已包含：

  - 今天完成的配置包联调和哈希。
  - Windows/Linux 构建与测试结果。
  - Tinc 实际版本 1.0.36、路径和哈希。
  - tinc0、TAP 驱动及旧版 /24 行为。
  - 后端公钥上传、Token 鉴权和网关迁移待办。
  - 自动安装 Tinc/TAP 的后续方向。
  - Windows/Linux 网卡启动与停止策略。
  - 明日明确执行顺序和仍需实机验证的事项。

  docs/HANDOVER.md:3 也已经加入该提示词入口。  -->
