fn main() {
    slint_build::compile("ui/app.slint").expect("interface Slint");
    // Exécutable Windows : icône et nom affichés par l'Explorateur.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("packaging/pdfmanager.ico");
        res.set("ProductName", "PDFManager (Rust)");
        res.set("FileDescription", "PDFManager");
        res.compile().expect("ressources Windows");
    }
}
