//! One name per app across devices: the same samples the Mac's Swift tests
//! read (OrttaaiSync Tests/Fixtures/app-names.json).

use orttaai_core::apps::display_name;

#[test]
fn every_sample_gets_its_name() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/app-names.json");
    let samples: Vec<serde_json::Value> =
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert!(samples.len() > 20);
    for s in samples {
        let raw = s["raw"].as_str();
        assert_eq!(display_name(raw), s["display"].as_str().unwrap(), "{raw:?}");
    }
}
