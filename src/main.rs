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
use std::net::{Ipv4Addr, Shutdown, SocketAddr};
use std::process::exit;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::{mem, thread};
use tokio::net::TcpListener;

mod inverter;
mod udp_client;
mod log;

lazy_static! {
    static ref LOCK: Arc<Mutex<u32>> = Arc::new(Mutex::new(0_u32));
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
const AC_VOLTAGE: &str = "smainverter_spot_ac_voltage_millivolts";
const AC_CURRENT: &str = "smainverter_spot_ac_current_milliamperes";
const PRODUCTION_TOTAL: &str = "smainverter_metering_total_watthours";
const PRODUCTION_DAILY: &str = "smainverter_metering_daily_watthours";

fn is_discovery_response(response_packet: &[u8]) -> bool {
    // Discovery response packet as per https://cdn.sma.de/fileadmin/content/www.developer.sma.de/docs/SpeedwireDD-TI-en-10.pdf?v=1699275967
    let discovery_response = [
        0x53, 0x4d, 0x41, 0x00, 0x00, 0x04, 0x02, 0xA0,
        0x00, 0x00, 0x00, 0x01, 0x00, 0x02, 0x00, 0x00,
        0x00, 0x01,
    ];

    response_packet.starts_with(&discovery_response)
}

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
            log!(format!("{}", err));
            return Err(err);
        }
    }

    match socket.flush() {
        Ok(_x) => {}

        Err(err) => {
            log!(format!("{}", err));
            return Err(err);
        }
    }

    let mut inverters = Vec::new();
    let mut buf = [MaybeUninit::new(0_u8); 18];
    match socket.set_read_timeout(Some(Duration::from_millis(100))) {
        Ok(_x) => {}

        Err(err) => {
            log!(format!("{}", err));
            return Err(err);
        }
    }
    let mut readable = true;
    while readable {
        match socket.recv_from(&mut buf) {
            Ok((len, remote_addr)) => {
                if len == 18 {
                    let ibuf = unsafe { mem::transmute::<[MaybeUninit<u8>; 18], [u8; 18]>(buf) };
                    if is_discovery_response(&ibuf) {
                        println!("found {}", remote_addr.as_socket().unwrap());
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
            log!(format!("Unable to shutdown socket. {}", error));
        }
    }
    Ok(inverters)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
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

    let gauge_opts = Opts::new(AC_VOLTAGE, "Spot AC voltage");
    let gauge = GaugeVec::new(gauge_opts, &["line"]).unwrap();
    register(Box::new(gauge.borrow().clone())).unwrap();
    gauges.insert(AC_VOLTAGE, gauge);

    let gauge_opts = Opts::new(AC_CURRENT, "Spot AC current");
    let gauge = GaugeVec::new(gauge_opts, &["line"]).unwrap();
    register(Box::new(gauge.borrow().clone())).unwrap();
    gauges.insert(AC_CURRENT, gauge);

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
                    Config::builder().add_source(File::with_name("/etc/sma_inverter_exporter.ini").required(false));

                let settings = match builder.build() {
                    Err(error) => {
                        log!(format!("Config error: {}", error));
                        exit(1);
                    }
                    Ok(config) => config,
                };

                let inverters = match find_inverters() {
                    Ok(found_inverters) => found_inverters,
                    Err(err) => {
                        log!(format!("Error while finding inverters: {}", err));
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
                            log!(format!("Inverter {} error: {}", i.address, inverter_error.message));
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
                        log!(format!("Unable to shutdown socket. {}", error));
                    }
                }
            }

            log!("Getting data from inverters: ");
            for i in &mut logged_in_inverters {
                log!(format!("Getting data from inverter {}.", &i.address.ip().to_string()));
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
                            log!(format!("[{}] Unable to get battery info from inverter. {}",
                                &i.address.ip().to_string(), inverter_error.message));
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
                            log!(format!("[{}] Unable to get DC voltage from inverter. {}",
                                &i.address.ip().to_string(), inverter_error.message));
                        }
                    }
                }
                match i.get_ac_voltage(socket.borrow()) {
                    Ok(data) => {
                        gauges
                            .get(AC_CURRENT)
                            .unwrap()
                            .with_label_values(&["1"])
                            .set(data.current[0] as f64);
                        gauges
                            .get(AC_CURRENT)
                            .unwrap()
                            .with_label_values(&["2"])
                            .set(data.current[1] as f64);
                        gauges
                            .get(AC_CURRENT)
                            .unwrap()
                            .with_label_values(&["3"])
                            .set(data.current[2] as f64);
                        gauges
                            .get(AC_VOLTAGE)
                            .unwrap()
                            .with_label_values(&["1"])
                            .set(data.voltage[0] as f64 * 10_f64);
                        gauges
                            .get(AC_VOLTAGE)
                            .unwrap()
                            .with_label_values(&["2"])
                            .set(data.voltage[1] as f64 * 10_f64);
                        gauges
                            .get(AC_VOLTAGE)
                            .unwrap()
                            .with_label_values(&["3"])
                            .set(data.voltage[2] as f64 * 10_f64);
                    }
                    Err(inverter_error) => {
                        if inverter_error.message.ne("Unsupported") {
                            println!("Inverter error: {}", inverter_error.message);
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
                            log!(format!("Inverter error: {}", inverter_error.message));
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
                            log!(format!("[{}] Unable to get energy production from inverter. {}",
                                &i.address.ip().to_string(), inverter_error.message));
                        }
                    }
                }
            }
            log!("Finished getting data from all inverters.");
        }
    });

    let listener = TcpListener::bind(addr).await?;
    log!(format!("Listening on http://{}", addr));
    loop {
        let (stream, _) = listener.accept().await?;
        let io = TokioIo::new(stream);

        tokio::task::spawn(async move {
            if let Err(err) = http1::Builder::new()
                .serve_connection(io, service_fn(handle))
                .await {
                log!(format!("Error serving connection: {:?}", err));
            }
        });
    }
}
