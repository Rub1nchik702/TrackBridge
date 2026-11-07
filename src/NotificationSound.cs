using System;
using System.IO;
using System.Media;
using System.Threading;

namespace TrackBridge {
static class NotificationSound {
 static int playing;
 public static void Play(int volume){
  volume=Math.Max(0,Math.Min(40,volume));
  if(volume==0 || Interlocked.CompareExchange(ref playing,1,0)!=0)return;
  ThreadPool.QueueUserWorkItem(delegate {
   try {
    using(var stream=new MemoryStream(CreateWave(volume),false))
    using(var player=new SoundPlayer(stream)){player.Load();player.PlaySync();}
   } catch(Exception) { /* Audio device failures must not interrupt song requests. */ }
   finally {Interlocked.Exchange(ref playing,0);}
  });
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
