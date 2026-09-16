# 2026 湖南省大学生计算机程序设计竞赛：Tinc 三天执行计划与证据基线

> 项目：跨平台 Tinc 虚拟网络部署与管理平台
> 赛道：应用开发赛道
> 基线时间：2026-09-15 23:15（Asia/Shanghai）
> 原则：三天内只完成“一台测试网关 + 一台 Windows 客户端”的稳定闭环，不启动 fnps 或动态插件开发。

## 1. 当前结论

原说明中的“SSH 连接超时、网关尚未迁移为 router、尚未真实握手”已经不再是当前事实。

目前服务端 `new_network` 已满足 Tinc 数据面运行条件，并保留了 `new_client` 的真实握手和数据包日志。当前最高风险已转移到 Windows 客户端部署：最新 Agent 和桌面程序已经构建，但本机尚未安装 `TincClientAgent` 服务，因此当前没有专用路由，无法再次通过 UI 复现连接。

历史成功不能替代本次竞赛版本验收。以下严格区分“当前验证”“历史证据”和“未验证”。

## 2. 证据矩阵

| 项目 | 状态 | 2026-09-15 可复核证据 |
|---|---|---|
| SSH 到测试网关 | 当前已验证 | `ssh aliyun-tinc` 成功，检查时间 `2026-09-15T23:15:19+08:00` |
| Tinc 模式 | 当前已验证 | `/etc/tinc/new_network/tinc.conf`：`Name=server_master`、`Interface=tinc0`、`Mode=router`、`Port=600` |
| 服务端 host | 当前已验证 | `Address=8.148.13.80`、`Port=600`、`Subnet=10.0.11.1/32`；未输出公钥正文 |
| 服务进程 | 当前已验证 | `tinc@new_network.service` active/enabled，MainPID `2183848`，NRestarts `0` |
| TCP/UDP 监听 | 当前已验证 | TCP/UDP 600 均由 PID `2183848` 监听 IPv4/IPv6 |
| 主机防火墙 | 当前已验证 | firewalld 运行时和永久 `600/tcp`、`600/udp` 均为 `yes` |
| 服务端私钥 | 当前仅验证元数据 | 文件存在，`root:root`、`0600`、3243 bytes；未读取内容 |
| 客户端 peer 文件 | 当前已验证 | `hosts/new_client` 存在，`root:root`、`0600`、626 bytes；SHA-256 `e2e7a343ccdefe4f59f3b78f7a158c6c5d54b58a31a5fc3c6f0fa627c216d89d`；未输出公钥正文 |
| 公网 TCP 600 | 当前已验证 | Windows `Test-NetConnection 8.148.13.80 -Port 600` 为 `True` |
| Tinc 握手 | 历史真实证据 | 2026-09-14 日志包含 `Connections: new_client`、双向 Edge、`10.0.11.211/32 owner new_client` |
| 隧道数据包 | 历史真实证据 | 日志包含从 `new_client` 收包并写入 Linux tun/tap device |
| 最新 UI/Agent 再现握手 | 未验证 | Windows 尚无 `TincClientAgent` 服务 |
| 当前客户端路由 | 未建立 | 当前没有 `10.0.11.1/32` 路由，因为 Agent/VPN 未运行 |
| 当前 Ping | 未通过，不能归因网关 | 当前 `ping 10.0.11.1=False`，前置路由和 Tinc 进程均不存在 |
| 当前登录/下载/上传 HTTP | 本轮未验证 | 缺少本轮测试凭据；不得读取或记录密码、Token |

## 3. 已完成的本机构建验证

2026-09-15 已完成：

- Vue UI 单元测试：7 项通过，0 失败。
- Windows Process Job 测试：2 项通过。
- vpn-core 单元测试：3 项通过。
- API 契约测试：8 项通过。
- 配置安全测试：12 项通过。
- IPC 测试：3 项通过。
- 最新 Release Agent 构建成功：
  - SHA-256：`45FEF385B6D3284A6A85AD2DB3487655359504D1BE34E65FE9770A260B9756DD`
- 最新 Release 桌面程序构建成功：
  - SHA-256：`907E350C5C5EB3DD1DEF80B1D53991FCCE6A6BA380521E1A8339DD249C60229C`
- 审核的 Tinc 1.0.36：
  - SHA-256：`746999BAB0FE3839304A831A4BA021C190D8F20AF554ECC39D3B122DE3A8C21C`
- TAP 网卡 `tinc` 存在，类型为 TAP-Windows Adapter V9。

## 4. 风险排序

### P0：必须在第一天解决

1. 安装并运行最新 `TincClientAgent` Windows 服务。
2. 确认生产 IPC 可连接，桌面端显示 Agent 可用。
3. 使用真实测试账号通过 UI 完成登录、配置、上传公钥、启动。
4. 同时取得客户端路由/Ping和服务端当前握手日志，形成同一次运行的双端证据。

### P1：第二天解决

1. 验证相同公钥重复配置的幂等行为。
2. 验证停止、再次启动、Agent 重启后的状态恢复和路由清理。
3. 验证错误密码、服务器不可达等非破坏性失败路径及 UI 提示。
4. 修复在真实闭环中暴露的阻断性问题；不做无关重构。

### P2：第三天完成

1. 连续三次完整演示排练。
2. 固化构建物、哈希、演示账号准备流程和脱敏证据。
3. 准备网络不稳定时的录屏和截图，但明确它是历史演示证据，不冒充现场连接。
4. 代码冻结；只处理会阻断演示的问题。

### 延后

- fnps 插件。
- 动态插件加载、插件市场和热更新。
- 多网关自动调度。
- 大规模并发和复杂高可用。
- 对现有类、表和路由的批量重命名。

## 5. Day 1：完成最新版本真实闭环

### 5.1 安装 Agent

在管理员 PowerShell 执行经过审核的安装脚本。安装后记录：

```powershell
Get-CimInstance Win32_Service -Filter "Name='TincClientAgent'" |
  Select-Object Name, State, StartMode, ProcessId, PathName

Get-FileHash "C:\Program Files\TincClientAgent\vpn-agent.exe" -Algorithm SHA256
Get-FileHash "C:\Program Files\TincClientAgent\tinc\tincd.exe" -Algorithm SHA256
```

验收：服务为 Running/Auto，安装 Agent 哈希与本次 Release 一致，Tinc 哈希与批准值一致。

### 5.2 UI 登录与配置

使用服务端地址：

```text
http://8.148.13.80:83
```

按 UI 顺序执行登录和“下载并应用配置”。不得截图或记录密码、Token、公钥正文。

记录：

- 客户端阶段和脱敏日志摘要。
- 后端 HTTP 状态与稳定业务错误码。
- 服务端 `hosts/new_client` 修改时间、长度和 SHA-256。
- ZIP 条目列表，确认仍只有 `tinc.conf` 和 `hosts/server_master`。

### 5.3 启动和数据面验证

启动 VPN 后立即记录：

```powershell
Get-Process tincd -ErrorAction SilentlyContinue |
  Select-Object Id, StartTime, Path

Get-NetIPAddress -InterfaceAlias tinc -AddressFamily IPv4 -ErrorAction SilentlyContinue
Get-NetRoute -DestinationPrefix 10.0.11.1/32 -ErrorAction SilentlyContinue
Test-Connection 10.0.11.1 -Count 4
```

服务端同步记录：

```bash
systemctl is-active tinc@new_network.service
ss -lntup '( sport = :600 )'
journalctl -u tinc@new_network.service --since '-5 min' --no-pager
```

验收条件必须同时满足：

1. 客户端 tincd 进程运行。
2. TAP 网卡地址为分配的 `node_ip/32`。
3. 存在经 `tinc` 接口到 `10.0.11.1/32` 的精确路由。
4. Ping 成功，UI 显示已验证连通。
5. 服务端同一时间段出现 `new_client` Connection/Edge/数据包证据。

仅满足进程运行不能宣称隧道连通。

## 6. Day 2：可靠性和失败路径

按以下顺序执行，每项都保留脱敏证据：

1. 点击停止，确认 tincd 退出、本次地址和 `10.0.11.1/32` 路由被精确删除。
2. 再次启动，确认无需重新登录服务器即可恢复连接。
3. 重启 `TincClientAgent`，确认状态恢复符合预期，不删除不属于本程序的地址或路由。
4. 重复配置相同公钥，确认服务端 peer 哈希不变且操作成功。
5. 使用错误密码验证明确的认证失败，不泄露后端原始消息。
6. 使用不可达测试地址验证超时、重试按钮和旧有效配置保留；不修改服务器防火墙制造故障。
7. 若真实联调暴露问题，只修阻断性代码，并重跑 UI 7 项和 Rust 28 项测试。

## 7. Day 3：演示冻结

### 7.1 三次排练

每次按固定脚本执行：

```text
打开客户端
→ Agent 状态正常
→ 输入服务端地址和测试账号
→ 登录
→ 下载并应用安全配置
→ 启动 VPN
→ UI 显示已验证连通
→ 展示精确路由和 Ping
→ 展示服务端当前 Connection/Edge 日志
→ 停止 VPN并展示路由清理
```

三次都成功才将该版本标为竞赛演示候选版。

### 7.2 冻结构建物

记录但不公开敏感信息：

- Git/源码快照标识。
- 桌面程序和 Agent 的 SHA-256。
- Tinc 发行物批准哈希。
- 构建和测试命令摘要。
- 服务器配置摘要，不包含私钥和公钥正文。

### 7.3 演示兜底

- 准备一次完整成功录屏。
- 准备 router 配置、双协议监听、客户端精确路由、Ping、服务端握手日志的脱敏截图。
- 现场网络失败时明确说明当前现场未连通，并使用录屏展示已复核历史结果，不伪造实时状态。

## 8. 后续七天竞赛材料清单

### 项目简介

- 一句话定位：让普通用户通过桌面 UI 完成 Tinc 身份认证、配置交换、密钥接入、进程管理和隧道验证。
- 明确范围：竞赛版本聚焦单测试网关和单客户端稳定闭环。

### 问题与价值

- 传统 Tinc 需要手工生成密钥、编辑配置、交换 host、公网端口和网卡操作。
- 配置错误容易表现为“进程运行但隧道不通”。
- 本项目把上述步骤收敛为可审计、可诊断、可重试的 UI 工作流。

### 架构图

- RuoYi/Vue 管理面。
- Spring Boot API 和 Redis 临时会话。
- 安全 ZIP 与公钥交换。
- Tauri 普通用户 UI。
- 受限 IPC。
- 高权限 Rust Agent。
- tincd、TAP/TUN、精确路由和真实探测。

### 核心创新

1. 前端 UI 与高权限网络操作隔离。
2. ZIP 内容和 Tinc 指令白名单。
3. 私钥永不进入 UI 和服务端。
4. 版本化配置和失败保留旧配置。
5. 公钥上传与节点身份绑定，支持幂等。
6. 将“进程运行”和“隧道已验证连通”明确区分。
7. Windows/Linux 统一业务流程、平台侧分别执行网卡生命周期。

### 演示脚本

- 采用 Day 3 固定脚本，控制在可重复的单网关/单客户端范围内。

### 测试证据

- API、配置安全、IPC、进程、UI 自动测试结果。
- ZIP 条目与媒体类型证据。
- 服务端 router、监听、防火墙和 systemd 摘要。
- 客户端地址、路由、Ping 与状态截图。
- 同一次运行的服务端 Connection/Edge/数据包日志。
- 构建物 SHA-256。

### 已知限制

- 当前只支持 Tinc 1.0.x RSA 工作流。
- 当前聚焦单机一个活动网络和一个授权用户。
- 未实现完整插件平台、fnps、多网关调度和高可用。
- ICMP 可能受网络策略影响，后续应增加协议级健康检查。
- 现场互联网质量不属于系统完全可控范围。

## 9. 竞赛表述修正

答辩材料不应继续写“SSH 一直超时、尚未真实握手”。更准确的表述是：

> 测试网关 `new_network` 已完成 router 化、服务启用、TCP/UDP 600 和防火墙验证；2026-09-14 已取得客户端真实握手、Edge、Subnet 和数据包证据。竞赛候选版的当前待办是安装最新 Windows Agent，并以最新 UI 再执行一次可重复的同批次端到端验收。

## 10. 2026-09-16 接续验证记录

### Windows Agent 安装状态

- `TincClientAgent` 已安装。
- Service State：`Running`。
- StartMode：`Auto`。
- Service PID：`5840`。
- Service Path：`C:\Program Files\TincClientAgent\vpn-agent.exe --service`。
- 生产 IPC `\\.\pipe\tinc-client-v1` 已存在。
- 安装目录仅允许 SYSTEM/Administrators 读取；普通用户读取被拒绝符合安装脚本的 ACL 设计，不能误记为文件缺失。

### 生产 IPC 状态

通过受限长度前缀 JSON IPC 的 `get_status` 取得：

```json
{
  "phase": "stopped",
  "session": null,
  "configured": {
    "sid": "Ali_node1",
    "net_name": "network1",
    "node_ip": "10.0.10.11"
  },
  "process_running": false,
  "connection": "unknown",
  "stage": "已恢复本地配置，VPN 未启动",
  "last_error": null
}
```

该结果证明 Agent 和 IPC 可用，但当前恢复的是旧 `network1`，不是竞赛测试网络 `new_network`。

### 当前网卡和连通状态

- TAP `tinc` 当前可见 `10.0.11.211/24` 和 Windows APIPA 地址，地址状态为 Tentative。
- 当前不存在到 `10.0.11.1/32` 的 Agent 管理路由。
- 公网 TCP 600 仍可达。
- 当前 Ping `10.0.11.1` 失败；由于客户端 tincd 未运行且精确路由不存在，不能把该失败归因于服务端。

### 桌面程序

- 2026-09-15 构建的最新 Release 桌面程序已启动。
- 下一步必须由用户在 UI 内输入测试凭据，登录 `http://8.148.13.80:83`，再执行配置和启动。
- 密码和 Token 不由自动化读取、记录或代填。

### 快捷方式导致 Agent 中断的故障与修复

2026-09-16 复测桌面快捷方式时，UI 返回：

```text
agent_unavailable：无法连接本地服务，或服务连接中断
```

已确认根因不是服务器或 Tinc 600 端口：

1. 桌面 `Tinc Connect.lnk` 原本指向 `scripts/dev/launch.ps1`，并非正式 Release UI。
2. 该开发脚本使用 `Get-Process -Name vpn-agent | Stop-Process -Force`，会把 Program Files 中由 SCM 托管的正式 `TincClientAgent` 一并终止。
3. System Event 7034 记录 `Tinc Client Agent` 于 `2026-09-16 08:39:28` 意外停止。
4. 正式服务停止后生产管道消失，UI 因而正确返回 `agent_unavailable`。

已完成修复：

- `scripts/dev/launch.ps1` 现在只结束路径精确等于仓库 `target/debug/vpn-agent.exe` 且命令行包含 `--dev` 的 Debug Agent，不再触碰正式服务。
- 桌面 `Tinc Connect.lnk` 已改为直接启动最新 `target/release/tinc-desktop.exe`。
- 原开发入口保留为 `Tinc Connect Dev.lnk`，用于明确的开发场景。
- `launch.ps1` Windows PowerShell 语法检查通过。
- `npm.cmd run test:packaging` 通过。

当前普通进程无权启动 Windows 服务；需要在管理员 PowerShell 执行一次：

```powershell
Start-Service TincClientAgent
```

启动成功后，普通 `Tinc Connect` 快捷方式不会再杀死该服务。

### Release 窗口加载 127.0.0.1 的构建故障与修复

2026-09-16 快捷方式改为 Release EXE 后，窗口曾显示 Edge
`ERR_CONNECTION_REFUSED`，目标为 `127.0.0.1`。该问题与 Agent 和服务端无关：

- `apps/desktop/src-tauri/Cargo.toml` 缺少 Tauri `custom-protocol` feature。
- 直接 `cargo build --release` 生成的程序虽然是优化构建，但仍加载
  `tauri.conf.json` 中的开发地址 `http://127.0.0.1:1420`。
- 本机没有运行 Vite 开发服务器，因此 WebView 正确返回拒绝连接。

已完成修复：

- 增加默认 `custom-protocol = ["tauri/custom-protocol"]` feature。
- 打包前置测试增加生产桌面必须启用 custom protocol 的静态断言。
- UI 7 项测试通过。
- 打包前置测试通过。
- 使用 `tauri build --no-bundle` 正式重建，内嵌 `frontendDist`。
- 新 Release EXE SHA-256：`6A22B401E570FA565FF7B7365BEB6AB72719382BD082E522C3154C30D2308DA3`。
- 快捷方式仍指向同一 Release 路径，项目 “t” 图标保持不变。
- 新程序 PID `19612` 已启动并保持响应。

### 普通用户生产 IPC 身份校验故障与修复

2026-09-16 恢复 `TincClientAgent` 后，桌面端曾返回：

```text
ipc_identity：本地服务身份验证失败
```

已确认服务本身正常：

- Service State：`Running`。
- Service Account：`LocalSystem`。
- 生产命名管道 `\\.\pipe\tinc-client-v1` 存在。
- 使用长度前缀 JSON 可直接读取服务状态。

根因在客户端校验逻辑：旧实现通过 `OpenProcessToken` 读取管道服务端进程的
LocalSystem Token。普通交互用户默认没有读取系统服务进程 Token 的权限，因此把
正常的权限拒绝误报为服务身份不可信。

已完成修复：

- 正式模式先取得命名管道服务端 PID，再通过 Windows Service Control Manager 查询
  注册服务 `TincClientAgent` 的运行 PID；两者必须完全相同且服务必须为 Running。
- 普通用户无权注册、替换或修改系统服务，因此仍能防止普通本地进程冒充 Agent，
  没有取消身份校验。
- 开发模式继续校验管道服务端进程与当前用户 SID 相同。
- 新增显式生产服务联调测试 `windows_service.rs`。
- Rust 相关测试 5 项通过，生产服务联调测试 1 项通过，UI 7 项通过，打包检查通过。
- 联调后 Agent 仍为 Running，PID `29028`。
- 新 Release EXE SHA-256：`6A3B352F98F880ADFD25F0FD53A3EC7BC33D8D1C6912C7A262990DE1DFA74FA2`。
- 新桌面程序 PID `27560` 已启动并保持响应；快捷方式和项目 “t” 图标保持正确。
