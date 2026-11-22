use crate::{api::*,profile::*};use serde_json::{json,Value};use anyhow::{Result,ensure};use std::collections::{HashSet,VecDeque};
#[derive(Default)]
pub struct Player{pub data:Value,pub viewers:Option<i64>,pub threshold:i64,pub voters:HashSet<String>,latched:bool,observed:String,observed_uri:String,last_song:i64,last_unavailable:i64,seen:VecDeque<String>}
impl Player{
    pub fn uri(&self)->&str{s(&self.data["item"],"uri")}
    fn playing(&self)->bool{b(&self.data,"is_playing")&&self.data["item"]["type"]=="track"}
    pub fn refresh(&mut self,a:&Api,p:&mut Profile)->Result<()>{
        let data=a.call(p,true,"GET","me/player",None)?;let uri=s(&data["item"],"uri");let restart=self.uri()==uri&&n(&data,"progress_ms")+3000<n(&self.data,"progress_ms");
        if self.uri()!=uri||restart{self.voters.clear();self.threshold=0;self.latched=false}
        if !uri.is_empty(){
            let mut dirty=false;
            if !self.observed.is_empty()&&(uri!=self.observed_uri||restart){
                if let Some(o)=p.v["orders"].as_array_mut().unwrap().iter_mut().find(|o|s(o,"id")==self.observed){o["played"]=json!(true);dirty=true}
                self.observed.clear();
            }
            if b(&data,"is_playing")&&self.observed.is_empty(){if let Some(o)=p.v["orders"].as_array().unwrap().iter().find(|o|s(o,"uri")==uri&&!b(o,"played")&&!b(o,"historyHidden")&&matches!(s(o,"status"),"done"|"queued")){self.observed=s(o,"id").into();self.observed_uri=uri.into()}}
            if dirty{p.save()?}
        }self.data=data;if b(&p.v,"autoHidePlayed"){self.clean(p)?}Ok(())
    }
    pub fn clean(&self,p:&mut Profile)->Result<()>{
        let mut dirty=false;for o in p.v["orders"].as_array_mut().unwrap(){if o["status"]=="done"&&b(o,"played")&&!b(o,"historyHidden")&&s(o,"id")!=self.observed&&s(o,"uri")!=self.uri(){o["historyHidden"]=json!(true);dirty=true}}if dirty{p.save()?}Ok(())
    }
    pub fn count(&mut self,a:&Api,p:&mut Profile)->Result<()>{
        let r=a.call(p,false,"GET",&format!("streams?user_id={}",enc(s(&p.v,"broadcaster"))),None)?;self.viewers=Some(match rows(&r)?.first(){None=>0,Some(v)=>v["viewer_count"].as_i64().filter(|n|*n>=0).ok_or_else(||anyhow::anyhow!("Invalid viewer count"))?});Ok(())
    }
    pub fn skip(&mut self,a:&Api,p:&mut Profile)->Result<()>{self.refresh(a,p)?;ensure!(self.playing()&&!self.latched,"Нет доступного трека / No track to skip");self.latched=true;a.call(p,true,"POST","me/player/next",None)?;Ok(())}
    pub fn command(&mut self,a:&Api,p:&mut Profile,m:&Value)->Result<()>{
        let cmd=s(&m["message"],"text").trim().to_lowercase();let id=s(m,"message_id");let user=s(m,"chatter_user_id");
        if m["broadcaster_user_id"]!=p.v["broadcaster"]||id.is_empty()||user.is_empty()||!matches!(&*cmd,"!song"|"!nowplaying"|"!skip"|"!voteskip")||self.seen.iter().any(|x|x==id){return Ok(())}
        self.seen.push_back(id.into());if self.seen.len()>1000{self.seen.pop_front();}
        let en=p.v["language"]=="en";if matches!(&*cmd,"!song"|"!nowplaying"){
            if now()-self.last_song<5{return Ok(())}self.refresh(a,p)?;self.last_song=now();
            let item=&self.data["item"];let text=if let Some(track)=self.uri().strip_prefix("spotify:track:"){
                let artists=item["artists"].as_array().into_iter().flatten().map(|v|s(v,"name")).collect::<Vec<_>>().join(", ");let label=format!("{} — {artists}",s(item,"name")).chars().map(|c|if c.is_control(){' '}else{c}).take(350).collect::<String>();
                format!("{} {label} https://open.spotify.com/track/{track}",if en{"Now playing:"}else{"Сейчас играет:"})
            }else{if en{"No current Spotify track"}else{"Текущий трек недоступен"}.into()};return a.chat(p,&text)
        }
        if !b(&p.v,"votingEnabled"){return Ok(())}self.refresh(a,p)?;if !self.playing()||self.latched{return Ok(())}if self.threshold==0{
            self.count(a,p)?;let viewers=self.viewers.unwrap_or(0);if viewers<=0{if now()-self.last_unavailable>=10{self.last_unavailable=now();a.chat(p,if en{"No live viewer count"}else{"Нет данных о зрителях эфира"})?}return Ok(())}
            self.threshold=if b(&p.v,"skipAutomatic")||viewers>=100{(viewers+1)/2}else{n(&p.v,"skipManualVotes").clamp(1,99)}
        }
        if !self.voters.insert(user.into()){return Ok(())}
        if (self.voters.len() as i64)<self.threshold{return a.chat(p,&format!("{} {}/{}",if en{"Skip votes:"}else{"Голоса за пропуск:"},self.voters.len(),self.threshold))}
        let uri=self.uri().to_owned();self.refresh(a,p)?;if self.uri()==uri&&self.playing()&&!self.voters.is_empty(){self.latched=true;a.call(p,true,"POST","me/player/next",None)?;a.chat(p,if en{"Skipping the track"}else{"Пропускаем трек"})?}Ok(())
    }
}
