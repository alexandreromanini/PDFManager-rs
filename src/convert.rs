//! Conversions : PDF → Word, images, texte ; images et documents Office → PDF ; réduction
//! de taille ; PDF/A (Ghostscript). Sans interface. Les fichiers produits sont nouveaux :
//! le document ouvert n'est pas modifié.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use mupdf::pdf::{Encryption, PdfDocument, PdfWriteOptions};
use mupdf::Size;

use crate::pdf::{Pdf, Rendered};
use crate::settings::write_atomic;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

// ----- depuis ce PDF -----------------------------------------------------------------------

/// Texte de toutes les pages, dans l'ordre de lecture ; saut de page entre les pages.
pub fn to_text(pdf: &Pdf) -> Result<String, String> {
    let mut pages = vec![];
    for i in 0..pdf.page_count() {
        let p = pdf.doc().load_page(i as i32).map_err(err)?;
        let tp = p.to_text_page(mupdf::text_page::TextPageFlags::empty()).map_err(err)?;
        pages.push(tp.to_text().map_err(err)?.trim().to_string());
    }
    Ok(pages.join("\n\u{c}\n") + "\n")
}

/// Image d'une page (telle qu'affichée) en PNG ou JPEG.
pub fn encode(r: &Rendered, jpeg: bool) -> Result<Vec<u8>, String> {
    let mut out = vec![];
    if jpeg {
        let enc = jpeg_encoder::Encoder::new(&mut out, 90);
        enc.encode(&r.rgb, r.width as u16, r.height as u16, jpeg_encoder::ColorType::Rgb).map_err(err)?;
    } else {
        let mut enc = png::Encoder::new(&mut out, r.width, r.height);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header().and_then(|mut w| w.write_image_data(&r.rgb)).map_err(err)?;
    }
    Ok(out)
}

/// Noms des images exportées : « Rapport - page 007.png ».
pub fn image_targets(stem: &str, pages: &[usize], count: usize, folder: &Path, ext: &str) -> Vec<PathBuf> {
    let width = count.to_string().len().max(3);
    pages.iter().map(|i| folder.join(format!("{stem} - page {:0width$}.{ext}", i + 1))).collect()
}

/// Word (.docx) : MuPDF reconstruit paragraphes et images ; mise en page approximative
/// pour les documents complexes. Écrit dans un fichier voisin puis renommé (atomique).
pub fn to_docx(pdf: &Pdf, target: &Path) -> Result<(), String> {
    let data = crate::docx::write(pdf)?;
    crate::settings::write_atomic(target, &data).map_err(err)
}

// ----- créer un PDF ------------------------------------------------------------------------

/// Une page A4 par image (portrait ou paysage selon l'image), image ajustée et centrée.
pub fn images_to_pdf(images: &[Vec<u8>]) -> Result<Vec<u8>, String> {
    let (a4w, a4h) = crate::edit::A4;
    let mut doc = PdfDocument::new();
    for data in images {
        let img = mupdf::Image::from_bytes(data).map_err(|e| format!("Image illisible : {e}"))?;
        let (w, h) = (img.width() as f32, img.height() as f32);
        let (pw, ph) = if h >= w { (a4w, a4h) } else { (a4h, a4w) };
        let k = (pw / w).min(ph / h);
        let rect = mupdf::Rect::new((pw - w * k) / 2.0, (ph - h * k) / 2.0, (pw + w * k) / 2.0, (ph + h * k) / 2.0);
        let mut page = doc.new_page(Size::new(pw, ph)).map_err(err)?;
        let src = mupdf::pdf::PageImageSource::Bytes { data, format_hint: None };
        page.insert_image(&mut doc, rect, src, Default::default()).map_err(err)?;
    }
    let mut opts = PdfWriteOptions::default();
    opts.set_garbage_level(3).set_compress(true);
    let mut out = vec![];
    doc.write_to_with_options(&mut out, opts).map_err(err)?;
    Ok(out)
}

// ----- réduire la taille ------------------------------------------------------------------

/// Niveaux : (clé, libellé, photos au-delà de … dpi, ramenées à … dpi, qualité JPEG).
pub const LEVELS: [(&str, &str, f32, f32, u8); 3] = [
    ("lossless", "Sans perte (structure, flux compressés)", 0.0, 0.0, 0),
    ("balanced", "Équilibré (photos à 150 dpi)", 200.0, 150.0, 75),
    ("strong", "Maximal (photos à 96 dpi)", 120.0, 96.0, 55),
];

/// Copie réduite : objets inutiles retirés, flux compressés et regroupés ; selon le niveau,
/// photos (JPEG) réencodées, et rééchantillonnées si leur résolution affichée dépasse le
/// seuil. Le chiffrement est conservé. (Pas de linéarisation : MuPDF ne la fait plus.)
pub fn compress(pdf: &Pdf, level: &str) -> Result<Vec<u8>, String> {
    let (_, _, threshold, target, quality) = *LEVELS.iter().find(|l| l.0 == level).ok_or("niveau inconnu")?;
    let mut copy = Pdf::open_with_password(&pdf.bytes()?, pdf.password()).map_err(|e| format!("{e:?}"))?;
    if threshold > 0.0 {
        copy.edit("Réduire", |doc| shrink_photos(doc, threshold, target, quality))?;
    }
    let mut opts = PdfWriteOptions::default();
    opts.set_encryption(Encryption::Keep).set_garbage_level(4).set_compress(true).set_compress_images(true)
        .set_compress_fonts(true).set_clean(true).set_object_streams(true);
    let mut out = vec![];
    copy.doc().write_to_with_options(&mut out, opts).map_err(err)?;
    Ok(out)
}

/// Résolution affichée la plus fine de chaque image de la page, par dimensions (pixels) :
/// la page indique où les images sont dessinées, pas leur numéro d'objet.
fn displayed_dpi(page: &mupdf::Page) -> std::collections::HashMap<(u32, u32), f32> {
    let mut out = std::collections::HashMap::new();
    let Ok(tp) = page.to_text_page(mupdf::TextPageFlags::PRESERVE_IMAGES) else { return out };
    for block in tp.blocks() {
        let (Some(img), r) = (block.image(), block.bounds()) else { continue };
        let inches = ((r.x1 - r.x0).max(r.y1 - r.y0) / 72.0).max(0.01);
        let px = img.width().max(img.height()) as f32;
        let dpi = out.entry((img.width(), img.height())).or_insert(0.0f32);
        *dpi = dpi.max(px / inches);
    }
    out
}

/// Photos JPEG réencodées à la qualité du niveau ; celles dont la résolution affichée
/// dépasse `threshold` sont d'abord réduites (par moitiés) vers `target`. Comme la version
/// Python (PyMuPDF `rewrite_images`). Une image n'est remplacée que si elle y gagne.
fn shrink_photos(doc: &mut PdfDocument, threshold: f32, target: f32, quality: u8) -> Result<(), String> {
    let mut done = std::collections::HashSet::new();
    for i in 0..doc.page_count().map_err(err)? {
        let page = doc.load_pdf_page(i).map_err(err)?;
        let shown = displayed_dpi(&doc.load_page(i).map_err(err)?);
        for info in page.images().map_err(err)? {
            // le filtre est lu avec sa barre oblique (« /DCTDecode »)
            if !done.insert(info.xref) || info.filter.as_deref().map(|f| f.trim_start_matches('/')) != Some("DCTDecode") {
                continue;
            }
            let dpi = shown.get(&(info.width, info.height)).copied().unwrap_or(0.0);
            let raw = doc.extract_image(info.xref).map_err(err)?;
            let Ok(img) = mupdf::Image::from_bytes(&raw.encoded) else { continue };
            let Ok(mut pix) = img.to_pixmap() else { continue };
            let mut level = 0;
            while dpi > threshold && dpi / 2f32.powi(level + 1) >= target {
                level += 1;
            }
            if level > 0 {
                pix.shrink(level).map_err(err)?;
            }
            let (w, h, n) = (pix.width() as usize, pix.height() as usize, pix.n() as usize);
            let color = match (n, pix.alpha()) {
                (3, false) => jpeg_encoder::ColorType::Rgb,
                (1, false) => jpeg_encoder::ColorType::Luma,
                _ => continue, // CMJN, transparence : laissé tel quel
            };
            let stride = pix.stride() as usize;
            let mut samples = Vec::with_capacity(w * h * n);
            for row in pix.samples().chunks(stride).take(h) {
                samples.extend_from_slice(&row[..w * n]);
            }
            let mut jpeg = vec![];
            let mut enc = jpeg_encoder::Encoder::new(&mut jpeg, quality);
            // couleurs non sous-échantillonnées : même qualité que la version Python
            enc.set_sampling_factor(jpeg_encoder::SamplingFactor::R_4_4_4);
            enc.encode(&samples, w as u16, h as u16, color).map_err(err)?;
            if jpeg.len() < raw.encoded.len() {
                // En place, dans l'objet image : toutes les pages qui le partagent en profitent
                // (remplacer la ressource d'une page laissait l'ancienne image aux autres).
                let mut obj = doc.new_indirect(info.xref, 0).map_err(err)?;
                obj.write_raw_stream_buffer(&mupdf::Buffer::from_bytes(&jpeg).map_err(err)?).map_err(err)?;
                obj.dict_put("Filter", doc.new_name("DCTDecode").map_err(err)?).map_err(err)?;
                obj.dict_put("Width", doc.new_int(w as i32).map_err(err)?).map_err(err)?;
                obj.dict_put("Height", doc.new_int(h as i32).map_err(err)?).map_err(err)?;
                obj.dict_put("BitsPerComponent", doc.new_int(8).map_err(err)?).map_err(err)?;
                obj.dict_delete("DecodeParms").map_err(err)?;
            }
        }
    }
    Ok(())
}

// ----- programmes externes (non embarqués) -------------------------------------------------

pub const GS_URL: &str = "https://ghostscript.com/releases/gsdnld.html";
pub const LO_URL: &str = "https://fr.libreoffice.org/download/telecharger-libreoffice/";
pub const OFFICE: &[&str] = &["doc", "docx", "odt", "rtf", "xls", "xlsx", "ods", "ppt", "pptx", "odp", "txt"];

fn program_files() -> Vec<PathBuf> {
    ["ProgramFiles", "ProgramFiles(x86)"].iter().filter_map(std::env::var_os).map(PathBuf::from).collect()
}

fn which(name: &str) -> Option<PathBuf> {
    let exe = if cfg!(windows) { format!("{name}.exe") } else { name.to_string() };
    std::env::split_paths(&std::env::var_os("PATH")?).map(|d| d.join(&exe)).find(|p| p.is_file())
}

pub fn find_ghostscript() -> Option<PathBuf> {
    if let Some(p) = ["gswin64c", "gswin32c", "gs"].iter().find_map(|n| which(n)) {
        return Some(p);
    }
    // Installateur officiel : C:\Program Files\gs\gs10.04.0\bin\gswin64c.exe (hors PATH)
    let mut hits: Vec<PathBuf> = program_files().iter()
        .filter_map(|root| std::fs::read_dir(root.join("gs")).ok())
        .flat_map(|d| d.flatten().map(|e| e.path().join("bin").join("gswin64c.exe")))
        .filter(|p| p.is_file())
        .collect();
    hits.sort_by_key(|p| crate::updates::parse_version(&p.to_string_lossy()));
    hits.pop()
}

pub fn find_libreoffice() -> Option<PathBuf> {
    ["soffice", "libreoffice"].iter().find_map(|n| which(n)).or_else(|| {
        program_files().into_iter().map(|r| r.join(r"LibreOffice\program\soffice.exe")).find(|p| p.is_file())
    })
}

/// Lance un programme sans fenêtre console, avec une limite de temps. Erreur si son code
/// de retour n'est pas 0 (sa sortie est jointe).
fn run(cmd: &Path, args: &[String], timeout: Duration) -> Result<String, String> {
    let name = cmd.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let log = std::env::temp_dir().join(format!("pdfmanager-{}.log", std::process::id()));
    let file = std::fs::File::create(&log).map_err(err)?;
    let mut c = Command::new(cmd);
    c.args(args).stdin(Stdio::null()).stdout(file.try_clone().map_err(err)?).stderr(file);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = c.spawn().map_err(|e| format!("Impossible de lancer {name} : {e}"))?;
    let start = Instant::now();
    let status = loop {
        if let Some(s) = child.try_wait().map_err(err)? {
            break s;
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("{name} ne répond plus (plus de {} min).", timeout.as_secs() / 60));
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let output = std::fs::read_to_string(&log).unwrap_or_default();
    let _ = std::fs::remove_file(&log);
    if !status.success() {
        let tail: String = output.trim().chars().rev().take(800).collect::<Vec<_>>().into_iter().rev().collect();
        return Err(format!("{name} a échoué ({status}) :\n{tail}"));
    }
    Ok(output)
}

/// Dossier de travail temporaire, effacé à la fin (noms ASCII : Ghostscript et
/// LibreOffice gèrent mal les accents dans leurs arguments sous Windows).
struct Work(PathBuf);

impl Work {
    fn new() -> Result<Work, String> {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
        let dir = std::env::temp_dir().join(format!("pdfmanager-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).map_err(err)?;
        Ok(Work(dir))
    }
}

impl Drop for Work {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Document Office → PDF avec LibreOffice (profil utilisateur jetable : une instance déjà
/// ouverte de LibreOffice ne bloque pas la conversion).
pub fn office_to_pdf(source: &Path, soffice: &Path) -> Result<Vec<u8>, String> {
    let work = Work::new()?;
    let ext = source.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let src = work.0.join(format!("document.{ext}"));
    std::fs::copy(source, &src).map_err(|e| format!("Lecture impossible de « {} » : {e}", source.display()))?;
    let profile = format!("-env:UserInstallation=file:///{}",
                          work.0.join("profil").to_string_lossy().replace('\\', "/").trim_start_matches('/'));
    run(soffice, &[profile, "--headless".into(), "--norestore".into(), "--convert-to".into(), "pdf".into(),
                   "--outdir".into(), work.0.to_string_lossy().into_owned(), src.to_string_lossy().into_owned()],
        Duration::from_secs(600))?;
    std::fs::read(work.0.join("document.pdf"))
        .map_err(|_| format!("LibreOffice n'a pas produit de PDF pour « {} ».", source.display()))
}

/// PDF/A-2b par Ghostscript (polices intégrées, couleurs en RVB). Renvoie le PDF et ses
/// remarques (ex. éléments retirés). ponytail : la conformité est celle que garantit
/// Ghostscript ; pas de second contrôle (la version Python validait avec pikepdf).
pub fn to_pdfa(data: &[u8], gs: &Path) -> Result<(Vec<u8>, Vec<String>), String> {
    let work = Work::new()?;
    let (src, dst) = (work.0.join("in.pdf"), work.0.join("out.pdf"));
    std::fs::write(&src, data).map_err(err)?;
    let out = run(gs, &["-dPDFA=2".into(), "-dBATCH".into(), "-dNOPAUSE".into(), "-dSAFER".into(), "-dQUIET".into(),
                        "-sDEVICE=pdfwrite".into(), "-sColorConversionStrategy=RGB".into(),
                        "-dPDFACompatibilityPolicy=1".into(), format!("-sOutputFile={}", dst.display()),
                        src.to_string_lossy().into_owned()], Duration::from_secs(600))?;
    let notes = out.lines().map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect();
    Ok((std::fs::read(&dst).map_err(err)?, notes))
}

/// Écrit `data` dans `target` (atomique) ; message d'erreur lisible.
pub fn save(target: &Path, data: &[u8]) -> Result<(), String> {
    write_atomic(target, data).map_err(|e| format!("Impossible d'enregistrer « {} » : {e}.", target.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Pdf {
        Pdf::open(&std::fs::read("tests/fixtures/Été déjà vu.pdf").unwrap()).unwrap()
    }

    #[test]
    fn texte_images_word() {
        let pdf = fixture();
        let text = to_text(&pdf).unwrap();
        assert!(text.contains("Bonjour page 1") && text.contains('\u{c}'), "{text}");
        let r = pdf.render(0, 0.5, 0).unwrap();
        assert!(encode(&r, false).unwrap().starts_with(b"\x89PNG"));
        assert!(encode(&r, true).unwrap().starts_with(&[0xFF, 0xD8]));
        let t = image_targets("Été", &[0, 9], 12, Path::new("/x"), "png");
        assert!(t[1].ends_with("Été - page 010.png"));

        let dir = std::env::temp_dir().join(format!("pdfm-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let docx = dir.join("Été.docx");
        to_docx(&pdf, &docx).unwrap();
        let data = std::fs::read(&docx).unwrap();
        assert!(data.starts_with(b"PK"), "archive Word");
        assert!(data.windows(17).any(|w| w == b"word/document.xml"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn images_en_pdf_et_reduction() {
        let pdf = fixture();
        let jpg = encode(&pdf.render(0, 4.0, 0).unwrap(), true).unwrap();
        let png = std::fs::read("tests/fixtures/image.png").unwrap();
        let made = Pdf::open(&images_to_pdf(&[jpg, png]).unwrap()).unwrap();
        assert_eq!(made.page_count(), 2);
        let (w, h) = made.page_sizes()[1];
        assert!(w > h, "image en largeur : page paysage");

        let before = made.bytes_for_disk().unwrap().len();
        let small = compress(&made, "strong").unwrap();
        assert!(small.len() < before / 2, "photo réduite : {} / {before}", small.len());
        assert_eq!(Pdf::open(&small).unwrap().page_count(), 2);
        assert!(!compress(&fixture(), "lossless").unwrap().is_empty());
    }
}
