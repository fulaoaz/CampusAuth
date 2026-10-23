//! Chinese terminal management. All account/configuration actions use the same Core as WebUI.
pub use crate::service_manager::{Components, ServiceOptions, render_unit, service};
use crate::{
    core::{Account, AccountInput, ConnectionConfig, Core, ReconnectSettings},
    power,
};
use anyhow::{Context, Result, bail};
use dialoguer::{Confirm, Input, MultiSelect, Password, Select, theme::ColorfulTheme};
use std::{
    fs,
    io::{self, IsTerminal},
    path::{Path, PathBuf},
    sync::Arc,
};

fn confirm(theme: &ColorfulTheme, prompt: impl Into<String>) -> Result<bool> {
    Ok(Confirm::with_theme(theme)
        .with_prompt(prompt)
        .default(false)
        .interact()?)
}
fn print(value: impl serde::Serialize) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

pub fn set_admin_password(data: &Path) -> Result<()> {
    if !io::stdin().is_terminal() {
        bail!(
            "管理密码只能在本机交互终端设置，请运行 campus-auth password set；密码不会作为命令参数或日志保存"
        );
    }
    let theme = ColorfulTheme::default();
    println!("管理员账号固定为 admin。设置后，所有已有网页登录会话立即失效。");
    let password = Password::with_theme(&theme)
        .with_prompt("新管理密码（至少 12 个字符）")
        .with_confirmation("再次输入管理密码", "两次密码不一致")
        .interact()?;
    crate::server_auth::set_password(data, &password)?;
    println!("管理密码已安全保存；请使用新密码登录 WebUI。");
    Ok(())
}
pub fn administrator_reset(data: &Path) -> Result<()> {
    set_admin_password(data)
}

fn account_labels(accounts: &[Account]) -> Vec<String> {
    accounts
        .iter()
        .enumerate()
        .map(|(i, a)| {
            format!(
                "{}. {} · {} · {}{}",
                i + 1,
                if a.label.is_empty() {
                    &a.username
                } else {
                    &a.label
                },
                a.username,
                a.service,
                if a.enabled { "" } else { "（已停用）" }
            )
        })
        .collect()
}
fn account_form(core: &Core, theme: &ColorfulTheme, account: Option<&Account>) -> Result<()> {
    let label: String = Input::with_theme(theme)
        .with_prompt("账号名称")
        .with_initial_text(account.map(|a| a.label.as_str()).unwrap_or(""))
        .allow_empty(true)
        .interact_text()?;
    let username: String = Input::with_theme(theme)
        .with_prompt("校园网用户名")
        .with_initial_text(account.map(|a| a.username.as_str()).unwrap_or(""))
        .interact_text()?;
    let password = Password::with_theme(theme)
        .with_prompt(if account.is_some() {
            "校园网密码（留空保留原密码）"
        } else {
            "校园网密码"
        })
        .allow_empty_password(account.is_some())
        .interact()?;
    let service: String = Input::with_theme(theme)
        .with_prompt("运营商 / 服务名称（留空使用学校配置）")
        .with_initial_text(account.map(|a| a.service.as_str()).unwrap_or(""))
        .allow_empty(true)
        .interact_text()?;
    let enabled = Confirm::with_theme(theme)
        .with_prompt("启用此账号参与认证和自动重连？")
        .default(account.is_none_or(|a| a.enabled))
        .interact()?;
    let note: String = Input::with_theme(theme)
        .with_prompt("备注（可留空）")
        .with_initial_text(account.map(|a| a.note.as_str()).unwrap_or(""))
        .allow_empty(true)
        .interact_text()?;
    core.save_account(AccountInput {
        id: account.map(|a| a.id.clone()),
        label,
        username,
        password,
        service,
        enabled,
        note,
    })?;
    println!("账号已保存。WebUI 和后台服务将读取同一配置。");
    Ok(())
}
fn manage_accounts(core: &Core, theme: &ColorfulTheme) -> Result<()> {
    loop {
        let accounts = core.accounts()?;
        if accounts.is_empty() {
            println!("还没有账号，请先添加。");
            return Ok(());
        }
        let Some(index) = Select::with_theme(theme)
            .with_prompt("选择账号（Esc 返回）")
            .items(&account_labels(&accounts))
            .interact_opt()?
        else {
            return Ok(());
        };
        let account = &accounts[index];
        let Some(action) = Select::with_theme(theme)
            .with_prompt(format!("管理 {}", account.username))
            .items(&[
                "编辑账号",
                "启用 / 停用",
                "上移优先级",
                "下移优先级",
                "连接此账号",
                "删除账号",
                "返回",
            ])
            .interact_opt()?
        else {
            continue;
        };
        match action {
            0 => account_form(core, theme, Some(account))?,
            1 => {
                core.save_account(AccountInput {
                    id: Some(account.id.clone()),
                    label: account.label.clone(),
                    username: account.username.clone(),
                    password: String::new(),
                    service: account.service.clone(),
                    enabled: !account.enabled,
                    note: account.note.clone(),
                })?;
                println!("账号状态已更新。");
            }
            2 | 3 => {
                let mut ids: Vec<_> = accounts.iter().map(|a| a.id.clone()).collect();
                let destination = if action == 2 {
                    index.saturating_sub(1)
                } else {
                    (index + 1).min(ids.len() - 1)
                };
                ids.swap(index, destination);
                core.reorder_accounts(&ids)?;
                println!("账号优先级已更新。");
            }
            4 => connect_account(core, theme, Some(&account.id))?,
            5 => {
                if confirm(theme, format!("确认删除账号 {}？", account.username))? {
                    core.delete_account(&account.id)?;
                    println!("账号已删除。");
                }
            }
            _ => return Ok(()),
        }
    }
}
fn connect_account(core: &Core, theme: &ColorfulTheme, id: Option<&str>) -> Result<()> {
    let mut params = serde_json::json!({"account_id":id,"force":false});
    let result = crate::api::dispatch(core, "connect", Some(params.clone()))?;
    if result["needs_confirmation"].as_bool() == Some(true) {
        println!(
            "{}",
            result["message"]
                .as_str()
                .unwrap_or("切换账号需要注销当前校园网会话。")
        );
        if confirm(
            theme,
            format!(
                "从 {} 切换到 {}？",
                result["current_username"].as_str().unwrap_or("当前账号"),
                result["target_username"].as_str().unwrap_or("新账号")
            ),
        )? {
            params["confirm_switch"] = true.into();
            print(crate::api::dispatch(core, "connect", Some(params))?)?;
        }
    } else {
        print(result)?;
    }
    Ok(())
}
fn reconnect(core: &Core, theme: &ColorfulTheme) -> Result<()> {
    let current = core.reconnect_settings()?;
    let enabled = Confirm::with_theme(theme)
        .with_prompt("启用自动重连（后台系统服务将持续检查）？")
        .default(current.enabled)
        .interact()?;
    let interval_seconds = Input::with_theme(theme)
        .with_prompt("检查间隔，单位秒（10–86400）")
        .default(current.interval_seconds)
        .interact_text()?;
    let fail_threshold = Input::with_theme(theme)
        .with_prompt("连续失败后重连的阈值（1–10）")
        .default(current.fail_threshold)
        .interact_text()?;
    core.set_reconnect(ReconnectSettings {
        enabled,
        interval_seconds,
        fail_threshold,
    })?;
    println!("自动重连设置已保存。退出 TUI 不会停止已安装的系统服务。");
    Ok(())
}
fn url_lines(theme: &ColorfulTheme, label: &str, current: &[String]) -> Result<Vec<String>> {
    let text: String = Input::with_theme(theme)
        .with_prompt(label)
        .with_initial_text(current.join(","))
        .allow_empty(true)
        .interact_text()?;
    Ok(text
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect())
}
fn connection_form(core: &Core, theme: &ColorfulTheme, mut config: ConnectionConfig) -> Result<()> {
    config.display_name = Input::with_theme(theme)
        .with_prompt("学校 / 网络名称")
        .with_initial_text(&config.display_name)
        .interact_text()?;
    config.gateway = Input::with_theme(theme)
        .with_prompt("认证网关，例如 http://10.0.0.1")
        .with_initial_text(&config.gateway)
        .interact_text()?;
    config.service = Input::with_theme(theme)
        .with_prompt("默认运营商 / 服务名称")
        .with_initial_text(&config.service)
        .allow_empty(true)
        .interact_text()?;
    config.user_group = Input::with_theme(theme)
        .with_prompt("用户组（可留空）")
        .with_initial_text(&config.user_group)
        .allow_empty(true)
        .interact_text()?;
    if Confirm::with_theme(theme)
        .with_prompt("编辑连通性探测、发现地址和重试参数？")
        .default(false)
        .interact()?
    {
        config.probe_urls = url_lines(theme, "连通性探测 URL（英文逗号分隔）", &config.probe_urls)?;
        config.discovery_urls = url_lines(
            theme,
            "认证发现 URL（英文逗号分隔）",
            &config.discovery_urls,
        )?;
        config.timeout_seconds = Input::with_theme(theme)
            .with_prompt("单次网络超时，单位秒（1–30）")
            .default(config.timeout_seconds)
            .interact_text()?;
        config.retry_per_account = Input::with_theme(theme)
            .with_prompt("每个账号重试次数（1–6）")
            .default(config.retry_per_account)
            .interact_text()?;
        config.retry_interval_seconds = Input::with_theme(theme)
            .with_prompt("重试间隔，单位秒（0–30）")
            .default(config.retry_interval_seconds)
            .interact_text()?;
    }
    config.password_encrypt = false;
    core.set_connection_config(config)?;
    println!("学校和网络配置已保存。");
    Ok(())
}
fn school(core: &Core, theme: &ColorfulTheme) -> Result<()> {
    let profiles = crate::core::built_in_profiles();
    let mut items: Vec<_> = profiles
        .iter()
        .map(|p| format!("选择预设：{}", p.name))
        .collect();
    items.extend([
        "编辑当前学校配置".into(),
        "查看当前配置".into(),
        "从文件导入配置".into(),
        "返回".into(),
    ]);
    let Some(choice) = Select::with_theme(theme)
        .with_prompt("学校 / 认证网络")
        .items(&items)
        .interact_opt()?
    else {
        return Ok(());
    };
    if choice < profiles.len() {
        connection_form(core, theme, profiles[choice].config.clone())?;
    } else if choice == profiles.len() {
        connection_form(core, theme, core.connection_config()?)?;
    } else if choice == profiles.len() + 1 {
        print(core.connection_config()?)?;
    } else if choice == profiles.len() + 2 {
        let path: String = Input::with_theme(theme)
            .with_prompt("配置文件的完整路径（留空取消）")
            .allow_empty(true)
            .interact_text()?;
        if !path.is_empty() {
            let config: ConnectionConfig = serde_json::from_slice(&fs::read(path)?)?;
            connection_form(core, theme, config)?;
        }
    }
    Ok(())
}
fn power_menu(core: &Core, theme: &ColorfulTheme) -> Result<()> {
    let data = core.data_dir();
    let draft = data.join("power_draft.json");
    loop {
        let Some(choice) = Select::with_theme(theme)
            .with_prompt("可选定时电源计划（系统操作需要在本机明确确认）")
            .items(&[
                "编辑计划草稿",
                "预览计划，不改变系统",
                "查看已安装计划和倒计时",
                "应用草稿到系统计划",
                "删除 CampusAuth 电源计划",
                "取消当前倒计时",
                "返回",
            ])
            .interact_opt()?
        else {
            return Ok(());
        };
        let config_path = if draft.exists() {
            draft.clone()
        } else {
            data.join("power_schedule.json")
        };
        let mut cfg = power::load(&config_path)?;
        match choice {
            0 => {
                cfg.enabled = Confirm::with_theme(theme)
                    .with_prompt("启用定时电源计划？")
                    .default(cfg.enabled)
                    .interact()?;
                cfg.shutdown_time = Input::with_theme(theme)
                    .with_prompt("执行时间（HH:MM）")
                    .with_initial_text(&cfg.shutdown_time)
                    .interact_text()?;
                cfg.wake_time = Input::with_theme(theme)
                    .with_prompt("唤醒时间（HH:MM）")
                    .with_initial_text(&cfg.wake_time)
                    .interact_text()?;
                let days = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];
                let labels = [
                    "星期一",
                    "星期二",
                    "星期三",
                    "星期四",
                    "星期五",
                    "星期六",
                    "星期日",
                ];
                let defaults: Vec<_> = days
                    .iter()
                    .map(|d| cfg.days.iter().any(|value| value == d))
                    .collect();
                let selected = MultiSelect::with_theme(theme)
                    .with_prompt("执行日（空格切换，回车确认）")
                    .items(&labels)
                    .defaults(&defaults)
                    .interact()?;
                cfg.days = selected
                    .into_iter()
                    .map(|index| days[index].into())
                    .collect();
                let action = Select::with_theme(theme)
                    .with_prompt("定时动作")
                    .items(&["睡眠", "休眠", "关机"])
                    .default(match cfg.shutdown_action.as_str() {
                        "hibernate" => 1,
                        "shutdown" => 2,
                        _ => 0,
                    })
                    .interact()?;
                cfg.shutdown_action = ["suspend", "hibernate", "shutdown"][action].into();
                cfg.shutdown_delay_seconds = Input::with_theme(theme)
                    .with_prompt("执行前倒计时，单位秒（0–3600）")
                    .default(cfg.shutdown_delay_seconds)
                    .interact_text()?;
                let method = Select::with_theme(theme)
                    .with_prompt("唤醒方式")
                    .items(&["系统唤醒计时器（睡眠 / 休眠）", "RTC（需要硬件及固件支持）"])
                    .default(usize::from(cfg.wake_method == "rtc"))
                    .interact()?;
                cfg.wake_method = if method == 0 { "timer" } else { "rtc" }.into();
                power::save(&draft, &cfg)?;
                println!(
                    "草稿已保存，尚未改变系统计划。完整关机后的开机依赖 BIOS/UEFI RTC 或其他硬件支持。"
                );
            }
            1 => print(power::preview(data, &cfg)?)?,
            2 => {
                print(power::status(data)?)?;
                print(power::countdown_status(data)?)?;
            }
            3 => {
                print(power::preview(data, &cfg)?)?;
                if confirm(theme, "确认将以上计划应用到本机系统？需要 root 权限。")?
                {
                    print(power::install(data, &cfg, true)?)?;
                }
            }
            4 => {
                print(power::remove(data, false)?)?;
                if confirm(
                    theme,
                    "确认删除本机属于这个 CampusAuth 数据目录的电源计划？",
                )? {
                    print(power::remove(data, true)?)?;
                }
            }
            5 => {
                if confirm(theme, "确认取消当前尚未执行的电源倒计时？")? {
                    print(power::cancel_pending(data)?)?;
                }
            }
            _ => return Ok(()),
        }
    }
}
fn install_service(core: &Core, theme: &ColorfulTheme) -> Result<()> {
    let mut options = ServiceOptions::default();
    let choice = Select::with_theme(theme)
        .with_prompt("选择服务器组件")
        .items(&[
            "完整安装：WebUI + TUI + 后台认证",
            "WebUI + 后台认证",
            "TUI + 后台认证（无 HTTP）",
            "仅后台认证核心（无 HTTP）",
        ])
        .interact()?;
    options.components = [
        Components::All,
        Components::Webui,
        Components::Tui,
        Components::Core,
    ][choice];
    options.data_dir = PathBuf::from(
        Input::<String>::with_theme(theme)
            .with_prompt("后台服务共享数据目录")
            .default(options.data_dir.to_string_lossy().into_owned())
            .interact_text()?,
    );
    let copy = options.data_dir != core.data_dir()
        && Confirm::with_theme(theme)
            .with_prompt("将当前账号、学校和自动重连配置复制到服务数据目录？")
            .default(true)
            .interact()?;
    if options.components.has_webui() {
        options.port = Input::with_theme(theme)
            .with_prompt("本机 WebUI 端口")
            .default(options.port)
            .interact_text()?;
        if Confirm::with_theme(theme)
            .with_prompt("配置已有 HTTPS 反向代理的访问地址？")
            .default(false)
            .interact()?
        {
            options.public_url = Some(
                Input::with_theme(theme)
                    .with_prompt("外部 HTTPS 来源，例如 https://auth.example.com")
                    .interact_text()?,
            );
        }
        if Confirm::with_theme(theme).with_prompt("允许同一可信局域网内的设备访问 WebUI？（HTTP 将明文传输管理密码，推荐 HTTPS 代理）").default(false).interact()? {
            options.host = "0.0.0.0".into();
            options.allow_insecure_lan = true;
            println!("已选择局域网访问；仅接受私有网段客户端，仍请限制防火墙访问范围。");
        } else {
            println!("WebUI 只监听 127.0.0.1；远程访问请使用 HTTPS 反向代理或 SSH 隧道。");
        }
    }
    println!("{}", render_unit(&options)?);
    if !confirm(theme, "确认安装 / 更新上述系统服务？需要 root 权限。")? {
        return Ok(());
    }
    crate::service_manager::checked_path(&options.data_dir)?;
    if copy {
        fs::create_dir_all(&options.data_dir)?;
        for name in [
            "accounts.json",
            "native_config.json",
            "reconnect.json",
            "admin_auth.json",
        ] {
            let source = core.data_dir().join(name);
            let target = options.data_dir.join(name);
            if source.is_file()
                && (!target.exists() || confirm(theme, format!("目标中已存在 {name}，确认覆盖？"))?)
            {
                crate::service_manager::write_private(&target, &fs::read(&source)?, 0o600)?;
            }
        }
    }
    if options.components.has_webui() && !crate::server_auth::configured(&options.data_dir)? {
        set_admin_password(&options.data_dir)?;
    }
    service("install", options)
}
fn service_menu(core: &Core, theme: &ColorfulTheme) -> Result<()> {
    let Some(choice) = Select::with_theme(theme)
        .with_prompt("后台系统服务")
        .items(&[
            "安装 / 更改组件模式",
            "服务状态",
            "启动服务",
            "停止服务",
            "重启服务",
            "回滚到上一版",
            "卸载，保留账号和配置",
            "卸载并清除已知 CampusAuth 数据",
            "返回",
        ])
        .interact_opt()?
    else {
        return Ok(());
    };
    if choice == 0 {
        return install_service(core, theme);
    }
    if choice >= 8 {
        return Ok(());
    }
    let mut options = ServiceOptions::default();
    options.data_dir = PathBuf::from(
        Input::<String>::with_theme(theme)
            .with_prompt("服务数据目录")
            .default(if core.data_dir().starts_with("/var/lib") {
                core.data_dir().to_string_lossy().into_owned()
            } else {
                options.data_dir.to_string_lossy().into_owned()
            })
            .interact_text()?,
    );
    let action = [
        "",
        "status",
        "start",
        "stop",
        "restart",
        "rollback",
        "uninstall",
        "uninstall",
    ][choice];
    options.purge = choice == 7;
    if choice == 1
        || confirm(
            theme,
            if options.purge {
                "确认卸载并清除已知应用数据？此操作会删除保存的账号和管理密码。"
            } else {
                "确认执行此服务操作？"
            },
        )?
    {
        service(action, options)?;
    }
    Ok(())
}
pub fn run(mut core: Arc<Core>) -> Result<()> {
    if !io::stdin().is_terminal() {
        bail!("TUI 需要交互式终端。自动部署请使用 service 命令，参数见 --help。");
    }
    let theme = ColorfulTheme::default();
    let deployment = Path::new("/usr/local/lib/campus-auth/deployment.json");
    if let Ok(value) = fs::read(deployment).and_then(|bytes| {
        serde_json::from_slice::<serde_json::Value>(&bytes).map_err(io::Error::other)
    }) {
        if let Some(data) = value["data_dir"].as_str() {
            if core.data_dir() != Path::new(data)
                && Confirm::with_theme(&theme)
                    .with_prompt(format!(
                        "检测到后台服务使用 {data}，切换到相同账号和配置目录？"
                    ))
                    .default(true)
                    .interact()?
            {
                core = Arc::new(Core::new(data)?);
            }
        }
    }
    loop {
        println!(
            "\nCampusAuth · 服务器管理\n共享数据目录：{}\n退出这个界面不会停止已安装的后台认证服务。",
            core.data_dir().display()
        );
        let Some(choice) = Select::with_theme(&theme)
            .with_prompt("选择操作")
            .items(&[
                "检查网络连接",
                "添加认证账号",
                "编辑、排序和管理账号",
                "连接校园网",
                "注销校园网",
                "设置自动重连",
                "学校和认证网络设置",
                "设置 / 重设 WebUI 管理密码",
                "后台系统服务",
                "可选定时电源计划",
                "查看日志",
                "导出脱敏诊断",
                "退出",
            ])
            .default(0)
            .interact_opt()?
        else {
            break;
        };
        let result: Result<()> = (|| match choice {
            0 => print(core.status()),
            1 => account_form(&core, &theme, None),
            2 => manage_accounts(&core, &theme),
            3 => connect_account(&core, &theme, None),
            4 => {
                if confirm(&theme, "确认注销当前校园网会话？")? {
                    print(core.logout()?)
                } else {
                    Ok(())
                }
            }
            5 => reconnect(&core, &theme),
            6 => school(&core, &theme),
            7 => set_admin_password(core.data_dir()),
            8 => service_menu(&core, &theme),
            9 => power_menu(&core, &theme),
            10 => {
                for line in core.logs() {
                    println!("{line}");
                }
                Ok(())
            }
            11 => {
                let value = crate::api::dispatch(&core, "diagnostics/export", None)?;
                let filename = value["filename"]
                    .as_str()
                    .unwrap_or("campus-auth-diagnostics.json");
                let path = std::env::current_dir()?.join(filename);
                fs::write(&path, value["content"].as_str().context("诊断内容无效")?)?;
                println!("已导出脱敏诊断：{}", path.display());
                Ok(())
            }
            _ => Ok(()),
        })();
        if choice >= 12 {
            break;
        }
        if let Err(error) = result {
            eprintln!("操作失败：{error:#}");
        }
    }
    Ok(())
}
