#![windows_subsystem = "windows"]
mod language;mod api;mod events;mod orders;mod player;mod profile;mod runtime;mod sound;mod ui;mod update;mod checks;
use anyhow::{Context,Result};use std::{path::PathBuf,sync::{Arc,atomic::AtomicBool,mpsc}};
fn timestamp()->String{chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()}
fn icon()->eframe::egui::IconData{let img=image::load_from_memory(include_bytes!("../TrackBridge.ico")).expect("bundled icon").to_rgba8();eframe::egui::IconData{width:img.width(),height:img.height(),rgba:img.into_raw()}}
fn run()->Result<()>{
    let args:Vec<_>=std::env::args_os().collect();let has=|s:&str|args.iter().any(|x|x==s);let exe=std::env::current_exe()?;let root=exe.parent().context("Application directory")?.to_path_buf();
    if has("--self-test"){let report=checks::run();let text=match &report{Ok(n)=>format!("PASS: {n} Rust checks. Offline."),Err(e)=>format!("FAIL: {e:#}")};std::fs::write(root.join("self-test.txt"),text)?;report?;return Ok(())}
    if has("--apply-update"){anyhow::ensure!(args.len()==5,"Invalid update arguments");return update::apply(args[2].to_string_lossy().parse()?,&PathBuf::from(&args[3]),&PathBuf::from(&args[4]))}
    let preview=has("--preview-ui");let _instance=if preview{None}else{Some(profile::Instance::lock()?)};
    let option=|flag:&str|args.windows(2).find(|pair|pair[0]==flag).and_then(|pair|pair[1].to_str());
    let page=if preview {
        option("--preview-page").unwrap_or(if has("--preview-settings"){"settings"}else{"overview"})
    } else {"overview"};
    anyhow::ensure!(matches!(page,"overview"|"settings"|"wizard"|"history"|"review"|"demo"),"Unknown preview page");
    let dir=if preview{std::env::temp_dir().join(format!("TrackBridge-preview-{:x}",rand::random::<u128>()))}else{root.join("data")};
    let mut p=profile::Profile::open(dir.clone())?;
    if preview && let Some(language @ ("en" | "ru"))=option("--preview-language") {
        p.v["language"]=serde_json::json!(language);
    }
    let(tx,rx)=mpsc::channel();let(rtx,rrx)=mpsc::channel();let mut engine=runtime::Engine::new(p,rtx);
    if matches!(page,"history"|"review"|"demo"){engine.load_preview_orders();}
    let v=engine.view();let exit=Arc::new(AtomicBool::new(false));let c=exit.clone();let h=std::thread::spawn(move||engine.run(rx,c,preview));
    let shot=preview.then(||root.join(if page=="overview"{"ui-preview.png".into()}else{format!("ui-{page}-preview.png")}));
    let diag=if has("--ui-diagnostics"){let path=root.join("diagnostics");std::fs::create_dir_all(&path)?;Some(path.join(format!("ui-{}.log",profile::now())))}else{None};
    let opts=eframe::NativeOptions{viewport:eframe::egui::ViewportBuilder::default().with_inner_size([1150.,760.]).with_min_inner_size([820.,520.]).with_icon(icon()),..Default::default()};
    eframe::run_native("TrackBridge · Twitch → Spotify",opts,Box::new(move|cc|{
        crate::ui::style(&cc.egui_ctx);
        let mut desktop=ui::Desktop::new(v,tx,rrx,exit,h,root,preview,shot,page=="settings",diag);
        desktop.configure_preview(page);
        Ok(Box::new(desktop))
    })).map_err(|e|anyhow::anyhow!(e.to_string()))?;
    if preview{let _=std::fs::remove_file(dir.join("settings.bin"));let _=std::fs::remove_dir(dir);}Ok(())
}
fn main(){if let Err(e)=run(){if !std::env::args_os().any(|x|x=="--self-test"){rfd::MessageDialog::new().set_title("TrackBridge").set_description(format!("{e:#}")).set_level(rfd::MessageLevel::Error).show();}std::process::exit(1)}}
