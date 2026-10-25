# Développement

Compiler, tester et comprendre PDFManager-rs. Retour à l'[accueil du projet](../README.fr.md).

## Développement (Linux)

```bash
cargo run --release -- fichier.pdf
cargo test --release
# Captures d'écran hors écran (développement) :
cargo run --release --features capture -- --capture sortie.ppm 1280 820 light fichier.pdf -- search:mot
# Parcours de bout en bout (154 scénarios, l'appli pilotée hors écran ; pip install pymupdf) :
cargo build --release --features capture && python tests/parcours.py
```

Si la compilation de `mupdf-sys` échoue sur « 'stddef.h' file not found » (en-têtes de clang
absents) : `export BINDGEN_EXTRA_CLANG_ARGS=-I/usr/lib/gcc/x86_64-linux-gnu/13/include`, ou
installer `libclang-common-18-dev`.

## Organisation

- `src/` : le code. `pdf.rs` (document MuPDF, annuler/rétablir), `engine.rs` (fil qui possède les
  documents), `app.rs` et `*_ui.rs` (contrôleur de l'interface), modules sans interface
  (`edit.rs`, `annot.rs`, `textedit.rs`, `ocr.rs`, `signature.rs`, `translate.rs`…).
- `ui/` : l'interface Slint. `resources/` : icônes, logo, modèles OCR, profil sRGB.
- `vendor/mupdf` : le crate `mupdf` avec un correctif (voir `vendor/mupdf/CORRECTIF.md`).
- `tests/parcours.py` : scénarios de bout en bout, l'application pilotée hors écran.
- [AUDIT.md](AUDIT.md) : audit complet ; [COMPARAISON.md](COMPARAISON.md) : comparaison avec la
  version Python.

## Licences des ressources

AGPL-3.0-or-later (MuPDF est sous AGPL). Interface réalisée avec Slint. Profil `resources/sRGB.icc`
(OutputIntent des PDF/A) : celui de pikepdf, produit par lcms, « no copyright, use freely ».
