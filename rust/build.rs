fn main(){
    let major: u16 = env!("CARGO_PKG_VERSION_MAJOR").parse().unwrap();
    let minor: u16 = env!("CARGO_PKG_VERSION_MINOR").parse().unwrap();
    let patch: u16 = env!("CARGO_PKG_VERSION_PATCH").parse().unwrap();
    let display = if patch == 0 { format!("{major}.{minor}") } else { format!("{major}.{minor}.{patch}") };
    let numeric = (u64::from(major) << 48) | (u64::from(minor) << 32) | (u64::from(patch) << 16);
    println!("cargo:rustc-env=TRACKBRIDGE_VERSION={display}");
    let mut r=winresource::WindowsResource::new();
    r.set_icon("TrackBridge.ico").set("ProductName","TrackBridge").set("FileDescription","TrackBridge").set("OriginalFilename","TrackBridge.exe").set("FileVersion",&display).set("ProductVersion",&display);
    r.set_version_info(winresource::VersionInfo::FILEVERSION,numeric);
    r.set_version_info(winresource::VersionInfo::PRODUCTVERSION,numeric);
    if let Ok(path)=std::env::var("RC"){r.set_windres_path(&path);}
    if let Ok(path)=std::env::var("AR"){r.set_ar_path(&path);}
    r.compile().expect("Windows application resources");
}
