# Comparaison avec la version Python (PDFManager 0.10.3)

Chaque fonction de la version Python a été cherchée dans la version Rust, puis, quand les deux
l'avaient, comparée sur les mêmes documents (résultats relus avec PyMuPDF, python-docx et le
validateur PDF/A de pikepdf). Les écarts ont été corrigés ; ceux qui restent sont listés en fin
de document avec leur raison. Date : 2026-10-20.

## Résumé

- **Fonctions absentes en Rust, ajoutées** : 20 (outil Lien, Modifier les images, outil Main,
  présentation, recherche d'outil, autre couleur, raccourcis dès l'ouverture…).
- **Fonctions présentes mais moins bonnes, corrigées** : 10, dont 5 mesurées avant / après
  (export Word, réduction de taille, OCR, PDF/A, fusion) ; les autres : scission par signets,
  marge de l'en-tête, couleur du filigrane, ouverture des fichiers produits, nom de l'étape à
  annuler.
- **Défaut grave trouvé en route** : en Rust, tout glisser à la souris (tracer une forme,
  sélectionner du texte, déplacer un commentaire) s'interrompait au premier mouvement. Corrigé.
- **Différences restantes** : 4, expliquées plus bas.

## Qualité mesurée (mêmes documents)

| Fonction | Python | Rust avant | Rust après |
|---|---|---|---|
| Export Word, facture à tableau bordé | tableau 4 × 4, ordre juste | tableau perdu, cellules en fin de document | tableau 4 × 4, ordre juste, saut de page |
| Export Word, tableau sans bordures | tableau 4 × 3 | perdu | tableau 4 × 3 |
| Export Word, texte en deux colonnes | - | lignes des colonnes entremêlées | une colonne après l'autre |
| Export Word, 300 pages / 101 Mo | 33 s, fichier de 214 Mo | - | moins de 30 s, fichier de 65 Mo |
| Réduire la taille, photo de page (553 Ko), équilibré | 343 Ko, écart de rendu 0,10 | 553 Ko (aucune réduction) | 433 Ko, écart 0,10 |
| Réduire la taille, même photo, maximal | 139 Ko, écart 3,5 | 553 Ko | 121 Ko, écart 3,5 |
| Réduire la taille, 101 Mo de photos, maximal | 59 Mo, écart 0,8 | 101 Mo | 68 Mo, écart 0,8 |
| OCR, 8 pages scannées | 2,0 s | 5,7 s | 2,2 s, même texte reconnu |
| PDF/A (validateur pikepdf) | 0 violation | 2 violations (pas d'OutputIntent) | 0 violation |
| Fusion | un signet par fichier | aucun signet | un signet par fichier |

« Écart de rendu » : différence moyenne des pixels entre l'original et la copie réduite
(0 = identique).

## Fonctions, une par une

| Fonction (Python) | Rust avant | Rust maintenant |
|---|---|---|
| Ouvrir, onglets, accueil, récents | ✅ | ✅ ; récents : « Effacer la liste » ajouté |
| Fichier endommagé signalé « réparé » | ❌ silencieux | ✅ note à l'ouverture |
| Vue : modes, zoom, ajuster, rotation de la vue | ✅ | ✅ ; rotation aussi au clavier (Ctrl+Maj+- / +) |
| Outil Main (H), Sélection (V) | ❌ | ✅ (Commenter, touches H et V) |
| Présentation (F5) | ❌ | ✅ (plein écran, flèches, clic, Échap) |
| Plein écran (F11), panneau latéral (F4) | ✅ | ✅ |
| Raccourcis dès l'ouverture | ❌ seulement après un clic sur une page | ✅ |
| Recherche dans le document | ✅ | ✅ |
| Panneau d'outils : recherche d'outil | ❌ | ✅ (sans accents ni majuscules) |
| Accueil : « Outils populaires » | ❌ | ➖ remplacé par le catalogue complet, visible dès l'accueil |
| Commenter : 8 types, tampons, déplacer, modifier, supprimer | ✅ | ✅ ; glisser à la souris réparé |
| Couleur et épaisseur mémorisées par outil | ❌ | ✅ |
| « Autre couleur… » | ❌ 6 couleurs fixes | ✅ code #RRGGBB |
| Liste des commentaires : aller, modifier, supprimer, couleur | ✅ (couleur : sélection puis pastille) | ✅ |
| Organiser : pivoter, supprimer, extraire, insérer, déplacer, recadrer | ✅ | ✅ |
| Déposer des PDF / images pour les insérer | ❌ | ✅ (avec « Organiser les pages » ouvert) |
| Fusionner (un signet par fichier) | ⚠️ sans signets | ✅ |
| Scinder : plages, toutes les N pages, par signets | ⚠️ sans signets | ✅ |
| En-tête / pied / numérotation : marge | ⚠️ marge fixe | ✅ réglable (mm) |
| Filigrane : couleur | ⚠️ gris fixe | ✅ 5 couleurs |
| Modifier le texte, Ajouter du texte | ✅ | ✅ ; Ctrl+Entrée valide |
| Texte ajouté : poignée pour élargir | ❌ | ✅ |
| Modifier les images (déplacer, redimensionner, supprimer) | ❌ | ✅ |
| Lien (adresse, e-mail, page ; supprimer d'un clic) | ❌ | ✅ |
| Caviarder (zones, texte partout, appliquer) | ✅ | ✅ |
| Nettoyer : métadonnées, JavaScript, pièces jointes, miniatures | ✅ | ✅ |
| Nettoyer : texte invisible, liens, valeurs des champs | ❌ | ✅ |
| OCR : langue, pages, pages avec texte | ✅ | ✅ |
| OCR : résolution d'analyse, traitement parallèle | ❌ | ✅ |
| Traduire le document (DeepL, même mise en page), ajouté aux deux versions | ❌ | ✅ 0.9.0 |
| Réduire la taille : 3 niveaux, gain affiché, rien si aucun gain | ⚠️ photos jamais réduites | ✅ |
| Réduire : « Optimiser pour le web (linéariser) » | ❌ | ❌ voir plus bas |
| Protéger, retirer la protection, propriétés | ✅ | ✅ |
| Comparer deux PDF | ✅ | ✅ |
| PDF/A | ⚠️ non conforme | ✅ conforme (Ghostscript + OutputIntent) |
| Exporter en Word | ⚠️ tableaux perdus | ✅ |
| Exporter en images, en texte ; créer depuis images / Office | ✅ | ✅ |
| Fichier produit : « L'ouvrir ? » dans l'application | ⚠️ ouvert dans un autre lecteur | ✅ en onglet |
| Formulaires : remplir, créer 4 types, supprimer | ✅ | ✅ |
| Formulaires : nom demandé à la création | ❌ nom automatique | ✅ |
| Formulaires : Tab / Maj+Tab entre champs texte | ❌ | ✅ |
| Signature manuscrite : dessinée, tapée, image, mémorisée | ✅ | ✅ |
| … « Rendre le fond clair transparent », « Mémoriser » | ❌ toujours | ✅ au choix |
| Signature numérique, horodatage, vérification, confiance, version signée | ✅ | ✅ |
| Assistant IA hors ligne | ✅ | ✅ |
| Imprimer | ✅ | ✅ |
| Annuler / Rétablir, nom de l'étape dans le menu | ⚠️ sans nom | ✅ « Annuler « Pivoter des pages » » |
| Aide : outils externes détectés, dossier des journaux | ❌ | ✅ |
| Mises à jour depuis l'application | ✅ | ✅ |

✅ présent et équivalent · ⚠️ présent mais moins bon · ❌ absent · ➖ remplacé autrement

## Différences restantes

1. **Polices réduites aux caractères utilisés** (« Réduire la taille ») : la version Python
   appelle `subset_fonts()` de PyMuPDF. En Rust, la fonction de MuPDF existe mais lève des
   exceptions C qu'on ne peut rattraper qu'en ajoutant du code C à la bibliothèque MuPDF
   elle-même. Effet : seuls les PDF à polices complètes (rares : Word, LibreOffice et les
   navigateurs réduisent déjà leurs polices) restent plus gros qu'avec Python.
2. **Linéarisation** (« Optimiser pour l'affichage web ») : MuPDF 1.27 ne la fait plus
   (« Linearisation is no longer supported ») ; Python passait par qpdf. Intérêt aujourd'hui
   faible (affichage progressif d'un PDF dans un navigateur).
3. **Outils populaires** à l'accueil : non repris, le catalogue complet y est déjà visible.
4. **Commentaires en fil** (réponses, statut) : absents des deux versions.

## Comment c'est vérifié

`tests/parcours.py` rejoue maintenant aussi ces fonctions (groupes `parite`, `images_liens`,
`souris`, `couleurs`, `champs`, `signature_visuelle`, `repare`, `depot`, `poignee`), dont
des gestes avec vraie souris et vrai clavier (`mouse-drag`, `key`) : ce sont eux qui ont
révélé le glisser interrompu, invisible quand les tests appelaient le contrôleur
directement. Tests unitaires ajoutés : export Word (tableau, ordre), réduction (photo
divisée par deux au moins), images et liens, nettoyage complet, recherche d'outil, fusion
avec signets, scission par signets.
