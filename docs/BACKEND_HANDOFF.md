# 客户端与后端配置包联调单

更新时间：2026-09-08。用途：把本文路径直接交给后端 Agent，双方依据同一份文件和脱敏 ZIP 样本联调。

## 当前问题

真实测试已确认：

1. `POST /api/tinc/client/login` 成功，客户端取得 `sid`、`token`、`net_name` 和 `node_ip`。
2. `POST /api/tinc/client/config/download` 返回的数据能解析为 ZIP。
3. ZIP 至少包含一个客户端当前未允许的条目，因此配置在生成或上传密钥之前被拒绝，旧配置不会被覆盖。

旧客户端会直接解压下载包，无法证明包中每个文件都必须执行或保留。客户端不会为了兼容而直接放开脚本、私钥、可执行文件或任意路径。

客户端更新后，`unsafe_config` 错误会包含具体被拒绝的 ZIP 条目。请重试一次，将完整错误代码和文件名附在后端回复中；不要发送密码、Token、私钥或完整响应日志。

## 客户端当前接受的 ZIP

接受以下两种布局之一，其中 `office` 表示登录响应的 `net_name`：

```text
tinc.conf
hosts/
  peer_a
  peer_b
```

```text
office/
  tinc.conf
  hosts/
    peer_a
    peer_b
```

`hosts/server_master` 可以不在 ZIP 中；客户端在后续公钥上传响应中取得并校验它。`ConnectTo = server_master` 因此允许暂时没有对应 ZIP 文件。

ZIP 限制：

- 下载响应最多 4 MiB，解压后总计最多 2 MiB。
- 最多 128 个条目，每个文件最多 64 KiB，文本必须为 UTF-8。
- 禁止绝对路径、盘符、反斜杠、`..`、`.`、ADS、符号链接以及大小写冲突路径。
- 只允许 `tinc.conf` 和 `hosts/<合法节点名>`；不接受说明文件、嵌套的第二层目录或其他文件。
- 不接受 `rsa_key.priv`、任何私钥、EXE/DLL、`tinc-up/down`、`host-up/down`、`subnet-up/down` 或其他脚本。

`tinc.conf` 当前允许：

```text
Name = <必须与登录 sid 相同>
ConnectTo = <合法节点名，可重复>
Mode = router
AddressFamily = ipv4
Interface = <服务端值会被忽略并替换为管理员配置的网卡名>
```

必须有一个 `Name` 和至少一个 `ConnectTo`。`AddressFamily` 也可为 `any`；`Mode` 如存在只能为 `router`。其他指令需要双方说明实际用途、安全边界和平台行为后再纳入契约。

`hosts/<节点名>` 当前允许 `Address`、`Port`、IPv4/CIDR 格式的 `Subnet` 和 PKCS#1 RSA 公钥。禁止私钥和其他指令。最终应用配置时，每个 host 必须包含 2048–8192 位 RSA 公钥。

## 后端 Agent 必须返回的证据

请在后端仓库创建 `docs/TINC_CLIENT_CONFIG_RESPONSE.md`，回答以下内容：

1. 产生下载 ZIP 的控制器、服务和模板文件路径。
2. 测试账号本次响应的 HTTP 状态、`Content-Type` 和响应字节数；不得记录 Token。
3. ZIP 的完整条目清单：路径、文件/目录类型、解压后大小。不要贴私钥或文件正文。
4. 每个额外条目的生成原因，以及 Tinc 运行是否真正需要它。
5. `tinc.conf` 使用的全部指令名，以及每项的实际用途。
6. `hosts/*` 使用的全部指令名。
7. 是否包含客户端私钥、脚本或可执行文件；如果包含，说明为什么由服务器下发。
8. 提供一个脱敏但结构完全相同的 ZIP 测试样本，保存为后端仓库的 `tests/fixtures/tinc-client-config.zip`，并给出绝对路径和 SHA-256。样本不得包含凭据、Token 或真实私钥。
9. 说明建议由后端删除哪些多余条目，建议客户端新增允许哪些确有必要的配置指令。

客户端 Agent收到后端回复文件路径和样本路径后，将：

1. 对照真实清单确认差异属于多余文件、必要配置还是旧包遗留。
2. 优先让后端停止下发无用和危险文件。
3. 对确有必要的安全 Tinc 指令逐项添加校验，不使用通配放行。
4. 把脱敏 ZIP 固定为回归测试，并验证失败时旧配置仍保留。
5. 更新本文和 `API_CONTRACT.md`，记录最终双方契约。

## 可直接发送给后端 Agent 的提示词

```text
请处理 Tinc 新客户端配置包兼容问题。先阅读：
D:\Codes\Java\KenDeJi_RuoYi\tinc_client\docs\BACKEND_HANDOFF.md

客户端已确认登录和 ZIP 下载成功，但 ZIP 含未允许条目。请不要直接让客户端关闭安全校验，也不要修改客户端项目。请在后端仓库定位 /api/tinc/client/config/download 的 ZIP 生成代码，按照交接单列出实际 ZIP 条目和配置指令，创建 docs/TINC_CLIENT_CONFIG_RESPONSE.md，并生成不含凭据、Token、真实私钥的结构等价测试样本 tests/fixtures/tinc-client-config.zip。回复这两个文件的绝对路径和样本 SHA-256。缺少测试条件时，也要先完成静态代码分析并准确记录阻塞。
```

## 交接回客户端

后端 Agent 完成后，把以下一句连同真实路径发给客户端 Agent：

```text
继续配置包联调。读取后端回复 <绝对路径>/docs/TINC_CLIENT_CONFIG_RESPONSE.md 和样本 <绝对路径>/tests/fixtures/tinc-client-config.zip，对照 tinc_client/docs/BACKEND_HANDOFF.md 修改并测试客户端。
```

## 联调结论

2026-09-08 已读取后端回复
`D:\Codes\Java\KenDeJi_RuoYi\RuoYi-Vue-master\docs\TINC_CLIENT_CONFIG_RESPONSE.md`
并核对脱敏样本。样本 SHA-256 为
`EDFEE81D64348D8E889DA22FE1B3FD7B57D8FD7F8913DEE1D4DF0CD80F5858FF`，共 721 字节，只有：

- `office/tinc.conf`（82 字节）；
- `office/hosts/server_master`（482 字节）。

样本 `tinc.conf` 不含 `Interface`。客户端真实校验器已接受后端提供的原始 ZIP，
自动补入本机管理员配置的受控网卡名，并成功完成版本化配置应用。无需放宽脚本、私钥、
可执行文件或任意路径的安全限制。
等价文本样本已固定在 `crates/vpn-core/tests/fixtures/backend-config/`，回归测试也可通过
`TINC_BACKEND_FIXTURE` 环境变量直接读取后端 ZIP。

后端已经重新构建、重启并使用测试账号完成实际下载：HTTP 200，`Content-Type` 为
`application/zip`，响应 1027 字节，SHA-256 为
`4A1C2E1BD43A0C8B8F874239C7A5CF2DBD822214321C76F4AAD64E4354200FB5`；只有
`<netName>/tinc.conf`（74 字节）和 `<netName>/hosts/server_master`（820 字节），
无显式目录及其他条目。

客户端启动时分别在 Windows 使用 `netsh interface ipv4 set address`，在 Linux 使用
`/usr/sbin/ip address replace` 和 `ip link set ... up` 设置管理员指定的网卡；Tinc 以
`tincd -D -c <受控配置版本目录>` 运行。Windows 服务生命周期由 SCM 适配处理，Linux
由 systemd unit 和信号处理实现，不依赖下载脚本，也不向后端增加 `os` 字段。

剩余联调项是网关迁移：后端核查的 3 份现有网关配置仍为 `Mode = switch`，需由管理员通过
网络编辑或重新生成流程同步为 `router`，推送后重载网关，再做公钥上传和真实隧道握手测试。

客户端平台验证结果：

- Windows：直接读取后端 721 字节脱敏 ZIP，完成校验、管理员网卡名补入、本地密钥生成、
  主机文件写入和配置版本原子切换；Windows Agent 与桌面端构建成功，工作区 17 项测试通过。
- Linux：在 `rust:1.98-bookworm` Linux/amd64 容器中构建 Agent 和 Tauri 桌面端成功；
  使用同一份内置等价样本完成配置应用，Linux 适用的 15 项测试通过。
- 两个平台都由编译目标选择网卡和服务实现，没有新增后端 `os` 字段，没有执行下载脚本，
  ZIP 白名单保持不变。

上述配置应用测试覆盖受控文件生成和版本切换。由于当前没有可用的真实 Tinc 1.0.x、
Windows TAP 网卡、Linux TUN 设备和已迁移网关，本轮没有修改本机网卡，也没有宣称完成
真实隧道握手、Windows 服务安装或 Linux systemd/TUN 运行验收。

2026-09-13 客户端已将严格校验的 `hosts/server_master` 唯一 `/32` 作为精确 active 路由和 ICMP 探测目标；不会从地址猜测 `/24`。本机 21:16 后复测网关公网 TCP/600 可达，但仍请网关管理员确认 TCP/UDP 600 的实际监听、主机防火墙、云安全组及服务端 VPN 地址是否允许 ICMP。客户端只有探测成功才报告 `connection=connected`。

09-09 后端公钥交换契约已确认 `node_ip` 与上传 Subnet 固定为 `/32`；客户端开发初始化和
Windows/Linux 新安装设置已显式写入 `interface_prefix: 32`，Linux 安装脚本也已检查
`/usr/sbin/ip`。客户端已实现停止、启动中失败、Tinc 异常退出及服务正常关闭时移除本次
`/32` 地址；Windows 保留专用 TAP，Linux 随后将本次触碰的接口置 down。命令构造已有单元
测试。09-10 已增加受保护的 `adapter-state.json`：只有接口、精确地址/前缀、generation
和 profile 与当前管理员设置及有效配置完全一致，Agent 重启才恢复清理；不匹配时拒绝自动
操作。该策略和记录校验仍需真实 TAP/TUN 环境共同验收，不能当作生产验收完成。
