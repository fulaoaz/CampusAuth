# 构建与安装包设计

本页面向开发者。普通用户可直接从 [GitHub Release](https://github.com/fulaoaz/CampusAuth/releases/latest) 下载对应安装包；以下步骤用于从源码构建相同类型的产物。

## 工程关系

仓库包含两个 Rust crate，分别保留 `Cargo.lock`：

- 根目录 `campus-auth`：共用认证库、服务器、TUI、命令行及系统服务管理。
- `desktop/` 中的 `campus-auth-desktop`：Tauri 桌面程序，通过路径依赖直接使用根目录库。

`ui/` 生成静态中文界面：服务器将其嵌入二进制，桌面通过 `frontendDist` 打包。先运行 `npm ci` 与 `npm run build`，才能得到完整界面；开发依赖由各目录的 `package-lock.json` 固定。

成品无需用户安装 Rust、Node.js、Python 或编译工具。开发与打包机器需要对应工具链、平台开发库，以及首次下载依赖的网络。

## Windows 桌面端

构建环境：

- Windows x64。
- Rust 稳定版 `stable-x86_64-pc-windows-msvc`；本轮实际构建使用 Rust 1.99.1。
- Visual Studio 2022 C++ Build Tools、MSVC x64 编译器和 Windows SDK。
- 可运行 Vite 7 的 Node.js；建议使用支持的 Node.js LTS 版本。

从仓库根目录执行：

```powershell
rustup toolchain install stable-x86_64-pc-windows-msvc --profile minimal
cd ui
npm ci
cd ../desktop
npm ci
npm run build:windows
```

`build:windows` 构建一次应用，再生成两个中文 NSIS 安装包，默认输出到 `.build-work/desktop-release/` 并写入 SHA-256 清单：

| 产物 | 运行时处理 |
|---|---|
| `CampusAuth-<版本>-windows-x64-setup.exe` | 检测系统 WebView2；已有时跳过，缺少时中文询问是否联网获取微软官方运行时 |
| `CampusAuth-<版本>-windows-x64-full-offline.exe` | 内嵌完整 WebView2 离线安装程序；已有运行时时跳过，不主动重装或降级 |

也可以分别运行 `npm run build:standard`、`npm run build:offline`。正式安装包注册系统卸载入口，卸载时清理本应用的自启与所属电源任务，并让用户决定是否删除数据。

### 复用已核验的离线运行时

Tauri 打包完整离线版时会查询微软下载重定向。若构建机已保存完整离线安装程序，但该查询失败，可先构建标准版，再运行：

```powershell
cd desktop
.\scripts\bundle-offline-cached.ps1 `
  -RuntimeInstaller "C:\构建资料\MicrosoftEdgeWebView2RuntimeInstallerX64.exe" `
  -RuntimeSha256 "由你事先核对的64位SHA256"
```

脚本要求有效的微软签名及精确哈希，复用 Tauri 生成的 NSIS 离线安装分支，在私有副本中设置与标准包一致的包类型标记，并生成打包来源记录。这里的哈希参数是占位说明，必须替换为实际文件的核验值。

## Linux 桌面端

现有桌面基线为 Ubuntu 22.04 x64，运行验证也覆盖 Ubuntu 24.04。请在最旧支持基线上构建，避免较新 glibc 或系统库要求进入产物。

构建机需准备 Rust 稳定版、Node.js、C/C++ 构建工具、`pkg-config`、musl 编译工具，以及 GTK3、WebKitGTK 4.1、AppIndicator、librsvg 等开发组件和 `patchelf`。这些是开发环境要求，完整 AppImage 会打包应用运行库。

```bash
rustup target add x86_64-unknown-linux-musl
cd ui
npm ci
cd ../desktop
npm ci
npm run build:linux
```

该命令按顺序执行：

1. `build-power-helper.sh` 构建 musl 静态命令行辅助程序，供需要系统权限的电源计划使用。
2. `prepare-appimage-libs.sh` 准备 `libfribidi`、`libharfbuzz`、`libgraphite2`，补齐打包工具默认可能排除的文字渲染库，并带上对应版权声明。
3. Tauri 构建界面、应用和 DEB / AppImage，资源中包含辅助程序与许可文件。

产物位于 `desktop/target/release/bundle/`。如果设置了 `CARGO_TARGET_DIR`，输出位置随该变量变化。

| 产物 | 依赖策略 |
|---|---|
| 标准 `.deb` | 使用发行版 GTK / WebKitGTK；通过 apt 安装时由软件源处理依赖，缺包且离线时不能保证安装完成 |
| 完整 `.AppImage` | 携带 GTK / WebKitGTK 等应用库；仍需要兼容的 glibc、显示环境、基础图形库及驱动 |

无 FUSE 环境使用仓库附带入口：

```bash
chmod +x CampusAuth.AppImage
sh desktop/scripts/run-appimage.sh ./CampusAuth.AppImage
```

入口采用 AppImage 的提取运行方式。它解决 FUSE 执行条件，不改变 CPU 架构、glibc、X11 / Wayland 或显卡兼容要求。

通过 Tauri CLI 构建会启用内嵌资源协议。若直接调用 Cargo 编译桌面 crate，生产构建需指定 `--features custom-protocol`，不能把依赖开发服务器的构建作为离线成品。

## Linux 服务器端

服务器不依赖 GTK、WebKitGTK 或桌面窗口。先构建共享界面，再生成对应 CPU 架构的静态程序：

```bash
cd ui
npm ci
npm run build
cd ..

rustup target add x86_64-unknown-linux-musl
cargo build --release --locked --target x86_64-unknown-linux-musl
```

程序位于 `target/x86_64-unknown-linux-musl/release/campus-auth`。也可使用 `bash tools/build.sh server` 输出到 `dist/`。

ARM64 使用 `aarch64-unknown-linux-musl`，并需配置对应 musl 交叉编译器与链接器；仅运行 `rustup target add` 不会安装 C 交叉工具链。准备好交叉环境后可运行 `bash tools/build.sh server-arm64`。ARM64 桌面安装包不属于当前构建目标。

将程序带到目标服务器后，使用 [服务器部署文档](server.md)中的 `--binary` 方式安装，无需目标机器联网下载应用依赖。

## 交付前核对

检查版本、目标架构、最终文件哈希、界面资源和许可文件；在目标系统执行安装、启动、退出与卸载。Linux 完整包还应在缺少系统 GTK / WebKitGTK 和 FUSE 的受支持环境验证。已完成的实际检查及限制见[验证记录](validation.md)。
