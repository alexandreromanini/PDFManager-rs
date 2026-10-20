//! Contrôleur, partie conversions, OCR et assistant IA.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::channel;
use std::sync::Arc;

use slint::{ModelRc, VecModel};

use super::{file_name, from_engine, App};
use crate::assistant::{self, Conversation};
use crate::engine::DocId;
use crate::{convert, edit, ocr, ChatMessage};

fn save_as(title: &str, suggested: &Path, filter: (&str, &[&str])) -> Option<PathBuf> {
    crate::dialogs::save_file(title, filter, suggested)
}

fn mb(n: usize) -> String {
    format!("{:.1} Mo", n as f64 / 1_048_576.0).replace('.', ",")
}

/// Conversation de l'assistant sur un document, refaite quand il est modifié.
pub(super) struct Chat {
    version: u64,
    conversation: Option<Conversation>,
    messages: Vec<(String, String)>, // (« user » | « assistant », texte affiché)
}

impl App {
    fn doc_stem(&mut self) -> Option<(DocId, PathBuf, String)> {
        let tab = self.tab()?;
        let stem = tab.path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        Some((tab.id, tab.path.clone(), stem))
    }

    fn offer_open(&mut self, path: PathBuf, what: &str) {
        if super::edit_ui::ask(what, &format!("{what} : {}\n\nOuvrir le fichier ?", path.display())) {
            let _ = open::that(&path);
        }
    }

    // ----- depuis ce PDF -------------------------------------------------------------------

    pub(super) fn export_word(&mut self) {
        let Some((id, path, stem)) = self.doc_stem() else { return };
        let Some(target) = save_as("Exporter en Word", &path.with_file_name(format!("{stem}.docx")), ("Document Word", &["docx"]))
        else { return };
        self.toast("Conversion en Word…");
        self.engine.run(move |docs| {
            let result = docs.get(&id).ok_or("document fermé".to_string()).and_then(|pdf| convert::to_docx(pdf, &target));
            from_engine(move |app| match result {
                Ok(()) => app.offer_open(target, "Document Word créé"),
                Err(e) => app.error(&format!("Conversion en Word impossible : {e}")),
            });
        });
    }

    pub(super) fn export_text(&mut self) {
        let Some((id, path, stem)) = self.doc_stem() else { return };
        let Some(target) = save_as("Exporter en texte", &path.with_file_name(format!("{stem}.txt")), ("Texte", &["txt"]))
        else { return };
        self.engine.run(move |docs| {
            let result = docs.get(&id).ok_or("document fermé".to_string()).and_then(convert::to_text)
                .and_then(|t| convert::save(&target, t.as_bytes()));
            from_engine(move |app| match result {
                Ok(()) => app.offer_open(target, "Texte exporté"),
                Err(e) => app.error(&e),
            });
        });
    }

    pub fn export_images_run(&mut self, jpeg: bool, dpi: i32, pages: String) {
        let Some((id, path, stem)) = self.doc_stem() else { return };
        let count = self.tab().map_or(0, |t| t.sizes.len());
        let pages = match edit::parse_pages(&pages, count) {
            Ok(p) => p,
            Err(e) => return self.error(&e),
        };
        let Some(folder) = crate::dialogs::pick_folder("Dossier des images", path.parent()) else { return };
        let ext = if jpeg { "jpg" } else { "png" };
        let targets = convert::image_targets(&stem, &pages, count, &folder, ext);
        let existing = targets.iter().filter(|t| t.exists()).count();
        if existing > 0 && !super::edit_ui::ask("Exporter en images", &format!("{existing} image(s) existe(nt) déjà. Les remplacer ?")) {
            return;
        }
        let zoom = dpi.clamp(36, 600) as f32 / 72.0;
        self.engine.run(move |docs| {
            let Some(pdf) = docs.get(&id) else { return };
            let n = pages.len();
            let result = pages.iter().zip(&targets).enumerate().try_for_each(|(k, (&i, t))| {
                from_engine(move |app| app.toast(&format!("Export des images : {} sur {n}…", k + 1)));
                let data = convert::encode(&pdf.render(i, zoom, 0)?, jpeg)?;
                convert::save(t, &data)
            });
            from_engine(move |app| match result {
                Ok(()) => {
                    if super::edit_ui::ask("Exporter en images", &format!("{n} image(s) créée(s) dans « {} ».\n\nOuvrir le dossier ?", folder.display())) {
                        let _ = open::that(&folder);
                    }
                }
                Err(e) => app.error(&e),
            });
        });
    }

    // ----- créer un PDF --------------------------------------------------------------------

    pub(super) fn from_images(&mut self) {
        let files = crate::dialogs::pick_files("Images à rassembler en PDF", ("Images", &["png", "jpg", "jpeg", "tif", "tiff", "bmp", "gif", "webp"]), None);
        if files.is_empty() {
            return;
        }
        let mut images = vec![];
        for f in &files {
            match std::fs::read(f) {
                Ok(d) => images.push(d),
                Err(e) => return self.error(&format!("Impossible de lire « {} » : {e}.", file_name(f))),
            }
        }
        let first = files[0].with_extension("pdf");
        let Some(target) = save_as("Enregistrer le PDF", &first, ("Documents PDF", &["pdf"])) else { return };
        std::thread::spawn(move || {
            let result = convert::images_to_pdf(&images).and_then(|d| convert::save(&target, &d));
            from_engine(move |app| match result {
                Ok(()) => app.open_path(target),
                Err(e) => app.error(&e),
            });
        });
    }

    pub(super) fn from_office(&mut self) {
        let Some(soffice) = convert::find_libreoffice() else {
            return self.error(&format!("LibreOffice (gratuit) est nécessaire pour convertir les documents Office ; il n'a \
                                        pas été trouvé sur cet ordinateur.\n\n{}", convert::LO_URL));
        };
        let files = crate::dialogs::pick_files("Documents à convertir en PDF", ("Documents Office", convert::OFFICE), None);
        if files.is_empty() {
            return;
        }
        let targets: Vec<PathBuf> = files.iter().map(|f| f.with_extension("pdf")).collect();
        let existing = targets.iter().filter(|t| t.exists()).count();
        if existing > 0 && !super::edit_ui::ask("Convertir en PDF", &format!("{existing} PDF existe(nt) déjà à côté des documents. Les remplacer ?")) {
            return;
        }
        self.toast("Conversion avec LibreOffice…");
        std::thread::spawn(move || {
            for (src, target) in files.iter().zip(targets) {
                let result = convert::office_to_pdf(src, &soffice).and_then(|d| convert::save(&target, &d));
                let name = file_name(src);
                from_engine(move |app| match result {
                    Ok(()) => app.open_path(target),
                    Err(e) => app.error(&format!("« {name} » : {e}")),
                });
            }
        });
    }

    // ----- réduire, archiver ---------------------------------------------------------------

    pub fn compress_run(&mut self, level: usize) {
        let Some((id, path, stem)) = self.doc_stem() else { return };
        let level = convert::LEVELS[level.min(2)].0;
        let Some(target) = save_as("Enregistrer la copie réduite", &path.with_file_name(format!("{stem} - réduit.pdf")),
                                   ("Documents PDF", &["pdf"])) else { return };
        let before = std::fs::metadata(&path).map_or(0, |m| m.len() as usize);
        self.toast("Réduction de la taille…");
        self.engine.run(move |docs| {
            // Comme la version Python : sans gain, rien n'est écrit.
            let result = docs.get(&id).ok_or("document fermé".to_string()).and_then(|pdf| convert::compress(pdf, level))
                .and_then(|d| if before > 0 && d.len() >= before { Ok(None) } else { convert::save(&target, &d).map(|_| Some(d.len())) });
            from_engine(move |app| match result {
                Ok(Some(after)) => {
                    let gain = 100 - 100 * after / before.max(1);
                    app.offer_open(target, &format!("Copie réduite ({} → {}, {gain} % de moins)", mb(before), mb(after)))
                }
                Ok(None) => crate::dialogs::info("Réduire la taille",
                    &format!("Aucun gain possible à ce niveau ({}) : rien n'a été écrit.", mb(before))),
                Err(e) => app.error(&e),
            });
        });
    }

    pub(super) fn pdfa(&mut self) {
        let Some(gs) = convert::find_ghostscript() else {
            return self.error(&format!("Ghostscript (gratuit) est nécessaire pour créer un PDF/A (polices intégrées, \
                                        couleurs) ; il n'a pas été trouvé sur cet ordinateur.\n\n{}", convert::GS_URL));
        };
        let Some((id, path, stem)) = self.doc_stem() else { return };
        let Some(target) = save_as("Enregistrer en PDF/A", &path.with_file_name(format!("{stem} - PDFA.pdf")), ("Documents PDF", &["pdf"]))
        else { return };
        self.toast("Conversion en PDF/A…");
        self.engine.run(move |docs| {
            let data = docs.get(&id).ok_or("document fermé".to_string()).and_then(|pdf| pdf.plain_bytes());
            std::thread::spawn(move || {
                let result = data.and_then(|d| convert::to_pdfa(&d, &gs)).and_then(|(d, notes)| convert::save(&target, &d).map(|_| notes));
                from_engine(move |app| match result {
                    Ok(notes) => {
                        let extra = if notes.is_empty() { String::new() } else { format!("\n\nRemarques de Ghostscript :\n{}", notes.join("\n")) };
                        app.offer_open(target, &format!("PDF/A-2b créé par Ghostscript{extra}"));
                    }
                    Err(e) => app.error(&e),
                });
            });
        });
    }

    // ----- OCR -----------------------------------------------------------------------------

    pub fn ocr_run(&mut self, language: usize, pages: String, all: bool) {
        let Some((id, _, _)) = self.doc_stem() else { return };
        let count = self.tab().map_or(0, |t| t.sizes.len());
        let pages = match edit::parse_pages(&pages, count) {
            Ok(p) => p,
            Err(e) => return self.error(&e),
        };
        let lang = ocr::LANGUAGES[language.min(2)].0;
        let cancel = Arc::new(AtomicBool::new(false));
        self.ocr_cancel = Some(cancel.clone());
        let engine = self.engine.clone();
        self.engine.run(move |docs| {
            let Some(pdf) = docs.get(&id) else { return };
            let todo = ocr::pages_to_process(pdf, &pages, all);
            std::thread::spawn(move || {
                const DPI: f32 = 300.0;
                const CLOSED: &str = "document fermé";
                let n = todo.len();
                let mut results = vec![];
                for (k, &page) in todo.iter().enumerate() {
                    if cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    from_engine(move |app| app.toast(&format!("Reconnaissance du texte : page {} sur {n}… (Échap pour arrêter)", k + 1)));
                    // Rendu dans le thread moteur (le document y vit), reconnaissance ici.
                    let (tx, rx) = channel();
                    engine.run(move |docs| {
                        let _ = tx.send(docs.get(&id).ok_or(CLOSED.to_string()).and_then(|p| ocr::render(p, page, DPI)));
                    });
                    let result = rx.recv().unwrap_or(Err("moteur arrêté".into()))
                        .and_then(|(gray, w, h)| ocr::recognize(&gray, w, h, DPI, lang));
                    match result {
                        Ok(words) => results.push((page, ocr::to_points(words, DPI))),
                        // Document fermé entre-temps : on s'arrête sans rien signaler.
                        Err(e) if e == CLOSED => return from_engine(|app| app.ocr_cancel = None),
                        Err(e) => return from_engine(move |app| app.error(&format!("Reconnaissance impossible : {e}"))),
                    }
                }
                let stopped = cancel.load(Ordering::Relaxed);
                from_engine(move |app| {
                    app.ocr_cancel = None;
                    if n == 0 {
                        return app.toast("Ces pages contiennent déjà du texte (cochez « Aussi les pages avec du texte » pour les traiter).");
                    }
                    let keep = app.tabs.iter().find(|t| t.id == id).map(|t| t.selected.clone()).unwrap_or_default();
                    app.edit_doc(id, move |pdf| {
                        let done = ocr::add_text_layers(pdf, &results)?;
                        let mut m = format!("Texte reconnu sur {done} page{} : recherche, sélection et copie possibles (Ctrl+Z pour annuler).",
                                            if done > 1 { "s" } else { "" });
                        if stopped {
                            m = format!("Arrêté. {m}");
                        }
                        Ok((Some(keep), m))
                    });
                });
            });
        });
    }

    /// Échap pendant l'OCR : arrêt après la page en cours.
    pub(super) fn ocr_stop(&mut self) -> bool {
        match self.ocr_cancel.take() {
            Some(c) => {
                c.store(true, Ordering::Relaxed);
                self.toast("Arrêt après la page en cours…");
                true
            }
            None => false,
        }
    }

    // ----- assistant IA --------------------------------------------------------------------

    pub(super) fn sync_assistant(&mut self) {
        let ui = self.ui();
        ui.set_ai_installed(assistant::installed());
        let id = self.tab().map(|t| t.id);
        let rows: Vec<ChatMessage> = id.and_then(|id| self.chats.get(&id)).map(|c| {
            c.messages.iter().map(|(role, text)| {
                let pages: Vec<i32> = if role == "assistant" { assistant::cited_pages(text).into_iter().map(|p| p as i32).collect() } else { vec![] };
                ChatMessage { user: role == "user", text: text.into(), pages: ModelRc::new(VecModel::from(pages)) }
            }).collect()
        }).unwrap_or_default();
        ui.set_chat(ModelRc::new(VecModel::from(rows)));
        ui.set_ai_busy(self.ai_cancel.is_some());
    }

    pub(super) fn ai_download(&mut self) {
        if !super::edit_ui::ask("Assistant IA", "Télécharger le modèle de l'assistant (2,5 Go, une seule fois) ?\n\n\
                                Il fonctionne ensuite hors ligne, sur cet ordinateur : vos documents ne sont envoyés nulle \
                                part. Prévoir 8 Go de mémoire vive.") {
            return;
        }
        self.ui().set_ai_progress("Téléchargement…".into());
        std::thread::spawn(|| {
            let result = assistant::download_model(|done, total| {
                from_engine(move |app| app.ui().set_ai_progress(format!("Téléchargement : {done} / {total} Mo").into()));
            });
            from_engine(move |app| {
                app.ui().set_ai_progress("".into());
                if let Err(e) = result {
                    app.error(&format!("Téléchargement impossible.\n\n{e}"));
                }
                app.sync_assistant();
            });
        });
    }

    /// `question` vide : résumer le document.
    pub fn ai_ask(&mut self, question: String) {
        let question = question.trim().to_string();
        if self.ai_cancel.is_some() {
            return;
        }
        let Some(tab) = self.tab() else { return };
        let (id, version) = (tab.id, tab.version);
        let chat = self.chats.entry(id).or_insert(Chat { version, conversation: None, messages: vec![] });
        if chat.version != version {
            chat.conversation = None; // document modifié : relu
            chat.version = version;
        }
        if chat.conversation.is_none() {
            let (tx, rx) = channel();
            self.engine.run(move |docs| {
                let _ = tx.send(docs.get(&id).ok_or("document fermé".to_string()).and_then(Conversation::of));
            });
            match rx.recv().unwrap_or(Err("moteur arrêté".into())) {
                Ok(c) => self.chats.get_mut(&id).unwrap().conversation = Some(c),
                Err(e) => return self.error(&e),
            }
        }
        let chat = self.chats.get_mut(&id).unwrap();
        let conversation = chat.conversation.as_ref().unwrap();
        let (messages, shown) = conversation.request((!question.is_empty()).then_some(question.as_str()));
        chat.messages.push(("user".into(), shown.clone()));
        chat.messages.push(("assistant".into(), "…".into()));
        let cancel = Arc::new(AtomicBool::new(false));
        self.ai_cancel = Some(cancel.clone());
        self.ai.get_or_insert_with(assistant::Engine::start).ask(assistant::Job {
            messages,
            cancel,
            on_text: Box::new(move |text| from_engine(move |app| app.ai_text(id, text, false))),
            done: Box::new(move |result| from_engine(move |app| {
                app.ai_cancel = None;
                match result {
                    Ok(answer) => {
                        if let Some(c) = app.chats.get_mut(&id).and_then(|c| c.conversation.as_mut()) {
                            c.history.push(("user".into(), shown));
                            c.history.push(("assistant".into(), answer.clone()));
                        }
                        app.ai_text(id, if answer.is_empty() { "(pas de réponse)".into() } else { answer }, true);
                    }
                    Err(e) => app.ai_text(id, format!("Erreur : {e}"), true),
                }
            })),
        });
        self.sync_assistant();
    }

    fn ai_text(&mut self, id: DocId, text: String, _done: bool) {
        if let Some(last) = self.chats.get_mut(&id).and_then(|c| c.messages.last_mut()) {
            last.1 = text;
        }
        if self.tab().is_some_and(|t| t.id == id) {
            self.sync_assistant();
        }
    }

    pub(super) fn ai_stop(&mut self) {
        if let Some(c) = &self.ai_cancel {
            c.store(true, Ordering::Relaxed);
        }
    }
}
