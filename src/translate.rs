//! Traduire un document en gardant sa mise en page. Sans interface ; le traducteur (DeepL)
//! est passé en paramètre.
//!
//! Chaque page est découpée en passages : paragraphes (lignes d'un même bloc, même taille,
//! qui se suivent) et morceaux de ligne séparés par un grand blanc (cellules de tableau).
//! Les textes sans lettres (nombres, dates) ne partent pas. Chaque passage traduit est effacé
//! (texte seul : images, tracés et couleurs de fond intacts) puis réécrit dans le même cadre,
//! même couleur, même alignement, avec la même police si elle est installée (sinon Arial,
//! Times New Roman ou Courier New ; Helvetica / Times / Courier standard en dernier recours,
//! alphabet latin seulement). Plus long que l'original, il est réduit jusqu'à 70 % ; s'il ne
//! tient toujours pas, il déborde vers le bas (signalé).
//!
//! Limites : texte incliné ou invisible (couche OCR d'un scan) laissé tel quel ; une ligne à
//! styles mixtes (un mot en gras) prend le style de son premier mot.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use mupdf::pdf::{PdfDocument, PdfRedactImageMethod, PdfRedactLineArtMethod, PdfRedactOptions, PdfRedactTextMethod};
use mupdf::shape::{PdfColor, Shape, TextAlign, TextboxOptions};
use mupdf::text_page::{TextBlockType, TextChar, TextPageFlags};

use crate::pdf::{Pdf, Rect};
use crate::textedit::{self, Line};

/// Réduction maximale d'un passage trop long.
const SHRINK: [f32; 5] = [1.0, 0.92, 0.85, 0.78, 0.7];

#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Report {
    pub translated: usize,
    pub shrunk: usize,
    pub overflow: usize,
    /// Passages laissés en langue d'origine : texte incliné, invisible, ou impossible à écrire.
    pub skipped: usize,
}

struct Passage {
    page: usize,
    lines: Vec<Line>,
    text: String,
    align: TextAlign,
}

impl Passage {
    fn rect(&self) -> Rect {
        self.lines.iter().map(|l| l.rect).reduce(union).unwrap()
    }
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn has_words(text: &str) -> bool {
    text.chars().filter(|c| c.is_alphabetic()).count() >= 2
}

/// Morceaux d'une ligne séparés par un blanc de plus de 1,2 fois la taille du texte.
fn segments<'a, 'b>(chars: &'a [TextChar<'b>]) -> Vec<&'a [TextChar<'b>]> {
    let mut out = vec![];
    let mut start = 0;
    let mut right: Option<f32> = None;
    for (i, c) in chars.iter().enumerate() {
        if c.char().is_some_and(char::is_whitespace) {
            continue;
        }
        let q = c.quad();
        if right.is_some_and(|r| q.ul.x - r > 1.2 * c.size()) {
            out.push(&chars[start..i]);
            start = i;
        }
        right = Some(q.ur.x);
    }
    out.push(&chars[start..]);
    out
}

fn union(a: Rect, b: Rect) -> Rect {
    Rect::new(a.x0.min(b.x0), a.y0.min(b.y0), a.x1.max(b.x1), a.y1.max(b.y1))
}

fn union_of(chars: &[TextChar]) -> Rect {
    chars.iter().filter(|c| c.char().is_some_and(|ch| !ch.is_whitespace()))
        .map(|c| Rect::from(c.quad())).reduce(union).unwrap_or(Rect::new(0.0, 0.0, 0.0, 0.0))
}

/// « exem- » + « ple » → « exemple » ; sinon une espace.
fn join(text: &mut String, next: &str) {
    let hyphen = text.ends_with('-') && text.chars().rev().nth(1).is_some_and(char::is_alphabetic)
        && next.chars().next().is_some_and(char::is_lowercase);
    if hyphen {
        text.pop();
    } else if !text.is_empty() {
        text.push(' ');
    }
    text.push_str(next);
}

/// `margin` : marge gauche du texte de la page (la droite est supposée symétrique).
fn align_of(lines: &[Line], page_w: f32, margin: f32) -> TextAlign {
    let spread = |f: &dyn Fn(&Line) -> f32| {
        let v: Vec<f32> = lines.iter().map(f).collect();
        v.iter().cloned().fold(f32::MIN, f32::max) - v.iter().cloned().fold(f32::MAX, f32::min)
    };
    if lines.len() >= 2 {
        if spread(&|l| l.rect.x0) <= 2.0 {
            let right = lines.iter().map(|l| l.rect.x1).fold(f32::MIN, f32::max);
            let full = lines[..lines.len() - 1].iter().all(|l| right - l.rect.x1 <= 3.0);
            return if lines.len() >= 3 && full { TextAlign::Justify } else { TextAlign::Left };
        }
        if spread(&|l| (l.rect.x0 + l.rect.x1) / 2.0) <= 3.0 {
            return TextAlign::Center;
        }
        if spread(&|l| l.rect.x1) <= 2.0 {
            return TextAlign::Right;
        }
        return TextAlign::Left;
    }
    let r = lines[0].rect;
    if ((r.x0 + r.x1) / 2.0 - page_w / 2.0).abs() < page_w * 0.02 && r.x0 > page_w * 0.15 {
        TextAlign::Center
    } else if r.x0 > page_w * 0.4 && r.x1 >= page_w - margin - 3.0 {
        TextAlign::Right
    } else {
        TextAlign::Left
    }
}

/// Passages d'une page (non tournée) ; et le nombre de passages laissés tels quels.
fn passages(doc: &PdfDocument, page: usize) -> Result<(Vec<Passage>, usize), String> {
    let p = doc.load_page(page as i32).map_err(err)?;
    let page_w = p.bounds().map_err(err)?.x1;
    let tp = p.to_text_page(TextPageFlags::empty()).map_err(err)?;
    let (mut out, mut skipped) = (vec![], 0);
    for b in tp.blocks() {
        if b.r#type() != TextBlockType::Text {
            continue;
        }
        let mut current: Option<Passage> = None;
        for l in b.lines() {
            let chars: Vec<TextChar> = l.chars().collect();
            let segs = segments(&chars);
            let single = segs.len() == 1;
            for seg in segs {
                let line = match textedit::line_of(seg, union_of(seg)) {
                    None => continue,
                    Some(Err(_)) => {
                        skipped += has_words(&seg.iter().filter_map(|c| c.char()).collect::<String>()) as usize;
                        continue;
                    }
                    Some(Ok(line)) if line.angle != 0 => {
                        skipped += has_words(&line.text) as usize;
                        continue;
                    }
                    Some(Ok(line)) => line,
                };
                let text = line.text.trim().to_string();
                // suite du paragraphe : même style, ligne suivante, colonnes qui se recouvrent
                let follows = single && current.as_ref().is_some_and(|c| {
                    let last = c.lines.last().unwrap();
                    let dy = line.origin.1 - last.origin.1;
                    (line.size - last.size).abs() <= last.size * 0.15 && line.bold == last.bold
                        && dy > 0.0 && dy <= last.size * 1.8 && line.rect.x0 < last.rect.x1 && line.rect.x1 > last.rect.x0
                });
                if follows {
                    let c = current.as_mut().unwrap();
                    join(&mut c.text, &text);
                    c.lines.push(line);
                } else {
                    out.extend(current.take());
                    current = Some(Passage { page, lines: vec![line], text, align: TextAlign::Left });
                }
                if !single {
                    out.extend(current.take()); // une cellule ne se prolonge pas
                }
            }
        }
        out.extend(current.take());
    }
    let margin = out.iter().map(|p| p.rect().x0).fold(page_w / 2.0, f32::min);
    for p in &mut out {
        p.align = align_of(&p.lines, page_w, margin);
    }
    Ok((out, skipped))
}

fn family(line: &Line) -> &'static str {
    if line.mono { "mono" } else if line.serif { "serif" } else { "sans" }
}

/// Caractères que les polices standard (codage WinAnsi) savent écrire.
fn winansi(text: &str) -> bool {
    text.chars().all(|c| (' '..='\u{ff}').contains(&c) || "€‚ƒ„…†‡ˆ‰Š‹ŒŽ‘’“”•–\u{2014}˜™š›œžŸ".contains(c))
}

/// Traduit les pages `pages` (toutes si vide) de `data` ; `translator` reçoit tous les textes
/// à traduire d'un coup et rend leurs traductions, dans l'ordre.
pub fn translate_pdf(data: &[u8], pages: &[usize],
                     translator: impl FnOnce(&[String]) -> Result<Vec<String>, String>) -> Result<(Vec<u8>, Report), String> {
    let mut pdf = Pdf::open(data).map_err(|e| format!("{e:?}"))?;
    if textedit::pending_redactions(&pdf) > 0 {
        return Err("Des zones sont marquées pour caviardage : appliquez-les ou supprimez-les d'abord.".into());
    }
    let pages: Vec<usize> = if pages.is_empty() { (0..pdf.page_count()).collect() } else { pages.to_vec() };
    let mut report = Report::default();
    pdf.edit("Traduire", |doc| {
        // Pages tournées : traitées à plat (texte horizontal), puis remises comme avant.
        let mut rotations = vec![];
        let mut all = vec![];
        for &i in &pages {
            let mut p = doc.load_pdf_page(i as i32).map_err(err)?;
            let r = p.rotation().map_err(err)?;
            if r != 0 {
                p.set_rotation(0).map_err(err)?;
                rotations.push((i, r));
            }
            let (found, skipped) = passages(doc, i)?;
            report.skipped += skipped;
            all.extend(found);
        }
        // Une seule fois chaque texte, et seulement ceux qui ont des mots.
        let mut unique: Vec<String> = all.iter().map(|p| p.text.clone()).filter(|t| has_words(t)).collect();
        unique.sort();
        unique.dedup();
        let translated = if unique.is_empty() { vec![] } else { translator(&unique)? };
        if translated.len() != unique.len() {
            return Err("traduction incomplète".into());
        }
        let dict: HashMap<&str, &str> = unique.iter().map(String::as_str).zip(translated.iter().map(String::as_str)).collect();
        // Passages à réécrire, et la police de chacun (réduite aux caractères utilisés).
        let mut todo: Vec<(&Passage, String, Option<PathBuf>)> = vec![];
        for p in &all {
            let Some(t) = dict.get(p.text.as_str()).map(|t| t.trim().to_string()).filter(|t| !t.is_empty() && *t != p.text) else { continue };
            let first = &p.lines[0];
            let path = crate::fonts::find(&first.font)
                .or_else(|| crate::fonts::generic(family(first), first.bold, first.italic).map(|g| g.0));
            if path.is_none() && !winansi(&t) {
                report.skipped += 1;
                continue;
            }
            todo.push((p, t, path));
        }
        let mut texts: BTreeMap<PathBuf, String> = BTreeMap::new();
        for (_, t, path) in &todo {
            if let Some(path) = path {
                texts.entry(path.clone()).or_default().push_str(t);
            }
        }
        let mut fonts: HashMap<PathBuf, (String, Vec<u8>)> = HashMap::new();
        for (path, text) in texts {
            let data = crate::fonts::subset(&path, &text)?;
            let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            fonts.insert(path, (textedit::unique_name(&stem), data));
        }
        for &page in &pages {
            let here: Vec<&(&Passage, String, Option<PathBuf>)> = todo.iter().filter(|(p, ..)| p.page == page).collect();
            if here.is_empty() {
                continue;
            }
            let bounds = doc.load_page(page as i32).and_then(|p| p.bounds()).map_err(err)?;
            // Cadres de tout le texte de la page : un passage aligné à gauche peut s'étendre
            // vers la droite jusqu'au texte voisin, ou jusqu'à la marge (symétrique de la gauche).
            let boxes: Vec<Rect> = all.iter().filter(|q| q.page == page).map(Passage::rect).collect();
            let margin = boxes.iter().map(|b| b.x0).fold(bounds.x1 / 2.0, f32::min).max(18.0);
            let mut p = doc.load_pdf_page(page as i32).map_err(err)?;
            for (passage, ..) in &here {
                for l in &passage.lines {
                    // bandes resserrées : ne pas toucher les lignes voisines
                    let dy = l.rect.height() * 0.2;
                    p.add_redact_annotation(mupdf::Rect::new(l.rect.x0, l.rect.y0 + dy, l.rect.x1, l.rect.y1 - dy)).map_err(err)?;
                }
            }
            p.apply_redactions_with_options(PdfRedactOptions {
                black_boxes: false,
                image_method: PdfRedactImageMethod::None,
                line_art: PdfRedactLineArtMethod::None,
                text: PdfRedactTextMethod::Remove,
            }).map_err(err)?;
            let mut shape = Shape::new(&mut p).map_err(err)?;
            for (passage, text, path) in here {
                let first = &passage.lines[0];
                let (name, file) = match path {
                    Some(path) => { let (n, d) = &fonts[path]; (n.clone(), Some(d.as_slice())) }
                    None => (textedit::substitute(first).0, None),
                };
                let leading = passage.lines.get(1).map_or(1.2, |l| ((l.origin.1 - first.origin.1) / first.size).clamp(1.0, 2.0));
                let r = passage.rect();
                let right = if passage.align == TextAlign::Left {
                    boxes.iter().filter(|o| o.y0 < r.y1 && o.y1 > r.y0 && o.x0 >= r.x1 - 0.5)
                        .map(|o| o.x0 - 4.0).fold(bounds.x1 - margin, f32::min).max(r.x1 + 1.0)
                } else {
                    r.x1 + 1.0
                };
                let top = first.origin.1 - first.size * 0.95;
                let attempt = |shape: &mut Shape, scale: f32, bottom: f32| -> Result<bool, String> {
                    let opts = TextboxOptions {
                        fontsize: first.size * scale,
                        lineheight: leading,
                        fontname: name.clone(),
                        fontfile: file,
                        simple: file.is_none(),
                        fill: Some(PdfColor::rgb(first.color.0, first.color.1, first.color.2)),
                        align: passage.align,
                        ..Default::default()
                    };
                    Ok(shape.insert_textbox(mupdf::Rect::new(r.x0, top, right, bottom), text, &opts).map_err(err)? >= 0.0)
                };
                let mut done = false;
                for scale in SHRINK {
                    if attempt(&mut shape, scale, r.y1 + first.size * 0.3)? {
                        report.shrunk += (scale < 1.0) as usize;
                        done = true;
                        break;
                    }
                }
                if !done {
                    // ne tient pas : déborde vers le bas plutôt que de perdre le texte
                    done = attempt(&mut shape, SHRINK[SHRINK.len() - 1], bounds.y1)?;
                    report.overflow += 1;
                }
                if !done {
                    return Err(format!("Le texte traduit ne tient pas sur la page {} : « {} ».", page + 1, passage.text));
                }
                report.translated += 1;
            }
            shape.commit(doc, true).map_err(err)?;
        }
        for (i, r) in rotations {
            doc.load_pdf_page(i as i32).and_then(|mut p| p.set_rotation(r)).map_err(err)?;
        }
        Ok(())
    })?;
    Ok((pdf.bytes_for_disk()?, report))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_of(data: &[u8], page: usize) -> String {
        let pdf = Pdf::open(data).unwrap();
        pdf.words(page).iter().map(|w| w.text.clone()).collect::<Vec<_>>().join(" ")
    }

    /// Traducteur factice : en majuscules (un peu plus large, comme une vraie traduction).
    fn fake(texts: &[String]) -> Result<Vec<String>, String> {
        Ok(texts.iter().map(|t| t.to_uppercase()).collect())
    }

    #[test]
    fn traduit_en_place() {
        let data = std::fs::read("tests/fixtures/Été déjà vu.pdf").unwrap();
        let mut sent = vec![];
        let (out, report) = translate_pdf(&data, &[], |t| { sent = t.to_vec(); fake(t) }).unwrap();
        assert!(sent.iter().any(|t| t == "Bonjour page 1"), "{sent:?}");
        assert!(report.translated >= 3 && report.overflow == 0, "{report:?}");
        let page1 = text_of(&out, 0);
        assert!(page1.contains("BONJOUR PAGE 1") && !page1.contains("Bonjour"), "{page1}");
        // même place : le titre traduit commence là où commençait l'original
        let before = Pdf::open(&data).unwrap().words(0)[0].rect;
        let after = Pdf::open(&out).unwrap().words(0)[0].rect;
        assert!((before.x0 - after.x0).abs() < 1.5 && (before.y0 - after.y0).abs() < 3.0, "{before:?} {after:?}");
    }

    #[test]
    fn tableau_cellules_et_nombres() {
        let data = std::fs::read("tests/fixtures/tableau.pdf").unwrap();
        let mut sent = vec![];
        let (out, report) = translate_pdf(&data, &[], |t| { sent = t.to_vec(); fake(t) }).unwrap();
        assert!(sent.iter().all(|t| has_words(t)), "nombres envoyés : {sent:?}");
        assert!(report.translated > 0, "{report:?}");
        let before = text_of(&data, 0);
        let after = text_of(&out, 0);
        // les nombres restent, chaque mot d'origine est traduit
        for w in before.split_whitespace().filter(|w| !w.chars().any(char::is_alphabetic)) {
            assert!(after.contains(w), "{w} perdu : {after}");
        }
        assert!(after.contains("DÉSIGNATION") && after.contains("PAIEMENT") && !after.contains("Conseil"), "{after}");
        assert!(report.overflow == 0 && report.shrunk <= 2, "{report:?}"); // le paragraphe, plus large en majuscules
        std::fs::write(std::env::temp_dir().join("traduit-tableau.pdf"), &out).ok();
        assert_eq!(Pdf::open(&out).unwrap().page_count(), Pdf::open(&data).unwrap().page_count());
    }

    #[test]
    fn decoupage() {
        let mut t = "exem-".to_string();
        join(&mut t, "ple");
        assert_eq!(t, "exemple");
        join(&mut t, "Suite");
        assert_eq!(t, "exemple Suite");
        assert!(winansi("Été 10 € « oui »") && !winansi("Привет"));
        assert!(!has_words("12/03/2026 1 250,00") && has_words("Total HT"));
    }
}
