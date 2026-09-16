# 平台化演进后期规划报告

> 基线日期：2026-09-15
> 当前决策：先彻底跑通并固化 Tinc 全链路，再开始插件化重构。
> 演进目标：从 Tinc 专属管理工具升级为通用网络节点调度平台，使 Tinc、fnps 等网络工具成为可扩展插件。

## 1. 执行原则

当前已经跑通的 Tinc 链路是未来插件系统的第一个参考实现，不应推倒重写。平台化改造采用“绞杀者模式”：先冻结现有契约并建立通用接口，再把现有 Tinc 实现包入 `plugin-tinc`，同时保留旧 API、数据库和 UI，最后逐步迁移。

在进入插件化阶段前，Tinc 基线必须达到以下门槛：

1. 用户只通过 UI 输入服务端地址、账号和密码，即可完成登录、配置、启动与连通验证。
2. 服务端能自动完成配置生成、网关目标选择、服务启动/重载、防火墙检查和运行状态确认。
3. 不需要运维人员手工停止旧网络、启动 systemd、修复接口、补防火墙规则或上传公钥。
4. 客户端显示“已连接”必须来自真实的数据面探测，不能只依据 tincd 进程存在。
5. 失败时给出可行动的阶段、错误码和诊断信息，并且重试不会破坏原有有效配置。
6. 同一操作可安全重试，客户端、后端和网关状态最终一致。
7. Windows 重启、Agent 重启、Tinc 异常退出后能够恢复或明确提示，不遗留错误地址和路由。

## 2. 当前架构与耦合评估

### 2.1 服务端

当前 Spring Boot 服务端在以下层级与 Tinc 强绑定：

- API 固定为 `/api/tinc/client/*`，直接返回 Tinc ZIP、PEM 公钥和 `server_master`。
- Controller 直接依赖 Tinc Network/Node Service、`TincConfigUtils` 和 `TincRuntimeManager`。
- Network Service 在单个业务事务中完成 RSA 生成、`tinc.conf`、up/down 脚本、host、私钥、远程推送和 tincd 就绪检查。
- Runtime 固定操作 `/etc/tinc`、`tinc@<network>.service`、tincd、Linux 接口、TCP/UDP 监听和 firewalld。
- Transport 同时承担文件传输、Tinc 安装、reload、restart 和删除，职责过宽。
- 数据表和领域对象为 `tinc_network`、`tinc_node`、`tinc_server`，没有 `pluginId`、插件版本、Schema、能力描述和版本化配置载荷。
- 若依管理页面的字段、API、路由和权限均固定为 Tinc。

现有 `TincRuntimeManager` 和 `TincConfigTransport` 是可利用的抽象缝隙，但它们仍是 Tinc 专属接口，不是通用插件边界。

### 2.2 最高优先级架构缺口：运行目标上下文丢失

配置文件推送会携带 `gatewayIp`，但 `reloadGatewayTinc(gatewayIp, netName)` 实际只调用 `runtimeManager.reloadNetwork(netName)`。当前运行管理器读取本机 `/etc/tinc` 并执行本机 systemd。

因此，当 Spring Boot 和网关同机时可以工作；在 SSH 管理另一台网关时，可能出现配置写到远端，但 reload、peer 校验和健康检查发生在后端本机的情况。

未来所有运行操作必须显式携带：

```text
RuntimeTarget {
  targetId,
  transportType,
  endpoint,
  credentialRef,
  operatingSystem,
  executionMode
}
```

不能继续仅凭 `netName` 推断执行位置。

### 2.3 服务端其他问题

- Spring 配置只能全局选择 local 或 SSH，不支持每台目标服务器独立选择执行方式。
- JVM 内锁只能防止单进程并发，未来多实例部署需要分布式租约或数据库操作锁。
- 数据库事务不能回滚已经执行的远程文件、systemd 和 firewall 副作用，需要 Operation、Saga/补偿和 Outbox。
- 网络和服务器主要以名称关联，应迁移为不可变 ID 和正式外键。
- 节点 `password` 曾同时承载登录密码和公钥输入，语义与安全边界混乱。
- 节点登录密码为明文等价比较，应迁移为密码散列或设备凭证。
- SSH 使用跳过 host key 校验的模式，正式集群调度需要可信指纹或 known_hosts。
- 客户端 API 依赖控制器内部 sid/password 和 Redis Token，应演进为有 scope、可撤销、设备绑定的访问令牌。
- 管理端存在新旧重复路由、API 和页面，应在兼容期后统一。

### 2.4 客户端

现代客户端的实际结构为：

```text
Vue 3 + TypeScript
        ↓ Tauri Command
受限 JSON IPC
        ↓
高权限 Rust Agent
        ↓
vpn-core + tincd + TAP/TUN + 路由与探测
```

主要 Tinc 耦合包括：

- IPC 只有 login、configure、start_vpn、stop_vpn、status、logs、logout 七类固定请求。
- API 固定调用 `/api/tinc/client/login`、`config/download` 和 `key/upload`。
- 配置层只接受 `tinc.conf`、`hosts/server_master` 和 Tinc RSA PKCS#1。
- Engine 同时承担登录、下载、RSA、公钥交换、配置、tincd 进程、虚拟网卡、路由和 ping。
- Settings 固定包含 `tinc_binary`、接口名和接口前缀。
- UI 品牌、状态、按钮、提示和安装流程均使用 Tinc 语义。
- Windows Service、Linux 路径、IPC 名称和打包检查均使用 `TincClientAgent` 或 `tinc-client`。

客户端现有的权限分离、IPC 身份校验、帧大小限制、配置白名单、原子配置切换、进程所有权和真实连通探测应原样保留。未来只把 Tinc 实现迁入客户端插件，不削弱这些安全边界。

## 3. 目标平台架构

```text
若依管理 UI
      ↓
平台通用 REST API
      ↓
Network Orchestrator ── Plugin Registry ── Tinc/fnps Server Plugin
      ↓                                      ↓
Operation / Audit / Secret / Artifact   Target-aware Execution Plan
      ↓                                      ↓
Local Executor / SSH Executor / Remote Gateway Agent

服务端 LaunchPlan JSON
      ↓
桌面通用 UI ── 通用 IPC ── Client Plugin Registry
                              ├── Tinc Client Plugin
                              └── fnps Client Plugin
```

### 3.1 平台核心职责

- 用户、节点、网络、节点分配和运行目标。
- 插件注册、版本选择、兼容性和能力查询。
- Desired State、Observed State、Operation、幂等、审计和补偿。
- 制品下载、完整性验证和 Secret Reference。
- 通用状态机、错误信封和 Schema 渲染器。

### 3.2 插件职责

- 具体工具的配置格式、校验、密钥算法和制品生成。
- 具体进程的安装前检查、启动、停止、重载和观察。
- 端口协议、接口行为、路由策略和专属健康检查。
- 插件配置 Schema、默认值、UI 提示和配置版本迁移。
- 插件命名空间内的错误详情。

## 4. 插件化通信协议

服务端应下发声明式 `LaunchPlan`，不能下发任意 Shell、脚本、绝对路径或动态 JavaScript。

### 4.1 LaunchPlan 示例

```json
{
  "protocol": "network-orchestrator.launch-plan/v1",
  "planId": "plan_01JXYZ",
  "revision": 12,
  "expiresAt": "2026-09-15T18:30:00+08:00",
  "assignment": {
    "assignmentId": "asn_10023",
    "networkId": "net_10001",
    "nodeId": "node_20008"
  },
  "plugin": {
    "id": "builtin.tinc",
    "version": "1.0.0",
    "apiVersion": "client-plugin/v1",
    "minAgentVersion": "0.2.0",
    "capabilities": ["configure", "start", "stop", "observe"]
  },
  "actions": [
    {
      "id": "configure",
      "label": "应用配置",
      "inputSchema": {
        "type": "object",
        "additionalProperties": false
      }
    },
    {
      "id": "start",
      "label": "启动网络",
      "inputSchema": {
        "type": "object",
        "additionalProperties": false
      }
    },
    {
      "id": "stop",
      "label": "停止网络",
      "inputSchema": {
        "type": "object",
        "additionalProperties": false
      }
    }
  ],
  "config": {
    "schemaVersion": 1,
    "payload": {
      "networkName": "new_network",
      "node": {
        "name": "new_client",
        "address": "10.0.11.2/32"
      },
      "gateway": {
        "address": "8.148.13.80",
        "port": 600,
        "transports": ["tcp", "udp"],
        "probeAddress": "10.0.11.1"
      },
      "interface": {
        "policy": "managed",
        "logicalName": "tinc0"
      },
      "keyExchange": {
        "type": "tinc-rsa-pkcs1",
        "bits": 3072
      }
    }
  },
  "artifacts": [
    {
      "id": "bootstrap-config",
      "mediaType": "application/vnd.example.tinc-config+zip",
      "url": "/api/platform/v1/artifacts/art_70001",
      "size": 1027,
      "sha256": "..."
    }
  ],
  "healthChecks": [
    {
      "type": "icmp",
      "target": "10.0.11.1",
      "timeoutMs": 1000,
      "intervalMs": 5000
    }
  ],
  "integrity": {
    "algorithm": "ed25519",
    "keyId": "platform-signing-2026-01",
    "signature": "..."
  }
}
```

### 4.2 客户端操作协议

```json
{
  "protocol": "network-agent-command/v1",
  "requestId": "req_01JXYZ",
  "operation": "executeAction",
  "planId": "plan_01JXYZ",
  "actionId": "start",
  "expectedRevision": 12,
  "idempotencyKey": "idem_01JXYZ"
}
```

通用状态响应：

```json
{
  "state": "RUNNING",
  "health": "HEALTHY",
  "plugin": {
    "id": "builtin.tinc",
    "version": "1.0.0"
  },
  "stage": "隧道已验证",
  "observedRevision": 12,
  "error": null
}
```

协议约束：

- 插件必须按 `plugin.id + version` 精确匹配。
- 未安装插件返回 `PLUGIN_NOT_INSTALLED`，不得回退到任意进程执行。
- 所有写操作携带 `idempotencyKey` 和 `expectedRevision`。
- 制品必须校验大小、SHA-256、签名、有效期和 revision。
- 密码、SSH 私钥、Token 不进入插件 payload，只使用短期 Secret Reference 或设备加密密文。
- 核心错误码保持通用；工具错误使用如 `plugin.tinc.port_conflict` 的命名空间。

## 5. 服务端解耦方案

第一阶段采用 Spring 策略注册，不立即动态加载外部 JAR：

```java
public interface NetworkPlugin {
    PluginDescriptor descriptor();
    ValidationResult validate(NetworkSpec spec);
    ProvisionPlan planGateway(NetworkSpec spec, RuntimeTarget target);
    ClientLaunchPlan createLaunchPlan(NodeAssignment assignment);
    ObservedState inspect(RuntimeTarget target, DeploymentRef deployment);
    ReconcileResult reconcile(ReconcileContext context);
}
```

Spring 注入 `List<NetworkPlugin>`，由 `PluginRegistry` 按 `pluginId/version` 建立索引。第一版 `TincNetworkPlugin` 委托现有实现，保持旧行为。

基础设施进一步拆分为：

- `ArtifactStore`：保存不可变配置制品。
- `TargetResolver`：由服务器 ID 解析 RuntimeTarget。
- `ExecutionGateway`：本机、SSH 或远程 Agent 执行。
- `SecretProvider`：通过引用解析凭据。
- `OperationRepository`：持久化操作阶段、幂等键和失败原因。
- `DistributedLease`：替代 JVM 内本地锁。
- `Reconciler`：持续比较期望状态和实际状态。

只有在 Tinc 和第二个真实插件稳定后，才评估 Java ServiceLoader、PF4J 或独立 Plugin Host。插件必须签名、固定版本并声明兼容 API，未经审核的 JAR 不得直接进入主 Spring Boot 权限域。

## 6. UI 与客户端解耦方案

### 6.1 Schema 动态表单

插件提供 JSON Schema Draft 2020-12 和独立 UiSchema。平台只提供白名单控件：

```text
text / password / integer / boolean / enum
cidr / port / port-range / server-picker / secret-picker
```

不允许插件下发 HTML、Vue 模板、JavaScript 表达式、任意 URL 或脚本。

核心页面固定为插件目录、网络实例、节点分配、目标服务器、操作记录和运行状态。选择插件后再渲染插件自己的配置字段。

### 6.2 Rust 客户端

```text
agent-core
├── 通用会话、LaunchPlan、Operation、Status
├── PluginRegistry
├── IPC
├── 受保护存储
└── 权限与进程基础设施

plugins
├── plugin-tinc
└── plugin-fnps
```

概念接口：

```rust
trait ClientNetworkPlugin {
    fn descriptor(&self) -> PluginDescriptor;
    fn validate_plan(&self, plan: &LaunchPlan) -> Result<()>;
    async fn configure(&mut self, ctx: PluginContext) -> Result<PluginStatus>;
    async fn start(&mut self, ctx: PluginContext) -> Result<PluginStatus>;
    async fn stop(&mut self, ctx: PluginContext) -> Result<PluginStatus>;
    async fn inspect(&mut self, ctx: PluginContext) -> Result<PluginStatus>;
}
```

Tinc 的 RSA、ZIP、`server_master`、tincd、TAP/TUN、路由和 ping 迁入 `plugin-tinc`。IPC 身份检查、长度上限、配置白名单和 UI 无提权边界继续保留。

## 7. 路线图

### Phase 0：Tinc 基线彻底跑通

- 完成 UI 登录、配置、启动、停止和真实连通探测的重复验收。
- 新建网络必须由后端自动启动并启用对应 Tinc 服务。
- 自动检查接口/端口冲突、TCP/UDP、防火墙和运行状态。
- 验证公钥首次上传、相同公钥幂等、变化公钥 HUP 和失败回滚。
- 验证 Agent、Windows、后端和服务器重启后的恢复路径。
- 为 API、ZIP、公钥交换、进程、路由和探测建立脱敏 Golden Sample 与回归测试。

退出门槛：换一台干净测试客户端，仅通过 UI 可以稳定连接，连续多次执行无需人工登录服务器修复。

### Phase 1：通用服务端内核和 Tinc 适配器

- 引入 `NetworkPlugin`、`PluginRegistry`、`RuntimeTarget`、`ExecutionGateway` 和 `Operation`。
- `TincNetworkPlugin` 委托现有实现。
- 所有运行操作显式接收 RuntimeTarget。
- 保留旧 Controller、数据库和外部 API。

退出门槛：旧接口兼容，本机和 SSH 目标不混淆，Tinc 连通无回归。

### Phase 2：通用 API 与增量数据模型

新增而不是立即重命名旧表：

- `net_plugin`
- `net_network`
- `net_node`
- `net_assignment`
- `net_runtime_target`
- `net_operation`
- `net_artifact`
- `net_secret_ref`

关键字段包含 `plugin_id`、`plugin_version`、`config_json`、`schema_version` 和 `revision`。推出 `/api/platform/v1/*` 与 LaunchPlan，旧 `/api/tinc/client/*` 作为兼容 Facade。

### Phase 3：客户端内核和 Tinc 插件分离

- 拆分 `agent-core` 与 `plugin-tinc`。
- 通用化 IPC 为 discoverPlugins、getPlan、executeAction、getStatus、getLogs。
- 原有七个 Tauri Command 暂时保留为包装层。
- Tinc 插件仍随安装包内置，不做在线动态安装。

退出门槛：平台核心不再解析 tinc.conf、RSA PEM 或直接启动 tincd。

### Phase 4：Schema 驱动 UI

- 管理端引入通用网络实例页面和插件选择器。
- 桌面端根据 LaunchPlan actions、Schema 和状态动态渲染。
- 原 Tinc 页面通过功能开关保留并逐步跳转到通用页面。

退出门槛：增加插件表单不需要修改核心页面组件。

### Phase 5：以 fnps 验证抽象

fnps 只能新增插件实现、Schema、Manifest 和测试：

- 不修改平台核心数据模型。
- 不新增 fnps 专属 Controller。
- 不在核心 UI 添加 fnps 条件分支。

如果实现 fnps 仍需修改核心业务，说明插件边界尚未稳定，应先修正抽象。

### Phase 6：插件发布与隔离

- 签名插件包、兼容矩阵和依赖检查。
- 灰度启用、版本锁定和回滚。
- 服务端 PF4J 或独立 Plugin Host。
- 客户端隔离插件进程或 WASM 宿主。
- 插件目录和生命周期管理。

高权限客户端 Agent 不直接加载网络下载的任意 DLL；外部插件运行在受限宿主中。

### Phase 7：清理遗留 Tinc 外壳

- 迁移旧 `tinc_*` 数据。
- 下线旧 API 和重复 Vue 页面。
- 删除静态 Utils、字段复用和兼容路由。
- 统一权限名、文档和产品命名。

## 8. 不采用的做法

- 当前阶段批量把类名中的 Tinc 替换成 Plugin。
- 先重命名数据库表，再设计领域边界。
- 用大量 `Map<String, Object>` 代替稳定核心类型。
- 通过 JSON 下发 Shell、脚本或任意路径。
- 在高权限 Agent 中加载未签名 DLL。
- 第一阶段就实现动态安装、热卸载和插件市场。
- 为 fnps 复制一套专属 Controller、Service 和 Vue 页面。

## 9. 最终决策

实施顺序固定为：

```text
Tinc UI 自动连通与恢复验收
→ 冻结 Tinc 契约
→ 修复 Target-aware 运行边界
→ Tinc 包装为第一个内置插件
→ 通用 LaunchPlan 与增量数据模型
→ Schema UI
→ fnps 验证
→ 动态插件发布与遗留清理
```

第一步不是重命名，也不是建立插件市场，而是让现有 Tinc 在没有人工修服务器的情况下，通过 UI 可重复完成完整连接。
