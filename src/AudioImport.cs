using System;
using System.IO;
using System.Reflection;
using System.Runtime.CompilerServices;
using NAudio.Wave;

namespace TrackBridge {
static class AudioDependencies {
 static bool initialized;static readonly object gate=new object();
 internal static void Initialize(){lock(gate){if(initialized)return;AppDomain.CurrentDomain.AssemblyResolve+=(sender,args)=>{
  string name=new AssemblyName(args.Name).Name;
  if(name!="NAudio.Core" && name!="NAudio.Wasapi")return null;
  using(var input=typeof(AudioDependencies).Assembly.GetManifestResourceStream("TrackBridge.Dependencies."+name+".dll")){
   if(input==null)return null;using(var output=new MemoryStream()){input.CopyTo(output);return Assembly.Load(output.ToArray());}
  }
 };initialized=true;}}
}
static class AudioImport {
 internal static byte[] ConvertFile(string source){
  AudioDependencies.Initialize();string local=Path.GetFullPath(source);var info=new FileInfo(local);
  if(!info.Exists || info.Length==0 || info.Length>50*1024*1024)throw new IOException(L.T("Выберите аудиофайл размером до 50 МБ."));
  if(string.Equals(info.Extension,".wav",StringComparison.OrdinalIgnoreCase)){try{return NotificationSound.ImportWave(local);}catch(IOException){}}
  try{return Decode(local);}catch(Exception ex){throw new IOException(L.T("Не удалось прочитать аудио. Формат должен поддерживаться кодеками Windows."),ex);}
 }
 [MethodImpl(MethodImplOptions.NoInlining)]
 static byte[] Decode(string source){
  var format=new WaveFormat(22050,16,1);
  using(var reader=new MediaFoundationReader(source,new MediaFoundationReader.MediaFoundationReaderSettings{SingleReaderObject=true}))
  using(var converter=new MediaFoundationResampler(reader,format))
  using(var output=new MemoryStream()){
   converter.ResamplerQuality=60;int remaining=format.AverageBytesPerSecond*5;byte[] buffer=new byte[8192];
   using(var writer=new WaveFileWriter(output,format)){
    while(remaining>0){int read=converter.Read(buffer,0,Math.Min(buffer.Length,remaining));if(read==0)break;writer.Write(buffer,0,read);remaining-=read;}
    if(output.Length<=44)throw new IOException("No decoded audio.");writer.Flush();
   }
   return output.ToArray();
  }
 }
}
}
