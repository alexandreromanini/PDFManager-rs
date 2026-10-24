//! Signatures numériques PAdES (CMS détaché, ETSI.CAdES.detached) avec un certificat
//! .pfx / .p12, horodatage RFC 3161, vérification. Sans interface. Équivalent de
//! core/signatures.py (pyHanko), écrit avec les bibliothèques RustCrypto.
//!
//! La signature s'ajoute au fichier par mise à jour incrémentale : les octets déjà signés
//! ne changent pas, ce qui préserve les signatures précédentes.

use std::path::Path;

use cms::cert::{CertificateChoices, IssuerAndSerialNumber};
use cms::content_info::{CmsVersion, ContentInfo};
use cms::signed_data::{
    CertificateSet, EncapsulatedContentInfo, SignedData, SignerIdentifier, SignerInfo, SignerInfos,
};
use der::asn1::{ObjectIdentifier as Oid, OctetString, SetOfVec};
use der::{Any, Decode, Encode};
use sha2::{Digest, Sha256, Sha384, Sha512};
use x509_cert::attr::Attribute;
use x509_cert::spki::AlgorithmIdentifierOwned;
use x509_cert::Certificate;

use crate::forms;
use crate::pdf::{Pdf, Rect};

pub const DEFAULT_TSA: &str = "http://timestamp.digicert.com";
/// Place réservée à la signature (CMS en hexadécimal) : certificats et horodatage compris.
const RESERVED: usize = 24_000;

const OID_DATA: Oid = Oid::new_unwrap("1.2.840.113549.1.7.1");
const OID_SIGNED_DATA: Oid = Oid::new_unwrap("1.2.840.113549.1.7.2");
const OID_CONTENT_TYPE: Oid = Oid::new_unwrap("1.2.840.113549.1.9.3");
const OID_MESSAGE_DIGEST: Oid = Oid::new_unwrap("1.2.840.113549.1.9.4");
const OID_SIGNING_TIME: Oid = Oid::new_unwrap("1.2.840.113549.1.9.5");
const OID_SIGNING_CERT_V2: Oid = Oid::new_unwrap("1.2.840.113549.1.9.16.2.47");
const OID_TIMESTAMP_TOKEN: Oid = Oid::new_unwrap("1.2.840.113549.1.9.16.2.14");
const OID_TST_INFO: Oid = Oid::new_unwrap("1.2.840.113549.1.9.16.1.4");
const OID_SHA1: Oid = Oid::new_unwrap("1.3.14.3.2.26");
const OID_SHA256: Oid = Oid::new_unwrap("2.16.840.1.101.3.4.2.1");
const OID_SHA384: Oid = Oid::new_unwrap("2.16.840.1.101.3.4.2.2");
const OID_SHA512: Oid = Oid::new_unwrap("2.16.840.1.101.3.4.2.3");
const OID_RSA: Oid = Oid::new_unwrap("1.2.840.113549.1.1.1");
const OID_RSA_SHA1: Oid = Oid::new_unwrap("1.2.840.113549.1.1.5");
const OID_RSA_SHA256: Oid = Oid::new_unwrap("1.2.840.113549.1.1.11");
const OID_RSA_SHA384: Oid = Oid::new_unwrap("1.2.840.113549.1.1.12");
const OID_RSA_SHA512: Oid = Oid::new_unwrap("1.2.840.113549.1.1.13");
const OID_EC_PUBLIC_KEY: Oid = Oid::new_unwrap("1.2.840.10045.2.1");
const OID_ECDSA_SHA256: Oid = Oid::new_unwrap("1.2.840.10045.4.3.2");
const OID_ECDSA_SHA384: Oid = Oid::new_unwrap("1.2.840.10045.4.3.3");
const OID_ECDSA_SHA512: Oid = Oid::new_unwrap("1.2.840.10045.4.3.4");
const OID_P256: Oid = Oid::new_unwrap("1.2.840.10045.3.1.7");
const OID_P384: Oid = Oid::new_unwrap("1.3.132.0.34");
const OID_CN: Oid = Oid::new_unwrap("2.5.4.3");

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

// ----- empreintes ----------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
enum Hash {
    Sha1,
    Sha256,
    Sha384,
    Sha512,
}

impl Hash {
    fn from_oid(oid: &Oid) -> Option<Hash> {
        Some(match *oid {
            OID_SHA1 => Hash::Sha1,
            OID_SHA256 => Hash::Sha256,
            OID_SHA384 => Hash::Sha384,
            OID_SHA512 => Hash::Sha512,
            _ => return None,
        })
    }

    fn oid(self) -> Oid {
        match self {
            Hash::Sha1 => OID_SHA1,
            Hash::Sha256 => OID_SHA256,
            Hash::Sha384 => OID_SHA384,
            Hash::Sha512 => OID_SHA512,
        }
    }

    fn digest(self, parts: &[&[u8]]) -> Vec<u8> {
        fn run<D: Digest>(parts: &[&[u8]]) -> Vec<u8> {
            let mut d = D::new();
            for p in parts {
                d.update(p);
            }
            d.finalize().to_vec()
        }
        match self {
            Hash::Sha1 => run::<sha1::Sha1>(parts),
            Hash::Sha256 => run::<Sha256>(parts),
            Hash::Sha384 => run::<Sha384>(parts),
            Hash::Sha512 => run::<Sha512>(parts),
        }
    }

    fn algorithm(self) -> AlgorithmIdentifierOwned {
        AlgorithmIdentifierOwned { oid: self.oid(), parameters: None }
    }
}

// ----- clés et vérification de signatures ----------------------------------------------------

enum Key {
    Rsa(rsa::RsaPrivateKey),
    P256(p256::ecdsa::SigningKey),
    P384(p384::ecdsa::SigningKey),
}

/// Certificat et clé chargés d'un fichier .pfx / .p12.
pub struct Signer {
    key: Key,
    pub cert: Certificate,
    chain: Vec<Certificate>,
}

impl Signer {
    pub fn name(&self) -> String {
        cert_name(&self.cert)
    }

    fn hash(&self) -> Hash {
        if matches!(self.key, Key::P384(_)) { Hash::Sha384 } else { Hash::Sha256 }
    }

    /// (algorithme de signature, signature) de `data`.
    fn sign(&self, data: &[u8]) -> Result<(AlgorithmIdentifierOwned, Vec<u8>), String> {
        use signature::{SignatureEncoding, Signer as _};
        Ok(match &self.key {
            Key::Rsa(k) => {
                let s = rsa::pkcs1v15::SigningKey::<Sha256>::new(k.clone()).try_sign(data).map_err(err)?;
                (AlgorithmIdentifierOwned { oid: OID_RSA, parameters: Some(Any::null()) }, s.to_vec())
            }
            Key::P256(k) => {
                let s: p256::ecdsa::Signature = k.try_sign(data).map_err(err)?;
                (AlgorithmIdentifierOwned { oid: OID_ECDSA_SHA256, parameters: None }, s.to_der().to_vec())
            }
            Key::P384(k) => {
                let s: p384::ecdsa::Signature = k.try_sign(data).map_err(err)?;
                (AlgorithmIdentifierOwned { oid: OID_ECDSA_SHA384, parameters: None }, s.to_der().to_vec())
            }
        })
    }
}

/// Nom lisible d'un certificat : son « common name », sinon le sujet entier.
pub fn cert_name(cert: &Certificate) -> String {
    let subject = &cert.tbs_certificate.subject;
    for rdn in subject.0.iter() {
        for atv in rdn.0.iter() {
            if atv.oid == OID_CN {
                if let Ok(s) = der::asn1::Utf8StringRef::try_from(&atv.value) {
                    return s.as_str().to_string();
                }
                if let Ok(s) = der::asn1::PrintableStringRef::try_from(&atv.value) {
                    return s.as_str().to_string();
                }
            }
        }
    }
    subject.to_string()
}

pub fn load_signer(path: &Path, password: &str) -> Result<Signer, String> {
    let data = std::fs::read(path).map_err(|e| format!("Certificat illisible : {e}"))?;
    load_signer_data(&data, password)
}

pub fn load_signer_data(data: &[u8], password: &str) -> Result<Signer, String> {
    let bad = "Mot de passe incorrect, ou fichier .pfx / .p12 invalide.";
    let store = p12_keystore::KeyStore::from_pkcs12(data, password).map_err(|_| bad.to_string())?;
    let (_, chain) = store.private_key_chain().ok_or("Ce fichier ne contient pas de clé privée.")?;
    let certs: Vec<Certificate> = chain.chain().iter().filter_map(|c| Certificate::from_der(c.as_der()).ok()).collect();
    let cert = certs.first().cloned().ok_or("Ce fichier ne contient pas de certificat.")?;
    use pkcs8::DecodePrivateKey;
    let pk = chain.key();
    let key = if let Ok(k) = rsa::RsaPrivateKey::from_pkcs8_der(pk) {
        Key::Rsa(k)
    } else if let Ok(k) = p256::ecdsa::SigningKey::from_pkcs8_der(pk) {
        Key::P256(k)
    } else if let Ok(k) = p384::ecdsa::SigningKey::from_pkcs8_der(pk) {
        Key::P384(k)
    } else {
        return Err("Type de clé non pris en charge (RSA, ECDSA P-256 et P-384 le sont).".into());
    };
    Ok(Signer { key, chain: certs[1..].to_vec(), cert })
}

/// Vérifie `sig` sur `data` avec la clé publique du certificat.
fn verify_with(cert: &Certificate, alg: &AlgorithmIdentifierOwned, hash: Option<Hash>, data: &[u8], sig: &[u8]) -> bool {
    let spki = &cert.tbs_certificate.subject_public_key_info;
    let key_der = spki.subject_public_key.raw_bytes();
    let hash = match alg.oid {
        OID_RSA_SHA1 => Some(Hash::Sha1),
        OID_RSA_SHA256 | OID_ECDSA_SHA256 => Some(Hash::Sha256),
        OID_RSA_SHA384 | OID_ECDSA_SHA384 => Some(Hash::Sha384),
        OID_RSA_SHA512 | OID_ECDSA_SHA512 => Some(Hash::Sha512),
        _ => hash,
    };
    let Some(hash) = hash else { return false };
    let digest = hash.digest(&[data]);
    if spki.algorithm.oid == OID_RSA {
        use rsa::pkcs1::DecodeRsaPublicKey;
        let Ok(key) = rsa::RsaPublicKey::from_pkcs1_der(key_der) else { return false };
        let scheme = match hash {
            Hash::Sha1 => rsa::Pkcs1v15Sign::new::<sha1::Sha1>(),
            Hash::Sha256 => rsa::Pkcs1v15Sign::new::<Sha256>(),
            Hash::Sha384 => rsa::Pkcs1v15Sign::new::<Sha384>(),
            Hash::Sha512 => rsa::Pkcs1v15Sign::new::<Sha512>(),
        };
        return key.verify(scheme, &digest, sig).is_ok();
    }
    if spki.algorithm.oid == OID_EC_PUBLIC_KEY {
        use ecdsa::signature::hazmat::PrehashVerifier;
        let curve = spki.algorithm.parameters.as_ref().and_then(|p| p.decode_as::<Oid>().ok());
        return match curve {
            Some(OID_P256) => p256::ecdsa::VerifyingKey::from_sec1_bytes(key_der).ok()
                .zip(p256::ecdsa::Signature::from_der(sig).ok())
                .is_some_and(|(k, s)| k.verify_prehash(&digest, &s).is_ok()),
            Some(OID_P384) => p384::ecdsa::VerifyingKey::from_sec1_bytes(key_der).ok()
                .zip(p384::ecdsa::Signature::from_der(sig).ok())
                .is_some_and(|(k, s)| k.verify_prehash(&digest, &s).is_ok()),
            _ => false,
        };
    }
    false
}

/// Le certificat `issuer` a-t-il signé `cert` ?
fn issued_by(cert: &Certificate, issuer: &Certificate) -> bool {
    cert.tbs_certificate.issuer == issuer.tbs_certificate.subject
        && cert.tbs_certificate.to_der().is_ok_and(|tbs| {
            verify_with(issuer, &cert.signature_algorithm, None, &tbs, cert.signature.raw_bytes())
        })
}

/// Chaîne jusqu'à une autorité de confiance (`roots`), par les certificats fournis.
fn trusted(cert: &Certificate, pool: &[Certificate], roots: &[Certificate]) -> bool {
    let mut current = cert.clone();
    for _ in 0..8 {
        if roots.iter().any(|r| r == &current || issued_by(&current, r)) {
            return true;
        }
        match pool.iter().find(|c| *c != &current && issued_by(&current, c)) {
            Some(next) => current = next.clone(),
            None => return false,
        }
    }
    false
}

/// Autorités racines du système (magasin de Windows, ou du système sous Linux).
pub fn system_roots() -> Vec<Certificate> {
    rustls_native_certs::load_native_certs().certs.iter().filter_map(|c| Certificate::from_der(c.as_ref()).ok()).collect()
}

// ----- CMS : construire et lire ----------------------------------------------------------------

fn attribute(oid: Oid, value: Any) -> Result<Attribute, String> {
    let mut values = SetOfVec::new();
    values.insert(value).map_err(err)?;
    Ok(Attribute { oid, values })
}

/// Encodage DER minimal (attribut ESS, non fourni par les bibliothèques).
fn tlv(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    let n = content.len();
    if n < 128 {
        out.push(n as u8);
    } else {
        let bytes: Vec<u8> = n.to_be_bytes().into_iter().skip_while(|b| *b == 0).collect();
        out.push(0x80 | bytes.len() as u8);
        out.extend(bytes);
    }
    out.extend_from_slice(content);
    out
}

/// signingCertificateV2 (ESS, exigé par PAdES) : empreinte SHA-256 du certificat.
fn signing_certificate_v2(cert: &Certificate) -> Result<Any, String> {
    let hash = Sha256::digest(cert.to_der().map_err(err)?);
    let id = tlv(0x30, &tlv(0x04, &hash));
    let value = tlv(0x30, &tlv(0x30, &id));
    Any::from_der(&value).map_err(err)
}

fn issuer_and_serial(cert: &Certificate) -> SignerIdentifier {
    SignerIdentifier::IssuerAndSerialNumber(IssuerAndSerialNumber {
        issuer: cert.tbs_certificate.issuer.clone(),
        serial_number: cert.tbs_certificate.serial_number.clone(),
    })
}

/// CMS SignedData détaché pour l'empreinte `digest` du document.
fn build_cms(signer: &Signer, digest: &[u8], tsa: Option<&str>) -> Result<Vec<u8>, String> {
    let hash = signer.hash();
    let mut attrs = SetOfVec::new();
    attrs.insert(attribute(OID_CONTENT_TYPE, Any::encode_from(&OID_DATA).map_err(err)?)?).map_err(err)?;
    attrs.insert(attribute(OID_MESSAGE_DIGEST, Any::encode_from(&OctetString::new(digest).map_err(err)?).map_err(err)?)?)
        .map_err(err)?;
    attrs.insert(attribute(OID_SIGNING_CERT_V2, signing_certificate_v2(&signer.cert)?)?).map_err(err)?;
    // Signée : le DER du SET d'attributs (balise SET, pas la balise implicite [0]).
    let (alg, signature) = signer.sign(&attrs.to_der().map_err(err)?)?;
    let mut info = SignerInfo {
        version: CmsVersion::V1,
        sid: issuer_and_serial(&signer.cert),
        digest_alg: hash.algorithm(),
        signed_attrs: Some(attrs),
        signature_algorithm: alg,
        signature: OctetString::new(signature.clone()).map_err(err)?,
        unsigned_attrs: None,
    };
    if let Some(url) = tsa {
        let token = timestamp(url, &Sha256::digest(&signature))?;
        let mut unsigned = SetOfVec::new();
        unsigned.insert(attribute(OID_TIMESTAMP_TOKEN, Any::from_der(&token).map_err(err)?)?).map_err(err)?;
        info.unsigned_attrs = Some(unsigned);
    }
    let mut certs = SetOfVec::new();
    for c in std::iter::once(&signer.cert).chain(&signer.chain) {
        certs.insert(CertificateChoices::Certificate(c.clone())).map_err(err)?;
    }
    let mut digests = SetOfVec::new();
    digests.insert(hash.algorithm()).map_err(err)?;
    let mut infos = SetOfVec::new();
    infos.insert(info).map_err(err)?;
    let signed = SignedData {
        version: CmsVersion::V1,
        digest_algorithms: digests,
        encap_content_info: EncapsulatedContentInfo { econtent_type: OID_DATA, econtent: None },
        certificates: Some(CertificateSet(certs)),
        crls: None,
        signer_infos: SignerInfos(infos),
    };
    let ci = ContentInfo { content_type: OID_SIGNED_DATA, content: Any::encode_from(&signed).map_err(err)? };
    ci.to_der().map_err(err)
}

/// Jeton d'horodatage (RFC 3161) pour l'empreinte SHA-256 de la signature.
fn timestamp(url: &str, imprint: &[u8]) -> Result<Vec<u8>, String> {
    use x509_tsp::{MessageImprint, TimeStampReq, TimeStampResp, TspVersion};
    let nonce: u64 = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64);
    let req = TimeStampReq {
        version: TspVersion::V1,
        message_imprint: MessageImprint {
            hash_algorithm: x509_cert::spki::AlgorithmIdentifier { oid: OID_SHA256, parameters: None },
            hashed_message: OctetString::new(imprint).map_err(err)?,
        },
        req_policy: None,
        nonce: Some(der::asn1::Int::new(&nonce.to_be_bytes()).map_err(err)?),
        cert_req: true,
        extensions: None,
    };
    let body = req.to_der().map_err(err)?;
    let fail = |e: String| format!("Horodatage impossible ({url}) : {e}. Le serveur est-il joignable ?");
    let mut resp = ureq::post(url).header("Content-Type", "application/timestamp-query")
        .config().timeout_global(Some(std::time::Duration::from_secs(20))).build()
        .send(&body[..]).map_err(|e| fail(e.to_string()))?;
    let data = resp.body_mut().with_config().limit(1 << 20).read_to_vec().map_err(|e| fail(e.to_string()))?;
    let parsed = TimeStampResp::from_der(&data).map_err(|e| fail(e.to_string()))?;
    let status = parsed.status.status as u8;
    if status > 1 {
        return Err(fail(format!("refusé (statut {status})")));
    }
    parsed.time_stamp_token.ok_or_else(|| fail("pas de jeton".into()))?.to_der().map_err(err)
}

// ----- signer un PDF -----------------------------------------------------------------------------

/// Où signer : un champ de signature existant (vide), ou un nouveau cadre.
pub enum Place {
    Field(String),
    New(usize, Rect),
}

/// Apparence visible : lignes de texte, et image facultative (PNG).
pub struct Look {
    pub lines: Vec<String>,
    pub image: Option<Vec<u8>>,
}

pub fn appearance_lines(name: &str, when: chrono::DateTime<chrono::Local>, reason: &str, location: &str) -> Vec<String> {
    let mut lines = vec!["Signé numériquement par".to_string(), name.to_string(), format!("Date : {}", when.format("%d/%m/%Y %H:%M"))];
    if !reason.trim().is_empty() {
        lines.push(format!("Motif : {}", reason.trim()));
    }
    if !location.trim().is_empty() {
        lines.push(format!("Lieu : {}", location.trim()));
    }
    lines
}

fn pdf_date(t: chrono::DateTime<chrono::Local>) -> String {
    let off = t.offset().local_minus_utc() / 60;
    format!("D:{}{}{:02}'{:02}'", t.format("%Y%m%d%H%M%S"), if off < 0 { '-' } else { '+' }, off.abs() / 60, off.abs() % 60)
}

/// Signe le document (`data`, non chiffré) ; renvoie le PDF signé (data + mises à jour).
pub fn sign(data: &[u8], place: Place, signer: &Signer, look: &Look, reason: &str, location: &str, tsa: Option<&str>)
            -> Result<Vec<u8>, String> {
    let protected = "Un document protégé par mot de passe ne peut pas être signé ici : retirez d'abord la protection, \
                     puis signez.";
    let mut pdf = Pdf::open(data).map_err(|_| protected.to_string())?;
    if pdf.is_encrypted() {
        return Err(protected.into());
    }
    pdf.incremental = true;
    let name = match place {
        Place::Field(name) => name,
        Place::New(page, rect) => {
            let name = forms::unique_name(&pdf, "signature");
            forms::add_field(&mut pdf, page, rect, "signature", &name, &[])?;
            name
        }
    };
    let field = forms::fields(&pdf).into_iter().find(|f| f.name == name && f.kind == "signature")
        .ok_or_else(|| format!("Champ de signature « {name} » introuvable."))?;
    if field.signed {
        return Err(format!("Le champ « {name} » est déjà signé."));
    }
    let now = chrono::Local::now();
    let who = signer.name();
    pdf.edit("Signer", |doc| {
        let p = doc.load_pdf_page(field.page as i32).map_err(err)?;
        let rotation = p.rotation().map_err(err)?;
        let w = p.widgets().find(|w| w.xref().ok() == Some(field.xref)).ok_or("champ introuvable")?;
        let mut obj = w.annotation().object();
        // Taille du cadre dans le repère PDF (non tourné).
        let r = obj.get_dict("Rect").ok().flatten().ok_or("cadre du champ illisible")?;
        let n = |k: i32| r.get_array(k).ok().flatten().and_then(|v| v.as_float().ok()).unwrap_or(0.0);
        let (w_pt, h_pt) = ((n(2) - n(0)).abs(), (n(3) - n(1)).abs());
        drop(w);
        drop(p);
        let ap = appearance_stream(doc, (w_pt, h_pt), rotation, look)?;
        let mut sig = doc.new_object_from_str(&format!(
            "<</Type/Sig/Filter/Adobe.PPKLite/SubFilter/ETSI.CAdES.detached\
             /ByteRange[0 1000000000 1000000000 1000000000]/Contents<{}>>>", "0".repeat(2 * RESERVED))).map_err(err)?;
        let s = |t: &str| mupdf::pdf::PdfObject::new_string(t).map_err(err);
        sig.dict_put("M", s(&pdf_date(now))?).map_err(err)?;
        sig.dict_put("Name", s(&who)?).map_err(err)?;
        if !reason.trim().is_empty() {
            sig.dict_put("Reason", s(reason.trim())?).map_err(err)?;
        }
        if !location.trim().is_empty() {
            sig.dict_put("Location", s(location.trim())?).map_err(err)?;
        }
        let sig = doc.add_object(&sig).map_err(err)?;
        obj.dict_put("V", sig).map_err(err)?;
        obj.dict_put("F", mupdf::pdf::PdfObject::new_int(132).map_err(err)?).map_err(err)?; // imprimable, verrouillé
        let mut apd = doc.new_dict().map_err(err)?;
        apd.dict_put("N", ap).map_err(err)?;
        obj.dict_put("AP", apd).map_err(err)?;
        if let Some(mut form) = doc.catalog().map_err(err)?.get_dict("AcroForm").map_err(err)? {
            form.dict_put("SigFlags", mupdf::pdf::PdfObject::new_int(3).map_err(err)?).map_err(err)?;
        }
        Ok(())
    })?;
    let mut out = pdf.bytes()?;
    fill_signature(&mut out, signer, tsa)?;
    Ok(out)
}

/// Remplit la place réservée : plages signées (/ByteRange) puis CMS (/Contents).
fn fill_signature(out: &mut [u8], signer: &Signer, tsa: Option<&str>) -> Result<(), String> {
    let placeholder = b"[0 1000000000 1000000000 1000000000]";
    let zeros = format!("<{}>", "0".repeat(2 * RESERVED)).into_bytes();
    let br = out.windows(placeholder.len()).rposition(|w| w == placeholder)
        .ok_or("place de la signature introuvable (/ByteRange)")?;
    let start = out.windows(zeros.len()).rposition(|w| w == zeros.as_slice())
        .ok_or("place de la signature introuvable (/Contents)")?;
    let end = start + zeros.len();
    let ranges = format!("[0 {start} {end} {}]", out.len() - end);
    if ranges.len() > placeholder.len() {
        return Err("document trop volumineux pour la signature".into());
    }
    let padded = format!("{ranges:<width$}", width = placeholder.len());
    out[br..br + placeholder.len()].copy_from_slice(padded.as_bytes());
    let digest = signer.hash().digest(&[&out[..start], &out[end..]]);
    let cms = build_cms(signer, &digest, tsa)?;
    let hex: String = cms.iter().map(|b| format!("{b:02X}")).collect();
    if hex.len() > 2 * RESERVED {
        return Err("signature trop volumineuse (chaîne de certificats très longue)".into());
    }
    out[start + 1..start + 1 + hex.len()].copy_from_slice(hex.as_bytes());
    Ok(())
}

/// Apparence (Form XObject) de la signature visible : image à gauche (ou en haut si le
/// cadre est étroit), texte à côté, droite à l'écran sur une page tournée.
fn appearance_stream(doc: &mut mupdf::pdf::PdfDocument, (w, h): (f32, f32), rotation: i32, look: &Look)
                     -> Result<mupdf::pdf::PdfObject, String> {
    // Dimensions à l'écran (page tournée d'un quart de tour : largeur et hauteur échangées).
    let turned = rotation.rem_euclid(180) == 90;
    let (sw, sh) = if turned { (h, w) } else { (w, h) };
    // Octets : le texte est en WinAnsi (é = 0xE9), pas en UTF-8.
    let mut ops: Vec<u8> = vec![];
    let mut resources = doc.new_dict().map_err(err)?;
    let (mut tx, ty, mut tw, mut th) = (3.0_f32, 3.0_f32, sw - 6.0, sh - 6.0);
    if let Some(png) = &look.image {
        let img = mupdf::Image::from_bytes(png).map_err(|e| format!("Image illisible : {e}"))?;
        let (iw, ih) = (img.width() as f32, img.height() as f32);
        let wide = sw >= 1.6 * sh;
        let (bx, by, bw, bh) = if wide { (tx, ty, sw * 0.42 - 6.0, th) } else { (tx, ty + th * 0.45, tw, th * 0.55) };
        let k = (bw / iw).min(bh / ih);
        let (dw, dh) = (iw * k, ih * k);
        let (dx, dy) = (bx + (bw - dw) / 2.0, by + (bh - dh) / 2.0);
        ops.extend(format!("q {dw} 0 0 {dh} {dx} {dy} cm /Im0 Do Q\n").bytes());
        let mut xobj = doc.new_dict().map_err(err)?;
        xobj.dict_put("Im0", doc.add_image(&img).map_err(err)?).map_err(err)?;
        resources.dict_put("XObject", xobj).map_err(err)?;
        if wide {
            tx = sw * 0.42 + 4.0;
            tw = sw - tx - 3.0;
        } else {
            th = th * 0.45 - 2.0;
        }
    }
    // Texte : Helvetica, taille ajustée au cadre ; ligne 2 (le nom) en gras.
    let n = look.lines.len().max(1) as f32;
    let longest = look.lines.iter().map(|l| crate::annot::text_width(l, 1.0)).fold(0.0, f32::max).max(1.0);
    let size = (th / (n * 1.2)).min(tw / longest).clamp(3.0, 12.0);
    ops.extend(b"BT 0 g\n");
    for (k, line) in look.lines.iter().enumerate() {
        let y = ty + th - (k as f32 + 1.0) * size * 1.2 + size * 0.2;
        let font = if k == 1 { "/HeBo" } else { "/Helv" };
        ops.extend(format!("{font} {size} Tf 1 0 0 1 {tx} {y} Tm ").bytes());
        ops.extend(crate::ocr::literal(line));
        ops.extend(b" Tj\n");
    }
    ops.extend(b"ET\n");
    let mut fonts = doc.new_dict().map_err(err)?;
    for (key, base) in [("Helv", "Helvetica"), ("HeBo", "Helvetica-Bold")] {
        let f = doc.new_object_from_str(&format!("<</Type/Font/Subtype/Type1/BaseFont/{base}/Encoding/WinAnsiEncoding>>"))
            .map_err(err)?;
        fonts.dict_put(key, f).map_err(err)?;
    }
    resources.dict_put("Font", fonts).map_err(err)?;
    // Repère de l'écran → repère du cadre (non tourné).
    let matrix = match rotation.rem_euclid(360) {
        90 => format!("0 1 -1 0 {w} 0"),
        180 => format!("-1 0 0 -1 {w} {h}"),
        270 => format!("0 -1 1 0 0 {h}"),
        _ => "1 0 0 1 0 0".to_string(),
    };
    // BBox : repère du dessin (l'écran) ; Matrix le tourne dans le cadre du champ.
    let dict = doc.new_object_from_str(&format!("<</Type/XObject/Subtype/Form/BBox[0 0 {sw} {sh}]/Matrix[{matrix}]>>"))
        .map_err(err)?;
    let buf = mupdf::Buffer::from_bytes(&ops).map_err(err)?;
    let mut stream = doc.add_stream(&buf, Some(&dict), false).map_err(err)?;
    stream.dict_put("Resources", resources).map_err(err)?;
    Ok(stream)
}

// ----- vérifier ----------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct Status {
    pub field: String,
    pub signer: String,
    /// « jj/mm/aaaa hh:mm:ss » : heure de l'horodatage, sinon celle déclarée par le signataire.
    pub when: String,
    pub timestamped: bool,
    /// Contenu signé inchangé et signature mathématiquement valide.
    pub intact: bool,
    /// Certificat rattaché à une autorité de confiance.
    pub trusted: bool,
    /// Le fichier continue après la partie signée (mises à jour ajoutées depuis).
    pub modified: bool,
    pub reason: String,
    pub location: String,
    /// Taille du fichier au moment de la signature (version signée).
    pub revision_end: usize,
    /// Certificat (DER), pour « faire confiance à ce certificat ».
    pub certificate: Vec<u8>,
    pub error: String,
}

impl Status {
    /// "ok", "warning" ou "error".
    pub fn level(&self) -> &'static str {
        if !self.intact { "error" } else if self.modified || !self.trusted { "warning" } else { "ok" }
    }

    pub fn summary(&self) -> &'static str {
        if !self.intact {
            "Signature INVALIDE"
        } else if self.modified {
            "Valide, document complété depuis"
        } else if !self.trusted {
            "Valide, identité non vérifiée"
        } else {
            "Signature valide"
        }
    }

    pub fn details(&self) -> Vec<String> {
        let mut out = vec![format!("Signataire : {}", self.signer)];
        if !self.intact {
            out.push(if self.error.is_empty() { "Le contenu signé a été modifié depuis la signature.".into() } else { self.error.clone() });
        }
        if !self.when.is_empty() {
            out.push(format!("Date : {}{}", self.when,
                             if self.timestamped { " (horodatage certifié)" } else { " (horloge du signataire)" }));
        }
        if !self.reason.is_empty() {
            out.push(format!("Motif : {}", self.reason));
        }
        if !self.location.is_empty() {
            out.push(format!("Lieu : {}", self.location));
        }
        if self.intact {
            out.push(if self.modified {
                "Document : des mises à jour ont été ajoutées depuis (commentaires, formulaire, autres signatures… non \
                 analysées en détail) : contrôlez la version signée.".into()
            } else {
                "Document : aucune modification depuis la signature.".into()
            });
            out.push(if self.trusted { "Certificat : émis par une autorité de confiance.".into() }
                     else { "Certificat : non approuvé (auto-signé ou autorité inconnue).".into() });
        }
        out.push("Révocation non vérifiée (aucune requête réseau).".into());
        out
    }
}

/// « D:20261015103100+02'00' » → « 15/10/2026 10:31:00 ».
fn date_text(d: &str) -> String {
    let d = d.trim_start_matches("D:");
    if d.len() < 14 || !d.is_char_boundary(14) {
        return String::new();
    }
    format!("{}/{}/{} {}:{}:{}", &d[6..8], &d[4..6], &d[0..4], &d[8..10], &d[10..12], &d[12..14])
}

/// État de chaque signature de `data` (octets du fichier). `roots` : autorités de
/// confiance (système et certificats approuvés). Sans réseau : révocation non vérifiée.
pub fn verify(data: &[u8], password: Option<&str>, roots: &[Certificate]) -> Result<Vec<Status>, String> {
    let pdf = Pdf::open_with_password(data, password).map_err(|e| format!("{e:?}"))?;
    let mut out = vec![];
    for i in 0..pdf.page_count() {
        let Ok(page) = pdf.doc().load_pdf_page(i as i32) else { continue };
        for w in page.widgets() {
            if w.r#type().ok() != Some(mupdf::pdf::WidgetType::Signature) {
                continue;
            }
            let obj = w.annotation().object();
            let Some(v) = obj.get_dict_inheritable("V").ok().flatten() else { continue };
            let text = |k: &str| v.get_dict(k).ok().flatten().and_then(|o| o.as_string().ok()).unwrap_or_default();
            let ranges: Vec<usize> = v.get_dict("ByteRange").ok().flatten()
                .map(|a| (0..a.len().unwrap_or(0) as i32)
                    .filter_map(|k| a.get_array(k).ok().flatten()?.as_int().ok().map(|n| n.max(0) as usize)).collect())
                .unwrap_or_default();
            let contents = v.get_dict("Contents").ok().flatten().and_then(|o| o.as_bytes().ok().map(|b| b.to_vec())).unwrap_or_default();
            let mut st = Status {
                field: w.name().ok().flatten().unwrap_or_default(),
                signer: text("Name"),
                when: date_text(&text("M")),
                timestamped: false,
                intact: false,
                trusted: false,
                modified: false,
                reason: text("Reason"),
                location: text("Location"),
                revision_end: ranges.get(2).zip(ranges.get(3)).map_or(0, |(a, b)| a + b),
                certificate: vec![],
                error: String::new(),
            };
            if let Err(e) = check(data, &ranges, &contents, roots, &mut st) {
                st.error = e;
            }
            out.push(st);
        }
    }
    Ok(out)
}

fn certificates(sd: &SignedData) -> Vec<Certificate> {
    sd.certificates.iter().flat_map(|s| s.0.iter())
        .filter_map(|c| match c { CertificateChoices::Certificate(c) => Some(c.clone()), _ => None }).collect()
}

fn signer_cert<'a>(info: &SignerInfo, certs: &'a [Certificate]) -> Option<&'a Certificate> {
    match &info.sid {
        SignerIdentifier::IssuerAndSerialNumber(id) => certs.iter().find(|c| {
            c.tbs_certificate.issuer == id.issuer && c.tbs_certificate.serial_number == id.serial_number
        }),
        SignerIdentifier::SubjectKeyIdentifier(_) => certs.first(),
    }
}

/// L'empreinte annoncée (attribut messageDigest) et la signature des attributs.
fn signer_ok(info: &SignerInfo, cert: &Certificate, hash: Hash, content_digest: &[u8]) -> Result<(), String> {
    let attrs = info.signed_attrs.as_ref().ok_or("Signature sans attributs signés (non prise en charge).")?;
    let md = attrs.iter().find(|x| x.oid == OID_MESSAGE_DIGEST).and_then(|x| x.values.iter().next())
        .and_then(|v| v.decode_as::<OctetString>().ok()).ok_or("Empreinte du document absente de la signature.")?;
    if md.as_bytes() != content_digest {
        return Err("Le contenu signé a été modifié depuis la signature.".into());
    }
    if !verify_with(cert, &info.signature_algorithm, Some(hash), &attrs.to_der().map_err(err)?, info.signature.as_bytes()) {
        return Err("La signature ne correspond pas au certificat (falsifiée, ou algorithme non pris en charge).".into());
    }
    Ok(())
}

fn check(data: &[u8], ranges: &[usize], contents: &[u8], roots: &[Certificate], st: &mut Status) -> Result<(), String> {
    let [a, la, b, lb] = ranges[..] else { return Err("Plages signées (/ByteRange) illisibles.".into()) };
    if a != 0 || a + la > b || b + lb > data.len() {
        return Err("Plages signées (/ByteRange) incohérentes avec le fichier.".into());
    }
    // Après la partie signée : seulement des blancs, sinon des mises à jour ont suivi.
    st.modified = data[b + lb..].iter().any(|c| !c.is_ascii_whitespace());
    let mut reader = der::SliceReader::new(contents).map_err(err)?;
    let ci = ContentInfo::decode(&mut reader).map_err(|e| format!("Signature illisible (CMS) : {e}"))?;
    let sd: SignedData = ci.content.decode_as().map_err(|e| format!("Signature illisible (SignedData) : {e}"))?;
    let certs = certificates(&sd);
    let info = sd.signer_infos.0.iter().next().ok_or("Signature sans signataire.")?;
    let cert = signer_cert(info, &certs).ok_or("Certificat du signataire absent de la signature.")?;
    st.signer = cert_name(cert);
    st.certificate = cert.to_der().map_err(err)?;
    let hash = Hash::from_oid(&info.digest_alg.oid).ok_or("Algorithme d'empreinte non pris en charge.")?;
    signer_ok(info, cert, hash, &hash.digest(&[&data[a..a + la], &data[b..b + lb]]))?;
    st.intact = true;
    st.trusted = trusted(cert, &certs, roots);
    let attrs = info.signed_attrs.as_ref();
    if let Some(t) = attrs.and_then(|a| a.iter().find(|x| x.oid == OID_SIGNING_TIME)).and_then(|x| x.values.iter().next())
        .and_then(|v| v.to_der().ok()).and_then(|d| x509_cert::time::Time::from_der(&d).ok())
    {
        st.when = local(t.to_system_time());
    }
    // Horodatage : jeton valide pour cette signature, signé par l'autorité d'horodatage.
    let token = info.unsigned_attrs.iter().flat_map(|u| u.iter()).find(|x| x.oid == OID_TIMESTAMP_TOKEN)
        .and_then(|x| x.values.iter().next());
    if let Some(t) = token.and_then(|t| timestamp_time(t, info.signature.as_bytes())) {
        st.when = t;
        st.timestamped = true;
    }
    Ok(())
}

fn local(t: std::time::SystemTime) -> String {
    chrono::DateTime::<chrono::Local>::from(t).format("%d/%m/%Y %H:%M:%S").to_string()
}

/// Heure d'un jeton d'horodatage s'il couvre bien `signature` et que sa signature est valide.
fn timestamp_time(token: &Any, signature: &[u8]) -> Option<String> {
    let ci: ContentInfo = token.decode_as().ok()?;
    let sd: SignedData = ci.content.decode_as().ok()?;
    if sd.encap_content_info.econtent_type != OID_TST_INFO {
        return None;
    }
    let tst_der = sd.encap_content_info.econtent.as_ref()?.decode_as::<OctetString>().ok()?.as_bytes().to_vec();
    let tst = x509_tsp::TstInfo::from_der(&tst_der).ok()?;
    let h = Hash::from_oid(&tst.message_imprint.hash_algorithm.oid)?;
    if tst.message_imprint.hashed_message.as_bytes() != h.digest(&[signature]).as_slice() {
        return None;
    }
    let info = sd.signer_infos.0.iter().next()?;
    let certs = certificates(&sd);
    let cert = signer_cert(info, &certs)?;
    let hash = Hash::from_oid(&info.digest_alg.oid)?;
    signer_ok(info, cert, hash, &hash.digest(&[&tst_der])).ok()?;
    Some(local(tst.gen_time.to_system_time()))
}

// ----- image de signature (manuscrite) ---------------------------------------------------------

/// PNG RVBA (`width` × `height`, lignes contiguës).
fn png_rgba(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
    let mut out = vec![];
    let mut enc = png::Encoder::new(&mut out, width, height);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().and_then(|mut w| w.write_image_data(rgba)).map_err(err)?;
    Ok(out)
}

/// Recadre sur l'encre (pixels non transparents), avec une petite marge.
fn crop_ink(rgba: &[u8], w: u32, h: u32) -> Result<Vec<u8>, String> {
    let (w, h) = (w as usize, h as usize);
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
    for y in 0..h {
        for x in 0..w {
            if rgba[(y * w + x) * 4 + 3] > 0 {
                (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
            }
        }
    }
    if x0 > x1 {
        return Err("La signature est vide.".into());
    }
    let m = 4;
    let (x0, y0, x1, y1) = (x0.saturating_sub(m), y0.saturating_sub(m), (x1 + m).min(w - 1), (y1 + m).min(h - 1));
    let (cw, ch) = (x1 - x0 + 1, y1 - y0 + 1);
    let mut out = Vec::with_capacity(cw * ch * 4);
    for y in y0..=y1 {
        out.extend_from_slice(&rgba[(y * w + x0) * 4..(y * w + x1 + 1) * 4]);
    }
    png_rgba(&out, cw as u32, ch as u32)
}

/// Rendu RVBA (fond transparent) d'une page dessinée par `draw`, recadré sur l'encre.
fn render_ink(w: f32, h: f32, draw: impl FnOnce(&mut mupdf::pdf::PdfDocument, &mut mupdf::pdf::PdfPage) -> Result<(), String>)
              -> Result<Vec<u8>, String> {
    let mut doc = mupdf::pdf::PdfDocument::new();
    let mut page = doc.new_page(mupdf::Size::new(w, h)).map_err(err)?;
    draw(&mut doc, &mut page)?;
    drop(page);
    let page = doc.load_page(0).map_err(err)?;
    let pix = page.to_pixmap(&mupdf::Matrix::new_scale(2.0, 2.0), &mupdf::Colorspace::device_rgb(), true, false).map_err(err)?;
    let (pw, ph, stride) = (pix.width(), pix.height(), pix.stride() as usize);
    let mut rgba = Vec::with_capacity((pw * ph * 4) as usize);
    for row in pix.samples().chunks(stride).take(ph as usize) {
        // Couleurs prémultipliées par l'opacité : on les rétablit.
        for px in row[..pw as usize * 4].chunks(4) {
            let a = px[3] as u32;
            let un = |c: u8| if a == 0 { 0 } else { ((c as u32 * 255 + a / 2) / a).min(255) as u8 };
            rgba.extend_from_slice(&[un(px[0]), un(px[1]), un(px[2]), px[3]]);
        }
    }
    crop_ink(&rgba, pw, ph)
}

/// Signature dessinée : traits (points en pixels d'un cadre `w` × `h`) → PNG transparent.
pub fn strokes_png(strokes: &[Vec<(f32, f32)>], w: f32, h: f32) -> Result<Vec<u8>, String> {
    use mupdf::shape::{FinishOptions, PdfColor, Shape};
    if strokes.iter().all(|s| s.len() < 2) {
        return Err("La signature est vide.".into());
    }
    render_ink(w, h, |doc, page| {
        let mut shape = Shape::new(page).map_err(err)?;
        for stroke in strokes.iter().filter(|s| s.len() >= 2) {
            let pts: Vec<mupdf::Point> = stroke.iter().map(|&(x, y)| mupdf::Point::new(x, y)).collect();
            shape.draw_polyline(&pts).map_err(err)?;
        }
        shape.finish(&FinishOptions { color: Some(PdfColor::rgb(0.05, 0.1, 0.35)), width: 2.2, line_cap: Some(1), line_join: Some(1),
                                      ..Default::default() }).map_err(err)?;
        shape.commit(doc, true).map_err(err)
    })
}

/// Police d'écriture manuscrite installée (Windows), sinon Times italique.
fn script_font() -> Option<Vec<u8>> {
    ["SegoeScript", "Segoe Script", "LucidaHandwriting-Italic", "BrushScriptMT", "Inkfree"].iter()
        .find_map(|n| crate::fonts::find(n)).and_then(|p| std::fs::read(p).ok())
}

/// Signature tapée : le nom écrit dans une police manuscrite → PNG transparent.
pub fn text_png(text: &str) -> Result<Vec<u8>, String> {
    use mupdf::shape::{PdfColor, Shape, TextOptions};
    let text = text.trim();
    if text.is_empty() {
        return Err("Tapez votre nom.".into());
    }
    let font = script_font();
    let size = 48.0;
    let w = crate::annot::text_width(text, size) * 1.6 + 40.0;
    render_ink(w, size * 2.0, |doc, page| {
        let mut shape = Shape::new(page).map_err(err)?;
        let opts = TextOptions {
            fontsize: size,
            fontname: if font.is_some() { "Script".into() } else { "Times-Italic".into() },
            fontfile: font.as_deref(),
            simple: font.is_none(),
            fill: Some(PdfColor::rgb(0.05, 0.1, 0.35)),
            ..Default::default()
        };
        shape.insert_text(mupdf::Point::new(20.0, size * 1.35), text, &opts).map_err(err)?;
        shape.commit(doc, true).map_err(err)
    })
}

/// Image (scan, photo) → PNG dont le fond clair devient transparent ; l'encre garde sa
/// couleur, avec des bords adoucis. Réduite à 1000 px de large au plus.
/// `threshold` : `None` garde le fond tel quel (image seulement recadrée et réduite).
pub fn transparent_white(data: &[u8], threshold: Option<u8>) -> Result<Vec<u8>, String> {
    let img = mupdf::Image::from_bytes(data).map_err(|e| format!("Image illisible : {e}"))?;
    let mut pix = img.to_pixmap().map_err(err)?;
    while pix.width() > 1000 {
        pix.shrink(1).map_err(err)?; // divise par 2
    }
    let (w, h, n, stride) = (pix.width() as usize, pix.height() as usize, pix.n() as usize, pix.stride() as usize);
    let color = n - pix.alpha() as usize;
    if color != 1 && color != 3 {
        return Err("Image en couleurs CMJN : enregistrez-la en RVB (PNG ou JPEG).".into());
    }
    let mut rgba = Vec::with_capacity(w * h * 4);
    for row in pix.samples().chunks(stride).take(h) {
        for px in row[..w * n].chunks(n) {
            let (r, g, b) = if color == 1 { (px[0], px[0], px[0]) } else { (px[0], px[1], px[2]) };
            let gray = (r as u32 * 299 + g as u32 * 587 + b as u32 * 114) / 1000;
            // Plus clair que le seuil : transparent ; en dessous, opacité croissante.
            let a = match threshold {
                Some(t) if gray > t as u32 => 0,
                Some(t) => ((t as u32 - gray) * 255 / 100).min(255),
                None => 255,
            };
            rgba.extend_from_slice(&[r, g, b, a as u8]);
        }
    }
    crop_ink(&rgba, w as u32, h as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    const D: &str = "tests/fixtures/sign/";

    fn ca() -> Vec<Certificate> {
        vec![Certificate::from_der(&std::fs::read(format!("{D}ca.cer")).unwrap()).unwrap()]
    }

    fn fixture() -> Vec<u8> {
        std::fs::read("tests/fixtures/Été déjà vu.pdf").unwrap()
    }

    #[test]
    fn certificats() {
        for f in ["rsa-aes.pfx", "rsa-3des.pfx", "ec.pfx"] {
            let s = load_signer(Path::new(&format!("{D}{f}")), "test").unwrap_or_else(|e| panic!("{f} : {e}"));
            assert!(s.name().starts_with("Alex Testeur"), "{f} : {}", s.name());
        }
        assert!(load_signer(Path::new(&format!("{D}rsa-aes.pfx")), "faux").is_err());
    }

    #[test]
    fn signature_pyhanko_verifiee() {
        let data = std::fs::read(format!("{D}signe-pyhanko.pdf")).unwrap();
        let st = verify(&data, None, &ca()).unwrap();
        assert_eq!(st.len(), 1);
        let s = &st[0];
        assert!(s.intact, "{s:?}");
        assert!(s.trusted && !s.modified, "{s:?}");
        assert_eq!((s.signer.as_str(), s.reason.as_str(), s.location.as_str()), ("Alex Testeur RSA", "Test", "Paris"));
        assert_eq!(s.level(), "ok");
        // Sans l'autorité de test : valide mais identité non vérifiée.
        assert_eq!(verify(&data, None, &[]).unwrap()[0].level(), "warning");
        // Un octet changé dans la partie signée : invalide.
        let mut bad = data.clone();
        let pos = bad.windows(7).position(|w| w == b"Bonjour").unwrap_or(400);
        bad[pos + 1] ^= 1;
        let st = verify(&bad, None, &ca());
        assert!(st.map_or(true, |s| s.first().is_none_or(|s| !s.intact)));
    }

    #[test]
    fn signer_puis_verifier() {
        for (f, rotation) in [("rsa-aes.pfx", 0), ("ec.pfx", 90)] {
            let signer = load_signer(Path::new(&format!("{D}{f}")), "test").unwrap();
            let mut src = Pdf::open(&fixture()).unwrap();
            if rotation != 0 {
                crate::edit::rotate(&mut src, &[0], rotation).unwrap();
            }
            let data = src.bytes_for_disk().unwrap();
            let look = Look { lines: appearance_lines(&signer.name(), chrono::Local::now(), "Accord", "Lyon"), image: None };
            // Page tournée : affichée en paysage (842 × 595).
            let at = if rotation == 0 { Rect::new(300.0, 600.0, 520.0, 680.0) } else { Rect::new(500.0, 400.0, 720.0, 480.0) };
            let signed = sign(&data, Place::New(0, at), &signer, &look, "Accord", "Lyon", None).unwrap();
            assert!(signed.starts_with(&data), "ajout à la suite");
            let st = verify(&signed, None, &ca()).unwrap();
            assert_eq!(st.len(), 1, "{f}");
            assert_eq!(st[0].level(), "ok", "{f} : {:?}", st[0]);
            assert_eq!(st[0].reason, "Accord");

            // Un commentaire ajouté après : signature intacte, document « complété depuis ».
            let mut pdf = Pdf::open(&signed).unwrap();
            assert!(pdf.signed && pdf.incremental);
            crate::annot::add_note(&mut pdf, 0, 40.0, 40.0, "vu", (1.0, 0.0, 0.0), "B").unwrap();
            let after = pdf.bytes_for_disk().unwrap();
            let st = verify(&after, None, &ca()).unwrap();
            assert!(st[0].intact && st[0].modified, "{:?}", st[0]);
            assert_eq!(&after[..st[0].revision_end], &signed[..st[0].revision_end]);

            // Deux signatures : la première reste valide.
            let at = if rotation == 0 { Rect::new(60.0, 600.0, 280.0, 680.0) } else { Rect::new(200.0, 400.0, 420.0, 480.0) };
            let mut look = look;
            look.image = Some(text_png("Alex Testeur").unwrap());
            let again = sign(&after, Place::New(0, at), &signer, &look, "", "", None).unwrap();
            let st = verify(&again, None, &ca()).unwrap();
            assert_eq!(st.len(), 2);
            assert!(st.iter().all(|s| s.intact), "{st:?}");
            std::fs::write(std::env::temp_dir().join(format!("signe-{f}.pdf")), &again).ok();
        }
    }

    #[test]
    fn images_de_signature() {
        let is_transparent_png = |png: &[u8]| {
            let img = mupdf::Image::from_bytes(png).unwrap();
            assert!(img.width() > 10 && img.height() > 5);
            png.starts_with(b"\x89PNG")
        };
        let drawn = strokes_png(&[vec![(10.0, 50.0), (60.0, 10.0), (110.0, 60.0)], vec![(20.0, 70.0), (120.0, 70.0)]], 200.0, 100.0).unwrap();
        assert!(is_transparent_png(&drawn));
        assert!(strokes_png(&[], 200.0, 100.0).is_err());
        assert!(is_transparent_png(&text_png("Alex Testeur").unwrap()));
        // Image à fond blanc (rendu d'une page) : le fond devient transparent.
        let page = Pdf::open(&fixture()).unwrap().render(0, 1.0, 0).unwrap();
        let png = crate::convert::encode(&page, false).unwrap();
        let clear = transparent_white(&png, Some(200)).unwrap();
        assert!(is_transparent_png(&clear));
        let small = mupdf::Image::from_bytes(&clear).unwrap();
        assert!(small.width() < page.width, "recadré sur l'encre");
    }

    /// Réseau : `cargo test --release -- --ignored horodatage`.
    #[test]
    #[ignore]
    fn horodatage() {
        let signer = load_signer(Path::new(&format!("{D}rsa-aes.pfx")), "test").unwrap();
        let look = Look { lines: vec!["Test".into()], image: None };
        let signed = sign(&fixture(), Place::New(0, Rect::new(300.0, 600.0, 520.0, 680.0)), &signer, &look, "", "", Some(DEFAULT_TSA)).unwrap();
        let st = verify(&signed, None, &ca()).unwrap();
        eprintln!("{:?}", st[0].details());
        assert!(st[0].intact && st[0].timestamped, "{:?}", st[0]);
        std::fs::write(std::env::temp_dir().join("signe-horodate.pdf"), &signed).unwrap();
    }
}
