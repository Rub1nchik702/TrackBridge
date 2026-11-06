using System;
using System.Linq;
using System.Text;
using System.Text.RegularExpressions;
using System.Collections;
using System.Collections.Generic;
using System.Threading;
using System.Threading.Tasks;
using System.Globalization;

namespace TrackBridge {
public class ResolveError : Exception { public ResolveError(string message):base(message){} }
public partial class Engine {
 internal Func<string,CancellationToken,Task<Dictionary<string,object>>> TestYouTube;
 public static string YouTubeId(string input) {
  Uri u;
  if(!Uri.TryCreate((input??"").Trim(),UriKind.Absolute,out u) || u.Scheme!="https" || !u.IsDefaultPort || u.UserInfo!="")return null;
  string id=null;
  if(u.Host=="youtu.be")id=u.AbsolutePath.Trim('/');
  else if(new[]{"youtube.com","www.youtube.com","m.youtube.com","music.youtube.com"}.Contains(u.Host)) {
   if(u.AbsolutePath=="/watch")id=System.Web.HttpUtility.ParseQueryString(u.Query)["v"];
   else {
    var m=Regex.Match(u.AbsolutePath,@"^/(?:shorts|embed)/([A-Za-z0-9_-]{11})/?$");
    if(m.Success)id=m.Groups[1].Value;
   }
  }
  return id!=null && Regex.IsMatch(id,@"^[A-Za-z0-9_-]{11}$")?id:null;
 }
 public static string CleanTitle(string title) {
  string s=title??"";
  // Ignore all parenthesized/bracketed YouTube annotations, including nested ones.
  string previous;
  do {previous=s;s=Regex.Replace(s,@"\([^()]*\)|\[[^\[\]]*\]"," ");}while(s!=previous);
  s=Regex.Replace(s,@"\s*\|\s*official\s+(?:music\s+)?(?:video|audio)\s*$","",RegexOptions.IgnoreCase);
  return Regex.Replace(s,@"\s+"," ").Trim();
 }
 static string Normal(string s) {
  var b=new StringBuilder();
  foreach(char c in (s??"").Normalize(NormalizationForm.FormD).ToLowerInvariant()) {
   if(CharUnicodeInfo.GetUnicodeCategory(c)==UnicodeCategory.NonSpacingMark)continue;
   if(char.IsLetterOrDigit(c))b.Append(c);else b.Append(' ');
  }
  return Regex.Replace(b.ToString(),@"\s+"," ").Trim();
 }
 static IEnumerable<Dictionary<string,object>> Items(object value) {
  var seq=value as IEnumerable;
  if(seq==null || value is string)throw new Exception(L.T("Не удалось прочитать результаты поиска Spotify."));
  foreach(var item in seq) {
   var row=item as Dictionary<string,object>;
   if(row==null)throw new Exception(L.T("Некорректный результат поиска Spotify."));
   yield return row;
  }
 }
 public static string MatchTrack(string title,string author,IEnumerable<Dictionary<string,object>> tracks) {
  string clean=CleanTitle(title), full=Normal(clean);
  string channel=Regex.Replace(author??"",@"\s*-\s*Topic$","",RegexOptions.IgnoreCase);
  var pieces=Regex.Split(clean,@"\s+[-–—]\s+",RegexOptions.None);
  var hits=new List<Dictionary<string,object>>();
  foreach(var t in tracks) {
   if(t.ContainsKey("is_playable") && t["is_playable"] is bool && !(bool)t["is_playable"])continue;
   if(t.ContainsKey("restrictions") && D(t,"restrictions").Count>0)continue;
   string name=Normal(Str(t,"name"));
   if(name=="" || !t.ContainsKey("artists"))continue;
   var artists=Items(t["artists"]).Select(a=>Normal(Str(a,"name"))).Where(x=>x!="").ToList();
   bool match=artists.Any(a=>full==Normal(a+" "+name) || full==Normal(name+" "+a));
   if(!match && pieces.Length==2) {
    match=(Normal(pieces[1])==name && artists.Contains(Normal(pieces[0]))) || (Normal(pieces[0])==name && artists.Contains(Normal(pieces[1])));
   }
   if(!match && full==name && artists.Contains(Normal(channel)))match=true;
   if(match)hits.Add(t);
  }
  // Album reissues of the same named recording are acceptable; distinct artist sets are ambiguous.
  var groups=hits.GroupBy(t=>Normal(Str(t,"name"))+"|"+string.Join("|",Items(t["artists"]).Select(a=>Normal(Str(a,"name"))).OrderBy(a=>a))).ToList();
  if(groups.Count!=1)throw new ResolveError(L.T("Не удалось найти однозначное совпадение трека в Spotify. Баллы возвращены."));
  return Track(Str(groups[0].First(),"uri"));
 }
 public void CheckTrackPolicy(Dictionary<string,object> track){
  string name=Str(track,"name");
  if(S.maxTrackMinutes>0){
   if(!track.ContainsKey("duration_ms") || track["duration_ms"]==null || Convert.ToInt64(track["duration_ms"])<=0)throw new Exception(L.T("Spotify не вернул длительность трека."));
   if(Convert.ToInt64(track["duration_ms"])>(long)S.maxTrackMinutes*60000)throw new ResolveError(L.T("Трек превышает лимит длительности. Баллы возвращены."));
  }
  if(S.blockExplicit){
   if(!track.ContainsKey("explicit") || !(track["explicit"] is bool))throw new Exception(L.T("Spotify не вернул отметку Explicit."));
   if((bool)track["explicit"])throw new ResolveError(L.T("Треки Explicit запрещены. Баллы возвращены."));
  }
  var blocked=(S.blockedTracks??"").Split(new[]{'\r','\n'},StringSplitOptions.RemoveEmptyEntries);
  string uri=Str(track,"uri"),linked=track.ContainsKey("linked_from")?Str(D(track,"linked_from"),"uri"):"";
  foreach(string entry in blocked){string text=entry.Trim();bool match=string.Equals(text,name,StringComparison.OrdinalIgnoreCase);try{string parsed=Track(text);match=match || parsed==uri || parsed==linked;}catch{}if(match)throw new ResolveError(L.T("Этот трек запрещён. Баллы возвращены."));}
  var artists=(S.blockedArtists??"").Split(new[]{'\r','\n'},StringSplitOptions.RemoveEmptyEntries).Select(x=>x.Trim()).ToList();
  if(artists.Count>0){
   if(!track.ContainsKey("artists"))throw new Exception(L.T("Spotify не вернул исполнителей."));
   if(Items(track["artists"]).Any(a=>artists.Any(x=>string.Equals(x,Str(a,"name"),StringComparison.OrdinalIgnoreCase))))throw new ResolveError(L.T("Этот исполнитель запрещён. Баллы возвращены."));
  }
 }
 public async Task<Dictionary<string,object>> ValidateOrderTrack(string uri){
  if(S.maxTrackMinutes<=0 && !S.blockExplicit && string.IsNullOrWhiteSpace(S.blockedTracks) && string.IsNullOrWhiteSpace(S.blockedArtists))return null;
  var track=await Api(true,"GET","tracks/"+Track(uri).Substring("spotify:track:".Length));
  if(Str(track,"type")!="track" || string.IsNullOrEmpty(Str(track,"uri")))throw new Exception(L.T("Spotify не вернул данные трека."));
  // Also match the originally ordered ID when Spotify relinks a recording.
  var original=new Dictionary<string,object>(track);original["uri"]=uri;CheckTrackPolicy(original);CheckTrackPolicy(track);return track;
 }
 public async Task<string> ResolveTrack(string input,CancellationToken ct) {
  string video=YouTubeId(input);
  if(video==null) {
   try{return Track(input);}catch(Exception){throw new ResolveError(L.T("Нужна ссылка на трек Spotify, YouTube или YouTube Music. Баллы возвращены."));}
  }
  Dictionary<string,object> metadata;
  try {
   metadata=TestYouTube!=null ? await TestYouTube(video,ct) : await Raw("GET","https://www.youtube.com/oembed?format=json&url="+Q("https://www.youtube.com/watch?v="+video),null,null,null,false,ct);
  } catch(ApiError ex) {
   if(ex.Status==400 || ex.Status==401 || ex.Status==403 || ex.Status==404)throw new ResolveError(L.T("Не удалось прочитать название видео YouTube: оно может быть закрыто или недоступно. Баллы возвращены."));
   throw;
  }
  ct.ThrowIfCancellationRequested();
  string title=Str(metadata,"title"),author=Str(metadata,"author_name");
  if(string.IsNullOrWhiteSpace(title))throw new ResolveError(L.T("YouTube не вернул название трека. Баллы возвращены."));
  Log(L.T("Поиск в Spotify: ")+title);
  string query=CleanTitle(title);
  if(!Regex.IsMatch(query,@"\s[-–—]\s"))query+=" "+Regex.Replace(author,@"\s*-\s*Topic$","",RegexOptions.IgnoreCase);
  var result=await Api(true,"GET","search?type=track&limit=10&q="+Q(query));
  var tracks=D(result,"tracks");
  return MatchTrack(title,author,Items(tracks["items"]));
 }
 async Task NotifyRejected(Order o) {
  // Persist before sending: do not spam chat after a crash or an ambiguous response.
  o.notificationAttempted=true;Save();
  if(!S.chatAllowed){Log(L.T("Уведомление не отправлено: подключите Twitch заново и разрешите отправку сообщений в чат."));return;}
  string user=Regex.Replace(o.user??L.T("зритель"),@"[^\p{L}\p{N}_]","");
  string text="@"+user+" "+o.note;
  if(text.Length>450)text=text.Substring(0,450);
  try {
   var response=await Api(false,"POST","chat/messages",new {broadcaster_id=S.broadcaster,sender_id=S.broadcaster,message=text});
   var sent=Data(response).FirstOrDefault();
   if(sent==null || !sent.ContainsKey("is_sent") || !Convert.ToBoolean(sent["is_sent"]))Log(L.T("Twitch не доставил сообщение о возврате баллов."));
   else Log(L.T("Уведомление отправлено в чат: ")+text);
  } catch(Exception ex){Log(L.T("Баллы возвращены, но уведомление в чат не отправлено: ")+ex.Message);}
 }
}
}
