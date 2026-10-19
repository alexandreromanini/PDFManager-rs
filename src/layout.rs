//! Mise en page de la vue (sans interface) : position de chaque page selon le zoom, la
//! rotation d'affichage et le mode (continu, page unique, double page) ; zoom ajusté à la
//! largeur ou à la page ; page courante et pages visibles d'après le défilement.

pub const SPACING: f32 = 12.0;
pub const ZOOM_MIN: f32 = 0.1;
pub const ZOOM_MAX: f32 = 8.0;
pub const ZOOM_STEPS: [f32; 12] = [0.1, 0.25, 0.5, 0.75, 1.0, 1.25, 1.5, 2.0, 3.0, 4.0, 6.0, 8.0];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Continuous,
    Single,
    Double,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Fit {
    None,
    #[default]
    Width,
    Page,
}

/// Une page placée dans la vue, en pixels logiques.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placed {
    pub index: usize,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Layout {
    pub pages: Vec<Placed>,
    pub width: f32,
    pub height: f32,
}

fn shown_size((w, h): (f32, f32), zoom: f32, rotation: i32) -> (f32, f32) {
    if rotation.rem_euclid(180) == 90 { (h * zoom, w * zoom) } else { (w * zoom, h * zoom) }
}

/// Lignes de pages affichées : une page par ligne, une seule page, ou deux par ligne.
fn rows(count: usize, mode: Mode, current: usize) -> Vec<Vec<usize>> {
    match mode {
        Mode::Single => vec![vec![current.min(count.saturating_sub(1))]],
        Mode::Double => (0..count).step_by(2).map(|i| (i..(i + 2).min(count)).collect()).collect(),
        Mode::Continuous => (0..count).map(|i| vec![i]).collect(),
    }
}

pub fn layout(sizes: &[(f32, f32)], zoom: f32, rotation: i32, mode: Mode, current: usize) -> Layout {
    let rows = rows(sizes.len(), mode, current);
    let mut measured = vec![];
    let mut width: f32 = 0.0;
    for row in &rows {
        let dims: Vec<(f32, f32)> = row.iter().map(|&i| shown_size(sizes[i], zoom, rotation)).collect();
        let w = dims.iter().map(|d| d.0).sum::<f32>() + SPACING * (row.len() as f32 - 1.0);
        width = width.max(w);
        measured.push((row, dims, w));
    }
    let mut pages = vec![];
    let mut y = SPACING;
    for (row, dims, w) in measured {
        let h = dims.iter().map(|d| d.1).fold(0.0, f32::max);
        let mut x = SPACING + (width - w) / 2.0;
        for (&index, &(pw, ph)) in row.iter().zip(&dims) {
            pages.push(Placed { index, x, y: y + (h - ph) / 2.0, w: pw, h: ph });
            x += pw + SPACING;
        }
        y += h + SPACING;
    }
    Layout { pages, width: width + 2.0 * SPACING, height: y }
}

/// Zoom qui fait tenir la ligne de la page courante dans la largeur (ou dans la page).
pub fn fit_zoom(sizes: &[(f32, f32)], rotation: i32, mode: Mode, current: usize, fit: Fit,
                viewport: (f32, f32)) -> Option<f32> {
    if fit == Fit::None || sizes.is_empty() {
        return None;
    }
    let row: Vec<usize> = match mode {
        Mode::Double => {
            let start = current - current % 2;
            (start..(start + 2).min(sizes.len())).collect()
        }
        _ => vec![current.min(sizes.len() - 1)],
    };
    let dims: Vec<(f32, f32)> = row.iter().map(|&i| shown_size(sizes[i], 1.0, rotation)).collect();
    let w = dims.iter().map(|d| d.0).sum::<f32>() + SPACING * (row.len() as f32 - 1.0);
    let h = dims.iter().map(|d| d.1).fold(0.0, f32::max);
    let mut zoom = (viewport.0 - 2.0 * SPACING - 2.0) / w; // -2 : pas d'ascenseur horizontal
    if fit == Fit::Page {
        zoom = zoom.min((viewport.1 - 2.0 * SPACING) / h);
    }
    Some(zoom.clamp(ZOOM_MIN, ZOOM_MAX))
}

/// Pages en partie visibles entre `top` et `top + height` (plus une marge de préchargement).
pub fn visible(layout: &Layout, top: f32, height: f32, margin: f32) -> Vec<Placed> {
    let (a, b) = (top - margin, top + height + margin);
    layout.pages.iter().filter(|p| p.y + p.h > a && p.y < b).copied().collect()
}

/// Page la plus visible (la plus grande surface affichée) ; la première en cas d'égalité.
pub fn current_page(layout: &Layout, top: f32, height: f32) -> Option<usize> {
    let shown = |p: &Placed| (p.y + p.h).min(top + height) - p.y.max(top);
    layout.pages.iter()
        .filter(|p| shown(p) > 0.0)
        .max_by(|a, b| shown(a).total_cmp(&shown(b)).then(b.index.cmp(&a.index)))
        .map(|p| p.index)
}

/// Défilement qui amène la page `index` en haut de la vue.
pub fn scroll_to(layout: &Layout, index: usize) -> f32 {
    layout.pages.iter().find(|p| p.index == index).map(|p| (p.y - SPACING).max(0.0)).unwrap_or(0.0)
}

pub fn zoom_in(zoom: f32) -> f32 {
    ZOOM_STEPS.into_iter().find(|&z| z > zoom + 1e-3).unwrap_or(ZOOM_MAX)
}

pub fn zoom_out(zoom: f32) -> f32 {
    ZOOM_STEPS.into_iter().rev().find(|&z| z < zoom - 1e-3).unwrap_or(ZOOM_MIN)
}

#[cfg(test)]
mod tests {
    use super::*;

    const A4: (f32, f32) = (595.0, 842.0);

    #[test]
    fn continuous_layout_stacks_pages() {
        let l = layout(&[A4, A4, (842.0, 595.0)], 1.0, 0, Mode::Continuous, 0);
        assert_eq!(l.pages.len(), 3);
        assert_eq!((l.pages[0].y, l.pages[1].y), (SPACING, SPACING * 2.0 + 842.0));
        assert_eq!(l.width, 842.0 + 2.0 * SPACING); // la page paysage, la plus large
        assert_eq!(l.pages[0].x, SPACING + (842.0 - 595.0) / 2.0); // centrée
    }

    #[test]
    fn modes_rotation_and_fit() {
        let sizes = [A4; 5];
        assert_eq!(layout(&sizes, 1.0, 0, Mode::Single, 3).pages[0].index, 3);
        let d = layout(&sizes, 1.0, 0, Mode::Double, 0);
        assert_eq!(d.pages.len(), 5);
        assert_eq!(d.pages[1].y, d.pages[0].y); // côte à côte
        let r = layout(&sizes, 1.0, 90, Mode::Continuous, 0);
        assert_eq!((r.pages[0].w, r.pages[0].h), (842.0, 595.0));
        let z = fit_zoom(&sizes, 0, Mode::Continuous, 0, Fit::Width, (619.0 + 2.0, 400.0)).unwrap();
        assert!((z - 1.0).abs() < 1e-3);
        let p = fit_zoom(&sizes, 0, Mode::Continuous, 0, Fit::Page, (2000.0, 866.0)).unwrap();
        assert!((p - 1.0).abs() < 1e-3);
        assert_eq!(fit_zoom(&sizes, 0, Mode::Continuous, 0, Fit::None, (1.0, 1.0)), None);
    }

    #[test]
    fn scrolling_finds_current_and_visible_pages() {
        let l = layout(&[A4; 10], 0.5, 0, Mode::Continuous, 0);
        let top = scroll_to(&l, 4);
        assert_eq!(current_page(&l, top, 600.0), Some(4));
        let shown: Vec<usize> = visible(&l, top, 600.0, 0.0).iter().map(|p| p.index).collect();
        assert_eq!(shown, vec![4, 5]);
        assert_eq!(zoom_in(1.0), 1.25);
        assert_eq!(zoom_out(1.0), 0.75);
        assert_eq!(zoom_in(8.0), ZOOM_MAX);
    }
}
