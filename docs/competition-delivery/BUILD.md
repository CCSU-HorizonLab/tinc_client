# TincLink 源码构建

## Java 服务端

要求 Maven 3.9.x；源码目标 Java 8。本次验证使用 Maven 3.9.9 + JDK 21.0.9。

```powershell
cd server
mvn -B test package -DskipTests=false
```

产物：`ruoyi-admin/target/ruoyi-admin.jar`。

## Web 管理端

```powershell
cd web-admin
npm ci
npm run build:prod
```

产物：`dist/`。当前构建有资源体积警告，但构建成功。

## Windows 桌面端与 Rust Agent

要求 Node.js 22、Rust stable、Windows 10/11 x64、WebView2。回到包含根 `Cargo.toml` 和 `package.json` 的源码目录：

```powershell
npm ci
. .\scripts\dev\env.ps1
npm run test:ui
npm run test:packaging
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
npm run desktop:build:windows
```

Windows 构建脚本会校验固定版本的 Tinc 和 TAP 驱动，再生成 NSIS 安装包。联网下载只允许脚本中固定 URL 与固定 SHA-256 对应的依赖；也可通过脚本支持的环境变量提供已校验本地文件。

## 可复现性边界

- 服务端仓库当前包含未提交的比赛开发改动；本源码包是交付时工作树快照，其基线提交号记录在 `VERSION.txt`。
- Windows 安装包未做代码签名。
- 干净 Windows 24 项验收未完成前，构建产物只能标为候选版。
