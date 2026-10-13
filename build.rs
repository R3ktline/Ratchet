fn main() {
    glib_build_tools::compile_resources(&["data"], "data/ratchet.gresource.xml", "ratchet.gresource");

    // Icon for the .exe in Explorer and the taskbar.
    #[cfg(windows)]
    {
        println!("cargo:rerun-if-changed=data/windows/ratchet.ico");
        winresource::WindowsResource::new()
            .set_icon("data/windows/ratchet.ico")
            .compile()
            .expect("embed Windows icon");
    }
}
