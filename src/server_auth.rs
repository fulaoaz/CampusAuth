//! Local administrator credentials and revocable browser sessions.
//! Password creation/reset is deliberately available only from the local CLI/TUI.

use std::{
    collections::HashMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use argon2::{
    Algorithm, Argon2, Params, PasswordHash, PasswordHasher, PasswordVerifier, Version,
    password_hash::SaltString,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const SESSION_COOKIE: &str = "campus_auth_session";
pub const SESSION_SECONDS: u64 = 12 * 60 * 60;
const CREDENTIAL_FILE: &str = "admin_auth.json";

#[derive(Deserialize, Serialize)]
struct Credentials {
    version: u32,
    username: String,
    password_hash: String,
    revision: String,
}

struct Session {
    revision: String,
    expires: Instant,
}

#[derive(Default)]
struct LoginAttempts {
    failures: u32,
    retry_after: Option<Instant>,
}

pub struct Administrator {
    path: PathBuf,
    sessions: Mutex<HashMap<String, Session>>,
    attempts: Mutex<LoginAttempts>,
}

#[derive(Debug)]
pub enum LoginError {
    NotConfigured,
    Invalid,
    RateLimited,
    Storage,
}

fn read(path: &Path) -> Result<Option<Credentials>> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let value: Credentials =
        serde_json::from_slice(&bytes).context("管理员配置损坏，请在本机重设密码")?;
    if value.version != 1
        || value.username != "admin"
        || !value.password_hash.starts_with("$argon2id$")
    {
        bail!("管理员配置格式不受支持，请在本机重设密码");
    }
    Ok(Some(value))
}

fn engine() -> Result<Argon2<'static>> {
    let params =
        Params::new(19_456, 2, 1, None).map_err(|_| anyhow::anyhow!("无法配置密码保护参数"))?;
    Ok(Argon2::new(Algorithm::Argon2id, Version::V0x13, params))
}

pub fn configured(data: &Path) -> Result<bool> {
    Ok(read(&data.join(CREDENTIAL_FILE))?.is_some())
}

pub fn set_password(data: &Path, password: &str) -> Result<()> {
    if password.chars().count() < 12 || password.len() > 1024 {
        bail!("管理密码需要至少 12 个字符，且不能超过 1024 字节");
    }
    fs::create_dir_all(data)?;
    let path = data.join(CREDENTIAL_FILE);
    if fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
        bail!("管理员配置不能使用符号链接");
    }
    let salt = SaltString::encode_b64(Uuid::new_v4().as_bytes())
        .map_err(|_| anyhow::anyhow!("无法生成密码保护盐值"))?;
    let value = Credentials {
        version: 1,
        username: "admin".into(),
        password_hash: engine()?
            .hash_password(password.as_bytes(), &salt)
            .map_err(|_| anyhow::anyhow!("无法保护管理密码"))?
            .to_string(),
        revision: Uuid::new_v4().simple().to_string(),
    };
    let mut temporary = tempfile::NamedTempFile::new_in(data)?;
    temporary.write_all(&serde_json::to_vec_pretty(&value)?)?;
    temporary.as_file().sync_all()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        fs::set_permissions(temporary.path(), fs::Permissions::from_mode(0o600))?;
        // A root TUI may reset a service-owned password without making it unreadable
        // to that service. Preserve the known file owner, or the private data owner
        // for first setup. Never recursively adopt unrelated files.
        let metadata = fs::metadata(&path).or_else(|_| fs::metadata(data))?;
        let owner = format!("{}:{}", metadata.uid(), metadata.gid());
        if std::process::Command::new("id").arg("-u").output()?.stdout == b"0\n" {
            let status = std::process::Command::new("chown")
                .arg(owner)
                .arg(temporary.path())
                .status()?;
            if !status.success() {
                bail!("无法保留管理员配置的服务用户权限");
            }
        }
    }
    temporary.persist(&path).map_err(|error| error.error)?;
    Ok(())
}

impl Administrator {
    pub fn new(data: &Path) -> Self {
        Self {
            path: data.join(CREDENTIAL_FILE),
            sessions: Mutex::new(HashMap::new()),
            attempts: Mutex::new(LoginAttempts::default()),
        }
    }

    pub fn configured(&self) -> Result<bool> {
        Ok(read(&self.path)?.is_some())
    }

    pub fn login(&self, password: &str) -> std::result::Result<String, LoginError> {
        {
            let mut attempts = self.attempts.lock().map_err(|_| LoginError::Storage)?;
            if attempts
                .retry_after
                .is_some_and(|until| until > Instant::now())
            {
                return Err(LoginError::RateLimited);
            }
            if attempts.retry_after.is_some() {
                *attempts = LoginAttempts::default();
            }
        }
        let credential = read(&self.path)
            .map_err(|_| LoginError::Storage)?
            .ok_or(LoginError::NotConfigured)?;
        let hash = PasswordHash::new(&credential.password_hash).map_err(|_| LoginError::Storage)?;
        let valid = password.len() <= 1024
            && engine()
                .map_err(|_| LoginError::Storage)?
                .verify_password(password.as_bytes(), &hash)
                .is_ok();
        if !valid {
            let mut attempts = self.attempts.lock().map_err(|_| LoginError::Storage)?;
            attempts.failures += 1;
            if attempts.failures >= 5 {
                attempts.retry_after = Some(Instant::now() + Duration::from_secs(30));
            }
            return Err(LoginError::Invalid);
        }
        *self.attempts.lock().map_err(|_| LoginError::Storage)? = LoginAttempts::default();
        let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        let mut sessions = self.sessions.lock().map_err(|_| LoginError::Storage)?;
        sessions.retain(|_, session| session.expires > Instant::now());
        if sessions.len() >= 64 {
            if let Some(oldest) = sessions
                .iter()
                .min_by_key(|(_, value)| value.expires)
                .map(|(key, _)| key.clone())
            {
                sessions.remove(&oldest);
            }
        }
        sessions.insert(
            token.clone(),
            Session {
                revision: credential.revision,
                expires: Instant::now() + Duration::from_secs(SESSION_SECONDS),
            },
        );
        Ok(token)
    }

    pub fn authenticated(&self, token: &str) -> bool {
        if token.len() != 64 || !token.bytes().all(|ch| ch.is_ascii_hexdigit()) {
            return false;
        }
        let Ok(Some(credentials)) = read(&self.path) else {
            return false;
        };
        let Ok(mut sessions) = self.sessions.lock() else {
            return false;
        };
        sessions.retain(|_, session| {
            session.expires > Instant::now() && session.revision == credentials.revision
        });
        sessions.contains_key(token)
    }

    pub fn logout(&self, token: &str) {
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.remove(token);
        }
    }
}

pub fn cookie_token(header: Option<&str>) -> Option<&str> {
    header?.split(';').find_map(|part| {
        let (name, value) = part.trim().split_once('=')?;
        (name == SESSION_COOKIE).then_some(value)
    })
}
