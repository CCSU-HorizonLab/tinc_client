# 服务端部署说明

## 组件与建议环境

- Java：源码目标 Java 8；本次 Maven 构建由 JDK 21.0.9 执行并生成可运行 JAR。
- MySQL：建议 8.0；初始化脚本使用 MySQL 语法。
- Redis：建议 6.x 或 7.x。
- Tinc：测试网关使用 Tinc 1.0 系列，网络 `new_network`，TCP/UDP 600。
- HTTP：Spring Boot 8081；示例 Nginx 对外监听 83，并原样代理 `/api/`。

实际部署版本应以组织安全基线为准。不要把示例占位符当作生产密码。

## 部署步骤

1. 创建独立数据库，先导入 `sql/init-ruoyi.sql`，再导入 `sql/init-tinclink.sql`。导入前先审阅，禁止覆盖已有生产库。
2. 安装并启动 Redis。
3. 将 `ruoyi-admin.jar`、`application-example.yml` 放入 `/opt/tinclink/server/`。
4. 通过环境变量提供数据库密码和至少 32 字节随机 JWT 密钥；不要把真实值提交到仓库。
5. 按实际部署路径修改 `systemd/ruoyi-tinc-backend.service.example`，再复制到 `/etc/systemd/system/`。
6. 执行 `systemctl daemon-reload`、`systemctl enable --now ruoyi-tinc-backend`，检查日志和 8081 监听。
7. 部署 Web 管理端静态文件并启用 `nginx/nginx-example.conf`。
8. 确认网关 Tinc 服务处于 active/enabled，虚拟地址正确，并同时放行 TCP/UDP 600 及云安全组规则。

## 必需环境变量

- `DB_URL`：例如 `jdbc:mysql://127.0.0.1:3306/tinclink?...`。
- `DB_USERNAME`、`DB_PASSWORD`。
- `REDIS_HOST`、`REDIS_PORT`、`REDIS_PASSWORD`。
- `JWT_SECRET`：自行生成的强随机值。

## 当前运行方式说明

当前 Tinc 管理采用 `tinc.transport.mode=local`，后端与网关 Tinc 位于同一 Linux 主机，配置根目录为 `/etc/tinc`。示例 systemd 以 root 运行是为了与当前实现匹配；正式产品应改造为最小权限 helper 后再降低权限。不得在未完成权限设计前盲目改成普通用户，否则会导致写配置或重载失败。

## 验证

- `curl` 或前端请求登录接口得到明确 HTTP/业务结果。
- 网关 `tinc@new_network` 为 active/enabled。
- `ss` 可见 TCP/UDP 600。
- 客户端上传后服务端出现合法 `hosts/<sid>`，且只含节点 `/32` 与 PKCS#1 RSA 公钥。
- 客户端最终显示 running + connected，目标 `/32` 路由存在且 Ping 成功。

不得把“JAR 启动成功”或“tincd 进程存在”单独写成隧道连通成功。
