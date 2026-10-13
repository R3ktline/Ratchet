//! Platform-specific adjustments.

use gtk::gdk;

/// GTK on Windows only supports whole-number scales: at 150% display
/// scaling it renders at 1x and everything looks two thirds the size it
/// would on Linux. This is the fractional part GTK leaves out, which the
/// app makes up by scaling fonts, icons and its own sizes (1 elsewhere).
/// `RATCHET_UI_SCALE` overrides the factor (1 turns this off).
#[cfg(windows)]
pub fn ui_scale() -> f64 {
    static SCALE: std::sync::OnceLock<f64> = std::sync::OnceLock::new();
    *SCALE.get_or_init(|| {
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
        if factor <= 1.01 { 1.0 } else { factor }
    })
}

#[cfg(not(windows))]
pub fn ui_scale() -> f64 {
    1.0
}

/// A size in pixels at the UI scale, for widths and heights set in code
/// (fonts and icons grow with the scale, so fixed boxes must too).
pub fn px(v: f64) -> i32 {
    (v * ui_scale()).round() as i32
}

/// Before GTK starts: on Windows, GTK only draws with the GPU (GL or
/// Vulkan) through Direct Composition, which is opt-in. Use it when the
/// machine has a single graphics adapter. With more than one (a laptop's
/// two GPUs, or a virtual display adapter such as Parsec's) the screen
/// flickers in fullscreen whenever a menu opens, so those stay in software,
/// which the app is kept cheap enough to animate in (see
/// NO_TRANSPARENCY_CSS). `RATCHET_GPU=1` / `RATCHET_GPU=0` force either.
#[cfg(windows)]
pub fn early_setup() {
    if std::env::var_os("GDK_DEBUG").is_some() {
        return;
    }
    let gpu = match std::env::var("RATCHET_GPU").as_deref() {
        Ok("1") => true,
        Ok("0") => false,
        _ => win32::hardware_adapters() == 1,
    };    if gpu {
        // SAFETY: called first thing in main, before any other thread runs.
        unsafe { std::env::set_var("GDK_DEBUG", "dcomp") };
    }
}

#[cfg(not(windows))]
pub fn early_setup() {}

/// With Direct Composition (see `early_setup`), GTK's Windows backend draws
/// see-through pixels as black: window and popover shadows, rounded
/// corners and popover arrows turn into black boxes. Draw nothing
/// see-through instead. The window keeps a 12px border around it for
/// resizing whatever the CSS says (libadwaita's window can't use the native
/// frame), so fill it with the window color and a hairline edge: a slim
/// frame instead of a black one. Popovers get a hairline border and no
/// arrow tail; corners are square.
#[cfg(windows)]
const NO_TRANSPARENCY_CSS: &str = "
window.csd, window.csd:backdrop {
  box-shadow: 0 0 0 12px var(--window-bg-color), 0 0 0 13px var(--border-color);
  margin: 0; border-radius: 0;
}
window.csd > .titlebar, window.csd headerbar { border-radius: 0; }
popover.background, popover.menu { box-shadow: none; margin: 0; padding: 0; }
popover > contents { box-shadow: none; border-radius: 0; border: 1px solid alpha(currentColor, 0.18); }
popover > arrow { background: none; border: none; }
tooltip.background { box-shadow: none; border-radius: 0; }
/* Dialogs: a blurred shadow and a fading backdrop meant software rendering
   repainted the whole window and re-blurred the shadow on every frame of
   the opening animation (~70ms a frame). A hairline border and a backdrop
   that appears at once cost almost nothing. */
floating-sheet > sheet, bottom-sheet > sheet { box-shadow: 0 0 0 1px var(--border-color); }
floating-sheet > dimming, bottom-sheet > dimming { background: none; }
floating-sheet, bottom-sheet { background: alpha(black, 0.28); }
";

/// Popover arrows sit in a see-through strip, which turns black (see
/// `NO_TRANSPARENCY_CSS`). CSS can't remove them, so switch the arrow off
/// on every popover as it is shown, wherever it comes from (our menus,
/// dropdowns and combo rows inside libadwaita, …).
#[cfg(windows)]
fn hide_popover_arrows() {
    use gtk::glib::gobject_ffi;
    use gtk::glib::translate::{from_glib_none, IntoGlib};
    use gtk::prelude::*;

    unsafe extern "C" fn on_show(
        _hint: *mut gobject_ffi::GSignalInvocationHint,
        n_values: u32,
        values: *const gobject_ffi::GValue,
        _data: gtk::glib::ffi::gpointer,
    ) -> gtk::glib::ffi::gboolean {
        if n_values > 0 {
            // SAFETY: the first value of a signal emission is the instance.
            let obj: Option<gtk::glib::Object> = unsafe { from_glib_none(gobject_ffi::g_value_get_object(values)) };
            if let Some(popover) = obj.and_then(|o| o.downcast::<gtk::Popover>().ok()) {
                popover.set_has_arrow(false);
            }
        }
        gtk::glib::ffi::GTRUE // keep the hook
    }

    // Signals exist once the class is set up, which may not have happened yet.
    let _class = gtk::glib::Class::<gtk::Popover>::from_type(gtk::Popover::static_type());
    let Some(show) = gtk::glib::subclass::signal::SignalId::lookup("show", gtk::Widget::static_type()) else { return };
    // SAFETY: a plain function hook with no data, kept for the app's life.
    unsafe {
        gobject_ffi::g_signal_add_emission_hook(show.into_glib(), 0, Some(on_show), std::ptr::null_mut(), None);
    }
}

#[cfg(windows)]
pub fn setup() {
    if let Some(display) = gdk::Display::default() {
        let provider = gtk::CssProvider::new();
        provider.load_from_string(NO_TRANSPARENCY_CSS);
        gtk::style_context_add_provider_for_display(&display, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 3);
    }
    hide_popover_arrows();
    let factor = ui_scale();
    if factor <= 1.0 {
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

    #[repr(C)]
    struct Guid(u32, u16, u16, [u8; 8]);

    /// IID_IDXGIFactory1.
    const IID_IDXGI_FACTORY1: Guid = Guid(0x770a_ae78, 0xf26f, 0x4dba, [0xa8, 0x29, 0x25, 0x3c, 0x83, 0xd1, 0xb3, 0x87]);
    const DXGI_ADAPTER_FLAG_SOFTWARE: u32 = 2;

    #[repr(C)]
    struct AdapterDesc1 {
        description: [u16; 128],
        vendor_id: u32,
        device_id: u32,
        sub_sys_id: u32,
        revision: u32,
        dedicated_video_memory: usize,
        dedicated_system_memory: usize,
        shared_system_memory: usize,
        luid: [u32; 2],
        flags: u32,
    }

    #[link(name = "dxgi")]
    unsafe extern "system" {
        fn CreateDXGIFactory1(riid: *const Guid, factory: *mut *mut c_void) -> i32;
    }

    /// Calls entry `index` of a COM object's vtable.
    unsafe fn vtable(obj: *mut c_void, index: usize) -> *const c_void {
        // SAFETY: a COM object starts with a pointer to its vtable.
        unsafe { *(*(obj as *const *const *const c_void)).add(index) }
    }

    /// How many hardware graphics adapters DXGI lists (real GPUs and
    /// virtual display adapters, not the software one). 0 if unknown.
    pub fn hardware_adapters() -> usize {
        type EnumAdapters1 = unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> i32;
        type GetDesc1 = unsafe extern "system" fn(*mut c_void, *mut AdapterDesc1) -> i32;
        type Release = unsafe extern "system" fn(*mut c_void) -> u32;
        // IDXGIFactory1::EnumAdapters1 and IDXGIAdapter1::GetDesc1 come after
        // IUnknown (3), IDXGIObject (4) and their base interface's methods.
        const ENUM_ADAPTERS1: usize = 12;
        const GET_DESC1: usize = 10;
        const RELEASE: usize = 2;

        let mut factory = std::ptr::null_mut();
        // SAFETY: plain COM calls through the documented vtable layouts; every
        // object obtained is released.
        unsafe {
            if CreateDXGIFactory1(&IID_IDXGI_FACTORY1, &mut factory) < 0 || factory.is_null() {
                return 0;
            }
            let enum_adapters: EnumAdapters1 = std::mem::transmute(vtable(factory, ENUM_ADAPTERS1));
            let mut count = 0;
            for i in 0.. {
                let mut adapter = std::ptr::null_mut();
                if enum_adapters(factory, i, &mut adapter) < 0 || adapter.is_null() {
                    break;
                }
                let get_desc: GetDesc1 = std::mem::transmute(vtable(adapter, GET_DESC1));
                let mut desc: AdapterDesc1 = std::mem::zeroed();
                if get_desc(adapter, &mut desc) >= 0 && desc.flags & DXGI_ADAPTER_FLAG_SOFTWARE == 0 {
                    count += 1;
                }
                let release: Release = std::mem::transmute(vtable(adapter, RELEASE));
                release(adapter);
            }
            let release: Release = std::mem::transmute(vtable(factory, RELEASE));
            release(factory);
            count
        }
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
