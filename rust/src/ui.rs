use crate::{api::{Cancel,DevicePrompt,open_url},profile::*,runtime::{Cmd,View,Reply},update};
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
pub struct Desktop{v:View,tx:Sender<Cmd>,rx:Receiver<Reply>,exit:Cancel,join:Option<std::thread::JoinHandle<()>>,pending:usize,login:Option<Cancel>,auth_window:AuthWindow,page:usize,lang:String,ids:[String;2],rules:[String;2],minutes:i64,choice_minutes:i64,explicit:bool,title:String,cost:i64,selected:HashSet<String>,review:bool,choose:Option<(String,String)>,log:VecDeque<String>,scroll:bool,root:PathBuf,tray:Option<(TrayIcon,MenuItem,MenuItem)>,update_rx:Option<Receiver<anyhow::Result<Option<PathBuf>>>>,stage:Option<PathBuf>,next_update:i64,preview:bool,shot:Option<PathBuf>,frames:u32,art_url:String,art:Option<TextureHandle>,art_rx:Option<Receiver<(String,Option<ColorImage>)>>,diag:Option<PathBuf>}
impl Desktop{
    pub fn new(v:View,tx:Sender<Cmd>,rx:Receiver<Reply>,exit:Cancel,join:std::thread::JoinHandle<()>,root:PathBuf,preview:bool,shot:Option<PathBuf>,settings:bool,diag:Option<PathBuf>)->Self{
        let lang=s(&v.prefs,"language").to_string();let en=lang=="en";let tray=(||{let menu=Menu::new();let open=MenuItem::new(if en{"Open"}else{"Открыть"},true,None);let quit=MenuItem::new(if en{"Exit"}else{"Выйти"},true,None);menu.append(&open).ok()?;menu.append(&quit).ok()?;let img=crate::icon();let icon=tray_icon::Icon::from_rgba(img.rgba,img.width,img.height).ok()?;Some((TrayIconBuilder::new().with_menu(Box::new(menu)).with_icon(icon).with_tooltip("TrackBridge").build().ok()?,open,quit))})();
        Self{ids:[s(&v.prefs,"twitchId").into(),s(&v.prefs,"spotifyId").into()],rules:[s(&v.prefs,"blockedTracks").into(),s(&v.prefs,"blockedArtists").into()],minutes:n(&v.prefs,"maxTrackMinutes"),choice_minutes:n(&v.prefs,"trackChoiceMinutes"),explicit:b(&v.prefs,"blockExplicit"),v,tx,rx,exit,join:Some(join),pending:0,login:None,auth_window:AuthWindow::default(),page:if settings{4}else{0},lang,title:if en{"Request a song"}else{"Заказать трек"}.into(),cost:500,selected:HashSet::new(),review:false,choose:None,log:VecDeque::new(),scroll:true,root,tray,update_rx:None,stage:None,next_update:0,preview,shot,frames:0,art_url:String::new(),art:None,art_rx:None,diag}
    }
    fn t(&self,ru:&'static str,en:&'static str)->&'static str{if self.lang=="en"{en}else{ru}}
    fn send(&mut self,c:Cmd){if self.tx.send(c).is_ok(){self.pending+=1}}
    fn set(&mut self,k:&str,v:Value){self.v.prefs[k]=v.clone();self.send(Cmd::Set(json!({(k):v})))}
    fn record(&mut self,text:String){let text=crate::language::message(&self.lang,&text);let entry=format!("{}  {text}",crate::timestamp());if let Some(path)=&self.diag{use std::io::Write;if let Ok(mut f)=std::fs::OpenOptions::new().create(true).append(true).open(path){let _=writeln!(f,"{entry}");}}self.log.push_back(entry);if self.log.len()>2000{self.log.pop_front();}}
    fn ids(&mut self){self.send(Cmd::Set(json!({"twitchId":self.ids[0].trim(),"spotifyId":self.ids[1].trim()})))}
    fn check_updates(&mut self){if self.update_rx.is_some()||self.preview{return}let(tx,rx)=mpsc::channel();let d=self.root.clone();std::thread::spawn(move||{let _=tx.send(update::prepare(&d));});self.update_rx=Some(rx);self.next_update=now()+21600}
    fn poll(&mut self,ctx:&Context){
        while let Ok(reply)=self.rx.try_recv(){match reply{Reply::Log(msg)=>self.record(msg),Reply::Ready=>{self.pending=self.pending.saturating_sub(1);if self.pending==0{self.login=None}},Reply::State(v)=>self.v=v}}
        if self.stage.is_none()&&now()>=self.next_update{self.check_updates()}
        if let Some(result)=self.update_rx.as_ref().and_then(|rx|rx.try_recv().ok()){self.update_rx=None;match result{Ok(stage)=>{self.stage=stage;self.record(if self.stage.is_some(){self.t("Обновление готово","Update ready")}else{self.t("Установлена актуальная версия","Up to date")}.into())},Err(e)=>self.record(format!("{} · {}",self.t("Обновления","Updates"),crate::language::message(&self.lang,&crate::api::describe(&e))))}}
        if let Some((_,open,quit))=&self.tray{while let Ok(m)=MenuEvent::receiver().try_recv(){if m.id==*quit.id(){ctx.send_viewport_cmd(ViewportCommand::Close)}else if m.id==*open.id(){ctx.send_viewport_cmd(ViewportCommand::Visible(true));ctx.send_viewport_cmd(ViewportCommand::Minimized(false));ctx.send_viewport_cmd(ViewportCommand::Focus)}}}
        while let Ok(m)=TrayIconEvent::receiver().try_recv(){if matches!(m,TrayIconEvent::DoubleClick{..}){ctx.send_viewport_cmd(ViewportCommand::Visible(true));ctx.send_viewport_cmd(ViewportCommand::Minimized(false));ctx.send_viewport_cmd(ViewportCommand::Focus)}}
        let url=self.v.player["item"]["album"]["images"][0]["url"].as_str().unwrap_or("");if url!=self.art_url{self.art_url=url.into();self.art=None;self.art_rx=None;if !self.preview&&url.starts_with("https://"){let url=url.to_owned();let(tx,rx)=mpsc::channel();self.art_rx=Some(rx);std::thread::spawn(move||{use std::io::Read;let img=(||{let r=crate::api::client().get(&url).call().ok()?;let mut bytes=vec![];r.into_reader().take(4<<20).read_to_end(&mut bytes).ok()?;let img=image::load_from_memory(&bytes).ok()?.thumbnail(256,256).to_rgba8();Some(ColorImage::from_rgba_unmultiplied([img.width()as _,img.height()as _],img.as_raw()))})();let _=tx.send((url,img));});}}
        if let Some((u,img))=self.art_rx.as_ref().and_then(|rx|rx.try_recv().ok()){self.art_rx=None;if u==self.art_url{self.art=img.map(|v|ctx.load_texture("cover",v,Default::default()))}}
    }
    fn overview(&mut self,ui:&mut Ui){
        ui.horizontal(|ui|{if ui.add_enabled(self.pending==0,Button::new(if self.v.running{self.t("Остановить","Stop")}else{self.t("Запустить","Start")})).clicked(){self.send(if self.v.running{Cmd::Stop}else{Cmd::Start})}if ui.add_enabled(self.pending==0,Button::new(self.t("Проверить плеер","Check player"))).clicked(){self.send(Cmd::Check)}ui.label(&self.v.device);});ui.add_space(16.);
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
    fn table(&mut self,ui:&mut Ui,recent:bool){
        let orders=self.v.orders.clone();let mut resolve=None;
        if !recent{ui.horizontal(|ui|{ui.selectable_value(&mut self.review,false,if self.lang=="en"{"History"}else{"История"});ui.selectable_value(&mut self.review,true,if self.lang=="en"{"Needs review"}else{"Требуют решения"});});}
        ui.horizontal(|ui|{if ui.add_enabled(!self.selected.is_empty(),Button::new(self.t("Скрыть выбранные","Hide selected"))).clicked(){let ids=self.selected.drain().collect();self.send(Cmd::Hide(ids))}if ui.button(self.t("Очистить сыгранные","Clear played")).clicked(){self.send(Cmd::Clean)}});
        ScrollArea::both().id_salt(("orders",recent)).show(ui,|ui|{Grid::new(("rows",recent)).striped(true).spacing([14.,10.]).show(ui,|ui|{for h in ["",self.t("Зритель","Viewer"),self.t("Трек","Track"),self.t("Статус","Status"),self.t("Время","Time")]{ui.strong(h);}ui.end_row();
            let mut count=0;for o in orders.iter().rev(){let review=needs_review(o);if !recent&&self.review{if !review{continue}}else if b(o,"historyHidden"){continue}count+=1;let id=s(o,"id");let mut sel=self.selected.contains(id);if ui.checkbox(&mut sel,"").changed(){if sel{self.selected.insert(id.into());}else{self.selected.remove(id);}}ui.label(s(o,"user"));ui.add(Label::new(if s(o,"trackTitle").is_empty(){s(o,"input")}else{s(o,"trackTitle")}).truncate()).on_hover_text(s(o,"input"));let status=match s(o,"status"){"done" if b(o,"played")=>self.t("Сыгран","Played"),"done"=>self.t("В очереди Spotify","In Spotify queue"),"canceled"=>self.t("Баллы возвращены","Refunded"),"pending"=>self.t("Ожидание","Pending"),"queued"=>self.t("Подтверждение","Confirming"),"reject"=>self.t("Возврат","Refunding"),"sending"=>self.t("Отправка","Sending"),_=>self.t("Требует решения","Needs review")};ui.label(status).on_hover_text(crate::language::message(&self.lang,s(o,"note")).as_ref());ui.label(s(o,"time"));
                if o["status"]=="needs_track" && n(&self.v.prefs,"trackChoiceMinutes")>0{
                    let seconds=(n(o,"choiceSince").saturating_add(n(&self.v.prefs,"trackChoiceMinutes")*60)-now()).max(0);
                    ui.label(format!("{} {}:{:02}",self.t("До возврата","Refund in"),seconds/60,seconds%60));
                }
                if o["status"]=="needs_track" && ui.small_button(self.t("Указать трек","Choose track")).clicked(){self.choose=Some((id.into(),String::new()));}
                if review{for(added,label)in [(true,self.t("Уже добавлен","Already added")),(false,self.t("Вернуть баллы","Refund"))]{if (!added || (o["status"]!="needs_track" && !b(o,"choiceTimedOut"))) && ui.small_button(label).clicked(){resolve=Some((id.to_owned(),added))}}}ui.end_row();if recent&&count>=8{break}}
            if count==0{ui.label("—");ui.label(self.t("Заказов пока нет","No requests yet"));ui.end_row();}
        });});
        if let Some((id,mut raw))=self.choose.take().filter(|(id,_)|self.v.orders.iter().any(|o|s(o,"id")==id && o["status"]=="needs_track")){
            let mut open=true;let mut submit=false;
            Window::new(self.t("Трек для заказа","Track for request")).open(&mut open).collapsible(false).show(ui.ctx(),|ui|{
                ui.label(self.t("Вставьте ссылку Spotify. Перед добавлением действуют обычные правила модерации.","Paste a Spotify track link. The usual moderation rules apply."));
                ui.text_edit_singleline(&mut raw);
                if ui.add_enabled(crate::orders::spotify(&raw).is_some(),Button::new(self.t("Передать в очередь заказов","Submit for processing"))).clicked(){submit=true}
            });
            if submit{self.send(Cmd::ChooseTrack(id,raw))}else if open{self.choose=Some((id,raw))}
        }
        if let Some((id,added))=resolve{if confirm(if added{self.t("Трек уже в Spotify? Подтвердить покупку?","Track already in Spotify? Fulfill redemption?")}else{self.t("Вернуть баллы зрителю?","Refund this viewer?")}){self.send(Cmd::Resolve(id,added))}}
    }
    fn moderation(&mut self,ui:&mut Ui){ui.add_enabled_ui(!self.v.running,|ui|{
        ui.horizontal(|ui|{ui.label(self.t("Лимит минут (0 — без лимита)","Limit in minutes (0 for unlimited)"));ui.add(DragValue::new(&mut self.minutes).range(0..=1440));});let label=self.t("Запретить Explicit","Block explicit tracks");ui.checkbox(&mut self.explicit,label);ui.add_space(10.);
        ui.columns(2,|cols|{for(i,col)in cols.iter_mut().enumerate(){col.strong(if i==0{self.t("Запрещённые треки","Blocked tracks")}else{self.t("Запрещённые исполнители","Blocked artists")});col.label(self.t("По одному на строку","One per line"));col.add(TextEdit::multiline(&mut self.rules[i]).desired_rows(12).desired_width(f32::INFINITY));}});
        if ui.button(self.t("Сохранить правила","Save rules")).clicked(){self.send(Cmd::Set(json!({"maxTrackMinutes":self.minutes,"blockExplicit":self.explicit,"blockedTracks":self.rules[0],"blockedArtists":self.rules[1]})));}
    });}
    fn settings(&mut self,ui:&mut Ui){let en=self.lang=="en";ui.horizontal(|ui|{ui.label(self.t("Язык","Language"));for(code,label)in [("ru","Русский"),("en","English")]{if ui.selectable_label(self.lang==code,label).clicked(){self.lang=code.into();self.set("language",json!(code));if let Some((_,open,quit))=&self.tray{open.set_text(self.t("Открыть","Open"));quit.set_text(self.t("Выход","Exit"));}}}});
        ui.separator();ui.add_enabled_ui(!self.v.running&&self.pending==0,|ui|{for(i,name)in ["Twitch","Spotify"].into_iter().enumerate(){ui.horizontal(|ui|{ui.label(format!("{name} Client ID"));ui.text_edit_singleline(&mut self.ids[i]);if ui.button(self.t("Подключить","Connect")).clicked(){self.ids();let c=Arc::new(AtomicBool::new(false));self.login=Some(c.clone());let (tx,rx)=mpsc::channel();self.auth_window=AuthWindow::new(rx);self.send(Cmd::Login(i==1,c,tx))}ui.label(if self.v.account[i]{self.t("Подключён","Connected")}else{"—"});});}
        if ui.button(self.t("Сохранить Client ID","Save Client IDs")).clicked(){self.ids()}
        ui.separator();ui.horizontal(|ui|{let rewards=self.v.rewards.clone();let current=s(&self.v.prefs,"reward");let label=rewards.iter().find(|r|s(r,"id")==current).map(|r|s(r,"title")).unwrap_or(self.t("Выберите награду","Select reward"));ComboBox::from_id_salt("reward").selected_text(label).show_ui(ui,|ui|{for r in &rewards{if ui.selectable_label(self.v.prefs["reward"]==r["id"],s(r,"title")).clicked(){self.set("reward",r["id"].clone())}}});if ui.button(self.t("Обновить список","Refresh rewards")).clicked(){self.ids();self.send(Cmd::Rewards)}});ui.horizontal(|ui|{ui.text_edit_singleline(&mut self.title);ui.add(DragValue::new(&mut self.cost).range(1..=1_000_000_000));if ui.button(self.t("Создать награду","Create reward")).clicked(){self.send(Cmd::Create(self.title.clone(),self.cost))}});
        });if let Some(c)=&self.login{if ui.button(self.t("Отменить подключение","Cancel connection")).clicked(){c.store(true,Ordering::Relaxed)}}
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
        SidePanel::left("sidebar").resizable(false).exact_width(185.).frame(Frame::side_top_panel(&ctx.style()).inner_margin(16)).show(ctx,|ui|{ui.add_space(12.);ui.heading(RichText::new("TrackBridge").color(GREEN));ui.add_space(20.);for(i,title)in pages.iter().enumerate(){let r=ui.add_sized([149.,38.],Button::new(*title).selected(i==self.page));if r.clicked(){self.page=i;self.record(format!("· {title}"));}if self.page==i{let y=ctx.animate_value_with_time(Id::new("nav-marker"),r.rect.center().y,ctx.style().animation_time);ui.painter().rect_filled(Rect::from_center_size(pos2(r.rect.left()-5.,y),vec2(3.,24.)),2.,GREEN);}}ui.with_layout(Layout::bottom_up(Align::LEFT),|ui|{ui.small(format!("v{}",update::CURRENT));});});
        egui::TopBottomPanel::bottom("status").show(ctx,|ui|{ui.horizontal(|ui|{ui.colored_label(if self.v.connected{GREEN}else{Color32::GRAY},if self.v.connected{self.t("● Приём заказов активен","● Receiving requests")}else if self.v.running{self.t("Подключение · резервный опрос активен","Connecting · fallback polling active")}else{self.t("Приём остановлен","Reception stopped")});if self.pending>0{ui.spinner();}if let Some(last)=self.log.back(){ui.separator();ui.add(Label::new(last).truncate()).on_hover_text(last);}});});
        CentralPanel::default().frame(Frame::central_panel(&ctx.style()).inner_margin(22)).show(ctx,|ui|{ui.heading(pages[self.page]);ui.add_space(14.);match self.page{0=>self.overview(ui),1=>self.table(ui,false),2=>self.moderation(ui),3=>self.journal(ui),4=>{ScrollArea::vertical().show(ui,|ui|self.settings(ui));},_=>self.about(ui)}});
        self.frames+=1;if let Some(path)=&self.shot{if self.frames==4{ctx.send_viewport_cmd(ViewportCommand::Screenshot(Default::default()));}if let Some(image)=ctx.input(|i|i.events.iter().find_map(|event|if let egui::Event::Screenshot{image,..}=event{Some(image.clone())}else{None})){let bytes:Vec<_>=image.pixels.iter().flat_map(|c|c.to_array()).collect();let _=image::save_buffer(path,&bytes,image.width()as _,image.height()as _,image::ColorType::Rgba8);ctx.send_viewport_cmd(ViewportCommand::Close);}}
    }
}
impl Drop for Desktop{fn drop(&mut self){if let Some(c)=&self.login{c.store(true,Ordering::Relaxed)}self.exit.store(true,Ordering::Relaxed);let _=self.tx.send(Cmd::Quit);if let Some(h)=self.join.take(){let _=h.join();}}}

#[cfg(test)]
mod privacy_tests {
    use super::*;
    use crate::runtime::Engine;

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
