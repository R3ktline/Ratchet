//! Text boxes: rich text runs (bold/italic/underline), "- " bullets and
//! LaTeX math in `$…$` (inline) or `$$…$$` (display), laid out with Pango.
//!
//! Math is reserved in the Pango layout as an object-replacement character
//! with a shape attribute of the formula's size, then drawn on top.

use std::rc::Rc;

use gtk::pango;
use gtk::prelude::*;
use gtk::{gdk, graphene};
use serde::{Deserialize, Serialize};

use crate::i18n::tr;
use crate::ink::InkColor;
use crate::math::{self, Formula};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TextSize {
    Small,
    #[default]
    Normal,
    Large,
    Heading,
}

impl TextSize {
    pub const ALL: [TextSize; 4] = [TextSize::Small, TextSize::Normal, TextSize::Large, TextSize::Heading];

    /// Font size in page units.
    pub fn px(self) -> f32 {
        match self {
            TextSize::Small => 13.0,
            TextSize::Normal => 16.0,
            TextSize::Large => 21.0,
            TextSize::Heading => 28.0,
        }
    }

    pub fn label(self) -> String {
        match self {
            TextSize::Small => tr("Small"),
            TextSize::Normal => tr("Normal"),
            TextSize::Large => tr("Large"),
            TextSize::Heading => tr("Heading"),
        }
    }
}

fn is_false(b: &bool) -> bool {
    !*b
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Style {
    #[serde(default, skip_serializing_if = "is_false")]
    pub bold: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub italic: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub underline: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Run {
    pub text: String,
    #[serde(flatten)]
    pub style: Style,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TextBox {
    /// Top-left corner, page units.
    pub x: f32,
    pub y: f32,
    /// Wrap width; `None` grows with the text.
    #[serde(default)]
    pub width: Option<f32>,
    #[serde(default)]
    pub size: TextSize,
    /// Font family; `None` is the system UI font.
    #[serde(default)]
    pub font: Option<String>,
    pub color: InkColor,
    pub runs: Vec<Run>,
    /// Laid-out size, filled in by `measure`.
    #[serde(skip)]
    pub extent: (f32, f32),
}

impl TextBox {
    pub fn new(x: f32, y: f32, width: Option<f32>, size: TextSize, font: Option<String>, color: InkColor) -> Self {
        TextBox { x, y, width, size, font, color, runs: Vec::new(), extent: (0.0, 0.0) }
    }

    pub fn plain_text(&self) -> String {
        self.runs.iter().map(|r| r.text.as_str()).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.plain_text().trim().is_empty()
    }

    pub fn bounds(&self) -> crate::ink::Bounds {
        let w = self.width.unwrap_or(self.extent.0).max(self.extent.0);
        crate::ink::Bounds { x0: self.x, y0: self.y, x1: self.x + w, y1: self.y + self.extent.1 }
    }

    pub fn contains(&self, x: f32, y: f32, pad: f32) -> bool {
        let b = self.bounds();
        x >= b.x0 - pad && x <= b.x1 + pad && y >= b.y0 - pad && y <= b.y1 + pad
    }

    /// Lay out and record the size.
    pub fn measure(&mut self, ctx: &pango::Context) {
        let l = layout(self, ctx, false);
        self.extent = l.size;
    }

    pub fn font_description(&self) -> pango::FontDescription {
        let mut desc = match &self.font {
            Some(f) => pango::FontDescription::from_string(f),
            None => pango::FontDescription::from_string("Sans"),
        };
        desc.set_absolute_size(self.size.px() as f64 * pango::SCALE as f64);
        desc
    }
}

/// A math span in the source text.
#[derive(Debug, PartialEq)]
pub struct MathSpan {
    /// Byte range including the dollar signs.
    pub range: std::ops::Range<usize>,
    pub latex: String,
    pub display: bool,
}

/// Find `$…$` and `$$…$$` spans. An unclosed `$` is plain text.
pub fn math_spans(text: &str) -> Vec<MathSpan> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'$' {
            i += 1;
            continue;
        }
        let display = bytes.get(i + 1) == Some(&b'$');
        let open = if display { 2 } else { 1 };
        let close = if display { "$$" } else { "$" };
        let body_start = i + open;
        match text[body_start..].find(close) {
            Some(rel) if rel > 0 => {
                let end = body_start + rel + open;
                out.push(MathSpan {
                    range: i..end,
                    latex: text[body_start..body_start + rel].to_string(),
                    display,
                });
                i = end;
            }
            _ => i += open,
        }
    }
    out
}

pub struct Laid {
    pub layout: pango::Layout,
    /// Formulas to draw: (formula, x, baseline y) in box coordinates.
    pub formulas: Vec<(Rc<Formula>, f32, f32)>,
    pub size: (f32, f32),
}

const OBJECT: char = '\u{FFFC}';

/// Build the Pango layout for a box. `editing_errors` isn't needed: broken
/// math is shown as its source in a warning color.
pub fn layout(b: &TextBox, ctx: &pango::Context, dark: bool) -> Laid {
    let em = b.size.px();
    let source: String = b.plain_text();
    // Style of every source byte (runs are contiguous).
    let mut styles = Vec::with_capacity(source.len());
    for r in &b.runs {
        styles.extend(std::iter::repeat_n(r.style, r.text.len()));
    }
    let spans = math_spans(&source);

    let mut display = String::with_capacity(source.len());
    let attrs = pango::AttrList::new();
    let mut shapes: Vec<(usize, Rc<Formula>)> = Vec::new();
    let mut style_marks: Vec<(usize, usize, Style)> = Vec::new();
    let error_color = if dark { (0xff, 0x7b, 0x63) } else { (0xc0, 0x1c, 0x28) };

    let mut i = 0;
    let mut at_line_start = true;
    let mut push_text = |display: &mut String, text: &str, start: usize, at_line_start: &mut bool| {
        for (off, ch) in text.char_indices() {
            let src = start + off;
            let pos = display.len();
            // "- " at the start of a line shows as a bullet.
            if *at_line_start && ch == '-' && source[src..].starts_with("- ") {
                display.push('•');
            } else {
                display.push(ch);
            }
            style_marks.push((pos, display.len(), styles.get(src).copied().unwrap_or_default()));
            *at_line_start = ch == '\n';
        }
    };
    for span in &spans {
        push_text(&mut display, &source[i..span.range.start], i, &mut at_line_start);
        match math::formula(&span.latex, span.display) {
            Ok(f) => {
                let pos = display.len();
                display.push(OBJECT);
                let s = pango::SCALE as f32;
                let rect = pango::Rectangle::new(
                    0,
                    (-f.ascent * em * s) as i32,
                    (f.width * em * s) as i32,
                    ((f.ascent + f.descent) * em * s) as i32,
                );
                let mut shape = pango::AttrShape::new(&rect, &rect);
                shape.set_start_index(pos as u32);
                shape.set_end_index(display.len() as u32);
                attrs.insert(shape);
                shapes.push((pos, f));
                at_line_start = false;
            }
            Err(_) => {
                let pos = display.len();
                display.push_str(&source[span.range.clone()]);
                let mut c = pango::AttrColor::new_foreground(
                    error_color.0 * 257,
                    error_color.1 * 257,
                    error_color.2 * 257,
                );
                c.set_start_index(pos as u32);
                c.set_end_index(display.len() as u32);
                attrs.insert(c);
                at_line_start = false;
            }
        }
        i = span.range.end;
    }
    push_text(&mut display, &source[i..], i, &mut at_line_start);

    // Merge per-char styles into attribute ranges.
    let mut k = 0;
    while k < style_marks.len() {
        let (start, _, style) = style_marks[k];
        let mut end = style_marks[k].1;
        while k + 1 < style_marks.len() && style_marks[k + 1].2 == style && style_marks[k + 1].0 == end {
            k += 1;
            end = style_marks[k].1;
        }
        let range = |mut a: pango::Attribute| {
            a.set_start_index(start as u32);
            a.set_end_index(end as u32);
            attrs.insert(a);
        };
        if style.bold {
            range(pango::AttrInt::new_weight(pango::Weight::Bold).into());
        }
        if style.italic {
            range(pango::AttrInt::new_style(pango::Style::Italic).into());
        }
        if style.underline {
            range(pango::AttrInt::new_underline(pango::Underline::Single).into());
        }
        k += 1;
    }

    let layout = pango::Layout::new(ctx);
    layout.set_text(&display);
    layout.set_attributes(Some(&attrs));
    layout.set_font_description(Some(&b.font_description()));
    if let Some(w) = b.width {
        layout.set_width((w * pango::SCALE as f32) as i32);
        layout.set_wrap(pango::WrapMode::WordChar);
    }

    let formulas = shapes
        .into_iter()
        .map(|(idx, f)| {
            let pos = layout.index_to_pos(idx as i32);
            let s = pango::SCALE as f32;
            let x = pos.x() as f32 / s;
            // Baseline of the line holding the formula.
            let (line, _) = layout.index_to_line_x(idx as i32, false);
            let baseline = layout
                .line_readonly(line)
                .map(|_| {
                    let mut iter = layout.iter();
                    for _ in 0..line {
                        iter.next_line();
                    }
                    iter.baseline() as f32 / s
                })
                .unwrap_or(pos.y() as f32 / s + f.ascent * em);
            (f, x, baseline)
        })
        .collect();
    let (w, h) = layout.size();
    let s = pango::SCALE as f32;
    Laid { layout, formulas, size: ((w as f32 / s).max(4.0), (h as f32 / s).max(em * 1.2)) }
}

/// Draw a laid-out box at its position.
pub fn draw(snapshot: &gtk::Snapshot, b: &TextBox, laid: &Laid, color: &gdk::RGBA) {
    snapshot.save();
    snapshot.translate(&graphene::Point::new(b.x, b.y));
    snapshot.append_layout(&laid.layout, color);
    for (f, x, baseline) in &laid.formulas {
        f.draw(snapshot, *x, *baseline, b.size.px(), color);
    }
    snapshot.restore();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_math_spans() {
        let spans = math_spans("area $\\pi r^2$ and $$\\int f$$ but $5 alone");
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].latex, "\\pi r^2");
        assert!(!spans[0].display);
        assert_eq!(spans[1].latex, "\\int f");
        assert!(spans[1].display);
        assert!(math_spans("costs $5").is_empty());
        assert!(math_spans("empty $$ here").is_empty());
    }

    #[test]
    fn runs_roundtrip_compactly() {
        let mut b = TextBox::new(1.0, 2.0, None, TextSize::Large, None, InkColor::Ink);
        b.runs = vec![
            Run { text: "plain ".into(), style: Style::default() },
            Run { text: "bold".into(), style: Style { bold: true, ..Default::default() } },
        ];
        let json = serde_json::to_string(&b).unwrap();
        assert!(!json.contains("italic"), "{json}");
        let back: TextBox = serde_json::from_str(&json).unwrap();
        assert_eq!(back.runs, b.runs);
        assert_eq!(back.size, TextSize::Large);
        assert_eq!(back.plain_text(), "plain bold");
    }
}
