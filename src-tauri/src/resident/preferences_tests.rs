use super::*;
use serde_json::json;
use std::fs;

#[test]
fn explicit_save_replaces_unknown_versions_without_backups() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("resident.json");
    for (version, preferences) in [
        (26, ResidentPreferences::default()),
        (
            u64::from(PREFERENCE_SCHEMA_VERSION) + 1,
            ResidentPreferences::default(),
        ),
        (
            u64::MAX,
            ResidentPreferences {
                menu_bar_compact: true,
                revision: 5,
                ..Default::default()
            },
        ),
    ] {
        let previous = json!({ "schemaVersion": version, "unknownOption": true });
        assert_eq!(decode(previous.clone()).unwrap_err(), "preferences_version");
        fs::write(
            &path,
            serde_json::to_vec(&json!({
                "preferences": previous,
                "floatingPosition": { "x": 10, "y": 20 }
            }))
            .unwrap(),
        )
        .unwrap();

        save_to_path(&path, &preferences).unwrap();

        let document = super::super::preference_file::read(&path).unwrap();
        assert_eq!(
            document["preferences"],
            serde_json::to_value(&preferences).unwrap()
        );
        assert_eq!(document["floatingPosition"], json!({ "x": 10, "y": 20 }));
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }
}

#[test]
fn invalid_known_configurations_remain_untouched() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("resident.json");
    for previous in [
        json!({ "schemaVersion": PREFERENCE_SCHEMA_VERSION }),
        json!({ "schemaVersion": 37 }),
        json!({ "schemaVersion": "39" }),
        json!({ "enabled": true }),
    ] {
        let content = serde_json::to_vec(&json!({ "preferences": previous })).unwrap();
        fs::write(&path, &content).unwrap();
        assert!(save_to_path(&path, &ResidentPreferences::default()).is_err());
        assert_eq!(fs::read(&path).unwrap(), content);
    }
}

#[test]
fn outdated_requests_cannot_replace_unknown_configurations() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("resident.json");
    let content = br#"{"preferences":{"schemaVersion":39}}"#;
    fs::write(&path, content).unwrap();
    let preferences = ResidentPreferences {
        schema_version: PREFERENCE_SCHEMA_VERSION + 1,
        ..Default::default()
    };
    assert!(save_to_path(&path, &preferences).is_err());
    assert_eq!(fs::read(&path).unwrap(), content);
}

#[test]
fn failed_unknown_version_replacement_preserves_the_original() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("resident.json");
    let content = br#"{"preferences":{"schemaVersion":39}}"#;
    fs::write(&path, content).unwrap();
    let original_permissions = fs::metadata(&path).unwrap().permissions();
    let mut read_only = original_permissions.clone();
    read_only.set_readonly(true);
    fs::set_permissions(&path, read_only).unwrap();

    let result = save_to_path(&path, &ResidentPreferences::default());

    fs::set_permissions(&path, original_permissions).unwrap();
    assert!(result.is_err());
    assert_eq!(fs::read(&path).unwrap(), content);
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
}
