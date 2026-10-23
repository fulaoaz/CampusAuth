//! Common application actions shared by desktop IPC and the server API.
use crate::{core::{AccountInput, Core, built_in_profiles}, diagnostics, power, updates};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

pub fn dispatch(core: &Core, method: &str, params: Option<Value>) -> Result<Value> {
    let data = core.data_dir();
    let input = params.as_ref();
    let value = match method {
        "health" => json!({"ok":true,"version":env!("CARGO_PKG_VERSION")}),
        "app/info" => json!({"version":env!("CARGO_PKG_VERSION"),"data_dir":data,"platform":std::env::consts::OS,"desktop":false,"mode":"server","protocol":"ruijie-eportal","author":"fulao","blog":"https://fulao.cc"}),
        "snapshot" => serde_json::to_value(core.snapshot()?)?,
        "status" => serde_json::to_value(core.status())?,
        "accounts" => serde_json::to_value(core.accounts()?)?,
        "accounts/save" => serde_json::to_value(core.save_account(serde_json::from_value::<AccountInput>(params.context("缺少账号信息")?)?)?)?,
        "accounts/delete" => json!({"ok":core.delete_account(input.and_then(|p|p["id"].as_str()).context("缺少账号标识")?)?}),
        "accounts/reorder" => serde_json::to_value(core.reorder_accounts(&serde_json::from_value::<Vec<String>>(input.context("缺少账号顺序")?["ids"].clone())?)?)?,
        "connect" => {
            let account = input.and_then(|p| p["account_id"].as_str()).filter(|id|!id.is_empty());
            let force = input.and_then(|p|p["force"].as_bool()).unwrap_or(false);
            core.connect(account,force,input.and_then(|p|p["confirm_switch"].as_bool()).unwrap_or(false))?
        }
        "disconnect" => serde_json::to_value(core.logout()?)?,
        "reconnect" => match params { Some(value)=>serde_json::to_value(core.set_reconnect(serde_json::from_value(value)?)?)?,None=>serde_json::to_value(core.reconnect_settings()?)? },
        "reconnect/tick" => serde_json::to_value(core.reconnect_tick()?)?,
        "connection" => match params { Some(value)=>serde_json::to_value(core.set_connection_config(serde_json::from_value(value)?)?)?,None=>serde_json::to_value(core.connection_config()?)? },
        "services" => serde_json::to_value(core.services()?)?,
        "profiles" => serde_json::to_value(built_in_profiles())?,
        "logs" => serde_json::to_value(core.logs())?,
        "power" => match params {Some(value)=>{let cfg=serde_json::from_value(value)?;power::save(&data.join("power_draft.json"),&cfg)?;serde_json::to_value(cfg)?},None=>serde_json::to_value(power::load(&data.join("power_schedule.json"))?)?},
        "power/preview" => power::preview(data,&serde_json::from_value(params.context("缺少电源计划")?)?)?,
        "power/install" => power::install(data,&serde_json::from_value(params.context("缺少电源计划")?)?,true)?,
        "power/remove" => power::remove(data,true)?,
        "power/status" => power::status(data)?,
        "power/countdown" => power::countdown_status(data)?,
        "power/cancel" => power::cancel_pending(data)?,
        "diagnostics/export" => diagnostics::export(core)?,
        "updates/check" => updates::check()?,
        _ => bail!("不支持的操作：{method}"),
    };
    Ok(value)
}
