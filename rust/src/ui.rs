use crate::{api::{Cancel,DevicePrompt,open_url},profile::*,runtime::{CheckState,Cmd,DemoScenario,View,Reply,DEMO_TRACK_LINK},update};
use eframe::egui::{self,*};use serde_json::{Value,json};use std::{collections::{HashSet,VecDeque},path::PathBuf,sync::{Arc,atomic::{AtomicBool,Ordering},mpsc::{self,Receiver,Sender}},time::Duration};
use tray_icon::{TrayIcon,TrayIconBuilder,TrayIconEvent,menu::{Menu,MenuItem,MenuEvent}};
const GREEN:Color32=Color32::from_rgb(110,228,162);
#[derive(Default)]
struct AuthWindow{
    rx:Option<Receiver<DevicePrompt>>,
    prompt:Option<DevicePrompt>,
    reveal:bool,
}
impl AuthWindow{
    fn new(rx:Receiver<DevicePrompt>)->Self{Self{rx:Some(rx),..Default::default()}}
    fn poll(&mut self,cancel:Option<&Cancel>,at:i64){
        if cancel.is_none_or(|c|c.load(Ordering::Relaxed)){*self=Self::default();return}
        if let Some(rx)=&self.rx{
            match rx.try_recv(){
                Ok(prompt)=>self.prompt=Some(prompt),
                Err(mpsc::TryRecvError::Disconnected)=>{*self=Self::default();return}
                Err(mpsc::TryRecvError::Empty)=>()
            }
        }
        if self.prompt.as_ref().is_some_and(|prompt|prompt.expires<=at){*self=Self::default();}
    }
    fn show(&mut self,ctx:&Context,cancel:Option<&Cancel>,en:bool){
        self.poll(cancel,now());
        let Some(prompt)=self.prompt.as_ref() else{return};
        let title=if en{"Connect Twitch"}else{"Подключение Twitch"};
        let close=ctx.show_viewport_immediate(ViewportId::from_hash_of("twitch-login"),ViewportBuilder::default().with_title(title).with_inner_size([430.,210.]).with_resizable(false),|ctx,_|{
            let mut close=ctx.input(|i|i.viewport().close_requested());
            CentralPanel::default().show(ctx,|ui|{
                ui.heading(title);
                ui.label(if en{"Enter this code on the Twitch page opened in your browser."}else{"Введите код на странице Twitch, открытой в браузере."});
                ui.monospace(if self.reveal{prompt.code.as_str()}else{"•••• ••••"});
                ui.checkbox(&mut self.reveal,if en{"Show code"}else{"Показать код"});
                if ui.button(if en{"Cancel connection"}else{"Отменить подключение"}).clicked(){close=true}
            });
            close
        });
        if close{if let Some(c)=cancel{c.store(true,Ordering::Relaxed)}*self=Self::default();}
    }
}
struct ReviewCard {
    id: String,
    track: Option<String>,
    error: Option<String>,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum OrderStatus {
    Waiting,
    Sending,
    Confirming,
    InQueue,
    Played,
    Refunding,
    Refunded,
    NeedsTrack,
    NeedsReview,
}
impl OrderStatus {
    const ALL: [Self; 9] = [
        Self::Waiting, Self::Sending, Self::Confirming, Self::InQueue, Self::Played,
        Self::Refunding, Self::Refunded, Self::NeedsTrack, Self::NeedsReview,
    ];

    fn from_order(order: &Value) -> Self {
        match s(order, "status") {
            "pending" => Self::Waiting,
            "sending" => Self::Sending,
            "queued" => Self::Confirming,
            "done" if b(order, "played") => Self::Played,
            "done" => Self::InQueue,
            "reject" => Self::Refunding,
            "canceled" => Self::Refunded,
            "needs_track" => Self::NeedsTrack,
            _ => Self::NeedsReview,
        }
    }

    fn label(self, en: bool) -> &'static str {
        let (ru, english) = match self {
            Self::Waiting => ("Ожидание", "Pending"),
            Self::Sending => ("Отправка", "Sending"),
            Self::Confirming => ("Подтверждение", "Confirming"),
            Self::InQueue => ("В очереди Spotify", "In Spotify queue"),
            Self::Played => ("Сыгран", "Played"),
            Self::Refunding => ("Возврат", "Refunding"),
            Self::Refunded => ("Баллы возвращены", "Refunded"),
            Self::NeedsTrack => ("Нужен выбор трека", "Track selection needed"),
            Self::NeedsReview => ("Требует решения", "Needs review"),
        };
        if en { english } else { ru }
    }
}

fn order_track(order: &Value) -> &str {
    let title = s(order, "trackTitle");
    if title.is_empty() { s(order, "input") } else { title }
}

fn history_matches(order: &Value, query: &str, status: Option<OrderStatus>) -> bool {
    status.is_none_or(|status| status == OrderStatus::from_order(order))
        && (query.is_empty() || [s(order, "user"), order_track(order)]
            .iter().any(|text| text.to_lowercase().contains(query)))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SetupStep { Twitch, Spotify, Reward, Player }

impl SetupStep {
    const ALL: [Self; 4] = [Self::Twitch, Self::Spotify, Self::Reward, Self::Player];

    fn complete(self, view: &View) -> bool {
        use CheckState::*;
        match self {
            Self::Twitch => view.account[0] && b(&view.prefs, "chatAllowed") && b(&view.prefs, "chatReadAllowed")
                && matches!(view.readiness.twitch, Unchecked | Ready),
            Self::Spotify => view.account[1] && !matches!(view.readiness.spotify, Reconnect | AccessDenied),
            Self::Reward => !s(&view.prefs, "reward").is_empty() && view.readiness.reward == Ready,
            Self::Player => Self::ALL[..3].iter().all(|step| step.complete(view))
                && view.readiness.twitch == Ready && view.readiness.player == Ready,
        }
    }

    fn first_incomplete(view: &View) -> Self {
        Self::ALL.into_iter().find(|step| !step.complete(view)).unwrap_or(Self::Player)
    }
}

pub struct Desktop{v:View,tx:Sender<Cmd>,rx:Receiver<Reply>,exit:Cancel,join:Option<std::thread::JoinHandle<()>>,pending:usize,login:Option<Cancel>,auth_window:AuthWindow,page:usize,lang:String,ids:[String;2],rules:[String;2],minutes:i64,choice_minutes:i64,viewer_limit:i64,viewer_window:i64,explicit:bool,title:String,cost:i64,selected:HashSet<String>,review:bool,history_query:String,history_status:Option<OrderStatus>,review_card:Option<ReviewCard>,setup_step:Option<SetupStep>,setup_error:Option<String>,log:VecDeque<String>,scroll:bool,root:PathBuf,tray:Option<(TrayIcon,MenuItem,MenuItem)>,update_rx:Option<Receiver<anyhow::Result<Option<PathBuf>>>>,stage:Option<PathBuf>,next_update:i64,preview:bool,shot:Option<PathBuf>,frames:u32,art_url:String,art:Option<TextureHandle>,art_rx:Option<Receiver<(String,Option<ColorImage>)>>,diag:Option<PathBuf>}
impl Desktop{
    pub fn new(v:View,tx:Sender<Cmd>,rx:Receiver<Reply>,exit:Cancel,join:std::thread::JoinHandle<()>,root:PathBuf,preview:bool,shot:Option<PathBuf>,settings:bool,diag:Option<PathBuf>)->Self{
        let lang=s(&v.prefs,"language").to_string();let en=lang=="en";let tray=(||{let menu=Menu::new();let open=MenuItem::new(if en{"Open"}else{"Открыть"},true,None);let quit=MenuItem::new(if en{"Exit"}else{"Выйти"},true,None);menu.append(&open).ok()?;menu.append(&quit).ok()?;let img=crate::icon();let icon=tray_icon::Icon::from_rgba(img.rgba,img.width,img.height).ok()?;Some((TrayIconBuilder::new().with_menu(Box::new(menu)).with_icon(icon).with_tooltip("TrackBridge").build().ok()?,open,quit))})();
        Self{ids:[s(&v.prefs,"twitchId").into(),s(&v.prefs,"spotifyId").into()],rules:[s(&v.prefs,"blockedTracks").into(),s(&v.prefs,"blockedArtists").into()],minutes:n(&v.prefs,"maxTrackMinutes"),viewer_limit:n(&v.prefs,"viewerTrackLimit"),viewer_window:n(&v.prefs,"viewerWindowMinutes"),choice_minutes:n(&v.prefs,"trackChoiceMinutes"),explicit:b(&v.prefs,"blockExplicit"),v,tx,rx,exit,join:Some(join),pending:0,login:None,auth_window:AuthWindow::default(),page:if settings{4}else{0},lang,title:if en{"Request a song"}else{"Заказать трек"}.into(),cost:500,selected:HashSet::new(),review:false,history_query:String::new(),history_status:None,review_card:None,setup_step:None,setup_error:None,log:VecDeque::new(),scroll:true,root,tray,update_rx:None,stage:None,next_update:0,preview,shot,frames:0,art_url:String::new(),art:None,art_rx:None,diag}
    }
    fn t(&self,ru:&'static str,en:&'static str)->&'static str{if self.lang=="en"{en}else{ru}}
    pub(crate) fn configure_preview(&mut self, page: &str) {
        match page {
            "wizard" => self.setup_step = Some(SetupStep::Spotify),
            "history" => self.page = 1,
            "review" => {
                self.page = 1;
                self.review = true;
                if let Some(order) = self.v.orders.iter().find(|order| needs_review(order)) {
                    self.review_card = Some(ReviewCard { id:s(order, "id").to_owned(), track:None, error:None });
                }
            }
            _ => (),
        }
    }
    fn send(&mut self,c:Cmd){if self.tx.send(c).is_ok(){self.pending+=1}}
    fn set(&mut self,k:&str,v:Value){self.v.prefs[k]=v.clone();self.send(Cmd::Set(json!({(k):v})))}
    fn record(&mut self,text:String){let text=crate::language::message(&self.lang,&text);let entry=format!("{}  {text}",crate::timestamp());if let Some(path)=&self.diag{use std::io::Write;if let Ok(mut f)=std::fs::OpenOptions::new().create(true).append(true).open(path){let _=writeln!(f,"{entry}");}}self.log.push_back(entry);if self.log.len()>2000{self.log.pop_front();}}
    fn ids(&mut self){self.send(Cmd::Set(json!({"twitchId":self.ids[0].trim(),"spotifyId":self.ids[1].trim()})))}
    fn check_updates(&mut self){if self.update_rx.is_some()||self.preview||self.v.demo{return}let(tx,rx)=mpsc::channel();let d=self.root.clone();std::thread::spawn(move||{let _=tx.send(update::prepare(&d));});self.update_rx=Some(rx);self.next_update=now()+21600}
    fn poll(&mut self,ctx:&Context){
        while let Ok(reply)=self.rx.try_recv(){match reply{
            Reply::Log(msg)=>self.record(msg),
            Reply::SetupFailed(message)=>{
                if self.setup_step.is_some() {
                    self.setup_error=Some(message);
                }
            },
            Reply::OrderActionFailed(id, message)=>{
                if let Some(card)=self.review_card.as_mut().filter(|card|card.id==id){
                    card.error=Some(message);
                }
            },
            Reply::Ready=>{self.pending=self.pending.saturating_sub(1);if self.pending==0{self.login=None}},
            Reply::OpenDemoOrder(id)=>{
                if self.v.demo {
                    self.review_card=Some(ReviewCard { id, track:None, error:None });
                }
            },
            Reply::State(v)=>{
                if self.v.demo != v.demo {
                    self.page=0;
                    self.selected.clear();
                    self.review=false;
                    self.review_card=None;
                    self.setup_step=None;
                    self.setup_error=None;
                    self.history_query.clear();
                    self.history_status=None;
                }
                self.v=v;
            }
        }}
        if self.stage.is_none()&&now()>=self.next_update{self.check_updates()}
        if let Some(result)=self.update_rx.as_ref().and_then(|rx|rx.try_recv().ok()){self.update_rx=None;match result{Ok(stage)=>{self.stage=stage;self.record(if self.stage.is_some(){self.t("Обновление готово","Update ready")}else{self.t("Установлена актуальная версия","Up to date")}.into())},Err(e)=>self.record(format!("{} · {}",self.t("Обновления","Updates"),crate::language::message(&self.lang,&crate::api::describe(&e))))}}
        if let Some((_,open,quit))=&self.tray{while let Ok(m)=MenuEvent::receiver().try_recv(){if m.id==*quit.id(){ctx.send_viewport_cmd(ViewportCommand::Close)}else if m.id==*open.id(){ctx.send_viewport_cmd(ViewportCommand::Visible(true));ctx.send_viewport_cmd(ViewportCommand::Minimized(false));ctx.send_viewport_cmd(ViewportCommand::Focus)}}}
        while let Ok(m)=TrayIconEvent::receiver().try_recv(){if matches!(m,TrayIconEvent::DoubleClick{..}){ctx.send_viewport_cmd(ViewportCommand::Visible(true));ctx.send_viewport_cmd(ViewportCommand::Minimized(false));ctx.send_viewport_cmd(ViewportCommand::Focus)}}
        let url=self.v.player["item"]["album"]["images"][0]["url"].as_str().unwrap_or("");if url!=self.art_url{self.art_url=url.into();self.art=None;self.art_rx=None;if !self.preview&&url.starts_with("https://"){let url=url.to_owned();let(tx,rx)=mpsc::channel();self.art_rx=Some(rx);std::thread::spawn(move||{use std::io::Read;let img=(||{let r=crate::api::client().get(&url).call().ok()?;let mut bytes=vec![];r.into_reader().take(4<<20).read_to_end(&mut bytes).ok()?;let img=image::load_from_memory(&bytes).ok()?.thumbnail(256,256).to_rgba8();Some(ColorImage::from_rgba_unmultiplied([img.width()as _,img.height()as _],img.as_raw()))})();let _=tx.send((url,img));});}}
        if let Some((u,img))=self.art_rx.as_ref().and_then(|rx|rx.try_recv().ok()){self.art_rx=None;if u==self.art_url{self.art=img.map(|v|ctx.load_texture("cover",v,Default::default()))}}
    }
    fn readiness_card(&mut self, ui: &mut Ui) {
        use CheckState::*;
        Frame::group(ui.style()).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            let t = |ru, en| self.t(ru, en);
            let checks = &self.v.readiness;
            let connected = t("Подключён", "Connected");
            let unchecked = t("Не проверено", "Unchecked");
            let check_on_start = t("Проверяется при запуске приёма.", "Checked when reception starts.");
            let retry = t("Ошибка проверки", "Check failed");
            let retry_hint = t("Проверьте интернет и повторите попытку. Подробности в журнале.", "Check your connection and retry. See the event log for details.");
            let storage_hint = t("Проверьте доступ к папке data и перезапустите приложение.", "Check access to the data folder and restart the app.");
            let reconnect = t("Требуется вход", "Sign-in required");
            let denied = t("Нет доступа", "Access denied");
            let no_account = t("Не подключён", "Disconnected");
            let twitch_hint = t("Подключите Twitch в настройках.", "Connect Twitch in Settings.");
            let spotify_hint = t("Подключите аккаунт Spotify Premium в настройках.", "Connect your Spotify Premium account in Settings.");
            let reward_hint = t("Выберите или создайте награду в настройках.", "Select or create a reward in Settings.");
            let twitch = match (self.v.account[0], checks.twitch) {
                (false, _) => (no_account, Some(twitch_hint)),
                (_, Ready) => (connected, None),
                (_, MissingPermissions) => (t("Нет прав чата", "Missing chat permissions"), Some(t("Переподключите Twitch и разрешите чтение и отправку сообщений.", "Reconnect Twitch and allow chat read and write access."))),
                (_, Reconnect) => (reconnect, Some(twitch_hint)),
                (_, AccessDenied) => (denied, Some(twitch_hint)),
                (_, Storage) => (retry, Some(storage_hint)),
                (_, Unchecked) => (t("Вход выполнен", "Signed in"), Some(check_on_start)),
                _ => (retry, Some(retry_hint)),
            };
            let spotify = match (self.v.account[1], checks.spotify) {
                (false, _) => (no_account, Some(spotify_hint)),
                (_, Ready) => (connected, None),
                (_, Reconnect) => (reconnect, Some(spotify_hint)),
                (_, AccessDenied) => (denied, Some(t("Проверьте Premium и доступ аккаунта в Spotify Developer Dashboard.", "Check Premium and account access in Spotify Developer Dashboard."))),
                (_, Storage) => (retry, Some(storage_hint)),
                (_, Unchecked) => (t("Вход выполнен", "Signed in"), Some(t("Нажмите «Проверить плеер», чтобы проверить доступ.", "Select Check player to verify access."))),
                _ => (retry, Some(retry_hint)),
            };
            let reward = match (s(&self.v.prefs, "reward").is_empty(), checks.reward) {
                (true, _) => (t("Не выбрана", "Not selected"), Some(reward_hint)),
                (_, Ready) => (t("Готова", "Ready"), self.v.rewards.iter().find(|reward| reward["id"] == self.v.prefs["reward"]).map(|reward| s(reward, "title"))),
                (_, RewardUnavailable | AccessDenied) => (t("Недоступна", "Unavailable"), Some(t("Обновите список наград и выберите доступную для TrackBridge.", "Refresh the reward list and select a reward TrackBridge can manage."))),
                (_, RewardSettings) => (t("Проверьте правила", "Check settings"), Some(t("В Twitch включите ввод текста и отключите пропуск очереди заявок.", "In Twitch, require text input and disable skipping the request queue."))),
                (_, Reconnect) => (reconnect, Some(twitch_hint)),
                (_, Storage) => (retry, Some(storage_hint)),
                (_, Unchecked) => (t("Выбрана", "Selected"), Some(check_on_start)),
                _ => (retry, Some(retry_hint)),
            };
            let player = match (self.v.account[1], checks.player) {
                (false, _) => (unchecked, Some(t("Сначала подключите Spotify.", "Connect Spotify first."))),
                (_, Ready) => (t("Готов", "Ready"), Some(self.v.device.as_str())),
                (_, NoActivePlayer) => (t("Нет устройства", "No active device"), Some(t("Запустите музыку в Spotify и нажмите «Проверить плеер».", "Play music in Spotify, then select Check player."))),
                (_, RestrictedPlayer) => (t("Есть ограничения", "Device restricted"), Some(t("Переключите воспроизведение на другое устройство Spotify.", "Switch playback to another Spotify device."))),
                _ if matches!(checks.spotify, Reconnect | AccessDenied | Retry | Storage) => (unchecked, Some(t("Сначала восстановите доступ к Spotify.", "Restore Spotify access first."))),
                _ => (unchecked, Some(t("Откройте Spotify и включите музыку.", "Open Spotify and play music."))),
            };

            ui.strong(t("Готовность к приёму заказов", "Request readiness"));
            let hint_width = ui.available_width() - 84. - 164. - 2. * ui.spacing().item_spacing.x;
            Grid::new("readiness").num_columns(3).min_row_height(22.).spacing([ui.spacing().item_spacing.x, 5.]).show(ui, |ui| {
                for (name, (status, hint), state) in [
                    ("Twitch", twitch, checks.twitch),
                    ("Spotify", spotify, checks.spotify),
                    (t("Награда", "Reward"), reward, checks.reward),
                    (t("Плеер", "Player"), player, checks.player),
                ] {
                    let status = if state == Ready { RichText::new(status).color(GREEN) } else { RichText::new(status) };
                    for (width, text) in [
                        (84., RichText::new(name).strong()),
                        (164., status),
                        (hint_width.max(80.), RichText::new(hint.unwrap_or_default())),
                    ] {
                        ui.allocate_ui_with_layout(vec2(width, 20.), Layout::left_to_right(Align::Center), |ui| {
                            ui.set_min_width(width);
                            ui.add(Label::new(text).wrap());
                        });
                    }
                    ui.end_row();
                }
            });
            ui.horizontal_wrapped(|ui| {
                if ui.add_enabled(!self.v.running && self.pending == 0,
                    Button::new(self.t("Мастер настройки", "Setup wizard")))
                    .on_disabled_hover_text(if self.v.running {
                        self.t("Перед настройкой остановите приём заказов.", "Stop reception before changing setup.")
                    } else {
                        self.t("Дождитесь окончания текущего действия.", "Wait for the current action to finish.")
                    }).clicked() {
                    self.setup_step = Some(SetupStep::first_incomplete(&self.v));
                    self.setup_error = None;
                }
                if ui.button(self.t("Открыть настройки", "Open settings")).clicked() {
                    self.page = 4;
                }
                if ui.add_enabled(self.pending == 0 && self.v.account[1],
                    Button::new(self.t("Проверить плеер", "Check player"))).clicked() {
                    self.send(Cmd::Check);
                }
            });
        });
    }

    fn demo_overview(&mut self, ui: &mut Ui) {
        Frame::group(ui.style()).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.heading(self.t("Демо-режим", "Demo mode"));
            ui.label(self.t("Выберите сценарий, чтобы попробовать работу с тестовыми заказами.",
                "Choose a scenario to practice with test requests."));
            ui.horizontal_wrapped(|ui| {
                for (scenario, label) in [
                    (DemoScenario::Success, self.t("Успешный заказ", "Successful request")),
                    (DemoScenario::Refund, self.t("Возврат баллов", "Points refund")),
                    (DemoScenario::ChooseTrack, self.t("Ручной выбор трека", "Manual track selection")),
                ] {
                    if ui.add_enabled(self.pending == 0, Button::new(label)).clicked() {
                        self.send(Cmd::RunDemo(scenario));
                    }
                }
            });
            if ui.add_enabled(self.pending == 0, Button::new(self.t("Выйти из демо", "Exit demo"))).clicked() {
                self.send(Cmd::ExitDemo);
            }
        });
        ui.add_space(16.);
        ui.heading(self.t("Тестовые заказы", "Test requests"));
        self.table(ui, true);
    }

    fn setup_wizard(&mut self, ui: &mut Ui, step: SetupStep) {
        use SetupStep::*;
        let index = step as usize;
        let labels = ["Twitch", "Spotify", self.t("Награда", "Reward"), self.t("Плеер", "Player")];
        let busy = self.pending > 0;
        let mut complete = step.complete(&self.v);
        if index < 2 {
            let key = ["twitchId", "spotifyId"][index];
            complete &= self.ids[index].trim() == s(&self.v.prefs, key);
        }

        Frame::group(ui.style()).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.heading(self.t("Мастер настройки", "Setup wizard"));
            ui.horizontal_wrapped(|ui| {
                for (i, label) in labels.into_iter().enumerate() {
                    let text = RichText::new(format!("{}. {label}", i + 1));
                    ui.label(if i == index { text.strong().color(GREEN) } else { text });
                }
            });
            ui.separator();
            ui.add_enabled_ui(!busy, |ui| {
                match step {
                    Twitch | Spotify => {
                        self.provider_link(ui, step == Spotify);
                        self.provider_help(ui, step == Spotify);
                        self.connect_account(ui, index, true);
                        if step == Twitch && self.v.account[0] && !step.complete(&self.v) {
                            ui.label(self.t("Переподключите Twitch и разрешите чтение и отправку сообщений чата.",
                                "Reconnect Twitch and allow chat read and write access."));
                        }
                    }
                    Reward => {
                        ui.label(self.t(
                            "Выберите доступную TrackBridge награду или создайте новую. Затем проверьте, что она принимает ссылку и сохраняет покупку в очереди заявок.",
                            "Select a reward TrackBridge can manage or create one. Then verify that it accepts a link and keeps the redemption in the request queue.",
                        ));
                        self.reward_controls(ui, true);
                        if ui.add_enabled(!s(&self.v.prefs, "reward").is_empty(),
                            Button::new(self.t("Проверить награду", "Check reward"))).clicked() {
                            self.setup_error = None;
                            self.send(Cmd::CheckReward);
                        }
                    }
                    Player => {
                        ui.label(self.t("Откройте Spotify и запустите музыку на устройстве, которое будет использоваться в эфире.",
                            "Open Spotify and play music on the device you’ll use on stream."));
                        if ui.button(self.t("Проверить плеер и настройки", "Check player and settings")).clicked() {
                            self.setup_error = None;
                            self.send(Cmd::CheckSetup);
                        }
                        if complete {
                            ui.colored_label(GREEN, self.t("Всё готово. Приём можно запустить на «Обзоре».",
                                "You’re ready. You can start reception from Overview."));
                            if !self.v.device.is_empty() { ui.label(&self.v.device); }
                        }
                    }
                }
            });
            self.cancel_connection(ui);
            if busy {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(self.t("Ожидание результата…", "Waiting for result…"));
                });
            } else if let Some(error) = &self.setup_error {
                ui.colored_label(Color32::LIGHT_RED, crate::language::message(&self.lang, error));
                let repair = SetupStep::first_incomplete(&self.v);
                if (repair as usize) < index
                    && ui.button(self.t("Вернуться к нужному шагу", "Go to the step that needs attention")).clicked() {
                    self.setup_step = Some(repair);
                    self.setup_error = None;
                }
            } else if complete && step != Player {
                ui.colored_label(GREEN, self.t("Шаг выполнен", "Step complete"));
            }
            ui.separator();
            ui.horizontal_wrapped(|ui| {
                if ui.add_enabled(!busy, Button::new(self.t("Закрыть мастер", "Close wizard"))).clicked() {
                    self.setup_step = None;
                }
                if index > 0 && ui.add_enabled(!busy, Button::new(self.t("Назад", "Back"))).clicked() {
                    self.setup_step = Some(SetupStep::ALL[index - 1]);
                    self.setup_error = None;
                }
                let next = if step == Player { self.t("Готово", "Finish") } else { self.t("Далее", "Next") };
                if ui.add_enabled(complete && !busy, Button::new(next)).clicked() {
                    self.setup_step = SetupStep::ALL.get(index + 1).copied();
                    self.setup_error = None;
                }
            });
        });
    }

    fn overview(&mut self,ui:&mut Ui){
        if self.v.demo {
            self.demo_overview(ui);
            return;
        }
        if let Some(step) = self.setup_step {
            self.setup_wizard(ui, step);
            return;
        }
        ui.horizontal_wrapped(|ui| {
            if ui.add_enabled(self.pending==0,Button::new(if self.v.running{self.t("Остановить","Stop")}else{self.t("Запустить","Start")})).clicked(){self.send(if self.v.running{Cmd::Stop}else{Cmd::Start})}
            if ui.add_enabled(!self.v.running && self.pending == 0, Button::new(self.t("Демо-режим", "Demo mode")))
                .on_disabled_hover_text(self.t("Для входа в демо остановите приём заказов.", "Stop reception before entering demo mode.")).clicked() {
                self.send(Cmd::EnterDemo);
            }
        });
        ui.add_space(10.);
        self.readiness_card(ui);
        ui.add_space(16.);
        Frame::group(ui.style()).show(ui,|ui|{ui.set_min_width(ui.available_width()-20.);ui.horizontal(|ui|{if let Some(art)=&self.art{ui.image((art.id(),vec2(80.,80.)));}ui.vertical(|ui|{let item=&self.v.player["item"];ui.strong(if s(item,"name").is_empty(){self.t("Нет текущего трека","No current track")}else{s(item,"name")});ui.label(item["artists"].as_array().into_iter().flatten().map(|v|s(v,"name")).collect::<Vec<_>>().join(", "));let duration=n(item,"duration_ms");if duration>0{ui.add(ProgressBar::new(n(&self.v.player,"progress_ms") as f32/duration as f32).desired_width(280.));}})});ui.separator();let viewers=self.v.viewers.map(|v|v.to_string()).unwrap_or_else(||"—".into());
        ui.label(format!("{} · {}",self.t("Зрители","Viewers"),viewers));
        if !b(&self.v.prefs,"votingEnabled"){ui.label(self.t("Голосование отключено","Voting disabled"));}
        else if self.v.threshold==0{ui.label(self.t("Порог определится при первом голосе","The first vote sets the threshold"));}
        else{ui.label(format!("{} · {} / {}",self.t("Голоса","Votes"),self.v.votes,self.v.threshold));}ui.horizontal(|ui|{if ui.add_enabled(self.v.running,Button::new(self.t("Пропустить","Skip"))).clicked(){self.send(Cmd::Skip)}if ui.add_enabled(self.v.running,Button::new(self.t("Сбросить голоса","Reset votes"))).clicked(){self.send(Cmd::Reset)}});});
        let count=self.v.orders.iter().filter(|o|needs_review(o)).count();
        if count>0 && ui.button(format!("{} · {count}",self.t("Требуют решения","Needs review"))).clicked(){
            self.page=1;self.review=true;self.selected.clear();
        }
        ui.add_space(16.);ui.heading(self.t("Последние заказы","Recent requests"));self.table(ui,true)
    }
    fn history_filters(&mut self, ui: &mut Ui) -> bool {
        let en = self.lang == "en";
        let hint = self.t("Имя зрителя или название трека", "Viewer name or track title");
        let all = self.t("Все статусы", "All statuses");
        let clear = self.t("Сбросить", "Clear");
        let previous_status = self.history_status;
        let mut query_changed = false;
        ui.horizontal_wrapped(|ui| {
            query_changed = ui.add(TextEdit::singleline(&mut self.history_query)
                .hint_text(hint)
                .desired_width(252.)).changed();
            ComboBox::from_id_salt("history-status")
                .width(190.)
                .selected_text(self.history_status.map_or(all, |status| status.label(en)))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.history_status, None, all);
                    for status in OrderStatus::ALL {
                        ui.selectable_value(&mut self.history_status, Some(status), status.label(en));
                    }
                });
            if ui.add_enabled(!self.history_query.is_empty() || self.history_status.is_some(), Button::new(clear)).clicked() {
                self.history_query.clear();
                self.history_status = None;
                query_changed = true;
            }
        });
        query_changed || previous_status != self.history_status
    }

    fn table(&mut self, ui: &mut Ui, recent: bool) {
        let mut open_order = None;
        if !recent {
            let history = self.t("История", "History");
            let review = self.t("Требуют решения", "Needs review");
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.review, false, history);
                ui.selectable_value(&mut self.review, true, review);
            });
        }
        let review_tab = !recent && self.review;
        let history_tab = !recent && !review_tab;
        let filters_changed = history_tab && self.history_filters(ui);
        let query = if history_tab { self.history_query.trim().to_lowercase() } else { String::new() };
        let status_filter = if history_tab { self.history_status } else { None };
        let filtering = !query.is_empty() || status_filter.is_some();
        let mut filtered_selection = if filtering && !self.selected.is_empty() {
            self.v.orders.iter()
                .filter(|order| self.selected.contains(s(order, "id"))
                    && !b(order, "historyHidden") && history_matches(order, &query, status_filter))
                .map(|order| s(order, "id").to_owned())
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        ui.horizontal(|ui| {
            let can_hide = if filtering { !filtered_selection.is_empty() } else { !self.selected.is_empty() };
            if ui.add_enabled(can_hide, Button::new(self.t("Скрыть выбранные", "Hide selected"))).clicked() {
                let ids = if filtering {
                    for id in &filtered_selection { self.selected.remove(id); }
                    std::mem::take(&mut filtered_selection)
                } else {
                    self.selected.drain().collect()
                };
                self.send(Cmd::Hide(ids));
            }
            if !self.v.demo && ui.button(self.t("Очистить сыгранные", "Clear played")).clicked() {
                self.send(Cmd::Clean);
            }
        });

        let mut scroll = ScrollArea::new([true, !recent]).id_salt(("orders", recent, review_tab));
        let track_width = (ui.available_width() - 660.).clamp(146., 240.);
        if filters_changed {
            scroll = scroll.vertical_scroll_offset(0.);
        }
        scroll.show(ui, |ui| {
            Grid::new(("rows", recent, review_tab)).striped(true).spacing([14., 10.]).show(ui, |ui| {
                for title in ["", self.t("Зритель", "Viewer"), self.t("Трек", "Track"), self.t("Статус", "Status"), self.t("Время", "Time"), ""] {
                    ui.strong(title);
                }
                ui.end_row();

                let at = now();
                let choice_minutes = n(&self.v.prefs, "trackChoiceMinutes");
                let en = self.lang == "en";
                let mut count = 0;
                for order in self.v.orders.iter().rev() {
                    let review = needs_review(order);
                    if (review_tab && !review) || (!review_tab && b(order, "historyHidden")) {
                        continue;
                    }
                    if filtering && !history_matches(order, &query, status_filter) {
                        continue;
                    }
                    let id = s(order, "id");
                    let mut selected = self.selected.contains(id);
                    if ui.push_id(id, |ui| ui.checkbox(&mut selected, "")).inner.changed() {
                        if selected { self.selected.insert(id.into()); }
                        else { self.selected.remove(id); }
                    }

                    let track = order_track(order);
                    let status = OrderStatus::from_order(order).label(en);
                    let status = match track_choice_deadline(order, choice_minutes) {
                        Some(deadline) => {
                            let seconds = deadline.saturating_sub(at).max(0);
                            format!("{status} · {}:{:02}", seconds / 60, seconds % 60).into()
                        }
                        None => std::borrow::Cow::Borrowed(status),
                    };
                    let details = review || self.v.demo;
                    let sense = if details { Sense::click() } else { Sense::hover() };
                    let mut clicked = false;
                    for (index, value) in [s(order, "user"), track, status.as_ref(), s(order, "time")].into_iter().enumerate() {
                        let label = Label::new(value).sense(sense).selectable(false);
                        let mut cell = if index == 1 {
                            ui.allocate_ui_with_layout(vec2(track_width, 18.), Layout::left_to_right(Align::Center), |ui| {
                                ui.set_min_width(track_width);
                                ui.add(label.truncate())
                            }).inner
                        } else { ui.add(label) };
                        if details { cell = cell.on_hover_cursor(CursorIcon::PointingHand); }
                        if index == 1 { cell = cell.on_hover_text(value); }
                        if index == 2 { cell = cell.on_hover_text(crate::language::message(&self.lang, s(order, "note"))); }
                        clicked |= cell.clicked();
                    }
                    if details {
                        clicked |= ui.small_button(self.t("Подробнее", "Details")).clicked();
                        if clicked { open_order = Some(id.to_owned()); }
                    } else {
                        ui.label("");
                    }
                    ui.end_row();
                    count += 1;
                    if recent && count >= 8 { break; }
                }
                if count == 0 {
                    ui.label("—");
                    ui.label(if review_tab {
                        self.t("Нет заказов, требующих решения", "No requests need review")
                    } else if filtering {
                        self.t("По вашему запросу ничего не найдено", "No requests match your filters")
                    } else {
                        self.t("Заказов пока нет", "No requests yet")
                    });
                    ui.end_row();
                }
            });
        });
        if let Some(id) = open_order {
            self.review_card = Some(ReviewCard { id, track: None, error: None });
        }
    }

    fn show_review_card(&mut self, ctx: &Context) {
        let Some(mut card) = self.review_card.take() else { return };
        let Some(order) = self.v.orders.iter().find(|order| order["id"] == card.id) else { return };
        let needs_track = order["status"] == "needs_track";
        let review = needs_review(order);
        let timed_out = b(order, "choiceTimedOut");
        let status = OrderStatus::from_order(order).label(self.lang == "en");
        let deadline = track_choice_deadline(order, n(&self.v.prefs, "trackChoiceMinutes"));
        let seconds = deadline.map(|deadline| deadline.saturating_sub(now()).max(0));
        let channel = s(&self.v.prefs, "broadcaster");
        let same_channel = !channel.is_empty() && s(order, "broadcaster") == channel;
        let can_act = review && self.pending == 0 && same_channel && seconds != Some(0);
        let mut close = false;
        let mut command = None;
        let frame = Frame::popup(&ctx.style()).inner_margin(18);
        let modal = Modal::new(Id::new("order-review-card")).frame(frame).show(ctx, |ui| {
            let width = (ctx.screen_rect().width() - 64.).clamp(240., 560.);
            ui.set_width(width);
            ui.heading(if self.v.demo { self.t("Тестовый заказ", "Test request") }
                else { self.t("Спорный заказ", "Request review") });
            ui.horizontal_wrapped(|ui| {
                ui.strong(s(order, "user"));
                ui.label(status);
            });
            ui.separator();

            let input = s(order, "input");
            ui.horizontal_wrapped(|ui| {
                ui.strong(self.t("Исходная ссылка", "Original link"));
                if !input.is_empty() && ui.small_button(self.t("Копировать", "Copy")).clicked() {
                    ctx.copy_text(input.to_owned());
                }
            });
            ui.add(Label::new(if input.is_empty() { self.t("Ссылка не указана", "No link provided") } else { input })
                .wrap().selectable(true));

            ui.add_space(6.);
            ui.strong(self.t("Причина", "Reason"));
            let note = s(order, "note");
            if !note.is_empty() {
                ui.add(Label::new(crate::language::message(&self.lang, note)).wrap());
            } else if review {
                let reason = match s(order, "status") {
                    "needs_track" => self.t("Нужно уточнить, какой трек Spotify заказал зритель.", "Choose the Spotify track requested by the viewer."),
                    "uncertain" => self.t("Spotify не подтвердил добавление трека.", "Spotify did not confirm that the track was added."),
                    _ => self.t("Twitch не завершил обработку заказа.", "Twitch could not complete the request."),
                };
                ui.label(reason);
            } else {
                ui.label(status);
            }

            ui.add_space(6.);
            ui.strong(self.t("Оставшееся время", "Time remaining"));
            match seconds {
                Some(remaining) if remaining > 0 => {
                    ui.label(format!("{} {}:{:02}", self.t("До автовозврата", "Until automatic refund"), remaining / 60, remaining % 60));
                }
                Some(_) => {
                    ui.label(self.t("Время выбора истекло. Заказ передаётся на автовозврат.", "Selection time expired. The request is being sent for automatic refund."));
                }
                None if timed_out => {
                    ui.label(self.t("Срок выбора трека истёк.", "Track selection time expired."));
                }
                None if needs_track => {
                    ui.label(self.t("Автовозврат по таймеру отключён.", "Automatic refund timer is off."));
                }
                None => {
                    ui.label(self.t("Для этого заказа таймера нет.", "There is no timer for this request."));
                }
            }
            if review && !same_channel {
                ui.label(self.t("Для действий подключите Twitch-канал, на котором сделан заказ.", "Connect the Twitch channel that received this request to take action."));
            }

            ui.separator();
            if review && let Some(error) = &card.error {
                ui.colored_label(Color32::LIGHT_RED, crate::language::message(&self.lang, error));
            }
            if needs_track && let Some(track) = &mut card.track {
                ui.label(self.t("Вставьте ссылку на трек Spotify. Действуют обычные правила модерации.", "Paste a Spotify track link. The usual moderation rules apply."));
                ui.add_enabled(can_act, TextEdit::singleline(track).desired_width(width)
                    .hint_text("https://open.spotify.com/track/…"));
                if self.v.demo && ui.add_enabled(can_act, Button::new(self.t("Подставить тестовую ссылку", "Use a sample link"))).clicked() {
                    *track = DEMO_TRACK_LINK.to_owned();
                }
                let valid = crate::orders::spotify(track).is_some();
                if ui.add_enabled(can_act && valid, Button::new(self.t("Передать в очередь заказов", "Submit for processing"))).clicked() {
                    command = Some(Cmd::ChooseTrack(card.id.clone(), track.clone()));
                }
            }
            ui.horizontal_wrapped(|ui| {
                if needs_track && card.track.is_none() && ui.add_enabled(can_act, Button::new(self.t("Указать трек", "Choose track"))).clicked() {
                    card.track = Some(String::new());
                }
                if review && !needs_track && !timed_out &&
                    ui.add_enabled(can_act, Button::new(self.t("Уже добавлен", "Already added"))).clicked() &&
                    confirm(self.t("Трек уже в Spotify? Подтвердить покупку?", "Track already in Spotify? Fulfill redemption?"))
                {
                    command = Some(Cmd::Resolve(card.id.clone(), true));
                }
                if review && ui.add_enabled(can_act, Button::new(self.t("Вернуть баллы", "Refund points"))).clicked() &&
                    confirm(self.t("Вернуть баллы зрителю?", "Refund this viewer?"))
                {
                    command = Some(Cmd::Resolve(card.id.clone(), false));
                }
                close = ui.button(self.t("Закрыть", "Close")).clicked();
            });
        });
        if let Some(command) = command {
            card.error = None;
            self.send(command);
        }
        if !close && !modal.should_close() {
            self.review_card = Some(card);
        }
    }
    fn moderation(&mut self,ui:&mut Ui){ui.add_enabled_ui(!self.v.running,|ui|{
        ui.horizontal(|ui|{ui.label(self.t("Лимит минут (0 — без лимита)","Limit in minutes (0 for unlimited)"));ui.add(DragValue::new(&mut self.minutes).range(0..=1440));});let label=self.t("Запретить Explicit","Block explicit tracks");ui.checkbox(&mut self.explicit,label);ui.add_space(10.);
        ui.horizontal(|ui| {
            ui.label(self.t("Лимит на зрителя","Viewer limit"));
            ui.add(DragValue::new(&mut self.viewer_limit).range(1..=100));
            ui.label(self.t("треков за","tracks per"));
            ui.add(DragValue::new(&mut self.viewer_window).range(1..=1440));
            ui.label(self.t("минут","minutes"));
        });
        ui.columns(2,|cols|{for(i,col)in cols.iter_mut().enumerate(){col.strong(if i==0{self.t("Запрещённые треки","Blocked tracks")}else{self.t("Запрещённые исполнители","Blocked artists")});col.label(self.t("По одному на строку","One per line"));col.add(TextEdit::multiline(&mut self.rules[i]).desired_rows(12).desired_width(f32::INFINITY));}});
        if ui.button(self.t("Сохранить правила","Save rules")).clicked(){self.send(Cmd::Set(json!({"viewerTrackLimit":self.viewer_limit,"viewerWindowMinutes":self.viewer_window,"maxTrackMinutes":self.minutes,"blockExplicit":self.explicit,"blockedTracks":self.rules[0],"blockedArtists":self.rules[1]})));}
    });}
    fn provider_link(&self, ui: &mut Ui, spotify: bool) {
        let (label, url) = if spotify {
            ("Spotify Developer Dashboard", "https://developer.spotify.com/dashboard")
        } else {
            ("Twitch Developer Console", "https://dev.twitch.tv/console/apps")
        };
        ui.scope(|ui| {
            ui.visuals_mut().override_text_color = None;
            ui.hyperlink_to(label, url);
        });
    }

    fn provider_help(&self, ui: &mut Ui, spotify: bool) {
        if spotify {
            ui.label(self.t(
                "Создайте приложение в Spotify Developer Dashboard, выберите Web API и добавьте этот адрес в Redirect URIs.",
                "Create an app in Spotify Developer Dashboard, select Web API, and add this address to Redirect URIs.",
            ));
            ui.horizontal_wrapped(|ui| {
                ui.monospace(crate::api::SPOTIFY_REDIRECT_URI);
                if ui.small_button(self.t("Копировать адрес", "Copy address")).clicked() {
                    ui.ctx().copy_text(crate::api::SPOTIFY_REDIRECT_URI.to_owned());
                }
            });
            ui.label(self.t(
                "Вставьте Client ID приложения. В браузере разрешите доступ аккаунту Spotify Premium, с которого будете воспроизводить музыку.",
                "Paste the app’s Client ID. In your browser, authorize the Spotify Premium account you’ll use for playback.",
            ));
            ui.small(self.t(
                "Если Spotify требует добавить пользователя, добавьте аккаунт в Users and Access.",
                "If Spotify requires an allowed user, add the account under Users and Access.",
            ));
        } else {
            ui.label(self.t(
                "Войдите как владелец канала и зарегистрируйте приложение типа Public. Если форма требует OAuth Redirect URL, добавьте http://localhost:43821.",
                "Sign in as the channel owner and register a Public app. If the form requires an OAuth Redirect URL, add http://localhost:43821.",
            ));
            ui.label(self.t(
                "Откройте Manage, вставьте Client ID и нажмите «Подключить». В окне TrackBridge нажмите «Показать код», затем введите его на странице Twitch и разрешите доступ.",
                "Open Manage, paste the Client ID, and select Connect. Select Show code in TrackBridge, then enter it on Twitch and approve access.",
            ));
        }
    }

    fn connection_help(&self, ui: &mut Ui) {
        ui.heading(self.t("Подключение", "Connections"));
        ui.horizontal_wrapped(|ui| {
            self.provider_link(ui, false);
            self.provider_link(ui, true);
        });
        CollapsingHeader::new(self.t("Как подключить Twitch и Spotify", "How to connect Twitch and Spotify"))
            .id_salt("connection-help")
            .show(ui, |ui| {
                ui.strong("Twitch");
                self.provider_help(ui, false);
                ui.add_space(6.);
                ui.strong("Spotify");
                self.provider_help(ui, true);
            });
        ui.add_space(8.);
    }

    fn connect_account(&mut self, ui: &mut Ui, index: usize, wizard: bool) {
        let name = ["Twitch", "Spotify"][index];
        let key = ["twitchId", "spotifyId"][index];
        ui.horizontal_wrapped(|ui| {
            ui.label(format!("{name} Client ID"));
            ui.text_edit_singleline(&mut self.ids[index]);
            let connect = self.t("Подключить", "Connect");
            if ui.add_enabled(!self.ids[index].trim().is_empty(), Button::new(connect)).clicked() {
                let ids = if wizard { json!({(key):self.ids[index].trim()}) }
                    else { json!({"twitchId":self.ids[0].trim(),"spotifyId":self.ids[1].trim()}) };
                self.setup_error = None;
                let cancel = Arc::new(AtomicBool::new(false));
                self.login = Some(cancel.clone());
                let (tx, rx) = mpsc::channel();
                self.auth_window = AuthWindow::new(rx);
                self.send(Cmd::Login(index == 1, ids, cancel, tx));
            }
            if self.v.account[index] {
                ui.label(self.t("Подключён", "Connected"));
            }
        });
    }

    fn cancel_connection(&self, ui: &mut Ui) {
        if let Some(cancel) = &self.login {
            if ui.button(self.t("Отменить подключение", "Cancel connection")).clicked() {
                cancel.store(true, Ordering::Relaxed);
            }
        }
    }

    fn reward_controls(&mut self, ui: &mut Ui, wizard: bool) {
        ui.horizontal_wrapped(|ui| {
            let prompt = self.t("Выберите награду", "Select reward");
            let mut choice = None;
            let selected = &self.v.prefs["reward"];
            let label = self.v.rewards.iter().find(|reward| &reward["id"] == selected)
                .map_or(prompt, |reward| s(reward, "title"));
            ComboBox::from_id_salt("reward").selected_text(label).show_ui(ui, |ui| {
                for reward in &self.v.rewards {
                    if ui.selectable_label(selected == &reward["id"], s(reward, "title")).clicked() {
                        choice = Some(reward["id"].clone());
                    }
                }
            });
            if let Some(choice) = choice {
                self.setup_error = None;
                self.set("reward", choice);
            }
            if ui.button(self.t("Обновить список", "Refresh rewards")).clicked() {
                self.setup_error = None;
                if !wizard { self.ids(); }
                self.send(Cmd::Rewards);
            }
        });
        ui.horizontal_wrapped(|ui| {
            if wizard { ui.label(self.t("Название и цена в баллах", "Name and cost in points")); }
            ui.text_edit_singleline(&mut self.title);
            ui.add(DragValue::new(&mut self.cost).range(1..=1_000_000_000));
            if ui.button(self.t("Создать награду", "Create reward")).clicked() {
                self.setup_error = None;
                self.send(Cmd::Create(self.title.clone(), self.cost));
            }
        });
    }

    fn settings(&mut self,ui:&mut Ui){let en=self.lang=="en";ui.horizontal(|ui|{ui.label(self.t("Язык","Language"));for(code,label)in [("ru","Русский"),("en","English")]{if ui.selectable_label(self.lang==code,label).clicked(){self.lang=code.into();self.set("language",json!(code));if let Some((_,open,quit))=&self.tray{open.set_text(self.t("Открыть","Open"));quit.set_text(self.t("Выход","Exit"));}}}});
        ui.separator();
        self.connection_help(ui);
        ui.add_enabled_ui(!self.v.running && self.pending == 0, |ui| {
            self.connect_account(ui, 0, false);
            self.connect_account(ui, 1, false);
            if ui.button(self.t("Сохранить Client ID", "Save Client IDs")).clicked() { self.ids(); }
            ui.separator();
            self.reward_controls(ui, false);
        });
        self.cancel_connection(ui);
        ui.separator();for (k,ru,en) in [("autoHidePlayed","Скрывать сыгранные","Automatically hide played requests"),("votingEnabled","Разрешить голосование","Enable voting"),("skipAutomatic","Автоматический порог голосов","Automatic vote threshold"),("requestSoundEnabled","Сигнал при добавлении трека","Play sound when queued")]{let mut value=b(&self.v.prefs,k);if ui.add_enabled(k!="skipAutomatic"||!self.v.running,Checkbox::new(&mut value,self.t(ru,en))).changed(){self.set(k,json!(value))}}
        ui.horizontal(|ui|{
            ui.label(self.t("Автовозврат при ожидании трека, мин","Refund unselected tracks after (minutes)"));
            ui.add(DragValue::new(&mut self.choice_minutes).range(0..=1440));
            ui.small(self.t("0 — выключить","0 disables"));
            if ui.add_enabled(self.choice_minutes!=n(&self.v.prefs,"trackChoiceMinutes"),Button::new(self.t("Применить","Apply"))).clicked(){
                self.set("trackChoiceMinutes",json!(self.choice_minutes));
            }
        });
        ui.small(self.t("Срок считается с начала ожидания, в том числе для уже открытых заказов.","The interval starts when selection is requested and applies to existing requests."));
        let mut votes=n(&self.v.prefs,"skipManualVotes");if ui.add_enabled(!self.v.running&&!b(&self.v.prefs,"skipAutomatic"),Slider::new(&mut votes,1..=99).text(self.t("Ручной порог","Manual votes"))).changed(){self.set("skipManualVotes",json!(votes))}
        let mut vol=n(&self.v.prefs,"requestSoundVolume");if ui.add(Slider::new(&mut vol,0..=40).text(self.t("Громкость","Volume"))).changed(){self.set("requestSoundVolume",json!(vol))}
        ui.horizontal(|ui|{if ui.button(self.t("Прослушать","Preview sound")).clicked(){self.send(Cmd::Preview)}if ui.add_enabled(!self.v.running,Button::new(self.t("Выбрать аудио…","Import audio…"))).clicked(){if let Some(path)=rfd::FileDialog::new().pick_file(){self.send(Cmd::Import(path))}}if ui.add_enabled(!self.v.running,Button::new(self.t("Стандартный сигнал","Default sound"))).clicked(){self.set("requestSoundName",json!(""))}ui.label(s(&self.v.prefs,"requestSoundName"));});
        ui.separator();if self.tray.is_some() && ui.button(self.t("Свернуть в трей","Minimize to tray")).clicked(){ui.ctx().send_viewport_cmd(ViewportCommand::Visible(false));}ui.label(if en{"Spotify Premium is required."}else{"Для добавления в очередь нужен Spotify Premium."});
    }
    fn journal(&mut self,ui:&mut Ui){ui.horizontal(|ui|{if ui.button(self.t("Копировать","Copy")).clicked(){ui.ctx().copy_text(self.log.iter().cloned().collect::<Vec<_>>().join("\n"))}if ui.button(self.t("Очистить","Clear")).clicked(){self.log.clear()}let l=self.t("Автопрокрутка","Auto-scroll");ui.checkbox(&mut self.scroll,l);});ScrollArea::vertical().stick_to_bottom(self.scroll).show(ui,|ui|{for line in &self.log{ui.monospace(line);}});}
    fn about(&mut self,ui:&mut Ui){ui.heading(format!("TrackBridge {}",update::CURRENT));ui.label("Rub1nchik · Rust");for(label,url)in [(self.t("Репозиторий","Repository"),"https://github.com/Rub1nchik702/TrackBridge"),(self.t("Инструкция","Setup guide"),if self.lang=="ru"{"https://github.com/Rub1nchik702/TrackBridge/blob/main/docs/GUIDE.ru.md"}else{"https://github.com/Rub1nchik702/TrackBridge/blob/main/docs/GUIDE.md"})]{if ui.link(label).clicked(){let _=open_url(url);}}
        if let Some(stage)=&self.stage{if ui.button(self.t("Установить обновление","Install update")).clicked()&&confirm(self.t("Остановить приём и установить обновление?","Stop and install update?")){match update::launch(stage){Ok(())=>ui.ctx().send_viewport_cmd(ViewportCommand::Close),Err(e)=>self.record(e.to_string())}}}else if ui.add_enabled(self.update_rx.is_none()&&!self.preview,Button::new(self.t("Проверить обновления","Check updates"))).clicked(){self.check_updates()}
    }
}
pub fn style(ctx:&Context){
        let mut v=Visuals::dark();
        v.panel_fill=Color32::from_rgb(22,26,33);v.window_fill=v.panel_fill;
        v.extreme_bg_color=Color32::from_rgb(16,20,25);v.faint_bg_color=Color32::from_rgb(30,35,43);
        v.widgets.inactive.bg_fill=Color32::from_rgb(45,53,64);
        v.widgets.hovered.bg_fill=Color32::from_rgb(56,67,79);
        v.selection.bg_fill=Color32::from_rgb(41,82,69);v.selection.stroke=Stroke::new(1_f32,GREEN);
        v.override_text_color=Some(Color32::from_rgb(233,237,243));ctx.set_visuals(v);
        ctx.style_mut(|s|{s.spacing.item_spacing=vec2(9.,9.);s.spacing.button_padding=vec2(12.,7.);s.spacing.interact_size.y=30.;
        for(key,size)in [(TextStyle::Body,16.),(TextStyle::Button,15.),(TextStyle::Small,13.),(TextStyle::Heading,25.),(TextStyle::Monospace,14.)]{if let Some(font)=s.text_styles.get_mut(&key){font.size=size;}}
        unsafe{use windows_sys::Win32::UI::WindowsAndMessaging::{SystemParametersInfoW,SPI_GETCLIENTAREAANIMATION};let mut enabled=1i32;if SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION,0,&mut enabled as *mut _ as _,0)!=0&&enabled==0{s.animation_time=0.;}}
        });
    }
fn confirm(text:&str)->bool{rfd::MessageDialog::new().set_title("TrackBridge").set_description(text).set_buttons(rfd::MessageButtons::YesNo).show()==rfd::MessageDialogResult::Yes}
impl eframe::App for Desktop{
    fn update(&mut self,ctx:&Context,_:&mut eframe::Frame){self.poll(ctx);self.auth_window.show(ctx,self.login.as_ref(),self.lang=="en");ctx.request_repaint_after(Duration::from_millis(150));
        let pages=[self.t("Обзор","Overview"),self.t("Заказы зрителей","Viewer requests"),self.t("Модерация","Moderation"),self.t("Журнал","Event log"),self.t("Настройки","Settings"),self.t("О программе","About")];
        SidePanel::left("sidebar").resizable(false).exact_width(185.).frame(Frame::side_top_panel(&ctx.style()).inner_margin(16)).show(ctx,|ui|{ui.add_space(12.);ui.heading(RichText::new("TrackBridge").color(GREEN));ui.add_space(20.);for(i,title)in pages.iter().enumerate(){if self.v.demo&&i>1{break}let r=ui.add_sized([149.,38.],Button::new(*title).selected(i==self.page));if r.clicked(){self.page=i;if !self.v.demo{self.record(format!("· {title}"));}}if self.page==i{let y=ctx.animate_value_with_time(Id::new("nav-marker"),r.rect.center().y,ctx.style().animation_time);ui.painter().rect_filled(Rect::from_center_size(pos2(r.rect.left()-5.,y),vec2(3.,24.)),2.,GREEN);}}ui.with_layout(Layout::bottom_up(Align::LEFT),|ui|{ui.small(format!("v{}",update::CURRENT));});});
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                let (color, status) = if self.v.demo {
                    (Color32::LIGHT_YELLOW, self.t("Демо-режим · тестовые заказы", "Demo mode · test requests"))
                } else if self.v.connected {
                    (GREEN, self.t("● Приём заказов активен", "● Receiving requests"))
                } else if self.v.running {
                    (Color32::GRAY, self.t("Подключение · резервный опрос активен", "Connecting · fallback polling active"))
                } else {
                    (Color32::GRAY, self.t("Приём остановлен", "Reception stopped"))
                };
                ui.colored_label(color, status);
                if self.pending > 0 { ui.spinner(); }
                if !self.v.demo && let Some(last) = self.log.back() {
                    ui.separator();
                    ui.add(Label::new(last).truncate()).on_hover_text(last);
                }
            });
        });
        CentralPanel::default().frame(Frame::central_panel(&ctx.style()).inner_margin(22)).show(ctx,|ui|{ui.heading(pages[self.page]);ui.add_space(14.);match self.page{0=>{ScrollArea::vertical().id_salt("overview").show(ui,|ui|self.overview(ui));},1=>self.table(ui,false),2=>self.moderation(ui),3=>self.journal(ui),4=>{ScrollArea::vertical().show(ui,|ui|self.settings(ui));},_=>self.about(ui)}});
        self.show_review_card(ctx);
        self.frames+=1;if let Some(path)=&self.shot{if self.frames==4{ctx.send_viewport_cmd(ViewportCommand::Screenshot(Default::default()));}if let Some(image)=ctx.input(|i|i.events.iter().find_map(|event|if let egui::Event::Screenshot{image,..}=event{Some(image.clone())}else{None})){let bytes:Vec<_>=image.pixels.iter().flat_map(|c|c.to_array()).collect();let _=image::save_buffer(path,&bytes,image.width()as _,image.height()as _,image::ColorType::Rgba8);ctx.send_viewport_cmd(ViewportCommand::Close);}}
    }
}
impl Drop for Desktop{fn drop(&mut self){if let Some(c)=&self.login{c.store(true,Ordering::Relaxed)}self.exit.store(true,Ordering::Relaxed);let _=self.tx.send(Cmd::Quit);if let Some(h)=self.join.take(){let _=h.join();}}}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::Engine;

    fn text_position(output: &FullOutput, label: &str) -> Option<Pos2> {
        output.shapes.iter().find_map(|shape| match &shape.shape {
            epaint::Shape::Text(text) if text.galley.text() == label => {
                Some(text.pos + text.galley.size() / 2.)
            }
            _ => None,
        })
    }

    #[test]
    fn setup_wizard_opens_from_overview_and_requires_verified_steps() -> anyhow::Result<()> {
        let dir = std::env::temp_dir().join(format!("TrackBridge-setup-{:x}", rand::random::<u128>()));
        let mut profile = Profile::open(dir.clone())?;
        profile.v["language"] = json!("en");
        let (commands, received) = mpsc::channel();
        let (replies, rx) = mpsc::channel();
        let mut app = Desktop::new(Engine::new(profile, replies).view(), commands, rx,
            Arc::new(AtomicBool::new(false)), std::thread::spawn(|| {}), dir.clone(), true, None, false, None);
        let ctx = Context::default();
        let frame = |app: &mut Desktop, events| ctx.run(RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000., 640.))),
            events,
            ..Default::default()
        }, |ctx| { CentralPanel::default().show(ctx, |ui| app.overview(ui)); });
        let click = |app: &mut Desktop, label| {
            frame(app, Vec::new());
            let output = frame(app, Vec::new());
            let pos = text_position(&output, label).unwrap_or_else(|| panic!("Missing {label}"));
            let pointer = |pressed| Event::PointerButton {
                pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE,
            };
            frame(app, vec![Event::PointerMoved(pos), pointer(true)]);
            frame(app, vec![pointer(false)]);
        };
        click(&mut app, "Setup wizard");
        assert!(app.setup_step == Some(SetupStep::Twitch));
        click(&mut app, "Next");
        assert!(app.setup_step == Some(SetupStep::Twitch));
        app.v.account[0] = true;
        app.v.prefs["chatAllowed"] = json!(true);
        app.v.prefs["chatReadAllowed"] = json!(true);
        click(&mut app, "Next");
        assert!(app.setup_step == Some(SetupStep::Spotify));
        app.v.account[1] = true;
        click(&mut app, "Next");
        assert!(app.setup_step == Some(SetupStep::Reward));
        app.v.prefs["reward"] = json!("test-reward");
        click(&mut app, "Next");
        assert!(app.setup_step == Some(SetupStep::Reward));
        click(&mut app, "Check reward");
        assert!(matches!(received.try_recv()?, Cmd::CheckReward));
        app.pending = 0;
        app.v.readiness.reward = CheckState::Ready;
        click(&mut app, "Next");
        assert!(app.setup_step == Some(SetupStep::Player));
        click(&mut app, "Finish");
        assert!(app.setup_step == Some(SetupStep::Player));
        click(&mut app, "Check player and settings");
        assert!(matches!(received.try_recv()?, Cmd::CheckSetup));
        app.pending = 0;
        app.v.readiness.twitch = CheckState::Ready;
        app.v.readiness.spotify = CheckState::Ready;
        app.v.readiness.player = CheckState::Ready;
        click(&mut app, "Finish");
        assert!(app.setup_step.is_none());
        assert!(received.try_recv().is_err());

        drop(app);
        std::fs::remove_file(dir.join("settings.bin"))?;
        std::fs::remove_dir(dir)?;
        Ok(())
    }

    #[test]
    fn history_combines_search_and_status_without_filtering_the_review_tab() -> anyhow::Result<()> {
        let dir = std::env::temp_dir().join(format!("TrackBridge-history-{:x}", rand::random::<u128>()));
        let mut profile = Profile::open(dir.clone())?;
        profile.v["language"] = json!("en");
        profile.v["orders"] = json!([
            {"id":"1","user":"Алиса","trackTitle":"First song","status":"done","played":true},
            {"id":"2","user":"Боб","trackTitle":"Алиса в стране чудес","status":"done"},
            {"id":"3","user":"Другой зритель","input":"Review fixture","status":"uncertain","historyHidden":true},
        ]);
        let (commands, _commands_rx) = mpsc::channel();
        let (replies, rx) = mpsc::channel();
        let view = Engine::new(profile, replies).view();
        let mut app = Desktop::new(view, commands, rx, Arc::new(AtomicBool::new(false)),
            std::thread::spawn(|| {}), dir.clone(), true, None, false, None);
        let ctx = Context::default();
        let frame = |app: &mut Desktop| ctx.run(RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000., 640.))),
            ..Default::default()
        }, |ctx| {
            CentralPanel::default().show(ctx, |ui| app.table(ui, false));
        });
        app.history_query = "  АЛИС  ".into();
        frame(&mut app);
        let all = frame(&mut app);
        assert!(text_position(&all, "Алиса").is_some());
        assert!(text_position(&all, "Боб").is_some());
        assert!(text_position(&all, "First song").is_some());
        app.history_status = Some(OrderStatus::Played);
        let played = frame(&mut app);
        assert!(text_position(&played, "Алиса").is_some());
        assert!(text_position(&played, "Боб").is_none());
        app.history_query = "нет совпадений".into();
        assert!(text_position(&frame(&mut app), "No requests match your filters").is_some());
        app.review = true;
        frame(&mut app);
        let review = frame(&mut app);
        assert!(text_position(&review, "Другой зритель").is_some());
        assert!(text_position(&review, "Viewer name or track title").is_none());

        drop(app);
        std::fs::remove_file(dir.join("settings.bin"))?;
        std::fs::remove_dir(dir)?;
        Ok(())
    }

    #[test]
    fn clicking_a_review_order_opens_its_details_and_track_picker() -> anyhow::Result<()> {
        let dir = std::env::temp_dir().join(format!("TrackBridge-order-card-{:x}", rand::random::<u128>()));
        let mut profile = Profile::open(dir.clone())?;
        profile.v["language"] = json!("en");
        profile.v["broadcaster"] = json!("channel");
        let source = "https://youtu.be/dQw4w9WgXcQ";
        profile.v["orders"] = json!([{
            "id":"request", "broadcaster":"channel", "user":"Fixture viewer",
            "status":"needs_track", "input":source, "note":"Choose the Spotify track",
            "choiceSince":now(), "historyHidden":true
        }]);
        let (commands, _commands_rx) = mpsc::channel();
        let (replies, rx) = mpsc::channel();
        let view = Engine::new(profile, replies).view();
        let mut app = Desktop::new(view, commands, rx, Arc::new(AtomicBool::new(false)),
            std::thread::spawn(|| {}), dir.clone(), true, None, false, None);
        app.review = true;
        let ctx = Context::default();
        let mut frame = |events| ctx.run(RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000., 640.))),
            events,
            ..Default::default()
        }, |ctx| {
            CentralPanel::default().show(ctx, |ui| app.table(ui, false));
            app.show_review_card(ctx);
        });
        let click = |pos, pressed| Event::PointerButton {
            pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE,
        };

        frame(Vec::new());
        let list = frame(Vec::new());
        let row = text_position(&list, "Fixture viewer").unwrap();
        frame(vec![Event::PointerMoved(row), click(row, true)]);
        frame(vec![click(row, false)]);
        frame(Vec::new());
        let details = frame(Vec::new());
        for label in ["Request review", source, "Choose the Spotify track", "Refund points"] {
            assert!(text_position(&details, label).is_some(), "Missing {label}");
        }
        assert!(details.shapes.iter().any(|shape| matches!(&shape.shape,
            epaint::Shape::Text(text) if text.galley.text().starts_with("Until automatic refund"))));
        let choose = text_position(&details, "Choose track").unwrap();
        frame(vec![Event::PointerMoved(choose), click(choose, true)]);
        frame(vec![click(choose, false)]);
        let picker = frame(Vec::new());
        assert!(text_position(&picker, "Submit for processing").is_some());

        drop(app);
        std::fs::remove_file(dir.join("settings.bin"))?;
        std::fs::remove_dir(dir)?;
        Ok(())
    }

    #[test]
    fn twitch_code_only_reaches_the_temporary_login_window() -> anyhow::Result<()> {
        let dir=std::env::temp_dir().join(format!("TrackBridge-login-test-{:x}",rand::random::<u128>()));
        let p=Profile::open(dir.clone())?;
        let (commands,_commands_rx)=mpsc::channel();
        let (replies,rx)=mpsc::channel();
        let view=Engine::new(p,replies.clone()).view();
        let diagnostics=dir.join("diagnostics.log");
        let cancel=Arc::new(AtomicBool::new(false));
        let mut app=Desktop::new(view,commands,rx,Arc::new(AtomicBool::new(false)),std::thread::spawn(||{}),dir.clone(),true,None,true,Some(diagnostics.clone()));
        let (private_tx,private_rx)=mpsc::channel();
        app.login=Some(cancel.clone());app.auth_window=AuthWindow::new(private_rx);
        let secret="FIXTURE-PRIVATE";
        let prompt=DevicePrompt::from_response(&json!({"user_code":secret,"expires_in":300}))?;
        private_tx.send(prompt).unwrap();
        replies.send(Reply::Log("fixture connection started".into())).unwrap();
        app.poll(&Context::default());
        app.auth_window.poll(app.login.as_ref(),now());
        assert_eq!(app.auth_window.prompt.as_ref().unwrap().code,secret);
        assert!(!app.auth_window.reveal);
        assert!(app.log.iter().any(|line|line.contains("fixture connection started")));
        assert!(app.log.iter().all(|line|!line.contains(secret)));
        assert!(!std::fs::read_to_string(&diagnostics)?.contains(secret));
        drop(private_tx);
        app.auth_window.poll(app.login.as_ref(),now());
        assert!(app.auth_window.prompt.is_none());
        for canceled in [true,false] {
            let (tx,rx)=mpsc::channel();
            let mut window=AuthWindow::new(rx);
            tx.send(DevicePrompt{code:secret.into(),expires:100}).unwrap();
            cancel.store(canceled,Ordering::Relaxed);
            window.poll(Some(&cancel),if canceled{99}else{100});
            assert!(window.prompt.is_none());
        }
        drop(app);
        std::fs::remove_file(diagnostics)?;
        std::fs::remove_file(dir.join("settings.bin"))?;
        std::fs::remove_dir(dir)?;
        Ok(())
    }
}
