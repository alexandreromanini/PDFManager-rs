//! Document PDF : ouverture (fichier lu en mémoire), mot de passe, rendu, mots, recherche,
//! signets. Sans interface. Un document MuPDF reste dans le thread qui l'a ouvert : ce
//! module n'est utilisé que par le thread moteur (`engine`), les tests et l'autotest.

use mupdf::text_page::TextPageFlags;
use mupdf::{Colorspace, Document, Matrix, Outline, TextExtractOptions};

/// Rectangle en points, repère de la page affichée (rotation /Rotate comprise).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

impl Rect {
    pub fn new(x0: f32, y0: f32, x1: f32, y1: f32) -> Self {
        Self { x0: x0.min(x1), y0: y0.min(y1), x1: x0.max(x1), y1: y0.max(y1) }
    }
    pub fn width(&self) -> f32 {
        self.x1 - self.x0
    }
    pub fn height(&self) -> f32 {
        self.y1 - self.y0
    }
    pub fn contains(&self, x: f32, y: f32) -> bool {
        (self.x0..=self.x1).contains(&x) && (self.y0..=self.y1).contains(&y)
    }
}

impl From<mupdf::Rect> for Rect {
    fn from(r: mupdf::Rect) -> Self {
        Rect::new(r.x0, r.y0, r.x1, r.y1)
    }
}

impl From<mupdf::Quad> for Rect {
    fn from(q: mupdf::Quad) -> Self {
        let xs = [q.ul.x, q.ur.x, q.ll.x, q.lr.x];
        let ys = [q.ul.y, q.ur.y, q.ll.y, q.lr.y];
        let min = |v: [f32; 4]| v.into_iter().fold(f32::INFINITY, f32::min);
        let max = |v: [f32; 4]| v.into_iter().fold(f32::NEG_INFINITY, f32::max);
        Rect::new(min(xs), min(ys), max(xs), max(ys))
    }
}

/// Un mot de la page et sa position (ordre de lecture).
#[derive(Debug, Clone, PartialEq)]
pub struct Word {
    pub text: String,
    pub rect: Rect,
    pub block: usize,
    pub line: usize,
}

/// Un lien de la page : vers une page du document, ou une adresse (web, e-mail…).
#[derive(Debug, Clone, PartialEq)]
pub struct Link {
    pub rect: Rect,
    pub page: Option<usize>,
    pub uri: Option<String>,
}

/// Une entrée de la table des matières, aplatie : niveau (0 = premier), titre, page.
#[derive(Debug, Clone, PartialEq)]
pub struct Bookmark {
    pub level: usize,
    pub title: String,
    pub page: Option<usize>,
}

/// Image d'une page : RGB, `width` × `height` pixels, lignes contiguës.
pub struct Rendered {
    pub width: u32,
    pub height: u32,
    pub rgb: Vec<u8>,
}

/// Plafond de pixels par rendu (~120 Mo en RGB) : au-delà l'image est réduite (floue aux
/// zooms extrêmes plutôt que de saturer la mémoire), comme dans la version Python.
const MAX_RENDER_PIXELS: f32 = 40_000_000.0;

pub struct Pdf {
    doc: Document,
    sizes: Vec<(f32, f32)>,
}

#[derive(Debug, PartialEq)]
pub enum OpenError {
    NeedsPassword,
    Unreadable(String),
}

impl Pdf {
    /// Ouvre le contenu d'un fichier. `NeedsPassword` : rappeler avec `open_with_password`.
    pub fn open(data: &[u8]) -> Result<Pdf, OpenError> {
        Self::open_with_password(data, None)
    }

    pub fn open_with_password(data: &[u8], password: Option<&str>) -> Result<Pdf, OpenError> {
        let unreadable = |e: mupdf::Error| {
            OpenError::Unreadable(format!(
                "ce n'est pas un PDF valide, ou il est trop endommagé pour être lu ({e})"
            ))
        };
        let mut doc = Document::from_bytes(data, "application/pdf").map_err(unreadable)?;
        if doc.needs_password().map_err(unreadable)? {
            let Some(pw) = password else { return Err(OpenError::NeedsPassword) };
            if !doc.authenticate(pw).map_err(unreadable)? {
                return Err(OpenError::NeedsPassword);
            }
        }
        let count = doc.page_count().map_err(unreadable)?;
        let mut sizes = Vec::with_capacity(count.max(0) as usize);
        for i in 0..count {
            let b = doc.load_page(i).and_then(|p| p.bounds()).map_err(unreadable)?;
            sizes.push((b.x1 - b.x0, b.y1 - b.y0));
        }
        if sizes.is_empty() {
            return Err(OpenError::Unreadable("le document ne contient aucune page".into()));
        }
        Ok(Pdf { doc, sizes })
    }

    pub fn page_count(&self) -> usize {
        self.sizes.len()
    }

    /// Taille affichée de chaque page, en points.
    pub fn page_sizes(&self) -> &[(f32, f32)] {
        &self.sizes
    }

    /// Image de la page `index` ; `zoom` 1.0 = 72 dpi ; `rotation` : rotation d'affichage
    /// (0, 90, 180, 270), en plus de celle du document.
    pub fn render(&self, index: usize, zoom: f32, rotation: i32) -> Result<Rendered, String> {
        let (w, h) = self.sizes.get(index).ok_or("page inexistante")?;
        let mut zoom = zoom;
        let pixels = w * h * zoom * zoom;
        if pixels > MAX_RENDER_PIXELS {
            zoom *= (MAX_RENDER_PIXELS / pixels).sqrt();
        }
        let mut m = Matrix::new_scale(zoom, zoom);
        m.rotate(rotation as f32);
        let page = self.doc.load_page(index as i32).map_err(|e| e.to_string())?;
        let pix = page
            .to_pixmap(&m, &Colorspace::device_rgb(), false, true)
            .map_err(|e| e.to_string())?;
        let (width, height) = (pix.width(), pix.height());
        let (row, stride) = (width as usize * 3, pix.stride() as usize);
        let mut rgb = Vec::with_capacity(row * height as usize);
        for line in pix.samples().chunks(stride).take(height as usize) {
            rgb.extend_from_slice(&line[..row]);
        }
        Ok(Rendered { width, height, rgb })
    }

    /// Mots de la page, dans l'ordre de lecture.
    pub fn words(&self, index: usize) -> Vec<Word> {
        let Ok(page) = self.doc.load_page(index as i32) else { return vec![] };
        let opts = TextExtractOptions { flags: TextPageFlags::PRESERVE_WHITESPACE, ..Default::default() };
        page.words(opts)
            .map(|ws| {
                ws.into_iter()
                    .filter(|w| !w.text.trim().is_empty())
                    .map(|w| Word { text: w.text, rect: w.bounds.into(), block: w.block, line: w.line })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Liens de la page (zones cliquables).
    pub fn links(&self, index: usize) -> Vec<Link> {
        let Ok(page) = self.doc.load_page(index as i32) else { return vec![] };
        let Ok(links) = page.links() else { return vec![] };
        links
            .map(|l| Link {
                rect: l.bounds.into(),
                page: l.dest.map(|d| d.loc.page_number as usize),
                uri: (l.dest.is_none() && !l.uri.is_empty()).then_some(l.uri),
            })
            .collect()
    }

    /// Occurrences de `needle` sur la page (insensible à la casse, comme MuPDF).
    pub fn search(&self, index: usize, needle: &str) -> Vec<Rect> {
        let Ok(page) = self.doc.load_page(index as i32) else { return vec![] };
        page.search(needle, 1000)
            .map(|quads| quads.iter().map(|q| Rect::from(q.clone())).collect())
            .unwrap_or_default()
    }

    /// Table des matières aplatie (signets), pages numérotées depuis 0.
    pub fn bookmarks(&self) -> Vec<Bookmark> {
        fn walk(items: &[Outline], level: usize, out: &mut Vec<Bookmark>) {
            for o in items {
                out.push(Bookmark {
                    level,
                    title: o.title.clone(),
                    page: o.dest.as_ref().map(|d| d.loc.page_number as usize),
                });
                walk(&o.down, level + 1, out);
            }
        }
        let mut out = vec![];
        walk(&self.doc.outlines().unwrap_or_default(), 0, &mut out);
        out
    }
}

/// Mots sélectionnés en faisant glisser la souris de `a` à `b` (points de la page) : du mot
/// le plus proche du départ à celui le plus proche de l'arrivée, dans l'ordre de lecture.
pub fn select_words(words: &[Word], a: (f32, f32), b: (f32, f32)) -> Vec<Word> {
    if words.is_empty() {
        return vec![];
    }
    let nearest = |(x, y): (f32, f32)| {
        let dist = |w: &Word| {
            let dx = (w.rect.x0 - x).max(0.0).max(x - w.rect.x1);
            let dy = (w.rect.y0 - y).max(0.0).max(y - w.rect.y1);
            dy * 4.0 + dx // une autre ligne compte plus qu'un autre mot de la même ligne
        };
        (0..words.len())
            .min_by(|&i, &j| dist(&words[i]).total_cmp(&dist(&words[j])))
            .unwrap()
    };
    let (i, j) = (nearest(a), nearest(b));
    let (i, j) = (i.min(j), i.max(j));
    words[i..=j].to_vec()
}

/// Texte des mots sélectionnés : espaces entre les mots, retour à la ligne entre les lignes.
pub fn words_to_text(words: &[Word]) -> String {
    let mut out = String::new();
    let mut last: Option<(usize, usize)> = None;
    for w in words {
        if let Some(prev) = last {
            out.push(if prev == (w.block, w.line) { ' ' } else { '\n' });
        }
        out.push_str(&w.text);
        last = Some((w.block, w.line));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Vec<u8> {
        std::fs::read("tests/fixtures/Été déjà vu.pdf").expect("PDF de test")
    }

    #[test]
    fn opens_renders_and_reads_text() {
        let pdf = Pdf::open(&fixture()).unwrap();
        assert_eq!(pdf.page_count(), 3);
        let (w, h) = pdf.page_sizes()[0];
        assert!((w - 595.0).abs() < 1.0 && (h - 842.0).abs() < 1.0);
        let img = pdf.render(0, 0.5, 0).unwrap();
        assert_eq!(img.rgb.len(), (img.width * img.height * 3) as usize);
        let rotated = pdf.render(0, 0.5, 90).unwrap();
        assert_eq!((rotated.width, rotated.height), (img.height, img.width));
        let words: Vec<String> = pdf.words(1).into_iter().map(|w| w.text).collect();
        assert!(words.starts_with(&["Bonjour".into(), "page".into(), "2".into()]));
    }

    #[test]
    fn search_and_bookmarks() {
        let pdf = Pdf::open(&fixture()).unwrap();
        let hits = pdf.search(0, "renard");
        assert_eq!(hits.len(), 1);
        assert!(hits[0].width() > 10.0 && hits[0].y0 > 100.0);
        assert!(pdf.search(0, "absent").is_empty());
        let marks = pdf.bookmarks();
        assert_eq!(marks[0], Bookmark { level: 0, title: "Début".into(), page: Some(0) });
        assert_eq!((marks[1].level, marks[1].page), (1, Some(1)));
    }

    #[test]
    fn selection_follows_reading_order() {
        let pdf = Pdf::open(&fixture()).unwrap();
        let words = pdf.words(0);
        let first = &words[0].rect;
        let last = &words[words.len() - 1].rect;
        let all = select_words(&words, (last.x1, last.y1), (first.x0, first.y0)); // à rebours
        assert_eq!(words_to_text(&all), "Bonjour page 1\nLe renard brun saute.");
        let one = select_words(&words, (first.x0 + 1.0, first.y0 + 1.0), (first.x1, first.y0 + 1.0));
        assert_eq!(words_to_text(&one), "Bonjour");
    }

    #[test]
    fn password_and_garbage() {
        let mut doc = mupdf::pdf::PdfDocument::new();
        doc.new_page(mupdf::Size::A4).unwrap();
        let mut opts = mupdf::pdf::PdfWriteOptions::default();
        opts.set_encryption(mupdf::pdf::Encryption::Aes256).set_user_password("s3cret").set_owner_password("s3cret");
        let mut data = vec![];
        doc.write_to_with_options(&mut data, opts).unwrap();
        assert_eq!(Pdf::open(&data).err(), Some(OpenError::NeedsPassword));
        assert_eq!(Pdf::open_with_password(&data, Some("faux")).err(), Some(OpenError::NeedsPassword));
        assert_eq!(Pdf::open_with_password(&data, Some("s3cret")).unwrap().page_count(), 1);
        assert!(matches!(Pdf::open(b"pas un pdf"), Err(OpenError::Unreadable(_))));
    }
}
