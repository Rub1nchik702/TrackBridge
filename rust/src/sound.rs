use crate::profile::*;use anyhow::{Result,ensure};use std::{path::Path,sync::atomic::{AtomicBool,Ordering}};
use windows::{core::PCWSTR,Win32::{Media::MediaFoundation::*,System::Com::*}};
const RATE:usize=22050;
fn wave(pcm:&[i16])->Vec<u8>{let size=(pcm.len()*2)as u32;let mut data=Vec::new();data.extend(b"RIFF");data.extend((36+size).to_le_bytes());data.extend(b"WAVEfmt ");data.extend(16u32.to_le_bytes());data.extend(1u16.to_le_bytes());data.extend(1u16.to_le_bytes());data.extend((RATE as u32).to_le_bytes());data.extend((RATE as u32*2).to_le_bytes());data.extend(2u16.to_le_bytes());data.extend(16u16.to_le_bytes());data.extend(b"data");data.extend(size.to_le_bytes());for s in pcm{data.extend(s.to_le_bytes())}data}
fn decode(path:&Path)->Result<Vec<i16>>{unsafe{
    let reader=MFCreateSourceReaderFromURL(PCWSTR(wide(path.as_os_str()).as_ptr()),None)?;let stream=MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32;
    reader.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32,false)?;reader.SetStreamSelection(stream,true)?;let ty=MFCreateMediaType()?;ty.SetGUID(&MF_MT_MAJOR_TYPE,&MFMediaType_Audio)?;ty.SetGUID(&MF_MT_SUBTYPE,&MFAudioFormat_PCM)?;ty.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE,16)?;reader.SetCurrentMediaType(stream,None,&ty)?;
    let actual=reader.GetCurrentMediaType(stream)?;let rate=actual.GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND)? as usize;let channels=actual.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS)? as usize;ensure!((8000..=384000).contains(&rate)&&(1..=32).contains(&channels)&&actual.GetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE)?==16,"Unsupported audio format");
    let mut mono=Vec::new();while mono.len()<rate*5{let mut flags=0;let mut sample=None;reader.ReadSample(stream,0,None,Some(&mut flags),None,Some(&mut sample))?;
        if let Some(sample)=sample{let buf=sample.ConvertToContiguousBuffer()?;let mut p=std::ptr::null_mut();let mut len=0;buf.Lock(&mut p,None,Some(&mut len))?;if len>0{let bytes=std::slice::from_raw_parts(p,len as usize);for frame in bytes.chunks_exact(2*channels).take(rate*5-mono.len()){let sum:i32=frame.chunks_exact(2).map(|x|i16::from_le_bytes([x[0],x[1]])as i32).sum();mono.push((sum/channels as i32) as i16)}}buf.Unlock()?}
        if flags&MF_SOURCE_READERF_ENDOFSTREAM.0 as u32!=0{break}
    }ensure!(!mono.is_empty(),"Empty audio");Ok((0..mono.len()*RATE/rate).map(|i|mono[i*rate/RATE]).collect())
}}
pub fn import(p:&mut Profile,path:&Path)->Result<()>{ensure!(path.metadata()?.len()<=50*1024*1024,"Maximum audio size: 50 MB");let pcm=unsafe{
    CoInitializeEx(None,COINIT_MULTITHREADED).ok()?;let start=MFStartup(MF_VERSION,MFSTARTUP_FULL);if let Err(e)=start{CoUninitialize();return Err(e.into())}let result=decode(path);let _=MFShutdown();CoUninitialize();result?
};atomic(&p.dir.join("request-notification.wav"),&wave(&pcm))?;p.v["requestSoundName"]=serde_json::json!(path.file_name().unwrap_or_default().to_string_lossy());p.save()}
fn tone()->Vec<i16>{let mut v=Vec::new();for (freq,len) in [(659.25,0.14),(0.,0.025),(987.77,0.20)]{for i in 0..(RATE as f64*len) as usize{let t=i as f64/RATE as f64;let a=std::f64::consts::TAU*freq*t;v.push(((a.sin()*0.9+(2.*a).sin()*0.1)*(t/0.012).min(1.)*((len-t)/0.08).min(1.)*32767.) as i16)}}v}
pub fn play(p:&Profile)->Result<()>{static BUSY:AtomicBool=AtomicBool::new(false);let volume=n(&p.v,"requestSoundVolume").clamp(0,40);if volume==0||BUSY.swap(true,Ordering::SeqCst){return Ok(())}
    let custom=(!s(&p.v,"requestSoundName").is_empty()).then(||p.dir.join("request-notification.wav"));std::thread::spawn(move||{let mut pcm=custom.and_then(|path|std::fs::read(path).ok()).filter(|v|v.len()>44&&v.len()<=2*1024*1024&&v.get(36..40)==Some(b"data")).map(|v|v[44..].chunks_exact(2).map(|x|i16::from_le_bytes([x[0],x[1]])).collect()).unwrap_or_else(tone);
        let peak=pcm.iter().map(|x|i32::from(*x).abs()).max().unwrap_or(1).max(1) as f64;for s in &mut pcm{*s=(*s as f64/peak*32767.*0.6*volume as f64/100.) as i16}
        let data=wave(&pcm);unsafe{use windows_sys::Win32::Media::Audio::*;PlaySoundW(data.as_ptr() as _,std::ptr::null_mut(),SND_MEMORY|SND_SYNC|SND_NODEFAULT);}BUSY.store(false,Ordering::SeqCst);
    });Ok(())}
