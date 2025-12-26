use fleet_swifty_wire::{parse_mod_srf, parse_repo_spec_json, ModSrfWire};

#[test]
fn repo_spec_allows_bom_unknown_fields_and_string_ports() {
    let json = r#"{
        "repoName":"R",
        "checksum":"ignored",
        "requiredMods":[{"modName":"@m","checkSum":"D41D8CD98F00B204E9800998ECF8427E","enabled":true,"extra":1}],
        "optionalMods":[],
        "clientParameters":"",
        "version":"1",
        "servers":[{"name":"S","address":"127.0.0.1","port":"3000","password":"","battleEye":false,"extraServer":true}],
        "iconImagePath":"icon.png",
        "repoImageChecksum":"abcd",
        "requiredDLCS":["test"],
        "someFutureField":{"a":1}
    }"#;
    let bytes = [b"\xEF\xBB\xBF".as_slice(), json.as_bytes()].concat();

    let spec = parse_repo_spec_json(&bytes).expect("parse repo spec");
    assert_eq!(spec.repo_name, "R");
    assert_eq!(spec.required_mods.len(), 1);
    assert_eq!(spec.servers.len(), 1);
    assert_eq!(spec.servers[0].port, 3000);
}

#[test]
fn mod_srf_json_allows_bom_unknown_fields_and_pascal_or_camel_case() {
    let json = r#"{
        "Name":"@m",
        "Checksum":"D41D8CD98F00B204E9800998ECF8427E",
        "Files":[
            {
                "Path":"addons\\\\a.pbo",
                "Length":0,
                "Checksum":"D41D8CD98F00B204E9800998ECF8427E",
                "Type":"file",
                "Parts":[],
                "ExtraFileField":123
            }
        ],
        "ExtraTopLevelField":false
    }"#;
    let bytes = [b"\xEF\xBB\xBF".as_slice(), json.as_bytes()].concat();

    let srf = match parse_mod_srf(&bytes).expect("parse mod.srf") {
        ModSrfWire::Json(srf) => srf,
        ModSrfWire::LegacyText(_) => panic!("expected json srf"),
    };
    assert_eq!(srf.name, "@m");
    assert_eq!(srf.files.len(), 1);
}

#[test]
fn mod_srf_legacy_text_is_detected_and_parsed() {
    let legacy = concat!(
        "ADDON:@m:1:D41D8CD98F00B204E9800998ECF8427E\r\n",
        "file:addons\\a.pbo:0:0:D41D8CD98F00B204E9800998ECF8427E\r\n"
    );
    let bytes = [b"\xEF\xBB\xBF".as_slice(), legacy.as_bytes()].concat();

    let srf = match parse_mod_srf(&bytes).expect("parse mod.srf") {
        ModSrfWire::LegacyText(srf) => srf,
        ModSrfWire::Json(_) => panic!("expected legacy text srf"),
    };
    assert_eq!(srf.name, "@m");
    assert_eq!(srf.files.len(), 1);
    assert_eq!(srf.files[0].path, "addons/a.pbo");
}
