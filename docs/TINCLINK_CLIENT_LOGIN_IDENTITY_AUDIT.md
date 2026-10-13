# TincLink 客户端登录与 Profile 定位逻辑专项源码审计报告

> **历史调查记录（已被后续实现取代）**：本文记录的是接入码改造前的 `sid + password` 登录歧义及当时的修复建议，不代表 Phase 0 当前实现。当前客户端已改为 `accessCode + password` 主通道，并使用 `Management origin + networkId + nodeId` 建立稳定 Profile；现状以 `TINCLINK_ACCESS_CODE_CLIENT_MIGRATION_REPORT.md` 和当前源码为准。本文保留用于追溯问题来源与设计决策。

> **审计执行时间**：2026-09-23  
> **审计范围**：`tinc_client`（Tauri/Vue 桌面端、vpn-core 核心库、Agent 守护进程）与 `RuoYi-Vue-master`（ruoyi-admin、ruoyi-system、ruoyi-common、tinclink-access-agent、SQL 迁移脚本）  
> **审计原则**：零代码修改，全链路源码比对，依据精确 SQL、Mapper、Controller、Service、Agent 及 IPC 实现。

---

## 目录
1. [当前客户端登录 UI 检查](#一当前客户端登录-ui-检查)
2. [实际登录 Request 报文与字段定位](#二实际登录-request-报文与字段定位)
3. [后端 Controller 鉴权处理与调用链](#三后端-controller-鉴权处理与调用链)
4. [Service 业务层与校验逻辑](#四service-业务层与校验逻辑)
5. [MyBatis Mapper 与实际执行 SQL](#五mybatis-mapper-与实际执行-sql)
6. [最终 Node 查询条件与判定模式判定](#六最终-node-查询条件与判定模式判定)
7. [数据库当前真实约束检查](#七数据库当前真实约束检查)
8. [登录成功响应报文检查](#八登录成功响应报文检查)
9. [node_id 与 network_id 在客户端的使用情况](#九node_id-与-network_id-在客户端的使用情况)
10. [Profile Identity 稳定身份建立流程](#十profile-identity-稳定身份建立流程)
11. [三个同名 Node 场景实证推演](#十一三个同名-node-场景实证推演)
12. [客户端“服务端地址”字段真实含义](#十二客户端服务端地址字段真实含义)
13. [Access Server 不参与登录请求的影响链](#十三access-server-不参与登录请求的影响链)
14. [Profile 存储与切换能力检查](#十四profile-存储与切换能力检查)
15. [客户端 Runtime 切换真实流程](#十五客户端-runtime-切换真实流程)
16. [服务器端多 Network Runtime 模型检查](#十六服务器端多-network-runtime-模型检查)
17. [网卡切换理解纠正](#十七网卡切换理解纠正)
18. [重复网段场景分析](#十八重复网段场景分析)
19. [问题风险分级](#十九问题风险分级)
20. [最小修复方案建议（未执行）](#二十最小修复方案建议未执行)
21. [核心问题逐项结论（Q1 ~ Q8）](#二十一核心问题逐项结论q1--q8)

---

## 一、当前客户端登录 UI 检查

源码位置：[`apps/desktop/src/App.vue`](file:///D:/Codes/Java/KenDeJi_RuoYi/tinc_client/apps/desktop/src/App.vue#L16-L267)

### 1.1 界面输入组件定义
在 `App.vue` 模板第 214~267 行中，用户可见的表单元素包括：
1. **服务端地址**（`v-model="server"`，默认值为 `'https://'`，占位符 `"https://vpn.example.com"`）
2. **节点名**（`v-model="sid"`，占位符 `"例如 office_laptop"`，最大长度限制 64）
3. **密码**（`v-model="password"`，类型 `password`，占位符 `"请输入节点密码"`）
4. **允许 HTTP 明文登录**（`v-model="allowHttp"`，复选框）

### 1.2 界面缺失字段
界面中**没有任何**关于以下维度的选择框、输入框或隐藏字段：
- `networkId` / `networkName`（所属内网 ID 或名称）
- `serverId` / `serverName`（所属接入服务器 ID 或名称）
- `nodeId`（节点唯一数字 ID）

---

## 二、实际登录 Request 报文与字段定位

源码位置：
- 前端触发：[`apps/desktop/src/App.vue:126-134`](file:///D:/Codes/Java/KenDeJi_RuoYi/tinc_client/apps/desktop/src/App.vue#L126-L134)
- Tauri 桥接：[`apps/desktop/src-tauri/src/main.rs:11-19`](file:///D:/Codes/Java/KenDeJi_RuoYi/tinc_client/apps/desktop/src-tauri/src/main.rs#L11-L19)
- IPC 协议：[`crates/vpn-core/src/model.rs:86-92`](file:///D:/Codes/Java/KenDeJi_RuoYi/tinc_client/crates/vpn-core/src/model.rs#L86-L92)
- HTTP 请求封装：[`crates/vpn-core/src/api.rs:178-205`](file:///D:/Codes/Java/KenDeJi_RuoYi/tinc_client/crates/vpn-core/src/api.rs#L178-L205)

### 2.1 链路报文传递
1. **前端向 Tauri 发送**：
   ```typescript
   call<Status>('login', {
       server: server.value.trim(),
       sid: sid.value.trim(),
       password: password.value,
       allowHttp: allowHttp.value,
   });
   ```
2. **Tauri 转换成 IPC Request**：
   ```rust
   Request::Login {
       server,
       sid,
       password: zeroize::Zeroizing::new(password),
       allow_http,
   }
   ```
3. **Agent Core 执行 HTTP 请求**：
   在 `crates/vpn-core/src/api.rs:198-204`：
   ```rust
   let response = post(
       &client,
       &origin,
       "/api/tinc/client/login",
       json!({"sid": sid, "password": password}),
       64 * 1024,
   ).await?;
   ```

### 2.2 真实发往后端的 HTTP 请求
- **HTTP Method**：`POST`
- **Request URL**：`{server}/api/tinc/client/login`
- **Content-Type**：`application/json`
- **Body JSON**：
  ```json
  {
    "sid": "<输入的节点名称>",
    "password": "<输入的明文密码>"
  }
  ```
> **结论**：客户端向后端发送的登录请求**仅包含 `sid`（节点名）与 `password`（密码）两个业务字段**，完全不包含 `networkId`、`networkName`、`serverId`、`nodeId` 或 `deviceId`。

---

## 三、后端 Controller 鉴权处理与调用链

源码位置：[`ruoyi-admin/src/main/java/com/ruoyi/web/controller/system/TincClientApiController.java`](file:///D:/Codes/Java/KenDeJi_RuoYi/RuoYi-Vue-master/ruoyi-admin/src/main/java/com/ruoyi/web/controller/system/TincClientApiController.java#L84-L146)

### 3.1 完整调用链梳理
```text
Client UI (App.vue: act('login'))
  ↓ invoke('login')
Tauri Desktop Rust (main.rs: login)
  ↓ vpn_ipc::call(Request::Login)
Client Agent Engine (engine.rs: execute)
  ↓ Session::login (api.rs)
HTTP POST /api/tinc/client/login {"sid": sid, "password": password}
  ↓ Spring MVC Dispatcher
TincClientApiController.login(@RequestBody Map<String, String> params)
  ↓ findPasswordBoundNode(username, password)
  ↓ findExactNodes(nodeName)
TincNodeMangeServiceImpl.selectByNodeNameExact(nodeName)
  ↓ TincNodeMangeMapper.selectByNodeNameExact(nodeName)
MySQL Database: SELECT ... FROM tinc_node WHERE node_name = ?
  ↓ 返回 List<TincNodeMange> 所有同名候选节点
TincClientApiController 内存遍历循环匹配 candidate.getPassword().equals(password)
```

### 3.2 Controller 关键代码分析
在 `TincClientApiController.java` 第 105 行：
```java
TincNodeMange node = findPasswordBoundNode(username, password);
if (node == null) {
    log.warn("客户端登录认证失败: sid={}", username);
    result.put("status", 0);
    result.put("msg", "sid 或密码无效");
    return result;
}
```
在第 659~675 行：
```java
private List<TincNodeMange> findExactNodes(String nodeName) {
    List<TincNodeMange> matches = nodeMangeService.selectByNodeNameExact(nodeName);
    return matches == null ? java.util.Collections.emptyList() : matches;
}

private TincNodeMange findPasswordBoundNode(String nodeName, String password) {
    TincNodeMange match = null;
    for (TincNodeMange candidate : findExactNodes(nodeName)) {
        if (password.equals(candidate.getPassword())) {
            if (match != null) {
                throw new IllegalStateException("sid 与密码匹配到多条节点记录");
            }
            match = candidate;
        }
    }
    return match;
}
```

---

## 四、Service 业务层与校验逻辑

源码位置：[`ruoyi-system/src/main/java/com/ruoyi/tinc_node/service/impl/TincNodeMangeServiceImpl.java`](file:///D:/Codes/Java/KenDeJi_RuoYi/RuoYi-Vue-master/ruoyi-system/src/main/java/com/ruoyi/tinc_node/service/impl/TincNodeMangeServiceImpl.java#L63-L253)

### 4.1 节点查询方法
```java
@Override
public List<TincNodeMange> selectByNodeNameExact(String nodeName) {
    return tincNodeMangeMapper.selectByNodeNameExact(nodeName);
}
```
此处为透传调用，没有任何关于当前网络、服务器维度的截断或默认过滤。

### 4.2 节点创建与修改时的唯一性校验
在 `TincNodeMangeServiceImpl.java:244-253`：
```java
private void validateNodeUniqueness(TincNodeMange node) {
    if (tincNodeMangeMapper.countNodeNameInNetwork(
            node.getNetworkId(), node.getNodeName(), node.getId()) > 0) {
        throw new IllegalStateException("TINC_NODE_NAME_CONFLICT: 同一网络内节点名称必须唯一");
    }
    if (tincNodeMangeMapper.countNetworkIpInNetwork(
            node.getNetworkId(), node.getNetworkIp(), node.getId()) > 0) {
        throw new IllegalStateException("TINC_NODE_IP_CONFLICT: 同一网络内节点地址必须唯一");
    }
}
```
**关键发现**：业务层校验明确定义——**仅在同一网络（`network_id`）内要求 `node_name` 唯一**。在不同网络之间，完全允许存在同名的 `node_name`。

---

## 五、MyBatis Mapper 与实际执行 SQL

源码位置：[`ruoyi-system/src/main/resources/mapper/tinc_node/TincNodeMangeMapper.xml`](file:///D:/Codes/Java/KenDeJi_RuoYi/RuoYi-Vue-master/ruoyi-system/src/main/resources/mapper/tinc_node/TincNodeMangeMapper.xml#L45-L48)

### 5.1 Mapper XML 定义
```xml
<select id="selectByNodeNameExact" parameterType="String" resultMap="TincNodeMangeResult">
    <include refid="selectTincNodeMangeVo"/>
    where node_name = #{nodeName}
</select>
```
引用的 `selectTincNodeMangeVo`：
```xml
<sql id="selectTincNodeMangeVo">
    select id, network_id, user_name, device_id, server_name, network_name, password, node_name, network_ip, remark, create_time, node_status, status, create_by, update_by, update_time from tinc_node
</sql>
```

### 5.2 实际执行 SQL
```sql
SELECT id, network_id, user_name, device_id, server_name, network_name, password, node_name, network_ip, remark, create_time, node_status, status, create_by, update_by, update_time 
FROM tinc_node 
WHERE node_name = ?
```

---

## 六、最终 Node 查询条件与判定模式判定

### 6.1 对应模式回答
当前登录模式属于：
> **A（变形模式）：SQL 执行全局 `nodeName` 查询，随后在 Controller 内存中按 `password` 线性筛选候选节点。**

既不是 B（无 `networkId` 入参），也不是 C（无 `nodeId` 入参），也不是 D（无 `deviceId` 入参）。

### 6.2 多行命中时的具体行为
如果数据库中存在 `Network-A / nodeName=a` 和 `Network-B / nodeName=a`：
1. **SELECT / Mapper 返回**：返回包含**多行**数据的 `List<TincNodeMange>`。
2. **Controller 筛选行为**：
   - **情况 1：密码不同**（例如 Node-A 密码为 `pwdA`，Node-B 密码为 `pwdB`）：
     - 当客户端输入 `sid="a", password="pwdA"` 时，`candidate.getPassword().equals(password)` 仅命中 Node-A，循环结束返回 Node-A，登录成功。
     - 当客户端输入 `sid="a", password="pwdB"` 时，仅命中 Node-B，返回 Node-B，登录成功。
     - **实质**：**密码实际充当了区分不同 Network 下同名 Node 的“路由键”与“主键选择器”！**
   - **情况 2：密码相同**（例如 Node-A 与 Node-B 密码均为 `123456`）：
     - 循环第一次命中 Node-A，`match = candidate`；
     - 循环第二次命中 Node-B，此时 `match != null`，代码主动抛出异常：
       ```java
       throw new IllegalStateException("sid 与密码匹配到多条节点记录");
       ```
     - 该异常被 `login` 方法第 139 行捕获：
       ```java
       } catch (Exception e) {
           log.error("登录接口异常，错误类型={}", e.getClass().getSimpleName());
           result.put("status", 0);
           result.put("msg", "系统暂时不可用");
       }
       ```
     - 客户端收到 `{"status": 0, "msg": "系统暂时不可用"}`，**两个节点均无法登录，系统陷入局部拒绝服务（DoS）**。

---

## 七、数据库当前真实约束检查

源码依据：
- 基础建表：[`model_SQL/$R4XDT2Q.sql`](file:///D:/Codes/Java/KenDeJi_RuoYi/RuoYi-Vue-master/model_SQL/$R4XDT2Q.sql#L12-L26)
- 历史修复：[`sql/rename_and_fix_tinc_tables.sql`](file:///D:/Codes/Java/KenDeJi_RuoYi/RuoYi-Vue-master/sql/rename_and_fix_tinc_tables.sql#L47-L58)
- Phase 1 迁移：[`sql/20260919_tinc_stable_ids_phase1.sql`](file:///D:/Codes/Java/KenDeJi_RuoYi/RuoYi-Vue-master/sql/20260919_tinc_stable_ids_phase1.sql#L101-L120)

### 7.1 真实约束检查汇总表

| 约束项 | 是否存在于 DB | 是否存在于代码校验 | 源码位置与证据 |
| :--- | :---: | :---: | :--- |
| `PRIMARY KEY (id)` | **是** | 是 | `$R4XDT2Q.sql:25`（自增主键） |
| `UNIQUE(network_id, node_name)` | **否** | **是** | DB 脚本 `20260919_tinc_stable_ids_phase1.sql:114` 处于**注释状态**未执行；由 Java `validateNodeUniqueness` 在应用层保障 |
| 全局 `UNIQUE(node_name)` | **否** | **否** | 允许不同网络存在同名节点 |
| `UNIQUE(node_name, password)` | **否** | **否** | 数据库与代码均无此约束 |
| `network_id` 外键/稳定列 | **是（无FK）** | 是 | `ADD COLUMN network_id BIGINT NULL`，有普通索引 `idx_node_network_id` |
| `server_id` 外键/稳定列 | **是（无FK）** | 是 | `tinc_network` 表有 `server_id BIGINT NULL`，有普通索引 `idx_network_server_id` |

> **结论**：数据库底层物理表仅有 `id` 主键与普通索引，未施加物理 UNIQUE 约束。同网络内同名限制仅靠 Java 业务代码维持；不同网络间同名同密节点在数据库层面完全合法，无任何阻断。

---

## 八、登录成功响应报文检查

源码位置：[`ruoyi-admin/src/main/java/com/ruoyi/web/controller/system/TincClientApiController.java:128-137`](file:///D:/Codes/Java/KenDeJi_RuoYi/RuoYi-Vue-master/ruoyi-admin/src/main/java/com/ruoyi/web/controller/system/TincClientApiController.java#L128-L137)

### 8.1 响应报文 JSON
```java
String clientToken = createClientToken(node);
result.put("sid", username);
result.put("status", 1);
result.put("token", clientToken);
result.put("net_name", network.getNetworkName());
result.put("network_id", network.getId());
result.put("node_id", node.getId());
result.put("msg", "登录成功");
result.put("node_ip", node.getNetworkIp());
```
序列化后返回给客户端的真实 JSON：
```json
{
  "sid": "a",
  "status": 1,
  "token": "4f8a... (64 hex)",
  "net_name": "net_office",
  "network_id": 11,
  "node_id": 101,
  "node_ip": "10.0.1.5",
  "msg": "登录成功"
}
```

### 8.2 字段完整性确认
- `node_id`：**已返回**（Long）
- `network_id`：**已返回**（Long）
- `net_name`：**已返回**（String）
- `node_ip`：**已返回**（String）
- `token`：**已返回**（String）
- `server_id`：**未返回**
- `server_name`：**未返回**
- `config status`：**未返回**（仅返回操作结果 `status: 1`）

---

## 九、node_id 与 network_id 在客户端的使用情况

源码位置：
- 客户端解析：[`crates/vpn-core/src/api.rs:225-231`](file:///D:/Codes/Java/KenDeJi_RuoYi/tinc_client/crates/vpn-core/src/api.rs#L225-L231)
- 桌面 UI 匹配：[`apps/desktop/src/services/client.ts:31-37`](file:///D:/Codes/Java/KenDeJi_RuoYi/tinc_client/apps/desktop/src/services/client.ts#L31-L37)

### 9.1 客户端模型接收
在 `api.rs` 中：
```rust
let identity = Identity {
    sid: field("sid")?,
    net_name: field("net_name")?,
    node_ip: field("node_ip")?,
    node_id: v["node_id"].as_u64(),
    network_id: v["network_id"].as_u64(),
};
```
### 9.2 客户端状态机比对
在 `client.ts` 中：
```typescript
const sameProfile = !!session && !!configured
  && session.sid === configured.sid
  && session.net_name === configured.net_name
  && (session.node_id == null || configured.node_id == null || session.node_id === configured.node_id)
  && (session.network_id == null || configured.network_id == null || session.network_id === configured.network_id);
```
> **确认**：Phase 2.5D 中后端增加的 `node_id` 与 `network_id`，**已在客户端成功接收并实际用于登录状态校验与 Profile 稳定身份构建**。

---

## 十、Profile Identity 稳定身份建立流程

源码位置：[`crates/vpn-core/src/keys.rs:9-21`](file:///D:/Codes/Java/KenDeJi_RuoYi/tinc_client/crates/vpn-core/src/keys.rs#L9-L21)

### 10.1 Profile ID 算法实现
```rust
pub fn profile_id(origin: &str, identity: &Identity) -> String {
    let scope = match (identity.network_id, identity.node_id) {
        (Some(network_id), Some(node_id)) => format!("id\0{network_id}\0{node_id}"),
        _ => format!("{}\0{}", identity.net_name, identity.sid),
    };
    format!("{:x}", Sha256::digest(format!("{origin}\0{scope}")))
}
```
### 10.2 身份判定特征
1. 当 `network_id` 与 `node_id` 存在时，Profile 稳定身份是由：
   $$\text{profile\_id} = \text{SHA256}(\text{origin} \mathbin{\Vert} \text{"\textbackslash 0id\textbackslash 0"} \mathbin{\Vert} \text{network\_id} \mathbin{\Vert} \text{"\textbackslash 0"} \mathbin{\Vert} \text{node\_id})$$
   构建而成。
2. 本地私钥存储目录固定为：`keys/<profile_id>/rsa_key.priv`。
3. **重要特征**：登录成功后的配置应用阶段，客户端的 Profile 隔离**已经摆脱了对容易重名的 `nodeName` / `net_name` 的依赖**，具备抗重名冲突的物理存储隔离能力。
4. **薄弱环节**：这种稳定隔离**仅在登录成功后生效**；在发起登录请求的那一瞬间，客户端尚未取得 `network_id` 和 `node_id`，因此无法在登录前用该哈希去精确定位目标。

---

## 十一、三个同名 Node 场景实证推演

### 11.1 测试场景配置
- **Node 1**：`serverId=4, networkId=11, nodeId=101, nodeName=a`
- **Node 2**：`serverId=4, networkId=12, nodeId=102, nodeName=a`
- **Node 3**：`serverId=5, networkId=21, nodeId=103, nodeName=a`

全部由同一个 Management Server（`https://management.example`）管理。

### 11.2 输入 `sid="a", password=...` 的命中分析

| 子场景 | 密码分配情况 | 客户端输入密码 | 后端实际执行行为 | 最终命中结果 | 诊断判定 |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **场景 A** | Node 1: `p1`<br>Node 2: `p2`<br>Node 3: `p3` | `p1` | 遍历候选列表，仅 Node 1 密码匹配，`match` 赋值 Node 1 | 命中 **Node 1** (ID: 101, Net: 11) | **存在身份歧义**<br>密码被非法当作网络路由键 |
| **场景 B** | Node 1: `p1`<br>Node 2: `p2`<br>Node 3: `p3` | `p2` | 遍历候选列表，仅 Node 2 密码匹配，`match` 赋值 Node 2 | 命中 **Node 2** (ID: 102, Net: 12) | **存在身份歧义**<br>用户输入不同密码跳入不同内网 |
| **场景 C** | Node 1: `p0`<br>Node 2: `p0`<br>Node 3: `p3` | `p0` | 匹配到 Node 1 后再匹配到 Node 2，触发 `match != null` 抛出 `IllegalStateException` | **两败俱伤**<br>返回“系统暂时不可用”，均无法登录 | **拒绝服务 (DoS)** |
| **场景 D** | Node 1: `p0`<br>Node 2: `p0`<br>Node 3: `p0` | `p0` | 匹配到第 2 条记录时直接抛出异常 | **三方均瘫痪**<br>均无法登录 | **拒绝服务 (DoS)** |

> **审查结论**：在三个同名 Node 场景下，**明确存在严重身份歧义（Identity Ambiguity）与可用性崩溃风险**。

---

## 十二、客户端“服务端地址”字段真实含义

源码依据：[`crates/vpn-core/src/api.rs:34-49, 178-205`](file:///D:/Codes/Java/KenDeJi_RuoYi/tinc_client/crates/vpn-core/src/api.rs#L34-L49)

### 12.1 源码证据
1. `api.rs` 的 `origin(server, allow_http)` 解析用户输入的 `server` 字符串。
2. 该地址直接作为 HTTP Base URL 请求 `/api/tinc/client/login`、`/api/tinc/client/config/download`、`/api/tinc/client/key/upload`、`/api/tinc/client/keepalive`。
3. 这些接口全部由 Spring Boot（`RuoYi-Vue-master/ruoyi-admin`）提供服务。

### 12.2 影响分析
- 客户端输入的“服务端地址”字段**毫无疑问是 Management（管控端）地址**，而不是具体的 Tinc Access Server 地址。
- 同一个 Management 管控端实例下挂载了所有的 Server、Network 与 Node。
- **因此，用户输入任何有效的 Management 地址，都无法为后端提供任何关于“我想要登录哪个 Server 或哪个 Network”的区分信息**。

---

## 十三、Access Server 不参与登录请求的影响链

源码位置：[`ruoyi-admin/src/main/java/com/ruoyi/web/controller/system/TincClientApiController.java:190-214`](file:///D:/Codes/Java/KenDeJi_RuoYi/RuoYi-Vue-master/ruoyi-admin/src/main/java/com/ruoyi/web/controller/system/TincClientApiController.java#L190-L214)

### 13.1 接入服务器解析链
1. **登录阶段**：Client 只请求 Management，Management 内部根据 `findPasswordBoundNode` 找出 Node，根据 `node.getNetworkId()` 找出 Network。此时 Access Server 完全不参与，也没有任何数据返回。
2. **下载配置阶段**（`/config/download`）：
   ```java
   TincNetworkMange network = resolveNetworkForNode(node);
   MangeServer server = resolveServerForNetwork(network);
   String serverHostContent = tincRuntimeRouter.readServerMaster(server, network.getNetworkName());
   ```
   Management 从数据库查出该 Network 所绑定的 `server_id`，进而得到对应的 `MangeServer`，再通过 SSH 或 Access Agent 读取该 Access Server 上的 `hosts/server_master` 公钥与外网连接 IP/端口，打包进 ZIP 下发给客户端。
3. **数据通道建立**：
   客户端拿到 ZIP 内的 `hosts/server_master`（包含 Access Server 的实际外网 Address 和 Port），交由本地 `tincd` 核心去发起隧道握手。

> **结论**：Access Server 属于数据面与节点接入网关，完全不作为客户端认证端点；认证完全收拢在 Management 端。因此登录请求无法通过网络层目标 IP 来隐含区分接入服务器。

---

## 十四、Profile 存储与切换能力检查

源码位置：
- 目录结构与读取：[`crates/vpn-core/src/config.rs:284-307`](file:///D:/Codes/Java/KenDeJi_RuoYi/tinc_client/crates/vpn-core/src/config.rs#L284-L307)
- 密钥存储：[`crates/vpn-core/src/keys.rs:56-82`](file:///D:/Codes/Java/KenDeJi_RuoYi/tinc_client/crates/vpn-core/src/keys.rs#L56-L82)

### 14.1 存储维度区分度分析
针对三个 Profile：
- Profile 1: Access-A / Network-a / a (netId=11, nodeId=101)
- Profile 2: Access-A / Network-b / a (netId=12, nodeId=102)
- Profile 3: Access-B / Network-a / a (netId=21, nodeId=103)

| 存储层级 | 能否物理区分上述 3 个 Profile | 源码实现机理 |
| :--- | :---: | :--- |
| **Key Storage** (`keys/`) | **能** | 目录名为 `profile_id`（由 `origin + netId + nodeId` 哈希），3 个 Profile 拥有完全独立的 3 个密钥文件夹 |
| **Generation Storage** (`generations/`) | **能** | 每次应用配置生成独立的 `gen-<uuid>` 目录，内含该 Profile 专属的 `tinc.conf` 与 `hosts` |
| **Active Profile** (`active.json`) | **不能同时保存** | 根目录下仅有唯一一个 `active.json`，只指向当前活动的单一 Profile |
| **Profile List** | **不存在** | 客户端既无 `profiles.json`，也无 Profile 列表管理模块，UI 亦无 Profile 切换列表组件 |

> **结论**：客户端底层的密钥和历史配置版本库是具备多 Profile 区分能力的，但上层管理逻辑被实现为**单一活跃 Profile 模式（Single Active Profile）**，未提供 Profile 列表的持久化索引与无缝切换 UI。

---

## 十五、客户端 Runtime 切换真实流程

源码位置：[`crates/vpn-core/src/engine.rs:298-370, 525-647, 855-870`](file:///D:/Codes/Java/KenDeJi_RuoYi/tinc_client/crates/vpn-core/src/engine.rs)

### 15.1 切换时真实执行步骤
客户端从 Profile 1 切换到 Profile 2 时，必须完整经历如下 5 个串行阶段：
1. **停止旧 Tinc Runtime**：
   - 调用 `stop()`：向运行中的 Tinc 子进程发送退出信号（`child.kill()`）。
   - 彻底清理虚拟网卡：调用 `clear_adapter(&ip)`，删除旧有的网关主机路由（`netsh interface ipv4 delete route` 或 `ip route del`），清除网卡绑定的虚拟 IP 地址（`netsh interface ipv4 delete address` 或 `ip addr del`）。
   - 删除 `adapter-state.json` 恢复记录。
2. **切换会话与配置**：
   - 调用 `login`：使用新节点凭据登录 Management，取得新 `session.identity`（包含新 `node_id` 与 `network_id`）。
   - 调用 `configure`：
     - 若旧 Tinc 进程未退出或网卡未清理完毕，直接拒绝并报错 `busy`；
     - 下载新网络 ZIP 包并校验安全路径；
     - 切换并复用/生成 `keys/<new_profile_id>/rsa_key.priv`；
     - 上传公钥至 Management 并交换 `server_master`；
     - 生成全新的 `generations/gen-<new-uuid>`；
     - 原子覆写 `active.json`，将活动指针指向新 generation。
3. **启动新 Tinc Runtime**：
   - 调用 `start()`：校验 `session.identity == active.identity` 且 `profile_id` 匹配。
   - 调起 `tincd -D -c generations/gen-<new-uuid>`。
4. **配置专用网卡**：
   - 调用 `set_adapter`：为本地 TAP 网卡绑定新节点的虚拟 IPv4 地址。
   - 添加直连路由：向 TAP 网卡添加通往新网络 `server_master` IPv4 的精确 `/32` 主机路由。
5. **连通性验证**：
   - 开启后台探针轮询（`probe_connection`），向新网络的 `server_master` 发起 ICMP/TCP 探测；
   - 验证通过后状态跃迁为 `connected`（“已验证连通”）。

---

## 十六、服务器端多 Network Runtime 模型检查

源码位置：[`ruoyi-common/src/main/java/com/ruoyi/common/tinc/runtime/LocalTincRuntimeManager.java`](file:///D:/Codes/Java/KenDeJi_RuoYi/RuoYi-Vue-master/ruoyi-common/src/main/java/com/ruoyi/common/tinc/runtime/LocalTincRuntimeManager.java)

### 16.1 服务端多实例并发模型确认
在 `LocalTincRuntimeManager.java` 源码中：
1. **独立配置目录**（第 530~532 行）：
   `configRoot.resolve(netName)` $\rightarrow$ 每个网络拥有独立的 `/etc/tinc/<network>`。
2. **独立 systemd Unit**（第 526~528 行）：
   `unit(netName)` 返回 `"tinc@" + netName + ".service"`。
3. **独立网卡与端口冲突排查**（第 319~328 行）：
   ```java
   if (own.interfaceName.equals(otherInterface)) {
       throw new TincRuntimeException(TincRuntimeErrorCode.TINC_INTERFACE_CONFLICT, ...);
   }
   if (otherPort != null && own.port == otherPort) {
       throw new TincRuntimeException(TincRuntimeErrorCode.TINC_PORT_CONFLICT, ...);
   }
   ```
   **源码硬性规定**：在同一台 Access Server 上，不同网络**必须拥有不同的 Interface 虚拟网卡名称**（如 `tinc_net1` 与 `tinc_net2`），且**必须监听在不同的端口**（如 `655` 与 `656`）。
4. **独立进程与路由**：
   每个 `tinc@<network>` 服务启动时独立执行其目录下的 `tinc-up` 脚本，拉起属于自己的虚拟网卡并配置该网段的虚拟 IP 与路由。

> **结论**：一台 Access Server 上**完全可以且本来就支持同时运行多个独立的 Tinc Network Runtime**。每个网络拥有专属的目录、systemd unit、虚拟网卡接口、监听端口与系统进程，彼此并行不悖。

---

## 十七、网卡切换理解纠正

### 17.1 核心问题回答
> **问**：如果 Access-A 同时运行 Network-a 与 Network-b，客户端从 Network-a 切到 Network-b，服务端是否需要把虚拟网卡从 a 切换成 b？
>
> **答**：**服务端绝对不需要切换虚拟网卡！服务端的两个 Network Runtime 本来就一直同时存在并并行运行。**

### 17.2 事实推演依据
1. **服务端状态**：
   - `tinc@Network-a.service` 正在运行，绑定虚拟网卡 `tinc_a`，监听端口 `655`；
   - `tinc@Network-b.service` 也在运行，绑定虚拟网卡 `tinc_b`，监听端口 `656`；
   - 两个服务端进程与虚拟网卡全年无休同时在线。
2. **切换发生时**：
   - 客户端单方面停止本地指向 Access-A:655 的 tinc 进程；
   - 客户端单方面清洗本地单网卡的 IP 和路由；
   - 客户端重新启动本地 tinc 进程，发起通往 Access-A:656 的 VPN 隧道握手；
   - 服务端没有任何服务重启、网卡注销或配置重载动作发生（除接收公钥时通过 `kill -HUP` 动态通知外）。

---

## 十八、重复网段场景分析

### 18.1 场景描述
- Profile A 属于 Network-a，分配内网网段 `10.0.50.0/24`（客户端 IP 为 `10.0.50.2`）
- Profile B 属于 Network-b，分配内网网段 `10.0.50.0/24`（客户端 IP 为 `10.0.50.3`）

### 18.2 单 Active Profile 模式下的顺序切换可行性
- **可行性**：**完全可行**。
- **原因**：在 `crates/vpn-core/src/engine.rs:855-870` 中，客户端在切换 profile 前必须先执行 `stop()`，`clear_adapter` 会通过 `netsh` 或 `ip` 命令把 `10.0.50.2` 以及指向网关的 `/32` 主机路由从 TAP 网卡上**彻底删除**。当 Profile B 启动时，网卡是一张纯净的空网卡，重新绑定 `10.0.50.3`，因此在操作系统路由表中不会有历史残留冲突。

### 18.3 多个 Network Runtime 若在客户端同时运行的冲突分析
如果未来客户端试图支持“多网络并发同时连接”：
1. **TAP 网卡单义性冲突**：Windows 下若只安装了一个 TAP-Windows 适配器，无法同时被两个不同的 tincd 进程独占打开。
2. **路由表黑洞与度量冲突**：若安装了双 TAP 网卡，两个网卡同时配置 `10.0.50.0/24`：
   - Windows 路由表会出现两条完全相同的目标网段路由条目；
   - 系统将仅根据 Metric（跃点数）将流量单向发送到其中一张网卡，通往另一个网络的流量将被完全黑洞化；
   - ARP 广播将产生歧义，导致严重断网。

---

## 十九、问题风险分级

### 🔴 P0 级严重缺陷（会导致登录到错误 Node / 安全机制被颠覆 / 局部拒绝服务）
1. **登录请求严重缺少 Network 定位维度，非法利用密码充当资源身份区分器**：
   - 客户端发起登录仅传 `nodeName + password`，Controller 使用 `selectByNodeNameExact` 全局搜索。
   - 两个不同网络下的同名 Node 若密码不同，用户输入对应密码将直接导致静默登录到不同网络，密码承担了资源定位的主键职责，颠覆认证凭据与身份标识的边界。
2. **同名同密节点引发系统级异常导致登录瘫痪（DoS）**：
   - 若两个同名 Node 被管理员或用户设置了相同密码，Controller 遍历触发 `IllegalStateException("sid 与密码匹配到多条节点记录")`，接口返回 `status: 0, msg: "系统暂时不可用"`，两个节点的用户均被彻底锁死无法登录。

### 🟡 P1 级缺陷（多 Profile 场景下配置串线与数据库缺乏刚性防护）
1. **客户端缺乏多 Profile 本地索引与持久化列表**：
   - 客户端只有单活跃 `active.json`，无 Profile 切换管理器，每次切换必须重新走完整登录流程覆盖原活跃状态。
2. **MySQL 物理表缺少 `(network_id, node_name)` 联合唯一索引**：
   - 数据库脚本将唯一约束注释掉，仅靠 Java 应用代码校验。若有脚本直插或并发绕过，数据库将无法阻止同网络下产生重复节点。

### 🔵 P2 级设计隐患（当前单网络能跑，但未来扩展与维护存在混淆风险）
1. **客户端“服务端地址”UI 语义混淆**：
   - UI 标注为“服务端地址”，实为“Management 管控端 URL”。用户容易误以为这是 Access Server 地址。
2. **登录成功响应缺少 `server_id` 与 `server_name`**：
   - 登录响应只返回了 `node_id` 和 `network_id`，未返回所属服务器 ID，客户端无法建立完整的三层层级元数据视图。

### ⚪ P3 级优化项（文案与展示细节）
1. **登录区域缺少“所属网络”辅助提示文案**。

---

## 二十、最小修复方案建议（未执行）

> ⚠️ **声明**：本轮审计严格遵守“只检查、不修改代码”的要求，以下方案仅作为设计参考，本轮未进行任何文件修改。

### 20.1 推荐的最小修复步骤（非破坏性平滑升级）

#### 步骤 1：前端与客户端登录 API 增加 `networkId` 或 `networkName` 可选/必选维度
- 在 `App.vue` 界面增加“网络名称或网络ID”输入项（或提供两步式交互：先输入 server+sid 列出归属网络，或直接由管理员提供的规范标识一次性输入）。
- 客户端登录请求 Body 由：
  ```json
  {"sid": "a", "password": "******"}
  ```
  扩充为：
  ```json
  {"sid": "a", "password": "******", "network_id": 11}
  ```
  （或兼容传 `net_name: "net_office"`）

#### 步骤 2：后端 Controller 与 Mapper 改造为按网络精确查询
- Mapper 废弃无网络约束的 `selectByNodeNameExact`，改用已有的或新增联合查询：
  ```xml
  <select id="selectByNetworkAndNodeName" resultMap="TincNodeMangeResult">
      <include refid="selectTincNodeMangeVo"/>
      where network_id = #{networkId} and node_name = #{nodeName}
  </select>
  ```
- `findPasswordBoundNode` 彻底退役。
- 逻辑还原为标准认证范式：
  1. 通过 `networkId + nodeName` 唯一定位单个 Node 实体；
  2. 若找不到实体，返回用户不存在；
  3. 若找到实体，**仅比对该实体的密码**：`if (!node.getPassword().equals(password)) return 密码错误;`
  4. 密码**仅用于验真**，彻底不再参与节点的遍历与定位。

#### 步骤 3：数据库激活联合唯一索引
在数据库执行原本被注释的 SQL：
```sql
ALTER TABLE tinc_node ADD CONSTRAINT uk_tinc_node_network_name UNIQUE (network_id, node_name);
```
从底层彻底消除数据冗余与歧义可能。

---

## 二十一、核心问题逐项结论（Q1 ~ Q8）

### Q1：客户端只输入 nodeName + password 时，服务端能否唯一确定 Node？
> **明确回答**：**不能安全、确定地唯一确定。**  
> **标注**：【源码确认】  
> **依据**：Controller 会首先通过 `where node_name = #{nodeName}` 查出所有网络的同名节点。若恰好密码各不相同，代码会通过内存密码比对“撞”出一个 Node；若密码相同，则抛出异常完全无法确定并报错。

---

### Q2：如果三个不同 Network 下都存在 nodeName=a，当前登录会发生什么？
> **明确回答**：  
> 1. 若三个 Node 密码不同，输入谁的密码就命中谁，密码被非法用作网络分流键；  
> 2. 若存在两个或三个节点密码相同，输入该密码将直接触发 `IllegalStateException("sid 与密码匹配到多条节点记录")`，导致服务端抛出异常，客户端收到 `status: 0, msg: "系统暂时不可用"`，发生局部拒绝服务。  
> **标注**：【源码确认】  
> **依据**：`TincClientApiController.java:664-675`。

---

### Q3：当前登录是否应该增加 networkId / nodeId 维度？
> **明确回答**：**必须增加。**  
> **标注**：【推断与架构收敛确认】  
> **依据**：资源主键依赖链为 `Server -> Network -> Node`。在允许不同 Network 存在同名 Node 的架构前提下，仅凭全局 `nodeName` 必然产生投影歧义，必须引入 `networkId`（或全局唯一的 `nodeId`）形成确定性定位复合键。

---

### Q4：登录成功后的 nodeId/networkId 是否已经足够建立稳定 Profile？
> **明确回答**：**完全足够。**  
> **标注**：【已验证】  
> **依据**：客户端 `crates/vpn-core/src/keys.rs:9-15` 的 `profile_id` 算法在收到 `network_id` 与 `node_id` 后，会生成 `Sha256(origin + "\0id\0" + network_id + "\0" + node_id)`。该哈希在全局绝对唯一且不随节点重命名改变，足以建立高可靠的稳定 Profile。

---

### Q5：同一 Access Server 下多个 Network 是否同时拥有独立 Runtime？
> **明确回答**：**是，同时拥有且完全独立。**  
> **标注**：【源码确认】  
> **依据**：`LocalTincRuntimeManager.java:319-328, 526-532` 中明确每个网络独立拥有 `/etc/tinc/<netName>`、独立 `tinc@<netName>.service`、独立虚拟网卡 Interface 以及独立端口 Port。冲突检测机制强制排除了同一服务器复用端口与接口。

---

### Q6：客户端从同一 Access Server 的 Network-a 切到 Network-b，服务端是否需要切换虚拟网卡？
> **明确回答**：**完全不需要。**  
> **标注**：【源码确认】  
> **依据**：服务端的 `tinc_a` 和 `tinc_b` 网卡本来就是两个独立服务进程并发运行、常驻系统的。客户端切换只涉及客户端本机 TAP 网卡的 IP/路由清理与重新连接，服务端不发生任何网卡切换。

---

### Q7：客户端从 Access-A 切换到 Access-B 时，本地哪些配置会切换？
> **明确回答**：本地会切换以下 7 项配置：  
> 1. 本地 Tinc 子进程杀死并重启；  
> 2. 本地 TAP 虚拟网卡绑定的 IPv4 地址从 Node-A 的 IP 变为 Node-B 的 IP；  
> 3. 本地前往网关主机的 `/32` 精确路由由 Access-A 的 VPN IP 切换为 Access-B 的 VPN IP；  
> 4. `active.json` 指针切换到新 Profile 的哈希与新 generation 目录；  
> 5. 配置文件目录从旧 `generations/gen-old` 切换至新 `generations/gen-new`（内含 Access-B 的外网 Address、端口和主机公钥）；  
> 6. 本地私钥上下文切换至 `keys/<new_profile_id>/rsa_key.priv`；  
> 7. 连接探针目标 IP 切换至 Access-B 网络的 `server_master`。  
> **标注**：【源码确认】  
> **依据**：`crates/vpn-core/src/engine.rs:352-360, 584-674, 855-870`。

---

### Q8：当前实现是否存在登录歧义或 Profile 串线风险？
> **明确回答**：  
> 1. **登录阶段：存在极高严重级别的登录歧义风险（P0）**，表现为跨网络同名同密节点瘫痪、同名异密节点由密码决定进入哪个网络。  
> 2. **Profile 存储与运行阶段：不存在串线风险**。因为一旦登录成功拿到 `network_id` 和 `node_id`，客户端会依靠严格的 SHA-256 哈希完全物理隔离密钥与配置，且切换时有严格的先清理、后挂载网卡机制。  
> **标注**：【已验证】  
> **依据**：前后端全链路比对结果。

---
*报告生成完毕，本轮未改动任何源码或系统配置。*
