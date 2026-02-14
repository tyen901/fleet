use inventory::{compute_stamp, ScanPolicy};
use std::fs;
use tempfile::TempDir;

#[test]
#[cfg(unix)]
fn compute_stamp_skips_hidden_dirs() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path();

    fs::write(root.join("visible.txt"), b"ok").expect("write visible.txt");

    let hidden_dir = root.join(".hidden");
    fs::create_dir_all(&hidden_dir).expect("create .hidden");
    fs::write(hidden_dir.join("ignored.bin"), b"ignore").expect("write ignored.bin");

    let perms = fs::metadata(&hidden_dir)
        .expect("stat .hidden")
        .permissions()
        .mode();
    fs::set_permissions(&hidden_dir, fs::Permissions::from_mode(0o000)).expect("chmod .hidden");

    let policy = ScanPolicy::default();
    let stamp = compute_stamp(root, &policy).expect("compute_stamp");
    assert_eq!(stamp.file_count, 1, "should only count visible files");

    fs::set_permissions(&hidden_dir, fs::Permissions::from_mode(perms)).expect("restore perms");
}
