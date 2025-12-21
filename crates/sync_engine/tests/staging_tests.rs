use sync_engine::staging::StagedFile;
use sync_engine::types::Durability;

#[tokio::test]
async fn staging_creates_temp_next_to_destination() {
    let tmp = tempfile::tempdir().unwrap();
    let final_path = tmp.path().join("dir").join("file.bin");
    let staged = StagedFile::create_next_to(&final_path).await.unwrap();
    assert_eq!(
        staged.tmp_path.parent().unwrap(),
        final_path.parent().unwrap()
    );
}

#[tokio::test]
async fn staging_commit_replaces_existing_file() {
    let tmp = tempfile::tempdir().unwrap();
    let final_path = tmp.path().join("file.bin");
    tokio::fs::write(&final_path, b"old").await.unwrap();

    let staged = StagedFile::create_next_to(&final_path).await.unwrap();
    tokio::fs::write(&staged.tmp_path, b"new").await.unwrap();
    staged
        .commit(&final_path, Durability::BestEffort)
        .await
        .unwrap();

    let data = tokio::fs::read(&final_path).await.unwrap();
    assert_eq!(data, b"new");
}

#[tokio::test]
async fn staging_commit_removes_temp() {
    let tmp = tempfile::tempdir().unwrap();
    let final_path = tmp.path().join("file.bin");

    let staged = StagedFile::create_next_to(&final_path).await.unwrap();
    let tmp_path = staged.tmp_path.clone();
    tokio::fs::write(&tmp_path, b"data").await.unwrap();
    staged
        .commit(&final_path, Durability::BestEffort)
        .await
        .unwrap();

    assert!(!tmp_path.exists());
}
