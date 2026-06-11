mod cli;
mod config;
mod constants;
mod mcp;
mod qa;

#[tokio::main]
async fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let result = if cli::should_run_cli(&args) {
        cli::run_cli(&args).await
    } else {
        mcp::run_mcp_stdio().await
    };

    if let Err(error) = result {
        eprintln!("qamcp: {error}");
        std::process::exit(1);
    }
}
