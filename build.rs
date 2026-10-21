use std::path::Path;

fn main() {
    for asset in ["assets/app.ico", "assets/window-cap.manifest"] {
        println!("cargo:rerun-if-changed={asset}");
        assert!(Path::new(asset).exists(), "缺少资源文件: {asset}");
    }

    let mut resource = winresource::WindowsResource::new();
    resource.set_icon("assets/app.ico");
    resource.set_manifest_file("assets/window-cap.manifest");
    resource.set("ProductName", "window-cap");
    resource.set("FileDescription", "window-cap screenshot tool");
    resource.set("LegalCopyright", "MIT");

    if let Err(error) = resource.compile() {
        panic!("嵌入 Windows 资源失败: {error}");
    }
}
