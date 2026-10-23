# CampusAuth

**校园网自动认证、断线重连与设备管理。**

[项目源码](https://github.com/fulaoaz/CampusAuth) · [下载 Release](https://github.com/fulaoaz/CampusAuth/releases/latest) · 作者 **fulao** · [个人博客](https://fulao.cc)

CampusAuth 是独立的校园网认证工具，内置青海大学锐捷 ePortal / SAM+ 配置，也支持自定义兼容的 ePortal 网关。认证核心使用 Rust，桌面端和 Linux 服务器端共用认证逻辑，提供完整中文界面。

源码仓库只包含当前实现与文档，安装包单独放在 [GitHub Release](https://github.com/fulaoaz/CampusAuth/releases/latest)。需要自行构建时，请参阅[构建文档](docs/build.md)。

## 两种使用方式

| 产品 | 适用设备 | 使用方式 |
|---|---|---|
| 桌面端 | Windows 10/11 x64、Linux x64 图形桌面 | 独立应用窗口、中文首次向导、系统托盘与可选开机自启 |
| 服务器端 | Linux x64 / ARM64 无图形设备 | 静态程序、TUI 终端管理、可选 WebUI、systemd 常驻服务 |

桌面使用 Tauri 2、React 和 Fluent UI，界面资源随应用提供，直接调用 Rust 核心，不启动本地 HTTP 服务。服务器端可以选择 WebUI、TUI 或纯后台模式；关闭浏览器、终端或 SSH 不影响已安装的后台服务。

## 主要功能

- **认证与账号**：添加、编辑、启停账号；默认按添加顺序尝试，也可拖动或使用按钮调整顺序；编辑时密码留空保留原值。
- **自动重连**：自定义检测间隔与失败阈值；备用账号成功后保持连接，不主动切回；手动切换在线账号先确认。
- **学校配置**：内置青海大学；自定义网关、运营商服务、用户组、发现地址与联网检测地址。
- **中文界面**：Fluent 2 浅色、深色与跟随系统主题；可收起、调宽的侧栏；文字大小 13–20 可调，默认 15，并保存偏好。
- **桌面常驻**：顶部整合窗口按钮；关闭到托盘后继续重连；“退出 CampusAuth”结束后台；开机自启默认关闭。
- **可选电源计划**：按星期安排睡眠、休眠、关机与唤醒；先预览后授权应用；倒计时内可取消本次动作。
- **管理与诊断**：服务器管理员密码、服务安装维护、日志与脱敏诊断导出；更新由用户主动检查。

## 下载与安装

在 [最新 Release](https://github.com/fulaoaz/CampusAuth/releases/latest) 选择对应文件：

| 文件 | 适合场景 |
|---|---|
| `windows-x64-setup.exe` | Windows 已有 WebView2，使用小体积标准安装包；缺少时会询问是否联网获取 |
| `windows-x64-full-offline.exe` | Windows 完整离线安装，内含 WebView2 安装组件，已有运行时则跳过 |
| `linux-amd64.deb` | Debian/Ubuntu 标准桌面包，系统依赖由 apt 处理 |
| `linux-amd64.AppImage` 或 `linux-amd64-portable.zip` | Linux 完整桌面包；ZIP 附带无 FUSE 启动入口 |
| `CampusAuth-server-<版本>-linux-amd64.tar.gz` | Linux x64 服务器离线部署包 |
| `CampusAuth-server-<版本>-linux-arm64.tar.gz` | Linux ARM64 服务器离线部署包 |

Windows 双击安装包即可。Linux DEB 使用 `sudo apt install ./文件名.deb`；AppImage 赋予执行权限后启动，无 FUSE 环境使用便携 ZIP 内的 `run-appimage.sh`。服务器包解压后运行 `sudo bash deploy/linux/install_linux.sh`。

Release 同时提供 `SHA256SUMS` 和 `VALIDATION.md`，分别用于文件校验与了解实际验证边界。安装包和运行时不放进 Git 源码历史。

## 从源码开始

### 构建 Windows 桌面端

开发机需要 Rust 稳定版 MSVC 工具链、Visual Studio C++ Build Tools 与 Node.js。以下命令从仓库根目录开始：

```powershell
rustup toolchain install stable-x86_64-pc-windows-msvc --profile minimal
cd ui
npm ci
cd ../desktop
npm ci
npm run build:windows
```

构建会生成标准安装包与完整离线安装包。标准版复用已有 WebView2，缺少时询问是否联网获取；完整版携带微软离线安装组件。成品用户不需要安装 Rust、Node.js、Python 或编译器。

Linux 桌面构建、依赖基线、无 FUSE 运行方式及打包细节见[构建文档](docs/build.md)。

### 构建并部署 Linux 服务器端

在具备对应 musl 编译工具的 Linux 开发机上，先构建共享界面，再构建服务器程序：

```bash
cd ui
npm ci
npm run build
cd ..
rustup target add x86_64-unknown-linux-musl
cargo build --release --locked --target x86_64-unknown-linux-musl

sudo bash deploy/linux/install_linux.sh \
  --binary "$PWD/target/x86_64-unknown-linux-musl/release/campus-auth"
```

安装器提供 `all`、`webui`、`tui`、`core` 四种组件模式，默认使用专用用户和 `campus-auth.service`。程序使用静态二进制，目标服务器不需要语言运行时；安装系统服务仍需 systemd 与相应权限。

管理员密码、SSH 转发、更新回滚和卸载见[服务器文档](docs/server.md)。

## 支持范围

当前实现锐捷 ePortal 明文密码表单协议。要求 RSA 加密的网关会明确报错；深澜、802.1X、Wi-Fi 密钥认证不在当前范围。不同学校的定制行为需要在目标校园网验证。

互联网连通状态与门户认证状态分别显示；没有响应时保留“未知”，普通 HTTP 200 拦截页不会直接视为联网成功。自定义联网检测地址应返回 HTTP 204。

完整离线桌面包携带应用依赖，仍需要受支持的操作系统、CPU 架构、Linux 图形环境和 glibc 基线。现有 Linux 桌面运行核验覆盖 Ubuntu 22.04 / 24.04，详细边界见[验证记录](docs/validation.md)。

睡眠与休眠唤醒依赖系统计时器和硬件。完全关机后的开机需要 BIOS/UEFI RTC 或外部 WOL，软件无法保证。

## 数据与隐私

账号密码和配置保存在当前设备，通过系统文件权限保护；公开接口不返回密码。导出诊断前展示范围，并隐藏密码、管理凭据、会话令牌、账号和 IP。没有账号云同步功能。

使用 `--data-dir` 或 `CAMPUS_AUTH_HOME` 指定数据目录。桌面默认使用当前用户的应用数据目录，已安装服务器服务默认使用 `/var/lib/campus-auth`。终端管理服务时应使用同一目录。

默认卸载保留账号、设置和日志；明确清理时仅处理本应用所属数据。电源计划与自动重连均默认关闭。

## 源码结构

```text
src/         共用认证核心、服务器、TUI、服务管理与电源计划
desktop/     Tauri 原生应用入口、安装器配置与打包脚本
ui/          桌面与 WebUI 共用的中文 React 界面
deploy/      Linux 服务安装与卸载入口
tools/       开发与维护脚本
examples/    无真实凭据的配置示例
assets/      图标、字体与许可资料
docs/        构建、部署、验证与界面设计文档
```

仓库不包含真实账号、运行日志、安装包、构建缓存和本机验证脚本。

## 文档

- [构建与安装包设计](docs/build.md)
- [Linux 服务器部署与维护](docs/server.md)
- [已完成验证与实际限制](docs/validation.md)
- [界面设计与交互约定](docs/ui-design.md)

## 许可

项目采用 [MIT License](LICENSE)。第三方组件与字体的许可见 [THIRD_PARTY_NOTICES.txt](THIRD_PARTY_NOTICES.txt) 和 [Noto Sans SC 字体许可](assets/NotoSansSC-OFL.txt)。
