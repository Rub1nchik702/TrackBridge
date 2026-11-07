using System;
using System.Linq;
using System.Collections.Generic;
using System.Collections.Concurrent;
using System.Threading;
using System.Threading.Tasks;
using System.Text.RegularExpressions;

namespace TrackBridge {
public class VoteSession {
 public string Uri="",Description="",Title="",Artist="",ImageUrl="";
 public long Duration,Progress;
 public bool Playing,SkipLatched;
 public long Generation;
 long progress=-1;
 readonly HashSet<string> voters=new HashSet<string>();
 public int Count {get{return voters.Count;}}
 public void Observe(Dictionary<string,object> data){
  string uri="",description="";long next=-1;Title="";Artist="";ImageUrl="";Duration=0;
  Playing=data.ContainsKey("is_playing") && Convert.ToBoolean(data["is_playing"]);
  if(data.ContainsKey("item") && data["item"] is Dictionary<string,object>){
   var item=Engine.D(data,"item");if(Engine.Str(item,"type")=="track"){
    try{uri=Engine.Track(Engine.Str(item,"uri"));}catch{uri="";}
    Title=Engine.Str(item,"name");description=Title;
    if(item.ContainsKey("duration_ms") && item["duration_ms"]!=null)Duration=Convert.ToInt64(item["duration_ms"]);
    if(item.ContainsKey("album")){var album=Engine.D(item,"album");if(album.ContainsKey("images")){var images=album["images"] as System.Collections.IEnumerable;if(images!=null)foreach(var img in images){var row=img as Dictionary<string,object>;if(row!=null){ImageUrl=Engine.Str(row,"url");break;}}}}
    if(item.ContainsKey("artists"))description+=" — "+string.Join(", ",((System.Collections.IEnumerable)item["artists"]).Cast<Dictionary<string,object>>().Select(a=>Engine.Str(a,"name")));
   }
  }
  if(data.ContainsKey("progress_ms") && data["progress_ms"]!=null)next=Convert.ToInt64(data["progress_ms"]);
  if(uri!=Uri || (next>=0 && progress>=0 && next+3000<progress)){voters.Clear();SkipLatched=false;Generation++;}
  Uri=uri;Description=description;Artist=description.StartsWith(Title+" — ")?description.Substring(Title.Length+3):"";progress=next;Progress=Math.Max(0,next);
 }
 public bool Add(string user){return Playing && Uri!="" && !SkipLatched && user!="" && voters.Add(user);}
 public void Reset(){voters.Clear();}
 public void Invalidate(){Playing=false;voters.Clear();}
}
public class ChatCommands {
 readonly Engine e;
 readonly ConcurrentQueue<Dictionary<string,object>> inbox=new ConcurrentQueue<Dictionary<string,object>>();
 readonly HashSet<string> seen=new HashSet<string>();readonly Queue<string> seenOrder=new Queue<string>();
 public readonly VoteSession Votes=new VoteSession();
 long songReply;
 long thresholdGeneration=-1, unavailableReply;
 int threshold,liveViewers;long viewerRead;
 readonly ConcurrentQueue<string> controls=new ConcurrentQueue<string>();
 public void Control(string action){if((action=="reset" || action=="skip") && controls.Count<10)controls.Enqueue(action);}
 public async Task ApplyControl(string action){
  if(action=="reset"){Votes.Reset();Publish();return;}
  if(action!="skip")return;await Refresh();if(Votes.Uri=="" || !Votes.Playing || Votes.SkipLatched)return;
  Votes.SkipLatched=true;try{await e.Api(true,"POST","me/player/next");}catch{Publish();throw;}Publish();
 }
 async Task ReadViewers(){
  var stream=Engine.Data(await e.Api(false,"GET","streams?user_id="+System.Uri.EscapeDataString(e.S.broadcaster))).FirstOrDefault();
  liveViewers=stream==null?0:Convert.ToInt32(stream["viewer_count"]);viewerRead=Engine.Now;
 }
 void Publish(){
  e.TrackTitle=Votes.Title;e.TrackArtist=Votes.Artist;e.TrackImage=Votes.ImageUrl;e.TrackProgress=Votes.Progress;e.TrackDuration=Votes.Duration;e.SkipVotes=Votes.Count;e.SkipThreshold=thresholdGeneration==Votes.Generation?threshold:0;
  e.NowPlaying=Votes.Description;e.CurrentTrackUrl=Votes.Uri==""?"":"https://open.spotify.com/track/"+Votes.Uri.Substring("spotify:track:".Length);
  e.VoteStatus=(L.Language=="en"?"Viewers: ":"Зрителей: ")+(viewerRead==0 || liveViewers<0?"—":liveViewers.ToString())+" · "+(L.Language=="en"?"Votes: ":"Голоса: ")+Votes.Count+" / "+(thresholdGeneration==Votes.Generation?threshold.ToString():"—")+(e.S.votingEnabled?"":L.Language=="en"?" · Voting disabled":" · Голосование отключено");e.Changed();
 }
 async Task<bool> PrepareThreshold(){
  if(thresholdGeneration==Votes.Generation && threshold>0)return true;
  await ReadViewers();int viewers=liveViewers;
  if(viewers<=0){
   if(Engine.Now-unavailableReply>=10){unavailableReply=Engine.Now;await Reply(L.Language=="en"?"Skip voting is unavailable: no live viewer count.":"Голосование недоступно: нет данных о зрителях прямого эфира.");}
   return false;
  }
  threshold=e.S.skipAutomatic || viewers>=100?viewers/2+viewers%2:Math.Max(1,Math.Min(99,e.S.skipManualVotes));
  thresholdGeneration=Votes.Generation;return true;
 }
 static string Command(Dictionary<string,object> message){
  string command=Engine.Str(Engine.D(message,"message"),"text").Trim().ToLowerInvariant();
  return command=="!skip"?"!voteskip":command=="!nowplaying"?"!song":command;
 }
 public ChatCommands(Engine engine){e=engine;}
 public void Accept(Dictionary<string,object> message){
  if(Engine.Str(message,"broadcaster_user_id")!=e.S.broadcaster || !message.ContainsKey("message"))return;
  string text=Command(message);
  if(text!="!song" && text!="!voteskip")return;
  string id=Engine.Str(message,"message_id");if(id=="" || Engine.Str(message,"chatter_user_id")=="" || inbox.Count>=100)return;
  lock(seen){if(!seen.Add(id))return;seenOrder.Enqueue(id);if(seenOrder.Count>1000)seen.Remove(seenOrder.Dequeue());}
  inbox.Enqueue(message);
 }
 async Task Refresh(){long prior=Votes.Generation,progress=Votes.Progress;string uri=Votes.Uri;Votes.Observe(await e.Api(true,"GET","me/player"));bool restarted=uri!="" && uri==Votes.Uri && Votes.Progress+3000<progress && Votes.Generation!=prior;e.ObserveOrderedPlayback(Votes.Uri,Votes.Playing,restarted);Publish();}
 async Task Reply(string text){
  var result=await e.Api(false,"POST","chat/messages",new {broadcaster_id=e.S.broadcaster,sender_id=e.S.broadcaster,message=text});
  var row=Engine.Data(result).FirstOrDefault();if(row==null || !row.ContainsKey("is_sent") || !Convert.ToBoolean(row["is_sent"]))e.Log(L.Language=="en"?"Twitch did not deliver a command reply.":"Twitch не доставил ответ на команду.");
 }
 public async Task Handle(Dictionary<string,object> message){
  string command=Command(message);
  if(command=="!song" && Engine.Now-songReply<5)return;
  await Refresh();
  if(command=="!song"){
   songReply=Engine.Now;
   if(Votes.Uri==""){await Reply(L.Language=="en"?"No Spotify track is currently available.":"Текущий трек Spotify сейчас недоступен.");return;}
   string label=Regex.Replace(Votes.Description,@"[\r\n\x00-\x1F]"," ");if(label.Length>350)label=label.Substring(0,350);
   await Reply((L.Language=="en"?"Now playing: ":"Сейчас играет: ")+label+" https://open.spotify.com/track/"+Votes.Uri.Substring("spotify:track:".Length));return;
  }
  if(command!="!voteskip" || !e.S.votingEnabled || !Votes.Playing || Votes.Uri=="" || Votes.SkipLatched)return;
  if(!await PrepareThreshold() || !Votes.Add(Engine.Str(message,"chatter_user_id")))return;
  Publish();
  if(Votes.Count<threshold){await Reply((L.Language=="en"?"Skip votes: ":"Голоса за пропуск: ")+Votes.Count+"/"+threshold);return;}
  long generation=Votes.Generation;string track=Votes.Uri;
  await Refresh();if(Votes.Generation!=generation || Votes.Uri!=track || !Votes.Playing)return;
  Votes.SkipLatched=true;
  try{await e.Api(true,"POST","me/player/next");}
  catch(Exception){e.Log(L.Language=="en"?"Skip response uncertain; it will not be retried for this track.":"Ответ на пропуск неоднозначен; повторного запроса для этого трека не будет.");return;}
  await Reply(L.Language=="en"?Votes.Count+" different viewers voted — skipping the track.":Votes.Count+" разных зрителей проголосовали — трек пропущен.");
 }
 public async Task Run(CancellationToken ct){
  long polled=0;
  while(!ct.IsCancellationRequested){
   try{
    if(!e.S.votingEnabled && Votes.Count>0)Votes.Reset();
    if(Engine.Now-polled>=3){polled=Engine.Now;await Refresh();}
    if(Engine.Now-viewerRead>=30){try{await ReadViewers();}catch{liveViewers=-1;viewerRead=Engine.Now;}Publish();}
    string action;if(controls.TryDequeue(out action)){await ApplyControl(action);continue;}
    Dictionary<string,object> message;if(inbox.TryDequeue(out message))await Handle(message);else await Task.Delay(200,ct);
   }catch(OperationCanceledException){if(ct.IsCancellationRequested)break;Votes.Invalidate();}
   catch(Exception ex){Votes.Invalidate();e.Log("Chat commands: "+ex.Message);}
   try{await Task.Delay(200,ct);}catch(OperationCanceledException){break;}
  }
 }
}
}

