# TincLink 测试报告

测试时间：2026-09-17（Asia/Shanghai）  
候选版本：`v1.0-competition-2026-rc1`

## 自动化结果

| 范围 | 结果 | 说明 |
|---|---:|---|
| 桌面 UI | 7/7 通过 | Vitest |
| Rust 工作区 | 30/30 通过 | 原基线 28 项；本次新增 2 项 `settings.json/owner` 安装契约测试 |
| Rust 严格检查 | 通过 | `cargo clippy --workspace --all-targets -- -D warnings` |
| 生产 IPC | 1/1 通过 | 对当前运行的 `TincClientAgent` 完成身份认证 |
| Windows 打包预检 | 通过 | 校验依赖、钩子、配置文件名和字段契约 |
| Java 服务端 | 17 项；0 Failure；0 Error；1 Skip | Maven reactor `BUILD SUCCESS` |
| Web 管理端 | 构建成功 | 有资源体积警告，无构建失败 |

## 实机与网络证据

- 历史开发机联调曾取得：`10.0.11.1/32` 精确路由、Ping 4/4、界面 running + connected。该结论不等于干净机安装验收。
- 2026-09-17 本次重检网关：`tinc@new_network` active/enabled；tinc0=`10.0.11.1/24`；TCP/UDP 600 均监听；firewalld 运行时和永久规则均允许 TCP/UDP 600。
- 2026-09-17 本次客户端重检：Agent 服务 Running/Automatic，但 VPN 未启动，未发现 tincd 与 `10.0.11.1/32` 路由，原生命令 Ping 0/4。因此当前连通状态是**未连接**，不得记录成当前 Ping 4/4。

## 干净 Windows 验收

状态：**未验证 / 未通过最终放行门槛**。

当前主机存在历史服务、TAP 和配置；未发现可立即使用的 Windows Sandbox、Hyper-V 虚拟机或其他干净 Windows 10/11 环境。因而安装、卸载、重启、重装等 24 项流程尚无独立证据。验收完成前，安装包名称和清单必须保留 `rc1`/候选标记。

## 未执行

- 正式性能和压力测试：未执行。
- 两个 Windows 客户端互联：未验证。
- Linux 实机安装交付：未完成。
- Android：未实现。
- Windows 安装包代码签名：未完成。
