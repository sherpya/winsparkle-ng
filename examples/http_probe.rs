use WinSparkle::http::{CancellationToken, HttpClient, HttpRequest, WinHttp};
fn main() {
    let url = std::env::args().nth(1).expect("URL required");
    match WinHttp.get_bytes(
        HttpRequest {
            url: &url,
            headers: &[],
            user_agent: "WinSparkle-NG probe",
        },
        &CancellationToken::default(),
    ) {
        Ok(response) => println!(
            "HTTP {}: {} bytes",
            response.metadata.status,
            response.body.len()
        ),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}
