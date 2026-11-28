use anyhow::{Context, Result, ensure};
use serde_json::{json,Value};
#[derive(Debug)] pub struct SaveError(pub anyhow::Error);
impl std::fmt::Display for SaveError {fn fmt(&self,f:&mut std::fmt::Formatter)->std::fmt::Result{write!(f,"Ошибка сохранения профиля / Cannot save profile: {}",self.0)}}
impl std::error::Error for SaveError{}
use std::{fs,path::{Path,PathBuf},ptr,os::windows::ffi::OsStrExt};
use windows_sys::Win32::{Foundation::{LocalFree,GetLastError,ERROR_ALREADY_EXISTS,CloseHandle,HANDLE},Security::Cryptography::*,Storage::FileSystem::{MoveFileExW,MOVEFILE_REPLACE_EXISTING,MOVEFILE_WRITE_THROUGH},System::Threading::CreateMutexW,Globalization::GetUserDefaultUILanguage};

pub fn wide(s:&std::ffi::OsStr)->Vec<u16>{s.encode_wide().chain(Some(0)).collect()}
pub struct Instance(HANDLE);
impl Instance{pub fn lock()->Result<Self>{unsafe{
    let h=CreateMutexW(ptr::null(),0,wide("Local\\TrackBridge.SingleInstance".as_ref()).as_ptr());
    ensure!(!h.is_null(),"{}",std::io::Error::last_os_error());
    if GetLastError()==ERROR_ALREADY_EXISTS{CloseHandle(h);anyhow::bail!("TrackBridge уже запущен")}
    Ok(Self(h))
}}}
impl Drop for Instance{fn drop(&mut self){unsafe{CloseHandle(self.0);}}}
pub fn atomic(path:&Path,bytes:&[u8])->Result<()>{
    use std::io::Write;
    let tmp=path.with_added_extension(format!("{:032x}.tmp",rand::random::<u128>()));
    let mut f=fs::File::create_new(&tmp)?; f.write_all(bytes)?; f.sync_all()?;drop(f);
    ensure!(unsafe{MoveFileExW(wide(tmp.as_os_str()).as_ptr(),wide(path.as_os_str()).as_ptr(),MOVEFILE_REPLACE_EXISTING|MOVEFILE_WRITE_THROUGH)}!=0,"{}",std::io::Error::last_os_error());Ok(())
}
fn dpapi(data:&[u8],seal:bool)->Result<Vec<u8>>{unsafe{
    let input=CRYPT_INTEGER_BLOB{cbData:data.len().try_into()?,pbData:data.as_ptr() as _};
    let mut out:CRYPT_INTEGER_BLOB=std::mem::zeroed();
    let ok=if seal{CryptProtectData(&input,ptr::null(),ptr::null(),ptr::null_mut(),ptr::null(),CRYPTPROTECT_UI_FORBIDDEN,&mut out)}else{CryptUnprotectData(&input,ptr::null_mut(),ptr::null(),ptr::null_mut(),ptr::null(),CRYPTPROTECT_UI_FORBIDDEN,&mut out)};
    ensure!(ok!=0,"DPAPI: {}",std::io::Error::last_os_error());
    let bytes=std::slice::from_raw_parts(out.pbData,out.cbData as usize).to_vec();LocalFree(out.pbData as _);Ok(bytes)
}}
pub fn s<'a>(v:&'a Value,key:&str)->&'a str{v[key].as_str().unwrap_or("")}
pub fn n(v:&Value,key:&str)->i64{v[key].as_i64().unwrap_or_default()}
pub fn b(v:&Value,key:&str)->bool{v[key].as_bool().unwrap_or(false)}
pub fn now()->i64{std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() as i64}
pub fn token()->Value{json!({"access_token":"","refresh_token":"","expires":0})}
pub fn defaults()->Value{json!({"language":if unsafe{GetUserDefaultUILanguage()} & 0x3ff == 0x19 {"ru"}else{"en"},"orders":[],"recentReceipts":[],"autoHidePlayed":false,"requestSoundEnabled":true,"requestSoundVolume":20,"requestSoundName":"","chatAllowed":false,"chatReadAllowed":false,"skipAutomatic":true,"skipManualVotes":5,"votingEnabled":true,"blockExplicit":false,"viewerTrackLimit":3,"viewerWindowMinutes":30,"quotaArchive":[],"trackChoiceMinutes":15,"maxTrackMinutes":10,"blockedTracks":"","blockedArtists":"","twitchId":"","spotifyId":"","broadcaster":"","reward":"","twitch":token(),"spotify":token()})}
pub fn needs_review(o:&Value)->bool{matches!(s(o,"status"),"manual"|"uncertain"|"needs_track")}
pub fn track_choice_deadline(order: &Value, minutes: i64) -> Option<i64> {
    let minutes = minutes.clamp(0, 1440);
    let since = n(order, "choiceSince");
    (order["status"] == "needs_track" && since > 0 && minutes > 0)
        .then(|| since.saturating_add(minutes * 60))
}
pub fn finished(o:&Value)->bool{s(o,"status")=="done" || (s(o,"status")=="canceled" && (!b(o,"notify")||b(o,"notificationAttempted")))}
fn counts_for_quota(order: &Value) -> bool {
    !matches!(s(order, "status"), "reject" | "canceled") && !b(order, "refundPending")
}

fn same_viewer(order: &Value, user_id: &str, name: &str) -> bool {
    let stored = s(order, "userId");
    if stored.is_empty() || user_id.is_empty() {
        s(order, "user").eq_ignore_ascii_case(name)
    } else {
        stored == user_id
    }
}

struct ViewerQuota {
    limit: usize,
    minutes: i64,
    wait_seconds: i64,
    active_full: bool,
}

impl ViewerQuota {
    fn blocked(&self) -> bool { self.wait_seconds > 0 || self.active_full }

    fn message(&self, lang: &str) -> String {
        let en = lang == "en";
        let mut text = if en {
            format!("Your limit is {} tracks per {} min.", self.limit, self.minutes)
        } else {
            format!("Ваш лимит заказов: {} за {} мин.", self.limit, self.minutes)
        };
        if self.wait_seconds > 0 {
            let wait = (self.wait_seconds + 59) / 60;
            text.push_str(&if en { format!(" The next slot opens in {wait} min.") }
                else { format!(" Следующий заказ по этому лимиту доступен через {wait} мин.") });
        }
        if self.active_full {
            text.push_str(if en { " You need to wait for one of your 3 active requests to finish." }
                else { " Дождитесь завершения одного из ваших 3 активных заказов." });
        } else if self.wait_seconds == 0 {
            text.push_str(if en { " You can request a track now." } else { " Уже можно заказать трек." });
        }
        text
    }
}

pub struct Profile{pub dir:PathBuf,pub v:Value}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viewer_quota_survives_rename_restart_and_history_cleanup() -> Result<()> {
        let dir=std::env::temp_dir().join(format!("tb-quota-{:x}",rand::random::<u128>()));
        let mut p=Profile::open(dir.clone())?;
        p.v["reward"]=json!("music");
        p.v["broadcaster"]=json!("owner");
        let event=|id:&str,viewer:&str,name:&str| json!({
            "id":id,"reward":{"id":"music"},"user_id":viewer,"user_name":name
        });
        for id in ["one","two","three"] {
            assert!(p.accept(&event(id,"100","first"))?);
            let o=p.v["orders"].as_array_mut().unwrap().last_mut().unwrap();
            assert_eq!(o["status"],"pending");
            o["status"]=json!("done");
        }
        p.v["orders"].as_array_mut().unwrap().extend((0..500).map(|id|json!({"id":id,"status":"done"})));
        p.save()?;
        let mut p=Profile::open(dir.clone())?;
        assert!(p.accept(&event("four","100","renamed"))?);
        assert_eq!(p.v["orders"].as_array().unwrap().last().unwrap()["status"],"reject");
        let rejected = p.v["orders"].as_array().unwrap().last().unwrap().clone();
        let start = now();
        p.v["language"]=json!("en");
        for (i, row) in p.v["quotaArchive"].as_array_mut().unwrap().iter_mut().enumerate() {
            row["created"] = json!(start - 600 + 60 * i as i64);
        }
        assert!(p.refund_reason(&rejected,start+1199)?.contains("in 1 min."));
        assert!(p.refund_reason(&rejected,start+1200)?.contains("request a track now"));
        p.v["viewerTrackLimit"]=json!(2);
        p.v["language"]=json!("ru");
        assert!(p.refund_reason(&rejected,start)?.contains("через 21 мин."));
        p.v["viewerTrackLimit"]=json!(3);
        assert!(p.accept(&event("another","200","renamed"))?);
        assert_eq!(p.v["orders"].as_array().unwrap().last().unwrap()["status"],"pending");
        p.v["quotaArchive"][0]["created"]=json!(now()-30*60);
        assert!(p.accept(&event("expired","100","renamed"))?);
        assert_eq!(p.v["orders"].as_array().unwrap().last().unwrap()["status"],"pending");
        p.v["viewerTrackLimit"]=json!(4);
        assert!(p.accept(&event("raised","100","renamed"))?);
        assert_eq!(p.v["orders"].as_array().unwrap().last().unwrap()["status"],"pending");
        std::fs::remove_file(dir.join("settings.bin"))?;
        std::fs::remove_dir(dir)?;
        Ok(())
    }
}
impl Profile{
    pub fn open(dir:PathBuf)->Result<Self>{
        fs::create_dir_all(&dir)?;let path=dir.join("settings.bin");let mut v=defaults();
        if path.try_exists()?{
            let old:Value=serde_json::from_slice(&dpapi(&fs::read(&path)?,false)?).context("settings.bin")?;
            for(k,val) in old.as_object().context("settings.bin: expected object")?{v[k]=val.clone();}
        }
        if v["recentReceipts"].is_null(){v["recentReceipts"]=json!([])}
        ensure!(v["orders"].is_array()&&v["recentReceipts"].is_array(),"Invalid order history");
        let mut receipts=v["recentReceipts"].as_array().unwrap().clone();
        let opened_at=now();
        for o in v["orders"].as_array_mut().unwrap(){
            let r=json!(format!("{}:{}",s(o,"broadcaster"),s(o,"id")));if !receipts.contains(&r){receipts.push(r)}
            if o["status"]=="needs_track" && n(o,"choiceSince")<=0 {o["choiceSince"]=json!(opened_at);}
            if s(o,"status")=="sending"{o["status"]=json!("uncertain");o["note"]=json!("Проверьте очередь Spotify / Check Spotify queue")}
        }
        v["recentReceipts"]=json!(receipts);let mut p=Self{dir,v};p.save()?;Ok(p)
    }
    pub fn save(&mut self)->Result<()>{self.write_profile().map_err(|e|SaveError(e).into())}
    fn write_profile(&mut self)->Result<()>{
        let cutoff = now() - 1440 * 60;
        let mut removed = Vec::new();
        let orders=self.v["orders"].as_array_mut().context("orders")?;
        let mut trim=orders.iter().filter(|x|finished(x)).count().saturating_sub(500);
        orders.retain(|o| {
            if trim == 0 || !finished(o) { return true }
            trim -= 1;
            if o["status"] == "done" && n(o,"created") > cutoff {
                removed.push(json!({"broadcaster":o["broadcaster"],"user":o["user"],
                    "userId":o["userId"],"created":o["created"]}));
            }
            false
        });
        let archive = self.v["quotaArchive"].as_array_mut().context("Invalid quota history")?;
        archive.retain(|o| n(o,"created") > cutoff);
        archive.extend(removed);
        let r=self.v["recentReceipts"].as_array_mut().context("receipts")?;r.drain(..r.len().saturating_sub(5000));
        atomic(&self.dir.join("settings.bin"),&dpapi(&serde_json::to_vec(&self.v)?,true)?)
    }
    fn viewer_quota(&self, channel: &str, user_id: &str, user: &str, at: i64) -> Result<ViewerQuota> {
        let minutes = n(&self.v, "viewerWindowMinutes").clamp(1, 1440);
        let limit = n(&self.v, "viewerTrackLimit").clamp(1, 100) as usize;
        let orders = self.v["orders"].as_array().context("Invalid orders")?;
        let archive = self.v["quotaArchive"].as_array().context("Invalid quota history")?;
        let matches = |o: &Value| s(o,"broadcaster") == channel && same_viewer(o,user_id,user) && counts_for_quota(o);
        let mut timestamps: Vec<_> = orders.iter().chain(archive).filter(|o| matches(o))
            .map(|o| n(o,"created")).filter(|&t| t > at - minutes * 60).collect();
        let wait_seconds = if timestamps.len() >= limit {
            let index = timestamps.len() - limit;
            *timestamps.select_nth_unstable(index).1 + minutes * 60 - at
        } else { 0 };
        let active_full = orders.iter().filter(|o| matches(o) && !finished(o)).nth(2).is_some();
        Ok(ViewerQuota { limit, minutes, wait_seconds, active_full })
    }

    pub fn refund_reason(&self, order: &Value, at: i64) -> Result<String> {
        let lang = s(&self.v,"language");
        if b(order,"viewerQuotaRejected") {
            let quota = self.viewer_quota(s(order,"broadcaster"), s(order,"userId"), s(order,"user"), at)?;
            return Ok(quota.message(lang))
        }
        Ok(crate::language::message(lang, s(order,"note")).into_owned())
    }

    pub fn accept(&mut self,event:&Value)->Result<bool>{
        let id=s(event,"id");let reward=s(&event["reward"],"id");let channel=s(&self.v,"broadcaster");
        if id.is_empty()||reward!=s(&self.v,"reward")||!matches!(s(event,"status").to_lowercase().as_str(),""|"unfulfilled"){return Ok(false)}
        let receipt=json!(format!("{channel}:{id}"));let orders=self.v["orders"].as_array().unwrap();
        if self.v["recentReceipts"].as_array().unwrap().contains(&receipt)||orders.iter().any(|o|s(o,"id")==id&&s(o,"broadcaster")==channel)||orders.iter().filter(|o|!finished(o)).count()>=100{return Ok(false)}
        let user=s(event,"user_name");
        let user_id=s(event,"user_id");
        let at=now();
        let quota = self.viewer_quota(channel, user_id, user, at)?;
        let reject = quota.blocked();
        let note = if reject {
            format!("{} / {}", quota.message("ru"), quota.message("en"))
        } else { String::new() };
        let o=json!({"id":id,"reward":reward,"broadcaster":channel,"user":user,"userId":user_id,"input":s(event,"user_input"),"created":at,"time":crate::timestamp(),"status":if reject{"reject"}else{"pending"},"notify":reject,"viewerQuotaRejected":reject,"note":note});
        self.v["orders"].as_array_mut().unwrap().push(o);self.v["recentReceipts"].as_array_mut().unwrap().push(receipt);self.save()?;Ok(true)
    }
    /// Start counting when the resolver first asks the streamer for a track.
    /// Older profiles get a full interval on their first launch after the upgrade.
    pub fn expire_track_choices(&mut self, at:i64)->Result<bool>{
        let minutes=n(&self.v,"trackChoiceMinutes").clamp(0,1440);
        if minutes==0{return Ok(false)}
        let mut changed=false;
        for order in self.v["orders"].as_array_mut().unwrap(){
            if !track_choice_deadline(order, minutes).is_some_and(|deadline| at >= deadline) { continue }
            order["status"]=json!("reject");
            order["choiceTimedOut"]=json!(true);
            order["notify"]=json!(true);
            order["note"]=json!("Время выбора трека истекло / Track selection timed out");
            order["nextAttempt"]=json!(0);
            order["attempts"]=json!(0);
            changed=true;
        }
        if changed{self.save()?}
        Ok(changed)
    }
    pub fn due_choice_refund(&self, at:i64)->Option<usize>{
        if s(&self.v,"broadcaster").is_empty() || s(&self.v["twitch"],"access_token").is_empty(){return None}
        self.v["orders"].as_array()?.iter().position(|o|
            b(o,"choiceTimedOut") && o["broadcaster"]==self.v["broadcaster"] && n(o,"nextAttempt")<=at &&
            (o["status"]=="reject" || (o["status"]=="canceled" && !finished(o))))
    }
    pub fn next(&self,t:i64)->Option<usize>{
        let chan=|o:&Value|!s(&self.v,"broadcaster").is_empty() && s(o,"broadcaster")==s(&self.v,"broadcaster");
        let orders=self.v["orders"].as_array()?;
        orders.iter().position(|o|chan(o)&&n(o,"nextAttempt")<=t&&(matches!(s(o,"status"),"queued"|"reject")||(s(o,"status")=="canceled"&&!finished(o)))).or_else(||orders.iter().position(|o|chan(o)&&s(o,"status")=="pending").filter(|&i|n(&orders[i],"nextAttempt")<=t&&n(&orders[i],"spotifyRetryAt")<=t))
    }
}
