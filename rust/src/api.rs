use crate::profile::*;
use anyhow::{Result,Context,ensure,bail};
use serde_json::{Value,json};
use std::{sync::{Arc,atomic::{AtomicBool,Ordering}},time::{Duration,Instant},net::TcpListener,io::{Read,Write}};
use url::Url;
use base64::{Engine,engine::general_purpose::URL_SAFE_NO_PAD};
use sha2::{Digest,Sha256};
/// Deliberately neither Debug nor serializable: only the login window receives this.
pub struct DevicePrompt{pub code:String,pub expires:i64}
impl DevicePrompt{
    pub fn from_response(v:&Value)->Result<Self>{
        let code=s(v,"user_code");let ttl=n(v,"expires_in");
        ensure!(!code.is_empty() && ttl>0,"Invalid Twitch login response");
        Ok(Self{code:code.to_owned(),expires:now().saturating_add(ttl)})
    }
}
pub type Cancel=Arc<AtomicBool>;
pub fn check(c:&Cancel)->Result<()>{ensure!(!c.load(Ordering::Relaxed),"Отменено / Canceled");Ok(())}
pub fn wait(c:&Cancel,secs:u64)->Result<()>{for _ in 0..secs*10{check(c)?;std::thread::sleep(Duration::from_millis(100))}check(c)}
pub fn client()->ureq::Agent{ureq::AgentBuilder::new().tls_connector(Arc::new(native_tls::TlsConnector::new().unwrap())).timeout(Duration::from_secs(20)).redirects(0).user_agent(concat!("TrackBridge/",env!("TRACKBRIDGE_VERSION"))).build()}
pub fn enc(x:&str)->String{url::form_urlencoded::byte_serialize(x.as_bytes()).collect()}
pub fn open_url(u:&str)->Result<()>{let url=Url::parse(u)?;ensure!(url.scheme()=="https","HTTPS required");unsafe{
    let result=windows_sys::Win32::UI::Shell::ShellExecuteW(std::ptr::null_mut(),wide("open".as_ref()).as_ptr(),wide(u.as_ref()).as_ptr(),std::ptr::null(),std::ptr::null(),1);
    ensure!(result as usize>32,"Cannot open browser");
}Ok(())}
pub fn status(err:&anyhow::Error)->u16{match err.downcast_ref::<ureq::Error>(){Some(ureq::Error::Status(n,_))=>*n,_=>0}}
pub fn retry(err:&anyhow::Error)->i64{match err.downcast_ref::<ureq::Error>(){Some(ureq::Error::Status(_,r))=>r.header("Retry-After").and_then(|x|x.parse().ok()).unwrap_or(5),_=>5}}
pub fn describe(e:&anyhow::Error)->String{if let Some(err)=e.downcast_ref::<ureq::Error>(){match err{ureq::Error::Status(n,_)=>format!("HTTP {n}"),_=>"Ошибка сети / Network error".into()}}else{e.to_string()}}
pub fn rows(v:&Value)->Result<&Vec<Value>>{let list=v["data"].as_array().context("Invalid API data")?;ensure!(list.iter().all(Value::is_object),"Invalid API rows");Ok(list)}
pub struct Api{pub http:ureq::Agent,fixture_port:Option<u16>}
impl Api{
    pub fn new()->Self{Self{http:client(),fixture_port:None}}
    pub(crate) fn loopback_fixture(port:u16)->Self{Self{http:client(),fixture_port:Some(port)}}
    pub fn access(&self,p:&mut Profile,sp:bool)->Result<String>{let key=if sp{"spotify"}else{"twitch"};
        ensure!(!s(&p.v[key],"access_token").is_empty(),"Подключите аккаунт / Connect {key}");
        if n(&p.v[key],"expires")<now()+90{
            let url=if sp{"https://accounts.spotify.com/api/token"}else{"https://id.twitch.tv/oauth2/token"};
            let val=self.http.post(url).send_form(&[("grant_type","refresh_token"),("refresh_token",s(&p.v[key],"refresh_token")),("client_id",s(&p.v,&format!("{key}Id")))])?.into_json()?;
            self.token(p,key,val)?;
        }
        Ok(s(&p.v[key],"access_token").into())
    }
    fn token(&self,p:&mut Profile,key:&str,v:Value)->Result<()>{ensure!(!s(&v,"access_token").is_empty(),"Token missing");
        p.v[key]["access_token"]=v["access_token"].clone();if v["refresh_token"].is_string(){p.v[key]["refresh_token"]=v["refresh_token"].clone()}
        p.v[key]["expires"]=json!(now()+n(&v,"expires_in"));p.save()
    }
    pub fn call(&self,p:&mut Profile,sp:bool,method:&str,path:&str,body:Option<Value>)->Result<Value>{
        let token=self.access(p,sp)?;let base=if sp{"https://api.spotify.com/v1/"}else{"https://api.twitch.tv/helix/"};
        let url=match self.fixture_port{Some(port)=>format!("http://127.0.0.1:{port}/{}/{path}",if sp{"spotify"}else{"twitch"}),None=>format!("{base}{path}")};
        let mut req=self.http.request(method,&url).set("Authorization",&format!("Bearer {token}"));
        if !sp{req=req.set("Client-Id",s(&p.v,"twitchId"))}
        let resp=if let Some(v)=body{req.send_json(v)}else{req.call()}?;
        let data=resp.into_string()?;if data.trim().is_empty(){Ok(json!({}))}else{Ok(serde_json::from_str(&data)?)}
    }
    pub fn validate(&self,p:&mut Profile)->Result<()>{let t=self.access(p,false)?;let v:Value=self.http.get("https://id.twitch.tv/oauth2/validate").set("Authorization",&format!("OAuth {t}")).call()?.into_json()?;
        ensure!(v["client_id"]==p.v["twitchId"],"Twitch Client ID mismatch");let scopes=v["scopes"].as_array().context("scopes")?;
        p.v["chatAllowed"]=json!(scopes.contains(&json!("user:write:chat")));p.v["chatReadAllowed"]=json!(scopes.contains(&json!("user:read:chat")));p.v["broadcaster"]=v["user_id"].clone();p.save()
    }
    pub fn connect(&self,p:&mut Profile,sp:bool,c:&Cancel,prompt:&std::sync::mpsc::Sender<DevicePrompt>)->Result<()>{
        let id=s(&p.v,if sp{"spotifyId"}else{"twitchId"}).to_owned();ensure!(!id.trim().is_empty(),"Введите Client ID / Enter Client ID");
        if !sp{
            let scopes="channel:read:redemptions channel:manage:redemptions user:write:chat user:read:chat";
            let v:Value=self.http.post("https://id.twitch.tv/oauth2/device").send_form(&[("client_id",id.as_str()),("scopes",scopes)])?.into_json()?;
            prompt.send(DevicePrompt::from_response(&v)?).map_err(|_|anyhow::anyhow!("Отменено / Canceled"))?;open_url(s(&v,"verification_uri"))?;
            let mut interval=n(&v,"interval").max(1) as u64;let end=now()+n(&v,"expires_in");
            while now()<end{wait(c,interval)?;let reply=self.http.post("https://id.twitch.tv/oauth2/token").send_form(&[("client_id",&id),("scopes",scopes),("device_code",s(&v,"device_code")),("grant_type","urn:ietf:params:oauth:grant-type:device_code")]);
                match reply{Ok(r)=>{self.token(p,"twitch",r.into_json()?)?;return self.validate(p)},Err(ureq::Error::Status(_,r))=>{let err:Value=r.into_json()?;match s(&err,"message"){"authorization_pending"=>(),"slow_down"=>interval+=5,_=>bail!("Twitch login failed")}},Err(e)=>return Err(e.into())}
            }bail!("Twitch login expired")
        }
        let listener=TcpListener::bind(("127.0.0.1",43821))?;listener.set_nonblocking(true)?;
        let random=||URL_SAFE_NO_PAD.encode(rand::random::<[u8;32]>());let state=random();let verifier=random();
        let redirect="http://127.0.0.1:43821/callback";let mut url=Url::parse("https://accounts.spotify.com/authorize")?;
        url.query_pairs_mut().extend_pairs([("client_id",id.as_str()),("response_type","code"),("redirect_uri",redirect),("scope","user-modify-playback-state user-read-playback-state"),("state",&state),("code_challenge_method","S256"),("code_challenge",&URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())))]);
        open_url(url.as_str())?;let end=Instant::now()+Duration::from_secs(240);
        while Instant::now()<end{check(c)?;match listener.accept(){
            Ok((mut socket,_))=>{socket.set_read_timeout(Some(Duration::from_millis(500)))?;let start=Instant::now();let mut line=Vec::new();let mut byte=[0];while line.len()<8192&&start.elapsed()<Duration::from_secs(5){check(c)?;if socket.read(&mut byte).unwrap_or(0)!=1||byte[0]==10{break}line.push(byte[0]);}
                let line=String::from_utf8_lossy(&line);let parts:Vec<_>=line.split_whitespace().collect();let callback=parts.get(1).and_then(|path|Url::parse(&format!("http://127.0.0.1{path}")).ok());
                let valid=parts.len()==3&&parts[0]=="GET"&&parts[2].starts_with("HTTP/1.")&&callback.as_ref().is_some_and(|u|u.path()=="/callback"&&u.query_pairs().any(|(k,v)|k=="state"&&v==state));
                let reply=if valid{"200 OK"}else{"400 Bad Request"};let _=write!(socket,"HTTP/1.1 {reply}\r\nConnection: close\r\n\r\nTrackBridge: {reply}");if !valid{continue}
                let code=callback.unwrap().query_pairs().find(|(k,_)|k=="code").map(|(_,v)|v.into_owned()).context("Spotify login canceled")?;
                let data=self.http.post("https://accounts.spotify.com/api/token").send_form(&[("client_id",&id),("grant_type","authorization_code"),("code",&code),("redirect_uri",redirect),("code_verifier",&verifier)])?.into_json()?;return self.token(p,"spotify",data)
            },Err(e) if e.kind()==std::io::ErrorKind::WouldBlock=>std::thread::sleep(Duration::from_millis(100)),Err(e)=>return Err(e.into())
        }}bail!("Spotify login expired")
    }
    pub fn rewards(&self,p:&mut Profile)->Result<Vec<Value>>{self.validate(p)?;let path=format!("channel_points/custom_rewards?broadcaster_id={}&only_manageable_rewards=true",enc(s(&p.v,"broadcaster")));Ok(rows(&self.call(p,false,"GET",&path,None)?)?.clone())}
    pub fn mark(&self,p:&mut Profile,o:&Value,success:bool)->Result<()>{
        ensure!(o["broadcaster"]==p.v["broadcaster"],"Connect original Twitch account");
        let path=format!("channel_points/custom_rewards/redemptions?broadcaster_id={}&reward_id={}&id={}",enc(s(o,"broadcaster")),enc(s(o,"reward")),enc(s(o,"id")));
        self.call(p,false,"PATCH",&path,Some(json!({"status":if success{"FULFILLED"}else{"CANCELED"}})))?;Ok(())
    }
    pub fn chat(&self,p:&mut Profile,text:&str)->Result<()>{
        let body=json!({"broadcaster_id":p.v["broadcaster"],"sender_id":p.v["broadcaster"],"message":text});
        let r=self.call(p,false,"POST","chat/messages",Some(body))?;ensure!(rows(&r)?.first().is_some_and(|v|b(v,"is_sent")),"Chat message not delivered");Ok(())
    }
}
