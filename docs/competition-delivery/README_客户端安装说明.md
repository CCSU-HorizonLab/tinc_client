# Windows 客户端安装说明

## 状态提示

当前安装包是 `v1.0-competition-2026-rc1` 验收候选包，尚未完成干净 Windows 24 项全流程验收，不是最终发行版。

## 环境

- Windows 10/11 x64。
- 安装操作需要管理员权限，用于安装 TAP-Windows 驱动、创建专用网卡及注册 Windows 服务。
- 安装包包含 Tinc 1.0.36、Rust Agent 和已验证签名的 TAP-Windows 9.24.7 驱动安装器。
- 桌面界面依赖 Microsoft Edge WebView2 Runtime；现代 Windows 10/11 通常已提供。

## 安装与连接

1. 校验安装包 SHA-256 与 `SHA256.txt` 一致。
2. 右键以管理员身份运行安装程序。安装包本身尚未代码签名，SmartScreen 提示属于已知限制；只能从本交付目录取得并先核对哈希。
3. 安装器创建 `TincClientAgent` 自动服务、专用 TAP 网卡，并写入 `%ProgramData%\TincClient\settings.json`。
4. 启动 Tinc Connect，输入服务端地址、节点名和节点密码。比赛测试服务默认地址格式为 `http://服务器地址:83`，不应省略端口。
5. 依次执行登录、配置与启动。只有界面显示“运行中 + 已验证连通”时，才代表 ICMP 探测已成功；“进程运行中”本身不等于隧道可达。

## 诊断

- 安装日志：`%ProgramData%\TincClient\installer.log`。
- Agent 配置：`%ProgramData%\TincClient\settings.json`。
- 服务状态：Windows 服务管理器中的 `TincClientAgent`。
- 期望路由：网关主机地址对应的 `/32` 精确路由，例如 `10.0.11.1/32`，出口为专用 TAP 网卡。
- 服务端公网必须同时允许 Tinc 的 TCP 和 UDP 端口；当前测试网关使用 600。

## 关闭与卸载

- 关闭桌面窗口不会主动停止 Agent 服务或已运行的 VPN。
- 使用界面的“停止 VPN”只清理由本项目创建并记录的地址和精确路由。
- 卸载会移除桌面程序、Agent 服务和安装运行时；用户网络数据默认保留在 `%ProgramData%\TincClient`，用于防止误删密钥和配置。

## 已知限制

- 干净 Windows 首装、卸载、重启、重装：未验证。
- 安装包代码签名：未完成。
- Linux 实机发行包与 Android 客户端：未完成。
