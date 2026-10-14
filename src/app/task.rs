use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc, Arc,
};

/// One worker, one completion message, and explicit cooperative cancellation.
pub struct BackgroundTask<T> {
    pub is_running: bool,
    pub progress: Arc<AtomicUsize>,
    pub total_tasks: Arc<AtomicUsize>,
    pub cancelled: Arc<AtomicBool>,
    rx: Option<mpsc::Receiver<Result<T, String>>>,
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        return (*message).into();
    }
    payload
        .downcast_ref::<String>()
        .map_or_else(|| "后台线程发生未知异常".into(), Clone::clone)
}

impl<T> Default for BackgroundTask<T> {
    fn default() -> Self {
        Self {
            is_running: false,
            progress: Arc::new(AtomicUsize::new(0)),
            total_tasks: Arc::new(AtomicUsize::new(0)),
            cancelled: Arc::new(AtomicBool::new(false)),
            rx: None,
        }
    }
}

impl<T> Drop for BackgroundTask<T> {
    fn drop(&mut self) {
        self.cancel();
    }
}

impl<T> BackgroundTask<T> {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    // The counters can exceed f32's integer precision; this is display-only.
    #[allow(clippy::cast_precision_loss, clippy::as_conversions)]
    pub fn fraction(&self) -> f32 {
        let total = self.total_tasks.load(Ordering::Relaxed);
        if total == 0 {
            return 0.0;
        }
        (self.progress.load(Ordering::Relaxed) as f32 / total as f32).clamp(0.0, 1.0)
    }
}

impl<T: Send + 'static> BackgroundTask<T> {
    pub fn start<F>(&mut self, total: usize, work: F) -> Result<(), String>
    where
        F: FnOnce(Arc<AtomicUsize>, Arc<AtomicUsize>, Arc<AtomicBool>) -> Result<T, String>
            + Send
            + 'static,
    {
        if self.is_running {
            return Err("已有任务正在运行".into());
        }
        // A new task must never share a previous worker's counters or token.
        self.progress = Arc::new(AtomicUsize::new(0));
        self.total_tasks = Arc::new(AtomicUsize::new(total));
        self.cancelled = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let progress = Arc::clone(&self.progress);
        let total_tasks = Arc::clone(&self.total_tasks);
        let cancelled = Arc::clone(&self.cancelled);
        std::thread::Builder::new()
            .name("acs-worker".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    work(progress, total_tasks, cancelled)
                }));
                let completion = match result {
                    Ok(value) => value,
                    Err(payload) => Err(panic_message(payload.as_ref())),
                };
                let _ = tx.send(completion);
            })
            .map_err(|error| format!("无法创建后台线程: {error}"))?;
        self.rx = Some(rx);
        self.is_running = true;
        Ok(())
    }

    pub fn poll(&mut self) -> Option<Result<T, String>> {
        let message = self.rx.as_ref()?.try_recv();
        if matches!(message, Err(mpsc::TryRecvError::Empty)) {
            return None;
        }
        self.is_running = false;
        self.rx = None;
        Some(message.unwrap_or_else(|_| Err("后台线程未返回结果".into())))
    }
}
