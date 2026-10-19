//! Contrôleur de la fenêtre : onglets, rendu des pages en arrière-plan (thread moteur),
//! vignettes, signets, recherche, sélection et copie, liens, thème, réglages.
//! Vit dans le thread de l'interface ; le moteur lui renvoie ses résultats par
//! `slint::invoke_from_event_loop`.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use slint::{
    ComponentHandle, Image, Model, ModelNotify, ModelRc, ModelTracker, Rgb8Pixel,
    SharedPixelBuffer, Timer, TimerMode, VecModel, Weak,
};

use crate::engine::{DocId, Engine};
use crate::layout::{self, Fit, Layout, Mode};
use crate::pdf::{self, Bookmark, Link, OpenError, Pdf, Rect, Word};
use crate::settings::Settings;
use crate::{AppWindow, Area, BookmarkData, PageData, RecentData, TabData, ThumbData};

const THUMB_WIDTH: f32 = 110.0;
const RAIL_WIDTH: f32 = 260.0 + 1.0 + 48.0 + 1.0;
const SCROLLBAR: f32 = 14.0;

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

/// Exécute `f` sur le contrôleur ; s'il est déjà occupé (appel réentrant venu de
/// l'interface), juste après.
pub fn with_app(f: impl FnOnce(&mut App) + 'static) {
    APP.with(|cell| match cell.try_borrow_mut() {
        Ok(mut guard) => {
            if let Some(app) = guard.as_mut() {
                f(app)
            }
        }
        Err(_) => Timer::single_shot(Duration::ZERO, move || with_app(f)),
    });
}

/// Depuis le thread moteur : `f` s'exécutera dans le thread de l'interface.
fn from_engine(f: impl FnOnce(&mut App) + Send + 'static) {
    let _ = slint::invoke_from_event_loop(move || with_app(f));
}

fn to_image(r: pdf::Rendered) -> Image {
    Image::from_rgb8(SharedPixelBuffer::<Rgb8Pixel>::clone_from_slice(&r.rgb, r.width, r.height))
}

/// Rectangle en points (repère de la page) → zone en pixels sur la page affichée
/// (zoom et rotation d'affichage).
fn to_view(r: &Rect, (w, h): (f32, f32), rotation: i32, zoom: f32) -> Area {
    let (x0, y0, x1, y1) = match rotation.rem_euclid(360) {
        90 => (h - r.y1, r.x0, h - r.y0, r.x1),
        180 => (w - r.x1, h - r.y1, w - r.x0, h - r.y0),
        270 => (r.y0, w - r.x1, r.y1, w - r.x0),
        _ => (r.x0, r.y0, r.x1, r.y1),
    };
    Area { x: x0 * zoom, y: y0 * zoom, w: (x1 - x0) * zoom, h: (y1 - y0) * zoom }
}

/// Point en pixels sur la page affichée → point en points (repère de la page).
fn from_view(px: f32, py: f32, (w, h): (f32, f32), rotation: i32, zoom: f32) -> (f32, f32) {
    let (x, y) = (px / zoom, py / zoom);
    match rotation.rem_euclid(360) {
        90 => (y, h - x),
        180 => (w - x, h - y),
        270 => (w - y, x),
        _ => (x, y),
    }
}

/// Vignettes du rail : la liste ne demande que les lignes visibles ; chacune est alors
/// rendue en arrière-plan (comme le modèle de vignettes de la version Python).
pub struct ThumbModel {
    sizes: Vec<(f32, f32)>,
    images: RefCell<HashMap<usize, Image>>,
    asked: RefCell<HashSet<usize>>,
    notify: ModelNotify,
    ask: Box<dyn Fn(usize)>,
}

impl Model for ThumbModel {
    type Data = ThumbData;

    fn row_count(&self) -> usize {
        self.sizes.len()
    }

    fn row_data(&self, i: usize) -> Option<ThumbData> {
        let (w, h) = *self.sizes.get(i)?;
        if !self.images.borrow().contains_key(&i) && self.asked.borrow_mut().insert(i) {
            (self.ask)(i);
        }
        Some(ThumbData {
            index: i as i32,
            image: self.images.borrow().get(&i).cloned().unwrap_or_default(),
            w: THUMB_WIDTH,
            h: (THUMB_WIDTH * h / w).min(THUMB_WIDTH * 3.0),
        })
    }

    fn model_tracker(&self) -> &dyn ModelTracker {
        &self.notify
    }
}

impl ThumbModel {
    fn set(&self, i: usize, image: Image) {
        self.images.borrow_mut().insert(i, image);
        self.notify.row_changed(i);
    }
}

type RenderKey = (DocId, usize, u32, i32); // document, page, échelle × 1000, rotation

struct Tab {
    id: DocId,
    path: PathBuf,
    name: String,
    sizes: Vec<(f32, f32)>,
    bookmarks: Vec<Bookmark>,
    zoom: f32,
    fit: Fit,
    mode: Mode,
    rotation: i32,
    current: usize,
    top: f32,
    layout: Layout,
    images: HashMap<usize, (u32, i32, Image)>,
    words: HashMap<usize, (Vec<Word>, Vec<Link>)>,
    query: String,
    search_gen: u64,
    hits: Vec<(usize, Rect)>,
    hit_index: Option<usize>,
    selection: Option<(usize, Vec<Word>)>,
    press: Option<(usize, (f32, f32), bool)>,
    thumbs: Rc<ThumbModel>,
}

struct PendingOpen {
    path: PathBuf,
    data: Arc<Vec<u8>>,
    tries: u32,
}

pub struct App {
    ui: Weak<AppWindow>,
    engine: Engine,
    settings: Settings,
    tabs: Vec<Tab>,
    current: usize,
    on_home: bool,
    next_id: DocId,
    viewport: (f32, f32),
    wanted: Arc<Mutex<HashSet<RenderKey>>>,
    pending: HashSet<RenderKey>,
    pending_words: HashSet<(DocId, usize)>,
    password: Option<PendingOpen>,
    toast_timer: Timer,
}

impl App {
    fn ui(&self) -> AppWindow {
        self.ui.upgrade().expect("fenêtre")
    }

    fn scale(&self) -> f32 {
        self.ui.upgrade().map(|u| u.window().scale_factor()).unwrap_or(1.0)
    }

    fn tab(&mut self) -> Option<&mut Tab> {
        if self.on_home { None } else { self.tabs.get_mut(self.current) }
    }

    // ----- ouverture, onglets ---------------------------------------------------------------

    pub fn open_dialog(&mut self) {
        let start = self.settings.recent.first().and_then(|p| Path::new(p).parent().map(Path::to_path_buf));
        let mut dialog = rfd::FileDialog::new().set_title("Ouvrir des PDF").add_filter("Documents PDF", &["pdf"]);
        if let Some(dir) = start {
            dialog = dialog.set_directory(dir);
        }
        for path in dialog.pick_files().unwrap_or_default() {
            self.open_path(path);
        }
    }

    pub fn open_path(&mut self, path: PathBuf) {
        let path = std::path::absolute(&path).unwrap_or(path);
        if let Some(i) = self.tabs.iter().position(|t| t.path == path) {
            self.select_tab(i);
            return;
        }
        match std::fs::read(&path) {
            Ok(data) => self.load(path, Arc::new(data), None, 0),
            Err(e) => self.error(&format!("Impossible d'ouvrir « {} » : {}.", file_name(&path), reason(&e))),
        }
    }

    fn load(&mut self, path: PathBuf, data: Arc<Vec<u8>>, password: Option<String>, tries: u32) {
        let id = self.next_id;
        self.next_id += 1;
        self.engine.run(move |docs| match Pdf::open_with_password(&data, password.as_deref()) {
            Ok(pdf) => {
                let sizes = pdf.page_sizes().to_vec();
                let marks = pdf.bookmarks();
                docs.insert(id, pdf);
                from_engine(move |app| app.on_opened(id, path, sizes, marks));
            }
            Err(OpenError::NeedsPassword) => {
                let tried = password.is_some();
                from_engine(move |app| app.ask_password(path, data, tries + tried as u32));
            }
            Err(OpenError::Unreadable(msg)) => {
                from_engine(move |app| app.error(&format!("Impossible d'ouvrir « {} » : {msg}.", file_name(&path))));
            }
        });
    }

    fn ask_password(&mut self, path: PathBuf, data: Arc<Vec<u8>>, tries: u32) {
        if tries >= 3 {
            self.error("Mot de passe : trois essais infructueux.");
            return;
        }
        let ui = self.ui();
        ui.set_password_prompt(if tries == 0 {
            format!("« {} » est protégé par un mot de passe :", file_name(&path)).into()
        } else {
            "Mot de passe incorrect. Réessayez :".into()
        });
        self.password = Some(PendingOpen { path, data, tries });
        ui.invoke_ask_password();
    }

    pub fn password_entered(&mut self, password: String) {
        if let Some(p) = self.password.take() {
            self.load(p.path, p.data, Some(password), p.tries);
        }
    }

    pub fn password_cancelled(&mut self) {
        self.password = None;
    }

    fn on_opened(&mut self, id: DocId, path: PathBuf, sizes: Vec<(f32, f32)>, bookmarks: Vec<Bookmark>) {
        let engine = self.engine.clone();
        let scale = self.scale();
        let thumb_sizes = sizes.clone();
        let ask = Box::new(move |i: usize| {
            let zoom = THUMB_WIDTH / thumb_sizes[i].0 * scale;
            engine.run(move |docs| {
                if let Some(Ok(r)) = docs.get(&id).map(|pdf| pdf.render(i, zoom, 0)) {
                    from_engine(move |app| app.on_thumb(id, i, r));
                }
            });
        });
        let thumbs = Rc::new(ThumbModel {
            sizes: sizes.clone(),
            images: RefCell::default(),
            asked: RefCell::default(),
            notify: ModelNotify::default(),
            ask,
        });
        self.tabs.push(Tab {
            id,
            name: file_name(&path),
            path: path.clone(),
            sizes,
            bookmarks,
            zoom: 1.0,
            fit: Fit::Width,
            mode: Mode::Continuous,
            rotation: 0,
            current: 0,
            top: 0.0,
            layout: Layout::default(),
            images: HashMap::new(),
            words: HashMap::new(),
            query: String::new(),
            search_gen: 0,
            hits: vec![],
            hit_index: None,
            selection: None,
            press: None,
            thumbs,
        });
        self.settings.add_recent(&path);
        self.settings.save();
        crate::log(&format!("Ouvert : {}", path.display()));
        self.select_tab(self.tabs.len() - 1);
    }

    pub fn select_tab(&mut self, i: usize) {
        if i >= self.tabs.len() {
            return;
        }
        self.current = i;
        self.on_home = false;
        self.sync_tabs();
        self.sync_document();
    }

    pub fn close_tab(&mut self, i: usize) {
        if i >= self.tabs.len() {
            return;
        }
        let tab = self.tabs.remove(i);
        self.engine.run(move |docs| {
            docs.remove(&tab.id);
        });
        if self.tabs.is_empty() {
            self.on_home = true;
        } else if self.current >= i && self.current > 0 {
            self.current -= 1;
        }
        self.sync_tabs();
        self.sync_document();
    }

    pub fn go_home(&mut self) {
        self.on_home = true;
        self.sync_tabs();
    }

    fn sync_tabs(&mut self) {
        let ui = self.ui();
        let tabs: Vec<TabData> = self.tabs.iter()
            .map(|t| TabData { name: t.name.clone().into(), path: t.path.to_string_lossy().into_owned().into() })
            .collect();
        ui.set_tabs(ModelRc::new(VecModel::from(tabs)));
        ui.set_current_tab(self.current as i32);
        ui.set_on_home(self.on_home || self.tabs.is_empty());
        let title = self.tab().map(|t| t.name.clone()).unwrap_or_default();
        ui.set_doc_title(title.into());
        ui.set_recent(ModelRc::new(VecModel::from(self.recent_rows())));
    }

    fn recent_rows(&self) -> Vec<RecentData> {
        self.settings.recent.iter()
            .map(PathBuf::from)
            .filter(|p| p.exists())
            .take(6)
            .map(|p| {
                let parts: Vec<String> = p.parent().into_iter()
                    .flat_map(|d| d.components().rev().take(2).collect::<Vec<_>>().into_iter().rev())
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect();
                RecentData {
                    name: file_name(&p).into(),
                    folder: parts.join(" › ").into(),
                    when: modified(&p).into(),
                    path: p.to_string_lossy().into_owned().into(),
                }
            })
            .collect()
    }

    /// Onglet courant affiché : signets, vignettes, mise en page.
    fn sync_document(&mut self) {
        let ui = self.ui();
        let Some(tab) = self.tab() else {
            ui.set_pages(ModelRc::default());
            return;
        };
        let marks: Vec<BookmarkData> = tab.bookmarks.iter()
            .map(|b| BookmarkData { title: b.title.clone().into(), level: b.level as i32, page: b.page.map_or(-1, |p| p as i32) })
            .collect();
        ui.set_bookmarks(ModelRc::new(VecModel::from(marks)));
        ui.set_thumbs(ModelRc::from(tab.thumbs.clone() as Rc<dyn Model<Data = ThumbData>>));
        ui.set_page_count(tab.sizes.len() as i32);
        ui.set_mode_index(match tab.mode { Mode::Continuous => 0, Mode::Single => 1, Mode::Double => 2 });
        let info = search_info(tab);
        ui.set_search_info(info.into());
        self.relayout(false);
    }

    // ----- mise en page et défilement -------------------------------------------------------

    pub fn viewport_resized(&mut self, w: f32, h: f32) {
        if w <= 1.0 || h <= 1.0 {
            return;
        }
        self.viewport = ((w - SCROLLBAR).max(100.0), h);
        if self.tab().is_some_and(|t| t.fit != Fit::None) {
            self.relayout(true);
        }
    }

    /// Recalcule la position des pages (zoom ajusté compris) ; `keep` : garde la page
    /// courante au même endroit de la vue.
    fn relayout(&mut self, keep: bool) {
        let viewport = self.viewport_or_guess();
        let ui = self.ui();
        let Some(tab) = self.tab() else { return };
        let anchor = tab.layout.pages.iter().find(|p| p.index == tab.current)
            .map(|p| (p.index, (tab.top - p.y) / p.h.max(1.0)));
        if let Some(z) = layout::fit_zoom(&tab.sizes, tab.rotation, tab.mode, tab.current, tab.fit, viewport) {
            tab.zoom = z;
        }
        tab.layout = layout::layout(&tab.sizes, tab.zoom, tab.rotation, tab.mode, tab.current);
        tab.top = match anchor {
            Some((index, rel)) if keep => tab.layout.pages.iter().find(|p| p.index == index)
                .map_or(0.0, |p| p.y + rel * p.h),
            _ => layout::scroll_to(&tab.layout, tab.current),
        }
        .max(0.0);
        ui.set_content_width(tab.layout.width);
        ui.set_content_height(tab.layout.height);
        ui.set_zoom_text(format!("{} %", (tab.zoom * 100.0).round()).into());
        let top = tab.top;
        ui.set_scroll_y(-top);
        self.update_view();
    }

    fn viewport_or_guess(&self) -> (f32, f32) {
        if self.viewport.0 > 1.0 {
            return self.viewport;
        }
        let ui = self.ui();
        let size = ui.window().size().to_logical(ui.window().scale_factor());
        let rail = if self.settings.sidebar { RAIL_WIDTH } else { 0.0 };
        ((size.width - rail - SCROLLBAR).max(200.0), (size.height - 98.0).max(200.0))
    }

    pub fn scrolled(&mut self, top: f32, height: f32) {
        if height > 1.0 {
            self.viewport.1 = height;
        }
        let vh = self.viewport.1;
        let ui = self.ui();
        let Some(tab) = self.tab() else { return };
        tab.top = top.max(0.0);
        if tab.mode != Mode::Single {
            if let Some(p) = layout::current_page(&tab.layout, tab.top, vh) {
                tab.current = p;
            }
        }
        ui.set_page_text((tab.current + 1).to_string().into());
        ui.set_current_page(tab.current as i32);
        self.update_view();
    }

    /// Pages visibles : images (rendues en arrière-plan si besoin), surlignages.
    fn update_view(&mut self) {
        let scale = self.scale();
        let vh = self.viewport.1.max(200.0);
        let ui = self.ui();
        let Some(tab) = (if self.on_home { None } else { self.tabs.get_mut(self.current) }) else { return };
        let key = ((tab.zoom * scale * 1000.0).round() as u32, tab.rotation);
        let shown = layout::visible(&tab.layout, tab.top, vh, vh / 2.0);
        let mut wanted = HashSet::new();
        let mut rows = vec![];
        let current_hit = tab.hit_index.and_then(|i| tab.hits.get(i)).copied();
        for p in &shown {
            let i = p.index;
            let size = tab.sizes[i];
            let image = tab.images.get(&i);
            if image.is_none_or(|(s, r, _)| (*s, *r) != key) {
                wanted.insert((tab.id, i, key.0, key.1));
            }
            let boxes = |rects: Vec<&Rect>| -> ModelRc<Area> {
                ModelRc::new(VecModel::from(rects.into_iter().map(|r| to_view(r, size, tab.rotation, tab.zoom)).collect::<Vec<_>>()))
            };
            rows.push(PageData {
                index: i as i32,
                x: p.x,
                y: p.y,
                w: p.w,
                h: p.h,
                image: image.map(|(_, _, img)| img.clone()).unwrap_or_default(),
                hits: boxes(tab.hits.iter().filter(|(pg, _)| *pg == i).map(|(_, r)| r).collect()),
                current: boxes(current_hit.iter().filter(|(pg, _)| *pg == i).map(|(_, r)| r).collect()),
                selection: boxes(tab.selection.iter().filter(|(pg, _)| *pg == i)
                    .flat_map(|(_, ws)| ws.iter().map(|w| &w.rect)).collect()),
            });
        }
        // Les images loin de la vue sont oubliées : la mémoire reste bornée.
        let keep: HashSet<usize> = layout::visible(&tab.layout, tab.top, vh, vh * 3.0).iter().map(|p| p.index).collect();
        tab.images.retain(|i, _| keep.contains(i));
        let (id, zoom, rotation) = (tab.id, tab.zoom * scale, tab.rotation);
        ui.set_pages(ModelRc::new(VecModel::from(rows)));
        *self.wanted.lock().unwrap() = wanted.clone();
        self.pending.retain(|k| wanted.contains(k));
        for k in wanted {
            if self.pending.insert(k) {
                let wanted = self.wanted.clone();
                self.engine.run(move |docs| {
                    if !wanted.lock().unwrap().contains(&k) {
                        return; // la vue a bougé entre-temps
                    }
                    if let Some(Ok(r)) = docs.get(&id).map(|pdf| pdf.render(k.1, zoom, rotation)) {
                        from_engine(move |app| app.on_rendered(k, r));
                    }
                });
            }
        }
    }

    fn on_rendered(&mut self, k: RenderKey, r: pdf::Rendered) {
        self.pending.remove(&k);
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == k.0) else { return };
        tab.images.insert(k.1, (k.2, k.3, to_image(r)));
        if self.tab().is_some_and(|t| t.id == k.0) {
            self.update_view();
        }
    }

    fn on_thumb(&mut self, id: DocId, i: usize, r: pdf::Rendered) {
        if let Some(tab) = self.tabs.iter().find(|t| t.id == id) {
            tab.thumbs.set(i, to_image(r));
        }
    }

    // ----- navigation, zoom, modes ----------------------------------------------------------

    pub fn go_to(&mut self, page: usize) {
        let Some(tab) = self.tab() else { return };
        tab.current = page.min(tab.sizes.len() - 1);
        tab.top = layout::scroll_to(&tab.layout, tab.current);
        if tab.mode == Mode::Single {
            self.relayout(false);
        } else {
            let top = tab.top;
            self.ui().set_scroll_y(-top);
            self.scrolled(top, 0.0);
        }
    }

    pub fn step_page(&mut self, delta: i32) {
        let Some(tab) = self.tab() else { return };
        let step = if tab.mode == Mode::Double { 2 } else { 1 };
        let target = (tab.current as i32 + delta * step).max(0) as usize;
        self.go_to(target);
    }

    pub fn page_entered(&mut self, text: &str) {
        match text.trim().parse::<usize>() {
            Ok(n) if n >= 1 => self.go_to(n - 1),
            _ => {
                let top = self.tab().map_or(0.0, |t| t.top);
                self.scrolled(top, 0.0); // valeur affichée rétablie
            }
        }
    }

    pub fn zoom_step(&mut self, delta: i32) {
        let Some(tab) = self.tab() else { return };
        let z = if delta > 0 { layout::zoom_in(tab.zoom) } else { layout::zoom_out(tab.zoom) };
        self.set_zoom(z);
    }

    pub fn set_zoom(&mut self, zoom: f32) {
        let Some(tab) = self.tab() else { return };
        tab.fit = Fit::None;
        tab.zoom = zoom.clamp(layout::ZOOM_MIN, layout::ZOOM_MAX);
        self.relayout(true);
    }

    pub fn zoom_entered(&mut self, text: &str) {
        let cleaned = text.replace('%', "").replace(',', ".");
        match cleaned.trim().parse::<f32>() {
            Ok(v) if v > 0.0 => self.set_zoom(v / 100.0),
            _ => self.relayout(true), // valeur affichée rétablie
        }
    }

    pub fn set_fit(&mut self, fit: i32) {
        let Some(tab) = self.tab() else { return };
        tab.fit = if fit == 2 { Fit::Page } else { Fit::Width };
        self.relayout(true);
    }

    pub fn rotate(&mut self, delta: i32) {
        let Some(tab) = self.tab() else { return };
        tab.rotation = (tab.rotation + delta).rem_euclid(360);
        tab.selection = None;
        self.relayout(true);
    }

    pub fn set_mode(&mut self, index: i32) {
        let Some(tab) = self.tab() else { return };
        tab.mode = match index { 1 => Mode::Single, 2 => Mode::Double, _ => Mode::Continuous };
        self.relayout(false);
    }

    pub fn wheel_zoom(&mut self, delta: f32) {
        if delta != 0.0 {
            self.zoom_step(if delta > 0.0 { 1 } else { -1 });
        }
    }

    // ----- recherche ------------------------------------------------------------------------

    pub fn search(&mut self, query: String) {
        let query = query.trim().to_string();
        let ui = self.ui();
        let Some(tab) = self.tab() else { return };
        if query == tab.query && !tab.hits.is_empty() {
            self.search_step(1);
            return;
        }
        tab.query = query.clone();
        tab.hits.clear();
        tab.hit_index = None;
        tab.search_gen += 1;
        let (id, generation, count) = (tab.id, tab.search_gen, tab.sizes.len());
        ui.set_search_info(if query.is_empty() { "".into() } else { "Recherche…".into() });
        self.update_view();
        if query.is_empty() {
            return;
        }
        self.engine.run(move |docs| {
            let Some(pdf) = docs.get(&id) else { return };
            for i in 0..count {
                let rects = pdf.search(i, &query);
                if !rects.is_empty() {
                    from_engine(move |app| app.on_hits(id, generation, i, rects));
                }
            }
            from_engine(move |app| app.on_search_done(id, generation));
        });
    }

    fn on_hits(&mut self, id: DocId, generation: u64, page: usize, rects: Vec<Rect>) {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id && t.search_gen == generation) else { return };
        tab.hits.extend(rects.into_iter().map(|r| (page, r)));
        if tab.hit_index.is_none() {
            tab.hit_index = Some(0);
            self.show_hit();
        } else {
            self.refresh_search_info();
            self.update_view();
        }
    }

    fn on_search_done(&mut self, id: DocId, generation: u64) {
        let Some(tab) = self.tabs.iter().find(|t| t.id == id && t.search_gen == generation) else { return };
        if tab.hits.is_empty() {
            let q = tab.query.clone();
            self.toast(&format!("Aucun résultat pour « {q} »."));
        }
        self.refresh_search_info();
    }

    fn refresh_search_info(&mut self) {
        let ui = self.ui();
        if let Some(tab) = self.tab() {
            ui.set_search_info(search_info(tab).into());
        }
    }

    pub fn search_step(&mut self, delta: i32) {
        let Some(tab) = self.tab() else { return };
        if tab.hits.is_empty() {
            return;
        }
        let n = tab.hits.len() as i32;
        tab.hit_index = Some((tab.hit_index.unwrap_or(0) as i32 + delta).rem_euclid(n) as usize);
        self.show_hit();
    }

    /// Amène le résultat courant dans la vue (au tiers supérieur).
    fn show_hit(&mut self) {
        let vh = self.viewport.1;
        let Some(tab) = self.tab() else { return };
        let Some(&(page, rect)) = tab.hit_index.and_then(|i| tab.hits.get(i)) else { return };
        if tab.mode == Mode::Single && page != tab.current {
            tab.current = page;
            self.relayout(false);
        }
        let Some(tab) = self.tab() else { return };
        if let Some(p) = tab.layout.pages.iter().find(|p| p.index == page) {
            let area = to_view(&rect, tab.sizes[page], tab.rotation, tab.zoom);
            tab.top = (p.y + area.y - vh / 3.0).max(0.0);
            let top = tab.top;
            self.ui().set_scroll_y(-top);
            self.scrolled(top, 0.0);
        }
        self.refresh_search_info();
    }

    // ----- sélection, copie, liens ----------------------------------------------------------

    fn page_point(&mut self, page: usize, x: f32, y: f32) -> Option<(f32, f32)> {
        let tab = self.tab()?;
        Some(from_view(x, y, *tab.sizes.get(page)?, tab.rotation, tab.zoom))
    }

    /// Mots et liens de la page : demandés une fois au moteur, puis gardés.
    fn ensure_words(&mut self, page: usize) {
        let Some(tab) = self.tab() else { return };
        let id = tab.id;
        if tab.words.contains_key(&page) || !self.pending_words.insert((id, page)) {
            return;
        }
        self.engine.run(move |docs| {
            if let Some(pdf) = docs.get(&id) {
                let (words, links) = (pdf.words(page), pdf.links(page));
                from_engine(move |app| app.on_words(id, page, words, links));
            }
        });
    }

    fn on_words(&mut self, id: DocId, page: usize, words: Vec<Word>, links: Vec<Link>) {
        self.pending_words.remove(&(id, page));
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id) {
            tab.words.insert(page, (words, links));
        }
    }

    pub fn page_press(&mut self, page: usize, x: f32, y: f32) {
        let Some(pt) = self.page_point(page, x, y) else { return };
        self.ensure_words(page);
        if let Some(tab) = self.tab() {
            tab.press = Some((page, pt, false));
            tab.selection = None;
        }
        self.update_view();
    }

    pub fn page_drag(&mut self, page: usize, x: f32, y: f32) {
        let Some(pt) = self.page_point(page, x, y) else { return };
        let Some(tab) = self.tab() else { return };
        let Some((start_page, start, _)) = tab.press else { return };
        if start_page != page {
            return; // ponytail : sélection limitée à la page de départ, comme en Python
        }
        tab.press = Some((page, start, true));
        if let Some((words, _)) = tab.words.get(&page) {
            let sel = pdf::select_words(words, start, pt);
            tab.selection = (!sel.is_empty()).then_some((page, sel));
            self.update_view();
        }
    }

    pub fn page_release(&mut self, page: usize, x: f32, y: f32) {
        let Some(pt) = self.page_point(page, x, y) else { return };
        let Some(tab) = self.tab() else { return };
        let click = matches!(tab.press.take(), Some((p, _, false)) if p == page);
        if !click {
            return;
        }
        let link = tab.words.get(&page)
            .and_then(|(_, links)| links.iter().find(|l| l.rect.contains(pt.0, pt.1)).cloned());
        if let Some(link) = link {
            self.follow(link);
        }
    }

    fn follow(&mut self, link: Link) {
        if let Some(page) = link.page {
            self.go_to(page);
            return;
        }
        let Some(uri) = link.uri else { return };
        let scheme = uri.split(':').next().unwrap_or("").to_ascii_lowercase();
        if !["http", "https", "mailto"].contains(&scheme.as_str()) {
            // file:, protocoles d'applications… : un PDF piégé pourrait lancer un programme
            self.error(&format!("Ce lien n'est pas une adresse web ni une adresse e-mail : \
                                 PDFManager ne l'ouvre pas, par sécurité.\n\n{uri}"));
            return;
        }
        let yes = rfd::MessageDialog::new()
            .set_title("Ouvrir le lien")
            .set_description(format!("Ouvrir {uri} ?"))
            .set_buttons(rfd::MessageButtons::YesNo)
            .show();
        if yes == rfd::MessageDialogResult::Yes {
            let _ = open::that(&uri);
        }
    }

    pub fn copy(&mut self) {
        let Some(tab) = self.tab() else { return };
        let Some((_, words)) = &tab.selection else { return };
        let text = pdf::words_to_text(words);
        match arboard::Clipboard::new().and_then(|mut c| c.set_text(text)) {
            Ok(()) => self.toast("Texte copié."),
            Err(e) => self.error(&format!("Copie impossible : {e}")),
        }
    }

    // ----- rail, thème, fenêtre, messages ---------------------------------------------------

    pub fn toggle_sidebar(&mut self) {
        self.settings.sidebar = !self.settings.sidebar;
        self.settings.save();
        self.ui().set_sidebar_visible(self.settings.sidebar);
    }

    pub fn set_theme(&mut self, mode: &str) {
        self.settings.theme = mode.into();
        self.settings.save();
        self.apply_theme();
    }

    pub fn toggle_theme(&mut self) {
        let dark = self.ui().get_dark();
        self.set_theme(if dark { "light" } else { "dark" });
    }

    fn apply_theme(&self) {
        let ui = self.ui();
        let dark = match self.settings.theme.as_str() {
            "dark" => true,
            "light" => false,
            _ => system_dark(&ui),
        };
        ui.set_dark(dark);
    }

    pub fn toggle_fullscreen(&self) {
        let w = self.ui();
        w.window().set_fullscreen(!w.window().is_fullscreen());
    }

    /// Lancement suivant (Explorateur) : fenêtre au premier plan, fichiers en onglets.
    pub fn bring_to_front(&mut self, paths: Vec<PathBuf>) {
        let ui = self.ui();
        ui.window().set_minimized(false);
        use slint::winit_030::WinitWindowAccessor;
        ui.window().with_winit_window(|w| w.focus_window());
        for p in paths {
            self.open_path(p);
        }
    }

    pub fn dropped(&mut self, path: PathBuf) {
        if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf")) {
            self.open_path(path);
        } else {
            self.toast(&format!("Seuls les PDF s'ouvrent ici ({} ignoré).", file_name(&path)));
        }
    }

    pub fn toast(&mut self, text: &str) {
        let ui = self.ui();
        ui.set_toast_text(text.into());
        ui.set_toast_visible(true);
        let ms = (3000 + 45 * text.chars().count() as u64).min(10_000);
        let weak = self.ui.clone();
        self.toast_timer.start(TimerMode::SingleShot, Duration::from_millis(ms), move || {
            if let Some(ui) = weak.upgrade() {
                ui.set_toast_visible(false);
            }
        });
    }

    fn error(&mut self, text: &str) {
        crate::log(text);
        rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Error)
            .set_title("PDFManager")
            .set_description(text)
            .show();
    }
}

fn search_info(tab: &Tab) -> String {
    match (tab.hits.len(), tab.hit_index) {
        (0, _) if tab.query.is_empty() => String::new(),
        (0, _) => "Aucun résultat".into(),
        (n, Some(i)) => format!("{} / {n}", i + 1),
        (n, None) => format!("{n}"),
    }
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Cause d'une erreur système en français, sans code ni chemin.
pub fn reason(e: &std::io::Error) -> String {
    use std::io::ErrorKind::*;
    match e.kind() {
        NotFound => "fichier introuvable (déplacé ou supprimé ?)".into(),
        PermissionDenied => "accès refusé (dossier protégé, ou fichier ouvert dans un autre programme)".into(),
        _ => e.to_string(),
    }
}

/// Date de modification à la française : « Aujourd'hui, 14:12 », « Hier, 09:30 », « 12 octobre ».
fn modified(p: &Path) -> String {
    const MONTHS: [&str; 12] = ["janvier", "février", "mars", "avril", "mai", "juin", "juillet",
                                "août", "septembre", "octobre", "novembre", "décembre"];
    use chrono::{Datelike, Local};
    let Ok(t) = std::fs::metadata(p).and_then(|m| m.modified()) else { return String::new() };
    let t: chrono::DateTime<Local> = t.into();
    let today = Local::now().date_naive();
    match (today - t.date_naive()).num_days() {
        0 => format!("Aujourd'hui, {}", t.format("%H:%M")),
        1 => format!("Hier, {}", t.format("%H:%M")),
        _ if t.year() == today.year() => format!("{} {}", t.day(), MONTHS[t.month0() as usize]),
        _ => format!("{} {} {}", t.day(), MONTHS[t.month0() as usize], t.year()),
    }
}

fn system_dark(ui: &AppWindow) -> bool {
    use slint::winit_030::{winit, WinitWindowAccessor};
    ui.window().with_winit_window(|w| w.theme() == Some(winit::window::Theme::Dark)).unwrap_or(false)
}

/// Crée le contrôleur et relie chaque action de la fenêtre.
pub fn install(ui: &AppWindow, files: Vec<PathBuf>) {
    let settings = Settings::load();
    ui.set_sidebar_visible(settings.sidebar);
    ui.set_version(env!("CARGO_PKG_VERSION").into());
    let app = App {
        ui: ui.as_weak(),
        engine: Engine::start(),
        settings,
        tabs: vec![],
        current: 0,
        on_home: true,
        next_id: 1,
        viewport: (0.0, 0.0),
        wanted: Arc::default(),
        pending: HashSet::new(),
        pending_words: HashSet::new(),
        password: None,
        toast_timer: Timer::default(),
    };
    app.apply_theme();
    APP.with(|cell| *cell.borrow_mut() = Some(app));
    with_app(|app| app.sync_tabs());

    ui.on_open(|| with_app(App::open_dialog));
    ui.on_open_recent(|p| with_app(move |a| a.open_path(PathBuf::from(p.as_str()))));
    ui.on_select_tab(|i| with_app(move |a| a.select_tab(i.max(0) as usize)));
    ui.on_close_tab(|i| with_app(move |a| a.close_tab(i.max(0) as usize)));
    ui.on_go_home(|| with_app(App::go_home));
    ui.on_toggle_theme(|| with_app(App::toggle_theme));
    ui.on_set_theme(|m| with_app(move |a| a.set_theme(m.as_str())));
    ui.on_search(|q| with_app(move |a| a.search(q.to_string())));
    ui.on_search_step(|d| with_app(move |a| a.search_step(d)));
    ui.on_page_entered(|t| with_app(move |a| a.page_entered(t.as_str())));
    ui.on_step_page(|d| with_app(move |a| a.step_page(d)));
    ui.on_zoom_step(|d| with_app(move |a| a.zoom_step(d)));
    ui.on_zoom_entered(|t| with_app(move |a| a.zoom_entered(t.as_str())));
    ui.on_fit(|f| with_app(move |a| a.set_fit(f)));
    ui.on_rotate(|d| with_app(move |a| a.rotate(d)));
    ui.on_mode_selected(|i| with_app(move |a| a.set_mode(i)));
    ui.on_scrolled(|top, h| with_app(move |a| a.scrolled(top, h)));
    ui.on_viewport_resized(|w, h| with_app(move |a| a.viewport_resized(w, h)));
    ui.on_wheel_zoom(|d| with_app(move |a| a.wheel_zoom(d)));
    ui.on_page_press(|p, x, y| with_app(move |a| a.page_press(p as usize, x, y)));
    ui.on_page_drag(|p, x, y| with_app(move |a| a.page_drag(p as usize, x, y)));
    ui.on_page_release(|p, x, y| with_app(move |a| a.page_release(p as usize, x, y)));
    ui.on_copy(|| with_app(App::copy));
    ui.on_thumb_clicked(|i| with_app(move |a| a.go_to(i.max(0) as usize)));
    ui.on_bookmark_clicked(|p| with_app(move |a| if p >= 0 { a.go_to(p as usize) }));
    ui.on_set_rail_tab({
        let weak = ui.as_weak();
        move |t| if let Some(ui) = weak.upgrade() { ui.set_rail_tab(t) }
    });
    ui.on_toggle_sidebar(|| with_app(App::toggle_sidebar));
    ui.on_toggle_fullscreen(|| with_app(|a| a.toggle_fullscreen()));
    ui.on_quit(|| {
        let _ = slint::quit_event_loop();
    });
    ui.on_password_entered(|pw| with_app(move |a| a.password_entered(pw.to_string())));
    ui.on_password_cancelled(|| with_app(App::password_cancelled));

    use slint::winit_030::{winit, EventResult, WinitWindowAccessor};
    ui.window().on_winit_window_event(|_, event| {
        if let winit::event::WindowEvent::DroppedFile(path) = event {
            let path = path.clone();
            with_app(move |a| a.dropped(path));
        }
        EventResult::Propagate
    });
    for f in files {
        with_app(move |a| a.open_path(f));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_and_page_coordinates_round_trip() {
        let size = (600.0, 800.0);
        let r = Rect::new(100.0, 50.0, 200.0, 70.0);
        for rot in [0, 90, 180, 270] {
            let a = to_view(&r, size, rot, 2.0);
            let (cx, cy) = from_view(a.x + a.w / 2.0, a.y + a.h / 2.0, size, rot, 2.0);
            assert!((cx - 150.0).abs() < 1e-3 && (cy - 60.0).abs() < 1e-3, "rotation {rot}");
        }
        assert_eq!(to_view(&r, size, 90, 1.0).w, 20.0); // largeur et hauteur échangées
    }

    #[test]
    fn reasons_are_readable() {
        let e = std::io::Error::from(std::io::ErrorKind::NotFound);
        assert!(reason(&e).starts_with("fichier introuvable"));
    }
}
