//! Fill & Sign (Acrobat's Fill & Sign tool, execution plan M5.7): type text onto the page, place
//! ✓ ✕ ● ─ marks and today's date, and sign with a drawn signature. Everything is an annotation
//! (typewriter text, PdfCraft-drawn stamps, ink), so it can be moved, deleted and undone like
//! any comment. A signature can also be a picture (a photo or scan of it), placed as a stamp.

use std::sync::Arc;

use egui::{Color32, CornerRadius, Pos2, Sense, Stroke, pos2, vec2};
use pdfcraft_engine::{Edit, FillMark, NewAnnotation, Shape, Style};
use pdfcraft_render::DocInfo;

use crate::canvas::{DocView, PageXform};
use crate::theme::Tokens;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FillTool {
    Text,
    Check,
    Cross,
    Dot,
    Line,
    Date,
    Signature,
    Initials,
}

pub const FILL_TOOLS: [FillTool; 8] =
    [FillTool::Text, FillTool::Cross, FillTool::Check, FillTool::Dot, FillTool::Line, FillTool::Date, FillTool::Signature, FillTool::Initials];

/// A saved signature or initials: drawn strokes (normalised to the pad width, y up), typed
/// text (drawn in the script font), or a picture (a PNG prepared by
/// [`pdfcraft_engine::signature_png`]: ink on transparency).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum SavedSig {
    Drawn(Vec<Vec<[f32; 2]>>),
    Typed(String),
    Image(#[serde(with = "crate::stamps_ui::b64")] Arc<Vec<u8>>),
}

/// The largest picture file read for a signature.
pub const MAX_SIGNATURE_FILE: usize = 30 << 20;

/// A saved value read back from the settings file is usable: a picture is a PNG of sane size.
pub(crate) fn saved_is_sound(s: &SavedSig) -> bool {
    match s {
        SavedSig::Image(png) => png.len() <= MAX_SIGNATURE_FILE && png_size(png).is_some(),
        SavedSig::Drawn(strokes) => strokes.iter().all(|st| st.iter().all(|p| p.iter().all(|x| x.is_finite()))),
        SavedSig::Typed(t) => !t.trim().is_empty(),
    }
}

/// A prepared signature picture's size in pixels (its PNG header).
pub(crate) fn png_size(png: &[u8]) -> Option<(u32, u32)> {
    let w = u32::from_be_bytes(png.get(16..20)?.try_into().ok()?);
    let h = u32::from_be_bytes(png.get(20..24)?.try_into().ok()?);
    (png.starts_with(b"\x89PNG") && w > 0 && h > 0).then_some((w, h))
}

/// The Create signature / initials dialog.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SigDraft {
    pub strokes: Vec<Vec<[f32; 2]>>,
    pub text: String,
    /// The Draw tab (otherwise Type, or Image when `picture`).
    pub drawing: bool,
    /// The Image tab, and its prepared picture.
    pub picture: bool,
    pub image: Option<Arc<Vec<u8>>>,
    /// Choose picture… was clicked: the app opens a file picker.
    pub pick: bool,
    /// Creating initials (otherwise the signature).
    pub initials: bool,
    /// Replacing an existing saved value.
    pub editing: bool,
}

impl SigDraft {
    pub fn new(initials: bool, name: &str) -> Self {
        let text = if initials { name.split_whitespace().filter_map(|w| w.chars().next()).collect() } else { name.trim().to_string() };
        Self { text, initials, ..Default::default() }
    }

    /// Edit a copy of the saved value; Cancel must leave the original untouched.
    pub fn from_saved(initials: bool, saved: &SavedSig) -> Self {
        match saved {
            SavedSig::Drawn(strokes) => Self { strokes: strokes.clone(), drawing: true, initials, editing: true, ..Default::default() },
            SavedSig::Typed(text) => Self { text: text.clone(), initials, editing: true, ..Default::default() },
            SavedSig::Image(png) => Self { image: Some(png.clone()), picture: true, initials, editing: true, ..Default::default() },
        }
    }

    fn ready(&self) -> bool {
        if self.picture {
            self.image.is_some()
        } else if self.drawing {
            self.strokes.iter().any(|s| s.len() > 1)
        } else {
            !self.text.trim().is_empty()
                && self.text.chars().take(pdfcraft_engine::MAX_SIGNATURE_CHARS + 1).count() <= pdfcraft_engine::MAX_SIGNATURE_CHARS
        }
    }

    pub fn saved(&self) -> SavedSig {
        match &self.image {
            Some(png) if self.picture => SavedSig::Image(png.clone()),
            _ if self.drawing => SavedSig::Drawn(self.strokes.clone()),
            _ => SavedSig::Typed(self.text.trim().to_string()),
        }
    }
}

impl FillTool {
    pub fn command(self) -> &'static str {
        match self {
            FillTool::Text => "sign.fill.text",
            FillTool::Check => "sign.fill.check",
            FillTool::Cross => "sign.fill.cross",
            FillTool::Dot => "sign.fill.dot",
            FillTool::Line => "sign.fill.line",
            FillTool::Date => "sign.fill.date",
            FillTool::Signature => "sign.fill.signature",
            FillTool::Initials => "sign.fill.initials",
        }
    }

    pub fn from_command(id: &str) -> Option<Self> {
        FILL_TOOLS.into_iter().find(|t| t.command() == id)
    }

    pub fn label(self) -> &'static str {
        match self {
            FillTool::Text => "Add text",
            FillTool::Check => "Checkmark",
            FillTool::Cross => "Cross",
            FillTool::Dot => "Dot",
            FillTool::Line => "Line",
            FillTool::Date => "Date",
            FillTool::Signature => "Sign",
            FillTool::Initials => "Initials",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            FillTool::Text => "type",
            FillTool::Check => "check",
            FillTool::Cross => "x",
            FillTool::Dot => "circle-dot",
            FillTool::Line => "minus",
            FillTool::Date => "clock-3",
            FillTool::Signature | FillTool::Initials => "signature",
        }
    }

    fn mark(self) -> Option<FillMark> {
        match self {
            FillTool::Check => Some(FillMark::Check),
            FillTool::Cross => Some(FillMark::Cross),
            FillTool::Dot => Some(FillMark::Dot),
            FillTool::Line => Some(FillMark::Line),
            _ => None,
        }
    }
}

/// Text being typed onto a page: (page, top-left in user space, text, focus requested).
#[derive(Clone, Debug, PartialEq)]
pub struct TypeBox {
    pub page: usize,
    pub at: [f64; 2],
    pub text: String,
    pub focus: bool,
}

/// The size Fill & Sign uses for typed text (Acrobat's default is 10 pt).
pub const TEXT_SIZE: f64 = 10.0;

fn to_user(xf: &PageXform, info: &DocInfo, page: usize, p: Pos2) -> [f64; 2] {
    let (vx, vy) = xf.screen_to_view(p);
    let u = info.pages[page].view_to_user(vx, vy);
    [u[0] as f64, u[1] as f64]
}

fn new(page: usize, shape: Shape, contents: String, author: &str) -> Edit {
    let style = Style::default_for(&shape);
    Edit::AddAnnotation(NewAnnotation { page, shape, style, contents, author: author.to_string() })
}

/// A typewriter annotation sized to its text.
pub fn typed(page: usize, at: [f64; 2], text: &str, author: &str) -> Edit {
    let rect = crate::comments::text_box_rect(at, text, TEXT_SIZE);
    new(page, Shape::Typewriter { rect, font_size: TEXT_SIZE }, text.to_string(), author)
}

/// Place a saved signature (strokes normalised to a 0–1 box, y up) with its left edge at `at`,
/// 150 pt wide.
pub fn signature_at(page: usize, at: [f64; 2], strokes: &[Vec<[f32; 2]>], author: &str) -> Option<Edit> {
    let w = 150.0;
    let (min_y, max_y) = strokes.iter().flatten().fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p[1]), b.max(p[1])));
    if !min_y.is_finite() {
        return None;
    }
    let h = f64::from(max_y - min_y).max(0.05) * w;
    let strokes: Vec<Vec<[f64; 2]>> = strokes
        .iter()
        .filter(|s| !s.is_empty())
        .map(|s| s.iter().map(|p| [at[0] + f64::from(p[0]) * w, at[1] - h / 2.0 + f64::from(p[1] - min_y) * w]).collect())
        .collect();
    (!strokes.is_empty()).then(|| new(page, Shape::Signature { strokes }, String::new(), author))
}

/// Place typed text in the script font with its left edge at `at`, `height` points tall.
pub fn typed_signature_at(page: usize, at: [f64; 2], text: &str, height: f64, author: &str) -> Option<Edit> {
    pdfcraft_engine::typed_signature_shape(at, text, height).map(|shape| new(page, shape, String::new(), author))
}

/// Place a signature picture with its left edge at `at`: 150 pt wide (initials 70 pt), at
/// most 60 pt tall (initials 40 pt).
pub fn picture_signature_at(page: usize, at: [f64; 2], png: &Arc<Vec<u8>>, initials: bool, author: &str) -> Option<Edit> {
    let (pw, ph) = png_size(png)?;
    let (mut w, max_h) = if initials { (70.0, 40.0) } else { (150.0, 60.0) };
    let mut h = w * f64::from(ph) / f64::from(pw);
    if h > max_h {
        w *= max_h / h;
        h = max_h;
    }
    let what = if initials { "Initials" } else { "Signature" };
    Some(Edit::AddCustomStamp {
        page,
        rect: [at[0], at[1] - h / 2.0, at[0] + w, at[1] + h / 2.0],
        name: what.to_string(),
        file: pdfcraft_engine::MarkFile { name: format!("{}.png", what.to_lowercase()), bytes: png.clone(), page: 0 },
        author: author.to_string(),
    })
}

/// Place a saved signature or initials on a page shown turned by `rotation` degrees
/// clockwise (its `/Rotate`): laid out as for an upright page, then turned about `at` so it
/// reads upright as the page is shown (the engine turns stamp appearances to match).
pub fn place_on(page: usize, at: [f64; 2], sig: &SavedSig, initials: bool, author: &str, rotation: u16) -> Option<Edit> {
    let edit = place(page, at, sig, initials, author)?;
    if rotation.is_multiple_of(360) {
        return Some(edit);
    }
    // Counterclockwise by the page's angle, about `at`.
    let (s, c) = match rotation % 360 {
        90 => (1.0, 0.0),
        180 => (0.0, -1.0),
        _ => (-1.0, 0.0),
    };
    let turn = |p: [f64; 2]| {
        let (dx, dy) = (p[0] - at[0], p[1] - at[1]);
        [at[0] + c * dx - s * dy, at[1] + s * dx + c * dy]
    };
    let turn_rect = |r: [f64; 4]| {
        let (a, b) = (turn([r[0], r[1]]), turn([r[2], r[3]]));
        [a[0].min(b[0]), a[1].min(b[1]), a[0].max(b[0]), a[1].max(b[1])]
    };
    Some(match edit {
        Edit::AddAnnotation(mut a) => {
            match &mut a.shape {
                Shape::Signature { strokes } => {
                    for p in strokes.iter_mut().flatten() {
                        *p = turn(*p);
                    }
                }
                Shape::TypedSignature { rect, .. } => *rect = turn_rect(*rect),
                _ => {}
            }
            Edit::AddAnnotation(a)
        }
        Edit::AddCustomStamp { page, rect, name, file, author } => Edit::AddCustomStamp { page, rect: turn_rect(rect), name, file, author },
        other => other,
    })
}

/// Place a saved signature or initials (on an upright page; see [`place_on`]).
pub fn place(page: usize, at: [f64; 2], sig: &SavedSig, initials: bool, author: &str) -> Option<Edit> {
    match sig {
        SavedSig::Drawn(strokes) => signature_at(page, at, strokes, author),
        SavedSig::Image(png) => picture_signature_at(page, at, png, initials, author),
        SavedSig::Typed(text) => {
            let [left, bottom, right, top] = pdfcraft_engine::script_outline(text).bounds();
            let height = if initials { 24.0_f64 } else { 32.0_f64 };
            // Keep long names within the same placement width as drawn signatures.
            let height = height.min(150.0 * (top - bottom).max(0.1) / (right - left).max(0.01));
            typed_signature_at(page, at, text, height, author)
        }
    }
}

/// The text in the script font as a picture (`w`×`h` px, black on transparent), for previews.
pub(crate) fn script_preview(text: &str, w: usize, h: usize) -> egui::ColorImage {
    let o = pdfcraft_engine::script_outline(text);
    let mut img = egui::ColorImage::filled([w, h], Color32::TRANSPARENT);
    let [left, bottom, right, top] = o.bounds();
    let span = (top - bottom).max(0.1);
    let width = (right - left).max(0.01);
    if o.contours.is_empty() {
        return img;
    }
    let k = ((h as f64 * 0.9) / span).min((w as f64 * 0.95) / width);
    let x0 = (w as f64 - width * k) / 2.0;
    // Device points (y down), then an even-odd scanline fill.
    let polys: Vec<Vec<(f64, f64)>> = o
        .contours
        .iter()
        .map(|c| c.iter().map(|p| (x0 + (p[0] - left) * k, h as f64 * 0.5 + (top + bottom) / 2.0 * k - p[1] * k)).collect())
        .collect();
    for y in 0..h {
        let sy = y as f64 + 0.5;
        let mut xs: Vec<f64> = Vec::new();
        for poly in &polys {
            for i in 0..poly.len() {
                let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
                if (a.1 <= sy) != (b.1 <= sy) {
                    xs.push(a.0 + (sy - a.1) / (b.1 - a.1) * (b.0 - a.0));
                }
            }
        }
        xs.sort_by(|a, b| a.total_cmp(b));
        for pair in xs.as_chunks::<2>().0 {
            let (from, to) = (pair[0].round().clamp(0.0, w as f64) as usize, pair[1].round().clamp(0.0, w as f64) as usize);
            for x in from..to {
                img[(x, y)] = Color32::BLACK;
            }
        }
    }
    img
}

/// Saved previews and add/remove controls, shared by the left panel and quick-tool picker.
pub(crate) fn signature_entries(ui: &mut egui::Ui, app: &mut crate::PdfCraftApp, t: &Tokens) -> Option<&'static str> {
    ui.set_width(ui.available_width().clamp(240.0, 248.0));
    let mut command = None;
    for (i, (saved, what, use_id, change_id, remove_id)) in [
        (app.signature.as_ref(), "signature", "sign.fill.signature", "sign.fill.signature.change", "sign.fill.signature.remove"),
        (app.initials.as_ref(), "initials", "sign.fill.initials", "sign.fill.initials.change", "sign.fill.initials.remove"),
    ]
    .into_iter()
    .enumerate()
    {
        let label = |template: &str| crate::i18n::fmt(tl!(template), &[("what", tl!(what))]);
        let Some(saved) = saved else {
            if crate::widgets::ghost_button(ui, "plus", &label("Add {what}")).clicked() {
                command = Some(use_id);
            }
            continue;
        };
        ui.horizontal(|ui| {
            let (rect, response) = ui.allocate_exact_size(vec2((ui.available_width() - 68.0).max(140.0), 52.0), Sense::click());
            let use_label = label("Use {what}");
            response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, use_label.clone()));
            let painter = ui.painter_at(rect);
            painter.rect_filled(rect, CornerRadius::same(4), Color32::WHITE);
            painter.rect_stroke(
                rect,
                CornerRadius::same(4),
                Stroke::new(1.0, if response.hovered() { t.accent } else { t.border }),
                egui::StrokeKind::Inside,
            );
            let preview_rect = rect.shrink(8.0);
            match saved {
                SavedSig::Typed(text) => {
                    let cache = &mut app.saved_signature_previews[i];
                    if cache.as_ref().is_none_or(|(s, _)| s != text) {
                        *cache = Some((
                            text.clone(),
                            ui.ctx().load_texture(format!("saved-{what}"), script_preview(text, 480, 104), egui::TextureOptions::LINEAR),
                        ));
                    }
                    if let Some((_, tex)) = cache {
                        let size = vec2(480.0, 104.0) * (preview_rect.width() / 480.0).min(preview_rect.height() / 104.0);
                        painter.image(
                            tex.id(),
                            egui::Rect::from_center_size(preview_rect.center(), size),
                            egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                            Color32::WHITE,
                        );
                    }
                }
                SavedSig::Image(png) => {
                    let cache = &mut app.saved_signature_previews[i];
                    let key = picture_key(png);
                    if cache.as_ref().is_none_or(|(s, _)| *s != key)
                        && let Some(tex) = picture_texture(ui.ctx(), &format!("saved-{what}"), png, 480)
                    {
                        *cache = Some((key, tex));
                    }
                    if let Some((_, tex)) = cache {
                        paint_fitted(&painter, tex, preview_rect);
                    }
                }
                SavedSig::Drawn(strokes) => {
                    let bounds = strokes
                        .iter()
                        .flatten()
                        .filter(|p| p.iter().all(|v| v.is_finite()))
                        .fold(egui::Rect::NOTHING, |r, p| r.union(egui::Rect::from_min_max(pos2(p[0], p[1]), pos2(p[0], p[1]))));
                    if bounds.is_finite() {
                        let scale = (preview_rect.width() / bounds.width().max(0.01)).min(preview_rect.height() / bounds.height().max(0.01));
                        for stroke in strokes {
                            let pts = stroke
                                .iter()
                                .filter(|p| p.iter().all(|v| v.is_finite()))
                                .map(|p| preview_rect.center() + vec2(p[0] - bounds.center().x, bounds.center().y - p[1]) * scale)
                                .collect();
                            painter.add(egui::Shape::line(pts, Stroke::new(1.5, Color32::BLACK)));
                        }
                    }
                }
            }
            if response.on_hover_text(label("Place saved {what}")).clicked() {
                command = Some(use_id);
            }
            if crate::icons::button(ui, "pencil", 26.0, false, &label("Change {what}")).clicked() {
                command = Some(change_id);
            }
            if crate::icons::button(ui, "x", 26.0, false, &label("Remove saved {what}")).clicked() {
                command = Some(remove_id);
            }
        });
        ui.add_space(4.0);
    }
    command
}

/// Clicks with a Fill & Sign tool on one page. Returns `true` when the click was used.
#[allow(clippy::too_many_arguments)]
pub(crate) fn page_input(
    ui: &egui::Ui,
    resp: &egui::Response,
    xf: &PageXform,
    page: usize,
    info: &DocInfo,
    tool: FillTool,
    view: &mut DocView,
    signature: Option<&SavedSig>,
    initials: Option<&SavedSig>,
    author: &str,
    today: (i64, u32, u32),
    picture: Option<&egui::TextureHandle>,
) -> Option<FillAction> {
    let pointer = ui.input(|i| i.pointer.hover_pos())?;
    if !xf.rect.contains(pointer) {
        return None;
    }
    ui.ctx().set_cursor_icon(if tool == FillTool::Text { egui::CursorIcon::Text } else { egui::CursorIcon::Crosshair });
    // The signature or initials as they will be placed, under the cursor.
    let saved = match tool {
        FillTool::Signature => signature.map(|s| (s, false)),
        FillTool::Initials => initials.map(|s| (s, true)),
        _ => None,
    };
    if let Some((s, init)) = saved
        && !resp.clicked()
        && let Some(edit) = place_on(page, to_user(xf, info, page, pointer), s, init, author, info.pages[page].rotation)
    {
        paint_ghost(ui, xf, info, page, &edit, picture);
    }
    if !resp.clicked() {
        return None;
    }
    let at = to_user(xf, info, page, pointer);
    match tool {
        FillTool::Text => {
            view.fill_text = Some(TypeBox { page, at: [at[0], at[1] + TEXT_SIZE * 0.6], text: String::new(), focus: true });
            None
        }
        FillTool::Date => {
            let (y, m, d) = today;
            Some(FillAction::Edit(Box::new(typed(page, [at[0], at[1] + TEXT_SIZE * 0.6], &format!("{m}/{d}/{y}"), author))))
        }
        FillTool::Signature => match signature {
            Some(s) => place_on(page, at, s, false, author, info.pages[page].rotation).map(|e| FillAction::Edit(Box::new(e))),
            None => Some(FillAction::CreateSignature),
        },
        FillTool::Initials => match initials {
            Some(s) => place_on(page, at, s, true, author, info.pages[page].rotation).map(|e| FillAction::Edit(Box::new(e))),
            None => Some(FillAction::CreateInitials),
        },
        mark => {
            let mark = mark.mark()?;
            let (w, h) = if mark == FillMark::Line { (36.0, 4.0) } else { (12.0, 12.0) };
            let rect = [at[0] - w / 2.0, at[1] - h / 2.0, at[0] + w / 2.0, at[1] + h / 2.0];
            Some(FillAction::Edit(Box::new(new(page, Shape::Mark { rect, mark }, String::new(), author))))
        }
    }
}

/// What a Fill & Sign click asks the app to do.
#[derive(Clone, Debug, PartialEq)]
pub enum FillAction {
    Edit(Box<Edit>),
    /// No signature yet: open the signature pad.
    CreateSignature,
    /// No initials yet.
    CreateInitials,
}

/// The in-place editor for typed text. Returns the edit once committed.
pub(crate) fn type_box(ctx: &egui::Context, view: &mut DocView, info: &DocInfo, author: &str) -> Option<Edit> {
    let tb = view.fill_text.clone()?;
    let xf = view.page_xform(tb.page)?;
    let v = info.pages.get(tb.page)?.user_to_view(tb.at[0] as f32, tb.at[1] as f32);
    let pos = xf.norm_to_screen(v[0] / xf.pw, v[1] / xf.ph);
    let zoom = xf.rect.width() / xf.pw.max(1.0);
    let mut commit = false;
    let mut cancel = false;
    egui::Area::new(egui::Id::new(("fill-text", view.id.0))).order(egui::Order::Foreground).fixed_pos(pos).show(ctx, |ui| {
        let Some(t) = view.fill_text.as_mut() else { return };
        let width = ((t.text.len().max(8) as f32) * TEXT_SIZE as f32 * 0.6 * zoom).clamp(60.0, 600.0);
        let r = ui.add(
            egui::TextEdit::singleline(&mut t.text)
                .font(egui::FontId::proportional((TEXT_SIZE as f32 * zoom).max(8.0)))
                .desired_width(width)
                .background_color(Color32::from_rgba_unmultiplied(255, 255, 255, 230))
                .text_color(Color32::BLACK)
                .hint_text(tl!("Type text"))
                .id_salt("fill-text-edit"),
        );
        if t.focus {
            r.request_focus();
            t.focus = false;
        }
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            cancel = true;
        } else if r.lost_focus() {
            commit = true;
        }
    });
    if cancel {
        view.fill_text = None;
        return None;
    }
    if commit {
        let tb = view.fill_text.take()?;
        if !tb.text.trim().is_empty() {
            return Some(typed(tb.page, tb.at, tb.text.trim(), author));
        }
    }
    None
}

/// The signature pad: draw with the pointer; returns the strokes (normalised) on Apply.
pub(crate) fn signature_pad(ui: &mut egui::Ui, t: &Tokens, d: &mut SigDraft, preview: &mut Option<(String, egui::TextureHandle)>) -> (bool, bool) {
    let what = if d.initials { tl!("initials") } else { tl!("signature") };
    let title = if d.editing { tl!("Change {what}") } else { tl!("Create {what}") };
    ui.label(egui::RichText::new(crate::i18n::fmt(title, &[("what", what)])).font(crate::theme::semibold(18.0)));
    ui.horizontal(|ui| {
        // "Type" as a verb (type your name), not "Type" as in kind.
        let type_tab = crate::i18n::tr_ctx(crate::i18n::current(), "signature", "Type");
        if crate::widgets::pill_button(ui, type_tab, !d.drawing && !d.picture).clicked() {
            (d.drawing, d.picture) = (false, false);
        }
        if crate::widgets::pill_button(ui, tl!("Draw"), d.drawing && !d.picture).clicked() {
            (d.drawing, d.picture) = (true, false);
        }
        if crate::widgets::pill_button(ui, tl!("Image"), d.picture).clicked() {
            d.picture = true;
        }
    });
    ui.add_space(6.0);
    if d.picture {
        ui.label(
            egui::RichText::new(crate::i18n::fmt(
                tl!("A photo or scan of your {what} on white paper, or a PNG with transparency: the paper becomes transparent."),
                &[("what", what)],
            ))
            .color(t.text_muted),
        );
        let (rect, _) = ui.allocate_exact_size(vec2(460.0, 150.0), Sense::hover());
        let painter = ui.painter_at(rect);
        // A light check pattern shows what is transparent.
        painter.rect_filled(rect, CornerRadius::same(6), Color32::WHITE);
        painter.rect_stroke(rect, CornerRadius::same(6), Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
        match &d.image {
            Some(png) => {
                let key = picture_key(png);
                if preview.as_ref().is_none_or(|(s, _)| *s != key)
                    && let Some(tex) = picture_texture(ui.ctx(), "picture-signature", png, 920)
                {
                    *preview = Some((key, tex));
                }
                if let Some((_, tex)) = preview {
                    paint_fitted(&painter, tex, rect.shrink(10.0));
                }
            }
            None => {
                painter.text(rect.center(), egui::Align2::CENTER_CENTER, tl!("No picture chosen"), crate::theme::regular(13.0), t.text_faint);
            }
        }
        ui.add_space(6.0);
        if ui.button(tl!("Choose picture…")).clicked() {
            d.pick = true;
        }
        return pad_buttons(ui, d);
    }
    if !d.drawing {
        let l = ui.label(egui::RichText::new(crate::i18n::fmt(tl!("Type your {what}."), &[("what", what)])).color(t.text_muted));
        ui.add(
            egui::TextEdit::singleline(&mut d.text)
                .char_limit(pdfcraft_engine::MAX_SIGNATURE_CHARS)
                .desired_width(460.0)
                .hint_text(tl!(if d.initials { "Initials" } else { "Your name" })),
        )
        .labelled_by(l.id);
        let (rect, _) = ui.allocate_exact_size(vec2(460.0, 150.0), Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, CornerRadius::same(6), Color32::WHITE);
        painter.rect_stroke(rect, CornerRadius::same(6), Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
        if preview.as_ref().is_none_or(|(s, _)| *s != d.text) {
            let img = script_preview(&d.text, 920, 300);
            *preview = Some((d.text.clone(), ui.ctx().load_texture("typed-signature", img, egui::TextureOptions::LINEAR)));
        }
        if let Some((_, tex)) = preview {
            painter.image(tex.id(), rect.shrink(4.0), egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
        return pad_buttons(ui, d);
    }
    ui.label(egui::RichText::new(crate::i18n::fmt(tl!("Draw your {what} below."), &[("what", what)])).color(t.text_muted));
    let strokes = &mut d.strokes;
    let (rect, resp) = ui.allocate_exact_size(vec2(460.0, 150.0), Sense::drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, CornerRadius::same(6), Color32::WHITE);
    painter.rect_stroke(rect, CornerRadius::same(6), Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
    painter.hline(rect.x_range().shrink(24.0), rect.bottom() - 34.0, Stroke::new(1.0, t.divider));
    // Normalised: x 0–1 across the pad, y up, in units of the pad's width.
    let norm = |p: Pos2| [(p.x - rect.left()) / rect.width(), (rect.bottom() - p.y) / rect.width()];
    if resp.drag_started() {
        strokes.push(Vec::new());
    }
    if resp.dragged()
        && let (Some(p), Some(s)) = (resp.interact_pointer_pos(), strokes.last_mut())
    {
        let n = norm(p.clamp(rect.min, rect.max));
        if s.last().is_none_or(|l| (l[0] - n[0]).abs() + (l[1] - n[1]).abs() > 0.002) {
            s.push(n);
        }
    }
    for s in strokes.iter() {
        let pts: Vec<Pos2> = s.iter().map(|p| pos2(rect.left() + p[0] * rect.width(), rect.bottom() - p[1] * rect.width())).collect();
        painter.add(egui::Shape::line(pts, Stroke::new(2.0, Color32::BLACK)));
    }
    pad_buttons(ui, d)
}

/// A key for a picture's preview cache.
pub(crate) fn picture_key(bytes: &[u8]) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut h);
    format!("picture-{}-{:x}", bytes.len(), h.finish())
}

/// A picture as a texture at most `side` pixels on its longer side.
pub(crate) fn picture_texture(ctx: &egui::Context, name: &str, bytes: &[u8], side: u32) -> Option<egui::TextureHandle> {
    let (w, h, rgba) = pdfcraft_engine::preview_rgba(bytes, side)?;
    let img = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba);
    Some(ctx.load_texture(name, img, egui::TextureOptions::LINEAR))
}

/// Draw `tex` as large as fits in `rect`, centred, keeping its proportions.
pub(crate) fn paint_fitted(painter: &egui::Painter, tex: &egui::TextureHandle, rect: egui::Rect) {
    let [w, h] = tex.size();
    let k = (rect.width() / (w.max(1) as f32)).min(rect.height() / (h.max(1) as f32));
    let size = vec2(w as f32, h as f32) * k;
    painter.image(
        tex.id(),
        egui::Rect::from_center_size(rect.center(), size),
        egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
        Color32::WHITE,
    );
}

/// How an edit that places a signature, initials or stamp will look, drawn faintly where it
/// will go (under the cursor while a placing tool is chosen). `picture` is the picture of an
/// image signature or custom stamp.
pub(crate) fn paint_ghost(ui: &egui::Ui, xf: &PageXform, info: &DocInfo, page: usize, edit: &Edit, picture: Option<&egui::TextureHandle>) {
    let fade = 0.6;
    let painter = ui.painter_at(xf.rect);
    let rect_of = |r: [f64; 4]| xf.user_rect(info, page, [r[0] as f32, r[1] as f32, r[2] as f32, r[3] as f32]);
    let point = |p: [f64; 2]| rect_of([p[0], p[1], p[0], p[1]]).min;
    match edit {
        Edit::AddAnnotation(a) => match &a.shape {
            Shape::Signature { strokes } => {
                let k = rect_of([0.0, 0.0, 1.0, 1.0]).width().max(0.5);
                for s in strokes {
                    painter.add(egui::Shape::line(
                        s.iter().map(|p| point(*p)).collect(),
                        Stroke::new((1.5 * k).max(1.0), Color32::BLACK.gamma_multiply(fade)),
                    ));
                }
            }
            Shape::TypedSignature { rect, contours } => {
                let r = rect_of(*rect);
                for c in contours {
                    let pts: Vec<Pos2> = c.iter().map(|p| pos2(r.left() + p[0] as f32 * r.width(), r.bottom() - p[1] as f32 * r.height())).collect();
                    painter.add(egui::Shape::closed_line(pts, Stroke::new(1.2, Color32::BLACK.gamma_multiply(fade))));
                }
            }
            Shape::Stamp { rect, stamp, .. } => {
                let r = rect_of(*rect);
                let [cr, cg, cb] = stamp.color();
                let c = Color32::from_rgb((cr * 255.0) as u8, (cg * 255.0) as u8, (cb * 255.0) as u8).gamma_multiply(fade);
                painter.rect_filled(r, CornerRadius::same(4), Color32::WHITE.gamma_multiply(0.35));
                painter.rect_stroke(r, CornerRadius::same(4), Stroke::new(2.0, c), egui::StrokeKind::Inside);
                let size = (r.height() * 0.45).clamp(6.0, 64.0);
                painter.text(r.center(), egui::Align2::CENTER_CENTER, stamp.label(), egui::FontId::proportional(size), c);
            }
            _ => {}
        },
        Edit::AddCustomStamp { rect, name, .. } => {
            let r = rect_of(*rect);
            match picture {
                Some(tex) => {
                    painter.image(tex.id(), r, egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE.gamma_multiply(fade));
                }
                None => {
                    let c = Color32::from_rgb(0x1a, 0x4d, 0xb3).gamma_multiply(fade);
                    painter.rect_stroke(r, CornerRadius::same(4), Stroke::new(1.5, c), egui::StrokeKind::Inside);
                    painter.text(r.center(), egui::Align2::CENTER_CENTER, name, egui::FontId::proportional(13.0), c);
                }
            }
        }
        _ => {}
    }
}

fn pad_buttons(ui: &mut egui::Ui, d: &mut SigDraft) -> (bool, bool) {
    ui.add_space(10.0);
    let (mut apply, mut cancel) = (false, false);
    ui.horizontal(|ui| {
        if ui.button(tl!("Clear")).clicked() {
            d.strokes.clear();
            d.text.clear();
            d.image = None;
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let ready = d.ready();
            if ui.add_enabled_ui(ready, |ui| crate::widgets::pill_button(ui, tl!("Apply"), true)).inner.clicked() {
                apply = true;
            }
            if crate::widgets::pill_button(ui, tl!("Cancel"), false).clicked() {
                cancel = true;
            }
        });
    });
    (apply, cancel)
}

#[cfg(test)]
mod tests {
    #[test]
    fn long_signature_previews_have_clear_margins() {
        for text in ["Alexandria Catherine Elizabeth Montgomery-Wellington", "Jg Jg Jg Jg Jg Jg Jg Jg Jg Jg Jg"] {
            for [w, h] in [[920, 300], [480, 104]] {
                let image = super::script_preview(text, w, h);
                assert!(image.pixels.iter().any(|p| p.a() > 0));
                for x in 0..w {
                    assert_eq!(image[(x, 0)].a(), 0);
                    assert_eq!(image[(x, h - 1)].a(), 0);
                }
                for y in 0..h {
                    assert_eq!(image[(0, y)].a(), 0);
                    assert_eq!(image[(w - 1, y)].a(), 0);
                }
            }
        }
    }
}
