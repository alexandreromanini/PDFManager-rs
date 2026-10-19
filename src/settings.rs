//! Réglages et journal : %APPDATA%\PDFManager-rs (Linux : ~/.config/PDFManager-rs).
//! Écriture atomique (fichier temporaire puis remplacement).

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const MAX_RECENT: usize = 10;

pub fn data_dir() -> PathBuf {
    let dir = dirs::config_dir().unwrap_or_else(std::env::temp_dir).join("PDFManager-rs");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub recent: Vec<String>,
    pub theme: String, // "system", "light" ou "dark"
    pub sidebar: bool,
    /// Auteur des commentaires (vide : nom de session).
    pub author: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { recent: vec![], theme: "system".into(), sidebar: true, author: String::new() }
    }
}

impl Settings {
    pub fn load_from(path: &Path) -> Settings {
        std::fs::read(path).ok().and_then(|d| serde_json::from_slice(&d).ok()).unwrap_or_default()
    }

    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        write_atomic(path, &serde_json::to_vec_pretty(self).expect("réglages"))
    }

    pub fn load() -> Settings {
        Self::load_from(&data_dir().join("settings.json"))
    }

    pub fn save(&self) {
        if let Err(e) = self.save_to(&data_dir().join("settings.json")) {
            crate::log(&format!("Réglages non enregistrés : {e}"));
        }
    }

    /// Le fichier passe en tête des récents (sans doublon, 10 au plus).
    pub fn add_recent(&mut self, path: &Path) {
        let p = path.to_string_lossy().into_owned();
        self.recent.retain(|r| r != &p);
        self.recent.insert(0, p);
        self.recent.truncate(MAX_RECENT);
    }
}

/// Écrit dans un fichier temporaire du même dossier puis le substitue d'un coup : en cas
/// d'erreur, le fichier d'origine reste intact.
pub fn write_atomic(target: &Path, data: &[u8]) -> std::io::Result<()> {
    let tmp = target.with_extension("tmp~");
    let result = (|| {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
        std::fs::rename(&tmp, target)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_files_and_round_trip() {
        let dir = std::env::temp_dir().join(format!("pdfm-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("réglages.json");
        let mut s = Settings::default();
        for i in 0..12 {
            s.add_recent(Path::new(&format!("/docs/{i}.pdf")));
        }
        s.add_recent(Path::new("/docs/5.pdf"));
        assert_eq!(s.recent.len(), MAX_RECENT);
        assert_eq!(s.recent[0], "/docs/5.pdf");
        s.theme = "dark".into();
        s.save_to(&path).unwrap();
        assert_eq!(Settings::load_from(&path), s);
        std::fs::write(&path, b"{ abime").unwrap();
        assert_eq!(Settings::load_from(&path), Settings::default()); // illisible : défauts
        std::fs::remove_dir_all(dir).unwrap();
    }
}
