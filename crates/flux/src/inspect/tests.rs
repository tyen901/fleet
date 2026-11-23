use super::*;
use crate::{
    input::FLEET_DIRECTORY,
    profile::{read_recipe, SwiftyVerifier, PROFILE_ID},
};
use flux::{Goal, ProtectedNames};
use relative_path::RelativePath;
use std::{fs, sync::atomic::Ordering};

#[tokio::test]
async fn cold_donor_and_warm_deletion_only_sync_account_for_local_reads() {
    let dir = tempfile::tempdir().unwrap();
    let data = b"payload already installed";
    let path: Arc<RelativePath> = RelativePath::new("installed").into();
    let recipe = read_recipe(&path, data.len() as u64, &mut &data[..]).unwrap();
    let goal = Goal::new(PROFILE_ID, vec![(path, recipe)], vec![])
        .unwrap()
        .with_protected_names(ProtectedNames::new([FLEET_DIRECTORY]).unwrap())
        .unwrap();
    let request = Request {
        goal,
        target: dir.path().into(),
        sources: vec![],
        verifier: Arc::new(SwiftyVerifier),
        local: Default::default(),
    };
    fs::write(dir.path().join("old-name"), data).unwrap();
    let bytes = Arc::new(AtomicU64::new(0));
    let cold = prepare(request.clone(), CancellationToken::new(), bytes.clone())
        .await
        .unwrap()
        .expect("sync required");
    assert_eq!(bytes.swap(0, Ordering::Relaxed), data.len() as u64);
    let outcome = cold.materialize(Default::default()).0.await.unwrap();
    assert_eq!(outcome.fetched_bytes, 0);
    assert_eq!(fs::read(dir.path().join("installed")).unwrap(), data);
    fs::write(dir.path().join("extra"), b"unneeded donor").unwrap();
    let warm = prepare(request.clone(), CancellationToken::new(), bytes.clone())
        .await
        .unwrap()
        .expect("sync required");
    assert_eq!(bytes.load(Ordering::Relaxed), 0);
    let outcome = warm.materialize(Default::default()).0.await.unwrap();
    assert_eq!((outcome.deleted_entries, outcome.written_bytes), (1, 0));
    assert!(verify(request, CancellationToken::new(), bytes.clone())
        .await
        .unwrap());
    assert_eq!(bytes.load(Ordering::Relaxed), data.len() as u64);
}

const CHECKPOINT_LINE: &str = "CHECKPOINT ";
const CHILD_ROOT: &str = "FLEET_CHECKPOINT_TEST_CHILD";

fn scan_request(root: &std::path::Path, expected: bool) -> Request {
    let mut entries = Vec::new();
    for entry in fs::read_dir(root).unwrap().map(std::result::Result::unwrap) {
        if !entry.file_type().unwrap().is_file() {
            continue;
        }
        let name = entry.file_name().into_string().unwrap();
        let path: Arc<RelativePath> = RelativePath::new(&name).into();
        let recipe = if expected {
            let mut file = fs::File::open(entry.path()).unwrap();
            read_recipe(&path, file.metadata().unwrap().len(), &mut file).unwrap()
        } else {
            flux::Recipe::new(vec![]).unwrap()
        };
        entries.push((path, recipe));
    }
    let goal = Goal::new(PROFILE_ID, entries, vec![])
        .unwrap()
        .with_protected_names(ProtectedNames::new([FLEET_DIRECTORY]).unwrap())
        .unwrap();
    Request {
        goal,
        target: root.to_owned(),
        sources: vec![],
        verifier: Arc::new(SwiftyVerifier),
        local: Default::default(),
    }
}

struct CheckpointSignal(Arc<AtomicU64>);
impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for CheckpointSignal {
    fn on_event(&self, event: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
        if event.metadata().name() == super::scanner::CHECKPOINT_EVENT {
            use std::io::Write;
            let mut out = std::io::stdout().lock();
            writeln!(out, "{CHECKPOINT_LINE}{}", self.0.load(Ordering::Relaxed)).unwrap();
            out.flush().unwrap();
            // Keep the scanner at the durable boundary until the parent kills this process.
            drop(out);
            std::thread::sleep(std::time::Duration::from_secs(30));
            panic!("parent did not interrupt the scanner");
        }
    }
}

#[tokio::test]
async fn killed_scan_reuses_committed_files() {
    use std::{
        io::{BufRead, BufReader},
        process::{Command, Stdio},
        time::Instant,
    };
    use tracing_subscriber::prelude::*;
    if let Some(root) = std::env::var_os(CHILD_ROOT) {
        let bytes = Arc::new(AtomicU64::new(0));
        tracing_subscriber::registry()
            .with(CheckpointSignal(bytes.clone()))
            .init();
        prepare(
            scan_request(std::path::Path::new(&root), false),
            CancellationToken::new(),
            bytes,
        )
        .await
        .unwrap();
        panic!("scan ended before the first checkpoint");
    }
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("payload");
    fs::create_dir(&root).unwrap();
    if let Some(source) = std::env::var_os("FLEET_TEST_SCAN_SOURCE") {
        for entry in fs::read_dir(source)
            .unwrap()
            .map(std::result::Result::unwrap)
        {
            if entry.file_type().unwrap().is_file() {
                fs::copy(entry.path(), root.join(entry.file_name())).unwrap();
            }
        }
    } else {
        for index in 0..140 {
            fs::write(root.join(index.to_string()), format!("payload-{index}")).unwrap();
        }
    }
    let request = scan_request(&root, true);
    let bytes = Arc::new(AtomicU64::new(0));
    let started = Instant::now();
    assert!(
        prepare(request.clone(), CancellationToken::new(), bytes.clone())
            .await
            .unwrap()
            .is_none()
    );
    let cold_ms = started.elapsed().as_millis();
    let full = bytes.swap(0, Ordering::Relaxed);
    fs::rename(
        root.join(FLEET_DIRECTORY),
        tmp.path().join("control-inventory"),
    )
    .unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "inspect::tests::killed_scan_reuses_committed_files",
            "--nocapture",
        ])
        .env(CHILD_ROOT, &root)
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    let committed = loop {
        line.clear();
        assert_ne!(
            output.read_line(&mut line).unwrap(),
            0,
            "child exited before its first commit"
        );
        if let Some((_, n)) = line.trim().split_once(CHECKPOINT_LINE) {
            break n.parse::<u64>().unwrap();
        }
    };
    child.kill().unwrap();
    assert!(!child.wait().unwrap().success());
    let started = Instant::now();
    assert!(prepare(request, CancellationToken::new(), bytes.clone())
        .await
        .unwrap()
        .is_none());
    let restarted = bytes.load(Ordering::Relaxed);
    assert!(committed > 0 && restarted > 0);
    assert_eq!(committed + restarted, full);
    println!("cold_bytes={full} checkpoint_bytes={committed} restart_bytes={restarted} cold_ms={cold_ms} restart_ms={}",started.elapsed().as_millis());
}
