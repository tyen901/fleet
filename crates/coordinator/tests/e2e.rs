use axum::routing::get;
use axum::{extract::Path, Router};
use camino::{Utf8Path, Utf8PathBuf};
use manifest_types::{FileManifest, Md5Digest, ModManifest, PartManifest};
use md5::{Digest, Md5};
use serde_json::json;
use std::sync::Arc;
use std::sync::Mutex;
use tokio::net::TcpListener;
use tower_http::services::ServeDir;

fn md5_of(bytes: &[u8]) -> Md5Digest {
    let mut ctx = Md5::new();
    ctx.update(bytes);
    Md5Digest::from_bytes(ctx.finalize().into())
}

fn build_manifest(mod_name: &str, rel_path: &str, bytes: &[u8]) -> ModManifest {
    let part_len = (bytes.len() / 2).max(1) as u64;
    let parts = vec![
        PartManifest {
            start: 0,
            length: part_len,
            checksum: md5_of(&bytes[..part_len as usize]),
        },
        PartManifest {
            start: part_len,
            length: (bytes.len() as u64) - part_len,
            checksum: md5_of(&bytes[part_len as usize..]),
        },
    ];

    let file = FileManifest {
        path: relative_path::RelativePathBuf::from(rel_path),
        length: bytes.len() as u64,
        checksum: manifest_types::file_checksum_from_parts(&parts),
        parts,
    };

    let checksum = manifest_types::mod_checksum_from_files(std::slice::from_ref(&file));

    ModManifest {
        name: mod_name.to_string(),
        checksum,
        files: vec![file],
    }
}

fn repo_json(mod_name: &str) -> serde_json::Value {
    json!({
        "repoName": "test",
        "checksum": "0000000000000000000000000000000000000000",
        "requiredMods": [
            {"modName": mod_name, "checkSum": "00000000000000000000000000000000", "enabled": true}
        ],
        "optionalMods": [],
        "clientParameters": "",
        "repoBasicAuthentication": null,
        "version": "0",
        "servers": []
    })
}

async fn serve_repo_dir(repo_root: &Utf8Path) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new().nest_service("/", ServeDir::new(repo_root.as_std_path()));

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    format!("http://{addr}/")
}

async fn serve_repo_dir_no_range(
    repo_root: &Utf8Path,
    mod_name: &str,
) -> (String, Arc<Mutex<Vec<(u64, u64)>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let calls_clone = calls.clone();
    let root = repo_root.to_path_buf();
    let mod_name = mod_name.to_string();
    let file_route = format!("/{mod_name}/addons/:file");

    let app = Router::new()
        .route(
            &file_route,
            get(move |Path(file): Path<String>| {
                let root = root.clone();
                let calls = calls_clone.clone();
                let mod_name = mod_name.clone();
                async move {
                    let path = root.join(mod_name).join("addons").join(file);
                    let bytes = tokio::fs::read(path).await.unwrap();
                    if let Ok(mut c) = calls.lock() {
                        c.push((0, bytes.len() as u64));
                    }
                    bytes
                }
            }),
        )
        .nest_service("/", ServeDir::new(repo_root.as_std_path()));

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    (format!("http://{addr}/"), calls)
}

async fn write_repo_files(
    root: &Utf8Path,
    mod_name: &str,
    manifest: &ModManifest,
    file_rel: &str,
    file_bytes: &[u8],
) {
    let repo_json_path = root.join("repo.json");
    tokio::fs::write(
        repo_json_path.as_std_path(),
        serde_json::to_vec(&repo_json(mod_name)).unwrap(),
    )
    .await
    .unwrap();

    let manifest_dir = root.join(mod_name);
    tokio::fs::create_dir_all(manifest_dir.as_std_path())
        .await
        .unwrap();
    let manifest_path = manifest_dir.join("manifest.json");
    tokio::fs::write(
        manifest_path.as_std_path(),
        serde_json::to_vec(manifest).unwrap(),
    )
    .await
    .unwrap();

    let file_path = root.join(mod_name).join(file_rel);
    if let Some(parent) = file_path.parent() {
        tokio::fs::create_dir_all(parent.as_std_path())
            .await
            .unwrap();
    }
    tokio::fs::write(file_path.as_std_path(), file_bytes)
        .await
        .unwrap();
}

fn apply_opts(index: local_index::LocalIndex) -> sync_apply::ApplyOptions {
    sync_apply::ApplyOptions {
        full_download_part_threshold: 10_000,
        full_download_byte_ratio_threshold: 1.0,
        index: Some(index),
        ..sync_apply::ApplyOptions::default()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn e2e_repairs_corruption_and_missing_files_with_index_reset() {
    let mod_name = "@mod";
    let rel_path = "addons/data.bin";
    let file_bytes: Vec<u8> = (0..4096).map(|i| (i % 251) as u8).collect();
    let manifest = build_manifest(mod_name, rel_path, &file_bytes);

    let temp = tempfile::tempdir().unwrap();
    let repo_root = Utf8Path::from_path(temp.path()).unwrap();
    write_repo_files(repo_root, mod_name, &manifest, rel_path, &file_bytes).await;
    let base_url = serve_repo_dir(repo_root).await;

    let temp_checkout = tempfile::tempdir().unwrap();
    let checkout = Utf8Path::from_path(temp_checkout.path()).unwrap();
    let checkout_buf = Utf8PathBuf::from_path_buf(temp_checkout.path().to_path_buf()).unwrap();

    let index = local_index::LocalIndex::open(checkout).await.unwrap();
    let apply = apply_opts(index);

    coordinator::sync_checkout_with_events(
        &base_url,
        &checkout_buf,
        coordinator::SyncOptions {
            apply,
            ..Default::default()
        },
        None,
    )
    .await
    .unwrap();

    let final_path = checkout.join(mod_name).join(rel_path);
    let on_disk = tokio::fs::read(final_path.as_std_path()).await.unwrap();
    assert_eq!(on_disk, file_bytes);

    let extra_file = checkout.join(mod_name).join("addons/extra.bin");
    tokio::fs::create_dir_all(extra_file.parent().unwrap().as_std_path())
        .await
        .unwrap();
    tokio::fs::write(extra_file.as_std_path(), b"stale")
        .await
        .unwrap();

    let mut corrupted = file_bytes.clone();
    corrupted[10] ^= 0xFF;
    tokio::fs::write(final_path.as_std_path(), corrupted)
        .await
        .unwrap();

    let index_path = checkout.join(".fleet").join("index.sqlite");
    tokio::fs::remove_file(index_path.as_std_path())
        .await
        .unwrap();

    let index = local_index::LocalIndex::open(checkout).await.unwrap();
    let apply = apply_opts(index);

    coordinator::sync_checkout_with_events(
        &base_url,
        &checkout_buf,
        coordinator::SyncOptions {
            apply,
            ..Default::default()
        },
        None,
    )
    .await
    .unwrap();

    let repaired = tokio::fs::read(final_path.as_std_path()).await.unwrap();
    assert_eq!(repaired, file_bytes);
    assert!(!extra_file.exists());

    tokio::fs::remove_file(final_path.as_std_path())
        .await
        .unwrap();

    let index = local_index::LocalIndex::open(checkout).await.unwrap();
    let apply = apply_opts(index);

    coordinator::sync_checkout_with_events(
        &base_url,
        &checkout_buf,
        coordinator::SyncOptions {
            apply,
            ..Default::default()
        },
        None,
    )
    .await
    .unwrap();

    let restored = tokio::fs::read(final_path.as_std_path()).await.unwrap();
    assert_eq!(restored, file_bytes);
    assert!(index_path.exists());

    let _ = base_url;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn e2e_resumes_partial_tmp_download() {
    let mod_name = "@mod";
    let rel_path = "addons/data.bin";
    let file_bytes: Vec<u8> = (0..10_240).map(|i| (i % 251) as u8).collect();
    let manifest = build_manifest(mod_name, rel_path, &file_bytes);

    let temp = tempfile::tempdir().unwrap();
    let repo_root = Utf8Path::from_path(temp.path()).unwrap();
    write_repo_files(repo_root, mod_name, &manifest, rel_path, &file_bytes).await;
    let base_url = serve_repo_dir(repo_root).await;

    let temp_checkout = tempfile::tempdir().unwrap();
    let checkout = Utf8Path::from_path(temp_checkout.path()).unwrap();
    let checkout_buf = Utf8PathBuf::from_path_buf(temp_checkout.path().to_path_buf()).unwrap();

    let tmp_path = checkout.join(mod_name).join("addons").join(format!(
        ".fleet_tmp_{}_data.bin.part",
        manifest.files[0].checksum.to_hex_upper()
    ));
    tokio::fs::create_dir_all(tmp_path.parent().unwrap().as_std_path())
        .await
        .unwrap();
    tokio::fs::write(tmp_path.as_std_path(), &file_bytes[..2048])
        .await
        .unwrap();

    let apply = apply_opts(local_index::LocalIndex::open(checkout).await.unwrap());
    coordinator::sync_checkout_with_events(
        &base_url,
        &checkout_buf,
        coordinator::SyncOptions {
            apply,
            ..Default::default()
        },
        None,
    )
    .await
    .unwrap();

    let final_path = checkout.join(mod_name).join(rel_path);
    let on_disk = tokio::fs::read(final_path.as_std_path()).await.unwrap();
    assert_eq!(on_disk, file_bytes);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn e2e_range_not_honored_falls_back_to_full() {
    let mod_name = "@mod";
    let rel_path = "addons/data.bin";
    let file_bytes: Vec<u8> = (0..4096).map(|i| (i % 251) as u8).collect();
    let manifest = build_manifest(mod_name, rel_path, &file_bytes);

    let temp = tempfile::tempdir().unwrap();
    let repo_root = Utf8Path::from_path(temp.path()).unwrap();
    write_repo_files(repo_root, mod_name, &manifest, rel_path, &file_bytes).await;
    let (base_url, calls) = serve_repo_dir_no_range(repo_root, mod_name).await;

    let temp_checkout = tempfile::tempdir().unwrap();
    let checkout = Utf8Path::from_path(temp_checkout.path()).unwrap();
    let checkout_buf = Utf8PathBuf::from_path_buf(temp_checkout.path().to_path_buf()).unwrap();

    let tmp_path = checkout.join(mod_name).join("addons").join(format!(
        ".fleet_tmp_{}_data.bin.part",
        manifest.files[0].checksum.to_hex_upper()
    ));
    tokio::fs::create_dir_all(tmp_path.parent().unwrap().as_std_path())
        .await
        .unwrap();
    tokio::fs::write(tmp_path.as_std_path(), &file_bytes[..1024])
        .await
        .unwrap();

    let apply = apply_opts(local_index::LocalIndex::open(checkout).await.unwrap());
    coordinator::sync_checkout_with_events(
        &base_url,
        &checkout_buf,
        coordinator::SyncOptions {
            apply,
            ..Default::default()
        },
        None,
    )
    .await
    .unwrap();

    let recorded_len = calls.lock().unwrap().len();
    assert_eq!(recorded_len, 1);

    let final_path = checkout.join(mod_name).join(rel_path);
    let on_disk = tokio::fs::read(final_path.as_std_path()).await.unwrap();
    assert_eq!(on_disk, file_bytes);
}
