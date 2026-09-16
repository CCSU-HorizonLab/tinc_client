# 旧客户端接口契约与阻塞

分析日期：2026-09-07。只读来源：`D:/Codes/Java/KenDeJi_RuoYi/tinc_cli_gui/windows/demo_win/code_win`。以下以 C++ 实际执行代码为准，旧 README 中 `/login`、`/downloadConfig`、旧 PHP 上传路径及 username/code 字段已经过时。

## 已确认接口

所有调用都是 JSON POST，Content-Type 为 application/json。旧程序没有 Authorization header，也没有 cookie 会话管理代码。

| 操作 | 路径 | 请求 | 旧代码读取的响应 | 依据 |
|---|---|---|---|---|
| 登录 | `/api/tinc/client/login` | `sid`, `password` | 顶层 `status`, `msg`, `sid`, `token`, `net_name`, `node_ip`；status=0 表示拒绝 | `LoginDialog/logindialog.cpp:290–334,353–403` |
| 下载 | `/api/tinc/client/config/download` | `sid`, `token` | 二进制 ZIP，未检查 Content-Type/业务 JSON | `LoginDialog/download.cpp:195–225,245–286` |
| 公钥上传 | `/api/tinc/client/key/upload` | `sid`, `content`, `action:"exchangeFile"` | 纯文本 server_master host 配置 | `conf_package/conf_package.cpp:240–341` |
| 结果上报 | `/api/tinc/client/status/update` | `type:"add"`, `result:"success"/"fail"`, `ids`, `details` | 未解析结果，finished 就显示完成 | `LoginDialog/download.cpp:544–598` |
| 后台心跳（遗留） | `/XVntQFJCjc.php/promin/Api/keepalive` | `Heart:"Heart"`, `sid` | 文本 `1` 正常、`-1` 重试、`2` 断开通知；其他按 `Subnet：值` / `Name：值` 解析 | `Daemons/DaemonServer.cpp:75–78,292–307,365–550` |
| 修改结果（遗留） | `/api/tinc/client/status/update` | `type:"edit"`, `result`, `ids`, `details`，成功时还有 `key`,`value` | 未可靠验证 | `Daemons/DaemonServer.cpp:310–361` |

## 2026-09-09 已确认的公钥交换与运行契约

后端交接文件为 `D:/Codes/Java/KenDeJi_RuoYi/RuoYi-Vue-master/docs/TINC_CLIENT_KEY_EXCHANGE_RESPONSE.md`。新客户端已按其实现以下上传请求，不能回退到旧客户端只传 `sid` 的行为：

```json
{
  "sid": "<登录响应 sid>",
  "token": "<登录响应 token>",
  "action": "exchangeFile",
  "content": "Subnet = <登录响应 node_ip>/32\\n\\n<2048–8192 位 PKCS#1 RSA 公钥>"
}
```

- 上传 HTTP 200 必须是 `text/plain`（允许 UTF-8 charset）且正文为严格校验后的 `hosts/server_master`；客户端拒绝 JSON 或缺失 Content-Type 的伪成功响应。
- 客户端对 `server_master` 强制要求唯一 Address、唯一 `/32` Subnet、至多一个 Port、2048–8192 位 PKCS#1 RSA 公钥且 PEM 后无额外内容；下载 ZIP、上传响应和最终落盘使用同一严格校验。客户端自身 host 只允许与登录 `node_ip` 精确一致的单个 `/32` Subnet 和公钥。
- HTTP 401/403（包括 `CLIENT_TOKEN_INVALID`）清除内存会话并返回 `auth_expired`；上传重试复用同一把私钥与完全相同的公钥内容。
- 客户端只映射已确认的后端业务码，分别区分公钥拒绝、会话服务不可用、网关不可用、网关写入/重载失败和节点关联缺失；只使用 `code`，不显示或记录后端 `msg`。未知响应仍退回脱敏 HTTP 状态错误。
- 后端已验证相同公钥上传幂等；变化的公钥只有在远程 SFTP 原子写入并 `tincd -n <netName> -kHUP` 成功后才提交本地副本。远程失败返回 502，旧 host 保留。
- `node_ip` 和上传 Subnet 均固定为 `/32`，没有独立前缀字段；新创建的开发与安装设置显式写入 `interface_prefix: 32`。旧设置可为兼容读取其已有默认值。
- 停止、启动中失败、Tinc 异常退出和服务正常关闭会清理本次 `/32` 地址；Windows 保留 TAP，Linux 额外将接口置 down。成功设置地址后会原子保存受保护的恢复记录；Agent 重启只在接口、前缀、地址、generation 和 profile 与当前管理员设置及有效配置完全匹配时恢复清理，不匹配时返回 `adapter_state_invalid` 而不猜测。命令构造和记录校验已回归，真实网卡仍待验收。
- `status/update` 仍是匿名遗留接口，不能启用；`ids` 虽为数据库节点主键，但登录响应未提供且接口没有 sid/Token 鉴权。

早期交接时网关远程 SSH 不可达，3 个旧网关尚未从 `switch` 迁移为 `router`。2026-09-13 本机配置已经取得 `Mode = router` 的测试网络和严格校验的 `server_master`（公网 TCP/600、VPN `/32`）；21:16 后复测公网 TCP/600 可达。服务端 `/32` 现作为客户端唯一可证明的连通探测目标，但网关 Tinc 版本、UDP/600、主机防火墙及云安全组仍需服务器权限核验。

新客户端保持实际请求字段，不擅自添加 Bearer、替换 sid/ids 或发明审批接口。密码保留空格；旧客户端 `remove(QChar(' '))` 会改变密码。

## 登录和网络关联

节点名即 sid。登录响应给出 net_name/node_ip，客户端没有创建网络、创建客户节点或绑定网络的调用。必须由后台预先完成关联。未找到注销、刷新 token、有效期、验证码或登录状态查询接口。新实现本地 logout 仅清除服务内存会话，不宣称服务端撤销成功。HTTP 401/403 清会话；HTTP 200 中其它自定义失效码尚未有契约，返回协议错误，不猜测。

旧代码只检查 status=0，不能据此确定所有非零 status 都成功。新实现要求正整数 status 和完整身份字段，再检查 sid 一致、名称合法、IPv4 合法；仍需真实后端确认成功 status 的精确取值。

## 实际调用顺序

`登录 → 下载 ZIP → 解压 → 生成 RSA → 上传公钥 → 接收 hosts/server_master → 查询 tinc 服务 → 结果上报 → 安装/启动服务 → Daemons`。

新实现保留下载在上传之前的顺序，下载后先在内存校验，再生成/复用密钥、上传、校验 server_master，最后落地并切换版本。没有凭空改成“审批轮询后下载”。每次显式重试重新下载，复用按服务器/网络/sid 保存的密钥。公钥重复上传是否幂等未由后端证实；客户端能保证内容一致，不能保证服务端事务幂等。

## 密钥与版本证据

`conf_package.cpp:103–217` 调用 `tincd.exe -n <net> -K`，输入两个换行，等待 15 秒；使用 `rsa_key.priv` 和 `hosts/<sid>`，上传内容含 `Subnet = node_ip/32` 和 RSA PUBLIC KEY PEM。旧程序对空 IP/127.0.0.1 可能硬编码为 12.12.12.12。

构造函数已经生成密钥，`conf_package/main.cpp` 结尾再次 `w.generate_key()`，每次先删除密钥，造成重复生成及上传竞争。新客户端用操作系统随机源生成兼容 PKCS#1 的 RSA 3072 位密钥，私钥不交给 UI。密钥工作进程有 60 秒上限，失败不覆盖有效密钥。

指定旧源码目录内未找到可执行的 tincd，也没有锁定其版本的构建文件。随后在旧发布目录确认 `tincd.exe --version` 为 `tinc version 1.0.36`，SHA-256 为 `746999BAB0FE3839304A831A4BA021C190D8F20AF554ECC39D3B122DE3A8C21C`。当前运行适配仍只支持 1.0.x 的 RSA 配置，独立安装脚本进一步要求经过审核的 1.0.36；1.1 的 Ed25519 和控制 socket 尚未实现，不能宣称兼容。网关实际运行版本仍需后端确认。

## 配置结构、路径和系统行为

旧程序寻找包含 Tinc/tincd.exe 或 serverIp.conf 的祖先目录，登录写明文 private.txt（sid/token/net_name/server_ip/node_ip），ZIP 存到 `Tinc/configure.zip`，PowerShell Expand-Archive -Force 解压到 Tinc。后续路径推断包内应有 `<net_name>/tinc.conf`、`<net_name>/hosts/<sid>` 等；上传响应单独写 `hosts/server_master`。

2026-09-08 后端已将下载响应收敛为 `<net_name>/tinc.conf` 和 `<net_name>/hosts/server_master` 两个文件，并提供不含 `Interface` 的 721 字节脱敏 ZIP。客户端真实校验器已接受并成功应用该样本；样本 SHA-256 为 `EDFEE81D64348D8E889DA22FE1B3FD7B57D8FD7F8913DEE1D4DF0CD80F5858FF`。

后端随后重新构建并重启服务，使用测试账号实际取得 HTTP 200、`application/zip` 响应。真实响应为 1027 字节，SHA-256 为 `4A1C2E1BD43A0C8B8F874239C7A5CF2DBD822214321C76F4AAD64E4354200FB5`，也只有上述两个文件且不含 `Interface`。凭据和真实公钥正文没有写入客户端文档。尚未完成网关从 `switch` 到 `router` 的迁移及真实隧道握手。

客户端下载成功响应同时要求 `Content-Type: application/zip` 和有效 ZIP 魔数；即使正文以 `PK` 开头，错误媒体类型也不会被当作配置包接受。

新客户端接受根目录或一层 net_name 包裹的 tinc.conf/hosts 文件，不接受其它文件、链接、脚本、私钥。tinc.conf 允许 Name、ConnectTo、Interface（替换为管理员指定网卡）、Mode=router、AddressFamily=ipv4/any。host 允许 Address、Port、Subnet 与 RSA PEM。这是有意的兼容边界；遇到不支持的 ZIP 明确失败并保留旧配置。

旧 Windows 依赖 tap0901，安装器调用 `tapinstall.exe install OemVista.inf tap0901`（`LoginDialog/envcheck.cpp`）。VPN 使用 `tincd -n <net> --install`、`net start tinc.<net>`；按 sc 输出文字判断状态。新服务直接托管 `tincd -D -c <受控配置目录>`，系统服务管理的是 vpn-agent。

旧 Daemons 每秒心跳，每 10 秒 ping；网关把 node_ip 最后一段替换为 .0，未证明是有效网关；ping 输出不含“不可达”就当成功。后台更名/改 Subnet 会删密钥、改文件并重新配置。这些远程控制未迁移：新心跳接口在旧代码里明确 TODO，缺少鉴权和版本化控制契约。

## 问题归属与明确阻塞

| 类别 | 具体问题 | 新客户端处理 |
|---|---|---|
| 客户端缺陷 | 重复密钥生成、删除有效私钥、超时继续、上传失败仍进完成页面 | 稳定密钥、互斥操作、真实错误、事务切换 |
| 客户端缺陷 | token 明文文件/日志/命令行、路径猜测、无约束解压、以 ping 文本当连通 | 内存会话、固定受保护根目录、白名单、连通未知 |
| 后端协议限制 | 未找到审批查询/确认状态，下载拒绝的具体业务码未定义 | 显示“后台确认无法查询”，不自动轮询未知端点 |
| 后端协议限制 | 上报 ids 来自 private.txt 的 id，但登录写入没有 id；响应未定义 | 暂不发送 status/update，避免空 ids 或假 success；需后台明确 id 和成功/失败响应 |
| 后端协议限制 | 遗留上传无 Token；新 `/key/upload` 已明确 sid + Token 绑定、错误码和幂等行为，但网关 SSH 不可达 | 新客户端按新契约上传；不接入遗留远程控制，等待真实网关重载与握手验证 |
| 环境 | 无测试服务地址/账号、网关 tincd 版本、TAP、安装授权 | 可做本地契约测试；真实连接联调未完成 |

后端需一次性补充：测试根地址、已绑定节点账号（安全方式提供）、成功/拒绝/待审批/会话失效样例、ZIP 样本、ids 的来源与状态上报响应、当前 Tinc 版本、网卡及路由/连通探测要求。凭据不得写入本文档或测试快照。

配置下载 ZIP 的双方联调步骤、当前白名单和后端回复格式见 [客户端与后端配置包联调单](BACKEND_HANDOFF.md)。
