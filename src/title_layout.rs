//! Layout for the content header: start side, title, end side.
//!
//! The title sits in the exact middle of the bar while the start side's
//! tools leave room, moves right to make room for them when they don't, and
//! only then do the tools give up space (they scroll). The end side keeps
//! its full width. An AdwHeaderBar instead asks for twice its wider side to
//! keep the title centered, more than a small window has, and squeezes the
//! title before the tools.

use std::cell::Cell;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct TitleLayout {
        pub gap: Cell<i32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for TitleLayout {
        const NAME: &'static str = "RatchetTitleLayout";
        type Type = super::TitleLayout;
        type ParentType = gtk::LayoutManager;
    }

    impl ObjectImpl for TitleLayout {}

    impl LayoutManagerImpl for TitleLayout {
        fn request_mode(&self, _widget: &gtk::Widget) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::ConstantSize
        }

        fn measure(
            &self,
            widget: &gtk::Widget,
            orientation: gtk::Orientation,
            _for_size: i32,
        ) -> (i32, i32, i32, i32) {
            let kids = children(widget);
            if orientation == gtk::Orientation::Horizontal {
                let gaps = self.gap.get() * (kids.len() as i32 - 1).max(0);
                let (min, nat) = kids.iter().map(width).fold((0, 0), |(a, b), (m, n)| (a + m, b + n));
                (min + gaps, nat + gaps, -1, -1)
            } else {
                let (min, nat) = kids.iter().fold((0, 0), |(a, b), c| {
                    let (m, n, _, _) = c.measure(orientation, -1);
                    (a.max(m), b.max(n))
                });
                (min, nat, -1, -1)
            }
        }

        fn allocate(&self, widget: &gtk::Widget, w: i32, h: i32, _baseline: i32) {
            let kids = children(widget);
            let [start, title, end] = &kids[..] else { return };
            let gap = self.gap.get();
            let ((s_min, s_nat), (t_min, t_nat), (e_min, e_nat)) = (width(start), width(title), width(end));
            let end_w = e_nat.min(w - s_min - t_min - 2 * gap).max(e_min);
            // Room for the start side and the title.
            let room = w - end_w - 2 * gap;
            let title_w = t_nat.min(room - s_min).max(t_min);
            let title_x = ((w - title_w) / 2)
                .max(s_nat.min(room - title_w) + gap)
                .min(w - end_w - gap - title_w);
            let place = |c: &gtk::Widget, x: i32, cw: i32| {
                let at = gtk::gsk::Transform::new().translate(&gtk::graphene::Point::new(x as f32, 0.0));
                c.allocate(cw.max(0), h, -1, Some(at));
            };
            place(start, 0, title_x - gap);
            place(title, title_x, title_w);
            place(end, w - end_w, end_w);
        }
    }

    fn children(widget: &gtk::Widget) -> Vec<gtk::Widget> {
        let mut kids = Vec::new();
        let mut child = widget.first_child();
        while let Some(c) = child {
            child = c.next_sibling();
            if c.should_layout() {
                kids.push(c);
            }
        }
        kids
    }

    fn width(c: &gtk::Widget) -> (i32, i32) {
        let (min, nat, _, _) = c.measure(gtk::Orientation::Horizontal, -1);
        (min, nat)
    }
}

glib::wrapper! {
    pub struct TitleLayout(ObjectSubclass<imp::TitleLayout>) @extends gtk::LayoutManager;
}

impl TitleLayout {
    /// `gap`: space between the three parts.
    pub fn new(gap: i32) -> Self {
        let this: Self = glib::Object::new();
        this.imp().gap.set(gap);
        this
    }
}
