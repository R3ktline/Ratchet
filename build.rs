fn main() {
    glib_build_tools::compile_resources(&["data"], "data/ratchet.gresource.xml", "ratchet.gresource");
    compile_translations();

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

/// po/<lang>.po -> share/locale/<lang>/LC_MESSAGES/ratchet.mo beside the
/// folder the executable is built into (target/share for target/release),
/// where i18n.rs looks for it. Packaging copies that folder.
fn compile_translations() {
    use std::path::PathBuf;
    use std::process::Command;

    println!("cargo:rerun-if-changed=po");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    // OUT_DIR is <target>/<profile>/build/<crate>-<hash>/out.
    let Some(locale) = out.ancestors().nth(4).map(|t| t.join("share").join("locale")) else { return };
    let langs = std::fs::read_to_string("po/LINGUAS").unwrap_or_default();
    for lang in langs.split_whitespace() {
        println!("cargo:rerun-if-changed=po/{lang}.po");
        let dir = locale.join(lang).join("LC_MESSAGES");
        std::fs::create_dir_all(&dir).expect("create locale folder");
        let status = Command::new("msgfmt")
            .arg("-o")
            .arg(dir.join("ratchet.mo"))
            .arg(format!("po/{lang}.po"))
            .status();
        match status {
            Ok(s) if s.success() => {}
            Ok(s) => panic!("msgfmt failed on po/{lang}.po ({s})"),
            // Without gettext's tools the app still runs, in English.
            Err(e) => {
                println!("cargo:warning=No translations: can't run msgfmt ({e}); install gettext");
                return;
            }
        }
    }
}
