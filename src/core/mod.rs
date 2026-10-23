mod eportal;
mod models;
mod storage;
mod log_store;

pub use models::*;

use anyhow::{Context, Result, bail};
use chrono::Local;
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};
use storage::{ProcessLock, StoredAccount};
use uuid::Uuid;

pub struct Core {
    data_dir: PathBuf,
    operation: Mutex<()>,
    runtime: Mutex<ReconnectRuntime>,
    logs: Mutex<Vec<String>>,
    known_secrets: Mutex<HashSet<String>>,
}

#[derive(Default)]
struct ReconnectRuntime {
    last_check: Option<Instant>,
    consecutive_failures: u32,
}

impl Core {
    pub fn new(data_dir: impl AsRef<Path>) -> Result<Self> {
        let path = data_dir.as_ref();
        let data_dir = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()?.join(path)
        };
        storage::private_directory(&data_dir)?;
        let lock = ProcessLock::acquire(&data_dir.join("logs/authentication.lock"))?
            .context("已有校园网应用正在初始化，请稍后重试。")?;
        storage::protect_existing_store(&data_dir)?;
        storage::import_legacy(&data_dir)?;
        drop(lock);
        let this = Self {
            data_dir,
            operation: Mutex::new(()),
            runtime: Mutex::new(ReconnectRuntime::default()),
            logs: Mutex::new(Vec::new()),
            known_secrets: Mutex::new(HashSet::new()),
        };
        if let Ok(accounts) = storage::load_accounts(&this.data_dir) {
            this.remember_secrets(&accounts);
        }
        this.log("CampusAuth 已启动，认证数据保存在本机。");
        Ok(this)
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    fn guard<T>(lock: &Mutex<T>) -> MutexGuard<'_, T> {
        lock.lock().unwrap_or_else(|poison| poison.into_inner())
    }

    fn lock(&self) -> Result<Option<(MutexGuard<'_, ()>, ProcessLock)>> {
        let guard = match self.operation.try_lock() {
            Ok(guard) => guard,
            Err(std::sync::TryLockError::WouldBlock) => return Ok(None),
            Err(std::sync::TryLockError::Poisoned(poison)) => poison.into_inner(),
        };
        match ProcessLock::acquire(&self.data_dir.join("logs/authentication.lock"))? {
            Some(lock) => Ok(Some((guard, lock))),
            None => Ok(None),
        }
    }

    fn remember_secrets(&self, accounts: &[StoredAccount]) {
        let mut secrets = Self::guard(&self.known_secrets);
        for account in accounts {
            if !account.password.is_empty() {
                secrets.insert(account.password.clone());
                secrets.insert(eportal::quote_component(&account.password));
                secrets.insert(eportal::quote_component(&eportal::quote_component(
                    &account.password,
                )));
            }
        }
    }

    fn redact(&self, text: impl AsRef<str>) -> String {
        let values = Self::guard(&self.known_secrets);
        let mut secrets: Vec<_> = values.iter().collect();
        secrets.sort_by_key(|value| std::cmp::Reverse(value.len()));
        let mut text = text.as_ref().to_owned();
        for secret in secrets {
            text = text.replace(secret, "[已隐藏]");
        }
        text
    }

    fn log(&self, text: impl AsRef<str>) {
        let line = format!("{} {}", Local::now().format("%Y-%m-%d %H:%M:%S"), self.redact(text));
        let _=log_store::append(&self.data_dir,&line);
        let mut logs = Self::guard(&self.logs);
        logs.push(line);
        if logs.len() > 500 {
            let count = logs.len() - 500;
            logs.drain(..count);
        }
    }

    pub fn logs(&self) -> Vec<String> {
        let lines = log_store::read(&self.data_dir).unwrap_or_else(|_|Self::guard(&self.logs).clone());
        lines.into_iter().map(|line| self.redact(line)).collect()
    }
    pub fn diagnostic_logs(&self)->Vec<String> {
        log_store::read_history(&self.data_dir).unwrap_or_else(|_|self.logs()).into_iter().map(|line|self.redact(line)).collect()
    }

    pub fn connection_config(&self) -> Result<ConnectionConfig> {
        storage::read_json(&storage::config_path(&self.data_dir))
    }

    pub fn set_connection_config(&self, config: ConnectionConfig) -> Result<ConnectionConfig> {
        eportal::validate_config(&config)?;
        let _lock = self
            .lock()?
            .context("认证操作正在进行，请稍后修改学校设置。")?;
        storage::write_json(&storage::config_path(&self.data_dir), &config)?;
        *Self::guard(&self.runtime) = ReconnectRuntime::default();
        self.log("学校认证设置已保存。");
        Ok(config)
    }

    pub fn snapshot(&self) -> Result<Snapshot> {
        Ok(Snapshot {
            accounts: self.accounts()?,
            reconnect: self.reconnect_settings()?,
            logs: self.logs(),
            config: self.connection_config()?,
        })
    }

    pub fn accounts(&self) -> Result<Vec<Account>> {
        let accounts = storage::load_accounts(&self.data_dir)?;
        self.remember_secrets(&accounts);
        Ok(accounts
            .iter()
            .map(|account| self.public_account(account))
            .collect())
    }

    fn public_account(&self, account: &StoredAccount) -> Account {
        let mut value = account.public();
        value.last_result = self.redact(&value.last_result);
        value.note = self.redact(&value.note);
        value
    }

    pub fn save_account(&self, input: AccountInput) -> Result<Account> {
        if input.username.trim().is_empty() {
            bail!("请填写校园网账号。")
        }
        if input.username.len() > 256
            || input.password.len() > 2048
            || input.label.len() > 256
            || input.note.len() > 4096
            || input.service.len() > 1024
        {
            bail!("账号内容过长，请检查输入。")
        }
        let _lock = self.lock()?.context("认证操作正在进行，请稍后修改账号。")?;
        let mut accounts = storage::load_accounts(&self.data_dir)?;
        self.remember_secrets(&accounts);
        let index = if let Some(id) = input.id.as_ref().filter(|id| !id.is_empty()) {
            accounts
                .iter()
                .position(|account| &account.id == id)
                .context("此账号已删除，请刷新列表。")?
        } else {
            if input.password.is_empty() {
                bail!("请填写校园网密码。")
            }
            if accounts.len() >= 50 {
                bail!("最多保存 50 个校园网账号。")
            }
            accounts.push(StoredAccount {
                id: Uuid::new_v4().simple().to_string(),
                ..StoredAccount::default()
            });
            accounts.len() - 1
        };
        let account = &mut accounts[index];
        account.label = input.label.trim().into();
        account.username = input.username.trim().into();
        if !input.password.is_empty() {
            account.password = input.password;
        }
        account.service = input.service.trim().into();
        account.enabled = input.enabled;
        account.note = input.note;
        self.remember_secrets(&accounts);
        storage::save_accounts(&self.data_dir, &accounts)?;
        self.log("账号已保存；编辑时留空密码将保留原密码。");
        Ok(self.public_account(&accounts[index]))
    }

    pub fn delete_account(&self, id: &str) -> Result<bool> {
        let _lock = self.lock()?.context("认证操作正在进行，请稍后删除账号。")?;
        let mut accounts = storage::load_accounts(&self.data_dir)?;
        self.remember_secrets(&accounts);
        let before = accounts.len();
        accounts.retain(|account| account.id != id);
        let deleted = accounts.len() != before;
        if deleted {
            storage::save_accounts(&self.data_dir, &accounts)?;
            self.log("账号已删除。");
        }
        Ok(deleted)
    }

    pub fn reorder_accounts(&self, ids: &[String]) -> Result<Vec<Account>> {
        let _lock = self.lock()?.context("认证操作正在进行，请稍后更改顺序。")?;
        let mut accounts = storage::load_accounts(&self.data_dir)?;
        let known: HashSet<_> = accounts.iter().map(|account| account.id.clone()).collect();
        let unique: HashSet<_> = ids.iter().collect();
        if unique.len() != ids.len() || ids.iter().any(|id| !known.contains(id)) {
            bail!("账号顺序包含重复或不存在的账号，请刷新列表。")
        }
        accounts.sort_by_key(|account| {
            ids.iter()
                .position(|id| id == &account.id)
                .unwrap_or(usize::MAX)
        });
        storage::save_accounts(&self.data_dir, &accounts)?;
        self.remember_secrets(&accounts);
        Ok(accounts
            .iter()
            .map(|account| self.public_account(account))
            .collect())
    }

    pub fn status(&self) -> Status {
        let config = match self.connection_config() {
            Ok(config) => config,
            Err(error) => {
                return Status {
                    online: None,
                    network_online: None,
                    portal_online: None,
                    reason: self.redact(error.to_string()),
                    message: "无法读取应用配置，请检查数据目录。".into(),
                    username: String::new(),
                    service: String::new(),
                    local_ip: String::new(),
                    checked_at: Local::now().timestamp(),
                };
            }
        };
        let mut result = eportal::status(&config);
        result.reason = self.redact(result.reason);
        result.message = self.redact(result.message);
        result
    }

    fn marked(&self, id: &str, ok: bool, message: &str) -> Result<()> {
        let mut accounts = storage::load_accounts(&self.data_dir)?;
        if let Some(account) = accounts.iter_mut().find(|account| account.id == id) {
            account.last_ok = Some(ok);
            account.last_result = format!(
                "{} {}",
                Local::now().format("%Y-%m-%d %H:%M"),
                self.redact(message)
            );
            storage::save_accounts(&self.data_dir, &accounts)?;
        }
        Ok(())
    }

    pub fn authenticate(&self, account_id: Option<&str>, force: bool) -> Result<ActionResult> {
        let Some(_lock) = self.lock()? else {
            return Ok(ActionResult::busy());
        };
        self.authenticate_locked(account_id, force)
    }
    pub fn connect(&self, account_id:Option<&str>,force:bool,confirm_switch:bool)->Result<serde_json::Value> {
        let Some(_lock)=self.lock()? else {return Ok(serde_json::to_value(ActionResult::busy())?);};
        if let Some(id)=account_id {
            let accounts=storage::load_accounts(&self.data_dir)?;
            self.remember_secrets(&accounts);
            let target=accounts.iter().find(|a|a.id==id).context("该账号已不存在，请刷新列表")?;
            if !target.enabled || target.username.is_empty() || target.password.is_empty(){
                return Ok(serde_json::to_value(ActionResult::new(false,"该账号未启用或缺少认证信息，当前连接保持不变"))?);
            }
            let config=self.connection_config()?;
            let session=eportal::PortalSession::new(&config)?;
            let info=session.online_info(Instant::now()+Duration::from_secs(3),true).ok();
            let current=info.as_ref().and_then(|v|v["userId"].as_str()).unwrap_or("");
            let portal_online=info.as_ref().is_some_and(|v|v["result"].as_str()==Some("success"));
            if portal_online && !current.is_empty() && current!=target.username {
                if !confirm_switch {
                    return Ok(serde_json::json!({"ok":false,"needs_confirmation":true,"current_username":current,"target_username":target.username,"message":"切换账号需要断开当前校园网会话，是否继续？"}));
                }
                let (ok,message)=session.logout(Instant::now()+Duration::from_secs(12))?;
                if !ok{return Ok(serde_json::to_value(ActionResult::new(false,self.redact(message)))?);}
                self.log("已确认断开原会话，正在切换到选定账号。");
            }
        }
        // Hold the same process lock across confirmation checking, logout and login.
        Ok(serde_json::to_value(self.authenticate_locked(account_id,force)?)?)
    }

    fn authenticate_locked(&self, account_id: Option<&str>, _force: bool) -> Result<ActionResult> {
        let config = self.connection_config()?;
        eportal::validate_config(&config)?;
        let accounts = storage::load_accounts(&self.data_dir)?;
        self.remember_secrets(&accounts);
        if account_id.is_some_and(|id| !accounts.iter().any(|account| account.id == id)) {
            return Ok(ActionResult::new(false, "此账号不存在，请刷新账号列表。"));
        }
        let session = eportal::PortalSession::new(&config)?;
        let deadline = Instant::now() + Duration::from_secs(120);
        let (online, _) = session.probe(Instant::now() + Duration::from_secs(6), true);
        if online {
            if let Some(id) = account_id {
                if let Some(target) = accounts.iter().find(|account| account.id == id) {
                    let info = session
                        .online_info(Instant::now() + Duration::from_secs(3), true)
                        .ok();
                    let current = info
                        .as_ref()
                        .and_then(|info| info.get("userId"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    if !current.is_empty() && current.trim() != target.username.trim() {
                        return Ok(ActionResult::new(
                            false,
                            "已有其他账号在线。请先断开现有会话，再测试此账号。",
                        ));
                    }
                }
            }
            self.log("当前互联网连接正常，已保留在线会话。");
            return Ok(ActionResult::new(true, "互联网已连接，无需重复认证。"));
        }
        let usable: Vec<_> = accounts
            .iter()
            .filter(|account| {
                account_id.is_none_or(|id| account.id == id)
                    && account.enabled
                    && !account.username.trim().is_empty()
                    && !account.password.is_empty()
            })
            .collect();
        if usable.is_empty() {
            return Ok(ActionResult::new(
                false,
                "没有可用账号，请添加账号和密码并开启此账号。",
            ));
        }
        let mut last = "所有账号均未能连接互联网。".to_owned();
        for (index, account) in usable.iter().enumerate() {
            if Instant::now() >= deadline {
                last = "认证操作超时，请检查校园网或稍后重试。".into();
                break;
            }
            self.log(format!("正在认证账号 {}/{}。", index + 1, usable.len()));
            // A fresh jar per account prevents an old identity or JSESSIONID
            // from leaking into the next account's login attempt.
            let account_session = eportal::PortalSession::new(&config)?;
            let mut ok = false;
            let mut submitted = false;
            for attempt in 0..config.retry_per_account {
                let query = match account_session.discover_query(deadline) {
                    Ok(query) => query,
                    Err(error) => {
                        last = self.redact(error.to_string());
                        if account_session
                            .probe(deadline.min(Instant::now() + Duration::from_secs(4)), true)
                            .0
                        {
                            ok = true;
                            last = "互联网已连接。".into();
                        }
                        break;
                    }
                };
                submitted = true;
                match account_session.login(account, &query, deadline) {
                    Ok((accepted, message)) => {
                        last = self.redact(message);
                        self.log(&last);
                        if accepted || last.contains("已存在在线用户") {
                            for _ in 0..3 {
                                if Instant::now() >= deadline {
                                    break;
                                }
                                std::thread::sleep(Duration::from_millis(700));
                                if account_session
                                    .probe(
                                        deadline.min(Instant::now() + Duration::from_secs(4)),
                                        true,
                                    )
                                    .0
                                {
                                    ok = true;
                                    last = "认证成功，互联网已连接。".into();
                                    break;
                                }
                            }
                            if !ok {
                                last = "网关接受认证，但互联网仍不可达；请检查账号余额或服务权限。"
                                    .into();
                            }
                            break;
                        }
                        if ["密码", "账号", "用户名", "password", "credential"]
                            .iter()
                            .any(|word| last.contains(word))
                        {
                            break;
                        }
                    }
                    Err(error) => {
                        last = self.redact(error.to_string());
                        self.log(&last);
                    }
                }
                if attempt + 1 < config.retry_per_account {
                    let wait = Duration::from_secs(config.retry_interval_seconds)
                        .min(deadline.saturating_duration_since(Instant::now()));
                    std::thread::sleep(wait);
                }
            }
            if let Err(error) = self.marked(&account.id, ok, &last) {
                self.log(format!("账号结果未保存：{error}"));
            }
            if ok {
                if let Ok(services) =
                    account_session.services(Instant::now() + Duration::from_secs(3))
                {
                    self.save_services_cache(&config, &services).ok();
                }
                Self::guard(&self.runtime).consecutive_failures = 0;
                self.log("认证成功，互联网已连接。");
                return Ok(ActionResult::new(true, "认证成功，互联网已连接。"));
            }
            self.log(format!("此账号未能连接：{last}"));
            if submitted && index + 1 < usable.len() {
                let _ =
                    account_session.logout(deadline.min(Instant::now() + Duration::from_secs(5)));
            }
        }
        Ok(ActionResult::new(false, last))
    }

    pub fn logout(&self) -> Result<ActionResult> {
        let Some(_lock) = self.lock()? else {
            return Ok(ActionResult::busy());
        };
        let mut settings = self.reconnect_settings()?;
        settings.enabled = false;
        storage::write_json(&self.data_dir.join("reconnect.json"), &settings)?;
        *Self::guard(&self.runtime) = ReconnectRuntime::default();
        let config = self.connection_config()?;
        let session = eportal::PortalSession::new(&config)?;
        match session.logout(Instant::now() + Duration::from_secs(12)) {
            Ok((ok, message)) => {
                let message = self.redact(message);
                self.log(format!(
                    "注销{}，自动重连已暂停。{message}",
                    if ok { "成功" } else { "未成功" }
                ));
                Ok(ActionResult::new(
                    ok,
                    if message.is_empty() {
                        "自动重连已暂停。".into()
                    } else {
                        message
                    },
                ))
            }
            Err(error) => {
                let message = self.redact(error.to_string());
                self.log(format!("注销未成功，自动重连已暂停：{message}"));
                Ok(ActionResult::new(false, message))
            }
        }
    }

    pub fn reconnect_settings(&self) -> Result<ReconnectSettings> {
        let settings: ReconnectSettings =
            storage::read_json(&self.data_dir.join("reconnect.json"))?;
        Self::validate_reconnect(&settings)?;
        Ok(settings)
    }

    fn validate_reconnect(settings: &ReconnectSettings) -> Result<()> {
        if !(10..=86400).contains(&settings.interval_seconds)
            || !(1..=10).contains(&settings.fail_threshold)
        {
            bail!("自动重连间隔需为 10–86400 秒，连续失败阈值需为 1–10 次。")
        }
        Ok(())
    }

    pub fn set_reconnect(&self, settings: ReconnectSettings) -> Result<ReconnectSettings> {
        Self::validate_reconnect(&settings)?;
        let _lock = self
            .lock()?
            .context("认证操作正在进行，请稍后修改自动重连。")?;
        storage::write_json(&self.data_dir.join("reconnect.json"), &settings)?;
        *Self::guard(&self.runtime) = ReconnectRuntime::default();
        self.log(if settings.enabled {
            "自动重连已开启。"
        } else {
            "自动重连已暂停。"
        });
        Ok(settings)
    }

    pub fn reconnect_tick(&self) -> Result<ReconnectResult> {
        let mut result = ReconnectResult {
            ok: true,
            skipped: true,
            attempted: false,
            reconnected: false,
            online: None,
            reason: String::new(),
            consecutive_failures: Self::guard(&self.runtime).consecutive_failures,
            busy: false,
        };
        let settings = self.reconnect_settings()?;
        if !settings.enabled {
            result.reason = "自动重连已暂停。".into();
            return Ok(result);
        }
        {
            let runtime = Self::guard(&self.runtime);
            if runtime
                .last_check
                .is_some_and(|last| last.elapsed() < Duration::from_secs(settings.interval_seconds))
            {
                result.reason = "尚未到检测时间。".into();
                return Ok(result);
            }
        }
        let Some(_lock) = self.lock()? else {
            result.busy = true;
            result.reason = ActionResult::busy().message;
            return Ok(result);
        };
        let settings = self.reconnect_settings()?;
        if !settings.enabled {
            result.reason = "自动重连已暂停。".into();
            return Ok(result);
        }
        Self::guard(&self.runtime).last_check = Some(Instant::now());
        let session = eportal::PortalSession::new(&self.connection_config()?)?;
        let (online, reason) = session.probe(Instant::now() + Duration::from_secs(6), true);
        result.online = Some(online);
        result.reason = reason;
        result.skipped = false;
        {
            let mut runtime = Self::guard(&self.runtime);
            runtime.consecutive_failures = if online {
                0
            } else {
                runtime.consecutive_failures.saturating_add(1)
            };
            result.consecutive_failures = runtime.consecutive_failures;
        }
        if online || result.consecutive_failures < settings.fail_threshold {
            return Ok(result);
        }
        result.attempted = true;
        self.log("校园网检测连续失败，开始自动认证。");
        let auth = self.authenticate_locked(None, false)?;
        result.ok = auth.ok;
        result.reconnected = auth.ok;
        result.online = Some(auth.ok);
        result.reason = auth.message;
        result.consecutive_failures = Self::guard(&self.runtime).consecutive_failures;
        Ok(result)
    }

    fn cache_path(&self, config: &ConnectionConfig) -> PathBuf {
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        config.gateway.hash(&mut hash);
        let name = format!("{:016x}", hash.finish());
        self.data_dir
            .join("service-cache")
            .join(format!("{name}.json"))
    }

    fn save_services_cache(&self, config: &ConnectionConfig, services: &[Service]) -> Result<()> {
        if !services.is_empty() {
            storage::write_json(&self.cache_path(config), &services)?;
        }
        Ok(())
    }

    pub fn services(&self) -> Result<Vec<Service>> {
        let config = self.connection_config()?;
        let mut services = if config.service.is_empty() {
            Vec::new()
        } else {
            vec![Service {
                value: config.service.clone(),
                name: config.service.clone(),
            }]
        };
        let cached: Vec<Service> =
            storage::read_json(&self.cache_path(&config)).unwrap_or_default();
        let fresh = eportal::PortalSession::new(&config)
            .and_then(|session| session.services(Instant::now() + Duration::from_secs(8)))
            .unwrap_or_default();
        for item in fresh.iter().chain(cached.iter()) {
            if !services.iter().any(|service| service.value == item.value) {
                services.push(item.clone());
            }
        }
        if !fresh.is_empty() {
            self.save_services_cache(&config, &services)?;
        }
        Ok(services)
    }
}
