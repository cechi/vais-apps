//! Embeds the application icon into the Windows executable; a no-op elsewhere.

fn main() {
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/vais-commander.ico");
        res.set("ProductName", "VAIS Commander");
        res.set("FileDescription", "VAIS Commander file manager");
        res.compile().expect("failed to compile Windows resources");
    }
    println!("cargo:rerun-if-changed=assets/vais-commander.ico");
}
