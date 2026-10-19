//! Contrôleur, partie commentaires et ajouts : outils (surligner, note, formes, dessin,
//! tampon, texte ajouté, image), sélection / déplacement / suppression d'un commentaire,
//! éditeur de texte sur la page, liste des commentaires, en-têtes et filigrane.

use slint::{ModelRc, SharedString, VecModel};

use super::{from_engine, to_view, App, Tab};
use crate::annot::{self, AnnotInfo, Color};
use crate::pdf::{Pdf, Rect};
use crate::{edit, CommentData};

/// Mêmes pastilles que la version Python (commentaires, puis texte ajouté).
pub const SWATCHES: [u32; 6] = [0xFFD60A, 0x7BD389, 0x64B5F6, 0xF48FB1, 0xE53935, 0x1B1B1B];
pub const TEXT_SWATCHES: [u32; 6] = [0x000000, 0xE53935, 0x1E88E5, 0x43A047, 0xFB8C00, 0x8E24AA];

const MARKUP: [&str; 3] = ["highlight", "underline", "strikeout"];

fn color(hex: u32) -> Color {
    (((hex >> 16) & 255) as f32 / 255.0, ((hex >> 8) & 255) as f32 / 255.0, (hex & 255) as f32 / 255.0)
}

/// Geste en cours sur une page (coordonnées en points, repère de la page).
pub(super) enum Gesture {
    Draw { page: usize, a: (f32, f32), b: (f32, f32) },
    Ink { page: usize, points: Vec<(f32, f32)> },
    Move { page: usize, xref: i32, rect: Rect, a: (f32, f32), b: (f32, f32) },
}

/// Texte ajouté en cours d'écriture sur la page ; `xref` : texte existant modifié.
#[derive(Clone)]
pub(super) struct Editor {
    pub(super) page: usize,
    pub(super) rect: Rect,
    pub(super) xref: Option<i32>,
    pub(super) size: f32,
    pub(super) color: Color,
    /// Champ de formulaire en cours de remplissage.
    pub(super) field: Option<crate::forms::FieldInfo>,
}

/// Ce que valide la fenêtre « Note ».
pub(super) enum NoteTarget {
    Note(usize, (f32, f32)),
    FreeText(usize, Rect),
    Edit(usize, i32),
    Combo(usize, Rect),
}

impl App {
    fn author(&self) -> String {
        let a = self.settings.author.trim();
        if !a.is_empty() {
            return a.to_string();
        }
        std::env::var("USERNAME").or_else(|_| std::env::var("USER")).unwrap_or_else(|_| "Utilisateur".into())
    }

    pub fn set_author(&mut self, author: String) {
        self.settings.author = author;
        self.settings.save();
    }

    pub fn set_tool(&mut self, tool: &str) {
        self.editor_commit();
        self.tool = tool.to_string();
        let ui = self.ui();
        ui.set_tool(tool.into());
        ui.set_drawing(!tool.is_empty() && !MARKUP.contains(&tool));
        if let Some(tab) = self.tab() {
            tab.gesture = None;
            if !tool.is_empty() {
                tab.picked = None;
            }
        }
        let hint = match tool {
            t if MARKUP.contains(&t) => "Sélectionnez le texte à marquer.",
            "note" => "Cliquez à l'endroit de la note.",
            "freetext" => "Tracez le cadre du texte (ou cliquez).",
            "rect" | "ellipse" | "line" | "arrow" => "Tracez la forme sur la page.",
            "ink" => "Dessinez à main levée sur la page.",
            "stamp" => "Cliquez ou tracez l'emplacement du tampon.",
            "text" => "Cliquez ou tracez une bulle, puis écrivez (cliquez à côté pour valider, Échap pour annuler).",
            "image" => "Tracez le cadre de l'image (ou cliquez).",
            t if t.starts_with("field-") => "Tracez le cadre du champ (ou cliquez). Cliquer un champ existant propose de le supprimer.",
            _ => "",
        };
        if !hint.is_empty() {
            self.toast(hint);
        }
        self.sync_picked();
        self.update_view();
    }

    pub fn pick_color(&mut self, text: bool, i: usize) {
        let ui = self.ui();
        if text { ui.set_text_color_index(i as i32) } else { ui.set_color_index(i as i32) }
        let editing = self.tab().and_then(|t| t.editor.as_mut()).map(|e| {
            e.color = color(TEXT_SWATCHES[i]);
        });
        if editing.is_some() && text {
            self.update_view();
            return;
        }
        let Some(a) = self.tab().and_then(|t| t.picked.clone()) else { return };
        if a.is_text() == text {
            let c = color(if text { TEXT_SWATCHES[i] } else { SWATCHES[i] });
            let author = self.author();
            self.annot_edit(move |pdf| annot::set_color(pdf, a.page, a.xref, c, &author).map(|_| 0));
        }
    }

    pub fn text_size_changed(&mut self, size: i32) {
        let size = size.clamp(4, 200) as f32;
        if let Some(e) = self.tab().and_then(|t| t.editor.as_mut()) {
            e.size = size;
            self.update_view();
            return;
        }
        let Some(a) = self.tab().and_then(|t| t.picked.clone()).filter(|a| a.is_text()) else { return };
        let author = self.author();
        self.annot_edit(move |pdf| annot::set_text_size(pdf, a.page, a.xref, size, &author).map(|_| 0));
    }

    fn comment_color(&self) -> Color {
        color(SWATCHES[self.ui().get_color_index().clamp(0, 5) as usize])
    }

    fn text_color(&self) -> Color {
        color(TEXT_SWATCHES[self.ui().get_text_color_index().clamp(0, 5) as usize])
    }

    /// Modification de commentaire : la sélection de pages est gardée.
    fn annot_edit(&mut self, op: impl FnOnce(&mut Pdf) -> Result<i32, String> + Send + 'static) {
        let keep = self.tab().map(|t| t.selected.clone()).unwrap_or_default();
        self.edit("", move |pdf| op(pdf).map(|_| Some(keep)));
    }

    // ----- gestes sur la page ---------------------------------------------------------------

    fn hit(tab: &Tab, page: usize, pt: (f32, f32)) -> Option<AnnotInfo> {
        let tol = 4.0 / tab.zoom.max(0.1);
        tab.comments.iter().rev().find(|a| {
            a.page == page && Rect::new(a.rect.x0 - tol, a.rect.y0 - tol, a.rect.x1 + tol, a.rect.y1 + tol).contains(pt.0, pt.1)
        }).cloned()
    }

    /// Appui sur une page ; `true` : géré ici (sinon sélection de texte).
    pub(super) fn tool_press(&mut self, page: usize, pt: (f32, f32)) -> bool {
        if self.tab().is_some_and(|t| t.editor.is_some()) {
            self.editor_commit(); // cliquer à côté valide le texte
            return true;
        }
        if self.field_press(page, pt) {
            return true;
        }
        let tool = self.tool.clone();
        let Some(tab) = self.tab() else { return false };
        let handled = match tool.as_str() {
            "" => match Self::hit(tab, page, pt) {
                Some(a) => {
                    if a.movable() {
                        tab.gesture = Some(Gesture::Move { page, xref: a.xref, rect: a.rect, a: pt, b: pt });
                    }
                    tab.picked = Some(a);
                    true
                }
                None => {
                    tab.picked = None;
                    false
                }
            },
            t if MARKUP.contains(&t) => false,
            "note" => {
                self.pending_note = Some(NoteTarget::Note(page, pt));
                self.show_note("Nouvelle note", "");
                return true;
            }
            "ink" => {
                tab.gesture = Some(Gesture::Ink { page, points: vec![pt] });
                true
            }
            "text" if Self::hit(tab, page, pt).is_some_and(|a| a.kind == "Typewriter") => {
                let a = Self::hit(tab, page, pt).unwrap();
                self.edit_text(a);
                return true;
            }
            _ => {
                tab.gesture = Some(Gesture::Draw { page, a: pt, b: pt });
                true
            }
        };
        self.sync_picked();
        self.update_view();
        handled
    }

    pub(super) fn tool_drag(&mut self, page: usize, pt: (f32, f32)) -> bool {
        let Some(tab) = self.tab() else { return false };
        let (w, h) = tab.sizes[page];
        let pt = (pt.0.clamp(0.0, w), pt.1.clamp(0.0, h));
        match &mut tab.gesture {
            Some(Gesture::Draw { page: p, b, .. } | Gesture::Move { page: p, b, .. }) if *p == page => *b = pt,
            Some(Gesture::Ink { page: p, points }) if *p == page => {
                let last = *points.last().unwrap();
                if (last.0 - pt.0).abs() + (last.1 - pt.1).abs() > 1.0 {
                    points.push(pt);
                }
            }
            Some(_) => return true,
            None => return false,
        }
        self.update_view();
        true
    }

    pub(super) fn tool_release(&mut self, page: usize) -> bool {
        let tool = self.tool.clone();
        let (color, text_color, author) = (self.comment_color(), self.text_color(), self.author());
        let stroke = self.ui().get_stroke_width().clamp(1, 20) as f32;
        let size = self.ui().get_text_size().clamp(4, 200) as f32;
        let stamp = annot::STAMPS[self.ui().get_stamp_index().clamp(0, annot::STAMPS.len() as i32 - 1) as usize].0;
        let Some(tab) = self.tab() else { return false };
        if MARKUP.contains(&tool.as_str()) {
            let Some((p, words)) = tab.selection.take().filter(|(p, _)| *p == page) else {
                tab.press = None;
                return false;
            };
            tab.press = None;
            let kind = match tool.as_str() { "highlight" => "Highlight", "underline" => "Underline", _ => "StrikeOut" };
            self.annot_edit(move |pdf| annot::add_markup(pdf, p, kind, &words, color, &author));
            return true;
        }
        let Some(gesture) = tab.gesture.take() else { return false };
        let small = |a: (f32, f32), b: (f32, f32)| (a.0 - b.0).abs() < 5.0 && (a.1 - b.1).abs() < 5.0;
        let around = |a: (f32, f32), w: f32, h: f32| Rect::new(a.0 - w / 2.0, a.1 - h / 2.0, a.0 + w / 2.0, a.1 + h / 2.0);
        match gesture {
            Gesture::Move { page, xref, a, b, .. } => {
                if !small(a, b) {
                    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
                    self.annot_edit(move |pdf| annot::move_by(pdf, page, xref, dx, dy, &author).map(|_| 0));
                }
            }
            Gesture::Ink { page, points } => {
                if points.len() >= 2 {
                    self.annot_edit(move |pdf| annot::add_ink(pdf, page, &points, color, stroke, &author));
                }
            }
            Gesture::Draw { page, a, b } => {
                let r = Rect::new(a.0, a.1, b.0, b.1);
                match tool.as_str() {
                    t if t.starts_with("field-") => {
                        self.field_release(page, a, b);
                    }
                    "rect" | "ellipse" | "line" | "arrow" => {
                        if !small(a, b) {
                            self.annot_edit(move |pdf| annot::add_shape(pdf, page, &tool, a, b, color, stroke, &author));
                        }
                    }
                    "stamp" => {
                        let r = if small(a, b) { around(a, 160.0, 50.0) } else { r };
                        self.annot_edit(move |pdf| annot::add_stamp(pdf, page, r, stamp, &author));
                    }
                    "image" => {
                        let r = if small(a, b) { around(a, 200.0, 150.0) } else { r };
                        if let Some(data) = self.pending_image.take() {
                            self.annot_edit(move |pdf| annot::add_image(pdf, page, r, &data).map(|_| 0));
                        }
                        self.set_tool("");
                    }
                    "freetext" => {
                        let r = if small(a, b) { Rect::new(a.0, a.1, a.0 + 200.0, a.1 + 60.0) } else { r };
                        self.pending_note = Some(NoteTarget::FreeText(page, r));
                        self.show_note("Texte libre", "");
                    }
                    "text" => {
                        let r = if small(a, b) { Rect::new(a.0, a.1 - size * 0.6, a.0 + 150.0, a.1 + size * 0.8) } else { r };
                        if let Some(tab) = self.tab() {
                            tab.editor = Some(Editor { page, rect: r, xref: None, size, color: text_color, field: None });
                        }
                        self.ui().set_editor_text("".into());
                    }
                    _ => {}
                }
            }
        }
        self.update_view();
        true
    }

    /// Double-clic : modifier un texte ajouté sur place, ou le texte d'une note.
    pub fn page_double(&mut self, page: usize, x: f32, y: f32) {
        let Some(pt) = self.page_point(page, x, y) else { return };
        let Some(a) = self.tab().and_then(|t| Self::hit(t, page, pt)) else { return };
        match a.kind {
            "Typewriter" => self.edit_text(a),
            _ => {
                self.pending_note = Some(NoteTarget::Edit(a.page, a.xref));
                self.show_note(&format!("{} (page {})", a.label(), a.page + 1), &a.content);
            }
        }
    }

    fn edit_text(&mut self, a: AnnotInfo) {
        let Some(tab) = self.tab() else { return };
        let id = tab.id;
        self.engine.run(move |docs| {
            let style = docs.get(&id).and_then(|pdf| annot::text_style(pdf, a.page, a.xref));
            from_engine(move |app| {
                let Some((text, size, color)) = style else { return };
                let Some(tab) = app.tabs.iter_mut().find(|t| t.id == id) else { return };
                tab.editor = Some(Editor { page: a.page, rect: a.rect, xref: Some(a.xref), size, color, field: None });
                tab.picked = None;
                let ui = app.ui();
                ui.set_editor_text(text.into());
                ui.set_text_size(size.round() as i32);
                app.update_view();
            });
        });
    }

    pub fn editor_commit(&mut self) {
        let text = self.ui().get_editor_text().to_string();
        let author = self.author();
        let Some(e) = self.tab().and_then(|t| t.editor.take()) else { return };
        self.ui().set_editor_visible(false);
        self.ui().set_editor_single(false);
        if let Some(f) = e.field {
            return self.field_commit(f, text.trim_end_matches(['\n', '\r']).to_string());
        }
        let text = text.trim_end().to_string();
        if text.trim().is_empty() {
            if let Some(xref) = e.xref {
                self.annot_edit(move |pdf| annot::delete(pdf, e.page, xref).map(|_| 0));
            }
            self.update_view();
            return;
        }
        // Le cadre suit le texte : pas de retour à la ligne imprévu, rien de coupé.
        let longest = text.lines().map(|l| annot::text_width(l, e.size)).fold(0.0, f32::max);
        let lines = text.lines().count().max(1) as f32;
        let r = Rect::new(e.rect.x0, e.rect.y0, e.rect.x0 + e.rect.width().max(longest + 8.0),
                          e.rect.y0 + lines * e.size * 1.2 + 6.0);
        match e.xref {
            None => self.annot_edit(move |pdf| annot::add_typewriter(pdf, e.page, r, &text, e.color, e.size, &author)),
            Some(xref) => self.annot_edit(move |pdf| {
                annot::update_text(pdf, e.page, xref, &text, Some(r), &author)?;
                annot::set_color(pdf, e.page, xref, e.color, &author)?;
                annot::set_text_size(pdf, e.page, xref, e.size, &author).map(|_| 0)
            }),
        }
    }

    pub fn escape(&mut self) {
        if self.crop_mode {
            return self.action("crop-cancel");
        }
        let tool_on = !self.tool.is_empty();
        let Some(tab) = self.tab() else { return };
        if tab.editor.take().is_none() && tab.gesture.take().is_none() {
            if tool_on {
                return self.set_tool("");
            }
            tab.picked = None;
        }
        self.sync_picked();
        self.update_view();
    }

    /// Suppr : supprime le commentaire sélectionné, s'il y en a un.
    pub(super) fn delete_picked(&mut self) -> bool {
        let Some(a) = self.tab().and_then(|t| t.picked.take()) else { return false };
        self.annot_edit(move |pdf| annot::delete(pdf, a.page, a.xref).map(|_| 0));
        true
    }

    // ----- note, liste des commentaires -------------------------------------------------------

    pub(super) fn show_note(&mut self, title: &str, text: &str) {
        let ui = self.ui();
        ui.set_note_title(title.into());
        ui.set_note_text(text.into());
        ui.invoke_show_note();
    }

    pub fn note_done(&mut self, text: String) {
        let Some(target) = self.pending_note.take() else { return };
        let (color, author, size) = (self.comment_color(), self.author(), self.ui().get_text_size() as f32);
        match target {
            NoteTarget::Note(page, (x, y)) => {
                self.annot_edit(move |pdf| annot::add_note(pdf, page, x, y, &text, color, &author));
            }
            NoteTarget::FreeText(page, r) if !text.trim().is_empty() => {
                self.annot_edit(move |pdf| annot::add_freetext(pdf, page, r, &text, (0.0, 0.0, 0.0), size, &author));
            }
            NoteTarget::Combo(page, r) => self.add_combo(page, r, &text),
            NoteTarget::Edit(page, xref) => {
                self.annot_edit(move |pdf| annot::set_content(pdf, page, xref, &text, &author).map(|_| 0));
            }
            _ => {}
        }
    }

    /// Liste du rail et sélection : à jour avec l'onglet courant.
    pub(super) fn sync_comments(&mut self) {
        let ui = self.ui();
        let rows: Vec<CommentData> = self.tab().map(|t| {
            t.comments.iter().map(|a| CommentData {
                page: a.page as i32 + 1,
                label: a.label().into(),
                author: a.author.clone().into(),
                date: a.modified.clone().into(),
                content: a.content.lines().next().unwrap_or("").into(),
                selected: t.picked.as_ref().is_some_and(|p| p.xref == a.xref && p.page == a.page),
            }).collect()
        }).unwrap_or_default();
        ui.set_comments(ModelRc::new(VecModel::from(rows)));
    }

    fn sync_picked(&mut self) {
        let text = self.tab().and_then(|t| t.picked.clone());
        let info: SharedString = match text {
            Some(a) => format!("{} sélectionné(e) - Suppr pour le supprimer, glisser pour le déplacer.", a.label()).into(),
            None => "".into(),
        };
        self.ui().set_picked_info(info);
        self.sync_comments();
    }

    pub fn comment_clicked(&mut self, i: usize) {
        let Some(a) = self.tab().and_then(|t| t.comments.get(i).cloned()) else { return };
        self.go_to(a.page);
        if let Some(tab) = self.tab() {
            tab.picked = Some(a);
        }
        self.sync_picked();
        self.update_view();
    }

    pub fn comment_edit(&mut self, i: usize) {
        let Some(a) = self.tab().and_then(|t| t.comments.get(i).cloned()) else { return };
        self.pending_note = Some(NoteTarget::Edit(a.page, a.xref));
        self.show_note(&format!("{} (page {})", a.label(), a.page + 1), &a.content);
    }

    pub fn comment_delete(&mut self, i: usize) {
        let Some(a) = self.tab().and_then(|t| t.comments.get(i).cloned()) else { return };
        self.annot_edit(move |pdf| annot::delete(pdf, a.page, a.xref).map(|_| 0));
    }

    /// Après une modification : liste des commentaires, commentaire sélectionné.
    pub(super) fn comments_changed(tab: &mut Tab, comments: Vec<AnnotInfo>) {
        tab.picked = tab.picked.take()
            .and_then(|p| comments.iter().find(|a| a.page == p.page && a.xref == p.xref).cloned());
        tab.comments = comments;
        tab.gesture = None;
    }

    // ----- dessin par-dessus les pages ----------------------------------------------------------

    /// Cadres (commentaire sélectionné, forme en cours) et tracé (dessin, ligne) de la page.
    pub(super) fn overlay(tab: &Tab, page: usize, line: bool) -> (Vec<Rect>, String) {
        let mut frames = vec![];
        let mut path = String::new();
        if let Some(a) = tab.picked.as_ref().filter(|a| a.page == page) {
            frames.push(a.rect);
        }
        let size = tab.sizes[page];
        let mut point = |cmd: char, (x, y): (f32, f32)| {
            let v = to_view(&Rect::new(x, y, x, y), size, tab.rotation, tab.zoom);
            path.push_str(&format!("{cmd} {} {} ", v.x, v.y));
        };
        match &tab.gesture {
            Some(Gesture::Draw { page: p, a, b }) if *p == page => {
                if line {
                    point('M', *a);
                    point('L', *b);
                } else {
                    frames.push(Rect::new(a.0, a.1, b.0, b.1));
                }
            }
            Some(Gesture::Move { page: p, rect, a, b, .. }) if *p == page => {
                let (dx, dy) = (b.0 - a.0, b.1 - a.1);
                frames.push(Rect::new(rect.x0 + dx, rect.y0 + dy, rect.x1 + dx, rect.y1 + dy));
            }
            Some(Gesture::Ink { page: p, points }) if *p == page => {
                for (k, &q) in points.iter().enumerate() {
                    point(if k == 0 { 'M' } else { 'L' }, q);
                }
            }
            _ => {}
        }
        (frames, path)
    }

    /// Éditeur de texte : position sur la vue (pixels, repère du contenu défilant).
    pub(super) fn sync_editor(&mut self) {
        let ui = self.ui();
        let Some(tab) = self.tab() else {
            ui.set_editor_visible(false);
            return;
        };
        let Some(e) = tab.editor.clone() else {
            ui.set_editor_visible(false);
            return;
        };
        let Some(p) = tab.layout.pages.iter().find(|p| p.index == e.page) else { return };
        let a = to_view(&e.rect, tab.sizes[e.page], tab.rotation, tab.zoom);
        ui.set_editor_x(p.x + a.x);
        ui.set_editor_y(p.y + a.y);
        ui.set_editor_w(a.w);
        ui.set_editor_h(a.h);
        ui.set_editor_font(e.size * tab.zoom);
        let (r, g, b) = e.color;
        ui.set_editor_color(slint::Color::from_rgb_u8((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8));
        ui.set_editor_visible(true);
    }

    // ----- image, en-têtes, filigrane --------------------------------------------------------------

    pub(super) fn add_image_start(&mut self) {
        if self.tab().is_none() {
            return;
        }
        let Some(path) = rfd::FileDialog::new().set_title("Ajouter une image")
            .add_filter("Images", &["png", "jpg", "jpeg", "tif", "tiff", "bmp", "gif", "webp"]).pick_file()
        else { return };
        match std::fs::read(&path) {
            Ok(data) => {
                self.pending_image = Some(data);
                self.set_tool("image");
            }
            Err(e) => self.error(&format!("Impossible de lire « {} » : {}.", super::file_name(&path), super::reason(&e))),
        }
    }

    fn pages_of(&mut self, spec: &str) -> Option<Vec<usize>> {
        let count = self.tab()?.sizes.len();
        match edit::parse_pages(spec, count) {
            Ok(p) => Some(p),
            Err(e) => {
                self.error(&e);
                None
            }
        }
    }

    pub fn header_run(&mut self, texts: [String; 6], size: i32, start: i32, pages: String) {
        let Some(pages) = self.pages_of(&pages) else { return };
        let file = self.tab().and_then(|t| t.path.file_stem().map(|s| s.to_string_lossy().into_owned())).unwrap_or_default();
        let texts: Vec<(&'static str, String)> = annot::POSITIONS.into_iter().zip(texts).collect();
        let (size, start) = (size.clamp(4, 72) as f32, start.max(0) as usize);
        let keep = self.tab().map(|t| t.selected.clone()).unwrap_or_default();
        self.edit("En-tête et pied de page ajoutés (Ctrl+Z pour annuler).", move |pdf| {
            annot::header_footer(pdf, &pages, &texts, size, 28.35, start, (0.0, 0.0, 0.0), &file).map(|_| Some(keep))
        });
    }

    pub fn watermark_run(&mut self, text: String, size: i32, opacity: i32, angle: i32, pages: String) {
        let Some(pages) = self.pages_of(&pages) else { return };
        let keep = self.tab().map(|t| t.selected.clone()).unwrap_or_default();
        let (size, opacity, angle) = (size.clamp(8, 300) as f32, opacity.clamp(5, 100) as f32 / 100.0, angle as f32);
        self.edit("Filigrane ajouté (Ctrl+Z pour annuler).", move |pdf| {
            annot::watermark(pdf, &pages, &text, size, (0.6, 0.6, 0.6), opacity, angle).map(|_| Some(keep))
        });
    }
}

