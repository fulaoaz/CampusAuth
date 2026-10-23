use super::models::{Account, ConnectionConfig, ReconnectSettings};
use anyhow::{Context, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct StoredAccount {
    pub id: String,
    pub label: String,
    pub username: String,
    pub password: String,
    pub service: String,
    pub user_group: String,
    #[serde(deserialize_with = "read_bool")]
    pub enabled: bool,
    pub note: String,
    pub last_result: String,
    pub last_ok: Option<bool>,
}

impl Default for StoredAccount {
    fn default() -> Self {
        Self {
            id: String::new(),
            label: String::new(),
            username: String::new(),
            password: String::new(),
            service: String::new(),
            user_group: String::new(),
            enabled: true,
            note: String::new(),
            last_result: String::new(),
            last_ok: None,
        }
    }
}

fn read_bool<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<bool, D::Error> {
    let value = Value::deserialize(d)?;
    Ok(match value {
        Value::Bool(value) => value,
        Value::String(value) => !matches!(
            value.trim().to_lowercase().as_str(),
            "" | "0" | "false" | "no" | "off" | "否"
        ),
        Value::Number(value) => value.as_i64().unwrap_or(0) != 0,
        _ => false,
    })
}

impl StoredAccount {
    pub fn public(&self) -> Account {
        Account {
            id: self.id.clone(),
            label: self.label.clone(),
            username: self.username.clone(),
            service: self.service.clone(),
            enabled: self.enabled,
            note: self.note.clone(),
            last_result: self.last_result.clone(),
            last_ok: self.last_ok,
            has_password: !self.password.is_empty(),
        }
    }
}

#[derive(Serialize, Deserialize)]
pub(crate) struct AccountsFile {
    pub version: u32,
    pub accounts: Vec<StoredAccount>,
}

pub(crate) struct ProcessLock(File);

impl ProcessLock {
    pub fn acquire(path: &Path) -> Result<Option<Self>> {
        if let Some(parent) = path.parent() {
            private_directory(parent)?;
        }
        let file = private_open(path, false)?;
        match file.try_lock_exclusive() {
            Ok(()) => Ok(Some(Self(file))),
            Err(error)
                if error.kind() == ErrorKind::WouldBlock
                    || matches!(error.raw_os_error(), Some(11 | 13 | 32 | 33)) =>
            {
                Ok(None)
            }
            Err(error) => Err(error).context("无法取得校园网操作锁"),
        }
    }

    fn acquire_store(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            private_directory(parent)?;
        }
        let file = private_open(path, false)?;
        file.lock_exclusive()
            .context("无法读取账号库，文件正在被其他程序使用")?;
        Ok(Self(file))
    }
}

impl Drop for ProcessLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.0);
    }
}

pub(crate) fn private_directory(path: &Path) -> Result<()> {
    let existed=path.exists();
    fs::create_dir_all(path).context("无法创建应用数据目录")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
        use std::os::unix::fs::MetadataExt;
        if !existed && fs::metadata(path)?.uid()==0 {
            if let Some(parent)=path.parent(){let owner=fs::metadata(parent)?;std::os::unix::fs::chown(path,Some(owner.uid()),Some(owner.gid()))?;}
        }
    }
    #[cfg(not(unix))] let _=existed;
    Ok(())
}

pub(crate) fn protect_existing_store(dir: &Path) -> Result<()> {
    for name in [
        "accounts.json",
        "accounts.json.bak",
        "config.ini",
        "native_config.json",
    ] {
        let path = dir.join(name);
        if path.is_file() {
            protect_file(&path)?;
        }
    }
    Ok(())
}

fn private_open(path: &Path, new: bool) -> Result<File> {
    let existed=path.exists();
    let mut options = OpenOptions::new();
    options.read(true).write(true);
    if new {
        options.create_new(true);
    } else {
        options.create(true).truncate(false);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file=options.open(path).context("无法访问应用数据文件")?;
    #[cfg(unix)] {
        use std::os::unix::fs::MetadataExt;
        if !existed && file.metadata()?.uid()==0 {
            if let Some(parent)=path.parent(){let owner=fs::metadata(parent)?;std::os::unix::fs::chown(path,Some(owner.uid()),Some(owner.gid()))?;}
        }
    }
    #[cfg(not(unix))] let _=existed;
    Ok(file)
}

pub(crate) fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let bytes = fs::read(path).context("无法读取应用数据文件")?;
    serde_json::from_slice(&bytes).context("应用数据格式不正确，请从备份恢复")
}

#[cfg(windows)]
fn protect_file(path: &Path) -> Result<()> {
    use std::os::windows::process::CommandExt;
    use std::process::Command;
    let output = Command::new("whoami")
        .arg("/user")
        .arg("/fo")
        .arg("csv")
        .arg("/nh")
        .creation_flags(0x08000000)
        .output()
        .context("无法确认 Windows 文件所有者")?;
    let owner = String::from_utf8_lossy(&output.stdout)
        .split('"')
        .find(|part| part.starts_with("S-1-"))
        .map(str::to_owned)
        .context("无法确认 Windows 文件所有者")?;
    let result = Command::new("icacls")
        .arg(path)
        .arg("/inheritance:r")
        .arg("/grant:r")
        .arg(format!("*{owner}:(F)"))
        .arg("*S-1-5-18:(F)")
        .arg("*S-1-5-32-544:(F)")
        .creation_flags(0x08000000)
        .output()
        .context("无法保护凭据文件权限")?;
    if !result.status.success() {
        anyhow::bail!("无法保护凭据文件权限；未保存凭据。")
    }
    Ok(())
}

#[cfg(not(windows))]
fn protect_file(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(windows)]
fn replace_file(source: &Path, target: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(existing: *const u16, target: *const u16, flags: u32) -> i32;
    }
    let existing: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let target: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
    if unsafe { MoveFileExW(existing.as_ptr(), target.as_ptr(), 1 | 8) } == 0 {
        return Err(std::io::Error::last_os_error()).context("无法替换应用数据文件");
    }
    Ok(())
}

#[cfg(not(windows))]
fn replace_file(source: &Path, target: &Path) -> Result<()> {
    fs::rename(source, target).context("无法替换应用数据文件")
}

pub(crate) fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let parent = path.parent().context("数据目录无效")?;
    private_directory(parent)?;
    let temporary = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name().unwrap().to_string_lossy(),
        Uuid::new_v4()
    ));
    let result = (|| {
        let mut handle = private_open(&temporary, true)?;
        protect_file(&temporary)?;
        serde_json::to_writer_pretty(&mut handle, value).context("无法写入应用数据文件")?;
        handle.write_all(b"\n")?;
        handle.sync_all()?;
        drop(handle);
        #[cfg(unix)] {
            use std::os::unix::fs::MetadataExt;
            let current=fs::metadata(&temporary)?;
            if current.uid()==0 {
                let owner=fs::metadata(path).or_else(|_|fs::metadata(parent))?;
                std::os::unix::fs::chown(&temporary,Some(owner.uid()),Some(owner.gid()))?;
            }
        }
        replace_file(&temporary, path)?;
        Ok(())
    })();
    let _ = fs::remove_file(temporary);
    result
}

pub(crate) fn load_accounts(dir: &Path) -> Result<Vec<StoredAccount>> {
    let _store_lock = ProcessLock::acquire_store(&dir.join("logs/accounts-store.lock"))?;
    let path = dir.join("accounts.json");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let mut data: AccountsFile = match read_json(&path) {
        Ok(data) => data,
        Err(error) => {
            let backup = dir.join("accounts.json.bak");
            if backup.exists() {
                read_json(&backup).context("账号库及备份损坏，请先恢复文件")?
            } else {
                return Err(error);
            }
        }
    };
    let mut ids = std::collections::HashSet::new();
    let mut changed = false;
    for account in &mut data.accounts {
        if account.id.is_empty() || !ids.insert(account.id.clone()) {
            account.id = Uuid::new_v4().simple().to_string();
            ids.insert(account.id.clone());
            changed = true;
        }
    }
    // Repaired identifiers must survive the next read so a UI can edit or
    // delete the exact account it just displayed.
    if changed {
        write_json(&path, &data)?;
    }
    Ok(data.accounts)
}

pub(crate) fn save_accounts(dir: &Path, accounts: &[StoredAccount]) -> Result<()> {
    let _store_lock = ProcessLock::acquire_store(&dir.join("logs/accounts-store.lock"))?;
    let path = dir.join("accounts.json");
    if path.exists() {
        if let Ok(previous) = read_json::<AccountsFile>(&path) {
            write_json(&dir.join("accounts.json.bak"), &previous)?;
        }
    }
    write_json(
        &path,
        &AccountsFile {
            version: 1,
            accounts: accounts.to_vec(),
        },
    )
}

pub(crate) fn legacy_ini(path: &Path) -> Result<HashMap<String, String>> {
    if !path.exists() {
        return Ok(HashMap::new());
    }
    let text = fs::read_to_string(path).context("旧版 INI 配置无法读取")?;
    let mut section = String::new();
    let mut values = HashMap::new();
    for raw in text.trim_start_matches('\u{feff}').lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            section = line[1..line.len() - 1].trim().to_ascii_lowercase();
        } else if let Some((key, value)) = raw.split_once('=') {
            values.insert(
                format!("{}.{}", section, key.trim().to_ascii_lowercase()),
                value.trim().into(),
            );
        }
    }
    Ok(values)
}

pub(crate) fn import_legacy(dir: &Path) -> Result<()> {
    let legacy = legacy_ini(&dir.join("config.ini"))?;
    let target = dir.join("native_config.json");
    if !target.exists() {
        let mut cfg = ConnectionConfig::default();
        if let Some(value) = legacy.get("auth.gateway") {
            cfg.gateway = value.to_string();
            if !value.contains("210.27.177.172") {
                cfg.profile_id = "custom".into();
                cfg.display_name = "已导入的 ePortal 校园网".into();
            }
        }
        if let Some(value) = legacy.get("auth.service") {
            cfg.service = value.clone();
        }
        if let Some(value) = legacy.get("auth.user_group") {
            cfg.user_group = value.clone();
        }
        if let Some(value) = legacy.get("auth.password_encrypt") {
            cfg.password_encrypt = matches!(
                value.to_ascii_lowercase().as_str(),
                "true" | "1" | "yes" | "on"
            );
        }
        if let Some(value) = legacy.get("network.probe_urls") {
            cfg.probe_urls = value
                .split(',')
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_owned)
                .collect();
        }
        if let Some(value) = legacy
            .get("network.timeout")
            .and_then(|v| v.parse::<u64>().ok())
        {
            cfg.timeout_seconds = value.clamp(1, 30);
        }
        if let Some(value) = legacy
            .get("network.retry_per_account")
            .or_else(|| legacy.get("network.retry"))
            .and_then(|v| v.parse::<u32>().ok())
        {
            cfg.retry_per_account = value.clamp(1, 6);
        }
        if let Some(value) = legacy
            .get("network.retry_interval")
            .and_then(|v| v.parse::<u64>().ok())
        {
            cfg.retry_interval_seconds = value.min(30);
        }
        write_json(&target, &cfg)?;
    }
    let accounts_path = dir.join("accounts.json");
    if !accounts_path.exists() {
        let username = legacy.get("auth.username").cloned().unwrap_or_default();
        let mut accounts = Vec::new();
        if !username.is_empty() {
            accounts.push(StoredAccount {
                id: Uuid::new_v4().simple().to_string(),
                username,
                password: legacy.get("auth.password").cloned().unwrap_or_default(),
                service: legacy.get("auth.service").cloned().unwrap_or_default(),
                user_group: legacy.get("auth.user_group").cloned().unwrap_or_default(),
                label: "导入的主账号".into(),
                ..StoredAccount::default()
            });
        }
        save_accounts(dir, &accounts)?;
    }
    let reconnect_path = dir.join("reconnect.json");
    if !reconnect_path.exists() {
        let mut reconnect = ReconnectSettings::default();
        if let Some(value) = legacy
            .get("watchdog.interval")
            .and_then(|v| v.parse::<u64>().ok())
        {
            reconnect.interval_seconds = value.clamp(10, 86400);
        }
        if let Some(value) = legacy
            .get("watchdog.fail_threshold")
            .and_then(|v| v.parse::<u32>().ok())
        {
            reconnect.fail_threshold = value.clamp(1, 10);
        }
        write_json(&reconnect_path, &reconnect)?;
    }
    Ok(())
}

pub(crate) fn config_path(dir: &Path) -> PathBuf {
    dir.join("native_config.json")
}
