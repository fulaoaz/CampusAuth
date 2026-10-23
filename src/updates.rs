use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::time::Duration;

pub fn check() -> Result<Value> {
    let client=reqwest::blocking::Client::builder().timeout(Duration::from_secs(15))
        .user_agent("Mozilla/5.0 CampusAuth update checker").build()?;
    let response=client.get("https://api.github.com/repos/fulaoaz/CampusAuth/releases/latest")
        .header("Accept","application/vnd.github+json")
        .header("X-GitHub-Api-Version","2022-11-28").send()
        .context("无法连接更新服务器，请检查网络后重试")?;
    if response.status().as_u16()==404 {return Ok(json!({"ok":true,"available":false,"current_version":env!("CARGO_PKG_VERSION"),"message":"暂未发布新的正式版本"}));}
    ensure!(response.status().is_success(),"更新服务器暂不可用（{}）",response.status());
    let release:Value=response.json().context("更新信息格式无效")?;
    let latest=release["tag_name"].as_str().unwrap_or("").trim_start_matches('v');
    let assets=release["assets"].as_array().into_iter().flatten().filter_map(|asset|{
        let url=asset["browser_download_url"].as_str()?;
        if !url.starts_with("https://github.com/fulaoaz/CampusAuth/releases/download/"){return None;}
        Some(json!({"name":asset["name"],"url":url,"size":asset["size"]}))
    }).collect::<Vec<_>>();
    let version_tuple=|s:&str|s.split('.').take(3).map(|x|x.parse::<u64>().unwrap_or(0)).collect::<Vec<_>>();
    let available=version_tuple(latest)>version_tuple(env!("CARGO_PKG_VERSION"));
    Ok(json!({"ok":true,"available":available,"current_version":env!("CARGO_PKG_VERSION"),"latest_version":latest,
        "release_url":release["html_url"].as_str().filter(|url|url.starts_with("https://github.com/fulaoaz/CampusAuth/releases/")).unwrap_or("https://github.com/fulaoaz/CampusAuth/releases"),"notes":release["body"],"assets":assets,
        "message":if available{"发现新版本，请阅读更新说明后选择下载"}else{"当前已是最新版本"}}))
}
