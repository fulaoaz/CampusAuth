//! Explicit, optional OS power schedules. Authentication never invokes this module.
use anyhow::{Context, Result, bail, ensure};
use chrono::{Datelike, Duration, Local, NaiveTime, TimeZone};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, path::{Path, PathBuf}, process::{Command, Output}, thread, time};

const DAYS: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];
const TASKS: [&str; 2] = ["CampusAuth-PowerSuspend", "CampusAuth-PowerWake"];
const UNITS: [&str; 4] = ["campus-auth-power.service", "campus-auth-power.timer", "campus-auth-wake.service", "campus-auth-wake.timer"];
static HELPER_EXECUTABLE:std::sync::OnceLock<PathBuf>=std::sync::OnceLock::new();
/// Desktop AppImages must use their bundled static CLI for privileged schedules,
/// since copying only the GUI ELF would lose its GTK/WebKit runtime environment.
pub fn set_helper_executable(path:PathBuf)->Result<()> {
    ensure!(path.is_absolute() && path.is_file(),"电源辅助程序不存在");
    HELPER_EXECUTABLE.set(path).map_err(|_|anyhow::anyhow!("电源辅助程序已初始化"))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct PowerConfig {
    pub enabled: bool,
    pub shutdown_time: String,
    pub wake_time: String,
    pub days: Vec<String>,
    pub shutdown_delay_seconds: u64,
    pub wake_method: String,
    pub shutdown_action: String,
}
impl Default for PowerConfig {
    fn default() -> Self {
        Self { enabled: false, shutdown_time: "23:00".into(), wake_time: "07:00".into(), days: DAYS.iter().map(|s| s.to_string()).collect(), shutdown_delay_seconds: 60, wake_method: "timer".into(), shutdown_action: "suspend".into() }
    }
}
pub fn validate(cfg: &PowerConfig) -> Result<()> {
    for value in [&cfg.shutdown_time, &cfg.wake_time] {
        let clock = NaiveTime::parse_from_str(value, "%H:%M").context("时间必须是 HH:MM")?;
        ensure!(clock.format("%H:%M").to_string() == *value, "时间必须是两位 HH:MM");
    }
    ensure!(!cfg.days.is_empty() && cfg.days.iter().all(|d| DAYS.contains(&d.as_str())), "请选择有效的执行日");
    ensure!(cfg.shutdown_delay_seconds <= 3600, "执行延时不得超过 3600 秒");
    ensure!(["timer", "rtc"].contains(&cfg.wake_method.as_str()), "唤醒方式无效");
    ensure!(["suspend", "hibernate", "shutdown"].contains(&cfg.shutdown_action.as_str()), "电源动作无效");
    Ok(())
}
pub fn load(path: &Path) -> Result<PowerConfig> {
    if !path.exists() { return Ok(PowerConfig::default()); }
    let cfg: PowerConfig = serde_json::from_slice(&fs::read(path)?)
        .context("电源配置损坏，请修复配置后重试")?;
    validate(&cfg)?;
    Ok(cfg)
}
pub fn save(path: &Path, cfg: &PowerConfig) -> Result<()> {
    validate(cfg)?;
    write_state(path,&serde_json::to_value(cfg)?)
}
fn write_state(path:&Path,value:&Value)->Result<()> {
    let parent = path.parent().context("电源配置缺少父目录")?;
    fs::create_dir_all(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    use std::io::Write;
    temp.write_all(&serde_json::to_vec_pretty(value)?)?;
    temp.as_file().sync_all()?;
    #[cfg(unix)] {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o600))?;
        use std::os::unix::fs::MetadataExt;
        let owner=fs::metadata(parent)?;
        let own=fs::metadata(temp.path())?;
        if own.uid()==0 {std::os::unix::fs::chown(temp.path(),Some(owner.uid()),Some(owner.gid()))?;}
    }
    temp.persist(path).map_err(|e| e.error)?;
    Ok(())
}
fn xml(text: &str) -> String { text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;") }
fn unit_escape(text: &str) -> Result<String> {
    ensure!(!text.contains(['\n','\r','\0']), "路径不能含换行或 NUL");
    Ok(format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\"").replace('%', "%%").replace('$', "$$")))
}
fn task_xml(clock: &str, days: &[String], exe: &str, args: &str, wake: bool) -> String {
    let names = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];
    let selected = DAYS.iter().enumerate().filter(|(_,d)| days.iter().any(|x| x == **d)).map(|(i,_)| format!("<{} />", names[i])).collect::<String>();
    format!(r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task"><Triggers><CalendarTrigger><StartBoundary>2026-01-05T{clock}:00</StartBoundary><Enabled>true</Enabled><ScheduleByWeek><WeeksInterval>1</WeeksInterval><DaysOfWeek>{selected}</DaysOfWeek></ScheduleByWeek></CalendarTrigger></Triggers><Principals><Principal id="Author"><UserId>S-1-5-18</UserId><LogonType>ServiceAccount</LogonType><RunLevel>HighestAvailable</RunLevel></Principal></Principals><Settings><MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy><DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries><StopIfGoingOnBatteries>false</StopIfGoingOnBatteries><StartWhenAvailable>false</StartWhenAvailable><Enabled>true</Enabled><WakeToRun>{wake}</WakeToRun><ExecutionTimeLimit>PT2H</ExecutionTimeLimit></Settings><Actions Context="Author"><Exec><Command>{}</Command><Arguments>{}</Arguments></Exec></Actions></Task>"#, xml(exe), xml(args))
}
pub fn render_schedule(system: &str, executable: &Path, data_dir: &Path, cfg: &PowerConfig) -> Result<Value> {
    validate(cfg)?;
    let exe = executable.to_string_lossy();
    let config = data_dir.join("power_schedule.json");
    ensure!(!exe.contains(['\n','\r','\0','"']) && !config.to_string_lossy().contains(['\n','\r','\0','"']), "计划路径含不支持字符");
    if system == "windows" {
        ensure!(cfg.wake_method == "timer", "Windows 请使用 timer；RTC 由固件设置");
        let args = format!("power run --config \"{}\" --apply", config.display());
        return Ok(json!({"platform":"windows","enabled":cfg.enabled,"warning":"睡眠/休眠唤醒需要系统唤醒计时器和硬件支持。完全关机开机需要 BIOS/UEFI RTC 或 WOL。","task_xml":{
            TASKS[0]:task_xml(&cfg.shutdown_time,&cfg.days,&exe,&args,false),
            TASKS[1]:task_xml(&cfg.wake_time,&cfg.days,"C:\\Windows\\System32\\cmd.exe","/c exit 0",true)
        },"executable":exe,"config":config}));
    }
    ensure!(system == "linux", "不支持的系统");
    let selected = DAYS.iter().filter(|d| cfg.days.iter().any(|x| x == **d)).map(|d| format!("{}{}",d[..1].to_uppercase(),&d[1..])).collect::<Vec<_>>().join(",");
    let command = format!("{} power run --config {} --apply",unit_escape(&exe)?,unit_escape(&config.to_string_lossy())?);
    Ok(json!({"platform":"linux","enabled":cfg.enabled,"warning":"WakeSystem/RTC 唤醒依赖硬件；完全关机后的开机需要固件支持。","units":{
        UNITS[0]:format!("[Unit]\nDescription=CampusAuth scheduled power action\n\n[Service]\nType=oneshot\nExecStart={command}\nTimeoutStartSec=infinity\n"),
        UNITS[1]:format!("[Unit]\nDescription=CampusAuth power timer\n\n[Timer]\nOnCalendar={selected} *-*-* {}:00\nPersistent=false\nAccuracySec=1s\nUnit={}\n\n[Install]\nWantedBy=timers.target\n",cfg.shutdown_time,UNITS[0]),
        UNITS[2]:"[Unit]\nDescription=CampusAuth wake event\n\n[Service]\nType=oneshot\nExecStart=/usr/bin/true\n",
        UNITS[3]:format!("[Unit]\nDescription=CampusAuth wake timer\n\n[Timer]\nOnCalendar={selected} *-*-* {}:00\nWakeSystem=true\nPersistent=false\nAccuracySec=1s\nUnit={}\n\n[Install]\nWantedBy=timers.target\n",cfg.wake_time,UNITS[2])
    },"executable":exe,"config":config}))
}
fn protected_exe() -> PathBuf {
    #[cfg(windows)] { PathBuf::from(std::env::var_os("ProgramFiles").unwrap_or_else(|| "C:\\Program Files".into())).join("CampusAuth-Power").join("campus-auth.exe") }
    #[cfg(not(windows))] { PathBuf::from("/usr/local/lib/campus-auth-power/campus-auth") }
}
fn secure_program_directory(path: &Path) -> Result<()> {
    ensure!(path.is_absolute(), "系统程序目录必须使用绝对路径");
    // Never follow an attacker-controlled link while installing a privileged task.
    for ancestor in path.ancestors() {
        if let Ok(metadata) = fs::symlink_metadata(ancestor) {
            ensure!(!metadata.file_type().is_symlink(), "系统程序目录不能经过符号链接");
            #[cfg(windows)] {
                use std::os::windows::fs::MetadataExt;
                ensure!(metadata.file_attributes() & 0x400 == 0, "系统程序目录不能经过重解析点");
            }
            #[cfg(unix)] {
                use std::os::unix::fs::MetadataExt;
                ensure!(metadata.uid() == 0 && metadata.mode() & 0o022 == 0, "系统程序目录必须由 root 所有且不能被普通用户写入");
            }
        }
    }
    fs::create_dir_all(path)?;
    #[cfg(unix)] {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let metadata=fs::symlink_metadata(path)?;
        ensure!(metadata.uid()==0 && !metadata.file_type().is_symlink(),"系统程序目录所有者不正确");
        fs::set_permissions(path,fs::Permissions::from_mode(0o755))?;
    }
    #[cfg(windows)] {
        let value=path.to_string_lossy();
        check(cmd("icacls.exe", &[&value,"/setowner","*S-1-5-32-544"])?)?;
        check(cmd("icacls.exe", &[&value,"/reset"])?)?;
        check(cmd("icacls.exe", &[&value,"/inheritance:r","/grant:r","*S-1-5-18:(OI)(CI)(F)","*S-1-5-32-544:(OI)(CI)(F)","*S-1-5-32-545:(OI)(CI)(RX)"])?)?;
    }
    Ok(())
}
fn admin_command(exe:&Path, config:&Path)->String {
    #[cfg(windows)] { format!("& '{}' power install --config '{}' --apply",exe.display().to_string().replace('\'',"''"),config.display().to_string().replace('\'',"''")) }
    #[cfg(not(windows))] { format!("sudo '{}' power install --config '{}' --apply",exe.display().to_string().replace('\'',"'\\''"),config.display().to_string().replace('\'',"'\\''")) }
}
fn owned_state(data:&Path)->Result<PathBuf> {
    if data.exists(){Ok(data.canonicalize()?)}else if data.is_absolute(){Ok(data.to_path_buf())}else{Ok(std::env::current_dir()?.join(data))}
}
fn verify_owner(data:&Path)->Result<()> {
    let marker=protected_exe().parent().context("程序目录无效")?.join("owner.json");
    if marker.exists(){
        let value:Value=serde_json::from_slice(&fs::read(marker)?)?;
        ensure!(value["data_dir"].as_str()==Some(owned_state(data)?.to_string_lossy().as_ref()),"系统电源计划属于另一个 CampusAuth 数据目录，请从原应用中管理");
    }
    Ok(())
}
pub fn preview(data: &Path, cfg: &PowerConfig) -> Result<Value> {
    render_schedule(std::env::consts::OS, &protected_exe(), data, cfg)
}
fn output(mut command: Command) -> Result<Output> {
    #[cfg(windows)] {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command.output().context("无法运行系统计划命令")
}
fn cmd(program: &str, args: &[&str]) -> Result<Output> { let mut c=Command::new(program); c.args(args); output(c) }
fn require_admin() -> Result<()> {
    #[cfg(windows)] {
        ensure!(unsafe { windows_sys::Win32::UI::Shell::IsUserAnAdmin() } != 0, "请以管理员身份运行以应用系统电源计划；当前配置仅作预览");
    }
    #[cfg(unix)] {
        ensure!(cmd("id", &["-u"])?.stdout == b"0\n", "应用系统电源计划需要 sudo/root 权限");
    }
    Ok(())
}
fn check(result: Output) -> Result<()> {
    ensure!(result.status.success(), "系统计划操作失败：{}", String::from_utf8_lossy(&result.stderr));
    Ok(())
}
#[cfg(windows)]
fn command_text(bytes:&[u8])->String {
    if bytes.starts_with(&[0xff,0xfe]) || bytes.iter().skip(1).step_by(2).take(30).filter(|b|**b==0).count()>10 {
        let start=if bytes.starts_with(&[0xff,0xfe]){2}else{0};
        String::from_utf16_lossy(&bytes[start..].chunks_exact(2).map(|c|u16::from_le_bytes([c[0],c[1]])).collect::<Vec<_>>())
    }else{String::from_utf8_lossy(bytes).into_owned()}
}
pub fn install(data: &Path, cfg: &PowerConfig, apply: bool) -> Result<Value> {
    ensure!(cfg.enabled, "请先启用电源计划");
    let plan = preview(data,cfg)?;
    if !apply { return Ok(json!({"ok":true,"dry_run":true,"plan":plan})); }
    if let Err(e) = require_admin() {
        save(&data.join("pending_power_schedule.json"),cfg)?;
        return Ok(json!({"ok":false,"permission_error":true,"message":e.to_string(),"pending_config":data.join("pending_power_schedule.json"),"admin_command":admin_command(&std::env::current_exe()?,&data.join("pending_power_schedule.json"))}));
    }
    let target=protected_exe();
    let parent=target.parent().context("程序目录无效")?;
    secure_program_directory(parent)?;
    verify_owner(data)?;
    let running=match HELPER_EXECUTABLE.get(){Some(path)=>path.clone(),None=>std::env::current_exe()?};
    if running != target {
        let temp=parent.join(format!("campus-auth-{}.tmp",uuid::Uuid::new_v4()));
        let result=(||->Result<()> {
            let mut source=fs::File::open(&running)?;
            let mut destination=fs::OpenOptions::new().write(true).create_new(true).open(&temp)?;
            std::io::copy(&mut source,&mut destination)?;
            destination.sync_all()?;drop(destination);
            if let Ok(metadata)=fs::symlink_metadata(&target) {ensure!(!metadata.file_type().is_symlink(),"系统程序不能是符号链接");}
            #[cfg(windows)] if target.exists(){fs::remove_file(&target)?;}
            fs::rename(&temp,&target)?;Ok(())
        })();
        let _=fs::remove_file(&temp);result?;
    }
    #[cfg(unix)] {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&target,fs::Permissions::from_mode(0o755))?;
    }
    #[cfg(windows)] apply_windows(&plan)?;
    #[cfg(unix)] apply_linux(&plan)?;
    save(&data.join("power_schedule.json"),cfg)?;
    write_state(&parent.join("owner.json"),&json!({"data_dir":owned_state(data)?,"application":"CampusAuth","version":1}))?;
    Ok(json!({"ok":true,"dry_run":false,"plan":plan}))
}
#[cfg(windows)]
fn apply_windows(plan:&Value)->Result<()> {
    let mut backups=BTreeMap::new();
    for name in TASKS {
        let old=cmd("schtasks.exe", &["/Query","/TN",name,"/XML"])?;
        backups.insert(name,if old.status.success(){Some(command_text(&old.stdout))}else{None});
    }
    for name in TASKS {
        let task=plan["task_xml"][name].as_str().context("计划 XML 缺失")?;
        if let Err(error)=register_task(name,task) {
            for (name,backup) in &backups {
                if let Some(xml)=backup { let _=register_task(name,xml); }
                else { let _=cmd("schtasks.exe", &["/Delete","/TN",name,"/F"]); }
            }
            return Err(error.context("已尝试恢复原系统任务"));
        }
    }
    Ok(())
}
#[cfg(windows)]
fn register_task(name:&str, xml:&str)->Result<()> {
    let mut temp=tempfile::NamedTempFile::new()?;
    use std::io::Write;
    temp.write_all(&[0xff,0xfe])?;
    for ch in xml.encode_utf16() { temp.write_all(&ch.to_le_bytes())?; }
    temp.flush()?;
    check(cmd("schtasks.exe", &["/Create","/TN",name,"/XML",&temp.path().to_string_lossy(),"/F"])?)
}
#[cfg(unix)]
fn apply_linux(plan:&Value)->Result<()> {
    let root=Path::new("/etc/systemd/system");
    let mut backups=BTreeMap::new();
    for name in UNITS { backups.insert(name,fs::read(root.join(name)).ok()); }
    let mut old_states=Vec::new();
    for name in [UNITS[1],UNITS[3]] {
        old_states.push((name,cmd("systemctl", &["is-enabled",name])?.status.success(),cmd("systemctl", &["is-active",name])?.status.success()));
    }
    let apply = || -> Result<()> {
        for name in UNITS { fs::write(root.join(name),plan["units"][name].as_str().context("系统服务内容缺失")?)?; }
        check(cmd("systemctl", &["daemon-reload"])?)?;
        check(cmd("systemctl", &["enable","--now",UNITS[1],UNITS[3]])?)
    };
    if let Err(e)=apply() {
        for (name,backup) in backups { if let Some(bytes)=backup { fs::write(root.join(name),bytes)?; } else { let _=fs::remove_file(root.join(name)); } }
        let _=cmd("systemctl", &["daemon-reload"]);
        for (name,enabled,active) in old_states {
            let _=cmd("systemctl", &[if enabled{"enable"}else{"disable"},name]);
            let _=cmd("systemctl", &[if active{"start"}else{"stop"},name]);
        }
        return Err(e.context("已恢复原系统服务文件"));
    }
    Ok(())
}
pub fn remove(data:&Path,apply:bool)->Result<Value> {
    if !apply { return Ok(json!({"ok":true,"dry_run":true,"tasks":TASKS,"units":UNITS})); }
    require_admin()?;
    verify_owner(data)?;
    #[cfg(windows)] for name in TASKS {
        if cmd("schtasks.exe", &["/Query","/TN",name])?.status.success() {
            // /End can fail for an idle task. /Delete itself must succeed.
            let _=cmd("schtasks.exe", &["/End","/TN",name]);
            check(cmd("schtasks.exe", &["/Delete","/TN",name,"/F"])?)?;
        }
    }
    #[cfg(unix)] {
        let root=Path::new("/etc/systemd/system");
        for name in [UNITS[1],UNITS[3]] {
            if root.join(name).exists() { check(cmd("systemctl", &["disable","--now",name])?)?; }
        }
        for name in [UNITS[0],UNITS[2]] {
            if root.join(name).exists(){check(cmd("systemctl", &["stop",name])?)?;}
        }
        for name in UNITS { if root.join(name).exists(){fs::remove_file(root.join(name))?;} }
        check(cmd("systemctl", &["daemon-reload"])?)?;
    }
    let path=data.join("power_schedule.json"); let mut cfg=load(&path)?;cfg.enabled=false;save(&path,&cfg)?;
    // Keep the protected binary until no scheduler can be referring to it.
    let target=protected_exe();
    if target.exists() { fs::remove_file(target)?; }
    let marker=protected_exe().parent().unwrap().join("owner.json");
    if marker.exists(){fs::remove_file(marker)?;}
    Ok(json!({"ok":true,"dry_run":false}))
}
pub fn cleanup_owned(data:&Path)->Result<Value> {
    let marker=protected_exe().parent().context("程序目录无效")?.join("owner.json");
    if !marker.exists(){return Ok(json!({"ok":true,"skipped":true,"message":"没有属于此数据目录的系统电源计划"}));}
    if !owns_schedule(data)?{return Ok(json!({"ok":true,"skipped":true,"message":"系统电源计划属于其他数据目录，已保留"}));}
    remove(data,true)
}
pub fn owns_schedule(data:&Path)->Result<bool> {
    let marker=protected_exe().parent().context("程序目录无效")?.join("owner.json");
    if !marker.exists(){return Ok(false);}
    let value:Value=serde_json::from_slice(&fs::read(marker)?)?;
    Ok(value["data_dir"].as_str()==Some(owned_state(data)?.to_string_lossy().as_ref()))
}
pub fn status(_data:&Path)->Result<Value> {
    let mut tasks=Vec::new();
    #[cfg(windows)] for name in TASKS { let o=cmd("schtasks.exe", &["/Query","/TN",name,"/FO","LIST"])?;tasks.push(json!({"name":name,"installed":o.status.success(),"details":String::from_utf8_lossy(&o.stdout)})); }
    #[cfg(unix)] for name in [UNITS[1],UNITS[3]] { let o=cmd("systemctl", &["is-enabled",name])?;tasks.push(json!({"name":name,"installed":o.status.success(),"details":String::from_utf8_lossy(&o.stdout)})); }
    Ok(json!({"ok":true,"tasks":tasks}))
}
pub fn next_wake(cfg:&PowerConfig,now:chrono::DateTime<Local>)->Result<chrono::DateTime<Local>> {
    validate(cfg)?;
    let time=NaiveTime::parse_from_str(&cfg.wake_time,"%H:%M")?;
    for offset in 0..=8 {
        let day=now.date_naive()+Duration::days(offset);
        if !cfg.days.iter().any(|x| x==DAYS[day.weekday().num_days_from_monday() as usize]) { continue; }
        if let Some(candidate)=Local.from_local_datetime(&day.and_time(time)).earliest() {
            if candidate>now{return Ok(candidate);}
        }
    }
    bail!("下一次唤醒时间无效")
}
pub fn run_scheduled(path:&Path)->Result<()> {
    require_admin()?;
    let cfg=load(path)?;
    if !cfg.enabled{return Ok(());}
    validate(&cfg)?;
    let data=path.parent().context("电源配置目录无效")?;
    let id=uuid::Uuid::new_v4().to_string();
    let execute_at=Local::now().timestamp()+cfg.shutdown_delay_seconds as i64;
    let pending=data.join("power_countdown.json");
    write_state(&pending,&json!({"id":id,"active":true,"action":cfg.shutdown_action,"execute_at":execute_at}))?;
    let deadline=std::time::Instant::now()+time::Duration::from_secs(cfg.shutdown_delay_seconds);
    while std::time::Instant::now()<deadline {
        if canceled(data,&id) || !load(path)?.enabled {finish_countdown(data,&id)?;return Ok(());}
        thread::sleep(time::Duration::from_millis(250).min(deadline.saturating_duration_since(std::time::Instant::now())));
    }
    // Re-read so disabling the plan during its delay prevents the operation.
    let latest=load(path)?;
    if !latest.enabled || canceled(data,&id) {finish_countdown(data,&id)?;return Ok(());}
    finish_countdown(data,&id)?;
    #[cfg(unix)] {
        if cfg.wake_method=="rtc" { check(cmd("rtcwake", &["-m","no","-t",&next_wake(&cfg,Local::now())?.timestamp().to_string()])?)?; }
        check(cmd("systemctl", &[if cfg.shutdown_action=="shutdown"{"poweroff"}else{&cfg.shutdown_action}])?)
    }
    #[cfg(windows)] {
        match cfg.shutdown_action.as_str() {
            "shutdown"=>check(cmd("shutdown.exe", &["/s","/t","0"])?),
            "hibernate"=>check(cmd("shutdown.exe", &["/h"])?),
            _=>suspend_windows(),
        }
    }
}
fn canceled(data:&Path,id:&str)->bool {
    fs::read(data.join("power_canceled.json")).ok().and_then(|bytes|serde_json::from_slice::<Value>(&bytes).ok()).is_some_and(|v|v["id"].as_str()==Some(id))
}
fn finish_countdown(data:&Path,id:&str)->Result<()> {
    let path=data.join("power_countdown.json");
    if let Ok(bytes)=fs::read(&path) {
        if let Ok(mut value)=serde_json::from_slice::<Value>(&bytes){
            if value["id"].as_str()==Some(id){value["active"]=json!(false);write_state(&path,&value)?;}
        }
    }
    Ok(())
}
pub fn countdown_status(data:&Path)->Result<Value> {
    let mut value=match fs::read(data.join("power_countdown.json")) {
        Ok(bytes)=>serde_json::from_slice::<Value>(&bytes).context("无法读取电源倒计时状态")?,
        Err(e) if e.kind()==std::io::ErrorKind::NotFound=>return Ok(json!({"ok":true,"active":false})),
        Err(e)=>return Err(e).context("无法读取电源倒计时状态"),
    };
    let remaining=value["execute_at"].as_i64().unwrap_or(0)-Local::now().timestamp();
    let id=value["id"].as_str().unwrap_or("");
    let active=value["active"]==true && remaining>=0 && !canceled(data,id);
    value["active"]=json!(active);value["remaining_seconds"]=json!(if active{remaining}else{0});value["ok"]=json!(true);
    Ok(value)
}
pub fn cancel_pending(data:&Path)->Result<Value> {
    let pending=countdown_status(data)?;
    if pending["active"]!=true{return Ok(json!({"ok":true,"message":"当前没有待执行的电源动作"}));}
    let id=pending["id"].as_str().context("倒计时标识无效")?;
    write_state(&data.join("power_canceled.json"),&json!({"id":id,"canceled_at":Local::now().timestamp()}))?;
    Ok(json!({"ok":true,"message":"已取消本次电源动作，下次计划保持不变"}))
}
#[cfg(windows)]
fn suspend_windows()->Result<()> {
    use windows_sys::Win32::{Foundation::{CloseHandle,GetLastError,HANDLE}, Security::*, System::{Threading::{GetCurrentProcess,OpenProcessToken},Power::SetSuspendState}};
    unsafe {
        let mut token:HANDLE=std::ptr::null_mut();
        ensure!(OpenProcessToken(GetCurrentProcess(),TOKEN_ADJUST_PRIVILEGES|TOKEN_QUERY,&mut token)!=0,"无法获取电源权限");
        let result=(||->Result<()> {
            let name:Vec<u16>="SeShutdownPrivilege\0".encode_utf16().collect();
            let mut privileges:TOKEN_PRIVILEGES=std::mem::zeroed();privileges.PrivilegeCount=1;privileges.Privileges[0].Attributes=SE_PRIVILEGE_ENABLED;
            ensure!(LookupPrivilegeValueW(std::ptr::null(),name.as_ptr(),&mut privileges.Privileges[0].Luid)!=0,"无法查询电源权限");
            windows_sys::Win32::Foundation::SetLastError(0);
            ensure!(AdjustTokenPrivileges(token,0,&privileges,0,std::ptr::null_mut(),std::ptr::null_mut())!=0&&GetLastError()==0,"无法启用电源权限");
            ensure!(SetSuspendState(false,false,false),"系统未接受睡眠请求");Ok(())
        })();CloseHandle(token);result
    }
}
