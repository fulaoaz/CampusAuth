use crate::core::Core;
use anyhow::Result;
use chrono::Local;
use regex::Regex;
use serde_json::{Value,json};

pub fn export(core:&Core)->Result<Value> {
    let mut snapshot=serde_json::to_value(core.snapshot()?)?;
    snapshot["logs"]=serde_json::to_value(core.diagnostic_logs())?;
    if let Some(accounts)=snapshot["accounts"].as_array_mut(){
        for (i,account) in accounts.iter_mut().enumerate(){
            account["username"]=json!(format!("账号{}",i+1));
            account["label"]=json!(format!("账号{}",i+1));
            account["note"]=json!("");
            account["id"]=json!(format!("account-{}",i+1));
        }
    }
    // Never export arbitrary settings files, authentication hashes, account stores,
    // portal cookies or a complete environment dump.
    let mut report=json!({"应用版本":env!("CARGO_PKG_VERSION"),"系统":std::env::consts::OS,"架构":std::env::consts::ARCH,
        "导出时间":Local::now().to_rfc3339(),"数据":snapshot,"范围":"应用版本、学校设置、已脱敏账号列表和最近日志；不含密码、管理凭据、会话令牌"});
    report["数据"]["config"]["gateway"]=json!("[已隐藏网关地址]");
    report["数据"]["config"]["probe_urls"]=json!("[已隐藏检测地址]");
    report["数据"]["config"]["discovery_urls"]=json!("[已隐藏发现地址]");
    let identities=core.accounts()?.into_iter().flat_map(|a|[a.username,a.id,a.label]).filter(|v|!v.is_empty()).collect::<Vec<_>>();
    let ipv4=Regex::new(r"\b(?:\d{1,3}\.){3}\d{1,3}\b")?;
    let ipv6=Regex::new(r"(?i)\b(?:[0-9a-f]{1,4}:){2,}[0-9a-f:]*\b")?;
    fn redact(value:&mut Value,ids:&[String],ipv4:&Regex,ipv6:&Regex){
        match value{
            Value::String(text)=>{for id in ids{*text=text.replace(id,"[已隐藏账号]");}*text=ipv4.replace_all(text,"[已隐藏IP]").into_owned();*text=ipv6.replace_all(text,"[已隐藏IP]").into_owned();}
            Value::Array(values)=>for v in values{redact(v,ids,ipv4,ipv6);},
            Value::Object(map)=>for v in map.values_mut(){redact(v,ids,ipv4,ipv6);},
            _=>{}
        }
    }
    redact(&mut report,&identities,&ipv4,&ipv6);
    let text=serde_json::to_string_pretty(&report)?;
    Ok(json!({"ok":true,"filename":format!("CampusAuth-诊断-{}.json",Local::now().format("%Y%m%d-%H%M%S")),"content":text,"mime_type":"application/json"}))
}
