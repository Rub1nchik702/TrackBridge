using System;
using System.IO;
using System.IO.Compression;
using System.Net.Http;
using System.Linq;
using System.Text.RegularExpressions;
using System.Collections.Generic;
using System.Security.Cryptography;
using System.Threading;
using System.Threading.Tasks;
using System.Diagnostics;
using System.Windows.Forms;

namespace TrackBridge {
static class Updates {
 public const string Current="1.8.7-beta.1";
 public const string Repository="https://github.com/Rub1nchik702/TrackBridge";
 static readonly string[] Files={"TrackBridge.exe","README.md","README.html","LICENSE.txt","THIRD-PARTY-NOTICES.txt","CHANGELOG.md"};
 internal static Func<HttpMessageHandler> TestHandler;
 public static int Compare(string a,string b) {
  var pattern=@"^v?(\d+)\.(\d+)\.(\d+)(?:-beta\.(\d+))?$";
  var x=Regex.Match(a??"",pattern);var y=Regex.Match(b??"",pattern);
  if(!x.Success || !y.Success)throw new FormatException(L.T("Некорректная версия релиза."));
  for(int i=1;i<=3;i++){int c=int.Parse(x.Groups[i].Value).CompareTo(int.Parse(y.Groups[i].Value));if(c!=0)return c;}
  if(x.Groups[4].Success!=y.Groups[4].Success)return x.Groups[4].Success?-1:1;
  return x.Groups[4].Success?int.Parse(x.Groups[4].Value).CompareTo(int.Parse(y.Groups[4].Value)):0;
 }
 public static Dictionary<string,object> SelectRelease(string json,string current) {
  var releases=new System.Web.Script.Serialization.JavaScriptSerializer().DeserializeObject(json) as object[];
  if(releases==null)throw new Exception(L.T("GitHub вернул неверный список релизов."));
  Dictionary<string,object> best=null;
  foreach(var item in releases) {
   var r=item as Dictionary<string,object>;if(r==null || (r.ContainsKey("draft") && Convert.ToBoolean(r["draft"])))continue;
   string version=Engine.Str(r,"tag_name");
   if(!Regex.IsMatch(version,@"^v\d+\.\d+\.\d+(?:-beta\.\d+)?$"))continue;
   if(Compare(version,current)>0 && (best==null || Compare(version,Engine.Str(best,"tag_name"))>0))best=r;
  }
  return best;
 }
 public static bool TrustedUrl(string url,string tag,string asset) {
  return url==Repository+"/releases/download/"+tag+"/"+asset;
 }
 static HttpClient Client() {
  System.Net.ServicePointManager.SecurityProtocol=System.Net.SecurityProtocolType.Tls12;
  var c=new HttpClient(TestHandler!=null?TestHandler():(HttpMessageHandler)new HttpClientHandler{AllowAutoRedirect=true,MaxAutomaticRedirections=5}){Timeout=TimeSpan.FromSeconds(60)};
  c.DefaultRequestHeaders.UserAgent.ParseAdd("TrackBridge/"+Current);return c;
 }
 public static string Hash(byte[] data){using(var h=SHA256.Create())return BitConverter.ToString(h.ComputeHash(data)).Replace("-","").ToLowerInvariant();}
 public static Dictionary<string,object> SignedManifest(byte[] json,string signature){
  if(json.Length>8192 || signature.Length>2048)throw new IOException("Update manifest is too large.");
  using(var rsa=new RSACryptoServiceProvider()){rsa.PersistKeyInCsp=false;rsa.FromXmlString(SigningKey.PublicXml);if(!rsa.VerifyData(json,CryptoConfig.MapNameToOID("SHA256"),Convert.FromBase64String(signature.Trim())))throw new IOException("Publisher signature verification failed. Update refused.");}
  var manifest=Engine.Obj(System.Text.Encoding.UTF8.GetString(json));
  string version=Engine.Str(manifest,"version");
  if(!Regex.IsMatch(version,@"^\d+\.\d+\.\d+(?:-beta\.\d+)?$") || Engine.Str(manifest,"archive")!="TrackBridge-"+version+"-win.zip" || !Regex.IsMatch(Engine.Str(manifest,"sha256"),@"^[a-f0-9]{64}$"))throw new IOException("Invalid signed update manifest.");
  return manifest;
 }
 static async Task<byte[]> DownloadSmall(HttpClient client,string url,CancellationToken ct){
  using(var response=await client.GetAsync(url,HttpCompletionOption.ResponseHeadersRead,ct)){response.EnsureSuccessStatusCode();if(response.RequestMessage.RequestUri.Scheme!="https")throw new IOException("Insecure signature redirect.");using(var stream=await response.Content.ReadAsStreamAsync())using(var memory=new MemoryStream()){byte[] b=new byte[2048];int n;while((n=await stream.ReadAsync(b,0,b.Length,ct))>0){if(memory.Length+n>8192)throw new IOException("Update metadata too large.");memory.Write(b,0,n);}return memory.ToArray();}}
 }
 static void NoLink(string path) {if((File.Exists(path)||Directory.Exists(path)) && (File.GetAttributes(path)&FileAttributes.ReparsePoint)!=0)throw new IOException(L.T("Обновление через ссылку файловой системы запрещено."));}
 public static void Extract(string archive,string payload,string digest) {
  if(!Regex.IsMatch(digest??"",@"^[a-f0-9]{64}$") || Hash(File.ReadAllBytes(archive))!=digest)throw new IOException(L.T("Контрольная сумма обновления не совпала."));
  NoLink(payload);Directory.CreateDirectory(payload);
  using(var zip=ZipFile.OpenRead(archive)) {
   if(zip.Entries.Count!=Files.Length || !zip.Entries.Select(e=>e.FullName).OrderBy(x=>x).SequenceEqual(Files.OrderBy(x=>x)))throw new IOException(L.T("В архиве обновления неожиданные файлы."));
   foreach(var entry in zip.Entries) {
    if(entry.Length>20*1024*1024)throw new IOException(L.T("Слишком большой файл обновления."));
    string path=Path.Combine(payload,entry.FullName);NoLink(path);
    using(var input=entry.Open())using(var output=new FileStream(path,FileMode.Create,FileAccess.Write,FileShare.None))input.CopyTo(output);
   }
  }
  var info=FileVersionInfo.GetVersionInfo(Path.Combine(payload,"TrackBridge.exe"));
  if(info.ProductName!="TrackBridge")throw new IOException(L.T("Архив не содержит приложение TrackBridge."));
 }
 public static async Task<string> Prepare(string directory,Action<string> log,CancellationToken ct) {
  using(var client=Client()) {
   string json;
   using(var response=await client.GetAsync("https://api.github.com/repos/Rub1nchik702/TrackBridge/releases?per_page=30",ct)) {response.EnsureSuccessStatusCode();json=await response.Content.ReadAsStringAsync();}
   var release=SelectRelease(json,Current);if(release==null)return null;
   string tag=Engine.Str(release,"tag_name"),name="TrackBridge-"+tag.Substring(1)+"-win.zip";
   var assets=((System.Collections.IEnumerable)release["assets"]).Cast<Dictionary<string,object>>();
   var asset=assets.FirstOrDefault(a=>Engine.Str(a,"name")==name && Engine.Str(a,"state")=="uploaded");
   if(asset==null)throw new IOException(L.T("Новый релиз пока не содержит архива Windows."));
   string url=Engine.Str(asset,"browser_download_url"),digest=Engine.Str(asset,"digest");
   if(!TrustedUrl(url,tag,name) || !Regex.IsMatch(digest,@"^sha256:[a-f0-9]{64}$"))throw new IOException(L.T("Не подтверждён источник или SHA-256 обновления."));
   var manifestAsset=assets.FirstOrDefault(a=>Engine.Str(a,"name")=="TrackBridge-update.json");var signatureAsset=assets.FirstOrDefault(a=>Engine.Str(a,"name")=="TrackBridge-update.sig");
   if(manifestAsset==null || signatureAsset==null || !TrustedUrl(Engine.Str(manifestAsset,"browser_download_url"),tag,"TrackBridge-update.json") || !TrustedUrl(Engine.Str(signatureAsset,"browser_download_url"),tag,"TrackBridge-update.sig"))throw new IOException("This release has no publisher-signed update manifest.");
   byte[] manifestBytes=await DownloadSmall(client,Engine.Str(manifestAsset,"browser_download_url"),ct),signatureBytes=await DownloadSmall(client,Engine.Str(signatureAsset,"browser_download_url"),ct);
   var manifest=SignedManifest(manifestBytes,System.Text.Encoding.ASCII.GetString(signatureBytes));
   if(Engine.Str(manifest,"version")!=tag.Substring(1) || Engine.Str(manifest,"archive")!=name || Engine.Str(manifest,"sha256")!=digest.Substring(7))throw new IOException("Signed update metadata does not match the release.");
   NoLink(directory);string root=Path.Combine(directory,".updates");NoLink(root);Directory.CreateDirectory(root);
   string stage=Path.Combine(root,Guid.NewGuid().ToString("N"));Directory.CreateDirectory(stage);
   File.WriteAllBytes(Path.Combine(stage,"manifest.json"),manifestBytes);File.WriteAllBytes(Path.Combine(stage,"manifest.sig"),signatureBytes);
   log(L.T("Загрузка обновления ")+tag+L.T(" с GitHub…"));
   using(var response=await client.GetAsync(url,HttpCompletionOption.ResponseHeadersRead,ct)) {
    response.EnsureSuccessStatusCode();
    if(response.RequestMessage.RequestUri.Scheme!="https")throw new IOException(L.T("Небезопасное перенаправление обновления."));
    using(var input=await response.Content.ReadAsStreamAsync())using(var output=File.Create(Path.Combine(stage,"update.zip"))) {
     var buffer=new byte[8192];int read;long total=0;
     while((read=await input.ReadAsync(buffer,0,buffer.Length,ct))>0) {total+=read;if(total>30*1024*1024)throw new IOException(L.T("Слишком большой архив обновления."));await output.WriteAsync(buffer,0,read,ct);}
    }
   }
   string hash=digest.Substring(7);Extract(Path.Combine(stage,"update.zip"),Path.Combine(stage,"payload"),hash);
   string expected=tag.Substring(1).Split('-')[0]+".0";
   if(FileVersionInfo.GetVersionInfo(Path.Combine(stage,"payload","TrackBridge.exe")).FileVersion!=expected)throw new IOException(L.T("Версия файла не совпадает с тегом релиза."));
   File.WriteAllText(Path.Combine(stage,"digest.txt"),hash);log(L.T("Обновление ")+tag+L.T(" загружено и проверено. Нажмите «Установить обновление»."));return stage;
  }
 }
 public static void ApplyFiles(string stage,string targetExe,Action<string> beforeCopy=null) {
  string folder=Path.GetDirectoryName(Path.GetFullPath(targetExe));
  if(!string.Equals(Path.GetDirectoryName(Path.GetFullPath(stage)),Path.Combine(folder,".updates"),StringComparison.OrdinalIgnoreCase))throw new IOException(L.T("Некорректная папка обновления."));
  NoLink(folder);NoLink(Path.Combine(folder,".updates"));NoLink(stage);
  var signed=SignedManifest(File.ReadAllBytes(Path.Combine(stage,"manifest.json")),File.ReadAllText(Path.Combine(stage,"manifest.sig")));
  if(Compare(Engine.Str(signed,"version"),Current)<=0)throw new IOException("Update is not newer than this installed version.");
  string payload=Path.Combine(stage,"payload");Extract(Path.Combine(stage,"update.zip"),payload,Engine.Str(signed,"sha256"));
  string backup=Path.Combine(stage,"backup");NoLink(backup);Directory.CreateDirectory(backup);
  var changed=new List<string>();
  try {
   foreach(string name in Files) {
    string dest=name=="TrackBridge.exe"?targetExe:Path.Combine(folder,name);NoLink(dest);
    string previous=Path.Combine(backup,name);if(File.Exists(dest))File.Copy(dest,previous,false);
    if(beforeCopy!=null)beforeCopy(name);
    // File.Replace is atomic per file. Settings and every other file are excluded.
    string temp=dest+".update-new";NoLink(temp);File.Copy(Path.Combine(payload,name),temp,true);
    if(File.Exists(dest))File.Replace(temp,dest,null);else File.Move(temp,dest);
    changed.Add(name);
   }
  } catch {
   foreach(string name in changed.AsEnumerable().Reverse()) {
    string dest=name=="TrackBridge.exe"?targetExe:Path.Combine(folder,name),previous=Path.Combine(backup,name);
    if(File.Exists(previous))File.Copy(previous,dest,true);else if(File.Exists(dest))File.Delete(dest);
   }
   throw;
  }
 }
 public static void LaunchHelper(string stage,string targetExe) {
  string helper=Path.Combine(stage,"UpdateRunner.exe");File.Copy(Application.ExecutablePath,helper,true);
  Process.Start(new ProcessStartInfo(helper,"--apply-update "+Process.GetCurrentProcess().Id+" \""+stage+"\" \""+targetExe+"\""){UseShellExecute=false,CreateNoWindow=true,WorkingDirectory=stage});
 }
 public static int RunHelper(string[] args) {
  try {
   if(args.Length!=4)throw new IOException(L.T("Неверные параметры обновления."));
   int pid=int.Parse(args[1]);bool exited=true;
   try{using(var process=Process.GetProcessById(pid))exited=process.WaitForExit(120000);}catch(ArgumentException){}
   if(!exited)throw new IOException(L.T("TrackBridge не закрылся. Обновление отложено."));
   ApplyFiles(args[2],args[3]);
   // The old app releases its named mutex when its process exits.
   Process.Start(new ProcessStartInfo(args[3]){UseShellExecute=true,WorkingDirectory=Path.GetDirectoryName(args[3])});return 0;
  } catch(Exception ex) {MessageBox.Show(L.T("Не удалось установить обновление: ")+ex.Message+L.T("\nНастройки сохранены. Запустите TrackBridge вручную."),"TrackBridge");return 1;}
 }
}
}
