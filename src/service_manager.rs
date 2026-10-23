//! Reversible deployment of the standalone, unprivileged Linux authentication service.
use anyhow::{Context, Result, bail, ensure};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Write},
    net::IpAddr,
    path::{Path, PathBuf},
    process::Command,
    str::FromStr,
};
use uuid::Uuid;

const SERVICE: &str = "campus-auth.service";
const MANIFEST: &str = "deployment.json";
const STATE_MARKER: &str = ".campus-auth-state";
const PROGRAM_FILES: [&str; 3] = ["campus-auth", "service.env", MANIFEST];

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Components {
    #[default]
    All,
    Webui,
    Tui,
    Core,
}
impl Components {
    pub fn has_webui(self) -> bool {
        matches!(self, Self::All | Self::Webui)
    }
    pub fn has_tui(self) -> bool {
        matches!(self, Self::All | Self::Tui)
    }
}
impl std::fmt::Display for Components {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::All => "all",
            Self::Webui => "webui",
            Self::Tui => "tui",
            Self::Core => "core",
        })
    }
}
impl FromStr for Components {
    type Err = anyhow::Error;
    fn from_str(value: &str) -> Result<Self> {
        match value {
            "all" => Ok(Self::All),
            "webui" => Ok(Self::Webui),
            "tui" => Ok(Self::Tui),
            "core" => Ok(Self::Core),
            _ => bail!("组件模式必须是 all、webui、tui 或 core"),
        }
    }
}
#[derive(Clone, Debug)]
pub struct ServiceOptions {
    pub data_dir: PathBuf,
    pub install_dir: PathBuf,
    pub unit_dir: PathBuf,
    pub host: String,
    pub port: u16,
    pub token: Option<String>,
    pub user: String,
    pub dry_run: bool,
    pub no_start: bool,
    pub components: Components,
    pub purge: bool,
    pub public_url: Option<String>,
    pub allow_insecure_lan: bool,
}
impl Default for ServiceOptions {
    fn default() -> Self {
        Self {
            data_dir: "/var/lib/campus-auth".into(),
            install_dir: "/usr/local/lib/campus-auth".into(),
            unit_dir: "/etc/systemd/system".into(),
            host: "127.0.0.1".into(),
            port: 8765,
            token: None,
            user: "campus-auth".into(),
            dry_run: false,
            no_start: false,
            components: Components::All,
            purge: false,
            public_url: None,
            allow_insecure_lan: false,
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
struct Deployment {
    application: String,
    version: String,
    data_dir: PathBuf,
    install_dir: PathBuf,
    unit_dir: PathBuf,
    host: String,
    port: u16,
    user: String,
    components: Components,
    public_url: Option<String>,
    #[serde(default)]
    allow_insecure_lan: bool,
}
impl Deployment {
    fn new(options: &ServiceOptions) -> Self {
        Self {
            application: "CampusAuth".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            data_dir: options.data_dir.clone(),
            install_dir: options.install_dir.clone(),
            unit_dir: options.unit_dir.clone(),
            host: options.host.clone(),
            port: options.port,
            user: options.user.clone(),
            components: options.components,
            public_url: options.public_url.clone(),
            allow_insecure_lan: options.allow_insecure_lan,
        }
    }
}

pub fn checked_path(path: &Path) -> Result<PathBuf> {
    ensure!(
        path.is_absolute()
            && !path
                .components()
                .any(|p| matches!(p, std::path::Component::ParentDir)),
        "系统服务路径必须是绝对路径，且不能包含 .."
    );
    ensure!(
        path != Path::new("/")
            && !path
                .to_string_lossy()
                .chars()
                .any(|ch| ch.is_control() || matches!(ch, '"' | '%' | '\\' | '$' | '\'')),
        "系统服务路径包含不支持的字符：{}",
        path.display()
    );
    for ancestor in path.ancestors() {
        if let Ok(meta) = fs::symlink_metadata(ancestor) {
            ensure!(
                !meta.file_type().is_symlink(),
                "系统服务路径不能经过符号链接：{}",
                ancestor.display()
            );
        }
    }
    Ok(path.to_owned())
}
fn unit_path(path: &Path) -> String {
    path.to_string_lossy().replace(' ', "\\x20")
}
fn quoted(path: &Path) -> String {
    format!("\"{}\"", path.display())
}
fn dedicated_paths(options: &ServiceOptions) -> Result<()> {
    let broad = [
        "/etc",
        "/usr",
        "/usr/local",
        "/usr/local/lib",
        "/var",
        "/var/lib",
        "/opt",
        "/tmp",
        "/var/tmp",
        "/home",
        "/root",
    ];
    for path in [&options.data_dir, &options.install_dir, &options.unit_dir] {
        checked_path(path)?;
    }
    ensure!(
        !broad
            .iter()
            .any(|p| options.data_dir == Path::new(p) || options.install_dir == Path::new(p)),
        "请选择 CampusAuth 专用子目录，不能直接使用系统目录"
    );
    let data = &options.data_dir;
    let install = &options.install_dir;
    let units = &options.unit_dir;
    ensure!(
        !data.starts_with(install)
            && !install.starts_with(data)
            && !units.starts_with(data)
            && !units.starts_with(install)
            && !data.starts_with(units)
            && !install.starts_with(units),
        "数据、程序和 unit 目录必须分开，不能相互包含"
    );
    ensure!(
        !data.starts_with("/home") && !data.starts_with("/root"),
        "系统服务的数据不能位于 /home 或 /root，请使用 /var/lib/campus-auth 等专用目录"
    );
    ensure!(
        !options.user.is_empty()
            && options.user != "root"
            && options
                .user
                .bytes()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == b'_' || ch == b'-'),
        "请选择有效的非 root 服务用户"
    );
    Ok(())
}
#[cfg(unix)]
fn protected_program_path(path: &Path) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    for ancestor in path.ancestors() {
        if let Ok(meta) = fs::symlink_metadata(ancestor) {
            ensure!(
                !meta.file_type().is_symlink() && meta.uid() == 0 && meta.mode() & 0o022 == 0,
                "服务程序必须位于 root 所有且其他用户不可写的目录：{}",
                ancestor.display()
            );
        }
    }
    Ok(())
}
#[cfg(not(unix))]
fn protected_program_path(_path: &Path) -> Result<()> {
    Ok(())
}

pub fn render_unit(options: &ServiceOptions) -> Result<String> {
    dedicated_paths(options)?;
    let ip: IpAddr = options.host.parse().context("监听 IP 无效")?;
    if options.components.has_webui() {
        ensure!(options.port != 0, "系统服务需要固定的非零端口");
        if let Some(public) = &options.public_url {
            let url = url::Url::parse(public).context("远程访问地址无效")?;
            ensure!(
                url.scheme() == "https"
                    && url.path() == "/"
                    && url.query().is_none()
                    && url.fragment().is_none()
                    && url.username().is_empty()
                    && url.password().is_none(),
                "远程访问地址必须是 HTTPS 来源，例如 https://auth.example.com"
            );
        }
        ensure!(
            ip.is_loopback() || options.allow_insecure_lan,
            "默认只监听本机；局域网监听需要明确使用 --allow-insecure-lan"
        );
    }
    let command = if options.components.has_webui() {
        let mut command = format!("serve --host {ip} --port {}", options.port);
        if options.allow_insecure_lan {
            command.push_str(" --allow-insecure-lan");
        }
        if let Some(public) = &options.public_url {
            command.push_str(&format!(
                " --public-url \"{}\"",
                url::Url::parse(public)?.origin().ascii_serialization()
            ));
        }
        command
    } else {
        "watch".into()
    };
    Ok(format!(
        "[Unit]\nDescription=CampusAuth 校园网自动认证\nWants=network-online.target\nAfter=network-online.target\n\n[Service]\nType=simple\nUser={}\nWorkingDirectory={}\nEnvironmentFile={}\nExecStart={} --data-dir {} {command}\nRestart=on-failure\nRestartSec=5\nTimeoutStopSec=35\nUMask=0077\nNoNewPrivileges=true\nPrivateTmp=true\nProtectSystem=strict\nProtectHome=true\nReadWritePaths={}\nProtectKernelTunables=true\nProtectKernelModules=true\nProtectControlGroups=true\nRestrictSUIDSGID=true\nLockPersonality=true\nCapabilityBoundingSet=\nAmbientCapabilities=\nRestrictAddressFamilies=AF_UNIX AF_INET AF_INET6 AF_NETLINK\n\n[Install]\nWantedBy=multi-user.target\n",
        options.user,
        unit_path(&options.data_dir),
        unit_path(&options.install_dir.join("service.env")),
        quoted(&options.install_dir.join("campus-auth")),
        quoted(&options.data_dir),
        unit_path(&options.data_dir)
    ))
}
fn run_command(program: &str, args: &[&str]) -> Result<()> {
    let status = Command::new(program)
        .args(args)
        .status()
        .with_context(|| format!("无法运行 {program}"))?;
    ensure!(status.success(), "{program} 操作失败（{status}）");
    Ok(())
}
fn systemctl(args: &[&str]) -> Result<()> {
    run_command("systemctl", args)
}
fn restart_verified() -> Result<()> {
    systemctl(&["restart", SERVICE])?;
    // Type=simple can be briefly active even if a broken executable exits
    // immediately. Give process initialization a bounded settling interval.
    std::thread::sleep(std::time::Duration::from_secs(1));
    systemctl(&["is-active", "--quiet", SERVICE])
}
pub fn write_private(path: &Path, content: &[u8], mode: u32) -> Result<()> {
    checked_path(path)?;
    let parent = path.parent().context("目标缺少父目录")?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(content)?;
    temporary.as_file().sync_all()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(temporary.path(), fs::Permissions::from_mode(mode))?;
    }
    #[cfg(not(unix))]
    let _ = mode;
    temporary.persist(path).map_err(|e| e.error)?;
    Ok(())
}
fn private_copy(source: &Path, target: &Path, mode: u32) -> Result<()> {
    checked_path(source)?;
    checked_path(target)?;
    let mut input = fs::File::open(source)?;
    let mut temporary =
        tempfile::NamedTempFile::new_in(target.parent().context("复制目标路径无效")?)?;
    io::copy(&mut input, &mut temporary)?;
    temporary.as_file().sync_all()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(temporary.path(), fs::Permissions::from_mode(mode))?;
    }
    #[cfg(not(unix))]
    let _ = mode;
    temporary.persist(target).map_err(|e| e.error)?;
    Ok(())
}
fn remove_file(path: &Path) -> Result<()> {
    checked_path(path)?;
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}
fn read_deployment(path: &Path) -> Result<Option<Deployment>> {
    if !path.exists() {
        return Ok(None);
    }
    checked_path(path)?;
    let value: Deployment = serde_json::from_slice(&fs::read(path)?)?;
    ensure!(
        value.application == "CampusAuth",
        "目标目录中存在不属于 CampusAuth 的部署记录"
    );
    Ok(Some(value))
}
fn backup(options: &ServiceOptions) -> Result<bool> {
    let directory = options.install_dir.join("rollback");
    if !options.install_dir.join("campus-auth").exists() {
        return Ok(false);
    }
    checked_path(&directory)?;
    fs::create_dir_all(&directory)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
    }
    for name in PROGRAM_FILES {
        let source = options.install_dir.join(name);
        let target = directory.join(name);
        if source.exists() {
            private_copy(
                &source,
                &target,
                if name == "campus-auth" { 0o755 } else { 0o600 },
            )?;
        } else {
            remove_file(&target)?;
        }
    }
    let unit = options.unit_dir.join(SERVICE);
    if unit.exists() {
        private_copy(&unit, &directory.join(SERVICE), 0o644)?;
    } else {
        remove_file(&directory.join(SERVICE))?;
    }
    write_private(
        &directory.join(".campus-auth-rollback"),
        b"CampusAuth\n",
        0o600,
    )?;
    Ok(true)
}
fn restore(options: &ServiceOptions) -> Result<()> {
    let backup = options.install_dir.join("rollback");
    checked_path(&backup)?;
    ensure!(
        fs::read_to_string(backup.join(".campus-auth-rollback"))? == "CampusAuth\n",
        "回滚备份标记不匹配"
    );
    ensure!(backup.join("campus-auth").is_file(), "没有可用的上一版程序");
    for name in PROGRAM_FILES {
        let source = backup.join(name);
        checked_path(&source)?;
        if source.exists() {
            private_copy(
                &source,
                &options.install_dir.join(name),
                if name == "campus-auth" { 0o755 } else { 0o600 },
            )?;
        } else {
            remove_file(&options.install_dir.join(name))?;
        }
    }
    if backup.join(SERVICE).exists() {
        private_copy(
            &backup.join(SERVICE),
            &options.unit_dir.join(SERVICE),
            0o644,
        )?;
    } else {
        remove_file(&options.unit_dir.join(SERVICE))?;
    }
    Ok(())
}
fn adopt_state(options: &ServiceOptions, isolated: bool) -> Result<()> {
    if isolated {
        return Ok(());
    }
    if !Command::new("id")
        .arg(&options.user)
        .output()?
        .status
        .success()
    {
        run_command(
            "useradd",
            &[
                "--system",
                "--user-group",
                "--home-dir",
                &options.data_dir.to_string_lossy(),
                "--no-create-home",
                "--shell",
                "/usr/sbin/nologin",
                &options.user,
            ],
        )?;
    }
    run_command(
        "chown",
        &[&options.user, &options.data_dir.to_string_lossy()],
    )?;
    for name in [
        "accounts.json",
        "accounts.json.bak",
        "native_config.json",
        "reconnect.json",
        "config.ini",
        "power_schedule.json",
        "power_draft.json",
        "pending_power_schedule.json",
        "power_countdown.json",
        "power_canceled.json",
        "power_cancel.json",
        "admin_auth.json",
        STATE_MARKER,
        "logs",
        "service-cache",
    ] {
        let path = options.data_dir.join(name);
        if !path.exists() {
            continue;
        }
        checked_path(&path)?;
        run_command("chown", &[&options.user, &path.to_string_lossy()])?;
        if path.is_dir() {
            for child in fs::read_dir(&path)? {
                let child = child?.path();
                checked_path(&child)?;
                if child.is_file() {
                    run_command("chown", &[&options.user, &child.to_string_lossy()])?;
                }
            }
        }
    }
    Ok(())
}
fn cleanup_owned(options: &ServiceOptions, isolated: bool) -> Result<()> {
    if isolated {
        return Ok(());
    }
    let value = crate::power::cleanup_owned(&options.data_dir)?;
    ensure!(
        value
            .get("ok")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true),
        "CampusAuth 电源计划清理失败：{value}"
    );
    // Only a desktop entry explicitly naming this installation is ours.
    let entry = Path::new("/etc/xdg/autostart/campus-auth.desktop");
    if fs::read_to_string(entry).is_ok_and(|text| {
        text.contains(
            &options
                .install_dir
                .join("campus-auth")
                .to_string_lossy()
                .to_string(),
        )
    }) {
        remove_file(entry)?;
    }
    Ok(())
}
fn purge_state(options: &ServiceOptions) -> Result<()> {
    let marker = options.data_dir.join(STATE_MARKER);
    ensure!(
        fs::read_to_string(&marker)
            .is_ok_and(|text| text.trim() == options.data_dir.to_string_lossy()),
        "数据目录缺少本次部署的所有权标记，拒绝自动清空。请备份后手动处理：{}",
        options.data_dir.display()
    );
    for name in [
        "accounts.json",
        "accounts.json.bak",
        "native_config.json",
        "reconnect.json",
        "config.ini",
        "power_schedule.json",
        "power_draft.json",
        "pending_power_schedule.json",
        "admin_auth.json",
        "power_countdown.json",
        "power_cancel.json",
        "power_canceled.json",
        STATE_MARKER,
    ] {
        remove_file(&options.data_dir.join(name))?;
    }
    for name in ["logs", "service-cache"] {
        let path = options.data_dir.join(name);
        if !path.exists() {
            continue;
        }
        checked_path(&path)?;
        for child in fs::read_dir(&path)? {
            let child = child?.path();
            checked_path(&child)?;
            let owned = if name == "logs" {
                let filename = child
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("");
                matches!(
                    filename,
                    "authentication.lock" | "accounts-store.lock" | "log-store.lock"
                ) || (filename.starts_with("campus-") && filename.ends_with(".log"))
            } else {
                child
                    .file_stem()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| {
                        name.len() == 16 && name.bytes().all(|ch| ch.is_ascii_hexdigit())
                    })
                    && child.extension().and_then(|extension| extension.to_str()) == Some("json")
            };
            if child.is_file() && owned {
                remove_file(&child)?;
            }
        }
        let _ = fs::remove_dir(path);
    }
    match fs::remove_dir(&options.data_dir) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::DirectoryNotEmpty => println!(
            "已清除 CampusAuth 已知数据；目录内其他文件予以保留：{}",
            options.data_dir.display()
        ),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    Ok(())
}

pub fn service(action: &str, mut options: ServiceOptions) -> Result<()> {
    ensure!(cfg!(target_os = "linux"), "系统服务部署功能仅支持 Linux");
    ensure!(
        [
            "install",
            "update",
            "rollback",
            "uninstall",
            "status",
            "start",
            "stop",
            "restart"
        ]
        .contains(&action),
        "未知的系统服务操作：{action}"
    );
    dedicated_paths(&options)?;
    if let Some(saved) = read_deployment(&options.install_dir.join(MANIFEST))? {
        ensure!(
            saved.install_dir == options.install_dir
                && saved.unit_dir == options.unit_dir
                && saved.data_dir == options.data_dir,
            "当前目录与已安装的服务数据目录不一致，请使用原部署目录参数"
        );
        if action != "install" {
            options.components = saved.components;
            options.host = saved.host;
            options.port = saved.port;
            options.user = saved.user;
            options.public_url = saved.public_url;
            options.allow_insecure_lan = saved.allow_insecure_lan;
        }
    }
    let unit = render_unit(&options)?;
    if ["status", "start", "stop", "restart"].contains(&action) {
        if action == "status" || !options.dry_run {
            return systemctl(&[action, SERVICE]);
        }
    }
    if options.dry_run {
        println!(
            "操作：{action}\n组件：{}\n程序：{}\n共享数据：{}\n服务：{}\n卸载清空数据：{}\n\n{unit}",
            options.components,
            options.install_dir.join("campus-auth").display(),
            options.data_dir.display(),
            options.unit_dir.join(SERVICE).display(),
            options.purge
        );
        return Ok(());
    }
    let root = Command::new("id").arg("-u").output()?.stdout == b"0\n";
    let isolated = options.no_start
        && options.unit_dir != Path::new("/etc/systemd/system")
        && options.install_dir != Path::new("/usr/local/lib/campus-auth");
    ensure!(root || isolated, "部署系统服务需要 root 权限，请使用 sudo");
    if !isolated {
        for path in [
            &options.install_dir,
            &options.unit_dir,
            &options.install_dir.join("campus-auth"),
            &options.install_dir.join("service.env"),
            &options.install_dir.join(MANIFEST),
            &options.unit_dir.join(SERVICE),
            &options.install_dir.join("rollback"),
        ] {
            protected_program_path(path)?;
        }
    }
    if action == "uninstall" {
        if !isolated && options.unit_dir.join(SERVICE).exists() {
            systemctl(&["disable", "--now", SERVICE])?;
        }
        cleanup_owned(&options, isolated)?;
        for name in PROGRAM_FILES {
            remove_file(&options.install_dir.join(name))?;
        }
        remove_file(&options.unit_dir.join(SERVICE))?;
        let rollback = options.install_dir.join("rollback");
        for name in [
            "campus-auth",
            "service.env",
            MANIFEST,
            SERVICE,
            ".campus-auth-rollback",
        ] {
            remove_file(&rollback.join(name))?;
        }
        let _ = fs::remove_dir(&rollback);
        remove_file(&options.install_dir.join(".deploy.lock"))?;
        let _ = fs::remove_dir(&options.install_dir);
        if !isolated {
            systemctl(&["daemon-reload"])?;
        }
        if options.purge {
            purge_state(&options)?;
            println!("CampusAuth 服务和已知应用数据已清理。");
        } else {
            println!(
                "CampusAuth 系统服务已卸载；账号和设置保留在 {}",
                options.data_dir.display()
            );
        }
        return Ok(());
    }
    fs::create_dir_all(&options.install_dir)?;
    fs::create_dir_all(&options.data_dir)?;
    fs::create_dir_all(&options.unit_dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&options.install_dir, fs::Permissions::from_mode(0o755))?;
        fs::set_permissions(&options.data_dir, fs::Permissions::from_mode(0o700))?;
    }
    let lock_path = options.install_dir.join(".deploy.lock");
    checked_path(&lock_path)?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)?;
    lock.try_lock_exclusive()
        .context("已有 CampusAuth 部署正在进行")?;
    if action == "rollback" {
        restore(&options)?;
        if !isolated {
            systemctl(&["daemon-reload"])?;
            if !options.no_start {
                restart_verified()?;
            }
        }
        println!("已恢复上一版程序、服务和监听配置；当前账号和管理密码保持不变。");
        return Ok(());
    }
    let has_backup = backup(&options)?;
    let existing_token = fs::read_to_string(options.install_dir.join("service.env"))
        .ok()
        .and_then(|text| {
            text.lines()
                .find_map(|line| line.strip_prefix("CAMPUS_AUTH_TOKEN=").map(str::to_owned))
        });
    let token = options
        .token
        .clone()
        .or(existing_token)
        .unwrap_or_else(|| format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple()));
    ensure!(
        token.len() >= 24
            && token
                .bytes()
                .all(|ch| ch.is_ascii_graphic() && !matches!(ch, b'"' | b'\'' | b'\\' | b'%')),
        "API 令牌至少需要 24 个可打印字符，不能包含引号、空格、反斜杠或百分号"
    );
    let source = std::env::current_exe()?;
    let result = (|| -> Result<()> {
        private_copy(&source, &options.install_dir.join("campus-auth"), 0o755)?;
        write_private(
            &options.install_dir.join("service.env"),
            format!("CAMPUS_AUTH_TOKEN={token}\n").as_bytes(),
            0o600,
        )?;
        write_private(
            &options.install_dir.join(MANIFEST),
            &serde_json::to_vec_pretty(&Deployment::new(&options))?,
            0o600,
        )?;
        write_private(
            &options.data_dir.join(STATE_MARKER),
            options.data_dir.to_string_lossy().as_bytes(),
            0o600,
        )?;
        adopt_state(&options, isolated)?;
        write_private(&options.unit_dir.join(SERVICE), unit.as_bytes(), 0o644)?;
        if !isolated {
            systemctl(&["daemon-reload"])?;
            if !options.no_start {
                systemctl(&["enable", "--now", SERVICE])?;
                restart_verified()?;
            }
        }
        Ok(())
    })();
    if let Err(error) = result {
        if has_backup {
            let recovery = (|| -> Result<()> {
                restore(&options)?;
                if !isolated {
                    systemctl(&["daemon-reload"])?;
                    if !options.no_start {
                        restart_verified()?;
                    }
                }
                Ok(())
            })();
            if let Err(recovery) = recovery {
                bail!(
                    "升级失败：{error:#}；自动回滚也失败：{recovery:#}。账号和配置仍保留在 {}",
                    options.data_dir.display()
                );
            }
            bail!("升级失败，已自动恢复上一版程序和服务：{error:#}");
        }
        return Err(error);
    }
    println!(
        "CampusAuth 系统服务操作完成：{action}\n组件模式：{}\n账号和配置共享目录：{}",
        options.components,
        options.data_dir.display()
    );
    if options.components.has_webui() {
        println!(
            "WebUI：{}",
            options
                .public_url
                .clone()
                .unwrap_or_else(|| format!("http://{}:{}", options.host, options.port))
        );
        match crate::server_auth::configured(&options.data_dir) {
            Ok(true) => {}
            Ok(false) => println!(
                "首次登录前请运行 sudo {} --data-dir {} password set 设置管理密码。",
                options.install_dir.join("campus-auth").display(),
                options.data_dir.display()
            ),
            Err(_) => println!(
                "已有管理员配置无法读取。后台服务安装已完成；请在本机运行 sudo {} --data-dir {} password reset 重设后使用 WebUI。",
                options.install_dir.join("campus-auth").display(),
                options.data_dir.display()
            ),
        }
    }
    if options.components.has_tui() {
        println!(
            "管理命令：sudo {} --data-dir {} tui；退出 TUI 不会停止后台认证。",
            options.install_dir.join("campus-auth").display(),
            options.data_dir.display()
        );
    }
    if !options.components.has_webui() {
        println!("此模式不监听 HTTP；后台服务持续执行已启用的自动重连。");
    }
    Ok(())
}
