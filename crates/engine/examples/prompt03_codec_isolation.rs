use wellfriendpdf_engine::{
    decode_filter_with_isolation, flate_encode, CodecIsolationConfig, CodecIsolationPolicy,
};

fn main() {
    let policy = std::env::args()
        .nth(1)
        .as_deref()
        .and_then(CodecIsolationPolicy::parse)
        .unwrap_or(CodecIsolationPolicy::InProcess);

    let input = flate_encode(b"hello wellfriendpdf", 6);
    let config = CodecIsolationConfig::with_policy(policy);
    let result = decode_filter_with_isolation("FlateDecode", &input, &config);

    println!(
        "{}",
        serde_json::to_string_pretty(&result.report).expect("report should serialize")
    );

    if result.report.ok {
        let decoded = result.decoded.expect("successful report has decoded bytes");
        println!("decoded: {}", String::from_utf8_lossy(&decoded));
    } else {
        eprintln!("decode did not complete: {}", result.report.status);
        std::process::exit(1);
    }
}
