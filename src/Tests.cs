using System;
using System.IO;
using System.Collections.Generic;
using System.Threading.Tasks;
using System.Reflection;
using System.Windows.Forms;
using System.Threading;
using System.Linq;
using System.IO.Compression;
using System.Net;
using System.Net.Http;
using System.Security.Cryptography;

namespace TrackBridge {
static class Tests {
 static int count;
 static void Check(bool ok,string message) {if(!ok)throw new Exception(message);count++;}
 public static int Run() {
  string report=Path.Combine(AppDomain.CurrentDomain.BaseDirectory,"self-test.txt");
  try {TestLoginControls();SynchronizationContext.SetSynchronizationContext(null);RunAsync().GetAwaiter().GetResult();File.WriteAllText(report,"PASS: "+count+" checks. No real accounts or network calls used.");return 0;}
  catch(Exception ex){File.WriteAllText(report,"FAIL: "+ex);return 1;}
 }
 static void TestLoginControls() {
  string dir=Path.Combine(AppDomain.CurrentDomain.BaseDirectory,"diagnostics","ui-"+Guid.NewGuid().ToString("N"));
  using(var e=new Engine(dir))using(var form=new MainWindow(e,true)) form.VerifyControls(Check);
  using(var reopened=new Engine(dir))Check(reopened.S.maxTrackMinutes==17 && reopened.S.blockExplicit && reopened.S.blockedTracks.Contains("Track B") && reopened.S.blockedArtists.Contains("Artist B"),"moderation settings survive application restart");
  using(var host=new Form()){host.Size=new System.Drawing.Size(320,240);host.Show();using(var transition=new PageTransition(new System.Drawing.Bitmap(300,200),new System.Drawing.Bitmap(300,200))){host.Controls.Add(transition);transition.Start();var elapsed=System.Diagnostics.Stopwatch.StartNew();while(!transition.IsDisposed && elapsed.ElapsedMilliseconds<1000){Application.DoEvents();Thread.Sleep(5);}Check(transition.IsDisposed,"page transition completes and releases its overlay");}host.Close();}
 }
 static Dictionary<string,object> Event(string id,string input) {
  return new Dictionary<string,object>{{"id",id},{"reward",new Dictionary<string,object>{{"id","r"}}},{"user_name","viewer"},{"user_input",input}};
 }
 static async Task RunAsync() {
  await TestCommands();await TestModeration();TestCleanup();TestLimits();await TestSoundNotifications();TestCustomSound();AudioDependencies.Initialize();TestAudioConversion();TestDefaultLanguage();
  var request=Engine.Obj(Engine.Json(new {broadcaster_id="test",sender_id="test",message="check"}));
  Check(Engine.Str(request,"broadcaster_id")=="test" && Engine.Str(request,"message")=="check","anonymous API body keeps wire names");
  var settingsRoundTrip=new System.Web.Script.Serialization.JavaScriptSerializer().Deserialize<Settings>(Engine.Json(new Settings()));
  Check(settingsRoundTrip.spotify.access_token=="" && settingsRoundTrip.twitch.access_token=="","settings JSON schema preserved");
  await TestUpdates();
  await TestUnsupportedSource();
  await TestRecovery();
  await TestYouTubeResolution();
  var parsed=Engine.Obj("{\"data\":[{\"id\":\"reward-1\",\"title\":\"Track\",\"cost\":500}],\"pagination\":{}}");
  Check(parsed["data"] is System.Collections.ArrayList,"real JSON decoder returns ArrayList");
  Check(Engine.Str(Engine.Data(parsed).Single(),"id")=="reward-1","decode actual reward response");
  Check(!Engine.Data(Engine.Obj("{\"data\":[]}")).Any(),"decode empty reward list");
  var legacy=new Dictionary<string,object>{{"data",new object[]{new Dictionary<string,object>{{"id","array-reward"}}}}};
  Check(Engine.Str(Engine.Data(legacy).Single(),"id")=="array-reward","accept object array too");
  Check(Engine.Data(Engine.Obj("{\"data\":[{\"id\":\"one\"},{\"id\":\"two\"}]}")).Count()==2,"decode multiple rewards");
  foreach(var invalid in new[]{"{}","{\"data\":null}","{\"data\":\"bad\"}","{\"data\":{}}","{\"data\":[42]}"}) {
   bool failed=false;try{Engine.Data(Engine.Obj(invalid)).ToList();}catch(Exception){failed=true;}
   Check(failed,"reject malformed data response");
  }
  const string id="4uLU6hMCjMI75M1A2tKUQC";
  Check(Engine.Track("https://open.spotify.com/track/"+id+"?si=123")=="spotify:track:"+id,"track url");
  Check(Engine.Track("https://open.spotify.com/intl-ru/track/"+id)=="spotify:track:"+id,"localized url");
  Check(Engine.Track("spotify:track:"+id)=="spotify:track:"+id,"uri");
  foreach(string bad in new[]{"https://open.spotify.com.evil/track/"+id,"http://open.spotify.com/track/"+id,"https://open.spotify.com/album/"+id,"https://spotify.link/abcd","https://open.spotify.com/track/abc","https://a@open.spotify.com/track/"+id}){
   bool rejected=false;try{Engine.Track(bad);}catch{rejected=true;}Check(rejected,"reject "+bad);
  }
  string dir=Path.Combine(AppDomain.CurrentDomain.BaseDirectory,"diagnostics","TrackBridge-tests-"+Guid.NewGuid().ToString("N"));
  using(var e=new Engine(dir)) {
   e.S.maxTrackMinutes=0;
   e.S.reward="r"; e.S.broadcaster="b"; e.S.spotify.access_token="test";e.S.spotify.expires=Engine.Now+3600;
   int adds=0,marks=0; bool failMark=true;
   e.TestApi=(sp,m,endpoint,b)=>{
    if(sp)adds++;else{marks++;if(failMark){failMark=false;throw new ApiError(503,0,"test");}}
    return Task.FromResult(new Dictionary<string,object>());
   };
   e.Accept(Event("one","spotify:track:"+id));e.Accept(Event("one","spotify:track:"+id));Check(e.S.orders.Count==1,"dedup");
   var o=e.S.orders[0];try{await e.ProcessOrder(o);}catch(ApiError){}
   Check(o.status=="queued" && adds==1,"Spotify success retained despite Twitch failure");
   await e.ProcessOrder(o);Check(o.status=="done" && adds==1 && marks==2,"fulfillment retry does not duplicate song");
   e.Accept(Event("bad","https://example.org/x"));await e.ProcessOrder(e.S.orders[1]);Check(e.S.orders[1].status=="canceled" && adds==1,"invalid link refund");
   e.Accept(Event("unknown","spotify:track:"+id));e.TestApi=(sp,m,p,b)=>{throw new IOException("timeout");};
   await e.ProcessOrder(e.S.orders[2]);Check(e.S.orders[2].status=="uncertain","ambiguous send quarantined");
   await e.ProcessOrder(e.S.orders[2]);Check(e.S.orders[2].status=="uncertain","ambiguous order never resent");
   e.Accept(Event("crash","spotify:track:"+id));e.S.orders[3].status="sending";e.Save();
  }
  using(var e=new Engine(dir)) {
   e.S.maxTrackMinutes=0;
   Check(e.S.orders.Count==4,"encrypted persistence");Check(e.S.orders[3].status=="uncertain","crash during send");
   e.Accept(Event("one","spotify:track:"+id));Check(e.S.orders.Count==4,"dedup survives restart");
  }
  // Test directory retained in work for diagnostics.
 }
 static async Task TestRecovery() {
  string dir=Path.Combine(AppDomain.CurrentDomain.BaseDirectory,"diagnostics","TrackBridge-recovery-tests-"+Guid.NewGuid().ToString("N"));
  using(var e=new Engine(dir)) {
   e.S.maxTrackMinutes=0;
   e.S.reward="r";e.S.broadcaster="b";
   int accepted=0;e.Log=s=>{if(s.Contains("Получен заказ"))accepted++;};
   e.TestApi=(sp,method,url,body)=> {
    bool page2=url.Contains("after=next");
    var response=new Dictionary<string,object>{{"data",new object[]{Event(page2?"poll2":"poll1","spotify:track:4uLU6hMCjMI75M1A2tKUQC")}},{"pagination",page2?new Dictionary<string,object>():new Dictionary<string,object>{{"cursor","next"}}}};
    return Task.FromResult(response);
   };
   await e.Recover();Check(e.S.orders.Count==2 && accepted==2,"polling recovers missed Twitch purchases across pages");
   e.Accept(Event("poll1","spotify:track:4uLU6hMCjMI75M1A2tKUQC"));await e.Recover();
   Check(e.S.orders.Count==2 && accepted==2,"polling and live event do not duplicate received purchases");
  }
 }
 static async Task TestUnsupportedSource() {
  string dir=Path.Combine(AppDomain.CurrentDomain.BaseDirectory,"diagnostics","removed-source-"+Guid.NewGuid().ToString("N"));
  using(var e=new Engine(dir)) {
   e.S.maxTrackMinutes=0;
   e.S.broadcaster="b";e.S.reward="r";int calls=0;
   e.TestApi=(sp,m,url,body)=>{Check(!sp,"removed unsupported source never queries Spotify");calls++;return Task.FromResult(new Dictionary<string,object>());};
   e.Accept(Event("removed-yandex","https://example.org/track/1"));await e.ProcessOrder(e.S.orders[0]);
   Check(e.S.orders[0].status=="canceled" && e.S.orders[0].note.Contains("Нужна ссылка") && calls==1,"unsupported link rejected with refund");
  }
 }
 class UpdateHttp : HttpMessageHandler {
  readonly string json;readonly byte[] zip,manifest,signature;
  public UpdateHttp(string text,byte[] bytes,byte[] manifestBytes,byte[] signatureBytes){json=text;zip=bytes;manifest=manifestBytes;signature=signatureBytes;}
  protected override Task<HttpResponseMessage> SendAsync(HttpRequestMessage request,CancellationToken ct) {
   ct.ThrowIfCancellationRequested();
   byte[] body=request.RequestUri.AbsolutePath.EndsWith("TrackBridge-update.json")?manifest:request.RequestUri.AbsolutePath.EndsWith("TrackBridge-update.sig")?signature:zip;
   return Task.FromResult(new HttpResponseMessage(HttpStatusCode.OK){RequestMessage=request,Content=request.RequestUri.Host=="api.github.com"?(HttpContent)new StringContent(json):new ByteArrayContent(body)});
  }
 }
 static byte[] UpdateZip(bool invalid=false) {
  using(var memory=new MemoryStream()) {
   using(var zip=new ZipArchive(memory,ZipArchiveMode.Create,true)) {
    foreach(string file in new[]{"TrackBridge.exe","README.md","README.html","LICENSE.txt","THIRD-PARTY-NOTICES.txt","CHANGELOG.md"}) {
     var entry=zip.CreateEntry(file);using(var output=entry.Open()) {
      byte[] bytes=file=="TrackBridge.exe"?File.ReadAllBytes(Application.ExecutablePath):System.Text.Encoding.UTF8.GetBytes("new-"+file);output.Write(bytes,0,bytes.Length);
     }
    }
    if(invalid){var entry=zip.CreateEntry("../data/settings.bin");using(var output=entry.Open())output.WriteByte(1);}
   }
   return memory.ToArray();
  }
 }
 static async Task TestUpdates() {
  Check(Updates.Compare("v1.8.0-beta.2","1.8.0-beta.1")>0,"numeric beta ordering");
  Check(Updates.Compare("1.8.0","1.8.0-beta.99")>0,"stable newer than same beta");
  Check(Updates.Compare("1.10.0","1.9.9")>0,"numeric minor ordering");
  Check(Updates.SelectRelease("[]",Updates.Current)==null,"no release is no update");
  Check(Updates.SelectRelease("[{\"tag_name\":\"v1.7.0-beta.1\",\"draft\":false}]",Updates.Current)==null,"never downgrade installed version");
  Check(Updates.SelectRelease("[{\"tag_name\":\"v9.0.0\",\"draft\":true},{\"tag_name\":\"bad-tag\",\"draft\":false}]",Updates.Current)==null,"ignore drafts and invalid tags");
  Check(!Updates.TrustedUrl("https://github.com/evil/repo/releases/download/v1.8.0/x.zip","v1.8.0","x.zip"),"reject foreign download repository");
  string dir=Path.Combine(AppDomain.CurrentDomain.BaseDirectory,"diagnostics","updates-"+Guid.NewGuid().ToString("N"));Directory.CreateDirectory(dir);
  byte[] bytes=UpdateZip();string archive=Path.Combine(dir,"fixture.zip");File.WriteAllBytes(archive,bytes);
  bool rejected=false;try{Updates.Extract(archive,Path.Combine(dir,"bad-hash"),new string('0',64));}catch(IOException){rejected=true;}Check(rejected,"reject corrupt package hash");
  byte[] unsafeZip=UpdateZip(true);string unsafeArchive=Path.Combine(dir,"unsafe.zip");File.WriteAllBytes(unsafeArchive,unsafeZip);
  rejected=false;try{Updates.Extract(unsafeArchive,Path.Combine(dir,"bad-path"),Updates.Hash(unsafeZip));}catch(IOException){rejected=true;}Check(rejected,"reject zip traversal and unexpected settings entry");
  string tag="v"+Updates.Current,asset="TrackBridge-"+Updates.Current+"-win.zip";
  byte[] manifestBytes=System.Text.Encoding.UTF8.GetBytes(Engine.Json(new {version=tag.Substring(1),archive=asset,sha256=Updates.Hash(bytes)})),signatureBytes;
  string keyPath=Environment.GetEnvironmentVariable("TRACKBRIDGE_TEST_SIGNING_KEY");if(string.IsNullOrEmpty(keyPath))throw new Exception("Signed updater checks require the private build environment.");
  using(var rsa=new RSACryptoServiceProvider()){rsa.PersistKeyInCsp=false;rsa.FromXmlString(System.Text.Encoding.UTF8.GetString(ProtectedData.Unprotect(File.ReadAllBytes(keyPath),null,DataProtectionScope.CurrentUser)));signatureBytes=System.Text.Encoding.ASCII.GetBytes(Convert.ToBase64String(rsa.SignData(manifestBytes,CryptoConfig.MapNameToOID("SHA256"))));}
  Check(Engine.Str(Updates.SignedManifest(manifestBytes,System.Text.Encoding.ASCII.GetString(signatureBytes)),"archive")==asset,"publisher signature accepted");
  bool tampered=false;try{Updates.SignedManifest(System.Text.Encoding.UTF8.GetBytes("{}"),System.Text.Encoding.ASCII.GetString(signatureBytes));}catch(CryptographicException){tampered=true;}catch(IOException){tampered=true;}Check(tampered,"tampered signature rejected");
  var downloadAssets=new[]{new {name=asset,state="uploaded",browser_download_url=Updates.Repository+"/releases/download/"+tag+"/"+asset,digest="sha256:"+Updates.Hash(bytes)},new {name="TrackBridge-update.json",state="uploaded",browser_download_url=Updates.Repository+"/releases/download/"+tag+"/TrackBridge-update.json",digest=""},new {name="TrackBridge-update.sig",state="uploaded",browser_download_url=Updates.Repository+"/releases/download/"+tag+"/TrackBridge-update.sig",digest=""}};
  string json=Engine.Json(new[]{new {tag_name=tag,draft=false,assets=downloadAssets}});
  Updates.TestHandler=()=>new UpdateHttp(json,bytes,manifestBytes,signatureBytes);Updates.TestInstalledVersion="0.0.0";
  try {
   string stage=await Updates.Prepare(dir,s=>{},CancellationToken.None);
   Check(stage!=null && File.Exists(Path.Combine(stage,"payload","TrackBridge.exe")),"automatic GitHub check downloads and validates package");
   string settings=Path.Combine(dir,"data");Directory.CreateDirectory(settings);byte[] soundFixture=NotificationSound.WaveForPlayback(20,null);File.WriteAllBytes(Path.Combine(settings,"request-notification.wav"),soundFixture);File.WriteAllText(Path.Combine(settings,"settings.bin"),"private-token-fixture");
   string exe=Path.Combine(dir,"TrackBridge.exe");File.WriteAllText(exe,"old-exe");File.WriteAllText(Path.Combine(dir,"README.md"),"old-readme");
   Updates.ApplyFiles(stage,exe);
   Check(File.ReadAllBytes(exe).SequenceEqual(File.ReadAllBytes(Application.ExecutablePath)),"verified executable installed");
   Check(File.ReadAllText(Path.Combine(settings,"settings.bin"))=="private-token-fixture","update preserves account data");Check(File.ReadAllBytes(Path.Combine(settings,"request-notification.wav")).SequenceEqual(soundFixture),"update preserves custom sound file");
   Check(File.ReadAllText(Path.Combine(dir,"README.md"))=="new-README.md","update installs packaged documents");
   string rollback=await Updates.Prepare(dir,s=>{},CancellationToken.None);File.WriteAllText(exe,"previous-exe");File.WriteAllText(Path.Combine(dir,"README.md"),"previous-readme");
   bool failed=false;try{Updates.ApplyFiles(rollback,exe,name=>{if(name=="README.html")throw new IOException("Simulated disk failure");});}catch(IOException){failed=true;}
   Check(failed && File.ReadAllText(exe)=="previous-exe" && File.ReadAllText(Path.Combine(dir,"README.md"))=="previous-readme","installation failure rolls back changed files");
   Check(File.ReadAllText(Path.Combine(settings,"settings.bin"))=="private-token-fixture","rollback preserves account data");Check(File.ReadAllBytes(Path.Combine(settings,"request-notification.wav")).SequenceEqual(soundFixture),"rollback preserves custom sound file");
  } finally{Updates.TestHandler=null;Updates.TestInstalledVersion=null;}
 }
 static Dictionary<string,object> SearchResult(params Dictionary<string,object>[] tracks) {
  return new Dictionary<string,object>{{"tracks",new Dictionary<string,object>{{"items",tracks}}}};
 }
 static Dictionary<string,object> Playback(string uri,long progress=5000){return new Dictionary<string,object>{{"is_playing",true},{"progress_ms",progress},{"item",new Dictionary<string,object>{{"type","track"},{"uri",uri},{"name","A song"},{"artists",new object[]{new Dictionary<string,object>{{"name","Artist"}}}}}}};}
 static Dictionary<string,object> Chat(string id,string command){return new Dictionary<string,object>{{"chatter_user_id",id},{"message",new Dictionary<string,object>{{"text",command}}}};}
 static async Task TestCommands(){
  string uri="spotify:track:4uLU6hMCjMI75M1A2tKUQC",other="spotify:track:1234567890123456789012";
  var votes=new VoteSession();votes.Observe(Playback(uri));Check(votes.Add("one") && !votes.Add("one") && votes.Count==1,"one vote per Twitch user ID");votes.Observe(Playback(other));Check(votes.Count==0,"track change resets votes");
  votes.Add("one");votes.Observe(Playback(other,0));Check(votes.Count==0,"repeated track instance resets votes");votes.SkipLatched=true;votes.Invalidate();votes.Observe(Playback(other,500));Check(votes.SkipLatched,"unknown playback does not unlock ambiguous skip");
  string dir=Path.Combine(AppDomain.CurrentDomain.BaseDirectory,"diagnostics","commands-"+Guid.NewGuid().ToString("N"));
  using(var e=new Engine(dir)){
   e.S.maxTrackMinutes=0;
   e.S.broadcaster="b";int skips=0,reads=0,viewerCount=10;string current=uri;bool switchAtThreshold=false,failSkip=false;var messages=new List<string>();
   e.TestApi=(sp,method,path,body)=>{
    if(path.StartsWith("streams?"))return Task.FromResult(Engine.Obj("{\"data\":[{\"viewer_count\":"+viewerCount+"}]}"));
    if(path=="me/player"){reads++;if(switchAtThreshold && reads==6)current=other;return Task.FromResult(Playback(current));}
    if(path=="me/player/next"){skips++;if(failSkip)throw new IOException("lost response");return Task.FromResult(new Dictionary<string,object>());}
    if(path=="chat/messages"){messages.Add(Engine.Str(Engine.Obj(Engine.Json(body)),"message"));return Task.FromResult(Engine.Obj("{\"data\":[{\"is_sent\":true}]}"));}
    throw new Exception("Unexpected command request");
   };
   var commands=new ChatCommands(e);await commands.Handle(Chat("reader","!song"));Check(messages.Last().Contains("https://open.spotify.com/track/4uLU6hMCjMI75M1A2tKUQC"),"song replies with canonical current Spotify link");
   await commands.Handle(Chat("1","!voteskip"));await commands.Handle(Chat("1","!voteskip"));for(int i=2;i<=4;i++)await commands.Handle(Chat(i.ToString(),"!voteskip"));Check(skips==0 && commands.Votes.Count==4,"four unique votes do not skip");await commands.Handle(Chat("5","!voteskip"));Check(skips==1,"five unique viewers trigger one skip");await commands.Handle(Chat("6","!voteskip"));Check(skips==1,"no repeated skip on same track");
   current=other;await commands.Handle(Chat("7","!voteskip"));Check(commands.Votes.Count==1,"new track starts new vote");
   current=uri;reads=0;skips=0;switchAtThreshold=true;commands=new ChatCommands(e);for(int i=1;i<=5;i++)await commands.Handle(Chat(i.ToString(),"!voteskip"));Check(skips==0,"track change before skip cancels old vote");
   switchAtThreshold=false;current=uri;failSkip=true;commands=new ChatCommands(e);for(int i=1;i<=6;i++)await commands.Handle(Chat(i.ToString(),"!voteskip"));Check(skips==1,"lost skip response is not retried");
   failSkip=false;current=uri;commands=new ChatCommands(e);e.S.votingEnabled=false;await commands.Handle(Chat("1","!skip"));Check(commands.Votes.Count==0,"disabled voting ignores skip command");e.S.votingEnabled=true;
   await commands.Handle(Chat("1","!skip"));await commands.ApplyControl("reset");Check(commands.Votes.Count==0,"owner resets votes");
   int before=skips;await commands.ApplyControl("skip");await commands.ApplyControl("skip");Check(skips==before+1,"owner skip is latched against duplicate clicks");
   e.S.skipAutomatic=false;e.S.skipManualVotes=2;commands=new ChatCommands(e);before=skips;await commands.Handle(Chat("1","!skip"));await commands.Handle(Chat("2","!skip"));Check(skips==before+1,"manual low audience threshold");
   e.S.skipAutomatic=true;viewerCount=11;commands=new ChatCommands(e);before=skips;for(int i=1;i<=5;i++)await commands.Handle(Chat(i.ToString(),"!skip"));Check(skips==before,"odd audience rounds threshold up");viewerCount=2;await commands.Handle(Chat("6","!skip"));Check(skips==before+1,"threshold stays fixed during track");
   viewerCount=100;e.S.skipAutomatic=false;e.S.skipManualVotes=1;commands=new ChatCommands(e);before=skips;await commands.Handle(Chat("1","!skip"));Check(skips==before && e.VoteStatus.Contains("50"),"large audience uses automatic mode despite manual setting");
   viewerCount=0;commands=new ChatCommands(e);await commands.Handle(Chat("offline","!skip"));Check(commands.Votes.Count==0,"offline stream rejects voting");
  }
 }
 static async Task TestModeration(){
  string dir=Path.Combine(AppDomain.CurrentDomain.BaseDirectory,"diagnostics","moderation-"+Guid.NewGuid().ToString("N"));
  using(var e=new Engine(dir)){
   const string uri="spotify:track:4uLU6hMCjMI75M1A2tKUQC";
   var track=Song("Artist","Song",uri);track["type"]="track";track["duration_ms"]=600000;track["explicit"]=false;
   e.CheckTrackPolicy(track);Check(true,"exact duration limit accepted");
   track["duration_ms"]=600001;bool blocked=false;try{e.CheckTrackPolicy(track);}catch(ResolveError){blocked=true;}Check(blocked,"overlong track rejected");
   track["duration_ms"]=200000;e.S.blockExplicit=true;track["explicit"]=true;blocked=false;try{e.CheckTrackPolicy(track);}catch(ResolveError){blocked=true;}Check(blocked,"explicit track rejected");
   track["explicit"]=false;e.S.blockedArtists="aRtIsT";blocked=false;try{e.CheckTrackPolicy(track);}catch(ResolveError){blocked=true;}Check(blocked,"artist ban case insensitive");
   e.S.blockedArtists="";e.S.blockedTracks="https://open.spotify.com/track/4uLU6hMCjMI75M1A2tKUQC?si=demo";blocked=false;try{e.CheckTrackPolicy(track);}catch(ResolveError){blocked=true;}Check(blocked,"track link ban");
   e.S.blockedTracks="song";blocked=false;try{e.CheckTrackPolicy(track);}catch(ResolveError){blocked=true;}Check(blocked,"exact title ban");
   e.S.blockedTracks="Son";e.CheckTrackPolicy(track);Check(true,"title bans are exact not substrings");
   e.S.blockedTracks="";e.S.maxTrackMinutes=0;e.S.blockExplicit=false;track.Remove("duration_ms");e.CheckTrackPolicy(track);Check(true,"duration limit can be disabled");
   e.S.maxTrackMinutes=10;bool missing=false;try{e.CheckTrackPolicy(track);}catch(ResolveError){throw;}catch(Exception){missing=true;}Check(missing,"missing duration is temporary failure");
   track["duration_ms"]=700000;e.S.broadcaster="b";e.S.reward="r";e.S.spotify.access_token="fixture";e.S.spotify.expires=Engine.Now+3600;e.S.chatAllowed=true;
   int queued=0,refunds=0;e.TestApi=(sp,method,path,body)=>{if(path.StartsWith("tracks/"))return Task.FromResult(track);if(path.StartsWith("me/player/queue"))queued++;if(method=="PATCH")refunds++;return Task.FromResult(Engine.Obj("{\"data\":[{\"is_sent\":true}]}"));};
   e.Accept(Event("blocked",uri));await e.ProcessOrder(e.S.orders[0]);Check(queued==0 && refunds==1 && e.S.orders[0].status=="canceled","policy rejects before queue and refunds");
   e.Save();
  }
  using(var e=new Engine(dir))Check(e.S.maxTrackMinutes==10 && e.S.votingEnabled,"moderation and voting settings persist");
 }
 static async Task TestSoundNotifications(){
  string dir=Path.Combine(AppDomain.CurrentDomain.BaseDirectory,"diagnostics","sound-"+Guid.NewGuid().ToString("N"));
  using(var e=new Engine(dir)){
   e.S.maxTrackMinutes=0;e.S.reward="r";e.S.broadcaster="b";e.S.spotify.access_token="fixture";e.S.spotify.expires=Engine.Now+3600;
   int signals=0,queueCalls=0;var reached=new TaskCompletionSource<bool>();var response=new TaskCompletionSource<bool>();
   e.TrackQueued=()=>signals++;
   e.TestApi=async(sp,method,endpoint,body)=>{if(sp && method=="POST"){queueCalls++;reached.TrySetResult(true);await response.Task;}return new Dictionary<string,object>();};
   e.Accept(Event("sound-success","spotify:track:4uLU6hMCjMI75M1A2tKUQC"));var request=e.S.orders.Last();var processing=e.ProcessOrder(request);
   await reached.Task;Check(signals==0,"no sound before Spotify responds");response.SetResult(true);await processing;
   Check(signals==1 && request.status=="done" && queueCalls==1,"one sound after successful viewer queue request");await e.ProcessOrder(request);Check(signals==1,"no sound for repeated completed request");
   foreach(int status in new[]{400,401,403,404,429,500}){
    e.TestApi=(sp,method,endpoint,body)=>{if(sp)throw new ApiError(status,0,"fixture");return Task.FromResult(new Dictionary<string,object>());};
    var failed=new Order{id="failure-"+status,broadcaster="b",reward="r",input="fixture",uri=request.uri,status="pending"};e.S.orders.Add(failed);
    // Avoid the production retry delay while preserving the rejected response branch.
    using(var cancel=new CancellationTokenSource()){cancel.Cancel();try{await e.ProcessOrder(failed,cancel.Token);}catch(OperationCanceledException){}}
    Check(signals==1,"no sound for Spotify HTTP "+status);
   }
   e.TestApi=(sp,method,endpoint,body)=>{if(sp)throw new IOException("simulated lost response");return Task.FromResult(new Dictionary<string,object>());};
   var uncertain=new Order{id="sound-uncertain",broadcaster="b",reward="r",uri=request.uri,status="pending"};e.S.orders.Add(uncertain);await e.ProcessOrder(uncertain);Check(signals==1 && uncertain.status=="uncertain","no sound on ambiguous queue response");
   e.TestApi=(sp,method,endpoint,body)=>Task.FromResult(new Dictionary<string,object>());
   var resumed=new Order{id="resumed",broadcaster="b",reward="r",uri=request.uri,status="queued"};e.S.orders.Add(resumed);await e.ProcessOrder(resumed);Check(signals==1 && resumed.status=="done","no sound when resuming Twitch fulfillment");
   e.TrackQueued=()=>{throw new InvalidOperationException("audio device fixture");};var silentFailure=new Order{id="sound-callback-failure",broadcaster="b",reward="r",uri=request.uri,status="pending"};e.S.orders.Add(silentFailure);await e.ProcessOrder(silentFailure);Check(silentFailure.status=="done","sound failure cannot turn accepted request into uncertain");
   e.S.requestSoundEnabled=false;e.S.requestSoundVolume=9;e.Save();
  }
  using(var e=new Engine(dir))Check(!e.S.requestSoundEnabled && e.S.requestSoundVolume==9,"sound preference and volume survive restart");
  var oldSettings=new System.Web.Script.Serialization.JavaScriptSerializer().Deserialize<Settings>("{\"language\":\"en\"}");Check(oldSettings.requestSoundEnabled && oldSettings.requestSoundVolume==20,"legacy settings receive quiet sound defaults");
  var wave=(byte[])typeof(NotificationSound).GetMethod("CreateWave",BindingFlags.Static|BindingFlags.NonPublic).Invoke(null,new object[]{20});
  Check(System.Text.Encoding.ASCII.GetString(wave,0,4)=="RIFF" && BitConverter.ToInt32(wave,4)==wave.Length-8 && BitConverter.ToInt32(wave,24)==22050,"notification has valid PCM WAV header");
  int peak=0;for(int i=44;i<wave.Length;i+=2)peak=Math.Max(peak,Math.Abs((int)BitConverter.ToInt16(wave,i)));Check(peak>0 && peak<5000 && (wave.Length-44)/44100.0<.5,"default signal is short and has low amplitude");
 }
 static int WavePeak(byte[] wave){int peak=0;for(int i=44;i<wave.Length;i+=2)peak=Math.Max(peak,Math.Abs((int)BitConverter.ToInt16(wave,i)));return peak;}
 static void TestCustomSound(){
  string dir=Path.Combine(AppDomain.CurrentDomain.BaseDirectory,"diagnostics","custom-sound-"+Guid.NewGuid().ToString("N"));Directory.CreateDirectory(dir);
  byte[] standard=NotificationSound.WaveForPlayback(20,null);string original=Path.Combine(dir,"My notification.wav");File.WriteAllBytes(original,standard);string stored;
  using(var e=new Engine(Path.Combine(dir,"data"))){
   Check(e.RequestSoundFile==null && e.S.requestSoundName=="","default sound selected for new settings");e.SetRequestSound(original);stored=e.RequestSoundFile;
   Check(stored!=original && File.Exists(stored) && e.S.requestSoundName==Path.GetFileName(original),"custom sound copied into persistent data with a display filename");
   Check(NotificationSound.ImportWave(stored).SequenceEqual(standard),"import retains valid PCM samples");File.Delete(original);
   Check(NotificationSound.WaveForPlayback(20,stored).Length==standard.Length,"custom sound remains playable after original file removed");
   Check(WavePeak(NotificationSound.WaveForPlayback(10,stored))<WavePeak(NotificationSound.WaveForPlayback(20,stored)) && WavePeak(NotificationSound.WaveForPlayback(40,stored))<8000,"custom sound volume applies with bounded peak");
   Check(WavePeak(NotificationSound.WaveForPlayback(0,stored))==0,"zero volume mutes custom WAV");
   byte[] broken=(byte[])standard.Clone();broken[0]=0;string invalid=Path.Combine(dir,"broken.wav");File.WriteAllBytes(invalid,broken);bool rejected=false;try{e.SetRequestSound(invalid);}catch(IOException){rejected=true;}
   Check(rejected && e.RequestSoundFile==stored && File.ReadAllBytes(stored).SequenceEqual(standard),"failed import leaves chosen sound and its saved bytes unchanged");
  }
  using(var e=new Engine(Path.Combine(dir,"data"))){
   Check(e.RequestSoundFile==stored && e.S.requestSoundName=="My notification.wav","custom selection persists after restart");File.WriteAllBytes(stored,new byte[]{1,2,3});
   Check(NotificationSound.WaveForPlayback(20,stored).SequenceEqual(standard),"corrupt saved sound falls back to the default");File.Delete(stored);
   Check(NotificationSound.WaveForPlayback(20,stored).SequenceEqual(standard),"missing saved sound falls back to the default");e.ResetRequestSound();Check(e.RequestSoundFile==null,"reset selects the standard signal");
  }
  using(var e=new Engine(Path.Combine(dir,"data")))Check(e.RequestSoundFile==null,"standard selection persists after restart");
  for(int mode=0;mode<7;mode++){
   byte[] invalid=(byte[])standard.Clone();
   if(mode==0)invalid=invalid.Take(invalid.Length-2).ToArray();
   if(mode==1)invalid[20]=3;
   if(mode==2)invalid[22]=3;
   if(mode==3)Array.Copy(BitConverter.GetBytes(0),0,invalid,24,4);
   if(mode==4)Array.Copy(BitConverter.GetBytes(uint.MaxValue),0,invalid,40,4);
   if(mode==5){invalid=new byte[44+22050*2*6];Array.Copy(standard,invalid,44);Array.Copy(BitConverter.GetBytes(invalid.Length-8),0,invalid,4,4);Array.Copy(BitConverter.GetBytes(invalid.Length-44),0,invalid,40,4);}
   if(mode==6)invalid=new byte[2*1024*1024+1];
   string bad=Path.Combine(dir,"invalid-"+mode+".wav");File.WriteAllBytes(bad,invalid);bool rejected=false;try{NotificationSound.ImportWave(bad);}catch(IOException){rejected=true;}Check(rejected,"invalid custom WAV rejected: "+mode);
  }
  byte[] stereo=(byte[])standard.Clone();Array.Copy(BitConverter.GetBytes((short)2),0,stereo,22,2);Array.Copy(BitConverter.GetBytes(22050*4),0,stereo,28,4);Array.Copy(BitConverter.GetBytes((short)4),0,stereo,32,2);string stereoFile=Path.Combine(dir,"stereo.wav");File.WriteAllBytes(stereoFile,stereo);Check(NotificationSound.ImportWave(stereoFile).SequenceEqual(stereo),"stereo PCM sound supported");
  byte[] withMetadata;using(var metadata=new MemoryStream())using(var writer=new BinaryWriter(metadata)){writer.Write(standard,0,36);writer.Write(System.Text.Encoding.ASCII.GetBytes("JUNK"));writer.Write(4);writer.Write(new byte[]{1,2,3,4});writer.Write(standard,36,standard.Length-36);withMetadata=metadata.ToArray();}Array.Copy(BitConverter.GetBytes(withMetadata.Length-8),0,withMetadata,4,4);string metadataFile=Path.Combine(dir,"metadata.wav");File.WriteAllBytes(metadataFile,withMetadata);Check(NotificationSound.ImportWave(metadataFile).SequenceEqual(standard),"import strips non-audio metadata chunks");
  byte[] silence=(byte[])standard.Clone();Array.Clear(silence,44,silence.Length-44);string silent=Path.Combine(dir,"silence.wav");File.WriteAllBytes(silent,silence);Check(WavePeak(NotificationSound.WaveForPlayback(40,silent))==0,"silent file does not cause gain errors");
 }
 static void TestDefaultLanguage(){var before=Thread.CurrentThread.CurrentUICulture;try{foreach(string culture in new[]{"ru-RU","en-US","de-DE"}){Thread.CurrentThread.CurrentUICulture=new System.Globalization.CultureInfo(culture);Check(new Settings().language==(culture=="ru-RU"?"ru":"en"),"system language default "+culture);}}finally{Thread.CurrentThread.CurrentUICulture=before;}}
 [System.Runtime.CompilerServices.MethodImpl(System.Runtime.CompilerServices.MethodImplOptions.NoInlining)]
 static void TestAudioConversion(){
  string dir=Path.Combine(AppDomain.CurrentDomain.BaseDirectory,"diagnostics","audio-import-"+Guid.NewGuid().ToString("N"));Directory.CreateDirectory(dir);
  string wav=Path.Combine(dir,"tone.wav"),mp3=Path.Combine(dir,"tone.mp3");
  var format=new NAudio.Wave.WaveFormat(44100,16,2);using(var writer=new NAudio.Wave.WaveFileWriter(wav,format)){byte[] data=new byte[format.AverageBytesPerSecond*7];for(int i=0;i<data.Length/4;i++){short sample=(short)(Math.Sin(2*Math.PI*440*i/44100)*3000);byte[] bytes=BitConverter.GetBytes(sample);data[i*4]=data[i*4+2]=bytes[0];data[i*4+1]=data[i*4+3]=bytes[1];}writer.Write(data,0,data.Length);}
  using(var input=new NAudio.Wave.WaveFileReader(wav))NAudio.Wave.MediaFoundationEncoder.EncodeToMp3(input,mp3,128000);
  byte[] original=File.ReadAllBytes(mp3);using(var e=new Engine(Path.Combine(dir,"data"))){
   foreach(string source in new[]{wav,mp3}){e.SetRequestSound(source);byte[] converted=NotificationSound.ImportWave(e.RequestSoundFile);Check(BitConverter.ToInt32(converted,24)==22050 && BitConverter.ToInt16(converted,22)==1 && BitConverter.ToInt16(converted,34)==16,"converted audio is mono PCM16 22050: "+Path.GetExtension(source));Check(converted.Length>22050*2*4 && converted.Length<=44+22050*2*5,"import trims audio to five seconds: "+Path.GetExtension(source));Check(WavePeak(converted)>100,"decoded audio contains tone");}
   Check(original.SequenceEqual(File.ReadAllBytes(mp3)),"MP3 source remains unchanged");Check(e.S.requestSoundName=="tone.mp3","original audio filename retained");
   string saved=e.RequestSoundFile;byte[] prior=File.ReadAllBytes(saved);string bad=Path.Combine(dir,"not-audio.mp3");File.WriteAllText(bad,"not an audio file");bool rejected=false;try{e.SetRequestSound(bad);}catch(IOException){rejected=true;}Check(rejected && prior.SequenceEqual(File.ReadAllBytes(saved)) && e.S.requestSoundName=="tone.mp3","failed conversion preserves selected sound");
  }
  using(var e=new Engine(Path.Combine(dir,"data")))Check(e.S.requestSoundName=="tone.mp3" && NotificationSound.ImportWave(e.RequestSoundFile).Length>44,"converted audio selection survives restart");
 }
 static void TestCleanup(){
  string dir=Path.Combine(AppDomain.CurrentDomain.BaseDirectory,"diagnostics","cleanup-"+Guid.NewGuid().ToString("N"));
  using(var e=new Engine(dir)){
   var first=new Order{id="first",uri="spotify:track:4uLU6hMCjMI75M1A2tKUQC",status="done"};
   var second=new Order{id="second",uri="spotify:track:1234567890123456789012",status="done"};
   var pending=new Order{id="pending",uri=second.uri,status="pending"};e.S.orders.AddRange(new[]{first,second,pending});e.S.recentReceipts.Add("b:first");
   Check(e.HidePlayedOrders()==0,"queue acceptance alone is not playback");e.ObserveOrderedPlayback(first.uri,true);Check(!e.CanHideOrder(first),"currently playing request protected");e.ObserveOrderedPlayback("",false);Check(!first.played,"missing playback response is not completion");e.ObserveOrderedPlayback(first.uri,true,true);Check(!e.CanHideOrder(first),"repeated current URI remains protected after a playback reset");e.ObserveOrderedPlayback(second.uri,true);
   Check(first.played && !second.played,"observed track transition marks only previous request");Check(e.HidePlayedOrders()==1 && first.historyHidden && !second.historyHidden && !pending.historyHidden,"cleanup preserves current and pending requests");Check(e.S.recentReceipts.Contains("b:first"),"cleanup retains deduplication receipts");
   Check(e.HideSelectedOrders(new[]{second.id,pending.id})==2 && second.historyHidden && pending.historyHidden && second.status=="done" && pending.status=="pending","manual hide handles current and pending rows without changing backend state");Check(e.HideSelectedOrders(new[]{second.id,pending.id})==0,"manual hide is idempotent");e.S.autoHidePlayed=true;e.Save();
  }
  using(var e=new Engine(dir))Check(e.S.autoHidePlayed && e.S.orders[0].historyHidden,"cleanup preference and hidden history persist");
  Check(Updates.Compare("1.9.0.1-beta.1","1.9.0-beta.1")>0,"four-part interface versions order correctly");Check(Updates.Compare("2.0.0","2.0.0-beta.99")>0,"2.0 stable newer than beta");
 }
 static void TestLimits(){
  bool rejected=false;try{Engine.ReadRequestLine(new MemoryStream(new byte[9000]));}catch(IOException){rejected=true;}Check(rejected,"OAuth line length bounded");
  string dir=Path.Combine(AppDomain.CurrentDomain.BaseDirectory,"diagnostics","limits-"+Guid.NewGuid().ToString("N"));
  using(var e=new Engine(dir)){
   e.S.maxTrackMinutes=0;
   e.S.broadcaster="b";e.S.reward="r";
   for(int i=0;i<4;i++)e.Accept(Event("limit-"+i,"spotify:track:4uLU6hMCjMI75M1A2tKUQC"));Check(e.S.orders[3].status=="reject","viewer active order cap");
   var pending=e.S.orders[0];e.Defer(pending,new IOException("offline"));Check(pending.nextAttempt>Engine.Now,"failed request deferred");e.Defer(pending,new IOException("offline"));e.Defer(pending,new IOException("offline"));Check(pending.status=="reject","bounded retry eventually refunds");
   for(int i=0;i<510;i++)e.S.orders.Add(new Order{id="done-"+i,broadcaster="b",status="done"});e.Save();Check(e.S.orders.Count(o=>o.status=="done")==500,"completed history bounded");
  }
 }
 static Dictionary<string,object> Song(string artist,string title,string uri) {
  return new Dictionary<string,object>{{"name",title},{"uri",uri},{"artists",new object[]{new Dictionary<string,object>{{"name",artist}}}}};
 }
 static async Task TestYouTubeResolution() {
  const string video="dQw4w9WgXcQ",uri="spotify:track:4uLU6hMCjMI75M1A2tKUQC";
  foreach(var link in new[]{"https://youtu.be/"+video+"?si=test","https://www.youtube.com/watch?v="+video,"https://music.youtube.com/watch?v="+video+"&list=abc","https://m.youtube.com/watch?v="+video,"https://youtube.com/shorts/"+video})Check(Engine.YouTubeId(link)==video,"YouTube link "+link);
  foreach(var link in new[]{"https://youtube.com.evil/watch?v="+video,"http://youtu.be/"+video,"https://user@youtu.be/"+video,"https://youtu.be/abc","https://youtube.com/playlist?list=abc","https://youtu.be:8080/"+video})Check(Engine.YouTubeId(link)==null,"reject unsafe video link");
  var song=Song("Rick Astley","Never Gonna Give You Up",uri);
  Check(Engine.MatchTrack("Rick Astley - Never Gonna Give You Up (Official Music Video)","RickAstleyVEVO",new[]{song})==uri,"match artist title");
  Check(Engine.MatchTrack("Never Gonna Give You Up","Rick Astley - Topic",new[]{song})==uri,"match Topic upload");
  Check(Engine.MatchTrack("Rick Astley – Never Gonna Give You Up [Official Audio]","Uploader",new[]{song,song})==uri,"equivalent album reissues");
  foreach(string suffix in new[]{"(Official Video) (4K Remaster)","(Live)","(Remix)","(Cover)","[Official Video] [4K Remaster]","(Video [4K])"}) {
   Check(Engine.MatchTrack("Rick Astley - Never Gonna Give You Up "+suffix,"Rick Astley",new[]{song})==uri,"ignore YouTube bracket annotations: "+suffix);
  }
  Check(Engine.CleanTitle("Rick Astley - Never Gonna Give You Up (Official Video) (4K Remaster)")=="Rick Astley - Never Gonna Give You Up","exact user title cleanup");
  foreach(string title in new[]{"Rick Astley - Never Gonna Give You Up sped up"}) {
   bool rejected=false;try{Engine.MatchTrack(title,"Rick Astley",new[]{song});}catch(ResolveError){rejected=true;}Check(rejected,"do not substitute recording version");
  }
  bool unknown=false;try{Engine.MatchTrack("Never Gonna Give You Up","Random uploader",new[]{song});}catch(ResolveError){unknown=true;}Check(unknown,"unknown uploader not treated as artist");
  string dir=Path.Combine(AppDomain.CurrentDomain.BaseDirectory,"diagnostics","TrackBridge-resolver-tests-"+Guid.NewGuid().ToString("N"));
  using(var e=new Engine(dir)) {
   e.S.maxTrackMinutes=0;
   e.S.broadcaster="b";e.S.reward="r";e.S.spotify.access_token="test";e.S.spotify.expires=Engine.Now+3600;e.S.chatAllowed=true;
   e.TestYouTube=(id,ct)=>Task.FromResult(Engine.Obj("{\"title\":\"Rick Astley - Never Gonna Give You Up (Official Video) (4K Remaster)\",\"author_name\":\"Rick Astley\"}"));
   var events=new List<string>();bool found=true;bool searchUnavailable=false;
   e.TestApi=(sp,method,endpoint,body)=> {
    if(endpoint.StartsWith("search?")) {
     Check(System.Web.HttpUtility.ParseQueryString(new Uri("https://api.spotify.com/v1/"+endpoint).Query)["q"]=="Rick Astley - Never Gonna Give You Up","search query excludes all video annotations");
     events.Add("search");if(searchUnavailable)throw new ApiError(503,0,"spotify");
     return Task.FromResult(found?SearchResult(song):SearchResult());
    }
    if(endpoint.StartsWith("me/player/queue")){events.Add("queue");return Task.FromResult(new Dictionary<string,object>());}
    if(endpoint.StartsWith("channel_points/")){events.Add("refund-or-fulfill");return Task.FromResult(new Dictionary<string,object>());}
    if(endpoint=="chat/messages") {events.Add("chat");return Task.FromResult(Engine.Obj("{\"data\":[{\"is_sent\":true}]}"));}
    throw new Exception("Unexpected API call: "+endpoint);
   };
   e.Accept(Event("yt-match","https://youtu.be/"+video));await e.ProcessOrder(e.S.orders[0]);
   Check(e.S.orders[0].status=="done" && e.S.orders[0].uri==uri,"YouTube order resolved and fulfilled");
   Check(string.Join(",",events)=="search,queue,refund-or-fulfill","search before queue before fulfillment");
   events.Clear();found=false;e.Accept(Event("yt-missing","https://music.youtube.com/watch?v="+video));await e.ProcessOrder(e.S.orders[1]);
   Check(e.S.orders[1].status=="canceled","no match refunds reward");
   Check(string.Join(",",events)=="search,refund-or-fulfill,chat","refund before notification and no queue on no match");
   await e.ProcessOrder(e.S.orders[1]);Check(events.Count(x=>x=="chat")==1,"do not duplicate chat notifications");
   events.Clear();searchUnavailable=true;e.Accept(Event("yt-outage","https://youtu.be/"+video));
   try{await e.ProcessOrder(e.S.orders[2]);}catch(ApiError){}
   Check(e.S.orders[2].status=="pending" && string.Join(",",events)=="search","search outage not mistaken for missing song");
   events.Clear();searchUnavailable=false;found=true;
   await e.ProcessOrder(e.S.orders[2]);Check(e.S.orders[2].status=="done","recover from search outage");
  }
 }
}
}
