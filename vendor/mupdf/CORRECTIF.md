# mupdf 0.8.0, corrigé

Copie de la bibliothèque `mupdf` 0.8.0 (https://github.com/messense/mupdf-rs, licence
AGPL-3.0) avec un seul changement, dans `src/pdf/document.rs`, `PdfDocument::permissions` :
`Permission::from_bits(...)` devient `Permission::from_bits_truncate(...)`.

Sans lui, les permissions d'un PDF chiffré (valeur /P, qui contient toujours des bits
réservés) étaient lues comme « tout est permis ». À retirer dès qu'une version publiée
corrige ce point.
