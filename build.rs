fn main() {
    println!("cargo:rerun-if-changed=assets/campus-auth.ico");
    println!("cargo:rerun-if-changed=Cargo.toml");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("assets/campus-auth.ico")
            .set("ProductName", "CampusAuth")
            .set("FileDescription", "CampusAuth 校园网认证")
            .set("CompanyName", "CampusAuth contributors")
            .set("LegalCopyright", "Copyright 2026 CampusAuth contributors. MIT license.");
        resource.compile().expect("Could not compile Windows executable metadata");
    }
    let root=std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let ui=root.join("ui/dist");
    let output=std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("web_assets.rs");
    let mut entries=Vec::new();
    fn collect(root:&std::path::Path,dir:&std::path::Path,entries:&mut Vec<(String,std::path::PathBuf)>){
        if let Ok(files)=std::fs::read_dir(dir){for file in files.flatten(){let path=file.path();if path.is_dir(){collect(root,&path,entries);}else{entries.push((path.strip_prefix(root).unwrap().to_string_lossy().replace('\\',"/"),path));}}}
    }
    if ui.join("index.html").is_file(){
        println!("cargo:rerun-if-changed={}",ui.display());collect(&ui,&ui,&mut entries);
    }else{
        println!("cargo:rerun-if-changed={}",ui.display());
        let placeholder="<!doctype html><html lang=\"zh-CN\"><meta charset=\"UTF-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>CampusAuth 前端尚未构建</title><body><main><h1>前端尚未构建</h1><p>这是源码开发构建。请先在 ui 目录运行 npm ci 和 npm run build，再重新构建 Rust 程序。</p><p>普通用户请下载完整的 CampusAuth 正式发行包。</p></main></body></html>";
        std::fs::write(output,format!("pub fn lookup(path:&str)->Option<(&'static [u8], &'static str)>{{match path.trim_start_matches('/'){{\"\"|\"index.html\"=>Some(({placeholder:?}.as_bytes(),\"text/html; charset=utf-8\")),_=>None}}}}\n")).unwrap();
        return;
    }
    let mut source=String::from("pub fn lookup(path:&str)->Option<(&'static [u8], &'static str)>{match path.trim_start_matches('/') {\n");
    entries.sort_by(|a,b|a.0.cmp(&b.0));
    for (name,path) in entries{
        if name.ends_with(".map"){continue;}
        let mime=match path.extension().and_then(|v|v.to_str()).unwrap_or(""){
            "html"=>"text/html; charset=utf-8","js"=>"text/javascript; charset=utf-8","css"=>"text/css; charset=utf-8","svg"=>"image/svg+xml","png"=>"image/png","ico"=>"image/x-icon","ttf"=>"font/ttf","woff2"=>"font/woff2",_=>"application/octet-stream"};
        if name=="index.html" {source.push_str(&format!("\"\" | "));}
        source.push_str(&format!("{:?} => Some((include_bytes!({:?}), {:?})),\n",name,path.to_string_lossy().replace('\\',"/"),mime));
    }
    source.push_str("_=>None}}\n");std::fs::write(output,source).unwrap();
}
