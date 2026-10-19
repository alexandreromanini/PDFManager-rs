//! Organisation des pages (sans interface) : pivoter, supprimer, déplacer, insérer (page
//! vierge, PDF, images), recadrer ; extraire, fusionner, scinder vers de nouveaux fichiers ;
//! plages de pages saisies (« 1-3, 5, 8- »). Les numéros de page sont 0-based ici.

use mupdf::pdf::{InsertPdfOptions, InsertPosition, PdfDocument};
use mupdf::{Rect as MRect, Size};

use crate::pdf::{Pdf, Rect};

pub const A4: (f32, f32) = (595.0, 842.0);

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

// ----- plages de pages ---------------------------------------------------------------------

/// « 1-3, 5, 8- » → groupes de pages (0-based). Vide = toutes les pages.
pub fn parse_ranges(spec: &str, count: usize) -> Result<Vec<Vec<usize>>, String> {
    if spec.trim().is_empty() {
        return Ok(vec![(0..count).collect()]);
    }
    let mut groups = vec![];
    for part in spec.split([',', ';']).map(str::trim).filter(|p| !p.is_empty()) {
        let invalid = || format!("Plage invalide : « {part} » (exemple : 1-3, 5, 8-)");
        let (a, b) = match part.split_once('-') {
            Some((a, b)) => (a.trim(), Some(b.trim())),
            None => (part, None),
        };
        let start: usize = if a.is_empty() { 1 } else { a.parse().map_err(|_| invalid())? };
        let end: usize = match b {
            None => start,
            Some("") => count,
            Some(b) => b.parse().map_err(|_| invalid())?,
        };
        if !(1 <= start && start <= end && end <= count) {
            return Err(format!("« {part} » sort du document (pages 1 à {count})."));
        }
        groups.push((start - 1..end).collect());
    }
    if groups.is_empty() {
        return Err("Aucune page indiquée.".into());
    }
    Ok(groups)
}

pub fn parse_pages(spec: &str, count: usize) -> Result<Vec<usize>, String> {
    let mut pages: Vec<usize> = parse_ranges(spec, count)?.into_iter().flatten().collect();
    pages.sort_unstable();
    pages.dedup();
    Ok(pages)
}

/// [0, 1, 2, 4] → « 1-3, 5 » (inverse de parse_pages).
pub fn format_pages(pages: &[usize]) -> String {
    let mut sorted = pages.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    let mut out: Vec<String> = vec![];
    let mut i = 0;
    while i < sorted.len() {
        let mut j = i;
        while j + 1 < sorted.len() && sorted[j + 1] == sorted[j] + 1 {
            j += 1;
        }
        out.push(if i == j { format!("{}", sorted[i] + 1) } else { format!("{}-{}", sorted[i] + 1, sorted[j] + 1) });
        i = j + 1;
    }
    out.join(", ")
}

/// Nouvel ordre quand `pages` sont déposées avant la page `to` (`count` = à la fin).
pub fn reorder(count: usize, pages: &[usize], to: usize) -> Vec<usize> {
    let rest: Vec<usize> = (0..count).filter(|p| !pages.contains(p)).collect();
    let mut sel: Vec<usize> = pages.to_vec();
    sel.sort_unstable();
    sel.dedup();
    let pos = to - sel.iter().filter(|&&p| p < to).count();
    [&rest[..pos], &sel[..], &rest[pos..]].concat()
}

/// Ne garde que les pages de `order`, dans cet ordre (`select_pages` de MuPDF garde l'ordre
/// du document : on déplace ensuite les pages une à une).
fn arrange(doc: &mut PdfDocument, order: &[usize]) -> Result<(), String> {
    let mut kept: Vec<usize> = order.to_vec();
    kept.sort_unstable();
    kept.dedup();
    if kept.len() != doc.page_count().map_err(err)? as usize {
        doc.select_pages(kept.clone()).map_err(err)?;
    }
    for (k, wanted) in order.iter().enumerate() {
        let cur = kept.iter().position(|p| p == wanted).ok_or("page absente")?;
        if cur != k {
            doc.move_page(cur, k).map_err(err)?;
            let page = kept.remove(cur);
            kept.insert(k, page);
        }
    }
    Ok(())
}

// ----- modifications du document (annulables via Pdf::edit) --------------------------------

pub fn rotate(pdf: &mut Pdf, pages: &[usize], delta: i32) -> Result<(), String> {
    pdf.edit("Pivoter des pages", |doc| {
        for &i in pages {
            let mut page = doc.load_pdf_page(i as i32).map_err(err)?;
            let r = page.rotation().map_err(err)?;
            page.set_rotation((r + delta).rem_euclid(360)).map_err(err)?;
        }
        Ok(())
    })
}

pub fn delete(pdf: &mut Pdf, pages: &[usize]) -> Result<(), String> {
    if pages.len() >= pdf.page_count() {
        return Err("Un PDF doit garder au moins une page : sélectionnez seulement les pages à retirer.".into());
    }
    let mut sorted = pages.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    pdf.edit("Supprimer des pages", |doc| {
        for &i in sorted.iter().rev() {
            doc.delete_page(i as i32).map_err(err)?;
        }
        Ok(())
    })
}

/// Déplace `pages` avant la page `to` ; renvoie leurs nouvelles positions.
pub fn move_pages(pdf: &mut Pdf, pages: &[usize], to: usize) -> Result<Vec<usize>, String> {
    let order = reorder(pdf.page_count(), pages, to);
    if order != (0..pdf.page_count()).collect::<Vec<_>>() {
        pdf.edit("Déplacer des pages", |doc| arrange(doc, &order))?;
    }
    Ok(order.iter().enumerate().filter(|(_, p)| pages.contains(p)).map(|(i, _)| i).collect())
}

pub fn insert_blank(pdf: &mut Pdf, at: usize, size: (f32, f32)) -> Result<(), String> {
    pdf.edit("Insérer une page vierge", |doc| {
        doc.new_page_at(at as i32, Size::new(size.0, size.1)).map(|_| ()).map_err(err)
    })
}

/// Insère toutes les pages de `src` (contenu d'un PDF, déverrouillé avec `password`) avant
/// la page `at`. Renvoie le nombre de pages insérées.
pub fn insert_pdf(pdf: &mut Pdf, at: usize, src: &[u8], password: Option<&str>, name: &str)
                  -> Result<usize, String> {
    let source = Pdf::open_with_password(src, password).map_err(|e| format!("{e:?}"))?;
    let count = source.page_count();
    let src_doc = PdfDocument::from_bytes(&source.bytes()?).map_err(err)?;
    let target = if at >= pdf.page_count() { InsertPosition::Append } else { InsertPosition::Before(at) };
    pdf.edit(&format!("Insérer « {name} »"), |doc| {
        let opts = InsertPdfOptions { target, ..Default::default() };
        doc.insert_pdf(&src_doc, opts).map(|_| ()).map_err(err)
    })?;
    Ok(count)
}

/// Une page A4 par image (portrait ou paysage selon l'image), image ajustée à la page.
pub fn insert_images(pdf: &mut Pdf, at: usize, images: &[Vec<u8>]) -> Result<(), String> {
    let mut sized = vec![];
    for data in images {
        let img = mupdf::Image::from_bytes(data).map_err(|e| format!("Image illisible : {e}"))?;
        sized.push((data, img.width() as f32, img.height() as f32));
    }
    pdf.edit("Insérer des images", |doc| {
        for (k, (data, w, h)) in sized.iter().enumerate() {
            let (pw, ph) = if h >= w { A4 } else { (A4.1, A4.0) };
            let mut page = doc.new_page_at((at + k) as i32, Size::new(pw, ph)).map_err(err)?;
            let scale = (pw / w).min(ph / h);
            let (iw, ih) = (w * scale, h * scale);
            let rect = MRect::new((pw - iw) / 2.0, (ph - ih) / 2.0, (pw + iw) / 2.0, (ph + ih) / 2.0);
            let src = mupdf::pdf::PageImageSource::Bytes { data, format_hint: None };
            page.insert_image(doc, rect, src, Default::default()).map_err(err)?;
        }
        Ok(())
    })
}

/// Recadre `pages` : `rect` (points, repère affiché de la page `on`) devient la zone gardée ;
/// les autres pages perdent les mêmes marges, mesurées sur leur propre zone visible.
pub fn crop(pdf: &mut Pdf, on: usize, rect: Rect, pages: &[usize]) -> Result<(), String> {
    let (w, h) = pdf.page_sizes()[on];
    let margins = (rect.x0.max(0.0), rect.y0.max(0.0), (w - rect.x1).max(0.0), (h - rect.y1).max(0.0));
    pdf.edit("Recadrer", |doc| {
        for &i in pages {
            let mut page = doc.load_pdf_page(i as i32).map_err(err)?;
            let ctm = page.ctm().map_err(err)?; // repère PDF → repère affiché
            let inv = ctm.invert().ok_or("transformation de page invalide")?;
            let shown = page.bounds().map_err(err)?;
            let keep = MRect::new(shown.x0 + margins.0, shown.y0 + margins.1,
                                  shown.x1 - margins.2, shown.y1 - margins.3);
            if keep.x1 - keep.x0 < 10.0 || keep.y1 - keep.y0 < 10.0 {
                return Err("Zone trop petite pour certaines pages.".into());
            }
            let pdf_rect = keep.transform(&inv); // repère PDF (y vers le haut)
            page.set_crop_box(pdf_rect).map_err(err)?;
        }
        Ok(())
    })
}

pub fn reset_crop(pdf: &mut Pdf, pages: &[usize]) -> Result<(), String> {
    pdf.edit("Annuler le recadrage", |doc| {
        for &i in pages {
            let mut page = doc.load_pdf_page(i as i32).map_err(err)?;
            let media = page.media_box().map_err(err)?;
            page.set_crop_box(media).map_err(err)?;
        }
        Ok(())
    })
}

// ----- nouveaux fichiers ------------------------------------------------------------------

/// Contenu d'un nouveau PDF ne contenant que `pages` de `pdf`, dans cet ordre.
pub fn extract(pdf: &Pdf, pages: &[usize]) -> Result<Vec<u8>, String> {
    let mut copy = PdfDocument::from_bytes(&pdf.bytes_for_disk()?).map_err(err)?;
    if copy.needs_password().map_err(err)? {
        return Err("Retirez d'abord la protection par mot de passe.".into());
    }
    arrange(&mut copy, pages)?;
    let mut out = vec![];
    copy.write_to(&mut out).map_err(err)?;
    Ok(out)
}

/// Fusionne des documents (contenus déjà déverrouillés) dans l'ordre donné.
pub fn merge(sources: &[Vec<u8>]) -> Result<Vec<u8>, String> {
    let mut out_doc = PdfDocument::new();
    for data in sources {
        let src = PdfDocument::from_bytes(data).map_err(err)?;
        out_doc.insert_pdf(&src, InsertPdfOptions::default()).map_err(err)?;
    }
    let mut out = vec![];
    out_doc.write_to(&mut out).map_err(err)?;
    Ok(out)
}

/// Pages groupées pour scinder : toutes les `n` pages.
pub fn split_every(count: usize, n: usize) -> Vec<Vec<usize>> {
    (0..count).step_by(n.max(1)).map(|i| (i..(i + n.max(1)).min(count)).collect()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Pdf {
        Pdf::open(&std::fs::read("tests/fixtures/Été déjà vu.pdf").unwrap()).unwrap()
    }

    fn first_words(pdf: &Pdf) -> Vec<String> {
        (0..pdf.page_count()).map(|i| pdf.words(i).iter().map(|w| w.text.clone()).take(3)
            .collect::<Vec<_>>().join(" ")).collect()
    }

    #[test]
    fn ranges() {
        assert_eq!(parse_pages("1-2, 5, 4-", 6).unwrap(), vec![0, 1, 3, 4, 5]);
        assert_eq!(parse_pages("", 3).unwrap(), vec![0, 1, 2]);
        assert!(parse_pages("0", 3).is_err() && parse_pages("2-9", 3).is_err() && parse_pages("a", 3).is_err());
        assert_eq!(format_pages(&[0, 1, 2, 4]), "1-3, 5");
        assert_eq!(reorder(4, &[0], 3), vec![1, 2, 0, 3]);
        assert_eq!(split_every(5, 2), vec![vec![0, 1], vec![2, 3], vec![4]]);
    }

    #[test]
    fn edits_undo_and_redo() {
        let mut pdf = fixture();
        assert!(!pdf.modified());
        rotate(&mut pdf, &[0], 90).unwrap();
        assert_eq!(pdf.page_sizes()[0], (842.0, 595.0));
        assert!(pdf.modified() && pdf.undo_label() == Some("Pivoter des pages"));
        delete(&mut pdf, &[1]).unwrap();
        assert_eq!(first_words(&pdf), ["Bonjour page 1", "Bonjour page 3"]);
        assert!(delete(&mut pdf, &[0, 1]).is_err());
        pdf.undo().unwrap();
        pdf.undo().unwrap();
        assert!(!pdf.modified() && pdf.page_sizes()[0] == (595.0, 842.0) && pdf.page_count() == 3);
        pdf.redo().unwrap();
        assert!(pdf.modified() && pdf.redo_label() == Some("Supprimer des pages"));
        pdf.mark_saved();
        assert!(!pdf.modified());
    }

    #[test]
    fn move_insert_crop() {
        let mut pdf = fixture();
        assert_eq!(move_pages(&mut pdf, &[2], 0).unwrap(), vec![0]);
        assert_eq!(first_words(&pdf), ["Bonjour page 3", "Bonjour page 1", "Bonjour page 2"]);
        insert_blank(&mut pdf, 1, A4).unwrap();
        assert_eq!(pdf.page_count(), 4);
        let src = std::fs::read("tests/fixtures/Été déjà vu.pdf").unwrap();
        assert_eq!(insert_pdf(&mut pdf, 4, &src, None, "a.pdf").unwrap(), 3);
        assert_eq!(pdf.page_count(), 7);
        crop(&mut pdf, 0, Rect::new(50.0, 60.0, 400.0, 500.0), &[0, 2]).unwrap();
        assert_eq!(pdf.page_sizes()[0], (350.0, 440.0));
        assert_eq!(pdf.page_sizes()[2], (350.0, 440.0));
        reset_crop(&mut pdf, &[0]).unwrap();
        assert_eq!(pdf.page_sizes()[0], (595.0, 842.0));
    }

    #[test]
    fn images_extract_merge() {
        let mut pdf = fixture();
        let pix = mupdf::Pixmap::new_with_w_h(&mupdf::Colorspace::device_rgb(), 60, 40, false).unwrap();
        let mut png = vec![];
        pix.write_to(&mut png, mupdf::pixmap::ImageFormat::PNG).unwrap();
        insert_images(&mut pdf, 0, &[png]).unwrap();
        assert_eq!(pdf.page_sizes()[0], (842.0, 595.0)); // image en largeur : page paysage
        let out = extract(&pdf, &[3, 1]).unwrap();
        let extracted = Pdf::open(&out).unwrap();
        assert_eq!(first_words(&extracted), ["Bonjour page 3", "Bonjour page 1"]);
        let merged = Pdf::open(&merge(&[out.clone(), out]).unwrap()).unwrap();
        assert_eq!(merged.page_count(), 4);
    }
}
