use std::{io::Write, sync::OnceLock};
use vpn_core::{api, config, keys, *};
fn identity() -> Identity {
    Identity {
        sid: "client_a".into(),
        net_name: "office".into(),
        node_ip: "10.42.0.2".into(),
        node_id: None,
        network_id: None,
        access_code: None,
    }
}
fn pem() -> &'static str {
    static PEM: OnceLock<String> = OnceLock::new();
    PEM.get_or_init(|| {
        use rsa::pkcs1::EncodeRsaPublicKey;
        let k = rsa::RsaPrivateKey::new(&mut rand::rngs::OsRng, 2048).unwrap();
        rsa::RsaPublicKey::from(k)
            .to_pkcs1_pem(rsa::pkcs1::LineEnding::LF)
            .unwrap()
    })
}
fn master() -> String {
    format!(
        "Address = gateway.example\nSubnet = 10.42.0.1/32\n{}",
        pem()
    )
}
fn zip(entries: &[(&str, &str)]) -> Vec<u8> {
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, data) in entries {
        w.start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        w.write_all(data.as_bytes()).unwrap();
    }
    w.finish().unwrap().into_inner()
}
const CONF: &str = "Name = client_a\nConnectTo = server_master\nInterface = legacy_adapter\n";
#[test]
fn refuses_path_traversal_and_platform_aliases() {
    for path in [
        "../victim",
        "/victim",
        "C:/victim",
        "hosts\\victim",
        "hosts/../victim",
        "hosts/CON",
        "hosts/NUL",
        "hosts/client_a:stream",
    ] {
        assert!(
            config::validate_zip(
                &zip(&[("office/tinc.conf", CONF), (path, "x")]),
                &identity(),
                "tinc0"
            )
            .is_err(),
            "{path}"
        );
    }
}
#[test]
fn refuses_scripts_private_keys_and_exec_directives() {
    for path in [
        "tinc-up",
        "tinc-up.bat",
        "rsa_key.priv",
        "host-up",
        "download.exe",
    ] {
        assert!(config::validate_zip(
            &zip(&[("tinc.conf", CONF), (path, "payload")]),
            &identity(),
            "tinc0"
        )
        .is_err());
    }
    for line in [
        "PrivateKeyFile = /etc/shadow",
        "Device = /dev/random",
        "ScriptsInterpreter = cmd.exe",
        "ProcessPriority = high",
    ] {
        assert!(config::validate_zip(
            &zip(&[("tinc.conf", &format!("{CONF}{line}\n"))]),
            &identity(),
            "tinc0"
        )
        .is_err());
    }
}
#[test]
fn identifies_the_rejected_archive_entry_without_log_control_characters() {
    let error = match config::validate_zip(
        &zip(&[("tinc.conf", CONF), ("legacy-config.txt", "payload")]),
        &identity(),
        "tinc0",
    ) {
        Ok(_) => panic!("unexpectedly accepted an extra archive entry"),
        Err(error) => error,
    };
    assert_eq!(error.code, "unsafe_config");
    assert!(error.message.contains("legacy-config.txt"));
}
#[test]
fn rejects_case_collision_and_mismatched_identity() {
    assert!(config::validate_zip(
        &zip(&[("tinc.conf", CONF), ("hosts/peer", ""), ("hosts/PEER", "")]),
        &identity(),
        "tinc0"
    )
    .is_err());
    assert!(config::validate_zip(
        &zip(&[("tinc.conf", "Name = stranger\nConnectTo = server_master")]),
        &identity(),
        "tinc0"
    )
    .is_err());
}
#[test]
fn validates_nested_layout_and_overrides_adapter() {
    let v =
        config::validate_zip(&zip(&[("office/tinc.conf", CONF)]), &identity(), "tinc0").unwrap();
    assert!(v.files["tinc.conf"].contains("Interface = tinc0"));
    assert!(!v.files["tinc.conf"].contains("legacy_adapter"));
}
#[test]
fn accepts_and_applies_backend_config_contract_fixture() {
    let identity = Identity {
        sid: "client_fixture".into(),
        net_name: "office".into(),
        node_ip: "10.254.0.2".into(),
        node_id: None,
        network_id: None,
        access_code: None,
    };
    let bytes = match std::env::var_os("TINC_BACKEND_FIXTURE") {
        Some(path) => std::fs::read(path).expect("read backend ZIP fixture"),
        None => zip(&[
            (
                "office/tinc.conf",
                include_str!("fixtures/backend-config/tinc.conf"),
            ),
            (
                "office/hosts/server_master",
                include_str!("fixtures/backend-config/server_master"),
            ),
        ]),
    };

    let validated = config::validate_zip(&bytes, &identity, "tinc_test").unwrap();
    assert_eq!(validated.files.len(), 2);
    assert!(validated.files["tinc.conf"].contains("Interface = tinc_test"));
    assert!(validated.files["tinc.conf"].contains("mode = router"));
    assert!(validated.files.contains_key("hosts/server_master"));

    let master = validated.files["hosts/server_master"].clone();
    let root = tempfile::tempdir().unwrap();
    let profile = keys::profile_id("https://server/", &identity);
    let public = keys::ensure_key(root.path(), &profile, &identity).unwrap();
    let active = config::apply(
        root.path(),
        validated,
        &identity,
        &profile,
        &public,
        &master,
    )
    .unwrap();
    let generation = config::generation_path(root.path(), &active);
    let applied_conf = std::fs::read_to_string(generation.join("tinc.conf")).unwrap();
    assert_eq!(applied_conf.matches("Interface = tinc_test").count(), 1);
    assert!(generation.join("hosts/server_master").is_file());
    assert!(generation.join("hosts/client_fixture").is_file());
    assert!(generation.join("rsa_key.priv").is_file());
    assert_eq!(
        config::server_probe_target(root.path(), &active).unwrap(),
        "10.254.0.1".parse::<std::net::Ipv4Addr>().unwrap()
    );
}
#[test]
fn rejects_http_without_explicit_choice_and_never_accepts_credentials() {
    assert!(api::origin("http://localhost:8080", false).is_err());
    assert!(api::origin("http://localhost:8080", true).is_ok());
    for url in [
        "https://user:secret@example.com",
        "https://example.com/path",
        "https://example.com/?token=a",
        "file:///tmp/file",
    ] {
        assert!(api::origin(url, true).is_err());
    }
}
#[test]
fn rejects_html_and_injected_host_commands() {
    assert!(config::host("<html>success</html>", true).is_err());
    assert!(config::host(
        &format!("Address = good\nSubnet = 10.0.0.0/24\n{}", pem()),
        true
    )
    .is_ok());
    assert!(config::host(
        &format!("Subnet = 10.0.0.0/24\nDevice = C:/secret\n{}", pem()),
        true
    )
    .is_err());
    assert!(config::host("-----BEGIN RSA PRIVATE KEY-----", true).is_err());
}
#[test]
fn server_master_requires_exact_cardinality_and_a_32_bit_subnet() {
    let valid = format!(
        "Address = gateway.example\nPort = 655\nSubnet = 10.0.0.1/32\n{}",
        pem()
    );
    config::server_host(&valid).unwrap();
    for invalid in [
        format!("Subnet = 10.0.0.1/32\n{}", pem()),
        format!("Address = a\nAddress = b\nSubnet = 10.0.0.1/32\n{}", pem()),
        format!("Address = a\nSubnet = 10.0.0.1/24\n{}", pem()),
        format!(
            "Address = a\nSubnet = 10.0.0.1/32\nSubnet = 10.0.0.2/32\n{}",
            pem()
        ),
        format!(
            "Address = a\nPort = 1\nPort = 2\nSubnet = 10.0.0.1/32\n{}",
            pem()
        ),
        format!("{valid}\nAddress = after-pem"),
    ] {
        assert!(config::server_host(&invalid).is_err());
    }
}
#[test]
fn client_host_is_bound_to_the_logged_in_node_32_bit_address() {
    let valid = format!("Subnet = 10.42.0.2/32\n{}", pem());
    config::client_host(&valid, "10.42.0.2").unwrap();
    for invalid in [
        format!("Subnet = 10.42.0.3/32\n{}", pem()),
        format!("Subnet = 10.42.0.2/24\n{}", pem()),
        format!("Address = gateway\nSubnet = 10.42.0.2/32\n{}", pem()),
    ] {
        assert!(config::client_host(&invalid, "10.42.0.2").is_err());
    }
}
#[test]
fn upload_retry_preserves_key_and_failed_apply_preserves_active() {
    let root = tempfile::tempdir().unwrap();
    let id = identity();
    let profile = keys::profile_id("https://server/", &id);
    let first = keys::ensure_key(root.path(), &profile, &id).unwrap();
    let path = root.path().join("keys").join(&profile).join("rsa_key.priv");
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(first, keys::ensure_key(root.path(), &profile, &id).unwrap());
    assert_eq!(bytes, std::fs::read(&path).unwrap());
    let valid = || config::validate_zip(&zip(&[("tinc.conf", CONF)]), &id, "tinc0").unwrap();
    let active = config::apply(root.path(), valid(), &id, &profile, &first, &master()).unwrap();
    let before = std::fs::read(root.path().join("active.json")).unwrap();
    assert!(config::apply(
        root.path(),
        valid(),
        &id,
        &profile,
        &first,
        "server returned error"
    )
    .is_err());
    assert_eq!(
        before,
        std::fs::read(root.path().join("active.json")).unwrap()
    );
    assert_eq!(
        config::read_active(root.path())
            .unwrap()
            .unwrap()
            .generation,
        active.generation
    );
    // Corrupted local key must fail without silently rotating it.
    std::fs::write(&path, b"corrupt").unwrap();
    assert!(keys::ensure_key(root.path(), &profile, &id).is_err());
    assert_eq!(std::fs::read(path).unwrap(), b"corrupt");
}
#[test]
fn storage_failure_before_commit_keeps_old_generation() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("active.json"), b"previous pointer").unwrap();
    let valid = config::validate_zip(&zip(&[("tinc.conf", CONF)]), &identity(), "tinc0").unwrap();
    assert!(config::apply(
        root.path(),
        valid,
        &identity(),
        "missing",
        &format!("Subnet = 10.42.0.2/32\n{}", pem()),
        &master()
    )
    .is_err());
    assert_eq!(
        std::fs::read(root.path().join("active.json")).unwrap(),
        b"previous pointer"
    );
}

#[test]
fn stable_ids_isolate_same_names_and_migrate_the_legacy_key() {
    let root = tempfile::tempdir().unwrap();
    let legacy_identity = identity();
    let legacy = keys::profile_id("https://management/", &legacy_identity);
    let before = keys::ensure_key(root.path(), &legacy, &legacy_identity).unwrap();

    let mut first = legacy_identity.clone();
    first.network_id = Some(20);
    first.node_id = Some(101);
    let canonical = keys::profile_id("https://management/", &first);
    assert_ne!(legacy, canonical);
    keys::migrate_legacy_key(root.path(), &legacy, &canonical).unwrap();
    assert_eq!(
        before,
        keys::ensure_key(root.path(), &canonical, &first).unwrap()
    );
    assert!(!root.path().join("keys").join(legacy).exists());

    let mut second = first.clone();
    second.network_id = Some(21);
    second.node_id = Some(102);
    assert_ne!(canonical, keys::profile_id("https://management/", &second));
    let mut other_server = first.clone();
    other_server.node_id = Some(201);
    assert_ne!(
        canonical,
        keys::profile_id("https://other-management/", &other_server)
    );
}

#[test]
fn profile_generations_are_isolated_and_switch_atomically() {
    let root = tempfile::tempdir().unwrap();
    let mut first = identity();
    first.network_id = Some(20);
    first.node_id = Some(101);
    let first_profile = keys::profile_id("https://management/", &first);
    let first_public = keys::ensure_key(root.path(), &first_profile, &first).unwrap();
    let first_active = config::apply(
        root.path(),
        config::validate_zip(&zip(&[("tinc.conf", CONF)]), &first, "tinc0").unwrap(),
        &first,
        &first_profile,
        &first_public,
        &master(),
    )
    .unwrap();
    let first_conf =
        std::fs::read(config::generation_path(root.path(), &first_active).join("tinc.conf"))
            .unwrap();

    let mut second = first.clone();
    second.sid = "client_b".into();
    second.node_ip = "10.42.0.3".into();
    second.network_id = Some(30);
    second.node_id = Some(103);
    let second_profile = keys::profile_id("https://management/", &second);
    let second_public = keys::ensure_key(root.path(), &second_profile, &second).unwrap();
    let second_conf =
        "Name = client_b\nMode = router\nAddressFamily = ipv4\nConnectTo = server_master\n";
    let second_active = config::apply(
        root.path(),
        config::validate_zip(&zip(&[("tinc.conf", second_conf)]), &second, "tinc0").unwrap(),
        &second,
        &second_profile,
        &second_public,
        &master(),
    )
    .unwrap();

    assert_ne!(first_profile, second_profile);
    assert_eq!(
        config::read_active(root.path()).unwrap().unwrap().profile,
        second_profile
    );
    assert_eq!(
        std::fs::read(config::generation_path(root.path(), &first_active).join("tinc.conf"))
            .unwrap(),
        first_conf
    );
    assert!(config::generation_path(root.path(), &second_active)
        .join("hosts/client_b")
        .is_file());
    assert!(!config::generation_path(root.path(), &second_active)
        .join("hosts/client_a")
        .exists());
}

#[test]
fn candidate_key_remains_a_recovery_marker_until_commit_finishes() {
    let root = tempfile::tempdir().unwrap();
    let profile = "profile";
    keys::replace_private_key(root.path(), profile, b"old-private").unwrap();
    keys::store_candidate(root.path(), profile, b"candidate-private").unwrap();

    let old = keys::activate_candidate(root.path(), profile).unwrap();

    assert_eq!(&*old, b"old-private");
    assert!(keys::candidate_exists(root.path(), profile));
    assert_eq!(
        std::fs::read(root.path().join("keys/profile/rsa_key.priv")).unwrap(),
        b"candidate-private"
    );
    keys::discard_candidate(root.path(), profile).unwrap();
    assert!(!keys::candidate_exists(root.path(), profile));
}
