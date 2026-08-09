use std::sync::{
    atomic::{AtomicUsize, Ordering},
    mpsc, Arc,
};

pub struct BackgroundTask<T> {
    pub is_running: bool,
    pub progress: Arc<AtomicUsize>,
    pub total_tasks: Arc<AtomicUsize>,
    rx: Option<mpsc::Receiver<Result<T, String>>>,
}

impl<T> Default for BackgroundTask<T> {
    fn default() -> Self {
        Self {
            is_running: false,
            progress: Arc::new(AtomicUsize::new(0)),
            total_tasks: Arc::new(AtomicUsize::new(0)),
            rx: None,
        }
    }
}

impl<T: Send + 'static> BackgroundTask<T> {
    pub fn start<F>(&mut self, t: usize, f: F)
    where
        F: FnOnce(Arc<AtomicUsize>, Arc<AtomicUsize>) -> T + Send + 'static,
    {
        self.is_running = true;
        self.total_tasks.store(t, Ordering::Relaxed);
        self.progress.store(0, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        let p = self.progress.clone();
        let tot = self.total_tasks.clone();
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(p, tot)));
            let _ = tx.send(match result {
                Ok(val) => Ok(val),
                Err(panic) => {
                    let msg = if let Some(s) = panic.downcast_ref::<&str>() {
                        s.to_string()
                    } else if let Some(s) = panic.downcast_ref::<String>() {
                        s.clone()
                    } else {
                        "Background thread panicked".to_string()
                    };
                    Err(msg)
                }
            });
        });
    }

    pub fn poll(&mut self) -> Option<Result<Result<T, String>, mpsc::TryRecvError>> {
        let res = self.rx.as_ref()?.try_recv();
        match res {
            Ok(r) => {
                self.is_running = false;
                self.rx = None;
                Some(Ok(r))
            }
            Err(mpsc::TryRecvError::Empty) => None,
            Err(e) => {
                self.is_running = false;
                self.rx = None;
                Some(Err(e))
            }
        }
    }

    pub fn fraction(&self) -> f32 {
        let t = self.total_tasks.load(Ordering::Relaxed);
        if t > 0 {
            (self.progress.load(Ordering::Relaxed) as f32 / t as f32).clamp(0., 1.)
        } else {
            0.
        }
    }
}
