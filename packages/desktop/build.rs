fn main() {
    println!("cargo:rerun-if-changed=../../assets/icon/kitty-pro.ico");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let mut resource = winresource::WindowsResource::new();
    resource
        .set_icon("../../assets/icon/kitty-pro.ico")
        .set("ProductName", "Kitty Pro")
        .set("FileDescription", "Kitty Pro proxy client")
        .set("OriginalFilename", "Kitty-Pro.exe");
    resource
        .compile()
        .expect("failed to embed Windows application resources");
}
