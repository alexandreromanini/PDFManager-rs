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
| 0 | Essai : ouvrir un PDF, faire défiler ses pages, build Windows | en cours |

## Développement (Linux)

```bash
cargo run --release -- fichier.pdf
cargo test
```

Si la compilation de `mupdf-sys` échoue sur « 'stddef.h' file not found » (en-têtes de clang
absents) : `export BINDGEN_EXTRA_CLANG_ARGS=-I/usr/lib/gcc/x86_64-linux-gnu/13/include`, ou
installer `libclang-common-18-dev`.

## Licence

AGPL-3.0-or-later (MuPDF est sous AGPL). Interface réalisée avec Slint.
