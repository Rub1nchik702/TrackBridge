use crate::{api::*,events::{self,Event},orders,player::Player,profile::*,sound};
use anyhow::{Result,Context,ensure,bail};use serde_json::{Value,json};use std::{path::PathBuf,sync::{Arc,atomic::{AtomicBool,Ordering},mpsc::{self,Receiver,Sender}},time::Duration};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CheckState {
    #[default]
    Unchecked,
    Ready,
    Reconnect,
    AccessDenied,
    Retry,
    Storage,
    MissingPermissions,
    RewardUnavailable,
    RewardSettings,
    NoActivePlayer,
    RestrictedPlayer,
}
impl CheckState {
    fn from_error(error: &anyhow::Error) -> Self {
        if error.is::<SaveError>() {
            return Self::Storage;
        }
        match status(error) {
            401 => Self::Reconnect,
            403 => Self::AccessDenied,
            _ => Self::Retry,
        }
    }
}

#[derive(Clone, Default)]
pub struct Readiness {
    pub twitch: CheckState,
    pub spotify: CheckState,
    pub reward: CheckState,
    pub player: CheckState,
}

#[derive(Clone, Default)]
pub struct View{pub prefs:Value,pub orders:Vec<Value>,pub rewards:Vec<Value>,pub running:bool,pub connected:bool,pub device:String,pub player:Value,pub viewers:Option<i64>,pub votes:usize,pub threshold:i64,pub account:[bool;2],pub readiness:Readiness,pub demo:bool}

#[derive(Clone, Copy)]
pub enum DemoScenario { Success, Refund, ChooseTrack }

const DEMO_CHANNEL: &str = "trackbridge-demo";
const DEMO_CHOICE_MINUTES: i64 = 15;
pub const DEMO_TRACK_LINK: &str = "https://open.spotify.com/track/0000000000000000000001";

#[derive(Default)]
struct Demo {
    orders: Vec<Value>,
}
impl Demo {
    fn add_order(&mut self, scenario: DemoScenario, en: bool) -> Option<String> {
        let id = format!("demo-{}", self.orders.len() + 1);
        let (user, title, input, status, reason) = match scenario {
            DemoScenario::Success => ("DemoViewer1", if en { "Test track" } else { "Тестовый трек" },
                DEMO_TRACK_LINK, "done", ""),
            DemoScenario::Refund => ("DemoViewer2", "", if en { "Invalid track link" } else { "Некорректная ссылка" },
                "canceled", "Ссылка на трек не распознана / Track link not recognized"),
            DemoScenario::ChooseTrack => ("DemoViewer3", "", "https://youtu.be/00000000001",
                "needs_track", "Найдено несколько совпадений. Укажите трек Spotify. / Several matches found. Choose a Spotify track."),
        };
        let at = now();
        let mut order = json!({
            "id":id, "broadcaster":DEMO_CHANNEL, "user":user, "input":input,
            "trackTitle":title, "status":status, "note":reason, "time":crate::timestamp(), "created":at
        });
        if status == "needs_track" {
            order["choiceSince"] = json!(at);
        }
        self.orders.push(order);
        (status == "needs_track").then_some(id)
    }

    fn expire_choices(&mut self, at: i64) -> bool {
        let mut changed = false;
        for order in &mut self.orders {
            if track_choice_deadline(order, DEMO_CHOICE_MINUTES).is_some_and(|deadline| deadline <= at) {
                order["status"] = json!("canceled");
                order["choiceTimedOut"] = json!(true);
                order["note"] = json!("Время выбора трека истекло / Track selection timed out");
                changed = true;
            }
        }
        changed
    }

    fn review_order(&mut self, id: &str) -> Result<&mut Value> {
        self.expire_choices(now());
        self.orders.iter_mut().find(|order| order["id"] == id && order["status"] == "needs_track")
            .context("Order unavailable")
    }
}

pub enum Cmd{Set(Value),Login(bool,Value,Cancel,Sender<DevicePrompt>),Rewards,Create(String,i64),Check,CheckReward,CheckSetup,Start,Stop,Skip,Reset,Hide(Vec<String>),Clean,Resolve(String,bool),ChooseTrack(String,String),Import(PathBuf),Preview,EnterDemo,ExitDemo,RunDemo(DemoScenario),Quit}
pub enum Reply{State(View),Log(String),SetupFailed(String),OrderActionFailed(String,String),OpenDemoOrder(String),Ready}
pub struct Engine{pub p:Profile,a:Api,tx:Sender<Reply>,pub player:Player,rewards:Vec<Value>,device:String,readiness:Readiness,session:Option<(Cancel,Receiver<Event>)>,connected:bool,next_poll:i64,next_player:i64,next_count:i64,next_validate:i64,last_retry:i64,demo:Option<Demo>}
impl Engine{
    pub fn new(p:Profile,tx:Sender<Reply>)->Self{Self{p,a:Api::new(),tx,player:Player::default(),rewards:vec![],device:String::new(),readiness:Readiness::default(),session:None,connected:false,next_poll:0,next_player:0,next_count:0,next_validate:0,last_retry:0,demo:None}}
    pub(crate) fn load_preview_orders(&mut self) {
        let mut demo = Demo::default();
        for scenario in [DemoScenario::Success, DemoScenario::Refund, DemoScenario::ChooseTrack] {
            demo.add_order(scenario, self.p.v["language"] == "en");
        }
        self.demo = Some(demo);
    }
    pub fn view(&self)->View{
        if let Some(demo) = &self.demo {
            return View {
                prefs: json!({"language":self.p.v["language"], "broadcaster":DEMO_CHANNEL, "trackChoiceMinutes":DEMO_CHOICE_MINUTES}),
                orders: demo.orders.clone(),
                demo: true,
                ..View::default()
            };
        }
        let mut prefs=self.p.v.clone();let orders=prefs["orders"].take().as_array().unwrap().clone();for k in ["orders","recentReceipts","quotaArchive","twitch","spotify"]{prefs.as_object_mut().unwrap().remove(k);}
        View{prefs,orders,rewards:self.rewards.clone(),running:self.session.is_some(),connected:self.connected,device:self.device.clone(),player:self.player.data.clone(),viewers:self.player.viewers,votes:self.player.voters.len(),threshold:self.player.threshold,account:["twitch","spotify"].map(|k|!s(&self.p.v[k],"access_token").is_empty()),readiness:self.readiness.clone(),demo:false}
    }
    fn publish(&self){let _=self.tx.send(Reply::State(self.view()));}
    fn log(&self,v:impl Into<String>){let _=self.tx.send(Reply::Log(v.into()));}
    fn update_player_readiness(&mut self, result: Result<Value>) -> Result<()> {
        self.device.clear();
        let data = match result {
            Ok(data) => data,
            Err(error) => {
                self.readiness.spotify = CheckState::from_error(&error);
                self.readiness.player = CheckState::Unchecked;
                return Err(error);
            }
        };
        self.readiness.spotify = CheckState::Ready;
        let device = &data;
        if !b(device, "is_active") {
            self.readiness.player = CheckState::NoActivePlayer;
        } else if b(device, "is_restricted") {
            self.readiness.player = CheckState::RestrictedPlayer;
        } else {
            self.readiness.player = CheckState::Ready;
            self.device = s(device, "name").into();
        }
        Ok(())
    }
    fn check_player(&mut self) -> Result<()> {
        let result = self.a.call(&mut self.p, true, "GET", "me/player", None)
            .map(|mut data| data["device"].take());
        self.update_player_readiness(result)?;
        ensure!(self.readiness.player == CheckState::Ready,
            "Откройте Spotify, включите музыку на доступном устройстве / Start Spotify playback on an available device");
        Ok(())
    }
    fn validate_twitch(&mut self) -> Result<()> {
        let result = self.a.validate(&mut self.p);
        self.readiness.twitch = match &result {
            Ok(()) if b(&self.p.v, "chatAllowed") && b(&self.p.v, "chatReadAllowed") => CheckState::Ready,
            Ok(()) => CheckState::MissingPermissions,
            Err(error) => CheckState::from_error(error),
        };
        result
    }
    fn check_reward(&mut self) -> Result<()> {
        ensure!(!s(&self.p.v, "reward").is_empty(), "Выберите награду / Select a reward");
        let path = format!(
            "channel_points/custom_rewards?broadcaster_id={}&id={}&only_manageable_rewards=true",
            enc(s(&self.p.v, "broadcaster")), enc(s(&self.p.v, "reward"))
        );
        let result = self.a.call(&mut self.p, false, "GET", &path, None)
            .and_then(|data| Ok(rows(&data)?.first().cloned()));
        let reward = match result {
            Ok(reward) => reward,
            Err(error) => {
                self.readiness.reward = match status(&error) {
                    404 => CheckState::RewardUnavailable,
                    _ => CheckState::from_error(&error),
                };
                return Err(error);
            }
        };
        let Some(reward) = reward else {
            self.readiness.reward = CheckState::RewardUnavailable;
            bail!("Награда недоступна / Reward unavailable");
        };
        if !b(&reward, "is_user_input_required") || b(&reward, "should_redemptions_skip_request_queue") {
            self.readiness.reward = CheckState::RewardSettings;
            bail!("Проверьте параметры награды / Check reward settings");
        }
        self.readiness.reward = CheckState::Ready;
        Ok(())
    }
    fn check_setup(&mut self) -> Result<()> {
        self.readiness.player = CheckState::Unchecked;
        self.device.clear();
        ensure!(!s(&self.p.v["twitch"], "access_token").is_empty(), "Подключите аккаунт / Connect Twitch");
        ensure!(!s(&self.p.v["spotify"], "access_token").is_empty(), "Подключите аккаунт / Connect Spotify");
        ensure!(!s(&self.p.v, "reward").is_empty(), "Выберите награду / Select a reward");
        self.readiness.reward = CheckState::Unchecked;
        self.validate_twitch()?;
        ensure!(self.readiness.twitch == CheckState::Ready,
            "Подключите Twitch с правами чата / Reconnect Twitch with chat permissions");
        self.check_reward()?;
        self.check_player()
    }
    fn start(&mut self) -> Result<()> {
        ensure!(self.session.is_none(), "Already running");
        self.check_setup()?;
        self.next_poll = 0;
        self.next_validate = now() + 3500;
        self.socket();
        Ok(())
    }
    fn socket(&mut self){if let Some((old,_))=self.session.take(){old.store(true,Ordering::Relaxed)}let c=Arc::new(AtomicBool::new(false));let(tx,rx)=mpsc::sync_channel(256);self.session=Some((c.clone(),rx));std::thread::spawn(move||events::run(c,tx));}
    fn error(&mut self,e:&anyhow::Error){self.log(describe(e));if e.is::<SaveError>(){self.stop();self.log("Приём остановлен. Проверьте доступ к папке data и перезапустите приложение. / Storage error. Check the data directory and restart.");}}
    fn stop(&mut self){if let Some((c,_))=self.session.take(){c.store(true,Ordering::Relaxed)}self.player=Player::default();self.connected=false;}
    fn command(&mut self,cmd:Cmd)->Result<()>{
        if let Some(demo) = &mut self.demo {
            match cmd {
                Cmd::ExitDemo => self.demo = None,
                Cmd::RunDemo(scenario) => {
                    if let Some(id) = demo.add_order(scenario, self.p.v["language"] == "en") {
                        let _ = self.tx.send(Reply::OpenDemoOrder(id));
                    }
                }
                Cmd::ChooseTrack(id, raw) => {
                    let uri = orders::spotify(&raw).context("Введите полную ссылку на трек Spotify / Enter a full Spotify track link")?;
                    let order = demo.review_order(&id)?;
                    order["uri"] = json!(uri);
                    order["trackTitle"] = json!(if self.p.v["language"] == "en" { "Selected demo track" } else { "Выбранный демо-трек" });
                    order["status"] = json!("done");
                    order["note"] = json!("");
                }
                Cmd::Resolve(id, false) => {
                    let order = demo.review_order(&id)?;
                    order["status"] = json!("canceled");
                    order["note"] = json!("Баллы возвращены вручную в демо-режиме / Points refunded manually in demo mode");
                }
                Cmd::Hide(ids) => {
                    for order in &mut demo.orders {
                        if ids.iter().any(|id| order["id"] == *id) {
                            order["historyHidden"] = json!(true);
                        }
                    }
                }
                _ => bail!("Действие недоступно в демо-режиме / Action unavailable in demo mode"),
            }
            return Ok(());
        }
        match cmd{
        Cmd::EnterDemo => {
            ensure!(self.session.is_none(), "Остановите приём заказов перед входом в демо-режим / Stop reception before entering demo mode");
            self.demo = Some(Demo::default());
        },
        Cmd::ExitDemo | Cmd::RunDemo(_) => bail!("Сначала откройте демо-режим / Enter demo mode first"),
        Cmd::Set(v)=>{for(k,val)in v.as_object().context("Invalid settings")?{
            match k.as_str(){"twitchId"|"spotifyId" if self.p.v[k]!=*val=>{ensure!(self.session.is_none(),"Stop first");let provider=k.trim_end_matches("Id");self.p.v[provider]=token();if provider=="twitch"{self.p.v["reward"]=json!("");self.p.v["broadcaster"]=json!("");self.rewards.clear();self.readiness.twitch=CheckState::Unchecked;self.readiness.reward=CheckState::Unchecked;}else{self.device.clear();self.readiness.spotify=CheckState::Unchecked;self.readiness.player=CheckState::Unchecked;}},
                "reward" if self.p.v[k]!=*val=>{ensure!(self.session.is_none(),"Stop first");self.readiness.reward=CheckState::Unchecked;},
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
        Cmd::Login(sp,ids,c,prompt)=>{ensure!(self.session.is_none(),"Stop first");self.command(Cmd::Set(ids))?;if sp{self.device.clear();self.readiness.spotify=CheckState::Unchecked;self.readiness.player=CheckState::Unchecked;}else{self.readiness.twitch=CheckState::Unchecked;self.readiness.reward=CheckState::Unchecked;}let result=self.a.connect(&mut self.p,sp,&c,&prompt);drop(prompt);result?;if !sp{self.rewards=self.a.rewards(&mut self.p)?}},
        Cmd::Rewards=>self.rewards=self.a.rewards(&mut self.p)?,
        Cmd::Create(title,cost)=>{ensure!(!title.trim().is_empty()&&cost>0,"Invalid reward");self.a.validate(&mut self.p)?;let path=format!("channel_points/custom_rewards?broadcaster_id={}",enc(s(&self.p.v,"broadcaster")));let body=json!({"title":title,"cost":cost,"prompt":if self.p.v["language"]=="en"{"Spotify / YouTube track link"}else{"Ссылка на трек Spotify / YouTube"},"is_user_input_required":true,"should_redemptions_skip_request_queue":false,"is_global_cooldown_enabled":true,"global_cooldown_seconds":15});
            let r=self.a.call(&mut self.p,false,"POST",&path,Some(body))?;self.p.v["reward"]=rows(&r)?.first().context("Missing reward")?["id"].clone();self.readiness.reward=CheckState::Unchecked;self.p.save()?;self.rewards=self.a.rewards(&mut self.p)?},
        Cmd::Check=>self.check_player()?,
        Cmd::CheckReward=>self.check_reward()?,
        Cmd::CheckSetup=>{ensure!(self.session.is_none(),"Stop first");self.check_setup()?},
        Cmd::Start=>self.start()?,
        Cmd::Stop=>self.stop(),Cmd::Reset=>self.player.voters.clear(),Cmd::Skip=>self.player.skip(&self.a,&mut self.p)?,Cmd::Clean=>self.player.clean(&mut self.p)?,
        Cmd::Hide(ids)=>{for o in self.p.v["orders"].as_array_mut().unwrap(){if ids.iter().any(|id|id==s(o,"id")){o["historyHidden"]=json!(true)}}self.p.save()?},
        Cmd::ChooseTrack(id,raw)=>{
            self.p.expire_track_choices(now())?;
            let uri=orders::spotify(&raw).context("Введите полную ссылку на трек Spotify / Enter a full Spotify track link")?;
            let broadcaster=self.p.v["broadcaster"].clone();
            let o=self.p.v["orders"].as_array_mut().unwrap().iter_mut().find(|o|o["id"]==id && o["broadcaster"]==broadcaster && o["status"]=="needs_track").context("Order unavailable")?;
            o["uri"]=json!(uri);o["status"]=json!("pending");o["note"]=json!("");o["attempts"]=json!(0);o["nextAttempt"]=json!(0);self.p.save()?
        },
        Cmd::Resolve(id,added)=>{self.p.expire_track_choices(now())?;let i=self.p.v["orders"].as_array().unwrap().iter().position(|o|s(o,"id")==id&&s(o,"broadcaster")==s(&self.p.v,"broadcaster")&&(!added||!b(o,"choiceTimedOut"))&&(matches!(s(o,"status"),"uncertain"|"manual")||(!added&&o["status"]=="needs_track"))).context("Order unavailable")?;let o=self.p.v["orders"][i].clone();self.a.mark(&mut self.p,&o,added)?;self.p.v["orders"][i]["refundPending"]=json!(!added);self.p.v["orders"][i]["status"]=json!(if added{"done"}else{"canceled"});self.p.save()?},
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
        if let Some(demo) = &mut self.demo {
            return Ok(demo.expire_choices(at));
        }
        let mut changed=self.p.expire_track_choices(at)?;
        if let Some(i)=self.p.due_choice_refund(at){self.process_due(i)?;changed=true}
        if self.session.is_none(){return Ok(changed)}
        for _ in 0..8{let Some(e)=self.session.as_ref().and_then(|(_,rx)|rx.try_recv().ok())else{break};self.event(e)?;changed=true}
        let t=now();if t>=self.next_validate{self.next_validate=t+3500;self.validate_twitch()?}
        if t>=self.next_poll{self.next_poll=t+10;if let Err(e)=orders::recover(&self.a,&mut self.p){if e.is::<SaveError>(){return Err(e)}self.log(describe(&e))}changed=true}
        if let Some(i)=self.p.next(t){self.process_due(i)?;changed=true}
        if t>=self.next_player{
            self.next_player=t+3;
            let result=self.player.refresh(&self.a,&mut self.p).map(|()|self.player.data["device"].clone());
            if let Err(error)=self.update_player_readiness(result){self.log(describe(&error))}
            changed=true;
        }
        if t>=self.next_count{self.next_count=t+30;if self.player.count(&self.a,&mut self.p).is_err(){self.player.viewers=None}changed=true}Ok(changed)
    }
    pub fn run(mut self,rx:Receiver<Cmd>,exit:Cancel,preview:bool){
        if !preview&&!s(&self.p.v["twitch"],"access_token").is_empty(){if let Err(e)=self.command(Cmd::Rewards){self.log(describe(&e))}self.publish()}
        while !exit.load(Ordering::Relaxed){match rx.recv_timeout(Duration::from_millis(150)){
            Ok(Cmd::Quit)|Err(mpsc::RecvTimeoutError::Disconnected)=>break,
            Ok(command)=>{
                let order_id = match &command {
                    Cmd::ChooseTrack(id, _) | Cmd::Resolve(id, _) => Some(id.clone()),
                    _ => None,
                };
                let setup_action = matches!(&command,
                    Cmd::Login(..) | Cmd::Rewards | Cmd::Create(..) | Cmd::CheckReward | Cmd::CheckSetup | Cmd::Set(_));
                if let Err(error) = self.command(command) {
                    if let Some(id) = order_id {
                        let _ = self.tx.send(Reply::OrderActionFailed(id, describe(&error)));
                    }
                    if setup_action {
                        let _ = self.tx.send(Reply::SetupFailed(describe(&error)));
                    }
                    self.error(&error);
                }
                self.publish();
                let _ = self.tx.send(Reply::Ready);
            },
            _=>()
        }
            if now()>=self.last_retry{match self.tick(){Ok(true)=>self.publish(),Ok(false)=>(),Err(e)=>{self.error(&e);self.publish();self.last_retry=now()+3}}}
        }self.stop()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_runs_all_scenarios_without_touching_the_real_profile() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("TrackBridge-demo-test-{:x}", rand::random::<u128>()));
        let mut profile = Profile::open(dir.clone())?;
        profile.v["language"] = json!("en");
        profile.v["broadcaster"] = json!("real-channel");
        profile.v["orders"] = json!([{"id":"real-order","status":"done"}]);
        profile.save()?;
        let stored = std::fs::read(dir.join("settings.bin"))?;
        let original = profile.v.clone();
        let (replies, received) = mpsc::channel();
        let mut engine = Engine::new(profile, replies);

        engine.command(Cmd::EnterDemo)?;
        let view = engine.view();
        assert!(view.demo && view.orders.is_empty());
        assert_eq!(view.account, [false, false]);
        assert!(engine.command(Cmd::Start).is_err());
        assert!(engine.command(Cmd::Set(json!({"language":"ru"}))).is_err());
        engine.command(Cmd::RunDemo(DemoScenario::Success))?;
        engine.command(Cmd::RunDemo(DemoScenario::Refund))?;
        engine.command(Cmd::RunDemo(DemoScenario::ChooseTrack))?;
        let Reply::OpenDemoOrder(id) = received.try_recv()? else { panic!("Expected a demo review card") };
        let view = engine.view();
        assert_eq!(view.orders[0]["status"], "done");
        assert_eq!(view.orders[1]["status"], "canceled");
        assert_eq!(view.orders[2]["status"], "needs_track");
        assert!(track_choice_deadline(&view.orders[2], DEMO_CHOICE_MINUTES).is_some());
        assert!(engine.command(Cmd::ChooseTrack(id.clone(), "invalid".into())).is_err());
        engine.command(Cmd::ChooseTrack(id, DEMO_TRACK_LINK.into()))?;
        assert_eq!(engine.view().orders[2]["status"], "done");

        engine.command(Cmd::RunDemo(DemoScenario::ChooseTrack))?;
        let Reply::OpenDemoOrder(id) = received.try_recv()? else { panic!("Expected a demo review card") };
        engine.command(Cmd::Resolve(id, false))?;
        assert_eq!(engine.view().orders[3]["status"], "canceled");
        engine.command(Cmd::RunDemo(DemoScenario::ChooseTrack))?;
        let demo = engine.demo.as_mut().unwrap();
        let deadline = track_choice_deadline(&demo.orders[4], DEMO_CHOICE_MINUTES).unwrap();
        assert!(!demo.expire_choices(deadline - 1));
        assert!(demo.expire_choices(deadline));
        assert_eq!(engine.view().orders[4]["status"], "canceled");
        assert!(b(&engine.view().orders[4], "choiceTimedOut"));

        engine.command(Cmd::ExitDemo)?;
        assert!(!engine.view().demo);
        assert_eq!(engine.view().orders[0]["id"], "real-order");
        assert_eq!(engine.p.v, original);
        assert_eq!(std::fs::read(dir.join("settings.bin"))?, stored);
        engine.command(Cmd::EnterDemo)?;
        assert!(engine.view().orders.is_empty());
        std::fs::remove_file(dir.join("settings.bin"))?;
        std::fs::remove_dir(dir)?;
        Ok(())
    }

    #[test]
    fn readiness_distinguishes_spotify_access_from_player_availability() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("TrackBridge-readiness-{:x}", rand::random::<u128>()));
        let mut engine = Engine::new(Profile::open(dir)?, mpsc::channel().0);

        engine.update_player_readiness(Ok(Value::Null))?;
        assert_eq!(engine.readiness.spotify, CheckState::Ready);
        assert_eq!(engine.readiness.player, CheckState::NoActivePlayer);
        engine.update_player_readiness(Ok(json!({"is_active":true,"is_restricted":true})))?;
        assert_eq!(engine.readiness.player, CheckState::RestrictedPlayer);
        engine.update_player_readiness(Ok(json!({"is_active":true,"name":"Desktop"})))?;
        assert_eq!(engine.view().device, "Desktop");
        assert_eq!(engine.view().readiness.player, CheckState::Ready);
        assert!(engine.update_player_readiness(Err(anyhow::anyhow!("Network unavailable"))).is_err());
        assert_eq!(engine.view().readiness.spotify, CheckState::Retry);
        assert_eq!(engine.view().readiness.player, CheckState::Unchecked);
        assert!(engine.view().device.is_empty());
        Ok(())
    }

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
