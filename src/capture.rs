//! Développement seulement (option `capture`) : rend la fenêtre hors écran et l'écrit en
//! image PPM, pour vérifier l'interface sans affichage.
//! `pdfmanager --capture sortie.ppm largeur hauteur thème fichier.pdf… [-- action…]`
//! Actions : `wait`, `home`, `sidebar`, `zoom+`, `page:N`, `search:texte`, `rail:N`, `panel:N`,
//! `action:nom` (sans dialogue), `select:N` (Ctrl+clic sur une vignette), `tool:nom`, `color:N`,
//! `drag:page,x0,y0,x1,y1`, `type:texte` (éditeur), `header`, `watermark:texte`.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use slint::platform::software_renderer::{MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType};
use slint::platform::{EventLoopProxy, Platform, WindowAdapter};
use slint::{ComponentHandle, EventLoopError, PhysicalSize, PlatformError};

use crate::{app, AppWindow};

type Queue = Arc<Mutex<VecDeque<Box<dyn FnOnce() + Send>>>>;

struct Proxy(Queue);

impl EventLoopProxy for Proxy {
    fn quit_event_loop(&self) -> Result<(), EventLoopError> {
        Ok(())
    }
    fn invoke_from_event_loop(&self, event: Box<dyn FnOnce() + Send>) -> Result<(), EventLoopError> {
        self.0.lock().unwrap().push_back(event);
        Ok(())
    }
}

struct Offscreen {
    window: Rc<MinimalSoftwareWindow>,
    queue: Queue,
}

impl Platform for Offscreen {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.window.clone())
    }
    fn new_event_loop_proxy(&self) -> Option<Box<dyn EventLoopProxy>> {
        Some(Box::new(Proxy(self.queue.clone())))
    }
}

/// Fait tourner l'interface : tâches du moteur, minuteries, pendant `ms` millisecondes.
fn pump(queue: &Queue, ms: u64) {
    let end = std::time::Instant::now() + Duration::from_millis(ms);
    while std::time::Instant::now() < end {
        let jobs: Vec<_> = queue.lock().unwrap().drain(..).collect();
        for job in jobs {
            job();
        }
        slint::platform::update_timers_and_animations();
        std::thread::sleep(Duration::from_millis(5));
    }
}

pub fn run(args: &[String]) -> std::process::ExitCode {
    let (out, w, h, theme) = (&args[0], args[1].parse().unwrap(), args[2].parse().unwrap(), args[3].clone());
    let split = args.iter().position(|a| a == "--").unwrap_or(args.len());
    let files = args[4..split].iter().map(std::path::PathBuf::from).collect();
    let actions = args.get(split + 1..).unwrap_or_default().to_vec();

    // SAFETY : avant tout autre thread de l'application.
    unsafe { std::env::set_var("PDFMANAGER_CAPTURE", "1") };
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    let queue = Queue::default();
    slint::platform::set_platform(Box::new(Offscreen { window: window.clone(), queue: queue.clone() }))
        .expect("plateforme hors écran");
    window.set_size(PhysicalSize::new(w, h));
    let ui = AppWindow::new().expect("fenêtre");
    app::install(&ui, files);
    app::with_app(move |a| a.set_theme(&theme));
    ui.show().expect("affichage");
    pump(&queue, 1500);
    for action in actions {
        let (verb, arg) = action.split_once(':').unwrap_or((&action, ""));
        let arg = arg.to_string();
        match verb {
            "home" => app::with_app(|a| a.go_home()),
            "sidebar" => app::with_app(|a| a.toggle_sidebar()),
            "zoom+" => app::with_app(|a| a.zoom_step(1)),
            "page" => app::with_app(move |a| a.page_entered(&arg)),
            "search" => app::with_app(move |a| a.search(arg)),
            "rail" => ui.set_rail_tab(arg.parse().unwrap_or(0)),
            "panel" => app::with_app(move |a| a.set_panel(arg.parse().unwrap_or(0))),
            "action" => app::with_app(move |a| a.action(&arg)),
            "select" => app::with_app(move |a| a.thumb_press(arg.parse().unwrap_or(0), true, false)),
            "tool" => app::with_app(move |a| a.set_tool(&arg)),
            "sleep" => pump(&queue, arg.parse().unwrap_or(1000)),
            "ask" => app::with_app(move |a| a.ai_ask(arg)),
            "compare" => app::with_app(move |a| a.compare_with(std::path::PathBuf::from(arg))),
            // Parcours complets (audit) : un appel du contrôleur par commande.
            "tab" => app::with_app(move |a| a.select_tab(arg.parse().unwrap_or(0))),
            "close" => app::with_app(move |a| a.request_close_tab(arg.parse().unwrap_or(0))),
            "quit" => app::with_app(|a| a.quit()),
            "merge" => app::with_app(move |a| a.merge_command(&arg)),
            "split" => app::with_app(move |a| a.split_run(0, 1, arg)),
            "split-every" => app::with_app(move |a| a.split_run(1, arg.parse().unwrap_or(1), String::new())),
            "split-bookmarks" => app::with_app(|a| a.split_run(2, 1, String::new())),
            // protect:ouverture,propriétaire,imprimer,copier,commenter,modifier (1/0)
            "protect" => {
                let v: Vec<String> = arg.split(',').map(String::from).collect();
                let b = |k: usize| v.get(k).is_some_and(|x| x == "1");
                let allow = [b(2), b(3), b(4), b(5)];
                let (o, w) = (v.first().cloned().unwrap_or_default(), v.get(1).cloned().unwrap_or_default());
                app::with_app(move |a| a.protect_run(o.clone(), o, w.clone(), w, allow));
            }
            "unprotect" => app::with_app(move |a| a.owner_password(arg)),
            "note" => app::with_app(move |a| a.note_done(arg)),
            "choice" => app::with_app(move |a| a.choice_picked(arg.parse().unwrap_or(0))),
            // prop:index=texte
            "prop" => {
                let (i, t) = arg.split_once('=').unwrap_or(("0", ""));
                let (i, t) = (i.parse().unwrap_or(0), t.to_string());
                app::with_app(move |a| a.prop_edited(i, t));
            }
            "props-save" => app::with_app(|a| a.props_save()),
            "clean" => app::with_app(|a| a.clean_run(true, true, true, true)),
            "ocr" => app::with_app(move |a| a.ocr_run(0, arg, false, 300)),
            "export-images" => app::with_app(move |a| a.export_images_run(false, 72, arg)),
            "compress" => app::with_app(move |a| a.compress_run(arg.parse().unwrap_or(0))),
            // digital:fichier.pfx,mot de passe
            "digital" => {
                let (f, pw) = arg.split_once(',').unwrap_or((&arg, ""));
                let (f, pw) = (f.to_string(), pw.to_string());
                app::with_app(move |a| a.digital_run(f, pw, "Audit".into(), "Ici".into(), false, String::new(), 0));
            }
            "sig-typed" => app::with_app(move |a| {
                a.sig_typed(arg);
                a.sig_save(1, 400.0, 150.0);
            }),
            "sig-place" => app::with_app(move |a| a.sig_place(arg.parse().unwrap_or(0))),
            "sig-delete" => app::with_app(move |a| a.sig_delete(arg.parse().unwrap_or(0))),
            "trust" => app::with_app(move |a| a.sig_trust(arg.parse().unwrap_or(0))),
            "version" => app::with_app(move |a| a.sig_signed_version(arg.parse().unwrap_or(0))),
            "diff" => app::with_app(move |a| a.diff_clicked(arg.parse().unwrap_or(0))),
            "comment" => app::with_app(move |a| a.comment_clicked(arg.parse().unwrap_or(0))),
            "comment-delete" => app::with_app(move |a| a.comment_delete(arg.parse().unwrap_or(0))),
            "thumb" => app::with_app(move |a| a.thumb_press(arg.parse().unwrap_or(0), false, false)),
            "zoom" => app::with_app(move |a| a.zoom_entered(&arg)),
            "rotate" => app::with_app(move |a| a.rotate(arg.parse().unwrap_or(90))),
            "mode" => app::with_app(move |a| a.set_mode(arg.parse().unwrap_or(0))),
            "fit" => app::with_app(move |a| a.set_fit(arg.parse().unwrap_or(1))),
            "copy" => app::with_app(|a| a.copy()),
            "drop" => app::with_app(move |a| a.dropped(std::path::PathBuf::from(arg))),
            "open" => app::with_app(move |a| a.open_path(std::path::PathBuf::from(arg))),
            "author" => app::with_app(move |a| a.set_author(arg)),
            "color" => app::with_app(move |a| a.pick_color(false, arg.parse().unwrap_or(0))),
            "text-color" => app::with_app(move |a| a.pick_color(true, arg.parse().unwrap_or(0))),
            "text-size" => app::with_app(move |a| a.text_size_changed(arg.parse().unwrap_or(12))),
            "etat" => app::with_app(|a| a.dump_state()),
            "menu" => ui.invoke_show_menu(),
            // click:x,y (pixels logiques de la fenêtre)
            "click" => {
                let (x, y) = arg.split_once(',').unwrap_or(("0", "0"));
                let position = slint::LogicalPosition::new(x.parse().unwrap_or(0.0), y.parse().unwrap_or(0.0));
                let button = slint::platform::PointerEventButton::Left;
                window.dispatch_event(slint::platform::WindowEvent::PointerMoved { position });
                window.dispatch_event(slint::platform::WindowEvent::PointerPressed { position, button });
                window.dispatch_event(slint::platform::WindowEvent::PointerReleased { position, button });
            }
            // mouse-drag:x0,y0,x1,y1 (vrai glisser à la souris, pixels logiques de la fenêtre)
            "mouse-drag" => {
                let v: Vec<f32> = arg.split(',').filter_map(|n| n.parse().ok()).collect();
                if v.len() == 4 {
                    use slint::platform::{PointerEventButton, WindowEvent};
                    // un dessin d'abord : les pages (liste) n'existent qu'une fois dessinées
                    let size = window.size();
                    let mut scratch = vec![PremultipliedRgbaColor::default(); (size.width * size.height) as usize];
                    window.request_redraw();
                    window.draw_if_needed(|r| {
                        r.render(&mut scratch, size.width as usize);
                    });
                    pump(&queue, 50);
                    let at = |k: f32| slint::LogicalPosition::new(v[0] + (v[2] - v[0]) * k, v[1] + (v[3] - v[1]) * k);
                    let button = PointerEventButton::Left;
                    window.dispatch_event(WindowEvent::PointerMoved { position: at(0.0) });
                    pump(&queue, 50);
                    window.dispatch_event(WindowEvent::PointerPressed { position: at(0.0), button });
                    pump(&queue, 50);
                    for k in 1..=10 {
                        window.dispatch_event(WindowEvent::PointerMoved { position: at(k as f32 / 10.0) });
                        pump(&queue, 20);
                    }
                    window.dispatch_event(WindowEvent::PointerReleased { position: at(1.0), button });
                }
            }
            // key:Tab, key:Return, key:espace… (touche envoyée à la fenêtre)
            "key" => {
                let text: slint::SharedString = match arg.as_str() {
                    "Tab" => slint::platform::Key::Tab.into(),
                    "Return" => slint::platform::Key::Return.into(),
                    "Escape" => slint::platform::Key::Escape.into(),
                    "espace" => " ".into(),
                    t => t.into(),
                };
                window.dispatch_event(slint::platform::WindowEvent::KeyPressed { text: text.clone() });
                window.dispatch_event(slint::platform::WindowEvent::KeyReleased { text });
            }
            "resize" => {
                let (w, h) = arg.split_once(',').unwrap_or(("1280", "800"));
                window.set_size(PhysicalSize::new(w.parse().unwrap_or(1280), h.parse().unwrap_or(800)));
            }
            "password" => app::with_app(move |a| a.password_entered(arg)),
            "password-cancel" => app::with_app(|a| a.password_cancelled()),
            "step" => app::with_app(move |a| a.step_page(arg.parse().unwrap_or(1))),
            "search-step" => app::with_app(move |a| a.search_step(arg.parse().unwrap_or(1))),
            "type" => ui.set_editor_text(arg.into()),
            // drag:page,x0,y0,x1,y1 (pixels de la page affichée)
            "drag" => {
                let v: Vec<f32> = arg.split(',').map(|n| n.parse().unwrap_or(0.0)).collect();
                let page = v[0] as usize;
                app::with_app(move |a| {
                    a.page_press(page, v[1], v[2]);
                    a.page_drag(page, (v[1] + v[3]) / 2.0, (v[2] + v[4]) / 2.0 + 3.0);
                    a.page_drag(page, v[3], v[4]);
                    a.page_release(page, v[3], v[4]);
                });
            }
            "header" => app::with_app(|a| a.header_run(["{fichier}".into(), String::new(), "{date}".into(), String::new(),
                                                         "Page {n} sur {total}".into(), String::new()], 10, 1, String::new())),
            "watermark" => app::with_app(move |a| a.watermark_run(arg, 72, 30, 45, String::new())),
            _ => {}
        }
        pump(&queue, 800);
    }
    let (w, h) = (window.size().width, window.size().height); // après un éventuel « resize »
    let pixels = Rc::new(RefCell::new(vec![PremultipliedRgbaColor::default(); (w * h) as usize]));
    // Premier dessin : les listes demandent alors leurs lignes (vignettes à rendre).
    for _ in 0..2 {
        window.request_redraw();
        window.draw_if_needed(|renderer| {
            renderer.render(&mut pixels.borrow_mut(), w as usize);
        });
        pump(&queue, 800);
    }
    let mut ppm = format!("P6\n{w} {h}\n255\n").into_bytes();
    for p in pixels.borrow().iter() {
        ppm.extend_from_slice(&[p.red, p.green, p.blue]);
    }
    std::fs::write(out, ppm).expect("image");
    std::process::ExitCode::SUCCESS
}
