using System;
using System.Linq;
using System.Text;
using System.IO;
using System.Net.WebSockets;
using System.Threading;
using System.Threading.Tasks;

namespace TrackBridge {
public class Listener {
 readonly Engine e;
 readonly ChatCommands commands;
 public Listener(Engine engine) { e=engine;commands=new ChatCommands(e); }
 public async Task Run(CancellationToken ct) {
  using(var linked=CancellationTokenSource.CreateLinkedTokenSource(ct)) {
   var worker=Work(linked.Token);
   e.Chat=commands;var chat=commands.Run(linked.Token);
   try { await Listen(linked.Token); }
   finally { linked.Cancel();e.Chat=null;e.NowPlaying="";e.CurrentTrackUrl="";e.VoteStatus="";e.TrackTitle="";e.TrackArtist="";e.TrackImage="";e.TrackProgress=0;e.TrackDuration=0;e.SkipVotes=0;e.SkipThreshold=0;e.Changed(); }
   await worker;
   await chat;
  }
 }
 async Task Work(CancellationToken ct) {
  long validated=0,polled=0;
  while(!ct.IsCancellationRequested) {
   bool failed=false;
   try {
    if(Engine.Now-validated>3500) { await e.ValidateTwitch(); validated=Engine.Now; }
    if(Engine.Now-polled>=10) {
     polled=Engine.Now;
     try {await e.Recover();}catch(Exception ex){e.Log(L.T("Резервная проверка покупок Twitch: ")+ex.Message);}
    }
    Order o;
    lock(e.Gate) o=e.S.orders.FirstOrDefault(x=>x.nextAttempt<=Engine.Now && x.reward==e.S.reward && x.broadcaster==e.S.broadcaster && (x.status=="pending" || x.status=="queued" || x.status=="reject" || (x.status=="canceled" && x.notify && !x.notificationAttempted)));
    if(o!=null){try{await e.ProcessOrder(o,ct);}catch(OperationCanceledException){throw;}catch(Exception ex){e.Defer(o,ex);}}
    else await Task.Delay(700,ct);
   } catch(OperationCanceledException) { break; }
   catch(Exception ex) { e.Log(ex.Message); failed=true; }
   if(failed) { try { await Task.Delay(10000,ct); } catch(OperationCanceledException) { break; } }
  }
 }
 async Task<string> Receive(ClientWebSocket ws,int seconds,CancellationToken ct) {
  using(var deadline=CancellationTokenSource.CreateLinkedTokenSource(ct)) {
   deadline.CancelAfter(TimeSpan.FromSeconds(seconds));
   using(var ms=new MemoryStream()) {
    var b=new byte[8192];
    WebSocketReceiveResult r;
    do {
     r=await ws.ReceiveAsync(new ArraySegment<byte>(b),deadline.Token);
     if(r.MessageType==WebSocketMessageType.Close) throw new IOException(L.T("Twitch закрыл соединение."));
     ms.Write(b,0,r.Count);
     if(ms.Length>1024*1024) throw new IOException(L.T("Слишком большое сообщение Twitch."));
    } while(!r.EndOfMessage);
    return Encoding.UTF8.GetString(ms.ToArray());
   }
  }
 }
 async Task Listen(CancellationToken ct) {
  ClientWebSocket old=null;
  string url="wss://eventsub.wss.twitch.tv/ws";
  bool migrating=false;
  while(!ct.IsCancellationRequested) {
   e.Reception(false,L.T("Подключение к Twitch… Резервная проверка заказов каждые 10 секунд"));
   using(var ws=new ClientWebSocket()) {
    try {
     await ws.ConnectAsync(new Uri(url),ct);
     int timeout=15;
     while(!ct.IsCancellationRequested) {
      var msg=Engine.Obj(await Receive(ws,timeout,ct));
      var meta=Engine.D(msg,"metadata"); var payload=Engine.D(msg,"payload");
      string type=Engine.Str(meta,"message_type");
      if(type=="session_welcome") {
       var session=Engine.D(payload,"session");
       if(session.ContainsKey("keepalive_timeout_seconds") && session["keepalive_timeout_seconds"]!=null) timeout=Convert.ToInt32(session["keepalive_timeout_seconds"])+5;
       else timeout=30;
       if(!migrating) {
        await e.Api(false,"POST","eventsub/subscriptions",new {type="channel.channel_points_custom_reward_redemption.add",version="1",condition=new {broadcaster_user_id=e.S.broadcaster,reward_id=e.S.reward},transport=new {method="websocket",session_id=Engine.Str(session,"id")}});
        await e.Api(false,"POST","eventsub/subscriptions",new {type="channel.chat.message",version="1",condition=new {broadcaster_user_id=e.S.broadcaster,user_id=e.S.broadcaster},transport=new {method="websocket",session_id=Engine.Str(session,"id")}});
       }
       if(old!=null) { old.Dispose(); old=null; }
       migrating=false; url="wss://eventsub.wss.twitch.tv/ws";
       e.Reception(true,L.T("Приём заказов активен · Twitch подключён"));
       e.Log(L.T("Twitch: соединение установлено. Приём заказов активен."));
      } else if(type=="notification") {
       if(Engine.Str(Engine.D(payload,"subscription"),"type")=="channel.channel_points_custom_reward_redemption.add") e.Accept(Engine.D(payload,"event"));
       else if(Engine.Str(Engine.D(payload,"subscription"),"type")=="channel.chat.message")commands.Accept(Engine.D(payload,"event"));
      } else if(type=="session_reconnect") {
       string next=Engine.Str(Engine.D(payload,"session"),"reconnect_url"); Uri nextUri;
       if(!Uri.TryCreate(next,UriKind.Absolute,out nextUri) || nextUri.Scheme!="wss" || nextUri.Host!="eventsub.wss.twitch.tv") throw new IOException(L.T("Некорректный адрес переподключения."));
       url=next; migrating=true;
       // Connect the replacement before disposing this socket.
       old=ws;
       await Migrate(old,url,ct);
       old=null; migrating=false; url="wss://eventsub.wss.twitch.tv/ws";
       break;
      } else if(type=="revocation") {
       throw new IOException(L.T("Twitch отозвал подписку: ")+Engine.Str(Engine.D(payload,"subscription"),"status"));
      }
     }
    } catch(OperationCanceledException) { if(ct.IsCancellationRequested) break; e.Log(L.T("Twitch: нет ответа, переподключение…")); }
    catch(Exception ex) { e.Log("Twitch: "+ex.Message+L.T(" Переподключение через 5 секунд.")); }
    finally { e.Reception(false,L.T("Нет соединения с событиями Twitch · резервная проверка заказов активна"));if(old!=null) { old.Dispose(); old=null; } }
   }
   url="wss://eventsub.wss.twitch.tv/ws"; migrating=false;
   try { await Task.Delay(5000,ct); } catch(OperationCanceledException) { break; }
  }
 }
 async Task Migrate(ClientWebSocket previous,string url,CancellationToken ct) {
  using(var ws=new ClientWebSocket()) {
   await ws.ConnectAsync(new Uri(url),ct);
   var welcome=Engine.Obj(await Receive(ws,15,ct));
   if(Engine.Str(Engine.D(welcome,"metadata"),"message_type")!="session_welcome") throw new IOException(L.T("Нет приветствия Twitch."));
   previous.Dispose();
   var session=Engine.D(Engine.D(welcome,"payload"),"session");
   int timeout=session.ContainsKey("keepalive_timeout_seconds") && session["keepalive_timeout_seconds"]!=null ? Convert.ToInt32(session["keepalive_timeout_seconds"])+5:30;
   e.Log(L.T("Twitch: соединение перенесено без новой подписки."));
   e.Reception(true,L.T("Приём заказов активен · Twitch подключён"));
   while(!ct.IsCancellationRequested) {
    var msg=Engine.Obj(await Receive(ws,timeout,ct)); var p=Engine.D(msg,"payload");
    string type=Engine.Str(Engine.D(msg,"metadata"),"message_type");
    if(type=="notification"){
     string topic=Engine.Str(Engine.D(p,"subscription"),"type");
     if(topic=="channel.chat.message")commands.Accept(Engine.D(p,"event"));else if(topic=="channel.channel_points_custom_reward_redemption.add")e.Accept(Engine.D(p,"event"));
    }
    else if(type=="session_reconnect") {
     string next=Engine.Str(Engine.D(p,"session"),"reconnect_url"); var u=new Uri(next);
     if(u.Scheme!="wss" || u.Host!="eventsub.wss.twitch.tv") throw new IOException(L.T("Некорректный адрес Twitch."));
     await Migrate(ws,next,ct); return;
    } else if(type=="revocation") throw new IOException(L.T("Подписка Twitch отозвана."));
   }
  }
 }
}
}
