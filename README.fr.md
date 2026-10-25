<div align="center">

<img src="resources/logo/pdfmanager.svg" width="88" alt="Logo de PDFManager">

# PDFManager

**Un éditeur PDF gratuit et hors ligne pour Windows, écrit en Rust.**<br>
Modifier, commenter, signer, reconnaître le texte (OCR), traduire et alléger vos PDF : sans
compte, sans filigrane, sans abonnement.

[![Dernière version](https://img.shields.io/github/v/release/alexandreromanini/PDFManager-rs?label=version&color=d93a2b)](https://github.com/alexandreromanini/PDFManager-rs/releases/latest)
[![Compilation](https://github.com/alexandreromanini/PDFManager-rs/actions/workflows/build-windows.yml/badge.svg)](https://github.com/alexandreromanini/PDFManager-rs/actions/workflows/build-windows.yml)
[![Plateforme](https://img.shields.io/badge/Windows-10%20%7C%2011-0078D6)](#télécharger)
[![Rust](https://img.shields.io/badge/rust-Slint%20%2B%20MuPDF-B7410E)](docs/DEVELOPMENT.md)
[![Licence](https://img.shields.io/badge/licence-AGPL--3.0-blue)](LICENSE)

[**Télécharger pour Windows**](https://github.com/alexandreromanini/PDFManager-rs/releases/latest) ·
[Fonctions](#fonctions) ·
[Captures](#captures-décran) ·
[Toutes les fonctions](docs/FEATURES.md) ·
[English](README.md)

<img src="docs/images/screenshot.png" width="900" alt="PDFManager affichant un rapport commenté, avec les outils de commentaire à gauche et les vignettes des pages à droite">

</div>

## Pourquoi PDFManager ?

- **Tout ce qu'on attend d'un éditeur PDF, dans une seule application** : les fonctions
  courantes d'Acrobat Pro (modification, commentaires, formulaires, signatures, OCR,
  conversions), gratuites et open source.
- **Léger et rapide** : un installeur de 16 Mo, un démarrage immédiat, le moteur
  [MuPDF](https://mupdf.com) utilisé par de nombreux lecteurs PDF.
- **Respect de la vie privée** : vos documents sont traités sur votre ordinateur. Rien n'est
  envoyé en ligne, sauf le texte que vous choisissez de traduire avec DeepL.
- **Interface Windows 11 moderne**, thèmes clair et sombre, onglets, glisser-déposer,
  raccourcis clavier.
- **Modifications sûres** : tout s'annule, les fichiers sont enregistrés sans risque de les
  corrompre, et un document signé est complété sans toucher à ses signatures.

## Fonctions

| | |
|---|---|
| **Afficher et organiser** | Onglets, vignettes, signets, recherche, affichage continu, page unique ou double page, mode présentation. Réordonner, pivoter, supprimer, extraire, recadrer et insérer des pages ; fusionner et scinder des PDF. |
| **Modifier** | Modifier le texte existant sur place (police d'origine gardée si possible), ajouter du texte, des images et des liens, déplacer et redimensionner les images, en-têtes, pieds de page, numéros et filigranes. Caviardage qui supprime vraiment le contenu. |
| **Commenter** | Surligner, souligner, barrer, notes, texte libre, formes, dessin à main levée et tampons, avec la liste de tous les commentaires. |
| **Formulaires** | Remplir les formulaires, et créer des champs texte, cases à cocher, listes et champs de signature. |
| **Signer et protéger** | Signature manuscrite, signature numérique (PAdES, horodatage RFC 3161) et vérification des signatures ; protection par mot de passe AES-256 et permissions ; impression. |
| **Convertir et optimiser** | OCR (français et anglais, intégré, sur plusieurs cœurs), réduction de la taille, PDF vers Word (avec les tableaux), images ou texte, images et documents Office vers PDF, archivage PDF/A, comparaison de deux PDF. |
| **Traduire** | Traduire tout un document avec DeepL en gardant sa mise en page : chaque paragraphe et chaque cellule de tableau est réécrit à sa place. 28 langues. |
| **Assistant IA** | Résumer un document et lui poser des questions, avec un petit modèle qui fonctionne **hors ligne** sur votre ordinateur (téléchargement facultatif de 2,5 Go). |

Chaque fonction et ses limites connues sont décrites dans [docs/FEATURES.md](docs/FEATURES.md).

## Captures d'écran

<table>
  <tr>
    <td><img src="docs/images/dark.png" alt="Thème sombre"></td>
    <td><img src="docs/images/home.png" alt="Accueil avec les fichiers récents"></td>
  </tr>
  <tr>
    <td align="center">Thème sombre</td>
    <td align="center">Accueil et catalogue complet des outils</td>
  </tr>
</table>

<p align="center"><img src="docs/images/translate.png" width="480" alt="Fenêtre de traduction"><br>Traduire un document avec DeepL</p>

## Télécharger

1. Téléchargez **`PDFManager-rs-x.y.z-setup.exe`** depuis la
   [dernière version](https://github.com/alexandreromanini/PDFManager-rs/releases/latest).
2. Lancez-le. Windows peut afficher *« Windows a protégé votre ordinateur »*, car l'installeur
   n'est pas signé : cliquez sur **Informations complémentaires**, puis **Exécuter quand même**.
3. PDFManager vérifie lui-même les nouvelles versions et se met à jour en un clic.

Configuration : Windows 10 ou 11, 64 bits. Il s'installe sous le nom *PDFManager (Rust)* et
peut cohabiter avec l'édition Python. Facultatif :
[Ghostscript](https://ghostscript.com/releases/gsdnld.html) pour le PDF/A,
[LibreOffice](https://fr.libreoffice.org/download/telecharger-libreoffice/) pour convertir les
documents Office.

### Deux éditions

PDFManager existe en deux éditions, avec les mêmes fonctions et la même apparence :

| | PDFManager-rs (ce dépôt) | [PDFManager](https://github.com/alexandreromanini/PDFManager) |
|---|---|---|
| Écrit en | Rust, Slint, MuPDF | Python 3.12, PySide6 (Qt), PyMuPDF |
| Installeur | ~16 Mo | ~100 Mo |
| Idéal pour | L'usage quotidien : démarrage plus rapide, plus léger | Modifier le code en Python |

La comparaison détaillée est dans [docs/COMPARAISON.md](docs/COMPARAISON.md).

## Vie privée

PDFManager fonctionne hors ligne. Il ne se connecte que pour :

- vérifier les mises à jour sur GitHub ;
- traduire, quand vous utilisez **Traduire le document** (le texte est envoyé à DeepL) ;
- télécharger le modèle d'IA, une seule fois, si vous choisissez d'activer l'assistant.

## Compiler depuis les sources

```bash
cargo run --release -- document.pdf      # lancer
cargo test --release                     # tests unitaires
cargo build --release --features capture && python tests/parcours.py   # scénarios de bout en bout
```

L'installeur Windows est construit par GitHub Actions à chaque version. Voir
[docs/DEVELOPMENT.md](docs/DEVELOPMENT.md) pour l'installation et l'organisation du projet.

## Contribuer

Signalements de bugs, idées et pull requests sont les bienvenus, en français ou en anglais.
Lisez d'abord [CONTRIBUTING.md](CONTRIBUTING.md). Pour signaler une faille de sécurité, voir
[SECURITY.md](SECURITY.md).

## Licence

PDFManager est un logiciel libre, publié sous la [licence GNU AGPL v3.0 ou ultérieure](LICENSE).

Construit avec [MuPDF](https://mupdf.com) (par le crate [mupdf](https://crates.io/crates/mupdf)),
[Slint](https://slint.dev), [Tesseract](https://github.com/tesseract-ocr/tesseract),
[llama.cpp](https://github.com/ggml-org/llama.cpp) et les icônes [Lucide](https://lucide.dev).
