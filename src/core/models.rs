use serde::{Deserialize, Serialize};

fn enabled_default() -> bool {
    true
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ConnectionConfig {
    pub profile_id: String,
    pub display_name: String,
    pub gateway: String,
    pub service: String,
    pub user_group: String,
    pub probe_urls: Vec<String>,
    pub discovery_urls: Vec<String>,
    pub timeout_seconds: u64,
    pub retry_per_account: u32,
    pub retry_interval_seconds: u64,
    pub password_encrypt: bool,
}

impl Default for ConnectionConfig {
    fn default() -> Self {
        Self {
            profile_id: "qhu".into(),
            display_name: "青海大学".into(),
            gateway: "http://210.27.177.172".into(),
            service: "校园移动".into(),
            user_group: "本科生组".into(),
            probe_urls: vec![
                "http://connect.rom.miui.com/generate_204".into(),
                "http://www.baidu.com".into(),
                "http://1.1.1.1".into(),
            ],
            discovery_urls: vec!["http://www.msftconnecttest.com/redirect".into()],
            timeout_seconds: 8,
            retry_per_account: 2,
            retry_interval_seconds: 3,
            password_encrypt: false,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct SchoolProfile {
    pub id: String,
    pub name: String,
    pub protocol: String,
    pub description: String,
    pub config: ConnectionConfig,
}

pub fn built_in_profiles() -> Vec<SchoolProfile> {
    let qhu = ConnectionConfig::default();
    let custom = ConnectionConfig {
        profile_id: "custom".into(),
        display_name: "自定义 ePortal 校园网".into(),
        gateway: String::new(),
        service: "[-1-1]系统默认服务[-1-1]".into(),
        user_group: String::new(),
        ..qhu.clone()
    };
    vec![
        SchoolProfile {
            id: "qhu".into(),
            name: "青海大学".into(),
            protocol: "ruijie-eportal".into(),
            description: "内置锐捷 ePortal / SAM+ 认证参数。".into(),
            config: qhu,
        },
        SchoolProfile {
            id: "custom".into(),
            name: "自定义 ePortal 校园网".into(),
            protocol: "ruijie-eportal".into(),
            description: "适用于兼容锐捷 ePortal / SAM+ 的校园网；需填写学校网关和服务。".into(),
            config: custom,
        },
    ]
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
    pub label: String,
    pub username: String,
    pub service: String,
    pub enabled: bool,
    pub note: String,
    pub last_result: String,
    pub last_ok: Option<bool>,
    pub has_password: bool,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct AccountInput {
    pub id: Option<String>,
    pub label: String,
    pub username: String,
    pub password: String,
    pub service: String,
    #[serde(default = "enabled_default")]
    pub enabled: bool,
    pub note: String,
}

impl Default for AccountInput {
    fn default() -> Self {
        Self {
            id: None,
            label: String::new(),
            username: String::new(),
            password: String::new(),
            service: String::new(),
            enabled: true,
            note: String::new(),
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ReconnectSettings {
    pub enabled: bool,
    pub interval_seconds: u64,
    pub fail_threshold: u32,
}

impl Default for ReconnectSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_seconds: 180,
            fail_threshold: 3,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Status {
    pub online: Option<bool>,
    pub network_online: Option<bool>,
    pub portal_online: Option<bool>,
    pub reason: String,
    pub message: String,
    pub username: String,
    pub service: String,
    pub local_ip: String,
    pub checked_at: i64,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ActionResult {
    pub ok: bool,
    pub message: String,
    pub busy: bool,
}

impl ActionResult {
    pub(crate) fn new(ok: bool, message: impl Into<String>) -> Self {
        Self {
            ok,
            message: message.into(),
            busy: false,
        }
    }

    pub(crate) fn busy() -> Self {
        Self {
            ok: false,
            message: "已有认证或设置操作正在进行，请稍后重试。".into(),
            busy: true,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ReconnectResult {
    pub ok: bool,
    pub skipped: bool,
    pub attempted: bool,
    pub reconnected: bool,
    pub online: Option<bool>,
    pub reason: String,
    pub consecutive_failures: u32,
    pub busy: bool,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Service {
    pub value: String,
    pub name: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub accounts: Vec<Account>,
    pub reconnect: ReconnectSettings,
    pub logs: Vec<String>,
    pub config: ConnectionConfig,
}
