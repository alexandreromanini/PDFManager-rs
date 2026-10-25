//! Contrôleur, partie édition : enregistrer, annuler / rétablir, organiser les pages
//! (vignettes sélectionnables et déplaçables), recadrer, fusionner, scinder, extraire.
//! Les modifications s'exécutent dans le thread moteur ; la fenêtre ne fige pas.

use std::path::{Path, PathBuf};

use slint::{ModelRc, VecModel};

use super::{file_name, from_engine, reason, App, Tab};
use crate::edit;
use crate::engine::DocId;
use crate::pdf::{OpenError, Pdf, Rect};
use crate::settings::write_atomic;

pub(super) const IMAGES: &[&str] = &["png", "jpg", "jpeg", "tif", "tiff", "bmp", "gif", "webp"];

/// État d'un document après une modification, renvoyé par le moteur.
struct Edited {
    sizes: Vec<(f32, f32)>,
    bookmarks: Vec<crate::pdf::Bookmark>,
    comments: Vec<crate::annot::AnnotInfo>,
    fields: Vec<crate::forms::FieldInfo>,
    /// Signé et encore enregistré « en ajout » (signatures préservées).
    signed: bool,
    modified: bool,
    can_undo: bool,
    can_redo: bool,
    undo_label: String,
    redo_label: String,
}

fn state_of(pdf: &Pdf) -> Edited {
    Edited {
        sizes: pdf.page_sizes().to_vec(),
        bookmarks: pdf.bookmarks(),
        comments: crate::annot::list(pdf),
        fields: crate::forms::fields(pdf),
        signed: pdf.signed,
        modified: pdf.modified(),
        can_undo: pdf.undo_label().is_some(),
        can_redo: pdf.redo_label().is_some(),
        undo_label: pdf.undo_label().unwrap_or_default().to_string(),
        redo_label: pdf.redo_label().unwrap_or_default().to_string(),
    }
}

pub(super) use crate::dialogs::ask;

fn save_dialog(title: &str, suggested: &Path) -> Option<PathBuf> {
    crate::dialogs::save_file(title, ("Documents PDF", &["pdf"]), suggested)
}

fn write_error(target: &Path, e: &std::io::Error) -> String {
    format!("Impossible d'enregistrer « {} » dans « {} » : {}.", file_name(target),
            target.parent().map(|p| p.display().to_string()).unwrap_or_default(), reason(e))
}

impl App {
    pub(super) fn current_tab(&self) -> Option<&Tab> {
        if self.on_home { None } else { self.tabs.get(self.current) }
    }

    pub fn has_unsaved(&self) -> bool {
        self.tabs.iter().any(|t| t.modified)
    }

    /// Pages visées : vignettes sélectionnées, sinon la page affichée.
    fn targets(&self) -> Vec<usize> {
        self.current_tab().map(|t| if t.selected.is_empty() { vec![t.current] } else { t.selected.clone() })
            .unwrap_or_default()
    }

    /// Propriétés de la fenêtre qui dépendent de l'onglet courant.
    pub(super) fn sync_state(&mut self) {
        let ui = self.ui();
        let tab = self.current_tab();
        ui.set_modified(tab.is_some_and(|t| t.modified));
        ui.set_can_undo(tab.is_some_and(|t| t.can_undo));
        ui.set_can_redo(tab.is_some_and(|t| t.can_redo));
        // « Annuler « Rotation » » dans le menu, comme la version Python
        let label = |l: Option<&String>| l.filter(|l| !l.is_empty()).map_or(String::new(), |l| format!(" « {l} »"));
        ui.set_undo_label(label(tab.filter(|t| t.can_undo).map(|t| &t.undo_label)).into());
        ui.set_redo_label(label(tab.filter(|t| t.can_redo).map(|t| &t.redo_label)).into());
        ui.set_crop_mode(self.crop_mode);
        let info = match tab {
            Some(t) if !t.selected.is_empty() => format!("Pages sélectionnées : {}", edit::format_pages(&t.selected)),
            Some(t) => format!("Page affichée : {}", t.current + 1),
            None => String::new(),
        };
        ui.set_selection_info(info.into());
    }

    pub fn set_panel(&mut self, panel: i32) {
        self.ui().set_panel(panel);
        if panel == 1 && !self.settings.sidebar {
            self.toggle_sidebar(); // les vignettes servent à organiser
        }
        self.ui().set_rail_tab(0);
    }

    pub fn action(&mut self, name: &str) {
        if name == "delete" && self.delete_picked() {
            return;
        }
        if name.starts_with("save") {
            self.editor_commit(); // texte en cours d'écriture compris
        }
        let targets = self.targets();
        match name {
            "escape" => {
                if !self.job_stop() {
                    self.escape();
                }
            }
            "export-word" => self.export_word(),
            "export-text" => self.export_text(),
            "from-images" => self.from_images(),
            "from-office" => self.from_office(),
            "pdfa" => self.pdfa(),
            "ai-download" => self.ai_download(),
            "ai-stop" => self.ai_stop(),
            "export-images" | "compress" | "ocr" => {
                if self.current_tab().is_some() {
                    let ui = self.ui();
                    match name {
                        "export-images" => ui.invoke_show_images(),
                        "compress" => ui.invoke_show_compress(),
                        _ => ui.invoke_show_ocr(self.current_tab().is_some_and(|t| t.signed)),
                    }
                }
            }
            "translate" => self.translate_show(),
            "deepl-key" => {
                let _ = open::that(crate::deepl::KEY_URL);
            }
            "editor-commit" => self.editor_commit(),
            "field-next" => self.field_step(true),
            "field-prev" => self.field_step(false),
            "sign-create" => self.sig_create(),
            "compare" => self.compare_start(),
            "verify" => self.verify_signatures(),
            "redact-apply" => self.redact_apply(),
            "redact-text" => {
                if self.current_tab().is_some() {
                    self.pending_note = Some(super::annot_ui::NoteTarget::RedactText);
                    self.show_note("Caviarder toutes les occurrences de ce texte", "");
                }
            }
            "clean" => {
                if self.current_tab().is_some() {
                    self.ui().invoke_show_clean();
                }
            }
            "print" => self.print(),
            "properties" => self.properties(),
            "unprotect" => self.unprotect(None),
            "check-updates" => self.check_updates(true),
            "protect" => {
                if self.current_tab().is_some() {
                    self.ui().invoke_show_protect();
                }
            }
            "add-image" => self.add_image_start(),
            "header" => {
                if self.current_tab().is_some() {
                    self.ui().invoke_show_header();
                }
            }
            "watermark" => {
                if self.current_tab().is_some() {
                    self.ui().invoke_show_watermark();
                }
            }
            "undo" => self.history(true),
            "redo" => self.history(false),
            "save" => self.save(),
            "save-as" => self.save_as(false),
            "save-copy" => self.save_as(true),
            "rotate-left" => self.edit("", move |pdf| edit::rotate(pdf, &targets, -90).map(|_| Some(targets))),
            "rotate-right" => self.edit("", move |pdf| edit::rotate(pdf, &targets, 90).map(|_| Some(targets))),
            "delete" => self.delete_pages(targets),
            "insert-blank" => self.insert_blank(),
            "insert-pdf" => self.insert_pdf(),
            "insert-images" => self.insert_images(),
            "extract" => self.extract(targets),
            "crop" => {
                if self.current_tab().is_some() {
                    self.crop_mode = true;
                    self.sync_state();
                    self.toast("Tracez sur la page la zone à garder (Échap pour annuler).");
                }
            }
            "crop-cancel" => {
                self.crop_mode = false;
                if let Some(tab) = self.tab() {
                    tab.crop = None;
                }
                self.sync_state();
                self.update_view();
            }
            "reset-crop" => self.edit("Recadrage annulé.", move |pdf| edit::reset_crop(pdf, &targets).map(|_| Some(targets))),
            "merge" => self.merge_start(),
            "split"
                if self.current_tab().is_some() => {
                    self.ui().invoke_show_split();
                }
            _ => {}
        }
    }

    /// Modification du document courant dans le moteur. `op` renvoie éventuellement les
    /// pages à sélectionner ensuite ; `done` : message affiché en cas de succès.
    pub(super) fn edit(&mut self, done: &str, op: impl FnOnce(&mut Pdf) -> Result<Option<Vec<usize>>, String> + Send + 'static) {
        let done = done.to_string();
        self.edit_report(move |pdf| op(pdf).map(|select| (select, done)));
    }

    /// Comme `edit`, l'opération donnant aussi le message à afficher ensuite (bref : en
    /// notification ; plusieurs lignes, des avertissements : dans une fenêtre).
    pub(super) fn edit_report(&mut self, op: impl FnOnce(&mut Pdf) -> Result<(Option<Vec<usize>>, String), String> + Send + 'static) {
        let Some(tab) = self.current_tab() else { return };
        let id = tab.id;
        self.edit_doc(id, op);
    }

    /// Comme `edit_report`, sur un document précis : pour une opération qui finit plus
    /// tard (OCR…), l'utilisateur ayant pu changer d'onglet entre-temps.
    pub(super) fn edit_doc(&mut self, id: DocId, op: impl FnOnce(&mut Pdf) -> Result<(Option<Vec<usize>>, String), String> + Send + 'static) {
        self.engine.run(move |docs| {
            let Some(pdf) = docs.get_mut(&id) else { return };
            let result = op(pdf).map(|(select, message)| (state_of(pdf), select, message));
            from_engine(move |app| match result {
                Ok((state, select, message)) => {
                    app.on_edited(id, state, select);
                    if message.contains('\n') {
                        crate::dialogs::info("PDFManager", &message);
                    } else if !message.is_empty() {
                        app.toast(&message);
                    }
                }
                Err(e) => app.error(&e),
            });
        });
    }

    fn on_edited(&mut self, id: DocId, state: Edited, select: Option<Vec<usize>>) {
        let Some(index) = self.tabs.iter().position(|t| t.id == id) else { return };
        let tab = &mut self.tabs[index];
        tab.version += 1;
        tab.thumbs.reset(&state.sizes, tab.version);
        tab.current = tab.current.min(state.sizes.len() - 1);
        tab.sizes = state.sizes;
        tab.bookmarks = state.bookmarks;
        App::comments_changed(tab, state.comments);
        tab.fields = state.fields;
        // Signé à l'ouverture mais plus « en ajout » : l'enregistrement réécrira tout.
        tab.signature_lost = tab.signed && !state.signed;
        tab.modified = state.modified;
        tab.can_undo = state.can_undo;
        tab.can_redo = state.can_redo;
        tab.undo_label = state.undo_label;
        tab.redo_label = state.redo_label;
        tab.images.clear();
        tab.words.clear();
        tab.hits.clear();
        tab.hit_index = None;
        tab.query.clear();
        tab.selection = None;
        tab.crop = None;
        tab.selected = select.unwrap_or_default().into_iter().filter(|&p| p < tab.sizes.len()).collect();
        tab.thumbs.set_selected(&tab.selected);
        tab.picked_image = None;
        self.pending.retain(|k| k.0 != id);
        if self.tool == "images" && index == self.current {
            self.load_page_images(); // positions changées
        }
        if index == self.current && !self.on_home {
            self.sync_tabs();
            self.sync_document();
        } else {
            self.sync_tabs();
        }
    }

    fn history(&mut self, undo: bool) {
        self.edit("", move |pdf| if undo { pdf.undo() } else { pdf.redo() }.map(|_| None));
    }

    // ----- vignettes : sélection, glisser pour déplacer ------------------------------------

    pub fn thumb_press(&mut self, i: usize, ctrl: bool, shift: bool) {
        let Some(tab) = self.tab() else { return };
        if ctrl {
            if let Some(k) = tab.selected.iter().position(|&p| p == i) {
                tab.selected.remove(k);
            } else {
                tab.selected.push(i);
            }
            tab.anchor = Some(i);
        } else if shift {
            let a = tab.anchor.unwrap_or(tab.current);
            tab.selected = (a.min(i)..=a.max(i)).collect();
        } else if !tab.selected.contains(&i) || tab.selected.len() == 1 {
            tab.selected = vec![i];
            tab.anchor = Some(i);
        }
        tab.selected.sort_unstable();
        tab.drag = None;
        let selected = tab.selected.clone();
        tab.thumbs.set_selected(&selected);
        if !ctrl && !shift {
            self.go_to(i);
        }
        self.sync_state();
    }

    /// Glisser une vignette : position de dépôt d'après le déplacement vertical (en lignes).
    pub fn thumb_drag(&mut self, i: usize, y: f32, row: f32) {
        let Some(tab) = self.tab() else { return };
        if row <= 0.0 || (y >= 0.0 && y <= row) && tab.drag.is_none() {
            return; // pas encore sorti de la vignette : simple clic
        }
        let target = (i as f32 + (y / row) + 0.5).floor().clamp(0.0, tab.sizes.len() as f32) as usize;
        tab.drag = Some(target);
        tab.thumbs.set_drop((target < tab.sizes.len()).then_some(target));
    }

    pub fn thumb_release(&mut self) {
        let Some(tab) = self.tab() else { return };
        let Some(target) = tab.drag.take() else { return };
        tab.thumbs.set_drop(None);
        let pages = if tab.selected.is_empty() { vec![tab.current] } else { tab.selected.clone() };
        let (first, last) = (pages[0], pages[pages.len() - 1]);
        let contiguous = last - first + 1 == pages.len();
        if contiguous && (target >= first && target <= last + 1) {
            return; // déposé à sa place
        }
        self.edit("", move |pdf| edit::move_pages(pdf, &pages, target).map(Some));
    }

    // ----- opérations ----------------------------------------------------------------------

    fn delete_pages(&mut self, pages: Vec<usize>) {
        let Some(tab) = self.current_tab() else { return };
        if pages.len() >= tab.sizes.len() {
            self.error("Un PDF doit garder au moins une page : sélectionnez seulement les pages à retirer.");
            return;
        }
        let n = pages.len();
        let text = format!("Supprimer {n} page{} ({}) ?\n(Ctrl+Z pour annuler)", if n > 1 { "s" } else { "" },
                           edit::format_pages(&pages));
        if ask("Supprimer des pages", &text) {
            self.edit("", move |pdf| edit::delete(pdf, &pages).map(|_| None));
        }
    }

    fn insert_at(&self) -> usize {
        self.targets().into_iter().max().map_or(0, |p| p + 1)
    }

    fn insert_blank(&mut self) {
        let Some(tab) = self.current_tab() else { return };
        let at = self.insert_at();
        let size = tab.sizes[at.saturating_sub(1).min(tab.sizes.len() - 1)]; // format de la voisine
        self.edit("", move |pdf| edit::insert_blank(pdf, at, size).map(|_| Some(vec![at])));
    }

    fn insert_pdf(&mut self) {
        if self.current_tab().is_none() {
            return;
        }
        let Some(path) = crate::dialogs::pick_file("Insérer un PDF", ("Documents PDF", &["pdf"]), None) else { return };
        self.insert_pdf_path(path);
    }

    /// Insère le PDF après les pages sélectionnées (ou la page affichée).
    pub(super) fn insert_pdf_path(&mut self, path: PathBuf) {
        let at = self.insert_at();
        let name = file_name(&path);
        // ponytail : le PDF inséré est lu sur le disque (un onglet ouvert dessus avec des
        // modifications non enregistrées n'est pas pris en compte).
        self.edit("", move |pdf| {
            let data = plain_from_file(&path)?;
            let n = edit::insert_pdf(pdf, at, &data, None, &name)?;
            Ok(Some((at..at + n).collect()))
        });
    }

    fn insert_images(&mut self) {
        if self.current_tab().is_none() {
            return;
        }
        let paths = crate::dialogs::pick_files("Insérer des images", ("Images", IMAGES), None);
        self.insert_image_paths(&paths);
    }

    /// Une page par image, après les pages sélectionnées (ou la page affichée).
    pub(super) fn insert_image_paths(&mut self, paths: &[PathBuf]) {
        if paths.is_empty() {
            return;
        }
        let mut images = vec![];
        for p in paths {
            match std::fs::read(p) {
                Ok(d) => images.push(d),
                Err(e) => return self.error(&format!("Impossible de lire « {} » : {}.", file_name(p), reason(&e))),
            }
        }
        let at = self.insert_at();
        let n = images.len();
        self.edit("", move |pdf| edit::insert_images(pdf, at, &images).map(|_| Some((at..at + n).collect())));
    }

    fn extract(&mut self, pages: Vec<usize>) {
        let Some(tab) = self.current_tab() else { return };
        let stem = tab.path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let suggested = tab.path.with_file_name(format!("{stem} - pages {}.pdf", edit::format_pages(&pages)));
        let Some(target) = save_dialog("Enregistrer les pages extraites", &suggested) else { return };
        let id = tab.id;
        self.engine.run(move |docs| {
            let result = docs.get(&id).ok_or_else(|| "document fermé".to_string())
                .and_then(|pdf| edit::extract(pdf, &pages))
                .and_then(|data| write_atomic(&target, &data).map_err(|e| write_error(&target, &e)));
            from_engine(move |app| match result {
                Ok(()) => app.open_path(target),
                Err(e) => app.error(&e),
            });
        });
    }

    pub(super) fn crop_done(&mut self) {
        self.crop_mode = false;
        let pages = self.targets();
        let Some(tab) = self.tab() else { return };
        let Some((page, a, b)) = tab.crop.take() else {
            self.sync_state();
            return;
        };
        let rect = Rect::new(a.0, a.1, b.0, b.1);
        self.sync_state();
        self.update_view();
        if rect.width() < 10.0 || rect.height() < 10.0 {
            self.toast("Zone trop petite : recadrage abandonné.");
            return;
        }
        let pages = if pages.contains(&page) { pages } else { vec![page] };
        let text = format!("Garder cette zone sur {} ({}) ?\n(Ctrl+Z pour annuler)",
                           if pages.len() > 1 { "les pages" } else { "la page" }, edit::format_pages(&pages));
        if ask("Recadrer", &text) {
            self.edit("", move |pdf| edit::crop(pdf, page, rect, &pages).map(|_| Some(pages)));
        }
    }

    // ----- fusionner, scinder -----------------------------------------------------------------

    fn merge_start(&mut self) {
        self.merge_files = self.current_tab().map(|t| vec![t.path.clone()]).unwrap_or_default();
        self.sync_merge();
        self.ui().invoke_show_merge();
    }

    fn sync_merge(&self) {
        let names: Vec<slint::SharedString> = self.merge_files.iter()
            .map(|p| format!("{}   ({})", file_name(p), p.parent().map(|d| d.display().to_string()).unwrap_or_default()).into())
            .collect();
        self.ui().set_merge_names(ModelRc::new(VecModel::from(names)));
    }

    pub fn merge_command(&mut self, command: &str) {
        let ui = self.ui();
        let i = ui.get_merge_index();
        let n = self.merge_files.len() as i32;
        match command {
            "add" => {
                self.merge_files.extend(crate::dialogs::pick_files("Ajouter des PDF", ("Documents PDF", &["pdf"]), None));
            }
            "remove" if (0..n).contains(&i) => {
                self.merge_files.remove(i as usize);
                ui.set_merge_index(-1);
            }
            "up" if i > 0 && i < n => {
                self.merge_files.swap(i as usize, i as usize - 1);
                ui.set_merge_index(i - 1);
            }
            "down" if i >= 0 && i + 1 < n => {
                self.merge_files.swap(i as usize, i as usize + 1);
                ui.set_merge_index(i + 1);
            }
            "run" => return self.merge_run(),
            _ => {}
        }
        self.sync_merge();
    }

    fn merge_run(&mut self) {
        let files = self.merge_files.clone();
        let Some(first) = files.first() else { return };
        let stem = first.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let Some(target) = save_dialog("Enregistrer le PDF fusionné", &first.with_file_name(format!("{stem} - fusion.pdf")))
        else { return };
        // Un document ouvert est pris tel qu'affiché, modifications comprises.
        let open: Vec<Option<DocId>> = files.iter().map(|p| self.tabs.iter().find(|t| &t.path == p).map(|t| t.id)).collect();
        self.engine.run(move |docs| {
            let result = (|| {
                let mut sources = vec![];
                for (path, id) in files.iter().zip(open) {
                    let title = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                    sources.push((title, match id.and_then(|id| docs.get(&id)) {
                        Some(pdf) => pdf.plain_bytes()?,
                        None => plain_from_file(path)?,
                    }));
                }
                let data = edit::merge(&sources)?;
                write_atomic(&target, &data).map_err(|e| write_error(&target, &e))
            })();
            from_engine(move |app| match result {
                Ok(()) => app.open_path(target),
                Err(e) => app.error(&e),
            });
        });
    }

    /// Scinder : `mode` 0 selon des plages, 1 toutes les `n` pages, 2 selon les signets de
    /// premier niveau (un fichier par signet, nommé d'après lui).
    pub fn split_run(&mut self, mode: i32, n: usize, ranges: String) {
        let Some(tab) = self.current_tab() else { return };
        let count = tab.sizes.len();
        let groups: Result<Vec<(Option<String>, Vec<usize>)>, String> = match mode {
            1 => Ok(edit::split_every(count, n).into_iter().map(|g| (None, g)).collect()),
            2 => edit::split_by_bookmarks(&tab.bookmarks, count).map(|g| g.into_iter().map(|(t, p)| (Some(t), p)).collect()),
            _ if ranges.trim().is_empty() => Err("Indiquez les plages, par exemple : 1-3, 4-10, 11-".to_string()),
            _ => edit::parse_ranges(&ranges, count).map(|g| g.into_iter().map(|p| (None, p)).collect()),
        };
        let groups = match groups {
            Ok(g) => g,
            Err(e) => return self.error(&e),
        };
        let Some(folder) = crate::dialogs::pick_folder("Dossier de destination", tab.path.parent()) else { return };
        let stem = tab.path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let targets: Vec<PathBuf> = groups.iter().enumerate()
            .map(|(k, (title, g))| folder.join(match title {
                // « Rapport - 01 - Introduction.pdf » (caractères interdits dans un nom remplacés)
                Some(t) => format!("{stem} - {:02} - {}.pdf", k + 1,
                                   t.chars().map(|c| if r#"\/:*?"<>|"#.contains(c) || c.is_control() { '_' } else { c })
                                       .collect::<String>().trim().chars().take(80).collect::<String>()),
                None => format!("{stem} - pages {}.pdf", edit::format_pages(g)),
            }))
            .collect();
        let existing = targets.iter().filter(|t| t.exists()).count();
        if existing > 0 && !ask("Scinder", &format!("{existing} fichier(s) existe(nt) déjà dans ce dossier. Les remplacer ?")) {
            return;
        }
        let id = tab.id;
        self.engine.run(move |docs| {
            let result = (|| {
                let pdf = docs.get(&id).ok_or("document fermé")?;
                for ((_, g), t) in groups.iter().zip(&targets) {
                    let data = edit::extract(pdf, g)?;
                    write_atomic(t, &data).map_err(|e| write_error(t, &e))?;
                }
                Ok::<usize, String>(targets.len())
            })();
            from_engine(move |app| match result {
                Ok(k) => {
                    if ask("Scinder", &format!("{k} fichiers créés dans « {} ».\n\nOuvrir le dossier ?", folder.display())) {
                        let _ = open::that(&folder);
                    }
                }
                Err(e) => app.error(&e),
            });
        });
    }

    // ----- enregistrer, fermer, quitter -----------------------------------------------------

    fn save(&mut self) {
        let Some(tab) = self.current_tab() else { return };
        if !tab.modified {
            return self.toast("Aucune modification à enregistrer.");
        }
        if !tab.overwrite_confirmed {
            let mut text = format!("Enregistrer dans « {} » ?\nLe fichier d'origine sera remplacé \
                                    (Enregistrer sous… pour garder l'original).", tab.name);
            if tab.signature_lost {
                text.push_str("\n\nCe document est signé : la protection, le caviardage ou le nettoyage réécrivent tout le \
                               fichier, ses signatures ne seront plus valides.");
            } else if tab.signed {
                text.push_str("\n\nCe document est signé : les modifications s'ajoutent à la suite, les signatures restent \
                               valides et indiqueront que le document a été complété.");
            }
            if !ask("Enregistrer", &text) {
                return;
            }
        }
        let (id, path) = (tab.id, tab.path.clone());
        self.write(id, path, false);
    }

    fn save_as(&mut self, copy: bool) {
        let Some(tab) = self.current_tab() else { return };
        let suggested = if copy {
            let stem = tab.path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            tab.path.with_file_name(format!("{stem} - copie.pdf"))
        } else {
            tab.path.clone()
        };
        let title = if copy { "Enregistrer une copie" } else { "Enregistrer sous" };
        let Some(target) = save_dialog(title, &suggested) else { return };
        let id = tab.id;
        // Écraser un fichier ouvert dans un autre onglet : deux onglets sur un même fichier,
        // l'un pouvant ensuite défaire ce que l'autre a enregistré.
        if let Some(other) = self.tabs.iter().find(|t| t.id != id && t.path == target) {
            return self.error(&format!("« {} » est ouvert dans un autre onglet : fermez-le d'abord.", other.name));
        }
        self.write(id, target, copy);
    }

    /// Écrit le document `id` dans `target` (atomique) ; `copy` : le document garde son
    /// fichier et ses modifications restent « non enregistrées ».
    fn write(&mut self, id: DocId, target: PathBuf, copy: bool) {
        self.engine.run(move |docs| {
            let result = docs.get_mut(&id).ok_or_else(|| "document fermé".to_string()).and_then(|pdf| {
                let data = pdf.bytes_for_disk()?;
                write_atomic(&target, &data).map_err(|e| write_error(&target, &e))?;
                if !copy {
                    pdf.mark_saved();
                }
                Ok(())
            });
            from_engine(move |app| app.on_saved(id, target, copy, result));
        });
    }

    fn on_saved(&mut self, id: DocId, target: PathBuf, copy: bool, result: Result<(), String>) {
        if let Err(e) = result {
            self.close_after_save = None;
            self.quit_after_saves = 0;
            return self.error(&e);
        }
        crate::log(&format!("Enregistré : {}", target.display()));
        if !copy {
            if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id) {
                tab.modified = false;
                tab.overwrite_confirmed = true;
                tab.name = file_name(&target);
                tab.path = target.clone();
            }
            self.settings.add_recent(&target);
            self.settings.save();
        }
        self.toast(&format!("{} : {}", if copy { "Copie enregistrée" } else { "Enregistré" }, target.display()));
        self.sync_tabs();
        if self.close_after_save == Some(id) {
            self.close_after_save = None;
            if let Some(i) = self.tabs.iter().position(|t| t.id == id) {
                self.close_tab_now(i);
            }
        }
        if self.quit_after_saves > 0 {
            self.quit_after_saves -= 1;
            if self.quit_after_saves == 0 && !self.has_unsaved() {
                self.exit_now();
            }
        }
    }

    /// Fermer un onglet : s'il est modifié, Enregistrer / Ne pas enregistrer / Annuler.
    pub fn request_close_tab(&mut self, i: usize) {
        let Some(tab) = self.tabs.get(i) else { return };
        if !tab.modified {
            return self.close_tab_now(i);
        }
        let answer = crate::dialogs::choose("Modifications non enregistrées",
            &format!("« {} » a été modifié. Enregistrer les modifications ?", tab.name),
            ["Enregistrer", "Ne pas enregistrer", "Annuler"]);
        match answer {
            0 => {
                let (id, path) = (tab.id, tab.path.clone());
                self.close_after_save = Some(id);
                self.write(id, path, false); // « Enregistrer » vaut confirmation
            }
            1 => self.close_tab_now(i),
            _ => {}
        }
    }

    /// Quitter : documents modifiés → Tout enregistrer / Quitter sans enregistrer / Annuler.
    pub fn quit(&mut self) {
        let unsaved: Vec<(DocId, PathBuf, String)> = self.tabs.iter().filter(|t| t.modified)
            .map(|t| (t.id, t.path.clone(), t.name.clone())).collect();
        if unsaved.is_empty() {
            self.exit_now();
            return;
        }
        let names: Vec<String> = unsaved.iter().map(|u| format!("« {} »", u.2)).collect();
        let answer = crate::dialogs::choose("Modifications non enregistrées",
            &format!("Ces documents ont été modifiés : {}.\nEnregistrer avant de quitter ?", names.join(", ")),
            ["Tout enregistrer", "Quitter sans enregistrer", "Annuler"]);
        match answer {
            0 => {
                self.quit_after_saves = unsaved.len();
                for (id, path, _) in unsaved {
                    self.write(id, path, false);
                }
            }
            1 => {
                self.exit_now();
            }
            _ => self.exit_cancelled(),
        }
    }
}

/// Contenu déchiffré d'un PDF pris sur le disque (fusion, insertion).
fn plain_from_file(path: &Path) -> Result<Vec<u8>, String> {
    let data = std::fs::read(path).map_err(|e| format!("Impossible de lire « {} » : {}.", file_name(path), reason(&e)))?;
    match Pdf::open(&data) {
        Ok(pdf) => pdf.plain_bytes(),
        Err(OpenError::NeedsPassword) => Err(format!(
            "« {} » est protégé par un mot de passe : ouvrez-le d'abord dans PDFManager.", file_name(path))),
        Err(OpenError::Unreadable(e)) => Err(format!("Impossible d'ouvrir « {} » : {e}.", file_name(path))),
    }
}
