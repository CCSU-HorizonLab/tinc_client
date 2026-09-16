# 2026-09-10 接手提示词

更新时间：2026-09-10（北京时间）。新会话直接复制以下内容：

```text
继续开发 Tinc 新客户端。项目根目录：
D:\Codes\Java\KenDeJi_RuoYi\tinc_client

先完整阅读并以这些文件为准：

1. D:\Codes\Java\KenDeJi_RuoYi\tinc_client\docs\HANDOVER.md
2. D:\Codes\Java\KenDeJi_RuoYi\tinc_client\docs\BACKEND_HANDOFF.md
3. D:\Codes\Java\KenDeJi_RuoYi\tinc_client\docs\API_CONTRACT.md
4. D:\Codes\Java\KenDeJi_RuoYi\tinc_client\docs\ARCHITECTURE.md
5. D:\Codes\Java\KenDeJi_RuoYi\tinc_client\docs\SECURITY.md
6. D:\Codes\Java\KenDeJi_RuoYi\RuoYi-Vue-master\docs\TINC_CLIENT_CONFIG_RESPONSE.md
7. D:\Codes\Java\KenDeJi_RuoYi\RuoYi-Vue-master\docs\TINC_CLIENT_KEY_EXCHANGE_RESPONSE.md

不要修改或清理旧客户端目录；它只作只读参考：
D:\Codes\Java\KenDeJi_RuoYi\tinc_cli_gui\windows\demo_win\code_win

不要猜测后端协议、网卡前缀、网关版本或整网网段。可使用经过严格校验的 `server_master` 唯一 `/32` 作为精确路由及探测目标，但不能据此推断 `/24`。不要把密码、Token、真实私钥、完整生产公钥或测试账号写入代码、文档和快照。项目当前没有 Git 仓库，不能依赖 git status 或提交记录判断变化。

截至 2026-09-09 已完成：

一、配置下载契约

- POST /api/tinc/client/config/download 已由后端重新构建、部署和实测。
- 真实响应：HTTP 200，Content-Type application/zip，1027 字节，SHA-256：
  4A1C2E1BD43A0C8B8F874239C7A5CF2DBD822214321C76F4AAD64E4354200FB5
- ZIP 只有 <netName>/tinc.conf 和 <netName>/hosts/server_master；不含 Interface、脚本、私钥、EXE、DLL、Token、密码或说明文件。
- 脱敏 ZIP：
  D:\Codes\Java\KenDeJi_RuoYi\RuoYi-Vue-master\tests\fixtures\tinc-client-config.zip
  721 字节，SHA-256：
  EDFEE81D64348D8E889DA22FE1B3FD7B57D8FD7F8913DEE1D4DF0CD80F5858FF
- 客户端同时校验 application/zip 和 ZIP 魔数，白名单没有放宽，不执行服务器下载的 .bat、shell、tinc-up/down，不向后端增加 os 字段。
- ZIP 缺少 Interface 时，客户端只补入管理员设置的本机网卡名。

二、公钥交换契约

- 后端契约文件：
  D:\Codes\Java\KenDeJi_RuoYi\RuoYi-Vue-master\docs\TINC_CLIENT_KEY_EXCHANGE_RESPONSE.md
- POST /api/tinc/client/key/upload 必须发送 sid、登录 Token、action=exchangeFile，以及精确的 Subnet=node_ip/32 与 2048–8192 位 PKCS#1 RSA 公钥。
- 不使用 Authorization Header、Cookie 或旧客户端仅 sid 的不安全兼容请求。
- HTTP 200 必须为 text/plain hosts/server_master；JSON 或缺失/错误媒体类型的伪成功响应会被拒绝。
- Token 失效清除内存会话；相同公钥重试复用同一私钥和完全相同的上传内容。
- 客户端只映射后端已确认的业务 code，不显示后端 msg；已区分公钥拒绝、会话服务异常、网关不可用、网关写入/重载失败和节点关联缺失。
- server_master 在 ZIP、上传响应和最终落盘阶段统一要求：唯一 Address、唯一 /32 Subnet、至多一个 Port、合规 PKCS#1 RSA，PEM 后无额外内容。
- 客户端自身 host 必须绑定登录返回的精确 node_ip/32。
- /api/tinc/client/status/update 仍是匿名遗留接口，ids 没有可靠客户端来源，继续保持禁用。

三、配置、密钥与状态

- 已实现 RSA 3072 位私钥生成与稳定复用、下载后内存校验、server_master 校验、版本化配置、原子 active.json 切换和失败保留旧配置。
- Running 只表示 tincd 进程存活，connection 仍为 unknown；未显示成“隧道已连接”。
- 后台审批没有可查询契约，confirmation 仍为 unavailable。
- Debug IPC 支持仅由字母、数字、短横线和下划线组成的受限实例后缀；生产 IPC 端点固定且不接受覆盖。

四、Tinc、TAP/TUN 和安装检查

- 已确认旧发布目录真实 Tinc：
  D:\Codes\Java\KenDeJi_RuoYi\tinc_cli_gui\windows\Tinc\tincd.exe
  输出 tinc version 1.0.36，SHA-256：
  746999BAB0FE3839304A831A4BA021C190D8F20AF554ECC39D3B122DE3A8C21C
- 该旧 tincd.exe 未签名，只可作隔离联调参考；尚未作为审核和签名完成的正式发布依赖。
- 旧客户端使用 tap0901/TAP-Windows Adapter V9 并命名为 tinc0；Interface=tinc0 不会自动安装驱动或创建网卡。
- Windows 管理员安装脚本在写入前验证调用方提供的审核 SHA-256、精确 Tinc 1.0.36 和指定 TAP-Windows Adapter V9；只复制已校验的 tincd.exe，不复制未校验 DLL。
- Linux 安装脚本在写入前验证绝对 Agent 路径、Tinc 1.0.36、iproute2 的 /usr/sbin/ip 和可读写 /dev/net/tun。
- Windows/Linux 新生成设置显式写入 interface_prefix=32；不要改回旧版 /24。
- 正常停止、启动中失败、Tinc 异常退出及服务正常关闭时，客户端会删除本次设置的精确 /32 地址。Windows 使用 active store 并保留 TAP；Linux 删除地址后将本次触碰的专用接口置 down。
- 清理失败返回 adapter_cleanup，可再次停止重试。09-10 已实现强制杀死 Agent 后的跨重启恢复：受保护记录必须精确匹配当前接口、地址/前缀、generation 和 profile，否则返回 adapter_state_invalid 且不自动清理。

五、截至 2026-09-10 最新验证

- Windows Rust 工作区 28 项测试通过：API 8、配置/密钥安全 12、IPC 3、Windows Job Object 2、Windows 网卡命令 1、网卡恢复记录安全 2。
- cargo fmt --all --check 通过。
- cargo clippy --locked --workspace --all-targets -- -D warnings 通过。
- npm.cmd run test:packaging 通过；Windows PowerShell 安装脚本语法通过。
- 前端单元测试 6 项通过。
- npm.cmd run test:e2e 通过 11 项本地 fixture 集成检查；最新结果：
  D:\Codes\Java\KenDeJi_RuoYi\tinc_client\.runtime\e2e-96rJnY\result.json
- E2E 使用真实 Agent、IPC、RSA 和文件落盘，但远端后端是 fixture，不代表真实 VPN 已连通。
- 09-10 已启动已有 Docker Desktop，并在 `rust:1.98-bookworm` Linux/amd64 容器通过 26 项适用测试、全工作区 Clippy（`-D warnings`）、Agent 和 Tauri 桌面构建；09-09 Linux 清理及 09-10 跨重启恢复分支均已编译。仍未进行 systemd/TUN 实机验收。
- 新增 `npm.cmd run test:linux`；Cargo/APT 下载缓存固定写入 D 盘项目 `.runtime/docker-cache/`，Linux 编译输出写入 `target-linux/`。本轮 C 盘 Docker 临时容器和 Cargo 卷已删除，未动已有镜像、PostgreSQL 容器或数据卷。

后端和实机仍阻塞：

- 后端保存的 3 个旧网关仍为 Mode=switch；测试网关 TCP/22 不可达，尚未迁移为 router。
- 尚未取得网关真实 OS、tincd --version、监听端口和运行方式。
- 尚未完成第一次变化公钥远程原子写入、HUP 重载成功及第二次远程幂等确认。
- 网关虚拟地址 <segment>.1 只是代码推导候选，尚未确认 ICMP 或替代 TCP/UDP 探测目标。
- 尚未完成真实 Tinc 握手、Windows TAP、Linux TUN、systemd、服务安装/卸载和网卡清理验收。
- 当前 Windows NSIS 只安装普通用户桌面 UI；Agent 服务、Tinc 和 TAP 仍由独立管理员脚本处理，尚未合并为客户一体化安装器。
- 正式发布仍需要经过审核/签名的 Tinc 来源、TAP 驱动签名验证、安装器签名、升级/回滚和已有 TAP 共存验证。

2026-09-10 执行顺序：

1. 先读取后端两份契约文件是否有新回复，不要重复修改已经闭环的 ZIP 和 key/upload 请求。
2. 如果网关 SSH 已恢复：只迁移一个隔离测试网络为 Mode=router；只读确认网关 tincd 版本、监听端口、虚拟地址和探测能力；再验证变化公钥写入/HUP、相同公钥幂等和真实握手。不要在未经授权时修改远程网关。
3. Linux 地址删除、link down 和跨重启恢复已在容器完成测试、Clippy、Agent/Tauri 构建；后续代码变化可运行 `npm.cmd run test:linux` 复验，但不要声称完成 Linux 实机验收。
4. 在隔离 Windows 10/11 节点验证安装前检查、Tinc 1.0.36、TAP-Windows Adapter V9、tinc0、服务安装/卸载、跨用户 IPC 拒绝、启动 /32 地址和停止精确清理。
5. 在隔离 Linux 节点验证 iproute2、/dev/net/tun、systemd、启动 /32 地址、停止删除地址和接口 down。
6. 已实现从严格校验的 `server_master` `/32` 创建 active 路由并每 5 秒 ICMP 探测；仅探测成功设置 `connection=connected`，失败为 `unreachable`，工具不可用/未运行为 `unknown`。仍需新二进制实机启停验证，且不能把 ICMP 失败等同于 tincd 已退出。
7. 在隔离 Windows/Linux 实机验证 Agent 强制终止后的恢复记录、服务重启与系统重启行为；仍只允许清理记录中与当前设置和有效配置完全匹配的专用接口及精确地址。评估系统命令成功到记录落盘之间的极短强杀窗口。
8. 实机链路顺序：登录 → 下载 ZIP → 校验 → 生成/复用密钥 → 上传公钥 → 校验 server_master → 原子应用 → 启动 Tinc → 设置 /32 地址 → 网关握手 → 约定目标探测 → 停止 → 地址/接口清理。
9. 后续变化继续更新 HANDOVER.md、API_CONTRACT.md、BACKEND_HANDOFF.md 和本提示词，并记录准确的 Windows/Linux 测试与构建结果。

常规代码修改和本地测试不需要再次询问。涉及真实系统服务、驱动、网卡、远程网关或凭据时，先核实环境和授权；没有条件就准确记录阻塞，不伪造完成结果。
```
