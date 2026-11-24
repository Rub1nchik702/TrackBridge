use crate::{api::*,events::{self,Event},orders,player::Player,profile::*,sound};
use anyhow::{Result,Context,ensure,bail};use serde_json::{Value,json};use std::{path::PathBuf,sync::{Arc,atomic::{AtomicBool,Ordering},mpsc::{self,Receiver,Sender}},time::Duration};
#[derive(Clone)]
pub struct View{pub prefs:Value,pub orders:Vec<Value>,pub rewards:Vec<Value>,pub running:bool,pub connected:bool,pub device:String,pub player:Value,pub viewers:Option<i64>,pub votes:usize,pub threshold:i64,pub account:[bool;2]}
pub enum Cmd{Set(Value),Login(bool,Cancel,Sender<DevicePrompt>),Rewards,Create(String,i64),Check,Start,Stop,Skip,Reset,Hide(Vec<String>),Clean,Resolve(String,bool),ChooseTrack(String,String),Import(PathBuf),Preview,Quit}
pub enum Reply{State(View),Log(String),Ready}
pub struct Engine{pub p:Profile,a:Api,tx:Sender<Reply>,pub player:Player,rewards:Vec<Value>,device:String,session:Option<(Cancel,Receiver<Event>)>,connected:bool,next_poll:i64,next_player:i64,next_count:i64,next_validate:i64,last_retry:i64}
impl Engine{
    pub fn new(p:Profile,tx:Sender<Reply>)->Self{Self{p,a:Api::new(),tx,player:Player::default(),rewards:vec![],device:String::new(),session:None,connected:false,next_poll:0,next_player:0,next_count:0,next_validate:0,last_retry:0}}
    pub fn view(&self)->View{let mut prefs=self.p.v.clone();let orders=prefs["orders"].take().as_array().unwrap().clone();for k in ["orders","recentReceipts","quotaArchive","twitch","spotify"]{prefs.as_object_mut().unwrap().remove(k);}
        View{prefs,orders,rewards:self.rewards.clone(),running:self.session.is_some(),connected:self.connected,device:self.device.clone(),player:self.player.data.clone(),viewers:self.player.viewers,votes:self.player.voters.len(),threshold:self.player.threshold,account:["twitch","spotify"].map(|k|!s(&self.p.v[k],"access_token").is_empty())}
    }
    fn publish(&self){let _=self.tx.send(Reply::State(self.view()));}
    fn log(&self,v:impl Into<String>){let _=self.tx.send(Reply::Log(v.into()));}
    fn check_player(&mut self)->Result<()>{self.device.clear();let v=self.a.call(&mut self.p,true,"GET","me/player",None)?;let d=&v["device"];ensure!(b(d,"is_active")&&!b(d,"is_restricted"),"Откройте Spotify и включите музыку / Start Spotify playback");self.device=s(d,"name").into();Ok(())}
    fn socket(&mut self){if let Some((old,_))=self.session.take(){old.store(true,Ordering::Relaxed)}let c=Arc::new(AtomicBool::new(false));let(tx,rx)=mpsc::sync_channel(256);self.session=Some((c.clone(),rx));std::thread::spawn(move||events::run(c,tx));}
    fn error(&mut self,e:&anyhow::Error){self.log(describe(e));if e.is::<SaveError>(){self.stop();self.log("Приём остановлен. Проверьте доступ к папке data и перезапустите приложение. / Storage error. Check the data directory and restart.");}}
    fn stop(&mut self){if let Some((c,_))=self.session.take(){c.store(true,Ordering::Relaxed)}self.player=Player::default();self.connected=false;}
    fn command(&mut self,cmd:Cmd)->Result<()>{match cmd{
        Cmd::Set(v)=>{for(k,val)in v.as_object().context("Invalid settings")?{
            match k.as_str(){"twitchId"|"spotifyId" if self.p.v[k]!=*val=>{ensure!(self.session.is_none(),"Stop first");let provider=k.trim_end_matches("Id");self.p.v[provider]=token();if provider=="twitch"{self.p.v["reward"]=json!("");self.p.v["broadcaster"]=json!("");self.rewards.clear()}else{self.device.clear()}},
                "twitchId"|"spotifyId"|"reward"|"maxTrackMinutes"|"blockExplicit"|"blockedTracks"|"blockedArtists"|"skipAutomatic"|"skipManualVotes"|"requestSoundName"=>ensure!(self.session.is_none(),"Stop first"),
                "viewerTrackLimit" | "viewerWindowMinutes" => {
                    ensure!(self.session.is_none(),"Stop first");
                    let max=if k=="viewerTrackLimit"{100}else{1440};
                    ensure!(val.as_i64().is_some_and(|v|(1..=max).contains(&v)),"Invalid viewer limit");
                }
                "trackChoiceMinutes"=>ensure!(val.as_i64().is_some_and(|m|(0..=1440).contains(&m)),"Invalid selection timeout"),
                "autoHidePlayed"|"requestSoundEnabled"|"requestSoundVolume"|"votingEnabled"|"language"=>(),_=>bail!("Unknown setting")}
            self.p.v[k]=val.clone()
        }if !b(&self.p.v,"votingEnabled"){self.player.voters.clear()}self.p.save()?},
        Cmd::Login(sp,c,prompt)=>{ensure!(self.session.is_none(),"Stop first");let result=self.a.connect(&mut self.p,sp,&c,&prompt);drop(prompt);result?;if !sp{self.rewards=self.a.rewards(&mut self.p)?}},
        Cmd::Rewards=>self.rewards=self.a.rewards(&mut self.p)?,
        Cmd::Create(title,cost)=>{ensure!(!title.trim().is_empty()&&cost>0,"Invalid reward");self.a.validate(&mut self.p)?;let path=format!("channel_points/custom_rewards?broadcaster_id={}",enc(s(&self.p.v,"broadcaster")));let body=json!({"title":title,"cost":cost,"prompt":if self.p.v["language"]=="en"{"Spotify / YouTube track link"}else{"Ссылка на трек Spotify / YouTube"},"is_user_input_required":true,"should_redemptions_skip_request_queue":false,"is_global_cooldown_enabled":true,"global_cooldown_seconds":15});
            let r=self.a.call(&mut self.p,false,"POST",&path,Some(body))?;self.p.v["reward"]=rows(&r)?.first().context("Missing reward")?["id"].clone();self.p.save()?;self.rewards=self.a.rewards(&mut self.p)?},
        Cmd::Check=>self.check_player()?,
        Cmd::Start=>{ensure!(self.session.is_none(),"Already running");self.a.validate(&mut self.p)?;ensure!(b(&self.p.v,"chatAllowed")&&b(&self.p.v,"chatReadAllowed"),"Подключите Twitch с правами чата / Reconnect Twitch with chat permissions");ensure!(!s(&self.p.v,"reward").is_empty(),"Выберите награду / Select a reward");let path=format!("channel_points/custom_rewards?broadcaster_id={}&id={}&only_manageable_rewards=true",enc(s(&self.p.v,"broadcaster")),enc(s(&self.p.v,"reward")));let v=self.a.call(&mut self.p,false,"GET",&path,None)?;let r=rows(&v)?.first().context("Reward unavailable")?;ensure!(b(r,"is_user_input_required")&&!b(r,"should_redemptions_skip_request_queue"),"Проверьте параметры награды / Check reward settings");self.check_player()?;self.next_poll=0;self.next_validate=now()+3500;self.socket()},
        Cmd::Stop=>self.stop(),Cmd::Reset=>self.player.voters.clear(),Cmd::Skip=>self.player.skip(&self.a,&mut self.p)?,Cmd::Clean=>self.player.clean(&mut self.p)?,
        Cmd::Hide(ids)=>{for o in self.p.v["orders"].as_array_mut().unwrap(){if ids.iter().any(|id|id==s(o,"id")){o["historyHidden"]=json!(true)}}self.p.save()?},
        Cmd::ChooseTrack(id,raw)=>{
            self.p.expire_track_choices(now())?;
            let uri=orders::spotify(&raw).context("Введите полную ссылку на трек Spotify / Enter a full Spotify track link")?;
            let broadcaster=self.p.v["broadcaster"].clone();
            let o=self.p.v["orders"].as_array_mut().unwrap().iter_mut().find(|o|o["id"]==id && o["broadcaster"]==broadcaster && o["status"]=="needs_track").context("Order unavailable")?;
            o["uri"]=json!(uri);o["status"]=json!("pending");o["note"]=json!("");o["attempts"]=json!(0);o["nextAttempt"]=json!(0);self.p.save()?
        },
        Cmd::Resolve(id,added)=>{let i=self.p.v["orders"].as_array().unwrap().iter().position(|o|s(o,"id")==id&&s(o,"broadcaster")==s(&self.p.v,"broadcaster")&&(!added||!b(o,"choiceTimedOut"))&&(matches!(s(o,"status"),"uncertain"|"manual")||(!added&&o["status"]=="needs_track"))).context("Order unavailable")?;let o=self.p.v["orders"][i].clone();self.a.mark(&mut self.p,&o,added)?;self.p.v["orders"][i]["refundPending"]=json!(!added);self.p.v["orders"][i]["status"]=json!(if added{"done"}else{"canceled"});self.p.save()?},
        Cmd::Import(path)=>sound::import(&mut self.p,&path)?,Cmd::Preview=>sound::play(&self.p)?,Cmd::Quit=>self.stop()
    }Ok(())}
    fn event(&mut self,e:Event)->Result<()>{match e{
        Event::Welcome(id)=>{let broadcaster=self.p.v["broadcaster"].clone();for(topic,condition)in [("channel.channel_points_custom_reward_redemption.add",json!({"broadcaster_user_id":broadcaster,"reward_id":self.p.v["reward"]})),("channel.chat.message",json!({"broadcaster_user_id":broadcaster,"user_id":broadcaster}))]{
            if let Err(e)=self.a.call(&mut self.p,false,"POST","eventsub/subscriptions",Some(json!({"type":topic,"version":"1","condition":condition,"transport":{"method":"websocket","session_id":id}}))){self.connected=false;self.socket();return Err(e)}
        }self.connected=true},Event::Live=>self.connected=true,Event::Lost(msg)=>{self.connected=false;self.log(msg)},Event::Purchase(v)=>{if self.p.accept(&v)?{self.log(format!("Twitch · {}",s(&v,"user_name"))) }},Event::Message(v)=>self.player.command(&self.a,&mut self.p,&v)?
    }Ok(())}
    fn process_due(&mut self,index:usize)->Result<()>{
        match orders::process(&self.a,&mut self.p,index){
            Ok(Some(warning))=>self.log(warning), Ok(None)=>(), Err(e)=>{
            if e.is::<SaveError>(){return Err(e)}
            orders::defer(&mut self.p,index,&e)?;
            self.log(describe(&e));
        }}
        Ok(())
    }
    fn tick(&mut self)->Result<bool>{
        let at=now();
        let mut changed=self.p.expire_track_choices(at)?;
        if let Some(i)=self.p.due_choice_refund(at){self.process_due(i)?;changed=true}
        if self.session.is_none(){return Ok(changed)}
        for _ in 0..8{let Some(e)=self.session.as_ref().and_then(|(_,rx)|rx.try_recv().ok())else{break};self.event(e)?;changed=true}
        let t=now();if t>=self.next_validate{self.next_validate=t+3500;self.a.validate(&mut self.p)?}
        if t>=self.next_poll{self.next_poll=t+10;if let Err(e)=orders::recover(&self.a,&mut self.p){if e.is::<SaveError>(){return Err(e)}self.log(describe(&e))}changed=true}
        if let Some(i)=self.p.next(t){self.process_due(i)?;changed=true}
        if t>=self.next_player{self.next_player=t+3;if let Err(e)=self.player.refresh(&self.a,&mut self.p){self.log(describe(&e))}changed=true}
        if t>=self.next_count{self.next_count=t+30;if self.player.count(&self.a,&mut self.p).is_err(){self.player.viewers=None}changed=true}Ok(changed)
    }
    pub fn run(mut self,rx:Receiver<Cmd>,exit:Cancel,preview:bool){
        if !preview&&!s(&self.p.v["twitch"],"access_token").is_empty(){if let Err(e)=self.command(Cmd::Rewards){self.log(describe(&e))}self.publish()}
        while !exit.load(Ordering::Relaxed){match rx.recv_timeout(Duration::from_millis(150)){Ok(Cmd::Quit)|Err(mpsc::RecvTimeoutError::Disconnected)=>break,Ok(c)=>{if let Err(e)=self.command(c){self.error(&e)}self.publish();let _=self.tx.send(Reply::Ready);},_=>()}
            if now()>=self.last_retry{match self.tick(){Ok(true)=>self.publish(),Ok(false)=>(),Err(e)=>{self.error(&e);self.publish();self.last_retry=now()+3}}}
        }self.stop()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn manual_track_selection_only_requeues_the_unresolved_purchase() -> Result<()> {
        let dir=std::env::temp_dir().join(format!("TrackBridge-choice-{:x}",rand::random::<u128>()));
        let mut p=Profile::open(dir.clone())?;
        p.v["broadcaster"]=json!("channel");
        p.v["reward"]=json!("music");
        p.v["orders"]=json!([{"id":"one","reward":"music","broadcaster":"channel","status":"needs_track","historyHidden":true}]);
        p.save()?;
        let (tx,_rx)=mpsc::channel();
        let mut e=Engine::new(p,tx);
        assert!(needs_review(&e.view().orders[0]));
        assert_eq!(e.p.next(now()),None);
        assert!(e.command(Cmd::Resolve("one".into(),true)).is_err());
        assert!(e.command(Cmd::ChooseTrack("one".into(),"invalid".into())).is_err());
        let link="https://open.spotify.com/track/3qWjq3rURGmsWEfIpAo6IP";
        e.command(Cmd::ChooseTrack("one".into(),link.into()))?;
        assert!(e.command(Cmd::ChooseTrack("one".into(),link.into())).is_err());
        drop(e);
        let p=Profile::open(dir.clone())?;
        assert_eq!(p.next(now()),Some(0));
        assert_eq!(p.v["orders"][0]["uri"],"spotify:track:3qWjq3rURGmsWEfIpAo6IP");
        assert!(!needs_review(&p.v["orders"][0]));
        std::fs::remove_file(dir.join("settings.bin"))?;
        std::fs::remove_dir(dir)?;
        Ok(())
    }

    #[test]
    fn choice_timeout_is_persistent_scoped_and_never_touches_uncertain_sends() -> Result<()> {
        let dir=std::env::temp_dir().join(format!("TrackBridge-clock-test-{:x}",rand::random::<u128>()));
        let mut p=Profile::open(dir.clone())?;
        p.v["broadcaster"]=json!("owner");
        p.v["reward"]=json!("current");
        p.v["twitch"]["access_token"]=json!("fixture");
        p.v["orders"]=json!([
            {"status":"needs_track","broadcaster":"owner","reward":"previous","id":"old","historyHidden":true},
            {"status":"uncertain","choiceSince":1},
            {"status":"sending","choiceSince":1},
            {"status":"manual","choiceSince":1},
            {"status":"queued","choiceSince":1},
            {"status":"pending","choiceSince":1},
            {"status":"needs_track","id":"foreign","broadcaster":"other","choiceSince":1}
        ]);
        p.save()?;drop(p);
        let opened=now();let p=Profile::open(dir.clone())?;
        let since=n(&p.v["orders"][0],"choiceSince");
        assert!(since>=opened);
        let (tx,_rx)=mpsc::channel();let mut engine=Engine::new(p,tx);
        engine.command(Cmd::Set(json!({"trackChoiceMinutes":0})))?;
        assert!(!engine.p.expire_track_choices(since+999999)?);
        engine.command(Cmd::Set(json!({"trackChoiceMinutes":2})))?;
        assert!(engine.command(Cmd::Set(json!({"trackChoiceMinutes":-1}))).is_err());
        drop(engine);
        let mut p=Profile::open(dir.clone())?;
        assert_eq!(n(&p.v,"trackChoiceMinutes"),2);
        assert_eq!(n(&p.v["orders"][0],"choiceSince"),since);
        p.expire_track_choices(since+119)?;
        assert_eq!(p.v["orders"][0]["status"],"needs_track");
        assert_eq!(p.due_choice_refund(since+119),None);
        assert!(p.expire_track_choices(since+120)?);
        assert_eq!(p.due_choice_refund(since+120),Some(0));
        assert!(!p.expire_track_choices(since+121)?);
        for (i,status) in ["reject","uncertain","uncertain","manual","queued","pending"].iter().enumerate(){
            assert_eq!(s(&p.v["orders"][i],"status"),*status);
        }
        let (tx,_rx)=mpsc::channel();let mut engine=Engine::new(p,tx);
        assert!(engine.command(Cmd::ChooseTrack("old".into(),"spotify:track:6nxwpI2PRJSgDwSxOl0n8e".into())).is_err());
        engine.p.v["orders"][0]["status"]=json!("manual");
        assert!(engine.command(Cmd::Resolve("old".into(),true)).is_err());
        drop(engine);
        std::fs::remove_file(dir.join("settings.bin"))?;
        std::fs::remove_dir(dir)?;
        Ok(())
    }
}
