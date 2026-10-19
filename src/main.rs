//! PDFManager en Rust, phase 0 (essai) : MuPDF pour le rendu, Slint pour l'interface.
//! `--selftest fichier.pdf` rend la première page sans ouvrir de fenêtre (CI Windows).

#![cfg_attr(windows, windows_subsystem = "windows")] // pas de console derrière la fenêtre

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use mupdf::{Colorspace, Document, Matrix};
use slint::{Image, ModelRc, Rgb8Pixel, SharedPixelBuffer, VecModel};

slint::include_modules!();

/// Image RGB d'une page ; `zoom` 1.0 = 72 dpi.
fn render_page(doc: &Document, index: i32, zoom: f32) -> Result<Image, mupdf::Error> {
    let page = doc.load_page(index)?;
    let pix = page.to_pixmap(&Matrix::new_scale(zoom, zoom), &Colorspace::device_rgb(), false, true)?;
    let (w, h) = (pix.width(), pix.height());
    let (row, stride) = (w as usize * 3, pix.stride() as usize);
    let mut buffer = SharedPixelBuffer::<Rgb8Pixel>::new(w, h);
    let dst = buffer.make_mut_bytes();
    for (y, line) in pix.samples().chunks(stride).take(h as usize).enumerate() {
        dst[y * row..(y + 1) * row].copy_from_slice(&line[..row]);
    }
    Ok(Image::from_rgb8(buffer))
}

struct State {
    doc: Option<Document>,
    name: String,
    zoom: f32,
}

fn refresh(ui: &AppWindow, state: &State) {
    let Some(doc) = &state.doc else { return };
    let count = doc.page_count().unwrap_or(0);
    let pages: Vec<Image> = (0..count)
        .filter_map(|i| render_page(doc, i, state.zoom).ok())
        .collect();
    ui.set_pages(ModelRc::new(VecModel::from(pages)));
    ui.set_status(format!("{} - {} page(s) - {} %", state.name, count, (state.zoom * 100.0).round()).into());
}

fn open(ui: &AppWindow, state: &mut State, path: &Path) {
    match Document::open(path.to_string_lossy().as_ref()) {
        Ok(doc) => {
            state.doc = Some(doc);
            state.name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            refresh(ui, state);
        }
        Err(e) => ui.set_status(format!("Impossible d'ouvrir le fichier : {e}").into()),
    }
}

fn selftest(path: &str) -> std::process::ExitCode {
    match Document::open(path).and_then(|doc| render_page(&doc, 0, 1.0)) {
        Ok(img) if img.size().width > 0 => std::process::ExitCode::SUCCESS,
        _ => std::process::ExitCode::FAILURE,
    }
}

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--selftest") {
        return selftest(args.get(2).map(String::as_str).unwrap_or_default());
    }
    let ui = AppWindow::new().expect("fenêtre");
    let state = Rc::new(RefCell::new(State { doc: None, name: String::new(), zoom: 1.0 }));
    if let Some(path) = args.get(1) {
        open(&ui, &mut state.borrow_mut(), Path::new(path));
    }
    let (weak, st) = (ui.as_weak(), state.clone());
    ui.on_open(move || {
        let ui = weak.unwrap();
        if let Some(path) = rfd::FileDialog::new().add_filter("Documents PDF", &["pdf"]).pick_file() {
            open(&ui, &mut st.borrow_mut(), &path);
        }
    });
    for (zoom_in, factor) in [(true, 1.25_f32), (false, 0.8)] {
        let (weak, st) = (ui.as_weak(), state.clone());
        let step = move || {
            let mut s = st.borrow_mut();
            s.zoom = (s.zoom * factor).clamp(0.25, 4.0);
            refresh(&weak.unwrap(), &s);
        };
        if zoom_in { ui.on_zoom_in(step) } else { ui.on_zoom_out(step) }
    }
    ui.run().expect("boucle de l'interface");
    std::process::ExitCode::SUCCESS
}
