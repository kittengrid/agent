use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::thread;
use std::thread::sleep;
use std::time::Duration;

fn start_listener<T: 'static + Send + Fn(&str)>(cb: T) {
    let child = Command::new("ping")
        .arg("google.com")
        .stdout(Stdio::piped())
        .spawn()
        .expect("Failed to start ping process");

    println!("Started process: {}", child.id());

    thread::spawn(move || {
        let mut f = BufReader::new(child.stdout.unwrap());
        loop {
            let mut buf = String::new();
            match f.read_line(&mut buf) {
                Ok(_) => {
                    cb(buf.as_str());
                }
                Err(e) => println!("an error!: {:?}", e),
            }
        }
    });
}

fn main() {
    start_listener(|s| {
        println!("Got this back: {}", s);
    });

    sleep(Duration::from_secs(5));
    println!("Done!");
}
