//! Traduction par DeepL (API v2) : clé gratuite (500 000 caractères par mois, terminée par
//! « :fx ») ou payante. Les textes partent par lots (50 au plus, ~100 Ko), en https.
//! Langues : alphabets latin, grec et cyrillique (les polices chinoises, japonaises et
//! coréennes ne sont pas disponibles pour réécrire la page ; arabe et hébreu s'écrivent de
//! droite à gauche).

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

pub const KEY_URL: &str = "https://www.deepl.com/pro-api";

/// (code DeepL, nom affiché), dans l'ordre alphabétique français.
pub const LANGUAGES: [(&str, &str); 28] = [
    ("DE", "Allemand"), ("EN-US", "Anglais (américain)"), ("EN-GB", "Anglais (britannique)"), ("BG", "Bulgare"),
    ("DA", "Danois"), ("ES", "Espagnol"), ("ET", "Estonien"), ("FI", "Finnois"), ("FR", "Français"), ("EL", "Grec"),
    ("HU", "Hongrois"), ("ID", "Indonésien"), ("IT", "Italien"), ("LV", "Letton"), ("LT", "Lituanien"),
    ("NL", "Néerlandais"), ("NB", "Norvégien"), ("PL", "Polonais"), ("PT-BR", "Portugais (Brésil)"),
    ("PT-PT", "Portugais (Portugal)"), ("RO", "Roumain"), ("RU", "Russe"), ("SK", "Slovaque"), ("SL", "Slovène"),
    ("SV", "Suédois"), ("CS", "Tchèque"), ("TR", "Turc"), ("UK", "Ukrainien"),
];

const BATCH_TEXTS: usize = 50;
const BATCH_BYTES: usize = 100_000; // limite DeepL : 128 Kio par requête

fn endpoint(key: &str) -> &'static str {
    if key.trim().ends_with(":fx") { "https://api-free.deepl.com/v2/translate" } else { "https://api.deepl.com/v2/translate" }
}

/// Lots consécutifs : (début, fin) dans `texts`.
fn batches(texts: &[String]) -> Vec<(usize, usize)> {
    let mut out = vec![];
    let (mut start, mut bytes) = (0, 0);
    for (i, t) in texts.iter().enumerate() {
        if i > start && (i - start == BATCH_TEXTS || bytes + t.len() > BATCH_BYTES) {
            out.push((start, i));
            (start, bytes) = (i, 0);
        }
        bytes += t.len() + 16;
    }
    if start < texts.len() {
        out.push((start, texts.len()));
    }
    out
}

fn request_body(texts: &[String], target: &str) -> String {
    serde_json::json!({ "text": texts, "target_lang": target }).to_string()
}

fn parse_response(body: &str, expected: usize) -> Result<Vec<String>, String> {
    let v: serde_json::Value = serde_json::from_str(body).map_err(|_| "réponse de DeepL illisible".to_string())?;
    let out: Vec<String> = v["translations"].as_array().ok_or("réponse de DeepL illisible")?
        .iter().map(|t| t["text"].as_str().unwrap_or_default().to_string()).collect();
    if out.len() != expected {
        return Err("réponse de DeepL incomplète".into());
    }
    Ok(out)
}

fn status_message(code: u16) -> String {
    match code {
        403 => "Clé DeepL refusée : vérifiez-la (compte DeepL, rubrique « Clés d'API »).".into(),
        456 => "Quota DeepL du mois épuisé (500 000 caractères avec une clé gratuite).".into(),
        429 | 529 => "DeepL reçoit trop de demandes : réessayez dans quelques minutes.".into(),
        c => format!("DeepL a répondu par une erreur ({c})."),
    }
}

/// Traduit `texts` vers `target` (code de `LANGUAGES`). `progress(textes faits, total)`.
/// `cancel` arrête entre deux lots.
pub fn translate(key: &str, texts: &[String], target: &str, cancel: &AtomicBool,
                 mut progress: impl FnMut(usize, usize)) -> Result<Vec<String>, String> {
    let key = key.trim();
    if key.is_empty() {
        return Err("Indiquez votre clé DeepL.".into());
    }
    let agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(60))).build().new_agent();
    let mut out = Vec::with_capacity(texts.len());
    for (a, b) in batches(texts) {
        if cancel.load(Ordering::Relaxed) {
            return Err("Traduction arrêtée.".into());
        }
        let body = request_body(&texts[a..b], target);
        let mut wait = 1;
        let reply = loop {
            let r = agent.post(endpoint(key))
                .header("Authorization", &format!("DeepL-Auth-Key {key}"))
                .header("Content-Type", "application/json")
                .send(body.as_str());
            match r {
                // trop de demandes ou serveur surchargé : on réessaie (1, 2, 4 s)
                Err(ureq::Error::StatusCode(c)) if (c == 429 || c >= 500) && wait <= 4 => {
                    std::thread::sleep(Duration::from_secs(wait));
                    wait *= 2;
                }
                Err(ureq::Error::StatusCode(c)) => return Err(status_message(c)),
                Err(e) => return Err(format!("Impossible de joindre DeepL ({e}) : vérifiez la connexion Internet.")),
                Ok(mut resp) => break resp.body_mut().read_to_string().map_err(|e| e.to_string())?,
            }
        };
        out.extend(parse_response(&reply, b - a)?);
        progress(b, texts.len());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lots_requete_reponse() {
        let texts: Vec<String> = (0..120).map(|i| format!("phrase {i}")).collect();
        assert_eq!(batches(&texts), vec![(0, 50), (50, 100), (100, 120)]);
        let long = vec!["x".repeat(60_000), "y".repeat(60_000), "z".into()];
        assert_eq!(batches(&long), vec![(0, 1), (1, 3)]);
        assert_eq!(endpoint("abc:fx"), "https://api-free.deepl.com/v2/translate");
        assert_eq!(endpoint("abc"), "https://api.deepl.com/v2/translate");
        let body: serde_json::Value = serde_json::from_str(&request_body(&["Été « 1 »".into()], "EN-US")).unwrap();
        assert_eq!(body["text"][0], "Été « 1 »");
        assert_eq!(body["target_lang"], "EN-US");
        let r = r#"{"translations":[{"detected_source_language":"FR","text":"Summer"},{"text":"two"}]}"#;
        assert_eq!(parse_response(r, 2).unwrap(), vec!["Summer", "two"]);
        assert!(parse_response(r, 3).is_err());
        assert!(status_message(456).contains("Quota"));
    }

    /// Avec une vraie clé : DEEPL_KEY=… cargo test --release deepl -- --ignored
    #[test]
    #[ignore]
    fn vraie_traduction() {
        let key = std::env::var("DEEPL_KEY").expect("DEEPL_KEY");
        let out = translate(&key, &["Bonjour le monde".into()], "EN-US", &AtomicBool::new(false), |_, _| {}).unwrap();
        assert!(out[0].to_lowercase().contains("hello"), "{out:?}");
    }
}
