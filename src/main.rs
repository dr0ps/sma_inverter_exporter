extern crate config;

use crate::inverter::Inverter;
use crate::udp_client::initialize_socket;
use config::{Config, File};
use http_body_util::{combinators::BoxBody, BodyExt, Full};
use hyper::body::Bytes;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use lazy_static::lazy_static;
use prometheus::{gather, register, Encoder, GaugeVec, Opts, TextEncoder};
use socket2::SockAddr;
use std::borrow::Borrow;
use std::collections::HashMap;
use std::convert::Infallible;
use std::io::{Error, Write};
use std::mem::MaybeUninit;
use std::net::{Ipv4Addr, Shutdown, SocketAddr, SocketAddrV4};
use std::process::exit;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::{mem, thread};
use tokio::net::TcpListener;
use clap::{ArgAction, Parser};

use log::info;
use fern::colors::{Color, ColoredLevelConfig};
use std::time::SystemTime;

mod inverter;
mod udp_client;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Turn debugging information on (multiple times increases the level)
    #[arg(short, long, action = ArgAction::Count, default_value_t = 2)]
    verbosity: u8, // The type should be an integer to store the count
}

lazy_static! {
    static ref LOCK: Arc<Mutex<u32>> = Arc::new(Mutex::new(0_u32));
}

fn setup_logging(verbosity: u8) -> Result<(), fern::InitError> {
    let colors_line = ColoredLevelConfig::new()
        .error(Color::Red)
        .warn(Color::Yellow)
        .info(Color::White)
        .debug(Color::White)
        .trace(Color::BrightBlack);

    let colors_level = colors_line.info(Color::Green);

    let mut base_config = fern::Dispatch::new();

    base_config = match verbosity {
        0 => base_config
            // .level(log::LevelFilter::Info),
            .level_for("sma_inverter_exporter", log::LevelFilter::Error),
        1 => base_config
            // .level(log::LevelFilter::Warn),
            .level_for("sma_inverter_exporter", log::LevelFilter::Warn),
        2 => base_config
            // .level(log::LevelFilter::Debug),
            .level_for("sma_inverter_exporter", log::LevelFilter::Info),
        3 => base_config
            // .level(log::LevelFilter::Trace),
            .level_for("sma_inverter_exporter", log::LevelFilter::Debug),
        _4_or_more => base_config
            // .level(log::LevelFilter::Trace),
            .level_for("sma_inverter_exporter", log::LevelFilter::Trace),
    };

    let stdout_config = fern::Dispatch::new()
        .format(move |out, message, record| {
            out.finish(format_args!(
                "{color_line}[{date} {level} {target} {file}:{line} {color_line}] {message}\x1B[0m",
                color_line = format_args!(
                    "\x1B[{}m",
                    colors_line.get_color(&record.level()).to_fg_str()
                ),
                date = humantime::format_rfc3339_seconds(SystemTime::now()),
                target = record.target(),
                file = record.file().unwrap_or("Unknown"),
                line = record.line().unwrap_or(0),
                level = colors_level.color(record.level()),
                message = message,
            ));
        })
        .level(log::LevelFilter::Info)
        .chain(std::io::stdout());

    base_config
        .chain(stdout_config)
        .apply()?;

    Ok(())
}

async fn handle(
    _: Request<hyper::body::Incoming>,
) -> Result<Response<BoxBody<Bytes, Infallible>>, hyper::Error> {
    let mut buffer = vec![];
    let encoder = TextEncoder::new();

    let _lock = LOCK.lock().unwrap();

    let metric_families = gather();
    encoder.encode(&metric_families, &mut buffer).unwrap();

    Ok(Response::new(full(buffer)))
}

fn full<T: Into<Bytes>>(chunk: T) -> BoxBody<Bytes, Infallible> {
    Full::new(chunk.into()).boxed()
}

const BAT_VOLTAGE: &str = "smainverter_battery_voltage_millivolts";
const BAT_CURRENT: &str = "smainverter_battery_current_milliamperes";
const BAT_CHARGE: &str = "smainverter_battery_charge_percentage";
const BAT_TEMPERATURE: &str = "smainverter_battery_temperature_degreescelsius";
const DC_VOLTAGE: &str = "smainverter_spot_dc_voltage_millivolts";
const DC_CURRENT: &str = "smainverter_spot_dc_current_milliamperes";
const PRODUCTION_TOTAL: &str = "smainverter_metering_total_watthours";
const PRODUCTION_DAILY: &str = "smainverter_metering_daily_watthours";

fn find_inverters() -> Result<Vec<Inverter>, Error> {
    let mut socket = initialize_socket(true);
    match socket.send_to(
        [
            0x53, 0x4D, 0x41, 0x00, 0x00, 0x04, 0x02, 0xA0, 0xFF, 0xFF, 0xFF, 0xFF, 0x00, 0x00,
            0x00, 0x20, 0x00, 0x00, 0x00, 0x00,
        ]
        .borrow(),
        &SockAddr::from(SocketAddr::new(
            Ipv4Addr::new(239, 12, 255, 254).into(),
            9522,
        )),
    ) {
        Ok(_size) => {}
        Err(err) => {
            info!("{}", err);
            return Err(err);
        }
    }

    match socket.flush() {
        Ok(_x) => {}

        Err(err) => {
            info!("{}", err);
            return Err(err);
        }
    }

    let mut inverters = Vec::new();
    let mut buf = [MaybeUninit::new(0_u8); 65];
    match socket.set_read_timeout(Some(Duration::from_millis(100))) {
        Ok(_x) => {}

        Err(err) => {
            info!("{}", err);
            return Err(err);
        }
    }
    let mut readable = true;
    while readable {
        match socket.recv_from(&mut buf) {
            Ok((len, remote_addr)) => {
                if len == 65 {
                    let ibuf = unsafe { mem::transmute::<[MaybeUninit<u8>; 65], [u8; 65]>(buf) };
                    if remote_addr.as_socket_ipv4().unwrap().eq(&SocketAddrV4::new(
                        Ipv4Addr::new(ibuf[38], ibuf[39], ibuf[40], ibuf[41]),
                        9522,
                    )) {
                        info!("found {}.{}.{}.{}", ibuf[38], ibuf[39], ibuf[40], ibuf[41]);
                        inverters.push(Inverter::new(remote_addr.as_socket().unwrap()))
                    }
                }
            }
            Err(_err) => {
                readable = false;
            }
        }
    }
    match socket.shutdown(Shutdown::Both) {
        Ok(_result) => {}
        Err(error) => {
            info!("Unable to shutdown socket. {}", error);
        }
    }
    Ok(inverters)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let args = Args::parse();
    let verbosity = args.verbosity;

    setup_logging(verbosity).expect("failed to initialize logging.");

    // Create a Counter.
    let mut gauges: HashMap<&'static str, GaugeVec> = HashMap::new();

    let gauge_opts = Opts::new(BAT_VOLTAGE, "Battery voltage");
    let gauge = GaugeVec::new(gauge_opts, &["line"]).unwrap();
    register(Box::new(gauge.borrow().clone())).unwrap();
    gauges.insert(BAT_VOLTAGE, gauge);

    let gauge_opts = Opts::new(BAT_CURRENT, "Battery current");
    let gauge = GaugeVec::new(gauge_opts, &["line"]).unwrap();
    register(Box::new(gauge.borrow().clone())).unwrap();
    gauges.insert(BAT_CURRENT, gauge);

    let gauge_opts = Opts::new(BAT_CHARGE, "Battery charge");
    let gauge = GaugeVec::new(gauge_opts, &["line"]).unwrap();
    register(Box::new(gauge.borrow().clone())).unwrap();
    gauges.insert(BAT_CHARGE, gauge);

    let gauge_opts = Opts::new(BAT_TEMPERATURE, "Battery temperature");
    let gauge = GaugeVec::new(gauge_opts, &["line"]).unwrap();
    register(Box::new(gauge.borrow().clone())).unwrap();
    gauges.insert(BAT_TEMPERATURE, gauge);

    let gauge_opts = Opts::new(DC_VOLTAGE, "Spot DC voltage");
    let gauge = GaugeVec::new(gauge_opts, &["line"]).unwrap();
    register(Box::new(gauge.borrow().clone())).unwrap();
    gauges.insert(DC_VOLTAGE, gauge);

    let gauge_opts = Opts::new(DC_CURRENT, "Spot DC current");
    let gauge = GaugeVec::new(gauge_opts, &["line"]).unwrap();
    register(Box::new(gauge.borrow().clone())).unwrap();
    gauges.insert(DC_CURRENT, gauge);

    let gauge_opts = Opts::new(PRODUCTION_TOTAL, "Total Production");
    let gauge = GaugeVec::new(gauge_opts, &["inverter"]).unwrap();
    register(Box::new(gauge.borrow().clone())).unwrap();
    gauges.insert(PRODUCTION_TOTAL, gauge);

    let gauge_opts = Opts::new(PRODUCTION_DAILY, "Daily Production");
    let gauge = GaugeVec::new(gauge_opts, &["inverter"]).unwrap();
    register(Box::new(gauge.borrow().clone())).unwrap();
    gauges.insert(PRODUCTION_DAILY, gauge);

    let addr = SocketAddr::from(([0, 0, 0, 0], 9756));

    thread::spawn(move || {
        let mut counter = 0;
        let mut socket = initialize_socket(false);

        let mut logged_in_inverters: Vec<Inverter> = Vec::new();

        loop {
            thread::sleep(Duration::from_secs(10));
            if counter == 0 {
                logged_in_inverters.clear();

                let builder =
                    Config::builder().add_source(File::with_name("/etc/sma_inverter_exporter.ini"));

                let settings = match builder.build() {
                    Err(error) => {
                        info!("Config error: {}", error);
                        exit(1);
                    }
                    Ok(config) => config,
                };

                let inverters = match find_inverters() {
                    Ok(found_inverters) => found_inverters,
                    Err(err) => {
                        info!("Error while finding inverters: {}", err);
                        Vec::new()
                    }
                };

                socket = initialize_socket(false);

                for mut i in inverters.iter().cloned() {
                    let pass_key = format!("{}{}", &i.address.ip().to_string(), ".password");
                    let password = settings
                        .get_string(pass_key.as_str())
                        .unwrap_or("0000".to_string());
                    match i.login(&socket, password.as_str()) {
                        Ok(_result) => {
                            logged_in_inverters.push(i);
                        }
                        Err(inverter_error) => {
                            info!("Inverter {} error: {}", i.address, inverter_error.message);
                        }
                    }
                }
            }

            counter += 1;
            if counter >= 60 {
                counter = 0;

                for i in &mut logged_in_inverters {
                    i.logoff(&socket);
                }

                logged_in_inverters.clear();

                match socket.shutdown(Shutdown::Both) {
                    Ok(_result) => {}
                    Err(error) => {
                        info!("Unable to shutdown socket. {}", error);
                    }
                }
            }

            info!("Getting data: ");
            for i in &mut logged_in_inverters {
                print!("inverter {}, ", &i.address.ip().to_string());
                match i.get_battery_info(&socket) {
                    Ok(data) => {
                        let _lock = LOCK.lock().unwrap();
                        gauges
                            .get(BAT_TEMPERATURE)
                            .unwrap()
                            .with_label_values(&["A"])
                            .set(data.temperature[0] as f64 / 10_f64);
                        gauges
                            .get(BAT_TEMPERATURE)
                            .unwrap()
                            .with_label_values(&["B"])
                            .set(data.temperature[1] as f64 / 10_f64);
                        gauges
                            .get(BAT_TEMPERATURE)
                            .unwrap()
                            .with_label_values(&["C"])
                            .set(data.temperature[2] as f64 / 10_f64);
                        gauges
                            .get(BAT_VOLTAGE)
                            .unwrap()
                            .with_label_values(&["A"])
                            .set(data.voltage[0] as f64 * 10_f64);
                        gauges
                            .get(BAT_VOLTAGE)
                            .unwrap()
                            .with_label_values(&["B"])
                            .set(data.voltage[1] as f64 * 10_f64);
                        gauges
                            .get(BAT_VOLTAGE)
                            .unwrap()
                            .with_label_values(&["C"])
                            .set(data.voltage[2] as f64 * 10_f64);
                        gauges
                            .get(BAT_CURRENT)
                            .unwrap()
                            .with_label_values(&["A"])
                            .set(data.current[0] as f64);
                        gauges
                            .get(BAT_CURRENT)
                            .unwrap()
                            .with_label_values(&["B"])
                            .set(data.current[1] as f64);
                        gauges
                            .get(BAT_CURRENT)
                            .unwrap()
                            .with_label_values(&["C"])
                            .set(data.current[2] as f64);
                    }
                    Err(inverter_error) => {
                        if inverter_error.message.ne("Unsupported") {
                            info!("Inverter error: {}", inverter_error.message);
                        }
                    }
                }
                match i.get_dc_voltage(socket.borrow()) {
                    Ok(data) => {
                        gauges
                            .get(DC_CURRENT)
                            .unwrap()
                            .with_label_values(&["1"])
                            .set(data.current[0] as f64);
                        gauges
                            .get(DC_CURRENT)
                            .unwrap()
                            .with_label_values(&["2"])
                            .set(data.current[1] as f64);
                        gauges
                            .get(DC_VOLTAGE)
                            .unwrap()
                            .with_label_values(&["1"])
                            .set(data.voltage[0] as f64 * 10_f64);
                        gauges
                            .get(DC_VOLTAGE)
                            .unwrap()
                            .with_label_values(&["2"])
                            .set(data.voltage[1] as f64 * 10_f64);
                    }
                    Err(inverter_error) => {
                        if inverter_error.message.ne("Unsupported") {
                            info!("Inverter error: {}", inverter_error.message);
                        }
                    }
                }
                match i.get_battery_charge_status(socket.borrow()) {
                    Ok(data) => {
                        let _lock = LOCK.lock().unwrap();
                        gauges
                            .get(BAT_CHARGE)
                            .unwrap()
                            .with_label_values(&["A"])
                            .set(data[0] as f64);
                        gauges
                            .get(BAT_CHARGE)
                            .unwrap()
                            .with_label_values(&["B"])
                            .set(data[1] as f64);
                        gauges
                            .get(BAT_CHARGE)
                            .unwrap()
                            .with_label_values(&["C"])
                            .set(data[2] as f64);
                    }
                    Err(inverter_error) => {
                        if inverter_error.message.ne("Unsupported") {
                            info!("Inverter error: {}", inverter_error.message);
                        }
                    }
                }
                match i.get_energy_production(socket.borrow()) {
                    Ok(data) => {
                        let _lock = LOCK.lock().unwrap();
                        gauges
                            .get(PRODUCTION_DAILY)
                            .unwrap()
                            .with_label_values(&[&i.address.ip().to_string()])
                            .set(data.daily_wh as f64);
                        gauges
                            .get(PRODUCTION_TOTAL)
                            .unwrap()
                            .with_label_values(&[&i.address.ip().to_string()])
                            .set(data.total_wh as f64);
                    }
                    Err(inverter_error) => {
                        if inverter_error.message.ne("Unsupported") {
                            info!("Inverter error: {}", inverter_error.message);
                        }
                    }
                }
            }
            info!("done.");
        }
    });

    let listener = TcpListener::bind(addr).await?;
    info!("Listening on http://{}", addr);
    loop {
        let (stream, _) = listener.accept().await?;
        let io = TokioIo::new(stream);

        tokio::task::spawn(async move {
            if let Err(err) = http1::Builder::new()
                .serve_connection(io, service_fn(handle))
                .await
            {
                info!("Error serving connection: {:?}", err);
            }
        });
    }
}
