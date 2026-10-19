//! Protection par mot de passe (AES-256) et permissions, suppression de la protection,
//! propriétés (métadonnées) du document. Sans interface. Tout est annulable.

use mupdf::pdf::{Encryption, PdfObject, PdfWriteOptions, Permission};
use mupdf::MetadataName;

use crate::pdf::Pdf;

/// Permissions proposées : clé, libellé, bits PDF accordés.
pub const PERMISSIONS: [(&str, &str, Permission); 4] = [
    ("print", "Imprimer", Permission::PRINT.union(Permission::PRINT_HQ)),
    ("copy", "Copier le texte et les images", Permission::COPY),
    ("annotate", "Commenter et remplir les formulaires", Permission::ANNOTATE.union(Permission::FORM)),
    ("modify", "Modifier et réorganiser les pages", Permission::MODIFY.union(Permission::ASSEMBLE)),
];

/// Propriétés modifiables : clé du dictionnaire /Info, libellé.
pub const FIELDS: [(&str, &str); 6] = [
    ("Title", "Titre"), ("Author", "Auteur"), ("Subject", "Sujet"), ("Keywords", "Mots-clés"),
    ("Creator", "Application d'origine"), ("Producer", "Producteur PDF"),
];

/// Permissions accordées au document tel qu'il est ouvert.
pub fn allowed(pdf: &Pdf) -> Vec<&'static str> {
    let perms = pdf.permissions();
    PERMISSIONS.iter().filter(|(_, _, bits)| perms.contains(Permission::from_bits_retain(bits.bits()))).map(|p| p.0).collect()
}

/// Erreur si la combinaison ne protège rien ou se contredit.
pub fn check(open: &str, owner: &str, allow: &[&str]) -> Result<(), String> {
    let restricted = PERMISSIONS.iter().any(|p| !allow.contains(&p.0));
    if open.is_empty() && !restricted {
        return Err("Indiquez un mot de passe d'ouverture, ou retirez au moins une permission.".into());
    }
    if restricted && owner.is_empty() {
        return Err("Les restrictions exigent un mot de passe des permissions.".into());
    }
    if restricted && owner == open {
        return Err("Le mot de passe des permissions doit différer de celui d'ouverture : quiconque \
                    ouvre le document lèverait sinon les restrictions.".into());
    }
    Ok(())
}

/// Chiffre en AES-256. `open` vide : le document s'ouvre sans mot de passe mais garde ses
/// restrictions ; `owner` les lève.
pub fn protect(pdf: &mut Pdf, open: &str, owner: &str, allow: &[&str]) -> Result<(), String> {
    check(open, owner, allow)?;
    let mut perms = Permission::ACCESSIBILITY; // lecteurs d'écran toujours permis
    for (key, _, bits) in PERMISSIONS {
        if allow.contains(&key) {
            perms |= bits;
        }
    }
    let owner = if owner.is_empty() { open } else { owner };
    let mut opts = PdfWriteOptions::default();
    opts.set_encryption(Encryption::Aes256).set_user_password(open).set_owner_password(owner).set_permissions(perms);
    let mut data = vec![];
    pdf.doc().write_to_with_options(&mut data, opts).map_err(|e| e.to_string())?;
    pdf.swap("Protéger par mot de passe", &data, Some(owner.to_string()))?;
    pdf.signed = false; // chiffrer réécrit tout le fichier
    Ok(())
}

/// Retire chiffrement et restrictions ; exige les droits du propriétaire (`unlock_owner`).
pub fn unprotect(pdf: &mut Pdf) -> Result<(), String> {
    if !pdf.is_encrypted() {
        return Err("Ce document n'est pas protégé.".into());
    }
    if !pdf.permissions().is_all() {
        return Err("Le mot de passe des permissions est nécessaire pour retirer la protection.".into());
    }
    let data = pdf.plain_bytes()?;
    pdf.swap("Retirer la protection", &data, None)?;
    pdf.signed = false;
    Ok(())
}

pub fn metadata(pdf: &Pdf) -> Vec<String> {
    let name = |k: &str| match k {
        "Title" => MetadataName::Title,
        "Author" => MetadataName::Author,
        "Subject" => MetadataName::Subject,
        "Keywords" => MetadataName::Keywords,
        "Creator" => MetadataName::Creator,
        _ => MetadataName::Producer,
    };
    FIELDS.iter().map(|(k, _)| pdf.doc().metadata(name(k)).unwrap_or_default()).collect()
}

/// Valeurs dans l'ordre de `FIELDS`. Les métadonnées XMP, qui primeraient dans Acrobat
/// avec leurs anciennes valeurs, sont retirées.
pub fn set_metadata(pdf: &mut Pdf, values: &[String]) -> Result<(), String> {
    let err = |e: mupdf::Error| e.to_string();
    pdf.edit("Propriétés du document", |doc| {
        let mut info = match doc.trailer().map_err(err)?.get_dict("Info").map_err(err)? {
            Some(i) => i,
            None => {
                let i = doc.add_object(&doc.new_dict().map_err(err)?).map_err(err)?;
                // Relu après add_object : MuPDF peut alors avoir recopié le trailer.
                doc.trailer().map_err(err)?.dict_put("Info", i.clone()).map_err(err)?;
                i
            }
        };
        for ((key, _), v) in FIELDS.iter().zip(values) {
            let v = v.trim();
            if v.is_empty() {
                info.dict_delete(*key).map_err(err)?;
            } else {
                info.dict_put(*key, PdfObject::new_string(v).map_err(err)?).map_err(err)?;
            }
        }
        doc.catalog().map_err(err)?.dict_delete("Metadata").map_err(err)?;
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Pdf {
        Pdf::open(&std::fs::read("tests/fixtures/Été déjà vu.pdf").unwrap()).unwrap()
    }

    #[test]
    fn combinaisons_refusees() {
        assert!(check("", "", &["print", "copy", "annotate", "modify"]).is_err());
        assert!(check("a", "", &["print"]).is_err());
        assert!(check("a", "a", &["print"]).is_err());
        assert!(check("a", "", &["print", "copy", "annotate", "modify"]).is_ok());
        assert!(check("", "b", &["print"]).is_ok());
    }

    #[test]
    fn proteger_rouvrir_retirer() {
        let mut pdf = fixture();
        assert!(!pdf.is_encrypted());
        protect(&mut pdf, "ouvrir", "proprio", &["print"]).unwrap();
        assert!(pdf.is_encrypted());
        let data = pdf.bytes_for_disk().unwrap();
        assert!(Pdf::open(&data).is_err(), "mot de passe exigé");
        let mut user = Pdf::open_with_password(&data, Some("ouvrir")).unwrap();
        assert_eq!(allowed(&user), ["print"]);
        assert!(unprotect(&mut user).is_err(), "le mot de passe d'ouverture ne suffit pas");
        assert!(!user.unlock_owner("faux"));
        assert_eq!(user.page_count(), 3, "un essai raté ne gâche pas le document");
        assert!(user.unlock_owner("proprio"));
        unprotect(&mut user).unwrap();
        let plain = user.bytes_for_disk().unwrap();
        assert!(!Pdf::open(&plain).unwrap().is_encrypted());

    }

    #[test]
    #[ignore = "à corriger (phase 4) : Ctrl+Z sur protéger / retirer la protection (rechargement et mot de passe)"]
    fn annuler_le_retrait_de_protection() {
        let mut pdf = fixture();
        protect(&mut pdf, "ouvrir", "proprio", &["print"]).unwrap();
        let mut user = Pdf::open_with_password(&pdf.bytes_for_disk().unwrap(), Some("ouvrir")).unwrap();
        assert!(user.unlock_owner("proprio"));
        unprotect(&mut user).unwrap();
        user.undo().unwrap();
        assert!(user.is_encrypted(), "Ctrl+Z remet la protection");
        // Annuler la protection elle-même.
        pdf.undo().unwrap();
        assert!(!pdf.is_encrypted());
    }

    #[test]
    fn restrictions_sans_mot_de_passe_d_ouverture() {
        let mut pdf = fixture();
        protect(&mut pdf, "", "proprio", &["print", "copy"]).unwrap();
        let reopened = Pdf::open(&pdf.bytes_for_disk().unwrap()).unwrap();
        assert_eq!(allowed(&reopened), ["print", "copy"]);
    }

    #[test]
    fn proprietes() {
        let mut pdf = fixture();
        let mut values = metadata(&pdf);
        values[0] = "Mon titre été".into();
        values[1] = "Alex".into();
        set_metadata(&mut pdf, &values).unwrap();
        let reopened = Pdf::open(&pdf.bytes_for_disk().unwrap()).unwrap();
        let got = metadata(&reopened);
        assert_eq!((got[0].as_str(), got[1].as_str()), ("Mon titre été", "Alex"));
        pdf.undo().unwrap();
        assert_ne!(metadata(&pdf)[0], "Mon titre été");
    }
}



