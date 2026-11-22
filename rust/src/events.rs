use crate::api::{Cancel,check,wait};
use anyhow::{Result,Context,ensure,bail};
use serde_json::Value;
use std::{net::TcpStream,sync::mpsc::SyncSender,time::Duration};
use tungstenite::{Message,WebSocket,stream::MaybeTlsStream};
use url::Url;
use crate::profile::s;
pub enum Event{Welcome(String),Live,Purchase(Value),Message(Value),Lost(String)}
fn connect(url:&str)->Result<WebSocket<MaybeTlsStream<TcpStream>>>{
    let uri=Url::parse(url)?;ensure!(uri.scheme()=="wss"&&uri.host_str()==Some("eventsub.wss.twitch.tv")&&uri.port().is_none()&&uri.username().is_empty(),"Unexpected Twitch URL");
    use std::net::ToSocketAddrs;
    let mut socket=None;for addr in ("eventsub.wss.twitch.tv",443).to_socket_addrs()?{if let Ok(s)=TcpStream::connect_timeout(&addr,Duration::from_secs(8)){socket=Some(s);break}}
    let socket=socket.context("Twitch connection failed")?;socket.set_read_timeout(Some(Duration::from_secs(12)))?;socket.set_write_timeout(Some(Duration::from_secs(10)))?;
    let cfg=tungstenite::protocol::WebSocketConfig{max_message_size:Some(1<<20),..Default::default()};
    Ok(tungstenite::client_tls_with_config(url,socket,Some(cfg),None)?.0)
}
fn read(ws:&mut WebSocket<MaybeTlsStream<TcpStream>>,c:&Cancel)->Result<Value>{loop{check(c)?;match ws.read()?{Message::Text(t)=>return Ok(serde_json::from_str(&t)?),Message::Close(_)=>bail!("Twitch disconnected"),_=>()}}}
fn welcome(ws:&mut WebSocket<MaybeTlsStream<TcpStream>>,c:&Cancel)->Result<String>{
    let m=read(ws,c)?;ensure!(m["metadata"]["message_type"]=="session_welcome","Missing welcome");let session=&m["payload"]["session"];let timeout=session["keepalive_timeout_seconds"].as_u64().unwrap_or(25)+5;
    if let MaybeTlsStream::NativeTls(t)=ws.get_mut(){t.get_ref().set_read_timeout(Some(Duration::from_secs(timeout)))?}
    Ok(s(session,"id").into())
}
fn session(c:&Cancel,tx:&SyncSender<Event>)->Result<()>{
    let mut ws=connect("wss://eventsub.wss.twitch.tv/ws")?;let id=welcome(&mut ws,c)?;tx.send(Event::Welcome(id))?;
    loop{let m=read(&mut ws,c)?;let p=&m["payload"];match s(&m["metadata"],"message_type"){
        "notification"=>match s(&p["subscription"],"type"){"channel.chat.message"=>tx.send(Event::Message(p["event"].clone()))?,"channel.channel_points_custom_reward_redemption.add"=>tx.send(Event::Purchase(p["event"].clone()))?,_=>()},
        "session_reconnect"=>{let mut next=connect(s(&p["session"],"reconnect_url"))?;welcome(&mut next,c)?;ws=next;tx.send(Event::Live)?},
        "revocation"=>bail!("Twitch subscription revoked"),_=>()
    }}
}
pub fn run(c:Cancel,tx:SyncSender<Event>){while check(&c).is_ok(){if let Err(e)=session(&c,&tx){if tx.send(Event::Lost(e.to_string())).is_err(){break}}if wait(&c,5).is_err(){break}}}
