use crate::{orders::{spotify,youtube},profile::*,update};use anyhow::{ensure,Result};use serde_json::json;
pub fn run()->Result<usize>{
    let mut n=0;macro_rules! check{($e:expr)=>{{ensure!($e,"Check failed: {}",stringify!($e));n+=1;}}}
    let id="4uLU6hMCjMI75M1A2tKUQC";for raw in [format!("spotify:track:{id}"),format!("https://open.spotify.com/track/{id}?si=1"),format!("https://open.spotify.com/intl-ru/track/{id}")]{check!(spotify(&raw)==Some(format!("spotify:track:{id}")))}
    for raw in ["https://open.spotify.com.evil/track/test","https://open.spotify.com/album/test","https://spotify.link/abc","spotify:track:1"]{check!(spotify(raw).is_none())}
    let video="dQw4w9WgXcQ";for raw in [format!("https://music.youtube.com/watch?v={video}"),format!("https://youtu.be/{video}"),format!("https://youtube.com/shorts/{video}")]{check!(youtube(&raw).as_deref()==Some(video))}
    check!(youtube("https://youtu.be:8080/dQw4w9WgXcQ").is_none());check!(update::version("v1.13")==update::version("1.13.0"));check!(update::version("1.13")>update::version("1.12.0.3"));check!(update::version("1.12.0.1")>update::version("1.12.0"));check!(update::version("1.12.0")>update::version("1.12.0-beta.4"));
    let dir=std::env::temp_dir().join(format!("TrackBridge-test-{:x}",rand::random::<u128>()));let mut p=Profile::open(dir.clone())?;p.v["reward"]=json!("r");p.v["broadcaster"]=json!("b");
    let event=|id:&str|json!({"id":id,"user_name":"viewer","reward":{"id":"r"},"user_input":"opaque fixture"});
    check!(p.accept(&event("one"))?);check!(!p.accept(&event("one"))?);for id in ["two","three","four"]{p.accept(&event(id))?;}check!(p.v["orders"][3]["status"]=="reject");
    p.v["orders"][0]["spotifyRetryAt"]=json!(now()+600);check!(p.next(now())==Some(3));p.v["orders"][3]["status"]=json!("canceled");p.v["orders"][3]["notificationAttempted"]=json!(true);check!(p.next(now()).is_none());
    p.v["orders"][0]["status"]=json!("sending");p.v["orders"][1]["status"]=json!("queued");p.save()?;let mut p=Profile::open(dir.clone())?;check!(p.v["orders"][0]["status"]=="uncertain");check!(p.next(now())==Some(1));check!(!p.accept(&event("one"))?);
    let bytes=std::fs::read(dir.join("settings.bin"))?;check!(!bytes.windows(7).any(|v|v==b"fixture"));
    n+=retry_after_restart(&dir,false)?;
    n+=retry_after_restart(&dir,true)?;
    n+=crate::orders::check_matches()?;
    check!(crate::language::message("ru","Ошибка сети / Network error")=="Ошибка сети");
    check!(crate::language::message("en","Трек запрещён / Blocked track")=="Blocked track");
    check!(needs_review(&json!({"status":"uncertain","historyHidden":true})));
    check!(!needs_review(&json!({"status":"done"})));
    std::fs::write(dir.join("settings.bin"),b"broken")?;check!(Profile::open(dir.clone()).is_err());check!(std::fs::read(dir.join("settings.bin"))?==b"broken");
    std::fs::remove_file(dir.join("settings.bin"))?;std::fs::remove_dir(dir)?;Ok(n)
}
#[test]fn offline_checks(){run().unwrap();}


/// Exercise the production HTTP, queue, Twitch confirmation, DPAPI and restart paths.
fn retry_after_restart(dir:&std::path::Path,timed_refund:bool)->Result<usize>{
    use std::{io::{BufRead,BufReader,Read,Write},net::TcpListener,time::{Duration,Instant}};
    use crate::{api::Api,orders};
    let fixture=dir.join("retry");
    let mut p=Profile::open(fixture.clone())?;
    p.v["reward"]=json!("reward");p.v["broadcaster"]=json!("channel");
    p.v["maxTrackMinutes"]=json!(0);p.v["requestSoundEnabled"]=json!(false);
    for service in ["spotify","twitch"]{p.v[service]=json!({"access_token":"fixture","expires":now()+3600});}
    let event=json!({"id":"redemption","reward":{"id":"reward"},"user_name":"fixture","user_input":"spotify:track:4uLU6hMCjMI75M1A2tKUQC"});
    ensure!(p.accept(&event)?);
    if timed_refund{
        p.v["orders"][0]["status"]=json!("needs_track");
        p.v["orders"][0]["choiceSince"]=json!(1000);
        ensure!(!p.expire_track_choices(1899)?);
        ensure!(p.expire_track_choices(1900)?);
        ensure!(p.due_choice_refund(1900)==Some(0));
    }
    let listener=TcpListener::bind(("127.0.0.1",0))?;
    listener.set_nonblocking(true)?;
    let api=Api::loopback_fixture(listener.local_addr()?.port());
    let server=std::thread::spawn(move||->Result<(usize,usize)>{
        let mut adds=0;let mut confirmations=0;
        let deadline=Instant::now()+Duration::from_secs(8);
        let mut chats=0;
        while confirmations<2 || (timed_refund && chats==0){
            ensure!(Instant::now()<deadline,"Missing Twitch retry");
            let (mut socket,_)=match listener.accept(){Ok(c)=>c,Err(e) if e.kind()==std::io::ErrorKind::WouldBlock=>{std::thread::sleep(Duration::from_millis(5));continue},Err(e)=>return Err(e.into())};
            socket.set_nonblocking(false)?;
            socket.set_read_timeout(Some(Duration::from_secs(2)))?;
            let mut reader=BufReader::new(&socket);let mut first=String::new();reader.read_line(&mut first)?;
            let mut length=0;
            loop{let mut line=String::new();ensure!(reader.read_line(&mut line)?>0);if line=="\r\n"{break}
                if let Some((k,v))=line.split_once(':'){if k.eq_ignore_ascii_case("content-length"){length=v.trim().parse()?}}}
            let mut body=vec![0;length];reader.read_exact(&mut body)?;
            let status=if first.starts_with("POST /spotify/me/player/queue?"){adds+=1;"204 No Content"}
            else if first.starts_with("POST /twitch/chat/messages "){chats+=1;"503 Service Unavailable"}
            else{ensure!(first.starts_with("PATCH /twitch/channel_points/custom_rewards/redemptions?")&&first.contains("reward_id=reward&")&&first.contains("id=redemption"));
                ensure!(serde_json::from_slice::<serde_json::Value>(&body)?["status"]==if timed_refund{"CANCELED"}else{"FULFILLED"});confirmations+=1;
                if confirmations==1{"503 Service Unavailable"}else{"200 OK"}};
            write!(socket,"HTTP/1.1 {status}\r\nConnection: close\r\nContent-Length: 0\r\n\r\n")?;
        }Ok((adds,confirmations))
    });
    let err=match orders::process(&api,&mut p,0){Err(e)=>e,Ok(_)=>anyhow::bail!("Expected Twitch failure. State {}, reason {}",p.v["orders"][0]["status"],p.v["orders"][0]["note"])};
    ensure!(crate::api::status(&err)==503);
    orders::defer(&mut p,0,&err)?;
    drop(p);
    let mut reopened=Profile::open(fixture.clone())?;
    ensure!(reopened.v["orders"][0]["status"]==if timed_refund{"reject"}else{"queued"});
    ensure!(!reopened.accept(&event)?);
    reopened.v["reward"]=json!("replacement");
    let due=n(&reopened.v["orders"][0],"nextAttempt");
    ensure!(reopened.next(due-1).is_none()&&reopened.next(due)==Some(0));
    let mut index=0;
    if timed_refund{
        let orders=reopened.v["orders"].as_array_mut().unwrap();
        orders.splice(0..0,(0..500).map(|id|json!({"id":id.to_string(),"status":"done"})));
        orders.push(json!({"id":"neighbor","status":"needs_track","choiceSince":now()}));
        reopened.v["chatAllowed"]=json!(true);
        reopened.save()?;
        index=500;
    }
    let warning=orders::process(&api,&mut reopened,index)?;
    if timed_refund{
        ensure!(warning.as_deref().is_some_and(|w|w.contains("503")));
        reopened=Profile::open(fixture.clone())?;
        let orders=reopened.v["orders"].as_array().unwrap();
        ensure!(orders.iter().filter(|o|finished(o)).count()==500 && orders[0]["id"]=="1");
        ensure!(orders[499]["id"]=="redemption" && b(&orders[499],"notificationAttempted"));
        ensure!(orders[500]["id"]=="neighbor" && orders[500]["status"]=="needs_track" && orders[500]["attempts"].is_null());
        ensure!(!reopened.accept(&event)?);
        index=499;
    }
    ensure!(reopened.v["orders"][index]["status"]==if timed_refund{"canceled"}else{"done"});
    ensure!(reopened.next(due).is_none() && reopened.due_choice_refund(due).is_none());
    ensure!(server.join().map_err(|_|anyhow::anyhow!("fixture stopped"))??==(if timed_refund{0}else{1},2),"Duplicate queue request or missing confirmation");
    std::fs::remove_file(fixture.join("settings.bin"))?;std::fs::remove_dir(fixture)?;
    Ok(if timed_refund{16}else{8})
}
