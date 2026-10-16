//! Files that serde_yaml 0.9 wrote, and the venues scope ships, read the
//! same with serde-saphyr (#118).
//!
//! `tests/fixtures/legacy-yaml/<name>.yaml` is what serde_yaml wrote for a
//! populated value, and `<name>.json` is what serde_yaml read back from it.
//! Both were recorded with serde_yaml before the move. A venue's `.json` is
//! the YAML data model serde_yaml read from the shipped `venues/<id>.yaml`.

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::path::{Path, PathBuf};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy-yaml")
}

fn recorded(name: &str) -> Value {
    let text = std::fs::read_to_string(fixtures().join(format!("{name}.json"))).unwrap();
    serde_json::from_str(&text).unwrap()
}

/// Reads `<name>.yaml` as `T` with serde-saphyr and compares it with what
/// serde_yaml read. Then writes it with serde-saphyr and reads it back:
/// the next save must keep every field.
fn same_as_serde_yaml<T: Serialize + DeserializeOwned>(name: &str) {
    let yaml = std::fs::read_to_string(fixtures().join(format!("{name}.yaml"))).unwrap();
    let read: T = serde_saphyr::from_str(&yaml).unwrap_or_else(|e| panic!("{name}: {e}"));
    let now = serde_json::to_value(&read).unwrap();
    assert_eq!(now, recorded(name), "{name}: read differs from serde_yaml");

    let rewritten = serde_saphyr::to_string(&read).unwrap();
    let again: T = serde_saphyr::from_str(&rewritten).unwrap_or_else(|e| panic!("{name}: {e}"));
    assert_eq!(
        serde_json::to_value(&again).unwrap(),
        now,
        "{name}: a save and a load change it:\n{rewritten}"
    );
}

#[test]
fn test_a_config_written_by_setup_reads_the_same() {
    same_as_serde_yaml::<scope::Config>("config-readme");
    same_as_serde_yaml::<scope::Config>("config-default");
}

#[test]
fn test_saved_token_choices_read_the_same() {
    // Includes a native coin (empty address) and a venue field.
    same_as_serde_yaml::<scope::tokens::TokenAliases>("tokens");
}

#[test]
fn test_an_address_book_reads_the_same() {
    // Includes a label with a colon and a tag with spaces.
    same_as_serde_yaml::<scope::domain::address_book::AddressBook>("address-book");
}

#[test]
fn test_every_shipped_venue_reads_the_same() {
    let venues = Path::new(env!("CARGO_MANIFEST_DIR")).join("venues");
    let mut seen = 0;
    for entry in std::fs::read_dir(venues).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "yaml") {
            continue;
        }
        let id = path.file_stem().unwrap().to_string_lossy().to_string();
        let yaml = std::fs::read_to_string(&path).unwrap();
        let now: Value = serde_saphyr::from_str(&yaml).unwrap_or_else(|e| panic!("{id}: {e}"));
        assert_eq!(now, recorded(&format!("venue-{id}")), "{id}");
        // And as the typed descriptor scope loads.
        serde_saphyr::from_str::<scope::market::descriptor::VenueDescriptor>(&yaml)
            .unwrap_or_else(|e| panic!("{id}: {e}"));
        seen += 1;
    }
    assert!(seen >= 11, "only {seen} venues");
}
