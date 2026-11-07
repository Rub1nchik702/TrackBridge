using System;
using System.IO;
using System.Text;
using System.Linq;
using System.Net;
using System.Net.Http;
using System.Net.Sockets;
using System.Security.Cryptography;
using System.Collections.Generic;
using System.Threading;
using System.Threading.Tasks;
using System.Web.Script.Serialization;
using System.Diagnostics;

namespace TrackBridge {
public class Token { public string access_token = ""; public string refresh_token = ""; public long expires; }
public class Order {
 public int attempts;
 public long nextAttempt,created;
 public bool notify, notificationAttempted,played,historyHidden;
 public string id, reward, broadcaster, user, input, uri, status, note,trackTitle,trackArtist;
 public string time = DateTime.Now.ToString("g");
}
public class Settings {
 public List<string> recentReceipts=new List<string>();
 public string language="ru";
 public bool autoHidePlayed;
 public bool requestSoundEnabled=true;
 public int requestSoundVolume=20;
 public string requestSoundName="";
 public bool chatAllowed;
 public bool chatReadAllowed;
 public bool skipAutomatic=true;
 public int skipManualVotes=5;
 public bool votingEnabled=true,blockExplicit;
 public int maxTrackMinutes=10;
 public string blockedTracks="",blockedArtists="";
 public string twitchId = "", spotifyId = "", broadcaster = "", reward = "";
 public Token twitch = new Token(), spotify = new Token();
 public List<Order> orders = new List<Order>();
}
public class ApiError : Exception {
 public int Status; public int Retry;
 public ApiError(int status, int retry, string service) : base(service + ": HTTP " + status + Hint(status)) { Status=status; Retry=retry; }
 static string Hint(int s) { return s==401 ? L.T(" — подключите аккаунт заново.") : s==403 ? L.T(" — проверьте права, Premium и доступ приложения.") : s==404 ? L.T(" — откройте Spotify и запустите музыку на нужном устройстве.") : s==429 ? L.T(" — превышен лимит запросов, подождите.") : L.T(" — запрос не выполнен."); }
}
public partial class Engine : IDisposable {
 public Settings S;
 public Action<string> Log = delegate {};
 public Action Changed = delegate {};
 public Action TrackQueued = delegate {};
 public bool EventsConnected;
 public ChatCommands Chat;
 public string NowPlaying="",CurrentTrackUrl="",VoteStatus="",TrackTitle="",TrackArtist="",TrackImage="";
 public long TrackProgress,TrackDuration;
 public int SkipVotes,SkipThreshold;
 public string ReceptionStatus=L.T("Приём остановлен — нажмите «Запустить»");
 public void Reception(bool connected,string text) { EventsConnected=connected;ReceptionStatus=text;Changed(); }
 internal Func<bool,string,string,object,Task<Dictionary<string,object>>> TestApi;
 public readonly object Gate = new object();
 readonly SemaphoreSlim auth = new SemaphoreSlim(1,1);
 readonly HttpClient http = new HttpClient(new HttpClientHandler { AllowAutoRedirect=false }) { Timeout=TimeSpan.FromSeconds(20) };
 string observedOrder="",observedUri="",currentPlaybackUri="";
 public void ObserveOrderedPlayback(string uri,bool playing,bool restarted=false){
  if(string.IsNullOrEmpty(uri))return;currentPlaybackUri=uri;
  lock(Gate){
   bool changed=false;
   if(observedOrder!="" && (observedUri!=uri || restarted)){
    var previous=S.orders.FirstOrDefault(o=>o.id==observedOrder);if(previous!=null){previous.played=true;changed=true;}observedOrder="";observedUri="";
   }
   if(playing && observedOrder==""){
    var current=S.orders.FirstOrDefault(o=>o.uri==uri && !o.played && !o.historyHidden && (o.status=="done" || o.status=="queued"));if(current!=null){observedOrder=current.id;observedUri=uri;}
   }
   if(changed)Save();
  }
 }
 public bool CanHideOrder(Order order){return order.status=="done" && order.played && order.id!=observedOrder && order.uri!=currentPlaybackUri;}
 public int HidePlayedOrders(string id=null){lock(Gate){int count=0;foreach(var order in S.orders.Where(o=>!o.historyHidden && CanHideOrder(o) && (id==null || o.id==id))){order.historyHidden=true;count++;}if(count>0)Save();return count;}}
 public int HideSelectedOrders(IEnumerable<string> ids){lock(Gate){var selected=new HashSet<string>(ids);int count=0;foreach(var order in S.orders.Where(o=>!o.historyHidden && selected.Contains(o.id))){order.historyHidden=true;count++;}if(count>0)Save();return count;}}
 readonly string path;
 public string RequestSoundFile {get{return string.IsNullOrEmpty(S.requestSoundName)?null:Path.Combine(Path.GetDirectoryName(path),"request-notification.wav");}}
 public void SetRequestSound(string selectedFile){
  byte[] wave=NotificationSound.ImportWave(selectedFile);string destination=Path.Combine(Path.GetDirectoryName(path),"request-notification.wav");
  File.WriteAllBytes(destination+".tmp",wave);if(File.Exists(destination))File.Replace(destination+".tmp",destination,null);else File.Move(destination+".tmp",destination);
  S.requestSoundName=Path.GetFileName(selectedFile);Save();
 }
 public void ResetRequestSound(){S.requestSoundName="";Save();}
 public static long Now { get { return DateTimeOffset.UtcNow.ToUnixTimeSeconds(); } }
 public static Dictionary<string,object> Obj(string s) { return new JavaScriptSerializer().Deserialize<Dictionary<string,object>>(s); }
 public static string Json(object o) { return Serializer().Serialize(o); }
 static JavaScriptSerializer Serializer(){return new JavaScriptSerializer{MaxJsonLength=64*1024*1024};}
 static bool Finished(Order o){return o.status=="done" || (o.status=="canceled" && (!o.notify || o.notificationAttempted));}
 long lastCapacityLog;
 public static string Str(Dictionary<string,object> d, string k) { return d.ContainsKey(k) && d[k]!=null ? Convert.ToString(d[k]) : ""; }
 public static Dictionary<string,object> D(Dictionary<string,object> d,string k) { return (Dictionary<string,object>)d[k]; }
 public static IEnumerable<Dictionary<string,object>> Data(Dictionary<string,object> d) {
  object value;
  if(!d.TryGetValue("data",out value) || value==null) throw new Exception(L.T("API вернул ответ без списка data."));
  var items=value as System.Collections.IEnumerable;
  if(items==null || value is string || value is System.Collections.IDictionary) throw new Exception(L.T("API вернул некорректный список data."));
  foreach(var item in items) {
   var row=item as Dictionary<string,object>;
   if(row==null) throw new Exception(L.T("API вернул некорректный элемент списка data."));
   yield return row;
  }
 }
 public static string Q(string s) { return Uri.EscapeDataString(s); }
 public Engine(string dataDir=null) {
  ServicePointManager.SecurityProtocol=SecurityProtocolType.Tls12;
  string dir=dataDir ?? Path.Combine(AppDomain.CurrentDomain.BaseDirectory,"data");
  Directory.CreateDirectory(dir); path=Path.Combine(dir,"settings.bin");
  if(File.Exists(path)) {
   try { S=Serializer().Deserialize<Settings>(Encoding.UTF8.GetString(ProtectedData.Unprotect(File.ReadAllBytes(path),null,DataProtectionScope.CurrentUser))); }
   catch { throw new Exception(L.T("Не удалось прочитать настройки. Сохраните резервную копию и переименуйте ") + path); }
  } else S=new Settings();
  if(S.recentReceipts==null)S.recentReceipts=new List<string>();
  foreach(var old in S.orders){string receipt=old.broadcaster+":"+old.id;if(!S.recentReceipts.Contains(receipt))S.recentReceipts.Add(receipt);}
  L.Language=S.language=="en"?"en":"ru";
  foreach(var o in S.orders.Where(x=>x.status=="sending")) { o.status="uncertain"; o.note=L.T("Программа закрылась во время отправки. Проверьте очередь Spotify вручную; повторной отправки не будет."); }
  Save();
 }
 public void Save() {
  lock(Gate) {
   var trim=S.orders.Where(Finished).Reverse().Skip(500).ToList();foreach(var item in trim)S.orders.Remove(item);
   if(S.recentReceipts.Count>5000)S.recentReceipts.RemoveRange(0,S.recentReceipts.Count-5000);
   var bytes=ProtectedData.Protect(Encoding.UTF8.GetBytes(Json(S)),null,DataProtectionScope.CurrentUser);
   File.WriteAllBytes(path+".tmp",bytes);
   if(File.Exists(path)) File.Replace(path+".tmp",path,null); else File.Move(path+".tmp",path);
  }
 }
 public static string Track(string raw) {
  string s=(raw??"").Trim();
  if(System.Text.RegularExpressions.Regex.IsMatch(s,@"^spotify:track:[A-Za-z0-9]{22}$")) return s;
  Uri u;
  if(!Uri.TryCreate(s,UriKind.Absolute,out u) || u.Scheme!="https" || u.Host!="open.spotify.com" || !u.IsDefaultPort || u.UserInfo!="") throw new Exception(L.T("Нужна полная ссылка https://open.spotify.com/track/…"));
  var m=System.Text.RegularExpressions.Regex.Match(u.AbsolutePath,@"^/(?:intl-[a-z]{2}/)?track/([A-Za-z0-9]{22})/?$");
  if(!m.Success) throw new Exception(L.T("Принимаются только треки Spotify, не альбомы, плейлисты или короткие ссылки."));
  return "spotify:track:"+m.Groups[1].Value;
 }
 async Task<Dictionary<string,object>> Raw(string method,string url,object body,string access,string client,bool form,CancellationToken ct=default(CancellationToken)) {
  using(var req=new HttpRequestMessage(new HttpMethod(method),url)) {
   if(access!=null) req.Headers.TryAddWithoutValidation("Authorization","Bearer "+access);
   if(client!=null) req.Headers.TryAddWithoutValidation("Client-Id",client);
   if(body!=null) req.Content=form ? (HttpContent)new FormUrlEncodedContent((Dictionary<string,string>)body) : new StringContent(Json(body),Encoding.UTF8,"application/json");
   using(var res=await http.SendAsync(req,ct)) {
    var text=await res.Content.ReadAsStringAsync();
    if(!res.IsSuccessStatusCode) {
     int retry=res.Headers.RetryAfter!=null && res.Headers.RetryAfter.Delta.HasValue ? (int)res.Headers.RetryAfter.Delta.Value.TotalSeconds : 5;
     // Only the device-code endpoint needs its documented pending/slow_down errors.
     if(url=="https://id.twitch.tv/oauth2/token" && form && ((Dictionary<string,string>)body).ContainsKey("device_code")) {
      var err=Obj(text); var msg=Str(err,"message");
      if(msg=="authorization_pending" || msg=="slow_down") return err;
     }
     throw new ApiError((int)res.StatusCode,retry,new Uri(url).Host);
    }
    return string.IsNullOrWhiteSpace(text) ? new Dictionary<string,object>() : Obj(text);
   }
  }
 }
 void StoreToken(Token t,Dictionary<string,object> d) {
  t.access_token=Str(d,"access_token");
  if(d.ContainsKey("refresh_token")) t.refresh_token=Str(d,"refresh_token");
  t.expires=Now+Convert.ToInt64(d["expires_in"]); Save(); Changed();
 }
 async Task<string> Access(bool spotify) {
  await auth.WaitAsync();
  try {
   var t=spotify?S.spotify:S.twitch; string id=spotify?S.spotifyId:S.twitchId;
   if(t.access_token=="") throw new Exception(L.T("Сначала подключите ")+(spotify?"Spotify":"Twitch")+".");
   if(t.expires<Now+90) {
    if(t.refresh_token=="") throw new Exception(L.T("Подключите аккаунт заново."));
    var d=await Raw("POST",spotify?"https://accounts.spotify.com/api/token":"https://id.twitch.tv/oauth2/token",new Dictionary<string,string>{{"grant_type","refresh_token"},{"refresh_token",t.refresh_token},{"client_id",id}},null,null,true);
    StoreToken(t,d);
   }
   return t.access_token;
  } finally { auth.Release(); }
 }
 public async Task<Dictionary<string,object>> Api(bool spotify,string method,string endpoint,object body=null) {
  if(TestApi!=null) return await TestApi(spotify,method,endpoint,body);
  var token=await Access(spotify);
  return await Raw(method,(spotify?"https://api.spotify.com/v1/":"https://api.twitch.tv/helix/")+endpoint,body,token,spotify?null:S.twitchId,false);
 }
 public async Task ConnectTwitch(CancellationToken ct) {
  if(string.IsNullOrWhiteSpace(S.twitchId)) throw new Exception(L.T("Введите Twitch Client ID из кабинета разработчика."));
  const string scopes="channel:read:redemptions channel:manage:redemptions user:write:chat user:read:chat";
  var d=await Raw("POST","https://id.twitch.tv/oauth2/device",new Dictionary<string,string>{{"client_id",S.twitchId},{"scopes",scopes}},null,null,true,ct);
  Log(L.T("Код Twitch: ")+Str(d,"user_code")+L.T(". Подтвердите подключение в браузере."));
  Process.Start(Str(d,"verification_uri"));
  long end=Now+Convert.ToInt32(d["expires_in"]); int interval=Convert.ToInt32(d["interval"]);
  while(Now<end) {
   await Task.Delay(TimeSpan.FromSeconds(interval),ct);
   var r=await Raw("POST","https://id.twitch.tv/oauth2/token",new Dictionary<string,string>{{"client_id",S.twitchId},{"scopes",scopes},{"device_code",Str(d,"device_code")},{"grant_type","urn:ietf:params:oauth:grant-type:device_code"}},null,null,true,ct);
   if(r.ContainsKey("access_token")) { StoreToken(S.twitch,r); await ValidateTwitch(); Log(L.T("Twitch подключён.")); return; }
   if(Str(r,"message")=="slow_down") interval+=5;
  }
  throw new Exception(L.T("Время подключения Twitch истекло."));
 }
 public async Task ValidateTwitch() {
  var access=await Access(false);
  using(var req=new HttpRequestMessage(HttpMethod.Get,"https://id.twitch.tv/oauth2/validate")) {
   req.Headers.TryAddWithoutValidation("Authorization","OAuth "+access);
   using(var res=await http.SendAsync(req)) {
    if(!res.IsSuccessStatusCode) throw new Exception(L.T("Авторизация Twitch отозвана. Подключите аккаунт заново."));
    var d=Obj(await res.Content.ReadAsStringAsync());
    if(Str(d,"client_id")!=S.twitchId) throw new Exception(L.T("Twitch Client ID не совпадает с токеном."));
    S.broadcaster=Str(d,"user_id");
    S.chatAllowed=d.ContainsKey("scopes") && ((System.Collections.IEnumerable)d["scopes"]).Cast<object>().Any(x=>Convert.ToString(x)=="user:write:chat");
    S.chatReadAllowed=d.ContainsKey("scopes") && ((System.Collections.IEnumerable)d["scopes"]).Cast<object>().Any(x=>Convert.ToString(x)=="user:read:chat");Save();
   }
  }
 }
 public const string Redirect="http://127.0.0.1:43821/callback";
 public async Task ConnectSpotify(CancellationToken ct) {
  if(string.IsNullOrWhiteSpace(S.spotifyId)) throw new Exception(L.T("Введите Spotify Client ID из кабинета разработчика."));
  string verifier=RandomString(),state=RandomString(),challenge;
  using(var sha=SHA256.Create()) challenge=B64(sha.ComputeHash(Encoding.ASCII.GetBytes(verifier)));
  var listener=new TcpListener(IPAddress.Loopback,43821); listener.Start();
  try {
   Process.Start("https://accounts.spotify.com/authorize?client_id="+Q(S.spotifyId)+"&response_type=code&redirect_uri="+Q(Redirect)+"&scope="+Q("user-modify-playback-state user-read-playback-state")+"&state="+Q(state)+"&code_challenge_method=S256&code_challenge="+Q(challenge));
   using(var timeout=CancellationTokenSource.CreateLinkedTokenSource(ct)) {
    timeout.CancelAfter(TimeSpan.FromMinutes(4));
    using(timeout.Token.Register(()=>listener.Stop())) {
     while(true) {
      using(var client=await listener.AcceptTcpClientAsync())
      using(timeout.Token.Register(()=>client.Close())) {
       client.ReceiveTimeout=5000; client.SendTimeout=5000;
       string line;
       try{line=await Task.Run(()=>ReadRequestLine(client.GetStream()));}catch(IOException){continue;}
       var parts=(line??"").Split(' '); Uri u;
       if(parts.Length<2 || !Uri.TryCreate("http://127.0.0.1"+parts[1],UriKind.Absolute,out u)) continue;
       var query=System.Web.HttpUtility.ParseQueryString(u.Query);
       bool valid=parts.Length==3 && parts[0]=="GET" && parts[2].StartsWith("HTTP/1.",StringComparison.Ordinal) && u.AbsolutePath=="/callback" && query["state"]==state;
       var text=valid ? L.T("Подключение обработано. Можно закрыть вкладку.") : L.T("Некорректный запрос авторизации.");
       byte[] body=Encoding.UTF8.GetBytes("<!doctype html><meta charset=utf-8><p>"+text+"</p>");
       byte[] head=Encoding.ASCII.GetBytes("HTTP/1.1 "+(valid?"200 OK":"400 Bad Request")+"\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: "+body.Length+"\r\nConnection: close\r\n\r\n");
       await client.GetStream().WriteAsync(head,0,head.Length); await client.GetStream().WriteAsync(body,0,body.Length);
       if(!valid) continue;
       if(query["code"]==null) throw new Exception(L.T("Подключение Spotify отменено."));
       var d=await Raw("POST","https://accounts.spotify.com/api/token",new Dictionary<string,string>{{"client_id",S.spotifyId},{"grant_type","authorization_code"},{"code",query["code"]},{"redirect_uri",Redirect},{"code_verifier",verifier}},null,null,true,timeout.Token);
       StoreToken(S.spotify,d); Log(L.T("Spotify подключён.")); return;
      }
     }
    }
   }
  } finally { listener.Stop(); }
 }
 static string B64(byte[] b) { return Convert.ToBase64String(b).TrimEnd('=').Replace('+','-').Replace('/','_'); }
 static string RandomString() { var b=new byte[32]; using(var rng=RandomNumberGenerator.Create()) rng.GetBytes(b); return B64(b); }
 public async Task<List<Dictionary<string,object>>> Rewards() {
  await ValidateTwitch(); return Data(await Api(false,"GET","channel_points/custom_rewards?broadcaster_id="+Q(S.broadcaster)+"&only_manageable_rewards=true")).ToList();
 }
 public async Task<string> CreateReward(string title,int cost) {
  await ValidateTwitch();
  var d=await Api(false,"POST","channel_points/custom_rewards?broadcaster_id="+Q(S.broadcaster),new {title=title,cost=cost,prompt=L.T("Ссылка на трек Spotify, YouTube или YouTube Music. Если совпадение в Spotify не найдено, баллы возвращаются."),is_user_input_required=true,should_redemptions_skip_request_queue=false,is_global_cooldown_enabled=true,global_cooldown_seconds=15});
  return Str(Data(d).First(),"id");
 }
 public void Accept(Dictionary<string,object> e) {
  string id=Str(e,"id"); string reward=Str(D(e,"reward"),"id");
  if(reward!=S.reward || id=="") return;
  string eventStatus=Str(e,"status");if(eventStatus!="" && !eventStatus.Equals("unfulfilled",StringComparison.OrdinalIgnoreCase))return;
  lock(Gate) {
   string receipt=S.broadcaster+":"+id;
   if(S.recentReceipts.Contains(receipt) || S.orders.Any(o=>o.id==id && o.broadcaster==S.broadcaster)) return;
   if(S.orders.Count(o=>!Finished(o))>=100){if(Now-lastCapacityLog>30){lastCapacityLog=Now;Log(L.Language=="en"?"Queue full. Additional requests remain pending in Twitch until capacity is available.":"Очередь заполнена. Дополнительные заказы остаются в очереди Twitch до освобождения места.");}return;}
   S.recentReceipts.Add(receipt);
   string viewer=Str(e,"user_name");bool tooMany=S.orders.Count(o=>o.broadcaster==S.broadcaster && o.user==viewer && !Finished(o))>=3;
   S.orders.Add(new Order {created=Now,id=id,reward=reward,broadcaster=S.broadcaster,user=viewer,input=Str(e,"user_input"),status=tooMany?"reject":"pending",notify=tooMany,note=tooMany?(L.Language=="en"?"You already have three active requests. Points refunded.":"У вас уже три активных заказа. Баллы возвращены."):L.T("Ожидает отправки")});
   Save();
  }
  Changed();
  Log(L.T("Получен заказ от ")+Str(e,"user_name")+L.T(". Источник: Twitch."));
 }
 public async Task Recover() {
  string cursor="";
  do {
   var d=await Api(false,"GET","channel_points/custom_rewards/redemptions?broadcaster_id="+Q(S.broadcaster)+"&reward_id="+Q(S.reward)+"&status=UNFULFILLED&sort=OLDEST&first=50"+(cursor==""?"":"&after="+Q(cursor)));
   foreach(var e in Data(d)) Accept(e);
   cursor=d.ContainsKey("pagination")?Str(D(d,"pagination"),"cursor"):"";
   if(S.orders.Count(o=>!Finished(o))>=100)break;
  } while(cursor!="");
 }
 internal static string ReadRequestLine(Stream stream){
  var bytes=new List<byte>();var timer=Stopwatch.StartNew();
  while(bytes.Count<8192 && timer.Elapsed<TimeSpan.FromSeconds(5)){
   if(stream.CanTimeout)stream.ReadTimeout=Math.Max(1,5000-(int)timer.ElapsedMilliseconds);
   int b=stream.ReadByte();if(b<0)throw new IOException("Incomplete HTTP request");
   if(b==10)return Encoding.ASCII.GetString(bytes.ToArray()).TrimEnd('\r');bytes.Add((byte)b);
  }
  throw new IOException("HTTP request line too long or timed out");
 }
 public void Defer(Order o,Exception error){
  o.attempts++;o.nextAttempt=Now+Math.Min(120,15*o.attempts);
  if(o.attempts>=3){
   if(o.status=="pending"){o.status="reject";o.notify=true;o.attempts=0;o.note=L.Language=="en"?"The request source remained unavailable. Points refunded.":"Источник заказа остаётся недоступен. Баллы возвращены.";}
   else{o.status="manual";o.note=L.Language=="en"?"Twitch could not finalize this request. Resolve it manually in Twitch.":"Twitch не подтвердил обработку заказа. Разрешите его вручную в Twitch.";}
  }
  Save();Changed();Log(error.Message);
 }
 public async Task Mark(Order o,string status) {
  if(o.broadcaster!=S.broadcaster) throw new Exception(L.T("Этот заказ принадлежит другому Twitch-аккаунту. Подключите исходный аккаунт."));
  await Api(false,"PATCH","channel_points/custom_rewards/redemptions?broadcaster_id="+Q(o.broadcaster)+"&reward_id="+Q(o.reward)+"&id="+Q(o.id),new {status=status});
 }
 public async Task ProcessOrder(Order o, CancellationToken ct=default(CancellationToken)) {
  if(o.status=="pending") {
   try { if(string.IsNullOrEmpty(o.uri))o.uri=await ResolveTrack(o.input,ct); var metadata=await ValidateOrderTrack(o.uri);if(metadata!=null){o.trackTitle=Str(metadata,"name");o.trackArtist=string.Join(", ",Items(metadata["artists"]).Select(a=>Str(a,"name")));} }
   catch(ResolveError ex) { o.status="reject"; o.note=ex.Message; o.notify=true; Save(); }
   if(o.status=="pending") {
    // Acquire/refresh token before entering the ambiguous non-idempotent request.
    await Access(true);
    o.status="sending"; o.note=L.T("Отправляется в Spotify"); Save(); Changed();
    int retryDelay=0;
    try {
     await Api(true,"POST","me/player/queue?uri="+Q(o.uri));
     o.status="queued"; o.note=L.T("Добавлен в очередь Spotify");
    } catch(ApiError ex) {
     if(ex.Status==429) { o.status="pending"; o.note=ex.Message; retryDelay=Math.Min(300,Math.Max(1,ex.Retry)); }
     else if(ex.Status>=500) { o.status="uncertain"; o.note=L.T("Ответ Spotify неоднозначен. Проверьте очередь вручную."); }
     else { o.status="reject"; o.note=ex.Message; }
    } catch(Exception) { o.status="uncertain"; o.note=L.T("Соединение оборвалось. Трек мог добавиться; проверьте очередь Spotify вручную."); }
    Save(); Changed();
    if(o.status=="queued") {try {TrackQueued();}catch(Exception){/* Notifications must not change an accepted queue request. */}}
    if(retryDelay>0) { await Task.Delay(TimeSpan.FromSeconds(retryDelay),ct); return; }
   }
  }
  if(o.status=="queued" || o.status=="reject") {
   string action=o.status=="queued"?"FULFILLED":"CANCELED";
   try { await Mark(o,action); o.status=action=="FULFILLED"?"done":"canceled"; Save(); Log(o.user+": "+o.note); }
   catch(ApiError ex) {
    if(ex.Status==400 || ex.Status==404) { o.status="manual"; o.note+=L.T("; статус награды изменён вне программы — проверьте Twitch."); Save(); }
    else throw;
   }
   Changed();
  }
  if(o.status=="canceled" && o.notify && !o.notificationAttempted) await NotifyRejected(o);
 }
 public void Dispose() { http.Dispose(); auth.Dispose(); }
}
}

