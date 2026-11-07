using System;
using System.IO;
using System.Media;
using System.Threading;

namespace TrackBridge {
static class NotificationSound {
 static int playing;
 public static void Play(int volume,string customFile=null){
  volume=Math.Max(0,Math.Min(40,volume));
  if(volume==0 || Interlocked.CompareExchange(ref playing,1,0)!=0)return;
  ThreadPool.QueueUserWorkItem(delegate {
   try {
    byte[] wave=WaveForPlayback(volume,customFile);
    using(var stream=new MemoryStream(wave,false))
    using(var player=new SoundPlayer(stream)){player.Load();player.PlaySync();}
   } catch(Exception) { /* Audio device failures must not interrupt song requests. */ }
   finally {Interlocked.Exchange(ref playing,0);}
  });
 }
 public static byte[] ImportWave(string file){return ReadCustomWave(file);}
 internal static byte[] WaveForPlayback(int volume,string customFile){
  volume=Math.Max(0,Math.Min(40,volume));
  try {return string.IsNullOrEmpty(customFile)?CreateWave(volume):CustomWave(customFile,volume);}
  catch(Exception){return CreateWave(volume);}
 }
 static byte[] ReadCustomWave(string file){
  byte[] bytes;
  using(var input=new FileStream(file,FileMode.Open,FileAccess.Read,FileShare.Read)){
   if(input.Length<44 || input.Length>2*1024*1024)throw new IOException(L.T("Выберите WAV-файл размером до 2 МБ."));
   using(var reader=new BinaryReader(input))bytes=reader.ReadBytes((int)input.Length);
  }
  if(System.Text.Encoding.ASCII.GetString(bytes,0,4)!="RIFF" || System.Text.Encoding.ASCII.GetString(bytes,8,4)!="WAVE" || BitConverter.ToUInt32(bytes,4)!=bytes.Length-8)throw new IOException(L.T("Нужен WAV PCM 16 бит, моно или стерео, длительностью до 5 секунд."));
  int rate=0,channels=0,align=0,dataOffset=0,dataLength=0;bool format=false;
  for(int offset=12;offset+8<=bytes.Length;){
   string kind=System.Text.Encoding.ASCII.GetString(bytes,offset,4);uint length=BitConverter.ToUInt32(bytes,offset+4);long end=(long)offset+8+length;
   if(end>bytes.Length)throw new IOException(L.T("Повреждённый WAV-файл."));
   if(kind=="fmt "){
    if(length<16 || BitConverter.ToUInt16(bytes,offset+8)!=1 || BitConverter.ToUInt16(bytes,offset+22)!=16)throw new IOException(L.T("Нужен WAV PCM 16 бит, моно или стерео, длительностью до 5 секунд."));
    channels=BitConverter.ToUInt16(bytes,offset+10);rate=BitConverter.ToInt32(bytes,offset+12);align=BitConverter.ToUInt16(bytes,offset+20);
    if((channels!=1 && channels!=2) || rate<8000 || rate>96000 || align!=channels*2 || BitConverter.ToInt32(bytes,offset+16)!=rate*align)throw new IOException(L.T("Неподдерживаемый формат WAV-файла."));
    format=true;
   } else if(kind=="data"){dataOffset=offset+8;dataLength=(int)length;}
   offset=(int)end+(int)(length&1);
  }
  if(!format || dataLength==0 || dataLength%align!=0 || dataLength/(double)(rate*align)>5)throw new IOException(L.T("Нужен WAV PCM 16 бит, моно или стерео, длительностью до 5 секунд."));
  using(var output=new MemoryStream())using(var writer=new BinaryWriter(output)){
   writer.Write(System.Text.Encoding.ASCII.GetBytes("RIFF"));writer.Write(36+dataLength);writer.Write(System.Text.Encoding.ASCII.GetBytes("WAVEfmt "));
   writer.Write(16);writer.Write((short)1);writer.Write((short)channels);writer.Write(rate);writer.Write(rate*align);writer.Write((short)align);writer.Write((short)16);
   writer.Write(System.Text.Encoding.ASCII.GetBytes("data"));writer.Write(dataLength);writer.Write(bytes,dataOffset,dataLength);writer.Flush();return output.ToArray();
  }
 }
 static byte[] CustomWave(string file,int volume){
  byte[] wave=ReadCustomWave(file);int peak=0;
  for(int i=44;i<wave.Length;i+=2)peak=Math.Max(peak,Math.Abs((int)BitConverter.ToInt16(wave,i)));
  double gain=peak==0?0:.6*volume/100.0*short.MaxValue/peak;
  for(int i=44;i<wave.Length;i+=2){short sample=(short)(BitConverter.ToInt16(wave,i)*gain);wave[i]=(byte)(sample&255);wave[i+1]=(byte)((sample>>8)&255);}
  return wave;
 }
 static byte[] CreateWave(int volume){
  const int rate=22050;
  int first=(int)(rate*.14),gap=(int)(rate*.025),second=(int)(rate*.20);
  int samples=first+gap+second;
  using(var stream=new MemoryStream())using(var writer=new BinaryWriter(stream)){
   writer.Write(new[]{(byte)'R',(byte)'I',(byte)'F',(byte)'F'});writer.Write(36+samples*2);
   writer.Write(new[]{(byte)'W',(byte)'A',(byte)'V',(byte)'E',(byte)'f',(byte)'m',(byte)'t',(byte)' '});
   writer.Write(16);writer.Write((short)1);writer.Write((short)1);writer.Write(rate);writer.Write(rate*2);writer.Write((short)2);writer.Write((short)16);
   writer.Write(new[]{(byte)'d',(byte)'a',(byte)'t',(byte)'a'});writer.Write(samples*2);
   for(int i=0;i<samples;i++){
    double value=0;
    if(i<first || i>=first+gap){
     bool initial=i<first;double t=(initial?i:i-first-gap)/(double)rate;
     double duration=(initial?first:second)/(double)rate;
     double envelope=Math.Min(1,t/.012)*Math.Min(1,(duration-t)/.08);
     double phase=2*Math.PI*(initial?659.25:987.77)*t;
     value=(.9*Math.Sin(phase)+.1*Math.Sin(2*phase))*envelope*.6*volume/100.0;
    }
    writer.Write((short)(value*short.MaxValue));
   }
   writer.Flush();return stream.ToArray();
  }
 }
}
}

