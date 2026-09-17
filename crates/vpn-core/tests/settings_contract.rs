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
