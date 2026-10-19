//! Comparaison du texte de deux PDF, mot à mot, sur tout le document (un ajout qui décale
//! la suite sur la page suivante ne rend pas tout « différent »). Sans interface.

use similar::{capture_diff_slices, Algorithm, DiffOp};

use crate::pdf::{Pdf, Rect};

/// (page, cadre du mot, mot)
pub type Located = (usize, Rect, String);

#[derive(Debug, Clone, PartialEq)]
pub struct Change {
    /// "added", "removed" ou "changed".
    pub kind: &'static str,
    /// Mots du premier document.
    pub old: Vec<Located>,
    /// Mots du second.
    pub new: Vec<Located>,
}

impl Change {
    pub fn old_text(&self) -> String {
        self.old.iter().map(|w| w.2.as_str()).collect::<Vec<_>>().join(" ")
    }
    pub fn new_text(&self) -> String {
        self.new.iter().map(|w| w.2.as_str()).collect::<Vec<_>>().join(" ")
    }
}

pub fn document_words(pdf: &Pdf) -> Vec<Located> {
    (0..pdf.page_count()).flat_map(|i| pdf.words(i).into_iter().map(move |w| (i, w.rect, w.text))).collect()
}

/// Différences entre deux suites de mots (comparaison exacte, casse comprise), par
/// l'algorithme de Myers : rapide pour des versions proches.
pub fn compare(a: &[Located], b: &[Located]) -> Vec<Change> {
    let wa: Vec<&str> = a.iter().map(|w| w.2.as_str()).collect();
    let wb: Vec<&str> = b.iter().map(|w| w.2.as_str()).collect();
    capture_diff_slices(Algorithm::Myers, &wa, &wb).into_iter().filter_map(|op| {
        let (kind, old, new) = match op {
            DiffOp::Equal { .. } => return None,
            DiffOp::Delete { old_index, old_len, .. } => ("removed", old_index..old_index + old_len, 0..0),
            DiffOp::Insert { new_index, new_len, .. } => ("added", 0..0, new_index..new_index + new_len),
            DiffOp::Replace { old_index, old_len, new_index, new_len } => {
                ("changed", old_index..old_index + old_len, new_index..new_index + new_len)
            }
        };
        Some(Change { kind, old: a[old].to_vec(), new: b[new].to_vec() })
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(words: &str, page: usize) -> Vec<Located> {
        words.split(' ').map(|t| (page, Rect::default(), t.to_string())).collect()
    }

    #[test]
    fn differences_mot_a_mot() {
        let a = [w("le renard brun saute", 0), w("par dessus le chien", 1)].concat();
        // « brun » → « roux », « vite » ajouté ; le reste, décalé sur une autre page, est égal.
        let b = [w("le renard roux saute vite", 0), w("par", 0), w("dessus le chien", 1)].concat();
        let c = compare(&a, &b);
        assert_eq!(c.len(), 2, "{c:?}");
        assert_eq!((c[0].kind, c[0].old_text().as_str(), c[0].new_text().as_str()), ("changed", "brun", "roux"));
        assert_eq!((c[1].kind, c[1].new_text().as_str()), ("added", "vite"));
        assert!(compare(&a, &a).is_empty());
    }

    #[test]
    fn deux_documents() {
        let a = Pdf::open(&std::fs::read("tests/fixtures/Été déjà vu.pdf").unwrap()).unwrap();
        let mut b = Pdf::open(&std::fs::read("tests/fixtures/Été déjà vu.pdf").unwrap()).unwrap();
        crate::edit::delete(&mut b, &[1]).unwrap();
        let c = compare(&document_words(&a), &document_words(&b));
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].kind, "removed");
        // Les 7 mots d'une page (texte répétitif : l'alignement peut les prendre à cheval).
        assert_eq!(c[0].old.len(), 7, "{:?}", c[0]);
    }
}
