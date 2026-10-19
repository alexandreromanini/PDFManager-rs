//! Thread moteur : seul propriétaire des documents MuPDF (un document MuPDF ne peut pas
//! changer de thread). L'interface lui confie des tâches et reçoit leurs résultats sans
//! jamais attendre : la fenêtre ne fige pas pendant un rendu ou une recherche.

use std::collections::HashMap;
use std::sync::mpsc;

use crate::pdf::Pdf;

pub type DocId = u64;
pub type Docs = HashMap<DocId, Pdf>;
type Job = Box<dyn FnOnce(&mut Docs) + Send>;

#[derive(Clone)]
pub struct Engine {
    tx: mpsc::Sender<Job>,
}

impl Engine {
    pub fn start() -> Engine {
        let (tx, rx) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("moteur-pdf".into())
            .spawn(move || {
                let mut docs = Docs::new();
                for job in rx {
                    job(&mut docs);
                }
            })
            .expect("thread moteur");
        Engine { tx }
    }

    /// Exécute `job` dans le thread moteur, après les tâches déjà confiées.
    pub fn run(&self, job: impl FnOnce(&mut Docs) + Send + 'static) {
        let _ = self.tx.send(Box::new(job));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jobs_run_in_order_on_one_thread() {
        let engine = Engine::start();
        let (tx, rx) = mpsc::channel();
        let data = std::fs::read("tests/fixtures/Été déjà vu.pdf").unwrap();
        engine.run(move |docs| {
            docs.insert(1, Pdf::open(&data).unwrap());
        });
        engine.run(move |docs| {
            let _ = tx.send((docs[&1].page_count(), std::thread::current().name().map(String::from)));
        });
        assert_eq!(rx.recv().unwrap(), (3, Some("moteur-pdf".into())));
    }
}
