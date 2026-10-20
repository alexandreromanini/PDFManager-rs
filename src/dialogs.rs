//! Fenêtres du système (choix de fichiers, questions, erreurs), en un seul endroit.
//!
//! En mode capture (`PDFMANAGER_CAPTURE`, tests de bout en bout hors écran), rien ne
//! s'affiche : les choix de fichiers sont lus dans `PDFMANAGER_PICKS` (chemins séparés par
//! « | », pris dans l'ordre), les réponses dans `PDFMANAGER_ANSWERS` (« oui », « non »,
//! ou le numéro du bouton ; « oui » / premier bouton par défaut), et chaque message est
//! écrit sur la sortie d'erreur, pour être vérifié.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

fn scripted() -> bool {
    std::env::var_os("PDFMANAGER_CAPTURE").is_some()
}

fn queue(var: &'static str, store: &'static Mutex<Option<VecDeque<String>>>) -> Option<String> {
    let mut q = store.lock().unwrap();
    q.get_or_insert_with(|| {
        std::env::var(var).unwrap_or_default().split('|').filter(|s| !s.is_empty()).map(String::from).collect()
    }).pop_front()
}

fn next_pick() -> Option<PathBuf> {
    static PICKS: Mutex<Option<VecDeque<String>>> = Mutex::new(None);
    let p = queue("PDFMANAGER_PICKS", &PICKS).map(PathBuf::from);
    eprintln!("CHOIX DE FICHIER : {}", p.as_ref().map_or("(annulé)".into(), |p| p.display().to_string()));
    p
}

fn next_answer() -> Option<String> {
    static ANSWERS: Mutex<Option<VecDeque<String>>> = Mutex::new(None);
    queue("PDFMANAGER_ANSWERS", &ANSWERS)
}

fn dialog(title: &str, filter: (&str, &[&str]), dir: Option<&Path>) -> rfd::FileDialog {
    let mut d = rfd::FileDialog::new().set_title(title);
    if !filter.1.is_empty() {
        d = d.add_filter(filter.0, filter.1);
    }
    if let Some(dir) = dir.filter(|d| d.is_dir()) {
        d = d.set_directory(dir);
    }
    d
}

pub fn pick_file(title: &str, filter: (&str, &[&str]), dir: Option<&Path>) -> Option<PathBuf> {
    if scripted() {
        return next_pick();
    }
    dialog(title, filter, dir).pick_file()
}

pub fn pick_files(title: &str, filter: (&str, &[&str]), dir: Option<&Path>) -> Vec<PathBuf> {
    if scripted() {
        return next_pick().into_iter().collect();
    }
    dialog(title, filter, dir).pick_files().unwrap_or_default()
}

pub fn pick_folder(title: &str, dir: Option<&Path>) -> Option<PathBuf> {
    if scripted() {
        return next_pick();
    }
    dialog(title, ("", &[]), dir).pick_folder()
}

/// Enregistrer sous : `suggested` propose dossier et nom ; l'extension (la première de
/// `filter`) est ajoutée si l'utilisateur ne l'a pas tapée.
pub fn save_file(title: &str, filter: (&str, &[&str]), suggested: &Path) -> Option<PathBuf> {
    let path = if scripted() {
        next_pick()?
    } else {
        let mut d = dialog(title, filter, suggested.parent());
        if let Some(name) = suggested.file_name() {
            d = d.set_file_name(name.to_string_lossy());
        }
        d.save_file()?
    };
    let ok = path.extension().is_some_and(|e| filter.1.iter().any(|x| e.eq_ignore_ascii_case(x)));
    Some(if ok || filter.1.is_empty() {
        path
    } else {
        let mut name = path.into_os_string();
        name.push(format!(".{}", filter.1[0]));
        PathBuf::from(name)
    })
}

/// Question Oui / Non.
pub fn ask(title: &str, text: &str) -> bool {
    if scripted() {
        let a = next_answer().unwrap_or_else(|| "oui".into());
        eprintln!("QUESTION : {title} : {} → {a}", text.replace('\n', " "));
        return a == "oui" || a == "0";
    }
    rfd::MessageDialog::new().set_title(title).set_description(text).set_buttons(rfd::MessageButtons::YesNo)
        .show() == rfd::MessageDialogResult::Yes
}

/// Trois boutons (ex. Enregistrer / Ne pas enregistrer / Annuler) : leur numéro, 2 si la
/// fenêtre est fermée.
pub fn choose(title: &str, text: &str, buttons: [&str; 3]) -> usize {
    if scripted() {
        let a = next_answer().and_then(|a| a.parse().ok()).unwrap_or(0usize).min(2);
        eprintln!("QUESTION : {title} : {} → {}", text.replace('\n', " "), buttons[a]);
        return a;
    }
    let answer = rfd::MessageDialog::new().set_title(title).set_description(text)
        .set_buttons(rfd::MessageButtons::YesNoCancelCustom(buttons[0].into(), buttons[1].into(), buttons[2].into()))
        .show();
    button_index(answer, &buttons)
}

/// Sous Windows (sans « common-controls-v6 »), rfd ignore les libellés : la boîte affiche
/// Oui / Non / Annuler et répond Yes / No / Cancel, et non `Custom`.
fn button_index(answer: rfd::MessageDialogResult, buttons: &[&str; 3]) -> usize {
    match answer {
        rfd::MessageDialogResult::Custom(c) => buttons.iter().position(|b| *b == c).unwrap_or(2),
        rfd::MessageDialogResult::Yes | rfd::MessageDialogResult::Ok => 0,
        rfd::MessageDialogResult::No => 1,
        _ => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rfd::MessageDialogResult as R;

    #[test]
    fn reponses_oui_non_annuler() {
        let b = ["Enregistrer", "Ne pas enregistrer", "Annuler"];
        assert_eq!(button_index(R::Yes, &b), 0);
        assert_eq!(button_index(R::No, &b), 1);
        assert_eq!(button_index(R::Cancel, &b), 2);
        assert_eq!(button_index(R::Custom("Ne pas enregistrer".into()), &b), 1);
    }
}

pub fn error(text: &str) {
    crate::log(text);
    if scripted() {
        eprintln!("ERREUR : {}", text.replace('\n', " "));
        return;
    }
    rfd::MessageDialog::new().set_level(rfd::MessageLevel::Error).set_title("PDFManager").set_description(text).show();
}

pub fn info(title: &str, text: &str) {
    if scripted() {
        eprintln!("INFO : {title} : {}", text.replace('\n', " "));
        return;
    }
    rfd::MessageDialog::new().set_level(rfd::MessageLevel::Info).set_title(title).set_description(text).show();
}
