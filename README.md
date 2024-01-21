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

Docker image: ```docker pull ptorstensson/aw-gateway-rs:latest```

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

Easist way to configure sensors is to check the logs for:

```[2024-01-18 07:11:04.210136 +00:00] DEBUG [src/main.rs:289] Failed to find sensor config for 098cce4ea80c3f:light - value <value>```

Easist way to understand more about the sensors is to use the web interface for the gateway or read the specification at https://osswww.ecowitt.net/uploads/20210716/WN1900%20GW1000,1100%20WH2680,2650%20telenet%20v1.6.0%20.pdf

My current config:

```{
    "outdoor_temp": {"class": "temperature", "unit": "°C", "value_template": "{{ value_json.outdoor_temp }}" },
    "indoor_temp": {"class": "temperature", "unit": "°C", "value_template": "{{ value_json.indoor_temp }}" },
    "temp_1": { "name": "outdoor1_temp", "class": "temperature", "unit": "°C", "value_template": "{{ value_json.outdoor1_temp }}" },
    "temp_4": { "name": "vinrum_temp", "class": "temperature", "unit": "°C", "value_template": "{{ value_json.vinrum_temp }}" },
    "temp_5": { "name": "livingroom_temp", "class": "temperature", "unit": "°C", "value_template": "{{ value_json.livingroom_temp }}" },
    "pm25_1": {"class": "pm25", "unit": "µg/m³", "value_template": "{{ value_json.pm25_1 }}"},
    "pm25_1_avg_24h": { "name": "outdoor_pm25_avg_24h", "class": "pm25", "unit": "µg/m³", "value_template": "{{ value_json.outdoor_pm25_avg_24h }}"},
    "in_humidity": {"class": "humidity", "unit": "%", "value_template": "{{ value_json.in_humidity }}"},
    "rel_barometer": {"class": "ATMOSPHERIC_PRESSURE", "unit": "hPa", "value_template": "{{ float(value_json.rel_barometer) }}"},
    "abs_barometer": {"class": "ATMOSPHERIC_PRESSURE", "unit": "hPa", "value_template": "{{ float(value_json.abs_barometer) }}"},
    "day_maxwind": { "class": "wind_speed", "unit": "m/s", "value_template": "{{ value_json.day_maxwind }}"},
    "gust_speed": {"class": "wind_speed", "unit": "m/s", "value_template": "{{ value_json.gust_speed }}"},
    "wind_speed": {"class": "wind_speed", "unit": "m/s", "value_template": "{{ value_json.wind_speed }}"},
    "rain_rate":  {"class": "PRECIPITATION_INTENSITY", "unit": "mm/h", "value_template": "{{ value_json.rain_rate }}"},
    "rain_day":  {"class": "PRECIPITATION", "unit": "mm", "value_template": "{{ value_json.rain_day }}"},
    "rain_week":  {"class": "PRECIPITATION", "unit": "mm", "value_template": "{{ value_json.rain_week }}"},
    "rain_month":  {"class": "PRECIPITATION", "unit": "mm", "value_template": "{{ value_json.rain_month }}"},
    "rain_year":  {"class": "PRECIPITATION", "unit": "mm", "value_template": "{{ value_json.rain_year }}"},
    "rain_event":  {"class": "PRECIPITATION", "unit": "mm", "value_template": "{{ value_json.rain_event }}"},
    "wind_dir":  { "unit": "º", "value_template": "{{ value_json.wind_dir }}"},
    "uv_index":  { "unit": "", "value_template": "{{ value_json.uv_index | int }}"},
    "uv":  {"class": "IRRADIANCE", "unit": "W/m²", "value_template": "{{ value_json.uv }}"},
    "out_humidity": {"class": "humidity", "unit": "%", "value_template": "{{ value_json.out_humidity }}" },
    "soil_moist_1": {"class": "humidity", "unit": "%", "value_template": "{{ value_json.soil_moist_1 }}" },
    "wh31_ch1_status": { "value_template": "{{ value_json.wh31_ch1_status | default(\"\") }}"}
}

```