//! Modifier le texte existant d'une page, caviardage définitif, nettoyage. Sans interface.
//! Coordonnées : points, repère de la page affichée (comme `pdf::Rect`).
//!
//! Modifier un texte (comme la version Python, core/editing.py), et ses limites signalées :
//! - les lignes changées du bloc cliqué sont effacées (texte seul : images et tracés
//!   intacts), puis réécrites à la même position, même taille, même couleur ;
//! - la police d'origine n'est réutilisée que si elle est intégrée au fichier et dessine
//!   TOUS les caractères voulus (les polices en sous-ensemble n'ont que les glyphes déjà
//!   utilisés) ; sinon Helvetica / Times / Courier (gras, italique) la plus proche ; le
//!   résultat est relu, et refait avec la police de remplacement s'il ne correspond pas ;
//! - une ligne à styles mixtes prend le style de son premier caractère ;
//! - changer le nombre de lignes recompose le paragraphe (interlignage approximatif) ;
//! - texte incliné ou invisible (couche OCR) : refusé.

use mupdf::pdf::{
    PdfAnnotationType, PdfDocument, PdfObject, PdfPage, PdfRedactImageMethod, PdfRedactLineArtMethod,
    PdfRedactOptions, PdfRedactTextMethod,
};
use mupdf::shape::{PdfColor, Shape, TextOptions, TextboxOptions};
use mupdf::text_page::{TextBlockType, TextChar, TextCharFlags, TextPageFlags};
use mupdf::{Font, Point};

use crate::pdf::{Pdf, Rect};

pub type Color = (f32, f32, f32);

/// Relecture ratée : l'essai est annulé et refait avec une autre police.
const NOT_FAITHFUL: &str = "\u{0}relecture";

#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub rect: Rect,
    pub origin: (f32, f32),
    pub text: String,
    pub size: f32,
    pub font: String,
    pub bold: bool,
    pub italic: bool,
    pub serif: bool,
    pub mono: bool,
    pub color: Color,
    /// Sens d'écriture à l'écran : 0, 90 (vers le haut), 180, 270 (vers le bas).
    pub angle: i32,
    /// Plusieurs polices / tailles / couleurs dans la ligne.
    pub mixed: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextBlock {
    pub page: usize,
    pub rect: Rect,
    pub lines: Vec<Line>,
}

impl TextBlock {
    pub fn text(&self) -> String {
        self.lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n")
    }
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Sens d'écriture (vecteur unitaire, y vers le bas) → angle ; `None` : incliné.
fn direction(dx: f32, dy: f32) -> Option<i32> {
    [(0, (1.0, 0.0)), (90, (0.0, -1.0)), (180, (-1.0, 0.0)), (270, (0.0, 1.0))]
        .into_iter()
        .find(|(_, (x, y))| (dx - x).abs() < 0.02 && (dy - y).abs() < 0.02)
        .map(|(a, _)| a)
}

/// Nom comparable : sans préfixe de sous-ensemble (« ABCDEF+ »), minuscules, alphanumérique.
fn norm(name: &str) -> String {
    name.rsplit('+').next().unwrap_or(name).chars().filter(|c| c.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase).collect()
}

fn visible(c: &TextChar) -> bool {
    c.char().is_some_and(|ch| !ch.is_whitespace())
}

/// Bloc de texte sous le point. Erreur si le texte n'est pas modifiable.
pub fn text_block_at(pdf: &Pdf, page: usize, x: f32, y: f32) -> Result<Option<TextBlock>, String> {
    let p = pdf.doc().load_page(page as i32).map_err(err)?;
    let tp = p.to_text_page(TextPageFlags::empty()).map_err(err)?;
    for b in tp.blocks() {
        if b.r#type() != TextBlockType::Text {
            continue;
        }
        let br: Rect = b.bounds().into();
        if !Rect::new(br.x0 - 2.0, br.y0 - 2.0, br.x1 + 2.0, br.y1 + 2.0).contains(x, y) {
            continue;
        }
        let mut lines = vec![];
        for l in b.lines() {
            let chars: Vec<_> = l.chars().collect();
            let Some(first) = chars.iter().find(|c| visible(c)) else { continue };
            let last = chars.iter().rev().find(|c| visible(c)).unwrap();
            if chars.iter().filter(|c| visible(c)).all(|c| !c.flags().intersects(TextCharFlags::FILLED | TextCharFlags::STROKED)) {
                return Err("Texte invisible (reconnaissance de caractères d'un scan) : modification prévue avec \
                            l'OCR, en phase 6.".into());
            }
            let (o1, o2) = (first.origin(), last.origin());
            let len = ((o2.x - o1.x).powi(2) + (o2.y - o1.y).powi(2)).sqrt();
            let angle = if len < 0.5 {
                0 // un seul caractère
            } else {
                direction((o2.x - o1.x) / len, (o2.y - o1.y) / len).ok_or("Texte incliné : modification non prise en charge.")?
            };
            let font = first.font();
            let name = font.as_ref().map(|f| f.name().to_string()).unwrap_or_default();
            let style = |c: &TextChar| (c.font().map(|f| f.name().to_string()).unwrap_or_default(),
                                        (c.size() * 10.0).round() as i32, c.argb() & 0xFFFFFF);
            let s0 = style(first);
            let lower = name.to_lowercase();
            let argb = first.argb();
            lines.push(Line {
                rect: l.bounds().into(),
                origin: (o1.x, o1.y),
                text: chars.iter().filter_map(|c| c.char()).collect::<String>().trim_end().replace('\u{ad}', "-"),
                size: first.size(),
                bold: font.as_ref().is_some_and(|f| f.is_bold()) || first.flags().contains(TextCharFlags::BOLD)
                    || ["bold", "black", "heavy", "semibold"].iter().any(|k| lower.contains(k)),
                italic: font.as_ref().is_some_and(|f| f.is_italic()) || lower.contains("italic") || lower.contains("oblique"),
                serif: font.as_ref().is_some_and(|f| f.is_serif())
                    || (["times", "serif", "roman", "georgia"].iter().any(|k| lower.contains(k)) && !lower.contains("sans")),
                mono: font.as_ref().is_some_and(|f| f.is_monospaced()) || lower.contains("courier") || lower.contains("mono"),
                font: name,
                color: (((argb >> 16) & 255) as f32 / 255.0, ((argb >> 8) & 255) as f32 / 255.0, (argb & 255) as f32 / 255.0),
                angle,
                mixed: chars.iter().filter(|c| visible(c)).any(|c| style(c) != s0),
            });
        }
        if !lines.is_empty() {
            return Ok(Some(TextBlock { page, rect: br, lines }));
        }
    }
    Ok(None)
}

/// Police standard la plus proche : (nom MuPDF, libellé).
pub fn substitute(line: &Line) -> (String, String) {
    let family = if line.mono { "Courier" } else if line.serif { "Times" } else { "Helvetica" };
    let suffix = match (line.bold, line.italic, family) {
        (false, false, "Times") => "-Roman",
        (false, false, _) => "",
        (true, false, _) => "-Bold",
        (false, true, "Times") => "-Italic",
        (false, true, _) => "-Oblique",
        (true, true, "Times") => "-BoldItalic",
        (true, true, _) => "-BoldOblique",
    };
    (format!("{family}{suffix}"),
     format!("{family}{}{}", if line.bold { " gras" } else { "" }, if line.italic { " italique" } else { "" }))
}

/// Fichier de la police intégrée au PDF sous ce nom (sur la page), s'il y en a un.
fn embedded_font(p: &PdfPage, name: &str) -> Option<Vec<u8>> {
    let fonts = p.object().get_dict_inheritable("Resources").ok()??.get_dict("Font").ok()??;
    let want = norm(name);
    for i in 0..fonts.dict_len().ok()? as i32 {
        let Some(f) = fonts.get_dict_val(i).ok().flatten() else { continue };
        let base = f.get_dict("BaseFont").ok().flatten().and_then(|b| b.as_name().ok().map(|n| norm(&String::from_utf8_lossy(&n))));
        let Some(base) = base.filter(|b| !b.is_empty()) else { continue };
        if !(base.starts_with(&want) || want.starts_with(&base)) {
            continue;
        }
        let desc = match f.get_dict("DescendantFonts").ok().flatten() {
            Some(d) => d.get_array(0).ok().flatten()?.get_dict("FontDescriptor").ok().flatten(),
            None => f.get_dict("FontDescriptor").ok().flatten(),
        }?;
        return ["FontFile2", "FontFile3", "FontFile"].iter()
            .find_map(|k| desc.get_dict(*k).ok().flatten())
            .and_then(|file| file.read_stream().ok());
    }
    None
}

/// La police dessine-t-elle chaque caractère ? Une police en sous-ensemble annonce des
/// glyphes dont le dessin a été retiré : on vérifie le contour de chacun.
fn draws_all(font: &Font, text: &str) -> bool {
    text.chars().filter(|c| !c.is_whitespace()).all(|c| {
        font.encode_character(c as i32).is_ok_and(|g| g > 0 && font.outline_glyph(g).is_ok_and(|o| o.is_some()))
    })
}

fn squash(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

/// Texte de la page sous et après le bloc (relecture).
fn page_text(doc: &PdfDocument, page: usize, block: Rect) -> Result<String, String> {
    let p = doc.load_page(page as i32).map_err(err)?;
    let tp = p.to_text_page(TextPageFlags::empty()).map_err(err)?;
    let mut out = String::new();
    for b in tp.blocks() {
        for l in b.lines() {
            let r: Rect = l.bounds().into();
            if r.y1 >= block.y0 - 2.0 && r.x1 >= block.x0 - 2.0 {
                out.extend(l.chars().filter_map(|c| c.char()));
            }
        }
    }
    Ok(out)
}

/// Efface et réécrit les lignes modifiées (tout le bloc si le nombre de lignes change).
/// Renvoie le débordement éventuel ; `NOT_FAITHFUL` si la relecture ne correspond pas.
fn rewrite(pdf: &mut Pdf, block: &TextBlock, lines: &[String], fontname: &str, file: Option<&[u8]>) -> Result<bool, String> {
    let measure = match file {
        Some(f) => Font::from_bytes("police", f).map_err(err)?,
        None => Font::new(fontname).map_err(err)?,
    };
    let width = |s: &str, size: f32| -> f32 {
        s.chars().map(|c| measure.encode_character(c as i32).and_then(|g| measure.advance_glyph(g)).unwrap_or(0.5)).sum::<f32>() * size
    };
    let same = lines.len() == block.lines.len();
    let targets: Vec<usize> = (0..block.lines.len()).filter(|&k| !same || lines[k] != block.lines[k].text).collect();
    let mut overflow = false;
    pdf.edit("Modifier le texte", |doc| {
        let page_h = doc.load_page(block.page as i32).and_then(|p| p.bounds()).map_err(err)?.y1;
        let mut p = doc.load_pdf_page(block.page as i32).map_err(err)?;
        let rotation = p.rotation().map_err(err)?;
        for &k in &targets {
            let r = block.lines[k].rect;
            // Bandes resserrées : ne pas toucher les lignes voisines.
            let (dx, dy) = if block.lines[k].angle % 180 == 0 { (0.0, r.height() * 0.2) } else { (r.width() * 0.2, 0.0) };
            p.add_redact_annotation(mupdf::Rect::new(r.x0 + dx, r.y0 + dy, r.x1 - dx, r.y1 - dy)).map_err(err)?;
        }
        p.apply_redactions_with_options(PdfRedactOptions {
            black_boxes: false,
            image_method: PdfRedactImageMethod::None,
            line_art: PdfRedactLineArtMethod::None,
            text: PdfRedactTextMethod::Remove,
        }).map_err(err)?;
        // Police du fichier : en CID (Unicode) ; police standard : codage latin.
        let simple = file.is_none();
        let mut shape = Shape::new(&mut p).map_err(err)?;
        if same {
            for &k in &targets {
                let (line, s) = (&block.lines[k], &lines[k]);
                if s.is_empty() {
                    continue;
                }
                let opts = TextOptions {
                    fontsize: line.size,
                    fontname: fontname.to_string(),
                    fontfile: file,
                    simple,
                    fill: Some(PdfColor::rgb(line.color.0, line.color.1, line.color.2)),
                    rotate: (rotation + line.angle).rem_euclid(360),
                    ..Default::default()
                };
                shape.insert_text(Point::new(line.origin.0, line.origin.1), s, &opts).map_err(err)?;
                overflow |= line.angle == 0 && line.origin.0 + width(s, line.size) > block.rect.x1 + 1.0;
            }
        } else {
            let first = &block.lines[0];
            if first.angle != 0 || rotation != 0 {
                return Err("Texte tourné : gardez le même nombre de lignes.".into());
            }
            let leading = block.lines.get(1).map_or(first.size * 1.25, |l| l.origin.1 - first.origin.1).max(first.size);
            let rect = mupdf::Rect::new(block.rect.x0, first.origin.1 - first.size * 0.95,
                                        block.rect.x1.max(block.rect.x0 + 50.0), page_h);
            let opts = TextboxOptions {
                fontsize: first.size,
                lineheight: leading / first.size,
                fontname: fontname.to_string(),
                fontfile: file,
                simple,
                fill: Some(PdfColor::rgb(first.color.0, first.color.1, first.color.2)),
                ..Default::default()
            };
            let rest = shape.insert_textbox(rect, &lines.join("\n"), &opts).map_err(err)?;
            if rest < 0.0 {
                return Err("Le texte ne tient pas jusqu'au bas de la page.".into());
            }
            overflow = rect.y1 - rest > block.rect.y1 + first.size * 0.5;
        }
        shape.commit(doc, true).map_err(err)?;
        drop(p);
        // Relecture : un texte illisible (glyphes absents) fait annuler l'essai.
        let written = squash(&page_text(doc, block.page, block.rect)?);
        let wanted: Vec<String> = if same { targets.iter().map(|&k| squash(&lines[k])).collect() } else { vec![squash(&lines.concat())] };
        if wanted.iter().any(|w| !written.contains(w.as_str())) {
            return Err(NOT_FAITHFUL.into());
        }
        Ok(())
    })?;
    Ok(overflow)
}

/// Nom de police unique (« PMAAAB+Calibri ») : MuPDF garde en cache, par nom, une police
/// déjà insérée ; un autre sous-ensemble de la même police doit donc porter un autre nom.
fn unique_name(base: &str) -> String {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let mut n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tag: String = (0..6).map(|_| { let c = (b'A' + (n % 26) as u8) as char; n /= 26; c }).collect();
    let clean: String = base.rsplit('+').next().unwrap_or(base).chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
    format!("{tag}+{}", if clean.is_empty() { "Police" } else { &clean })
}

/// Remplace le texte du bloc. Renvoie les avertissements à montrer à l'utilisateur.
pub fn edit_block(pdf: &mut Pdf, block: &TextBlock, new_text: &str) -> Result<Vec<String>, String> {
    let mut lines: Vec<String> = new_text.split('\n').map(|s| s.trim_end().to_string()).collect();
    while lines.len() > 1 && lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    let same = lines.len() == block.lines.len();
    let changed: Vec<usize> = (0..block.lines.len()).filter(|&k| !same || lines[k] != block.lines[k].text).collect();
    if changed.is_empty() {
        return Ok(vec![]);
    }
    if redactions_on(pdf, block.page) {
        return Err("Des zones sont marquées pour caviardage sur cette page : appliquez-les ou supprimez-les \
                    d'abord (sinon elles seraient appliquées en même temps).".into());
    }
    let mut warnings = vec![];
    let first = block.lines[changed[0]].clone();
    if changed.iter().any(|&k| block.lines[k].mixed) {
        warnings.push("Une ligne mêlait plusieurs styles (gras, couleur, taille…) : le style de son premier mot \
                       s'applique désormais à toute la ligne.".to_string());
    }
    if !same {
        warnings.push("Le nombre de lignes a changé : le paragraphe est recomposé (retours à la ligne et \
                       interlignage approximatifs).".to_string());
    }
    let file = embedded_font(&pdf.doc().load_pdf_page(block.page as i32).map_err(err)?, &first.font);
    let status = match &file {
        None => "absente",
        Some(f) if Font::from_bytes("police", f).is_ok_and(|font| draws_all(&font, new_text)) => "ok",
        Some(_) => "partielle",
    };
    let (sub, sub_label) = substitute(&first);
    let mut attempts: Vec<(String, Option<Vec<u8>>, Option<String>)> = vec![];
    if status == "ok" {
        attempts.push((unique_name(&first.font), file, None));
    }
    // La même police installée sur l'ordinateur, puis la plus proche (Arial, Times New
    // Roman, Courier New), réduites aux caractères utilisés.
    let family = if first.mono { "mono" } else if first.serif { "serif" } else { "sans" };
    let installed = crate::fonts::find(&first.font).map(|p| (p, None))
        .into_iter()
        .chain(crate::fonts::generic(family, first.bold, first.italic).map(|(p, l)| (p, Some(l))));
    for (path, label) in installed {
        if let Ok(data) = crate::fonts::subset(&path, new_text) {
            let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            attempts.push((unique_name(&name), Some(data), label));
        }
    }
    attempts.push((sub, None, Some(sub_label)));
    let original = first.font.rsplit('+').next().unwrap_or(&first.font).to_string();
    // Police standard non intégrée remplacée par son équivalent : rien à signaler.
    let family = norm(&original);
    let same_family = status == "absente"
        && if first.mono { family.contains("courier") } else if first.serif { family.contains("times") }
           else { family.contains("helvetica") || family.contains("arial") };
    for (name, buf, label) in attempts {
        match rewrite(pdf, block, &lines, &name, buf.as_deref()) {
            Err(e) if e == NOT_FAITHFUL => continue,
            Err(e) => return Err(e),
            Ok(overflow) => {
                if let Some(label) = label.filter(|_| !same_family) {
                    let why = if status == "absente" { "n'est pas intégrée au fichier" } else { "n'est intégrée que partiellement" };
                    warnings.push(format!("La police d'origine « {original} » {why} : remplacée par {label}."));
                }
                if overflow {
                    warnings.push("Le nouveau texte dépasse la place de l'ancien : vérifiez qu'il ne chevauche pas le \
                                   contenu voisin.".to_string());
                }
                return Ok(warnings);
            }
        }
    }
    Err("Ces caractères ne peuvent pas être écrits avec une police standard (alphabet latin uniquement).".into())
}

// ----- caviardage définitif ---------------------------------------------------------------

// ----- images de la page : déplacer, redimensionner, supprimer ---------------------------------

/// Image affichée sur la page (comme la version Python : les images d'un groupe, tournées ou
/// affichées plusieurs fois ne se modifient pas).
#[derive(Debug, Clone, PartialEq)]
pub struct PlacedImage {
    pub xref: i32,
    /// Nom de la ressource (« Im1 ») dans la page.
    pub name: String,
    pub rect: Rect,
    pub rotated: bool,
    /// Affichée plusieurs fois sur la page (ou plusieurs images de mêmes dimensions).
    pub shared: bool,
}

/// Images de la page, de la plus en dessous à la plus au-dessus.
pub fn images(pdf: &Pdf, page: usize) -> Vec<PlacedImage> {
    let doc = pdf.doc();
    let (Ok(p), Ok(pp)) = (doc.load_page(page as i32), doc.load_pdf_page(page as i32)) else { return vec![] };
    let resources = pp.images().unwrap_or_default();
    let Ok(tp) = p.to_text_page(TextPageFlags::PRESERVE_IMAGES) else { return vec![] };
    let shown: Vec<(u32, u32, Rect, bool)> = tp.blocks()
        .filter(|b| b.r#type() == TextBlockType::Image)
        .filter_map(|b| {
            let img = b.image()?;
            let m = b.ctm()?;
            let rotated = m.b.abs() > 1e-3 || m.c.abs() > 1e-3 || m.a < 0.0 || m.d < 0.0;
            Some((img.width(), img.height(), Rect::from(b.bounds()), rotated))
        })
        .collect();
    shown.iter().filter_map(|&(w, h, rect, rotated)| {
        let same: Vec<_> = resources.iter().filter(|r| r.width == w && r.height == h).collect();
        let res = same.first()?; // sinon : image d'un groupe (Form XObject)
        let shared = same.len() > 1 || shown.iter().filter(|s| s.0 == w && s.1 == h).count() > 1;
        Some(PlacedImage { xref: res.xref, name: res.name.trim_start_matches('/').to_string(), rect, rotated, shared })
    }).collect()
}

/// Retire « /nom Do » du contenu de la page (l'image reste pour les autres pages).
fn remove_placements(doc: &mut PdfDocument, page: usize, name: &str) -> Result<usize, String> {
    let p = doc.load_pdf_page(page as i32).map_err(err)?;
    let Some(contents) = p.object().get_dict("Contents").map_err(err)? else { return Ok(0) };
    let mut streams = vec![];
    if contents.is_array().map_err(err)? {
        for i in 0..contents.len().map_err(err)? as i32 {
            streams.extend(contents.get_array(i).map_err(err)?);
        }
    } else {
        streams.push(contents);
    }
    let pattern = format!("/{name}");
    let mut count = 0;
    for mut stream in streams {
        let data = stream.read_stream().map_err(err)?;
        let (mut out, mut i, mut changed) = (Vec::with_capacity(data.len()), 0, false);
        while i < data.len() {
            if data[i..].starts_with(pattern.as_bytes()) {
                let mut j = i + pattern.len();
                let ws = j;
                while j < data.len() && data[j].is_ascii_whitespace() {
                    j += 1;
                }
                let ends = |k: usize| k >= data.len() || data[k].is_ascii_whitespace() || b"/[<(%".contains(&data[k]);
                if j > ws && data[j..].starts_with(b"Do") && ends(j + 2) {
                    out.push(b' ');
                    i = j + 2;
                    count += 1;
                    changed = true;
                    continue;
                }
            }
            out.push(data[i]);
            i += 1;
        }
        if changed {
            stream.write_stream_buffer(&mupdf::Buffer::from_bytes(&out).map_err(err)?).map_err(err)?;
        }
    }
    Ok(count)
}

fn single(img: &PlacedImage) -> Result<(), String> {
    if img.shared {
        return Err("Cette image est affichée plusieurs fois sur la page : modification non prise en charge.".into());
    }
    Ok(())
}

pub fn delete_image(pdf: &mut Pdf, page: usize, img: &PlacedImage) -> Result<(), String> {
    single(img)?;
    pdf.edit("Supprimer l'image", |doc| match remove_placements(doc, page, &img.name)? {
        0 => Err("Image introuvable dans le contenu de la page.".into()),
        _ => Ok(()),
    })
}

/// Déplace / redimensionne l'image (même objet image : pas de recompression).
pub fn place_image(pdf: &mut Pdf, page: usize, img: &PlacedImage, rect: Rect) -> Result<(), String> {
    if img.rotated {
        return Err("Image tournée ou inclinée : déplacement non pris en charge.".into());
    }
    single(img)?;
    pdf.edit("Déplacer l'image", |doc| {
        if remove_placements(doc, page, &img.name)? != 1 {
            return Err("Image introuvable dans le contenu de la page.".into());
        }
        let mut p = doc.load_pdf_page(page as i32).map_err(err)?;
        let inv = p.ctm().map_err(err)?.invert().ok_or("transformation de page invalide")?;
        let target = mupdf::Rect::new(rect.x0, rect.y0, rect.x1, rect.y1).transform(&inv);
        p.insert_image(doc, target, mupdf::pdf::PageImageSource::ExistingXref(img.xref), Default::default()).map_err(err)?;
        Ok(())
    })
}

fn is_redact(a: &mupdf::pdf::PdfAnnotation) -> bool {
    a.r#type().ok() == Some(PdfAnnotationType::Redact)
}

fn redactions_on(pdf: &Pdf, page: usize) -> bool {
    pdf.doc().load_pdf_page(page as i32).is_ok_and(|p| p.annotations().any(|a| is_redact(&a)))
}

pub fn pending_redactions(pdf: &Pdf) -> usize {
    (0..pdf.page_count()).filter_map(|i| pdf.doc().load_pdf_page(i as i32).ok())
        .map(|p| p.annotations().filter(is_redact).count())
        .sum()
}

fn mark(p: &mut PdfPage, r: Rect) -> Result<(), String> {
    let mut a = p.add_redact_annotation(mupdf::Rect::new(r.x0, r.y0, r.x1, r.y1)).map_err(err)?;
    a.update().map_err(err)?;
    Ok(())
}

/// Zone marquée (visible, encore réversible) ; rien n'est retiré avant `apply_redactions`.
pub fn mark_redaction(pdf: &mut Pdf, page: usize, rect: Rect) -> Result<(), String> {
    if rect.width() < 2.0 || rect.height() < 2.0 {
        return Err("Zone trop petite : tracez un cadre sur la page.".into());
    }
    pdf.edit("Marquer pour caviardage", |doc| mark(&mut doc.load_pdf_page(page as i32).map_err(err)?, rect))
}

/// Marque toutes les occurrences de `query`. Renvoie leur nombre.
pub fn mark_text(pdf: &mut Pdf, query: &str) -> Result<usize, String> {
    if query.trim().is_empty() {
        return Err("Texte à rechercher vide.".into());
    }
    let hits: Vec<(usize, Vec<Rect>)> = (0..pdf.page_count())
        .map(|i| (i, pdf.search(i, query)))
        .filter(|(_, r)| !r.is_empty())
        .collect();
    let count = hits.iter().map(|h| h.1.len()).sum();
    if count > 0 {
        pdf.edit(&format!("Marquer « {query} »"), |doc| {
            for (i, rects) in &hits {
                let mut p = doc.load_pdf_page(*i as i32).map_err(err)?;
                for r in rects {
                    mark(&mut p, *r)?;
                }
            }
            Ok(())
        })?;
    }
    Ok(count)
}

/// Retire réellement texte, images (pixels) et tracés sous les zones marquées, ainsi que
/// les commentaires et champs de formulaire qui les chevauchent. Renvoie (zones, retirés).
pub fn apply_redactions(pdf: &mut Pdf) -> Result<(usize, usize), String> {
    let (mut zones, mut removed) = (0, 0);
    let pages = pdf.page_count();
    pdf.edit("Appliquer les caviardages", |doc| {
        for i in 0..pages {
            let mut p = doc.load_pdf_page(i as i32).map_err(err)?;
            let rects: Vec<Rect> = p.annotations().filter(is_redact).filter_map(|a| a.bounds().ok().map(Rect::from)).collect();
            if rects.is_empty() {
                continue;
            }
            let hits = |r: Rect| rects.iter().any(|z| r.x0 < z.x1 && z.x0 < r.x1 && r.y0 < z.y1 && z.y0 < r.y1);
            // Un commentaire posé dessus contient souvent le secret ; la valeur d'un champ aussi.
            let notes: Vec<i32> = p.annotations().filter(|a| !is_redact(a) && a.bounds().is_ok_and(|b| hits(b.into())))
                .filter_map(|a| a.xref().ok()).collect();
            for x in notes {
                if let Some(a) = p.annotations().find(|a| a.xref().ok() == Some(x)) {
                    p.delete_annotation(a).map_err(err)?;
                    removed += 1;
                }
            }
            let fields: Vec<i32> = p.widgets().filter(|w| w.annotation().bounds().is_ok_and(|b| hits(b.into())))
                .filter_map(|w| w.xref().ok()).collect();
            for x in fields {
                if let Some(w) = p.widgets().find(|w| w.xref().ok() == Some(x)) {
                    p.delete_widget(w).map_err(err)?;
                    removed += 1;
                }
            }
            p.apply_redactions_with_options(PdfRedactOptions {
                black_boxes: true,
                image_method: PdfRedactImageMethod::Pixels,
                line_art: PdfRedactLineArtMethod::RemoveIfCovered,
                text: PdfRedactTextMethod::Remove,
            }).map_err(err)?;
            zones += rects.len();
        }
        Ok(())
    })?;
    if zones > 0 {
        pdf.rewrite_all(); // l'enregistrement réécrit tout le fichier
    }
    Ok((zones, removed))
}

/// « Nettoyer le document » : informations cachées retirées. Renvoie ce qui l'a été.
pub fn clean(pdf: &mut Pdf, metadata: bool, javascript: bool, attachments: bool, thumbnails: bool)
             -> Result<Vec<&'static str>, String> {
    let mut done = vec![];
    let pages = pdf.page_count();
    pdf.edit("Nettoyer le document", |doc| {
        let mut catalog = doc.catalog().map_err(err)?;
        let mut names = catalog.get_dict("Names").map_err(err)?;
        if metadata {
            catalog.dict_delete("Metadata").map_err(err)?;
            doc.trailer().map_err(err)?.dict_delete("Info").map_err(err)?;
            done.push("métadonnées");
        }
        if javascript {
            if let Some(n) = names.as_mut() {
                n.dict_delete("JavaScript").map_err(err)?;
            }
            catalog.dict_delete("OpenAction").map_err(err)?;
            catalog.dict_delete("AA").map_err(err)?;
            done.push("JavaScript et actions automatiques");
        }
        if attachments {
            if let Some(n) = names.as_mut() {
                n.dict_delete("EmbeddedFiles").map_err(err)?;
            }
            done.push("pièces jointes");
        }
        if thumbnails {
            done.push("vignettes intégrées");
        }
        for i in 0..pages {
            let mut p = doc.load_pdf_page(i as i32).map_err(err)?;
            let mut obj: PdfObject = p.object();
            if thumbnails {
                obj.dict_delete("Thumb").map_err(err)?;
            }
            if javascript {
                obj.dict_delete("AA").map_err(err)?;
            }
            if attachments {
                let files: Vec<i32> = p.annotations().filter(|a| a.r#type().ok() == Some(PdfAnnotationType::FileAttachment))
                    .filter_map(|a| a.xref().ok()).collect();
                for x in files {
                    if let Some(a) = p.annotations().find(|a| a.xref().ok() == Some(x)) {
                        p.delete_annotation(a).map_err(err)?;
                    }
                }
            }
        }
        Ok(())
    })?;
    pdf.rewrite_all();
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Pdf {
        Pdf::open(&std::fs::read("tests/fixtures/Été déjà vu.pdf").unwrap()).unwrap()
    }

    fn words(pdf: &Pdf, page: usize) -> Vec<String> {
        pdf.words(page).into_iter().map(|w| w.text).collect()
    }

    /// Image ajoutée, retrouvée, déplacée et agrandie, puis supprimée ; liens créés et retirés.
    #[test]
    fn images_et_liens() {
        let mut pdf = fixture();
        let png = std::fs::read("tests/fixtures/image.png").unwrap();
        crate::annot::add_image(&mut pdf, 0, Rect::new(100.0, 300.0, 300.0, 450.0), &png).unwrap();
        let imgs = images(&pdf, 0);
        assert_eq!(imgs.len(), 1, "{imgs:?}");
        let img = &imgs[0];
        assert!(!img.rotated && !img.shared && img.rect.contains(200.0, 375.0), "{img:?}");
        let to = Rect::new(50.0, 500.0, 350.0, 700.0);
        place_image(&mut pdf, 0, img, to).unwrap();
        let moved = images(&pdf, 0);
        assert_eq!(moved.len(), 1);
        assert!((moved[0].rect.x0 - 50.0).abs() < 1.0 && (moved[0].rect.y1 - 700.0).abs() < 1.0, "{:?}", moved[0].rect);
        delete_image(&mut pdf, 0, &moved[0]).unwrap();
        assert!(images(&pdf, 0).is_empty());
        pdf.undo().unwrap();
        assert_eq!(images(&pdf, 0).len(), 1, "Ctrl+Z rend l'image");

        use crate::annot::{add_link, delete_link, link_target};
        add_link(&mut pdf, 0, Rect::new(100.0, 100.0, 200.0, 120.0), "exemple.fr").unwrap();
        add_link(&mut pdf, 0, Rect::new(100.0, 140.0, 200.0, 160.0), "3").unwrap();
        let links = pdf.links(0);
        assert!(links.iter().any(|l| l.uri.as_deref() == Some("https://exemple.fr") && l.rect.contains(150.0, 110.0)), "{links:?}");
        assert!(links.iter().any(|l| l.page == Some(2) && l.rect.contains(150.0, 150.0)), "{links:?}");
        delete_link(&mut pdf, 0, 150.0, 110.0).unwrap();
        assert_eq!(pdf.links(0).len(), 1);
        assert!(delete_link(&mut pdf, 0, 500.0, 500.0).is_err());
        assert!(link_target("javascript:alert(1)", 3).is_err() && link_target("9", 3).is_err() && link_target(" ", 3).is_err());
        assert!(matches!(link_target("moi@site.fr", 3), Ok(mupdf::pdf::LinkAction::Action(mupdf::pdf::PdfAction::Uri(u))) if u == "mailto:moi@site.fr"));
    }

    fn block_of(pdf: &Pdf, page: usize, word: &str) -> (TextBlock, Rect) {
        let r = pdf.words(page).into_iter().find(|w| w.text == word).unwrap().rect;
        (text_block_at(pdf, page, (r.x0 + r.x1) / 2.0, (r.y0 + r.y1) / 2.0).unwrap().unwrap(), r)
    }

    #[test]
    fn modifier_une_ligne() {
        let mut pdf = fixture();
        let (block, bonjour) = block_of(&pdf, 0, "Bonjour");
        assert!(block.text().contains("Bonjour page 1"), "{:?}", block.text());
        let new = block.text().replace("Bonjour page 1", "Au revoir, été 2026");
        let warnings = edit_block(&mut pdf, &block, &new).unwrap();
        let got = words(&pdf, 0);
        assert!(got.contains(&"revoir,".to_string()) && got.contains(&"été".to_string()), "{got:?} {warnings:?}");
        assert!(!got.contains(&"Bonjour".to_string()));
        assert!(got.contains(&"renard".to_string()), "la ligne voisine reste : {got:?}");
        // Même place : le premier mot commence où commençait l'ancien.
        let au = pdf.words(0).into_iter().find(|w| w.text == "Au").unwrap().rect;
        assert!((au.x0 - bonjour.x0).abs() < 1.5 && (au.y1 - bonjour.y1).abs() < 2.0, "{au:?} / {bonjour:?}");
        pdf.undo().unwrap();
        assert!(words(&pdf, 0).contains(&"Bonjour".to_string()));
    }

    #[test]
    fn recomposer_et_page_tournee() {
        let mut pdf = fixture();
        let (block, _) = block_of(&pdf, 0, "Bonjour");
        let more = format!("{}\nUne ligne de plus", block.text());
        edit_block(&mut pdf, &block, &more).unwrap();
        assert!(words(&pdf, 0).contains(&"plus".to_string()));

        // Page tournée de 90° : le texte réécrit garde son sens.
        let mut pdf = fixture();
        crate::edit::rotate(&mut pdf, &[1], 90).unwrap();
        let (block, r) = block_of(&pdf, 1, "Bonjour");
        let new = block.text().replace("Bonjour", "Salut");
        edit_block(&mut pdf, &block, &new).unwrap();
        let salut = pdf.words(1).into_iter().find(|w| w.text == "Salut").expect("réécrit").rect;
        assert_eq!(salut.width() > salut.height(), r.width() > r.height(), "même sens : {salut:?} / {r:?}");
        assert!((salut.x0 - r.x0).abs() < 2.0 && (salut.y0 - r.y0).abs() < 2.0, "même place : {salut:?} / {r:?}");
    }

    #[test]
    fn police_integree_reutilisee_ou_remplacee() {
        let data = std::fs::read("tests/fixtures/police-integree.pdf").unwrap();
        // Mêmes lettres : la police d'origine (sous-ensemble intégré) suffit.
        let mut pdf = Pdf::open(&data).unwrap();
        let (block, _) = block_of(&pdf, 0, "Quel");
        assert!((block.lines[0].color.2 - 0.6).abs() < 0.02, "couleur lue : {:?}", block.lines[0].color);
        let new = block.text().replace("Quel beau texte", "Quel texte beau");
        let warnings = edit_block(&mut pdf, &block, &new).unwrap();
        assert!(words(&pdf, 0).contains(&"beau".to_string()));
        let fonts: Vec<String> = pdf.doc().load_page(0).unwrap().to_text_page(TextPageFlags::empty()).unwrap()
            .blocks().flat_map(|b| b.lines().flat_map(|l| l.chars().filter_map(|c| c.font().map(|f| f.name().to_string())).collect::<Vec<_>>()).collect::<Vec<_>>())
            .collect();
        // DejaVu installée (Linux) : la même police, sans avertissement ; sinon (Windows),
        // la plus proche, et l'utilisateur est prévenu.
        if crate::fonts::find("DejaVu Sans Book").is_some() {
            assert!(warnings.is_empty(), "{warnings:?}");
            assert!(fonts.iter().all(|f| f.contains("DejaVu")), "{fonts:?}");
        } else {
            assert!(warnings.iter().any(|w| w.contains("remplacée par")), "{warnings:?}");
        }
        // Nouvelles lettres (absentes du sous-ensemble) : écrites quand même.
        let mut pdf = Pdf::open(&data).unwrap();
        let (block, _) = block_of(&pdf, 0, "Quel");
        let new = block.text().replace("Quel beau texte", "Zébrures vives à 10 €");
        edit_block(&mut pdf, &block, &new).unwrap();
        let got = words(&pdf, 0);
        assert!(got.contains(&"Zébrures".to_string()) && got.contains(&"€".to_string()), "{got:?}");
        // Modifié deux fois : chaque sous-ensemble a son nom (pas de police périmée en cache).
        let (block, _) = block_of(&pdf, 0, "Zébrures");
        let new = block.text().replace("vives", "ÿöü");
        edit_block(&mut pdf, &block, &new).unwrap();
        assert!(words(&pdf, 0).contains(&"ÿöü".to_string()), "{:?}", words(&pdf, 0));
    }

    #[test]
    fn caviarder() {
        let mut pdf = fixture();
        assert_eq!(mark_text(&mut pdf, "renard").unwrap(), 3);
        assert_eq!(pending_redactions(&pdf), 3);
        let (block, _) = block_of(&pdf, 0, "Bonjour");
        assert!(edit_block(&mut pdf, &block, "x").is_err(), "zones en attente sur la page");
        assert!(words(&pdf, 0).contains(&"renard".to_string()), "marqué, pas encore retiré");
        let (zones, _) = apply_redactions(&mut pdf).unwrap();
        assert_eq!(zones, 3);
        assert_eq!(pending_redactions(&pdf), 0);
        let reopened = Pdf::open(&pdf.bytes_for_disk().unwrap()).unwrap();
        for i in 0..3 {
            let w = words(&reopened, i);
            assert!(!w.contains(&"renard".to_string()), "page {i} : {w:?}");
            assert!(w.contains(&"brun".to_string()), "le reste demeure : {w:?}");
        }
    }

    #[test]
    fn nettoyer() {
        let mut pdf = fixture();
        let mut values = crate::security::metadata(&pdf);
        values[0] = "Secret".into();
        crate::security::set_metadata(&mut pdf, &values).unwrap();
        clean(&mut pdf, true, true, true, true).unwrap();
        let reopened = Pdf::open(&pdf.bytes_for_disk().unwrap()).unwrap();
        assert_eq!(crate::security::metadata(&reopened)[0], "");
    }
}
