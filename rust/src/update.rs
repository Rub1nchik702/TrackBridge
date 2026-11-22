use crate::{api::client,profile::{s,wide}};
use anyhow::{Result,Context,ensure,bail};use base64::{Engine,engine::general_purpose::STANDARD};use rsa::{BigUint,RsaPublicKey,pkcs1v15::Pkcs1v15Sign};use sha2::{Digest,Sha256};use serde_json::Value;
use std::{fs,io::{Read,Cursor},path::{Path,PathBuf},process::Command,os::windows::fs::MetadataExt};
pub const CURRENT:&str=env!("TRACKBRIDGE_VERSION");const REPO:&str="https://github.com/Rub1nchik702/TrackBridge";
const FILES:[&str;6]=["TrackBridge.exe","README.md","README.html","LICENSE.txt","THIRD-PARTY-NOTICES.txt","CHANGELOG.md"];
pub fn version(raw:&str)->Option<([u32;4],bool,u32)>{
    let re=regex::Regex::new(r"^v?(\d+)\.(\d+)(?:\.(\d+)(?:\.(\d+))?)?(?:-beta\.(\d+))?$").ok()?;let m=re.captures(raw)?;let mut parts=[0;4];for(i,p) in parts.iter_mut().enumerate(){if let Some(v)=m.get(i+1){*p=v.as_str().parse().ok()?}}
    Some((parts,m.get(5).is_none(),m.get(5).map(|v|v.as_str()).unwrap_or("0").parse().ok()?))
}
fn get(url:&str,max:u64)->Result<Vec<u8>>{let http=client();let mut u=url::Url::parse(url)?;
    for _ in 0..6{let host=u.host_str().unwrap_or("");ensure!(u.scheme()=="https"&&(matches!(host,"github.com"|"api.github.com")||host.ends_with(".githubusercontent.com")),"Unexpected update host");let r=http.get(u.as_str()).call()?;if (300..400).contains(&r.status()){u=u.join(r.header("Location").context("Redirect missing")?)?;continue}
        let mut bytes=Vec::new();r.into_reader().take(max+1).read_to_end(&mut bytes)?;ensure!(bytes.len()as u64<=max,"Update too large");return Ok(bytes)
    }bail!("Too many redirects")
}
fn checked(stage:&Path)->Result<(String,Vec<u8>)>{
    let manifest=fs::read(stage.join("manifest.json"))?;let sig=fs::read_to_string(stage.join("manifest.sig"))?;ensure!(manifest.len()<=8192&&sig.len()<=2048,"Invalid manifest");
    let key=RsaPublicKey::new(BigUint::from_bytes_be(&STANDARD.decode(include_str!("../publisher.modulus").trim())?),65537u32.into())?;
    key.verify(Pkcs1v15Sign::new::<Sha256>(),&Sha256::digest(&manifest),&STANDARD.decode(sig.trim())?)?;
    let m:Value=serde_json::from_slice(&manifest)?;let v=s(&m,"version");ensure!(version(v).context("Invalid version")?>version(CURRENT).unwrap()&&m["archive"]==format!("TrackBridge-{v}-win.zip"),"Invalid update version");
    let zip=fs::read(stage.join("update.zip"))?;ensure!(zip.len()<=30<<20&&format!("{:x}",Sha256::digest(&zip))==s(&m,"sha256"),"SHA-256 mismatch");Ok((v.into(),zip))
}
fn inspect(exe:&Path,ver:&str)->Result<()>{
    use windows_sys::Win32::Storage::FileSystem::*;unsafe{
        let name=wide(exe.as_os_str());let size=GetFileVersionInfoSizeW(name.as_ptr(),std::ptr::null_mut());ensure!(size>0,"Missing EXE version");let mut data=vec![0u8;size as usize];ensure!(GetFileVersionInfoW(name.as_ptr(),0,size,data.as_mut_ptr() as _)!=0,"Invalid EXE version");
        let query=|key:&str|->Result<(*mut std::ffi::c_void,u32)>{let mut p=std::ptr::null_mut();let mut n=0;ensure!(VerQueryValueW(data.as_ptr() as _,wide(key.as_ref()).as_ptr(),&mut p,&mut n)!=0&&n>0,"Missing EXE metadata");Ok((p,n))};
        let (p,n)=query("\\")?;ensure!(n as usize>=std::mem::size_of::<VS_FIXEDFILEINFO>(),"Invalid version");let f=std::ptr::read_unaligned(p as *const VS_FIXEDFILEINFO);let parts=[f.dwFileVersionMS>>16,f.dwFileVersionMS&65535,f.dwFileVersionLS>>16,f.dwFileVersionLS&65535];ensure!(parts==version(ver).unwrap().0,"EXE version mismatch");
        let (p,n)=query("\\VarFileInfo\\Translation")?;ensure!(n>=4,"Invalid translation");let t=p as *const u16;let key=format!("\\StringFileInfo\\{:04x}{:04x}\\ProductName",*t,*t.add(1));let(p,n)=query(&key)?;ensure!(String::from_utf16_lossy(std::slice::from_raw_parts(p as *const u16,n as usize)).trim_end_matches('\0')=="TrackBridge","Wrong product");Ok(())
    }
}
fn plain(path:&Path)->Result<()>{if let Ok(m)=fs::symlink_metadata(path){ensure!(m.file_attributes()&0x400==0,"Unexpected filesystem link")}Ok(())}
fn extract(stage:&Path)->Result<()>{
    let(v,bytes)=checked(stage)?;let mut zip=zip::ZipArchive::new(Cursor::new(bytes))?;ensure!(zip.len()==FILES.len(),"Unexpected ZIP entries");
    let d=stage.join("payload");plain(&d)?;fs::create_dir_all(&d)?;for name in FILES{let entry=zip.by_name(name)?;ensure!(entry.size()<=20<<20,"Update entry too large");let path=d.join(name);plain(&path)?;let mut bytes=Vec::new();entry.take((20<<20)+1).read_to_end(&mut bytes)?;ensure!(bytes.len()<=20<<20,"Entry too large");fs::write(path,bytes)?}inspect(&d.join(FILES[0]),&v)
}
pub fn prepare(root:&Path)->Result<Option<PathBuf>>{
    let releases:Value=serde_json::from_slice(&get("https://api.github.com/repos/Rub1nchik702/TrackBridge/releases?per_page=30",4<<20)?)?;
    let best=releases.as_array().context("Invalid releases")?.iter().filter(|r|r["draft"]!=true&&s(r,"tag_name").starts_with('v')).filter_map(|r|Some((version(s(r,"tag_name"))?,r))).filter(|(v,_)|*v>version(CURRENT).unwrap()).max_by_key(|(v,_)|*v);
    let Some((_,r))=best else{return Ok(None)};let tag=s(r,"tag_name");let name=format!("TrackBridge-{}-win.zip",&tag[1..]);let assets=r["assets"].as_array().context("Missing assets")?;
    let asset=|n:&str|->Result<&Value>{let a=assets.iter().find(|a|s(a,"name")==n&&a["state"]=="uploaded").context("Incomplete release")?;ensure!(s(a,"browser_download_url")==format!("{REPO}/releases/download/{tag}/{n}"),"Invalid asset URL");Ok(a)};
    let a=asset(&name)?;let manifest=get(s(asset("TrackBridge-update.json")?,"browser_download_url"),8192)?;let signature=get(s(asset("TrackBridge-update.sig")?,"browser_download_url"),2048)?;
    let m:Value=serde_json::from_slice(&manifest)?;ensure!(m["version"]==tag[1..]&&s(a,"digest")==format!("sha256:{}",s(&m,"sha256")),"Asset digest/version mismatch");
    let bytes=get(s(a,"browser_download_url"),30<<20)?;let updates=root.join(".updates");plain(&updates)?;fs::create_dir_all(&updates)?;let stage=updates.join(format!("{:032x}",rand::random::<u128>()));fs::create_dir(&stage)?;
    fs::write(stage.join("manifest.json"),manifest)?;fs::write(stage.join("manifest.sig"),signature)?;fs::write(stage.join("update.zip"),bytes)?;extract(&stage)?;Ok(Some(stage))
}
pub fn launch(stage:&Path)->Result<()>{let exe=std::env::current_exe()?;let helper=stage.join("UpdateRunner.exe");fs::copy(&exe,&helper)?;Command::new(helper).arg("--apply-update").arg(std::process::id().to_string()).arg(stage).arg(exe).spawn()?;Ok(())}
pub fn apply(pid:u32,stage:&Path,target:&Path)->Result<()>{
    use windows_sys::Win32::{System::Threading::*,Foundation::*};
    ensure!(stage.parent()==target.parent().map(|p|p.join(".updates")).as_deref()&&std::env::current_exe()?.canonicalize()?==stage.join("UpdateRunner.exe").canonicalize()?,"Invalid stage");
    for d in [target.parent().unwrap(),stage.parent().unwrap(),stage,target]{plain(d)?}
    unsafe{let h=OpenProcess(PROCESS_SYNCHRONIZE,0,pid);if !h.is_null(){let res=WaitForSingleObject(h,120000);CloseHandle(h);ensure!(res==WAIT_OBJECT_0,"Application is still running")}}
    extract(stage)?;let backup=stage.join("backup");fs::create_dir(&backup)?;let mut changed=Vec::new();let result=(||->Result<()>{for name in FILES{let dest=if name==FILES[0]{target.into()}else{target.with_file_name(name)};plain(&dest)?;let bak=backup.join(name);if dest.exists(){fs::copy(&dest,&bak)?;}let bytes=fs::read(stage.join("payload").join(name))?;plain(&dest.with_extension("tmp"))?;crate::profile::atomic(&dest,&bytes)?;changed.push((dest,bak));}Ok(())})();
    if let Err(e)=result{for(dest,bak)in changed.into_iter().rev(){if bak.exists(){fs::copy(bak,dest)?;}else{fs::remove_file(dest)?}}return Err(e)}
    Command::new(target).spawn()?;Ok(())
}
