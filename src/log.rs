#[macro_export]
macro_rules! log {
    ($message:expr) => {
        println!("[sma_inverter_exporter] [{}:{}] {}", file!(), line!(), $message)
    };
}
