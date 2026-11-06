//! Schedules render requests on a single renderer thread, by priority, with a PNG cache.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};

use crate::{Link, OutlineItem, PageSize, RenderError, Renderer};

const CACHE_BUDGET: usize = 128 << 20;

pub type RenderResult = Result<Arc<[u8]>, RenderError>;
type Callback = Box<dyn FnOnce(RenderResult) + Send>;
type OpenReply = mpsc::Sender<Result<OpenedDocument, RenderError>>;
type ReviseReply = mpsc::Sender<Result<(), RenderError>>;
type LinksReply = mpsc::Sender<Result<Vec<Link>, RenderError>>;

#[derive(Debug, Clone, PartialEq)]
pub struct OpenedDocument {
    /// Identifies this document in render requests; it changes on every successful open.
    pub id: u64,
    pub pages: Vec<PageSize>,
    pub outline: Vec<OutlineItem>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderRequest {
    /// Caller-chosen identifier used to cancel the request.
    pub id: u64,
    pub doc: u64,
    pub page: u32,
    pub width: u32,
    /// Lower values render first; equal priorities render in arrival order.
    pub priority: u8,
}

pub struct RenderService {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl RenderService {
    pub fn new(renderer: impl Renderer + 'static) -> Self {
        let shared = Arc::new(Shared::default());
        let thread = thread::Builder::new()
            .name("ukurasa-render".into())
            .spawn({
                let shared = Arc::clone(&shared);
                move || run(&shared, renderer)
            })
            .expect("failed to start the render thread");
        Self {
            shared,
            thread: Some(thread),
        }
    }

    /// Opens a document, blocking until the renderer has loaded it. On failure the previous
    /// document stays open. Pending renders for the previous document fail with
    /// [`RenderError::Stale`].
    pub fn open(&self, path: &Path) -> Result<OpenedDocument, RenderError> {
        let (reply, result) = mpsc::channel();
        let superseded = {
            let mut state = self.shared.lock();
            if state.shutdown {
                return Err(RenderError::Cancelled);
            }
            state.open.replace((path.to_path_buf(), reply))
        };
        self.shared.wake.notify_one();
        if let Some((_, reply)) = superseded {
            let _ = reply.send(Err(RenderError::Cancelled));
        }
        result.recv().unwrap_or(Err(RenderError::Cancelled))
    }

    /// Renders document `doc` from `pdf`, an edited copy, until the next open or revision;
    /// `None` returns to the file. Blocks until the renderer has loaded it, ahead of queued
    /// renders, and drops cached images of `pages`, the pages that differ from the previous
    /// revision.
    pub fn revise(
        &self,
        doc: u64,
        pdf: Option<Arc<[u8]>>,
        pages: &[u32],
    ) -> Result<(), RenderError> {
        let (reply, result) = mpsc::channel();
        {
            let mut state = self.shared.lock();
            if state.shutdown {
                return Err(RenderError::Cancelled);
            }
            state.revisions.push_back(Revision {
                doc,
                pdf,
                pages: pages.to_vec(),
                reply,
            });
        }
        self.shared.wake.notify_one();
        result.recv().unwrap_or(Err(RenderError::Cancelled))
    }

    /// Lists the links on `page` of document `doc`, blocking until the renderer replies.
    /// Runs ahead of queued renders.
    pub fn links(&self, doc: u64, page: u32) -> Result<Vec<Link>, RenderError> {
        let (reply, result) = mpsc::channel();
        {
            let mut state = self.shared.lock();
            if state.shutdown {
                return Err(RenderError::Cancelled);
            }
            state.queries.push_back(LinksQuery { doc, page, reply });
        }
        self.shared.wake.notify_one();
        result.recv().unwrap_or(Err(RenderError::Cancelled))
    }

    /// Queues a render. `callback` runs exactly once, on the calling thread for cache hits and
    /// immediate failures, and on the render thread otherwise.
    pub fn render(
        &self,
        request: RenderRequest,
        callback: impl FnOnce(RenderResult) + Send + 'static,
    ) {
        let immediate = {
            let mut state = self.shared.lock();
            let key = CacheKey::from(&request);
            if state.shutdown {
                Some(Err(RenderError::Cancelled))
            } else if request.doc != state.doc {
                Some(Err(RenderError::Stale))
            } else if let Some(png) = state.cache.get(&key) {
                Some(Ok(png))
            } else {
                state.seq += 1;
                let order = (request.priority, state.seq);
                state.ids.insert(request.id, order);
                state.jobs.insert(
                    order,
                    Job {
                        request,
                        callback: Box::new(callback),
                    },
                );
                self.shared.wake.notify_one();
                return;
            }
        };
        if let Some(result) = immediate {
            callback(result);
        }
    }

    /// Cancels queued renders; their callbacks receive [`RenderError::Cancelled`].
    pub fn cancel(&self, ids: &[u64]) {
        let cancelled: Vec<Job> = {
            let mut guard = self.shared.lock();
            let state = &mut *guard;
            ids.iter()
                .filter_map(|id| state.ids.remove(id))
                .filter_map(|order| state.jobs.remove(&order))
                .collect()
        };
        for job in cancelled {
            (job.callback)(Err(RenderError::Cancelled));
        }
    }
}

impl Drop for RenderService {
    fn drop(&mut self) {
        self.shared.lock().shutdown = true;
        self.shared.wake.notify_all();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    wake: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[derive(Default)]
struct State {
    doc: u64,
    open: Option<(PathBuf, OpenReply)>,
    revisions: VecDeque<Revision>,
    queries: VecDeque<LinksQuery>,
    jobs: BTreeMap<(u8, u64), Job>,
    ids: HashMap<u64, (u8, u64)>,
    seq: u64,
    cache: Cache,
    shutdown: bool,
}

impl State {
    fn drain_jobs(&mut self) -> Vec<Job> {
        self.ids.clear();
        std::mem::take(&mut self.jobs).into_values().collect()
    }
}

struct Job {
    request: RenderRequest,
    callback: Callback,
}

struct Revision {
    doc: u64,
    pdf: Option<Arc<[u8]>>,
    pages: Vec<u32>,
    reply: ReviseReply,
}

struct LinksQuery {
    doc: u64,
    page: u32,
    reply: LinksReply,
}

enum Task {
    Open(PathBuf, OpenReply),
    Revise(Revision),
    Links(LinksQuery),
    Render(Job),
    Cached(Job, Arc<[u8]>),
}

fn run(shared: &Shared, mut renderer: impl Renderer) {
    while let Some(task) = next_task(shared) {
        match task {
            Task::Open(path, reply) => {
                let result = renderer.open(&path);
                let (result, stale) = {
                    let mut state = shared.lock();
                    match result {
                        Ok(info) => {
                            state.doc += 1;
                            state.cache.clear();
                            let opened = OpenedDocument {
                                id: state.doc,
                                pages: info.pages,
                                outline: info.outline,
                            };
                            (Ok(opened), state.drain_jobs())
                        }
                        Err(error) => (Err(error), Vec::new()),
                    }
                };
                for job in stale {
                    (job.callback)(Err(RenderError::Stale));
                }
                let _ = reply.send(result);
            }
            Task::Revise(revision) => {
                // Opens run on this thread too, so the document cannot change underneath.
                let current = shared.lock().doc == revision.doc;
                let result = if current {
                    renderer.revise(revision.pdf)
                } else {
                    Err(RenderError::Stale)
                };
                if result.is_ok() {
                    shared
                        .lock()
                        .cache
                        .remove_pages(revision.doc, &revision.pages);
                }
                let _ = revision.reply.send(result);
            }
            Task::Links(query) => {
                let result = if shared.lock().doc == query.doc {
                    renderer.links(query.page)
                } else {
                    Err(RenderError::Stale)
                };
                let _ = query.reply.send(result);
            }
            Task::Cached(job, png) => (job.callback)(Ok(png)),
            Task::Render(job) => {
                let result = renderer
                    .render(job.request.page, job.request.width)
                    .map(Arc::<[u8]>::from);
                if let Ok(png) = &result {
                    let mut state = shared.lock();
                    if state.doc == job.request.doc {
                        state
                            .cache
                            .insert(CacheKey::from(&job.request), Arc::clone(png));
                    }
                }
                (job.callback)(result);
            }
        }
    }

    let (jobs, open, revisions, queries) = {
        let mut state = shared.lock();
        (
            state.drain_jobs(),
            state.open.take(),
            std::mem::take(&mut state.revisions),
            std::mem::take(&mut state.queries),
        )
    };
    for job in jobs {
        (job.callback)(Err(RenderError::Cancelled));
    }
    if let Some((_, reply)) = open {
        let _ = reply.send(Err(RenderError::Cancelled));
    }
    for revision in revisions {
        let _ = revision.reply.send(Err(RenderError::Cancelled));
    }
    for query in queries {
        let _ = query.reply.send(Err(RenderError::Cancelled));
    }
    let _ = renderer.close();
}

fn next_task(shared: &Shared) -> Option<Task> {
    let mut state = shared.lock();
    loop {
        if state.shutdown {
            return None;
        }
        if let Some((path, reply)) = state.open.take() {
            return Some(Task::Open(path, reply));
        }
        if let Some(revision) = state.revisions.pop_front() {
            return Some(Task::Revise(revision));
        }
        if let Some(query) = state.queries.pop_front() {
            return Some(Task::Links(query));
        }
        if let Some((_, job)) = state.jobs.pop_first() {
            state.ids.remove(&job.request.id);
            // An earlier job may have rendered the same page at the same width.
            return Some(match state.cache.get(&CacheKey::from(&job.request)) {
                Some(png) => Task::Cached(job, png),
                None => Task::Render(job),
            });
        }
        state = shared
            .wake
            .wait(state)
            .unwrap_or_else(PoisonError::into_inner);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct CacheKey {
    doc: u64,
    page: u32,
    width: u32,
}

impl From<&RenderRequest> for CacheKey {
    fn from(request: &RenderRequest) -> Self {
        Self {
            doc: request.doc,
            page: request.page,
            width: request.width,
        }
    }
}

/// Least recently used PNG cache bounded by total byte size.
struct Cache {
    budget: usize,
    used: usize,
    clock: u64,
    entries: HashMap<CacheKey, (Arc<[u8]>, u64)>,
}

impl Default for Cache {
    fn default() -> Self {
        Self {
            budget: CACHE_BUDGET,
            used: 0,
            clock: 0,
            entries: HashMap::new(),
        }
    }
}

impl Cache {
    fn get(&mut self, key: &CacheKey) -> Option<Arc<[u8]>> {
        self.clock += 1;
        let (png, used) = self.entries.get_mut(key)?;
        *used = self.clock;
        Some(Arc::clone(png))
    }

    fn insert(&mut self, key: CacheKey, png: Arc<[u8]>) {
        if png.len() > self.budget {
            return;
        }
        if let Some((old, _)) = self.entries.remove(&key) {
            self.used -= old.len();
        }
        while self.used + png.len() > self.budget {
            let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, (_, used))| *used)
                .map(|(key, _)| *key)
            else {
                break;
            };
            if let Some((evicted, _)) = self.entries.remove(&oldest) {
                self.used -= evicted.len();
            }
        }
        self.clock += 1;
        self.used += png.len();
        self.entries.insert(key, (png, self.clock));
    }

    fn clear(&mut self) {
        self.entries.clear();
        self.used = 0;
    }

    fn remove_pages(&mut self, doc: u64, pages: &[u32]) {
        self.entries.retain(|key, (png, _)| {
            let keep = key.doc != doc || !pages.contains(&key.page);
            if !keep {
                self.used -= png.len();
            }
            keep
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DocumentInfo, ErrorKind, LinkTarget};
    use std::time::Duration;

    const TIMEOUT: Duration = Duration::from_secs(5);

    /// Records calls; each render waits for a token when a gate is installed. Rendered bytes
    /// are the page number plus the length of the current revision.
    struct Fake {
        log: Log,
        gate: Option<mpsc::Receiver<()>>,
        revision: u8,
    }

    impl Renderer for Fake {
        fn open(&mut self, path: &Path) -> Result<DocumentInfo, RenderError> {
            self.log
                .lock()
                .unwrap()
                .push(format!("open {}", path.display()));
            if path.ends_with("bad.pdf") {
                return Err(RenderError::Worker {
                    kind: ErrorKind::Format,
                    message: "bad".into(),
                });
            }
            Ok(DocumentInfo {
                pages: vec![
                    PageSize {
                        width: 612.0,
                        height: 792.0
                    };
                    10
                ],
                outline: Vec::new(),
            })
        }

        fn render(&mut self, page: u32, width: u32) -> Result<Vec<u8>, RenderError> {
            if let Some(gate) = &self.gate {
                let _ = gate.recv();
            }
            self.log
                .lock()
                .unwrap()
                .push(format!("render {page} {width}"));
            Ok(vec![page as u8 + self.revision; width as usize])
        }

        fn revise(&mut self, pdf: Option<Arc<[u8]>>) -> Result<(), RenderError> {
            self.revision = pdf.as_deref().map_or(0, |pdf| pdf.len() as u8);
            self.log
                .lock()
                .unwrap()
                .push(format!("revise {}", self.revision));
            Ok(())
        }

        fn links(&mut self, page: u32) -> Result<Vec<Link>, RenderError> {
            self.log.lock().unwrap().push(format!("links {page}"));
            Ok(vec![Link {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
                target: LinkTarget::Page { page, top: None },
            }])
        }

        fn close(&mut self) -> Result<(), RenderError> {
            self.log.lock().unwrap().push("close".into());
            Ok(())
        }
    }

    type Log = Arc<Mutex<Vec<String>>>;

    fn service(gated: bool) -> (RenderService, Log, Option<mpsc::Sender<()>>) {
        let log = Arc::new(Mutex::new(Vec::new()));
        let (tokens, gate) = mpsc::channel();
        let fake = Fake {
            log: Arc::clone(&log),
            gate: gated.then_some(gate),
            revision: 0,
        };
        (RenderService::new(fake), log, gated.then_some(tokens))
    }

    fn request(id: u64, doc: u64, page: u32, priority: u8) -> RenderRequest {
        RenderRequest {
            id,
            doc,
            page,
            width: 100,
            priority,
        }
    }

    type Results = mpsc::Receiver<(u64, RenderResult)>;

    fn submit(
        service: &RenderService,
        results: &mpsc::Sender<(u64, RenderResult)>,
        request: RenderRequest,
    ) {
        let results = results.clone();
        service.render(request, move |result| {
            let _ = results.send((request.id, result));
        });
    }

    fn receive(results: &Results) -> (u64, RenderResult) {
        results
            .recv_timeout(TIMEOUT)
            .expect("render callback did not run")
    }

    fn wait_until(condition: impl Fn() -> bool) {
        let start = std::time::Instant::now();
        while !condition() {
            assert!(start.elapsed() < TIMEOUT, "condition was not reached");
            thread::yield_now();
        }
    }

    fn renders(log: &Mutex<Vec<String>>) -> Vec<String> {
        log.lock()
            .unwrap()
            .iter()
            .filter(|line| line.starts_with("render"))
            .cloned()
            .collect()
    }

    #[test]
    fn renders_and_caches_pages() {
        let (service, log, _) = service(false);
        let opened = service.open(Path::new("a.pdf")).unwrap();
        assert_eq!(opened.id, 1);
        assert_eq!(opened.pages.len(), 10);

        let (tx, rx) = mpsc::channel();
        submit(&service, &tx, request(1, opened.id, 2, 0));
        let (id, png) = receive(&rx);
        assert_eq!(id, 1);
        assert_eq!(&*png.unwrap(), &[2u8; 100][..]);

        submit(&service, &tx, request(2, opened.id, 2, 0));
        assert_eq!(receive(&rx).1.unwrap().len(), 100);
        assert_eq!(renders(&log), ["render 2 100"]);
    }

    #[test]
    fn higher_priority_renders_first_and_ties_keep_arrival_order() {
        let (service, log, tokens) = service(true);
        let doc = service.open(Path::new("a.pdf")).unwrap().id;
        let (tx, rx) = mpsc::channel();
        for (id, priority) in [(1, 0), (2, 1), (3, 0), (4, 1), (5, 0)] {
            submit(&service, &tx, request(id, doc, id as u32, priority));
        }
        let tokens = tokens.unwrap();
        for _ in 0..5 {
            tokens.send(()).unwrap();
        }
        let order: Vec<u64> = (0..5).map(|_| receive(&rx).0).collect();
        assert_eq!(order, [1, 3, 5, 2, 4]);
        assert_eq!(renders(&log).len(), 5);
    }

    #[test]
    fn cancelled_renders_never_reach_the_renderer() {
        let (service, log, tokens) = service(true);
        let doc = service.open(Path::new("a.pdf")).unwrap().id;
        let (tx, rx) = mpsc::channel();
        submit(&service, &tx, request(1, doc, 1, 0));
        submit(&service, &tx, request(2, doc, 2, 0));
        submit(&service, &tx, request(3, doc, 3, 0));
        service.cancel(&[2, 99]);
        let (id, result) = receive(&rx);
        assert_eq!((id, result), (2, Err(RenderError::Cancelled)));

        let tokens = tokens.unwrap();
        tokens.send(()).unwrap();
        tokens.send(()).unwrap();
        let mut done: Vec<u64> = (0..2).map(|_| receive(&rx).0).collect();
        done.sort_unstable();
        assert_eq!(done, [1, 3]);
        assert_eq!(renders(&log), ["render 1 100", "render 3 100"]);
    }

    #[test]
    fn opening_a_document_makes_old_requests_stale() {
        let (service, _, tokens) = service(true);
        let first = service.open(Path::new("a.pdf")).unwrap().id;
        let (tx, rx) = mpsc::channel();
        submit(&service, &tx, request(1, first, 1, 0));
        submit(&service, &tx, request(2, first, 2, 0));
        wait_until(|| service.shared.lock().jobs.len() == 1);

        let tokens = tokens.unwrap();
        let second = thread::scope(|scope| {
            let opening = scope.spawn(|| service.open(Path::new("b.pdf")).unwrap().id);
            wait_until(|| service.shared.lock().open.is_some());
            tokens.send(()).unwrap();
            opening.join().unwrap()
        });
        assert_eq!(second, first + 1);

        let mut results: Vec<(u64, bool)> = (0..2)
            .map(|_| receive(&rx))
            .map(|(id, r)| (id, r == Err(RenderError::Stale)))
            .collect();
        results.sort_unstable();
        assert_eq!(results, [(1, false), (2, true)]);

        submit(&service, &tx, request(3, first, 1, 0));
        assert_eq!(receive(&rx), (3, Err(RenderError::Stale)));
    }

    #[test]
    fn failed_open_keeps_the_current_document() {
        let (service, log, _) = service(false);
        let doc = service.open(Path::new("a.pdf")).unwrap().id;
        let (tx, rx) = mpsc::channel();
        submit(&service, &tx, request(1, doc, 1, 0));
        receive(&rx).1.unwrap();

        let error = service.open(Path::new("bad.pdf")).unwrap_err();
        assert!(matches!(
            error,
            RenderError::Worker {
                kind: ErrorKind::Format,
                ..
            }
        ));

        submit(&service, &tx, request(2, doc, 1, 0));
        assert!(receive(&rx).1.is_ok());
        assert_eq!(
            renders(&log).len(),
            1,
            "the cached page should survive a failed open"
        );
    }

    #[test]
    fn dropping_the_service_cancels_pending_work() {
        let (service, log, tokens) = service(true);
        let doc = service.open(Path::new("a.pdf")).unwrap().id;
        let (tx, rx) = mpsc::channel();
        submit(&service, &tx, request(1, doc, 1, 0));
        submit(&service, &tx, request(2, doc, 2, 0));
        wait_until(|| service.shared.lock().jobs.len() == 1);
        let tokens = tokens.unwrap();
        let shared = Arc::clone(&service.shared);
        thread::scope(|scope| {
            scope.spawn(|| drop(service));
            wait_until(|| shared.lock().shutdown);
            tokens.send(()).unwrap();
        });
        let mut results: Vec<(u64, bool)> = (0..2)
            .map(|_| receive(&rx))
            .map(|(id, r)| (id, r.is_ok()))
            .collect();
        results.sort_unstable();
        assert_eq!(results, [(1, true), (2, false)]);
        assert_eq!(
            log.lock().unwrap().last().map(String::as_str),
            Some("close")
        );
    }

    #[test]
    fn revisions_rerender_only_the_changed_pages() {
        let (service, log, _) = service(false);
        let doc = service.open(Path::new("a.pdf")).unwrap().id;
        let (tx, rx) = mpsc::channel();
        let render = |id, page| {
            submit(&service, &tx, request(id, doc, page, 0));
            receive(&rx).1.unwrap()[0]
        };
        assert_eq!((render(1, 1), render(2, 2)), (1, 2));

        service
            .revise(doc, Some(Arc::from(vec![0; 10])), &[2])
            .unwrap();
        assert_eq!((render(3, 1), render(4, 2)), (1, 12), "page 1 stays cached");

        service.revise(doc, None, &[2]).unwrap();
        assert_eq!(render(5, 2), 2);
        assert_eq!(
            log.lock().unwrap()[1..],
            [
                "render 1 100",
                "render 2 100",
                "revise 10",
                "render 2 100",
                "revise 0",
                "render 2 100"
            ]
        );

        assert_eq!(service.revise(doc + 1, None, &[]), Err(RenderError::Stale));
    }

    #[test]
    fn link_queries_run_ahead_of_queued_renders() {
        let (service, log, tokens) = service(true);
        let tokens = tokens.unwrap();
        let doc = service.open(Path::new("a.pdf")).unwrap().id;
        let (tx, rx) = mpsc::channel();
        submit(&service, &tx, request(1, doc, 1, 0));
        submit(&service, &tx, request(2, doc, 2, 0));
        wait_until(|| service.shared.lock().jobs.len() == 1);

        thread::scope(|scope| {
            let links = scope.spawn(|| service.links(doc, 3));
            wait_until(|| service.shared.lock().queries.len() == 1);
            tokens.send(()).unwrap();
            let links = links.join().unwrap().unwrap();
            assert_eq!(links[0].target, LinkTarget::Page { page: 3, top: None });
        });
        tokens.send(()).unwrap();
        assert!(receive(&rx).1.is_ok());
        assert!(receive(&rx).1.is_ok());
        assert_eq!(
            log.lock().unwrap()[1..],
            ["render 1 100", "links 3", "render 2 100"]
        );

        assert_eq!(service.links(doc + 1, 0), Err(RenderError::Stale));
    }

    #[test]
    fn cache_evicts_least_recently_used_entries() {
        let mut cache = Cache {
            budget: 10,
            ..Cache::default()
        };
        let key = |page| CacheKey {
            doc: 1,
            page,
            width: 1,
        };
        cache.insert(key(1), Arc::from(vec![0; 4]));
        cache.insert(key(2), Arc::from(vec![0; 4]));
        assert!(cache.get(&key(1)).is_some());
        cache.insert(key(3), Arc::from(vec![0; 4]));
        assert!(cache.get(&key(2)).is_none());
        assert!(cache.get(&key(1)).is_some());
        assert!(cache.get(&key(3)).is_some());
        assert_eq!(cache.used, 8);
        cache.insert(key(4), Arc::from(vec![0; 11]));
        assert!(cache.get(&key(4)).is_none());
    }
}
