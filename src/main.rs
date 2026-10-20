//! PDFManager en Rust : MuPDF pour les PDF, Slint pour l'interface.
//! `--selftest [fichier.pdf]` vérifie l'exécutable sans ouvrir de fenêtre (CI Windows).

#![cfg_attr(windows, windows_subsystem = "windows")] // pas de console derrière la fenêtre

mod annot;
mod app;
mod assistant;
#[cfg(feature = "capture")]
mod capture;
mod compare;
mod convert;
mod dialogs;
mod docx;
mod edit;
mod engine;
mod fonts;
mod forms;
mod instance;
mod layout;
mod ocr;
mod pdf;
mod print;
mod security;
mod settings;
mod signature;
mod textedit;
mod updates;

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

slint::include_modules!();

/// Ajoute une ligne au journal (%APPDATA%\PDFManager-rs\pdfmanager.log, 1 Mo au plus).
pub fn log(message: &str) {
    let path = settings::data_dir().join("pdfmanager.log");
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > 1_000_000) {
        let _ = std::fs::rename(&path, path.with_extension("log.1"));
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "{} {message}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"));
    }
}

/// Autotest de l'exécutable : ouverture (chemin accentué), rendu, texte, recherche, signets.
fn selftest(path: Option<&String>) -> ExitCode {
    let check = || -> Result<(), String> {
        let path = path.ok_or("chemin du PDF de test manquant")?;
        let data = std::fs::read(path).map_err(|e| e.to_string())?;
        let pdf = pdf::Pdf::open(&data).map_err(|e| format!("{e:?}"))?;
        let img = pdf.render(0, 1.0, 90)?;
        if img.width == 0 || pdf.words(0).is_empty() || pdf.search(0, "renard").is_empty()
            || pdf.bookmarks().is_empty() {
            return Err("rendu, texte, recherche ou signets absents".into());
        }
        Ok(())
    };
    match check() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            log(&format!("Autotest en échec : {e}"));
            eprintln!("Autotest en échec : {e}");
            ExitCode::FAILURE
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--ocr-worker") {
        return ocr::worker_main(&args[1..]);
    }
    if args.first().map(String::as_str) == Some("--selftest") {
        return selftest(args.get(1));
    }
    #[cfg(feature = "capture")]
    if args.first().map(String::as_str) == Some("--capture") {
        return capture::run(&args[1..]);
    }
    let files: Vec<PathBuf> = args.iter().map(PathBuf::from).collect();
    let channel = instance::channel_name();
    if instance::send_to_running(&channel, &files) {
        return ExitCode::SUCCESS; // la fenêtre déjà ouverte s'en charge
    }
    log(&format!("Démarrage de PDFManager (Rust) {}", env!("CARGO_PKG_VERSION")));
    let ui = match AppWindow::new() {
        Ok(ui) => ui,
        Err(e) => {
            log(&format!("Fenêtre impossible : {e}"));
            return ExitCode::FAILURE;
        }
    };
    app::install(&ui, files);
    instance::serve(&channel, |paths| {
        let _ = slint::invoke_from_event_loop(move || app::with_app(move |a| a.bring_to_front(paths)));
    });
    if let Err(e) = ui.run() {
        log(&format!("Erreur de la boucle d'interface : {e}"));
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
