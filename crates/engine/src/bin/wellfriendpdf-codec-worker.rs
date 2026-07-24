use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use wellfriendpdf_engine::codec_isolation::{
    worker_handle_request, write_worker_response, CodecWorkerRequest,
};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<(), String> {
    let mut request_path = None;
    let mut response_path = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--request" => request_path = args.next().map(PathBuf::from),
            "--response" => response_path = args.next().map(PathBuf::from),
            "--version" => {
                println!(
                    "{}",
                    wellfriendpdf_engine::codec_isolation::CODEC_WORKER_VERSION
                );
                return Ok(());
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }

    let request_path = request_path.ok_or_else(|| "--request is required".to_string())?;
    let response_path = response_path.ok_or_else(|| "--response is required".to_string())?;

    match std::env::var("WELLFRIENDPDF_CODEC_WORKER_SELF_TEST")
        .ok()
        .as_deref()
    {
        Some("nonzero") => std::process::exit(71),
        Some("crash") => std::process::exit(72),
        Some("timeout") => std::thread::sleep(Duration::from_secs(5)),
        Some("malformed") => {
            std::fs::write(response_path, b"{not-json")
                .map_err(|err| format!("failed to write malformed response: {err}"))?;
            return Ok(());
        }
        _ => {}
    }

    let request_bytes =
        std::fs::read(&request_path).map_err(|err| format!("failed to read request: {err}"))?;
    let request: CodecWorkerRequest = serde_json::from_slice(&request_bytes)
        .map_err(|err| format!("failed to parse request JSON: {err}"))?;
    let mode = std::env::var("WELLFRIENDPDF_CODEC_WORKER_SELF_TEST").ok();
    let response = worker_handle_request(request, mode.as_deref());
    write_worker_response(&response_path, &response)
        .map_err(|err| format!("failed to write response: {err}"))?;
    Ok(())
}
