use super::storage::{self,ProcessLock};
use anyhow::{Result,Context};
use chrono::{Local,NaiveDate,Duration};
use std::{fs,io::Write,path::{Path,PathBuf}};

const MAX_DAILY_BYTES:u64=1024*1024;
const DAYS:i64=7;

fn files(dir:&Path)->Vec<(NaiveDate,PathBuf)> {
    let mut result=Vec::new();
    if let Ok(entries)=fs::read_dir(dir){
        for item in entries.flatten(){let path=item.path();let name=item.file_name().to_string_lossy().into_owned();
            if let Some(date)=name.strip_prefix("campus-").and_then(|v|v.strip_suffix(".log")).and_then(|v|NaiveDate::parse_from_str(v,"%Y-%m-%d").ok()){
                if item.file_type().is_ok_and(|t|t.is_file()){result.push((date,path));}
            }
        }
    }
    result.sort_by_key(|(date,_)|*date);result
}
pub fn append(data:&Path,line:&str)->Result<()> {
    let dir=data.join("logs");storage::private_directory(&dir)?;
    let _lock=ProcessLock::acquire(&dir.join("log-store.lock"))?.context("日志正在写入")?;
    let today=Local::now().date_naive();
    for (date,path) in files(&dir){if date<today-Duration::days(DAYS-1){fs::remove_file(path)?;}}
    let file=dir.join(format!("campus-{today}.log"));
    if fs::symlink_metadata(&file).is_ok_and(|m|m.file_type().is_symlink()){anyhow::bail!("日志路径不能是符号链接");}
    let mut options=fs::OpenOptions::new();options.create(true).append(true);
    #[cfg(unix)] {use std::os::unix::fs::OpenOptionsExt;options.mode(0o600);}
    let existed=file.exists();
    let mut handle=options.open(&file)?;
    #[cfg(unix)] {
        use std::os::unix::fs::MetadataExt;
        if !existed && handle.metadata()?.uid()==0 {
            let owner=fs::metadata(&dir)?;std::os::unix::fs::chown(&file,Some(owner.uid()),Some(owner.gid()))?;
        }
    }
    #[cfg(not(unix))] let _=existed;
    if handle.metadata()?.len()>MAX_DAILY_BYTES {
        drop(handle);
        let text=fs::read_to_string(&file)?;
        let tail=text.lines().rev().take(1000).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n");
        fs::write(&file,tail+"\n")?;handle=options.open(file)?;
    }
    writeln!(handle,"{}",line.replace(['\r','\n']," "))?;Ok(())
}
pub fn read(data:&Path)->Result<Vec<String>> {
    let lines=read_history(data)?;
    let skip=lines.len().saturating_sub(500);Ok(lines.into_iter().skip(skip).collect())
}
pub fn read_history(data:&Path)->Result<Vec<String>> {
    let mut lines=Vec::new();
    for (_,path) in files(&data.join("logs")).into_iter().rev().take(DAYS as usize).rev(){
        if fs::metadata(&path)?.len()<=MAX_DAILY_BYTES+64*1024{lines.extend(fs::read_to_string(path)?.lines().map(str::to_owned));}
    }
    Ok(lines)
}
