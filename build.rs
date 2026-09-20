fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default() == "windows" {
        let mut res = winres::WindowsResource::new();
        res.set_icon("assets/app.ico");
        res.set("FileDescription", "ctrl-ime-key: US Keyboard IME Toggle");
        res.set("ProductName", "ctrl-ime-key");
        res.set("OriginalFilename", "ctrl-ime-key.exe");
        if let Err(e) = res.compile() {
            eprintln!("Failed to compile Windows resources: {}", e);
        }
    }
}

