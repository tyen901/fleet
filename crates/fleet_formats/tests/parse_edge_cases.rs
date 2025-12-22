use fleet_formats::{digest::Md5Digest, manifest_json, repo_json};
use serde_json::json;

#[test]
fn manifest_json_strips_bom_normalizes_paths_and_sorts_files() {
    let body = json!({
        "Name": "@x",
        "Checksum": "D41D8CD98F00B204E9800998ECF8427E",
        "Files": [
            {
                "Path": "b\\sub\\z.txt",
                "Length": 1,
                "Checksum": "D41D8CD98F00B204E9800998ECF8427E",
                "Parts": [{"Start": 0, "Length": 1, "Checksum": "D41D8CD98F00B204E9800998ECF8427E"}]
            },
            {
                "Path": "a/y.txt",
                "Length": 1,
                "Checksum": "D41D8CD98F00B204E9800998ECF8427E",
                "Parts": [{"Start": 0, "Length": 1, "Checksum": "D41D8CD98F00B204E9800998ECF8427E"}]
            }
        ]
    });

    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"\xEF\xBB\xBF");
    bytes.extend_from_slice(serde_json::to_string(&body).unwrap().as_bytes());

    let parsed = manifest_json::parse_mod_manifest(&bytes).expect("parse manifest json with BOM");

    let paths: Vec<String> = parsed.files.iter().map(|f| f.rel_path.clone()).collect();
    assert_eq!(
        paths,
        vec!["a/y.txt".to_string(), "b/sub/z.txt".to_string()]
    );
}

#[test]
fn repo_json_accepts_port_as_string_or_number() {
    let body = json!({
        "repoName": "R",
        "checkSum": "ignored",
        "requiredMods": [],
        "optionalMods": [],
        "requiredDlcs": [],
        "clientParameters": "",
        "version": "1",
        "servers": [
            {"name":"A","address":"127.0.0.1","port":"2302","password":"","battleEye":false},
            {"name":"B","address":"127.0.0.1","port":2303,"password":"","battleEye":true}
        ]
    });

    let bytes = serde_json::to_vec(&body).unwrap();
    let parsed = repo_json::parse_repo_spec(&bytes).expect("parse repo.json");

    assert_eq!(parsed.servers.len(), 2);
    assert_eq!(parsed.servers[0].port, 2302);
    assert_eq!(parsed.servers[1].port, 2303);

    let d = Md5Digest::parse_hex("d41d8cd98f00b204e9800998ecf8427e").unwrap();
    assert_eq!(d.to_hex_upper(), "D41D8CD98F00B204E9800998ECF8427E");
}
