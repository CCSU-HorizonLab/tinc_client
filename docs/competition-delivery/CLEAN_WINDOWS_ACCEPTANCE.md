# 干净 Windows 10/11 验收单

验收前快照必须确认：没有 `TincClientAgent` 服务、没有 `%ProgramData%\TincClient`、没有本项目 TAP 网卡、没有旧版 TincLink/Tinc Connect。每一步保存截图或文本日志。

- [ ] 1. 校验安装包 SHA-256
- [ ] 2. 管理员安装成功
- [ ] 3. `TincClientAgent` 已注册且 Automatic/Running
- [ ] 4. `%ProgramData%\TincClient\settings.json` 存在，字段恰为 `data_dir/tinc_binary/interface/interface_prefix/owner`
- [ ] 5. Tinc 1.0.36 与 TAP 网卡可用
- [ ] 6. Tinc Connect 可启动
- [ ] 7. 使用最小权限比赛节点登录成功
- [ ] 8. 配置下载成功
- [ ] 9. ZIP 严格校验通过
- [ ] 10. RSA 私钥生成或复用成功
- [ ] 11. 公钥上传成功
- [ ] 12. 服务端 HUP/READY 成功
- [ ] 13. VPN 启动
- [ ] 14. TAP 地址正确
- [ ] 15. `10.0.11.1/32` 精确路由存在
- [ ] 16. Ping `10.0.11.1` 4/4
- [ ] 17. UI 显示 running + connected / 已验证连通
- [ ] 18. 关闭 UI 后 Agent 与 VPN 继续运行
- [ ] 19. 重开 UI 后状态恢复正确
- [ ] 20. 停止 VPN 后只清理本项目拥有的地址和路由
- [ ] 21. 卸载成功，保留用户网络数据的行为与说明一致
- [ ] 22. 重启后无残留服务错误
- [ ] 23. 重新安装成功
- [ ] 24. 再次登录、配置、启动和连通成功

需归档：安装截图、服务状态、客户端状态、路由、Ping、停止、卸载、重启、重装截图，以及 `%ProgramData%\TincClient\installer.log`。任何一项失败时不得改勾，安装包不得去掉候选标记。
