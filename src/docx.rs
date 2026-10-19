//! PDF → Word (.docx) : paragraphes (police, taille, gras, italique, couleur, retrait,
//! centrage), images, tableaux (à bordures, d'après leurs filets, ou sans bordures, d'après
//! l'alignement des colonnes), un saut de page par page. Écrit à la main (WordprocessingML) :
//! le convertisseur de MuPDF aplatissait les tableaux et mélangeait l'ordre de lecture.

use std::io::Write;

use mupdf::text_page::TextBlockType;
use mupdf::{Font, TextCharFlags, TextPageFlags};

use crate::pdf::Pdf;

#[derive(Clone, PartialEq)]
struct Style {
    font: String,
    size: f32,
    bold: bool,
    italic: bool,
    color: u32,
}

#[derive(Clone)]
struct Run {
    text: String,
    style: Style,
}

#[derive(Clone)]
struct Ch {
    c: char,
    x0: f32,
    x1: f32,
    y0: f32,
    y1: f32,
    style: Style,
}

/// Une ligne de texte, découpée en morceaux là où un grand blanc la sépare (colonnes).
struct Line {
    block: usize,
    y0: f32,
    y1: f32,
    size: f32,
    segs: Vec<Vec<Ch>>,
}

impl Line {
    fn x0(&self) -> f32 {
        self.segs.first().and_then(|s| s.first()).map_or(0.0, |c| c.x0)
    }
    fn x1(&self) -> f32 {
        self.segs.last().and_then(|s| s.last()).map_or(0.0, |c| c.x1)
    }
}

enum Item {
    Para { indent: f32, center: bool, runs: Vec<Run> },
    /// (données, extension « jpeg » ou « png », largeur, hauteur en points)
    Image { data: Vec<u8>, ext: &'static str, w: f32, h: f32 },
    Table { widths: Vec<f32>, rows: Vec<Vec<Vec<Run>>>, borders: bool },
}

fn seg_x0(s: &[Ch]) -> f32 {
    s.first().map_or(0.0, |c| c.x0)
}
fn seg_x1(s: &[Ch]) -> f32 {
    s.last().map_or(0.0, |c| c.x1)
}

/// « ABCDEF+Arial-BoldMT » → « Arial » ; polices standard du PDF → équivalents Windows.
fn family(name: &str) -> String {
    let name = name.split_once('+').map_or(name, |(_, n)| n);
    let base = name.split(['-', ',']).next().unwrap_or(name);
    let base = base.strip_suffix("MT").unwrap_or(base);
    match base {
        "Helvetica" | "Helv" | "Arial" => "Arial".into(),
        "Times" | "TimesNewRoman" | "TimesNewRomanPS" | "Tiro" => "Times New Roman".into(),
        "Courier" | "CourierNew" | "CourierNewPS" | "Cour" => "Courier New".into(),
        _ => base.into(),
    }
}

fn style_of(font: Option<Font>, size: f32, argb: u32, flags: TextCharFlags) -> Style {
    let (name, bold, italic) = match font {
        Some(f) => {
            let n = f.name().to_string();
            let low = n.to_lowercase();
            (family(&n), f.is_bold() || low.contains("bold") || low.contains("black") || low.contains("heavy"),
             f.is_italic() || low.contains("italic") || low.contains("oblique"))
        }
        None => ("Arial".into(), false, false),
    };
    Style { font: name, size, bold: bold || flags.contains(TextCharFlags::BOLD), italic, color: argb & 0xFF_FFFF }
}

/// Morceaux d'une ligne : coupure là où le blanc dépasse 0,6 fois la taille du texte (une
/// espace en fait environ 0,25) ; seul un alignement sur plusieurs lignes en fera un tableau.
fn split(chars: Vec<Ch>) -> Vec<Vec<Ch>> {
    let mut segs: Vec<Vec<Ch>> = vec![];
    for ch in chars {
        let gap = segs.last().and_then(|s| s.iter().rev().find(|c| !c.c.is_whitespace())).map(|p| ch.x0 - p.x1);
        match (segs.last_mut(), gap) {
            (Some(s), Some(g)) if g <= 0.6 * ch.style.size.max(4.0) || ch.c.is_whitespace() => s.push(ch),
            (Some(s), None) => s.push(ch),
            _ => segs.push(vec![ch]),
        }
    }
    for s in &mut segs {
        while s.last().is_some_and(|c| c.c.is_whitespace()) {
            s.pop();
        }
        while s.first().is_some_and(|c| c.c.is_whitespace()) {
            s.remove(0);
        }
    }
    segs.retain(|s| !s.is_empty());
    segs
}

/// Caractères → morceaux de même style.
fn runs(chars: &[Ch], out: &mut Vec<Run>) {
    for ch in chars {
        match out.last_mut() {
            Some(r) if r.style == ch.style => r.text.push(ch.c),
            _ => out.push(Run { text: ch.c.to_string(), style: ch.style.clone() }),
        }
    }
}

/// Morceaux de texte mis bout à bout, séparés d'un espace.
fn join_runs<'a>(segs: impl Iterator<Item = &'a Vec<Ch>>) -> Vec<Run> {
    let mut out: Vec<Run> = vec![];
    for (k, s) in segs.enumerate() {
        if k > 0 {
            if let Some(r) = out.last_mut().filter(|r| !r.text.ends_with(' ')) {
                r.text.push(' ');
            }
        }
        runs(s, &mut out);
    }
    out
}

// ----- tableaux à bordures (filets) ------------------------------------------------------------

/// Filets horizontaux (y, x0, x1) et verticaux (x, y0, y1) tracés sur la page.
type Rulings = (Vec<(f32, f32, f32)>, Vec<(f32, f32, f32)>);

fn rulings(rects: &[mupdf::Rect]) -> Rulings {
    let (mut h, mut v) = (vec![], vec![]);
    for r in rects {
        let (w, ht) = (r.x1 - r.x0, r.y1 - r.y0);
        if ht <= 3.0 && w > 6.0 {
            h.push(((r.y0 + r.y1) / 2.0, r.x0, r.x1));
        } else if w <= 3.0 && ht > 6.0 {
            v.push(((r.x0 + r.x1) / 2.0, r.y0, r.y1));
        } else if w > 6.0 && ht > 6.0 {
            // cadre : ses quatre côtés
            h.push((r.y0, r.x0, r.x1));
            h.push((r.y1, r.x0, r.x1));
            v.push((r.x0, r.y0, r.y1));
            v.push((r.x1, r.y0, r.y1));
        }
    }
    (h, v)
}

fn snap(mut values: Vec<f32>, tol: f32) -> Vec<f32> {
    values.sort_by(f32::total_cmp);
    let mut out: Vec<f32> = vec![];
    for x in values {
        if out.last().is_none_or(|&l| x - l > tol) {
            out.push(x);
        }
    }
    out
}

/// Grilles formées par des filets qui se croisent : (colonnes x, lignes y).
fn grids(h: &[(f32, f32, f32)], v: &[(f32, f32, f32)]) -> Vec<(Vec<f32>, Vec<f32>)> {
    const TOL: f32 = 3.0;
    let n = h.len() + v.len();
    let mut parent: Vec<usize> = (0..n).collect();
    fn root(p: &mut [usize], mut i: usize) -> usize {
        while p[i] != i {
            p[i] = p[p[i]];
            i = p[i];
        }
        i
    }
    for (i, &(y, x0, x1)) in h.iter().enumerate() {
        for (j, &(x, y0, y1)) in v.iter().enumerate() {
            if x >= x0 - TOL && x <= x1 + TOL && y >= y0 - TOL && y <= y1 + TOL {
                let (a, b) = (root(&mut parent, i), root(&mut parent, h.len() + j));
                parent[a] = b;
            }
        }
    }
    let mut groups: std::collections::BTreeMap<usize, (Vec<f32>, Vec<f32>)> = Default::default();
    for i in 0..n {
        let r = root(&mut parent, i);
        let g = groups.entry(r).or_default();
        if i < h.len() {
            g.1.push(h[i].0);
        } else {
            g.0.push(v[i - h.len()].0);
        }
    }
    groups.into_values()
        .map(|(xs, ys)| (snap(xs, TOL), snap(ys, TOL)))
        .filter(|(xs, ys)| xs.len() >= 2 && ys.len() >= 2 && (xs.len() >= 3 || ys.len() >= 3))
        .collect()
}

/// Range dans la grille les caractères qui y tombent ; le tableau et son haut.
fn lattice(lines: &mut [Line], xs: &[f32], ys: &[f32]) -> Option<((usize, f32), Item)> {
    let (cols, nrows) = (xs.len() - 1, ys.len() - 1);
    let mut cells: Vec<Vec<Vec<Vec<Ch>>>> = vec![vec![vec![]; cols]; nrows];
    let mut found = 0;
    let mut first_block = usize::MAX;
    for line in lines.iter_mut() {
        let cy = (line.y0 + line.y1) / 2.0;
        let Some(r) = ys.windows(2).position(|w| cy > w[0] && cy < w[1]) else { continue };
        for seg in std::mem::take(&mut line.segs) {
            let mut keep = vec![];
            let mut per_col: Vec<Vec<Ch>> = vec![vec![]; cols];
            for ch in seg {
                let cx = (ch.x0 + ch.x1) / 2.0;
                match xs.windows(2).position(|w| cx > w[0] && cx < w[1]) {
                    Some(c) => per_col[c].push(ch),
                    None => keep.push(ch),
                }
            }
            for (c, chars) in per_col.into_iter().enumerate() {
                if !chars.is_empty() {
                    found += 1;
                    first_block = first_block.min(line.block);
                    cells[r][c].push(chars);
                }
            }
            if !keep.is_empty() {
                line.segs.push(keep);
            }
        }
    }
    if found < 2 {
        return None;
    }
    let rows = cells.into_iter()
        .map(|row| row.into_iter().map(|parts| join_runs(parts.iter())).collect())
        .collect();
    let widths = xs.windows(2).map(|w| w[1] - w[0]).collect();
    Some(((first_block, ys[0]), Item::Table { widths, rows, borders: true }))
}

// ----- tableaux sans bordures (colonnes alignées) ----------------------------------------------

/// Lignes consécutives découpées en autant de colonnes alignées (au moins 3 lignes) :
/// (première ligne, fin, colonnes).
fn streams(lines: &[Line], page_w: f32) -> Vec<(usize, usize, Vec<(f32, f32)>)> {
    let fits = |l: &Line| l.segs.len() >= 2 && l.segs.iter().all(|s| seg_x1(s) - seg_x0(s) < page_w * 0.45);
    let mut out = vec![];
    let mut i = 0;
    while i < lines.len() {
        if !fits(&lines[i]) {
            i += 1;
            continue;
        }
        let mut cols: Vec<(f32, f32)> = lines[i].segs.iter().map(|s| (seg_x0(s), seg_x1(s))).collect();
        let mut j = i + 1;
        while j < lines.len() {
            let (prev, l) = (&lines[j - 1], &lines[j]);
            if !fits(l) || l.segs.len() != cols.len() || l.y0 - prev.y1 > 2.5 * prev.size.max(l.size) {
                break;
            }
            let aligned = l.segs.iter().zip(&cols).all(|(s, &(a, b))| seg_x0(s) < b + 2.0 && seg_x1(s) > a - 2.0);
            let merged: Vec<(f32, f32)> = l.segs.iter().zip(&cols)
                .map(|(s, &(a, b))| (a.min(seg_x0(s)), b.max(seg_x1(s)))).collect();
            if !aligned || merged.windows(2).any(|w| w[0].1 >= w[1].0) {
                break; // colonnes qui se chevaucheraient : ce n'est pas un tableau
            }
            cols = merged;
            j += 1;
        }
        if j - i >= 3 {
            out.push((i, j, cols));
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

// ----- page ------------------------------------------------------------------------------------

/// Image du PDF → JPEG (photo opaque : fichier Word léger) ou PNG (transparence).
fn image_data(img: &mupdf::Image) -> Option<(Vec<u8>, &'static str)> {
    let pix = img.to_pixmap().ok()?;
    let (w, h, n, stride) = (pix.width(), pix.height(), pix.n() as usize, pix.stride() as usize);
    if w == 0 || h == 0 || (w as u64) * (h as u64) > 40_000_000 {
        return None;
    }
    let alpha = pix.alpha();
    let samples = pix.samples();
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h as usize {
        for px in samples[y * stride..y * stride + w as usize * n].chunks(n) {
            let a = if alpha { px[n - 1] } else { 255 };
            let (r, g, b) = match n - alpha as usize {
                1 => (px[0], px[0], px[0]),
                3 => (px[0], px[1], px[2]),
                4 => {
                    // CMJN → RVB (approché)
                    let k = 255 - px[3] as u32;
                    let f = |c: u8| ((255 - c as u32) * k / 255) as u8;
                    (f(px[0]), f(px[1]), f(px[2]))
                }
                _ => return None,
            };
            rgba.extend_from_slice(&[r, g, b, a]);
        }
    }
    let mut out = vec![];
    if rgba.chunks(4).all(|p| p[3] == 255) {
        let rgb: Vec<u8> = rgba.chunks(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
        let (w16, h16) = (u16::try_from(w).ok()?, u16::try_from(h).ok()?);
        jpeg_encoder::Encoder::new(&mut out, 88).encode(&rgb, w16, h16, jpeg_encoder::ColorType::Rgb).ok()?;
        return Some((out, "jpeg"));
    }
    let mut enc = png::Encoder::new(&mut out, w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().ok()?.write_image_data(&rgba).ok()?;
    Some((out, "png"))
}

/// Lignes d'un même bloc à la même hauteur (cellules d'une rangée, que MuPDF sépare)
/// réunies en une seule, redécoupée en morceaux.
fn rows(lines: Vec<Line>) -> Vec<Line> {
    let mut out: Vec<Line> = vec![];
    for line in lines {
        let same = out.last().is_some_and(|p: &Line| {
            let overlap = p.y1.min(line.y1) - p.y0.max(line.y0);
            p.block == line.block && overlap > 0.5 * (line.y1 - line.y0).min(p.y1 - p.y0)
        });
        if same {
            let p = out.last_mut().expect("ligne précédente");
            let mut chars: Vec<Ch> = std::mem::take(&mut p.segs).into_iter().flatten()
                .chain(line.segs.into_iter().flatten()).collect();
            chars.sort_by(|a, b| a.x0.total_cmp(&b.x0));
            p.segs = split(chars);
            p.y0 = p.y0.min(line.y0);
            p.y1 = p.y1.max(line.y1);
            p.size = p.size.max(line.size);
        } else {
            out.push(line);
        }
    }
    out
}

/// Éléments d'une page dans l'ordre de lecture, taille de la page, marge gauche du texte.
fn page_items(pdf: &Pdf, index: usize) -> Result<(Vec<Item>, (f32, f32), f32), String> {
    let page = pdf.doc().load_page(index as i32).map_err(|e| e.to_string())?;
    let bounds = page.bounds().map_err(|e| e.to_string())?;
    let (pw, ph) = (bounds.x1 - bounds.x0, bounds.y1 - bounds.y0);
    let tp = page.to_text_page(TextPageFlags::PRESERVE_IMAGES | TextPageFlags::COLLECT_VECTORS)
        .map_err(|e| e.to_string())?;
    let mut lines = vec![];
    // clé de tri : ordre de lecture de MuPDF (bloc), puis hauteur dans le bloc
    let mut placed: Vec<((usize, f32), Item)> = vec![];
    let mut vectors = vec![];
    for (b, block) in tp.blocks().enumerate() {
        match block.r#type() {
            TextBlockType::Text => {
                for line in block.lines() {
                    let chars: Vec<Ch> = line.chars().filter_map(|c| {
                        let ch = c.char().filter(|ch| !ch.is_control())?;
                        let q = c.quad();
                        Some(Ch { c: ch, x0: q.ul.x.min(q.ll.x), x1: q.ur.x.max(q.lr.x), y0: q.ul.y.min(q.ur.y),
                                  y1: q.ll.y.max(q.lr.y), style: style_of(c.font(), c.size(), c.argb(), c.flags()) })
                    }).collect();
                    if chars.iter().all(|c| c.c.is_whitespace()) {
                        continue;
                    }
                    let size = chars.iter().map(|c| c.style.size).fold(0.0, f32::max);
                    let y0 = chars.iter().map(|c| c.y0).fold(f32::MAX, f32::min);
                    let y1 = chars.iter().map(|c| c.y1).fold(0.0, f32::max);
                    lines.push(Line { block: b, y0, y1, size, segs: split(chars) });
                }
            }
            TextBlockType::Image => {
                let r = block.bounds();
                let (w, h) = (r.x1 - r.x0, r.y1 - r.y0);
                if w > 4.0 && h > 4.0 {
                    if let Some((data, ext)) = block.image().as_ref().and_then(image_data) {
                        let k = w.min(pw) / w;
                        placed.push(((b, r.y0), Item::Image { data, ext, w: w * k, h: h * k }));
                    }
                }
            }
            TextBlockType::Vector => vectors.push(block.bounds()),
            _ => {}
        }
    }
    lines.retain(|l| !l.segs.is_empty());
    lines.sort_by(|a, b| a.block.cmp(&b.block).then(a.y0.total_cmp(&b.y0)).then(a.x0().total_cmp(&b.x0())));
    let mut lines = rows(lines);
    let left = lines.iter().map(Line::x0).fold(f32::MAX, f32::min).clamp(0.0, pw / 3.0);

    // tableaux à bordures, puis sans bordures
    let (h, v) = rulings(&vectors);
    for (xs, ys) in grids(&h, &v) {
        if let Some(t) = lattice(&mut lines, &xs, &ys) {
            placed.push(t);
        }
    }
    lines.retain(|l| !l.segs.is_empty());
    let mut in_table = vec![false; lines.len()];
    for (i, j, cols) in streams(&lines, pw) {
        let rows = lines[i..j].iter().map(|l| l.segs.iter().map(|s| join_runs(std::iter::once(s))).collect()).collect();
        let mut edges: Vec<f32> = vec![left.min(cols[0].0)];
        edges.extend(cols.windows(2).map(|w| (w[0].1 + w[1].0) / 2.0));
        edges.push(cols[cols.len() - 1].1.max(edges[edges.len() - 1] + 20.0));
        let widths = edges.windows(2).map(|w| w[1] - w[0]).collect();
        placed.push(((lines[i].block, lines[i].y0), Item::Table { widths, rows, borders: false }));
        in_table[i..j].iter_mut().for_each(|t| *t = true);
    }

    // paragraphes : lignes d'un même bloc, coupées aux puces et aux grands écarts
    let mut k = 0;
    while k < lines.len() {
        if in_table[k] {
            k += 1;
            continue;
        }
        let mut end = k + 1;
        while end < lines.len() && !in_table[end] {
            let (prev, l) = (&lines[end - 1], &lines[end]);
            let bullet = l.segs[0].first().is_some_and(|c| matches!(c.c, '•' | '◦' | '▪' | '–' | '·'));
            if l.block != lines[k].block || bullet || l.y0 - prev.y1 > 0.9 * prev.size.max(l.size) {
                break;
            }
            end += 1;
        }
        let para = &lines[k..end];
        let runs = join_runs(para.iter().flat_map(|l| l.segs.iter()));
        let x0 = para.iter().map(Line::x0).fold(f32::MAX, f32::min);
        let x1 = para.iter().map(Line::x1).fold(0.0, f32::max);
        let center = ((x0 + x1) / 2.0 - pw / 2.0).abs() < pw * 0.04 && x1 - x0 < pw * 0.6 && x0 - left > 20.0;
        placed.push(((lines[k].block, lines[k].y0), Item::Para { indent: if center { 0.0 } else { (x0 - left).max(0.0) }, center, runs }));
        k = end;
    }
    placed.sort_by(|a, b| a.0 .0.cmp(&b.0 .0).then(a.0 .1.total_cmp(&b.0 .1)));
    Ok((placed.into_iter().map(|(_, i)| i).collect(), (pw, ph), left))
}

// ----- WordprocessingML ------------------------------------------------------------------------

fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c if (c as u32) < 0x20 && c != '\t' => {}
            c => out.push(c),
        }
    }
    out
}

fn twips(pt: f32) -> i64 {
    (pt * 20.0).round() as i64
}

fn run_xml(r: &Run) -> String {
    let s = &r.style;
    let mut p = format!("<w:rFonts w:ascii=\"{0}\" w:hAnsi=\"{0}\" w:cs=\"{0}\"/>", esc(&s.font));
    if s.bold {
        p.push_str("<w:b/>");
    }
    if s.italic {
        p.push_str("<w:i/>");
    }
    if s.color != 0 {
        p.push_str(&format!("<w:color w:val=\"{:06X}\"/>", s.color));
    }
    p.push_str(&format!("<w:sz w:val=\"{0}\"/><w:szCs w:val=\"{0}\"/>", (s.size * 2.0).round().max(2.0) as i64));
    format!("<w:r><w:rPr>{p}</w:rPr><w:t xml:space=\"preserve\">{}</w:t></w:r>", esc(&r.text))
}

fn para_xml(runs: &[Run], ppr: &str) -> String {
    format!("<w:p><w:pPr>{ppr}</w:pPr>{}</w:p>", runs.iter().map(run_xml).collect::<String>())
}

fn image_xml(id: usize, w: f32, h: f32) -> String {
    let (cx, cy) = ((w * 12700.0) as i64, (h * 12700.0) as i64);
    format!("<w:p><w:r><w:drawing><wp:inline distT=\"0\" distB=\"0\" distL=\"0\" distR=\"0\"><wp:extent cx=\"{cx}\" cy=\"{cy}\"/>\
<wp:docPr id=\"{id}\" name=\"Image {id}\"/><a:graphic><a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/picture\">\
<pic:pic><pic:nvPicPr><pic:cNvPr id=\"{id}\" name=\"image{id}\"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill>\
<a:blip r:embed=\"rId{id}\"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/>\
<a:ext cx=\"{cx}\" cy=\"{cy}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData>\
</a:graphic></wp:inline></w:drawing></w:r></w:p>")
}

fn table_xml(widths: &[f32], rows: &[Vec<Vec<Run>>], borders: bool) -> String {
    let b = if borders { "single" } else { "none" };
    let mut x = format!("<w:tbl><w:tblPr><w:tblW w:w=\"0\" w:type=\"auto\"/><w:tblBorders>\
<w:top w:val=\"{b}\" w:sz=\"4\"/><w:left w:val=\"{b}\" w:sz=\"4\"/><w:bottom w:val=\"{b}\" w:sz=\"4\"/><w:right w:val=\"{b}\" w:sz=\"4\"/>\
<w:insideH w:val=\"{b}\" w:sz=\"4\"/><w:insideV w:val=\"{b}\" w:sz=\"4\"/></w:tblBorders><w:tblLayout w:type=\"fixed\"/></w:tblPr><w:tblGrid>");
    for w in widths {
        x.push_str(&format!("<w:gridCol w:w=\"{}\"/>", twips(*w)));
    }
    x.push_str("</w:tblGrid>");
    for row in rows {
        x.push_str("<w:tr>");
        for (c, w) in widths.iter().enumerate() {
            let cell = row.get(c).map(|r| para_xml(r, "<w:spacing w:before=\"0\" w:after=\"0\"/>"))
                .unwrap_or_else(|| "<w:p/>".into());
            x.push_str(&format!("<w:tc><w:tcPr><w:tcW w:w=\"{}\" w:type=\"dxa\"/></w:tcPr>{cell}</w:tc>", twips(*w)));
        }
        x.push_str("</w:tr>");
    }
    x.push_str("</w:tbl><w:p><w:pPr><w:spacing w:before=\"0\" w:after=\"0\"/></w:pPr></w:p>");
    x
}

/// Archive zip (deflate) avec empreintes CRC-32 ; noms en UTF-8.
fn zip(files: &[(String, Vec<u8>)]) -> Result<Vec<u8>, String> {
    let (mut out, mut central) = (vec![], vec![]);
    for (name, data) in files {
        let mut enc = flate2::write::DeflateEncoder::new(vec![], flate2::Compression::default());
        enc.write_all(data).map_err(|e| e.to_string())?;
        let packed = enc.finish().map_err(|e| e.to_string())?;
        let crc = crc32fast::hash(data);
        let offset = out.len() as u32;
        let header = |sig: u32, central: bool| {
            let mut h = sig.to_le_bytes().to_vec();
            if central {
                h.extend(20u16.to_le_bytes()); // version de l'auteur
            }
            for v in [20u16, 0x0800, 8, 0, 0x21] {
                h.extend(v.to_le_bytes()); // version requise, UTF-8, deflate, heure, date (1980)
            }
            for v in [crc, packed.len() as u32, data.len() as u32] {
                h.extend(v.to_le_bytes());
            }
            h.extend((name.len() as u16).to_le_bytes());
            h.extend(0u16.to_le_bytes());
            if central {
                h.extend([0u8; 8]); // commentaire, disque, attributs internes et externes
                h.extend(0u16.to_le_bytes());
                h.extend(offset.to_le_bytes());
            }
            h.extend(name.as_bytes());
            h
        };
        out.extend(header(0x0403_4b50, false));
        out.extend(&packed);
        central.extend(header(0x0201_4b50, true));
    }
    let start = out.len() as u32;
    out.extend(&central);
    out.extend(0x0605_4b50u32.to_le_bytes());
    for v in [0u16, 0, files.len() as u16, files.len() as u16] {
        out.extend(v.to_le_bytes());
    }
    out.extend((central.len() as u32).to_le_bytes());
    out.extend(start.to_le_bytes());
    out.extend(0u16.to_le_bytes());
    Ok(out)
}

/// Le document Word complet (contenu du fichier .docx).
pub fn write(pdf: &Pdf) -> Result<Vec<u8>, String> {
    let mut body = String::new();
    let mut media: Vec<(String, Vec<u8>)> = vec![];
    let mut section = (595.0f32, 842.0f32, 72.0f32);
    for i in 0..pdf.page_count() {
        let (items, size, left) = page_items(pdf, i)?;
        if i == 0 {
            section = (size.0, size.1, left.clamp(36.0, 108.0));
        } else {
            body.push_str("<w:p><w:r><w:br w:type=\"page\"/></w:r></w:p>");
        }
        for item in items {
            match item {
                Item::Para { indent, center, runs } => {
                    let mut ppr = String::from("<w:spacing w:before=\"0\" w:after=\"120\"/>");
                    if center {
                        ppr.push_str("<w:jc w:val=\"center\"/>");
                    } else if indent > 2.0 {
                        ppr.push_str(&format!("<w:ind w:left=\"{}\"/>", twips(indent)));
                    }
                    body.push_str(&para_xml(&runs, &ppr));
                }
                Item::Image { data, ext, w, h } => {
                    media.push((format!("word/media/image{}.{ext}", media.len() + 1), data));
                    body.push_str(&image_xml(media.len(), w, h));
                }
                Item::Table { widths, rows, borders } => body.push_str(&table_xml(&widths, &rows, borders)),
            }
        }
    }
    let (w, h, m) = section;
    let doc = format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\" \
xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" \
xmlns:wp=\"http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing\" \
xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" \
xmlns:pic=\"http://schemas.openxmlformats.org/drawingml/2006/picture\"><w:body>{body}\
<w:sectPr><w:pgSz w:w=\"{}\" w:h=\"{}\"/><w:pgMar w:top=\"{m}\" w:right=\"{m}\" w:bottom=\"{m}\" w:left=\"{m}\" \
w:header=\"708\" w:footer=\"708\" w:gutter=\"0\"/></w:sectPr></w:body></w:document>", twips(w), twips(h), m = twips(m));
    let rels: String = media.iter().enumerate()
        .map(|(k, (name, _))| format!("<Relationship Id=\"rId{}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/image\" Target=\"{}\"/>", k + 1, name.trim_start_matches("word/")))
        .collect();
    let mut files = vec![
        ("[Content_Types].xml".to_string(), "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
<Default Extension=\"xml\" ContentType=\"application/xml\"/><Default Extension=\"png\" ContentType=\"image/png\"/>\
<Default Extension=\"jpeg\" ContentType=\"image/jpeg\"/>\
<Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/>\
</Types>".as_bytes().to_vec()),
        ("_rels/.rels".to_string(), "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/>\
</Relationships>".as_bytes().to_vec()),
        ("word/_rels/document.xml.rels".to_string(), format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">{rels}</Relationships>").into_bytes()),
        ("word/document.xml".to_string(), doc.into_bytes()),
    ];
    files.extend(media);
    zip(&files)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Facture : titre, tableau à bordures de 4 × 4 cellules, puis une seconde page.
    #[test]
    fn tableau_et_ordre() {
        let pdf = Pdf::open(&std::fs::read("tests/fixtures/tableau.pdf").unwrap()).unwrap();
        let (items, _, _) = page_items(&pdf, 0).unwrap();
        let tables: Vec<_> = items.iter().filter_map(|i| match i {
            Item::Table { rows, borders, .. } => Some((rows, *borders)),
            _ => None,
        }).collect();
        assert_eq!(tables.len(), 1);
        let (rows, borders) = tables[0];
        assert!(borders);
        let text = |r: &Vec<Run>| r.iter().map(|r| r.text.as_str()).collect::<String>();
        assert_eq!(rows.iter().map(|r| r.iter().map(text).collect::<Vec<_>>()).collect::<Vec<_>>()[..2],
                   [vec!["Désignation", "Qté", "Prix unitaire", "Total"], vec!["Conseil", "3", "450,00 ·", "1 350,00 ·"]]);
        let Item::Para { runs, .. } = &items[0] else { panic!("titre d'abord") };
        assert!(runs[0].text.starts_with("Facture") && runs[0].style.bold && runs[0].style.size > 17.0);

        let data = write(&pdf).unwrap();
        assert!(data.starts_with(b"PK\x03\x04"));
        let has = |s: &[u8]| data.windows(s.len()).any(|w| w == s);
        assert!(has(b"word/document.xml") && has(b"[Content_Types].xml"));
    }
}
