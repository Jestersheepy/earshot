//! Prints mic-in-use events: cargo run -p earshot-core --example micwatch
fn main() {
    println!("in use now: {:?}", earshot_core::micwatch::in_use());
    let (tx, rx) = std::sync::mpsc::channel();
    earshot_core::micwatch::watch(tx, std::time::Duration::from_secs(1), std::time::Duration::from_secs(3));
    for ev in rx {
        println!("{ev:?}");
    }
}
