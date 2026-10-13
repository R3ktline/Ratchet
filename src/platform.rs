//! Platform-specific adjustments.

use gtk::gdk;

/// GTK on Windows only supports whole-number scales: at 150% display
/// scaling it renders at 1x and everything looks two thirds the size it
/// would on Linux. Make up the fractional part by scaling fonts and icons.
/// `RATCHET_UI_SCALE` overrides the factor (1 turns this off).
#[cfg(windows)]
pub fn setup() {
    let factor = std::env::var("RATCHET_UI_SCALE")
        .ok()
        .and_then(|s| s.trim().parse::<f64>().ok())
        .filter(|f| (0.5..=4.0).contains(f))
        .unwrap_or_else(|| {
            // SAFETY: no arguments; available since Windows 10 1607.
            let dpi = unsafe { win32::GetDpiForSystem() } as f64;
            // What GDK uses: the integer part of the scale, at least 1.
            let gdk_scale = (dpi / 96.0).floor().max(1.0);
            dpi / 96.0 / gdk_scale
        });
    if factor <= 1.01 {
        return;
    }
    // The Windows backend ignores gtk-xft-dpi, so scale the font size.
    if let Some(settings) = gtk::Settings::default() {
        let name = settings.gtk_font_name().map(|s| s.to_string()).unwrap_or_default();
        let mut font = gtk::pango::FontDescription::from_string(&name);
        let size = if font.size() > 0 { font.size() as f64 } else { 9.0 * gtk::pango::SCALE as f64 };
        font.set_size((size * factor).round() as i32);
        settings.set_gtk_font_name(Some(&font.to_string()));
    }
    // Inherited, so widgets with their own icon size (status pages, large
    // icons) keep it.
    let provider = gtk::CssProvider::new();
    provider.load_from_string(&format!("window {{ -gtk-icon-size: {}px; }}", (16.0 * factor).round()));
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(&display, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1);
    }
}

#[cfg(not(windows))]
pub fn setup() {}

/// Pen side buttons GTK doesn't report, as held-button masks.
///
/// With Windows Ink, GTK ignores the pen's barrel flag, so a side button
/// pressed while hovering never shows up in its events. `hook_pen` watches
/// the window's pointer messages and keeps the flag here; the barrel counts
/// as button 3 (what GTK reports it as while the tip is down).
#[cfg(windows)]
pub fn pen_buttons() -> gdk::ModifierType {
    if win32::PEN_FLAGS.get() & win32::PEN_FLAG_BARREL != 0 {
        gdk::ModifierType::BUTTON3_MASK
    } else {
        gdk::ModifierType::empty()
    }
}

#[cfg(not(windows))]
pub fn pen_buttons() -> gdk::ModifierType {
    gdk::ModifierType::empty()
}

/// Start following the pen on the window behind `surface` (see
/// `pen_buttons`), and turn off Windows' press-and-hold right click there,
/// which otherwise takes over the pen when the side button is held.
#[cfg(windows)]
pub fn hook_pen(surface: &gdk::Surface) {
    use gtk::glib::translate::ToGlibPtr;
    use std::ffi::c_void;

    // SAFETY: a realized surface on the Windows backend.
    let hwnd = unsafe { win32::gdk_win32_surface_get_handle(ToGlibPtr::<*mut gdk::ffi::GdkSurface>::to_glib_none(surface).0 as *mut c_void) };
    if hwnd.is_null() {
        return;
    }
    let name: Vec<u16> = "MicrosoftTabletPenServiceProperty\0".encode_utf16().collect();
    // SAFETY: valid window and NUL-terminated name; subclassing the same
    // window again with the same id only replaces it.
    unsafe {
        win32::SetPropW(hwnd, name.as_ptr(), win32::TABLET_DISABLE_PRESSANDHOLD as *mut c_void);
        win32::SetWindowSubclass(hwnd, win32::subclass_proc, 0x5241_5443, 0);
    }
}

#[cfg(not(windows))]
pub fn hook_pen(_surface: &gdk::Surface) {}

#[cfg(windows)]
#[allow(non_snake_case)]
mod win32 {
    use std::cell::Cell;
    use std::ffi::c_void;

    pub type Hwnd = *mut c_void;

    pub const PEN_FLAG_BARREL: u32 = 0x1;
    pub const TABLET_DISABLE_PRESSANDHOLD: usize = 0x1;
    const PT_PEN: u32 = 3;
    const WM_POINTERUPDATE: u32 = 0x0245;
    const WM_POINTERDOWN: u32 = 0x0246;
    const WM_POINTERUP: u32 = 0x0247;
    const WM_POINTERLEAVE: u32 = 0x024A;

    thread_local! {
        /// `penFlags` of the last pen pointer message (UI thread only).
        pub static PEN_FLAGS: Cell<u32> = const { Cell::new(0) };
    }

    #[repr(C)]
    struct PointerInfo {
        pointer_type: u32,
        pointer_id: u32,
        frame_id: u32,
        pointer_flags: u32,
        source_device: *mut c_void,
        hwnd_target: Hwnd,
        pt_pixel_location: [i32; 2],
        pt_himetric_location: [i32; 2],
        pt_pixel_location_raw: [i32; 2],
        pt_himetric_location_raw: [i32; 2],
        time: u32,
        history_count: u32,
        input_data: i32,
        key_states: u32,
        performance_count: u64,
        button_change_type: u32,
    }

    #[repr(C)]
    struct PointerPenInfo {
        info: PointerInfo,
        pen_flags: u32,
        pen_mask: u32,
        pressure: u32,
        rotation: u32,
        tilt_x: i32,
        tilt_y: i32,
    }

    #[link(name = "user32")]
    unsafe extern "system" {
        pub fn GetDpiForSystem() -> u32;
        pub fn SetPropW(hwnd: Hwnd, name: *const u16, data: *mut c_void) -> i32;
        fn GetPointerType(id: u32, kind: *mut u32) -> i32;
        fn GetPointerPenInfo(id: u32, info: *mut PointerPenInfo) -> i32;
    }

    type SubclassProc = unsafe extern "system" fn(Hwnd, u32, usize, isize, usize, usize) -> isize;

    #[link(name = "comctl32")]
    unsafe extern "system" {
        pub fn SetWindowSubclass(hwnd: Hwnd, proc: SubclassProc, id: usize, data: usize) -> i32;
        fn DefSubclassProc(hwnd: Hwnd, msg: u32, wparam: usize, lparam: isize) -> isize;
    }

    unsafe extern "C" {
        pub fn gdk_win32_surface_get_handle(surface: *mut c_void) -> Hwnd;
    }

    pub unsafe extern "system" fn subclass_proc(
        hwnd: Hwnd,
        msg: u32,
        wparam: usize,
        lparam: isize,
        _id: usize,
        _data: usize,
    ) -> isize {
        if matches!(msg, WM_POINTERUPDATE | WM_POINTERDOWN | WM_POINTERUP | WM_POINTERLEAVE) {
            let id = (wparam & 0xffff) as u32;
            let mut kind = 0;
            // SAFETY: plain out-parameters for the pointer in this message.
            unsafe {
                if GetPointerType(id, &mut kind) != 0 && kind == PT_PEN {
                    let mut info: PointerPenInfo = std::mem::zeroed();
                    let flags = if msg == WM_POINTERLEAVE {
                        0
                    } else if GetPointerPenInfo(id, &mut info) != 0 {
                        info.pen_flags
                    } else {
                        PEN_FLAGS.get()
                    };
                    PEN_FLAGS.set(flags);
                }
            }
        }
        // Then GTK handles the message as usual.
        // SAFETY: forwarding the message unchanged.
        unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
    }
}
