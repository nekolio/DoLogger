//! Kafka sink end-to-end delivery test.
//!
//! Exercises the real producer path against a reachable Kafka broker and then
//! consumes the record back to prove a true round-trip. Because there is no
//! in-process Kafka broker (rskafka needs an external one), this test is
//! gated behind the `sink-kafka` feature AND an environment variable:
//!
//! ```text
//! DOLOG_KAFKA_BROKERS=localhost:9092 cargo test -p dologger-core \
//!   --features sink-kafka --test kafka_delivery
//! ```
//!
//! Without `DOLOG_KAFKA_BROKERS` the test skips (does not fail), so the
//! default workspace gates stay green without a broker.

#![cfg(feature = "sink-kafka")]

use dologger_core::sink::kafka::{KafkaSink, KafkaSinkConfig};
use dologger_core::sink::Sink;
use rskafka::client::partition::UnknownTopicHandling;
use rskafka::client::ClientBuilder;
use std::process::id;
use tokio::runtime::Runtime;

fn brokers() -> Option<String> {
    std::env::var("DOLOG_KAFKA_BROKERS")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

#[test]
fn kafka_produce_and_consume_round_trip() {
    let brokers = match brokers() {
        Some(b) => b,
        None => {
            eprintln!("DOLOG_KAFKA_BROKERS not set — skipping kafka round-trip test");
            return;
        }
    };

    let runtime = Runtime::new().expect("tokio runtime for the rskafka test client");
    let topic = format!(
        "dologger-test-{}-{}",
        id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
    );

    let addrs: Vec<String> = brokers
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    let client = runtime
        .block_on(ClientBuilder::new(addrs).build())
        .expect("connect to kafka broker");

    // Create a fresh single-partition topic, tolerating an existing one.
    let controller = client
        .controller_client()
        .expect("broker must expose a controller");
    let created = runtime.block_on(controller.create_topic(&topic, 1, 1, 5000));
    if let Err(e) = created {
        eprintln!("create_topic (ignored, may already exist): {e}");
    }

    let config = KafkaSinkConfig {
        brokers: brokers.clone(),
        topic: topic.clone(),
        ..KafkaSinkConfig::default()
    };
    let mut sink = KafkaSink::new(config);
    sink.open().expect("kafka sink open");
    sink.write("hello from dologger").expect("produce record");

    // Consume the record back from partition 0.
    let partition_client = runtime
        .block_on(client.partition_client(topic.clone(), 0, UnknownTopicHandling::Retry))
        .expect("partition client");
    let (records, _high_watermark) = runtime
        .block_on(partition_client.fetch_records(0, 1..10_000_000, 2000))
        .expect("fetch records");

    let got: Vec<String> = records
        .iter()
        .filter_map(|r| r.record.value.as_ref())
        .map(|v| String::from_utf8_lossy(v).into_owned())
        .collect();
    assert!(
        got.iter().any(|s| s == "hello from dologger"),
        "produced record not found in consumed set: {got:?}"
    );
    assert_eq!(
        sink.stats().records_sent,
        1,
        "sink must count the successful write"
    );

    sink.close().expect("kafka sink close");
}
