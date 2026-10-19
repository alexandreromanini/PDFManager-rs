//! Contrôleur, partie formulaires, protection, propriétés, impression et mises à jour.

use slint::{ModelRc, SharedString, VecModel};

use super::annot_ui::{Editor, NoteTarget};
use super::{from_engine, App, Tab};
use crate::forms::{self, FieldInfo};
use crate::pdf::Rect;
use crate::textedit::{self, TextBlock};
use crate::{print, security, updates, PropRow};

/// Outils de création de champs : nom de l'outil, type de champ, taille par défaut.
const FIELD_TOOLS: [(&str, &str, (f32, f32)); 4] = [
    ("field-text", "text", (160.0, 22.0)),
    ("field-check", "check", (14.0, 14.0)),
    ("field-combo", "combo", (130.0, 22.0)),
    ("field-signature", "signature", (180.0, 50.0)),
];

fn field_tool(tool: &str) -> Option<(&'static str, (f32, f32))> {
    FIELD_TOOLS.iter().find(|t| t.0 == tool).map(|t| (t.1, t.2))
}

impl App {
    fn field_at(tab: &Tab, page: usize, pt: (f32, f32)) -> Option<FieldInfo> {
        tab.fields.iter().rev().find(|f| f.page == page && f.rect.contains(pt.0, pt.1)).cloned()
    }

    /// Appui sur une page avec l'outil Sélection ou un outil de champ ; `true` : géré.
    pub(super) fn field_press(&mut self, page: usize, pt: (f32, f32)) -> bool {
        let tool = self.tool.clone();
        let Some(tab) = self.tab() else { return false };
        let hit = Self::field_at(tab, page, pt);
        if tool == "digital-sign" {
            if let Some(f) = hit.filter(|f| f.kind == "signature") {
                self.fill(f); // champ vide : signer dedans ; signé : vérifier
                return true;
            }
            return false;
        }
        if field_tool(&tool).is_some() {
            if let Some(f) = hit {
                let text = format!("Supprimer le champ « {} » ?\n(Ctrl+Z pour annuler)", f.name);
                if super::edit_ui::ask("Supprimer le champ", &text) {
                    self.edit("", move |pdf| forms::delete_field(pdf, f.page, f.xref).map(|_| None));
                }
            } else {
                tab.gesture = Some(super::annot_ui::Gesture::Draw { page, a: pt, b: pt });
                self.update_view();
            }
            return true;
        }
        if !tool.is_empty() {
            return false;
        }
        let Some(f) = hit else { return false };
        self.fill(f);
        true
    }

    /// Remplir un champ : saisie sur place, liste de choix, ou case cochée / décochée.
    fn fill(&mut self, f: FieldInfo) {
        if f.readonly {
            return self.toast(&format!("Le champ « {} » est en lecture seule.", f.name));
        }
        match f.kind {
            "text" => {
                let Some(tab) = self.tab() else { return };
                let size = if f.multiline { 12.0 } else { (f.rect.height() * 0.65).clamp(6.0, 14.0) };
                tab.editor = Some(Editor { page: f.page, rect: f.rect, xref: None, size, color: (0.0, 0.0, 0.0),
                                           field: Some(f.clone()), block: None });
                tab.picked = None;
                let ui = self.ui();
                ui.set_editor_text(f.value.clone().into());
                ui.set_editor_single(!f.multiline);
                self.update_view();
            }
            "combo" | "list" => {
                let ui = self.ui();
                let choices: Vec<SharedString> = f.choices.iter().map(|c| c.as_str().into()).collect();
                ui.set_choices(ModelRc::new(VecModel::from(choices)));
                ui.set_choice_title(format!("{} : choisissez une valeur", f.name).into());
                self.pending_choice = Some(f);
                ui.invoke_show_choices();
            }
            "check" | "radio" => {
                let value = if f.checked() { "Off" } else { "Yes" };
                self.edit("", move |pdf| forms::set_value(pdf, &f, value).map(|_| None));
            }
            "signature" if f.signed => {
                if !self.settings.sidebar {
                    self.toggle_sidebar();
                }
                self.ui().set_rail_tab(5);
                self.verify_signatures();
            }
            "signature" => self.digital_start(super::sign_ui::SignTarget::Field(f.name)),
            _ => {}
        }
    }

    pub fn choice_picked(&mut self, i: usize) {
        let Some(f) = self.pending_choice.take() else { return };
        let Some(value) = f.choices.get(i).cloned() else { return };
        self.edit("", move |pdf| forms::set_value(pdf, &f, &value).map(|_| None));
    }

    /// Texte saisi dans un champ (appelé par la validation de l'éditeur).
    pub(super) fn field_commit(&mut self, f: FieldInfo, text: String) {
        if text != f.value {
            self.edit("", move |pdf| forms::set_value(pdf, &f, &text).map(|_| None));
        }
    }

    /// Fin du tracé d'un nouveau champ ; `true` : géré.
    pub(super) fn field_release(&mut self, page: usize, a: (f32, f32), b: (f32, f32)) -> bool {
        let Some((kind, (w, h))) = field_tool(&self.tool) else { return false };
        let small = (a.0 - b.0).abs() < 5.0 && (a.1 - b.1).abs() < 5.0;
        let rect = if small { Rect::new(a.0, a.1, a.0 + w, a.1 + h) } else { Rect::new(a.0, a.1, b.0, b.1) };
        if kind == "combo" {
            self.pending_note = Some(NoteTarget::Combo(page, rect));
            self.show_note("Choix de la liste, un par ligne", "");
            return true;
        }
        self.edit("", move |pdf| {
            let name = forms::unique_name(pdf, kind);
            forms::add_field(pdf, page, rect, kind, &name, &[]).map(|_| None)
        });
        true
    }

    pub(super) fn add_combo(&mut self, page: usize, rect: Rect, text: &str) {
        let options: Vec<String> = text.lines().map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect();
        self.edit("", move |pdf| {
            let name = forms::unique_name(pdf, "combo");
            forms::add_field(pdf, page, rect, "combo", &name, &options).map(|_| None)
        });
    }

    // ----- protection, propriétés -------------------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    pub fn protect_run(&mut self, open: String, open2: String, owner: String, owner2: String, allow: [bool; 4]) {
        if open != open2 || owner != owner2 {
            return self.error("Les confirmations ne correspondent pas aux mots de passe.");
        }
        let allow: Vec<&'static str> = security::PERMISSIONS.iter().zip(allow).filter(|(_, a)| *a).map(|(p, _)| p.0).collect();
        if let Err(e) = security::check(&open, &owner, &allow) {
            return self.error(&e);
        }
        self.edit("Protection prête : elle s'appliquera au fichier à l'enregistrement (Ctrl+S).",
                  move |pdf| security::protect(pdf, &open, &owner, &allow).map(|_| None));
    }

    pub(super) fn unprotect(&mut self, password: Option<String>) {
        let Some(id) = self.tab().map(|t| t.id) else { return };
        self.engine.run(move |docs| {
            let Some(pdf) = docs.get_mut(&id) else { return };
            let state = if !pdf.is_encrypted() {
                Err("Ce document n'est pas protégé.".to_string())
            } else if pdf.permissions().is_all() || password.as_deref().is_some_and(|p| pdf.unlock_owner(p)) {
                Ok(true)
            } else {
                Ok(false)
            };
            from_engine(move |app| match state {
                Err(e) => app.error(&e),
                Ok(true) => app.edit("Protection retirée : effective à l'enregistrement (Ctrl+S).",
                                     |pdf| security::unprotect(pdf).map(|_| None)),
                Ok(false) => {
                    if password.is_some() {
                        app.error("Mot de passe des permissions incorrect.");
                    }
                    app.ui().invoke_show_owner();
                }
            });
        });
    }

    pub(super) fn properties(&mut self) {
        let Some(tab) = self.tab() else { return };
        let (id, path) = (tab.id, tab.path.clone());
        self.engine.run(move |docs| {
            let Some(pdf) = docs.get(&id) else { return };
            let values = security::metadata(pdf);
            let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            let info = format!(
                "Fichier : {}\nPages : {}\nTaille : {}\nProtection : {}\nSigné : {}\nChamps de formulaire : {}",
                path.display(), pdf.page_count(), size_text(size),
                if pdf.is_encrypted() { "oui (mot de passe)" } else { "aucune" },
                if pdf.signed { "oui" } else { "non" }, forms::fields(pdf).len());
            from_engine(move |app| {
                let rows: Vec<PropRow> = security::FIELDS.iter().zip(&values)
                    .map(|((_, label), v)| PropRow { label: (*label).into(), value: v.as_str().into() }).collect();
                app.props = values;
                let ui = app.ui();
                ui.set_prop_rows(ModelRc::new(VecModel::from(rows)));
                ui.set_prop_info(info.into());
                ui.invoke_show_props();
            });
        });
    }

    pub fn prop_edited(&mut self, i: usize, text: String) {
        if let Some(v) = self.props.get_mut(i) {
            *v = text;
        }
    }

    pub fn props_save(&mut self) {
        let values = self.props.clone();
        self.edit("Propriétés enregistrées dans le document (Ctrl+S pour l'écrire sur le disque).",
                  move |pdf| security::set_metadata(pdf, &values).map(|_| None));
    }

    // ----- impression ---------------------------------------------------------------------------

    pub(super) fn print(&mut self) {
        let Some(tab) = self.tab() else { return };
        let (id, title) = (tab.id, tab.name.clone());
        if !cfg!(windows) {
            return self.error("L'impression n'est disponible que sous Windows.");
        }
        self.engine.run(move |docs| {
            let Some(pdf) = docs.get(&id) else { return };
            let allowed = security::allowed(pdf).contains(&"print");
            let sizes = pdf.page_sizes().to_vec();
            from_engine(move |app| {
                if !allowed {
                    return app.error("Ce document est protégé : son auteur n'en autorise pas l'impression.");
                }
                let Some(job) = print::choose(sizes.len()) else { return };
                app.toast("Impression…");
                app.engine.run(move |docs| {
                    let Some(pdf) = docs.get(&id) else { return };
                    let result = print::run(job, &title, &sizes, |i, zoom, rot| pdf.render(i, zoom, rot), |k, n| {
                        from_engine(move |app| app.toast(&format!("Impression : page {k} sur {n}…")));
                    });
                    from_engine(move |app| match result {
                        Ok(n) => app.toast(&format!("{n} page{} envoyée{} à l'imprimante.",
                                                    if n > 1 { "s" } else { "" }, if n > 1 { "s" } else { "" })),
                        Err(e) => app.error(&e),
                    });
                });
            });
        });
    }

    // ----- mises à jour -------------------------------------------------------------------------

    /// Au démarrage (`manual` faux) : ne dit rien si l'application est à jour ou hors ligne.
    pub fn check_updates(&mut self, manual: bool) {
        if manual {
            self.toast("Recherche des mises à jour…");
        }
        std::thread::spawn(move || {
            let found = updates::latest();
            from_engine(move |app| match found {
                Ok(Some(r)) if updates::is_newer(&r, env!("CARGO_PKG_VERSION")) => app.offer_update(r),
                Ok(_) if manual => app.info("Mises à jour", &format!("PDFManager {} est à jour.", env!("CARGO_PKG_VERSION"))),
                Err(e) if manual => app.error(&format!("Impossible de joindre GitHub ; vérifiez la connexion.\n\n{e}")),
                Err(e) => crate::log(&format!("Mises à jour non vérifiées : {e}")),
                _ => {}
            });
        });
    }

    fn info(&self, title: &str, text: &str) {
        crate::dialogs::info(title, text);
    }

    fn offer_update(&mut self, r: updates::Release) {
        let current = env!("CARGO_PKG_VERSION");
        if !cfg!(windows) {
            return self.info("Mise à jour disponible", &format!("La version {} est publiée :\n{}", r.version, updates::RELEASES));
        }
        let text = format!("La version {} de PDFManager (Rust) est disponible (vous avez la {current}).\n\n\
                            L'application va se fermer, s'installer puis se relancer d'elle-même. Les documents \
                            modifiés vous seront d'abord proposés à l'enregistrement.\n\nInstaller maintenant ?", r.version);
        if !super::edit_ui::ask("Mise à jour disponible", &text) {
            return;
        }
        self.toast(&format!("Téléchargement de la version {}…", r.version));
        std::thread::spawn(move || {
            let version = r.version.clone();
            let result = updates::download(&r, |done, total| {
                let v = version.clone();
                from_engine(move |app| app.toast(&format!("Téléchargement de la version {v} : {done} / {total} Mo")));
            });
            from_engine(move |app| match result {
                Ok(path) => {
                    crate::log(&format!("Installation de la version {}", r.version));
                    app.pending_installer = Some(path);
                    app.quit();
                }
                Err(e) => app.error(&format!("Téléchargement impossible.\n\n{e}")),
            });
        });
    }

    /// Fin de l'application ; lance d'abord l'installeur d'une mise à jour en attente.
    pub(super) fn exit_now(&mut self) {
        if let Some(installer) = self.pending_installer.take() {
            if let Err(e) = updates::install(&installer) {
                return self.error(&e);
            }
        }
        let _ = slint::quit_event_loop();
    }

    /// Fermeture annulée par l'utilisateur.
    pub(super) fn exit_cancelled(&mut self) {
        if self.pending_installer.take().is_some() {
            self.toast("Mise à jour reportée.");
        }
    }
}

/// « 2 Ko », « 1,4 Mo ».
fn size_text(n: u64) -> String {
    if n < 1 << 20 {
        format!("{} Ko", n.div_ceil(1024))
    } else {
        format!("{:.1} Mo", n as f64 / 1_048_576.0).replace('.', ",")
    }
}

// ----- modifier le texte, caviardage, nettoyage ----------------------------------------------

impl App {
    /// Outil « Modifier le texte » : le paragraphe cliqué s'ouvre en saisie sur place.
    pub(super) fn edit_text_at(&mut self, page: usize, pt: (f32, f32)) {
        let Some(id) = self.tab().map(|t| t.id) else { return };
        self.engine.run(move |docs| {
            let Some(pdf) = docs.get(&id) else { return };
            let found = textedit::text_block_at(pdf, page, pt.0, pt.1);
            from_engine(move |app| match found {
                Ok(Some(block)) => {
                    let Some(tab) = app.tabs.iter_mut().find(|t| t.id == id) else { return };
                    let first = &block.lines[0];
                    tab.editor = Some(Editor { page, rect: block.rect, xref: None, size: first.size, color: first.color,
                                               field: None, block: Some(block.clone()) });
                    tab.picked = None;
                    let ui = app.ui();
                    ui.set_editor_text(block.text().into());
                    ui.set_editor_single(false);
                    app.update_view();
                }
                Ok(None) => app.toast("Pas de texte ici : cliquez sur un paragraphe (pour écrire ailleurs, utilisez « Ajouter du texte »)."),
                Err(e) => app.error(&e),
            });
        });
    }

    pub(super) fn text_commit(&mut self, block: TextBlock, text: String) {
        let keep = self.tab().map(|t| t.selected.clone()).unwrap_or_default();
        self.edit_report(move |pdf| {
            let warnings = textedit::edit_block(pdf, &block, &text)?;
            let message = if warnings.is_empty() { String::new() } else { format!("Texte modifié.\n\n{}", warnings.join("\n\n")) };
            Ok((Some(keep), message))
        });
    }

    pub(super) fn redact_zone(&mut self, page: usize, rect: Rect) {
        let keep = self.tab().map(|t| t.selected.clone()).unwrap_or_default();
        self.edit_report(move |pdf| {
            textedit::mark_redaction(pdf, page, rect)?;
            Ok((Some(keep), "Zone marquée. Rien n'est retiré avant « Appliquer les caviardages ».".into()))
        });
    }

    pub(super) fn redact_text(&mut self, query: String) {
        let keep = self.tab().map(|t| t.selected.clone()).unwrap_or_default();
        self.edit_report(move |pdf| {
            let n = textedit::mark_text(pdf, &query)?;
            let message = match n {
                0 => format!("« {query} » n'apparaît pas dans le document."),
                n => format!("{n} occurrence{} de « {query} » marquée{}. Vérifiez, puis « Appliquer les caviardages ».",
                             if n > 1 { "s" } else { "" }, if n > 1 { "s" } else { "" }),
            };
            Ok((Some(keep), message))
        });
    }

    pub(super) fn redact_apply(&mut self) {
        let Some(id) = self.tab().map(|t| t.id) else { return };
        self.engine.run(move |docs| {
            let n = docs.get(&id).map_or(0, textedit::pending_redactions);
            from_engine(move |app| {
                if n == 0 {
                    return app.toast("Aucune zone marquée : tracez-en avec l'outil « Caviarder » ou « Caviarder un texte… ».");
                }
                let text = format!("Supprimer définitivement le contenu de {n} zone{} marquée{} (texte, images, tracés, \
                                    commentaires et champs qui les touchent) ?\n\nCtrl+Z reste possible tant que le document \
                                    n'est pas enregistré.", if n > 1 { "s" } else { "" }, if n > 1 { "s" } else { "" });
                if !super::edit_ui::ask("Appliquer les caviardages", &text) {
                    return;
                }
                let keep = app.tabs.iter().find(|t| t.id == id).map(|t| t.selected.clone()).unwrap_or_default();
                app.edit_doc(id, move |pdf| {
                    let (zones, removed) = textedit::apply_redactions(pdf)?;
                    Ok((Some(keep), format!("{zones} zone{} caviardée{}{}.\nEnregistrez (Ctrl+S) : le fichier enregistré n'en \
                                             contiendra plus aucune trace.", if zones > 1 { "s" } else { "" },
                                            if zones > 1 { "s" } else { "" },
                                            if removed > 0 { format!(", {removed} commentaire(s) ou champ(s) retiré(s)") } else { String::new() })))
                });
            });
        });
    }

    pub fn clean_run(&mut self, metadata: bool, javascript: bool, attachments: bool, thumbnails: bool) {
        if !(metadata || javascript || attachments || thumbnails) {
            return;
        }
        let keep = self.tab().map(|t| t.selected.clone()).unwrap_or_default();
        self.edit_report(move |pdf| {
            let done = textedit::clean(pdf, metadata, javascript, attachments, thumbnails)?;
            Ok((Some(keep), format!("Nettoyé : {} (effectif à l'enregistrement).", done.join(", "))))
        });
    }
}

impl App {
    /// Mot de passe des permissions saisi (fenêtre « Retirer la protection »).
    pub fn owner_password(&mut self, password: String) {
        self.unprotect(Some(password));
    }

    /// Mode capture : état de l'application sur la sortie d'erreur (vérifié par l'audit).
    pub fn dump_state(&mut self) {
        let current = self.current;
        let on_home = self.on_home;
        for (i, t) in self.tabs.iter().enumerate() {
            eprintln!("ETAT : onglet {i}{} « {} » pages={} modifié={} annuler={} rétablir={} commentaires={} champs={} \
                       signé={} page={} zoom={:.2} rotation={} sélection={:?} recherche={}/{}",
                      if i == current && !on_home { "*" } else { "" }, t.name, t.sizes.len(), t.modified, t.can_undo, t.can_redo,
                      t.comments.len(), t.fields.len(), t.signed, t.current + 1, t.zoom, t.rotation, t.selected,
                      t.hit_index.map_or(0, |h| h + 1), t.hits.len());
        }
        let ui = self.ui();
        eprintln!("ETAT : accueil={} outil=« {} » panneau={} rail={} signatures={} différences={} comm.rail={}",
                  on_home, self.tool, ui.get_panel(), ui.get_rail_tab(), slint::Model::row_count(&ui.get_sigs()),
                  slint::Model::row_count(&ui.get_diffs()), slint::Model::row_count(&ui.get_comments()));
    }
}
