# PDFManager (Rust)

Réécriture en Rust de [PDFManager](https://github.com/alexandreromanini/PDFManager) (Python),
pour une application plus légère : installeur plus petit, démarrage instantané, moins de
mémoire. La version Python reste la référence tant que celle-ci ne couvre pas les mêmes
usages.

- Moteur PDF : [MuPDF](https://mupdf.com) via le crate `mupdf` (le même moteur que PyMuPDF).
- Interface : [Slint](https://slint.dev).
- Cible : Windows 11 64 bits ; développement sous Linux.

## État

| Phase | Contenu | État |
|---|---|---|
| 0 | Essai : ouvrir un PDF, faire défiler ses pages, build Windows | ✅ |
| 1 | Lecteur complet (voir ci-dessous) | ✅ 0.1.0 |
| 2 | Enregistrer, annuler/rétablir, organiser les pages | ✅ 0.2.0 |
| 3 | Commentaires, ajouter du texte et des images, en-têtes, numérotation, filigrane | à venir |
| 4 | Formulaires, protection, impression, mises à jour | à venir |
| 5 | Modifier le texte existant, caviardage | à venir |
| 6 | OCR, assistant IA, conversions | à venir |
| 7 | Signatures, comparaison | à venir |

## Fonctions (phase 1)

- Accueil : ouvrir (dialogue ou glisser-déposer), fichiers récents.
- Onglets, une seule fenêtre : un PDF ouvert depuis l'Explorateur arrive en onglet.
- Vue fluide : rendu en arrière-plan (thread moteur), seules les pages visibles sont
  dessinées ; modes continu, page unique, double page ; zoom (Ctrl+molette, Ctrl+/-),
  ajuster à la largeur (Ctrl+2) ou à la page (Ctrl+0) ; pivoter l'affichage.
- Rail de droite : vignettes (rendues à la demande), signets.
- Recherche (Ctrl+F, F3 / Maj+F3), surlignage, compteur « 3 / 12 ».
- Sélection du texte à la souris, Ctrl+C.
- Liens : vers une page, ou web / e-mail après confirmation (les autres sont refusés).
- Mots de passe (3 essais), thème clair / sombre / système, plein écran (F11),
  panneau latéral (F4).
- Réglages et journal : `%APPDATA%\PDFManager-rs`.

## Fonctions (phase 2)

- Enregistrer (Ctrl+S, confirmation la première fois : le fichier d'origine est remplacé),
  Enregistrer sous (Ctrl+Maj+S), Enregistrer une copie ; écriture atomique ; avertissement
  pour un document signé.
- Annuler / Rétablir (Ctrl+Z / Ctrl+Y) pour chaque modification, 30 étapes.
- Panneau « Organiser les pages » : pivoter, supprimer, extraire, insérer (page vierge, PDF,
  images), recadrer (tracer la zone à garder), annuler le recadrage, fusionner, scinder.
- Vignettes : sélection multiple (Ctrl+clic, Maj+clic), glisser pour déplacer des pages,
  Suppr pour supprimer.
- Fermer un onglet ou quitter avec des modifications : Enregistrer / Ne pas enregistrer /
  Annuler.

## Installation

Installeur `PDFManager-rs-<version>-setup.exe` (onglet Actions du dépôt, artefact
`PDFManager-rs-setup`). S'installe sous le nom « PDFManager (Rust) », à côté de la version
Python, sans la remplacer.

## Développement (Linux)

```bash
cargo run --release -- fichier.pdf
cargo test --release
# Captures d'écran hors écran (développement) :
cargo run --release --features capture -- --capture sortie.ppm 1280 820 light fichier.pdf -- search:mot
```

Si la compilation de `mupdf-sys` échoue sur « 'stddef.h' file not found » (en-têtes de clang
absents) : `export BINDGEN_EXTRA_CLANG_ARGS=-I/usr/lib/gcc/x86_64-linux-gnu/13/include`, ou
installer `libclang-common-18-dev`.

## Licence

AGPL-3.0-or-later (MuPDF est sous AGPL). Interface réalisée avec Slint.
