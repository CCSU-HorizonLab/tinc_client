# 项目交接

更新日期：2026-09-10（北京时间）。新会话先读本文，再读 [接手提示词](NEXT_SESSION_PROMPT.md)、[开发指南](README.md)、[接口契约](API_CONTRACT.md)、[架构](ARCHITECTURE.md) 和 [安全边界](SECURITY.md)。

## 工作范围

- 新项目：`D:\Codes\Java\KenDeJi_RuoYi\tinc_client`，所有新代码在这里维护。
- 旧项目：`D:\Codes\Java\KenDeJi_RuoYi\tinc_cli_gui\windows\demo_win\code_win`，只读参考，不修改或清理。
- 当前新目录没有 Git 仓库；不能用提交记录判断完成度。
- 产品目标是让非专业用户通过界面完成 Tinc 配置，优先保证真实错误、失败重试和旧配置保留。
- Vue 3 / TypeScript / Vite + Tauri 2 + Rust；Tinc 负责隧道通信。普通用户 UI 通过受限 IPC 调用独立后台服务。

## 当前实现

已实现登录和内存会话、下载与校验 ZIP、本地 RSA 密钥生成与复用、公钥上传、版本化配置与原子切换、进程启停和状态、最近日志、Windows Named Pipe / Service 与 Linux Unix Socket / systemd 适配代码。09-08 将可执行后台程序从 `crates/vpn-agent` 移至 `apps/agent`，并拆分为启动入口、IPC 服务和平台服务生命周期；Cargo 包名与生成的 `vpn-agent.exe` 保持不变。

当前只支持单机一个活动网络和一个授权用户。后台审批无法查询，状态上报的 `ids` 来源不明确，相关功能未启用。运行中仅表示进程运行；真实连接状态仍为未知，没有经过验证的路由或隧道探测。

Windows/Linux 服务安装和卸载脚本已实现。Linux 行为、真实服务安装、TAP/TUN 与隧道互通未验收。

09-08 本机只读检查中，`Get-Command tincd` 指向 `C:\Windows\System32\tincd`，但该文件大小为 0、没有扩展名，不能视为有效 Tinc 程序。随后确认旧项目的 `D:\Codes\Java\KenDeJi_RuoYi\tinc_cli_gui\windows\Tinc\tincd.exe` 可运行，实际输出为 `tinc version 1.0.36`（不是口述的 1.0.6），SHA-256 为 `746999BAB0FE3839304A831A4BA021C190D8F20AF554ECC39D3B122DE3A8C21C`。该文件可用于隔离联调，尚未作为经过审核和签名的正式发布依赖纳入新安装包。

## 已执行的验证

| 日期 | 检查 | 结果与范围 |
| --- | --- | --- |
| 09-07 22:18 | 前端构建、`npm.cmd run test:ui` | 构建成功，5 项测试通过 |
| 09-07 22:51 | 真实 Tauri WebView2 桌面集成 | 11 项通过；真实原生桥接、IPC、RSA 和文件落盘，后端为 fixture |
| 09-07 22:55 | `cargo clippy --offline --workspace --all-targets -- -D warnings` | 通过；同轮执行了 `cargo fmt --all` |
| 09-07 22:57 | `cargo test --offline --workspace --exclude tinc-desktop` | 15 项通过：API 3、配置安全 8、IPC 2、Windows Job Object 2 |
| 09-07 22:57 | Debug 桌面和服务、进程测试 fixture、Release 服务 | 构建成功；Release 桌面也已在打包尝试中编译成功 |
| 09-08 15:36 | 目录整理后前端构建、UI 单元测试、普通 IPC 集成 | 构建成功，5 项 UI 测试、11 项本地集成检查通过 |
| 09-08 | 整理后的 PowerShell 脚本、Node 测试脚本 | 语法、项目根目录解析、旧路径引用检查通过 |
| 09-08 15:51 | `npm.cmd run desktop:build:windows` | Release 桌面与后台服务构建、NSIS 打包成功；已补入动态依赖的 WebView2Loader.dll |
| 09-08 15:51 | 安装包静态检查 | 检查了桌面/服务 PE 导入表、NSIS 资源映射、产物大小、SHA-256 和签名状态；尚未运行安装器 |
| 09-08 16:13 | 应用目录重组后的 Rust 与前端检查 | Rust 15 项测试通过，Clippy 警告归零；前端构建成功，5 项测试通过 |
| 09-08 16:15 | 应用目录重组后的普通 IPC 集成 | 11 项通过；确认移动后的 `apps/agent` 可启动并完成完整 fixture 流程 |
| 09-08 16:23 | 应用目录重组后重新打包 | Release 桌面、后台服务和 NSIS 安装包构建成功，DLL 资源映射仍有效 |
| 09-08 | 后端配置包初版样本联调 | 初版 733 字节样本含服务端 Interface，客户端校验通过；该样本随后被不含 Interface 的最终 721 字节样本替代 |
| 09-08 | 后端部署后配置包联调 | 后端实际下载为 HTTP 200、1027 字节且仅含两个允许文件；新版 721 字节脱敏样本不含 Interface，客户端补入受控网卡名并完成配置应用 |
| 09-08 | Windows/Linux 配置契约复验 | Windows 工作区 17 项测试、Clippy、Agent 与桌面构建通过；Linux/amd64 容器 15 项适用测试、Agent 与 Tauri 桌面构建通过 |
| 09-09 | 安装前置检查 | `npm.cmd run test:packaging` 通过：静态核对 Windows Tinc SHA-256/1.0.36/TAP-Windows Adapter V9 与 Linux Tinc 1.0.36/`iproute2`/TUN 检查，并完成 Windows PowerShell 语法解析；未安装服务、Tinc 或驱动，也未修改本机网卡 |
| 09-09 | 常规回归 | 前端 6 项单元测试、Rust 工作区 26 项测试、`cargo fmt --all --check` 与 Clippy 均通过 |
| 09-09 | 公钥交换契约 | 已读取后端 `TINC_CLIENT_KEY_EXCHANGE_RESPONSE.md`；客户端严格发送 sid + Token、`exchangeFile` 和 `Subnet = node_ip/32` PKCS#1 RSA 内容，HTTP 200 仅接受 `text/plain`。本地 API 8 项覆盖成功、Token 失效、媒体类型拒绝、重复上传一致性及安全业务码映射；后端 `msg` 不展示 |
| 09-09 | 网卡停止清理 | 已实现正常停止、启动中失败、进程异常退出及服务正常关闭时删除本次 `/32` 地址；Windows 使用 active store 并保留 TAP，Linux 删除地址后置接口 down。Windows 命令构造单元测试通过；Linux 新分支因本机无 Docker/Linux target 尚未重新编译，仍需 CI 与实机验证 |
| 09-09 | 隔离 Debug IPC 与 E2E | Debug 模式新增受限实例后缀，生产端点不变；现有默认 Agent 运行时仍完成 11 项 fixture 集成检查，最新结果为 `.runtime/e2e-jjxABc/result.json`，未宣称真实 VPN 连通 |
| 09-09 | 严格 host 契约 | `server_master` 在 ZIP、上传响应和落盘阶段统一要求唯一 Address、唯一 `/32` Subnet、至多一个 Port、合规 PKCS#1 RSA 且 PEM 后无额外内容；客户端 host 绑定精确 `node_ip/32`。安全测试增至 12 项 |
| 09-10 | 强制终止后的网卡残留恢复 | 增加受保护、原子写入的 `adapter-state.json`；Agent 重启只按与管理员设置和 `active.json` 完全匹配的专用接口、精确 IP/前缀、generation/profile 清理，不匹配返回 `adapter_state_invalid`。新增 2 项记录校验测试；Windows 工作区共 28 项测试、Fmt、全工作区 Clippy、Agent/桌面构建均通过 |
| 09-10 | 常规与集成回归 | 前端构建、6 项 UI 测试、`test:packaging` 和 11 项 fixture E2E 通过；最新结果 `.runtime/e2e-96rJnY/result.json`，仍不代表真实 VPN 连通 |
| 09-10 | Linux/amd64 回归 | 启动已有 Docker Desktop 后，在 `rust:1.98-bookworm` 中通过 26 项 Linux 适用测试、全工作区 Clippy（`-D warnings`）、Agent 与 Tauri 桌面构建；覆盖 09-09 清理和 09-10 恢复分支。仍不是 systemd/TUN 实机验收 |
| 09-13 | 对端路由与连通探测 | 保留客户端 `/32`，从严格校验的 `server_master` 提取唯一对端 `/32`，启动时创建经专用接口的 active 路由，停止/异常/恢复时精确删除；每 5 秒 ICMP 探测并输出 connected/unreachable/unknown。Windows 工作区 28 项测试、Clippy、前端 7 项测试与构建通过；真实新二进制启停尚待完成 |
| 09-10 | D 盘缓存与 C 盘清理 | 新增 `npm.cmd run test:linux`，Cargo/APT 缓存写入项目 `.runtime/docker-cache/`，构建输出写入 `target-linux/`。删除本轮遗留的停止容器和未使用 Cargo 卷，Docker 可回收占用由约 413.5 MB 降至约 4 KB；未动已有镜像、PostgreSQL 容器或数据卷 |

09-07 的桌面集成有一次重启状态断言失败，随后修正并重跑通过。09-08 的 11 项普通 IPC 检查含新增的子进程提前退出场景，不包含桌面窗口；两次同为 11 项，但覆盖范围不同。目录重组后两次尝试执行桌面路径，`tinc-desktop.exe` 可以启动，但当前 WebView2 Runtime 未开放 9223 调试端点，Playwright 无法接管窗口，测试在进入页面断言前停止。测试脚本已为 WebView2 设置独立数据目录，仍未解决；不能将这两次失败归因于页面或 IPC 断言。

历史测试结果：`.runtime/e2e-UqIqIm/result.json`（09-07 桌面）、`.runtime/e2e-IbszM9/result.json`（09-08 普通 IPC）。这些本地产物不会提交到版本控制。

## 昨晚的中断与今天的整理

09-07 的 NSIS 打包在工具下载阶段先后遇到访问限制和超时。22:58 已将官方 NSIS 和插件缓存到 `target/.tauri/NSIS`，两项哈希校验成功，随后触发 `usage_limit_exceeded`。22:59 用户要求生成第二天接手提示词的回复也被额度限制中断，没有最终交付报告。

原始记录位于用户目录 `.codex/sessions/2026/09/07/rollout-2026-09-07T21-36-02-01a07c15-410d-79d3-80a5-8864aa59e3d3.jsonl`，这是回顾历史执行的依据。不要将完整会话、凭据或本地私钥复制进项目文档。

09-08 已将脚本分类，相关调用与 CI 同步更新：

| 旧路径 | 当前路径 |
| --- | --- |
| `scripts/dev-env.ps1` | `scripts/dev/env.ps1` |
| `scripts/init-dev.ps1` | `scripts/dev/init.ps1` |
| `scripts/e2e-local.mjs` | `scripts/test/e2e-local.mjs`；统一入口 `npm.cmd run test:e2e` |
| `scripts/cache-nsis.ps1` | `scripts/build/cache-nsis.ps1` |

同日按职责重组应用代码：可执行的 `vpn-agent` 从 `crates/vpn-agent` 移至 `apps/agent`；原来的单文件入口拆成 `main.rs`、`server.rs`、`service/windows.rs` 和 `service/linux.rs`。前端调用移至 `apps/desktop/src/services`，样式移至 `apps/desktop/src/styles`。`crates/` 现在只放共享库。

新增开发指南和 VS Code 生成目录隐藏设置；源码、工具链和运行数据根目录未移动。本文补齐了原始要求中遗漏的交接文档。

## 启动、调试和复验

以下命令在项目根目录执行。项目本地工具链位于 `.tools`，每个 PowerShell 终端先执行环境脚本。开发模式只在 Debug 程序中生效。

```powershell
. .\scripts\dev\env.ps1
npm.cmd ci
npm.cmd run build
npm.cmd run test:ui
npm.cmd run test:packaging
npm.cmd run test:linux
cargo fmt --all --check
cargo test --locked --workspace --exclude tinc-desktop
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo build --locked -p vpn-agent -p tinc-desktop
cargo build --locked -p vpn-agent --example process_fixture
npm.cmd run test:e2e
```

真实桌面路径：`npm.cmd run test:e2e -- --desktop`，要求 Windows 和 WebView2。测试脚本会为 Debug Agent 和桌面进程注入同一受限实例后缀，避免争用默认开发 IPC；生产端点始终固定。

手动开发首次运行 `powershell -NoProfile -File .\scripts\dev\init.ps1 -TincBinary '实际 tincd.exe 绝对路径' -Interface '实际网卡名'`。这两个参数都是必填项，脚本会确认 Tinc 文件存在并将其规范化为绝对路径，显式写入后端已确认的 `interface_prefix: 32`。已有 `.runtime/settings.json` 时直接编辑，初始化脚本拒绝覆盖。

Agent 找不到设置时返回 `settings_missing`，设置无法读取时返回 `settings_read`，JSON 格式错误时返回 `settings_invalid`；这些情况不再统一显示成笼统的 `storage` 错误。

终端一：先加载 `scripts/dev/env.ps1`，再运行 `cargo run --locked -p vpn-agent -- --dev .runtime/settings.json`。
终端二：同样先加载 `scripts/dev/env.ps1`，再运行 `npm.cmd run desktop:dev`。
浏览器 `npm.cmd run dev` 只预览界面；网卡操作仍需要系统权限。

## 打包与安装

```powershell
. .\scripts\dev\env.ps1
npm.cmd run desktop:build:windows
```

若缓存缺失或下载超时，先运行 `powershell -NoProfile -File .\scripts\build\cache-nsis.ps1`。Tauri 配置已启用 `useLocalToolsDir`，不需要重新移动缓存。

Windows 打包入口同时构建 Release 后台服务。09-08 检查 PE 导入表发现本机 GNU 桌面程序动态依赖 `WebView2Loader.dll`，直接调用 Tauri NSIS 构建会漏装该 DLL。因此新增 `scripts/build/windows.ps1`：读取 Cargo 实际产物路径，将旁边的 DLL 映射到安装目录根部，然后生成 NSIS 包。临时合并配置写入 `.runtime/build/windows-bundle.json`，CI 也改用统一入口。

### 2026-09-08 交付产物

| 产物 | 路径 | 大小 |
| --- | --- | --- |
| Windows x64 桌面安装包 | `target/release/bundle/nsis/Tinc Connect_0.1.0_x64-setup.exe` | 5,639,937 字节 |
| Windows 后台服务 | `target/release/vpn-agent.exe` | 10,539,567 字节 |

SHA-256：

```text
5EEC06E19637FDB5BA3D42F16C460CACCB3C362081845375AD554FA74CA69A30  Tinc Connect_0.1.0_x64-setup.exe
B26AB9BA9FE35C1575C73127FF748ED815A49F64B66B97D0A4804F9CACD9793C  vpn-agent.exe
```

两项产物的 Authenticode 状态均为 `NotSigned`。此前 15:45 生成的未带 DLL 安装包已被这次重新构建替换，应以以上哈希为准。

已核对生成的 `target/release/nsis/x64/installer.nsi`：安装模式为 `currentUser`，包含 `WebView2Loader.dll` 并安装在程序旁。WebView2 Runtime 不完整内嵌；未安装 Runtime 的机器会由安装器下载引导程序，需要网络。

桌面安装包只负责当前用户的 UI，不自动安装高权限后台服务或 TAP 驱动。生产 UI 使用生产 IPC，不能通过设置开发环境变量连接 Debug 服务。

旧发布目录包含 `tincd.exe`、`COPYING.txt` 和 `tap-win64`。旧安装流程通过
`tapinstall.exe install OemWin2k.inf tap0901` 创建 TAP-Win32 Adapter V9，再将新网卡
重命名为 `tinc0`；`Interface = tinc0` 只选择已经存在的网卡，不会自行安装驱动或创建网卡。
旧 Windows 启动脚本使用 `255.255.255.0`（`/24`），停止脚本将网卡改回 DHCP；Linux
停止脚本将接口置为 down。`tincd.exe`、`tapinstall.exe` 和 `tap0901.sys` 的文件级
Authenticode 状态为 `NotSigned`，`tap0901.cat` 的 OpenVPN Technologies 签名有效。

面向客户的后续安装器应自动完成哈希锁定的 Tinc 核心部署、TAP 驱动检测/安装、专用网卡
创建与重命名、Agent 服务安装和受保护设置写入。界面无需让普通用户选择 Tinc 路径、网卡名
或地址前缀。停止 VPN 时保留专用网卡和驱动，清除 VPN 地址或恢复安装器定义的空闲状态；
卸载产品时才移除由本产品创建的网卡。该流程需先在支持的 Windows 版本验证驱动签名、
升级/回滚和已有 TAP 网卡共存行为。

Windows 服务安装在管理员 PowerShell 中运行以下命令，将示例参数替换为真实值。`TincSha256` 必须来自已经审核的待部署 `tincd.exe`，不接受未经审核的任意二进制；脚本在写入任何安装目录前检查它、精确的 1.0.36 版本和指定 TAP-Windows Adapter V9 是否已经存在：

```powershell
.\packaging\windows\install-agent.ps1 -AgentBinary .\target\release\vpn-agent.exe -TincDirectory 'C:\Program Files\tinc' -OwnerSid 'S-1-5-21-替换为交互用户SID' -Interface 'tinc0' -TincSha256 '审核后的64位SHA-256'
```

交互用户在自己的普通终端运行 `whoami /user` 获取 SID。安装脚本将服务程序与经 SHA-256 审核的单文件 Tinc 复制到 `%ProgramFiles%\TincClientAgent`，将设置写入 `%ProgramData%\TincClient`，服务名称为 `TincClientAgent`。它不会复制未校验的同目录 DLL；多文件 Tinc 发布物必须先增加审核过的哈希清单。需要预先准备经过审核的 Tinc 1.0.36 程序及实际 TAP-Windows Adapter V9 网卡。

卸载服务注册：管理员运行 `.\packaging\windows\uninstall-agent.ps1`。脚本保留程序、配置与私钥；当前没有自动升级流程，现有安装目录/设置会让安装脚本拒绝覆盖。

Linux 构建与安装命令见 `.github/workflows/check.yml` 和 `packaging/linux/`。安装脚本在写入前会检查绝对 Agent 路径、`/usr/sbin/tincd` 的 1.0.36 版本、`iproute2` 的 `/usr/sbin/ip` 以及可读写的 `/dev/net/tun`。09-08 已在隔离的 Debian Bookworm Linux/amd64 容器中完成 Debug Agent、Tauri 桌面构建及适用测试；尚未生成 DEB，也未在真实 Linux 主机验收 systemd、TUN 和网卡权限。

## 待完成与接手顺序

1. Windows NSIS 打包、DLL 依赖补齐和交接文档已完成；需要复现时运行 `npm.cmd run desktop:build:windows`。
2. 在准备好的测试环境验收桌面安装、服务安装/卸载、跨用户 IPC 拒绝、服务退出与子进程回收。
3. 后端下载与公钥交换接口已经部署并复测；客户端已按 sid + Token、`Subnet = node_ip/32`、PKCS#1 RSA、`text/plain` 响应契约实现。`status/update` 仍是匿名遗留接口，保持禁用。
4. 后端核查的 3 份旧网关仍为 `Mode = switch`，且交接时测试网关 TCP/22 不可达；当前客户端已取得一个 `Mode = router` 测试配置。管理员恢复 SSH 后，核实网关 Tinc 版本、TCP/UDP 600 监听、主机防火墙及云安全组，再实测一次变化公钥写入、重载成功和一次远程幂等上传。
5. 前缀已确认固定 `/32`，新开发和安装设置已显式写入。停止清理、命令构造和跨 Agent 重启恢复已实现，并已通过 Linux/amd64 编译和自动测试：删除记录中精确匹配的本次地址、Windows 保留 TAP、Linux 将记录中本次触碰的专用接口置 down；仍需两端强杀/服务重启/系统重启实机验收。系统命令成功到恢复记录落盘之间的极短窗口也需结合实机故障注入评估。
6. 在受控实机上验收新安装前置检查：审核的 Tinc SHA-256 与 1.0.36、专用 `tinc0` 的 TAP 驱动识别、Linux `iproute2` 与 `/dev/net/tun`；再执行服务安装/卸载。当前只验证了脚本逻辑，未改变任何真实系统资源。
7. 当前 `server_master` 的唯一 `/32` 已作为精确路由和探测目标；完整真实联调仍需要安全提供测试凭据、网关 `tincd --version`、TCP/UDP 监听与服务端 ICMP 能力。不要在文档记录凭据。
8. Linux Agent、桌面 Debug 构建和测试已经在容器完成；仍需在真实 Linux 环境完成 DEB 打包以及 systemd/TUN 验收。CI 配置已存在，尚无远端运行记录。

新会话可直接按本文接手：先确认当前目录和最近验证结果，完成仍未完成的交付；常规实现与本地构建不需要重复确认。涉及真实系统服务安装时，先核实参数与当前机器环境，并按工具权限要求执行。
