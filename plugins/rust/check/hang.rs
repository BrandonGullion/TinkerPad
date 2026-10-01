// Runs until it is cancelled.
fn main() {
    println!("started");
    loop {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}
