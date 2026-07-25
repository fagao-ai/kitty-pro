use proxy_core::parse_subscription;
use std::collections::BTreeMap;
use std::io::{self, Read};

fn main() {
    let mut input = String::new();
    io::stdin()
        .read_to_string(&mut input)
        .expect("failed to read subscription from stdin");
    let report = parse_subscription(&input);
    let mut protocols = BTreeMap::new();
    for node in &report.nodes {
        *protocols.entry(node.protocol.label()).or_insert(0usize) += 1;
    }

    println!(
        "nodes={} rejected={}",
        report.nodes.len(),
        report.rejected.len()
    );
    for (protocol, count) in protocols {
        println!("{protocol}={count}");
    }
    if report.nodes.is_empty() {
        std::process::exit(2);
    }
}
