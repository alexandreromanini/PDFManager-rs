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
| 3 | Commentaires, ajouter du texte et des images, en-têtes, numérotation, filigrane | ✅ 0.3.0 |
| 4 | Formulaires, protection, impression, mises à jour | ✅ 0.4.0 |
| 5 | Modifier le texte existant, caviardage | ✅ 0.5.0 |
| 6 | OCR, assistant IA, conversions | ✅ 0.6.0 |
| 7 | Signatures, comparaison | ✅ 0.7.0 |
| Audit | Tests de bout en bout, 23 corrections (voir [AUDIT.md](AUDIT.md)) | ✅ 0.7.1 |
| Parité | Toutes les fonctions de la version Python (voir [COMPARAISON.md](COMPARAISON.md)) | ✅ 0.8.0 |

## Fonctions (phase 1)

- Accueil : ouvrir (dialogue ou glisser-déposer), fichiers récents.
- Onglets, une seule fenêtre : un PDF ouvert depuis l'Explorateur arrive en onglet.
- Vue fluide : rendu en arrière-plan (thread moteur), seules les pages visibles sont
  dessinées ; modes continu, page unique, double page ; zoom (Ctrl+molette, Ctrl+/-),
  ajuster à la largeur (Ctrl+2) ou à la page (Ctrl+0) ; pivoter l'affichage (menu).
- Rail de droite : vignettes (rendues à la demande), signets.
- Recherche (Ctrl+F, F3 / Maj+F3), surlignage, compteur « 3 / 12 ».
- Au clavier : Tab parcourt boutons et outils (anneau de focus), Entrée ou Espace active.
- Fenêtre étroite (sous 1000 px) : catalogue d'outils en colonne d'icônes, barre du document
  allégée, Enregistrer et le menu toujours visibles.
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

## Fonctions (phase 3)

- Panneau « Commenter » : surligner, souligner, barrer (sur le texte sélectionné), note,
  texte libre, rectangle, ellipse, ligne, flèche, dessin à main levée, tampon ; 6 couleurs,
  épaisseur des traits, nom de l'auteur.
- Avec l'outil Sélection : cliquer un commentaire le sélectionne, le glisser le déplace,
  double-clic pour modifier son texte, Suppr pour le supprimer.
- Liste des commentaires dans le rail de droite (page, auteur, date, texte) : clic pour y
  aller, double-clic pour modifier, corbeille pour supprimer.
- Panneau « Modifier » : ajouter du texte directement sur la page (taille, couleur ;
  modifiable ensuite), ajouter une image (cadre tracé à la souris).
- En-tête, pied de page et numérotation ({n}, {total}, {date}, {fichier}) et filigrane
  (taille, opacité, inclinaison), sur toutes les pages ou une plage.
- Tout est annulable (Ctrl+Z), et reste droit et bien placé sur les pages pivotées.

## Fonctions (phase 4)

- Formulaires : les champs sont surlignés en bleu ; cliquer un champ texte pour écrire
  dedans (Entrée valide), une case pour la cocher, une liste pour choisir. Panneau
  « Formulaires » pour créer des champs (texte, case à cocher, liste déroulante, signature)
  ou en supprimer.
- Protéger par mot de passe (AES-256) : mot de passe d'ouverture, mot de passe des
  permissions, choix des permissions (imprimer, copier, commenter, modifier). Retirer la
  protection (mot de passe des permissions demandé si besoin).
- Propriétés du document (Ctrl+D) : titre, auteur, sujet, mots-clés…
- Imprimer (Ctrl+P), avec la fenêtre d'impression de Windows : pages ajustées au papier et
  tournées si besoin ; refusé si l'auteur du document l'interdit.
- Mises à jour : vérification discrète au démarrage, ou menu « Rechercher des mises à
  jour… » ; l'installeur est téléchargé, vérifié (taille, empreinte SHA-256) puis installé
  sans assistant, et l'application se relance.
- `vendor/mupdf` : bibliothèque MuPDF (Rust) avec un correctif, voir `CORRECTIF.md`.

## Fonctions (phase 5)

- Modifier le texte d'origine : outil « Modifier le texte » (panneau Modifier), cliquer un
  paragraphe et le corriger sur la page. Seules les lignes changées sont réécrites, à la même
  place, même taille, même couleur, dans la police d'origine si elle contient les caractères,
  sinon la même police installée sur l'ordinateur (réduite aux caractères utilisés), sinon la
  plus proche (Arial, Times New Roman, Courier New, puis Helvetica / Times / Courier) ; le
  résultat est relu, et l'utilisateur est prévenu de tout remplacement ou débordement.
- Caviardage : tracer des zones, ou marquer toutes les occurrences d'un texte ; les marques
  restent réversibles (liste des commentaires, Suppr) jusqu'à « Appliquer les caviardages »,
  qui supprime vraiment texte, images, tracés, commentaires et champs couverts.
- Nettoyer le document : métadonnées, JavaScript et actions automatiques, pièces jointes,
  vignettes intégrées.

## Fonctions (phase 6)

- Reconnaître le texte (OCR) : Tesseract intégré à MuPDF, modèles français et anglais
  livrés (`resources/tessdata`, licence Apache 2.0) ; le texte reconnu est posé, invisible,
  sur les pages scannées (recherche, sélection, copie) ; Échap arrête.
- Réduire la taille (copie) : sans perte, ou photos JPEG ramenées à 150 / 96 dpi.
- Convertir : PDF vers Word (.docx, par MuPDF), images (PNG, JPEG), texte ; images et
  documents Office (LibreOffice, s'il est installé) vers PDF.
- Archiver en PDF/A-2b avec Ghostscript, s'il est installé.
- Assistant IA hors ligne (rail de droite) : résumé et questions sur le document, pages
  citées cliquables. Qwen3 4B Instruct par llama.cpp, sur le processeur ; le modèle (2,5 Go)
  est partagé avec la version Python, téléchargé par l'installeur (case à cocher) ou depuis
  l'application, taille et empreinte vérifiées.

## Fonctions (phase 7)

- Signature manuscrite : dessinée à la souris, tapée (police manuscrite) ou importée (scan,
  photo, fond rendu transparent), enregistrée pour être réutilisée, placée sur la page.
- Signature numérique PAdES (ETSI.CAdES.detached) avec un certificat .pfx / .p12 (RSA,
  ECDSA P-256 / P-384 ; chiffrements 3DES, RC2 et AES-256 des fichiers), apparence visible
  (texte, image de signature), horodatage RFC 3161. Ajoutée au fichier sans toucher aux
  signatures précédentes. Écrit avec les bibliothèques RustCrypto ; contre-vérifié avec
  pyHanko (signatures, horodatage).
- Vérification (rail de droite) : intégrité, signataire, chaîne jusqu'aux autorités du
  système ou aux certificats approuvés, horodatage, mises à jour ajoutées depuis ; « Faire
  confiance au certificat », « Voir la version signée ».
- Un document signé reste modifiable (commentaires, formulaires) : les modifications sont
  enregistrées en ajout, les signatures restent valides.
- Comparer deux PDF, mot à mot sur tout le document : différences listées dans le rail de
  droite et surlignées dans les deux documents.

## Fonctions ajoutées (parité avec la version Python)

- Modifier : **Modifier les images** (cliquer, glisser, poignée pour agrandir, Suppr) ;
  **Lien** (tracer la zone : adresse web, e-mail ou numéro de page ; un clic sur un lien
  existant propose de le supprimer) ; poignée pour élargir un texte ajouté.
- Affichage : outil **Main** (H ; V pour revenir à la sélection), **présentation** (F5),
  rotation de la vue au clavier (Ctrl+Maj+- / Ctrl+Maj++).
- Couleurs : « Autre couleur… » (code #RRGGBB), couleur et épaisseur mémorisées par outil,
  couleur du filigrane ; marge de l'en-tête et du pied de page.
- Pages : PDF et images déposés insérés quand « Organiser les pages » est ouvert ; fusion
  avec un signet par fichier ; scission selon les signets.
- Formulaires : nom demandé à la création ; Tab / Maj+Tab d'un champ texte à l'autre.
- Nettoyer : texte invisible (couche OCR), liens, valeurs des champs (au choix).
- OCR en parallèle et résolution d'analyse réglable ; export Word avec tableaux ; réduction
  des photos ; PDF/A conforme (OutputIntent sRGB).
- Signature manuscrite : fond transparent et mémorisation au choix.
- Recherche d'outil dans le catalogue ; menu : « Annuler « … » », outils externes détectés,
  dossier des journaux ; fichier endommagé signalé ; « Effacer la liste » des récents.

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
# Parcours de bout en bout (154 scénarios, l'appli pilotée hors écran ; pip install pymupdf) :
cargo build --release --features capture && python tests/parcours.py
```

Si la compilation de `mupdf-sys` échoue sur « 'stddef.h' file not found » (en-têtes de clang
absents) : `export BINDGEN_EXTRA_CLANG_ARGS=-I/usr/lib/gcc/x86_64-linux-gnu/13/include`, ou
installer `libclang-common-18-dev`.

## Licence

AGPL-3.0-or-later (MuPDF est sous AGPL). Interface réalisée avec Slint. Profil `resources/sRGB.icc`
(OutputIntent des PDF/A) : celui de pikepdf, produit par lcms, « no copyright, use freely ».
