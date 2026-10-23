# CampusAuth 桌面端

桌面端为 Tauri 2 应用，直接调用同仓库 Rust 核心，不启动本地 HTTP 服务，不启动浏览器。界面由 `ui/` 构建并编译到应用中。

开发环境需要 Rust、对应平台构建工具和 Node.js；发行后的应用不需要用户安装这些开发依赖。

```powershell
cd ui
npm ci
cd ../desktop
npm ci
npm run build:windows
```

Windows 标准安装包在已有 WebView2 时直接使用，缺少运行时时明确提示并联网下载 Microsoft 官方引导程序。完整离线安装包携带完整的 WebView2 离线安装程序，也会跳过已安装运行时。两者均为正式安装程序，可通过系统应用列表卸载。

`build:windows` 使用 MSVC 工具链构建一次应用，随后生成两种安装包，输出到 `.build-work/desktop-release/` 并生成 SHA-256 清单。也可以分别运行 `build:standard` 或 `build:offline`。系统需安装 Rust 的 `stable-x86_64-pc-windows-msvc` 工具链及 Visual Studio C++ Build Tools。

如果构建机已经缓存了微软官方 WebView2 完整离线安装程序，但 Tauri 的网络重定向查询失败，可以使用 `scripts/bundle-offline-cached.ps1 -RuntimeInstaller <文件路径> -RuntimeSha256 <预先核对的SHA256>`。脚本要求有效的微软签名和精确哈希，复用 Tauri 生成的 NSIS 离线安装逻辑，并设置与标准包一致的 NSIS 包类型标记，记录打包来源。

Linux 构建使用 `npm run build:linux`，先编译无 GTK/WebKit 依赖的 musl 静态辅助程序，再生成 DEB 和 AppImage。需要开发环境预先安装对应 Rust musl 目标和 musl 编译器；辅助程序只用于用户明确启用的系统电源计划。

Linux 的 DEB 使用发行版 WebKitGTK/GTK 依赖；AppImage 打包应用依赖，但仍需要兼容的 Linux 图形系统和 glibc 基线。无 FUSE 时使用 `scripts/run-appimage.sh` 自动提取运行。

开机自启与电源计划默认关闭。窗口关闭默认留在托盘，第一次关闭会说明；托盘“退出 CampusAuth”会结束应用及其自动重连线程。卸载会移除应用自己的自启项和电源计划，是否删除账号与设置由用户选择。
