# Prometheus Exporter for SMA Inverters

This creates a single binary which exports values from SMA Inverters (both PV and battery).

## Getting Started

You can download a build from the [release page](https://github.com/dr0ps/sma_inverter_exporter/releases/latest). When run it will create a http server running on port 9745 where it exports data for panel voltage and current, battery voltage and current and battery charge.

### Building from Source

You need a Rust/Cargo installation. See https://rustup.rs/. After checking out this repository you can simply run

```
cargo run
```

and if that works for you (point your browser to http://localhost:9745) you can install the binary.

## Configuration

Optionally you can create a config file. You will need to do this if your inverter passwords are not "0000". 
Currently the config file needs to be placed at /etc/sma_inverter_exporter.ini and it should contain one row per inverter:
 ```
 [inverter ip address]=[password]
 ```
for example with two inverters it would look like this:
 ```
192.168.1.101=s3cr3t
192.168.1.102=h4x0r
 ```
(Those are bad password, do not use those anywhere!)

## Deployment

Deployment is dependent on your needs. On a linux machine you will probably want to run this as a service.

Relevant values scraped by Prometheus:

```
Gauges (current values):

smainverter_spot_dc_voltage_millivolts (for two solar panel lines per inverter)
smainverter_spot_dc_current_milliamperes (for two solar panel lines per inverter)
smainverter_battery_voltage_millivolts (for up to three batteries per inverter)
smainverter_battery_current_milliamperes (for up to three batteries per inverter)
smainverter_battery_charge_percentage (for up to three batteries per inverter)

```

## Authors

See the list of [contributors](https://github.com/dr0ps/sma_inverter_exporter/contributors) who participated in this project.

## License

This project is licensed under the GNU GENERAL PUBLIC LICENSE, Version 2 - see the [LICENSE](LICENSE) file for details

## Acknowledgments

With help from SMAloggerAPI by **Harrum** - [SMAloggerAPI](https://github.com/Harrum/SMAloggerAPI)
Also with help from [SBFSpot](https://github.com/SBFspot/SBFspot)

