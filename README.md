[![Docker CI](https://github.com/ptorsten/aw-gateway-rs/actions/workflows/docker-image.yaml/badge.svg)](https://github.com/ptorsten/aw-gateway-rs/actions/workflows/docker-image.yaml)
[![Docker Release CI](https://github.com/ptorsten/aw-gateway-rs/actions/workflows/docker-image-release.yml/badge.svg)](https://github.com/ptorsten/aw-gateway-rs/actions/workflows/docker-image-release.yml)
[![Rust](https://github.com/ptorsten/aw-gateway-rs/actions/workflows/rust.yml/badge.svg)](https://github.com/ptorsten/aw-gateway-rs/actions/workflows/rust.yml)

### Overview

!! Still under development - feel free to help !!

AW-Gateway-RS is a Service to poll Ambient Weather gateways (like OBSERVER IP) and report sensors to Home Assistant. The service works 100% locally without any cloud service.

### Functionality

- Supports custom sensor definitions
- Supports auto-discovery for Home Assistant for sensors
- Docker support

### Installation

Docker pre-build can be found here

- Configuration file is expected to be found at ```/config/settings.toml```
- Log files will be written to ```/config/logs``` by default

### Configuration

```toml
[config]
gateways = [
    "<gateway ip>",
]
# global config for sensors
sensors = "sensors.json"
poll_interval_sec = 60

[log]
files = 5
rotate_size = "50MB"
directory = "logs"
stdout_level = "info"
logfile_level = "debug"

[mqtt]
user = ""
password = ""
host = "mqtt://<mqtt server>?client_id=<unique_id>"
keep_alive = 20

[192.168.1.10] # <gateway ip>
name = "gateway"
# Local added sensors, gets merged with global sensor config
sensors = "sensor_190.json"

```

The system supports sensor config per gateway by configurating a sensor json per gateway, and it gets merged with the global config.

#### Sensor Configuration

