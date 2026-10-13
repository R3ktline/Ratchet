//! Shared UI fixes applied to whole dialogs and popovers.

use gtk::prelude::*;

/// How far (in px) the middle of a label's capital letters sits below the
/// middle of its line box. Fonts like Segoe UI leave more room below the
/// letters than above, so a label's box is centered but its text isn't.
fn cap_offset(label: &gtk::Label) -> i32 {
    let layout = label.create_pango_layout(Some("H"));
    let (ink, logical) = layout.pixel_extents();
    let ink_mid = ink.y() as f64 + ink.height() as f64 / 2.0;
    let line_mid = logical.y() as f64 + logical.height() as f64 / 2.0;
    (ink_mid - line_mid).round() as i32
}

/// Give every icon the same center as the label beside it.
///
/// GTK lines an icon beside a label up with the text's baseline; with
/// Windows fonts that leaves the icon visibly higher or lower than the text
/// (tabs, toggle buttons, icon+label buttons). Wherever a box holds both an
/// icon and a label, center both, then nudge the icon onto the middle of
/// the letters themselves. Call on a dialog or popover once it is built,
/// and when it is shown (fonts are only final then, and some widgets build
/// their insides late).
pub fn align_icon_labels(root: &impl IsA<gtk::Widget>) {
    fn walk(w: &gtk::Widget) {
        let mut kids = Vec::new();
        let mut child = w.first_child();
        while let Some(c) = child {
            child = c.next_sibling();
            kids.push(c);
        }
        let label = kids.iter().find_map(|c| c.downcast_ref::<gtk::Label>().cloned());
        let has_icon = kids.iter().any(|c| c.is::<gtk::Image>());
        if let (Some(label), true) = (label, has_icon) {
            // A centered widget moves by half its margin.
            let shift = 2 * cap_offset(&label);
            for c in &kids {
                if c.is::<gtk::Label>() {
                    c.set_valign(gtk::Align::Center);
                } else if c.is::<gtk::Image>() {
                    c.set_valign(gtk::Align::Center);
                    c.set_margin_top(shift.max(0));
                    c.set_margin_bottom((-shift).max(0));
                }
            }
        }
        for c in &kids {
            walk(c);
        }
    }
    walk(root.upcast_ref());
}

/// `align_icon_labels` now and each time the widget is shown.
pub fn keep_icon_labels_aligned(root: &impl IsA<gtk::Widget>) {
    align_icon_labels(root);
    root.connect_map(|w| align_icon_labels(w));
}
