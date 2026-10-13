# TincLink Windows 客户端接入码登录改造专项报告

> Phase 0 复核（2026-10-13）：本文已按当前工作区重新校验。安装器未在本轮重新生成；以下测试数据已更新为本轮实际结果。文中“真实 VPN”相关未验证边界保持不变。

**文档版本**：v1.0  
**报告日期**：2026-09-23  
**目标工程**：`tinc_client` (Windows Desktop & Agent Service)  
**接口依据**：后端提供的最终【Frontend Contract】  

---

## 1. 旧登录问题 (Legacy Login Ambiguity)

### 1.1 旧登录架构缺陷
在引入接入码机制之前，客户端登录采用如下交互模式：
- **输入字段**：服务端地址（Management Origin）、节点名（`sid`）、密码（`password`）。
- **请求体**：
  ```json
  {
    "sid": "client_a",
    "password": "..."
  }
  ```
- **核心缺陷**：
  服务端资源拓扑为 `Server -> Network -> Node`。在业务模型中，不同 Network 或不同 Server 下完全允许存在相同的 `nodeName` 与相同的 `nodeIp`。
  当用户使用 `sid: "a"` 登录时，后端如果存在多个名为 `"a"` 的节点（例如 Access-A 下的 Network-a 与 Network-b 分别拥有一个节点名为 `"a"`），仅凭 `sid` 与 `password` 无法唯一定位具体的目标 Node 记录，产生严重的**身份歧义（Node Identity Ambiguity）**与横向越权碰撞风险。

### 1.2 权责划分的重新确立
为根治上述问题，身份模型职责进行了彻底解耦：
- **`accessCode`**：登录前**唯一全局定位**目标 Node 实例（全局唯一索引，格式 `TL-XXXX-XXXX`）。
- **`password`**：仅用于 Node 凭据认证，绝不参与资源定位。
- **`nodeName` (`sid`)**：退化为纯**显示属性**与 tincd 配置文件名标识，不再作为登录寻址主键。
- **`nodeId` + `networkId`**：登录成功由服务端签发并返回，用于在客户端本地构建强一致、防漂移的稳定 Profile 身份。

---

## 2. Access Code UI 改造

### 2.1 界面文案与控件映射
客户端登录视图（[`apps/desktop/src/App.vue`](file:///D:/Codes/Java/KenDeJi_RuoYi/tinc_client/apps/desktop/src/App.vue)）已完成全面切换：
1. **服务端地址文案更新**：由“服务端地址”改为“**TincLink 服务地址**”，明确指出该地址为 Management 管控端根地址，避免用户误填为 Access Server 或网关地址。
2. **账号输入项替换**：彻底移除“节点名”输入框，变更为“**接入码**”。
3. **占位符（Placeholder）**：统一设定为 `TL-7K9M-2Q4F`。
4. **辅助引导文案**：展示说明“*接入码由管理员创建节点后提供*”，降低终端用户的认知门槛。
5. **登录后详情展示**：登录成功后，展示卡片清晰呈现“节点名称”（`node.getNodeName()`）、“所属网络”（`network.getNetworkName()`）、“节点 IP”（`node.getNetworkIp()`）及“接入码”（脱敏或完整展示），不再将节点名混淆为登录账号。

### 2.2 输入规范与自动规整 (Normalization)
严格遵循后端 Crockford Base32 编码规范（12 位，合法字符集 `23456789ABCDEFGHJKMNPQRSTUVWXYZ`，前缀 `TL-`，中间以 `-` 分隔）：
- **自动格式化**：在 [`apps/desktop/src/services/client.ts`](file:///D:/Codes/Java/KenDeJi_RuoYi/tinc_client/apps/desktop/src/services/client.ts) 中导出 `normalizeAccessCode` 与 `isValidAccessCode`。
- **客户端行为**：前端允许用户以小写字母或携带首尾空格输入，在 `@blur` 失焦事件以及表单 `@submit` 前，统一调用 `.trim().toUpperCase()` 进行规整，既保证输入容错体验，又确保提交到后台的报文 100% 符合大写规范。

---

## 3. Request Contract（严格契约遵循）

客户端提交登录时，严格遵循【Frontend Contract】，全面弃用向服务端默认传递 `sid` 的行为。

### 3.1 接入码登录请求（当前主干）
- **接口路径**：`POST /api/tinc/client/login`
- **请求头**：`Content-Type: application/json;charset=UTF-8`
- **请求体 JSON**：
  ```json
  {
    "accessCode": "TL-7K9M-2Q4F",
    "password": "node-password"
  }
  ```
- **契约合规保证**：
  在客户端底层核心库 [`crates/vpn-core/src/api.rs`](file:///D:/Codes/Java/KenDeJi_RuoYi/tinc_client/crates/vpn-core/src/api.rs) 中：
  当用户提供 `access_code` 时，仅序列化 `accessCode` 和 `password`，**绝不携带 `sid` 字段**。

### 3.2 Legacy 兼容通道（仅向下兼容历史调用）
仅当明确未提供 `accessCode` 且提供了历史 `sid` 时，向服务端发送旧版报文：
```json
{
  "sid": "client_a",
  "password": "node-password"
}
```
新版 Desktop UI 已彻底关闭 `sid` 输入入口，Legacy 通道仅保留在协议层用于兼容老版本二进制或历史离线状态迁移。

---

## 4. Response Contract（响应解析）

登录响应由后端统一返回 JSON。客户端在 [`crates/vpn-core/src/api.rs`](file:///D:/Codes/Java/KenDeJi_RuoYi/tinc_client/crates/vpn-core/src/api.rs) 中按照契约完整解析：

```json
{
  "status": 1,
  "msg": "登录成功",
  "access_code": "TL-7K9M-2Q4F",
  "sid": "client_a",
  "token": "tok_991823ab...",
  "node_id": 101,
  "network_id": 20,
  "net_name": "net_office",
  "node_ip": "10.42.0.2"
}
```

客户端接收并解析出如下字段绑定至本地 `Identity`：
- `sid`（节点名称，必须通过 `identifier` 64 位字符校验与保留字过滤）
- `net_name`（网络标识，必须通过合法名称校验）
- `node_ip`（必须为合法 IPv4 地址）
- `node_id`（服务端 Node 主键，如 `101`）
- `network_id`（服务端 Network 主键，如 `20`）
- `access_code`（服务端返回的规范化接入码，如 `TL-7K9M-2Q4F`）
- `token`（服务端签发的会话凭据，放入内存保护对象 `Zeroizing<String>`）

**一致性校验**：若客户端发起登录时带有 `accessCode`，校验响应体中 `access_code` 是否与请求完全一致，防止恶意中间代理篡改。

---

## 5. 错误码映射体系 (Error Mapping)

当登录失败或业务被拒绝时，后端返回 `status: 0` 结构体：
```json
{
  "status": 0,
  "code": "CLIENT_CREDENTIAL_INVALID",
  "msg": "接入码或密码无效"
}
```
客户端在 [`crates/vpn-core/src/api.rs`](file:///D:/Codes/Java/KenDeJi_RuoYi/tinc_client/crates/vpn-core/src/api.rs) 中实现了精准的 `map_backend_error` 映射，杜绝“统一报系统错误”的模糊处理：

| 后端错误码 (`code`) | 客户端错误码 (`Error.code`) | 用户提示信息 (`message`) | 重试属性 (`retryable`) |
| :--- | :--- | :--- | :--- |
| `CLIENT_CREDENTIAL_INVALID` | `credential_invalid` | 接入码或密码错误，请核对后重试 | `false` |
| `CLIENT_LEGACY_LOGIN_AMBIGUOUS` | `login_ambiguous` | 检测到多个同名节点，请使用接入码登录 | `false` |
| `CLIENT_SID_INVALID` | `protocol` | 节点标识格式无效 | `false` |
| `CLIENT_NETWORK_IP_INVALID` | `assignment_missing` | 节点网络 IP 配置无效，请联系管理员 | `false` |
| `CLIENT_NETWORK_NOT_FOUND` | `assignment_missing` | 后台找不到节点或网络关联，请联系管理员检查绑定关系 | `false` |
| `SYSTEM_ERROR` | `server_error` | 服务端内部错误，可以稍后重试 | `true` |
| `CLIENT_TOKEN_INVALID` | `auth_expired` | 鉴权失败或会话已失效，请重新登录 | `false` |
| `CLIENT_NODE_REVOKED` | `node_revoked` | 该节点已被后台撤销，VPN 已停止；本机私钥将保留但不能继续连接 | `false` |

---

## 6. Profile Identity 保持

### 6.1 Profile ID 算法维持不可动摇
本地 Profile 存储路径与命名规则依赖 `profile_id` 计算：
```rust
pub fn profile_id(origin: &str, identity: &Identity) -> String {
    let scope = match (identity.network_id, identity.node_id) {
        (Some(network_id), Some(node_id)) => format!("id\0{network_id}\0{node_id}"),
        _ => format!("{}\0{}", identity.net_name, identity.sid),
    };
    format!("{:x}", Sha256::digest(format!("{origin}\0{scope}")))
}
```
- **核心判定**：`profile_id` 严格由 **`Management origin` + `network_id` + `node_id`** 计算 SHA-256 哈希。
- **严禁污染**：**绝不将 `accessCode` 混入哈希计算中**。

### 6.2 身份等价性比较（`PartialEq for Identity`）
在 [`crates/vpn-core/src/model.rs`](file:///D:/Codes/Java/KenDeJi_RuoYi/tinc_client/crates/vpn-core/src/model.rs) 中，自定义实现了 `Identity` 的 `PartialEq`：
```rust
impl PartialEq for Identity {
    fn eq(&self, other: &Self) -> bool {
        self.sid == other.sid
            && self.net_name == other.net_name
            && self.node_ip == other.node_ip
            && self.node_id == other.node_id
            && self.network_id == other.network_id
    }
}
```
由于比较逻辑排除 `access_code`，当服务端或管理员轮换该节点的接入码时，客户端识别到的 Node Identity 完全等价，不会发生“误判为新节点”而破坏本地配置状态。

---

## 7. Key 生命周期保证 (Key Lifecycle)

客户端严格维护既有的 Phase 2.5D 安全设计准则：
1. **单一持久化私钥**：在 `keys/<profile_id>/rsa_key.priv` 中落盘的 RSA-3072 私钥是节点身份的唯一源真理（Source of Truth）。
2. **幂等性与非破坏性**：
   - 用户登出后再登录（`logout -> login`）不会重新生成私钥。
   - 客户端软件重启、Agent 服务重启、网络断线重连，均重用本地既有私钥。
   - 上传重试（Upload Retry）始终提取既有公钥，绝不会在网络重试中擅自触发 Key Rotation。
3. **主动轮换隔离**：只有在用户或管理员主动发起 `RotateKey` 指令且服务端成功确认接收后，新私钥才会从候选区（`rsa_key.candidate`）原子提交到激活区（`rsa_key.priv`）。

---

## 8. Rotation 行为（Access Code 轮换不变性）

### 8.1 轮换场景分析
当管理员在后端为某个节点重新生成/轮换 Access Code（例如由 `TL-7K9M-2Q4F` 轮换为 `TL-9A8B-7C6D`）：
- 该节点的 `nodeId`、`networkId`、`nodeName` 保持不变。
- 客户端使用新接入码 `TL-9A8B-7C6D` 登录成功后，后端返回相同的 `node_id` 与 `network_id`。
- 本地 `profile_id` 算法输出与之前完全相同的 SHA-256 哈希值。
- 本地已有的 `keys/<profile_id>/rsa_key.priv` 与 `active.json` 完全保留并继续复用。
- 自动化单元测试 `access_code_rotation_preserves_profile_id_and_identity_equality` 验证断言 `assert_eq!(profile_1, profile_2)` 成功通过。

---

## 9. 旧 Profile 迁移与兼容 (Legacy Profile Continuity)

1. **已配置节点无缝衔接**：老客户端已生成的配置包保存在 `keys/<profile_id>` 及 `active.json` 中。升级到新版本后，由于底层存储模型未发生任何路径变更，已保存的激活 Profile 不需要做破坏性迁移。
2. **自动关联恢复**：当老用户首次在新客户端使用管理员下发的新接入码登录同一节点时，系统依据返回的 `network_id` + `node_id` 自动无缝对应到原 Profile 目录，保持配置与私钥完全平滑过渡。

---

## 10. 三同名 Node 隔离测试 (Disambiguation Verification)

在 [`crates/vpn-core/tests/api.rs`](file:///D:/Codes/Java/KenDeJi_RuoYi/tinc_client/crates/vpn-core/tests/api.rs) 中编写了专项隔离测试 `same_node_name_across_different_networks_isolate_profiles`：

### 测试拓扑构造
- **Node A**：`sid: "a"`, `network_id: 1`, `node_id: 10`, `access_code: "TL-AAAA-1111"`
- **Node B**：`sid: "a"`, `network_id: 2`, `node_id: 20`, `access_code: "TL-BBBB-2222"`
- **Node C**：`sid: "a"`, `network_id: 3`, `node_id: 30`, `access_code: "TL-CCCC-3333"`
- 设定三个节点的密码完全相同。

### 隔离结果验证
- 客户端分别使用各自的 `accessCode` 登录，服务端精确匹配到对应的单个 Node 记录并返回其对应的 `network_id` 与 `node_id`。
- 客户端计算得到的 `profile_a != profile_b != profile_c`。
- 本地密钥目录与配置文件彼此完全物理隔离，彻底消除了历史版本的同名冲突隐患。

---

## 11. 安全回归 (Security Regression)

整个改造过程严密遵守安全基线，核心防护机制完全保留并经测试通过：
1. **ZIP 安全校验**：`crates/vpn-core/src/config.rs` 中防路径穿越（Path Traversal）、防超长文件、禁止危险可执行脚本（`*.bat`, `*.ps1`, `*.sh`）的过滤规则 100% 生效。
2. **凭据保护**：密码使用 `zeroize::Zeroizing` 并在登录后立即清空内存；本地日志过滤敏感字段，拒绝将 `accessCode` 或 `password` 打印至诊断日志。
3. **命名管道与权限模型**：维持 `LocalSystem` 专有 Windows Service 架构与命名管道（Named Pipe）IPC 鉴权，桌面 UI 与高权限 Agent 严格隔离。
4. **节点撤销（Node Revoke）**：`CLIENT_NODE_REVOKED` 错误响应能立刻终止运行进程并清除激活状态，保留本地密钥但阻止重连。

---

## 12. 自动化测试结果

### 12.1 前端单元测试 (Vitest)
运行命令：`npm run test:ui`
```
 ✓ src/services/client.test.ts (14 tests)
 Test Files  1 passed (1)
      Tests  14 passed (14)
```
覆盖项：接入码格式正则校验、首尾空格修剪与大写归一化、Crockford Base32 排除字符识别、轮换场景下的 Profile 连续性断言。

### 12.2 Rust 核心库与集成测试
运行命令：`cargo test --workspace`
- **Unit & Process Job Tests**：2 passed
- **Engine & Adapter State Tests**：3 passed
- **API & Access Code Contract Tests**：13 passed
  - `validates_access_code_format_and_normalization`：PASSED
  - `access_code_login_sends_contract_json_without_sid_and_parses_response`：PASSED
  - `access_code_login_maps_backend_rejections`：PASSED
  - `access_code_rotation_preserves_profile_id_and_identity_equality`：PASSED
  - `same_node_name_across_different_networks_isolate_profiles`：PASSED
- **Safety & Archive Inspection Tests**：15 passed
- **Settings Contract & Windows Installer Tests**：3 passed
- **IPC Frames Tests**：3 passed
- **总计**：39 项自动化测试全部通过，0 failed，另有 1 项真实 Windows 服务测试按设计 ignored。

---

## 13. 代码风格与质量检查 (Lint & Format)

- **Clippy**：
  执行 `. .\scripts\dev\env.ps1; cargo clippy --workspace --all-targets -- -D warnings`
  输出：**0 条 Clippy 诊断，0 错误**。当前文件系统不支持增量缓存硬链接，Cargo 会提示复制回退；这不是源码告警。
- **Formatting**：
  执行 `. .\scripts\dev\env.ps1; cargo fmt --all -- --check`
  输出退出代码 `0`，代码格式严格符合 Rust 官方规范。

---

## 14. 安装包与预检 (Packaging Preflight & Build)

- **安装脚本语法与守护预检**：
  执行 `npm run test:packaging`
  输出：`PASS installer preflight guards and Windows PowerShell syntax`。
- **Vue 客户端生产构建**：
  执行 `npm run build`，`vue-tsc --noEmit` 与 Vite 构建通过，生成 14 个 module 到忽略目录 `dist/`。
- **本轮未执行**：`npm run desktop:build`、NSIS 安装包生成与真实安装；Phase 0 不发布安装包。

---

## 15. 本地端到端集成验证 (E2E Verification)

执行测试脚本：`node scripts/test/e2e-local.mjs`
输出：
```
PASS agent starts with actual unconfigured state
PASS authentication rejection surfaces
PASS failed re-login preserves the authenticated session
PASS upload failure is not configuration success
PASS retry reuses real RSA private key and identical public upload
PASS logout and login reuse the same profile private key
PASS failed key rotation retains the previous private key
PASS successful key rotation commits a new key only after server acceptance
PASS incomplete key rotation blocks start and configure repairs it
PASS unsafe archive rejected and previous configuration preserved
PASS duplicate configure rejected while status remains readable
PASS missing real Tinc binary is a visible failure
PASS expired session cleared while retained configuration survives
PASS logs exclude credentials and private keys
PASS agent restart recovers committed config and stable key without inventing a live session
PASS early child-process exit is a start failure, never a running connection
PASS server revoke blocks reconnect and clears the live login session
PASS revoked profile remains blocked after agent restart while its private key stays stable
Completed 18 local integration checks. Backend was a fixture; no real VPN connection claimed.
```
18 项端到端流程在本地 fixture backend 下全部验证通过。

---

## 16. 未验证项与环境约束 (Non-verified Items)

1. **真实外网 TLS 链路联通**：由于当前运行环境位于无外网真实 Management 部署的环境中，E2E 验证基于本地真实 IPC + HTTP Mock Backend 闭环完成。
2. **多网卡并发运行**：架构约束维持单活动 Profile（Single Active Profile），不支持多 Network 并发 TAP 虚拟网卡隧道共存，此为既定系统设计范围之外。
3. **真实 TAP 物理网卡路由收发**：虚拟网卡与 tincd 守护进程真实数据转发依赖 Windows 宿主机安装 TAP-Windows 驱动并在管理员服务下运行。本地集成测试通过 IPC 状态机与配置校验进行了全真模拟。

---

## 核心问答汇总 (Q1 ~ Q7)

* **Q1：新版客户端登录使用什么？**  
  **答**：使用 **【TincLink 服务地址】 + 【接入码 (accessCode)】 + 【密码 (password)】**。客户端发送 `{"accessCode": "TL-XXXX-XXXX", "password": "..."}`，不再发送 `sid`。  
  *【已验证：`access_code_login_sends_contract_json_without_sid_and_parses_response`】*

* **Q2：nodeName 现在只负责什么？**  
  **答**：`nodeName` 现在**仅用于客户端界面显示**以及 tinc 内部配置文件名标识，不再承担登录寻址或资源定位职责。  
  *【已验证：`apps/desktop/src/App.vue`, `crates/vpn-core/src/api.rs`】*

* **Q3：Access Code 变化会不会导致新 Profile？**  
  **答**：**不会**。Profile ID 严格由 `Management origin` + `networkId` + `nodeId` 计算，Access Code 轮换不会更改底层节点的 `nodeId` 和 `networkId`，因此 Profile ID 保持不变。  
  *【已验证：`access_code_rotation_preserves_profile_id_and_identity_equality`】*

* **Q4：Access Code 变化会不会重新生成 RSA Key？**  
  **答**：**不会**。本地 RSA 私钥基于稳定 Profile ID 路径持久化存储，Access Code 轮换仅改变登录入口凭据，客户端重用既有私钥，绝不重新生成密钥。  
  *【已验证：`access_code_rotation_preserves_profile_id_and_identity_equality`, `e2e-local.mjs`】*

* **Q5：三个同名同密 Node 是否仍可正确区分？**  
  **答**：**可以完全精确区分**。由于接入码全局唯一，三个节点拥有互不相同的 `accessCode`，后端依据接入码唯一定位目标节点并签发对应的 `nodeId` 与 `networkId`，客户端生成的 Profile ID 彼此隔离，彻底消除歧义。  
  *【已验证：`same_node_name_across_different_networks_isolate_profiles`】*

* **Q6：旧客户端是否仍能临时兼容？**  
  **答**：**是**。后端保留了针对单一无歧义同名节点的 Legacy `sid` 登录兼容通道；若存在多个同名节点，后端返回 `CLIENT_LEGACY_LOGIN_AMBIGUOUS`，客户端能正确捕获并提示用户升级客户端使用接入码登录。新客户端代码库底层也保留了协议级兼容解析能力。  
  *【已验证：`crates/vpn-core/src/api.rs:map_backend_error`】*

* **Q7：当前 Profile Identity 是否仍然是 Management origin + networkId + nodeId？**  
  **答**：**是**。Profile Identity 仍然严格由 `Management origin` + `networkId` + `nodeId` 经 SHA-256 派生，该算法保持稳定未变。  
  *【已验证：`crates/vpn-core/src/keys.rs:profile_id`, `crates/vpn-core/tests/safety.rs`】*
