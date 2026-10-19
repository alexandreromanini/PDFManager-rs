//! Commentaires (annotations), texte et images ajoutés, en-têtes / pieds de page, filigrane.
//! Sans interface. Coordonnées : points, repère de la page affichée (comme `pdf::Rect`) ;
//! MuPDF les ramène lui-même dans le repère PDF, rotation /Rotate comprise. Chaque fonction
//! qui modifie le document est une étape annulable (`Pdf::edit`).

use mupdf::color::AnnotationColor;
use mupdf::pdf::{LineEndingStyle, PdfAnnotation, PdfAnnotationType as T, PdfDocument, PdfObject, PdfPage};
use mupdf::shape::{PdfColor, Shape, TextOptions};
use mupdf::{Font, Matrix, Point, Quad};

use crate::pdf::{Pdf, Rect, Word};

pub type Color = (f32, f32, f32);

/// « Ajouter du texte » : texte libre sans cadre marqué « machine à écrire » (/IT), comme le
/// texte de « Remplir et signer » d'Acrobat (et de la version Python).
const TYPEWRITER: &str = "FreeTextTypeWriter";

/// Tampons standard : nom PDF, libellé. MuPDF dessine le libellé anglais (« APPROVED »).
pub const STAMPS: &[(&str, &str)] = &[
    ("Approved", "Approuvé"), ("NotApproved", "Non approuvé"), ("Draft", "Brouillon"),
    ("Final", "Final"), ("Confidential", "Confidentiel"), ("ForComment", "Pour commentaire"),
    ("Expired", "Expiré"), ("TopSecret", "Top secret"),
];

#[derive(Debug, Clone, PartialEq)]
pub struct AnnotInfo {
    pub page: usize,
    pub xref: i32,
    pub kind: &'static str,
    pub author: String,
    pub content: String,
    pub modified: String, // « jj/mm/aaaa hh:mm » ou ""
    pub rect: Rect,
}

impl AnnotInfo {
    pub fn label(&self) -> &'static str {
        label(self.kind)
    }
    pub fn movable(&self) -> bool {
        movable(self.kind)
    }
    pub fn is_text(&self) -> bool {
        matches!(self.kind, "FreeText" | "Typewriter")
    }
}

fn movable(kind: &str) -> bool {
    matches!(kind, "Text" | "FreeText" | "Square" | "Circle" | "Stamp" | "Typewriter")
}

pub fn label(kind: &str) -> &'static str {
    match kind {
        "Highlight" => "Surlignage",
        "Underline" => "Soulignement",
        "StrikeOut" => "Texte barré",
        "Text" => "Note",
        "FreeText" => "Texte libre",
        "Square" => "Rectangle",
        "Circle" => "Ellipse",
        "Line" => "Ligne",
        "Ink" => "Dessin",
        "Stamp" => "Tampon",
        "Typewriter" => "Texte ajouté",
        "Redact" => "Caviardage à appliquer",
        _ => "Commentaire",
    }
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn rgb((red, green, blue): Color) -> AnnotationColor {
    AnnotationColor::Rgb { red, green, blue }
}

fn pt(x: f32, y: f32) -> Point {
    Point::new(x, y)
}

fn mrect(r: Rect) -> mupdf::Rect {
    mupdf::Rect::new(r.x0, r.y0, r.x1, r.y1)
}

fn find(p: &PdfPage, xref: i32) -> Result<PdfAnnotation, String> {
    p.annotations().find(|a| a.xref().ok() == Some(xref)).ok_or_else(|| "Commentaire introuvable.".into())
}

/// Date PDF « D:AAAAMMJJHHmmSS+hh'mm' » (heure locale).
fn pdf_now() -> String {
    let now = chrono::Local::now();
    let off = now.offset().local_minus_utc() / 60;
    format!("D:{}{}{:02}'{:02}'", now.format("%Y%m%d%H%M%S"), if off < 0 { '-' } else { '+' },
            off.abs() / 60, off.abs() % 60)
}

/// « D:20261015103100+02'00' » → « 15/10/2026 10:31 ».
fn date_text(d: &str) -> String {
    let d = d.trim_start_matches("D:");
    if d.len() < 12 || !d.is_char_boundary(12) {
        return String::new();
    }
    format!("{}/{}/{} {}:{}", &d[6..8], &d[4..6], &d[0..4], &d[8..10], &d[10..12])
}

/// Auteur et date de modification (et de création pour une nouvelle annotation).
fn stamp(a: &mut PdfAnnotation, author: &str, created: bool) -> Result<(), String> {
    a.set_author(author).map_err(err)?;
    let mut obj = a.object();
    if created {
        obj.dict_put("CreationDate", PdfObject::new_string(&pdf_now()).map_err(err)?).map_err(err)?;
    }
    obj.dict_put("M", PdfObject::new_string(&pdf_now()).map_err(err)?).map_err(err)?;
    Ok(())
}

fn kind_of(a: &PdfAnnotation) -> Option<&'static str> {
    Some(match a.r#type().ok()? {
        T::Highlight => "Highlight",
        T::Underline => "Underline",
        T::StrikeOut => "StrikeOut",
        T::Text => "Text",
        T::FreeText => {
            let it = a.object().get_dict("IT").ok().flatten().and_then(|o| o.as_name().ok().map(|n| n.to_vec()));
            if it.as_deref() == Some(TYPEWRITER.as_bytes()) { "Typewriter" } else { "FreeText" }
        }
        T::Square => "Square",
        T::Circle => "Circle",
        T::Line => "Line",
        T::Ink => "Ink",
        T::Stamp => "Stamp",
        T::Redact => "Redact",
        T::Polygon | T::PolyLine | T::Squiggly | T::Caret | T::FileAttachment => "Other",
        _ => return None, // liens, champs de formulaire, fenêtres de note…
    })
}

/// Crée une annotation sur `page` puis la complète (`style`) ; renvoie son numéro d'objet.
fn add(pdf: &mut Pdf, label: &str, page: usize, author: &str,
       create: impl FnOnce(&mut PdfPage) -> Result<PdfAnnotation, mupdf::Error>,
       style: impl FnOnce(&mut PdfAnnotation, &PdfPage) -> Result<(), mupdf::Error>) -> Result<i32, String> {
    let mut xref = 0;
    pdf.edit(label, |doc| {
        let mut p = doc.load_pdf_page(page as i32).map_err(err)?;
        let mut a = create(&mut p).map_err(err)?;
        style(&mut a, &p).map_err(err)?;
        stamp(&mut a, author, true)?;
        a.update().map_err(err)?;
        xref = a.xref().map_err(err)?;
        Ok(())
    })?;
    Ok(xref)
}

/// Un rectangle par ligne de texte (union des mots), dans l'ordre de lecture.
pub fn line_rects(words: &[Word]) -> Vec<Rect> {
    let mut out: Vec<((usize, usize), Rect)> = vec![];
    for w in words {
        match out.iter_mut().find(|(k, _)| *k == (w.block, w.line)) {
            Some((_, r)) => *r = Rect::new(r.x0.min(w.rect.x0), r.y0.min(w.rect.y0), r.x1.max(w.rect.x1), r.y1.max(w.rect.y1)),
            None => out.push(((w.block, w.line), w.rect)),
        }
    }
    out.into_iter().map(|(_, r)| r).collect()
}

/// Surlignage ("Highlight"), soulignement ("Underline") ou texte barré ("StrikeOut").
pub fn add_markup(pdf: &mut Pdf, page: usize, kind: &str, words: &[Word], color: Color, author: &str)
                  -> Result<i32, String> {
    let quads: Vec<Quad> = line_rects(words).into_iter().map(|r| Quad::from(mrect(r))).collect();
    if quads.is_empty() {
        return Err("Sélectionnez d'abord du texte.".into());
    }
    let verb = match kind { "Highlight" => "Surligner", "Underline" => "Souligner", _ => "Barrer" };
    let kind = kind.to_string();
    add(pdf, verb, page, author, |p| match kind.as_str() {
        "Highlight" => p.add_highlight_annotation(quads),
        "Underline" => p.add_underline_annotation(quads),
        _ => p.add_strikeout_annotation(quads),
    }, |a, _| a.set_color(rgb(color)))
}

pub fn add_note(pdf: &mut Pdf, page: usize, x: f32, y: f32, text: &str, color: Color, author: &str)
                -> Result<i32, String> {
    add(pdf, "Note", page, author, |p| p.add_text_annotation(mupdf::Rect::new(x, y, x + 20.0, y + 20.0), text),
        |a, _| {
            a.set_icon_name("Comment")?;
            a.set_color(rgb(color))
        })
}

/// `kind` : "rect", "ellipse", "line" ou "arrow".
pub fn add_shape(pdf: &mut Pdf, page: usize, kind: &str, a: (f32, f32), b: (f32, f32), color: Color,
                 width: f32, author: &str) -> Result<i32, String> {
    let label = match kind { "rect" => "Rectangle", "ellipse" => "Ellipse", "line" => "Ligne", _ => "Flèche" };
    let r = mrect(Rect::new(a.0, a.1, b.0, b.1));
    let kind = kind.to_string();
    add(pdf, label, page, author, |p| match kind.as_str() {
        "rect" => p.add_rect_annotation(r),
        "ellipse" => p.add_circle_annotation(r),
        _ => p.add_line_annotation(pt(a.0, a.1), pt(b.0, b.1)),
    }, |an, _| {
        if label == "Flèche" {
            an.set_line_ending_styles(LineEndingStyle::None, LineEndingStyle::OpenArrow)?;
        }
        an.set_color(rgb(color))?;
        an.set_border_width(width)
    })
}

pub fn add_ink(pdf: &mut Pdf, page: usize, points: &[(f32, f32)], color: Color, width: f32, author: &str)
               -> Result<i32, String> {
    if points.len() < 2 {
        return Err("Tracé trop court.".into());
    }
    let stroke: Vec<Point> = points.iter().map(|&(x, y)| pt(x, y)).collect();
    add(pdf, "Dessin", page, author, |p| p.add_ink_annotation([stroke]), |a, _| {
        a.set_color(rgb(color))?;
        a.set_border_width(width)
    })
}

pub fn add_stamp(pdf: &mut Pdf, page: usize, rect: Rect, name: &str, author: &str) -> Result<i32, String> {
    let name = name.to_string();
    add(pdf, "Tampon", page, author, |p| p.add_stamp_annotation(mrect(rect), &name), |_, _| Ok(()))
}

/// Texte ajouté dans le cadre `rect` (retour à la ligne automatique), droit à l'écran.
pub fn add_typewriter(pdf: &mut Pdf, page: usize, rect: Rect, text: &str, color: Color, size: f32,
                      author: &str) -> Result<i32, String> {
    add_text(pdf, "Ajouter du texte", page, rect, text, color, size, author, true)
}

/// Commentaire « texte libre » : texte encadré, dans la liste des commentaires.
pub fn add_freetext(pdf: &mut Pdf, page: usize, rect: Rect, text: &str, color: Color, size: f32,
                    author: &str) -> Result<i32, String> {
    add_text(pdf, "Texte libre", page, rect, text, color, size, author, false)
}

#[allow(clippy::too_many_arguments)]
fn add_text(pdf: &mut Pdf, label: &str, page: usize, rect: Rect, text: &str, color: Color, size: f32,
            author: &str, typewriter: bool) -> Result<i32, String> {
    add(pdf, label, page, author, |p| p.add_free_text_annotation(mrect(rect), text), |a, p| {
        a.set_default_appearance("Helv", size, Some(rgb(color)))?;
        a.set_border_width(if typewriter { 0.0 } else { 1.0 })?;
        let mut obj = a.object();
        if typewriter {
            obj.dict_put("IT", PdfObject::new_name(TYPEWRITER)?)?;
        }
        obj.dict_put("Rotate", PdfObject::new_int(p.rotation()?)?)?; // droit sur une page tournée
        Ok(())
    })
}

// ----- liste, recherche, modifications ------------------------------------------------------

pub fn list(pdf: &Pdf) -> Vec<AnnotInfo> {
    let mut out = vec![];
    for i in 0..pdf.page_count() {
        let Ok(p) = pdf.doc().load_pdf_page(i as i32) else { continue };
        for a in p.annotations() {
            let Some(kind) = kind_of(&a) else { continue };
            let date = a.object().get_dict("M").ok().flatten()
                .and_then(|o| o.as_string().ok()).unwrap_or_default();
            out.push(AnnotInfo {
                page: i,
                xref: a.xref().unwrap_or(0),
                kind,
                author: a.author().ok().flatten().unwrap_or_default().to_string(),
                content: a.contents().ok().flatten().unwrap_or_default().to_string(),
                modified: date_text(&date),
                rect: a.bounds().map(Rect::from).unwrap_or_default(),
            });
        }
    }
    out
}

/// Modifie l'annotation `xref` de `page` dans une étape annulable.
fn change(pdf: &mut Pdf, label: &str, page: usize, xref: i32, author: &str,
          f: impl FnOnce(&mut PdfAnnotation, &'static str) -> Result<(), String>) -> Result<(), String> {
    pdf.edit(label, |doc| {
        let p = doc.load_pdf_page(page as i32).map_err(err)?;
        let mut a = find(&p, xref)?;
        let kind = kind_of(&a).unwrap_or("Other");
        f(&mut a, kind)?;
        stamp(&mut a, author, false)?;
        a.update().map_err(err)?;
        Ok(())
    })
}

pub fn set_content(pdf: &mut Pdf, page: usize, xref: i32, text: &str, author: &str) -> Result<(), String> {
    change(pdf, "Modifier le commentaire", page, xref, author, |a, _| a.set_contents(text).map_err(err))
}

/// Texte, taille et couleur d'un texte libre (pour l'éditer sur place).
pub fn text_style(pdf: &Pdf, page: usize, xref: i32) -> Option<(String, f32, Color)> {
    let p = pdf.doc().load_pdf_page(page as i32).ok()?;
    let a = find(&p, xref).ok()?;
    let da = a.default_appearance().ok().flatten();
    let color = match da.as_ref().and_then(|d| d.color) {
        Some(AnnotationColor::Rgb { red, green, blue }) => (red, green, blue),
        Some(AnnotationColor::Gray(g)) => (g, g, g),
        _ => (0.0, 0.0, 0.0),
    };
    Some((a.contents().ok().flatten().unwrap_or_default().to_string(), da.map_or(12.0, |d| d.size), color))
}

pub fn set_color(pdf: &mut Pdf, page: usize, xref: i32, color: Color, author: &str) -> Result<(), String> {
    let size = text_style(pdf, page, xref).map_or(12.0, |s| s.1);
    change(pdf, "Changer la couleur", page, xref, author, |a, kind| {
        if matches!(kind, "FreeText" | "Typewriter") {
            a.set_default_appearance("Helv", size, Some(rgb(color))).map_err(err)
        } else {
            a.set_color(rgb(color)).map_err(err)
        }
    })
}

/// Taille d'un texte ajouté ; le cadre suit, les retours à la ligne restent les mêmes.
pub fn set_text_size(pdf: &mut Pdf, page: usize, xref: i32, size: f32, author: &str) -> Result<(), String> {
    let (_, old, color) = text_style(pdf, page, xref).ok_or("Texte introuvable.")?;
    change(pdf, "Taille du texte", page, xref, author, |a, _| {
        let r = a.rect().map_err(err)?;
        let k = size / old;
        a.set_rect(mupdf::Rect::new(r.x0, r.y0, r.x0 + (r.x1 - r.x0) * k, r.y0 + (r.y1 - r.y0) * k)).map_err(err)?;
        a.set_default_appearance("Helv", size, Some(rgb(color))).map_err(err)
    })
}

/// Nouveau texte et, si donné, nouveau cadre (agrandi pendant la saisie).
pub fn update_text(pdf: &mut Pdf, page: usize, xref: i32, text: &str, rect: Option<Rect>, author: &str)
                   -> Result<(), String> {
    change(pdf, "Modifier le texte ajouté", page, xref, author, |a, _| {
        a.set_contents(text).map_err(err)?;
        if let Some(r) = rect {
            a.set_rect(mrect(r)).map_err(err)?;
        }
        Ok(())
    })
}

pub fn move_by(pdf: &mut Pdf, page: usize, xref: i32, dx: f32, dy: f32, author: &str) -> Result<(), String> {
    change(pdf, "Déplacer le commentaire", page, xref, author, |a, kind| {
        if !movable(kind) {
            return Err(format!("Un(e) {} ne se déplace pas.", label(kind).to_lowercase()));
        }
        let r = a.rect().map_err(err)?;
        a.set_rect(mupdf::Rect::new(r.x0 + dx, r.y0 + dy, r.x1 + dx, r.y1 + dy)).map_err(err)
    })
}

pub fn delete(pdf: &mut Pdf, page: usize, xref: i32) -> Result<(), String> {
    pdf.edit("Supprimer le commentaire", |doc| {
        let mut p = doc.load_pdf_page(page as i32).map_err(err)?;
        let a = find(&p, xref)?;
        p.delete_annotation(a).map_err(err)
    })
}

// ----- image, en-têtes et pieds de page, filigrane (dans le contenu des pages) ---------------

/// Image ajustée au rectangle (proportions conservées), centrée.
pub fn add_image(pdf: &mut Pdf, page: usize, rect: Rect, data: &[u8]) -> Result<(), String> {
    let img = mupdf::Image::from_bytes(data).map_err(|e| format!("Image illisible : {e}"))?;
    let (w, h) = (img.width() as f32, img.height() as f32);
    let k = (rect.width() / w).min(rect.height() / h);
    let (cx, cy) = ((rect.x0 + rect.x1) / 2.0, (rect.y0 + rect.y1) / 2.0);
    let fit = Rect::new(cx - w * k / 2.0, cy - h * k / 2.0, cx + w * k / 2.0, cy + h * k / 2.0);
    pdf.edit("Ajouter une image", |doc| {
        let mut p = doc.load_pdf_page(page as i32).map_err(err)?;
        // insert_image attend le repère PDF : on y ramène le cadre affiché.
        let inv = p.ctm().map_err(err)?.invert().ok_or("transformation de page invalide")?;
        let target = mrect(fit).transform(&inv);
        let src = mupdf::pdf::PageImageSource::Bytes { data, format_hint: None };
        p.insert_image(doc, target, src, Default::default()).map_err(err)?;
        Ok(())
    })
}

/// Largeur du texte en Helvetica (points).
pub fn text_width(text: &str, size: f32) -> f32 {
    let Ok(font) = Font::new("Helvetica") else { return text.chars().count() as f32 * size * 0.5 };
    text.chars()
        .map(|c| font.encode_character(c as i32).and_then(|g| font.advance_glyph(g)).unwrap_or(0.5))
        .sum::<f32>() * size
}

/// Champs : {n} numéro, {total} nombre de pages, {date}, {fichier}.
pub fn expand(template: &str, n: usize, total: usize, file: &str) -> String {
    template.replace("{n}", &n.to_string()).replace("{total}", &total.to_string())
        .replace("{date}", &chrono::Local::now().format("%d/%m/%Y").to_string())
        .replace("{fichier}", file)
}

/// Positions : "tl", "tc", "tr" (haut), "bl", "bc", "br" (bas).
pub const POSITIONS: [&str; 6] = ["tl", "tc", "tr", "bl", "bc", "br"];

/// Écrit `text` dans le contenu de la page, droit à l'écran, en `(x, y)` (ligne de base,
/// repère affiché). `turn` : inclinaison en degrés autour de `center` (filigrane).
#[allow(clippy::too_many_arguments)]
fn write_text(doc: &mut PdfDocument, p: &mut PdfPage, x: f32, y: f32, text: &str, size: f32, color: Color,
              opacity: Option<f32>, turn: Option<(f32, (f32, f32))>) -> Result<(), String> {
    let rotation = p.rotation().map_err(err)?;
    let ctm = p.ctm().map_err(err)?;
    let opts = TextOptions {
        fontsize: size,
        fill: Some(PdfColor::rgb(color.0, color.1, color.2)),
        rotate: rotation,
        fill_opacity: opacity,
        ..Default::default()
    };
    let mut shape = Shape::new(p).map_err(err)?;
    shape.insert_text(pt(x, y), text, &opts).map_err(err)?;
    let block = shape.text_cont().to_string();
    drop(shape);
    let cm = match turn {
        None => String::new(),
        Some((deg, (cx, cy))) => {
            // Rotation autour du centre, dans le repère PDF (même sens visuel qu'à l'écran).
            let inv = ctm.invert().ok_or("transformation de page invalide")?;
            let c = pt(cx, cy).transform(&inv);
            let (s, k) = deg.to_radians().sin_cos();
            let m = Matrix::new(k, s, -s, k, c.x - k * c.x + s * c.y, c.y - s * c.x - k * c.y);
            format!("{} {} {} {} {} {} cm\n", m.a, m.b, m.c, m.d, m.e, m.f)
        }
    };
    // Contenu existant entouré de q/Q : son état graphique ne déborde pas sur le nôtre.
    p.wrap_contents(doc).map_err(err)?;
    p.insert_contents(doc, format!("q\n{cm}{block}Q\n").as_bytes(), true).map_err(err)?;
    Ok(())
}

/// `texts` : (position, modèle). La première page traitée porte le numéro `start`.
#[allow(clippy::too_many_arguments)]
pub fn header_footer(pdf: &mut Pdf, pages: &[usize], texts: &[(&str, String)], size: f32, margin: f32,
                     start: usize, color: Color, file: &str) -> Result<(), String> {
    if texts.iter().all(|(_, t)| t.trim().is_empty()) {
        return Err("Saisissez au moins un texte.".into());
    }
    let mut pages = pages.to_vec();
    pages.sort_unstable();
    pages.dedup();
    let total = start + pages.len() - 1;
    let sizes = pdf.page_sizes().to_vec();
    pdf.edit("En-tête et pied de page", |doc| {
        for (k, &i) in pages.iter().enumerate() {
            let mut p = doc.load_pdf_page(i as i32).map_err(err)?;
            let (w, h) = sizes[i];
            for (pos, tpl) in texts {
                let s = expand(tpl, start + k, total, file);
                if s.trim().is_empty() {
                    continue;
                }
                let tw = text_width(&s, size);
                let x = match &pos[1..] { "l" => margin, "c" => (w - tw) / 2.0, _ => w - margin - tw };
                let y = if pos.starts_with('t') { margin + size } else { h - margin };
                write_text(doc, &mut p, x, y, &s, size, color, None, None)?;
            }
        }
        Ok(())
    })
}

/// Texte centré, incliné de `angle` degrés (montant vers la droite), semi-transparent.
/// Réduit automatiquement s'il dépasse de la page.
pub fn watermark(pdf: &mut Pdf, pages: &[usize], text: &str, size: f32, color: Color, opacity: f32,
                 angle: f32) -> Result<(), String> {
    if text.trim().is_empty() {
        return Err("Le texte du filigrane est vide.".into());
    }
    let sizes = pdf.page_sizes().to_vec();
    pdf.edit("Filigrane", |doc| {
        for &i in pages {
            let mut p = doc.load_pdf_page(i as i32).map_err(err)?;
            let (w, h) = sizes[i];
            let fs = size.min(size * 0.85 * (w * w + h * h).sqrt() / text_width(text, size));
            let tw = text_width(text, fs);
            let c = (w / 2.0, h / 2.0);
            write_text(doc, &mut p, c.0 - tw / 2.0, c.1 + fs * 0.35, text, fs, color, Some(opacity),
                       Some((angle, c)))?;
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(rotation: i32) -> Pdf {
        let mut d = PdfDocument::new();
        let mut p = d.new_page(mupdf::Size::new(595.0, 842.0)).unwrap();
        p.set_rotation(rotation).unwrap();
        let mut out = vec![];
        d.write_to(&mut out).unwrap();
        Pdf::open(&out).unwrap()
    }

    /// Le texte écrit est lisible et horizontal à l'écran (pas tourné avec la page).
    fn assert_upright(pdf: &Pdf, needle: &str) -> Rect {
        let ws = pdf.words(0);
        let w = ws.iter().find(|w| w.text.contains(needle)).unwrap_or_else(|| panic!("{needle} absent : {ws:?}"));
        assert!(w.rect.width() > w.rect.height(), "{needle} pas horizontal : {:?}", w.rect);
        w.rect
    }

    #[test]
    fn commentaires_crees_listes_modifies_supprimes() {
        let mut pdf = doc(0);
        let words = vec![Word { text: "x".into(), rect: Rect::new(50.0, 50.0, 150.0, 62.0), block: 0, line: 0 }];
        add_markup(&mut pdf, 0, "Highlight", &words, (1.0, 0.8, 0.0), "Alex").unwrap();
        let note = add_note(&mut pdf, 0, 300.0, 300.0, "À revoir", (1.0, 0.0, 0.0), "Alex").unwrap();
        add_shape(&mut pdf, 0, "arrow", (100.0, 400.0), (200.0, 450.0), (0.0, 0.0, 1.0), 2.0, "Alex").unwrap();
        add_ink(&mut pdf, 0, &[(10.0, 10.0), (20.0, 30.0), (40.0, 35.0)], (0.0, 0.0, 0.0), 1.5, "Alex").unwrap();
        add_stamp(&mut pdf, 0, Rect::new(300.0, 600.0, 450.0, 650.0), "Approved", "Alex").unwrap();
        let all = list(&pdf);
        let kinds: Vec<_> = all.iter().map(|a| a.kind).collect();
        assert_eq!(kinds, ["Highlight", "Text", "Line", "Ink", "Stamp"]);
        let n = all.iter().find(|a| a.xref == note).unwrap();
        assert_eq!((n.author.as_str(), n.content.as_str()), ("Alex", "À revoir"));
        assert_eq!(n.modified.len(), 16, "date : {}", n.modified);
        assert!(n.rect.contains(305.0, 305.0), "{:?}", n.rect);

        set_content(&mut pdf, 0, note, "Vu", "Alex").unwrap();
        move_by(&mut pdf, 0, note, 100.0, 0.0, "Alex").unwrap();
        let n = list(&pdf).into_iter().find(|a| a.xref == note).unwrap();
        assert_eq!(n.content, "Vu");
        assert!(n.rect.contains(405.0, 305.0), "{:?}", n.rect);
        let line = all.iter().find(|a| a.kind == "Line").unwrap().xref;
        assert!(move_by(&mut pdf, 0, line, 5.0, 5.0, "Alex").is_err());

        delete(&mut pdf, 0, note).unwrap();
        assert_eq!(list(&pdf).len(), 4);
        pdf.undo().unwrap();
        assert_eq!(list(&pdf).len(), 5, "Ctrl+Z rend le commentaire");
    }

    /// Cadre des pixels de la couleur donnée (canal dominant) sur le rendu à 100 %.
    fn ink_box(pdf: &Pdf, channel: usize) -> Rect {
        let r = pdf.render(0, 1.0, 0).unwrap();
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, 0f32, 0f32);
        for (i, px) in r.rgb.chunks(3).enumerate() {
            if px[channel] > 150 && px.iter().enumerate().all(|(k, &v)| k == channel || v < 100) {
                let (x, y) = ((i as u32 % r.width) as f32, (i as u32 / r.width) as f32);
                (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
            }
        }
        Rect::new(x0, y0, x1, y1)
    }

    #[test]
    fn texte_ajoute_droit_meme_sur_page_tournee() {
        for rot in [0, 90] {
            let mut pdf = doc(rot);
            let x = add_typewriter(&mut pdf, 0, Rect::new(100.0, 100.0, 300.0, 130.0), "Bonjour été",
                                   (1.0, 0.0, 0.0), 14.0, "Alex").unwrap();
            let a = list(&pdf).into_iter().find(|a| a.xref == x).unwrap();
            assert_eq!(a.kind, "Typewriter");
            let ink = ink_box(&pdf, 0);
            assert!(ink.width() > 2.0 * ink.height(), "rot {rot} : texte pas horizontal {ink:?}");
            assert!(ink.x0 >= 98.0 && ink.y0 >= 98.0 && ink.y1 <= 132.0, "rot {rot} : hors du cadre {ink:?}");
            assert_eq!(text_style(&pdf, 0, x).map(|s| (s.0, s.1)), Some(("Bonjour été".to_string(), 14.0)));
            set_text_size(&mut pdf, 0, x, 28.0, "Alex").unwrap();
            set_color(&mut pdf, 0, x, (0.0, 0.0, 1.0), "Alex").unwrap();
            let (_, size, color) = text_style(&pdf, 0, x).unwrap();
            assert_eq!((size, color), (28.0, (0.0, 0.0, 1.0)));
            let ink = ink_box(&pdf, 2);
            assert!(ink.width() > 2.0 * ink.height() && ink.height() > 15.0, "rot {rot} : 28 pt bleu {ink:?}");
            update_text(&mut pdf, 0, x, "Au revoir", None, "Alex").unwrap();
            assert_eq!(text_style(&pdf, 0, x).unwrap().0, "Au revoir");
        }
    }

    #[test]
    fn en_tetes_et_filigrane_droits_et_places() {
        for rot in [0, 90, 180, 270] {
            let mut pdf = doc(rot);
            let (w, h) = pdf.page_sizes()[0];
            let texts = [("tl", "Haut {fichier}".to_string()), ("br", "Page {n} sur {total}".to_string())];
            header_footer(&mut pdf, &[0], &texts, 10.0, 28.0, 3, (0.0, 0.0, 0.0), "Été").unwrap();
            let top = assert_upright(&pdf, "Haut");
            assert!(top.x0 < 40.0 && top.y1 < 45.0, "rot {rot} : en-tête en {top:?}");
            assert!(pdf.words(0).iter().any(|w| w.text == "Été"), "accents : {:?}", pdf.words(0));
            let bottom = assert_upright(&pdf, "Page");
            assert!(bottom.y0 > h - 45.0 && bottom.x0 > w / 2.0, "rot {rot} : pied en {bottom:?}");
            assert!(pdf.words(0).iter().any(|w| w.text == "3"));

            watermark(&mut pdf, &[0], "CONFIDENTIEL", 72.0, (0.6, 0.6, 0.6), 0.3, 45.0).unwrap();
            let mark = pdf.words(0).into_iter().find(|w| w.text == "CONFIDENTIEL").expect("filigrane");
            let (cx, cy) = ((mark.rect.x0 + mark.rect.x1) / 2.0, (mark.rect.y0 + mark.rect.y1) / 2.0);
            assert!((cx - w / 2.0).abs() < 30.0 && (cy - h / 2.0).abs() < 30.0, "rot {rot} : centre {cx},{cy}");
            assert!(mark.rect.height() > 100.0, "incliné : {:?}", mark.rect);
        }
        assert_eq!(expand("{n}/{total} {fichier}", 2, 9, "a"), "2/9 a");
    }

    #[test]
    fn image_ajoutee_dans_le_cadre() {
        let mut pdf = doc(90);
        let png = include_bytes!("../tests/fixtures/image.png");
        add_image(&mut pdf, 0, Rect::new(100.0, 100.0, 300.0, 400.0), png).unwrap();
        let page = pdf.doc().load_pdf_page(0).unwrap();
        assert_eq!(page.images().unwrap().len(), 1);
        assert!(pdf.render(0, 0.5, 0).is_ok());
    }
}
