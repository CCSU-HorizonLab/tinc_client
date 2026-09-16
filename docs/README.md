# 开发指南

进度、验证结果和下一步见 [项目交接](HANDOVER.md)。

## 从哪里开始

| 要做的事 | 位置 |
| --- | --- |
| 修改页面与交互 | `apps/desktop/src/App.vue` |
| 修改界面样式 | `apps/desktop/src/styles/` |
| 修改前端对 Tauri 的调用 | `apps/desktop/src/services/` |
| 修改桌面窗口、原生命令与应用打包配置 | `apps/desktop/src-tauri/` |
| 修改登录、配置校验、密钥与进程状态 | `crates/vpn-core/` |
| 修改后台程序启动入口 | `apps/agent/src/main.rs` |
| 修改 IPC 监听与请求分发 | `apps/agent/src/server.rs` |
| 修改 Windows Service 生命周期 | `apps/agent/src/service/windows.rs` |
| 修改 Linux/systemd 进程信号处理 | `apps/agent/src/service/linux.rs` |
| 修改本地通信 | `crates/vpn-ipc/` |
| 修改系统权限与进程适配 | `crates/platform-windows/`、`crates/platform-linux/` |
| 运行开发、测试或打包辅助脚本 | `scripts/dev/`、`scripts/test/`、`scripts/build/` |
| 修改系统服务安装与卸载 | `packaging/windows/`、`packaging/linux/` |

可独立运行的桌面端和后台 Agent 放在 `apps/`；供它们复用的 Rust 库放在 `crates/`。Rust 测试保留在各模块的 `tests/`，前端单元测试放在对应服务代码旁；跨应用的端到端脚本位于 `scripts/test/`。

进一步阅读：[接口契约与后端阻塞](API_CONTRACT.md)、[客户端与后端配置包联调单](BACKEND_HANDOFF.md)、[架构与状态](ARCHITECTURE.md)、[安全边界](SECURITY.md)。

## 本地生成目录

| 目录 | 内容 |
| --- | --- |
| `node_modules/` | npm 安装的依赖 |
| `target/` | Rust 编译缓存、Debug/Release 程序与安装包产物 |
| `apps/desktop/dist/` | Vite 构建的前端资源 |
| `apps/desktop/src-tauri/gen/` | Tauri 自动生成的 schema |
| `.tools/` | 项目本地 Rust 工具链、下载工具和缓存 |
| `.runtime/` | 开发设置、测试结果、截图及测试生成的配置与密钥 |

这些目录由 `.gitignore` 排除，并在 `.vscode/settings.json` 中默认隐藏；需要查看时，将对应的 `files.exclude` 项设为 `false`。隐藏不影响磁盘文件或程序运行。工具链和运行数据仍使用原路径。

## 开发与验证

以下命令在项目根目录执行。Windows 开发环境初始化和启动步骤见 [项目首页](../README.md)。

```powershell
. .\scripts\dev\env.ps1
npm.cmd run build
npm.cmd run test:ui
npm.cmd run test:packaging
npm.cmd run test:linux
cargo test --locked --workspace --exclude tinc-desktop
cargo build --locked -p vpn-agent -p tinc-desktop
cargo build --locked -p vpn-agent --example process_fixture
npm.cmd run test:e2e
```

`npm.cmd run test:linux` 使用 Docker Desktop 的 `rust:1.98-bookworm` Linux/amd64
容器执行 Rust 测试、全工作区 Clippy、Agent 和 Tauri 桌面构建。Cargo 与 APT 下载缓存固定
在项目的 `.runtime/docker-cache/`（D 盘），Linux 编译输出固定在 `target-linux/`；容器使用
`--rm`，结束后不保留可写层。Docker Desktop 自身镜像/虚拟磁盘位置仍由 Docker Desktop
设置控制。

Windows 上验证真实 Tauri 窗口可运行 `npm.cmd run test:e2e -- --desktop`，需要本机 WebView2。测试脚本为每次 Debug E2E 生成受限的独立 IPC 实例后缀，不会争用默认开发端点；生产端点仍固定且不接受该覆盖。

测试产物保存在 `.runtime/e2e-*/`。测试后端为本地 fixture，结果不代表真实 VPN 已互通。历史运行结果只证明当时的代码；脚本新增检查需要重新执行验证。

## 打包与服务安装

Windows 桌面安装包：`npm.cmd run desktop:build:windows`。此入口同时构建 Release 后台服务，并在需要时将 `WebView2Loader.dll` 放进桌面安装目录；本机 GNU 工具链请使用此入口。
NSIS 工具下载超时时，可先执行 `powershell -NoProfile -File .\scripts\build\cache-nsis.ps1`，再重试打包。
Linux 桌面安装包：`npm run tauri --workspace apps/desktop -- build --bundles deb`。

桌面安装包与后台服务分别安装。服务安装脚本位于 `packaging/`，由管理员执行；Windows 脚本需要 `AgentBinary`、`TincDirectory`、`OwnerSid`、`Interface` 和审核过的 `TincSha256` 参数，先验证 Tinc 1.0.36 与指定的 TAP-Windows Adapter V9；Linux 脚本需要服务程序绝对路径、用户 UID 和网卡名，先验证 Tinc 1.0.36、`iproute2` 的 `/usr/sbin/ip` 及可读写的 `/dev/net/tun`。两端新生成的设置明确写入后端确认的 `interface_prefix: 32`；这些检查不设置地址或路由。Tinc 与 TAP/TUN 等环境要求见 [接口契约](API_CONTRACT.md)。

安装包构建结果见 [项目交接](HANDOVER.md)。真实后端联调和系统服务安装验收仍待完成。
