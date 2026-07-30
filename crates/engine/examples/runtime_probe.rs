use wellfriendpdf_engine::sdk;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let profile = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "minimum".to_string());
    let (vcpu, ram_bytes) = match profile.as_str() {
        "minimum" | "2vcpu-6gb" => (2, 6_u64 * 1024 * 1024 * 1024),
        "recommended" | "4vcpu-8gb" => (4, 8_u64 * 1024 * 1024 * 1024),
        "scaling" | "8vcpu-16gb" => (8, 16_u64 * 1024 * 1024 * 1024),
        other => {
            return Err(format!(
                "unknown runtime probe profile '{other}', expected minimum, recommended, or scaling"
            )
            .into());
        }
    };
    println!("{}", sdk::standard_runtime_probe_json(vcpu, ram_bytes)?);
    Ok(())
}
