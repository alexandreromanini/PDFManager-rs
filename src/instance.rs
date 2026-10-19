//! Instance unique : un PDF ouvert depuis l'Explorateur alors que PDFManager tourne déjà
//! s'ajoute en onglet dans la fenêtre existante. Canal local (tube nommé sous Windows,
//! socket Unix sous Linux), une ligne JSON par lancement.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;

use interprocess::local_socket::{prelude::*, GenericNamespaced, ListenerOptions, Stream};

pub fn channel_name() -> String {
    let user = std::env::var("USERNAME").or_else(|_| std::env::var("USER")).unwrap_or_default();
    format!("PDFManager-rs-{user}.sock")
}

/// Une instance écoute déjà : lui confier `paths` (chemins absolus) et renvoyer true.
pub fn send_to_running(name: &str, paths: &[PathBuf]) -> bool {
    let Ok(ns) = name.to_ns_name::<GenericNamespaced>() else { return false };
    let Ok(mut stream) = Stream::connect(ns) else { return false };
    #[cfg(windows)]
    allow_foreground();
    let list: Vec<String> = paths.iter().map(|p| std::path::absolute(p).unwrap_or(p.clone())
        .to_string_lossy().into_owned()).collect();
    let mut line = serde_json::to_string(&list).expect("liste de chemins");
    line.push('\n');
    stream.write_all(line.as_bytes()).is_ok()
}

/// Écoute les lancements suivants dans un thread ; `on_paths` est appelé (dans ce thread)
/// avec les fichiers reçus, éventuellement vide (simple retour au premier plan).
pub fn serve(name: &str, on_paths: impl Fn(Vec<PathBuf>) + Send + 'static) -> bool {
    let Ok(ns) = name.to_ns_name::<GenericNamespaced>() else { return false };
    let Ok(listener) = ListenerOptions::new().name(ns).try_overwrite(true).create_sync() else {
        return false;
    };
    std::thread::Builder::new()
        .name("instance-unique".into())
        .spawn(move || {
            for conn in listener.incoming().flatten() {
                let mut line = String::new();
                if BufReader::new(conn).read_line(&mut line).is_ok() {
                    let paths: Vec<String> = serde_json::from_str(line.trim()).unwrap_or_default();
                    on_paths(paths.into_iter().map(PathBuf::from).collect());
                }
            }
        })
        .is_ok()
}

/// La fenêtre existante a le droit de passer au premier plan (Windows le refuse sinon).
#[cfg(windows)]
fn allow_foreground() {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn AllowSetForegroundWindow(process_id: u32) -> i32;
    }
    unsafe {
        AllowSetForegroundWindow(u32::MAX); // ASFW_ANY
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn second_launch_hands_files_to_the_first() {
        let name = format!("pdfm-test-{}.sock", std::process::id());
        assert!(!send_to_running(&name, &[PathBuf::from("a.pdf")])); // personne n'écoute
        let (tx, rx) = mpsc::channel();
        assert!(serve(&name, move |paths| {
            let _ = tx.send(paths);
        }));
        assert!(send_to_running(&name, &[PathBuf::from("/docs/été.pdf")]));
        let got = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        assert!(got[0].to_string_lossy().ends_with("été.pdf"));
    }
}
