use crate::{api::*,profile::*};
use anyhow::{Result,Context,ensure,bail};
use regex::Regex;
use serde_json::{Value,json};
use unicode_normalization::{UnicodeNormalization,char::is_combining_mark};
use url::Url;

fn link(raw:&str)->Option<Url>{let u=Url::parse(raw.trim()).ok()?;(u.scheme()=="https"&&u.port().is_none()&&u.username().is_empty()&&u.password().is_none()).then_some(u)}
fn valid_id(id:&str)->bool{id.len()==22&&id.bytes().all(|c|c.is_ascii_alphanumeric())}
pub fn spotify(raw:&str)->Option<String>{
    if let Some(id)=raw.trim().strip_prefix("spotify:track:"){return valid_id(id).then(||format!("spotify:track:{id}"))}
    let u=link(raw)?;if u.host_str()!=Some("open.spotify.com"){return None}
    let re=Regex::new(r"^/(?:intl-[a-z]{2}/)?track/([A-Za-z0-9]{22})/?$").unwrap();Some(format!("spotify:track:{}",&re.captures(u.path())?[1]))
}
pub fn youtube(raw:&str)->Option<String>{let u=link(raw)?;let id=match u.host_str()?{
    "youtu.be"=>u.path().trim_matches('/').into(),
    "youtube.com"|"music.youtube.com"|"www.youtube.com"|"m.youtube.com"=>if u.path()=="/watch"{u.query_pairs().find(|(k,_)|k=="v")?.1.into_owned()}else{u.path().strip_prefix("/shorts/").or_else(||u.path().strip_prefix("/embed/"))?.trim_end_matches('/').into()},
    _=>return None
};let id:String=id;(id.len()==11&&id.bytes().all(|c|c.is_ascii_alphanumeric()||c==b'-'||c==b'_')).then_some(id)}
fn clean(raw: &str) -> String {
    // Preserve edition names like "(Live)" and "(Acoustic)".
    let presentation = Regex::new(r"(?i)[\[(]\s*(?:official (?:music )?(?:audio|video)|lyrics?(?: video)?|music video|visuali[sz]er|song|4k|hd)\s*[\])]").unwrap();
    let stripped = presentation.replace_all(raw, " ");
    Regex::new(r"(?i)\s*(?:\||[-–—])\s*(?:official (?:music )?(?:video|audio)|lyric[s]? video)\s*$")
        .unwrap().replace(&stripped, "").split_whitespace().collect::<Vec<_>>().join(" ")
}

fn channel(raw: &str) -> String {
    Regex::new(r"(?i)(?:\s*-\s*Topic|\s*VEVO)$").unwrap().replace(raw.trim(), "").trim().to_owned()
}

// A subtitle may appear before a dash on YouTube and in parentheses on Spotify.
fn title_forms(raw: &str) -> Vec<String> {
    let mut forms=vec![norm(raw)];
    let parts=Regex::new(r"^(.+?)\s+[-–—]\s+(.+)$").unwrap();
    if let Some(c)=parts.captures(raw){forms.push(norm(&format!("{} {}",&c[2],&c[1])))}
    forms
}

pub(crate) fn match_results(title:&str,author:&str,items:&[Value])->Result<Option<String>>{
    let forms=title_forms(title);
    let channel=norm(&channel(author));
    let mut found=None;
    for t in items {
        if t["is_playable"]==false||t["restrictions"].as_object().is_some_and(|v|!v.is_empty()){continue}
        let Some(uri)=spotify(s(t,"uri")) else{continue};
        let name=norm(s(t,"name"));
        let Some(rows)=t["artists"].as_array() else{continue};
        let mut artists:Vec<_>=rows.iter().map(|v|norm(s(v,"name"))).filter(|v|!v.is_empty()).collect();
        if artists.is_empty()||name.is_empty(){continue}
        let full=&forms[0];
        let credited=artists.iter().any(|a|*full==format!("{a} {name}")||*full==format!("{name} {a}"))
            || *full==format!("{} {name}",artists.join(" "));
        let from_artist=artists.contains(&channel)&&forms.contains(&name);
        if !(credited||from_artist){continue}
        artists.sort();
        let group=(name,artists);
        if let Some((earlier,_))=&found{if earlier!=&group{return Err(NeedsTrack.into())}}
        else{found=Some((group,uri))}
    }
    Ok(found.map(|(_,uri)|uri))
}
fn norm(s:&str)->String{s.nfd().filter(|c|!is_combining_mark(*c)).flat_map(char::to_lowercase).map(|c|if c.is_alphanumeric(){c}else{' '}).collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ")}
fn plausible_candidate(title_words:&str,channel_name:&str,t:&Value)->bool{
    let name=norm(s(t,"name"));
    !name.is_empty() && title_words.split_whitespace().any(|word|word.len()>=3 && name.split_whitespace().any(|n|n==word)) &&
        t["artists"].as_array().is_some_and(|v|v.iter().any(|artist|norm(s(artist,"name"))==channel_name))
}
#[derive(Debug)]struct NeedsTrack;
impl std::fmt::Display for NeedsTrack{fn fmt(&self,f:&mut std::fmt::Formatter)->std::fmt::Result{f.write_str("Уточните ссылку на трек Spotify / Choose the Spotify track")}}
impl std::error::Error for NeedsTrack{}
#[derive(Debug)]struct Reject(&'static str);
impl std::fmt::Display for Reject{fn fmt(&self,f:&mut std::fmt::Formatter)->std::fmt::Result{f.write_str(self.0)}}
impl std::error::Error for Reject{}
fn resolve(a:&Api,p:&mut Profile,raw:&str)->Result<String>{
    if let Some(uri)=spotify(raw){return Ok(uri)}
    let id=youtube(raw).ok_or(Reject("Нужна ссылка Spotify или YouTube / Unsupported link"))?;
    let meta:Value=match a.http.get(&format!("https://www.youtube.com/oembed?format=json&url=https://www.youtube.com/watch?v={id}")).call(){Ok(r)=>r.into_json()?,Err(ureq::Error::Status(400|401|403|404,_))=>return Err(Reject("Видео недоступно / Video unavailable").into()),Err(e)=>return Err(e.into())};
    let title=clean(s(&meta,"title"));if title.is_empty(){return Err(Reject("Нет названия / Missing title").into())}
    let author=channel(s(&meta,"author_name"));
    let mut queries=vec![title.clone()];
    if !author.is_empty(){
        queries.push(format!("{title} {author}"));
        if let Some((_,part))=Regex::new(r"\s+[-–—]\s+").unwrap().find(&title).map(|m|title.split_at(m.end())){
            queries.push(format!("{part} {author}"));
        }
    }
    let first=Regex::new(r"\s+[-–—]\s+").unwrap().split(&title).next().unwrap_or(&title);
    if first!=title && !author.is_empty(){
        queries.push(format!("{} {author}",first.split('(').next().unwrap_or(first).trim()));
    }
    let title_words=norm(&title);
    let channel_name=norm(&author);
    let mut plausible=false;
    for query in queries{
        let result=a.call(p,true,"GET",&format!("search?type=track&limit=10&q={}",enc(&query)),None)?;
        let items=result["tracks"]["items"].as_array().context("Invalid Spotify search response")?;
        plausible|=items.iter().any(|t|plausible_candidate(&title_words,&channel_name,t));
        if let Some(uri)=match_results(&title,&author,items)?{return Ok(uri)}
    }
    if plausible{return Err(NeedsTrack.into())}
    Err(Reject("Трек не найден / No matching track").into())
}
fn policy(p:&Value,t:&Value,uri:&str)->Result<()>{
    if n(p,"maxTrackMinutes")>0{ensure!(n(t,"duration_ms")>0,"Missing duration");if n(t,"duration_ms")>60_000*n(p,"maxTrackMinutes"){return Err(Reject("Превышен лимит длительности / Track too long").into())}}
    if b(p,"blockExplicit")&&t["explicit"].as_bool().context("Missing explicit flag")?{return Err(Reject("Explicit запрещён / Explicit track blocked").into())}
    for entry in s(p,"blockedTracks").lines().map(str::trim).filter(|x|!x.is_empty()){if entry.to_lowercase()==s(t,"name").to_lowercase()||spotify(entry).is_some_and(|v|[uri,s(t,"uri"),s(&t["linked_from"],"uri")].contains(&v.as_str())){return Err(Reject("Трек запрещён / Blocked track").into())}}
    if !s(p,"blockedArtists").trim().is_empty(){let artists=t["artists"].as_array().context("Missing artists")?;if artists.iter().any(|v|s(p,"blockedArtists").lines().any(|x|x.trim().to_lowercase()==s(v,"name").to_lowercase())){return Err(Reject("Исполнитель запрещён / Blocked artist").into())}}
    Ok(())
}
fn prepare(a:&Api,p:&mut Profile,i:usize)->Result<String>{
    let o=&p.v["orders"][i];let uri=if s(o,"uri").is_empty(){let raw=s(o,"input").to_owned();resolve(a,p,&raw)?}else{s(o,"uri").to_owned()};p.v["orders"][i]["uri"]=json!(uri);
    if n(&p.v,"maxTrackMinutes")>0||b(&p.v,"blockExplicit")||!s(&p.v,"blockedTracks").trim().is_empty()||!s(&p.v,"blockedArtists").trim().is_empty(){
        let t=a.call(p,true,"GET",&format!("tracks/{}",uri.strip_prefix("spotify:track:").context("Invalid URI")?),None)?;ensure!(t["type"]=="track"&&t["uri"].is_string(),"Invalid metadata");policy(&p.v,&t,&uri)?;
        p.v["orders"][i]["trackTitle"]=t["name"].clone();p.v["orders"][i]["trackArtist"]=json!(t["artists"].as_array().context("Missing artists")?.iter().map(|v|s(v,"name")).collect::<Vec<_>>().join(", "));
    }Ok(uri)
}
pub fn process(a:&Api,p:&mut Profile,i:usize)->Result<Option<String>>{
    if p.v["orders"][i]["status"]=="pending"{match prepare(a,p,i){
        Ok(uri)=>{a.access(p,true)?;p.v["orders"][i]["status"]=json!("sending");p.save()?;
            let response=a.call(p,true,"POST",&format!("me/player/queue?uri={}",enc(&uri)),None);
            let o=&mut p.v["orders"][i];o["spotifyRetryAt"]=json!(0);match response{Ok(_)=>o["status"]=json!("queued"),Err(e)=>{let st=status(&e);o["note"]=json!(describe(&e));o["status"]=json!(match st{429=>{o["spotifyRetryAt"]=json!(now()+retry(&e).max(1));"pending"},0|500..=599=>"uncertain",_=>"reject"});}}
            p.save()?;if p.v["orders"][i]["status"]=="queued"&&b(&p.v,"requestSoundEnabled"){let _=crate::sound::play(p);}
        },Err(e) if status(&e)==429=>{
            p.v["orders"][i]["spotifyRetryAt"]=json!(now()+retry(&e).max(1));
            p.save()?;
        },Err(e) if e.is::<NeedsTrack>()=>{let o=&mut p.v["orders"][i];o["status"]=json!("needs_track");o["choiceSince"]=json!(now());o["note"]=json!(e.to_string());p.save()?},Err(e) if e.is::<Reject>()=>{let o=&mut p.v["orders"][i];o["status"]=json!("reject");o["notify"]=json!(true);o["note"]=json!(e.to_string());p.save()?},Err(e)=>return Err(e)
    }}
    let o=p.v["orders"][i].clone();if matches!(s(&o,"status"),"queued"|"reject"){let added=o["status"]=="queued";p.v["orders"][i]["refundPending"]=json!(!added);match a.mark(p,&o,added){Ok(())=>p.v["orders"][i]["status"]=json!(if added{"done"}else{"canceled"}),Err(e) if matches!(status(&e),400|404)=>p.v["orders"][i]["status"]=json!("manual"),Err(e)=>return Err(e)}
        let notify=p.v["orders"][i]["status"]=="canceled" && b(&p.v["orders"][i],"notify") && !b(&p.v["orders"][i],"notificationAttempted");
        p.save()?;
        // Completed orders may shift when saving trims the history.
        if !notify{return Ok(None)}
    }
    let o=p.v["orders"][i].clone();if o["status"]=="canceled"&&b(&o,"notify")&&!b(&o,"notificationAttempted"){p.v["orders"][i]["notificationAttempted"]=json!(true);p.save()?;if b(&p.v,"chatAllowed"){let user:String=s(&o,"user").chars().filter(|x|x.is_alphanumeric()||*x=='_').collect();let lang=s(&p.v,"language");let reason=p.refund_reason(&o,now())?;let refund=if lang=="en"{"Points refunded."}else{"Баллы возвращены."};if let Err(error)=a.chat(p,&format!("@{user} {refund} {reason}").chars().take(450).collect::<String>()){
            if error.is::<SaveError>(){return Err(error)}
            return Ok(Some(format!("Уведомление о возврате / Refund notification: {}",describe(&error))));
        }}}
    Ok(None)
}
pub fn defer(p:&mut Profile,i:usize,e:&anyhow::Error)->Result<()>{
    if status(e)==429 {
        p.v["orders"][i]["nextAttempt"]=json!(now().saturating_add(retry(e).max(1)));
        return p.save()
    }
    let o=&mut p.v["orders"][i];let attempts=n(o,"attempts")+1;o["attempts"]=json!(attempts);o["nextAttempt"]=json!(now()+(15*attempts).min(120));if o["status"]!="reject"{o["note"]=json!(describe(e));}
    if o["status"]=="sending"{o["status"]=json!("uncertain")}else if attempts>=3{if o["status"]=="pending"{o["status"]=json!("reject");o["notify"]=json!(true);o["attempts"]=json!(0)}else if matches!(s(o,"status"),"queued"|"reject"){o["status"]=json!("manual")}}
    p.save()
}
pub fn recover(a:&Api,p:&mut Profile)->Result<()>{
    let mut cursor=String::new();let mut seen=std::collections::HashSet::new();
    loop{let path=format!("channel_points/custom_rewards/redemptions?broadcaster_id={}&reward_id={}&status=UNFULFILLED&sort=OLDEST&first=50{}",enc(s(&p.v,"broadcaster")),enc(s(&p.v,"reward")),cursor);
        let v=a.call(p,false,"GET",&path,None)?;for e in rows(&v)?{p.accept(e)?;}
        let next=s(&v["pagination"],"cursor");if next.is_empty()||p.v["orders"].as_array().unwrap().iter().filter(|o|!finished(o)).count()>=100{break}if !seen.insert(next.to_string()){bail!("Repeated Twitch cursor")}cursor=format!("&after={}",enc(next));
    }Ok(())
}

pub(crate) fn check_matches()->Result<usize>{
    let uri="spotify:track:3qWjq3rURGmsWEfIpAo6IP";
    let row=|name:&str,artist:&str|json!({"uri":uri,"name":name,"artists":[{"name":artist}]});
    let title=clean("Lord Verity - Master of Humanity (song)");
    let official=row("Master of Humanity (Lord Verity)","Horror Skunx");
    ensure!(match_results(&title,"Horror Skunx",&[official.clone()])?.as_deref()==Some(uri));
    ensure!(match_results(&title,"Other uploader",&[official.clone()])?.is_none());
    ensure!(match_results(&clean("Song (Live)"),"Artist",&[row("Song","Artist")])?.is_none());
    ensure!(match_results(&clean("Song [Official Video]"),"ArtistVEVO",&[row("Song","Artist")])?.is_some());
    ensure!(match_results("Song","Artist",&[row("Song (Remix)","Artist")])?.is_none());
    ensure!(match_results(&title,"Horror Skunx",&[official.clone(),official])?.is_some());
    Ok(6)
}

#[cfg(test)]
mod history_audit {
    use super::*;
    fn candidate(title: &str, artist: &str) -> Value {
        json!({"name":title,"artists":[{"name":artist}],"uri":"spotify:track:6nxwpI2PRJSgDwSxOl0n8e"})
    }

    #[test]
    fn public_metadata_matches_and_review() -> Result<()> {
        // Exercise the release rules with individual candidates, independent of API ranking.
        for (video, uploader, song, artist, automatic, plausible) in [
            ("WINTER (Vivaldi) - The Four Seasons - Luca Stricagnoli - Fingerstyle Guitar","Luca Stricagnoli","Winter - The Four Seasons","Luca Stricagnoli",false,true),
            ("LITTLE BIG - LollyBomb [Official Music Video]","Little Big","LollyBomb","Little Big",true,true),
            ("Rick Astley - Never Gonna Give You Up (Official Video) (4K Remaster)","Rick Astley","Never Gonna Give You Up","Rick Astley",false,true),
            ("ЧИКИБАМБОНИ ПЕСНЯ | Все виды","Amy Leeman","Чикибамбони на гитаре","Amy Leeman",false,true),
            ("Lord Verity - Master of Humanity (song)","Horror Skunx","Master of Humanity (Lord Verity)","Horror Skunx",true,true),
        ] {
            let title=clean(video);
            let row=candidate(song,artist);
            assert_eq!(match_results(&title,uploader,std::slice::from_ref(&row))?.is_some(),automatic,"{video}");
            assert_eq!(plausible_candidate(&norm(&title),&norm(&channel(uploader)),&row),plausible,"{video}");
        }
        Ok(())
    }

    #[test]
    fn moderation_for_original_and_relinked_recordings() {
        let uri="spotify:track:6nxwpI2PRJSgDwSxOl0n8e";
        let alternate="spotify:track:4uLU6hMCjMI75M1A2tKUQC";
        let track=json!({"name":"Song","uri":alternate,"linked_from":{"uri":uri},"artists":[{"name":"Artist"}],"explicit":true,"duration_ms":120001});
        let mut prefs=json!({"maxTrackMinutes":2});
        assert!(policy(&prefs,&track,uri).unwrap_err().is::<Reject>());
        prefs=json!({"blockExplicit":true});
        assert!(policy(&prefs,&track,uri).unwrap_err().is::<Reject>());
        for entry in [uri,alternate,"sOnG"] {
            assert!(policy(&json!({"blockedTracks":entry}),&track,uri).unwrap_err().is::<Reject>());
        }
        assert!(policy(&json!({"blockedArtists":"aRTist"}),&track,uri).unwrap_err().is::<Reject>());
        assert!(policy(&json!({"maxTrackMinutes":3}),&track,uri).is_ok());
    }
}
