use crate::{api::*,profile::*};use serde_json::{json,Value};use anyhow::{Result,ensure};use std::collections::{HashSet,VecDeque};
#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::{BufRead,BufReader,Read,Write},net::TcpListener,time::Duration};

    #[test]
    fn duplicate_vote_and_skip_retries() -> Result<()> {
        let listener=TcpListener::bind(("127.0.0.1",0))?;
        let api=Api::loopback_fixture(listener.local_addr()?.port());
        let track=r#"{"is_playing":true,"item":{"uri":"spotify:track:one","type":"track"}}"#;
        let server=std::thread::spawn(move || -> Result<()> {
            for (path,code,body) in [
                ("GET /spotify/me/player ",200,track),("POST /spotify/me/player/next ",429,""),
                ("GET /spotify/me/player ",200,track),("POST /spotify/me/player/next ",204,""),
                ("POST /twitch/chat/messages ",200,r#"{"data":[{"is_sent":true}]}"#),
                ("GET /spotify/me/player ",200,track),("POST /spotify/me/player/next ",503,""),
            ] {
                let (mut stream,_)=listener.accept()?;
                stream.set_read_timeout(Some(Duration::from_secs(3)))?;
                let mut reader=BufReader::new(&stream);
                let mut first=String::new();reader.read_line(&mut first)?;
                ensure!(first.starts_with(path),"Expected {path}, received {first}");
                let mut length=0;
                loop {
                    let mut header=String::new();ensure!(reader.read_line(&mut header)?>0);
                    if header=="\r\n"{break}
                    if let Some((key,value))=header.split_once(':'){
                        if key.eq_ignore_ascii_case("content-length"){length=value.trim().parse()?}
                    }
                }
                reader.read_exact(&mut vec![0;length])?;
                write!(stream,"HTTP/1.1 {code} Response\r\nContent-Length: {}\r\nRetry-After: 60\r\nConnection: close\r\n\r\n{body}",body.len())?;
            }
            Ok(())
        });
        let mut p=Profile{dir:Default::default(),v:defaults()};
        p.v["broadcaster"]=json!("owner");
        for provider in ["twitch","spotify"]{p.v[provider]=json!({"access_token":"test","expires":now()+3600});}
        let mut player=Player{data:serde_json::from_str(track)?,threshold:2,last_player_read:now(),..Player::default()};
        player.voters.insert("a".into());
        let vote=|id,user| json!({"message_id":id,"chatter_user_id":user,"broadcaster_user_id":"owner","message":{"text":"!skip"}});
        player.command(&api,&mut p,&vote("1","a"))?;
        assert_eq!(player.voters.len(),1);
        assert!(player.command(&api,&mut p,&vote("2","b")).is_err());
        assert!(!player.latched && player.skip_retry_at>now());
        assert!(player.skip(&api,&mut p).is_err());
        player.skip_retry_at=0;
        player.command(&api,&mut p,&vote("3","b"))?;
        assert!(player.latched);
        player.latched=false;player.skip_retry_at=0;
        assert!(player.skip(&api,&mut p).is_err());
        assert!(player.latched);
        server.join().unwrap()
    }
}
#[derive(Default)]
pub struct Player{pub data:Value,pub viewers:Option<i64>,pub threshold:i64,pub voters:HashSet<String>,latched:bool,skip_retry_at:i64,last_player_read:i64,last_viewer_read:i64,observed:String,observed_uri:String,last_song:i64,last_unavailable:i64,seen:VecDeque<String>}
impl Player{
    pub fn uri(&self)->&str{s(&self.data["item"],"uri")}
    fn playing(&self)->bool{b(&self.data,"is_playing")&&self.data["item"]["type"]=="track"}
    pub fn refresh(&mut self,a:&Api,p:&mut Profile)->Result<()>{
        self.last_player_read=now();
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
        self.last_viewer_read=now();
        self.viewers=None;
        let r=a.call(p,false,"GET",&format!("streams?user_id={}",enc(s(&p.v,"broadcaster"))),None)?;self.viewers=Some(match rows(&r)?.first(){None=>0,Some(v)=>v["viewer_count"].as_i64().filter(|n|*n>=0).ok_or_else(||anyhow::anyhow!("Invalid viewer count"))?});Ok(())
    }
    fn submit_skip(&mut self,a:&Api,p:&mut Profile)->Result<()> {
        ensure!(self.playing() && !self.latched,"Нет доступного трека / No track to skip");
        match a.call(p,true,"POST","me/player/next",None) {
            Ok(_) => { self.latched=true; Ok(()) }
            Err(error) => {
                match status(&error) {
                    429 => {
                        let seconds=retry(&error).max(1);
                        self.skip_retry_at=now().saturating_add(seconds);
                        anyhow::bail!("Повтор через {seconds} с / Retry in {seconds} s")
                    }
                    400..=499 => self.voters.clear(),
                    _ => self.latched=true,
                }
                Err(error)
            }
        }
    }
    pub fn skip(&mut self,a:&Api,p:&mut Profile)->Result<()> {
        let wait=self.skip_retry_at.saturating_sub(now());
        ensure!(wait<=0,"Повтор через {wait} с / Retry in {wait} s");
        self.refresh(a,p)?;
        self.submit_skip(a,p)
    }
    pub fn command(&mut self,a:&Api,p:&mut Profile,m:&Value)->Result<()>{
        let cmd=s(&m["message"],"text").trim().to_lowercase();let id=s(m,"message_id");let user=s(m,"chatter_user_id");
        if m["broadcaster_user_id"]!=p.v["broadcaster"]||id.is_empty()||user.is_empty()||!matches!(&*cmd,"!song"|"!nowplaying"|"!skip"|"!voteskip")||self.seen.iter().any(|x|x==id){return Ok(())}
        self.seen.push_back(id.into());if self.seen.len()>1000{self.seen.pop_front();}
        let en=p.v["language"]=="en";if matches!(&*cmd,"!song"|"!nowplaying"){
            if now()-self.last_song<5{return Ok(())}self.last_song=now();self.refresh(a,p)?;
            let item=&self.data["item"];let text=if let Some(track)=self.uri().strip_prefix("spotify:track:"){
                let artists=item["artists"].as_array().into_iter().flatten().map(|v|s(v,"name")).collect::<Vec<_>>().join(", ");let label=format!("{} — {artists}",s(item,"name")).chars().map(|c|if c.is_control(){' '}else{c}).take(350).collect::<String>();
                format!("{} {label} https://open.spotify.com/track/{track}",if en{"Now playing:"}else{"Сейчас играет:"})
            }else{if en{"No current Spotify track"}else{"Текущий трек недоступен"}.into()};return a.chat(p,&text)
        }
        if !b(&p.v,"votingEnabled") || now()<self.skip_retry_at {return Ok(())}
        if now()-self.last_player_read>=3 {self.refresh(a,p)?;}
        if !self.playing()||self.latched{return Ok(())}if self.threshold==0{
            if now()-self.last_viewer_read>=30 {self.count(a,p)?;}let viewers=self.viewers.unwrap_or(0);if viewers<=0{if now()-self.last_unavailable>=10{self.last_unavailable=now();a.chat(p,if en{"No live viewer count"}else{"Нет данных о зрителях эфира"})?}return Ok(())}
            self.threshold=if b(&p.v,"skipAutomatic")||viewers>=100{(viewers+1)/2}else{n(&p.v,"skipManualVotes").clamp(1,99)}
        }
        if !self.voters.insert(user.into()) && (self.voters.len() as i64)<self.threshold{return Ok(())}
        if (self.voters.len() as i64)<self.threshold{return a.chat(p,&format!("{} {}/{}",if en{"Skip votes:"}else{"Голоса за пропуск:"},self.voters.len(),self.threshold))}
        self.skip_retry_at=now()+2;
        let uri=self.uri().to_owned();self.refresh(a,p)?;if self.uri()==uri&&self.playing()&&!self.voters.is_empty(){self.submit_skip(a,p)?;a.chat(p,if en{"Skipping the track"}else{"Пропускаем трек"})?}Ok(())
    }
}
