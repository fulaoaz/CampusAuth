use anyhow::{Context, Result, bail};
use campus_auth::{core::Core, power, server, tui};
use clap::{Parser, Subcommand};
use std::{path::PathBuf, sync::Arc};

#[derive(Parser)]
#[command(version, about = "CampusAuth 校园网认证服务器与命令行工具")]
struct Cli {
    #[arg(long, env = "CAMPUS_AUTH_HOME", global = true)]
    data_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// 持续认证后台，不启动 HTTP 服务
    Watch,
    /// 查询网络和门户状态（不提交登录）
    Status,
    /// 查询账号（不显示密码）
    Accounts,
    /// 认证；即使 force 也保护已在线会话
    Auth {
        #[arg(long)] account: Option<String>,
        #[arg(long)] force: bool,
    },
    /// 主动断开门户会话
    Logout,
    /// 运行一次断线检查
    Reconnect,
    /// 无图形服务器管理界面
    Tui,
    /// 服务器 WebUI 与自动重连
    Serve {
        #[arg(long, default_value = "127.0.0.1")] host: String,
        #[arg(long, default_value_t = 8765)] port: u16,
        #[arg(long, env = "CAMPUS_AUTH_TOKEN", hide_env_values = true)] token: Option<String>,
        #[arg(long)] public_url: Option<String>,
        #[arg(long)] allow_insecure_lan: bool,
    },
    /// systemd 安装、更新、状态、卸载；默认只预览
    Service {
        #[arg(value_parser = ["install", "update", "rollback", "status", "uninstall", "start", "stop", "restart"])] action: String,
        #[arg(long, default_value = "/usr/local/lib/campus-auth")] install_dir: PathBuf,
        #[arg(long, default_value = "/etc/systemd/system")] unit_dir: PathBuf,
        #[arg(long, default_value = "127.0.0.1")] host: String,
        #[arg(long, default_value_t = 8765)] port: u16,
        #[arg(long, env = "CAMPUS_AUTH_TOKEN", hide_env_values = true)] token: Option<String>,
        #[arg(long, default_value = "campus-auth")] user: String,
        #[arg(long)] apply: bool,
        #[arg(long)] no_start: bool,
        #[arg(long, default_value="all")] components: tui::Components,
        #[arg(long)] public_url: Option<String>,
        #[arg(long)] purge: bool,
        #[arg(long)] allow_insecure_lan: bool,
    },
    /// 可选系统电源计划；install/remove/run 需显式 --apply
    Power {
        #[arg(value_parser = ["preview", "install", "remove", "status", "run", "cancel", "countdown", "cleanup-owned"])] action: String,
        #[arg(long)] config: Option<PathBuf>,
        #[arg(long)] apply: bool,
    },
    /// 列出内置学校配置
    Profiles,
    /// 设置或重置服务器管理员密码
    Password {
        #[arg(default_value="set",value_parser=["set","reset"])] action: String,
    },
    /// 输出脱敏诊断报告
    Diagnostics,
    /// 主动检查正式版本更新
    CheckUpdate,
}

fn data_dir(cli: &Cli) -> Result<PathBuf> {
    if let Some(path) = &cli.data_dir { return Ok(path.clone()); }
    if matches!(cli.command, Some(Commands::Service { .. })) {
        return Ok(PathBuf::from("/var/lib/campus-auth"));
    }
    let dirs = directories::ProjectDirs::from("", "", "CampusAuth")
        .context("无法确定用户数据目录，请使用 --data-dir")?;
    Ok(dirs.data_local_dir().to_path_buf())
}

fn print_json(value: impl serde::Serialize) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let data = data_dir(&cli)?;
    match cli.command {
        None => tui::run(Arc::new(Core::new(&data)?)),
        Some(Commands::Profiles) => print_json(campus_auth::core::built_in_profiles()),
        Some(Commands::Password { action: _ }) => tui::set_admin_password(&data),
        Some(Commands::CheckUpdate) => print_json(campus_auth::updates::check()?),
        Some(Commands::Power { action, config, apply }) => {
            let config = config.unwrap_or_else(|| data.join("power_schedule.json"));
            let state = config.parent().context("配置路径缺少父目录")?;
            let cfg = power::load(&config)?;
            let result = match action.as_str() {
                "preview" => power::preview(state, &cfg)?,
                "install" => power::install(state, &cfg, apply)?,
                "remove" => power::remove(state, apply)?,
                "status" => power::status(state)?,
                "cancel" => power::cancel_pending(state)?,
                "countdown" => power::countdown_status(state)?,
                "cleanup-owned" => {if !apply {bail!("清理系统电源计划需要 --apply");}power::cleanup_owned(state)?},
                "run" => {
                    if !apply { bail!("执行电源动作需要 --apply"); }
                    power::run_scheduled(&config)?;
                    serde_json::json!({"ok":true})
                }
                _ => unreachable!(),
            };
            let success = result.get("ok").and_then(serde_json::Value::as_bool).unwrap_or(true);
            print_json(result)?;
            if !success { bail!("电源计划未应用，详情见上方结果"); }
            Ok(())
        }
        Some(Commands::Service { action, install_dir, unit_dir, host, port, token, user, apply, no_start, components, public_url, purge, allow_insecure_lan }) => {
            tui::service(&action, tui::ServiceOptions {
                data_dir: data, install_dir, unit_dir, host, port, token, user,
                dry_run: !apply, no_start, components, public_url, purge, allow_insecure_lan,
            })
        }
        Some(command) => {
            let core = Arc::new(Core::new(&data)?);
            match command {
                Commands::Status => print_json(core.status()),
                Commands::Accounts => print_json(core.accounts()?),
                Commands::Auth { account, force } => {
                    let result = core.authenticate(account.as_deref(), force)?;
                    let ok = result.ok;
                    print_json(result)?;
                    if !ok { bail!("认证未成功，详情见上方结果"); }
                    Ok(())
                }
                Commands::Logout => {
                    let result = core.logout()?;
                    let ok = result.ok;
                    print_json(result)?;
                    if !ok { bail!("注销未成功"); }
                    Ok(())
                }
                Commands::Reconnect => print_json(core.reconnect_tick()?),
                Commands::Tui => tui::run(core),
                Commands::Diagnostics => print_json(campus_auth::diagnostics::export(&core)?),
                Commands::Watch => tokio::runtime::Builder::new_current_thread().enable_all().build()?.block_on(server::run_background(core)),
                Commands::Serve { host, port, token, public_url, allow_insecure_lan } => {
                    tokio::runtime::Builder::new_multi_thread().worker_threads(2).max_blocking_threads(12).enable_all().build()?
                        .block_on(server::run(core, server::ServerOptions { host, port, token, public_url, allow_insecure_lan }))
                }
                _ => unreachable!(),
            }
        }
    }
}

fn main() {
    #[cfg(windows)]
    if std::env::args_os().len() > 1 {
        unsafe { windows_sys::Win32::System::Console::AttachConsole(u32::MAX); }
    }
    if let Err(error) = run() {
        eprintln!("CampusAuth: {error:#}");
        #[cfg(windows)]
        if std::env::args_os().len() == 1 {
            let message: Vec<u16> = format!("CampusAuth 启动失败：\n{error:#}\0").encode_utf16().collect();
            let title: Vec<u16> = "CampusAuth\0".encode_utf16().collect();
            unsafe { windows_sys::Win32::UI::WindowsAndMessaging::MessageBoxW(std::ptr::null_mut(), message.as_ptr(), title.as_ptr(), 0x10); }
        }
        std::process::exit(1);
    }
}
