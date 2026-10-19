//! Mises à jour : dernière version publiée sur GitHub (Releases du dépôt), téléchargement
//! vérifié de l'installeur (taille, empreinte SHA-256) puis installation silencieuse.
//! Comme la version Python (services/updates.py). Appelé hors du thread de l'interface.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

pub const RELEASES: &str = "https://github.com/alexandreromanini/PDFManager-rs/releases";
const API: &str = "https://api.github.com/repos/alexandreromanini/PDFManager-rs/releases/latest";
const DOWNLOADS: &str = "https://github.com/alexandreromanini/PDFManager-rs/releases/download/";

#[derive(Debug, Clone, PartialEq)]
pub struct Release {
    pub version: String,
    pub notes: String,
    pub url: String,
    pub size: u64,
    /// Empreinte publiée par GitHub (champ « digest » de l'API).
    pub sha256: Option<String>,
}

/// « v0.10.2 » → [0, 10, 2] : comparaison numérique, pas alphabétique.
pub fn parse_version(text: &str) -> Vec<u32> {
    text.split(|c: char| !c.is_ascii_digit()).filter(|s| !s.is_empty()).take(3)
        .map(|n| n.parse().unwrap_or(0)).collect()
}

pub fn is_newer(release: &Release, current: &str) -> bool {
    parse_version(&release.version) > parse_version(current)
}

fn agent() -> String {
    format!("PDFManager-rs/{}", env!("CARGO_PKG_VERSION"))
}

/// Lit la réponse de l'API : la dernière version qui a un installeur, ou `None`.
pub fn parse_release(json: &str) -> Option<Release> {
    let data: serde_json::Value = serde_json::from_str(json).ok()?;
    let tag = data["tag_name"].as_str()?;
    data["assets"].as_array()?.iter().find_map(|a| {
        let name = a["name"].as_str()?.to_lowercase();
        let url = a["browser_download_url"].as_str()?;
        // seul un installeur publié sur ce dépôt, en https, est accepté
        if !name.ends_with("-setup.exe") || !url.starts_with(DOWNLOADS) {
            return None;
        }
        Some(Release {
            version: tag.trim_start_matches('v').to_string(),
            notes: data["body"].as_str().unwrap_or("").to_string(),
            url: url.to_string(),
            size: a["size"].as_u64()?,
            sha256: a["digest"].as_str().and_then(|d| d.strip_prefix("sha256:")).map(str::to_lowercase),
        })
    })
}

/// La dernière version publiée, `None` s'il n'y en a pas ; erreur : hors ligne…
pub fn latest() -> Result<Option<Release>, String> {
    let resp = ureq::get(API).header("User-Agent", &agent()).header("Accept", "application/vnd.github+json").call();
    match resp {
        Ok(mut r) => Ok(parse_release(&r.body_mut().read_to_string().map_err(|e| e.to_string())?)),
        Err(ureq::Error::StatusCode(404)) => Ok(None), // aucune version publiée
        Err(e) => Err(e.to_string()),
    }
}

/// Dossier des téléchargements (versions précédentes effacées au suivant).
pub fn folder() -> PathBuf {
    std::env::temp_dir().join("PDFManager-rs-update")
}

/// Télécharge l'installeur, vérifié. `progress(Mo reçus, Mo au total)`.
pub fn download(release: &Release, progress: impl FnMut(u64, u64)) -> Result<PathBuf, String> {
    let dir = folder();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    for old in std::fs::read_dir(&dir).map_err(|e| e.to_string())?.flatten() {
        let _ = std::fs::remove_file(old.path());
    }
    let target = dir.join(format!("PDFManager-rs-{}-setup.exe", release.version));
    fetch(&release.url, release.size, release.sha256.as_deref(), &target, progress)?;
    Ok(target)
}

/// Télécharge `url` vers `target` (via un fichier .part), taille et empreinte vérifiées :
/// un fichier tronqué ou altéré n'est jamais gardé.
pub fn fetch(url: &str, size: u64, sha256: Option<&str>, target: &Path, mut progress: impl FnMut(u64, u64))
             -> Result<(), String> {
    let part = target.with_extension(format!("{}.part", target.extension().map(|e| e.to_string_lossy()).unwrap_or_default()));
    let result = fetch_part(url, size, sha256, &part, &mut progress);
    if let Err(e) = result {
        let _ = std::fs::remove_file(&part);
        return Err(e);
    }
    std::fs::rename(&part, target).map_err(|e| e.to_string())
}

fn fetch_part(url: &str, size: u64, sha256: Option<&str>, part: &Path, progress: &mut impl FnMut(u64, u64))
              -> Result<(), String> {
    let mut resp = ureq::get(url).header("User-Agent", &agent()).call().map_err(|e| e.to_string())?;
    let mut reader = resp.body_mut().as_reader();
    let mut out = std::fs::File::create(part).map_err(|e| e.to_string())?;
    let (mut sha, mut done, mut buf) = (Sha256::new(), 0u64, vec![0u8; 256 * 1024]);
    let mut shown = u64::MAX;
    loop {
        let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        done += n as u64;
        if done > size {
            break; // plus long qu'annoncé : refusé plus bas
        }
        out.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        sha.update(&buf[..n]);
        if done >> 20 != shown {
            shown = done >> 20;
            progress(shown, (size >> 20).max(1));
        }
    }
    out.flush().map_err(|e| e.to_string())?;
    let digest: String = sha.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if done != size || sha256.is_some_and(|s| !s.eq_ignore_ascii_case(&digest)) {
        return Err("Le fichier téléchargé est incomplet ou altéré ; réessayez plus tard.".into());
    }
    Ok(())
}

/// Lance l'installeur sans assistant ; il remplace l'application (qui se ferme aussitôt)
/// puis la relance. Le choix « pour moi / pour tous » d'origine est conservé.
pub fn install(installer: &Path) -> Result<(), String> {
    std::process::Command::new(installer)
        .args(["/SILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/CLOSEAPPLICATIONS"])
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Impossible de lancer l'installeur : {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert_eq!(parse_version("v0.10.2"), [0, 10, 2]);
        let r = |v: &str| Release { version: v.into(), notes: String::new(), url: String::new(), size: 0, sha256: None };
        assert!(is_newer(&r("0.10.0"), "0.9.9"));
        assert!(!is_newer(&r("0.3.0"), "0.3.0"));
        assert!(!is_newer(&r("0.2.9"), "0.3.0"));
    }

    #[test]
    fn reponse_de_l_api() {
        let json = format!(r#"{{"tag_name": "v0.4.0", "body": "Notes", "assets": [
            {{"name": "autre.zip", "browser_download_url": "{DOWNLOADS}v0.4.0/autre.zip", "size": 1}},
            {{"name": "PDFManager-rs-0.4.0-setup.exe", "browser_download_url": "{DOWNLOADS}v0.4.0/PDFManager-rs-0.4.0-setup.exe",
              "size": 1234, "digest": "sha256:ABCD"}}]}}"#);
        let r = parse_release(&json).unwrap();
        assert_eq!((r.version.as_str(), r.size, r.sha256.as_deref()), ("0.4.0", 1234, Some("abcd")));
        // Un installeur hébergé ailleurs est ignoré.
        let evil = json.replace(DOWNLOADS, "https://exemple.com/");
        assert_eq!(parse_release(&evil), None);
        assert_eq!(parse_release("{}"), None);
    }

    /// Réseau : `cargo test -- --ignored en_ligne`.
    #[test]
    #[ignore]
    fn en_ligne() {
        let r = latest().unwrap().expect("une version publiée");
        eprintln!("{r:?}");
        assert!(r.size > 1_000_000 && r.sha256.is_some());
        let path = download(&r, |_, _| {}).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().len(), r.size);
        // Empreinte fausse : refusé, rien de gardé.
        let bad = Release { sha256: Some("00".into()), ..r };
        assert!(download(&bad, |_, _| {}).is_err());
        assert!(std::fs::read_dir(folder()).unwrap().next().is_none());
    }
}
