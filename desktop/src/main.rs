#![cfg_attr(windows, windows_subsystem = "windows")]

use anyhow::{Context, Result, bail, ensure};
use campus_auth::{core::Core, power};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};
use tauri::{AppHandle, Emitter, Manager, RunEvent, WindowEvent};
use tauri_plugin_autostart::ManagerExt as AutostartExt;
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
struct Preferences {
    autostart: bool,
    close_to_tray: bool,
    minimize_to_tray: bool,
    close_notice_seen: bool,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            autostart: false,
            close_to_tray: true,
            minimize_to_tray: false,
            close_notice_seen: false,
        }
    }
}

struct DesktopState {
    core: Arc<Core>,
    preferences: Mutex<Preferences>,
    quitting: AtomicBool,
    tray_available: AtomicBool,
    worker_stop: Arc<(Mutex<bool>, Condvar)>,
    reconnect_menu: Mutex<Option<tauri::menu::CheckMenuItem<tauri::Wry>>>,
}

fn default_data_dir() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("CAMPUS_AUTH_HOME") {
        return Ok(PathBuf::from(path));
    }
    Ok(directories::ProjectDirs::from("", "", "CampusAuth")
        .context("无法确定用户数据目录，请指定 --data-dir")?
        .data_local_dir()
        .to_path_buf())
}

fn arg_value(args: &[String], name: &str) -> Option<PathBuf> {
    args.windows(2)
        .find(|pair| pair[0] == name)
        .map(|pair| PathBuf::from(&pair[1]))
}

fn save_preferences(data: &Path, value: &Preferences) -> Result<()> {
    use std::io::Write;
    fs::create_dir_all(data)?;
    let mut temp = tempfile::NamedTempFile::new_in(data)?;
    temp.write_all(&serde_json::to_vec_pretty(value)?)?;
    temp.as_file().sync_all()?;
    temp.persist(data.join("desktop_preferences.json"))
        .map_err(|e| e.error)?;
    Ok(())
}

fn load_preferences(data: &Path) -> Result<Preferences> {
    let path = data.join("desktop_preferences.json");
    if !path.exists() {
        return Ok(Preferences::default());
    }
    serde_json::from_slice(&fs::read(path)?)
        .context("桌面设置损坏，请修复 desktop_preferences.json 后重试")
}

fn show_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn stop_worker(state: &DesktopState) {
    state.quitting.store(true, Ordering::SeqCst);
    let (lock, condition) = &*state.worker_stop;
    if let Ok(mut stop) = lock.lock() {
        *stop = true;
        condition.notify_all();
    }
}

fn quit(app: &AppHandle) {
    stop_worker(&app.state::<DesktopState>());
    app.exit(0);
}

fn preferences(app: &AppHandle, patch: Option<Value>) -> Result<Value> {
    let state = app.state::<DesktopState>();
    let mut value = state
        .preferences
        .lock()
        .map_err(|_| anyhow::anyhow!("桌面设置正忙"))?;
    value.autostart = app.autolaunch().is_enabled().context("无法查询开机自启")?;
    if let Some(patch) = patch.filter(|v| !v.is_null()) {
        let obj = patch.as_object().context("设置必须是对象")?;
        for key in obj.keys() {
            ensure!(
                ["autostart", "close_to_tray", "minimize_to_tray"].contains(&key.as_str()),
                "未知桌面设置：{key}"
            );
        }
        let mut next = value.clone();
        for (key, target) in [
            ("autostart", &mut next.autostart),
            ("close_to_tray", &mut next.close_to_tray),
            ("minimize_to_tray", &mut next.minimize_to_tray),
        ] {
            if let Some(v) = obj.get(key) {
                *target = v.as_bool().context("桌面设置必须是布尔值")?;
            }
        }
        ensure!(
            !next.minimize_to_tray || state.tray_available.load(Ordering::SeqCst),
            "系统托盘不可用，无法启用最小化到托盘"
        );
        if obj.contains_key("autostart") {
            let old_enabled = app.autolaunch().is_enabled().context("无法查询开机自启")?;
            if next.autostart != old_enabled {
                if next.autostart {
                    app.autolaunch().enable()?;
                } else {
                    app.autolaunch().disable()?;
                }
            }
        }
        if let Err(error) = save_preferences(state.core.data_dir(), &next) {
            if next.autostart != value.autostart {
                let _ = if value.autostart {
                    app.autolaunch().enable()
                } else {
                    app.autolaunch().disable()
                };
            }
            return Err(error);
        }
        *value = next;
    }
    value.autostart = app.autolaunch().is_enabled().context("无法查询开机自启")?;
    let mut result = serde_json::to_value(&*value)?;
    result["tray_available"] = json!(state.tray_available.load(Ordering::SeqCst));
    Ok(result)
}

fn open_external(app: &AppHandle, params: Option<Value>) -> Result<Value> {
    let params = params.context("缺少链接")?;
    let value = params
        .get("url")
        .and_then(Value::as_str)
        .context("缺少链接")?;
    let url = url::Url::parse(value).context("链接格式不正确")?;
    let project = "/fulaoaz/CampusAuth";
    let permitted_host = url.host_str() == Some("fulao.cc")
        || (url.host_str() == Some("github.com")
            && (url.path() == project || url.path().starts_with(&format!("{project}/"))));
    ensure!(
        url.scheme() == "https"
            && permitted_host
            && url.port_or_known_default() == Some(443)
            && url.username().is_empty()
            && url.password().is_none(),
        "仅允许打开 CampusAuth 官方项目或作者博客链接"
    );
    app.opener().open_url(url.as_str(), None::<&str>)?;
    Ok(json!({"ok":true}))
}

fn dispatch_native(app: AppHandle, method: String, params: Option<Value>) -> Result<Value> {
    match method.as_str() {
        "app/preferences" => preferences(&app, params),
        "app/open-external" => open_external(&app, params),
        "app/quit" => {
            quit(&app);
            Ok(json!({"ok":true}))
        }
        "app/show" => {
            show_main(&app);
            Ok(json!({"ok":true}))
        }
        "app/power-elevate" => elevate_power(&app, params),
        "app/window/minimize" => {
            app.get_webview_window("main")
                .context("找不到应用窗口")?
                .minimize()?;
            Ok(json!({"ok":true}))
        }
        "app/window/maximize-toggle" => {
            let window = app.get_webview_window("main").context("找不到应用窗口")?;
            if window.is_maximized()? {
                window.unmaximize()?;
            } else {
                window.maximize()?;
            }
            Ok(json!({"ok":true,"maximized":window.is_maximized()?}))
        }
        "app/window/is-maximized" => Ok(
            json!({"ok":true,"maximized":app.get_webview_window("main").context("找不到应用窗口")?.is_maximized()?}),
        ),
        "app/window/close" => {
            app.get_webview_window("main")
                .context("找不到应用窗口")?
                .close()?;
            Ok(json!({"ok":true}))
        }
        "app/window/start-dragging" => {
            app.get_webview_window("main")
                .context("找不到应用窗口")?
                .start_dragging()?;
            Ok(json!({"ok":true}))
        }
        _ => {
            let state = app.state::<DesktopState>();
            let mut result = campus_auth::api::dispatch(&state.core, &method, params)?;
            if method == "app/info" {
                result["desktop"] = json!(true);
                result["mode"] = json!("desktop");
                result["tray_available"] = json!(state.tray_available.load(Ordering::SeqCst));
            }
            if method == "reconnect" || method == "disconnect" {
                if let Ok(settings) = state.core.reconnect_settings() {
                    if let Ok(item) = state.reconnect_menu.lock() {
                        if let Some(item) = item.as_ref() {
                            let _ = item.set_checked(settings.enabled);
                        }
                    }
                }
            }
            Ok(result)
        }
    }
}

#[tauri::command]
async fn dispatch(app: AppHandle, method: String, params: Option<Value>) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || dispatch_native(app, method, params))
        .await
        .map_err(|_| "应用操作异常终止".to_string())?
        .map_err(|error| format!("{error:#}"))
}

#[tauri::command]
async fn save_diagnostics(
    app: AppHandle,
    filename: String,
    content: String,
) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<Value> {
        ensure!(content.len() <= 8 * 1024 * 1024, "诊断内容过大");
        let safe_name = Path::new(&filename)
            .file_name()
            .and_then(|v| v.to_str())
            .filter(|v| !v.is_empty() && !v.contains(['\r', '\n', '\0']))
            .unwrap_or("CampusAuth-诊断.txt");
        let Some(destination) = app
            .dialog()
            .file()
            .set_title("导出 CampusAuth 诊断")
            .set_file_name(safe_name)
            .add_filter("诊断文件", &["txt", "json", "log"])
            .blocking_save_file()
        else {
            return Ok(json!({"ok":true,"canceled":true}));
        };
        let path = destination
            .into_path()
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        fs::write(&path, content.as_bytes()).context("无法保存诊断文件")?;
        Ok(json!({"ok":true,"canceled":false,"path":path}))
    })
    .await
    .map_err(|_| "诊断导出异常终止".to_string())?
    .map_err(|error| format!("{error:#}"))
}

fn elevate_power(app: &AppHandle, params: Option<Value>) -> Result<Value> {
    let params = params.unwrap_or_else(|| json!({}));
    let action = params
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or("install");
    ensure!(
        ["install", "remove"].contains(&action),
        "不支持的电源管理操作"
    );
    let state = app.state::<DesktopState>();
    let data = state.core.data_dir();
    let config = if action == "install" {
        data.join("pending_power_schedule.json")
    } else {
        data.join("power_schedule.json")
    };
    if action == "install" {
        power::load(&config)?;
        ensure!(config.is_file(), "请先预览并保存待应用的电源计划");
    }
    #[cfg(windows)]
    {
        let exe = std::env::current_exe()?;
        ensure!(
            !config.to_string_lossy().contains('"') && !data.to_string_lossy().contains('"'),
            "路径含无效字符"
        );
        let args = format!(
            "--data-dir \"{}\" power {action} --config \"{}\" --apply",
            data.display(),
            config.display()
        );
        run_elevated(&exe, &args)?;
        Ok(json!({"ok":true,"message":"系统电源计划已应用，请刷新查看。"}))
    }
    #[cfg(not(windows))]
    {
        // pkexec intentionally drops LD_LIBRARY_PATH. Launch the bundled static
        // helper so a full AppImage also works on hosts without GTK/WebKit.
        let helper = app
            .path()
            .resource_dir()?
            .join("resources/campus-auth-power-helper");
        ensure!(
            helper.is_file(),
            "缺少应用内的静态电源辅助程序，请重新安装完整软件包"
        );
        let status = std::process::Command::new("pkexec")
            .arg(&helper)
            .arg("--data-dir")
            .arg(data)
            .args(["power", action, "--config"])
            .arg(&config)
            .arg("--apply")
            .status()
            .context("无法启动系统授权窗口，请使用 sudo 应用电源计划")?;
        ensure!(status.success(), "未获得系统授权，电源计划未更改");
        Ok(json!({"ok":true,"requested":true,"message":"系统授权操作已完成，请刷新电源计划状态。"}))
    }
}

#[cfg(windows)]
fn run_elevated(exe: &Path, arguments: &str) -> Result<()> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, WAIT_OBJECT_0},
        System::Threading::{GetExitCodeProcess, WaitForSingleObject},
        UI::{
            Shell::{SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW},
            WindowsAndMessaging::SW_HIDE,
        },
    };
    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(Some(0)).collect()
    }
    let verb = wide("runas");
    let file = wide(&exe.to_string_lossy());
    let params = wide(arguments);
    let mut request: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    request.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
    request.fMask = SEE_MASK_NOCLOSEPROCESS;
    request.lpVerb = verb.as_ptr();
    request.lpFile = file.as_ptr();
    request.lpParameters = params.as_ptr();
    request.nShow = SW_HIDE;
    ensure!(
        unsafe { ShellExecuteExW(&mut request) } != 0,
        "未获得管理员授权，电源计划未更改"
    );
    ensure!(!request.hProcess.is_null(), "无法等待管理员操作结果");
    let result = (|| -> Result<()> {
        ensure!(
            unsafe { WaitForSingleObject(request.hProcess, 120_000) } == WAIT_OBJECT_0,
            "管理员操作仍在执行，请稍后刷新系统计划状态"
        );
        let mut code = 1u32;
        ensure!(
            unsafe { GetExitCodeProcess(request.hProcess, &mut code) } != 0 && code == 0,
            "管理员操作未完成，请查看系统计划状态和诊断日志"
        );
        Ok(())
    })();
    unsafe {
        CloseHandle(request.hProcess);
    }
    result
}

fn create_tray(app: &AppHandle) -> tauri::Result<()> {
    use tauri::{
        menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem},
        tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    };
    let open = MenuItem::with_id(app, "open", "打开 CampusAuth", true, None::<&str>)?;
    let connect = MenuItem::with_id(app, "connect", "立即连接校园网", true, None::<&str>)?;
    let enabled = app
        .state::<DesktopState>()
        .core
        .reconnect_settings()
        .map(|v| v.enabled)
        .unwrap_or(false);
    let reconnect =
        CheckMenuItem::with_id(app, "reconnect", "自动重连", true, enabled, None::<&str>)?;
    *app.state::<DesktopState>()
        .reconnect_menu
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = Some(reconnect.clone());
    let settings = MenuItem::with_id(app, "settings", "设置", true, None::<&str>)?;
    let exit = MenuItem::with_id(app, "quit", "退出 CampusAuth", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(
        app,
        &[&open, &connect, &reconnect, &separator, &settings, &exit],
    )?;
    let mut tray = TrayIconBuilder::with_id("campus-auth")
        .tooltip("CampusAuth 校园网认证")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => show_main(app),
            "settings" => {
                show_main(app);
                let _ = app.emit("campus-auth:navigate", json!({"page":"settings"}));
            }
            "quit" => quit(app),
            "connect" | "reconnect" => {
                let app = app.clone();
                let id = event.id.as_ref().to_owned();
                thread::spawn(move || {
                    let state = app.state::<DesktopState>();
                    let result = if id == "connect" {
                        state
                            .core
                            .authenticate(None, false)
                            .and_then(|value| serde_json::to_value(value).map_err(Into::into))
                    } else {
                        state.core.reconnect_settings().and_then(|mut value| {
                            value.enabled = !value.enabled;
                            state
                                .core
                                .set_reconnect(value)
                                .and_then(|v| serde_json::to_value(v).map_err(Into::into))
                        })
                    };
                    if let Ok(settings) = state.core.reconnect_settings() {
                        if let Ok(item) = state.reconnect_menu.lock() {
                            if let Some(item) = item.as_ref() {
                                let _ = item.set_checked(settings.enabled);
                            }
                        }
                    }
                    let _ = app.emit(
                        "campus-auth:changed",
                        result.unwrap_or_else(|e| json!({"ok":false,"message":e.to_string()})),
                    );
                });
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                show_main(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

fn on_window_event(window: &tauri::Window, event: &WindowEvent) {
    let state = window.state::<DesktopState>();
    match event {
        WindowEvent::CloseRequested { api, .. } if !state.quitting.load(Ordering::SeqCst) => {
            let mut prefs = state.preferences.lock().unwrap_or_else(|e| e.into_inner());
            if prefs.close_to_tray && state.tray_available.load(Ordering::SeqCst) {
                api.prevent_close();
                let first = !prefs.close_notice_seen;
                if first {
                    prefs.close_notice_seen = true;
                    if save_preferences(state.core.data_dir(), &prefs).is_err() {
                        prefs.close_notice_seen = false;
                    }
                }
                drop(prefs);
                if first {
                    let app = window.app_handle().clone();
                    app.dialog().message("关闭窗口后，CampusAuth 将继续留在系统托盘，按你的自动重连设置工作。\n\n双击托盘图标可以重新打开；选择托盘菜单“退出 CampusAuth”会结束应用及后台重连。可在设置中更改关闭行为。")
                        .title("CampusAuth 已留在托盘").show(move |_| { if let Some(w) = app.get_webview_window("main") { let _ = w.hide(); } });
                } else {
                    let _ = window.hide();
                }
            } else {
                stop_worker(&state);
            }
        }
        WindowEvent::Resized(_) => {
            let minimize = state
                .preferences
                .lock()
                .map(|p| p.minimize_to_tray)
                .unwrap_or(false);
            if minimize
                && state.tray_available.load(Ordering::SeqCst)
                && window.is_minimized().unwrap_or(false)
            {
                let _ = window.hide();
            }
        }
        _ => {}
    }
}

fn spawn_reconnect(app: AppHandle) {
    let state = app.state::<DesktopState>();
    let core = state.core.clone();
    let stop = state.worker_stop.clone();
    thread::spawn(move || {
        loop {
            let (lock, condition) = &*stop;
            let stopped = lock.lock().unwrap_or_else(|e| e.into_inner());
            if *stopped {
                break;
            }
            let (stopped, _) = condition
                .wait_timeout(stopped, Duration::from_secs(2))
                .unwrap_or_else(|e| e.into_inner());
            if *stopped {
                break;
            }
            drop(stopped);
            if core
                .reconnect_settings()
                .map(|v| v.enabled)
                .unwrap_or(false)
            {
                let result = core.reconnect_tick();
                if let Ok(value) = result {
                    if !value.skipped {
                        let _ = app.emit("campus-auth:reconnect", value);
                    }
                }
            }
        }
    });
}

fn power_cli(args: &[String], data: &Path) -> Result<bool> {
    let Some(index) = args.iter().position(|v| v == "power") else {
        return Ok(false);
    };
    let action = args
        .get(index + 1)
        .map(String::as_str)
        .context("缺少 power 子命令")?;
    let config = arg_value(args, "--config").unwrap_or_else(|| data.join("power_schedule.json"));
    let apply = args.iter().any(|v| v == "--apply");
    let value = match action {
        "preview" => power::preview(data, &power::load(&config)?)?,
        "install" => power::install(data, &power::load(&config)?, apply)?,
        "remove" => power::remove(data, apply)?,
        "cleanup-owned" => {
            ensure!(apply, "清理系统计划需要 --apply");
            power::cleanup_owned(data)?
        }
        "status" => power::status(data)?,
        "run" => {
            ensure!(apply, "执行电源动作需要 --apply");
            power::run_scheduled(&config)?;
            json!({"ok":true})
        }
        "cancel" => campus_auth::api::dispatch(&Core::new(data)?, "power/cancel", Some(json!({})))?,
        _ => bail!("不支持的 power 子命令"),
    };
    println!("{}", serde_json::to_string(&value)?);
    ensure!(
        value.get("ok").and_then(Value::as_bool).unwrap_or(true),
        "系统未接受电源计划，请查看详细结果"
    );
    Ok(true)
}

fn uninstall_cleanup(args: &[String], data: &Path) -> Result<bool> {
    if !args.iter().any(|v| v == "--uninstall-cleanup") {
        return Ok(false);
    }
    if args.iter().any(|v| v == "--remove-power-plan") {
        let installed = power::owns_schedule(data)?;
        if installed {
            #[cfg(windows)]
            {
                ensure!(
                    !data.to_string_lossy().contains('"'),
                    "应用数据路径含无效字符"
                );
                let command = format!(
                    "--data-dir \"{}\" power cleanup-owned --apply",
                    data.display()
                );
                run_elevated(&std::env::current_exe()?, &command)?;
            }
            #[cfg(not(windows))]
            {
                power::cleanup_owned(data)?;
            }
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("reg.exe")
            .args([
                "delete",
                "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run",
                "/v",
                "CampusAuth",
                "/f",
            ])
            .creation_flags(0x08000000)
            .output();
    }
    #[cfg(target_os = "linux")]
    {
        if let Some(base) = directories::BaseDirs::new() {
            let _ = fs::remove_file(base.config_dir().join("autostart/CampusAuth.desktop"));
        }
    }
    if args.iter().any(|v| v == "--remove-user-data") && data.exists() {
        // Deletion is restricted to the application-owned default data directory.
        // A CLI override must never turn uninstall into arbitrary directory removal.
        let expected = directories::ProjectDirs::from("", "", "CampusAuth")
            .context("无法确定应用数据目录")?
            .data_local_dir()
            .to_path_buf();
        ensure!(
            data.canonicalize()? == expected.canonicalize()?,
            "卸载清理只允许删除 CampusAuth 默认数据目录"
        );
        ensure!(
            !fs::symlink_metadata(data)?.file_type().is_symlink(),
            "应用数据路径不能是符号链接"
        );
        fs::remove_dir_all(data)?;
    }
    Ok(true)
}

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let data = arg_value(&args, "--data-dir").unwrap_or(default_data_dir()?);
    if power_cli(&args, &data)? || uninstall_cleanup(&args, &data)? {
        return Ok(());
    }
    let background = args.iter().any(|v| v == "--background");
    let core = Arc::new(Core::new(&data)?);
    let prefs = load_preferences(&data)?;
    let state = DesktopState {
        core,
        preferences: Mutex::new(prefs),
        quitting: AtomicBool::new(false),
        tray_available: AtomicBool::new(false),
        worker_stop: Arc::new((Mutex::new(false), Condvar::new())),
        reconnect_menu: Mutex::new(None),
    };
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            if !args.iter().any(|v| v == "--background") {
                show_main(app);
            }
        }))
        .plugin(
            tauri_plugin_autostart::Builder::new()
                .app_name("CampusAuth")
                .arg("--background")
                .build(),
        )
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(state)
        .invoke_handler(tauri::generate_handler![dispatch, save_diagnostics])
        .on_window_event(on_window_event)
        .setup(move |app| {
            #[cfg(target_os = "linux")]
            {
                let helper = app
                    .path()
                    .resource_dir()?
                    .join("resources/campus-auth-power-helper");
                if !helper.is_file() {
                    return Err(anyhow::anyhow!(
                        "缺少应用内的静态电源辅助程序，请重新安装完整软件包"
                    )
                    .into());
                }
                power::set_helper_executable(helper)?;
            }
            let available = create_tray(app.handle()).is_ok();
            app.state::<DesktopState>()
                .tray_available
                .store(available, Ordering::SeqCst);
            if !background || !available {
                show_main(app.handle());
            }
            spawn_reconnect(app.handle().clone());
            Ok(())
        })
        .build(tauri::generate_context!())
        .context("无法启动 CampusAuth 桌面应用")?;
    app.run(|app, event| {
        if matches!(event, RunEvent::Exit | RunEvent::ExitRequested { .. }) {
            stop_worker(&app.state::<DesktopState>());
        }
    });
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        let message = format!("CampusAuth 无法完成操作：\n{error:#}");
        eprintln!("{message}");
        if let Ok(data) = default_data_dir() {
            let _ = fs::create_dir_all(&data);
            let _ = fs::write(data.join("startup-error.log"), &message);
        }
        #[cfg(windows)]
        {
            use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
            let text: Vec<u16> = message.encode_utf16().chain(Some(0)).collect();
            let title: Vec<u16> = "CampusAuth 校园网认证"
                .encode_utf16()
                .chain(Some(0))
                .collect();
            // Scheduled commands cannot display interactive UI in session 0.
            if !std::env::args().any(|v| v == "power" || v == "--uninstall-cleanup") {
                unsafe {
                    MessageBoxW(
                        std::ptr::null_mut(),
                        text.as_ptr(),
                        title.as_ptr(),
                        MB_OK | MB_ICONERROR,
                    );
                }
            }
        }
        std::process::exit(1);
    }
}
