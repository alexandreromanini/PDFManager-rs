# Changelog

Format : [Keep a Changelog](https://keepachangelog.com/fr/1.1.0/), versions [SemVer](https://semver.org/lang/fr/).
Le détail de chaque fonction est dans [docs/FEATURES.md](docs/FEATURES.md).

## [Non publié]

### Modifié
- Fenêtre « À propos » : licence libre et adresse du projet (au lieu de « usage personnel »).
- Présentation du projet : README en anglais et en français, captures d'écran, guide de
  contribution, politique de sécurité, modèles de signalement.

## [0.9.0] - 2026-10-25

### Ajouté
- **Traduire le document** (catalogue *Convertir et optimiser*) : copie traduite par DeepL,
  même mise en page ; chaque paragraphe ou cellule est réécrit à sa place, même police
  (installée) ou la plus proche, même couleur. 28 langues ; clé DeepL mémorisée au choix ;
  Échap arrête.

## [0.8.1] - 2026-10-20

### Corrigé
- Les boutons des fenêtres internes (OCR, protection, couleur, signature…) lancent leur action
  une fois la fenêtre fermée : une boîte système ouverte pendant la fermeture pouvait planter
  l'application sous Windows.
- Un plantage est écrit dans le journal, avec sa trace.

## [0.8.0] - 2026-10-20

Parité avec la version Python ([docs/COMPARAISON.md](docs/COMPARAISON.md)).

### Ajouté
- Outils Lien et Modifier les images ; outil Main (H / V), présentation (F5), rotation de la
  vue au clavier.
- « Autre couleur… », couleur et épaisseur mémorisées par outil, couleur du filigrane, marge
  de l'en-tête ; scission par signets ; insertion de PDF ou d'images par glisser-déposer.
- Nom demandé à la création d'un champ, Tab d'un champ à l'autre ; trois options de
  nettoyage de plus ; options de la signature manuscrite ; recherche d'outil.

### Modifié
- Export Word avec tableaux ; réduction réelle des photos ; OCR en parallèle ; PDF/A conforme ;
  un signet par fichier fusionné.

### Corrigé
- Glisser à la souris (tracer, sélectionner, déplacer) interrompu au premier mouvement ;
  raccourcis clavier inactifs avant un premier clic.
- Quitter ou fermer un onglet modifié restait bloqué sous Windows.
- « toutes » dans un champ Pages faisait planter l'OCR ; les erreurs de pages s'affichent
  dans la fenêtre.

## [0.7.1] - 2026-10-20

### Corrigé
- 23 corrections issues de l'audit complet ([docs/AUDIT.md](docs/AUDIT.md)) ; interface
  utilisable au clavier ; catalogue d'outils compact dans les fenêtres étroites.

## [0.7.0] - 2026-10-19

### Ajouté
- Signatures manuscrites et numériques (PAdES, horodatage), vérification des signatures,
  comparaison de deux PDF.

## [0.6.0] - 2026-10-19

### Ajouté
- OCR (Tesseract intégré, français et anglais), assistant IA hors ligne, réduction de la taille,
  conversions (Word, images, texte, Office), PDF/A.

## [0.5.0] - 2026-10-19

### Ajouté
- Modifier le texte existant, caviardage définitif, nettoyage du document.

## [0.4.0] - 2026-10-19

### Ajouté
- Formulaires, protection par mot de passe, propriétés, impression, mises à jour intégrées.

## [0.3.0] - 2026-10-19

### Ajouté
- Commentaires, ajout de texte et d'images, en-têtes, pieds de page, numérotation, filigrane.

## [0.2.0] - 2026-10-19

### Ajouté
- Enregistrer, annuler / rétablir, organiser les pages ; publication des installeurs sur la
  page Releases.

## [0.1.0] - 2026-10-19

### Ajouté
- Lecteur complet : onglets, vignettes, signets, recherche, zoom, modes d'affichage.
