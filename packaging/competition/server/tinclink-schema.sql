-- TincLink 业务表结构（不含真实节点、口令、Token 或服务器数据）
SET NAMES utf8mb4;

CREATE TABLE IF NOT EXISTS `tinc_server` (
  `Id` int NOT NULL AUTO_INCREMENT COMMENT 'id',
  `server_name` varchar(255) NOT NULL DEFAULT '' COMMENT '服务器名称',
  `server_ip` varchar(255) NOT NULL DEFAULT '' COMMENT '服务器地址',
  `start_segment` varchar(255) NOT NULL DEFAULT '' COMMENT '起始网段',
  `end_segment` varchar(255) NOT NULL DEFAULT '' COMMENT '终止网段',
  `start_port` int NOT NULL DEFAULT 0 COMMENT '起始端口',
  `end_port` int NOT NULL DEFAULT 0 COMMENT '终止端口',
  `remark` varchar(500) DEFAULT NULL COMMENT '备注',
  `number` int NOT NULL DEFAULT 0 COMMENT '网络数量',
  `status` int NOT NULL DEFAULT 1 COMMENT '状态',
  `ssh_port` int DEFAULT 22 COMMENT 'SSH 端口',
  `ssh_user` varchar(64) DEFAULT 'root' COMMENT 'SSH 用户',
  `ssh_key_path` varchar(255) DEFAULT NULL COMMENT 'SSH 私钥路径',
  `ssh_password` varchar(128) DEFAULT NULL COMMENT 'SSH 密码',
  `create_by` varchar(64) DEFAULT '' COMMENT '创建者',
  `create_time` datetime DEFAULT CURRENT_TIMESTAMP COMMENT '创建时间',
  `update_by` varchar(64) DEFAULT '' COMMENT '更新者',
  `update_time` datetime DEFAULT NULL ON UPDATE CURRENT_TIMESTAMP COMMENT '更新时间',
  PRIMARY KEY (`Id`),
  UNIQUE KEY `uk_server_name_ip` (`server_name`,`server_ip`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS `tinc_network` (
  `id` int NOT NULL AUTO_INCREMENT,
  `create_by` varchar(64) NOT NULL DEFAULT '',
  `server_name` varchar(255) NOT NULL DEFAULT '',
  `network_name` varchar(255) NOT NULL DEFAULT '',
  `create_time` datetime NOT NULL DEFAULT CURRENT_TIMESTAMP,
  `update_by` varchar(64) DEFAULT '',
  `update_time` datetime DEFAULT NULL ON UPDATE CURRENT_TIMESTAMP,
  `port` varchar(255) NOT NULL DEFAULT '',
  `segment` varchar(255) NOT NULL DEFAULT '',
  `nodes` int NOT NULL DEFAULT 0,
  `network_status` varchar(255) NOT NULL DEFAULT '',
  `remark` varchar(500) DEFAULT NULL,
  PRIMARY KEY (`id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS `tinc_node` (
  `id` int NOT NULL AUTO_INCREMENT,
  `user_name` varchar(255) NOT NULL DEFAULT '',
  `device_id` varchar(255) NOT NULL DEFAULT '',
  `server_name` varchar(255) NOT NULL DEFAULT '',
  `network_name` varchar(255) NOT NULL DEFAULT '',
  `password` varchar(255) NOT NULL DEFAULT '',
  `node_name` varchar(255) NOT NULL DEFAULT '',
  `network_ip` varchar(255) NOT NULL DEFAULT '',
  `remark` varchar(500) DEFAULT NULL,
  `create_by` varchar(64) DEFAULT '',
  `create_time` datetime DEFAULT CURRENT_TIMESTAMP,
  `update_by` varchar(64) DEFAULT '',
  `update_time` datetime DEFAULT NULL ON UPDATE CURRENT_TIMESTAMP,
  `node_status` varchar(255) NOT NULL DEFAULT '',
  `status` varchar(255) NOT NULL DEFAULT '',
  PRIMARY KEY (`id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
