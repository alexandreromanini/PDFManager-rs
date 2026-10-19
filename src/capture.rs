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
            "color" => app::with_app(move |a| a.pick_color(false, arg.parse().unwrap_or(0))),
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
