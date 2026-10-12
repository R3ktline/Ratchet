//! Main window: notebook/section/page tree in a sidebar, page in the content.

use std::cell::RefCell;
use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::{Rc, Weak};

use adw::prelude::*;
use gtk::{gdk, gio, glib};

use crate::library::{Detached, Library, Target};
use crate::settings::{Settings, Theme};

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
    page_status: adw::StatusPage,
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
        let page_status = adw::StatusPage::builder()
            .icon_name("document-edit-symbolic")
            .description("The handwriting canvas arrives in phase 3.")
            .build();
        let stack = gtk::Stack::new();

        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("Ratchet")
            .default_width(1280)
            .default_height(800)
            .width_request(360)
            .height_request(300)
            .build();

        let this = Rc::new(Window {
            window,
            list,
            list_box,
            menu,
            toasts: adw::ToastOverlay::new(),
            title,
            stack,
            page_status,
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
        });
        this.build_layout();
        this.install_actions(app);
        this.install_sidebar_input();
        this.restore_last_page();
        this.rebuild_sidebar();
        this.update_content();
        if let Some(msg) = load_error {
            this.toast(&msg);
        }
        this
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
        about.append(Some("About Ratchet"), Some("win.about"));
        main_menu.append_section(None, &about);
        let menu_button = gtk::MenuButton::builder()
            .icon_name("open-menu-symbolic")
            .menu_model(&main_menu)
            .tooltip_text("Main Menu")
            .primary(true)
            .build();

        let sidebar_header = adw::HeaderBar::new();
        sidebar_header.set_title_widget(Some(&adw::WindowTitle::new("Ratchet", "")));
        sidebar_header.pack_start(&new_notebook);
        sidebar_header.pack_end(&menu_button);

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
        let content_header = adw::HeaderBar::new();
        content_header.set_title_widget(Some(&self.title));
        content_header.pack_start(&toggle_sidebar);
        content_header.pack_start(&new_page);

        let create_first = gtk::Button::builder()
            .label("New Notebook")
            .halign(gtk::Align::Center)
            .css_classes(["pill", "suggested-action"])
            .action_name("win.new-notebook")
            .build();
        let empty = adw::StatusPage::builder()
            .icon_name("accessories-text-editor-symbolic")
            .title("No Page Open")
            .description("Pick a page in the sidebar, or create a notebook for a course.")
            .child(&create_first)
            .build();
        self.stack.add_named(&empty, Some("empty"));
        self.stack.add_named(&self.page_status, Some("page"));

        let content = adw::ToolbarView::new();
        content.add_top_bar(&content_header);
        content.set_content(Some(&self.stack));

        let split = adw::OverlaySplitView::builder()
            .sidebar(&sidebar)
            .content(&content)
            .min_sidebar_width(220.0)
            .max_sidebar_width(320.0)
            .build();
        split.bind_property("show-sidebar", &toggle_sidebar, "active").bidirectional().build();

        // Collapse the sidebar into an overlay on narrow windows.
        let narrow = adw::Breakpoint::new(adw::BreakpointCondition::new_length(
            adw::BreakpointConditionLengthType::MaxWidth,
            600.0,
            adw::LengthUnit::Sp,
        ));
        narrow.add_setter(&split, "collapsed", Some(&true.to_value()));
        self.window.add_breakpoint(narrow);

        let toggle = gio::SimpleAction::new("toggle-sidebar", None);
        toggle.connect_activate(glib::clone!(
            #[weak]
            split,
            move |_, _| split.set_show_sidebar(!split.shows_sidebar())
        ));
        self.window.add_action(&toggle);

        self.toasts.set_child(Some(&split));
        self.window.set_content(Some(&self.toasts));

        // This handler owns the strong reference that keeps `Window` alive
        // for as long as the GTK window exists; everything else holds weak refs.
        let this = self.clone();
        self.window.connect_close_request(move |_| {
            this.flush_pending_delete();
            let mut st = this.state.borrow_mut();
            let mut expanded: Vec<String> = st.expanded.iter().cloned().collect();
            expanded.sort();
            st.settings.expanded = expanded;
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
        add("open-folder", false, Box::new(|w, _| w.open_folder()));
        add("change-folder", false, Box::new(|w, _| w.change_folder()));
        add("about", false, Box::new(|w, _| w.about()));

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
                    let row = self.make_row(1, Some(sec_open), None, &sec.name);
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
        match selected_row {
            Some(row) => self.list.select_row(Some(&row)),
            None => self.list.unselect_all(),
        }
    }

    fn make_row(
        self: &Rc<Self>,
        depth: i32,
        expanded: Option<bool>,
        color: Option<&str>,
        label: &str,
    ) -> gtk::ListBoxRow {
        let hbox = gtk::Box::builder()
            .spacing(8)
            .margin_start(depth * 16)
            .build();
        match expanded {
            Some(open) => {
                let icon = if open { "pan-down-symbolic" } else { "pan-end-symbolic" };
                hbox.append(&gtk::Image::from_icon_name(icon));
            }
            // Align pages with their section's label.
            None => hbox.append(&gtk::Box::builder().width_request(16).build()),
        }
        if let Some(color) = color {
            let dot = gtk::Box::builder()
                .valign(gtk::Align::Center)
                .css_classes(["nb-dot", &format!("nb-{color}")])
                .build();
            hbox.append(&dot);
        }
        hbox.append(
            &gtk::Label::builder()
                .label(label)
                .xalign(0.0)
                .hexpand(true)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .tooltip_text(label)
                .build(),
        );
        let row = gtk::ListBoxRow::builder().child(&hbox).selectable(false).build();
        self.attach_dnd(&row);
        row
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
        edit.append_item(&item("Move to Trash", "delete", &t));
        menu.append_section(None, &edit);

        self.menu.set_menu_model(Some(&menu));
        self.menu.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        self.menu.popup();
    }

    fn toggle_expanded(self: &Rc<Self>, n: usize, s: Option<usize>) {
        {
            let mut st = self.state.borrow_mut();
            let nb = &st.lib.notebooks[n];
            let key = match s {
                Some(s) => format!("{}/{}", nb.dir, nb.sections[s].dir),
                None => nb.dir.clone(),
            };
            if let Some(s) = s {
                st.current_section = Some([nb.dir.clone(), nb.sections[s].dir.clone()]);
            }
            if !st.expanded.remove(&key) {
                st.expanded.insert(key);
            }
        }
        self.rebuild_sidebar();
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
                self.page_status.set_title(title);
                self.window.set_title(Some(&format!("{title} – Ratchet")));
                self.stack.set_visible_child_name("page");
            }
            None => {
                self.title.set_title("Ratchet");
                self.title.set_subtitle("");
                self.window.set_title(Some("Ratchet"));
                self.stack.set_visible_child_name("empty");
            }
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
        let page = self.state.borrow_mut().lib.create_page(n, 0);
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
        let result = self.state.borrow_mut().lib.create_page(n, s);
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
        self.rebuild_sidebar();
        self.update_content();
    }

    fn delete(self: &Rc<Self>, target: Option<Target>) {
        let Some(target) = target else { return };
        self.flush_pending_delete();
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
            .version(env!("CARGO_PKG_VERSION"))
            .comments("Handwritten notes for university students.")
            .developer_name("r3ktline")
            .build()
            .present(Some(&self.window));
    }
}
