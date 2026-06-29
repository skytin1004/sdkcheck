mod cli;
mod fake_openai;
mod models;
mod recipes;
mod report;
mod runner;
mod secrets;

fn main() {
    if let Err(error) = cli::run() {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}
