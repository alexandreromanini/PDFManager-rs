//! Polices installées sur l'ordinateur, retrouvées par le nom que cite le PDF
//! (« ABCDEF+Calibri-Bold », « ArialMT », « Arial,Bold »). Un texte modifié garde ainsi sa
//! vraie police quand le PDF ne l'intègre qu'en partie (le cas le plus courant). La police
//! n'est pas intégrée entière : seuls les caractères utilisés le sont. Comme core/fonts.py.

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::OnceLock;

fn dirs() -> Vec<PathBuf> {
    if cfg!(windows) {
        let windir = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into());
        let local = std::env::var("LOCALAPPDATA").unwrap_or_default();
        vec![PathBuf::from(windir).join("Fonts"), PathBuf::from(local).join(r"Microsoft\Windows\Fonts")]
    } else {
        let home = dirs::home_dir().unwrap_or_default();
        vec!["/usr/share/fonts".into(), "/usr/local/share/fonts".into(), home.join(".local/share/fonts"), home.join(".fonts")]
    }
}

/// Nom comparable : sans préfixe de sous-ensemble, minuscules, lettres et chiffres.
pub fn key(name: &str) -> String {
    name.rsplit('+').next().unwrap_or(name).chars().filter(|c| c.is_ascii_alphanumeric()).flat_map(char::to_lowercase).collect()
}

fn walk(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("ttf") || x.eq_ignore_ascii_case("otf")) {
            out.push(p); // ponytail : collections .ttc ignorées (Cambria, polices asiatiques)
        }
    }
}

/// Nom PostScript et nom complet de chaque police installée → fichier. Lu une fois.
pub fn index() -> &'static HashMap<String, PathBuf> {
    static INDEX: OnceLock<HashMap<String, PathBuf>> = OnceLock::new();
    INDEX.get_or_init(|| {
        let mut files = vec![];
        for d in dirs() {
            walk(&d, &mut files);
        }
        files.sort();
        let mut found = HashMap::new();
        for path in files {
            let Ok(data) = std::fs::read(&path) else { continue };
            let Ok(face) = ttf_parser::Face::parse(&data, 0) else { continue };
            for name in face.names() {
                if matches!(name.name_id, ttf_parser::name_id::FULL_NAME | ttf_parser::name_id::POST_SCRIPT_NAME)
                    && let Some(n) = name.to_string() {
                        found.entry(key(&n)).or_insert_with(|| path.clone());
                    }
            }
        }
        found
    })
}

/// Fichier de la police installée qui porte ce nom.
pub fn find(pdf_name: &str) -> Option<PathBuf> {
    let k = key(pdf_name);
    let fonts = index();
    if let Some(p) = fonts.get(&k) {
        return Some(p.clone());
    }
    // « ArialMT », « TimesNewRomanPSMT », « Arial Regular » (nom complet : « Arial »)
    ["psmt", "mt", "ps", "regular", "normal", "roman", "book"].iter()
        .find_map(|s| k.strip_suffix(s).and_then(|base| fonts.get(base)).cloned())
}

/// Police installée de la famille ("sans", "serif", "mono") et son libellé (Windows).
pub fn generic(family: &str, bold: bool, italic: bool) -> Option<(PathBuf, String)> {
    let (label, names) = match family {
        "serif" => ("Times New Roman", ["TimesNewRomanPSMT", "TimesNewRomanPS-BoldMT", "TimesNewRomanPS-ItalicMT", "TimesNewRomanPS-BoldItalicMT"]),
        "mono" => ("Courier New", ["CourierNewPSMT", "CourierNewPS-BoldMT", "CourierNewPS-ItalicMT", "CourierNewPS-BoldItalicMT"]),
        _ => ("Arial", ["ArialMT", "Arial-BoldMT", "Arial-ItalicMT", "Arial-BoldItalicMT"]),
    };
    let path = find(names[bold as usize + 2 * italic as usize])?;
    Some((path, format!("{label}{}{}", if bold { " gras" } else { "" }, if italic { " italique" } else { "" })))
}

/// La police réduite aux caractères de `text` (quelques Ko au lieu de quelques Mo).
pub fn subset(path: &std::path::Path, text: &str) -> Result<Vec<u8>, String> {
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    // Pas d'espace insécable d'office : même glyphe que l'espace, elle la remplacerait
    // dans le texte extrait (copier, rechercher).
    let chars: BTreeSet<char> = text.chars().chain([' ']).collect();
    oxifont_subset::subset_font(&data, &chars).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noms() {
        assert_eq!(key("ABCDEF+Calibri-Bold"), "calibribold");
        assert_eq!(key("Arial,Bold"), "arialbold");
    }

    /// Une police installée (Arial sous Windows, DejaVu sous Linux) réduite puis relue.
    #[test]
    fn police_installee_reduite() {
        let Some(path) = find("ArialMT").or_else(|| find("DejaVuSans")) else {
            eprintln!("aucune police de test installée : ignoré");
            return;
        };
        let full = std::fs::metadata(&path).unwrap().len();
        let small = subset(&path, "Été à 10 €").unwrap();
        assert!((small.len() as u64) < full / 4, "{} / {full}", small.len());
        let font = mupdf::Font::from_bytes("test", &small).unwrap();
        for c in "Été€".chars() {
            let g = font.encode_character(c as i32).unwrap();
            assert!(g > 0 && font.outline_glyph(g).unwrap().is_some(), "{c}");
        }
    }
}
