//! Contrôleur, partie signatures (manuscrite, numérique, vérification) et comparaison.

use std::path::{Path, PathBuf};

use slint::{Image, ModelRc, Rgba8Pixel, SharedPixelBuffer, VecModel};

use super::{file_name, from_engine, App};
use crate::compare::{self, Change};
use crate::engine::DocId;
use crate::pdf::{Pdf, Rect};
use crate::signature::{self, Look, Place};
use crate::{settings, DiffData, SigData};

/// Comparaison en cours : document de départ (onglet) et second document (fichier).
pub(super) struct Comparison {
    a: DocId,
    b_path: PathBuf,
    changes: Vec<Change>,
}

/// Où signer, en attendant la fenêtre « Signer avec un certificat ».
pub(super) enum SignTarget {
    Field(String),
    New(usize, Rect),
}

fn folder(name: &str) -> PathBuf {
    let dir = settings::data_dir().join(name);
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Signatures manuscrites enregistrées (PNG), les plus récentes d'abord.
pub(super) fn saved_signatures() -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(folder("signatures")).map(|d| {
        d.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "png")).collect()
    }).unwrap_or_default();
    files.sort();
    files.reverse();
    files
}

fn trusted_certificates() -> Vec<x509_cert::Certificate> {
    use der::Decode;
    std::fs::read_dir(folder("certificats")).map(|d| {
        d.flatten().filter_map(|e| std::fs::read(e.path()).ok()).filter_map(|b| x509_cert::Certificate::from_der(&b).ok()).collect()
    }).unwrap_or_default()
}

/// PNG → image Slint.
fn png_image(png: &[u8]) -> Option<Image> {
    let decoder = png::Decoder::new(std::io::Cursor::new(png));
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    let rgba: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => buf[..info.buffer_size()].to_vec(),
        png::ColorType::Rgb => buf[..info.buffer_size()].chunks(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        _ => return None,
    };
    let pixels = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(&rgba, info.width, info.height);
    Some(Image::from_rgba8(pixels))
}

impl App {
    // ----- signatures manuscrites ------------------------------------------------------------

    pub(super) fn sync_signatures(&mut self) {
        let ui = self.ui();
        let images: Vec<Image> = saved_signatures().iter().filter_map(|p| std::fs::read(p).ok()).filter_map(|d| png_image(&d)).collect();
        ui.set_saved_sigs(ModelRc::new(VecModel::from(images)));
    }

    pub(super) fn sig_create(&mut self) {
        self.sig_strokes.clear();
        self.sig_png = None;
        let ui = self.ui();
        ui.set_sig_path("".into());
        ui.set_sig_preview(Image::default());
        ui.invoke_show_sig_create();
    }

    /// Dessin : `start` commence un nouveau trait.
    pub fn sig_draw(&mut self, x: f32, y: f32, start: bool) {
        if start || self.sig_strokes.is_empty() {
            self.sig_strokes.push(vec![]);
        }
        self.sig_strokes.last_mut().unwrap().push((x, y));
        let mut path = String::new();
        for s in &self.sig_strokes {
            for (k, (x, y)) in s.iter().enumerate() {
                path.push_str(&format!("{} {x} {y} ", if k == 0 { "M" } else { "L" }));
            }
        }
        self.ui().set_sig_path(path.into());
    }

    pub fn sig_clear(&mut self) {
        self.sig_strokes.clear();
        self.ui().set_sig_path("".into());
    }

    pub fn sig_typed(&mut self, text: String) {
        self.sig_png = signature::text_png(&text).ok();
        let img = self.sig_png.as_deref().and_then(png_image).unwrap_or_default();
        self.ui().set_sig_preview(img);
    }

    pub fn sig_import(&mut self) {
        let Some(path) = rfd::FileDialog::new().set_title("Image de la signature")
            .add_filter("Images", &["png", "jpg", "jpeg", "bmp", "gif", "tif", "tiff", "webp"]).pick_file() else { return };
        match std::fs::read(&path).map_err(|e| e.to_string()).and_then(|d| signature::transparent_white(&d, 200)) {
            Ok(png) => {
                self.ui().set_sig_preview(png_image(&png).unwrap_or_default());
                self.sig_png = Some(png);
            }
            Err(e) => self.error(&e),
        }
    }

    /// Enregistre la signature (mode 0 : dessinée, 1 : tapée, 2 : importée).
    pub fn sig_save(&mut self, mode: i32, w: f32, h: f32) {
        let png = match mode {
            0 => signature::strokes_png(&self.sig_strokes, w, h),
            _ => self.sig_png.clone().ok_or_else(|| "Créez d'abord la signature.".to_string()),
        };
        match png {
            Ok(png) => {
                let name = format!("signature-{}.png", chrono::Local::now().format("%Y%m%d-%H%M%S"));
                if let Err(e) = crate::settings::write_atomic(&folder("signatures").join(name), &png) {
                    return self.error(&format!("Enregistrement impossible : {e}"));
                }
                self.sync_signatures();
                self.toast("Signature enregistrée : cliquez-la dans le panneau, puis sur la page pour la placer.");
            }
            Err(e) => self.error(&e),
        }
    }

    /// Placer une signature enregistrée : comme une image (cadre tracé ou clic).
    pub fn sig_place(&mut self, i: usize) {
        let Some(data) = saved_signatures().get(i).and_then(|p| std::fs::read(p).ok()) else { return };
        if self.tab().is_none() {
            return;
        }
        self.pending_image = Some(data);
        self.set_tool("image");
        self.toast("Tracez l'emplacement de la signature (ou cliquez).");
    }

    pub fn sig_delete(&mut self, i: usize) {
        if let Some(p) = saved_signatures().get(i) {
            if super::edit_ui::ask("Signatures", "Supprimer cette signature enregistrée ?") {
                let _ = std::fs::remove_file(p);
                self.sync_signatures();
            }
        }
    }

    // ----- signature numérique -------------------------------------------------------------

    /// Fin du tracé (outil « Signer avec un certificat ») ou clic sur un champ vide.
    pub(super) fn digital_start(&mut self, target: SignTarget) {
        if self.tab().is_none() {
            return;
        }
        self.sign_target = Some(target);
        let ui = self.ui();
        let names: Vec<slint::SharedString> = std::iter::once("Aucune".into())
            .chain((1..=saved_signatures().len()).map(|k| format!("Signature {k}").into())).collect();
        ui.set_sig_choices(ModelRc::new(VecModel::from(names)));
        ui.set_pfx_path(self.settings.pfx.clone().into());
        ui.set_tsa_url(if self.settings.tsa.is_empty() { signature::DEFAULT_TSA.into() } else { self.settings.tsa.clone().into() });
        ui.invoke_show_digital();
    }

    pub fn pick_pfx(&mut self) {
        let mut d = rfd::FileDialog::new().set_title("Certificat de signature").add_filter("Certificat (.pfx, .p12)", &["pfx", "p12"]);
        if let Some(dir) = Path::new(&self.settings.pfx).parent() {
            d = d.set_directory(dir);
        }
        if let Some(p) = d.pick_file() {
            self.ui().set_pfx_path(p.to_string_lossy().into_owned().into());
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn digital_run(&mut self, pfx: String, password: String, reason: String, location: String, tsa: bool, tsa_url: String, image: usize) {
        let Some(target) = self.sign_target.take() else { return };
        let signer = match signature::load_signer(Path::new(&pfx), &password) {
            Ok(s) => s,
            Err(e) => {
                self.sign_target = Some(target);
                return self.error(&e);
            }
        };
        self.settings.pfx = pfx;
        self.settings.tsa = tsa_url.clone();
        self.settings.save();
        let Some(tab) = self.tab() else { return };
        let (id, path) = (tab.id, tab.path.clone());
        let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let mut d = rfd::FileDialog::new().set_title("Enregistrer le document signé").add_filter("Documents PDF", &["pdf"])
            .set_file_name(format!("{stem} - signé.pdf"));
        if let Some(dir) = path.parent() {
            d = d.set_directory(dir);
        }
        let Some(out) = d.save_file() else { return };
        let png = image.checked_sub(1).and_then(|k| saved_signatures().get(k).cloned()).and_then(|p| std::fs::read(p).ok());
        let place = match target {
            SignTarget::Field(n) => Place::Field(n),
            SignTarget::New(p, r) => Place::New(p, r),
        };
        let look = Look { lines: signature::appearance_lines(&signer.name(), chrono::Local::now(), &reason, &location), image: png };
        self.toast("Signature en cours…");
        self.engine.run(move |docs| {
            let data = docs.get(&id).ok_or("document fermé".to_string()).and_then(Pdf::bytes_for_disk);
            std::thread::spawn(move || {
                let tsa = tsa.then_some(tsa_url.trim()).filter(|u| !u.is_empty());
                let result = data.and_then(|d| signature::sign(&d, place, &signer, &look, &reason, &location, tsa))
                    .and_then(|signed| crate::convert::save(&out, &signed));
                from_engine(move |app| match result {
                    Ok(()) => {
                        crate::log(&format!("Signé : {}", out.display()));
                        app.toast(&format!("Document signé : {}", out.display()));
                        app.show_signatures_after_open = Some(out.clone());
                        app.open_path(out);
                    }
                    Err(e) => app.error(&format!("Signature impossible : {e}")),
                });
            });
        });
    }

    // ----- vérification ----------------------------------------------------------------------

    pub(super) fn verify_signatures(&mut self) {
        let Some(tab) = self.tab() else { return };
        let (id, modified) = (tab.id, tab.modified);
        let ui = self.ui();
        ui.set_sig_status("Vérification des signatures…".into());
        ui.set_sigs(ModelRc::default());
        self.engine.run(move |docs| {
            let Some(pdf) = docs.get(&id) else { return };
            let data = pdf.bytes();
            let password = pdf.password().map(String::from);
            std::thread::spawn(move || {
                let mut roots = signature::system_roots();
                roots.extend(trusted_certificates());
                let result = data.and_then(|d| signature::verify(&d, password.as_deref(), &roots));
                from_engine(move |app| {
                    if app.tab().is_none_or(|t| t.id != id) {
                        return;
                    }
                    let ui = app.ui();
                    match result {
                        Ok(list) => {
                            let rows: Vec<SigData> = list.iter().map(|s| SigData {
                                level: s.level().into(),
                                summary: s.summary().into(),
                                details: s.details().join("\n").into(),
                                can_trust: s.intact && !s.trusted,
                                can_view: s.intact && s.modified,
                            }).collect();
                            ui.set_sig_status(match list.len() {
                                0 => "Ce document n'est pas signé numériquement.".into(),
                                _ if modified => "État avec les modifications non enregistrées.".into(),
                                _ => "".into(),
                            });
                            ui.set_sigs(ModelRc::new(VecModel::from(rows)));
                            app.statuses = list;
                        }
                        Err(e) => ui.set_sig_status(format!("Vérification impossible : {e}").into()),
                    }
                });
            });
        });
    }

    pub fn sig_trust(&mut self, i: usize) {
        let Some(st) = self.statuses.get(i).cloned() else { return };
        if !super::edit_ui::ask("Faire confiance au certificat", &format!(
            "Considérer « {} » comme une identité vérifiée ?\n\nToutes les signatures faites avec ce certificat apparaîtront \
             comme valides. Ne le faites que si vous êtes sûr de sa provenance (votre propre certificat, ou remis en main \
             propre).", st.signer)) {
            return;
        }
        use sha2::Digest;
        let name: String = sha2::Sha256::digest(&st.certificate).iter().take(8).map(|b| format!("{b:02x}")).collect();
        if let Err(e) = crate::settings::write_atomic(&folder("certificats").join(format!("{name}.cer")), &st.certificate) {
            return self.error(&format!("Enregistrement impossible : {e}"));
        }
        self.verify_signatures();
    }

    /// Le fichier tel qu'il était au moment de cette signature, dans un nouvel onglet.
    pub fn sig_signed_version(&mut self, i: usize) {
        let Some(st) = self.statuses.get(i).cloned() else { return };
        let Some(tab) = self.tab() else { return };
        let (id, stem) = (tab.id, tab.path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default());
        self.engine.run(move |docs| {
            let data = docs.get(&id).ok_or("document fermé".to_string()).and_then(Pdf::bytes);
            from_engine(move |app| {
                let safe: String = st.signer.chars().filter(|c| !"\\/:*?\"<>|".contains(*c)).collect();
                let path = std::env::temp_dir().join("PDFManager-rs").join(format!("{stem} - version signée par {safe}.pdf"));
                let _ = std::fs::create_dir_all(path.parent().unwrap());
                match data.and_then(|d| crate::convert::save(&path, &d[..st.revision_end.min(d.len())])) {
                    Ok(()) => app.open_path(path),
                    Err(e) => app.error(&e),
                }
            });
        });
    }

    // ----- comparaison -------------------------------------------------------------------------

    pub(super) fn compare_start(&mut self) {
        let Some(tab) = self.tab() else { return };
        let a_path = tab.path.clone();
        let mut d = rfd::FileDialog::new().set_title("Comparer avec…").add_filter("Documents PDF", &["pdf"]);
        if let Some(dir) = a_path.parent() {
            d = d.set_directory(dir);
        }
        let Some(b_path) = d.pick_file() else { return };
        self.compare_with(b_path);
    }

    pub fn compare_with(&mut self, b_path: PathBuf) {
        let Some(tab) = self.tab() else { return };
        let (a, a_path) = (tab.id, tab.path.clone());
        if b_path == a_path {
            return self.error("Choisissez un autre document que celui affiché.");
        }
        self.toast("Comparaison…");
        self.engine.run(move |docs| {
            let Some(pa) = docs.get(&a) else { return };
            let words_a = compare::document_words(pa);
            let result = std::fs::read(&b_path).map_err(|e| e.to_string())
                .and_then(|d| Pdf::open(&d).map_err(|_| "Second document illisible ou protégé par mot de passe.".to_string()))
                .map(|pb| compare::compare(&words_a, &compare::document_words(&pb)));
            from_engine(move |app| match result {
                Ok(changes) => {
                    let n = changes.len();
                    app.comparison = Some(Comparison { a, b_path: b_path.clone(), changes });
                    app.sync_differences();
                    app.apply_comparison();
                    app.ui().set_rail_tab(4);
                    app.toast(&match n {
                        0 => "Aucune différence de texte.".to_string(),
                        n => format!("{n} différence{} : surlignées dans les deux documents.", if n > 1 { "s" } else { "" }),
                    });
                    app.open_path(b_path);
                }
                Err(e) => app.error(&e),
            });
        });
    }

    /// Surlignage des différences : mots retirés dans le premier document, ajoutés dans le second.
    pub(super) fn apply_comparison(&mut self) {
        let Some(c) = &self.comparison else { return };
        let old: Vec<(usize, Rect)> = c.changes.iter().flat_map(|ch| ch.old.iter().map(|w| (w.0, w.1))).collect();
        let new: Vec<(usize, Rect)> = c.changes.iter().flat_map(|ch| ch.new.iter().map(|w| (w.0, w.1))).collect();
        let (a, b_path) = (c.a, c.b_path.clone());
        for tab in self.tabs.iter_mut() {
            let hits = if tab.id == a { &old } else if tab.path == b_path { &new } else { continue };
            tab.hits = hits.clone();
            tab.hit_index = None;
            tab.query = "différences".into();
        }
        self.refresh_search_info();
        self.update_view();
    }

    pub(super) fn sync_differences(&mut self) {
        let rows: Vec<DiffData> = self.comparison.as_ref().map(|c| {
            c.changes.iter().map(|ch| {
                let pages = |ws: &[compare::Located]| ws.first().map_or(String::new(), |w| format!("p. {}", w.0 + 1));
                let short = |s: String| if s.chars().count() > 80 { format!("{}…", s.chars().take(80).collect::<String>()) } else { s };
                let (kind, text, page) = match ch.kind {
                    "added" => ("Ajouté", format!("« {} »", short(ch.new_text())), format!("second document, {}", pages(&ch.new))),
                    "removed" => ("Supprimé", format!("« {} »", short(ch.old_text())), format!("premier document, {}", pages(&ch.old))),
                    _ => ("Modifié", format!("« {} » → « {} »", short(ch.old_text()), short(ch.new_text())),
                          format!("{} → {}", pages(&ch.old), pages(&ch.new))),
                };
                DiffData { kind: kind.into(), text: text.into(), page: page.into() }
            }).collect()
        }).unwrap_or_default();
        self.ui().set_diffs(ModelRc::new(VecModel::from(rows)));
    }

    /// Aller à une différence : dans le premier document (ou le second pour un ajout).
    pub fn diff_clicked(&mut self, i: usize) {
        let Some(c) = &self.comparison else { return };
        let Some(ch) = c.changes.get(i) else { return };
        let in_old = !ch.old.is_empty();
        let index: usize = c.changes[..i].iter().map(|x| if in_old { x.old.len() } else { x.new.len() }).sum();
        let (a, b_path) = (c.a, c.b_path.clone());
        let Some(t) = self.tabs.iter().position(|t| if in_old { t.id == a } else { t.path == b_path }) else {
            return self.toast("Ce document a été fermé.");
        };
        self.select_tab(t);
        self.apply_comparison();
        if let Some(tab) = self.tab() {
            tab.hit_index = Some(index);
        }
        self.ui().set_rail_tab(4);
        self.show_hit();
        self.refresh_search_info();
    }

    /// Un document vient de s'ouvrir : second document comparé, ou document signé.
    pub(super) fn after_open(&mut self, path: &Path) {
        if self.comparison.as_ref().is_some_and(|c| c.b_path == path) {
            self.apply_comparison();
        }
        if self.show_signatures_after_open.as_deref() == Some(path) {
            self.show_signatures_after_open = None;
            if !self.settings.sidebar {
                self.toggle_sidebar();
            }
            self.ui().set_rail_tab(5);
            self.verify_signatures();
        }
        let _ = file_name(path);
    }
}
