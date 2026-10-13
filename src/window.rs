//! Main window: notebook/section/page tree in a sidebar, page in the content.

use std::cell::RefCell;
use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::time::Duration;

use adw::prelude::*;
use gtk::{gdk, gio, glib};

use crate::canvas::{Brush, Canvas, CanvasTool, DebugFlags, EntryLook, RadialEntry};
use crate::ink::Tool;
use crate::debug::{self, LogWindow};
use crate::ink::PX_PER_MM;
use crate::export;
use crate::export_dialog;
use crate::library::{Detached, Library, PageFile, Target};
use crate::settings::ExportFormat;
use crate::page_menu;
use crate::platform;
use crate::title_layout::TitleLayout;
use crate::preferences::{self, Prefs};
use crate::settings::{ActiveTool, RadialItem, Settings, Theme};
use crate::toolbar::{self, ToolState, Toolbar};

const SIDEBAR_ANIMATION_MS: u32 = 160;

/// Runs after an inline rename finishes.
type AfterRename = Box<dyn FnOnce(&Rc<Window>)>;
/// Handler for a window action, with its decoded target argument.
type ActionFn = Box<dyn Fn(&Rc<Window>, Option<Target>)>;

/// A page identified by folder/file names, which survive reordering.
type PageKey = [String; 3];

/// A removed item waiting for its "Undo" toast to expire before being trashed.
struct PendingDelete {
    id: u64,
    detached: Detached,
    path: PathBuf,
}

struct State {
    lib: Library,
    settings: Settings,
    /// Expanded notebooks ("nb") and sections ("nb/sec"), by folder name.
    expanded: HashSet<String>,
    open_page: Option<PageKey>,
    /// Section that "New page" without a target adds to.
    current_section: Option<[String; 2]>,
    /// Sidebar row index -> item.
    rows: Vec<Target>,
    pending: Option<PendingDelete>,
    /// An inline rename is in progress.
    editing: bool,
    next_pending_id: u64,
}

pub struct Window {
    window: adw::ApplicationWindow,
    list: gtk::ListBox,
    /// Wraps `list`; parents the context menu, since `ListBox::remove_all`
    /// would loop forever trying to remove a non-row child.
    list_box: gtk::Box,
    menu: gtk::PopoverMenu,
    toasts: adw::ToastOverlay,
    title: adw::WindowTitle,
    stack: gtk::Stack,
    canvas: Canvas,
    toolbar: Rc<Toolbar>,
    /// Header slot for the toolbar.
    tools_scroller: gtk::ScrolledWindow,
    zoom_button: gtk::Button,
    page_button: gtk::MenuButton,
    debug_flags: std::cell::Cell<DebugFlags>,
    log_window: std::cell::OnceCell<LogWindow>,
    split: std::cell::OnceCell<adw::OverlaySplitView>,
    /// Built once (ahead of time, when idle) and reused: building it on
    /// every open stalled the opening animation.
    prefs_dialog: std::cell::OnceCell<adw::Dialog>,
    state: RefCell<State>,
}

/// Upgrade a weak `Window` reference or bail out of the closure.
macro_rules! upgrade {
    ($weak:expr) => {
        upgrade!($weak, ())
    };
    ($weak:expr, $ret:expr) => {
        match $weak.upgrade() {
            Some(w) => w,
            None => return $ret,
        }
    };
}

fn encode(t: Target) -> String {
    match t {
        Target::Notebook(n) => format!("n:{n}"),
        Target::Section(n, s) => format!("s:{n}:{s}"),
        Target::Page(n, s, p) => format!("p:{n}:{s}:{p}"),
    }
}

fn decode(s: &str) -> Option<Target> {
    let mut parts = s.split(':');
    let kind = parts.next()?;
    let nums: Vec<usize> = parts.map(|p| p.parse().ok()).collect::<Option<_>>()?;
    match (kind, nums.as_slice()) {
        ("n", [n]) => Some(Target::Notebook(*n)),
        ("s", [n, s]) => Some(Target::Section(*n, *s)),
        ("p", [n, s, p]) => Some(Target::Page(*n, *s, *p)),
        _ => None,
    }
}

/// Where a dragged item lands relative to the row under the pointer.
#[derive(Clone, Copy, PartialEq)]
enum DropPos {
    Before,
    After,
    Into,
}

impl Window {
    pub fn new(app: &adw::Application) -> Rc<Self> {
        let settings = Settings::load();
        let expanded = settings.expanded.iter().cloned().collect();
        settings.theme.apply();
        let root = settings.library_dir();
        let (lib, load_error) = match Library::load(&root) {
            Ok(lib) => (lib, None),
            Err(e) => (
                Library { root: root.clone(), notebooks: Vec::new() },
                Some(format!("Couldn't open {}: {e}", root.display())),
            ),
        };

        let list = gtk::ListBox::builder()
            .css_classes(["navigation-sidebar"])
            .selection_mode(gtk::SelectionMode::Single)
            .build();
        let menu = gtk::PopoverMenu::from_model(None::<&gio::MenuModel>);
        menu.set_has_arrow(false);
        menu.set_halign(gtk::Align::Start);
        let list_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        list_box.append(&list);
        menu.set_parent(&list_box);

        let title = adw::WindowTitle::new("Ratchet", "");
        let stack = gtk::Stack::new();

        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("Ratchet")
            // Scaled like the rest of the UI, so the toolbar fits at first.
            .default_width(platform::px(1280.0))
            .default_height(platform::px(800.0))
            .width_request(360)
            .height_request(300)
            .build();

        let tools = ToolState {
            pens: settings.pens.clone(),
            active: settings.active_tool,
            eraser_mode: settings.eraser_mode,
            text_size: settings.text_size,
            text_font: settings.text_font.clone(),
            show_text_tool: settings.show_text_tool,
        };
        let this = Rc::new_cyclic(|weak: &Weak<Window>| {
            let (w1, w2, w3) = (weak.clone(), weak.clone(), weak.clone());
            let toolbar = Toolbar::new(
                tools,
                toolbar::Callbacks {
                    on_change: Box::new(move |t| {
                        if let Some(this) = w1.upgrade() {
                            this.tools_changed(t);
                        }
                    }),
                    on_pick: Box::new(move |slot| {
                        let Some(this) = w2.upgrade() else { return false };
                        this.canvas.restyle_selection(slot.color, slot.width_mm * PX_PER_MM)
                    }),
                    on_text: Box::new(move |cmd| {
                        if let Some(this) = w3.upgrade() {
                            this.canvas.text_command(cmd);
                        }
                    }),
                },
            );
            Window {
            window,
            list,
            list_box,
            menu,
            toasts: adw::ToastOverlay::new(),
            title,
            stack,
            canvas: Canvas::new(),
            toolbar,
            tools_scroller: gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::External)
                .vscrollbar_policy(gtk::PolicyType::Never)
                .propagate_natural_width(true)
                .build(),

            zoom_button: gtk::Button::builder()
                .label("100%")
                .tooltip_text("Reset Zoom (Ctrl+0)")
                .action_name("win.zoom-reset")
                .css_classes(["flat", "numeric", "text-button"])
                .build(),
            page_button: gtk::MenuButton::builder()
                .icon_name("document-page-setup-symbolic")
                .tooltip_text("Page Size and Background")
                .css_classes(["flat"])
                .build(),
            debug_flags: Default::default(),
            log_window: Default::default(),
            split: Default::default(),
            prefs_dialog: Default::default(),
            state: RefCell::new(State {
                lib,
                settings,
                expanded,
                open_page: None,
                current_section: None,
                rows: Vec::new(),
                pending: None,
                next_pending_id: 0,
                editing: false,
            }),
        }
        });
        this.build_layout();
        this.apply_tools();
        this.install_actions(app);
        this.install_sidebar_input();
        this.restore_last_page();
        this.rebuild_sidebar();
        this.update_content();
        if let Some(msg) = load_error {
            this.toast(&msg);
        }
        this.dev_show();
        // Build the preferences ahead of time, once the window is up, so
        // opening them doesn't stall.
        let weak = this.weak();
        glib::timeout_add_local_once(Duration::from_millis(800), move || {
            if let Some(this) = weak.upgrade() {
                this.preferences_dialog();
            }
        });
        this
    }

    /// `RATCHET_SHOW=perf…`: run `step` six times (e.g. open and close the
    /// sidebar or a dialog), then print the renderer and the time between
    /// frames (how smooth the animations were) to stderr and quit.
    fn perf_probe(self: &Rc<Self>, step_fn: Box<dyn Fn(u32)>) {
        let renderer = self.window.renderer().map(|r| r.type_().name().to_string()).unwrap_or_default();
        let gaps: Rc<RefCell<Vec<f64>>> = Rc::default();
        // When the last step ran, and which, to place slow frames.
        let stepped = Rc::new(std::cell::Cell::new((std::time::Instant::now(), 0u32)));
        {
            let (gaps, stepped) = (gaps.clone(), stepped.clone());
            let last = std::cell::Cell::new(0i64);
            self.window.add_tick_callback(move |_, clock| {
                let t = clock.frame_time();
                if last.get() != 0 {
                    let gap = (t - last.get()) as f64 / 1000.0;
                    gaps.borrow_mut().push(gap);
                    let (at, n) = stepped.get();
                    if gap > 17.0 && n > 0 {
                        eprintln!("PERF slow frame {gap:.1}ms, {:.0}ms after step {n}", at.elapsed().as_secs_f64() * 1000.0);
                    }
                }
                last.set(t);
                glib::ControlFlow::Continue
            });
        }
        let step = std::cell::Cell::new(0);
        glib::timeout_add_local(Duration::from_millis(700), move || {
            step.set(step.get() + 1);
            if step.get() <= 6 {
                if step.get() == 1 {
                    gaps.borrow_mut().clear();
                }
                stepped.set((std::time::Instant::now(), step.get()));
                step_fn(step.get());
                return glib::ControlFlow::Continue;
            }
            let mut g = gaps.borrow().clone();
            g.sort_by(f64::total_cmp);
            let pct = |p: usize| g.get(g.len() * p / 100).copied().unwrap_or(0.0);
            let slow = g.iter().filter(|&&x| x > 17.0).count();
            eprintln!(
                "PERF renderer={renderer} frames={} median={:.1}ms p95={:.1}ms max={:.1}ms over-16.7ms={slow}",
                g.len(),
                pct(50),
                pct(95),
                g.last().copied().unwrap_or(0.0)
            );
            std::process::exit(0);
        });
    }

    /// Developer aid for reviewing the UI from screenshots without driving
    /// the mouse: `RATCHET_SHOW=menu|page|export|prefs|fullscreen-menu|perf`
    /// opens that part of the UI (or runs that check) shortly after start.
    fn dev_show(self: &Rc<Self>) {
        let Ok(what) = std::env::var("RATCHET_SHOW") else { return };
        let weak = self.weak();
        glib::timeout_add_local_once(Duration::from_millis(1500), move || {
            let this = upgrade!(weak);
            match what.as_str() {
                "page" => this.page_button.popup(),
                // Slide the sidebar a few times, print frame times, quit.
                "perf" => {
                    let split = this.split.get().cloned();
                    this.perf_probe(Box::new(move |_| {
                        if let Some(s) = &split {
                            s.set_show_sidebar(!s.shows_sidebar());
                        }
                    }));
                }
                // The same without the canvas, to see what it costs.
                "perf-blank" => {
                    this.stack.set_visible_child_name("empty");
                    let split = this.split.get().cloned();
                    this.perf_probe(Box::new(move |_| {
                        if let Some(s) = &split {
                            s.set_show_sidebar(!s.shows_sidebar());
                        }
                    }));
                }
                // Open and close a dialog.
                "perf-export" | "perf-prefs" => {
                    let action = if what == "perf-export" { "win.export-page" } else { "win.preferences" };
                    let window = this.window.clone();
                    this.perf_probe(Box::new(move |step| {
                        if step % 2 == 1 {
                            let _ = WidgetExt::activate_action(&window, action, None);
                        } else if let Some(d) = window.visible_dialog() {
                            d.close();
                        }
                    }));
                }
                // Fullscreen, then open and close the page menu a few times.
                "fullscreen-menu" => {
                    this.window.fullscreen();
                    let n = Rc::new(std::cell::Cell::new(0));
                    let b = this.page_button.clone();
                    glib::timeout_add_local(Duration::from_millis(800), move || {
                        n.set(n.get() + 1);
                        if n.get() > 8 {
                            return glib::ControlFlow::Break;
                        }
                        if n.get() % 2 == 1 {
                            b.popup();
                        } else {
                            b.popdown();
                        }
                        glib::ControlFlow::Continue
                    });
                }
                "export" => {
                    let _ = WidgetExt::activate_action(&this.window, "win.export-page", None);
                }
                "prefs" => {
                    let _ = WidgetExt::activate_action(&this.window, "win.preferences", None);
                }
                "menu" => {
                    fn find(w: &gtk::Widget) -> Option<gtk::MenuButton> {
                        if let Some(b) = w.downcast_ref::<gtk::MenuButton>()
                            && b.has_css_class("app-logo")
                            && b.is_mapped()
                        {
                            return Some(b.clone());
                        }
                        let mut c = w.first_child();
                        while let Some(x) = c {
                            if let Some(b) = find(&x) {
                                return Some(b);
                            }
                            c = x.next_sibling();
                        }
                        None
                    }
                    if let Some(b) = find(this.window.upcast_ref()) {
                        b.popup();
                    }
                }
                _ => {}
            }
        });
    }

    pub fn present(&self) {
        self.window.present();
    }

    fn weak(self: &Rc<Self>) -> Weak<Self> {
        Rc::downgrade(self)
    }

    // ---- layout ----------------------------------------------------------------

    fn build_layout(self: &Rc<Self>) {
        // Sidebar
        let new_notebook = gtk::Button::builder()
            .icon_name("list-add-symbolic")
            .tooltip_text("New Notebook (Ctrl+Shift+N)")
            .action_name("win.new-notebook")
            .build();
        let main_menu = gio::Menu::new();
        let theme = gio::Menu::new();
        theme.append(Some("Follow System"), Some("win.theme::system"));
        theme.append(Some("Light"), Some("win.theme::light"));
        theme.append(Some("Dark"), Some("win.theme::dark"));
        main_menu.append_section(Some("Appearance"), &theme);
        let folder = gio::Menu::new();
        folder.append(Some("Open Notebooks Folder"), Some("win.open-folder"));
        folder.append(Some("Change Notebooks Folder…"), Some("win.change-folder"));
        main_menu.append_section(None, &folder);
        let about = gio::Menu::new();
        about.append(Some("Preferences"), Some("win.preferences"));
        about.append(Some("About Ratchet"), Some("win.about"));
        main_menu.append_section(None, &about);
        if debug::enabled() {
            let dbg = gio::Menu::new();
            dbg.append(Some("Pen Input Overlay"), Some("win.debug-input"));
            dbg.append(Some("Show Stroke Points"), Some("win.debug-points"));
            dbg.append(Some("Performance Stats"), Some("win.debug-perf"));
            dbg.append(Some("Event Log"), Some("win.debug-log"));
            main_menu.append_section(Some("Debug"), &dbg);
        }
        // The app logo doubles as the main menu button.
        // The app logo doubles as the main menu button.
        let logo_button = || {
            let button = gtk::MenuButton::builder()
                .icon_name("app.ratchet.notes-symbolic")
                .menu_model(&main_menu)
                .tooltip_text("Main Menu")
                .css_classes(["app-logo"])
                .primary(true)
                .build();
            // Open along the button's left edge, not centered on it (which
            // runs off the window at the far left).
            if let Some(popover) = button.popover() {
                popover.set_halign(gtk::Align::Start);
            }
            button
        };
        let menu_button = logo_button();

        let sidebar_header = adw::HeaderBar::builder().css_classes(["rt-header"]).build();
        sidebar_header.set_title_widget(Some(&adw::WindowTitle::new("Ratchet", "")));
        sidebar_header.pack_start(&menu_button);
        sidebar_header.pack_end(&new_notebook);

        let sidebar = adw::ToolbarView::new();
        sidebar.add_top_bar(&sidebar_header);
        sidebar.set_content(Some(
            &gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Never)
                .child(&self.list_box)
                .build(),
        ));

        // Content
        let toggle_sidebar = gtk::ToggleButton::builder()
            .icon_name("sidebar-show-symbolic")
            .tooltip_text("Toggle Sidebar (F9)")
            .active(true)
            .build();
        let new_page = gtk::Button::builder()
            .icon_name("document-new-symbolic")
            .tooltip_text("New Page (Ctrl+N)")
            .action_name("win.new-page-here")
            .build();
        // Shown while the sidebar (and its logo) is hidden.
        let content_logo = logo_button();
        let start_controls = gtk::WindowControls::new(gtk::PackType::Start);
        let end_controls = gtk::WindowControls::new(gtk::PackType::End);
        // The logo buttons take the place of the window icon some layouts
        // show, which is the fixed-color app icon.
        if let Some(settings) = gtk::Settings::default() {
            let sync = {
                let (sidebar_header, start, end) = (sidebar_header.clone(), start_controls.clone(), end_controls.clone());
                move |settings: &gtk::Settings| {
                    let layout = without_icon(&settings.gtk_decoration_layout().unwrap_or_default());
                    sidebar_header.set_decoration_layout(Some(&layout));
                    start.set_decoration_layout(Some(&layout));
                    end.set_decoration_layout(Some(&layout));
                }
            };
            sync(&settings);
            settings.connect_gtk_decoration_layout_notify(sync);
        }
        // Tools scroll sideways in the header instead of forcing a wide window.
        self.tools_scroller.set_child(Some(&self.toolbar.widget));
        self.tools_scroller.set_hexpand(true);
        self.toolbar.widget.set_halign(gtk::Align::Start);
        // Export gets a labeled button of its own, apart from the view
        // controls, so it's easy to find.
        let export = gtk::Button::builder()
            .child(&adw::ButtonContent::builder().icon_name("send-to-symbolic").label("Export").build())
            .tooltip_text("Export Page (Ctrl+E)")
            .action_name("win.export-page")
            .valign(gtk::Align::Center)
            .css_classes(["rt-export"])
            .build();
        // Shown together with the other page controls.
        self.page_button.bind_property("visible", &export, "visible").sync_create().build();
        // Page view controls as one cluster, which hides along with them.
        let page_group = gtk::Box::builder().css_classes(["rt-tool-group"]).valign(gtk::Align::Center).build();
        for b in [self.zoom_button.upcast_ref::<gtk::Widget>(), self.page_button.upcast_ref()] {
            page_group.append(b);
        }
        self.page_button.bind_property("visible", &page_group, "visible").sync_create().build();
        let weak = self.weak();
        self.page_button.set_create_popup_func(move |button| {
            let this = upgrade!(weak);
            button.set_popover(Some(&this.page_popover()));
        });
        let toolbar = self.toolbar.clone();
        self.canvas.connect_temp_tool(move |action| toolbar.set_temp_tool(action));
        let weak = self.weak();
        self.canvas.connect_menu(move |item| {
            let this = upgrade!(weak);
            this.radial_pick(item);
        });
        let zoom_button = self.zoom_button.clone();
        self.canvas.connect_zoom(move |z| zoom_button.set_label(&format!("{:.0}%", z * 100.0)));

        let create_first = gtk::Button::builder()
            .label("New Notebook")
            .halign(gtk::Align::Center)
            .css_classes(["text-button", "pill", "suggested-action"])
            .action_name("win.new-notebook")
            .build();
        let empty = adw::StatusPage::builder()
            .icon_name("accessories-text-editor-symbolic")
            .title("No Page Open")
            .description("Pick a page in the sidebar, or create a notebook for a course.")
            .child(&create_first)
            .build();
        self.stack.add_named(&empty, Some("empty"));
        self.stack.add_named(&self.canvas, Some("page"));

        self.stack.set_transition_type(gtk::StackTransitionType::Crossfade);
        self.stack.set_transition_duration(150);

        let fullscreen = gtk::Button::builder()
            .icon_name("view-fullscreen-symbolic")
            .tooltip_text("Fullscreen (F11)")
            .action_name("win.fullscreen")
            .build();

        // A bar of our own rather than an AdwHeaderBar (see TitleLayout).
        for b in [content_logo.upcast_ref::<gtk::Widget>(), toggle_sidebar.upcast_ref(), new_page.upcast_ref(), fullscreen.upcast_ref()] {
            b.add_css_class("flat");
        }
        let side = |children: &[&gtk::Widget]| {
            let b = gtk::Box::builder().css_classes(["rt-bar-side"]).build();
            for c in children {
                b.append(*c);
            }
            b
        };
        let start = side(&[
            start_controls.upcast_ref(),
            content_logo.upcast_ref(),
            toggle_sidebar.upcast_ref(),
            new_page.upcast_ref(),
            self.tools_scroller.upcast_ref(),
        ]);
        // Right side: history, page view, window.
        let end = side(&[
            self.toolbar.history.upcast_ref(),
            page_group.upcast_ref(),
            export.upcast_ref(),
            fullscreen.upcast_ref(),
            end_controls.upcast_ref(),
        ]);
        let bar = gtk::Box::builder().css_classes(["rt-bar"]).build();
        bar.set_layout_manager(Some(TitleLayout::new((8.0 * platform::ui_scale()).round() as i32)));
        bar.append(&start);
        bar.append(&self.title);
        bar.append(&end);
        end_controls.bind_property("empty", &end_controls, "visible").invert_boolean().sync_create().build();
        let content_header = gtk::WindowHandle::builder().child(&bar).build();

        let content = adw::ToolbarView::new();
        content.add_top_bar(&content_header);
        content.set_content(Some(&self.stack));

        let split = adw::OverlaySplitView::builder()
            .sidebar(&sidebar)
            .content(&content)
            .min_sidebar_width(220.0)
            .max_sidebar_width(320.0)
            // Only the user opens or closes the sidebar, not window resizes.
            .pin_sidebar(true)
            .show_sidebar(self.state.borrow().settings.show_sidebar)
            .build();
        split.bind_property("show-sidebar", &toggle_sidebar, "active").bidirectional().sync_create().build();
        split.bind_property("show-sidebar", &content_logo, "visible").invert_boolean().sync_create().build();
        // Start-side window buttons live in the sidebar's header while it shows.
        let sync_start = {
            let start = start_controls.clone();
            move |split: &adw::OverlaySplitView| start.set_visible(!start.is_empty() && !split.shows_sidebar())
        };
        sync_start(&split);
        split.connect_show_sidebar_notify(sync_start.clone());
        start_controls.connect_empty_notify(glib::clone!(
            #[weak]
            split,
            move |_| sync_start(&split)
        ));
        let _ = self.split.set(split.clone());


        let toggle = gio::SimpleAction::new("toggle-sidebar", None);
        toggle.connect_activate(glib::clone!(
            #[weak]
            split,
            move |_, _| split.set_show_sidebar(!split.shows_sidebar())
        ));
        self.window.add_action(&toggle);

        self.toasts.set_child(Some(&split));
        self.window.set_content(Some(&self.toasts));
        crate::ui::keep_icon_labels_aligned(&self.window);
        self.setup_fullscreen(&split, &content, &fullscreen);

        // Never narrower than the bar's buttons. The sidebar turns into an
        // overlay once a typical sidebar and the whole bar (all tools, full
        // title) no longer fit side by side, so the bar is never what gets
        // squeezed.
        let scale = platform::ui_scale();
        let (bar_min, bar_nat, _, _) = content_header.measure(gtk::Orientation::Horizontal, -1);
        self.window.set_width_request(bar_min.max(360));
        let narrow = adw::Breakpoint::new(adw::BreakpointCondition::new_length(
            adw::BreakpointConditionLengthType::MaxWidth,
            (bar_nat as f64 + 220.0 * scale).max(600.0),
            adw::LengthUnit::Px,
        ));
        narrow.add_setter(&split, "collapsed", Some(&true.to_value()));
        self.window.add_breakpoint(narrow);

        // This handler owns the strong reference that keeps `Window` alive
        // for as long as the GTK window exists; everything else holds weak refs.
        let this = self.clone();
        self.window.connect_close_request(move |_| {
            this.flush_pending_delete();
            this.canvas.flush();
            let mut st = this.state.borrow_mut();
            let mut expanded: Vec<String> = st.expanded.iter().cloned().collect();
            expanded.sort();
            st.settings.expanded = expanded;
            if let Some(split) = this.split.get() {
                st.settings.show_sidebar = split.shows_sidebar();
            }
            st.settings.save();
            glib::Propagation::Proceed
        });
    }

    // ---- actions -----------------------------------------------------------

    fn install_actions(self: &Rc<Self>, app: &adw::Application) {
        let add = |name: &str, param: bool, f: ActionFn| {
            let action = gio::SimpleAction::new(name, param.then_some(glib::VariantTy::STRING));
            let weak = self.weak();
            action.connect_activate(move |_, arg| {
                let this = upgrade!(weak);
                let target = arg.and_then(|v| v.str()).and_then(decode);
                f(&this, target);
            });
            self.window.add_action(&action);
        };
        add("new-notebook", false, Box::new(|w, _| w.new_notebook()));
        add("new-section", true, Box::new(|w, t| w.new_section(t)));
        add("new-page", true, Box::new(|w, t| w.new_page(t)));
        add("new-page-here", false, Box::new(|w, _| w.new_page(None)));
        add("rename", true, Box::new(|w, t| w.rename(t)));
        add("rename-page", false, Box::new(|w, _| w.rename(w.open_target())));
        add("delete", true, Box::new(|w, t| w.delete(t)));
        add("export", true, Box::new(|w, t| w.export(t)));
        add("export-page", false, Box::new(|w, _| w.export(w.open_target())));
        add("open-folder", false, Box::new(|w, _| w.open_folder()));
        add("change-folder", false, Box::new(|w, _| w.change_folder()));
        add("about", false, Box::new(|w, _| w.about()));
        add("preferences", false, Box::new(|w, _| w.preferences()));
        add("undo", false, Box::new(|w, _| w.canvas.undo()));
        add("redo", false, Box::new(|w, _| w.canvas.redo()));
        add("zoom-in", false, Box::new(|w, _| w.canvas.zoom_step(1.25)));
        add("zoom-out", false, Box::new(|w, _| w.canvas.zoom_step(1.0 / 1.25)));
        add("zoom-reset", false, Box::new(|w, _| w.canvas.zoom_centered(1.0)));
        if debug::enabled() {
            self.install_debug_actions();
        }

        let weak = self.weak();
        self.canvas.connect_history(move |can_undo, can_redo| {
            let this = upgrade!(weak);
            for (name, enabled) in [("undo", can_undo), ("redo", can_redo)] {
                if let Some(a) = this.window.lookup_action(name).and_downcast::<gio::SimpleAction>() {
                    a.set_enabled(enabled);
                }
            }
        });
        let weak = self.weak();
        self.canvas.connect_error(move |msg| {
            let this = upgrade!(weak);
            this.toast(&msg);
        });

        let current = self.state.borrow().settings.theme.as_str();
        let theme = gio::SimpleAction::new_stateful(
            "theme",
            Some(glib::VariantTy::STRING),
            &current.to_variant(),
        );
        let weak = self.weak();
        theme.connect_activate(move |action, arg| {
            let this = upgrade!(weak);
            let Some(theme) = arg.and_then(|v| v.str()).and_then(Theme::parse) else { return };
            action.set_state(&theme.as_str().to_variant());
            theme.apply();
            let mut st = this.state.borrow_mut();
            st.settings.theme = theme;
            st.settings.save();
        });
        self.window.add_action(&theme);

        app.set_accels_for_action("win.new-page-here", &["<Control>n"]);
        app.set_accels_for_action("win.new-notebook", &["<Control><Shift>n"]);
        app.set_accels_for_action("win.rename-page", &["F2"]);
        app.set_accels_for_action("win.toggle-sidebar", &["F9"]);
        app.set_accels_for_action("win.fullscreen", &["F11"]);
        app.set_accels_for_action("win.export-page", &["<Control>e"]);
        app.set_accels_for_action("win.undo", &["<Control>z"]);
        app.set_accels_for_action("win.redo", &["<Control><Shift>z", "<Control>y"]);
        app.set_accels_for_action("win.preferences", &["<Control>comma"]);
        app.set_accels_for_action("win.zoom-in", &["<Control>plus", "<Control>equal", "<Control>KP_Add"]);
        app.set_accels_for_action("win.zoom-out", &["<Control>minus", "<Control>KP_Subtract"]);
        app.set_accels_for_action("win.zoom-reset", &["<Control>0", "<Control>KP_0"]);
    }

    fn open_target(&self) -> Option<Target> {
        let st = self.state.borrow();
        let key = st.open_page.as_ref()?;
        Self::resolve_page(&st.lib, key).map(|(n, s, p)| Target::Page(n, s, p))
    }

    // ---- sidebar -------------------------------------------------------------

    fn resolve_page(lib: &Library, [nb, sec, file]: &PageKey) -> Option<(usize, usize, usize)> {
        let n = lib.notebooks.iter().position(|x| &x.dir == nb)?;
        let s = lib.notebooks[n].sections.iter().position(|x| &x.dir == sec)?;
        let p = lib.notebooks[n].sections[s].pages.iter().position(|x| &x.file == file)?;
        Some((n, s, p))
    }

    fn page_key(lib: &Library, n: usize, s: usize, p: usize) -> PageKey {
        let nb = &lib.notebooks[n];
        let sec = &nb.sections[s];
        [nb.dir.clone(), sec.dir.clone(), sec.pages[p].file.clone()]
    }

    fn rebuild_sidebar(self: &Rc<Self>) {
        self.list.remove_all();
        let mut rows = Vec::new();
        let mut selected_row = None;
        {
            let st = self.state.borrow();
            let open = st.open_page.as_ref().and_then(|k| Self::resolve_page(&st.lib, k));
            for (n, nb) in st.lib.notebooks.iter().enumerate() {
                let nb_open = st.expanded.contains(&nb.dir);
                let row = self.make_row(0, Some(nb_open), Some(&nb.color), &nb.name);
                row.add_css_class("notebook-row");
                self.list.append(&row);
                rows.push(Target::Notebook(n));
                if !nb_open {
                    continue;
                }
                for (s, sec) in nb.sections.iter().enumerate() {
                    let sec_open = st.expanded.contains(&format!("{}/{}", nb.dir, sec.dir));
                    let row = self.make_row(1, Some(sec_open), Some(&nb.color), &sec.name);
                    self.list.append(&row);
                    rows.push(Target::Section(n, s));
                    if !sec_open {
                        continue;
                    }
                    for (p, page) in sec.pages.iter().enumerate() {
                        let row = self.make_row(2, None, None, &page.title);
                        row.set_selectable(true);
                        self.list.append(&row);
                        if open == Some((n, s, p)) {
                            selected_row = Some(row.clone());
                        }
                        rows.push(Target::Page(n, s, p));
                    }
                }
            }
        }
        self.state.borrow_mut().rows = rows;
        self.fit_sidebar_width();
        match selected_row {
            Some(row) => self.list.select_row(Some(&row)),
            None => self.list.unselect_all(),
        }
    }

    /// Size the sidebar to its longest entry (all of them, collapsed or
    /// not, so it doesn't jump when expanding) plus a little room.
    fn fit_sidebar_width(&self) {
        let Some(split) = self.split.get() else { return };
        let st = self.state.borrow();
        let text_width = |text: &str, bold: bool| {
            let layout = self.list.create_pango_layout(Some(text));
            if bold {
                let attrs = gtk::pango::AttrList::new();
                attrs.insert(gtk::pango::AttrInt::new_weight(gtk::pango::Weight::Bold));
                layout.set_attributes(Some(&attrs));
            }
            layout.pixel_size().0 as f64
        };
        // Row margins (10 + 10), the list's own row inset (~12), the rows'
        // side margins (6 + 6, style.css) and a little breathing room.
        const CHROME: f64 = 10.0 + 10.0 + 12.0 + 12.0 + 6.0;
        let s = platform::ui_scale();
        let mut widest: f64 = 0.0;
        for nb in &st.lib.notebooks {
            widest = widest.max((10.0 + 8.0) * s + text_width(&nb.name, true));
            for sec in &nb.sections {
                // Indent, chevron, tab and their spacing.
                widest = widest.max((16.0 + 16.0 + 8.0 + 4.0 + 8.0) * s + text_width(&sec.name, false));
                for page in &sec.pages {
                    // Indent, spacer, page icon and their spacing.
                    widest = widest.max((32.0 + 16.0 + 8.0 + 16.0 + 8.0) * s + text_width(&page.title, false));
                }
            }
        }
        // At least as wide as the sidebar's own header needs.
        let width = (widest + CHROME * s).clamp(160.0 * s, 400.0 * s);
        split.set_min_sidebar_width(width);
        split.set_max_sidebar_width(width);
    }

    fn make_row(
        self: &Rc<Self>,
        depth: i32,
        expanded: Option<bool>,
        color: Option<&str>,
        label: &str,
    ) -> gtk::ListBoxRow {
        let hbox = gtk::Box::builder().spacing((8.0 * platform::ui_scale()).round() as i32).build();
        match expanded {
            // Notebooks show only their color dot (arrows there are clutter).
            Some(_) if depth == 0 => {}
            Some(open) => {
                let icon = if open { "pan-down-symbolic" } else { "pan-end-symbolic" };
                let chevron = gtk::Image::from_icon_name(icon);
                chevron.add_css_class("rt-chevron");
                hbox.append(&chevron);
            }
            // Align pages with their section's label.
            None => hbox.append(&gtk::Box::builder().width_request((16.0 * platform::ui_scale()).round() as i32).build()),
        }
        match (depth, color) {
            // Notebook: its color dot.
            (0, Some(color)) => {
                let dot = gtk::Box::builder()
                    .valign(gtk::Align::Center)
                    .css_classes(["nb-dot", &format!("nb-{color}")])
                    .build();
                hbox.append(&dot);
            }
            // Section: a OneNote-style tab in the notebook's color.
            (1, Some(color)) => {
                let tab = gtk::Box::builder()
                    .height_request(platform::px(14.0))
                    .valign(gtk::Align::Center)
                    .css_classes(["rt-section-tab", &format!("nb-{color}")])
                    .build();
                hbox.append(&tab);
            }
            _ => {}
        }
        // Pages: a small page icon (Notion).
        if depth == 2 {
            let icon = gtk::Image::from_icon_name("x-office-document-symbolic");
            icon.add_css_class("rt-page-icon");
            hbox.append(&icon);
        }
        let text = gtk::Label::builder()
            .label(label)
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .tooltip_text(label)
            .build();
        if depth == 1 {
            text.add_css_class("rt-section-label");
        }
        hbox.append(&text);
        let px = |v: f64| (v * platform::ui_scale()).round() as i32;
        hbox.set_margin_top(px(7.0));
        hbox.set_margin_bottom(px(7.0));
        hbox.set_margin_end(px(10.0));
        hbox.set_margin_start(px(10.0 + depth as f64 * 16.0));
        let revealer = gtk::Revealer::builder()
            .child(&hbox)
            .reveal_child(true)
            .transition_type(gtk::RevealerTransitionType::SlideDown)
            .transition_duration(SIDEBAR_ANIMATION_MS)
            .build();
        let row = gtk::ListBoxRow::builder()
            .child(&revealer)
            .selectable(false)
            .css_classes(["rt-row"])
            .build();
        self.attach_dnd(&row);
        row
    }

    fn row_revealer(row: &gtk::ListBoxRow) -> Option<gtk::Revealer> {
        row.child().and_downcast::<gtk::Revealer>()
    }

    /// Whether `t` is inside notebook `n` (or section `(n, s)`).
    fn is_descendant(t: Target, n: usize, s: Option<usize>) -> bool {
        match (t, s) {
            (Target::Section(tn, _) | Target::Page(tn, _, _), None) => tn == n,
            (Target::Page(tn, ts, _), Some(s)) => (tn, ts) == (n, s),
            _ => false,
        }
    }

    fn install_sidebar_input(self: &Rc<Self>) {
        let weak = self.weak();
        self.list.connect_row_activated(move |_, row| {
            let this = upgrade!(weak);
            let target = this.state.borrow().rows.get(row.index() as usize).copied();
            match target {
                Some(Target::Notebook(n)) => this.toggle_expanded(n, None),
                Some(Target::Section(n, s)) => this.toggle_expanded(n, Some(s)),
                Some(Target::Page(n, s, p)) => this.open_page(n, s, p),
                None => {}
            }
        });

        // Context menu: right click (pen barrel button) or long press.
        let click = gtk::GestureClick::builder().button(gdk::BUTTON_SECONDARY).build();
        let weak = self.weak();
        click.connect_pressed(move |g, _, x, y| {
            let this = upgrade!(weak);
            g.set_state(gtk::EventSequenceState::Claimed);
            this.show_context_menu(x, y);
        });
        self.list.add_controller(click);

        // Double click renames. Runs in the capture phase on the list (rows
        // are rebuilt by the first click) and claims the second press so it
        // doesn't also activate the row.
        let double = gtk::GestureClick::builder()
            .button(gdk::BUTTON_PRIMARY)
            .propagation_phase(gtk::PropagationPhase::Capture)
            .build();
        let weak = self.weak();
        double.connect_pressed(move |g, n_press, _, y| {
            let this = upgrade!(weak);
            if n_press != 2 || this.state.borrow().editing {
                return;
            }
            let Some(row) = this.list.row_at_y(y as i32) else { return };
            let Some(target) = this.row_target(&row) else { return };
            g.set_state(gtk::EventSequenceState::Claimed);
            this.start_inline_rename(target, None);
        });
        self.list.add_controller(double);

        let long_press = gtk::GestureLongPress::new();
        let weak = self.weak();
        long_press.connect_pressed(move |g, x, y| {
            let this = upgrade!(weak);
            g.set_state(gtk::EventSequenceState::Claimed);
            this.show_context_menu(x, y);
        });
        self.list.add_controller(long_press);
    }

    fn show_context_menu(&self, x: f64, y: f64) {
        let Some(row) = self.list.row_at_y(y as i32) else { return };
        let Some(target) = self.state.borrow().rows.get(row.index() as usize).copied() else {
            return;
        };
        let t = encode(target);
        let menu = gio::Menu::new();
        let item = |label: &str, action: &str, arg: &str| {
            gio::MenuItem::new(Some(label), Some(&format!("win.{action}::{arg}")))
        };
        match target {
            Target::Notebook(_) => menu.append_item(&item("New Section", "new-section", &t)),
            Target::Section(..) | Target::Page(..) => {
                menu.append_item(&item("New Page", "new-page", &t))
            }
        }
        let edit = gio::Menu::new();
        edit.append_item(&item("Rename…", "rename", &t));
        edit.append_item(&item("Export…", "export", &t));
        edit.append_item(&item("Move to Trash", "delete", &t));
        menu.append_section(None, &edit);

        self.menu.set_menu_model(Some(&menu));
        self.menu.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        self.menu.popup();
    }

    /// Expand or collapse with a slide: children slide in after expanding,
    /// or slide out before collapsing.
    fn toggle_expanded(self: &Rc<Self>, n: usize, s: Option<usize>) {
        let (key, expanding) = {
            let mut st = self.state.borrow_mut();
            let nb = &st.lib.notebooks[n];
            let key = match s {
                Some(s) => format!("{}/{}", nb.dir, nb.sections[s].dir),
                None => nb.dir.clone(),
            };
            if let Some(s) = s {
                st.current_section = Some([nb.dir.clone(), nb.sections[s].dir.clone()]);
            }
            let expanding = !st.expanded.contains(&key);
            (key, expanding)
        };
        let children: Vec<gtk::Revealer> = {
            let rows = self.state.borrow().rows.clone();
            if expanding {
                self.state.borrow_mut().expanded.insert(key.clone());
                self.rebuild_sidebar();
                let rows = self.state.borrow().rows.clone();
                rows.iter()
                    .enumerate()
                    .filter(|(_, t)| Self::is_descendant(**t, n, s))
                    .filter_map(|(i, _)| self.list.row_at_index(i as i32).as_ref().and_then(Self::row_revealer))
                    .collect()
            } else {
                rows.iter()
                    .enumerate()
                    .filter(|(_, t)| Self::is_descendant(**t, n, s))
                    .filter_map(|(i, _)| self.list.row_at_index(i as i32).as_ref().and_then(Self::row_revealer))
                    .collect()
            }
        };
        if expanding {
            for r in &children {
                r.set_transition_duration(0);
                r.set_reveal_child(false);
                r.set_transition_duration(SIDEBAR_ANIMATION_MS);
            }
            glib::idle_add_local_once(move || children.iter().for_each(|r| r.set_reveal_child(true)));
        } else {
            children.iter().for_each(|r| r.set_reveal_child(false));
            let weak = self.weak();
            glib::timeout_add_local_once(Duration::from_millis(SIDEBAR_ANIMATION_MS as u64), move || {
                let this = upgrade!(weak);
                this.state.borrow_mut().expanded.remove(&key);
                this.rebuild_sidebar();
            });
            // Update the arrow right away.
            self.set_row_arrow(n, s, false);
        }
    }

    fn set_row_arrow(&self, n: usize, s: Option<usize>, open: bool) {
        let target = match s {
            Some(s) => Target::Section(n, s),
            None => Target::Notebook(n),
        };
        let Some(i) = self.state.borrow().rows.iter().position(|t| *t == target) else { return };
        let icon = self
            .list
            .row_at_index(i as i32)
            .as_ref()
            .and_then(Self::row_revealer)
            .and_then(|r| r.child())
            .and_then(|hbox| hbox.first_child())
            .and_downcast::<gtk::Image>();
        if let Some(icon) = icon {
            icon.set_icon_name(Some(if open { "pan-down-symbolic" } else { "pan-end-symbolic" }));
        }
    }

    fn expand(&self, n: usize, s: Option<usize>) {
        let mut st = self.state.borrow_mut();
        let nb_dir = st.lib.notebooks[n].dir.clone();
        st.expanded.insert(nb_dir.clone());
        if let Some(s) = s {
            let sec_dir = st.lib.notebooks[n].sections[s].dir.clone();
            st.expanded.insert(format!("{nb_dir}/{sec_dir}"));
        }
    }

    // ---- drag & drop ---------------------------------------------------------

    fn row_target(&self, row: &gtk::ListBoxRow) -> Option<Target> {
        self.state.borrow().rows.get(usize::try_from(row.index()).ok()?).copied()
    }

    fn drop_pos(src: Target, dst: Target, y: f64, height: f64) -> Option<DropPos> {
        let half = if y < height / 2.0 { DropPos::Before } else { DropPos::After };
        match (src, dst) {
            (Target::Notebook(a), Target::Notebook(b)) if a != b => Some(half),
            (Target::Section(..), Target::Notebook(_)) => Some(DropPos::Into),
            (Target::Section(..), Target::Section(..)) if src != dst => Some(half),
            (Target::Page(..), Target::Section(..)) => Some(DropPos::Into),
            (Target::Page(..), Target::Page(..)) if src != dst => Some(half),
            _ => None,
        }
    }

    fn clear_drop_marks(row: &gtk::ListBoxRow) {
        for class in ["drop-before", "drop-after", "drop-into"] {
            row.remove_css_class(class);
        }
    }

    fn attach_dnd(self: &Rc<Self>, row: &gtk::ListBoxRow) {
        let source = gtk::DragSource::builder().actions(gdk::DragAction::MOVE).build();
        let weak = self.weak();
        source.connect_prepare(move |src, _, _| {
            let this = upgrade!(weak, None);
            let row = src.widget()?.downcast::<gtk::ListBoxRow>().ok()?;
            let target = this.row_target(&row)?;
            src.set_icon(Some(&gtk::WidgetPaintable::new(Some(&row))), 0, 0);
            Some(gdk::ContentProvider::for_value(&encode(target).to_value()))
        });
        row.add_controller(source);

        let drop = gtk::DropTarget::new(glib::Type::STRING, gdk::DragAction::MOVE);
        // Load the dragged value during hover, so motion can decide if the
        // drop is allowed; without this `value()` is None until the drop.
        drop.set_preload(true);
        let weak = self.weak();
        drop.connect_motion(move |dt, _, y| {
            let this = upgrade!(weak, gdk::DragAction::empty());
            let Some(row) = dt.widget().and_downcast::<gtk::ListBoxRow>() else {
                return gdk::DragAction::empty();
            };
            Self::clear_drop_marks(&row);
            let src = dt.value().and_then(|v| v.get::<String>().ok()).and_then(|s| decode(&s));
            let dst = this.row_target(&row);
            let pos = src
                .zip(dst)
                .and_then(|(s, d)| Self::drop_pos(s, d, y, row.height() as f64));
            match pos {
                Some(DropPos::Before) => row.add_css_class("drop-before"),
                Some(DropPos::After) => row.add_css_class("drop-after"),
                Some(DropPos::Into) => row.add_css_class("drop-into"),
                None => return gdk::DragAction::empty(),
            }
            gdk::DragAction::MOVE
        });
        drop.connect_leave(|dt| {
            if let Some(row) = dt.widget().and_downcast::<gtk::ListBoxRow>() {
                Self::clear_drop_marks(&row);
            }
        });
        let weak = self.weak();
        drop.connect_drop(move |dt, value, _, y| {
            let this = upgrade!(weak, false);
            let Some(row) = dt.widget().and_downcast::<gtk::ListBoxRow>() else { return false };
            Self::clear_drop_marks(&row);
            let src = value.get::<String>().ok().and_then(|s| decode(&s));
            let (Some(src), Some(dst)) = (src, this.row_target(&row)) else { return false };
            let Some(pos) = Self::drop_pos(src, dst, y, row.height() as f64) else { return false };
            // Rebuilding the list destroys this row, so finish the drop first.
            glib::idle_add_local_once(move || this.perform_move(src, dst, pos));
            true
        });
        row.add_controller(drop);
    }

    fn perform_move(self: &Rc<Self>, src: Target, dst: Target, pos: DropPos) {
        self.flush_pending_delete();
        self.canvas.flush();
        let result = {
            let mut st = self.state.borrow_mut();
            let st = &mut *st;
            let open = st.open_page.as_ref().and_then(|k| Self::resolve_page(&st.lib, k));
            let lib = &mut st.lib;
            let after = usize::from(pos == DropPos::After);
            // Where the item landed: (notebook, section) to reveal, and the
            // new position of the open page if the move renamed its path.
            let result = match (src, dst) {
                (Target::Notebook(a), Target::Notebook(b)) => {
                    lib.move_notebook(a, b + after).map(|_| (None, None))
                }
                (Target::Section(n, s), Target::Notebook(_) | Target::Section(..)) => {
                    let (to_nb, to) = match dst {
                        Target::Section(bn, bs) => (bn, bs + after),
                        Target::Notebook(b) => (b, lib.notebooks[b].sections.len()),
                        Target::Page(..) => unreachable!(),
                    };
                    lib.move_section((n, s), to_nb, to).map(|(rn, rs)| {
                        let open = open.filter(|o| (o.0, o.1) == (n, s)).map(|o| (rn, rs, o.2));
                        (Some((rn, rs)), open)
                    })
                }
                (Target::Page(n, s, p), Target::Section(..) | Target::Page(..)) => {
                    let (bn, bs, to) = match dst {
                        Target::Page(bn, bs, bp) => (bn, bs, bp + after),
                        Target::Section(bn, bs) => (bn, bs, lib.notebooks[bn].sections[bs].pages.len()),
                        Target::Notebook(_) => unreachable!(),
                    };
                    lib.move_page((n, s, p), (bn, bs), to).map(|r| {
                        let open = (open == Some((n, s, p))).then_some(r);
                        (Some((r.0, r.1)), open)
                    })
                }
                _ => Ok((None, None)),
            };
            if let Ok((_, Some((n, s, p)))) = result {
                let key = Self::page_key(&st.lib, n, s, p);
                st.current_section = Some([key[0].clone(), key[1].clone()]);
                st.settings.last_page = Some(key.clone());
                st.open_page = Some(key);
            }
            result
        };
        match result {
            Ok((Some((n, s)), _)) => self.expand(n, Some(s)),
            Ok(_) => {}
            Err(e) => self.toast(&format!("Couldn't move: {e}")),
        }
        self.retarget_open_page();
        self.rebuild_sidebar();
        self.update_content();
    }

    // ---- content -------------------------------------------------------------

    fn open_page(self: &Rc<Self>, n: usize, s: usize, p: usize) {
        {
            let mut st = self.state.borrow_mut();
            let key = Self::page_key(&st.lib, n, s, p);
            st.current_section = Some([key[0].clone(), key[1].clone()]);
            st.settings.last_page = Some(key.clone());
            st.open_page = Some(key);
        }
        self.update_content();
    }

    fn restore_last_page(&self) {
        let mut st = self.state.borrow_mut();
        let Some(key) = st.settings.last_page.clone() else { return };
        if let Some((n, s, _)) = Self::resolve_page(&st.lib, &key) {
            let nb = &st.lib.notebooks[n];
            let (nb_dir, sec_dir) = (nb.dir.clone(), nb.sections[s].dir.clone());
            st.expanded.insert(nb_dir.clone());
            st.expanded.insert(format!("{nb_dir}/{sec_dir}"));
            st.current_section = Some([nb_dir, sec_dir]);
            st.open_page = Some(key);
        }
    }

    fn update_content(&self) {
        let st = self.state.borrow();
        let open = st.open_page.as_ref().and_then(|k| Self::resolve_page(&st.lib, k));
        match open {
            Some((n, s, p)) => {
                let nb = &st.lib.notebooks[n];
                let sec = &nb.sections[s];
                let title = &sec.pages[p].title;
                self.title.set_title(title);
                self.title.set_subtitle(&format!("{} › {}", nb.name, sec.name));
                self.window.set_title(Some(&format!("{title} – Ratchet")));
                let path = st.lib.page_path(n, s, p);
                self.stack.set_visible_child_name("page");
                drop(st);
                self.canvas.load(Some(&path));
                self.set_page_controls_visible(true);
            }
            None => {
                self.title.set_title("Ratchet");
                self.title.set_subtitle("");
                self.window.set_title(Some("Ratchet"));
                self.stack.set_visible_child_name("empty");
                drop(st);
                self.canvas.load(None);
                self.set_page_controls_visible(false);
            }
        }
    }

    // ---- fullscreen -----------------------------------------------------------------

    /// Fullscreen shows only the canvas; the header slides down when the
    /// pointer (or hovering pen) reaches the top edge and hides again after
    /// it leaves for a moment.
    fn setup_fullscreen(self: &Rc<Self>, split: &adw::OverlaySplitView, content: &adw::ToolbarView, button: &gtk::Button) {
        let action = gio::SimpleAction::new("fullscreen", None);
        let window = self.window.clone();
        action.connect_activate(move |_, _| {
            if window.is_fullscreen() {
                window.unfullscreen();
            } else {
                window.fullscreen();
            }
        });
        self.window.add_action(&action);

        // Remembered sidebar state: (shown, collapsed).
        let saved = Rc::new(std::cell::Cell::new((true, false)));
        let (split, content, button) = (split.clone(), content.clone(), button.clone());
        {
            let content = content.clone();
            self.window.connect_fullscreened_notify(move |w| {
                if w.is_fullscreen() {
                    saved.set((split.shows_sidebar(), split.is_collapsed()));
                    // Overlay the sidebar and the header instead of laying
                    // them out beside the canvas, so the canvas keeps its
                    // size while they slide away (no flicker).
                    split.set_collapsed(true);
                    split.set_show_sidebar(false);
                    content.set_extend_content_to_top_edge(true);
                    content.set_reveal_top_bars(false);
                    button.set_icon_name("view-restore-symbolic");
                    button.set_tooltip_text(Some("Leave Fullscreen (F11)"));
                } else {
                    let (shown, collapsed) = saved.get();
                    content.set_reveal_top_bars(true);
                    content.set_extend_content_to_top_edge(false);
                    split.set_collapsed(collapsed);
                    split.set_show_sidebar(shown);
                    button.set_icon_name("view-fullscreen-symbolic");
                    button.set_tooltip_text(Some("Fullscreen (F11)"));
                }
            });
        }

        let hide_timer: Rc<RefCell<Option<glib::SourceId>>> = Rc::default();
        let last_y = Rc::new(std::cell::Cell::new(0.0f64));
        let motion = gtk::EventControllerMotion::builder().propagation_phase(gtk::PropagationPhase::Capture).build();
        let window = self.window.clone();
        motion.connect_motion(move |_, _, y| {
            last_y.set(y);
            if !window.is_fullscreen() {
                return;
            }
            let header_height = content.top_bar_height() as f64;
            if y <= 4.0 {
                if let Some(id) = hide_timer.take() {
                    id.remove();
                }
                content.set_reveal_top_bars(true);
            } else if content.reveals_top_bars() && y > header_height + 24.0 && hide_timer.borrow().is_none() {
                let (content, window, last_y, timer) =
                    (content.clone(), window.clone(), last_y.clone(), hide_timer.clone());
                let id = glib::timeout_add_local_once(Duration::from_millis(900), move || {
                    timer.take();
                    // Only hide if the pointer is still away from the header,
                    // and no menu from it is open: menus hang below the
                    // header, so pointing into one looked like leaving it,
                    // and hiding took the menu along (flicker).
                    if window.is_fullscreen()
                        && last_y.get() > content.top_bar_height() as f64 + 24.0
                        && !crate::ui::has_open_popover(&content)
                    {
                        content.set_reveal_top_bars(false);
                    }
                });
                *hide_timer.borrow_mut() = Some(id);
            } else if y <= header_height + 24.0
                && let Some(id) = hide_timer.take() {
                    id.remove();
                }
        });
        self.window.add_controller(motion);

        // Esc leaves fullscreen when nothing else used it.
        let keys = gtk::EventControllerKey::new();
        let window = self.window.clone();
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gdk::Key::Escape && window.is_fullscreen() {
                window.unfullscreen();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        self.window.add_controller(keys);
    }

    /// Run what was picked in the radial menu.
    fn radial_pick(self: &Rc<Self>, item: RadialItem) {
        match item {
            RadialItem::Slot(i) => self.toolbar.select_tool(ActiveTool::Slot(i)),
            RadialItem::Eraser => self.toolbar.select_tool(ActiveTool::Eraser),
            RadialItem::Select => self.toolbar.select_tool(ActiveTool::Select),
            RadialItem::Text => self.toolbar.select_tool(ActiveTool::Text),
            RadialItem::Hand => self.toolbar.select_tool(ActiveTool::Hand),
            RadialItem::Undo => self.canvas.undo(),
            RadialItem::Redo => self.canvas.redo(),
            RadialItem::NewPage => self.new_page(None),
            RadialItem::Paste => self.canvas.paste(true),
            RadialItem::ZoomReset => self.canvas.zoom_centered(1.0),
            RadialItem::Fullscreen => {
                let _ = WidgetExt::activate_action(&self.window, "win.fullscreen", None);
            }
            RadialItem::None => {}
        }
    }

    // ---- export -------------------------------------------------------------------

    /// Export a page, a whole section, or a whole notebook.
    fn export(self: &Rc<Self>, target: Option<Target>) {
        let Some(target) = target else { return };
        // Make sure the open page's latest ink is on disk.
        self.canvas.flush();
        let (title, entries) = {
            let st = self.state.borrow();
            let lib = &st.lib;
            let load = |n: usize, s: usize, p: usize, section: Option<String>| {
                let title = lib.notebooks[n].sections[s].pages[p].title.clone();
                PageFile::load(&lib.page_path(n, s, p)).ok().map(|page| export::Entry { page, section, title })
            };
            match target {
                Target::Page(n, s, p) => {
                    (lib.notebooks[n].sections[s].pages[p].title.clone(), load(n, s, p, None).into_iter().collect())
                }
                Target::Section(n, s) => {
                    let sec = &lib.notebooks[n].sections[s];
                    let entries: Vec<_> = (0..sec.pages.len()).filter_map(|p| load(n, s, p, None)).collect();
                    (sec.name.clone(), entries)
                }
                Target::Notebook(n) => {
                    let nb = &lib.notebooks[n];
                    let entries: Vec<_> = nb
                        .sections
                        .iter()
                        .enumerate()
                        .flat_map(|(s, sec)| (0..sec.pages.len()).map(move |p| (s, p, sec.name.clone())))
                        .filter_map(|(s, p, name)| load(n, s, p, Some(name)))
                        .collect();
                    (nb.name.clone(), entries)
                }
            }
        };
        if entries.is_empty() {
            self.toast("Nothing to export: no pages");
            return;
        }
        let current = self.state.borrow().settings.export.clone();
        let weak = self.weak();
        let file_name = crate::library::sanitize(&title);
        let pages = match entries.len() {
            1 => "1 page".to_string(),
            n => format!("{n} pages"),
        };
        let detail = match target {
            Target::Page(..) => "Page".to_string(),
            Target::Section(..) => format!("Section · {pages}"),
            Target::Notebook(..) => format!("Notebook · {pages}"),
        };
        let name = title.clone();
        let subject = export_dialog::Subject {
            name: &name,
            detail: &detail,
            has_infinite: entries.iter().any(|e| e.page.layout.size == crate::library::PageSize::Infinite),
        };
        export_dialog::show(&self.window, subject, &file_name, current, move |opts, path| {
            let this = upgrade!(weak);
            {
                let mut st = this.state.borrow_mut();
                st.settings.export = opts.clone();
                st.settings.save();
            }
            let doc = export::build(&title, &entries, Some(&this.window.pango_context()), &opts);
            let result = match opts.format {
                ExportFormat::Pdf => export::write_pdf(&doc, &path).map(|_| vec![path.clone()]),
                ExportFormat::Png => export::write_png(&doc, &path),
            };
            match result {
                Ok(files) => {
                    let what = if files.len() > 1 { format!("{} files", files.len()) } else { "1 file".into() };
                    this.toast(&format!("Exported {what} to {}", path.parent().map_or(String::new(), |p| p.display().to_string())));
                    if opts.open_after
                        && let Some(first) = files.first()
                    {
                        gtk::FileLauncher::new(Some(&gio::File::for_path(first))).launch(
                            Some(&this.window),
                            gio::Cancellable::NONE,
                            |_| {},
                        );
                    }
                }
                Err(e) => this.toast(&format!("Export failed: {e}")),
            }
        });
    }

    // ---- page controls -----------------------------------------------------------

    /// Orientation new sheet pages follow: the monitor's in fullscreen,
    /// otherwise the window's shape.
    fn screen_is_landscape(&self) -> bool {
        if self.window.is_fullscreen() {
            let monitor = self
                .window
                .surface()
                .and_then(|surface| gdk::Display::default()?.monitor_at_surface(&surface));
            if let Some(m) = monitor {
                let g = m.geometry();
                return g.width() >= g.height();
            }
        }
        self.window.width() >= self.window.height()
    }

    fn set_page_controls_visible(&self, visible: bool) {
        self.tools_scroller.set_visible(visible);
        self.toolbar.history.set_visible(visible);
        self.zoom_button.set_visible(visible);
        self.page_button.set_visible(visible);
    }

    fn open_section(&self) -> Option<(usize, usize)> {
        let st = self.state.borrow();
        let key = st.open_page.as_ref()?;
        Self::resolve_page(&st.lib, key).map(|(n, s, _)| (n, s))
    }

    fn page_popover(self: &Rc<Self>) -> gtk::Popover {
        let section = self.open_section();
        let section_name = {
            let st = self.state.borrow();
            section.map_or(String::new(), |(n, s)| st.lib.notebooks[n].sections[s].name.clone())
        };
        let weak = self.weak();
        let weak_unit = self.weak();
        let canvas = self.canvas.clone();
        let unit = self.state.borrow().settings.custom_unit;
        let label = section_name.clone();
        page_menu::build(
            self.canvas.layout(),
            &label,
            unit,
            page_menu::Callbacks {
                changed: Box::new(move |layout| canvas.set_layout(layout)),
                make_default: Box::new(move |layout| {
                    let this = upgrade!(weak);
                    let Some((n, s)) = this.open_section() else { return };
                    let result = this.state.borrow_mut().lib.set_section_layout(n, s, layout);
                    match result {
                        Ok(()) => this.toast(&format!("New pages in “{section_name}” will use this layout")),
                        Err(e) => this.toast(&format!("Couldn't save section default: {e}")),
                    }
                }),
                unit_changed: Box::new(move |unit| {
                    let this = upgrade!(weak_unit);
                    let mut st = this.state.borrow_mut();
                    st.settings.custom_unit = unit;
                    st.settings.save();
                }),
            },
        )
    }

    // ---- debug (RATCHET_DEBUG=1) ---------------------------------------------------

    fn install_debug_actions(self: &Rc<Self>) {
        type Setter = fn(&mut DebugFlags, bool);
        let toggles: [(&str, Setter); 4] = [
            ("debug-input", |f, v| f.input = v),
            ("debug-points", |f, v| f.points = v),
            ("debug-perf", |f, v| f.perf = v),
            ("debug-log", |f, v| f.log = v),
        ];
        for (name, set) in toggles {
            let action = gio::SimpleAction::new_stateful(name, None, &false.to_variant());
            let weak = self.weak();
            action.connect_activate(move |a, _| {
                let this = upgrade!(weak);
                let on = !a.state().and_then(|v| v.get::<bool>()).unwrap_or(false);
                a.set_state(&on.to_variant());
                let mut flags = this.debug_flags.get();
                set(&mut flags, on);
                this.debug_flags.set(flags);
                this.canvas.set_debug(flags);
                if name == "debug-log" {
                    let log = this.log_window();
                    if on {
                        log.window.present();
                        log.append("event log started");
                    } else {
                        log.window.set_visible(false);
                    }
                }
            });
            self.window.add_action(&action);
        }
        let weak = self.weak();
        self.canvas.connect_log(move |line| {
            let this = upgrade!(weak);
            this.log_window().append(&line);
        });
    }

    fn log_window(self: &Rc<Self>) -> &LogWindow {
        self.log_window.get_or_init(|| {
            let log = LogWindow::new(&self.window);
            // Closing the window turns the log off.
            let weak = self.weak();
            log.window.connect_close_request(move |_| {
                let this = upgrade!(weak, glib::Propagation::Proceed);
                if let Some(a) = this.window.lookup_action("debug-log")
                    && a.state().and_then(|v| v.get::<bool>()) == Some(true) {
                        a.activate(None);
                    }
                glib::Propagation::Proceed
            });
            log
        })
    }

    // ---- tools -------------------------------------------------------------------

    fn tools_changed(&self, t: &ToolState) {
        {
            let mut st = self.state.borrow_mut();
            // A removed slot shifts the ones after it; keep the radial
            // menu pointing at the same pens.
            let old = &st.settings.pens;
            if t.pens.len() + 1 == old.len() {
                let removed = (0..old.len()).find(|&i| t.pens.get(i) != old.get(i)).unwrap_or(old.len() - 1);
                for item in &mut st.settings.radial_items {
                    *item = match *item {
                        RadialItem::Slot(i) if i == removed => RadialItem::None,
                        RadialItem::Slot(i) if i > removed => RadialItem::Slot(i - 1),
                        other => other,
                    };
                }
            }
            st.settings.pens = t.pens.clone();
            st.settings.active_tool = t.active;
            st.settings.eraser_mode = t.eraser_mode;
            st.settings.text_size = t.text_size;
            st.settings.text_font = t.text_font.clone();
            st.settings.show_text_tool = t.show_text_tool;
            st.settings.save();
        }
        self.apply_tools();
    }

    /// Push the tool settings to the canvas.
    fn apply_tools(&self) {
        let st = self.state.borrow();
        let s = &st.settings;
        let tool = match s.active_tool {
            ActiveTool::Slot(_) => CanvasTool::Draw,
            ActiveTool::Eraser => CanvasTool::Erase,
            ActiveTool::Select => CanvasTool::Select,
            ActiveTool::Text => CanvasTool::Text,
            ActiveTool::Hand => CanvasTool::Hand,
        };
        let radial: Vec<RadialEntry> = s
            .radial_items
            .iter()
            .map(|item| {
                let look = match *item {
                    RadialItem::Slot(i) => match s.pens.get(i) {
                        Some(p) if p.tool == Tool::Highlighter => EntryLook::Highlighter(p.color),
                        Some(p) => EntryLook::Pen(p.color),
                        None => EntryLook::Icon("action-unavailable-symbolic"),
                    },
                    RadialItem::Eraser => EntryLook::Eraser,
                    RadialItem::Select => EntryLook::Select,
                    RadialItem::Hand => EntryLook::Hand,
                    other => EntryLook::Icon(other.icon_name().unwrap_or("")),
                };
                // A slot that no longer exists acts as empty.
                let item = match *item {
                    RadialItem::Slot(i) if i >= s.pens.len() => RadialItem::None,
                    other => other,
                };
                RadialEntry { item, look }
            })
            .collect();
        // The pen to draw with: the selected slot, or the first one when
        // another tool is active (used by inputs mapped to Draw).
        let pen = match s.active_tool {
            ActiveTool::Slot(i) => s.pens.get(i),
            _ => None,
        }
        .or(s.pens.first());
        let brush = pen.map(|p| Brush { tool: p.tool, color: p.color, width: p.width_mm * PX_PER_MM });
        if tool != CanvasTool::Select {
            self.canvas.clear_selection();
        }
        self.canvas.configure(|c| {
            c.tool = tool;
            if let Some(b) = brush {
                c.brush = b;
            }
            c.eraser_mode = s.eraser_mode;
            c.stabilizer_mm = s.smoothing.radius_mm();
            c.smooth_passes = s.smoothing.passes();
            c.button_2 = s.pen_button_2;
            c.button_3 = s.pen_button_3;
            c.eraser_end = s.pen_eraser;
            c.text_size = s.text_size;
            c.text_font = s.text_font.clone();
            c.radial = radial;
            c.long_press = s.long_press_ms.map(|ms| Duration::from_millis(ms as u64));
            c.shape_hold = s.shape_hold_ms.map(|ms| Duration::from_millis(ms as u64));
        });
    }

    fn preferences(self: &Rc<Self>) {
        self.preferences_dialog().present(Some(&self.window));
    }

    /// The preferences dialog, built on first use.
    fn preferences_dialog(self: &Rc<Self>) -> &adw::Dialog {
        self.prefs_dialog.get_or_init(|| self.build_preferences())
    }

    fn build_preferences(self: &Rc<Self>) -> adw::Dialog {
        let current = {
            let s = &self.state.borrow().settings;
            Prefs {
                smoothing: s.smoothing,
                button_2: s.pen_button_2,
                button_3: s.pen_button_3,
                eraser: s.pen_eraser,
                long_press_ms: s.long_press_ms,
                shape_hold_ms: s.shape_hold_ms,
                radial_items: s.radial_items.clone(),
            }
        };
        let live = {
            let weak = self.weak();
            move || {
                let Some(this) = weak.upgrade() else { return Default::default() };
                let s = &this.state.borrow().settings;
                (s.pens.clone(), s.radial_items.clone())
            }
        };
        let weak = self.weak();
        preferences::build(current, live, move |p| {
            let this = upgrade!(weak);
            {
                let mut st = this.state.borrow_mut();
                let s = &mut st.settings;
                s.smoothing = p.smoothing;
                s.pen_button_2 = p.button_2;
                s.pen_button_3 = p.button_3;
                s.pen_eraser = p.eraser;
                s.long_press_ms = p.long_press_ms;
                s.shape_hold_ms = p.shape_hold_ms;
                s.radial_items = p.radial_items.clone();
                s.save();
            }
            this.apply_tools();
        })
    }

    /// After a rename or move, point the canvas at the open page's new file
    /// (keeping its undo history) instead of reloading it.
    fn retarget_open_page(&self) {
        let st = self.state.borrow();
        let open = st.open_page.as_ref().and_then(|k| Self::resolve_page(&st.lib, k));
        if let Some((n, s, p)) = open {
            self.canvas.retarget(&st.lib.page_path(n, s, p));
        }
    }

    // ---- commands --------------------------------------------------------------

    fn toast(&self, msg: &str) {
        self.toasts.add_toast(adw::Toast::new(msg));
    }

    /// Swap the item's sidebar label for a text field. Enter or clicking
    /// away saves, Esc cancels. `then` runs afterwards either way.
    fn start_inline_rename(self: &Rc<Self>, target: Target, then: Option<AfterRename>) {
        let current = {
            let st = self.state.borrow();
            let lib = &st.lib;
            match target {
                Target::Notebook(n) => lib.notebooks[n].name.clone(),
                Target::Section(n, s) => lib.notebooks[n].sections[s].name.clone(),
                Target::Page(n, s, p) => lib.notebooks[n].sections[s].pages[p].title.clone(),
            }
        };
        let index = self.state.borrow().rows.iter().position(|t| *t == target);
        let row = index.and_then(|i| self.list.row_at_index(i as i32));
        let label = row
            .as_ref()
            .and_then(Self::row_revealer)
            .and_then(|r| r.child())
            .and_then(|hbox| hbox.last_child())
            .and_downcast::<gtk::Label>();
        let (Some(row), Some(label)) = (row, label) else { return };
        let hbox = label.parent().and_downcast::<gtk::Box>().expect("row layout");

        let entry = gtk::Entry::builder().text(&current).hexpand(true).build();
        label.set_visible(false);
        hbox.append(&entry);
        self.state.borrow_mut().editing = true;

        let then = RefCell::new(then);
        let done = Rc::new(std::cell::Cell::new(false));
        let weak = self.weak();
        let entry_ref = entry.downgrade();
        let finish = Rc::new(move |save: bool| {
            if done.replace(true) {
                return;
            }
            let this = upgrade!(weak);
            this.state.borrow_mut().editing = false;
            let text = entry_ref.upgrade().map(|e| e.text().trim().to_string()).unwrap_or_default();
            if save && !text.is_empty() && text != current {
                this.flush_pending_delete();
                this.apply_rename(target, &text);
            } else {
                this.rebuild_sidebar();
            }
            if let Some(f) = then.borrow_mut().take() {
                f(&this);
            }
        });

        let f = finish.clone();
        entry.connect_activate(move |_| f(true));
        let keys = gtk::EventControllerKey::new();
        let f = finish.clone();
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gdk::Key::Escape {
                // Defer: finishing rebuilds the list, destroying this entry.
                let f = f.clone();
                glib::idle_add_local_once(move || f(false));
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        entry.add_controller(keys);
        let focus = gtk::EventControllerFocus::new();
        let f = finish.clone();
        focus.connect_leave(move |_| {
            let f = f.clone();
            glib::idle_add_local_once(move || f(true));
        });
        entry.add_controller(focus);

        row.grab_focus();
        entry.grab_focus();
        entry.select_region(0, -1);
    }

    fn new_notebook(self: &Rc<Self>) {
        self.flush_pending_delete();
        let result = self.state.borrow_mut().lib.create_notebook("New Notebook");
        let n = match result {
            Ok(n) => n,
            Err(e) => return self.toast(&format!("Couldn't create notebook: {e}")),
        };
        // Ready to write straight away: page 1 of the first section is open.
        let landscape = self.screen_is_landscape();
        let page = self.state.borrow_mut().lib.create_page(n, 0, landscape);
        match page {
            Ok(p) => {
                self.expand(n, Some(0));
                self.open_page(n, 0, p);
            }
            Err(e) => self.toast(&format!("Couldn't create page: {e}")),
        }
        self.rebuild_sidebar();
        self.start_inline_rename(Target::Notebook(n), None);
    }

    fn new_section(self: &Rc<Self>, target: Option<Target>) {
        let Some(Target::Notebook(n)) = target else { return };
        self.flush_pending_delete();
        let result = self.state.borrow_mut().lib.create_section(n, "New Section");
        let s = match result {
            Ok(s) => s,
            Err(e) => return self.toast(&format!("Couldn't create section: {e}")),
        };
        self.expand(n, Some(s));
        self.rebuild_sidebar();
        // Create page 1 once the section is named, so it's named after it.
        self.start_inline_rename(
            Target::Section(n, s),
            Some(Box::new(move |this| this.new_page(Some(Target::Section(n, s))))),
        );
    }

    fn new_page(self: &Rc<Self>, target: Option<Target>) {
        self.flush_pending_delete();
        let section = match target {
            Some(Target::Section(n, s) | Target::Page(n, s, _)) => Some((n, s)),
            _ => {
                let st = self.state.borrow();
                st.current_section.as_ref().and_then(|[nb, sec]| {
                    let n = st.lib.notebooks.iter().position(|x| &x.dir == nb)?;
                    let s = st.lib.notebooks[n].sections.iter().position(|x| &x.dir == sec)?;
                    Some((n, s))
                })
            }
        };
        let Some((n, s)) = section else {
            self.toast("Select a section first");
            return;
        };
        let landscape = self.screen_is_landscape();
        let result = self.state.borrow_mut().lib.create_page(n, s, landscape);
        match result {
            Ok(p) => {
                self.expand(n, Some(s));
                self.open_page(n, s, p);
            }
            Err(e) => self.toast(&format!("Couldn't create page: {e}")),
        }
        self.rebuild_sidebar();
    }

    fn rename(self: &Rc<Self>, target: Option<Target>) {
        let Some(target) = target else { return };
        // The item must be visible in the sidebar to edit it in place.
        match target {
            Target::Notebook(_) => {}
            Target::Section(n, _) => self.expand(n, None),
            Target::Page(n, s, _) => self.expand(n, Some(s)),
        }
        self.rebuild_sidebar();
        self.start_inline_rename(target, None);
    }

    fn apply_rename(self: &Rc<Self>, target: Target, name: &str) {
        // Renaming rewrites the page file; save the ink in it first.
        self.canvas.flush();
        let result = {
            let mut st = self.state.borrow_mut();
            let st = &mut *st;
            // Folder names change on rename; carry expanded/open state over.
            let old_open = st.open_page.as_ref().and_then(|k| Self::resolve_page(&st.lib, k));
            let was = |st: &State, key: &str| st.expanded.contains(key);
            let result = match target {
                Target::Notebook(n) => {
                    let old = st.lib.notebooks[n].dir.clone();
                    let prefixes: Vec<String> =
                        st.expanded.iter().filter(|k| k.starts_with(&old)).cloned().collect();
                    let r = st.lib.rename_notebook(n, name);
                    let new = st.lib.notebooks[n].dir.clone();
                    for k in prefixes {
                        st.expanded.remove(&k);
                        st.expanded.insert(k.replacen(&old, &new, 1));
                    }
                    r
                }
                Target::Section(n, s) => {
                    let nb = st.lib.notebooks[n].dir.clone();
                    let old = format!("{nb}/{}", st.lib.notebooks[n].sections[s].dir);
                    let open = was(st, &old);
                    let r = st.lib.rename_section(n, s, name);
                    if open {
                        st.expanded.remove(&old);
                        st.expanded.insert(format!("{nb}/{}", st.lib.notebooks[n].sections[s].dir));
                    }
                    r
                }
                Target::Page(n, s, p) => st.lib.rename_page(n, s, p, name),
            };
            if let Some((n, s, p)) = old_open {
                let key = Self::page_key(&st.lib, n, s, p);
                st.current_section = Some([key[0].clone(), key[1].clone()]);
                st.settings.last_page = Some(key.clone());
                st.open_page = Some(key);
            }
            result
        };
        if let Err(e) = result {
            self.toast(&format!("Couldn't rename: {e}"));
        }
        self.retarget_open_page();
        self.rebuild_sidebar();
        self.update_content();
    }

    fn delete(self: &Rc<Self>, target: Option<Target>) {
        let Some(target) = target else { return };
        self.flush_pending_delete();
        self.canvas.flush();
        let (name, result, id) = {
            let mut st = self.state.borrow_mut();
            let name = match target {
                Target::Notebook(n) => st.lib.notebooks[n].name.clone(),
                Target::Section(n, s) => st.lib.notebooks[n].sections[s].name.clone(),
                Target::Page(n, s, p) => st.lib.notebooks[n].sections[s].pages[p].title.clone(),
            };
            let result = st.lib.detach(target);
            let id = st.next_pending_id;
            st.next_pending_id += 1;
            match result {
                Ok((detached, path)) => {
                    st.pending = Some(PendingDelete { id, detached, path });
                    (name, Ok(()), id)
                }
                Err(e) => (name, Err(e), id),
            }
        };
        if let Err(e) = result {
            self.toast(&format!("Couldn't delete “{name}”: {e}"));
            return;
        }
        self.rebuild_sidebar();
        self.update_content();

        let toast = adw::Toast::builder()
            .title(format!("“{name}” moved to trash"))
            .button_label("Undo")
            .priority(adw::ToastPriority::High)
            .build();
        let weak = self.weak();
        toast.connect_button_clicked(move |_| {
            let this = upgrade!(weak);
            let pending = this.state.borrow_mut().pending.take_if(|p| p.id == id);
            if let Some(p) = pending {
                let result = this.state.borrow_mut().lib.reattach(p.detached);
                if let Err(e) = result {
                    this.toast(&format!("Couldn't undo: {e}"));
                }
                this.rebuild_sidebar();
                this.update_content();
            }
        });
        let weak = self.weak();
        toast.connect_dismissed(move |_| {
            let this = upgrade!(weak);
            let is_this = this.state.borrow().pending.as_ref().is_some_and(|p| p.id == id);
            if is_this {
                this.flush_pending_delete();
            }
        });
        self.toasts.add_toast(toast);
    }

    /// Trash the item waiting on an "Undo" toast. Called before any other
    /// change to the library so undo never restores into a changed structure.
    fn flush_pending_delete(&self) {
        let Some(p) = self.state.borrow_mut().pending.take() else { return };
        if let Err(e) = gio::File::for_path(&p.path).trash(gio::Cancellable::NONE) {
            self.toast(&format!("Couldn't move {} to trash: {e}", p.path.display()));
        }
    }

    fn open_folder(&self) {
        let root = self.state.borrow().lib.root.clone();
        gtk::FileLauncher::new(Some(&gio::File::for_path(root))).launch(
            Some(&self.window),
            gio::Cancellable::NONE,
            |_| {},
        );
    }

    fn change_folder(self: &Rc<Self>) {
        let dialog = gtk::FileDialog::builder().title("Choose Notebooks Folder").build();
        let weak = self.weak();
        dialog.select_folder(Some(&self.window), gio::Cancellable::NONE, move |res| {
            let this = upgrade!(weak);
            let Some(dir) = res.ok().and_then(|f| f.path()) else { return };
            this.flush_pending_delete();
            this.canvas.flush();
            match Library::load(&dir) {
                Ok(lib) => {
                    let mut st = this.state.borrow_mut();
                    st.lib = lib;
                    st.settings.library_dir = Some(dir);
                    st.settings.last_page = None;
                    st.settings.save();
                    st.open_page = None;
                    st.current_section = None;
                    st.expanded.clear();
                }
                Err(e) => this.toast(&format!("Couldn't open {}: {e}", dir.display())),
            }
            this.rebuild_sidebar();
            this.update_content();
        });
    }

    fn about(&self) {
        adw::AboutDialog::builder()
            .application_name("Ratchet")
            .application_icon(crate::APP_ID)
            .version(env!("CARGO_PKG_VERSION"))
            .comments("Handwritten notes for university students.")
            .developer_name("r3ktline")
            .build()
            .present(Some(&self.window));
    }
}

/// A GTK decoration layout ("icon:minimize,close") with the window icon removed.
fn without_icon(layout: &str) -> String {
    layout
        .split(':')
        .map(|side| side.split(',').filter(|b| b.trim() != "icon").collect::<Vec<_>>().join(","))
        .collect::<Vec<_>>()
        .join(":")
}
