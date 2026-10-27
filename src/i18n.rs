//! UI translations with gettext, in the system's language. The catalogs
//! are po/*.po; build.rs compiles them to share/locale, which sits next to
//! the folder of the executable (target/, the Flatpak's /app, the Windows
//! install folder).

use gettextrs::{LocaleCategory, bind_textdomain_codeset, bindtextdomain, setlocale, textdomain};

const DOMAIN: &str = "ratchet";

pub fn init() {
    // SAFETY: called early in main, before any other thread exists.
    unsafe { setlocale(LocaleCategory::LcAll, "") };
    let dir = std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.parent()?.join("share").join("locale")));
    if let Some(dir) = dir {
        let _ = bindtextdomain(DOMAIN, dir);
    }
    let _ = bind_textdomain_codeset(DOMAIN, "UTF-8");
    let _ = textdomain(DOMAIN);
}

/// Marks a constant for translation; it's translated where it's shown.
pub const fn gettext_noop(msgid: &str) -> &str {
    msgid
}

/// `msgid` in the user's language.
pub fn tr(msgid: &str) -> String {
    gettextrs::gettext(msgid)
}

/// `tr`, then each `{}` filled with the next argument.
pub fn trf(msgid: &str, args: &[&str]) -> String {
    let mut out = tr(msgid);
    for arg in args {
        if let Some(at) = out.find("{}") {
            out.replace_range(at..at + 2, arg);
        }
    }
    out
}

/// "{} pages": the plural form for `n` in the user's language, with `n`
/// filled in.
pub fn trn(singular: &str, plural: &str, n: usize) -> String {
    gettextrs::ngettext(singular, plural, n as u32).replace("{}", &n.to_string())
}

#[cfg(test)]
mod tests {
    #[test]
    fn fills_in_order() {
        // Not a literal, so it stays out of the translation template.
        let msgid = "{} of {}";
        assert_eq!(super::trf(msgid, &["1", "3"]), "1 of 3");
    }
}
