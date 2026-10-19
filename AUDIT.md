# Audit de PDFManager-rs (0.7.0, corrections publiées en 0.7.1)

Audit complet du 2026-10-19/20 : compréhension, tests fonctionnels de bout en bout, revue de
code, critique produit et design, corrections. Chaque correction a été re-testée (scénario
automatisé rejoué, voir « Comment l'audit a testé ») avant d'être marquée « Corrigé ».

## Synthèse

**28 constats** : 0 🔴 critique, 6 🟠 majeurs, 11 🟡 mineurs, 11 🔵 améliorations.
**23 corrigés**, 5 ne seront pas corrigés (raison donnée), aucun en attente.

| Catégorie | Constats | Corrigés |
|---|---|---|
| Bug | 11 | 10 |
| Performance | 2 | 1 |
| UX | 5 | 4 |
| Design | 1 | 1 |
| Accessibilité | 2 | 2 |
| Sécurité | 1 | 0 |
| Code | 4 | 4 |
| Doublon | 1 | 0 |
| Inutile | 1 | 1 (le constat : rien d'inutile à retirer) |

### Les 5 problèmes les plus importants (tous corrigés)

1. **B4 - « Enregistrer sous » pouvait écraser un fichier ouvert dans un autre onglet** : deux
   onglets sur un même fichier, l'enregistrement de l'un défaisant silencieusement l'autre.
2. **B1 - L'OCR s'appliquait au mauvais document** si l'on changeait d'onglet pendant la
   reconnaissance : erreur « invalid page number », texte reconnu perdu.
3. **P1 - L'historique d'annulation pouvait saturer la mémoire** : 30 copies complètes du
   document, soit 4,3 Go pour 30 modifications d'un PDF de 101 Mo (1,16 Go après correction).
4. **B2 - « Enregistrer » sans modification réécrivait le fichier** (après une confirmation
   inutile).
5. **U1 - En fenêtre étroite, Enregistrer et le menu disparaissaient** (à 1024 px, Enregistrer
   était coupé ; à 720 px, le menu « ⋯ », le zoom et Enregistrer sortaient de la fenêtre).

### Décisions prises (2026-10-20)

Toutes les recommandations ont été retenues :

| ID | Décision | Réalisé |
|---|---|---|
| A2 | Toute l'interface atteignable au clavier, anneau de focus visible seulement au clavier. | Corrigé (4fafa94) |
| U3 | Rotation de la vue retirée de la barre, gardée dans le menu. | Corrigé (47e63fa) |
| U4 | Catalogue d'outils replié en colonne d'icônes sous 1000 px. | Corrigé (c4acee8) |
| S1 | Signer les binaires seulement en cas de diffusion au-delà de l'usage personnel. | Rien à faire aujourd'hui |

### Recommandations stratégiques

1. **Lancer les parcours de bout en bout dans la CI** (`tests/parcours.py`, ajouté par cet audit,
   129 scénarios) : sept des dix bugs corrigés venaient d'enchaînements (changer d'onglet
   pendant une tâche, ouvrir deux fois, enregistrer sans modification) que les tests unitaires
   ne voyaient pas.
2. **Historique d'annulation par différences** plutôt que par copies complètes : la mémoire
   est maintenant bornée, mais chaque modification d'un gros fichier coûte encore ~0,9 s
   (le document entier est réécrit en mémoire à chaque étape).
3. **Accessibilité** : clavier fait (A2) ; reste à valider avec un vrai lecteur d'écran
   (Narrateur), ce que le mode capture ne peut pas faire.
4. **Signer les binaires** (décision S1) avant toute diffusion large.
5. **Petites fenêtres** : faites (U1, U4) ; les panneaux d'outils ouverts gardent 272 px, à
   compacter à leur tour si l'usage en demi-écran se confirme.

## Phase 0 - cartographie

Application de bureau Windows (Rust, Slint, MuPDF), ~12 000 lignes. Un thread moteur possède
les documents, l'interface lui envoie des tâches ; annuler/rétablir par instantanés.

### Écrans

- **Accueil** : zone « Ouvrir un PDF » (glisser-déposer ou « Choisir des fichiers »), six
  fichiers récents (les absents sont masqués), catalogue d'outils à gauche.
- **Document** : onglets en haut, barre du document (annuler, rétablir, page, zoom,
  ajustements, rotation de la vue, mode Continu / Page unique / Double page, Enregistrer),
  vue des pages, panneau latéral à droite.

### Barre d'application

Accueil, onglets (fermer), « + » (ouvrir), recherche (précédent, suivant), thème clair/sombre,
menu « ⋯ » : Ouvrir, Enregistrer, Enregistrer sous, Enregistrer une copie, Imprimer,
Propriétés, Annuler, Rétablir, Fermer l'onglet, Accueil et fichiers récents, Panneau latéral,
Plein écran, Thème (système, clair, sombre), Rechercher des mises à jour, À propos, Quitter.

### Panneau d'outils (catalogue puis panneaux)

| Panneau | Fonctions |
|---|---|
| Catalogue | Organiser les pages, Fusionner, Scinder, Ajouter du texte et des images, Commenter, Formulaires, OCR, Réduire la taille, Convertir, Comparer, Archiver (PDF/A), Signer, Protéger, Retirer la protection, Assistant IA |
| Organiser | Pivoter à gauche/droite, Supprimer, Extraire, Insérer une page vierge / un PDF / des images, Recadrer, Annuler le recadrage, Fusionner, Scinder, En-tête et pied de page, Filigrane ; glisser les vignettes pour déplacer |
| Commenter | Sélection, Surligner, Souligner, Barrer, Note, Texte libre, Rectangle, Ellipse, Ligne, Flèche, Dessin, Tampon ; couleur, épaisseur, auteur, liste des commentaires |
| Modifier | Modifier le texte (sur place), Ajouter du texte, Image ; taille et couleur ; En-tête, Filigrane ; Caviarder une zone, Caviarder un texte partout, Appliquer les caviardages, Nettoyer le document |
| Formulaires | Remplir, Champ texte, Case à cocher, Liste déroulante, Champ de signature |
| Convertir | Exporter en Word, en images, en texte ; Créer un PDF depuis des images, depuis des documents Office (LibreOffice) |
| Signer | Signature manuscrite (créer : tracée, tapée, image ; placer, supprimer), signature numérique (certificat .pfx/.p12, motif, lieu, image, horodatage), Vérifier les signatures |

### Panneau latéral (rail)

Pages (vignettes), Signets, Commentaires, Assistant (télécharger le modèle, résumer, questions,
arrêter), Différences (comparaison), Signatures (vérification, faire confiance au certificat,
voir la version signée).

### Fenêtres

Mot de passe, Fusionner (ajouter, retirer, monter, descendre), Scinder (plages ou toutes les N
pages), En-tête et pied de page, Filigrane, Protéger, Retirer la protection, Propriétés,
Nettoyer, OCR (langue, pages, pages avec texte), Réduire la taille, Exporter en images,
Créer une signature, Signature numérique, Note / Texte libre / Liste (saisie), choix de liste.

### Raccourcis

Ctrl+O, Ctrl+W, Ctrl+F, Ctrl+C, Ctrl+P, Ctrl+D, Ctrl+Z, Ctrl+Y, Ctrl+S, Ctrl+Maj+S, Ctrl+±,
Ctrl+0, Ctrl+2, Ctrl+Tab, Suppr, Échap, F3 / Maj+F3, F4, F11, Page préc./suiv., Début, Fin.

### Données et persistance

Réglages JSON (`PDFManager-rs/réglages.json` : récents, thème, panneau latéral, auteur, chemin
du certificat, serveur d'horodatage, certificats de confiance), signatures manuscrites (PNG),
modèle d'IA (téléchargé, empreinte vérifiée). Les PDF sont écrits de façon atomique (fichier
temporaire puis remplacement), en ajout incrémental pour un document signé.

## Comment l'audit a testé

- **Mode capture piloté** : l'appli tourne hors écran ; les fenêtres système passent par un
  module unique (`src/dialogs.rs`) qui, dans ce mode, lit les fichiers choisis et les réponses
  dans des variables d'environnement et écrit chaque message (erreur, question, note) sur la
  sortie d'erreur. Une commande `etat` décrit l'appli (onglets, pages, modifié, historique,
  commentaires, champs, recherche…).
- **`tests/parcours.py`** : 129 scénarios en 19 groupes (ouverture, navigation, recherche,
  organiser, enregistrer, commenter, modifier, formulaires, protection, convertir, OCR, PDF/A,
  signer, comparer, assistant, cas limites, régressions de l'audit, bords de page, clavier et souris). Chaque
  fichier produit est relu avec PyMuPDF (pages, annotations, champs, texte, métadonnées,
  chiffrement, signature).
- **Mesures** : un PDF de 101 Mo / 300 pages (ouverture 4 s, recherche, 10 et 30 rotations ;
  temps et mémoire maximale).
- **Captures d'écran** à 1280, 1024, 800 et 720 px, thèmes clair et sombre.
- Tests unitaires : 47 au départ, 49 avec les 2 ajoutés (historique borné, cadre dans la page), tous verts ; 3 ignorés par défaut (réseau ou modèle d'IA requis).

Résultat avant corrections : 13 échecs révélant 11 bugs (les autres échecs étaient des
erreurs du script, corrigées). Après corrections : voir le dernier passage ci-dessous.

## Tableau des problèmes

Sévérité : 🔴 Critique · 🟠 Majeur · 🟡 Mineur · 🔵 Amélioration.

| ID | Sévérité | Catégorie | Où (écran/fichier:ligne) | Description | Étapes pour reproduire | Correction proposée | Statut |
|---|---|---|---|---|---|---|---|
| B4 | 🟠 | Bug | Enregistrer sous - `src/edit_ui.rs` `save_as` | Enregistrer sous vers le fichier d'un autre onglet l'écrase ; deux onglets pointent ensuite le même fichier, l'un pouvant défaire l'autre. | Ouvrir a.pdf et b.pdf ; dans a, modifier puis Enregistrer sous b.pdf. | Refuser avec « b.pdf est ouvert dans un autre onglet : fermez-le d'abord ». | Corrigé (9f15a99, scénario A1) |
| B1 | 🟠 | Bug | OCR - `src/convert_ui.rs` `ocr_run` | La fin de l'OCR (asynchrone) modifie l'onglet courant : texte perdu, erreur « invalid page number ». | Lancer l'OCR d'un scan de 12 pages, passer à un autre onglet. | Viser le document d'origine (`edit_doc(id)`). | Corrigé (0924029, A2) |
| B9 | 🟡 | Bug | Appliquer les caviardages - `src/forms_ui.rs` `redact_apply` | Même défaut que B1 après la confirmation. | Rare (changement d'onglet pendant la question). | Idem B1. | Corrigé (0924029) |
| B10 | 🟡 | Bug | OCR - `src/convert_ui.rs` | Fermer le document pendant l'OCR pouvait afficher « Reconnaissance impossible : document fermé ». | Lancer l'OCR, fermer l'onglet. | S'arrêter sans message. | Corrigé (0924029, cas limites) |
| P1 | 🟠 | Performance | Annuler - `src/pdf.rs` `push_undo` | 30 copies complètes en mémoire : 4,3 Go pour 30 rotations d'un PDF de 101 Mo. | Ouvrir un gros PDF, faire 30 modifications. | Plafond de 600 Mo en plus des 30 étapes (la dernière toujours gardée). Mesuré : 1,16 Go stable. | Corrigé (d64b35d, test `historique_borne`) |
| B2 | 🟠 | Bug | Enregistrer - `src/edit_ui.rs` `save` | Document non modifié : confirmation puis réécriture du fichier. | Ouvrir, Ctrl+S. | Ne rien écrire, note « Aucune modification à enregistrer ». | Corrigé (ecb019a, A3) |
| U1 | 🟠 | UX | Barre d'application et du document - `ui/app.slint` | Fenêtre étroite : Enregistrer coupé (1024 px), menu « ⋯ », zoom et Enregistrer hors fenêtre (720 px). | Réduire la fenêtre. | Recherche qui rétrécit ; commandes secondaires masquées par paliers ; Enregistrer en icône seule. | Corrigé (a6f2723, captures 720/1024) |
| A2 | 🟠 | Accessibilité | Tous les boutons - `ui/app.slint` (IconButton, ToolRow, Tile…) | Les boutons ne prennent pas le focus : impossible d'atteindre les outils au clavier. | Appuyer sur Tab. | Composant `FocusRing` : Tab, anneau visible, Entrée/Espace ; un clic ne prend pas le focus. | Corrigé (4fafa94, scénarios A7 à A9) |
| B3 | 🟡 | Bug | Ouverture - `src/app.rs` `on_opened` | Un fichier ouvert deux fois coup sur coup (double-clic, glisser) donne deux onglets. | Ouvrir le même fichier deux fois rapidement. | Garde à l'arrivée du document chargé. | Corrigé (898e0b3, A4) |
| B5 | 🟡 | Bug | Texte modifié/ajouté - `src/fonts.rs:89` | Les espaces du texte écrit sont extraites en espaces insécables : rechercher ou copier « Titre corrigé » échoue. | Modifier un texte, le rechercher. | Ne plus ajouter l'insécable d'office à la police réduite. | Corrigé (a2c58f7, groupe modifier) |
| B6 | 🟡 | Bug | Commenter, Modifier, Formulaires, Signer - `src/annot_ui.rs` `tool_release` | Objet posé d'un clic près d'un bord : il dépasse de la page (texte libre presque entièrement coupé). | Outil Tampon, cliquer au coin bas-droit. | Décaler le cadre par défaut dans la page (`Rect::inside`). | Corrigé (2227048, A6, test `cadre_ramene_dans_la_page`) |
| B7 | 🟡 | Bug | Recherche - `src/app.rs` `on_hits` | Changer d'onglet pendant une recherche faisait défiler le mauvais onglet. | Rechercher, changer d'onglet aussitôt. | N'amener le résultat que sur son onglet. | Corrigé (1a11b74, A5) |
| U2 | 🟡 | UX | Plein écran - `src/annot_ui.rs` `escape` | Échap ne quitte pas le plein écran (seul F11). | F11 puis Échap. | Échap, en dernier recours, quitte le plein écran. | Corrigé (de5b6b5) |
| D1 | 🟡 | Design | Vignettes - `ui/app.slint` | Contour recouvert par l'image : pages blanches sans bord visible sur fond clair. | Ouvrir un document à pages blanches. | Image en retrait d'1 px. | Corrigé (9865bbb, capture) |
| A1 | 🟡 | Accessibilité | Composants partagés - `ui/app.slint` | État (activé, coché) et action par défaut non exposés aux lecteurs d'écran ; pastilles de couleur muettes. | Lecteur d'écran (Narrateur). | `accessible-enabled/checked/action-default`, noms des couleurs. | Corrigé (0986e42) |
| U3 | 🟡 | UX | Barre du document et Organiser | Même icône pour la rotation de la vue et celle des pages (qui modifie le fichier). | Comparer les deux. | Rotation de la vue déplacée dans le menu (menu élargi, défilant si la fenêtre est basse). | Corrigé (47e63fa, capture) |
| U4 | 🟡 | UX | Mise en page - `ui/app.slint` | Sous ~1000 px, le document n'a plus que ~200 px de large. | Fenêtre à 720 px. | Catalogue replié en colonne d'icônes (76 px) : document à 58 % au lieu de 18 %. | Corrigé (c4acee8, captures, A10) |
| B8 | 🔵 | Bug | Imprimer - `src/forms_ui.rs` `print` | Sans document, message « disponible que sous Windows » (Linux seulement). | Linux, Ctrl+P à l'accueil. | Vérifier d'abord le document. | Corrigé (588ce42) |
| P2 | 🔵 | Performance | Toute modification - `src/pdf.rs` `edit` | Chaque modification réécrit le document entier en mémoire : ~0,9 s par étape sur 101 Mo. | 10 rotations sur gros.pdf. | Historique par différences (recommandation 2). | Ne sera pas corrigé ici : refonte de l'historique, gain nul sur les fichiers courants |
| U5 | 🔵 | UX | Comparaison - `src/sign_ui.rs` | Une modification efface les surlignages de la comparaison. | Comparer, puis modifier un document. | - | Ne sera pas corrigé : après modification les positions seraient fausses ; cliquer une différence les rétablit |
| S1 | 🔵 | Sécurité | Mises à jour, installeur | Binaires non signés ; intégrité vérifiée (taille, SHA-256) mais l'empreinte vient de la même source que le fichier. | - | Signer en cas de diffusion large. | Ne sera pas corrigé pour l'instant : usage personnel (décision du 2026-10-20) |
| L1 | 🔵 | Bug | Signature puis commentaire - MuPDF | pyHanko classe en « autre modification » un commentaire ajouté après signature (MuPDF réécrit l'objet page). | Signer, commenter, vérifier avec pyHanko. | Écrire l'annotation sans toucher l'objet page : hors de portée de MuPDF. | Ne sera pas corrigé : limite de MuPDF, documentée ; la signature reste valide dans l'appli et Acrobat |
| C1 | 🔵 | Code | 22 appels à rfd dans 6 fichiers | Fenêtres système dupliquées, impossibles à piloter dans les tests. | - | Module `src/dialogs.rs`. | Corrigé (b47ff14) |
| C2 | 🔵 | Code | clippy | 15 avertissements. | `cargo clippy` | 7 corrigés automatiquement ; 8 de style assumés (nombre d'arguments, nommage). | Corrigé (e9c32d4) |
| C3 | 🔵 | Code | `tests/` | Aucun test de bout en bout des parcours. | - | `tests/parcours.py` + mode capture piloté. | Corrigé |
| C4 | 🔵 | Code | `src/edit_ui.rs`, `src/convert_ui.rs` | Deux fonctions d'ajout d'extension (`with_pdf_extension`, `save_as`). | - | Une seule (`dialogs::save_file`). | Corrigé (b47ff14) |
| X1 | 🔵 | Doublon | Thème : bouton de la barre + 3 entrées du menu | Deux accès au thème. | - | - | Ne sera pas corrigé : le bouton bascule vite clair/sombre, le menu seul offre « système » |
| X2 | 🔵 | Inutile | Ensemble des fonctions | Recherche de fonctions inutiles ou redondantes : « Fusionner » présent au catalogue et dans Organiser est voulu (le catalogue est un sommaire). | - | Rien à retirer. | Corrigé (constat sans action) |

## Revue de code : points vérifiés sans problème

- Liens d'un PDF : seuls http, https et mailto s'ouvrent, après confirmation (`file:`, protocoles
  d'applications refusés).
- Téléchargements (mise à jour, modèle d'IA) : HTTPS, taille et SHA-256 vérifiés, fichier
  `.part` renommé seulement s'il est intact.
- Fichiers temporaires (OCR, ajout incrémental) supprimés après usage ; écriture atomique.
- Mot de passe du certificat jamais enregistré ; seul le chemin l'est.
- Cas limites passés : fichiers corrompu, vide, absent, `.txt` déposé, noms avec emoji / 180
  caractères / points multiples, mots de passe faux (3 essais), zoom 0 / 100 000 / texte, page
  0 / 99, 40 actions rapides, actions sans document, plages invalides, double Enregistrer.

## Dernier passage des parcours

Sur le code final (après le dernier commit de correction) : **129/129 scénarios OK** dans les
19 groupes (après les décisions A2, U3, U4), **49 tests unitaires OK** (3 ignorés : réseau ou modèle d'IA) (`cargo test --release`). Mesures sur le PDF de 101 Mo :
ouverture ~2 s hors délais du script, mémoire maximale après 30 modifications 1,16 Go (contre
4,3 Go avant P1).
