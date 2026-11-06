using System;
using System.Linq;
using System.IO;
using System.Drawing;
using System.Threading;
using System.Threading.Tasks;
using System.Windows.Forms;
using System.Collections.Generic;

namespace TrackBridge {
public class RewardItem {
 public string Id,Title;
 public override string ToString() { return Title; }
}
public partial class MainWindow : Form {
 readonly Engine e;
 TextBox twitch=new TextBox(),spotify=new TextBox(),title=new TextBox();
 NumericUpDown cost=new NumericUpDown();
 ComboBox rewards=new ComboBox();
 RichTextBox logs=new RichTextBox();
 ListView orders=new ListView();
 Label status=new Label(),accounts=new Label();
 Button start=new SoftButton(),stop=new SoftButton(),cancelLogin=new SoftButton();
 List<Control> locked=new List<Control>();
 NotifyIcon tray;
 CancellationTokenSource running, login=new CancellationTokenSource();
 Task runTask;
 string updateStage;
 bool checkingUpdates;
 Button updateButton=new SoftButton();
 CancellationTokenSource updatesCancellation=new CancellationTokenSource();
 System.Windows.Forms.Timer updateTimer=new System.Windows.Forms.Timer();
 bool busy,exiting,connecting;
 Label playbackPanel=new Label(),votePanel=new Label();
 ComboBox language=new ComboBox();
 CheckBox skipAutomatic=new CheckBox();
 NumericUpDown skipManual=new NumericUpDown();
 bool languageReady;
 readonly bool previewMode;
 public MainWindow(Engine engine,bool preview=false) {
  previewMode=preview;
  e=engine;Text="TrackBridge 1.8.7 beta · Twitch → Spotify";Size=new Size(1420,930);MinimumSize=new Size(1140,800);StartPosition=FormStartPosition.CenterScreen;
  Icon=Icon.ExtractAssociatedIcon(Application.ExecutablePath);Font=new Font("Segoe UI",10);BackColor=Color.FromArgb(19,22,29);ForeColor=Color.FromArgb(235,239,246);DoubleBuffered=true;
  BuildDashboard();
  locked.Add(twitch);locked.Add(spotify);
  e.Log=Log;e.Changed=RefreshView;
  twitch.Text=e.S.twitchId;spotify.Text=e.S.spotifyId;
  tray=new NotifyIcon {Icon=Icon,Text="TrackBridge",Visible=true};tray.DoubleClick+=(s,a)=>ShowWindow();
  var trayMenu=new ContextMenuStrip();trayMenu.Items.Add(L.T("Открыть"),null,(s,a)=>ShowWindow());trayMenu.Items.Add(L.T("Выход"),null,(s,a)=>Close());tray.ContextMenuStrip=trayMenu;
  CaptureTranslations(this);CaptureMenu(orders.ContextMenuStrip);CaptureMenu(trayMenu);foreach(ColumnHeader column in orders.Columns)column.Tag=L.Canonical(column.Text);languageReady=true;
  Shown+=async(s,a)=>{RefreshView();if(previewMode)return;Log(L.T("Для приёма заказов выберите награду и нажмите «Запустить»."));if(e.S.twitch.access_token!="")Guard(LoadRewards);updateTimer.Start();await CheckUpdates(false);};
  FormClosing+=async(s,a)=>{
   if(exiting)return; a.Cancel=true;
   if(busy && running==null) { login.Cancel(); Log(L.T("Отмена подключения… Повторите выход через несколько секунд."));return; }
   exiting=true;Enabled=false;updatesCancellation.Cancel();updateTimer.Stop();updateTimer.Dispose();login.Cancel();await Stop();tray.Visible=false;tray.Dispose();Close();
  };
 }
 void Moderation(){
  if(running!=null)return;
  using(var dialog=new Form{Text=L.T("Модерация заказов"),Size=new Size(690,650),MinimumSize=new Size(620,600),StartPosition=FormStartPosition.CenterParent,BackColor=BackColor,ForeColor=ForeColor,Font=Font}){
   var layout=new TableLayoutPanel{Dock=DockStyle.Fill,Padding=new Padding(18),ColumnCount=1,RowCount=8};
   layout.RowStyles.Add(new RowStyle(SizeType.Absolute,35));layout.RowStyles.Add(new RowStyle(SizeType.Absolute,35));layout.RowStyles.Add(new RowStyle(SizeType.Absolute,48));layout.RowStyles.Add(new RowStyle(SizeType.Percent,50));layout.RowStyles.Add(new RowStyle(SizeType.Absolute,35));layout.RowStyles.Add(new RowStyle(SizeType.Percent,50));layout.RowStyles.Add(new RowStyle(SizeType.Absolute,40));layout.RowStyles.Add(new RowStyle(SizeType.Absolute,44));dialog.Controls.Add(layout);
   var duration=new FlowLayoutPanel{Dock=DockStyle.Fill};duration.Controls.Add(new Label{Text=L.T("Лимит минут (0 — без лимита):"),AutoSize=true});var minutes=new NumericUpDown{Minimum=0,Maximum=1440,Value=Math.Max(0,Math.Min(1440,e.S.maxTrackMinutes)),Width=80};duration.Controls.Add(minutes);layout.Controls.Add(duration);
   var explicitBox=new CheckBox{Text=L.T("Запретить треки с отметкой Explicit"),AutoSize=true,Checked=e.S.blockExplicit};layout.Controls.Add(explicitBox);
   layout.Controls.Add(new Label{Text=L.T("Запрещённые треки: ссылка Spotify или точное название, по одному на строку."),Dock=DockStyle.Fill});
   var tracks=new TextBox{Multiline=true,ScrollBars=ScrollBars.Vertical,Dock=DockStyle.Fill,MaxLength=20000,Text=e.S.blockedTracks??""};layout.Controls.Add(tracks);
   layout.Controls.Add(new Label{Text=L.T("Запрещённые исполнители: точное имя, по одному на строку."),Dock=DockStyle.Fill});
   var artists=new TextBox{Multiline=true,ScrollBars=ScrollBars.Vertical,Dock=DockStyle.Fill,MaxLength=20000,Text=e.S.blockedArtists??""};layout.Controls.Add(artists);
   layout.Controls.Add(new Label{Text=L.T("Регистр не важен. Отклонённые заказы возвращают баллы."),Dock=DockStyle.Fill});
   var save=new SoftButton{Text=L.T("Сохранить"),Dock=DockStyle.Right,Width=140};save.Click+=(s,a)=>{e.S.maxTrackMinutes=(int)minutes.Value;e.S.blockExplicit=explicitBox.Checked;e.S.blockedTracks=tracks.Text;e.S.blockedArtists=artists.Text;e.Save();dialog.DialogResult=DialogResult.OK;};Style(save);layout.Controls.Add(save);ThemeInputs(dialog);dialog.ShowDialog(this);
  }
  RefreshView();
 }
 void ShowWindow(){ Show();WindowState=FormWindowState.Normal;Activate(); }
 void CaptureTranslations(Control parent) {
  foreach(Control c in parent.Controls) {
   if((c is Label || c is Button || c is CheckBox) && c!=accounts && c!=status)c.Tag=L.Canonical(c.Text);
   CaptureTranslations(c);
  }
 }
 void CaptureMenu(ContextMenuStrip menu){foreach(ToolStripItem item in menu.Items)item.Tag=L.Canonical(item.Text);}
 void TranslateControls(Control parent) {
  foreach(Control c in parent.Controls){if(c.Tag is string)c.Text=L.T((string)c.Tag);TranslateControls(c);}
 }
 internal void ChangeLanguage(string selected) {
  L.Language=selected=="en"?"en":"ru";e.S.language=L.Language;e.Save();
  languageReady=false;language.SelectedIndex=L.Language=="en"?1:0;languageReady=true;
  TranslateControls(this);foreach(ColumnHeader c in orders.Columns)c.Text=L.T((string)c.Tag);
  foreach(var menu in new[]{orders.ContextMenuStrip,tray.ContextMenuStrip})foreach(ToolStripItem item in menu.Items)item.Text=L.T((string)item.Tag);
  if(title.Text=="Заказать трек" || title.Text=="Request a song")title.Text=L.T("Заказать трек");
  updateButton.Text=L.T(updateStage==null?"Проверить обновления":"Установить обновление");
  RefreshView();
 }
 async Task CheckUpdates(bool manual) {
  if(checkingUpdates || updateStage!=null)return;checkingUpdates=true;updateButton.Enabled=false;
  try {
   updateStage=await Updates.Prepare(AppDomain.CurrentDomain.BaseDirectory,Log,updatesCancellation.Token);
   if(updateStage!=null)updateButton.Text=L.T("Установить обновление");
   else if(manual)Log(L.T("Установлена актуальная версия TrackBridge ")+Updates.Current+".");
  } catch(OperationCanceledException){}
  catch(Exception ex){Log(L.T("Проверка обновлений: ")+ex.Message);}
  finally{checkingUpdates=false;if(!IsDisposed)updateButton.Enabled=true;}
 }
 async Task InstallUpdate() {
  if(MessageBox.Show(this,L.T("Обновление загружено и проверено. Остановить приём заказов и перезапустить TrackBridge для установки?"),L.T("Обновление TrackBridge"),MessageBoxButtons.YesNo)!=DialogResult.Yes)return;
  await Stop();Updates.LaunchHelper(updateStage,Application.ExecutablePath);busy=false;Close();
 }
 void CancelConnection(){login.Cancel();cancelLogin.Enabled=false;Log(L.T("Отмена подключения… Поля Client ID станут доступны после отмены."));}
 internal void VerifyControls(Action<bool,string> check) {
  try {
   VerifyDashboard(check);RefreshView();check(status.Text.Contains(L.T("нажмите «Запустить»")),"stopped reception status");
   running=new CancellationTokenSource();e.Reception(true,L.T("Приём заказов активен · Twitch подключён"));
   check(status.Text.Contains(L.T("Приём заказов активен")),"live reception status");
   running.Dispose();running=null;check(Icon!=null,"embedded icon");
   busy=true;connecting=true;SetControls();check(!spotify.Enabled && !twitch.Enabled && cancelLogin.Enabled,"cancel available during auth");
   CancelConnection();var previous=login;check(login.IsCancellationRequested && !cancelLogin.Enabled,"cancel signals cancellation");
   busy=false;connecting=false;SetControls();check(spotify.Enabled && twitch.Enabled && !cancelLogin.Enabled,"IDs unlock after cancellation");
   for(int n=0;n<2;n++) {
    try{Connect(true).GetAwaiter().GetResult();}catch(Exception){}
    check(!login.IsCancellationRequested && !object.ReferenceEquals(login,previous),"retry has fresh cancellation state");previous=login;
   }
   twitch.Text="twitch-id-fixture";spotify.Text="spotify-id-fixture";title.Text="Custom reward title";
   ChangeLanguage("en");check(skipAutomatic.Text=="Automatic: 50% of viewers","voting option translated");check(start.Text=="▶ Start" && stop.Text=="■ Stop" && updateButton.Text=="Check for updates","English action labels");
   check(status.Text=="Requests stopped — click Start" && orders.Columns[0].Text=="Time","English dynamic status and columns");
   check(tray.ContextMenuStrip.Items[0].Text=="Open" && orders.ContextMenuStrip.Items[1].Text=="Reject and refund points","English tray and context menu");
   check(twitch.Text=="twitch-id-fixture" && spotify.Text=="spotify-id-fixture" && title.Text=="Custom reward title","language switch preserves input");
   check(e.S.language=="en" && language.SelectedIndex==1,"English selection saved");
   ChangeLanguage("ru");check(start.Text=="▶ Запустить" && orders.Columns[0].Text=="Время" && language.SelectedIndex==0,"switch back to Russian");
  } finally {tray.Dispose();login.Dispose();}
 }
 void Style(Button b) { b.AutoSize=true;b.MinimumSize=new Size(100,38);b.Height=38;b.FlatStyle=FlatStyle.Flat;b.FlatAppearance.BorderSize=0;b.BackColor=Color.FromArgb(37,43,54);b.ForeColor=ForeColor;b.Margin=new Padding(4);b.Cursor=Cursors.Hand;b.Padding=new Padding(10,2,10,2); }
 Button Button(string text,Action action,bool disable=true) {var b=new SoftButton {Text=text};Style(b);b.Click+=(s,a)=>action();if(disable)locked.Add(b);return b;}
 async void Guard(Func<Task> action) {
  if(busy)return;busy=true;SetControls();
  try {await action();}catch(OperationCanceledException){Log(L.T("Операция отменена. Можно изменить Client ID и подключиться заново."));}catch(Exception ex){Log(ex.Message);MessageBox.Show(this,ex.Message,"TrackBridge",MessageBoxButtons.OK,MessageBoxIcon.Information);}
  finally{busy=false;if(!IsDisposed){SetControls();RefreshView();}}
 }
 void SetControls() {foreach(var c in locked)c.Enabled=!busy && running==null;stop.Enabled=!busy && running!=null;cancelLogin.Enabled=connecting && !login.IsCancellationRequested;cancelLogin.Visible=connecting;}
 async Task Connect(bool isSpotify) {
  SaveFields();
  login.Dispose();login=new CancellationTokenSource();connecting=true;SetControls();
  try {
   if(isSpotify)await e.ConnectSpotify(login.Token);
   else {await e.ConnectTwitch(login.Token);login.Token.ThrowIfCancellationRequested();await LoadRewards();Log(L.T("Twitch подключён. Чтобы принимать заказы, нажмите «Запустить»."));}
  } catch(Exception) {login.Token.ThrowIfCancellationRequested();throw;}
  finally {connecting=false;SetControls();}
 }
 void SaveFields() {
  string t=twitch.Text.Trim(),s=spotify.Text.Trim();
  if(t!=e.S.twitchId) {e.S.twitch=new Token();e.S.broadcaster="";e.S.reward="";rewards.Items.Clear();}
  if(s!=e.S.spotifyId)e.S.spotify=new Token();
  e.S.twitchId=t;e.S.spotifyId=s;e.Save();
 }
 async Task LoadRewards() {
  SaveFields();var list=await e.Rewards();rewards.Items.Clear();
  foreach(var r in list) {var item=new RewardItem {Id=Engine.Str(r,"id"),Title=Engine.Str(r,"title")};rewards.Items.Add(item);if(item.Id==e.S.reward)rewards.SelectedItem=item;}
  if(rewards.SelectedIndex<0 && rewards.Items.Count>0)rewards.SelectedIndex=0;
  Log(L.T("Награды загружены. Показаны только награды, которыми может управлять этот Twitch Client ID."));
 }
 async Task Start() {
  SaveFields();
  var reward=rewards.SelectedItem as RewardItem;
  if(reward==null)throw new Exception(L.T("Обновите список и выберите награду либо создайте новую."));
  if(e.S.spotify.access_token=="")throw new Exception(L.T("Подключите Spotify."));
  e.S.reward=reward.Id;e.Save();await e.ValidateTwitch();
  if(!e.S.chatReadAllowed)throw new Exception(L.Language=="en"?"Reconnect Twitch and allow reading chat to enable !song and !voteskip.":"Подключите Twitch заново и разрешите чтение чата для !song и !voteskip.");
  if(!e.S.chatAllowed)throw new Exception(L.T("Нажмите «Подключить Twitch» заново и разрешите отправку сообщений в чат: она нужна для уведомлений о ненайденных треках."));
  var details=Engine.Data(await e.Api(false,"GET","channel_points/custom_rewards?broadcaster_id="+Engine.Q(e.S.broadcaster)+"&id="+Engine.Q(reward.Id)+"&only_manageable_rewards=true")).First();
  if(!Convert.ToBoolean(details["is_user_input_required"]) || Convert.ToBoolean(details["should_redemptions_skip_request_queue"]))throw new Exception(L.T("Награда должна требовать ввод ссылки и оставлять покупки в очереди запросов. Измените её настройки в Twitch."));
  await e.Api(true,"GET","me/player");
  Log(L.T("Выбрана награда: ")+reward.Title+L.T(". Заказы принимаются только через эту награду."));
  running=new CancellationTokenSource();runTask=new Listener(e).Run(running.Token);Log(L.T("Запуск. Резервная проверка незавершённых покупок каждые 10 секунд."));
 }
 async Task Stop() {
  if(running==null)return;running.Cancel();
  try{await runTask;}catch(Exception ex){Log(ex.Message);}
  running.Dispose();running=null;runTask=null;e.Reception(false,L.T("Приём остановлен — нажмите «Запустить»"));Log(L.T("Приём остановлен. Уже добавленные песни остаются в Spotify."));RefreshView();
 }
 async Task Resolve(bool added) {
  if(orders.SelectedItems.Count==0)return;
  var o=(Order)orders.SelectedItems[0].Tag;
  if(o.status!="uncertain" && o.status!="manual")throw new Exception(L.T("Ручное решение доступно только для неопределённых заказов."));
  if(MessageBox.Show(this,added?L.T("Вы проверили, что трек уже есть в Spotify? Баллы будут списаны."):L.T("Отклонить награду и вернуть зрителю баллы?"),L.T("Решение по заказу"),MessageBoxButtons.YesNo)!=DialogResult.Yes)return;
  await e.Mark(o,added?"FULFILLED":"CANCELED");o.status=added?"done":"canceled";o.note=L.T("Обработано вручную");e.Save();
 }
 public void Log(string text) {
  if(IsDisposed)return;if(InvokeRequired){BeginInvoke(new Action<string>(Log),text);return;}
  if(activityLine!=null)activityLine.Text=text;
  logs.AppendText(DateTime.Now.ToString("HH:mm:ss")+"  "+text+Environment.NewLine);if(logs.TextLength>50000)logs.Text=logs.Text.Substring(logs.TextLength-30000);logs.SelectionStart=logs.TextLength;logs.ScrollToCaret();
 }
 public void RefreshView() {
  if(IsDisposed)return;if(InvokeRequired){BeginInvoke(new Action(RefreshView));return;}
  accounts.Text="Twitch: "+(e.S.twitch.access_token==""?L.T("не подключён"):L.T("подключён"))+"   ·   Spotify: "+(e.S.spotify.access_token==""?L.T("не подключён"):L.T("подключён"));
  status.Text=connecting?L.T("Ожидает авторизации · после подключения нажмите «Запустить»"):running==null?L.T("Приём остановлен — нажмите «Запустить»"):L.Runtime(e.ReceptionStatus);
  status.ForeColor=running!=null && e.EventsConnected?Color.FromArgb(102,226,164):Color.FromArgb(250,193,98);
  playbackPanel.Text=L.T("Сейчас играет: ")+(string.IsNullOrEmpty(e.NowPlaying)?"—":e.NowPlaying);
  votePanel.Text=running==null?L.T("Голосование ожидает запуска"):e.VoteStatus;
  string selected=orders.SelectedItems.Count>0?((Order)orders.SelectedItems[0].Tag).id:null;
  orders.BeginUpdate();orders.Items.Clear();
  lock(e.Gate)foreach(var o in e.S.orders.AsEnumerable().Reverse().Take(300)) {
   string label=o.status=="done"?L.T("В очереди Spotify"):o.status=="canceled"?L.T("Баллы возвращены"):o.status=="uncertain"?L.T("Проверьте вручную"):o.status=="pending"?L.T("Ожидание"):o.status=="sending"?L.T("Отправка"):o.status=="queued"?L.T("Подтверждение"):o.status=="reject"?L.T("Возврат баллов"):L.T("Проверьте Twitch");
   DateTime orderedAt;string time=DateTime.TryParse(o.time,out orderedAt)?orderedAt.ToString("HH:mm"):o.time;
   var row=new ListViewItem(new[]{time,o.user,label,(string.IsNullOrEmpty(o.trackTitle)?(o.uri??o.input):o.trackTitle+" — "+o.trackArtist)+" — "+L.Runtime(o.note)}){Tag=o,ToolTipText=(o.uri??o.input)+" — "+L.Runtime(o.note)};orders.Items.Add(row);if(o.id==selected)row.Selected=true;
  }
  orders.EndUpdate();RefreshDashboard();
 }
}
static class Program {
 static int PreviewUi(bool settings){
  Application.EnableVisualStyles();Application.SetCompatibleTextRenderingDefault(false);
  string dir=Path.Combine(AppDomain.CurrentDomain.BaseDirectory,"diagnostics","preview-"+Guid.NewGuid().ToString("N"));
  using(var e=new Engine(dir)){
   e.S.twitchId="preview";e.S.spotifyId="preview";e.S.twitch.access_token="preview";e.S.spotify.access_token="preview";
   e.TrackTitle="Never Gonna Give You Up";e.TrackArtist="Rick Astley";e.TrackProgress=84000;e.TrackDuration=213000;e.SkipVotes=7;e.SkipThreshold=15;e.VoteStatus="Зрителей: 30 · Порог: 50%";
   foreach(string name in new[]{"The Weeknd — Blinding Lights","Daft Punk — Get Lucky","M83 — Midnight City"})e.S.orders.Add(new Order{id=Guid.NewGuid().ToString(),user="viewer",status="pending",input=name,note="Ожидает отправки"});
   using(var window=new MainWindow(e,true))using(var timer=new System.Windows.Forms.Timer()){
    timer.Interval=500;timer.Tick+=(s,a)=>{timer.Stop();window.manualSkipPreview();using(var bitmap=new Bitmap(window.Width,window.Height)){window.DrawToBitmap(bitmap,new Rectangle(Point.Empty,window.Size));bitmap.Save(Path.Combine(AppDomain.CurrentDomain.BaseDirectory,settings?"ui-settings-preview.png":"ui-preview.png"),System.Drawing.Imaging.ImageFormat.Png);}window.Close();};window.Shown+=(s,a)=>{if(settings)window.Navigate(false,true);timer.Start();};Application.Run(window);
   }
  }return 0;
 }
 [STAThread] static int Main(string[] args) {
  if(args.Length>0 && args[0]=="--apply-update")return Updates.RunHelper(args);
  if(args.Contains("--self-test"))return Tests.Run();
  if(args.Contains("--preview-ui"))return PreviewUi(args.Contains("--preview-settings"));
  bool created;
  using(var mutex=new Mutex(true,"Local\\TrackBridge.SingleInstance",out created)) {
   if(!created){MessageBox.Show(L.T("TrackBridge уже запущен. Откройте его через значок в трее."));return 1;}
   Application.EnableVisualStyles();Application.SetCompatibleTextRenderingDefault(false);
   try {using(var e=new Engine()) Application.Run(new MainWindow(e));return 0;}
   catch(Exception ex){MessageBox.Show(ex.Message,"TrackBridge");return 1;}
  }
 }
}
}
