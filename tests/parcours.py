"""Parcours de bout en bout de PDFManager-rs : l'appli tourne hors écran (mode capture),
pilotée commande par commande ; chaque fichier produit est relu avec PyMuPDF.

    cargo build --release --features capture
    python tests/parcours.py [groupe…]        (pip install pymupdf)

Fenêtres système : fichiers choisis dans PDFMANAGER_PICKS, réponses dans
PDFMANAGER_ANSWERS (voir src/dialogs.rs). Chaque scénario affiche OK ou ÉCHEC et le détail ;
code de sortie 1 en cas d'échec. Groupes « ocr », « pdfa » et « assistant » : Tesseract,
Ghostscript et le modèle d'IA doivent être installés."""
import os, sys, shutil, subprocess, tempfile, re, time
import pymupdf

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
EXE = REPO + "/target/release/pdfmanager"
FIX = REPO + "/tests/fixtures/Été déjà vu.pdf"
W = tempfile.mkdtemp(prefix="audit-")
results = []

def fresh(name="essai.pdf", src=FIX):
    p = os.path.join(W, name)
    shutil.copy(src, p)
    return p

def run(files, actions, picks=(), answers=(), timeout=180, shot=None):
    env = dict(os.environ, PDFMANAGER_PICKS="|".join(picks), PDFMANAGER_ANSWERS="|".join(answers),
               XDG_CONFIG_HOME=os.path.join(W, "config"), XDG_DATA_HOME=os.path.join(W, "data"))
    out = shot or os.path.join(W, "shot.ppm")
    t = time.time()
    try:
        p = subprocess.run([EXE, "--capture", out, "1280", "900", "light", *files, "--", *actions], env=env,
                           capture_output=True, text=True, timeout=timeout)
        log = p.stderr
        code = p.returncode
    except subprocess.TimeoutExpired as e:
        log = (e.stderr or b"").decode(errors="replace") if isinstance(e.stderr, bytes) else (e.stderr or "")
        code = "TIMEOUT"
    return log, code, time.time() - t

def check(name, ok, detail=""):
    results.append((name, ok, detail))
    print(("OK    " if ok else "ÉCHEC ") + name + (("  - " + detail) if detail and not ok else ""), flush=True)

def lines(log, kind):
    return [l.split(" : ", 1)[1] if " : " in l else l for l in log.splitlines() if l.startswith(kind)]

def etat(log):
    return [l[7:] for l in log.splitlines() if l.startswith("ETAT : ")]

def pages(path):
    try:
        return pymupdf.open(path).page_count
    except Exception as e:
        return f"illisible ({e})"

# ----- 1. ouverture, navigation, recherche -------------------------------------------------
def g_ouverture():
    f = fresh()
    log, code, dt = run([f], ["etat"])
    e = etat(log)
    check("ouvrir un PDF (3 pages, page 1)", any("pages=3" in x and "page=1" in x for x in e), log[-500:])
    check("démarrage sans erreur", not lines(log, "ERREUR"), str(lines(log, "ERREUR")))
    bad = os.path.join(W, "corrompu.pdf"); open(bad, "wb").write(b"%PDF-1.4\nceci n'est pas un pdf")
    empty = os.path.join(W, "vide.pdf"); open(empty, "wb").write(b"")
    txt = os.path.join(W, "notes.txt"); open(txt, "w").write("x")
    log, code, dt = run([], [f"open:{bad}", f"open:{empty}", f"open:{W}/absent.pdf", f"drop:{txt}", "etat"])
    errs = lines(log, "ERREUR")
    check("fichier corrompu : erreur claire", any("corrompu.pdf" in x for x in errs), str(errs))
    check("fichier vide : erreur claire", any("vide.pdf" in x for x in errs), str(errs))
    check("fichier absent : erreur claire", any("absent.pdf" in x and "introuvable" in x for x in errs), str(errs))
    check("déposer un .txt : refus expliqué", any("Seuls les PDF" in x for x in lines(log, "NOTE")), log[-300:])
    check("aucun onglet ouvert pour ces fichiers", not etat(log) or all("onglet" not in x for x in etat(log)), str(etat(log)))
    log, code, dt = run([f, f], [f"open:{f}", "etat"])
    check("même fichier ouvert 3 fois : un seul onglet", sum(1 for x in etat(log) if x.startswith("onglet")) == 1, str(etat(log)))
    # noms difficiles
    for name in ["été 😀 (copie) #1.pdf", "a" * 180 + ".pdf", "point.final..pdf"]:
        g = fresh(name)
        log, code, dt = run([g], ["etat"])
        check(f"nom difficile ouvert : {name[:30]}", any("pages=3" in x for x in etat(log)), log[-300:])
    # mot de passe
    d = pymupdf.open(FIX); enc = os.path.join(W, "protege.pdf")
    d.save(enc, encryption=pymupdf.PDF_ENCRYPT_AES_256, user_pw="ouvrir", owner_pw="proprio")
    log, code, dt = run([enc], ["password:faux", "password:ouvrir", "etat"])
    check("mot de passe : faux puis bon", any("pages=3" in x for x in etat(log)), log[-400:])
    log, code, dt = run([enc], ["password:a", "password:b", "password:c", "etat"])
    check("mot de passe : 3 échecs → message", any("trois essais" in x for x in lines(log, "ERREUR")), str(lines(log, "ERREUR")))

def g_navigation():
    f = fresh()
    acts = ["page:3", "etat", "page:0", "etat", "page:99", "etat", "page:abc", "etat", "step:1", "etat",
            "zoom:150", "etat", "zoom:0", "etat", "zoom:abc", "etat", "zoom:100000", "etat", "zoom:5", "etat",
            "rotate:90", "etat", "rotate:-90", "rotate:-90", "etat", "mode:1", "page:2", "etat", "mode:2", "etat", "mode:0", "fit:2", "etat"]
    log, code, dt = run([f], acts)
    e = [x for x in etat(log) if x.startswith("onglet")]
    pg = [re.search(r"page=(\d+)", x).group(1) for x in e]
    zm = [float(re.search(r"zoom=([\d.]+)", x).group(1)) for x in e]
    rot = [re.search(r"rotation=(-?\d+)", x).group(1) for x in e]
    check("aller page 3", pg[0] == "3", str(pg))
    check("page 0 : refusée (reste)", pg[1] == "3", str(pg))
    check("page 99 : dernière page", pg[2] == "3", str(pg))
    check("page « abc » : refusée", pg[3] == "3", str(pg))
    check("zoom 150 %", abs(zm[5] - 1.5) < 0.01, str(zm))
    check("zoom 0 refusé", abs(zm[6] - 1.5) < 0.01, str(zm))
    check("zoom « abc » refusé", abs(zm[7] - 1.5) < 0.01, str(zm))
    check("zoom 100000 plafonné", zm[8] <= 10.01, str(zm))
    check("zoom 5 % plancher", zm[9] >= 0.05, str(zm))
    check("rotation 90 puis -180 = 270", rot[10] == "90" and rot[11] == "270", str(rot))
    check("mode page simple + page 2", pg[12] == "2", str(pg))
    check("aucune erreur en navigation", not lines(log, "ERREUR"), str(lines(log, "ERREUR")))

def g_recherche():
    f = fresh()
    log, code, dt = run([f], ["search:renard", "sleep:800", "etat", "search-step:1", "etat", "search-step:-1", "search-step:-1", "etat",
                              "search:zzzintrouvable", "sleep:500", "etat", "search:été", "sleep:500", "etat", "search:(", "sleep:500",
                              "search:", "etat", "search:" + "x" * 500, "sleep:500", "etat"])
    e = [x for x in etat(log) if x.startswith("onglet")]
    r = [re.search(r"recherche=(\d+)/(\d+)", x).groups() for x in e]
    check("recherche « renard » : 3 résultats", r[0] == ("1", "3"), str(r))
    check("résultat suivant", r[1] == ("2", "3"), str(r))
    check("précédent en boucle", r[2] == ("3", "3"), str(r))
    check("introuvable : 0 et message", r[3][1] == "0" and any("Aucun résultat" in x for x in lines(log, "NOTE")), str(r))
    check("recherche vide efface", r[5] == ("0", "0"), str(r))
    check("recherche très longue sans plantage", len(r) == 7, str(r) + log[-300:])
    check("aucune erreur en recherche", not lines(log, "ERREUR"), str(lines(log, "ERREUR")))


# ----- 2. organiser, enregistrer --------------------------------------------------------------
def g_organiser():
    f = fresh("org.pdf")
    out = os.path.join(W, "extrait.pdf")
    acts = ["panel:1", "thumb:1", "action:rotate-right", "sleep:500", "etat",
            "action:delete", "sleep:500", "etat",  # réponse oui
            "action:undo", "sleep:500", "etat", "action:redo", "sleep:500", "action:undo", "sleep:500",
            "select:0", "select:1", "select:2", "action:delete", "sleep:500",  # toutes les pages : refus
            "thumb:2", "action:insert-blank", "sleep:500", "etat",
            "thumb:0", "action:extract", "sleep:800",
            "action:crop", "drag:0,50,50,300,300", "sleep:500", "etat", "action:reset-crop", "sleep:500", "etat"]
    log, code, dt = run([f], acts, picks=[out], answers=["oui"])
    e = [x for x in etat(log) if x.startswith("onglet 0")]
    get = lambda k: [re.search(k + r"=([^ ]+)", x).group(1) for x in e]
    check("pivoter la page sélectionnée", "true" in get("modifié")[0], str(e[:1]))
    check("supprimer (confirmé) : 2 pages", get("pages")[1] == "2", str(get("pages")))
    check("annuler la suppression : 3 pages", get("pages")[2] == "3", str(get("pages")))
    check("supprimer toutes les pages : refusé", any("au moins une page" in x for x in lines(log, "ERREUR")), str(lines(log, "ERREUR")))
    check("insérer une page vierge : 4 pages", get("pages")[3] == "4", str(get("pages")))
    check("extraire la page 1 dans un fichier", pages(out) == 1, str(pages(out)))
    check("recadrer puis annuler sans erreur", not lines(log, "ERREUR")[1:], str(lines(log, "ERREUR")))
    # fusion, scission
    g = fresh("b.pdf")
    merged = os.path.join(W, "fusion.pdf")
    log, code, dt = run([f], ["action:merge", "merge:add", "merge:run", "sleep:1500", "etat"], picks=[g, merged])
    check("fusionner deux PDF (6 pages)", pages(merged) == 6, str(pages(merged)) + str(lines(log, "ERREUR")))
    check("le PDF fusionné s'ouvre", any("fusion.pdf" in x for x in etat(log)), str(etat(log)))
    d = os.path.join(W, "scission"); os.makedirs(d, exist_ok=True)
    log, code, dt = run([f], ["split:1-1, 2-3", "sleep:1000"], picks=[d], answers=["non"])
    files = sorted(os.listdir(d))
    check("scinder en 1-1 et 2-3", files == ["org - pages 1.pdf", "org - pages 2-3.pdf"], str(files) + str(lines(log, "ERREUR")))
    log, code, dt = run([f], ["split:abc", "split:5-2", "split:", "split-every:0"], picks=[d, d, d, d], answers=["non"] * 4)
    errs = lines(log, "ERREUR")
    check("plages invalides refusées avec message", len(errs) >= 3, str(errs))
    check("« toutes les 0 pages » : pas de plantage", code != "TIMEOUT" and "panicked" not in log, log[-300:])

def g_enregistrer():
    f = fresh("save.pdf")
    before = open(f, "rb").read()
    log, code, dt = run([f], ["action:save", "sleep:500", "etat"], answers=["oui"])
    check("enregistrer sans modification : rien d'écrit", open(f, "rb").read() == before and not lines(log, "QUESTION"),
          str(lines(log, "QUESTION")))
    log, code, dt = run([f], ["action:rotate-right", "sleep:500", "action:save", "sleep:800", "etat"], answers=["non"])
    check("enregistrer, confirmation refusée : fichier intact", open(f, "rb").read() == before, "modifié")
    log, code, dt = run([f], ["action:rotate-right", "sleep:500", "action:save", "sleep:800", "etat"], answers=["oui"])
    check("enregistrer, confirmé : fichier modifié", open(f, "rb").read() != before and pymupdf.open(f)[0].rotation == 90,
          str(lines(log, "ERREUR")))
    check("onglet plus marqué modifié", any("modifié=false" in x for x in etat(log) if x.startswith("onglet")), str(etat(log)))
    other = os.path.join(W, "sous.pdf"); copy = os.path.join(W, "copie")
    log, code, dt = run([f], ["action:rotate-right", "sleep:500", "action:save-copy", "sleep:800", "etat",
                              "action:save-as", "sleep:800", "etat"], picks=[copy, other])
    check("enregistrer une copie (extension .pdf ajoutée)", os.path.exists(copy + ".pdf"), str(os.listdir(W)))
    check("copie : document toujours « modifié »", "modifié=true" in [x for x in etat(log) if x.startswith("onglet")][0], str(etat(log)))
    check("enregistrer sous : nouveau nom d'onglet", any("« sous.pdf »" in x and "modifié=false" in x for x in etat(log)), str(etat(log)))
    # dossier inexistant / protégé
    ro = os.path.join(W, "lecture-seule"); os.makedirs(ro, exist_ok=True); os.chmod(ro, 0o500)
    log, code, dt = run([f], ["action:rotate-right", "sleep:500", "action:save-as", "sleep:800",
                              "action:save-as", "sleep:800", "etat"], picks=[os.path.join(ro, "x.pdf"), os.path.join(W, "nulle/part/x.pdf")])
    errs = lines(log, "ERREUR")
    check("enregistrer dans un dossier protégé : message clair", any("accès refusé" in x for x in errs), str(errs))
    check("enregistrer dans un dossier inexistant : message clair", len(errs) >= 2, str(errs))
    os.chmod(ro, 0o700)
    # fermer / quitter avec modifications
    g = fresh("ferme.pdf"); gb = open(g, "rb").read()
    log, code, dt = run([g], ["action:rotate-right", "sleep:500", "close:0", "sleep:500", "etat"], answers=["2"])
    check("fermer + Annuler : onglet gardé", any("ferme.pdf" in x for x in etat(log)), str(etat(log)))
    log, code, dt = run([g], ["action:rotate-right", "sleep:500", "close:0", "sleep:500", "etat"], answers=["1"])
    check("fermer + Ne pas enregistrer : fichier intact, onglet fermé", open(g, "rb").read() == gb and not any("ferme.pdf" in x for x in etat(log)), str(etat(log)))
    log, code, dt = run([g], ["action:rotate-right", "sleep:500", "close:0", "sleep:800", "etat"], answers=["0"])
    check("fermer + Enregistrer : écrit puis fermé", open(g, "rb").read() != gb and not any("ferme.pdf" in x for x in etat(log)), str(etat(log)))


# ----- 3. commenter, modifier ----------------------------------------------------------------
def annots(path):
    d = pymupdf.open(path)
    return [(a.type[1], a.info.get("content", ""), a.info.get("title", "")) for pg in d for a in pg.annots()]

def g_commenter():
    f = fresh("comm.pdf")
    acts = ["panel:2", "author:Testeur 🎨", "tool:highlight", "drag:0,120,147,300,150", "drag:0,120,147,300,150", "sleep:500",
            "tool:note", "drag:0,400,300,400,300", "note:Ma note <b>&amp; « guillemets » 😀", "sleep:500",
            "tool:freetext", "drag:0,100,400,300,450", "note:Texte libre", "sleep:500",
            "tool:freetext", "drag:0,100,500,300,550", "note:", "sleep:500",  # vide : rien
            "color:4", "tool:rect", "drag:0,100,600,250,700", "tool:ellipse", "drag:0,300,600,450,700",
            "tool:line", "drag:0,100,750,300,760", "tool:arrow", "drag:0,320,750,500,780",
            "tool:ink", "drag:0,500,300,600,400", "tool:stamp", "drag:0,500,500,500,500",
            "tool:rect", "drag:0,10,10,12,12",  # trop petit : ignoré
            "sleep:800", "tool:", "etat", "rail:2",
            "comment:1", "action:delete", "sleep:500", "etat", "action:undo", "sleep:500", "etat",
            "action:save", "sleep:800"]
    log, code, dt = run([f], acts, answers=["oui"])
    e = [x for x in etat(log) if x.startswith("onglet")]
    n = [int(re.search(r"commentaires=(\d+)", x).group(1)) for x in e]
    check("9 commentaires créés (surlignage, note, texte libre, 4 formes, dessin, tampon)", n[0] == 9, str(n) + str(lines(log, "ERREUR")))
    check("texte libre vide : rien créé, trop petit : ignoré", n[0] == 9, str(n))
    check("Suppr supprime le commentaire sélectionné", n[1] == 8, str(n))
    check("Ctrl+Z le rend", n[2] == 9, str(n))
    a = annots(f)
    kinds = sorted(k for k, _, _ in a)
    check("enregistrés dans le fichier (types)", kinds.count("Square") == 1 and "Highlight" in kinds and "Ink" in kinds and "Stamp" in kinds, str(kinds))
    check("note : texte spécial conservé", any(c == "Ma note <b>&amp; « guillemets » 😀" for _, c, _ in a), str(a))
    check("auteur avec emoji", any(t == "Testeur 🎨" for _, _, t in a), str(a))
    check("aucune erreur", not lines(log, "ERREUR"), str(lines(log, "ERREUR")))

def g_modifier():
    f = fresh("modif.pdf")
    img = REPO + "/tests/fixtures/image.png"
    acts = ["panel:3", "tool:text", "drag:0,120,500,120,500", "type:Bonjour 😀 été €", "drag:0,600,800,600,800", "sleep:600",
            "tool:text", "drag:0,120,600,120,600", "type:   ", "drag:0,600,800,600,800", "sleep:500",  # blanc : rien
            "text-color:1", "text-size:24", "etat",
            "tool:edit-text", "drag:0,120,147,120,147", "sleep:500", "type:Titre corrigé", "drag:0,600,800,600,800", "sleep:800",
            "action:add-image", "drag:0,300,300,500,450", "sleep:800",
            "header", "sleep:800", "watermark:BROUILLON", "sleep:800", "watermark:", "sleep:500",
            "tool:redact", "drag:0,105,190,330,205", "sleep:500",
            "action:redact-text", "note:renard", "sleep:800", "action:redact-apply", "sleep:1000",
            "clean", "sleep:800", "search:Titre corrigé", "sleep:800", "etat", "action:save", "sleep:1000"]
    log, code, dt = run([f], acts, picks=[img], answers=["oui", "oui"])
    d = pymupdf.open(f)
    text = d[0].get_text()
    check("texte ajouté (sans l'emoji, police latine) ", any("Bonjour" in c for _, c, _ in annots(f)), str(annots(f)))
    check("texte d'origine corrigé", "Titre corrigé" in text and "Bonjour page 1" not in text, text[:200])
    check("image ajoutée", len(d[0].get_images()) >= 1, str(d[0].get_images()))
    check("texte corrigé trouvé par la recherche", any("recherche=1/1" in x for x in etat(log)), str(etat(log)))
    check("en-tête / pied ajouté", "Page 1 sur 3" in text, text[:300])
    check("filigrane ajouté", "BROUILLON" in text, text[:300])
    check("filigrane vide refusé avec message", any("filigrane est vide" in x for x in lines(log, "ERREUR")), str(lines(log, "ERREUR")))
    check("caviardage : « renard » retiré de toutes les pages", all("renard" not in pg.get_text() for pg in d), "")
    check("nettoyage : métadonnées vides", not any(d.metadata.get(k) for k in ("title", "author", "producer")), str(d.metadata))
    other = [x for x in lines(log, "ERREUR") if "filigrane" not in x]
    check("aucune autre erreur", not other, str(other))
    check("avertissements montrés (texte ajouté, police)", True, "\n".join(lines(log, "INFO")))
    print("     INFO :", lines(log, "INFO"))


# ----- 4. formulaires, protection, propriétés --------------------------------------------------
def g_formulaires():
    f = fresh("form.pdf")
    acts = ["panel:4", "tool:field-text", "drag:0,100,300,400,330", "tool:field-check", "drag:0,100,360,100,360",
            "tool:field-combo", "drag:0,100,400,300,425", "note:Rouge\nVert\n\nBleu", "sleep:500",
            "tool:field-signature", "drag:0,100,460,400,540", "sleep:500", "tool:", "etat",
            "drag:0,200,315,200,315", "sleep:300", "type:Jean Dupré 😀 <&>", "action:editor-commit", "sleep:500",
            "drag:0,110,370,110,370", "sleep:500", "drag:0,110,370,110,370", "sleep:500", "drag:0,110,370,110,370", "sleep:500",
            "drag:0,200,412,200,412", "choice:1", "sleep:500",
            "tool:field-text", "drag:0,200,315,200,315", "sleep:500", "tool:", "etat", "action:undo", "sleep:500", "etat",
            "action:save", "sleep:800"]
    log, code, dt = run([f], acts, answers=["oui", "oui"])
    e = [x for x in etat(log) if x.startswith("onglet")]
    n = [int(re.search(r"champs=(\d+)", x).group(1)) for x in e]
    check("4 champs créés", n[0] == 4, str(n) + str(lines(log, "ERREUR")))
    check("clic sur un champ avec un outil : suppression proposée", n[1] == 3 and n[2] == 4, str(n))
    d = pymupdf.open(f)
    w = {x.field_name: x for x in d[0].widgets()}
    vals = {k: v.field_value for k, v in w.items()}
    check("champ texte rempli (caractères spéciaux)", vals.get("Texte1", "").startswith("Jean Dupré"), str(vals))
    check("case cochée (3 clics : cochée)", vals.get("Case1") not in (None, "Off", False), str(vals))
    check("liste : 2e choix « Vert » (ligne vide ignorée)", vals.get("Liste1") == "Vert", str(vals))
    check("champ de signature vide créé", "Signature1" in w, str(list(w)))
    check("aucune erreur", not lines(log, "ERREUR"), str(lines(log, "ERREUR")))

def g_protection():
    f = fresh("prot.pdf")
    out = os.path.join(W, "prot-copie.pdf")
    log, code, dt = run([f], ["protect:abc,abc,1,1,1,0", "protect:,,1,1,1,1", "protect:ouvrir,proprio,1,0,0,0", "sleep:800",
                              "action:save-copy", "sleep:1000"], picks=[out])
    errs = lines(log, "ERREUR")
    check("protection : mots de passe identiques refusés", any("doit différer" in x for x in errs), str(errs))
    check("protection : rien à protéger refusé", any("Indiquez un mot de passe" in x for x in errs), str(errs))
    d = pymupdf.open(out)
    check("copie protégée : mot de passe exigé", d.needs_pass, "")
    check("mot de passe d'ouverture accepté", d.authenticate("ouvrir") > 0, "")
    log, code, dt = run([out], ["password:ouvrir", "action:unprotect", "unprotect:faux", "sleep:500", "unprotect:proprio", "sleep:800", "etat"])
    errs = lines(log, "ERREUR")
    check("retirer : mauvais mot de passe des permissions refusé", any("incorrect" in x for x in errs), str(errs))
    check("retirer avec le bon mot de passe", any("Protection retirée" in x for x in lines(log, "NOTE")), str(lines(log, "NOTE")))
    g = fresh("props.pdf")
    log, code, dt = run([g], ["action:properties", "sleep:500", "prop:0=Titre « été » 😀", "prop:1=Alex", "props-save", "sleep:500",
                              "action:save", "sleep:800"], answers=["oui"])
    m = pymupdf.open(g).metadata
    check("propriétés : titre et auteur enregistrés", m.get("title") == "Titre « été » 😀" and m.get("author") == "Alex", str(m))


# ----- 5. convertir, OCR -----------------------------------------------------------------------
def g_convertir():
    f = fresh("conv.pdf")
    docx = os.path.join(W, "conv.docx"); txt = os.path.join(W, "conv.txt"); imgs = os.path.join(W, "images"); os.makedirs(imgs, exist_ok=True)
    small = os.path.join(W, "conv-reduit.pdf"); fromimg = os.path.join(W, "depuis-images.pdf"); pdfa = os.path.join(W, "pdfa.pdf")
    img = REPO + "/tests/fixtures/image.png"
    log, code, dt = run([f], ["action:export-word", "sleep:1500", "action:export-text", "sleep:800",
                              "export-images:2-3", "sleep:1500", "compress:2", "sleep:1500",
                              "action:from-images", "sleep:1500", "action:from-office", "sleep:500", "etat"],
                        picks=[docx, txt, imgs, small, img, fromimg], answers=["non"] * 10, timeout=300)
    check("Word créé", os.path.exists(docx) and open(docx, "rb").read(2) == b"PK", str(lines(log, "ERREUR")))
    check("texte créé (3 pages, sauts de page)", os.path.exists(txt) and open(txt, encoding="utf-8").read().count("\f") == 2, "")
    check("images des pages 2-3", sorted(os.listdir(imgs)) == ["conv - page 002.png", "conv - page 003.png"], str(os.listdir(imgs)))
    check("copie réduite lisible", pages(small) == 3, str(pages(small)))
    check("PDF depuis une image", pages(fromimg) == 1, str(pages(fromimg)))
    lo = shutil.which("soffice") or shutil.which("libreoffice")
    check("Office : " + ("LibreOffice présent" if lo else "LibreOffice absent → message avec lien"), lo or any("libreoffice.org" in x for x in lines(log, "ERREUR")), str(lines(log, "ERREUR")))
    other = [x for x in lines(log, "ERREUR") if "ghostscript" not in x.lower() and "libreoffice" not in x.lower()]
    check("aucune autre erreur", not other, str(other))

def g_ocr():
    src = pymupdf.open(FIX)
    pix = src[0].get_pixmap(dpi=200)
    scan = pymupdf.open(); pg = scan.new_page(width=595, height=842); pg.insert_image(pg.rect, pixmap=pix)
    scanf = os.path.join(W, "scan.pdf"); scan.save(scanf)
    log, code, dt = run([scanf], ["ocr:", "sleep:15000", "search:renard", "sleep:800", "etat", "ocr:", "sleep:1000", "action:save", "sleep:800"],
                        answers=["oui"], timeout=300)
    e = [x for x in etat(log) if x.startswith("onglet")]
    check("OCR puis recherche « renard » trouvée", e and "recherche=1/1" in e[0], str(e) + str(lines(log, "ERREUR")))
    check("OCR relancé : pages déjà traitées signalées", any("contiennent déjà du texte" in x for x in lines(log, "NOTE")), str(lines(log, "NOTE")))
    check("texte reconnu enregistré", "renard" in pymupdf.open(scanf)[0].get_text(), "")
    check("aucune erreur", not lines(log, "ERREUR"), str(lines(log, "ERREUR")))

# ----- 6. signer, comparer, assistant -----------------------------------------------------------
def g_signer():
    f = fresh("signer.pdf")
    out = os.path.join(W, "signer - signé.pdf")
    pfx = REPO + "/tests/fixtures/sign/rsa-aes.pfx"
    log, code, dt = run([f], ["panel:6", "sig-typed:Alex Testeur", "sleep:300", "sig-place:0", "drag:0,100,600,300,680", "sleep:800",
                              "tool:digital-sign", "drag:0,320,600,520,680", "sleep:300", f"digital:{pfx},faux", "sleep:500",
                              f"digital:{pfx},test", "sleep:3000", "rail:5", "action:verify", "sleep:3000", "etat",
                              "trust:0", "sleep:3000", "etat", "version:0", "sleep:1500", "etat"],
                        picks=[out], answers=["oui", "oui", "oui"], timeout=300)
    errs = lines(log, "ERREUR")
    check("certificat : mauvais mot de passe refusé", any("Mot de passe incorrect" in x for x in errs), str(errs))
    d = pymupdf.open(out) if os.path.exists(out) else None
    check("document signé enregistré", d is not None and b"/ByteRange" in open(out, "rb").read() and len(d[0].get_images()) >= 1, str(errs))
    e = etat(log)
    check("signature vérifiée dans le rail", any("signatures=1" in x for x in e), str(e))
    check("faire confiance puis version signée ouverte", any("version signée" in x for x in e), str(e))
    other = [x for x in errs if "Mot de passe incorrect" not in x]
    check("aucune autre erreur", not other, str(other))
    print("     NOTES :", lines(log, "NOTE")[-4:])

def g_comparer():
    a = fresh("v1.pdf")
    d = pymupdf.open(FIX); p = d[0]; p.add_redact_annot(p.search_for("brun")[0]); p.apply_redactions()
    p.insert_text((72, 200), "Phrase ajoutée", fontname="helv", fontsize=12); b = os.path.join(W, "v2.pdf"); d.save(b)
    log, code, dt = run([a], [f"compare:{b}", "sleep:2000", "diff:0", "sleep:300", "etat", "diff:1", "sleep:300", "etat", f"compare:{b}"])
    e = etat(log)
    check("2 différences listées", any("différences=2" in x for x in e), str(e))
    check("clic sur une différence : bon onglet", any("onglet 0* « v1.pdf »" in x for x in e) and any("onglet 1* « v2.pdf »" in x for x in e), str(e))
    check("comparer un document avec lui-même : refusé", any("autre document" in x for x in lines(log, "ERREUR")), str(lines(log, "ERREUR")))

def g_assistant():
    f = fresh("ia.pdf")
    log, code, dt = run([f], ["rail:3", "ask:Que fait le renard ? Un mot.", "sleep:25000", "etat"], timeout=300)
    check("assistant : réponse sans erreur", not lines(log, "ERREUR") and code != "TIMEOUT", str(lines(log, "ERREUR")) + str(code))


def g_pdfa():
    f = fresh("pa.pdf"); pdfa = os.path.join(W, "pa-A.pdf")
    log, code, dt = run([f], ["action:pdfa", "sleep:15000"], picks=[pdfa], answers=["non"], timeout=300)
    check("PDF/A créé par Ghostscript", pages(pdfa) == 3, str(lines(log, "ERREUR")) + str(lines(log, "QUESTION")))


# ----- 7. cas limites ----------------------------------------------------------------------------
def g_limites():
    f = fresh("lim.pdf")
    log, code, dt = run([f], ["action:rotate-right"] * 40 + ["sleep:3000", "etat"] + ["action:undo"] * 45 + ["sleep:3000", "etat"])
    e = [x for x in etat(log) if x.startswith("onglet")]
    check("40 rotations rapides puis 45 annulations : pas de plantage", code != "TIMEOUT" and "panicked" not in log and len(e) == 2, log[-300:])
    check("historique limité à 30 : il reste des étapes non annulables", "annuler=false" in e[1], str(e))
    g = fresh("double.pdf")
    log, code, dt = run([g], ["action:rotate-right", "sleep:500", "action:save", "action:save", "sleep:1500", "etat"], answers=["oui", "oui"])
    q = lines(log, "QUESTION")
    check("double Enregistrer : une seule confirmation", len(q) == 1, str(q))
    check("fichier lisible après double enregistrement", pages(g) == 3, "")
    src = pymupdf.open(FIX); scan = pymupdf.open()
    for _ in range(4):
        pg = scan.new_page(width=595, height=842); pg.insert_image(pg.rect, pixmap=src[0].get_pixmap(dpi=150))
    sc = os.path.join(W, "scan4.pdf"); scan.save(sc)
    log, code, dt = run([sc], ["ocr:", "sleep:1500", "close:0", "sleep:12000", "etat"], timeout=300)
    check("fermer le document pendant l'OCR : pas de message d'erreur", not lines(log, "ERREUR"), str(lines(log, "ERREUR")))
    long = pymupdf.open()
    for _ in range(40):
        pg = long.new_page(width=595, height=842); pg.insert_image(pg.rect, pixmap=src[0].get_pixmap(dpi=300))
    long.save(sc)  # le passage précédent a enregistré le texte reconnu ; 40 pages pour avoir le temps d'arrêter
    log, code, dt = run([sc], ["ocr:", "sleep:500", "action:escape", "sleep:3000", "etat"], answers=["1"], timeout=300)
    check("Échap arrête l'OCR", any("Arrêt" in x or "Arrêté" in x for x in lines(log, "NOTE")), str(lines(log, "NOTE")))
    h = fresh("h.pdf")
    log, code, dt = run([h], ["search:renard", "close:0", "sleep:1000", "etat"])
    check("fermer pendant une recherche : pas d'erreur", not lines(log, "ERREUR") and "panicked" not in log, log[-300:])
    log, code, dt = run([h], ["action:undo", "action:redo", "action:delete", "action:extract", "etat"], picks=[], answers=["non"])
    check("annuler / rétablir / extraire sans rien à faire : pas d'erreur", not lines(log, "ERREUR") and "panicked" not in log, str(lines(log, "ERREUR")))
    log, code, dt = run([], ["action:save", "action:rotate-right", "action:undo", "action:ocr", "action:protect", "action:print",
                              "action:compare", "action:verify", "action:export-word", "ask:bonjour", "search:x", "etat"])
    check("actions sans document ouvert : ignorées sans erreur", not lines(log, "ERREUR") and "panicked" not in log, str(lines(log, "ERREUR")) + log[-200:])


# ----- 8. régressions des correctifs de l'audit --------------------------------------------------
def g_correctifs():
    a = fresh("ca.pdf"); b = fresh("cb.pdf"); bb = open(b, "rb").read()
    log, code, dt = run([a, b], ["tab:0", "action:rotate-right", "sleep:500", "action:save-as", "sleep:800", "etat"], picks=[b])
    check("A1 Enregistrer sous vers un fichier ouvert ailleurs : refusé, fichier intact",
          any("ouvert dans un autre onglet" in x for x in lines(log, "ERREUR")) and open(b, "rb").read() == bb, str(lines(log, "ERREUR")))
    src = pymupdf.open(FIX); scan = pymupdf.open()
    for _ in range(6):
        pg = scan.new_page(width=595, height=842); pg.insert_image(pg.rect, pixmap=src[0].get_pixmap(dpi=300))
    sc = os.path.join(W, "scan6.pdf"); scan.save(sc); other = fresh("autre.pdf")
    log, code, dt = run([sc, other], ["tab:0", "ocr:", "sleep:500", "tab:1", "sleep:30000", "etat"], answers=["2"], timeout=300)
    e = etat(log)
    check("A2 OCR puis changement d'onglet : texte sur le bon document, sans erreur",
          any("scan6" in x and "modifié=true" in x for x in e) and any("autre" in x and "modifié=false" in x for x in e)
          and not lines(log, "ERREUR"), str(e) + str(lines(log, "ERREUR")))
    g = fresh("cs.pdf"); gb = open(g, "rb").read()
    log, code, dt = run([g], ["action:save", "sleep:500"], answers=["oui"])
    check("A3 Enregistrer sans modification : aucune écriture, aucune question",
          open(g, "rb").read() == gb and not lines(log, "QUESTION") and any("Aucune modification" in x for x in lines(log, "NOTE")), log[-300:])
    log, code, dt = run([g, g], [f"open:{g}", "etat"])
    check("A4 même fichier ouvert 3 fois coup sur coup : un onglet", sum(1 for x in etat(log) if x.startswith("onglet")) == 1, str(etat(log)))
    h = fresh("ch.pdf"); k = fresh("ck.pdf")
    log, code, dt = run([h, k], ["tab:0", "search:renard", "tab:1", "sleep:1500", "etat", "tab:0", "search-step:1", "etat"])
    e = [x for x in etat(log) if x.startswith("onglet")]
    check("A5 recherche puis changement d'onglet : résultats gardés sur le bon onglet",
          "recherche=0/0" in e[1] and re.search(r"recherche=\d+/3", e[2]), str(e))

def g_bords():
    f = fresh("bord.pdf"); img = REPO + "/tests/fixtures/image.png"
    log, code, dt = run([f], ["tool:stamp", "drag:0,920,1300,920,1300", "tool:note", "drag:0,925,1305,925,1305", "note:x",
        "tool:freetext", "drag:0,920,1300,920,1300", "note:texte au bord", "action:add-image", "drag:0,920,1300,920,1300", "sleep:800",
        "panel:4", "tool:field-text", "drag:0,920,1300,920,1300", "sleep:500", "tool:", "action:save", "sleep:1000"],
        picks=[img], answers=["oui", "oui"])
    p = pymupdf.open(f)[0]; page = p.rect + (-0.5, -0.5, 0.5, 0.5)
    objs = [a.rect for a in p.annots()] + [w.rect for w in p.widgets()] + [pymupdf.Rect(i["bbox"]) for i in p.get_image_info()]
    check("A6 objets posés d'un clic au coin : dans la page", len(objs) == 5 and all(page.contains(r) for r in objs), str(objs))

# ----- 9. clavier et souris (fenêtre réelle : touches et clics envoyés) ---------------------
def g_clavier():
    f = fresh("clavier.pdf")
    log, code, dt = run([f], ["key:Tab"] * 8 + ["key:Return", "etat"])
    check("A7 au clavier : 8 Tab puis Entrée ouvrent « Organiser les pages »", any("panneau=1" in x for x in etat(log)), str(etat(log)))
    log, code, dt = run([f], ["tool:note", "key:Tab", "key:Tab", "key:Escape", "etat"])
    check("A8 bouton au focus : Échap atteint toujours les raccourcis", any("outil=«  »" in x for x in etat(log)), str(etat(log)))
    log, code, dt = run([f], ["click:100,108", "etat", "click:1256,158", "etat", "click:120,26", "etat"])
    e = [x for x in etat(log) if x.startswith("accueil")]
    check("A9 clics souris : catalogue, panneau latéral, Accueil",
          "panneau=1" in e[0] and "rail=1" in e[1] and "accueil=true" in e[2], str(e))
    log, code, dt = run([f], ["resize:720,640", "etat"])
    check("A10 fenêtre redimensionnée à 720 px : pas d'erreur", not lines(log, "ERREUR") and "panicked" not in log, log[-300:])

# ----- 10. parité avec la version Python ----------------------------------------------------
def g_parite():
    a = fresh("Partie A.pdf"); b = fresh("Partie B.pdf"); merged = os.path.join(W, "fusion-signets.pdf")
    log, code, dt = run([a], ["action:merge", "merge:add", "merge:run", "sleep:1500"], picks=[b, merged])
    toc = pymupdf.open(merged).get_toc()
    check("P1 fusion : un signet par fichier", [t[1:] for t in toc if t[0] == 1] == [["Partie A", 1], ["Partie B", 4]], str(toc))
    d = os.path.join(W, "par-signets"); os.makedirs(d, exist_ok=True)
    log, code, dt = run([merged], ["split-bookmarks", "sleep:1000"], picks=[d], answers=["non"])
    files = sorted(os.listdir(d))
    check("P2 scinder selon les signets : un fichier par signet, nommé d'après lui",
          files == ["fusion-signets - 01 - Partie A.pdf", "fusion-signets - 02 - Partie B.pdf"]
          and all(pages(os.path.join(d, f)) == 3 for f in files), str(files) + str(lines(log, "ERREUR")))
    log, code, dt = run([fresh("sans.pdf", REPO + "/tests/fixtures/tableau.pdf")], ["split-bookmarks"], picks=[d])
    check("P3 scinder selon les signets sans signet : message", any("signet" in x for x in lines(log, "ERREUR")), str(lines(log, "ERREUR")))

def g_images_liens():
    img = REPO + "/tests/fixtures/image.png"; f = fresh("img.pdf")
    log, code, dt = run([f], ["panel:3", "action:add-image", "drag:0,200,300,400,450", "sleep:600", "etat", "action:save", "sleep:800"], picks=[img], answers=["oui"])
    z = float(re.search(r"zoom=([\d.]+)", etat(log)[0]).group(1))
    r0 = pymupdf.open(f)[0].get_image_info()[0]["bbox"]
    cx, cy = (r0[0] + r0[2]) / 2 * z, (r0[1] + r0[3]) / 2 * z
    log, code, dt = run([f], ["panel:3", "tool:images", "sleep:800", f"drag:0,{cx},{cy},{cx+100*z},{cy+50*z}", "sleep:800", "action:save", "sleep:800"], answers=["oui"])
    r1 = pymupdf.open(f)[0].get_image_info()[0]["bbox"]
    check("I1 glisser l'image : déplacée de (100, 50) pt", abs(r1[0] - r0[0] - 100) < 2 and abs(r1[1] - r0[1] - 50) < 2, f"{r0} -> {r1}")
    hx, hy = r1[2] * z - 2, r1[3] * z - 2
    log, code, dt = run([f], ["panel:3", "tool:images", "sleep:800", f"drag:0,{hx},{hy},{hx+60*z},{hy+10*z}", "sleep:800", "action:save", "sleep:800"], answers=["oui"])
    r2 = pymupdf.open(f)[0].get_image_info()[0]["bbox"]
    w1, h1, w2, h2 = r1[2]-r1[0], r1[3]-r1[1], r2[2]-r2[0], r2[3]-r2[1]
    check("I2 poignée : agrandie de 60 pt, proportions gardées", abs(w2 - w1 - 60) < 2 and abs(w2 / h2 - w1 / h1) < 0.02, f"{r1} -> {r2}")
    cx, cy = (r2[0] + r2[2]) / 2 * z, (r2[1] + r2[3]) / 2 * z
    log, code, dt = run([f], ["panel:3", "tool:images", "sleep:800", f"drag:0,{cx},{cy},{cx},{cy}", "action:delete", "sleep:800", "action:save", "sleep:800"], answers=["oui"])
    check("I3 clic puis Suppr : image retirée", not pymupdf.open(f)[0].get_image_info(), str(lines(log, "ERREUR")))
    g = fresh("lien.pdf")
    log, code, dt = run([g], ["panel:3", "tool:link", f"drag:0,{100*z},{100*z},{250*z},{120*z}", "note:exemple.fr", "sleep:600",
                              f"drag:0,{100*z},{140*z},{250*z},{160*z}", "note:2", "sleep:600", "action:save", "sleep:800"], answers=["oui"])
    got = sorted((str(l.get("uri")), l.get("page")) for l in pymupdf.open(g)[0].get_links())
    check("L1 deux liens créés (adresse web, page 2)", got == [("None", 1), ("https://exemple.fr", None)], str(got))
    log, code, dt = run([g], ["panel:3", "tool:link", "sleep:500", f"drag:0,{150*z},{110*z},{150*z},{110*z}", "sleep:600", "action:save", "sleep:800"], answers=["oui", "oui"])
    links = pymupdf.open(g)[0].get_links()
    check("L2 clic sur un lien : suppression proposée et faite", len(links) == 1 and links[0].get("page") == 1, str(links))
    log, code, dt = run([g], ["panel:3", "tool:link", f"drag:0,{100*z},{300*z},{250*z},{320*z}", "note:javascript:alert(1)", "sleep:600"])
    check("L3 lien javascript: refusé", any("Seules les adresses web" in x for x in lines(log, "ERREUR")), str(lines(log, "ERREUR")))

# ----- 11. vraie souris et clavier : gestes, outil Main, présentation ------------------------
def g_souris():
    f = fresh("souris.pdf")
    log, code, dt = run([f], ["tool:rect", "mouse-drag:500,400,700,600", "sleep:500", "etat"])
    check("R1 tracer un rectangle à la souris (glisser réel)", any("commentaires=1" in x for x in etat(log)), str(etat(log)))
    log, code, dt = run([f], ["tool:ellipse", "mouse-drag:500,400,700,600", "sleep:500", "tool:", "mouse-drag:600,500,600,800", "sleep:500",
                              "action:save", "sleep:800"], answers=["oui"])
    r = [a.rect for a in pymupdf.open(f)[0].annots()]
    check("R2 déplacer un commentaire à la souris", len(r) == 1 and r[0].y0 > 400, str(r))
    log, code, dt = run([f], ["key:h", "mouse-drag:600,800,600,150", "sleep:300", "etat", "key:v", "etat"])
    e = [x for x in etat(log) if x.startswith("onglet")]
    a = [x for x in etat(log) if x.startswith("accueil")]
    # R3 commence par H sans aucun clic : les raccourcis sont actifs dès l'ouverture
    check("R3 H puis glisser : la page défile (Main), V : retour à la sélection",
          re.search(r"haut=(\d+)", e[0]) and int(re.search(r"haut=(\d+)", e[0]).group(1)) > 500 and "outil=«  »" in a[1], str(e) + str(a))
    log, code, dt = run([f], ["key:F5", "sleep:600", "key:Right", "sleep:600", "key:Escape", "sleep:300", "etat"])
    check("R4 F5, flèche, Échap : présentation puis retour à la page 2", any("page=2" in x for x in etat(log)) and not lines(log, "ERREUR"), str(etat(log)))

groups = {k[2:]: v for k, v in list(globals().items()) if k.startswith("g_")}
if __name__ == "__main__":
    for g in (sys.argv[1:] or groups):
        print(f"== {g}", flush=True)
        try:
            groups[g]()
        except Exception as e:
            check(f"{g} : exception du script", False, repr(e))
    bad = [r for r in results if not r[1]]
    print(f"\n{len(results) - len(bad)}/{len(results)} OK ; dossier : {W}")
    sys.exit(1 if bad else 0)
