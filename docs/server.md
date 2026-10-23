# Linux 服务器部署与维护

服务器端使用静态 Rust 程序，可在无图形环境运行。当前 GitHub 仅发布源码，请先按[构建文档](build.md)获得服务器程序，再使用本地文件部署；在线下载入口需要将来的正式发行附件。

## 组件模式

| 模式 | 管理入口 | 后台行为 |
|---|---|---|
| `all` | WebUI + TUI | 常驻认证与重连，按设置监听 HTTP |
| `webui` | WebUI | 常驻认证与重连，按设置监听 HTTP |
| `tui` | TUI | 常驻认证与重连，不监听 HTTP |
| `core` | 命令行 | 常驻认证与重连，不监听 HTTP |

systemd 管理后台进程。TUI 是管理入口，退出终端不会结束已安装的后台服务；手动运行 `watch` 而未安装服务时，进程仍受当前会话的生命周期影响。

## 使用本地程序安装

以下以 x64 构建产物为例，从仓库根目录执行。ARM64 应换为对应架构程序：

```bash
# 只预览，不改系统
bash deploy/linux/install_linux.sh \
  --binary "$PWD/target/x86_64-unknown-linux-musl/release/campus-auth" \
  --components all --dry-run

# 应用安装
sudo bash deploy/linux/install_linux.sh \
  --binary "$PWD/target/x86_64-unknown-linux-musl/release/campus-auth" \
  --components all
```

在交互终端省略 `--components` 时，安装器会提供中文组件选择。目标系统需要 systemd；安装需要 root 权限，运行中的认证服务默认使用专用非 root 用户。

默认路径与服务名：

| 项目 | 默认值 |
|---|---|
| 程序 | `/usr/local/lib/campus-auth/campus-auth` |
| 数据 | `/var/lib/campus-auth` |
| 服务 | `campus-auth.service` |
| WebUI | `127.0.0.1:8765`，仅 `all` / `webui` 模式 |

## 首次配置与管理密码

通过本机 TUI 选择学校、填写账号与服务、检查认证参数。自动重连默认关闭，配置完成后明确开启：

```bash
sudo /usr/local/lib/campus-auth/campus-auth \
  --data-dir /var/lib/campus-auth tui
```

WebUI 使用单个管理员密码，不提供注册。请在服务器本机交互设置密码：

```bash
sudo /usr/local/lib/campus-auth/campus-auth \
  --data-dir /var/lib/campus-auth password set
```

密码至少 12 个字符，以 Argon2id 哈希保存。使用 `password reset` 重置后，已有网页登录会话失效。密码不作为命令行参数传入。

TUI、WebUI 与命令行需使用同一个数据目录。账号启停、顺序和重连设置写入共享配置，后台读取后生效。

## 访问 WebUI

推荐保留本机监听，通过 SSH 转发访问：

```bash
ssh -L 8765:127.0.0.1:8765 user@server
```

随后在管理电脑打开 `http://127.0.0.1:8765/`。只有服务器产品提供网页管理；桌面产品直接显示应用窗口。

需要局域网明文 HTTP 时，必须明确使用 `--host 0.0.0.0 --allow-insecure-lan`，并接受密码在该 HTTP 连接中明文传输的条件。服务限制私网客户端；不能把此模式用于公网。

公网访问应使用 HTTPS 反向代理并指定 `--public-url https://你的管理域名`。Host 与 Origin 必须符合配置，应用不接受任意转发头替代校验。反向代理证书和公网防火墙由部署环境配置。

## 常用管理命令

```bash
systemctl status campus-auth.service
journalctl -u campus-auth.service -n 80

/usr/local/lib/campus-auth/campus-auth --data-dir /var/lib/campus-auth status
/usr/local/lib/campus-auth/campus-auth --data-dir /var/lib/campus-auth accounts
/usr/local/lib/campus-auth/campus-auth --data-dir /var/lib/campus-auth auth
/usr/local/lib/campus-auth/campus-auth --data-dir /var/lib/campus-auth reconnect
```

后四条命令需以有权读取数据目录的用户运行。账号密码只在本机文件中保存，查询账号不会返回密码。`auth --force` 仍保护现有在线连接，`logout` 会主动断开会话。

## 更新、回滚与卸载

使用自行构建、核对过架构和哈希的新程序更新：

```bash
sudo /完整路径/新版本campus-auth service update --apply
sudo /usr/local/lib/campus-auth/campus-auth service rollback --apply
```

更新保留组件模式与监听配置，备份上一版程序和服务配置；启动失败时尝试恢复。需要切换组件模式时，重新运行 `service install --components 新模式 --apply`。

默认卸载保留账号、管理密码、设置与日志：

```bash
sudo /usr/local/lib/campus-auth/campus-auth service uninstall --apply
```

明确清除已知 CampusAuth 数据时添加 `--purge`。清理根据部署记录确认归属，未知文件保留。卸载只处理本程序所属服务和电源计划。

## 可选电源计划

电源计划与认证独立，默认关闭。通过本机 TUI 预览、保存方案，再明确授权安装系统任务；WebUI 可以查看倒计时和取消本次动作。

```bash
sudo /usr/local/lib/campus-auth/campus-auth \
  --data-dir /var/lib/campus-auth power preview
sudo /usr/local/lib/campus-auth/campus-auth \
  --data-dir /var/lib/campus-auth power countdown
sudo /usr/local/lib/campus-auth/campus-auth \
  --data-dir /var/lib/campus-auth power cancel
```

安装或删除任务需要 root 权限。默认提前 60 秒倒计时，可取消本次而保留下次计划；错过的关机不补执行。完全关机后的开机需要固件或外部 WOL 支持，系统计划注册成功不代表硬件唤醒已经验证。

## 诊断

TUI 和 WebUI 支持导出诊断；命令行可运行 `diagnostics`。导出内容包括版本、学校设置、脱敏账号和近期日志，隐藏密码、管理凭据、令牌、账号标识及 IP，不包含任意环境变量或完整配置文件转储。
