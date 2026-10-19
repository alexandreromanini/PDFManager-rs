//! Reconnaissance de texte (OCR) : Tesseract intégré à MuPDF, modèles fra + eng livrés avec
//! l'application. Sans interface. Comme core/ocr.py : le texte reconnu est écrit sur la
//! page, invisible (mode de rendu 3), aligné sur les mots de l'image ; recherche,
//! sélection et copie fonctionnent, l'aspect de la page ne change pas.

use std::path::PathBuf;

use mupdf::pdf::{InsertFontOptions, PdfDocument};
use mupdf::{ColorParams, Colorspace, DocumentWriter, Image, Matrix, Pixmap};

use crate::pdf::{Pdf, Rect};

pub const LANGUAGES: [(&str, &str); 3] = [("fra+eng", "Français et anglais"), ("fra", "Français"), ("eng", "Anglais")];

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Dossier des modèles : à côté de l'exécutable (installé), sinon celui du dépôt.
pub fn tessdata() -> PathBuf {
    let installed = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("tessdata")));
    installed.filter(|d| d.join("fra.traineddata").is_file())
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources").join("tessdata"))
}

/// Par défaut, seules les pages sans texte (scans) : ailleurs le texte serait dupliqué.
pub fn pages_to_process(pdf: &Pdf, pages: &[usize], all: bool) -> Vec<usize> {
    pages.iter().copied().filter(|&i| all || pdf.words(i).is_empty()).collect()
}

/// Image en niveaux de gris de la page telle qu'affichée, sans les commentaires :
/// (pixels, largeur, hauteur).
pub fn render(pdf: &Pdf, page: usize, dpi: f32) -> Result<(Vec<u8>, u32, u32), String> {
    let p = pdf.doc().load_page(page as i32).map_err(err)?;
    let pix = p.to_pixmap(&Matrix::new_scale(dpi / 72.0, dpi / 72.0), &Colorspace::device_gray(), false, false).map_err(err)?;
    let (w, h, stride) = (pix.width(), pix.height(), pix.stride() as usize);
    let mut gray = Vec::with_capacity((w * h) as usize);
    for row in pix.samples().chunks(stride).take(h as usize) {
        gray.extend_from_slice(&row[..w as usize]);
    }
    Ok((gray, w, h))
}

/// Mots reconnus sur l'image : (cadre en pixels de l'image, mot). Indépendant du document :
/// peut tourner dans n'importe quel thread.
pub fn recognize(gray: &[u8], w: u32, h: u32, dpi: f32, language: &str) -> Result<Vec<(Rect, String)>, String> {
    let mut pix = Pixmap::new_with_w_h(&Colorspace::device_gray(), w as i32, h as i32, false).map_err(err)?;
    let stride = pix.stride() as usize;
    for (y, row) in gray.chunks(w as usize).enumerate() {
        pix.samples_mut()[y * stride..y * stride + w as usize].copy_from_slice(row);
    }
    let image = Image::from_pixmap(&pix).map_err(err)?;
    let tmp = std::env::temp_dir().join(format!("pdfmanager-ocr-{}-{:?}.pdf", std::process::id(), std::thread::current().id()));
    let (pw, ph) = (w as f32 * 72.0 / dpi, h as f32 * 72.0 / dpi);
    let datadir = tessdata();
    let options = format!("ocr-language={language},ocr-datadir={},resolution={}", datadir.display(), dpi as i32);
    {
        let path = tmp.to_str().ok_or("chemin illisible")?; // UTF-8 : MuPDF le convertit sous Windows
        let mut writer = DocumentWriter::with_ocr(path, &options).map_err(err)?;
        let device = writer.begin_page(mupdf::Rect::new(0.0, 0.0, pw, ph)).map_err(err)?;
        device.fill_image(&image, &Matrix::new_scale(pw, ph), 1.0, ColorParams::default()).map_err(err)?;
        writer.end_page(device).map_err(err)?;
    }
    let data = std::fs::read(&tmp).map_err(err);
    let _ = std::fs::remove_file(&tmp);
    let doc = Pdf::open(&data?).map_err(|e| format!("{e:?}"))?;
    let k = dpi / 72.0;
    Ok(doc.words(0).into_iter()
        .map(|w| (Rect::new(w.rect.x0 * k, w.rect.y0 * k, w.rect.x1 * k, w.rect.y1 * k), w.text))
        .collect())
}

/// Littéral PDF en WinAnsi (hors latin : « ? »).
fn literal(word: &str) -> Vec<u8> {
    const EXTRA: [(char, u8); 27] = [
        ('€', 0x80), ('‚', 0x82), ('ƒ', 0x83), ('„', 0x84), ('…', 0x85), ('†', 0x86), ('‡', 0x87), ('ˆ', 0x88),
        ('‰', 0x89), ('Š', 0x8A), ('‹', 0x8B), ('Œ', 0x8C), ('Ž', 0x8E), ('‘', 0x91), ('’', 0x92), ('“', 0x93),
        ('”', 0x94), ('•', 0x95), ('–', 0x96), ('—', 0x97), ('˜', 0x98), ('™', 0x99), ('š', 0x9A), ('›', 0x9B),
        ('œ', 0x9C), ('ž', 0x9E), ('Ÿ', 0x9F),
    ];
    let mut out = vec![b'('];
    for c in word.chars() {
        let b = match c as u32 {
            0x20..=0x7E | 0xA0..=0xFF => c as u32 as u8,
            _ => EXTRA.iter().find(|e| e.0 == c).map_or(b'?', |e| e.1),
        };
        if matches!(b, b'\\' | b'(' | b')') {
            out.push(b'\\');
        }
        out.push(b);
    }
    out.push(b')');
    out
}

/// Couche de texte invisible : chaque mot étiré à la largeur de son image. `words` :
/// cadres en points, repère de la page affichée.
fn add_layer(doc: &mut PdfDocument, page: usize, words: &[(Rect, String)]) -> Result<(), String> {
    let mut p = doc.load_pdf_page(page as i32).map_err(err)?;
    let (font, _, _) = p.insert_font(doc, &InsertFontOptions::new("helv")).map_err(err)?;
    let to_pdf = p.ctm().map_err(err)?.invert().ok_or("transformation de page invalide")?;
    let mut ops = b"q BT 3 Tr\n".to_vec();
    for (r, word) in words {
        let size = r.height().max(1.0);
        let stretch = r.width() / crate::annot::text_width(word, size).max(0.01);
        // Texte droit à l'écran, ligne de base près du bas du mot, puis repère PDF.
        let m = Matrix::new(stretch * size, 0.0, 0.0, -size, r.x0, r.y1 - 0.2 * size) * &to_pdf;
        ops.extend(format!("{font} 1 Tf {} {} {} {} {} {} Tm ", m.a, m.b, m.c, m.d, m.e, m.f).bytes());
        ops.extend(literal(word));
        ops.extend(b" Tj\n");
    }
    ops.extend(b"ET Q\n");
    p.wrap_contents(doc).map_err(err)?; // isole le contenu existant (état graphique)
    p.insert_contents(doc, &ops, true).map_err(err)?;
    Ok(())
}

/// Une seule étape annulable pour tout le document. `results` : (page, mots en points).
/// Renvoie le nombre de pages où du texte a été ajouté.
pub fn add_text_layers(pdf: &mut Pdf, results: &[(usize, Vec<(Rect, String)>)]) -> Result<usize, String> {
    let todo: Vec<_> = results.iter().filter(|(_, w)| !w.is_empty()).collect();
    if todo.is_empty() {
        return Ok(0);
    }
    pdf.edit("Reconnaissance du texte (OCR)", |doc| {
        for (page, words) in &todo {
            add_layer(doc, *page, words)?;
        }
        Ok(())
    })?;
    Ok(todo.len())
}

/// Pixels de l'image → points de la page affichée.
pub fn to_points(words: Vec<(Rect, String)>, dpi: f32) -> Vec<(Rect, String)> {
    let k = 72.0 / dpi;
    words.into_iter().map(|(r, w)| (Rect::new(r.x0 * k, r.y0 * k, r.x1 * k, r.y1 * k), w)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Un « scan » : la page rendue en image, sans texte ; l'OCR le lui rend.
    #[test]
    fn scan_reconnu_puis_cherchable() {
        let src = Pdf::open(&std::fs::read("tests/fixtures/Été déjà vu.pdf").unwrap()).unwrap();
        let png = crate::convert::encode(&src.render(0, 300.0 / 72.0, 0).unwrap(), false).unwrap();
        let mut scan = Pdf::open(&crate::convert::images_to_pdf(&[png]).unwrap()).unwrap();
        assert!(scan.words(0).is_empty());
        assert_eq!(pages_to_process(&scan, &[0], false), [0]);

        let (gray, w, h) = render(&scan, 0, 300.0).unwrap();
        let words = to_points(recognize(&gray, w, h, 300.0, "fra+eng").unwrap(), 300.0);
        let texts: Vec<&str> = words.iter().map(|w| w.1.as_str()).collect();
        assert!(texts.contains(&"Bonjour") && texts.contains(&"renard"), "{texts:?}");

        assert_eq!(add_text_layers(&mut scan, &[(0, words)]).unwrap(), 1);
        let found = scan.search(0, "renard brun");
        assert_eq!(found.len(), 1, "recherche dans le texte reconnu");
        // Invisible : l'aspect ne change pas.
        let before = crate::convert::images_to_pdf(&[crate::convert::encode(&src.render(0, 300.0 / 72.0, 0).unwrap(), false).unwrap()]).unwrap();
        let a = Pdf::open(&before).unwrap().render(0, 0.5, 0).unwrap();
        let b = scan.render(0, 0.5, 0).unwrap();
        let diff = a.rgb.iter().zip(&b.rgb).filter(|(x, y)| x.abs_diff(**y) > 8).count();
        // (Seul le lissage des bords varie un peu ; un texte visible changerait bien plus.)
        assert!(diff < a.rgb.len() / 100, "{diff} pixels différents");
        assert_eq!(pages_to_process(&scan, &[0], false), Vec::<usize>::new(), "déjà du texte");
        assert!(literal("é(€)") == b"(\xe9\\(\x80\\))");
    }
}
