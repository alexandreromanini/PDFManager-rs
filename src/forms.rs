//! Formulaires AcroForm : lister, remplir, créer et supprimer des champs. Sans interface.
//! Coordonnées : points, repère de la page affichée (comme `pdf::Rect`). Chaque
//! modification est une étape annulable (`Pdf::edit`).

use mupdf::pdf::{FieldFlags, PdfDocument, PdfObject, PdfPage, PdfWidget, WidgetType};

use crate::pdf::{Pdf, Rect};

#[derive(Debug, Clone, PartialEq)]
pub struct FieldInfo {
    pub page: usize,
    pub xref: i32,
    pub name: String,
    /// "text", "check", "radio", "combo", "list", "signature" ou "button".
    pub kind: &'static str,
    /// Cases et boutons radio : état (« Off » = décoché).
    pub value: String,
    pub choices: Vec<String>,
    pub rect: Rect,
    pub readonly: bool,
    pub multiline: bool,
    pub signed: bool,
}

impl FieldInfo {
    pub fn checked(&self) -> bool {
        !matches!(self.value.as_str(), "" | "Off")
    }
}

/// Champs que l'on sait créer : type, préfixe du nom proposé, libellé d'annulation.
pub const NEW: [(&str, &str, &str); 4] = [
    ("text", "Texte", "Champ texte"),
    ("check", "Case", "Case à cocher"),
    ("combo", "Liste", "Liste déroulante"),
    ("signature", "Signature", "Champ de signature"),
];

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn kind_of(w: &PdfWidget) -> &'static str {
    match w.r#type() {
        Ok(WidgetType::Text) => "text",
        Ok(WidgetType::Checkbox) => "check",
        Ok(WidgetType::RadioButton) => "radio",
        Ok(WidgetType::Combobox) => "combo",
        Ok(WidgetType::Listbox) => "list",
        Ok(WidgetType::Signature) => "signature",
        _ => "button",
    }
}

fn text_of(o: &PdfObject) -> Option<String> {
    if o.is_name().unwrap_or(false) {
        o.as_name().ok().map(|n| String::from_utf8_lossy(&n).into_owned())
    } else {
        o.as_string().ok()
    }
}

/// Choix d'une liste : chaînes, ou paires [valeur exportée, libellé].
fn choices(w: &PdfWidget) -> Vec<String> {
    let Ok(Some(opt)) = w.annotation().object().get_dict_inheritable("Opt") else { return vec![] };
    (0..opt.len().unwrap_or(0) as i32)
        .filter_map(|i| opt.get_array(i).ok().flatten())
        .filter_map(|c| if c.is_array().unwrap_or(false) { c.get_array(1).ok().flatten().and_then(|l| text_of(&l)) } else { text_of(&c) })
        .collect()
}

fn info(page: usize, w: &PdfWidget) -> FieldInfo {
    let flags = w.field_flags().unwrap_or(FieldFlags::empty());
    let kind = kind_of(w);
    FieldInfo {
        page,
        xref: w.xref().unwrap_or(0),
        name: w.name().ok().flatten().unwrap_or_default(),
        kind,
        value: w.value().ok().flatten().unwrap_or_default(),
        choices: choices(w),
        rect: w.annotation().bounds().map(Rect::from).unwrap_or_default(),
        readonly: flags.contains(FieldFlags::READ_ONLY),
        multiline: kind == "text" && flags.contains(FieldFlags::MULTILINE),
        signed: kind == "signature" && w.is_signed().unwrap_or(false),
    }
}

pub fn fields(pdf: &Pdf) -> Vec<FieldInfo> {
    (0..pdf.page_count())
        .filter_map(|i| pdf.doc().load_pdf_page(i as i32).ok().map(|p| (i, p)))
        .flat_map(|(i, p)| p.widgets().map(|w| info(i, &w)).collect::<Vec<_>>())
        .collect()
}

fn find(p: &PdfPage, xref: i32) -> Result<PdfWidget, String> {
    p.widgets().find(|w| w.xref().ok() == Some(xref)).ok_or_else(|| "Champ introuvable.".into())
}

/// État « coché » d'une case (« Yes » le plus souvent) : la clé de /AP /N autre que Off.
fn on_state(w: &PdfWidget) -> String {
    let n = w.annotation().object().get_dict("AP").ok().flatten().and_then(|ap| ap.get_dict("N").ok().flatten());
    n.and_then(|n| {
        (0..n.dict_len().unwrap_or(0) as i32)
            .filter_map(|i| n.get_dict_key(i).ok().flatten().and_then(|k| text_of(&k)))
            .find(|k| k != "Off")
    })
    .unwrap_or_else(|| "Yes".into())
}

/// Texte et listes : `value`. Case et bouton radio : "" ou "Off" = décoché, sinon coché ;
/// cocher un bouton radio décoche les autres du groupe (MuPDF s'en charge).
pub fn set_value(pdf: &mut Pdf, field: &FieldInfo, value: &str) -> Result<(), String> {
    if field.readonly {
        return Err(format!("Le champ « {} » est en lecture seule.", field.name));
    }
    if matches!(field.kind, "signature" | "button") {
        return Err("Ce champ ne se remplit pas (signature ou bouton).".into());
    }
    let (page, xref, toggle) = (field.page, field.xref, matches!(field.kind, "check" | "radio"));
    pdf.edit("Remplir le formulaire", |doc| {
        let p = doc.load_pdf_page(page as i32).map_err(err)?;
        let mut w = find(&p, xref)?;
        let v = if toggle && !matches!(value, "" | "Off") { on_state(&w) } else { value.to_string() };
        w.set_value(doc, &v, true).map_err(err)?;
        w.update().map_err(err)?;
        Ok(())
    })
}

pub fn names(pdf: &Pdf) -> Vec<String> {
    fields(pdf).into_iter().map(|f| f.name).collect()
}

pub fn unique_name(pdf: &Pdf, kind: &str) -> String {
    let taken = names(pdf);
    let prefix = NEW.iter().find(|n| n.0 == kind).map_or("Champ", |n| n.1);
    (1..).map(|n| format!("{prefix}{n}")).find(|n| !taken.contains(n)).unwrap()
}

fn name(n: &str) -> Result<PdfObject, String> {
    PdfObject::new_name(n).map_err(err)
}

fn array(doc: &PdfDocument, values: &[f32]) -> Result<PdfObject, String> {
    let mut a = doc.new_array().map_err(err)?;
    for &v in values {
        a.array_push(PdfObject::new_real(v).map_err(err)?).map_err(err)?;
    }
    Ok(a)
}

/// Dictionnaire /AcroForm du document (créé s'il manque), avec la police Helvetica.
fn acroform(doc: &mut PdfDocument) -> Result<PdfObject, String> {
    let mut catalog = doc.catalog().map_err(err)?;
    if let Some(form) = catalog.get_dict("AcroForm").map_err(err)? {
        return Ok(form);
    }
    let mut helv = doc.new_dict().map_err(err)?;
    helv.dict_put("Type", name("Font")?).map_err(err)?;
    helv.dict_put("Subtype", name("Type1")?).map_err(err)?;
    helv.dict_put("BaseFont", name("Helvetica")?).map_err(err)?;
    helv.dict_put("Encoding", name("WinAnsiEncoding")?).map_err(err)?;
    let mut fonts = doc.new_dict().map_err(err)?;
    fonts.dict_put("Helv", doc.add_object(&helv).map_err(err)?).map_err(err)?;
    let mut dr = doc.new_dict().map_err(err)?;
    dr.dict_put("Font", fonts).map_err(err)?;
    let mut form = doc.new_dict().map_err(err)?;
    form.dict_put("Fields", doc.new_array().map_err(err)?).map_err(err)?;
    form.dict_put("DA", PdfObject::new_string("/Helv 0 Tf 0 g").map_err(err)?).map_err(err)?;
    form.dict_put("DR", dr).map_err(err)?;
    let form = doc.add_object(&form).map_err(err)?;
    catalog.dict_put("AcroForm", form.clone()).map_err(err)?;
    Ok(form)
}

/// Crée un champ (`kind` : "text", "check", "combo" ou "signature") ; renvoie son xref.
/// Sur une page tournée, son contenu reste droit à l'écran (/MK /R).
pub fn add_field(pdf: &mut Pdf, page: usize, rect: Rect, kind: &str, field_name: &str, options: &[String])
                 -> Result<i32, String> {
    let field_name = field_name.trim().to_string();
    if field_name.is_empty() {
        return Err("Le champ doit avoir un nom.".into());
    }
    if names(pdf).contains(&field_name) {
        return Err(format!("Un champ s'appelle déjà « {field_name} »."));
    }
    if kind == "combo" && options.is_empty() {
        return Err("Une liste déroulante doit proposer au moins un choix.".into());
    }
    if rect.width() < 4.0 || rect.height() < 4.0 {
        return Err("Champ trop petit : tracez son cadre sur la page.".into());
    }
    let label = NEW.iter().find(|n| n.0 == kind).ok_or("Type de champ inconnu.")?.2;
    let mut xref = 0;
    pdf.edit(label, |doc| {
        let form = acroform(doc)?;
        let p = doc.load_pdf_page(page as i32).map_err(err)?;
        let inv = p.ctm().map_err(err)?.invert().ok_or("transformation de page invalide")?;
        let r = mupdf::Rect::new(rect.x0, rect.y0, rect.x1, rect.y1).transform(&inv);
        let rotation = p.rotation().map_err(err)?;
        let mut page_obj = p.object();
        drop(p);

        let mut f = doc.new_dict().map_err(err)?;
        f.dict_put("Type", name("Annot")?).map_err(err)?;
        f.dict_put("Subtype", name("Widget")?).map_err(err)?;
        f.dict_put("FT", name(match kind { "text" => "Tx", "check" => "Btn", "combo" => "Ch", _ => "Sig" })?).map_err(err)?;
        f.dict_put("T", PdfObject::new_string(&field_name).map_err(err)?).map_err(err)?;
        f.dict_put("Rect", array(doc, &[r.x0, r.y0, r.x1, r.y1])?).map_err(err)?;
        f.dict_put("F", PdfObject::new_int(4).map_err(err)?).map_err(err)?; // imprimable
        f.dict_put("P", page_obj.clone()).map_err(err)?;
        f.dict_put("DA", PdfObject::new_string(if kind == "check" { "/ZaDb 0 Tf 0 g" } else { "/Helv 0 Tf 0 g" })
            .map_err(err)?).map_err(err)?;
        let mut mk = doc.new_dict().map_err(err)?;
        mk.dict_put("BC", array(doc, &[0.55, 0.55, 0.55])?).map_err(err)?;
        if rotation != 0 {
            mk.dict_put("R", PdfObject::new_int(rotation).map_err(err)?).map_err(err)?;
        }
        if kind == "check" {
            mk.dict_put("CA", PdfObject::new_string("4").map_err(err)?).map_err(err)?; // coche
        }
        f.dict_put("MK", mk).map_err(err)?;
        let mut bs = doc.new_dict().map_err(err)?;
        bs.dict_put("W", PdfObject::new_int(1).map_err(err)?).map_err(err)?;
        f.dict_put("BS", bs).map_err(err)?;
        match kind {
            "check" => {
                f.dict_put("V", name("Off")?).map_err(err)?;
                f.dict_put("AS", name("Off")?).map_err(err)?;
            }
            "combo" => {
                f.dict_put("Ff", PdfObject::new_int(FieldFlags::COMBO.bits()).map_err(err)?).map_err(err)?;
                let mut opt = doc.new_array().map_err(err)?;
                for o in options {
                    opt.array_push(PdfObject::new_string(o).map_err(err)?).map_err(err)?;
                }
                f.dict_put("Opt", opt).map_err(err)?;
                f.dict_put("V", PdfObject::new_string(&options[0]).map_err(err)?).map_err(err)?;
            }
            _ => {}
        }
        let f = doc.add_object(&f).map_err(err)?;
        xref = f.as_indirect().map_err(err)?;
        match page_obj.get_dict("Annots").map_err(err)? {
            Some(mut annots) => annots.array_push(f.clone()).map_err(err)?,
            None => {
                let mut annots = doc.new_array().map_err(err)?;
                annots.array_push(f.clone()).map_err(err)?;
                page_obj.dict_put("Annots", annots).map_err(err)?;
            }
        }
        let mut list = form.get_dict("Fields").map_err(err)?.ok_or("formulaire sans /Fields")?;
        list.array_push(f).map_err(err)?;

        // Apparence dessinée par MuPDF (cadre, coche, texte).
        let p = doc.load_pdf_page(page as i32).map_err(err)?;
        let mut w = find(&p, xref)?;
        if kind == "check" {
            w.set_value(doc, "Off", true).map_err(err)?;
        }
        w.update().map_err(err)?;
        Ok(())
    })?;
    Ok(xref)
}

pub fn delete_field(pdf: &mut Pdf, page: usize, xref: i32) -> Result<(), String> {
    pdf.edit("Supprimer le champ", |doc| {
        let mut p = doc.load_pdf_page(page as i32).map_err(err)?;
        let w = find(&p, xref)?;
        if kind_of(&w) == "signature" && w.is_signed().unwrap_or(false) {
            return Err("Une signature apposée ne se supprime pas : elle atteste le contenu du document.".into());
        }
        p.delete_widget(w).map_err(err)?;
        // Aussi retiré de la liste des champs : sinon il resterait « fantôme » ailleurs.
        if let Some(mut list) = doc.catalog().map_err(err)?.get_dict("AcroForm").map_err(err)?
            .and_then(|f| f.get_dict("Fields").ok().flatten())
        {
            for i in (0..list.len().map_err(err)? as i32).rev() {
                if list.get_array(i).map_err(err)?.is_some_and(|o| o.as_indirect().ok() == Some(xref)) {
                    list.array_delete(i).map_err(err)?;
                }
            }
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(rotation: i32) -> Pdf {
        let mut d = PdfDocument::new();
        let mut p = d.new_page(mupdf::Size::new(595.0, 842.0)).unwrap();
        p.set_rotation(rotation).unwrap();
        let mut out = vec![];
        d.write_to(&mut out).unwrap();
        Pdf::open(&out).unwrap()
    }

    fn field<'a>(all: &'a [FieldInfo], name: &str) -> &'a FieldInfo {
        all.iter().find(|f| f.name == name).unwrap()
    }

    #[test]
    fn creer_remplir_supprimer() {
        for rot in [0, 90] {
            let mut pdf = doc(rot);
            assert!(fields(&pdf).is_empty());
            assert_eq!(unique_name(&pdf, "text"), "Texte1");
            add_field(&mut pdf, 0, Rect::new(100.0, 100.0, 300.0, 124.0), "text", "Nom", &[]).unwrap();
            add_field(&mut pdf, 0, Rect::new(100.0, 150.0, 116.0, 166.0), "check", "Accord", &[]).unwrap();
            let opts = vec!["Rouge".to_string(), "Vert".to_string()];
            add_field(&mut pdf, 0, Rect::new(100.0, 200.0, 250.0, 222.0), "combo", "Couleur", &opts).unwrap();
            add_field(&mut pdf, 0, Rect::new(100.0, 300.0, 300.0, 360.0), "signature", "Signature1", &[]).unwrap();
            assert!(add_field(&mut pdf, 0, Rect::new(0.0, 0.0, 50.0, 20.0), "text", "Nom", &[]).is_err(), "nom en double");

            let all = fields(&pdf);
            let kinds: Vec<_> = all.iter().map(|f| f.kind).collect();
            assert_eq!(kinds, ["text", "check", "combo", "signature"], "rot {rot}");
            let nom = field(&all, "Nom");
            assert!(nom.rect.contains(200.0, 112.0), "rot {rot} : {:?}", nom.rect);
            assert_eq!(field(&all, "Couleur").choices, opts);
            assert!(!field(&all, "Accord").checked());

            set_value(&mut pdf, nom, "Jean Dupré").unwrap();
            set_value(&mut pdf, field(&all, "Accord"), "Yes").unwrap();
            set_value(&mut pdf, field(&all, "Couleur"), "Vert").unwrap();
            assert!(set_value(&mut pdf, field(&all, "Signature1"), "x").is_err());
            let all = fields(&pdf);
            assert_eq!(field(&all, "Nom").value, "Jean Dupré");
            assert!(field(&all, "Accord").checked());
            assert_eq!(field(&all, "Couleur").value, "Vert");

            // Enregistré puis rouvert : tout est là.
            let reopened = Pdf::open(&pdf.bytes_for_disk().unwrap()).unwrap();
            let again = fields(&reopened);
            assert_eq!(field(&again, "Nom").value, "Jean Dupré");
            assert!(field(&again, "Accord").checked());

            let xref = field(&all, "Couleur").xref;
            delete_field(&mut pdf, 0, xref).unwrap();
            assert_eq!(fields(&pdf).len(), 3);
            assert!(!names(&pdf).contains(&"Couleur".to_string()));
            pdf.undo().unwrap();
            assert_eq!(fields(&pdf).len(), 4);
        }
    }

    #[test]
    fn decocher() {
        let mut pdf = doc(0);
        add_field(&mut pdf, 0, Rect::new(10.0, 10.0, 26.0, 26.0), "check", "C", &[]).unwrap();
        let f = fields(&pdf).remove(0);
        set_value(&mut pdf, &f, "Yes").unwrap();
        let f = fields(&pdf).remove(0);
        assert!(f.checked());
        set_value(&mut pdf, &f, "Off").unwrap();
        assert!(!fields(&pdf)[0].checked());
    }
}

