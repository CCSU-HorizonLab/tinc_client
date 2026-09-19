use vpn_core::engine::Settings;

#[test]
fn windows_installer_settings_contract_uses_owner() {
    let settings: Settings = serde_json::from_str(
        r#"{
            "data_dir":"C:\\ProgramData\\TincClient\\data",
            "tinc_binary":"C:\\Program Files\\TincClientAgent\\tinc\\tincd.exe",
            "interface":"tinc",
            "interface_prefix":32,
            "owner":"S-1-5-21-1000"
        }"#,
    )
    .expect("the clean-install settings.json contract must be accepted by the Agent");

    assert_eq!(settings.owner, "S-1-5-21-1000");
    assert_eq!(settings.interface, "tinc");
    assert_eq!(settings.interface_prefix, 32);
}

#[test]
fn obsolete_windows_installer_owner_field_is_rejected() {
    let obsolete = r#"{
        "data_dir":"C:\\ProgramData\\TincClient\\data",
        "tinc_binary":"C:\\Program Files\\TincClientAgent\\tinc\\tincd.exe",
        "interface":"tinc",
        "interface_prefix":32,
        "owner_windows_sid":"S-1-5-21-1000"
    }"#;

    assert!(serde_json::from_str::<Settings>(obsolete).is_err());
}

#[test]
fn windows_installer_settings_must_not_have_a_utf8_bom() {
    let json = br#"{
        "data_dir":"C:\\ProgramData\\TincClient\\data",
        "tinc_binary":"C:\\Program Files\\TincClientAgent\\tinc\\tincd.exe",
        "interface":"tinc",
        "interface_prefix":32,
        "owner":"S-1-5-21-1001"
    }"#;
    let mut with_bom = vec![0xEF, 0xBB, 0xBF];
    with_bom.extend_from_slice(json);

    assert!(serde_json::from_slice::<Settings>(json).is_ok());
    assert!(serde_json::from_slice::<Settings>(&with_bom).is_err());
}
