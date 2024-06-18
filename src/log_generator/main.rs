use clap::Parser;
use rand::{distributions::Alphanumeric, Rng}; // 0.8

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Args {
    /// The length of the lines to generate.
    #[arg(short, long, default_value_t = 200)]
    name: usize,

    /// Time to sleep between lines in milliseconds.
    #[arg(short, long, default_value_t = 100)]
    sleep: usize,

    /// Number of burst lines to generate.
    #[arg(short, long, default_value_t = 10)]
    burst: usize,
}

fn generate_line(length: usize) -> String {
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(length)
        .map(char::from)
        .collect()
}

fn main() {
    let args = Args::parse();

    for _ in 0..args.burst {
        println!("{}", generate_line(args.name));
    }

    loop {
        println!("{}", generate_line(args.name));
        std::thread::sleep(std::time::Duration::from_millis(args.sleep as u64));
    }
}
