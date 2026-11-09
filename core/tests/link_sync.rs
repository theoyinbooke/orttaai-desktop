//! Orttaai Link on this PC: the shared record format (against the sample
//! records the Mac's Swift tests write), the sync ledger, and HTTPS that
//! trusts only the linked Mac's key.
#![cfg(feature = "link")]

use orttaai_core::store::{Store, TranscriptionRecord};
use orttaai_core::sync::client::{ec_public_key, key_fingerprint, ClientError, LinkClient};
use orttaai_core::sync::model::*;
use orttaai_core::sync::store::{DeviceInfo, LinkedMac};
use orttaai_core::sync::wire::{LinkInvite, LinkRecord};
use std::path::Path;

const MAC: &str = "6f1d1c9e-1b7a-4c55-9c39-0d3c1e2a7b10";

// ---- The shared format -----------------------------------------------------

fn fixtures() -> Vec<(String, serde_json::Value)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/link");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    files.sort();
    files
        .into_iter()
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .map(|p| {
            let name = p.file_stem().unwrap().to_string_lossy().to_string();
            (
                name,
                serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap(),
            )
        })
        .collect()
}

/// Decodes a payload as this app's type and writes it again: the JSON must
/// carry the same values as the Mac's.
fn round_trip<T: serde::de::DeserializeOwned + serde::Serialize>(
    bytes: &[u8],
) -> serde_json::Value {
    let value: T = serde_json::from_slice(bytes).unwrap();
    serde_json::from_slice(&encode_payload(&value).unwrap()).unwrap()
}

#[test]
fn every_sample_record_reads_and_writes_back_the_same() {
    let samples = fixtures();
    assert_eq!(
        samples.len(),
        14,
        "copy tests/fixtures/link from the orttaai repo"
    );
    for (name, file) in samples {
        let record: LinkRecord = serde_json::from_value(file["record"].clone()).unwrap();
        // The record itself survives the wire.
        assert_eq!(
            serde_json::to_value(&record).unwrap(),
            file["record"],
            "{name}: record"
        );
        let envelope = record.envelope().unwrap();
        assert_eq!(envelope.problem(), None, "{name}");
        assert_eq!(
            LinkRecord::from(&envelope),
            record,
            "{name}: envelope round trip"
        );
        assert_eq!(
            Envelope::required_schema_version(
                &envelope.id,
                envelope.payload.as_deref(),
                envelope.origin.as_ref()
            ),
            envelope.schema_version,
            "{name}: schema version"
        );

        let Some(bytes) = envelope.payload.as_deref() else {
            continue;
        };
        let original: serde_json::Value = serde_json::from_slice(bytes).unwrap();
        assert_eq!(original, file["payloadReadable"], "{name}: payload");
        let written = match envelope.kind().unwrap() {
            RecordKind::Dictation => round_trip::<DictationPayload>(bytes),
            RecordKind::DictionaryEntry => round_trip::<DictionaryEntryPayload>(bytes),
            RecordKind::Snippet => round_trip::<SnippetPayload>(bytes),
            RecordKind::Device => round_trip::<DevicePayload>(bytes),
            RecordKind::LearningSuggestion => round_trip::<LearningSuggestionPayload>(bytes),
            RecordKind::WritingInsight => round_trip::<WritingInsightPayload>(bytes),
            _ => original.clone(),
        };
        assert_eq!(written, original, "{name}: payload written back");
    }
}

#[test]
fn record_names_match_the_macs() {
    assert_eq!(
        digest("orttaai"),
        "6e1a123b377ba0b325e7ad4382459caac0421e81f879a4d47a99cb4c7fbe99ab"
    );
    assert_eq!(normalized_key("  Tolu   Oyinbooke \n"), "tolu oyinbooke");
    assert_eq!(
        RecordId::dictionary_entry("cafe\u{301}"),
        RecordId::dictionary_entry("café")
    );
    let file = fixtures()
        .into_iter()
        .find(|(n, _)| n == "dictionary-entry")
        .unwrap()
        .1;
    assert_eq!(
        file["record"]["recordName"],
        RecordId::dictionary_entry(&normalized_key("orta ai")).name
    );
    let snippet = fixtures()
        .into_iter()
        .find(|(n, _)| n == "snippet")
        .unwrap()
        .1;
    assert_eq!(
        snippet["record"]["recordName"],
        RecordId::snippet("my address").name
    );
}

#[test]
fn an_unknown_device_kind_reads_as_unknown() {
    let json = br#"{"app":"Orttaai","deviceID":"a","firstSeenAt":1,"kind":"visionpro","lastSeenAt":1,"name":"X","totals":{"dictations":0,"recordingMs":0,"words":0}}"#;
    let device: DevicePayload = serde_json::from_slice(json).unwrap();
    assert_eq!(device.kind, DeviceKind::Unknown);
}

#[test]
fn a_fractional_millisecond_date_from_swift_reads() {
    let json = br#"{"createdAt":1790000000123.456,"text":"hi"}"#;
    let p: DictationPayload = serde_json::from_slice(json).unwrap();
    assert_eq!(p.created_at, 1_790_000_000_123);
}

fn rev(counter: i64, ms: i64, writer: &str) -> Revision {
    Revision {
        counter,
        modified_ms: ms,
        writer: writer.into(),
    }
}

fn word_envelope(counter: i64, generation: i64, deleted: bool) -> Envelope {
    Envelope {
        id: RecordId::dictionary_entry("lagos"),
        schema_version: 1,
        revision: rev(counter, 0, MAC),
        generation,
        is_deleted: deleted,
        deleted_at_ms: None,
        payload: None,
        origin: None,
        created_at_ms: None,
    }
}

#[test]
fn the_conflict_policy_matches_the_macs() {
    assert!(rev(2, 0, "a") > rev(1, 9, "z"));
    assert!(rev(1, 2, "a") > rev(1, 1, "z"));
    assert!(rev(1, 1, "b") > rev(1, 1, "a"));
    let local = |counter, generation, deleted| LocalState {
        revision: rev(counter, 0, MAC),
        is_deleted: deleted,
        generation,
    };
    assert_eq!(
        decide(None, &word_envelope(1, 0, false)),
        Decision::AcceptRemote
    );
    assert_eq!(
        decide(Some(&local(5, 0, false)), &word_envelope(3, 0, false)),
        Decision::KeepLocal
    );
    assert_eq!(
        decide(Some(&local(3, 0, false)), &word_envelope(5, 0, false)),
        Decision::AcceptRemote
    );
    assert_eq!(
        decide(Some(&local(3, 0, false)), &word_envelope(3, 0, false)),
        Decision::AlreadyCurrent
    );
    // A delete beats an edit in the same life; a re-add (new generation) wins.
    assert_eq!(
        decide(Some(&local(9, 0, false)), &word_envelope(2, 0, true)),
        Decision::AcceptRemote
    );
    assert_eq!(
        decide(Some(&local(9, 0, true)), &word_envelope(2, 1, false)),
        Decision::AcceptRemote
    );
    assert_eq!(
        decide(Some(&local(1, 1, false)), &word_envelope(9, 0, false)),
        Decision::KeepLocal
    );
}

#[test]
fn the_macs_link_parses() {
    let fp = "ab".repeat(32);
    let text = format!("orttaai://link?v=1&id={MAC}&n=Mac%20Studio&h=192.168.1.20,Mac-Studio.local&p=47821&fp={fp}&ts=mac-studio.tail1b0ba8.ts.net#s3cret");
    let invite = LinkInvite::parse(&text).unwrap();
    assert_eq!(invite.name, "Mac Studio");
    assert_eq!(invite.hosts, vec!["192.168.1.20", "Mac-Studio.local"]);
    assert_eq!(invite.port, 47821);
    assert_eq!(
        invite.tailscale_name.as_deref(),
        Some("mac-studio.tail1b0ba8.ts.net")
    );
    assert_eq!(invite.secret, "s3cret");
    assert!(LinkInvite::parse("https://example.com").is_none());
    assert!(LinkInvite::parse(&text.replace("#s3cret", "")).is_none());
}

// ---- The ledger ----------------------------------------------------------------

fn device() -> DeviceInfo {
    DeviceInfo {
        name: "ThinkPad".into(),
        model: None,
        app_version: "0.4.0".into(),
        os_version: "Ubuntu 26.04".into(),
    }
}

fn linked_store() -> Store {
    let store = Store::open_in_memory().unwrap();
    let mac = LinkedMac {
        mac_device_id: MAC.into(),
        name: "Mac Studio".into(),
        fingerprint: "ab".repeat(32),
        token: "t".into(),
        hosts: vec!["192.168.1.20".into()],
        port: 47821,
        tailscale_name: None,
        last_host: None,
    };
    store.sync_link(&mac, &device()).unwrap();
    store
}

fn outgoing(store: &Store) -> Vec<Envelope> {
    store.sync_collect().unwrap();
    store.sync_outgoing(500).unwrap()
}

fn ack_all(store: &Store) {
    for e in outgoing(store) {
        store.sync_acknowledge(&e.id.name, &e.revision).unwrap();
    }
}

#[test]
fn linking_queues_everything_already_here() {
    let store = Store::open_in_memory().unwrap();
    store
        .insert_transcription(&TranscriptionRecord::new(
            "Said before linking",
            Some("Code".into()),
            1200,
            1_790_000_000,
        ))
        .unwrap();
    store
        .upsert_dictionary("orta ai", "Orttaai", false)
        .unwrap();
    let mac = LinkedMac {
        mac_device_id: MAC.into(),
        name: "Mac Studio".into(),
        fingerprint: "ab".repeat(32),
        token: "t".into(),
        hosts: vec![],
        port: 47821,
        tailscale_name: Some("mac.ts.net".into()),
        last_host: None,
    };
    store.sync_link(&mac, &device()).unwrap();
    let out = outgoing(&store);
    let kinds: Vec<_> = out.iter().filter_map(|e| e.kind()).collect();
    assert!(kinds.contains(&RecordKind::Dictation));
    assert!(kinds.contains(&RecordKind::DictionaryEntry));
    assert!(kinds.contains(&RecordKind::Device));
    let me = store.sync_device_id().unwrap();
    let dictation = out
        .iter()
        .find(|e| e.kind() == Some(RecordKind::Dictation))
        .unwrap();
    assert_eq!(dictation.id.zone, Zone::Dictations(me.clone()));
    assert_eq!(
        dictation.origin.as_ref().unwrap().kind,
        DeviceKind::this_platform()
    );
    assert_eq!(dictation.schema_version, 2);
    assert_eq!(dictation.problem(), None);
    let payload: DictationPayload =
        serde_json::from_slice(dictation.payload.as_ref().unwrap()).unwrap();
    assert_eq!(payload.text, "Said before linking");
    assert_eq!(payload.target_app_name.as_deref(), Some("Code"));
    assert_eq!(payload.created_at, 1_790_000_000_000);
    let device = out
        .iter()
        .find(|e| e.kind() == Some(RecordKind::Device))
        .unwrap();
    let info: DevicePayload = serde_json::from_slice(device.payload.as_ref().unwrap()).unwrap();
    assert_eq!(info.device_id, me);
    assert_eq!(info.totals.dictations, 1);
    assert_eq!(device.schema_version, 2);
}

#[test]
fn nothing_is_noted_until_linked() {
    let store = Store::open_in_memory().unwrap();
    store
        .insert_transcription(&TranscriptionRecord::new("Not linked", None, 1000, 1))
        .unwrap();
    assert_eq!(store.sync_collect().unwrap(), 0);
    assert!(store.sync_outgoing(10).unwrap().is_empty());
}

#[test]
fn a_new_dictation_is_queued_and_an_acknowledgement_retires_it() {
    let store = linked_store();
    ack_all(&store);
    assert!(outgoing(&store).is_empty());
    store
        .insert_transcription(&TranscriptionRecord::new(
            "Ship it on Friday",
            Some("Claude Code".into()),
            2000,
            1_790_000_100,
        ))
        .unwrap();
    let out = outgoing(&store);
    assert_eq!(out.len(), 1);
    store
        .sync_acknowledge(&out[0].id.name, &out[0].revision)
        .unwrap();
    assert!(outgoing(&store).is_empty());
}

#[test]
fn deleting_sends_a_tombstone_that_carries_no_text() {
    let store = linked_store();
    let id = store
        .insert_transcription(&TranscriptionRecord::new("Delete me", None, 1000, 5))
        .unwrap();
    ack_all(&store);
    store.delete_transcription(id).unwrap();
    let out = outgoing(&store);
    assert_eq!(out.len(), 1);
    assert!(out[0].is_deleted);
    assert!(out[0].payload.is_none());
    assert_eq!(out[0].problem(), None);
}

#[test]
fn renaming_a_word_deletes_the_old_name_and_re_adding_starts_a_new_life() {
    let store = linked_store();
    let id = store.upsert_dictionary("lagos", "Lagos", false).unwrap();
    ack_all(&store);
    store
        .update_dictionary(id, "eko", "Lagos", false, true)
        .unwrap();
    let out = outgoing(&store);
    let old = out
        .iter()
        .find(|e| e.id == RecordId::dictionary_entry("lagos"))
        .unwrap();
    assert!(old.is_deleted);
    assert!(out
        .iter()
        .any(|e| e.id == RecordId::dictionary_entry("eko") && !e.is_deleted));
    ack_all(&store);
    store.upsert_dictionary("lagos", "LAGOS", false).unwrap();
    let again = outgoing(&store)
        .into_iter()
        .find(|e| e.id == RecordId::dictionary_entry("lagos"))
        .unwrap();
    assert!(!again.is_deleted);
    assert_eq!(again.generation, 1);
}

fn mac_dictation(sync_id: &str, text: &str, counter: i64) -> Envelope {
    Envelope {
        id: RecordId::dictation(sync_id, Some(MAC)),
        schema_version: 1,
        revision: rev(counter, 1_790_000_000_000, MAC),
        generation: 0,
        is_deleted: false,
        deleted_at_ms: None,
        payload: Some(
            encode_payload(&DictationPayload {
                created_at: 1_790_000_000_000,
                text: text.into(),
                target_app_name: Some("Claude Code".into()),
                recording_ms: Some(2500),
                ..Default::default()
            })
            .unwrap(),
        ),
        origin: Some(Origin {
            device_id: Some(MAC.into()),
            kind: DeviceKind::Mac,
        }),
        created_at_ms: Some(1_790_000_000_000),
    }
}

#[test]
fn a_mac_dictation_lands_labelled_and_is_never_sent_back() {
    let store = linked_store();
    ack_all(&store);
    let result = store
        .sync_apply(&[mac_dictation("m1", "Said on the Mac", 7)])
        .unwrap();
    assert_eq!(result.changed, 1);
    assert!(result.kinds.contains(&RecordKind::Dictation));
    let row = store.recent(1).unwrap().remove(0);
    assert_eq!(row.text, "Said on the Mac");
    assert_eq!(row.app.as_deref(), Some("Claude Code"));
    assert!(
        outgoing(&store).is_empty(),
        "records from the Mac aren't changes here"
    );

    // The same record again is a no-op; a newer one updates in place.
    assert_eq!(
        store
            .sync_apply(&[mac_dictation("m1", "Said on the Mac", 7)])
            .unwrap()
            .changed,
        0
    );
    store
        .sync_apply(&[mac_dictation("m1", "Said on the Mac, edited", 8)])
        .unwrap();
    assert_eq!(store.count().unwrap(), 1);
    assert_eq!(store.recent(1).unwrap()[0].text, "Said on the Mac, edited");

    // A deletion from the Mac removes it.
    let mut gone = mac_dictation("m1", "", 9);
    gone.is_deleted = true;
    gone.payload = None;
    store.sync_apply(&[gone]).unwrap();
    assert_eq!(store.count().unwrap(), 0);
    assert!(outgoing(&store).is_empty());
}

#[test]
fn a_local_edit_still_wins_over_an_older_mac_version() {
    let store = linked_store();
    let id = store.upsert_dictionary("lagos", "Lagos", false).unwrap();
    let mine = outgoing(&store)
        .into_iter()
        .find(|e| e.id == RecordId::dictionary_entry("lagos"))
        .unwrap();
    let mut older = word_envelope(0, 0, false);
    older.payload = Some(
        encode_payload(&DictionaryEntryPayload {
            source: "lagos".into(),
            target: "LAG".into(),
            normalized_source: "lagos".into(),
            is_case_sensitive: false,
            is_active: true,
            created_at: 0,
        })
        .unwrap(),
    );
    assert_eq!(store.sync_apply(&[older]).unwrap().changed, 0);
    assert_eq!(
        store
            .list_dictionary()
            .unwrap()
            .into_iter()
            .find(|d| d.id == id)
            .unwrap()
            .target,
        "Lagos"
    );
    assert!(
        outgoing(&store).iter().any(|e| e.id == mine.id),
        "still queued for the Mac"
    );
}

#[test]
fn words_from_the_mac_land_in_the_dictionary() {
    let store = linked_store();
    let mut word = word_envelope(5, 0, false);
    word.payload = Some(
        encode_payload(&DictionaryEntryPayload {
            source: "Lagos".into(),
            target: "Lagos".into(),
            normalized_source: "lagos".into(),
            is_case_sensitive: true,
            is_active: true,
            created_at: 1_790_000_000_000,
        })
        .unwrap(),
    );
    store.sync_apply(&[word]).unwrap();
    let entry = store
        .list_dictionary()
        .unwrap()
        .into_iter()
        .find(|d| d.source == "Lagos")
        .unwrap();
    assert!(entry.is_case_sensitive);
    // Deleted on the Mac: gone here.
    store.sync_apply(&[word_envelope(6, 0, true)]).unwrap();
    assert!(store
        .list_dictionary()
        .unwrap()
        .iter()
        .all(|d| d.source != "Lagos"));
}

#[test]
fn records_from_a_newer_schema_are_set_aside() {
    let store = linked_store();
    let mut future = mac_dictation("f1", "From the future", 3);
    future.schema_version = CURRENT_SCHEMA_VERSION + 1;
    let result = store.sync_apply(&[future]).unwrap();
    assert_eq!(result.changed, 0);
    assert_eq!(result.skipped, 1);
}

#[test]
fn unlinking_stops_noting_changes() {
    let store = linked_store();
    store.sync_unlink().unwrap();
    store
        .insert_transcription(&TranscriptionRecord::new("After unlinking", None, 1000, 9))
        .unwrap();
    assert_eq!(store.sync_collect().unwrap(), 0);
    assert!(store.sync_linked_mac().unwrap().is_none());
}

// ---- HTTPS that trusts one key ------------------------------------------------------

/// A one-shot TLS server answering every request with `body`.
fn serve_once(body: &'static str) -> (u16, String) {
    use std::io::{Read, Write};
    let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
    let cert = rcgen::CertificateParams::new(vec!["orttaai-link.local".into()])
        .unwrap()
        .self_signed(&key)
        .unwrap();
    let der = cert.der().to_vec();
    let fingerprint = key_fingerprint(ec_public_key(&der).unwrap());
    assert_eq!(
        fingerprint,
        key_fingerprint(key.public_key_raw()),
        "the key read from the certificate"
    );
    let config = rustls::ServerConfig::builder_with_provider(std::sync::Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(
        vec![rustls::pki_types::CertificateDer::from(der)],
        rustls::pki_types::PrivateKeyDer::try_from(key.serialize_der()).unwrap(),
    )
    .unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming().take(2) {
            let Ok(tcp) = stream else { continue };
            let conn = rustls::ServerConnection::new(std::sync::Arc::new(config.clone())).unwrap();
            let mut tls = rustls::StreamOwned::new(conn, tcp);
            let mut buf = [0u8; 4096];
            if tls.read(&mut buf).is_err() {
                continue;
            }
            let answer = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len());
            let _ = tls.write_all(answer.as_bytes());
            let _ = tls.flush();
        }
    });
    (port, fingerprint)
}

#[test]
fn https_works_with_the_pinned_key_and_refuses_any_other() {
    let (port, fingerprint) = serve_once(r#"{"ok":true}"#);
    let good = LinkClient::new(&fingerprint, Some("t".into()), "test");
    let answer: serde_json::Value = good
        .get(
            "127.0.0.1",
            port,
            "/link/v1/status",
            std::time::Duration::from_secs(5),
        )
        .unwrap();
    assert_eq!(answer["ok"], true);

    let impostor = LinkClient::new(&"00".repeat(32), None, "test");
    let refused = impostor.get::<serde_json::Value>(
        "127.0.0.1",
        port,
        "/link/v1/status",
        std::time::Duration::from_secs(5),
    );
    assert!(
        matches!(refused, Err(ClientError::WrongKey(_))),
        "{refused:?}"
    );
}

// ---- Against the real Mac service ----------------------------------------------------
//
// `swift run LinkTestServer` (orttaai repo, LocalPackages/OrttaaiLink) prints
// one JSON line; pass it in ORTTAAI_LINK_TEST and run with `--ignored`.

#[test]
#[ignore = "needs LinkTestServer running on this machine"]
fn links_and_syncs_with_the_real_mac_service() {
    use orttaai_core::sync::{discover, link_with_code, DiscoveredMac, LinkEngine};
    use std::time::{Duration, Instant};

    let info: serde_json::Value =
        serde_json::from_str(&std::env::var("ORTTAAI_LINK_TEST").expect("ORTTAAI_LINK_TEST"))
            .unwrap();
    let fingerprint = info["fingerprint"].as_str().unwrap().to_string();
    let port = info["port"].as_u64().unwrap() as u16;

    // Found by Bonjour when this runs on another machine; on the Mac
    // itself mDNSResponder owns the port, so fall back to the address.
    let mac = discover(Duration::from_secs(4))
        .unwrap_or_default()
        .into_iter()
        .find(|m| m.fingerprint == fingerprint)
        .unwrap_or(DiscoveredMac {
            mac_device_id: info["macDeviceID"].as_str().unwrap().into(),
            name: "Link Test Mac".into(),
            fingerprint: fingerprint.clone(),
            hosts: vec![],
            port,
            linking: true,
        });
    let mac = DiscoveredMac {
        hosts: vec!["127.0.0.1".into()],
        port,
        ..mac
    };

    let dir = std::env::temp_dir().join(format!("orttaai-link-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("history.db");
    let store = Store::open(&path).unwrap();
    store
        .insert_transcription(&TranscriptionRecord::new(
            "Said on this PC",
            Some("Claude Code".into()),
            1500,
            1_790_000_000,
        ))
        .unwrap();
    let linked = link_with_code(&store, &mac, info["code"].as_str().unwrap(), &device()).unwrap();
    assert_eq!(linked.mac_device_id, MAC);

    let engine = LinkEngine::start(path.clone(), device(), |_| {}, |_| {});
    engine.nudge();
    let started = Instant::now();
    loop {
        let status = engine.status();
        let texts: Vec<String> = Store::open(&path)
            .unwrap()
            .recent(10)
            .unwrap()
            .into_iter()
            .map(|r| r.text)
            .collect();
        if status.counts.waiting == 0
            && texts.iter().any(|t| t == "Said on the Mac")
            && status.state == "idle"
        {
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "didn't sync: {status:?} {texts:?}"
        );
        std::thread::sleep(Duration::from_millis(250));
    }
    // The word from the Mac landed in the dictionary too.
    let words = Store::open(&path).unwrap().list_dictionary().unwrap();
    assert!(words.iter().any(|w| w.source == "Orttaai"));
    engine.stop();
    let _ = std::fs::remove_dir_all(&dir);
}
